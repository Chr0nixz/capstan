param(
    [string]$EvidenceDirectory = (Join-Path $PSScriptRoot '../target/m4-update'),
    [string]$BaseExecutable = (Join-Path $PSScriptRoot '../target/release/capstan-app.exe'),
    [switch]$IsolatedHarness
)

$ErrorActionPreference = 'Stop'
$repository = (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot '..')).Path
$targetRoot = [IO.Path]::GetFullPath((Join-Path $repository 'target')).TrimEnd('\') + '\'
$evidence = [IO.Path]::GetFullPath($EvidenceDirectory)
if (-not $evidence.StartsWith($targetRoot, [StringComparison]::OrdinalIgnoreCase)) { throw 'Evidence must stay under repository target.' }
if (Test-Path -LiteralPath $evidence) { throw 'Use a fresh evidence directory.' }
if (-not $IsolatedHarness) {
    $principal = [Security.Principal.WindowsPrincipal]::new([Security.Principal.WindowsIdentity]::GetCurrent())
    if ($principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) { throw 'Run full update acceptance as an ordinary user.' }
    if ((Test-Path -LiteralPath (Join-Path $env:APPDATA 'Capstan')) -or (Test-Path -LiteralPath 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\Capstan')) { throw 'Run full update acceptance in a clean user profile.' }
}
$compiler = @('C:\Program Files (x86)\NSIS\makensis.exe', (Join-Path $env:LOCALAPPDATA 'tauri/NSIS/makensis.exe')) | Where-Object { Test-Path -LiteralPath $_ } | Select-Object -First 1
if (-not $compiler) { throw 'NSIS is required.' }
$base = (Resolve-Path -LiteralPath $BaseExecutable).Path
$null = New-Item -ItemType Directory -Path $evidence
$installation = Join-Path $evidence 'installed'
$null = New-Item -ItemType Directory -Path $installation
Copy-Item -LiteralPath $base -Destination (Join-Path $installation 'capstan-app.exe')
Set-Content -LiteralPath (Join-Path $installation 'capstan.portable') -Value 'CAPSTAN_PORTABLE_V1' -NoNewline -Encoding Ascii
$runtime = Join-Path (Split-Path -Parent $base) 'runtime'
if (-not (Test-Path -LiteralPath $runtime)) { $runtime = Join-Path $repository 'target/runtime' }
Copy-Item -LiteralPath $runtime -Destination (Join-Path $installation 'runtime') -Recurse
$null = New-Item -ItemType Directory -Path (Join-Path $installation 'data')
[IO.File]::WriteAllBytes((Join-Path $installation 'data/vault.capstan'), [Text.Encoding]::UTF8.GetBytes('disposable-data-must-not-change'))
$before = @{}
Get-ChildItem -LiteralPath $installation -File -Recurse | ForEach-Object { $before[$_.FullName.Substring($installation.Length)] = (Get-FileHash -LiteralPath $_.FullName).Hash }
$source = @'
Unicode true
!include "LogicLib.nsh"
!include "FileFunc.nsh"
!define CAPSTAN_UPDATE_HARNESS
Var UpdateMode
!include "@HOOKS@"
Name "Capstan update failure probe"
OutFile "@OUTPUT@"
RequestExecutionLevel user
SilentInstall silent
InstallDir "@INSTALLATION@"
Section
  StrCpy $UpdateMode 1
  !insertmacro NSIS_HOOK_PREINSTALL
  FileOpen $0 "$INSTDIR\capstan-app.exe" w
  FileWrite $0 "deliberately broken update"
  FileClose $0
  FileOpen $0 "$INSTDIR\runtime\x86_64\conpty.dll" w
  FileWrite $0 "deliberately broken runtime"
  FileClose $0
  Abort "injected_update_failure"
SectionEnd
'@
$output = Join-Path $evidence 'fault-installer.exe'
$source = $source.Replace('@HOOKS@', (Join-Path $repository 'src-tauri/windows/hooks.nsh')).Replace('@OUTPUT@', $output).Replace('@INSTALLATION@', $installation)
$probe = Join-Path $evidence 'failure-probe.nsi'
[IO.File]::WriteAllText($probe, $source, [Text.UTF8Encoding]::new($true))
& $compiler /V2 $probe
if ($LASTEXITCODE -ne 0) { throw 'Failed to compile the real NSIS hooks in the failure probe.' }
$process = Start-Process -FilePath $output -PassThru -Wait -WindowStyle Hidden
if ($process.ExitCode -eq 0) { throw 'Fault injection must fail installation.' }
foreach ($entry in $before.GetEnumerator()) {
    $file = $installation + $entry.Key
    if ((Get-FileHash -LiteralPath $file).Hash -cne $entry.Value) { throw "Rollback changed $($entry.Key)." }
}
if (-not (Test-Path -LiteralPath ($installation + '.capstan-update-backup/capstan-app.exe'))) { throw 'Rollback must keep its recovery copy.' }
# A second attempt must not consume/overwrite the previous recovery copy.
$backupHash = (Get-FileHash -LiteralPath ($installation + '.capstan-update-backup/capstan-app.exe')).Hash
$process = Start-Process -FilePath $output -PassThru -Wait -WindowStyle Hidden
if ($process.ExitCode -eq 0 -or (Get-FileHash -LiteralPath ($installation + '.capstan-update-backup/capstan-app.exe')).Hash -cne $backupHash) { throw 'Pending recovery was overwritten.' }
# Verify the restored binary can be loaded without creating a real Vault or touching user data.
Remove-Item -LiteralPath (Join-Path $installation 'data/vault.capstan')
$application = Start-Process -FilePath (Join-Path $installation 'capstan-app.exe') -PassThru -WindowStyle Hidden
try {
    Start-Sleep -Seconds 4
    $application.Refresh()
    if ($application.HasExited) { throw 'The restored previous executable did not stay running.' }
} finally {
    if (-not $application.HasExited) { Stop-Process -Id $application.Id -Force }
}
$libraryName = -join ([char[]]@(0x8fd0, 0x884c, 0x20, 0x65f6, 0x2e, 0x64, 0x6c, 0x6c))
$payloadScenarios = @()
foreach ($scenario in @('locked', 'unlocked', 'corrupted', 'missing-checker', 'preexisting-payload-path', 'unmanaged-addition', 'uninstaller-locked', 'uninstaller-unlocked')) {
    $scenarioRoot = Join-Path $evidence $scenario
    $destination = Join-Path $scenarioRoot 'installed'
    $payload = Join-Path $scenarioRoot 'payload'
    $null = New-Item -ItemType Directory -Path (Join-Path $destination 'runtime'), (Join-Path $payload 'runtime')
    [IO.File]::WriteAllText((Join-Path $destination 'capstan-app.exe'), 'old-executable')
    [IO.File]::WriteAllText((Join-Path $destination ('runtime/' + $libraryName)), 'old-runtime')
    [IO.File]::WriteAllText((Join-Path $payload 'capstan-app.exe'), 'new-executable')
    [IO.File]::WriteAllText((Join-Path $payload ('runtime/' + $libraryName)), 'new-runtime')
    [IO.File]::WriteAllText((Join-Path $destination 'settings.json'), '{"keep":"original"}')
    $null = New-Item -ItemType Directory -Path (Join-Path $payload 'runtime/new/deep'), (Join-Path $destination 'runtime/user-empty')
    [IO.File]::WriteAllText((Join-Path $payload 'runtime/new-version-only.dll'), 'new addition')
    [IO.File]::WriteAllText((Join-Path $payload 'runtime/new/deep/nested.dll'), 'new nested addition')
    [IO.File]::WriteAllText((Join-Path $destination 'runtime/user-note.txt'), 'original unmanaged user data')
    if ($scenario -eq 'preexisting-payload-path') {
        [IO.File]::WriteAllText((Join-Path $destination 'runtime/new-version-only.dll'), 'user-original-dll')
        [IO.File]::WriteAllText((Join-Path $destination 'uninstall.exe'), 'old uninstaller')
        $null = New-Item -ItemType Directory -Path (Join-Path $destination 'runtime/new/deep')
    }
    if ($scenario -eq 'uninstaller-locked' -or $scenario -eq 'uninstaller-unlocked') {
        [IO.File]::WriteAllText((Join-Path $destination 'uninstall.exe'), 'old uninstaller')
    }
    $shouldCommit = $scenario -eq 'unlocked' -or $scenario -eq 'uninstaller-unlocked'
    $scenarioBeforeFiles = @{}
    Get-ChildItem -LiteralPath $destination -File -Recurse | ForEach-Object { $scenarioBeforeFiles[$_.FullName.Substring($destination.Length)] = (Get-FileHash -LiteralPath $_.FullName).Hash }
    $scenarioBeforeDirectories = @(Get-ChildItem -LiteralPath $destination -Directory -Recurse | ForEach-Object { $_.FullName.Substring($destination.Length) })
    $config = Join-Path $scenarioRoot 'tauri.conf.json'
    [IO.File]::WriteAllText($config, '{"bundle":{"resources":{"payload/runtime/":"runtime/"}}}')
    $checker = Join-Path $scenarioRoot 'payload-check.ps1'
    python (Join-Path $PSScriptRoot 'release.py') nsis-payload-check (Join-Path $payload 'capstan-app.exe') $config $checker
    if ($LASTEXITCODE -ne 0) { throw 'Failed to generate the real payload verifier.' }
    $fault = switch ($scenario) {
        'corrupted' { 'FileOpen $0 "$INSTDIR\runtime\@LIBRARY@" w' + "`n" + 'FileWrite $0 "bad-runtime"' + "`n" + 'FileClose $0' }
        'missing-checker' { 'Delete "$PLUGINSDIR\capstan-payload-check.ps1"' }
        default { '' }
    }
    if ($scenario -eq 'preexisting-payload-path' -or $scenario -eq 'unmanaged-addition') {
        $fault += 'FileOpen $0 "$INSTDIR\runtime\@LIBRARY@" w' + "`n" + 'FileWrite $0 "bad-runtime"' + "`n" + 'FileClose $0'
    }
    if ($scenario -eq 'unmanaged-addition') {
        $fault += "`n" + 'FileOpen $0 "$INSTDIR\runtime\new\user-note.txt" w' + "`n" + 'FileWrite $0 "late user data"' + "`n" + 'FileClose $0'
    }
    $scenarioSource = @'
Unicode true
!include "LogicLib.nsh"
!include "FileFunc.nsh"
!define CAPSTAN_UPDATE_HARNESS
!define CAPSTAN_UPDATE_PAYLOAD_CHECK "@CHECKER@"
Var UpdateMode
Var TemplateUninstallerError
Var TemplateSizeError
!include "@HOOKS@"
Name "Capstan real extraction regression"
OutFile "@OUTPUT@"
RequestExecutionLevel user
SilentInstall silent
InstallDir "@INSTALLATION@"
Section
  StrCpy $UpdateMode 1
  !insertmacro NSIS_HOOK_PREINSTALL
  SetOutPath "$INSTDIR"
  File "/oname=capstan-app.exe" "@PAYLOAD@\capstan-app.exe"
  File "/oname=runtime\@LIBRARY@" "@PAYLOAD@\runtime\@LIBRARY@"
  CreateDirectory "$INSTDIR\runtime\new\deep"
  File "/oname=runtime\new-version-only.dll" "@PAYLOAD@\runtime\new-version-only.dll"
  File "/oname=runtime\new\deep\nested.dll" "@PAYLOAD@\runtime\new\deep\nested.dll"
  WriteUninstaller "$INSTDIR\uninstall.exe"
  StrCpy $TemplateUninstallerError 0
  IfErrors 0 +2
  StrCpy $TemplateUninstallerError 1
  @FAULT@
  ; Use the real Tauri operation that resets earlier write errors. Record both
  ; flags before any logging I/O, so the uninstaller refusal is not synthetic.
  ${GetSize} "$INSTDIR" "/M=uninstall.exe /S=0K /G=0" $0 $1 $2
  StrCpy $TemplateSizeError 0
  IfErrors 0 +2
  StrCpy $TemplateSizeError 1
  FileOpen $0 "@WRITE_STATUS@" w
  FileWrite $0 "$TemplateUninstallerError,$TemplateSizeError"
  FileClose $0
  !insertmacro CapstanCommitUpdate
SectionEnd
Section Uninstall
SectionEnd
'@
    $scenarioOutput = Join-Path $scenarioRoot 'installer.exe'
    $writeStatus = Join-Path $scenarioRoot 'write-status.txt'
    $scenarioSource = $scenarioSource.Replace('@HOOKS@', (Join-Path $repository 'src-tauri/windows/hooks.nsh')).Replace('@CHECKER@', $checker).Replace('@OUTPUT@', $scenarioOutput).Replace('@INSTALLATION@', $destination).Replace('@PAYLOAD@', $payload).Replace('@FAULT@', $fault).Replace('@LIBRARY@', $libraryName).Replace('@WRITE_STATUS@', $writeStatus)
    $scenarioScript = Join-Path $scenarioRoot 'probe.nsi'
    [IO.File]::WriteAllText($scenarioScript, $scenarioSource, [Text.UTF8Encoding]::new($true))
    & $compiler /V2 $scenarioScript
    if ($LASTEXITCODE -ne 0) { throw "Failed to compile $scenario extraction scenario." }
    $locked = $null
    if ($scenario -eq 'locked') {
        $locked = [IO.File]::Open((Join-Path $destination ('runtime/' + $libraryName)), [IO.FileMode]::Open, [IO.FileAccess]::Read, [IO.FileShare]::Read)
    }
    if ($scenario -eq 'uninstaller-locked') {
        $locked = [IO.File]::Open((Join-Path $destination 'uninstall.exe'), [IO.FileMode]::Open, [IO.FileAccess]::Read, [IO.FileShare]::Read)
    }
    try {
        $process = Start-Process -FilePath $scenarioOutput -PassThru -WindowStyle Hidden
        if (-not $process.WaitForExit(30000)) {
            Stop-Process -Id $process.Id -Force
            throw "Extraction scenario $scenario timed out."
        }
    } finally { if ($null -ne $locked) { $locked.Dispose() } }
    $expectedWriteStatus = if ($scenario -eq 'locked' -or $scenario -eq 'uninstaller-locked') { '1,0' } else { '0,0' }
    if ([IO.File]::ReadAllText($writeStatus) -cne $expectedWriteStatus) { throw "Scenario $scenario did not exercise the expected WriteUninstaller/GetSize error flags." }
    $expectedPrefix = if ($shouldCommit) { 'new' } else { 'old' }
    if ([IO.File]::ReadAllText((Join-Path $destination 'capstan-app.exe')) -cne "$expectedPrefix-executable" -or
        [IO.File]::ReadAllText((Join-Path $destination ('runtime/' + $libraryName))) -cne "$expectedPrefix-runtime" -or
        [IO.File]::ReadAllText((Join-Path $destination 'settings.json')) -cne '{"keep":"original"}') { throw "Payload scenario $scenario changed the wrong files." }
    $hasBackup = Test-Path -LiteralPath ($destination + '.capstan-update-backup/capstan-app.exe')
    if ($shouldCommit) {
        if ($process.ExitCode -ne 0 -or $hasBackup) { throw 'Successful payload verification did not commit.' }
    } elseif ($process.ExitCode -eq 0 -or -not $hasBackup) { throw "Payload scenario $scenario discarded recovery or reported success." }
    if ($shouldCommit) {
        if ([IO.File]::ReadAllText((Join-Path $destination 'runtime/new-version-only.dll')) -cne 'new addition' -or
            [IO.File]::ReadAllText((Join-Path $destination 'runtime/new/deep/nested.dll')) -cne 'new nested addition') { throw 'Successful update lost its added payload.' }
        if ($scenario -eq 'uninstaller-unlocked' -and
            (Get-FileHash -LiteralPath (Join-Path $destination 'uninstall.exe')).Hash -ceq $scenarioBeforeFiles['\uninstall.exe']) { throw 'Successful update retained the old uninstaller.' }
    } else {
        $expectedFiles = [Collections.Generic.HashSet[string]]::new([StringComparer]::OrdinalIgnoreCase)
        foreach ($entry in $scenarioBeforeFiles.GetEnumerator()) {
            $null = $expectedFiles.Add($entry.Key)
            if ((Get-FileHash -LiteralPath ($destination + $entry.Key)).Hash -cne $entry.Value) { throw "Rollback changed original file $($entry.Key)." }
        }
        $expectedDirectories = [Collections.Generic.HashSet[string]]::new([StringComparer]::OrdinalIgnoreCase)
        foreach ($relative in $scenarioBeforeDirectories) { $null = $expectedDirectories.Add($relative) }
        if ($scenario -eq 'unmanaged-addition') {
            $null = $expectedFiles.Add('\runtime\new\user-note.txt')
            $null = $expectedDirectories.Add('\runtime\new')
            if ([IO.File]::ReadAllText((Join-Path $destination 'runtime/new/user-note.txt')) -cne 'late user data') { throw 'Rollback removed late unmanaged user data.' }
        }
        $actualFiles = @(Get-ChildItem -LiteralPath $destination -File -Recurse)
        $actualDirectories = @(Get-ChildItem -LiteralPath $destination -Directory -Recurse)
        foreach ($file in $actualFiles) { if (-not $expectedFiles.Contains($file.FullName.Substring($destination.Length))) { throw "Rollback left unexpected file $($file.Name)." } }
        foreach ($directory in $actualDirectories) { if (-not $expectedDirectories.Contains($directory.FullName.Substring($destination.Length))) { throw "Rollback left unexpected directory $($directory.Name)." } }
        if ($actualFiles.Count -ne $expectedFiles.Count -or $actualDirectories.Count -ne $expectedDirectories.Count) { throw 'Rollback changed the original directory inventory.' }
    }
    if ([IO.File]::ReadAllText((Join-Path $destination 'runtime/user-note.txt')) -cne 'original unmanaged user data' -or
        -not (Test-Path -LiteralPath (Join-Path $destination 'runtime/user-empty') -PathType Container)) { throw 'Update removed original unmanaged paths.' }
    $payloadScenarios += [ordered]@{ scenario = $scenario; exitCode = $process.ExitCode; backupKept = $hasBackup; templateWriteAndSizeErrors = $expectedWriteStatus; passed = $true }
}
# MockRuntime must stay separate from Wry: linking its App builder into the
# desktop test harness requires a different Windows Common Controls manifest.
# Include the production SDK factory so its before-exit policy is tested exactly.
$sdkRoot = Join-Path $evidence 'sdk-probe'
$null = New-Item -ItemType Directory -Path (Join-Path $sdkRoot 'src')
$lockPath = Join-Path $repository 'Cargo.lock'
$sdkVersions = python -c "import json,sys,tomllib; p=tomllib.load(open(sys.argv[1],'rb'))['package']; print(json.dumps({n:next(x['version'] for x in p if x['name']==n) for n in ('tauri','tauri-plugin-updater')}))" $lockPath | ConvertFrom-Json
if ($LASTEXITCODE -ne 0) { throw 'Could not read pinned SDK versions.' }
$sdkManifest = @'
[package]
name = "capstan-update-launch-regression"
version = "0.1.0"
edition = "2021"

[workspace]

[dependencies]
tauri = { version = "=@TAURI@", default-features = false, features = ["test"] }
tauri-plugin-updater = { version = "=@UPDATER@", default-features = false, features = ["native-tls"] }
minisign = "=0.9.1"
base64 = "=0.22.1"
serde_json = "=1.0.151"
'@
$sdkManifest = $sdkManifest.Replace('@TAURI@', $sdkVersions.tauri).Replace('@UPDATER@', $sdkVersions.'tauri-plugin-updater')
[IO.File]::WriteAllText((Join-Path $sdkRoot 'Cargo.toml'), $sdkManifest, [Text.UTF8Encoding]::new($false))
# Reuse transitive versions too, rather than resolving newer cached dependencies.
Copy-Item -LiteralPath $lockPath -Destination (Join-Path $sdkRoot 'Cargo.lock')
$sdkModule = (Join-Path $repository 'src-tauri/src/update_commands/sdk.rs').Replace('\', '\\')
$sdkSource = [IO.File]::ReadAllText((Join-Path $repository 'src-tauri/src/update_commands/sdk_probe.rs')).Replace('@SDK_MODULE@', $sdkModule)
[IO.File]::WriteAllText((Join-Path $sdkRoot 'src/main.rs'), $sdkSource, [Text.UTF8Encoding]::new($false))
$sdkArguments = @('run', '--offline', '--manifest-path', ('"' + (Join-Path $sdkRoot 'Cargo.toml') + '"'), '--target-dir', ('"' + (Join-Path $repository 'target') + '"'))
$sdkProcess = Start-Process -FilePath (Get-Command cargo.exe).Source -ArgumentList $sdkArguments -PassThru -Wait -WindowStyle Hidden -RedirectStandardOutput (Join-Path $evidence 'sdk-probe.log') -RedirectStandardError (Join-Path $evidence 'sdk-probe.stderr.log')
if ($sdkProcess.ExitCode -ne 0) {
    Get-Content -LiteralPath (Join-Path $evidence 'sdk-probe.log') -Tail 30
    Get-Content -LiteralPath (Join-Path $evidence 'sdk-probe.stderr.log') -Tail 30
    throw 'Rejected installer did not preserve application resources and retry.'
}
$report = [ordered]@{ passed = $true; mode = 'isolated-real-hooks'; filesRestored = $before.Count; oldExecutableStarted = $true; payloadScenarios = $payloadScenarios; sdkRejectedLaunchAttempts = 2; sdkResourceDrops = 0; actualVersionUpgrade = $false; abruptPowerLossTested = $false }
$report | ConvertTo-Json | Set-Content -LiteralPath (Join-Path $evidence 'report.json') -Encoding UTF8
Write-Output "NSIS rollback/startup passed; $($before.Count) files restored, $($payloadScenarios.Count) payload scenarios passed, including uninstaller refusals, additions and user paths. SDK rejected both launches with all resources preserved. Full signed version upgrade still requires a protected release."
