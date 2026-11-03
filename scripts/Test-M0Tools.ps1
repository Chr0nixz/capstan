$ErrorActionPreference = 'Stop'
$directory = Join-Path $PSScriptRoot "../target/m0-tools-tests/$([Guid]::NewGuid().ToString('N'))"
New-Item -ItemType Directory -Path $directory -Force | Out-Null
$utf8 = [Text.UTF8Encoding]::new($false)
$sources = [ordered]@{}
$runs = @()
$sequence = 0
foreach ($kind in @('bulk', 'delayed_ack', 'tab_lifecycle:1', 'tab_lifecycle:2', 'tab_lifecycle:3')) {
    $parts = $kind.Split(':'); $terminals = @(); $count = if ($parts[0] -eq 'tab_lifecycle') { 20 } else { 1 }
    for ($index = 0; $index -lt $count; $index++) {
        $key = "session-$sequence"; $sequence++
        $terminals += [ordered]@{ id = $key; bytes = 4; checksum = 3716520713; frames = 1; raw_binary = $true; parsed_bytes = 4; acked_bytes = 4; max_pending_bytes = 4 }
        $sources[$key] = [ordered]@{ digest = @{ bytes = 4; checksum = 3716520713; eof = $true }; fixture_generated_bytes = $(if ($index -eq 0) { 100MB } else { 0 }); snapshot = @{
            state = 'exited'; exit_code = 0; metrics = @{ input_bytes = 4; output_bytes = 4; frame_count = 1; in_flight_bytes = 0; max_outstanding_bytes = 4; pause_count = 1; resume_count = 1 }
        } }
    }
    $runs += [ordered]@{ kind = $parts[0]; round = $(if ($parts.Length -gt 1) { [int]$parts[1] } else { 0 }); terminals = $terminals
        failure = $null; registered_after = 0; renderers_after = 0; renderer_peak = 1; input_response_ms = @(1..($count * 3)) }
}
$receipt = [ordered]@{ receiver = @{ schema = 1; runs = $runs }; sources = $sources; window_visible = $false }
function Write-Case([string]$name, $value) {
    $path = Join-Path $directory "$name.json"
    [IO.File]::WriteAllText($path, (ConvertTo-Json -InputObject $value -Depth 15), $utf8)
    return $path
}
$valid = Write-Case 'valid-contract' $receipt
& (Join-Path $PSScriptRoot 'Verify-M0Report.ps1') -ReportPath $valid | Out-Null
function Reject-Case([string]$name, $value) {
    $path = Write-Case $name $value
    $rejected = $false
    try { & (Join-Path $PSScriptRoot 'Verify-M0Report.ps1') -ReportPath $path | Out-Null } catch { $rejected = $true }
    if (!$rejected) { throw "Verifier incorrectly accepted $name." }
}
$receipt.receiver.runs[0].terminals[0].checksum = 0
Reject-Case 'checksum-mismatch' $receipt
$receipt.receiver.runs[0].terminals[0].checksum = 3716520713
$receipt.receiver.runs[0].registered_after = 1
Reject-Case 'session-leak' $receipt
$receipt.receiver.runs[0].registered_after = 0
$receipt.receiver.runs[0].terminals[0].raw_binary = 'false'
Reject-Case 'nonbinary-payload' $receipt
$receipt.receiver.runs[0].terminals[0].raw_binary = $true
$receipt.receiver.runs[1].terminals = @()
Reject-Case 'missing-delayed-ack-evidence' $receipt
$runtimeCase = Join-Path $directory 'invalid-runtime'
New-Item -ItemType Directory -Path $runtimeCase | Out-Null
$header = [byte[]]::new(64)
foreach ($case in @('dos-header', 'pe-offset')) {
    if ($case -eq 'pe-offset') {
        $header[0] = 0x4d; $header[1] = 0x5a
        [Buffer]::BlockCopy([BitConverter]::GetBytes([uint32]::MaxValue), 0, $header, 60, 4)
    }
    [IO.File]::WriteAllBytes((Join-Path $runtimeCase 'invalid.dll'), $header)
    $rejected = $false
    try { & (Join-Path $PSScriptRoot 'Inspect-ConPtyBundle.ps1') -RuntimeDirectory $runtimeCase | Out-Null } catch { $rejected = $true }
    if (!$rejected) { throw "Runtime inspector incorrectly accepted $case." }
}
foreach ($script in Get-ChildItem -LiteralPath $PSScriptRoot -Filter '*.ps1') {
    $tokens = $null; $errors = $null
    [Management.Automation.Language.Parser]::ParseFile($script.FullName, [ref]$tokens, [ref]$errors) | Out-Null
    if ($errors.Count -ne 0) { throw "PowerShell syntax failed: $($script.Name)" }
}
Write-Output 'M0 tools: valid contract + four report rejection cases + two malformed PE cases + PowerShell syntax passed. No device measurement was performed.'
