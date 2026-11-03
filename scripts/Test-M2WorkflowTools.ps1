param([Parameter(Mandatory=$true)][string]$EvidenceDirectory)
$ErrorActionPreference='Stop'
$repository=(Resolve-Path -LiteralPath (Join-Path $PSScriptRoot '..')).Path
$evidence=[IO.Path]::GetFullPath($EvidenceDirectory)
if(Test-Path -LiteralPath $evidence){throw 'Preserve every workflow-tools test attempt.'}
New-Item -ItemType Directory -Path $evidence | Out-Null
$utf8=[Text.UTF8Encoding]::new($false)
$workers=[Collections.Generic.List[object]]::new()
$checks=[Collections.Generic.List[string]]::new()
$failure=$null
function Save-Json([string]$Path,$Value){[IO.File]::WriteAllText($Path,(ConvertTo-Json -InputObject $Value -Depth 10),$utf8)}
function Encoded([string]$Source){return '-NoProfile -NonInteractive -EncodedCommand '+[Convert]::ToBase64String([Text.Encoding]::Unicode.GetBytes($Source))}
try {
    foreach($script in @('Build-M2Measurement.ps1','M2OwnedProcess.ps1','M2NetworkRelay.ps1','Measure-M2SshWindow.ps1','Measure-M2SftpLibrary.ps1','Measure-M2Workflow.ps1','Invoke-M2WinScpJob.ps1','Test-M2SftpProbe.ps1','Test-M2HeadlessMixed.ps1','Measure-M2Mixed.ps1','Verify-M2Mixed.ps1','Measure-M0.ps1','Measure-M1.ps1')){
        $parseTokens=$null;$parseErrors=$null
        $source=[Management.Automation.Language.Parser]::ParseFile((Join-Path $PSScriptRoot $script),[ref]$parseTokens,[ref]$parseErrors)
        if(@($parseErrors).Count){throw "Invalid PowerShell syntax: $script"}
        if($script -eq 'M2OwnedProcess.ps1'){
            # Exercise the shipped functions without creating a network fixture.
            foreach($name in @('Start-OwnedMatrix','Stop-OwnedMatrix','Wait-OwnedMatrix')){
                $definition=$source.Find({param($node) $node -is [Management.Automation.Language.FunctionDefinitionAst] -and $node.Name -eq $name},$true)
                if(-not $definition){throw 'The owned measurement process helper is missing.'}
                . ([scriptblock]::Create($definition.Extent.Text))
            }
        }
        if($script -eq 'Measure-M2Mixed.ps1'){
            $definition=$source.Find({param($node) $node -is [Management.Automation.Language.FunctionDefinitionAst] -and $node.Name -eq 'Assert-M2MeasurementBuild'},$true)
            if(-not $definition){throw 'The measurement entry identity check is missing.'}
            . ([scriptblock]::Create($definition.Extent.Text))
        }
        if($script -eq 'Measure-M0.ps1'){
            $definition=$source.Find({param($node) $node -is [Management.Automation.Language.FunctionDefinitionAst] -and $node.Name -eq 'Get-M0ProcessResourceSample'},$true)
            if(-not $definition){throw 'The owned resource sampling helper is missing.'}
            . ([scriptblock]::Create($definition.Extent.Text))
        }
        if($script -eq 'Measure-M2SftpLibrary.ps1'){
            $definition=$source.Find({param($node) $node -is [Management.Automation.Language.FunctionDefinitionAst] -and $node.Name -eq 'Invoke-MeasurementProbe'},$true)
            if(-not $definition){throw 'The single-file probe wrapper is missing.'}
            . ([scriptblock]::Create($definition.Extent.Text))
        }
        if($script -eq 'Measure-M2Workflow.ps1'){
            $definition=$source.Find({param($node) $node -is [Management.Automation.Language.FunctionDefinitionAst] -and $node.Name -eq 'Get-MatrixSampleDeadline'},$true)
            if(-not $definition){throw 'The workload-specific helper deadline is missing.'}
            . ([scriptblock]::Create($definition.Extent.Text))
        }
    }
    $checks.Add('All workflow and visible mixed scripts parse on the measurement PowerShell runtime.')
    . (Join-Path $PSScriptRoot 'M2NetworkRelay.ps1')
    foreach($scenario in @('loopback','controlled_high_rtt')) {
        foreach($relay in @('wsl_localhost','windows_private')) {
            $profile = Get-M2NetworkProfile $scenario $relay
            $delay = if($scenario -eq 'controlled_high_rtt'){50}else{0}
            if($profile.oneWayDelayMs -ne $delay -or $profile.expectedRttMs -ne (2*$delay) -or $profile.useNamespace -ne ($delay -gt 0 -or $relay -eq 'windows_private')) { throw 'Incorrect explicit network route or injected delay.' }
        }
    }
    $rejected=$false;try{Get-M2NetworkProfile 'unknown' 'windows_private'|Out-Null}catch{$rejected=$true}
    if(-not $rejected){throw 'Unknown network profile accepted.'}
    $checks.Add('Explicit private-address low-RTT control uses zero injected delay; default routes and high-RTT injection remain unchanged and unknown profiles reject.')
    $birth = [DateTime]::UtcNow
    $entry = [pscustomobject]@{ProcessId=12345;Name='synthetic-process'}
    $sampleProcess = [pscustomobject]@{SafeHandle=$null;StartTime=$birth;HasExited=$false;ProcessName='synthetic-process';WorkingSet64=1234;PrivateMemorySize64=2345;HandleCount=17;TotalProcessorTime=[TimeSpan]::FromMilliseconds(9)}
    $sampleProcess | Add-Member ScriptProperty MainModule { throw [InvalidOperationException]::new('Synthetic version metadata race') }
    $lookup = {param($Id) $sampleProcess}.GetNewClosure()
    $row = Get-M0ProcessResourceSample $entry $birth.Ticks $lookup
    if($row.unmeasured -or $row.departed -or $row.working_set_bytes -ne 1234 -or -not $row.version_unavailable){throw 'Optional version metadata discarded valid resource counters.'}
    $sampleProcess | Add-Member ScriptProperty WorkingSet64 { throw [InvalidOperationException]::new('Synthetic resource read failure') } -Force
    $row = Get-M0ProcessResourceSample $entry $birth.Ticks $lookup
    if(-not $row.unmeasured -or $row.departed){throw 'An unmeasured live process escaped resource rejection.'}
    $sampleProcess.HasExited=$true
    $row = Get-M0ProcessResourceSample $entry $birth.Ticks $lookup
    if(-not $row.departed -or $row.unmeasured){throw 'A confirmed process exit was mistaken for missing live resource data.'}
    $sampleProcess.HasExited=$false
    $row = Get-M0ProcessResourceSample $entry ($birth.Ticks-1) $lookup
    if($row.departed -ne 'identity_changed'){throw 'A reused process identity was sampled.'}
    $row = Get-M0ProcessResourceSample $entry $birth.Ticks {param($Id) throw [ArgumentException]::new('Synthetic process exited')}
    if(-not $row.departed){throw 'A missing exited process was not classified.'}
    $checks.Add('Owned resource sampling preserves counters across version races, records confirmed departures/PID reuse separately, and still rejects unmeasured live processes.')
    foreach($mode in @('files','tree','resume')){
        if((Get-MatrixSampleDeadline $false $mode) -ne 2400){throw 'A formal matrix deadline changed.'}
        $expected=if($mode -eq 'tree'){600}else{180}
        if((Get-MatrixSampleDeadline $true $mode) -ne $expected){throw 'Incorrect smoke workload deadline.'}
    }
    $checks.Add('All formal matrices retain their 2400-second helper bound; only the full 128-file tree smoke has a separate bounded deadline.')
    & python -m unittest discover -s (Join-Path $repository 'scripts') -p test_m2_net_proxy.py
    if($LASTEXITCODE -ne 0){throw 'The fixture relay transport/lifecycle regressions failed.'}
    $checks.Add('Fixture relay preserves full duplex and half-close, isolates a slow reader, bounds admission and joins owned shutdown.')
    & python -m unittest discover -s (Join-Path $repository 'scripts') -p test_m1_sshd_cleanup.py
    if($LASTEXITCODE -ne 0){throw 'The disposable SSH fixture cleanup identity regressions failed.'}
    $checks.Add('Disposable sshd cleanup rejects other fixtures/users and PID reuse, and signals only retained owned process handles.')
    $identityDirectory=Join-Path $evidence 'measurement-identity-fixture'
    New-Item -ItemType Directory -Path $identityDirectory|Out-Null
    $identityExecutable=Join-Path $identityDirectory 'Capstan.exe'
    $identityFixture=Join-Path $identityDirectory 'capstan-m0-fixture.exe'
    [IO.File]::WriteAllText($identityExecutable,'synthetic identity only; never executed',$utf8)
    [IO.File]::WriteAllText($identityFixture,'synthetic fixture identity only; never executed',$utf8)
    $rejected=$false;try{Assert-M2MeasurementBuild $identityDirectory}catch{$rejected=$true}
    if(-not $rejected){throw 'A build without an entry manifest was accepted.'}
    $identity=[ordered]@{schemaVersion=1;purpose='M2-B-quantitative-measurement';instrumentation=$true;candidate=$false;benchmarkUrl='index.html#/m0-benchmark';benchmarkConfigSha256=(Get-FileHash -LiteralPath (Join-Path $repository 'src-tauri/tauri.m0-benchmark.conf.json')).Hash;executableSha256=(Get-FileHash -LiteralPath $identityExecutable).Hash;fixtureSha256=(Get-FileHash -LiteralPath $identityFixture).Hash}
    $identityPath=Join-Path $identityDirectory 'm2-measurement.json'
    Save-Json $identityPath $identity;Assert-M2MeasurementBuild $identityDirectory
    [IO.File]::AppendAllText($identityExecutable,' changed',$utf8)
    $rejected=$false;try{Assert-M2MeasurementBuild $identityDirectory}catch{$rejected=$true}
    if(-not $rejected){throw 'A replaced measurement executable was accepted.'}
    $identity.executableSha256=(Get-FileHash -LiteralPath $identityExecutable).Hash
    $identity.benchmarkConfigSha256='0'*64;Save-Json $identityPath $identity
    $rejected=$false;try{Assert-M2MeasurementBuild $identityDirectory}catch{$rejected=$true}
    if(-not $rejected){throw 'A different entry configuration was accepted.'}
    $checks.Add('Synthetic manifests accept matching identities and reject missing manifests, replaced binaries and changed benchmark entry configuration.')
    $executable=Join-Path $PSHOME 'powershell.exe'
    $worker=Start-OwnedMatrix $executable (Encoded 'Write-Output "nonzero-preserved"; exit 7') (Join-Path $evidence 'nonzero.txt') ''
    $rejected=$false
    try{Wait-OwnedMatrix @($worker) 10|Out-Null}catch{$rejected=$_.Exception.Message.Contains('Matrix worker failed')}
    $record=Get-Content -LiteralPath "$($worker.log).execution.json" -Raw -Encoding UTF8|ConvertFrom-Json
    if(-not $rejected -or $record.exitCode -ne 7 -or $record.timedOut -or -not ([IO.File]::ReadAllText($worker.log)).Contains('nonzero-preserved')){throw 'Nonzero worker exit/output was not preserved.'}
    $checks.Add('Nonzero child exit and output are preserved before the measurement rejects it.')
    $pending=@(foreach($index in 0..2){Start-OwnedMatrix $executable (Encoded "Write-Output 'worker-$index'; Start-Sleep -Milliseconds 100; exit 0") (Join-Path $evidence "concurrent-$index.txt") ''})
    $results=@(Wait-OwnedMatrix $pending 10)
    if($results.Count -ne 3){throw 'Concurrent workers were lost.'}
    foreach($index in 0..2){if(-not $results[$index].text.Contains("worker-$index")){throw 'Concurrent output was assigned to a different worker.'}}
    $checks.Add('Three workers start before waiting, and retain independent output and status.')
    $worker=Start-OwnedMatrix $executable (Encoded '$record=@{p95Ms=104.5;padding=("x"*512)};[Console]::Error.WriteLine("SSH_RTT_SAMPLE "+($record|ConvertTo-Json -Compress));exit 0') (Join-Path $evidence 'stderr-json.txt') ''
    Wait-OwnedMatrix @($worker) 10|Out-Null
    $raw=[IO.File]::ReadAllText($worker.log)
    $match=[regex]::Match($raw,'(?m)^SSH_RTT_SAMPLE (\{[^\r\n]*\})\r?$')
    if(-not $match.Success -or ($match.Groups[1].Value|ConvertFrom-Json).padding.Length -ne 512){throw 'Native stderr JSON was wrapped or lost.'}
    $checks.Add('A long native stderr JSON line remains intact without PowerShell error-record formatting.')
    $childFile=Join-Path $evidence 'owned-child.json'
    $childProgram=[Convert]::ToBase64String([Text.Encoding]::Unicode.GetBytes('Start-Sleep -Seconds 60'))
    $childFileLiteral="'"+$childFile.Replace("'","''")+"'"
    $timeoutProgram=@"
`$child=Start-Process -FilePath '$executable' -WindowStyle Hidden -ArgumentList '-NoProfile -NonInteractive -EncodedCommand $childProgram' -PassThru
@{id=`$child.Id;started=`$child.StartTime.ToUniversalTime().ToString('o')}|ConvertTo-Json|Set-Content -LiteralPath $childFileLiteral -Encoding utf8
Write-Output 'timeout-before-stop'
Start-Sleep -Seconds 60
"@
    $worker=Start-OwnedMatrix $executable (Encoded $timeoutProgram) (Join-Path $evidence 'timeout.txt') ''
    $rejected=$false
    try{Wait-OwnedMatrix @($worker) 3|Out-Null}catch{$rejected=$_.Exception.Message.Contains('exceeded its wall-clock deadline')}
    $record=Get-Content -LiteralPath "$($worker.log).execution.json" -Raw -Encoding UTF8|ConvertFrom-Json
    if(-not $rejected -or -not $record.timedOut -or $record.elapsedMs -ge 13000 -or -not ([IO.File]::ReadAllText($worker.log)).Contains('timeout-before-stop')){throw 'Owned timeout status/output was not preserved.'}
    $child=Get-Content -LiteralPath $childFile -Raw -Encoding UTF8|ConvertFrom-Json
    $remaining=Get-Process -Id $child.id -ErrorAction SilentlyContinue
    if($remaining -and $remaining.StartTime.ToUniversalTime().ToString('o') -eq $child.started){throw 'The owned child tree survived its deadline.'}
    $checks.Add('Timeout stops only the retained owned process tree, preserves output and records timedOut.')
    $single=Invoke-MeasurementProbe $executable @('-NoProfile','-NonInteractive','-EncodedCommand',([Convert]::ToBase64String([Text.Encoding]::Unicode.GetBytes('Write-Output "single-file-wrapper";exit 0')))) (Join-Path $evidence 'single-file-success.txt') 10
    if($single.exitCode -ne 0 -or -not ([IO.File]::ReadAllText((Join-Path $evidence 'single-file-success.txt.stdout'))).Contains('single-file-wrapper')){throw 'The single-file wrapper lost success output.'}
    $rejected=$false
    try{Invoke-MeasurementProbe $executable @('-NoProfile','-NonInteractive','-EncodedCommand',([Convert]::ToBase64String([Text.Encoding]::Unicode.GetBytes('Write-Output "single-file-deadline";Start-Sleep -Seconds 60')))) (Join-Path $evidence 'single-file-timeout.txt') 1|Out-Null}catch{$rejected=$_.Exception.Message.Contains('exceeded its wall-clock deadline')}
    $record=Get-Content (Join-Path $evidence 'single-file-timeout.txt.execution.json') -Raw|ConvertFrom-Json
    if(-not $rejected -or -not $record.timedOut -or -not $record.processExited -or -not $record.outputDrained -or $record.cleanupError){throw 'The single-file timeout lost its primary failure or failed to terminate.'}
    $checks.Add('The shipped single-file probe wrapper shares owned-tree deadlines and preserves output and the primary timeout.')
    $worker=Start-OwnedMatrix $executable (Encoded 'Write-Output "exit-race";exit 0') (Join-Path $evidence 'exit-race.txt') ''
    if(-not $worker.process.WaitForExit(10000)){throw 'The exit-race fixture did not finish.'}
    Stop-OwnedMatrix $worker
    $results=@(Wait-OwnedMatrix @($worker) 30)
    if($results.Count -ne 1 -or -not $results[0].text.Contains('exit-race')){throw 'An exited owned process lost its output during cleanup.'}
    $checks.Add('Cleanup after natural child exit preserves its success/output without signalling a stale PID.')
    $failedOutput=[Threading.Tasks.TaskCompletionSource[string]]::new()
    $failedOutput.SetException([IO.IOException]::new('synthetic read failure'))
    $emptyOutput=[Threading.Tasks.TaskCompletionSource[string]]::new();$emptyOutput.SetResult('')
    $finished=[pscustomobject]@{ExitCode=0}
    $finished|Add-Member ScriptMethod WaitForExit {param($milliseconds) return $true}
    $expired=[Diagnostics.Stopwatch]::StartNew();$expired.Stop()
    $broken=[pscustomobject]@{process=$finished;clock=$expired;log=(Join-Path $evidence 'failed-output.txt');output=$failedOutput.Task;errors=$emptyOutput.Task}
    $rejected=$false
    try{Wait-OwnedMatrix @($broken) 0|Out-Null}catch{$rejected=$_.Exception.Message.Contains('Owned matrix cleanup failed')}
    $record=Get-Content "$($broken.log).execution.json" -Raw|ConvertFrom-Json
    if(-not $rejected -or -not $record.timedOut -or $record.outputDrained -or -not $record.cleanupError){throw 'A faulted output task hid its deadline or bypassed the structured cleanup record.'}
    $checks.Add('A faulted output reader preserves a structured deadline/cleanup failure instead of masking it with a task exception.')
    $cancellationPath=Join-Path $evidence 'cancel.request'
    $readyPath=Join-Path $evidence 'cancel-ready.txt'
    $readyLiteral="'"+$readyPath.Replace("'","''")+"'"
    $cancelProgram="Write-Output 'cancel-before-stop';[IO.File]::WriteAllText($readyLiteral,'ready');Start-Sleep -Seconds 60"
    $worker=Start-OwnedMatrix $executable (Encoded $cancelProgram) (Join-Path $evidence 'cancelled.txt') ''
    $readyDeadline=[Diagnostics.Stopwatch]::StartNew()
    while(-not (Test-Path -LiteralPath $readyPath -PathType Leaf)){
        if($worker.process.HasExited -or $readyDeadline.Elapsed.TotalSeconds -ge 5){throw 'Cancellation worker did not become ready.'}
        Start-Sleep -Milliseconds 25
    }
    [IO.File]::WriteAllText($cancellationPath,'cancel this owned fixture only',$utf8)
    $rejected=$false
    try{Wait-OwnedMatrix @($worker) 30 $cancellationPath|Out-Null}catch{$rejected=$_.Exception.Message.Contains('measurement cancelled')}
    $record=Get-Content -LiteralPath "$($worker.log).execution.json" -Raw -Encoding UTF8|ConvertFrom-Json
    if(-not $rejected -or -not $record.cancelled -or $record.timedOut -or $record.elapsedMs -ge 13000 -or -not $worker.process.HasExited -or -not ([IO.File]::ReadAllText($worker.log)).Contains('cancel-before-stop')){throw 'Explicit cancellation did not stop the owned worker and preserve its result.'}
    $checks.Add('An explicit cancellation request stops the owned worker promptly and preserves output separately from timeout.')
    $previous=$env:CAPSTAN_M2_MATRIX_WORKER_CONFIG
    try{
        $env:CAPSTAN_M2_MATRIX_WORKER_CONFIG=Join-Path $evidence 'untrusted-config.json'
        [IO.File]::WriteAllText($env:CAPSTAN_M2_MATRIX_WORKER_CONFIG,'{}',$utf8)
        $helper=Join-Path $PSScriptRoot 'Invoke-M2WinScpJob.ps1'
        $worker=Start-OwnedMatrix $executable ('-NoProfile -NonInteractive -File "'+$helper+'" -Index 0') (Join-Path $evidence 'scope-rejection.txt') ''
        $rejected=$false
        try{Wait-OwnedMatrix @($worker) 10|Out-Null}catch{$rejected=$_.Exception.Message.Contains('Matrix worker failed')}
        if(-not $rejected -or -not ([IO.File]::ReadAllText($worker.log)).Contains('WIN_MATRIX_JOB_FAILED')){throw 'The reference worker accepted a config outside the private fixture root.'}
    } finally {[Environment]::SetEnvironmentVariable('CAPSTAN_M2_MATRIX_WORKER_CONFIG',$previous,'Process')}
    $checks.Add('WinSCP rejects configuration outside the disposable private root before opening a connection.')
    & (Join-Path $PSScriptRoot 'Test-M2MixedVerifier.ps1') -EvidenceDirectory (Join-Path $evidence 'mixed-verifier')
    $checks.Add('Synthetic positive and adversarial mixed reports exercise every required evidence boundary.')
} catch {$failure=$_.Exception.Message} finally {
    foreach($worker in $workers){try{Stop-OwnedMatrix $worker}finally{$worker.process.Dispose()}}
    Save-Json (Join-Path $evidence 'report.json') ([ordered]@{checks=$checks.ToArray();passed=($null -eq $failure);failure=$failure;scope='Workflow measurement tooling self-check; no product performance or desktop acceptance';performanceAcceptance=$false;desktopAcceptance=$false})
}
if($failure){throw $failure}
Write-Output "Workflow measurement tooling: $($checks.Count) checks passed."
