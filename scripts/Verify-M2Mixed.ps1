param(
    [Parameter(Mandatory=$true)][string]$RoundDirectory,
    [ValidateSet('loopback','controlled_high_rtt')][string]$NetworkScenario='loopback',
    [switch]$Smoke
)
$ErrorActionPreference='Stop'
$directory=(Resolve-Path -LiteralPath $RoundDirectory).Path
$repository=(Resolve-Path -LiteralPath (Join-Path $PSScriptRoot '..')).Path
$policyPath=Join-Path $repository 'docs/agent/baselines/M2/performance-policy.json'
$policy=Get-Content -LiteralPath $policyPath -Raw -Encoding UTF8|ConvertFrom-Json
$m1Policy=Get-Content -LiteralPath (Join-Path $repository 'docs/agent/baselines/M1/performance-policy.json') -Raw -Encoding UTF8|ConvertFrom-Json
$report=Get-Content -LiteralPath (Join-Path $directory 'terminal-report.json') -Raw -Encoding UTF8|ConvertFrom-Json
$samples=Get-Content -LiteralPath (Join-Path $directory 'process-samples.json') -Raw -Encoding UTF8|ConvertFrom-Json
$environment=Get-Content -LiteralPath (Join-Path $directory 'environment.json') -Raw -Encoding UTF8|ConvertFrom-Json
$inputEnvironment=Get-Content -LiteralPath (Join-Path (Split-Path $directory -Parent) 'input-environment.json') -Raw -Encoding UTF8|ConvertFrom-Json
$runs=@();$failure=$null;$peakWorkingSet=0;$peakHandles=0
function P95([double[]]$Values){if($Values.Count -eq 0 -or @($Values|Where-Object {[double]::IsNaN($_) -or [double]::IsInfinity($_) -or $_ -lt 0 -or $_ -gt 3600000}).Count){throw 'Invalid measured response samples.'};$ordered=@($Values|Sort-Object);return $ordered[[Math]::Max(0,[Math]::Ceiling($ordered.Count*.95)-1)]}
try {
    if(-not $Smoke -and $report.diagnostic_mode){throw 'Diagnostic tracing is excluded from formal performance evidence.'}
    if($inputEnvironment.policySha256 -ine (Get-FileHash -LiteralPath $policyPath).Hash -or $inputEnvironment.network -cne $NetworkScenario -or $inputEnvironment.executableSha256 -ine $environment.executable_sha256 -or $inputEnvironment.fixtureSha256 -ine $environment.fixture_sha256){throw 'The visible workload does not match its registered policy or executable identities.'}
    if($report.measurement_mode -cne 'm2_mixed' -or -not $report.window_visible -or -not $environment.requested_visible -or $report.ssh_pool_before_disconnect -ne 1 -or $report.ssh_pool_after_disconnect -ne 0 -or @($report.startup_failure_codes).Count -ne 0 -or @($report.receiver.runs).Count -ne 2 -or @($report.transfer_loads).Count -ne 2){throw 'The report is not a complete visible mixed execution.'}
    if($report.visibility_samples.total -eq 0 -or $report.visibility_samples.visible -ne $report.visibility_samples.total){throw 'The measurement window was hidden during the workload.'}
    if($report.visibility_samples.focused/$report.visibility_samples.total -lt $m1Policy.invariants.minimum_focused_sample_ratio){throw 'The visible workload did not meet the unchanged M1 foreground requirement.'}
    foreach($index in 0..1){
        $run=$report.receiver.runs[$index];$load=$report.transfer_loads[$index];$direction=@('upload','download')[$index]
        if($run.failure -or $run.renderer_peak -ne 1 -or $run.renderers_after -ne 0 -or $run.registered_after -ne 0 -or @($run.terminals).Count -ne 3 -or $load.direction -cne $direction -or [bool]$load.smoke -ne [bool]$Smoke -or $load.round -ne $run.round -or @($load.taskIds|Select-Object -Unique).Count -ne 3 -or @($load.hashes).Count -ne 3 -or $load.activeJobsAfter -ne 0 -or $load.finalOwnedPayloadAdmissionBytes -ne 0 -or $load.peakOwnedPayloadAdmissionBytes -gt $policy.transferResourceLimits.globalTransferBufferBytes){throw 'An incomplete transfer, renderer or task cleanup result was reported.'}
        $expectedBytes=if($Smoke){67108864}else{1073741824}
        if($load.confirmedAllRunningChecks -lt 47){throw 'Input probes lack before/after proof that all three production jobs were Running.'}
        if([long]$load.bytesPerFile -ne $expectedBytes -or @($load.hashes|Select-Object -Unique).Count -ne 1 -or $load.hashes[0] -notmatch '^[0-9a-f]{64}$' -or $load.hashes[0] -ine $inputEnvironment.sha256){throw 'The measured transfer size or full hashes are incomplete.'}
        $local=@();$ssh=@()
        foreach($terminal in $run.terminals){
            $source=$report.sources.PSObject.Properties[$terminal.id].Value
            if(-not $source -or -not $source.digest.eof -or $source.digest.bytes -ne $terminal.bytes -or $source.digest.checksum -ne $terminal.checksum -or -not $terminal.raw_binary -or $terminal.parsed_bytes -ne $terminal.bytes -or $terminal.acked_bytes -ne $terminal.bytes -or $source.snapshot.metrics.input_bytes -ne $terminal.bytes -or $source.snapshot.metrics.output_bytes -ne $terminal.bytes -or $source.snapshot.metrics.max_outstanding_bytes -gt (1MB+64KB)){throw 'Raw terminal byte/ACK/source integrity did not converge.'}
            if($source.snapshot.state -cne 'exited' -or $source.snapshot.exit_code -ne 0 -or $source.snapshot.metrics.frame_count -ne $terminal.frames -or $source.snapshot.metrics.in_flight_bytes -ne 0){throw 'Terminal lifecycle/frame cleanup did not converge.'}
            if($source.backend -eq 'local'){$local+=@($terminal.input_response_ms)}
            else {
                if($source.backend -cne 'ssh' -or $source.connection_id -cne $load.connectionId -or $source.algorithms.cipher -cne 'aes256-gcm@openssh.com' -or $source.algorithms.kex -cne 'curve25519-sha256'){throw 'The transfer and measured shells do not share the pinned connection.'}
                $ssh+=@($terminal.input_response_ms)
            }
        }
        $bulkSource=$report.sources.PSObject.Properties[$run.terminals[1].id].Value
        if($bulkSource.fixture_generated_bytes -ne 100MB -or $bulkSource.snapshot.metrics.pause_count -lt 1 -or $bulkSource.snapshot.metrics.resume_count -lt 1){throw 'The same-connection held-ACK fixture did not exercise backpressure and convergence.'}
        if($local.Count -lt 11 -or $ssh.Count -lt 12 -or @($run.isolation_input_response_ms).Count -ne 10){throw 'Required visible input/isolation probes are missing.'}
        $localP95=P95 $local;$sshP95=P95 $ssh;$isolationP95=P95 $run.isolation_input_response_ms
        $passed=($localP95 -le $policy.mixedTerminalLoadLimits.maximumLocalInputP95Ms -and $sshP95 -le $policy.mixedTerminalLoadLimits.byNetwork.$NetworkScenario.maximumInputP95Ms -and $isolationP95 -le $policy.mixedTerminalLoadLimits.maximumSlowAckSiblingInputP95Ms)
        $runs+=@{direction=$direction;round=$load.round;localInputP95Ms=$localP95;sshInputP95Ms=$sshP95;heldAckSiblingInputP95Ms=$isolationP95;withinFrozenLatencyLimits=$passed;connectionMatched=$true;hashes=$load.hashes}
    }
    foreach($sample in $samples){
        if(@($sample.processes|Where-Object {$_.unmeasured}).Count){throw 'A process in the owned application tree was not measured.'}
        $workingSet=($sample.processes|Measure-Object working_set_bytes -Sum).Sum;$handles=($sample.processes|Measure-Object handles -Sum).Sum
        $peakWorkingSet=[Math]::Max($peakWorkingSet,$workingSet);$peakHandles=[Math]::Max($peakHandles,$handles)
    }
    if(@($samples).Count -lt 3 -or $peakWorkingSet -le 0 -or $peakWorkingSet -gt $policy.absoluteLimits.maximumPeakProcessTreeWorkingSetBytes -or $peakHandles -gt $policy.absoluteLimits.maximumPeakProcessTreeHandles){throw 'The unchanged M1 process-tree resource bounds were exceeded or unmeasured.'}
} catch {$failure=$_.Exception.Message} finally {
    [IO.File]::WriteAllText((Join-Path $directory 'm2-verification.json'),([ordered]@{schemaVersion=1;passed=($null -eq $failure);failure=$failure;network=$NetworkScenario;smoke=[bool]$Smoke;runs=$runs;peakWorkingSetBytes=$peakWorkingSet;peakHandles=$peakHandles;policySha256=(Get-FileHash -LiteralPath $policyPath).Hash;scope='Per-round integrity/resources and latency metrics; formal latency combines three measured rounds, excludes warmup; no physical keyboard/GPU or Explorer/editor acceptance';performanceAcceptance=$false;desktopAcceptance=$false}|ConvertTo-Json -Depth 12),[Text.UTF8Encoding]::new($false))
}
if($failure){throw $failure}
