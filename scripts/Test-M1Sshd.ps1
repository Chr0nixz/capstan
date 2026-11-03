param(
    [string]$Distribution = 'Ubuntu',
    [ValidateRange(1024, 65535)][int]$Port = 22292,
    [string]$EvidenceDirectory,
    [string]$PageantExecutable,
    [ValidateSet('ed25519', 'rsa')][string]$HostKeyAlgorithm = 'ed25519'
)

$ErrorActionPreference = 'Stop'
$repository = (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot '..')).Path
$token = [Guid]::NewGuid().ToString('N').Substring(0, 12)
$workspace = Join-Path $repository "target/m1-sshd-$token"
$secrets = Join-Path $workspace 'secrets'
$fixture = "/var/tmp/capstan-m1-ssh-$token"
$account = "cm1$token"
$fixtureScript = Join-Path $PSScriptRoot 'm1-sshd-fixture.sh'
$variables = @(
    'CAPSTAN_TEST_SSHD', 'CAPSTAN_TEST_SSH_PASSWORD_FILE',
    'CAPSTAN_TEST_SSH_KEY_FILE', 'CAPSTAN_TEST_SSH_KEY_PASSPHRASE_FILE',
    'CAPSTAN_TEST_SSH_AGENT', 'CAPSTAN_TEST_SSH_AGENT_FINGERPRINT',
    'CAPSTAN_TEST_SSH_AGENT_FORWARDING_PUBLIC_KEY',
    'CAPSTAN_TEST_SSH_TRUST_FILE', 'CAPSTAN_TEST_SSH_EXPECT_CHANGED'
)
$previous = @{}
foreach ($variable in $variables) {
    $previous[$variable] = [Environment]::GetEnvironmentVariable($variable, 'Process')
}

