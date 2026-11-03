param(
    [Parameter(Mandatory = $true)][string]$LowReferenceDirectory,
    [Parameter(Mandatory = $true)][string]$HighReferenceDirectory,
    [string]$M1FrozenLimits = 'docs/agent/baselines/M1/raw/2026-10-17-visible-performance-r4/frozen-limits.json'
)
$ErrorActionPreference = 'Stop'
$repository = (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot '..')).Path
$policyPath = Join-Path $repository 'docs/agent/baselines/M2/performance-policy.json'
$policy = Get-Content -LiteralPath $policyPath -Raw | ConvertFrom-Json
if ($policy.status -ne 'preregistered_reference_pending' -or $null -ne $policy.absoluteLimits -or $null -ne $policy.mixedTerminalLoadLimits) {
    throw 'Never replace an existing freeze or loosen its thresholds.'
}
$priorPolicyHash = (Get-FileHash -LiteralPath $policyPath).Hash
$m1Path = [IO.Path]::GetFullPath((Join-Path $repository $M1FrozenLimits))
$m1 = Get-Content -LiteralPath $m1Path -Raw | ConvertFrom-Json
$byNetwork = [ordered]@{}
$mixed = [ordered]@{}
$referenceIdentities = [Collections.Generic.List[object]]::new()
foreach ($item in @(@{ name='loopback'; directory=$LowReferenceDirectory }, @{ name='controlled_high_rtt'; directory=$HighReferenceDirectory })) {
    $directory = [IO.Path]::GetFullPath($item.directory)
    $root = Join-Path $directory 'library-performance'
    $complete = Get-Content -LiteralPath (Join-Path $root 'report.json') -Raw | ConvertFrom-Json
    $comparison = Get-Content -LiteralPath (Join-Path $root 'comparison.json') -Raw | ConvertFrom-Json
    $referencePath = Join-Path $root 'frozen-loopback-reference.json'
    $reference = Get-Content -LiteralPath $referencePath -Raw | ConvertFrom-Json
    $environment = Get-Content -LiteralPath (Join-Path $root 'input-environment.json') -Raw | ConvertFrom-Json
    $rttPath = Join-Path $root 'ssh-rtt-reference.json'
    $rtt = Get-Content -LiteralPath $rttPath -Raw | ConvertFrom-Json
    $outer = Get-Content -LiteralPath (Join-Path $directory 'report.json') -Raw | ConvertFrom-Json
    $samples = Get-Content -LiteralPath (Join-Path $root 'samples.json') -Raw | ConvertFrom-Json
    if (-not $complete.completed -or $complete.retainedSamples -ne 16 -or -not $comparison.feasibilityPassed -or
        -not $reference.frozenBeforeLibrarySample -or $reference.network -ne $item.name -or $rtt.samplesMs.Count -ne 20 -or
        $environment.cipher -ne 'aes256-gcm@openssh.com' -or $environment.kex -ne 'curve25519-sha256' -or $environment.compression -or $reference.policySha256 -ne $priorPolicyHash -or
        $environment.connectionMethod -ne 'Fresh data SSH connection per sample for both implementations; one shared idle control session performs unmeasured fixture cleanup.' -or
        -not $outer.passed -or -not $outer.cleanedUp -or -not $outer.networkCleaned -or $samples.Count -ne 16 -or
        @($samples | Where-Object { -not $_.hashMatched -or $_.bytes -ne $reference.bytes -or $_.sha256 -ne $environment.sha256 }).Count -ne 0 -or
        @($comparison.comparisons | Where-Object { -not $_.passed -or $_.minimumRatio -ne $policy.largeFileMinimumWinScpThroughputRatio }).Count -ne 0) {
        throw "Reference identity, feasibility or premeasurement policy mismatch: $($item.name)"
    }
    foreach ($direction in @('upload', 'download')) {
        foreach ($implementation in @('winscp', 'russh-sftp-3.0.1-library-pipeline')) {
            $group = @($samples | Where-Object { $_.direction -eq $direction -and $_.implementation -eq $implementation })
            if ($group.Count -ne 4 -or (@($group.round | Sort-Object) -join ',') -ne '0,1,2,3' -or
                @($group | Where-Object { $_.warmup -ne ($_.round -eq 0) -or $_.transferMs -le 0 -or $_.totalMs -lt $_.transferMs }).Count) {
                throw "Missing or invalid measured/warmup samples: $($item.name), $implementation, $direction"
            }
        }
        if (@($comparison.comparisons | Where-Object direction -eq $direction).Count -ne 1) { throw 'Missing direction comparison.' }
    }
    if (($item.name -eq 'loopback' -and $reference.bytes -ne $policy.payloadBytes) -or
        [long]$reference.bytes -lt 64MB -or $comparison.comparisons.Count -ne 2 -or $outer.performanceAcceptance) {
        throw 'Invalid feasibility payload or acceptance scope.'
    }
    $configuration = @($outer.serverVersion, $environment.os, $environment.powerScheme, $environment.rust,
        $environment.runtime, $environment.kex, $environment.cipher, $environment.compression) -join '|'
    $referenceIdentities.Add([pscustomobject]@{ winScp=$environment.winScpSha256; probe=$environment.probeSha256; requests=$environment.requestsPerFile; configuration=$configuration })
    $byNetwork[$item.name] = [ordered]@{
        minimumUploadBytesPerSecond = $reference.limits.upload.minimumBytesPerSecond
        minimumDownloadBytesPerSecond = $reference.limits.download.minimumBytesPerSecond
        referencePayloadBytes = $reference.bytes
        referenceSha256 = (Get-FileHash -LiteralPath $referencePath).Hash
        referencePath = $referencePath.Substring($repository.Length + 1).Replace('\','/')
        libraryRequestsPerFile = $environment.requestsPerFile
        scope = 'Absolute engineering targets derived only from WinSCP medians. Full product comparisons still require fresh same-condition 1 GiB and 3-worker references.'
    }
    $mixed[$item.name] = [ordered]@{
        maximumInputP95Ms = [Math]::Max([double]$m1.maximum_mixed_input_p95_ms, 2.0 * [double]$rtt.p95Ms + 50.0)
        sshReferenceP95Ms = $rtt.p95Ms
        sshReferenceSha256 = (Get-FileHash -LiteralPath $rttPath).Hash
        formula = 'max(frozen M1 mixed input p95 limit, unloaded authenticated SSH echo p95 * 2 + 50 ms)'
        scope = 'New same-connection three-transfer mixed load only. Does not replace or loosen any M1 regression limit; SSH echo reference excludes WebView rendering.'
    }
}
if (@($referenceIdentities.winScp | Select-Object -Unique).Count -ne 1 -or
    @($referenceIdentities.probe | Select-Object -Unique).Count -ne 1 -or
    @($referenceIdentities.configuration | Select-Object -Unique).Count -ne 1 -or
    @($referenceIdentities.requests | Select-Object -Unique).Count -ne 1) { throw 'Use the same fixed binaries and window configuration in both reference scenarios.' }
$policy.status = 'frozen_before_product_candidate'
$policy.absoluteLimits = [ordered]@{
    byNetwork = $byNetwork
    maximumPeakProcessTreeWorkingSetBytes = $m1.maximum_peak_working_set_bytes
    maximumPeakProcessTreeHandles = $m1.maximum_peak_handles
    resourceBasis = 'Unchanged frozen M1 whole-process-tree limits; record CPU separately without inventing a historical CPU threshold.'
}
$policy.mixedTerminalLoadLimits = [ordered]@{
    byNetwork = $mixed
    maximumLocalInputP95Ms = $m1.maximum_local_input_p95_ms
    maximumSlowAckSiblingInputP95Ms = $m1.maximum_isolation_input_p95_ms
    m1FrozenLimitsSha256 = (Get-FileHash -LiteralPath $m1Path).Hash
    m1LimitsUnchanged = $true
}
$policy | Add-Member -NotePropertyName frozenAtUtc -NotePropertyValue ([DateTime]::UtcNow.ToString('o'))
$policy | Add-Member -NotePropertyName preregisteredPolicySha256 -NotePropertyValue $priorPolicyHash
$requests = [int]$referenceIdentities[0].requests
$bufferBytes = [long]([Math]::Ceiling((3.0 * $requests * 32768 * 2 + 4MB) / 4MB) * 4MB)
$policy | Add-Member -NotePropertyName transferResourceLimits -NotePropertyValue ([ordered]@{
    requestPayloadBytes = 32768; requestsPerFile = $requests; globalTransferBufferBytes = $bufferBytes
    globalWorkers = 3; queuedTasks = 1024; retainedCompletedRows = 128; protocolInflight = (3 * $requests + 16); protocolWaiting = 128; remoteHandles = 64
    recursiveDepth = 64; directoryPageEntries = 256; localUnusedGrants = 128; localGrantLifetimeSeconds = 300
    requestTimeoutSeconds = 15; cancellationTargetSeconds = 5; exitDrainTimeoutSeconds = 30
    progressIntervalMs = 100; snapshotPollingMs = 250
    oscHintBytes = 8192; oscExplicitAliases = 8; oscNavigationIntervalMs = 250
    editorFileBytes = 8MB; editorSessions = 8; editorPollIntervalMs = 500; editorStableIntervalMs = 1000
    bufferBasis = 'Two copies per maximum file window across three workers, plus 4 MiB headroom rounded to 4 MiB. Payload admission is separate from SSH/wire/OS buffers; whole-process-tree memory is independently bounded.'
    cancellationBoundary = 'Protocol paths are bounded; blocking native I/O must finish safely and may exceed the target, with exit timeout keeping the window open.'
})
[IO.File]::WriteAllText($policyPath, (($policy | ConvertTo-Json -Depth 12) + "`n"), [Text.UTF8Encoding]::new($false))
Write-Output 'M2 absolute/resource/mixed-load limits frozen from fixed references before product candidate measurements; relative targets and M1 limits are unchanged.'
