param([Parameter(Mandatory = $true)][string]$ReportPath)
$ErrorActionPreference = 'Stop'
$report = Get-Content -LiteralPath $ReportPath -Raw | ConvertFrom-Json
if ($report.receiver.schema -ne 1 -or @($report.receiver.runs).Count -ne 5) { throw 'Incomplete or unknown benchmark report.' }
$required = @('bulk:0', 'delayed_ack:0', 'tab_lifecycle:1', 'tab_lifecycle:2', 'tab_lifecycle:3')
foreach ($run in $report.receiver.runs) {
    $key = "$($run.kind):$($run.round)"
    if ($required -notcontains $key) { throw 'Duplicate or unexpected benchmark scenario.' }
    $required = @($required | Where-Object { $_ -ne $key })
    $count = if ($run.kind -eq 'tab_lifecycle') { 20 } elseif ($run.kind -eq 'delayed_ack' -and $report.measurement_mode -eq 'm1_mixed') { 2 } else { 1 }
    if ($null -ne $run.failure -or @($run.terminals).Count -ne $count -or $run.registered_after -ne 0 -or $run.renderers_after -ne 0) { throw 'Scenario failed or resources remain.' }
    if ($run.renderer_peak -gt 1 -or @($run.input_response_ms).Count -lt ($count * 3)) { throw 'Incomplete renderer or input evidence.' }
    foreach ($terminal in $run.terminals) {
        $source = $report.sources.PSObject.Properties[$terminal.id].Value
        if ($null -eq $source -or $terminal.raw_binary -isnot [bool] -or !$terminal.raw_binary -or !$source.digest.eof -or $source.snapshot.state -ne 'exited' -or $source.snapshot.exit_code -ne 0) { throw 'Missing binary/EOF source evidence.' }
        if ($terminal.bytes -ne $source.digest.bytes -or $terminal.checksum -ne $source.digest.checksum -or $terminal.parsed_bytes -ne $terminal.bytes -or $terminal.acked_bytes -ne $terminal.bytes) { throw 'Byte integrity or ACK mismatch.' }
        $metrics = $source.snapshot.metrics
        if ($metrics.input_bytes -ne $terminal.bytes -or $metrics.output_bytes -ne $terminal.bytes -or $metrics.frame_count -ne $terminal.frames -or $metrics.in_flight_bytes -ne 0 -or $metrics.max_outstanding_bytes -gt (1MB + 64KB)) { throw 'Budget or frame contract failed.' }
        if ($run.kind -eq 'delayed_ack' -and $terminal.id -eq $run.terminals[0].id -and ($metrics.pause_count -lt 1 -or $metrics.resume_count -lt 1)) { throw 'Delayed ACK did not exercise pause and resume.' }
    }
    $firstSource = $report.sources.PSObject.Properties[$run.terminals[0].id].Value
    if ($firstSource.fixture_generated_bytes -ne 100MB) { throw 'The bulk fixture did not declare the required 100 MiB scenario.' }
    Write-Output "$key : binary integrity, input samples, budget and cleanup verified"
}
if (!$report.window_visible) { Write-Warning 'Window was hidden: this is functional evidence, not a visible device performance baseline.' }
Write-Output 'Fixed synthetic terminal scenarios verified; hardware and human interaction coverage is recorded separately.'