function Get-WslPath([string]$Path) {
    $translated = & wsl.exe -d $Distribution -- wslpath -a -u $Path.Replace('\', '/')
    if ($LASTEXITCODE -ne 0) { throw 'Could not translate the fixture path for WSL.' }
    return $translated.Trim()
}

function Invoke-Fixture([string]$Action) {
    $pageantOption = if ($PageantExecutable) { 'pageant' } else { 'none' }
    & wsl.exe -d $Distribution -u root -- sh $scriptPath $Action $fixture $secretPath $Port $account $pageantOption $HostKeyAlgorithm
    if ($LASTEXITCODE -ne 0) { throw "OpenSSH fixture action failed: $Action" }
}

function Clear-TestEnvironment {
    foreach ($variable in $variables) { [Environment]::SetEnvironmentVariable($variable, $null, 'Process') }
    $env:CAPSTAN_TEST_SSHD = "$account@127.0.0.1:$Port"
}

$started = $false
$cleanupSucceeded = $false
$results = [System.Collections.Generic.List[object]]::new()
$failure = $null
$pageantProcess = $null
$pageantVersion = $null
$version = $null
if ([string]::IsNullOrWhiteSpace($EvidenceDirectory)) {
    $stamp = Get-Date -Format 'yyyy-MM-dd-HHmmss'
    $EvidenceDirectory = Join-Path $repository "docs/agent/baselines/M1/raw/$stamp-sshd-$token"
}
$evidence = [System.IO.Path]::GetFullPath($EvidenceDirectory)
New-Item -ItemType Directory -Path $secrets -Force | Out-Null
New-Item -ItemType Directory -Path $evidence -Force | Out-Null
# Restrict ephemeral credential files before WSL creates them on the Windows volume.
$acl = [System.Security.AccessControl.DirectorySecurity]::new()
$acl.SetAccessRuleProtection($true, $false)
$sid = [System.Security.Principal.WindowsIdentity]::GetCurrent().User
$rule = [System.Security.AccessControl.FileSystemAccessRule]::new(
    $sid, 'FullControl', 'ContainerInherit,ObjectInherit', 'None', 'Allow'
)
$acl.AddAccessRule($rule)
Set-Acl -LiteralPath $secrets -AclObject $acl

Push-Location $repository
try {
    $scriptPath = Get-WslPath $fixtureScript
    $secretPath = Get-WslPath $secrets
    if ($PageantExecutable) {
        $PageantExecutable = (Resolve-Path -LiteralPath $PageantExecutable).Path
        $signature = Get-AuthenticodeSignature -LiteralPath $PageantExecutable
        if ($signature.Status -ne 'Valid' -or $signature.SignerCertificate.Subject -notmatch '^CN=Simon Tatham,') {
            throw 'Use an official Pageant binary with a valid Simon Tatham signature.'
        }
        if (Get-Process -Name pageant -ErrorAction SilentlyContinue) {
            throw 'An existing Pageant process is running; the isolated test will not alter its identities.'
        }
        $pageantVersion = (Get-Item -LiteralPath $PageantExecutable).VersionInfo.ProductVersion
    }
    # A collision fails without terminating or reconfiguring an existing listener.
    $probe = [System.Net.Sockets.TcpClient]::new()
    try {
        $connection = $probe.ConnectAsync('127.0.0.1', $Port)
        if ($connection.Wait(1000) -and $probe.Connected) { throw 'The fixture port is already in use.' }
    } catch [System.AggregateException] {
        # Connection refusal means the port is available.
    } finally { $probe.Dispose() }
    $started = $true
    Invoke-Fixture 'start'
    $version = (Get-Content -LiteralPath (Join-Path $secrets 'server-version.txt') -Raw).Trim()
    if ($PageantExecutable) {
        $pageantKey = Join-Path $secrets 'pageant.ppk'
        $pageantProcess = Start-Process -FilePath $PageantExecutable -ArgumentList @('-restrict-acl', ('"' + $pageantKey + '"')) -WindowStyle Hidden -PassThru
        Start-Sleep -Milliseconds 500
    }
    $methods = @(
        @{ name = 'password'; password = 'password.txt' },
        @{ name = 'openssh-ed25519'; key = 'openssh' },
        @{ name = 'openssh-ed25519-encrypted'; key = 'openssh-encrypted'; passphrase = 'passphrase.txt' },
        @{ name = 'pkcs8-ed25519'; key = 'pkcs8' },
        @{ name = 'pkcs8-ed25519-encrypted'; key = 'pkcs8-encrypted'; passphrase = 'passphrase.txt' },
        @{ name = 'openssh-rsa'; key = 'rsa-openssh' },
        @{ name = 'openssh-rsa-encrypted'; key = 'rsa-openssh-encrypted'; passphrase = 'passphrase.txt' },
        @{ name = 'pkcs1-rsa'; key = 'rsa-pkcs1' },
        @{ name = 'pkcs8-rsa'; key = 'rsa-pkcs8' },
        @{ name = 'pkcs8-rsa-encrypted'; key = 'rsa-pkcs8-encrypted'; passphrase = 'passphrase.txt' }
    )
    if ($PageantExecutable) {
        $methods += @{ name = 'pageant-ed25519-forwarding-off-on-off'; agent = 'pageant' }
    }
    $methods += @{ name = 'persist-ephemeral-host-trust'; key = 'openssh'; trust = $true }
    $methods += @{ name = 'changed-host-key-blocked'; key = 'openssh'; changed = $true }
    foreach ($method in $methods) {
        Clear-TestEnvironment
        if ($method.password) { $env:CAPSTAN_TEST_SSH_PASSWORD_FILE = Join-Path $secrets $method.password }
        if ($method.key) { $env:CAPSTAN_TEST_SSH_KEY_FILE = Join-Path $secrets $method.key }
        if ($method.passphrase) { $env:CAPSTAN_TEST_SSH_KEY_PASSPHRASE_FILE = Join-Path $secrets $method.passphrase }
        if ($method.agent) {
            $env:CAPSTAN_TEST_SSH_AGENT = $method.agent
            $env:CAPSTAN_TEST_SSH_AGENT_FORWARDING_PUBLIC_KEY = "$fixture/home/forwarding.pub"
        }
        if ($method.trust -or $method.changed) {
            $env:CAPSTAN_TEST_SSH_TRUST_FILE = Join-Path $secrets 'ephemeral-trust.json'
        }
        if ($method.changed) {
            Invoke-Fixture 'rotate'
            $env:CAPSTAN_TEST_SSH_EXPECT_CHANGED = '1'
        }
        $log = Join-Path $evidence "$($method.name).txt"
        $timer = [System.Diagnostics.Stopwatch]::StartNew()
        $ErrorActionPreference = 'Continue'
        try {
            & cargo test -p capstan-ssh --offline --test real_sshd -- --nocapture *> $log
            $exitCode = $LASTEXITCODE
        } finally { $ErrorActionPreference = 'Stop' }
        $timer.Stop()
        $results.Add(@{ method = $method.name; exit_code = $exitCode; elapsed_ms = $timer.ElapsedMilliseconds })
        Write-Output "$($method.name): exit $exitCode"
        if ($exitCode -ne 0) { throw "Real OpenSSH test failed: $($method.name); see $log" }
        if ((Get-Content -LiteralPath $log -Raw).Contains('SKIP:')) { throw 'Configured OpenSSH test unexpectedly skipped.' }
    }
} catch {
    $failure = $_.Exception.Message
} finally {
    if ($pageantProcess) {
        try {
            # Querying Pageant's owner can fail with -restrict-acl; taskkill requests only termination.
            & taskkill.exe /PID $pageantProcess.Id /F | Out-Null
            if ($LASTEXITCODE -ne 0) { throw 'Could not stop the isolated Pageant process.' }
        } catch { $failure = "Pageant cleanup failed: $($_.Exception.Message). Prior failure: $failure" }
    }
    if ($started) {
        try { Invoke-Fixture 'stop'; $cleanupSucceeded = $true } catch { $failure = "Fixture cleanup failed: $($_.Exception.Message). Prior failure: $failure" }
    }
    foreach ($variable in $variables) { [Environment]::SetEnvironmentVariable($variable, $previous[$variable], 'Process') }
    Pop-Location
    $workspacePath = (Resolve-Path -LiteralPath $workspace).Path
    $targetPrefix = (Resolve-Path -LiteralPath (Join-Path $repository 'target')).Path.TrimEnd('\') + '\'
    if (-not $workspacePath.StartsWith($targetPrefix, [System.StringComparison]::OrdinalIgnoreCase)) {
        throw 'The credential cleanup path is outside the repository target directory.'
    }
    Remove-Item -LiteralPath $workspacePath -Recurse -Force
    $report = @{
        schema_version = 1
        measured_at = (Get-Date).ToString('o')
        server_version = $version
        pageant_version = $pageantVersion
        distribution = $Distribution
        binding = "127.0.0.1:$Port"
        host_key_algorithm = $HostKeyAlgorithm
        methods = $results.ToArray()
        cleaned_up = $cleanupSucceeded
        passed = ($null -eq $failure -and $cleanupSucceeded)
        failure = $failure
    }
    $report | ConvertTo-Json -Depth 5 | Set-Content -LiteralPath (Join-Path $evidence 'report.json') -Encoding UTF8
}
Write-Output "Evidence directory: $evidence"
if ($failure) { throw $failure }
