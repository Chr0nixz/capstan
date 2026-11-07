param(
    [Parameter(Mandatory = $true)][string]$BaseInstaller,
    [Parameter(Mandatory = $true)][string]$UpgradeInstaller,
    [Parameter(Mandatory = $true)][string]$FixtureDirectory,
    [Parameter(Mandatory = $true)][string]$EvidenceDirectory
)

$ErrorActionPreference = 'Stop'
$repository = (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot '..')).Path
. (Join-Path $PSScriptRoot 'ConPty.ps1')
$targetPrefix = (Resolve-Path -LiteralPath (Join-Path $repository 'target')).Path.TrimEnd('\') + '\'
$fixture = (Resolve-Path -LiteralPath $FixtureDirectory).Path
if (-not $fixture.StartsWith($targetPrefix, [StringComparison]::OrdinalIgnoreCase)) { throw 'Fixture must be under repository target.' }
if (-not (Test-Path -LiteralPath (Join-Path $fixture 'vault.capstan') -PathType Leaf)) { throw 'Create the encrypted installer fixture first.' }
$base = (Resolve-Path -LiteralPath $BaseInstaller).Path
$upgrade = (Resolve-Path -LiteralPath $UpgradeInstaller).Path
if ((Get-FileHash -LiteralPath $base).Hash -eq (Get-FileHash -LiteralPath $upgrade).Hash) { throw 'Upgrade requires two different installer builds.' }
$principal = [Security.Principal.WindowsPrincipal]::new([Security.Principal.WindowsIdentity]::GetCurrent())
if ($principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) { throw 'Run installer acceptance as an ordinary user.' }
$uninstallRoot = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall'
$existing = @(Get-ChildItem $uninstallRoot | Get-ItemProperty | Where-Object { $_.DisplayName -eq 'Capstan' })
if ($existing.Count -ne 0) { throw 'An existing Capstan installation must not be changed by this test.' }
if (Get-Process -Name 'Capstan','capstan-app' -ErrorAction SilentlyContinue) { throw 'Close Capstan before installer acceptance.' }
$data = Join-Path $env:APPDATA 'Capstan'
if (Test-Path -LiteralPath $data) { throw 'Existing roaming data must not be changed by this test.' }
$integrationKeys = @(
    'Software\Classes\Directory\shell\Capstan',
    'Software\Classes\Directory\Background\shell\Capstan',
    'Software\Capstan\Capabilities',
    'Software\Classes\Capstan.ssh'
)
foreach ($key in $integrationKeys) {
    if (Test-Path -LiteralPath "HKCU:\$key") { throw 'Existing Capstan integration must not be changed by this test.' }
}
$registeredApplications = [Microsoft.Win32.Registry]::CurrentUser.OpenSubKey('Software\RegisteredApplications')
try {
    if ($null -ne $registeredApplications -and $null -ne $registeredApplications.GetValue('Capstan', $null)) {
        throw 'Existing registered application must not be changed by this test.'
    }
} finally { if ($null -ne $registeredApplications) { $registeredApplications.Dispose() } }

function Get-ProtocolSnapshot {
    $snapshot = @()
    foreach ($key in @('Software\Classes\ssh', 'Software\Microsoft\Windows\Shell\Associations\UrlAssociations\ssh\UserChoice')) {
        $root = "HKCU:\$key"
        if (-not (Test-Path -LiteralPath $root)) { $snapshot += [ordered]@{ path = $key; absent = $true }; continue }
        $paths = @($root) + @(Get-ChildItem -LiteralPath $root -Recurse | Sort-Object PSPath | ForEach-Object { $_.PSPath })
        foreach ($path in $paths) {
            $item = Get-Item -LiteralPath $path
            try {
                foreach ($name in ($item.GetValueNames() | Sort-Object)) {
                    $snapshot += [ordered]@{ path = $item.Name; name = $name; value = $item.GetValue($name); kind = [string]$item.GetValueKind($name) }
                }
            } finally { $item.Dispose() }
        }
    }
    return ($snapshot | ConvertTo-Json -Depth 8 -Compress)
}
$protocolBefore = Get-ProtocolSnapshot
$token = [Guid]::NewGuid().ToString('N')
$install = Join-Path $repository "target/m1-installed-$token"
if (Test-Path -LiteralPath $install) { throw 'Installer test directory already exists.' }
New-Item -ItemType Directory -Path $EvidenceDirectory -Force | Out-Null
New-Item -ItemType Directory -Path $data | Out-Null
$owner = Join-Path $data 'capstan-install-smoke.owner'
[IO.File]::WriteAllText($owner, $token)
$inventory = @()
foreach ($file in Get-ChildItem -LiteralPath $fixture -File) {
    Copy-Item -LiteralPath $file.FullName -Destination (Join-Path $data $file.Name)
    $inventory += @{ name = $file.Name; sha256 = (Get-FileHash -LiteralPath $file.FullName).Hash }
}
$results = [Collections.Generic.List[object]]::new()
$failure = $null
$versions = @()

function Assert-DataPreserved([string]$Scenario) {
    foreach ($file in $inventory) {
        if ((Get-FileHash -LiteralPath (Join-Path $data $file.name)).Hash -ne $file.sha256) { throw "Encrypted data changed: $Scenario" }
    }
    $results.Add(@{ scenario = $Scenario; passed = $true; preserved_files = $inventory.Count })
}

function Assert-SystemIntegration([string]$Executable, [bool]$Installed) {
    if ((Get-ProtocolSnapshot) -cne $protocolBefore) { throw 'Existing SSH protocol handler changed.' }
    if ($Installed) {
        $localCommand = '"' + $Executable + '" --local --cwd "%V/"'
        foreach ($key in $integrationKeys[0..1]) {
            $command = [Microsoft.Win32.Registry]::CurrentUser.OpenSubKey("$key\command")
            try {
                if ($null -eq $command -or $command.GetValue('') -cne $localCommand) { throw 'Explorer launch command is missing or incorrectly quoted.' }
            } finally { if ($null -ne $command) { $command.Dispose() } }
        }
        $capabilities = [Microsoft.Win32.Registry]::CurrentUser.OpenSubKey('Software\Capstan\Capabilities\URLAssociations')
        try {
            if ($null -eq $capabilities -or $capabilities.GetValue('ssh') -cne 'Capstan.ssh') { throw 'SSH capability is missing.' }
        } finally { if ($null -ne $capabilities) { $capabilities.Dispose() } }
        $protocolCommand = [Microsoft.Win32.Registry]::CurrentUser.OpenSubKey('Software\Classes\Capstan.ssh\shell\open\command')
        try {
            if ($null -eq $protocolCommand -or $protocolCommand.GetValue('') -cne ('"' + $Executable + '" "%1"')) { throw 'SSH launch command is incorrectly quoted.' }
        } finally { if ($null -ne $protocolCommand) { $protocolCommand.Dispose() } }
        $registration = [Microsoft.Win32.Registry]::CurrentUser.OpenSubKey('Software\RegisteredApplications')
        try {
            if ($null -eq $registration -or $registration.GetValue('Capstan') -cne 'Software\Capstan\Capabilities') { throw 'Optional SSH application registration is missing.' }
        } finally { if ($null -ne $registration) { $registration.Dispose() } }
        $shortcut = Join-Path $env:APPDATA 'Microsoft\Windows\Start Menu\Programs\Capstan.lnk'
        $shell = New-Object -ComObject Shell.Application
        try {
            $item = $shell.NameSpace((Split-Path -Parent $shortcut)).ParseName((Split-Path -Leaf $shortcut))
            if ($null -eq $item -or $item.ExtendedProperty('System.AppUserModel.ID') -cne 'dev.capstan.app') { throw 'Installed shortcut AUMID does not match the application.' }
        } finally { [Runtime.InteropServices.Marshal]::ReleaseComObject($shell) | Out-Null }
    } else {
        foreach ($key in $integrationKeys) {
            if (Test-Path -LiteralPath "HKCU:\$key") { throw 'Uninstall left a Capstan integration key behind.' }
        }
        $registration = [Microsoft.Win32.Registry]::CurrentUser.OpenSubKey('Software\RegisteredApplications')
        try {
            if ($null -ne $registration -and $null -ne $registration.GetValue('Capstan', $null)) { throw 'Uninstall left registered application capabilities behind.' }
        } finally { if ($null -ne $registration) { $registration.Dispose() } }
    }
    $results.Add(@{ scenario = 'system-integration'; installed = $Installed; passed = $true; existing_ssh_handler_preserved = $true })
}

function Invoke-Installer([string]$Path, [bool]$RequireIntegration) {
    $process = Start-Process -FilePath $Path -ArgumentList @('/S', "/D=$install") -WindowStyle Hidden -PassThru
    if (-not $process.WaitForExit(60000)) { throw 'Installer did not finish within 60 seconds.' }
    if ($process.ExitCode -ne 0) { throw "Installer exited $($process.ExitCode)" }
    $entries = @(Get-ChildItem $uninstallRoot | Get-ItemProperty | Where-Object { $_.DisplayName -eq 'Capstan' })
    if ($entries.Count -ne 1) { throw 'Expected one per-user installation entry.' }
    $script:versions += $entries[0].DisplayVersion
    Assert-ConPtyRuntime (Join-Path $install 'runtime/x86_64') | Out-Null
    foreach ($name in @('LICENSE','THIRD_PARTY_NOTICES.md','PORTABLE-README.md')) {
        if (-not (Test-Path -LiteralPath (Join-Path $install $name) -PathType Leaf)) { throw 'Installed legal/data-location documentation is missing.' }
    }
    $installed = @(Get-ChildItem -LiteralPath $install -Filter '*.exe' -File | Where-Object { $_.Name -notmatch 'uninstall' })
    if ($installed.Count -ne 1) { throw 'Expected one installed application executable.' }
    if ($RequireIntegration -or (Test-Path -LiteralPath "HKCU:\$($integrationKeys[0])")) {
        Assert-SystemIntegration $installed[0].FullName $true
    } elseif ((Get-ProtocolSnapshot) -cne $protocolBefore) { throw 'Legacy installation changed the existing SSH handler.' }
    $results.Add(@{
        scenario='installed-verified-runtime'; passed=$true; version=$entries[0].DisplayVersion
        executable_sha256=(Get-FileHash -LiteralPath $installed[0].FullName).Hash
        installed_file_bytes=(Get-ChildItem -LiteralPath $install -File -Recurse | Measure-Object Length -Sum).Sum
    })
    $application = Start-Process -FilePath $installed[0].FullName -WorkingDirectory $install -WindowStyle Hidden -PassThru
    try {
        Start-Sleep -Seconds 4
        $application.Refresh()
        if ($application.HasExited) { throw 'Installed application exited during startup.' }
        if ($application.MainWindowTitle -ne 'Capstan') { throw 'Installed application did not create its main window.' }
    } finally {
        if (-not $application.HasExited) {
            $application.CloseMainWindow() | Out-Null
            if (-not $application.WaitForExit(10000)) { throw 'Installed application did not close normally.' }
        }
        $application.Dispose()
    }
}

try {
    Invoke-Installer $base $false
    Assert-DataPreserved 'ordinary-user-install'
    Invoke-Installer $upgrade $true
    if ($versions.Count -ne 2 -or [version]$versions[1] -le [version]$versions[0]) { throw 'Installed version did not increase.' }
    Assert-DataPreserved 'upgrade'
    $uninstaller = @(Get-ChildItem -LiteralPath $install -Filter '*uninstall*.exe' -File)
    if ($uninstaller.Count -ne 1) { throw 'Expected one installed uninstaller.' }
    $process = Start-Process -FilePath $uninstaller[0].FullName -ArgumentList '/S' -WindowStyle Hidden -PassThru
    if (-not $process.WaitForExit(60000) -or $process.ExitCode -ne 0) { throw 'Uninstaller failed.' }
    $uninstallWait = [Diagnostics.Stopwatch]::StartNew()
    do {
        $remaining = @(Get-ChildItem $uninstallRoot | Get-ItemProperty | Where-Object { $_.DisplayName -eq 'Capstan' })
        if ($remaining.Count -eq 0) { break }
        Start-Sleep -Milliseconds 200
    } while ($uninstallWait.Elapsed.TotalSeconds -lt 20)
    if ($remaining.Count -ne 0) { throw 'Uninstall registration remained.' }
    if (Test-Path -LiteralPath (Join-Path $install 'runtime/x86_64/conpty.dll')) { throw 'Uninstall left bundled runtime binaries behind.' }
    Assert-SystemIntegration '' $false
    Assert-DataPreserved 'uninstall-retains-data'
} catch { $failure = $_.Exception.Message }

$cleaned = $false
if ($null -eq $failure) {
    if ([IO.File]::ReadAllText($owner) -ne $token) { throw 'Test data ownership changed.' }
    $ownedNames = @($inventory | ForEach-Object { $_.name }) + 'capstan-install-smoke.owner'
    $current = @(Get-ChildItem -LiteralPath $data -Force)
    if (@($current | Where-Object { $_.PSIsContainer -or $_.Name -notin $ownedNames }).Count -ne 0) { throw 'Unexpected data exists; preserve it for inspection.' }
    foreach ($file in $current) { Remove-Item -LiteralPath $file.FullName -Force }
    Remove-Item -LiteralPath $data
    $cleaned = $true
}
$report = @{
    date = (Get-Date).ToString('o'); ordinary_user = $true; versions = $versions
    scenarios = $results.ToArray(); passed = ($null -eq $failure); failure = $failure
    test_roaming_data_cleaned = $cleaned; install_directory = $install
    base_sha256 = (Get-FileHash -LiteralPath $base).Hash; upgrade_sha256 = (Get-FileHash -LiteralPath $upgrade).Hash
    limitation = 'Existing Windows user with no prior Capstan install/data, not a clean OS/new-user VM. Synthetic encrypted data; no credential-entry or cross-machine acceptance.'
}
$report | ConvertTo-Json -Depth 6 | Set-Content -LiteralPath (Join-Path $EvidenceDirectory 'report.json') -Encoding UTF8
Write-Output "Evidence directory: $EvidenceDirectory"
if ($failure) { throw $failure }
