param(
    [ValidateSet('Start', 'Stop')][string]$Action = 'Start',
    [string]$Workspace,
    [string]$Distribution = 'Ubuntu',
    [ValidateRange(1024, 65535)][int]$Port = 22312,
    [switch]$Smoke,
    [switch]$NoLaunch
)
$ErrorActionPreference = 'Stop'
$repository = (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot '..')).Path
$target = (Resolve-Path -LiteralPath (Join-Path $repository 'target')).Path
$targetPrefix = $target.TrimEnd('\') + '\'
function Check-Workspace([string]$Path) {
    $resolved = (Resolve-Path -LiteralPath $Path).Path
    if (-not $resolved.StartsWith($targetPrefix, [StringComparison]::OrdinalIgnoreCase) -or
        [IO.Path]::GetDirectoryName($resolved) -ne $target -or
        [IO.Path]::GetFileName($resolved) -notmatch '^m1-sshd-[0-9a-f]{12}$' -or
        ((Get-Item -LiteralPath $resolved -Force).Attributes -band [IO.FileAttributes]::ReparsePoint)) {
        throw 'Manual workspace must be a real isolated child of repository target.'
    }
    return $resolved
}
function Wsl-Path([string]$Path) {
    $result = & wsl.exe -d $Distribution -- wslpath -a -u $Path.Replace('\', '/')
    if ($LASTEXITCODE -ne 0) { throw 'Could not translate manual fixture path.' }
    return $result.Trim()
}
function Fixture-Action([string]$Operation) {
    & wsl.exe -d $Distribution -u root -- sh $script:fixtureScript $Operation $script:remoteRoot $script:secretWsl $Port $script:account none ed25519 sftp
    if ($LASTEXITCODE -ne 0) { throw "Manual fixture operation failed: $Operation" }
}
function Remove-Secrets {
    $secretRoot = Join-Path $Workspace 'secrets'
    if (Test-Path -LiteralPath $secretRoot) {
        $resolved = (Resolve-Path -LiteralPath $secretRoot).Path
        if ($resolved -ne (Join-Path (Check-Workspace $Workspace) 'secrets') -or
            ((Get-Item -LiteralPath $resolved -Force).Attributes -band [IO.FileAttributes]::ReparsePoint)) {
            throw 'Credential cleanup scope is invalid.'
        }
        Remove-Item -LiteralPath $resolved -Recurse -Force
    }
}
if ($Action -eq 'Stop') {
    if ([string]::IsNullOrWhiteSpace($Workspace)) {
        $current = Get-Content -LiteralPath (Join-Path $target 'm2-manual-current.json') -Raw | ConvertFrom-Json
        if ($current.purpose -ne 'M2-A-manual-review') { throw 'Invalid current manual fixture pointer.' }
        $Workspace = $current.workspace
    }
    $Workspace = Check-Workspace $Workspace
    $reviewExecutable = Join-Path $Workspace 'desktop/Capstan.exe'
    foreach ($running in @(Get-Process -Name Capstan -ErrorAction SilentlyContinue)) {
        if ($running.Path -eq $reviewExecutable) { throw 'Close this isolated review app before stopping its SSH fixture.' }
    }
    $statePath = Join-Path $Workspace 'manual-state.json'
    $state = Get-Content -LiteralPath $statePath -Raw | ConvertFrom-Json
    $suffix = [IO.Path]::GetFileName($Workspace).Substring(8)
    if ($state.schemaVersion -ne 1 -or $state.token -ne $suffix -or $state.purpose -ne 'M2-A-manual-review') { throw 'Manual ownership record is invalid.' }
    $Distribution = $state.distribution
    $Port = [int]$state.port
    $script:remoteRoot = "/var/tmp/capstan-m1-ssh-$suffix"
    $script:account = "cm1$suffix"
    $script:fixtureScript = Wsl-Path (Join-Path $PSScriptRoot 'm1-sshd-fixture.sh')
    $script:secretWsl = Wsl-Path (Join-Path $Workspace 'secrets')
    Fixture-Action 'stop'
    Remove-Secrets
    $state.active = $false
    $state | ConvertTo-Json -Depth 5 | Set-Content -LiteralPath $statePath -Encoding utf8
    Write-Output 'Fixture stopped and generated plaintext credentials removed. Desktop data and test files were retained.'
    exit 0
}
if (-not [string]::IsNullOrWhiteSpace($Workspace)) { throw 'Start generates its own isolated workspace.' }
$reviewRecord = Get-Content -LiteralPath (Join-Path $target 'm2-review-current.json') -Raw | ConvertFrom-Json
$review = [IO.Path]::GetFullPath($reviewRecord.directory)
if ([IO.Path]::GetDirectoryName($review) -ne $target -or [IO.Path]::GetFileName($review) -notmatch '^m2-review-[0-9a-f]{12}$' -or
    (Get-FileHash -LiteralPath (Join-Path $review 'manifest.json')).Hash -ne $reviewRecord.manifestSha256) { throw 'Review build pointer is invalid.' }
$reviewManifest = Get-Content -LiteralPath (Join-Path $review 'manifest.json') -Raw | ConvertFrom-Json
foreach ($file in $reviewManifest.files) {
    $path = [IO.Path]::GetFullPath((Join-Path $review $file.path))
    if (-not $path.StartsWith($review.TrimEnd('\') + '\', [StringComparison]::OrdinalIgnoreCase) -or
        -not (Test-Path -LiteralPath $path -PathType Leaf) -or (Get-FileHash -LiteralPath $path).Hash -ne $file.sha256) {
        throw 'Review build file identity does not match its manifest.'
    }
}
$desktopSource = Join-Path $review 'desktop'
$helper = Join-Path $review 'tools/m2_manual_fixture.exe'
if (-not (Test-Path -LiteralPath (Join-Path $desktopSource 'Capstan.exe')) -or -not (Test-Path -LiteralPath $helper)) { throw 'Run Build-M2Review.ps1 first.' }
$token = [Guid]::NewGuid().ToString('N').Substring(0, 12)
$Workspace = Join-Path $target "m1-sshd-$token"
New-Item -ItemType Directory -Path $Workspace | Out-Null
$Workspace = Check-Workspace $Workspace
$secrets = Join-Path $Workspace 'secrets'
New-Item -ItemType Directory -Path $secrets | Out-Null
$acl = [Security.AccessControl.DirectorySecurity]::new()
$acl.SetAccessRuleProtection($true, $false)
$sid = [Security.Principal.WindowsIdentity]::GetCurrent().User
$acl.AddAccessRule([Security.AccessControl.FileSystemAccessRule]::new($sid, 'FullControl', 'ContainerInherit,ObjectInherit', 'None', 'Allow'))
Set-Acl -LiteralPath $secrets -AclObject $acl
$script:remoteRoot = "/var/tmp/capstan-m1-ssh-$token"
$script:account = "cm1$token"
$script:fixtureScript = Wsl-Path (Join-Path $PSScriptRoot 'm1-sshd-fixture.sh')
$script:secretWsl = Wsl-Path $secrets
$started = $false
$complete = $false
$masterFile = Join-Path $secrets 'master.txt'
try {
    $listener = [Net.Sockets.TcpListener]::new([Net.IPAddress]::Loopback, $Port)
    try { $listener.Start() } finally { $listener.Stop() }
    $started = $true
    Fixture-Action 'start'
    Copy-Item -LiteralPath $desktopSource -Destination (Join-Path $Workspace 'desktop') -Recurse
    $local = Join-Path $Workspace 'files'
    New-Item -ItemType Directory -Path $local | Out-Null
    New-Item -ItemType Directory -Path (Join-Path $Workspace 'downloads') | Out-Null
    [IO.File]::WriteAllBytes((Join-Path $local 'empty.txt'), [byte[]]::new(0))
    [IO.File]::WriteAllText((Join-Path $local 'sample.txt'), "Capstan isolated M2 fixture`n", [Text.UTF8Encoding]::new($false))
    $unicodeFile = 'name-' + [char]0x4e2d + [char]0x6587 + ' space.txt'
    [IO.File]::WriteAllText((Join-Path $local $unicodeFile), "Unicode name fixture`n", [Text.UTF8Encoding]::new($false))
    $payload = [byte[]]::new(1024 * 1024)
    $random = [Random]::new(20261018)
    $stream = [IO.File]::Open((Join-Path $local 'binary.bin'), [IO.FileMode]::CreateNew, [IO.FileAccess]::Write, [IO.FileShare]::None)
    try {
        for ($block = 0; $block -lt 128; $block++) { $random.NextBytes($payload); $stream.Write($payload, 0, $payload.Length) }
    } finally { $stream.Dispose() }
    $manifest = foreach ($file in Get-ChildItem -LiteralPath $local -File) {
        [ordered]@{ name = $file.Name; bytes = $file.Length; sha256 = (Get-FileHash -LiteralPath $file.FullName -Algorithm SHA256).Hash }
    }
    $manifest | ConvertTo-Json -Depth 4 | Set-Content -LiteralPath (Join-Path $Workspace 'file-manifest.json') -Encoding utf8
    if ($Smoke) { $master = ConvertTo-SecureString ([Guid]::NewGuid().ToString('N')) -AsPlainText -Force }
    else { $master = Read-Host 'Choose a disposable Vault password (use the same password to unlock the review app)' -AsSecureString }
    $pointer = [Runtime.InteropServices.Marshal]::SecureStringToBSTR($master)
    $characters = [char[]]::new($master.Length)
    $encoded = $null
    try {
        for ($index = 0; $index -lt $characters.Length; $index++) { $characters[$index] = [char][Runtime.InteropServices.Marshal]::ReadInt16($pointer, $index * 2) }
        $encoded = [Text.Encoding]::UTF8.GetBytes($characters)
        [IO.File]::WriteAllBytes($masterFile, $encoded)
    } finally {
        [Array]::Clear($characters, 0, $characters.Length)
        if ($encoded) { [Array]::Clear($encoded, 0, $encoded.Length) }
        [Runtime.InteropServices.Marshal]::ZeroFreeBSTR($pointer)
        $master.Dispose()
    }
    & $helper $Workspace $Port
    if ($LASTEXITCODE -ne 0) { throw 'Native-side manual Vault preparation failed.' }
    Remove-Item -LiteralPath $masterFile -Force
    [ordered]@{
        schemaVersion = 1; token = $token; purpose = 'M2-A-manual-review'; distribution = $Distribution; port = $Port; active = $true
        executableSha256 = (Get-FileHash -LiteralPath (Join-Path $Workspace 'desktop/Capstan.exe')).Hash
        desktopAcceptance = $false; performanceAcceptance = $false
    } | ConvertTo-Json -Depth 5 | Set-Content -LiteralPath (Join-Path $Workspace 'manual-state.json') -Encoding utf8
    [ordered]@{ purpose = 'M2-A-manual-review'; workspace = $Workspace } | ConvertTo-Json |
        Set-Content -LiteralPath (Join-Path $target 'm2-manual-current.json') -Encoding utf8
    $complete = $true
    Write-Output "Workspace: $Workspace"
    Write-Output "Local test files: $local"
    Write-Output 'Unlock with the disposable password you chose; use the two preconfigured SFTP manual hosts.'
    Write-Output "After closing the app: powershell -NoProfile -File scripts/Prepare-M2Manual.ps1 -Action Stop -Workspace '$Workspace'"
    if (-not $Smoke -and -not $NoLaunch) {
        # A visible window is intentional: this is the maintainer's manual app.
        $start = [Diagnostics.ProcessStartInfo]::new()
        $start.FileName = Join-Path $Workspace 'desktop/Capstan.exe'
        $start.WorkingDirectory = Join-Path $Workspace 'desktop'
        $start.UseShellExecute = $false
        [Diagnostics.Process]::Start($start) | Out-Null
    }
} finally {
    if (Test-Path -LiteralPath $masterFile) { Remove-Item -LiteralPath $masterFile -Force }
    if (-not $complete -and $started) { Fixture-Action 'stop'; Remove-Secrets }
    if ($Smoke -and $complete) {
        Fixture-Action 'stop'
        Remove-Secrets
        $statePath = Join-Path $Workspace 'manual-state.json'
        $state = Get-Content -LiteralPath $statePath -Raw | ConvertFrom-Json
        $state.active = $false
        $state | ConvertTo-Json -Depth 5 | Set-Content -LiteralPath $statePath -Encoding utf8
    }
}
