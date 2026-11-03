param([Parameter(Mandatory=$true)][string]$EvidenceDirectory)
$ErrorActionPreference='Stop'
$evidence=[IO.Path]::GetFullPath($EvidenceDirectory)
if(Test-Path -LiteralPath $evidence){throw 'Preserve previous verifier self-checks.'}
New-Item -ItemType Directory -Path $evidence|Out-Null
$utf8=[Text.UTF8Encoding]::new($false)
function Save-Json([string]$Path,$Value){[IO.File]::WriteAllText($Path,(ConvertTo-Json -InputObject $Value -Depth 20),$utf8)}
$repository=(Resolve-Path -LiteralPath (Join-Path $PSScriptRoot '..')).Path
$policyHash=(Get-FileHash -LiteralPath (Join-Path $repository 'docs/agent/baselines/M2/performance-policy.json')).Hash
$parseTokens=$null;$parseErrors=$null
$source=[Management.Automation.Language.Parser]::ParseFile((Join-Path $PSScriptRoot 'Measure-M2Mixed.ps1'),[ref]$parseTokens,[ref]$parseErrors)
if(@($parseErrors).Count){throw 'Invalid mixed measurement source.'}
foreach($name in @('Get-M2P95','Get-M2LatencyComparison')){
    $definition=$source.Find({param($node) $node -is [Management.Automation.Language.FunctionDefinitionAst] -and $node.Name -eq $name},$true)
    if(-not $definition){throw 'The frozen latency comparator is missing.'}
    . ([scriptblock]::Create($definition.Extent.Text))
}
$connection=[Guid]::NewGuid().ToString();$hash='a'*64
$sources=@{};$runs=@();$loads=@()
foreach($index in 0..1){
    $terminals=@()
    foreach($position in 0..2){
        $id="fixture-$index-$position";$backend=if($position -eq 0){'local'}else{'ssh'}
        $responses=if($position -eq 1){@(10.0)}else{@(1..11|ForEach-Object {10.0})}
        $terminals+=@{id=$id;bytes=1024;checksum=123;frames=1;raw_binary=$true;parsed_bytes=1024;acked_bytes=1024;max_pending_bytes=1024;bulk_elapsed_ms=10;input_response_ms=$responses}
        $sources[$id]=@{backend=$backend;connection_id=$connection;algorithms=@{cipher='aes256-gcm@openssh.com';kex='curve25519-sha256'};digest=@{eof=$true;bytes=1024;checksum=123};snapshot=@{state='exited';exit_code=0;metrics=@{input_bytes=1024;output_bytes=1024;max_outstanding_bytes=1024;frame_count=1;in_flight_bytes=0;pause_count=1;resume_count=1}};fixture_generated_bytes=if($position -eq 1){100MB}else{0}}
    }
    $runs+=@{kind='tab_lifecycle';round=0;failure=$null;renderer_peak=1;renderers_after=0;registered_after=0;terminals=$terminals;input_response_ms=@(1..23|ForEach-Object {10.0});isolation_input_response_ms=@(1..10|ForEach-Object {10.0})}
    $loads+=@{id=[Guid]::NewGuid().ToString();round=0;direction=@('upload','download')[$index];connectionId=$connection;taskIds=@(0..2|ForEach-Object {[Guid]::NewGuid().ToString()});hashes=@($hash,$hash,$hash);bytesPerFile='67108864';activeJobsAfter=0;finalOwnedPayloadAdmissionBytes=0;peakOwnedPayloadAdmissionBytes=12MB;confirmedAllRunningChecks=47;smoke=$true}
}
$template=@{measurement_mode='m2_mixed';diagnostic_mode=$false;window_visible=$true;ssh_pool_before_disconnect=1;ssh_pool_after_disconnect=0;startup_failure_codes=@();visibility_samples=@{total=100;visible=100;focused=100};receiver=@{runs=$runs};sources=$sources;transfer_loads=$loads}|ConvertTo-Json -Depth 20
$checks=@()
foreach($case in @('positive','wrong-hash','wrong-connection','missing-running-proof','unacked-output','renderer-retained','excess-memory','slow-input','hidden-window','unfocused-window','failed-shell')){
    $caseRoot=Join-Path $evidence $case;$directory=Join-Path $caseRoot 'round-0';New-Item -ItemType Directory -Path $directory -Force|Out-Null
    $report=$template|ConvertFrom-Json
    $memory=512MB
    switch($case){
        'wrong-hash' {$report.transfer_loads[0].hashes=@(('b'*64),('b'*64),('b'*64))}
        'wrong-connection' {$report.transfer_loads[0].connectionId=[Guid]::NewGuid().ToString()}
        'missing-running-proof' {$report.transfer_loads[0].confirmedAllRunningChecks=46}
        'unacked-output' {$report.receiver.runs[0].terminals[0].acked_bytes=1023}
        'renderer-retained' {$report.receiver.runs[0].renderers_after=1}
        'excess-memory' {$memory=2GB}
        'slow-input' {$report.receiver.runs[0].terminals[0].input_response_ms=@(1..11|ForEach-Object {5000.0})}
        'hidden-window' {$report.window_visible=$false}
        'unfocused-window' {$report.visibility_samples.focused=79}
        'failed-shell' {$report.sources.'fixture-0-1'.snapshot.exit_code=1}
    }
    Save-Json (Join-Path $directory 'terminal-report.json') $report
    Save-Json (Join-Path $directory 'environment.json') @{requested_visible=$true;executable_sha256='C'*64;fixture_sha256='D'*64}
    Save-Json (Join-Path $caseRoot 'input-environment.json') @{network='loopback';sha256=$hash;policySha256=$policyHash;executableSha256='C'*64;fixtureSha256='D'*64}
    Save-Json (Join-Path $directory 'process-samples.json') @(0..2|ForEach-Object {@{elapsed_ms=$_*1000;processes=@(@{working_set_bytes=$memory;handles=100;cpu_ms=$_*20})}})
    $accepted=$true
    try{& (Join-Path $PSScriptRoot 'Verify-M2Mixed.ps1') -RoundDirectory $directory -NetworkScenario loopback -Smoke}catch{$accepted=$false}
    $expected=$case -in @('positive','slow-input');$checks+=@{case=$case;expectedAccepted=$expected;accepted=$accepted;passed=($accepted -eq $expected)}
    if($expected){
        $formal=@(foreach($round in 1..3){$copy=$report|ConvertTo-Json -Depth 20|ConvertFrom-Json;foreach($row in $copy.receiver.runs){$row.round=$round};foreach($load in $copy.transfer_loads){$load.round=$round};$copy})
        $comparisons=@(Get-M2LatencyComparison $formal 'loopback')
        $latencyAccepted=@($comparisons|Where-Object {-not $_.passed}).Count -eq 0
        $checks+=@{case="$case-three-round-latency";expectedAccepted=($case -eq 'positive');accepted=$latencyAccepted;passed=($latencyAccepted -eq ($case -eq 'positive'))}
    }
}
$rejected=$false
try{Get-M2LatencyComparison @($template|ConvertFrom-Json) 'loopback'|Out-Null}catch{$rejected=$true}
$checks+=@{case='missing-measured-rounds';expectedAccepted=$false;accepted=(-not $rejected);passed=$rejected}
$diagnosticReports=@(foreach($round in 1..3){$copy=$template|ConvertFrom-Json;$copy.diagnostic_mode=$true;foreach($row in $copy.receiver.runs){$row.round=$round};foreach($load in $copy.transfer_loads){$load.round=$round};$copy})
$rejected=$false
try{Get-M2LatencyComparison $diagnosticReports 'loopback'|Out-Null}catch{$rejected=$_.Exception.Message.Contains('Diagnostic tracing')}
$checks+=@{case='diagnostic-trace-cannot-enter-formal-timing';expectedAccepted=$false;accepted=(-not $rejected);passed=$rejected}
$passed=@($checks|Where-Object {-not $_.passed}).Count -eq 0
Save-Json (Join-Path $evidence 'report.json') @{checks=$checks;passed=$passed;scope='Synthetic validator self-tests; fabricated reports are not device measurements';performanceAcceptance=$false;desktopAcceptance=$false}
if(-not $passed){throw 'Mixed verifier accepted incomplete evidence or rejected its valid fixture.'}
Write-Output "Mixed verifier: $($checks.Count) synthetic boundary cases passed."
