param(
    [string]$Distribution = 'Ubuntu',
    [ValidateRange(1024, 65535)][int]$Port = 22302,
    [string]$EvidenceDirectory,
    [switch]$LibraryPerformance,
    [switch]$ProductionPerformance,
    [ValidateSet('files','tree','resume')][string]$WorkflowMatrix,
    [switch]$MatrixSmoke,
    [switch]$MatrixTrace,
    [switch]$VisibleMixed,
    [switch]$HeadlessMixed,
    [string]$MeasurementExecutable,
    [switch]$CoreWorkflow,
    [switch]$ChannelWindowReference,
    [ValidateSet('loopback','controlled_high_rtt')][string]$NetworkScenario = 'loopback',
    [ValidateSet('wsl_localhost','windows_private')][string]$NetworkRelay = 'wsl_localhost',
    [ValidateSet(0,4194304)][int]$RelaySendBufferBytes = 0,
    [ValidateSet(0,4194304)][int]$RelayReceiveBufferBytes = 0,
    [ValidateSet(64,1024)][int]$ProbeMiB = 1024,
    [ValidateSet(16,32,64)][int]$RequestsPerFile = 16
)

$ErrorActionPreference = 'Stop'
if ($RelaySendBufferBytes -ne 0 -and $NetworkScenario -ne 'controlled_high_rtt') { throw 'Socket configuration is limited to the owned high-RTT relay.' }
. (Join-Path $PSScriptRoot 'M2NetworkRelay.ps1')
$networkProfile = Get-M2NetworkProfile $NetworkScenario $NetworkRelay
if (([int][bool]$LibraryPerformance + [int][bool]$ProductionPerformance + [int][bool]$WorkflowMatrix + [int][bool]$VisibleMixed + [int][bool]$HeadlessMixed) -gt 1) { throw 'Use separate evidence/fixtures for each measurement family.' }
if ($RelayReceiveBufferBytes -ne 0 -and ($NetworkScenario -ne 'controlled_high_rtt' -or $RelaySendBufferBytes -ne 4194304)) { throw 'Receive-buffer control requires the explicit high-RTT enlarged-socket fixture.' }
if ($HeadlessMixed -and $ProbeMiB -eq 1024 -and -not $PSBoundParameters.ContainsKey('ProbeMiB')) { throw 'The full headless diagnostic requires an explicit 1024 MiB input option.' }
if ($MatrixSmoke -and -not ($WorkflowMatrix -or $VisibleMixed)) { throw 'MatrixSmoke requires an explicit workflow matrix.' }
if ($MatrixTrace -and -not ($VisibleMixed -and $MatrixSmoke)) { throw 'MatrixTrace is limited to visible mixed smoke diagnostics.' }
if ($VisibleMixed -and [string]::IsNullOrWhiteSpace($MeasurementExecutable)) { throw 'Visible mixed measurement requires a dedicated opt-in executable.' }
if ($ProductionPerformance -or $WorkflowMatrix -or $VisibleMixed -or $HeadlessMixed) { $RequestsPerFile = 64 }
$repository = (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot '..')).Path
$token = [Guid]::NewGuid().ToString('N').Substring(0, 12)
# Reuse the reviewed M1 fixture's exact path/account allowlists and cleanup.
$workspace = Join-Path $repository "target/m1-sshd-$token"
$secrets = Join-Path $workspace 'secrets'
$fixture = "/var/tmp/capstan-m1-ssh-$token"
$account = "cm1$token"
$variables = @('CAPSTAN_TEST_SSHD', 'CAPSTAN_TEST_SSH_PASSWORD_FILE', 'CAPSTAN_TEST_SSH_HOST_KEY_FILE', 'CAPSTAN_TEST_SFTP_ROOT')
$previous = @{}
foreach ($variable in $variables) { $previous[$variable] = [Environment]::GetEnvironmentVariable($variable, 'Process') }
if ([string]::IsNullOrWhiteSpace($EvidenceDirectory)) {
    $EvidenceDirectory = Join-Path $repository "docs/agent/baselines/M2/raw/$((Get-Date).ToString('yyyy-MM-dd-HHmmss'))-sshd"
}
$evidence = [IO.Path]::GetFullPath($EvidenceDirectory)
if ((Test-Path -LiteralPath (Join-Path $evidence 'report.json')) -or (Test-Path -LiteralPath (Join-Path $evidence 'sftp-probe.txt'))) { throw 'Preserve existing evidence; choose a new evidence directory.' }
New-Item -ItemType Directory -Path $secrets -Force | Out-Null
New-Item -ItemType Directory -Path $evidence -Force | Out-Null
$acl = [Security.AccessControl.DirectorySecurity]::new()
$acl.SetAccessRuleProtection($true, $false)
$sid = [Security.Principal.WindowsIdentity]::GetCurrent().User
$acl.AddAccessRule([Security.AccessControl.FileSystemAccessRule]::new($sid, 'FullControl', 'ContainerInherit,ObjectInherit', 'None', 'Allow'))
Set-Acl -LiteralPath $secrets -AclObject $acl
$started = $false
$networkAttempted = $false
$networkCleaned = $true
$windowsRelay = $null
$relayCleaned = $true
$cleaned = $false
$failure = $null
$exitCode = $null
$coreExitCode = $null
$serverVersion = $null
function Convert-WslPath([string]$Path) {
    $translated = & wsl.exe -d $Distribution -- wslpath -a -u $Path.Replace('\', '/')
    if ($LASTEXITCODE -ne 0) { throw 'Could not translate the SFTP fixture path.' }
    return $translated.Trim()
}
function Invoke-Fixture([string]$Action) {
    $mode = if ($VisibleMixed -or $HeadlessMixed) { 'benchmark-sftp' } else { 'sftp' }
    $fixtureOutput = & wsl.exe -d $Distribution -u root -- sh $scriptPath $Action $fixture $secretPath $Port $account none ed25519 $mode
    $fixtureCode = $LASTEXITCODE
    if ($Action -eq 'stop') { $fixtureOutput | Set-Content -LiteralPath (Join-Path $evidence 'sshd-stop.txt') -Encoding utf8 }
    $fixtureOutput | Write-Output
    if ($fixtureCode -ne 0) { throw "Isolated SFTP fixture failed: $Action" }
}
Push-Location $repository
try {
    $scriptPath = Convert-WslPath (Join-Path $PSScriptRoot 'm1-sshd-fixture.sh')
    $secretPath = Convert-WslPath $secrets
    $probe = [Net.Sockets.TcpListener]::new([Net.IPAddress]::Loopback, $Port)
    try { $probe.Start() } finally { $probe.Stop() }
    $relayAddress = '127.0.0.1'
    if ($NetworkRelay -eq 'windows_private') {
        $relayAddress = Get-M2WslPrivateAddress $Distribution
        $relayCleaned = $false
        $windowsRelay = Start-M2WindowsRelay $relayAddress $Port $token $workspace $evidence
    }
    $started = $true
    Invoke-Fixture 'start'
    if ($networkProfile.useNamespace) {
        $networkAttempted = $true
        $networkCleaned = $false
        $networkScript = Convert-WslPath (Join-Path $PSScriptRoot 'm2-net-fixture.sh')
        $ErrorActionPreference = 'Continue'
        try { & wsl.exe -d $Distribution -u root -- sh -x $networkScript start $fixture $networkProfile.oneWayDelayMs $relayAddress $RelaySendBufferBytes $RelayReceiveBufferBytes *> (Join-Path $evidence 'network-start.txt'); $networkCode = $LASTEXITCODE } finally { $ErrorActionPreference = 'Stop' }
        if ($networkCode -ne 0) { throw 'Controlled network fixture failed to start.' }
        $networkLog = & wsl.exe -d $Distribution -u root -- sh $networkScript observe $fixture $networkProfile.oneWayDelayMs
        if ($LASTEXITCODE -ne 0) { throw 'Controlled network RTT observation failed.' }
        $networkLog | Set-Content -LiteralPath (Join-Path $evidence 'network.txt') -Encoding utf8
        $measuredSubdirectory = if ($VisibleMixed) { 'visible-mixed' } elseif ($WorkflowMatrix) { 'workflow-matrix' } elseif ($ProductionPerformance) { 'product-performance' } else { 'library-performance' }
        [ordered]@{ scenario = $NetworkScenario; relay = $NetworkRelay; relayAddress = $relayAddress; relayRequestedSendBufferBytes = $RelaySendBufferBytes; relayRequestedReceiveBufferBytes = $RelayReceiveBufferBytes; relayConfiguredSocket = 'Linux relay -> owned sshd namespace only'; relaySocketCounters = 'network-stop.txt'; oneWayDelayMs = $networkProfile.oneWayDelayMs; expectedInjectedRttMs = $networkProfile.expectedRttMs; injection = 'netem on owned namespace veth pair only'; bandwidthCap = $null; jitterMs = 0; lossPercent = 0; actualIcmpRaw = 'network.txt'; actualSshEcho = if($HeadlessMixed){$null}else{"$measuredSubdirectory/ssh-rtt-reference.json"}; scope = 'isolated namespace plus explicitly recorded relay; same route for product, WinSCP and probes; not WAN or TCP-connect timing' } |
            ConvertTo-Json | Set-Content -LiteralPath (Join-Path $evidence 'network.json') -Encoding utf8
    }
    $serverVersion = (Get-Content -LiteralPath (Join-Path $secrets 'server-version.txt') -Raw).Trim()
    $env:CAPSTAN_TEST_SSHD = "$account@127.0.0.1:$Port"
    $env:CAPSTAN_TEST_SSH_PASSWORD_FILE = Join-Path $secrets 'password.txt'
    $env:CAPSTAN_TEST_SSH_HOST_KEY_FILE = Join-Path $secrets 'host-key.pub'
    $env:CAPSTAN_TEST_SFTP_ROOT = "$fixture/home"
    $log = Join-Path $evidence 'sftp-probe.txt'
    $ErrorActionPreference = 'Continue'
    try {
        & cargo test -p capstan-ssh --offline --locked --test sftp_protocol configured_openssh_sftp_probe -- --exact --nocapture *> $log
        $exitCode = $LASTEXITCODE
    } finally { $ErrorActionPreference = 'Stop' }
    if ($exitCode -ne 0) { throw "SFTP probe failed; see $log" }
    $output = Get-Content -LiteralPath $log -Raw
    if ($output.Contains('SKIP:') -or -not $output.Contains('SFTP_PROBE ')) { throw 'Configured SFTP probe did not produce real execution evidence.' }
    if ($CoreWorkflow) {
        $coreLog = Join-Path $evidence 'core-workflow.txt'
        $ErrorActionPreference = 'Continue'
        try {
            & cargo test -p capstan-core --offline --locked --test sftp_workflow transfers::configured_openssh_core_file_workflow -- --exact --nocapture *> $coreLog
            $coreExitCode = $LASTEXITCODE
        } finally { $ErrorActionPreference = 'Stop' }
        if ($coreExitCode -ne 0) { throw "Production core file workflow failed; see $coreLog" }
        $coreOutput = Get-Content -LiteralPath $coreLog -Raw
        if ($coreOutput.Contains('SKIP:') -or -not $coreOutput.Contains('SFTP_CORE_WORKFLOW ')) { throw 'Production core workflow did not produce real execution evidence.' }
    }
    if ($ChannelWindowReference) {
        & (Join-Path $PSScriptRoot 'Measure-M2SshWindow.ps1') -EvidenceDirectory (Join-Path $evidence 'channel-window') -Workspace $workspace
    }
    if ($LibraryPerformance) {
        & (Join-Path $PSScriptRoot 'Measure-M2SftpLibrary.ps1') -EvidenceDirectory (Join-Path $evidence 'library-performance') -Workspace $workspace -NetworkScenario $NetworkScenario -ProbeMiB $ProbeMiB -RequestsPerFile $RequestsPerFile
    }
    if ($ProductionPerformance) {
        & (Join-Path $PSScriptRoot 'Measure-M2SftpLibrary.ps1') -EvidenceDirectory (Join-Path $evidence 'product-performance') -Workspace $workspace -NetworkScenario $NetworkScenario -ProbeMiB $ProbeMiB -RequestsPerFile 64 -Implementation product
    }
    if ($WorkflowMatrix) {
        & (Join-Path $PSScriptRoot 'Measure-M2Workflow.ps1') -EvidenceDirectory (Join-Path $evidence 'workflow-matrix') -Workspace $workspace -NetworkScenario $NetworkScenario -ProbeMiB $ProbeMiB -Mode $WorkflowMatrix -Smoke:$MatrixSmoke
    }
    if ($VisibleMixed) {
        & (Join-Path $PSScriptRoot 'Measure-M2Mixed.ps1') -EvidenceDirectory (Join-Path $evidence 'visible-mixed') -Workspace $workspace -Executable $MeasurementExecutable -NetworkScenario $NetworkScenario -ProbeMiB $ProbeMiB -Smoke:$MatrixSmoke -DiagnosticTrace:$MatrixTrace
    }
    if ($HeadlessMixed) {
        & (Join-Path $PSScriptRoot 'Test-M2HeadlessMixed.ps1') -EvidenceDirectory (Join-Path $evidence 'headless-mixed') -Workspace $workspace -ProbeMiB $ProbeMiB
    }
} catch {
    $failure = $_.Exception.Message
} finally {
    if ($windowsRelay) {
        try { Stop-M2WindowsRelay $windowsRelay; $relayCleaned = $true } catch { $failure = "Windows relay cleanup failed: $($_.Exception.Message). Prior failure: $failure" }
    }
    if ($networkAttempted) {
        try {
            $networkStop = & wsl.exe -d $Distribution -u root -- sh $networkScript stop $fixture $networkProfile.oneWayDelayMs
            $networkStopCode = $LASTEXITCODE
            $networkStop | Set-Content -LiteralPath (Join-Path $evidence 'network-stop.txt') -Encoding utf8
            $networkStop | Write-Output
            if ($networkStopCode -ne 0) { throw 'Controlled network cleanup failed.' }
            $networkCleaned = $true
        } catch { $failure = "Network fixture cleanup failed: $($_.Exception.Message). Prior failure: $failure" }
    }
    if ($started) {
        try { Invoke-Fixture 'stop'; $cleaned = $true } catch { $failure = "SFTP fixture cleanup failed: $($_.Exception.Message). Prior failure: $failure" }
    }
    foreach ($variable in $variables) { [Environment]::SetEnvironmentVariable($variable, $previous[$variable], 'Process') }
    Pop-Location
    $resolvedWorkspace = (Resolve-Path -LiteralPath $workspace).Path
    $targetPrefix = (Resolve-Path -LiteralPath (Join-Path $repository 'target')).Path.TrimEnd('\') + '\'
    if (-not $resolvedWorkspace.StartsWith($targetPrefix, [StringComparison]::OrdinalIgnoreCase)) { throw 'Credential cleanup path is outside the workspace target directory.' }
    Remove-Item -LiteralPath $resolvedWorkspace -Recurse -Force
    [ordered]@{
        schemaVersion = 1; date = (Get-Date).ToString('o'); serverVersion = $serverVersion
        distribution = $Distribution; exitCode = $exitCode; cleanedUp = $cleaned
        coreWorkflowExitCode = $coreExitCode; coreWorkflowRequested = [bool]$CoreWorkflow
        channelWindowReferenceRequested = [bool]$ChannelWindowReference
        productionPerformanceRequested = [bool]$ProductionPerformance
        workflowMatrix = $WorkflowMatrix; matrixSmoke = [bool]$MatrixSmoke
        visibleMixedRequested = [bool]$VisibleMixed
        headlessMixedRequested = [bool]$HeadlessMixed
        passed = ($null -eq $failure -and $cleaned -and $networkCleaned -and $relayCleaned -and $exitCode -eq 0)
        networkRelay = $NetworkRelay; windowsRelayCleaned = $relayCleaned
        relayRequestedSendBufferBytes = $RelaySendBufferBytes
        relayRequestedReceiveBufferBytes = $RelayReceiveBufferBytes
        networkScenario = $NetworkScenario; networkCleaned = $networkCleaned; probeMiB = $ProbeMiB; requestsPerFile = $RequestsPerFile
        performanceAcceptance = $false; desktopAcceptance = $false; failure = $failure
    } | ConvertTo-Json -Depth 5 | Set-Content -LiteralPath (Join-Path $evidence 'report.json') -Encoding utf8
}
if ($failure) { throw $failure }
Write-Output "Real OpenSSH SFTP smoke probe passed. Evidence: $evidence"
