param(
    [string]$ExecutablePath = (Join-Path $PSScriptRoot '../target/release/capstan-app.exe'),
    [string]$EvidenceDirectory,
    [switch]$LeaveRunning
)

$ErrorActionPreference = 'Stop'
$repository = (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot '..')).Path
. (Join-Path $PSScriptRoot 'ConPty.ps1')
$executable = (Resolve-Path -LiteralPath $ExecutablePath).Path
$token = [Guid]::NewGuid().ToString('N')
$stage = [IO.Path]::GetFullPath((Join-Path $repository "target/m1-desktop-$token"))
$first = Join-Path $stage 'first'
$moved = Join-Path $stage 'moved'
$blocked = Join-Path $stage 'blocked'
$targetPrefix = (Resolve-Path -LiteralPath (Join-Path $repository 'target')).Path.TrimEnd('\') + '\'
if (-not $stage.StartsWith($targetPrefix, [StringComparison]::OrdinalIgnoreCase)) { throw 'Unexpected desktop test path.' }
if ([string]::IsNullOrWhiteSpace($EvidenceDirectory)) {
    $EvidenceDirectory = Join-Path $repository ('docs/agent/baselines/M1/raw/' + (Get-Date -Format 'yyyy-MM-dd-HHmmss') + '-desktop')
}
$evidence = [IO.Path]::GetFullPath($EvidenceDirectory)
New-Item -ItemType Directory -Path $evidence,$first,$blocked -Force | Out-Null
foreach ($directory in @($first, $blocked)) {
    Copy-Item -LiteralPath $executable -Destination (Join-Path $directory 'Capstan.exe')
    Copy-ConPtyRuntime (Join-Path (Split-Path $executable) 'runtime/x86_64') (Join-Path $directory 'runtime/x86_64')
    [IO.File]::WriteAllBytes((Join-Path $directory 'capstan.portable'), [Text.Encoding]::ASCII.GetBytes('CAPSTAN_PORTABLE_V1'))
}
[IO.File]::WriteAllText((Join-Path $blocked 'data'), 'not a directory')
$results = [Collections.Generic.List[object]]::new()
$failure = $null
$liveProcess = $null

function Test-Launch([string]$Directory, [string]$Name, [bool]$ShouldRun, [bool]$KeepRunning) {
    $info = [Diagnostics.ProcessStartInfo]::new()
    $info.FileName = Join-Path $Directory 'Capstan.exe'
    $info.WorkingDirectory = $Directory
    $info.UseShellExecute = $false
    $info.CreateNoWindow = $true
    $info.RedirectStandardOutput = $true
    $info.RedirectStandardError = $true
    $process = [Diagnostics.Process]::Start($info)
    $stdout = $process.StandardOutput.ReadToEndAsync()
    $stderr = $process.StandardError.ReadToEndAsync()
    $retained = $false
    try {
        $exited = $process.WaitForExit(5000)
        $process.Refresh()
        if ($ShouldRun) {
            if ($exited) { throw "Desktop exited before startup: $Name, exit $($process.ExitCode); $($stderr.GetAwaiter().GetResult())" }
            if ($process.MainWindowHandle -eq [IntPtr]::Zero -or $process.MainWindowTitle -ne 'Capstan') { throw 'Desktop did not create its main window.' }
            if (-not (Test-Path -LiteralPath (Join-Path $Directory 'data') -PathType Container)) { throw 'Desktop setup did not create the portable data directory.' }
            if (Test-Path -LiteralPath (Join-Path $Directory 'data/vault.capstan')) { throw 'Startup unexpectedly created a Vault without user input.' }
        } else {
            if (-not $exited -or $process.ExitCode -eq 0) { throw 'Blocked portable data path did not fail closed.' }
        }
        $result = @{ scenario = $Name; passed = $true; early_exit = $exited; portable_data_checked = $ShouldRun }
        $results.Add($result)
        if ($KeepRunning) {
            $script:liveProcess = $process
            $retained = $true
            Write-Output "Live desktop PID: $($process.Id); path: $($info.FileName)"
        }
    } finally {
        if (-not $retained) {
            if (-not $process.HasExited) {
                $process.CloseMainWindow() | Out-Null
                if (-not $process.WaitForExit(5000)) { $process.Kill(); $process.WaitForExit() }
            }
            [IO.File]::WriteAllText((Join-Path $evidence "$Name.stderr.txt"), $stderr.GetAwaiter().GetResult())
            [IO.File]::WriteAllText((Join-Path $evidence "$Name.stdout.txt"), $stdout.GetAwaiter().GetResult())
            $process.Dispose()
        }
    }
}

try {
    Test-Launch $first 'portable-startup' $true $false
    $source = (Resolve-Path -LiteralPath $first).Path
    $destination = [IO.Path]::GetFullPath($moved)
    if (-not $source.StartsWith($targetPrefix, [StringComparison]::OrdinalIgnoreCase) -or -not $destination.StartsWith($targetPrefix, [StringComparison]::OrdinalIgnoreCase)) { throw 'Move resolved outside target.' }
    Move-Item -LiteralPath $source -Destination $destination
    Test-Launch $moved 'portable-after-move' $true $LeaveRunning.IsPresent
    Test-Launch $blocked 'portable-blocked-data-path' $false $false
} catch { $failure = $_.Exception.Message }
$report = @{
    schema_version = 1
    measured_at = (Get-Date).ToString('o')
    executable_sha256 = (Get-FileHash -LiteralPath $executable -Algorithm SHA256).Hash
    scenarios = $results.ToArray()
    passed = ($null -eq $failure)
    failure = $failure
    vault_created = $false
    live_pid = $(if ($liveProcess) { $liveProcess.Id } else { $null })
    live_executable = $(if ($liveProcess) { Join-Path $moved 'Capstan.exe' } else { $null })
    limitation = 'Native process/window and portable-path smoke only; no Vault unlock or terminal interaction; blocked path uses a file rather than a read-only filesystem.'
}
$report | ConvertTo-Json -Depth 5 | Set-Content -LiteralPath (Join-Path $evidence 'report.json') -Encoding UTF8
Write-Output "Evidence directory: $evidence"
if ($failure) { throw $failure }
