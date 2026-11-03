param(
    [Parameter(Mandatory = $true)][string]$EvidenceDirectory,
    [Parameter(Mandatory = $true)][string]$Workspace,
    [string]$WinScpDirectory = 'C:/Program Files (x86)/WinSCP',
    [ValidateSet(64,1024)][int]$ProbeMiB = 1024,
    [ValidateSet('loopback','controlled_high_rtt')][string]$NetworkScenario = 'loopback',
    [ValidateSet(16,32,64)][int]$RequestsPerFile = 16,
    [ValidateSet('library','product')][string]$Implementation = 'library'
)
$ErrorActionPreference = 'Stop'
$repository = (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot '..')).Path
$workspaceRoot = [IO.Path]::GetFullPath($Workspace)
$targetPrefix = (Join-Path $repository 'target').TrimEnd('\') + '\'
if (-not $workspaceRoot.StartsWith($targetPrefix, [StringComparison]::OrdinalIgnoreCase) -or
    -not ([IO.Path]::GetFileName($workspaceRoot) -match '^m1-sshd-[0-9a-f]{12}$')) { throw 'Performance input must be in the isolated fixture workspace.' }
if ($env:CAPSTAN_TEST_SFTP_ROOT -notmatch '^/var/tmp/capstan-m1-ssh-[0-9a-f]{12}/home$' -or
    $env:CAPSTAN_TEST_SSHD -notmatch '^(cm1[0-9a-f]{12})@127\.0\.0\.1:([0-9]+)$') { throw 'Run this probe only through Test-M2SftpProbe.ps1.' }
$username = $Matches[1]
$port = [int]$Matches[2]
$evidence = [IO.Path]::GetFullPath($EvidenceDirectory)
if (Test-Path -LiteralPath (Join-Path $evidence 'samples.json')) { throw 'Preserve existing samples; choose a new evidence directory.' }
if ($Implementation -eq 'product' -and $RequestsPerFile -ne 64) { throw 'The production window is frozen at 64 requests.' }
New-Item -ItemType Directory -Path $evidence -Force | Out-Null
$samples = [Collections.Generic.List[object]]::new()
$variables = @('CAPSTAN_TEST_SFTP_PERFORMANCE', 'CAPSTAN_TEST_TRANSFER_PERFORMANCE', 'CAPSTAN_TEST_SFTP_INPUT', 'CAPSTAN_TEST_SFTP_OUTPUT', 'CAPSTAN_TEST_SFTP_SHA256', 'CAPSTAN_TEST_SFTP_DIRECTION','CAPSTAN_TEST_SFTP_REQUESTS')
$previous = @{}
foreach ($variable in $variables) { $previous[$variable] = [Environment]::GetEnvironmentVariable($variable, 'Process') }
$inputFile = Join-Path $workspaceRoot 'payload.bin'
$outputFile = Join-Path $workspaceRoot 'returned.bin'
$referenceFile = Join-Path $workspaceRoot 'reference.bin'
$remoteReference = "$env:CAPSTAN_TEST_SFTP_ROOT/reference-benchmark.bin"
$candidateLabel = if ($Implementation -eq 'product') { 'product' } else { 'library' }
$remoteLibrary = "$env:CAPSTAN_TEST_SFTP_ROOT/$candidateLabel-benchmark.bin"
$policyFile = Join-Path $repository 'docs/agent/baselines/M2/performance-policy.json'
$policyHash = (Get-FileHash -LiteralPath $policyFile -Algorithm SHA256).Hash
$registered = Get-Content -LiteralPath $policyFile -Raw | ConvertFrom-Json
$minimumRatio = [double]$registered.largeFileMinimumWinScpThroughputRatio
if ($Implementation -eq 'product' -and $registered.status -ne 'frozen_before_product_candidate') { throw 'Freeze the preregistered policy before production measurements.' }
$failure = $null
$feasibilityPassed = $false
$session = $null
$referenceSession = $null
$priorExecutionState = [uint32]0
$awakeReleased = $false
$workers = [Collections.Generic.List[object]]::new()
$utf8 = [Text.UTF8Encoding]::new($false)
function Save-Json([string]$Path,$Value) { [IO.File]::WriteAllText($Path,(ConvertTo-Json -InputObject $Value -Depth 12),$utf8) }
. (Join-Path $PSScriptRoot 'M2OwnedProcess.ps1')
function New-WinReferenceSession([string]$Name) {
    $created = New-Object WinSCP.Session
    $created.ExecutablePath = Join-Path $WinScpDirectory 'WinSCP.exe'
    $created.SessionLogPath = Join-Path $evidence ("winscp-$Name.log")
    try {
        $created.Open($options)
        $negotiated = Get-Content -LiteralPath $created.SessionLogPath -Raw
        if (-not $negotiated.Contains('Doing ECDH key exchange with curve Curve25519, using hash SHA-256') -or
            -not $negotiated.Contains('[aes256-gcm@openssh.com] outbound encryption') -or
            -not $negotiated.Contains('[aes256-gcm@openssh.com] inbound encryption') -or
            -not $negotiated.Contains('Compression: No')) { throw 'Reference negotiated settings changed.' }
        return $created
    } catch { $created.Dispose(); throw }
}
function Invoke-MeasurementProbe([string]$Executable, [string[]]$ProbeArguments, [string]$Log, [int]$TimeoutSeconds) {
    # Only fixed test filters/options are accepted here, never local paths,
    # user commands or shell text. Avoid Windows argv string interpolation.
    if (@($ProbeArguments | Where-Object { $_ -notmatch '^[A-Za-z0-9_:/=+.\-]+$' }).Count) { throw 'Unsupported measurement filter/option.' }
    $worker = Start-OwnedMatrix $Executable ($ProbeArguments -join ' ') $Log ''
    try {
        $result = @(Wait-OwnedMatrix @($worker) $TimeoutSeconds (Join-Path $evidence 'cancel.request'))[0]
        return [pscustomobject]@{exitCode=0;elapsedMs=$result.elapsedMs}
    } finally {
        if($worker.output.Status -eq [Threading.Tasks.TaskStatus]::RanToCompletion){[IO.File]::WriteAllText("$Log.stdout",$worker.output.GetAwaiter().GetResult(),$utf8)}
        if($worker.errors.Status -eq [Threading.Tasks.TaskStatus]::RanToCompletion){[IO.File]::WriteAllText("$Log.stderr",$worker.errors.GetAwaiter().GetResult(),$utf8)}
    }
}
Push-Location $repository
try {
    # A measurement is invalid across suspend. Hold only this helper thread's
    # temporary idle-sleep requirement; never alter the user's power plan.
    if (-not ('Capstan.M2MeasurePower' -as [type])) {
        Add-Type -TypeDefinition @'
using System.Runtime.InteropServices;
namespace Capstan {
    public static class M2MeasurePower {
        [DllImport("kernel32.dll", SetLastError = true)]
        public static extern uint SetThreadExecutionState(uint flags);
    }
}
'@
    }
    $priorExecutionState = [Capstan.M2MeasurePower]::SetThreadExecutionState([uint32]2147483649)
    if ($priorExecutionState -eq 0) { throw 'Could not hold the measurement helper idle-sleep requirement.' }
    # Compile before any measurement; never count build time as file transfer.
    $ErrorActionPreference = 'Continue'
    & cargo test -p capstan-ssh --release --test sftp_performance --offline --locked --no-run *> (Join-Path $evidence 'build.txt')
    $buildCode = $LASTEXITCODE
    $ErrorActionPreference = 'Stop'
    if ($buildCode -ne 0) { throw 'The release library probe did not build.' }
    $tests = @(Get-ChildItem (Join-Path $repository 'target/release/deps/sftp_performance-*.exe') | Sort-Object LastWriteTimeUtc -Descending)
    if (-not $tests.Count) { throw 'The compiled probe executable is missing.' }
    $probeExecutable = $tests[0].FullName
    $rttExecutable = $probeExecutable
    if ($Implementation -eq 'product') {
        $ErrorActionPreference = 'Continue'
        & cargo test -p capstan-core --release --test sftp_workflow --features benchmarks --offline --locked --no-run *> (Join-Path $evidence 'production-build.txt')
        $productionBuildCode = $LASTEXITCODE
        $ErrorActionPreference = 'Stop'
        if ($productionBuildCode -ne 0) { throw 'The release production transfer probe did not build.' }
        $productionTests = @(Get-ChildItem (Join-Path $repository 'target/release/deps/sftp_workflow-*.exe') | Sort-Object LastWriteTimeUtc -Descending)
        if (-not $productionTests.Count) { throw 'The production probe executable is missing.' }
        $probeExecutable = $productionTests[0].FullName
    }
    $generation = [Diagnostics.Stopwatch]::StartNew()
    $random = [Random]::new(20261017)
    $buffer = [byte[]]::new(1024 * 1024)
    $file = [IO.File]::Open($inputFile, [IO.FileMode]::CreateNew, [IO.FileAccess]::Write, [IO.FileShare]::None)
    try { for ($index = 0; $index -lt $ProbeMiB; $index++) { $random.NextBytes($buffer); $file.Write($buffer, 0, $buffer.Length) } } finally { $file.Dispose() }
    $generation.Stop()
    $hash = (Get-FileHash -LiteralPath $inputFile -Algorithm SHA256).Hash
    $version = (Get-Item (Join-Path $WinScpDirectory 'WinSCP.exe')).VersionInfo.ProductVersion
    [ordered]@{
        schemaVersion = 1; policySha256 = $policyHash; bytes = ([long]$ProbeMiB * 1MB).ToString(); seed = 20261017
        generator = 'System.Random.NextBytes, one continuous seeded stream'; runtime = [Environment]::Version.ToString()
        generationMs = $generation.ElapsedMilliseconds; sha256 = $hash; probeSha256 = (Get-FileHash $probeExecutable).Hash
        implementation = $Implementation; rttProbeSha256 = (Get-FileHash $rttExecutable).Hash
        temporarySystemAwakeRequest = $true; powerPlanChanged = $false
        winScpVersion = $version; winScpSha256 = (Get-FileHash (Join-Path $WinScpDirectory 'WinSCP.exe')).Hash
        os = [Environment]::OSVersion.VersionString; powerScheme = (& powercfg /getactivescheme) -join ' '
        rust = (& rustc --version) -join ' '; network = $NetworkScenario
        cipher = 'aes256-gcm@openssh.com'; kex = 'curve25519-sha256'; compression = $false
        productionAcceptance = $false; highRttMeasured = ($NetworkScenario -eq 'controlled_high_rtt'); concurrency = 1
        requestsPerFile = $RequestsPerFile
        connectionMethod = 'Fresh data SSH connection per sample for both implementations; one shared idle control session performs unmeasured fixture cleanup.'
    } | ConvertTo-Json -Depth 5 | Set-Content (Join-Path $evidence 'input-environment.json') -Encoding utf8
    Add-Type -Path (Join-Path $WinScpDirectory 'WinSCPnet.dll')
    $publicKey = & ssh-keygen -l -E sha256 -f $env:CAPSTAN_TEST_SSH_HOST_KEY_FILE
    if ($LASTEXITCODE -ne 0) { throw 'Could not read the pinned fixture public key.' }
    $fingerprint = ($publicKey -split '\s+')[1]
    if ($fingerprint -notmatch '^SHA256:([A-Za-z0-9+/]{43})$') { throw 'Unexpected pinned fingerprint format.' }
    $fingerprint = $Matches[1]
    $options = New-Object WinSCP.SessionOptions
    $options.Protocol = [WinSCP.Protocol]::Sftp
    $options.HostName = '127.0.0.1'
    $options.PortNumber = $port
    $options.UserName = $username
    $options.Password = ([IO.File]::ReadAllText($env:CAPSTAN_TEST_SSH_PASSWORD_FILE)).TrimEnd("`r", "`n")
    $options.SshHostKeyFingerprint = "ssh-ed25519 255 $fingerprint"
    $options.AddRawSettings('Compression', '0')
    $session = New-Object WinSCP.Session
    $session.ExecutablePath = Join-Path $WinScpDirectory 'WinSCP.exe'
    $session.SessionLogPath = Join-Path $evidence 'winscp.log'
    $session.Open($options)
    $negotiation = Get-Content -LiteralPath $session.SessionLogPath -Raw
    if (-not $negotiation.Contains('Doing ECDH key exchange with curve Curve25519, using hash SHA-256') -or
        -not $negotiation.Contains('[aes256-gcm@openssh.com] outbound encryption') -or
        -not $negotiation.Contains('[aes256-gcm@openssh.com] inbound encryption') -or
        -not $negotiation.Contains('Compression: No')) { throw 'WinSCP negotiated settings do not match the fixed library reference.' }
    [ordered]@{ checkedFrom = 'winscp.log'; kex = 'curve25519-sha256'; cipher = 'aes256-gcm@openssh.com'; compression = $false } |
        ConvertTo-Json | Set-Content -LiteralPath (Join-Path $evidence 'negotiation-checked.json') -Encoding utf8
    $transferOptions = New-Object WinSCP.TransferOptions
    $transferOptions.TransferMode = [WinSCP.TransferMode]::Binary
    $transferOptions.PreserveTimestamp = $false
    $transferOptions.ResumeSupport.State = [WinSCP.TransferResumeSupportState]::Off
    $env:CAPSTAN_TEST_SFTP_PERFORMANCE = '1'
    $env:CAPSTAN_TEST_TRANSFER_PERFORMANCE = if ($Implementation -eq 'product') { '1' } else { $null }
    $env:CAPSTAN_TEST_SFTP_INPUT = $inputFile
    $env:CAPSTAN_TEST_SFTP_OUTPUT = $outputFile
    $env:CAPSTAN_TEST_SFTP_SHA256 = $hash
    $env:CAPSTAN_TEST_SFTP_REQUESTS = $RequestsPerFile.ToString()
    $rttLog = Join-Path $evidence 'ssh-rtt-reference.txt'
    $rttExecution = Invoke-MeasurementProbe $rttExecutable @('configured_openssh_shell_rtt_reference', '--exact', '--nocapture') $rttLog 60
    $rttCode = $rttExecution.exitCode
    if ($rttCode -ne 0) { throw 'The authenticated SSH RTT reference failed.' }
    $rttRaw = Get-Content -LiteralPath $rttLog -Raw
    $rttMatch = [regex]::Match($rttRaw, 'SSH_RTT_SAMPLE (\{.*?\})', 'Singleline')
    if (-not $rttMatch.Success) { throw 'No actual SSH RTT reference was emitted.' }
    $rttReference = ($rttMatch.Groups[1].Value -replace "`r?`n", '') | ConvertFrom-Json
    $rttReference | ConvertTo-Json -Depth 5 | Set-Content -LiteralPath (Join-Path $evidence 'ssh-rtt-reference.json') -Encoding utf8
    # Reference runs finish first; retain all rounds and freeze the derived
    # loopback reference before executing any library sample.
    foreach ($candidate in @('winscp', $candidateLabel)) {
        if ($candidate -eq $candidateLabel) {
            $reference = @($samples | Where-Object { -not $_.warmup })
            $limits = [ordered]@{}
            foreach ($direction in @('upload', 'download')) {
                $ordered = @($reference | Where-Object direction -eq $direction | Sort-Object transferMs)
                $medianMs = [double]$ordered[1].transferMs
                $limits[$direction] = [ordered]@{ referenceMedianTransferMs = $medianMs; minimumBytesPerSecond = ([double]$ProbeMiB * 1MB) / ($medianMs/1000) * $minimumRatio }
            }
            [ordered]@{ policySha256 = $policyHash; scope = "$Implementation, one file, one active worker"; network = $NetworkScenario; bytes = ([long]$ProbeMiB * 1MB).ToString(); limits = $limits; frozenBeforeLibrarySample = $true; frozenBeforeCandidateSample = $true } |
                ConvertTo-Json -Depth 5 | Set-Content (Join-Path $evidence 'frozen-loopback-reference.json') -Encoding utf8
        }
        for ($round = 0; $round -le 3; $round++) {
            foreach ($direction in @('upload', 'download')) {
                if(Test-Path -LiteralPath (Join-Path $evidence 'cancel.request') -PathType Leaf){throw 'Single-file measurement cancelled before the next sample.'}
                if (@(Get-Process -Name cargo,rustc,clippy-driver -ErrorAction SilentlyContinue).Count) {
                    throw 'Stop concurrent compiler/check processes before collecting the fixed library reference.'
                }
                $sampleName = "$candidate-$round-$direction"
                Write-Output "SFTP $Implementation measurement: $sampleName"
                if ($direction -eq 'upload') {
                    $remoteFile = if ($candidate -eq 'winscp') { $remoteReference } else { $remoteLibrary }
                    if ($session.FileExists($remoteFile)) { $session.RemoveFile($remoteFile) | Out-Null }
                }
                if ($candidate -eq 'winscp') {
                    # Match the Rust probe's fresh connection per sample.
                    # The persistent $session remains an idle cleanup channel
                    # in both phases and never carries measured file bodies.
                    $prepare = [Diagnostics.Stopwatch]::StartNew()
                    $referenceSession = New-WinReferenceSession $sampleName
                    if (Test-Path -LiteralPath $referenceFile) { Remove-Item -LiteralPath $referenceFile -Force }
                    $prepare.Stop()
                    $timer = [Diagnostics.Stopwatch]::StartNew()
                    if ($direction -eq 'upload') { $referenceSession.PutFiles($inputFile, $remoteReference, $false, $transferOptions).Check() }
                    else { $referenceSession.GetFiles($remoteReference, $referenceFile, $false, $transferOptions).Check() }
                    $timer.Stop()
                    $verify = [Diagnostics.Stopwatch]::StartNew()
                    if ($direction -eq 'upload') { $referenceSession.GetFiles($remoteReference, $referenceFile, $false, $transferOptions).Check() }
                    $actualHash = (Get-FileHash -LiteralPath $referenceFile -Algorithm SHA256).Hash
                    if ($actualHash -ne $hash) { throw 'WinSCP reference hash mismatch.' }
                    $verify.Stop()
                    $close = [Diagnostics.Stopwatch]::StartNew()
                    $referenceSession.Dispose(); $referenceSession = $null
                    $close.Stop()
                    $sample = [ordered]@{ implementation = 'winscp'; round = $round; warmup = ($round -eq 0); direction = $direction; bytes = ([long]$ProbeMiB * 1MB).ToString(); prepareMs = $prepare.Elapsed.TotalMilliseconds; transferMs = $timer.Elapsed.TotalMilliseconds; verificationMs = $verify.Elapsed.TotalMilliseconds; connectionCloseMs = $close.Elapsed.TotalMilliseconds; totalMs = ($prepare.Elapsed.TotalMilliseconds + $timer.Elapsed.TotalMilliseconds + $verify.Elapsed.TotalMilliseconds + $close.Elapsed.TotalMilliseconds); sha256 = $actualHash; hashMatched = $true }
                } else {
                    if (Test-Path -LiteralPath $outputFile) { Remove-Item -LiteralPath $outputFile -Force }
                    $env:CAPSTAN_TEST_SFTP_DIRECTION = $direction
                    $log = Join-Path $evidence "$sampleName.txt"
                    $testName = if ($Implementation -eq 'product') { 'performance::configured_openssh_transfer_performance' } else { 'configured_openssh_sftp_performance_probe' }
                    $deadline = if ($ProbeMiB -eq 64) { 180 } else { 900 }
                    $execution = Invoke-MeasurementProbe $probeExecutable @($testName, '--exact', '--nocapture') $log $deadline
                    $code = $execution.exitCode
                    if ($code -ne 0) { throw "$Implementation sample failed: $sampleName (exit $code)" }
                    $raw = Get-Content -LiteralPath $log -Raw
                    if ($Implementation -eq 'product') {
                        $record = [regex]::Match($raw, '(?m)^TRANSFER_SAMPLE (\{[^\r\n]*\})\r?$')
                        if (-not $record.Success) { throw 'Production sample has no complete stdout record.' }
                        $sample = $record.Groups[1].Value | ConvertFrom-Json
                    } else {
                        $marker = $raw.IndexOf('SFTP_SAMPLE ', [StringComparison]::Ordinal)
                        if ($marker -lt 0) { throw 'Library sample has no execution record.' }
                        $payload = $raw.Substring($marker + 12)
                        $diagnostic = $payload.IndexOf('At D:', [StringComparison]::Ordinal)
                        if ($diagnostic -ge 0) { $payload = $payload.Substring(0, $diagnostic) }
                        # Legacy stderr records may be presentation-wrapped.
                        $sample = ($payload -replace '\s+', '') | ConvertFrom-Json
                    }
                    if ($sample.negotiated.cipher -ne 'aes256-gcm@openssh.com' -or $sample.negotiated.kex -ne 'curve25519-sha256') {
                        throw 'Library negotiated settings do not match the fixed WinSCP reference.'
                    }
                    $bodyWall = if ($Implementation -eq 'product') { [double]$sample.wallMsIncludingIndependentOracle } else { [double]$sample.totalMs }
                    $helperOverhead = [Math]::Max(0.0, [double]$execution.elapsedMs - $bodyWall)
                    $sample | Add-Member 'testReportedTotalMs' ([double]$sample.totalMs)
                    $sample | Add-Member 'helperOverheadMs' $helperOverhead
                    $sample | Add-Member 'externalProcessMs' ([double]$execution.elapsedMs)
                    $sample.totalMs = [double]$sample.totalMs + $helperOverhead
                    $sample.prepareMs = [double]$sample.prepareMs + $helperOverhead
                    $sample | Add-Member round $round
                    $sample | Add-Member warmup ($round -eq 0)
                }
                $samples.Add([pscustomobject]$sample)
                $samples.ToArray() | ConvertTo-Json -Depth 8 | Set-Content (Join-Path $evidence 'samples.json') -Encoding utf8
            }
        }
    }
    $comparisons = foreach ($direction in @('upload', 'download')) {
        $win = @($samples | Where-Object { $_.implementation -eq 'winscp' -and $_.direction -eq $direction -and -not $_.warmup } | Sort-Object transferMs)
        $lib = @($samples | Where-Object { $_.implementation -ne 'winscp' -and $_.direction -eq $direction -and -not $_.warmup } | Sort-Object transferMs)
        $ratio = [double]$win[1].transferMs / [double]$lib[1].transferMs
        $passed = ($ratio -ge $minimumRatio)
        $winTotal = @($win | Sort-Object totalMs)
        $libTotal = @($lib | Sort-Object totalMs)
        $totalRatio = [double]$winTotal[1].totalMs / [double]$libTotal[1].totalMs
        $absolute = $null
        $absolutePassed = $null
        $measuredBytesPerSecond = ([double]$ProbeMiB * 1MB) / ([double]$lib[1].transferMs / 1000)
        if ($Implementation -eq 'product') {
            $networkLimit = $registered.absoluteLimits.byNetwork.$NetworkScenario
            $absolute = if ($direction -eq 'upload') { $networkLimit.minimumUploadBytesPerSecond } else { $networkLimit.minimumDownloadBytesPerSecond }
            if ($null -eq $absolute -or $absolute -le 0) { throw 'The frozen absolute limit is missing.' }
            # The phase boundary is sampled every 5 ms. Include 10 ms of
            # conservative phase error before checking its absolute floor.
            $absolutePassed = (([double]$ProbeMiB * 1MB) / (([double]$lib[1].transferMs + 10) / 1000) -ge [double]$absolute)
            $passed = $passed -and $absolutePassed -and ($totalRatio -ge $minimumRatio)
        }
        [ordered]@{
            direction = $direction; winScpMedianTransferMs = $win[1].transferMs; libraryMedianTransferMs = $lib[1].transferMs
            throughputRatio = $ratio; minimumRatio = $minimumRatio; passed = $passed
            candidateImplementation = $Implementation; winScpMedianTotalMs = $winTotal[1].totalMs; candidateMedianTotalMs = $libTotal[1].totalMs
            endToEndThroughputRatio = $totalRatio; endToEndThresholdApplied = ($Implementation -eq 'product')
            transferBytesPerSecond = $measuredBytesPerSecond; frozenAbsoluteMinimumBytesPerSecond = $absolute; absolutePassed = $absolutePassed
        }
    }
    $feasibilityPassed = @($comparisons | Where-Object { -not $_.passed }).Count -eq 0
    [ordered]@{ policySha256 = $policyHash; comparisons = @($comparisons); feasibilityPassed = $feasibilityPassed; performanceAcceptance = $false; network = $NetworkScenario; bytes = ([long]$ProbeMiB * 1MB).ToString(); requestsPerFile = $RequestsPerFile; scope = "$Implementation single-task engineering measurement; full M2 performance acceptance remains pending"; pending = @('complete production 1 GiB low/high RTT matrix', '3 product workers', 'small tree', 'terminal mixed load', 'production drag', 'candidate build') } |
        ConvertTo-Json -Depth 6 | Set-Content (Join-Path $evidence 'comparison.json') -Encoding utf8
    if (-not $feasibilityPassed) { throw 'The selected implementation misses its frozen comparison/absolute targets. Retain all samples; do not loosen thresholds.' }
} catch { $failure = $_.Exception.Message } finally {
    foreach($worker in $workers){
        try { Stop-OwnedMatrix $worker } catch { $failure="Probe cleanup failed: $($_.Exception.Message). Prior failure: $failure" } finally { $worker.process.Dispose() }
    }
    if ($referenceSession) { $referenceSession.Dispose() }
    if ($session) { $session.Dispose() }
    if ($priorExecutionState -ne 0) {
        $awakeReleased = ([Capstan.M2MeasurePower]::SetThreadExecutionState($priorExecutionState) -ne 0)
        if (-not $awakeReleased) { $failure = "Could not restore the helper execution state. Prior failure: $failure" }
    }
    foreach ($variable in $variables) { [Environment]::SetEnvironmentVariable($variable, $previous[$variable], 'Process') }
    Pop-Location
    [ordered]@{ completed = ($samples.Count -eq 16); feasibilityPassed = $feasibilityPassed; failure = $failure; retainedSamples = $samples.Count; performanceAcceptance = $false; temporaryAwakeRequestReleased = $awakeReleased; powerPlanChanged = $false } |
        ConvertTo-Json | Set-Content (Join-Path $evidence 'report.json') -Encoding utf8
}
if ($failure) { throw $failure }
