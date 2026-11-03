param([Parameter(Mandatory = $true)][string]$BaselineDirectory, [Parameter(Mandatory = $true)][string]$MixedDirectory, [Parameter(Mandatory = $true)][string]$FrozenPolicy)
$ErrorActionPreference = 'Stop'
$policy = Get-Content -LiteralPath $FrozenPolicy -Raw | ConvertFrom-Json
$basePath = Join-Path $BaselineDirectory 'terminal-report.json'
& (Join-Path $PSScriptRoot 'Verify-M0Report.ps1') -ReportPath $basePath | Out-Null
& (Join-Path $PSScriptRoot 'Verify-M0Report.ps1') -ReportPath (Join-Path $MixedDirectory 'terminal-report.json') | Out-Null
if ((Get-FileHash -LiteralPath $basePath).Hash -ne $policy.baseline_report_sha256) { throw 'The local reference changed after freezing limits.' }
$report = Get-Content -LiteralPath (Join-Path $MixedDirectory 'terminal-report.json') -Raw | ConvertFrom-Json
$baseline = Get-Content -LiteralPath $basePath -Raw | ConvertFrom-Json
$environment = Get-Content -LiteralPath (Join-Path $MixedDirectory 'environment.json') -Raw | ConvertFrom-Json
$baseEnvironment = Get-Content -LiteralPath (Join-Path $BaselineDirectory 'environment.json') -Raw | ConvertFrom-Json
$samples = Get-Content -LiteralPath (Join-Path $MixedDirectory 'process-samples.json') -Raw | ConvertFrom-Json
$checks = [Collections.Generic.List[object]]::new()
function Check([string]$Name, [bool]$Passed) { $checks.Add([ordered]@{ name = $Name; passed = $Passed }) }
function Get-P95($Values) { $sorted = @($Values | Sort-Object); if ($sorted.Count -eq 0) { return [double]::PositiveInfinity }; return $sorted[[Math]::Ceiling($sorted.Count * 0.95) - 1] }
function Source($Terminal) { return $report.sources.PSObject.Properties[$Terminal.id].Value }
Check 'mode-and-same-binary' ($report.measurement_mode -eq 'm1_mixed' -and $environment.executable_sha256 -eq $baseEnvironment.executable_sha256)
Check 'same-viewport' (($report.receiver.viewport | ConvertTo-Json -Compress) -eq ($baseline.receiver.viewport | ConvertTo-Json -Compress))
foreach ($item in @($baseline, $report)) {
    Check ('visible-' + $item.measurement_mode) ($item.window_visible -and $item.visibility_samples.visible / $item.visibility_samples.total -ge $policy.invariants.minimum_visible_sample_ratio)
    Check ('foreground-' + $item.measurement_mode) ($item.visibility_samples.focused / $item.visibility_samples.total -ge $policy.invariants.minimum_focused_sample_ratio)
}
$localRates = @()
$localInputs = @()
foreach ($run in $report.receiver.runs | Where-Object kind -eq 'tab_lifecycle') {
    $local = @($run.terminals | Where-Object { (Source $_).backend -eq 'local' })
    $remote = @($run.terminals | Where-Object { (Source $_).backend -ne 'local' })
    $connections = @($remote | ForEach-Object { (Source $_).connection_id } | Sort-Object -Unique)
    $primary = @($remote | Where-Object { (Source $_).backend -eq 'ssh' } | ForEach-Object { (Source $_).connection_id } | Sort-Object -Unique)
    $alternate = @($remote | Where-Object { (Source $_).backend -eq 'ssh_alternate' } | ForEach-Object { (Source $_).connection_id } | Sort-Object -Unique)
    Check "mixed-topology-$($run.round)" ($local.Count -eq 4 -and $remote.Count -eq 16 -and $connections.Count -eq 2 -and $primary.Count -eq 1 -and $alternate.Count -eq 1 -and $primary[0] -ne $alternate[0])
    Check "bundled-runtime-$($run.round)" (@($local | Where-Object { (Source $_).runtime_source -ne 'Bundled' -or (Source $_).runtime_version -ne '1.25.260930003' }).Count -eq 0)
    Check "ssh-algorithms-$($run.round)" (@($remote | Where-Object { $null -eq (Source $_).algorithms -or [string]::IsNullOrEmpty((Source $_).algorithms.cipher) }).Count -eq 0)
    $bulk = @($local | Where-Object { (Source $_).fixture_generated_bytes -eq 100MB })[0]
    $remoteBulk = @($remote | Where-Object { (Source $_).fixture_generated_bytes -eq 100MB })
    Check "local-and-ssh-100mib-$($run.round)" ($null -ne $bulk -and $remoteBulk.Count -eq 1 -and $bulk.bulk_elapsed_ms -gt 0)
    $localRates += 100000.0 / $bulk.bulk_elapsed_ms
    # Probes are ordered by tab index, three per tab; local tab positions are fixed by the published plan.
    foreach ($index in @(0,5,10,15)) { $localInputs += $run.input_response_ms[($index * 3)..($index * 3 + 2)] }
}
$localMedian = @($localRates | Sort-Object)[1]
$isolatedLocal = @($report.receiver.runs | Where-Object kind -eq 'bulk')[0].terminals[0]
$isolatedLocalRate = 100000.0 / $isolatedLocal.bulk_elapsed_ms
$mixedP95 = Get-P95 @($report.receiver.runs | ForEach-Object { $_.input_response_ms })
$localP95 = Get-P95 $localInputs
$isolation = @($report.receiver.runs | Where-Object kind -eq 'delayed_ack')[0]
$isolationConnections = @($isolation.terminals | ForEach-Object { (Source $_).connection_id } | Sort-Object -Unique)
Check 'slow-ack-shared-transport' ($isolationConnections.Count -eq 1 -and $isolation.isolation_input_response_ms.Count -eq 3)
$isolationP95 = Get-P95 $isolation.isolation_input_response_ms
$peakMemory = ($samples | ForEach-Object { ($_.processes | Measure-Object working_set_bytes -Sum).Sum } | Measure-Object -Maximum).Maximum
$peakHandles = ($samples | ForEach-Object { ($_.processes | Measure-Object handles -Sum).Sum } | Measure-Object -Maximum).Maximum
Check 'local-throughput' ($isolatedLocalRate -ge $policy.minimum_local_mib_per_second)
Check 'local-input-p95' ($localP95 -le $policy.maximum_local_input_p95_ms)
Check 'mixed-input-p95' ($mixedP95 -le $policy.maximum_mixed_input_p95_ms)
Check 'slow-ack-neighbor-input-p95' ($isolationP95 -le $policy.maximum_isolation_input_p95_ms)
Check 'peak-process-tree-memory' ($peakMemory -le $policy.maximum_peak_working_set_bytes)
Check 'peak-process-tree-handles' ($peakHandles -le $policy.maximum_peak_handles)
Check 'pool-cleanup' ($report.ssh_pool_before_disconnect -eq 2 -and $report.ssh_pool_after_disconnect -eq 0)
Check 'startup-without-failure' (@($report.startup_failure_codes).Count -eq 0)
Check 'all-scenarios' ($report.receiver.runs.Count -eq 5 -and @($report.receiver.runs | Where-Object { $null -ne $_.failure -or $_.registered_after -ne 0 -or $_.renderers_after -ne 0 }).Count -eq 0)
$result = [ordered]@{
    schema = 1; measured_at_utc = [DateTime]::UtcNow.ToString('o'); passed = @($checks | Where-Object { !$_.passed }).Count -eq 0
    frozen_policy_sha256 = (Get-FileHash -LiteralPath $FrozenPolicy).Hash
    local_bulk_mib_per_second = $localRates; local_bulk_median_mib_per_second = $localMedian
    isolated_local_mib_per_second = $isolatedLocalRate
    local_input_p95_ms = $localP95; mixed_input_p95_ms = $mixedP95; isolation_input_p95_ms = $isolationP95
    peak_working_set_bytes = $peakMemory; peak_handles = $peakHandles; checks = $checks.ToArray()
}
$output = Join-Path (Split-Path $FrozenPolicy -Parent) 'comparison.json'
if (Test-Path -LiteralPath $output) { throw 'Comparison evidence already exists.' }
[IO.File]::WriteAllText($output, ($result | ConvertTo-Json -Depth 8) + "`n", [Text.UTF8Encoding]::new($false))
$checks | Format-Table -AutoSize
if (!$result.passed) { throw 'M1 performance comparison failed; retain the raw evidence and unchanged limits.' }
