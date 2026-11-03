param(
    [Parameter(Mandatory = $true)][string]$Executable,
    [Parameter(Mandatory = $true)][string]$OutputDirectory,
    [string]$Distribution = 'Ubuntu',
    [string]$PreviousFrozenPolicy,
    [ValidateRange(1024,65535)][int]$Port = 22317,
    [ValidateSet('wsl_localhost','windows_private')][string]$NetworkRelay = 'wsl_localhost'
)
$ErrorActionPreference = 'Stop'
$repository = (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot '..')).Path
. (Join-Path $PSScriptRoot 'M2NetworkRelay.ps1')
$executablePath = (Resolve-Path -LiteralPath $Executable).Path
$capability = & $executablePath --m0-check
if ($LASTEXITCODE -ne 0 -or $capability -ne 'capstan-m0-benchmarks-v1') { throw 'Use the opt-in measurement build.' }
$applicationDirectory = Split-Path $executablePath -Parent
if (-not (Test-Path -LiteralPath (Join-Path $applicationDirectory 'capstan.portable')) -or [IO.File]::ReadAllText((Join-Path $applicationDirectory 'capstan.portable')) -cne 'CAPSTAN_PORTABLE_V1' -or (Test-Path -LiteralPath (Join-Path $applicationDirectory 'data/vault.capstan'))) {
    throw 'Use a dedicated portable measurement directory without a user Vault.'
}
$output = [IO.Path]::GetFullPath($OutputDirectory)
if (Test-Path -LiteralPath $output) { throw 'Use a new evidence directory.' }
New-Item -ItemType Directory -Path $output | Out-Null
$policySource = Join-Path $repository 'docs/agent/baselines/M1/performance-policy.json'
Copy-Item -LiteralPath $policySource -Destination (Join-Path $output 'premeasurement-policy.json')
$policyHash = (Get-FileHash -LiteralPath $policySource).Hash
$token = [Guid]::NewGuid().ToString('N').Substring(0,12)
$workspace = Join-Path $repository "target/m1-sshd-$token"
$secrets = Join-Path $workspace 'secrets'
$fixture = "/var/tmp/capstan-m1-ssh-$token"
$account = "cm1$token"
New-Item -ItemType Directory -Path $secrets | Out-Null
$acl = [Security.AccessControl.DirectorySecurity]::new()
$acl.SetAccessRuleProtection($true,$false)
$sid = [Security.Principal.WindowsIdentity]::GetCurrent().User
$acl.AddAccessRule([Security.AccessControl.FileSystemAccessRule]::new($sid,'FullControl','ContainerInherit,ObjectInherit','None','Allow'))
Set-Acl -LiteralPath $secrets -AclObject $acl
function WslPath([string]$Path) {
    $value = & wsl.exe -d $Distribution -- wslpath -a -u $Path.Replace('\','/')
    if ($LASTEXITCODE -ne 0) { throw 'WSL path conversion failed.' }
    return $value.Trim()
}
$scriptPath = WslPath (Join-Path $PSScriptRoot 'm1-sshd-fixture.sh')
$secretPath = WslPath $secrets
$started = $false
$cleaned = $false
$windowsRelay = $null
$networkStarted = $false
$networkCleaned = $true
$relayCleaned = $true
$failure = $null
$previous = $env:CAPSTAN_M1_FIXTURE_CONFIG
$timer = [Diagnostics.Stopwatch]::StartNew()
try {
    $probe = [Net.Sockets.TcpClient]::new()
    try {
        $connection = $probe.ConnectAsync('127.0.0.1',$Port)
        if ($connection.Wait(1000) -and $probe.Connected) { throw 'Fixture port already in use.' }
    } catch [AggregateException] { } finally { $probe.Dispose() }
    if ($NetworkRelay -eq 'windows_private') {
        $relayAddress = Get-M2WslPrivateAddress $Distribution
        $relayCleaned = $false
        $windowsRelay = Start-M2WindowsRelay $relayAddress $Port $token $workspace $output
    }
    $started = $true
    & wsl.exe -d $Distribution -u root -- sh $scriptPath start $fixture $secretPath $Port $account none ed25519 benchmark
    if ($LASTEXITCODE -ne 0) { throw 'OpenSSH fixture startup failed.' }
    if ($windowsRelay) {
        $networkScript = WslPath (Join-Path $PSScriptRoot 'm2-net-fixture.sh')
        $networkStarted = $true; $networkCleaned = $false
        & wsl.exe -d $Distribution -u root -- sh $networkScript start $fixture 0 $relayAddress *> (Join-Path $output 'network-start.txt')
        if ($LASTEXITCODE -ne 0) { throw 'Explicit zero-delay M1 fixture startup failed.' }
    }
    Copy-Item -LiteralPath (Join-Path $secrets 'server-version.txt') -Destination (Join-Path $output 'server-version.txt')
    Copy-Item -LiteralPath (Join-Path $secrets 'server-environment.txt') -Destination (Join-Path $output 'server-environment.txt')
    $rtt = @()
    for ($index=0; $index -lt 10; $index++) {
        $tcp = [Net.Sockets.TcpClient]::new()
        $clock = [Diagnostics.Stopwatch]::StartNew()
        try { $tcp.Connect('127.0.0.1',$Port); $rtt += $clock.Elapsed.TotalMilliseconds } finally { $tcp.Dispose() }
    }
    [IO.File]::WriteAllText((Join-Path $output 'network.json'), (@{ transport='Windows loopback to WSL'; relay=$NetworkRelay; injectedOneWayDelayMs=0; rtt_method='TCP connect latency (not ICMP RTT)'; samples_ms=$rtt; compression='none'; server='disposable OpenSSH, password and Ed25519 identities' } | ConvertTo-Json -Depth 4),[Text.UTF8Encoding]::new($false))
    Remove-Item Env:CAPSTAN_M1_FIXTURE_CONFIG -ErrorAction SilentlyContinue
    & (Join-Path $PSScriptRoot 'Measure-M0.ps1') -Executable $executablePath -OutputDirectory (Join-Path $output 'local-reference') -Visible
    & (Join-Path $PSScriptRoot 'Freeze-M1Performance.ps1') -BaselineDirectory (Join-Path $output 'local-reference') -OutputPath (Join-Path $output 'frozen-limits.json') -PreviousFrozenPolicy $PreviousFrozenPolicy
    $config = @{
        port=$Port; username=$account; password_file=(Join-Path $secrets 'password.txt'); key_file=(Join-Path $secrets 'openssh')
        host_public_key_file=(Join-Path $secrets 'host-key.pub'); vault_file=(Join-Path $secrets 'measurement.capstan'); remote_fixture="$fixture/home/benchmark.py"
    }
    $configPath = Join-Path $secrets 'measurement.json'
    [IO.File]::WriteAllText($configPath, ($config | ConvertTo-Json), [Text.UTF8Encoding]::new($false))
    $env:CAPSTAN_M1_FIXTURE_CONFIG = $configPath
    & (Join-Path $PSScriptRoot 'Measure-M0.ps1') -Executable $executablePath -OutputDirectory (Join-Path $output 'mixed') -Visible
    if ((Get-FileHash -LiteralPath $policySource).Hash -ne $policyHash) { throw 'Premeasurement policy changed during the run.' }
    & (Join-Path $PSScriptRoot 'Verify-M1Performance.ps1') -BaselineDirectory (Join-Path $output 'local-reference') -MixedDirectory (Join-Path $output 'mixed') -FrozenPolicy (Join-Path $output 'frozen-limits.json')
} catch { $failure = $_.Exception.Message }
finally {
    if ($null -eq $previous) { Remove-Item Env:CAPSTAN_M1_FIXTURE_CONFIG -ErrorAction SilentlyContinue } else { $env:CAPSTAN_M1_FIXTURE_CONFIG=$previous }
    if ($windowsRelay) {
        try { Stop-M2WindowsRelay $windowsRelay; $relayCleaned = $true } catch { $failure = 'Windows relay cleanup failed; prior failure: ' + $failure }
    }
    if ($networkStarted) {
        try {
            & wsl.exe -d $Distribution -u root -- sh $networkScript stop $fixture 0 > (Join-Path $output 'network-stop.txt')
            if ($LASTEXITCODE -ne 0) { throw 'Owned namespace cleanup failed.' }
            $networkCleaned = $true
        } catch { $failure = 'Owned network cleanup failed; prior failure: ' + $failure }
    }
    if ($started) {
        try {
            & wsl.exe -d $Distribution -u root -- sh $scriptPath stop $fixture $secretPath $Port $account
            if ($LASTEXITCODE -ne 0) { throw 'Fixture cleanup failed.' }
            $cleaned = $true
        } catch { $failure = 'OpenSSH fixture cleanup failed; prior failure: ' + $failure }
    }
    if (Test-Path -LiteralPath $workspace) {
        $resolved = (Resolve-Path -LiteralPath $workspace).Path
        $prefix = (Resolve-Path -LiteralPath (Join-Path $repository 'target')).Path.TrimEnd('\')+'\'
        if (!$resolved.StartsWith($prefix,[StringComparison]::OrdinalIgnoreCase)) { throw 'Invalid private fixture cleanup path.' }
        Remove-Item -LiteralPath $resolved -Recurse -Force
    }
    [IO.File]::WriteAllText((Join-Path $output 'summary.json'), (@{ schema=1; passed=($null -eq $failure -and $networkCleaned -and $relayCleaned); fixture_cleaned=$cleaned; network_cleaned=$networkCleaned; windows_relay_cleaned=$relayCleaned; network_relay=$NetworkRelay; failure=$failure; elapsed_ms=$timer.ElapsedMilliseconds; policy_sha256=$policyHash; executable_sha256=(Get-FileHash -LiteralPath $executablePath).Hash } | ConvertTo-Json),[Text.UTF8Encoding]::new($false))
}
Write-Output "Evidence directory: $output"
if ($failure) { throw $failure }
