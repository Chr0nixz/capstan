param([Parameter(Mandatory = $true)][string]$BaselineDirectory, [Parameter(Mandatory = $true)][string]$OutputPath, [string]$PreviousFrozenPolicy)
$ErrorActionPreference = 'Stop'
if (Test-Path -LiteralPath $OutputPath) { throw 'Never overwrite frozen limits.' }
$policyPath = Join-Path $PSScriptRoot '../docs/agent/baselines/M1/performance-policy.json'
$policy = Get-Content -LiteralPath $policyPath -Raw | ConvertFrom-Json
$reportPath = Join-Path $BaselineDirectory 'terminal-report.json'
$report = Get-Content -LiteralPath $reportPath -Raw | ConvertFrom-Json
$samples = Get-Content -LiteralPath (Join-Path $BaselineDirectory 'process-samples.json') -Raw | ConvertFrom-Json
function Get-P95($Values) { $sorted = @($Values | Sort-Object); if ($sorted.Count -eq 0) { throw 'No input samples.' }; return $sorted[[Math]::Ceiling($sorted.Count * 0.95) - 1] }
$bulk = @($report.receiver.runs | Where-Object kind -eq 'bulk')[0].terminals[0]
if ($bulk.bulk_elapsed_ms -le 0) { throw 'Baseline has no timed bulk output.' }
$inputP95 = Get-P95 @($report.receiver.runs | ForEach-Object { $_.input_response_ms })
$peakMemory = ($samples | ForEach-Object { ($_.processes | Measure-Object working_set_bytes -Sum).Sum } | Measure-Object -Maximum).Maximum
$peakHandles = ($samples | ForEach-Object { ($_.processes | Measure-Object handles -Sum).Sum } | Measure-Object -Maximum).Maximum
$value = [ordered]@{
    schema = 1; frozen_at_utc = [DateTime]::UtcNow.ToString('o')
    policy_sha256 = (Get-FileHash -LiteralPath $policyPath).Hash; baseline_report_sha256 = (Get-FileHash -LiteralPath $reportPath).Hash
    baseline_mib_per_second = (100000.0 / $bulk.bulk_elapsed_ms); baseline_input_p95_ms = $inputP95
    baseline_peak_working_set_bytes = $peakMemory; baseline_peak_handles = $peakHandles
    minimum_local_mib_per_second = (100000.0 / $bulk.bulk_elapsed_ms * $policy.regression_limits.local_bulk_minimum_throughput_ratio)
    maximum_local_input_p95_ms = [Math]::Max($inputP95 * 2, $inputP95 + 50)
    maximum_mixed_input_p95_ms = [Math]::Max($inputP95 * 3, $inputP95 + 100)
    maximum_peak_working_set_bytes = ($peakMemory * 1.75 + 128MB)
    maximum_peak_handles = ($peakHandles * 2 + 128)
    maximum_isolation_input_p95_ms = $policy.regression_limits.normal_shell_during_slow_ack_p95_ms
    invariants = $policy.invariants
}
if ($PreviousFrozenPolicy) {
    $previous=Get-Content -LiteralPath $PreviousFrozenPolicy -Raw|ConvertFrom-Json
    if ($previous.policy_sha256 -ne $value.policy_sha256) { throw 'The premeasurement policy changed.' }
    foreach ($name in @('maximum_local_input_p95_ms','maximum_mixed_input_p95_ms','maximum_peak_working_set_bytes','maximum_peak_handles','maximum_isolation_input_p95_ms')) {
        $value[$name]=$previous.$name
    }
    $value.minimum_local_mib_per_second=$previous.minimum_local_mib_per_second
    $value.previous_frozen_policy_sha256=(Get-FileHash -LiteralPath $PreviousFrozenPolicy).Hash
}
[IO.File]::WriteAllText([IO.Path]::GetFullPath($OutputPath), ($value | ConvertTo-Json -Depth 7) + "`n", [Text.UTF8Encoding]::new($false))
Write-Output 'M1 regression limits frozen from the completed local reference; no mixed run has started.'
