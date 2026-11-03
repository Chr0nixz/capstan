param(
    [Parameter(Mandatory=$true)][string]$EvidenceDirectory,
    [Parameter(Mandatory=$true)][string]$Workspace,
    [Parameter(Mandatory=$true)][ValidateSet('files','tree','resume')][string]$Mode,
    [ValidateSet('loopback','controlled_high_rtt')][string]$NetworkScenario='loopback',
    [ValidateSet(64,1024)][int]$ProbeMiB=1024,
    [string]$WinScpDirectory='C:/Program Files (x86)/WinSCP',
    [switch]$Smoke
)
$ErrorActionPreference='Stop'
$repository=(Resolve-Path -LiteralPath (Join-Path $PSScriptRoot '..')).Path
$workspaceRoot=(Resolve-Path -LiteralPath $Workspace).Path
$targetPrefix=(Join-Path $repository 'target').TrimEnd('\')+'\'
if(-not $workspaceRoot.StartsWith($targetPrefix,[StringComparison]::OrdinalIgnoreCase) -or [IO.Path]::GetFileName($workspaceRoot) -notmatch '^m1-sshd-[0-9a-f]{12}$'){throw 'Use the disposable fixture workspace.'}
$token=[IO.Path]::GetFileName($workspaceRoot).Substring(8)
if($env:CAPSTAN_TEST_SFTP_ROOT -cne "/var/tmp/capstan-m1-ssh-$token/home" -or $env:CAPSTAN_TEST_SSHD -notmatch "^cm1$token@127\.0\.0\.1:([0-9]+)$"){throw 'Run through the isolated SFTP probe runner.'}
$port=[int]$Matches[1]
$evidence=[IO.Path]::GetFullPath($EvidenceDirectory)
if(Test-Path -LiteralPath $evidence){throw 'Preserve every prior matrix attempt; use a new evidence directory.'}
if(-not $Smoke -and $ProbeMiB -ne 1024){throw 'Formal workflow measurements require exact 1 GiB inputs.'}
New-Item -ItemType Directory -Path $evidence | Out-Null
$utf8=[Text.UTF8Encoding]::new($false)
$policyPath=Join-Path $repository 'docs/agent/baselines/M2/performance-policy.json'
$policyHash=(Get-FileHash -LiteralPath $policyPath).Hash
$policy=Get-Content -LiteralPath $policyPath -Raw | ConvertFrom-Json
if($policy.status -ne 'frozen_before_product_candidate' -or $policy.warmupRuns -ne 1 -or $policy.measuredRuns -ne 3 -or $policy.transferResourceLimits.globalWorkers -ne 3){throw 'Frozen matrix policy is required.'}
Copy-Item -LiteralPath $policyPath -Destination (Join-Path $evidence 'premeasurement-policy.json')
$samples=[Collections.Generic.List[object]]::new()
$workers=[Collections.Generic.List[object]]::new()
function Get-MatrixSampleDeadline([bool]$Smoke,[string]$Mode) {
    if(-not $Smoke){return 2400}
    # Tree smoke still transfers and independently hashes all 128 files. Its
    # many metadata round trips can exceed a large-file smoke's wall deadline.
    # This bounds helper lifetime only; the frozen 1.5 duration gate is unchanged.
    if($Mode -eq 'tree'){return 600}
    return 180
}
$sampleDeadline=Get-MatrixSampleDeadline ([bool]$Smoke) $Mode
$cancellationPath=Join-Path $evidence 'cancel.request'
$failure=$null; $session=$null; $priorExecutionState=[uint32]0; $awakeReleased=$false
$variables=@('CAPSTAN_TEST_TRANSFER_MATRIX','CAPSTAN_TEST_MATRIX_MODE','CAPSTAN_TEST_MATRIX_CUT_PERCENT','CAPSTAN_TEST_SFTP_INPUT','CAPSTAN_TEST_SFTP_SHA256','CAPSTAN_TEST_SFTP_DIRECTION','CAPSTAN_TEST_SFTP_PERFORMANCE')
$previous=@{};foreach($variable in $variables){$previous[$variable]=[Environment]::GetEnvironmentVariable($variable,'Process')}
function Save-Json([string]$Path,$Value){[IO.File]::WriteAllText($Path,(ConvertTo-Json -InputObject $Value -Depth 14),$utf8)}
. (Join-Path $PSScriptRoot 'M2OwnedProcess.ps1')
function Remove-OwnedFile([string]$Name){
    if($Name -notmatch '^(matrix-returned-[0-2]|resume-returned)\.bin$'){throw 'Invalid matrix output basename.'}
    $path=Join-Path $workspaceRoot $Name
    if(Test-Path -LiteralPath $path){Remove-Item -LiteralPath $path -Force}
}
function Reset-TreeOutput([ValidateSet('tree-output','reference-tree-output')][string]$Name='tree-output') {
    $path=Join-Path $workspaceRoot $Name
    if(Test-Path -LiteralPath $path){
        $resolved=(Resolve-Path -LiteralPath $path).Path
        if($resolved -cne $path -or -not $resolved.StartsWith($workspaceRoot.TrimEnd('\')+'\',[StringComparison]::OrdinalIgnoreCase) -or ((Get-Item -LiteralPath $resolved -Force).Attributes -band [IO.FileAttributes]::ReparsePoint)){throw 'Invalid owned output-tree cleanup path.'}
        Remove-Item -LiteralPath $resolved -Recurse -Force
    }
    New-Item -ItemType Directory -Path $path | Out-Null
}
function New-Options {
    $options=New-Object WinSCP.SessionOptions
    $options.Protocol=[WinSCP.Protocol]::Sftp;$options.HostName='127.0.0.1';$options.PortNumber=$port;$options.UserName="cm1$token"
    $options.Password=([IO.File]::ReadAllText($env:CAPSTAN_TEST_SSH_PASSWORD_FILE)).TrimEnd("`r","`n")
    $options.SshHostKeyFingerprint=$fingerprint;$options.AddRawSettings('Compression','0');return $options
}
Push-Location $repository
try {
    if(-not ('Capstan.M2WorkflowPower' -as [type])){
        Add-Type -TypeDefinition @'
using System.Runtime.InteropServices;
namespace Capstan { public static class M2WorkflowPower { [DllImport("kernel32.dll")] public static extern uint SetThreadExecutionState(uint flags); } }
'@
    }
    $priorExecutionState=[Capstan.M2WorkflowPower]::SetThreadExecutionState([uint32]2147483649)
    if($priorExecutionState -eq 0){throw 'Could not hold the measurement helper sleep requirement.'}
    $ErrorActionPreference='Continue'
    & cargo test -p capstan-core --release --test sftp_workflow --features benchmarks --offline --locked --no-run *> (Join-Path $evidence 'build.txt')
    $buildCode=$LASTEXITCODE;$ErrorActionPreference='Stop'
    if($buildCode -ne 0){throw 'The production matrix probe did not build.'}
    $probe=@(Get-ChildItem (Join-Path $repository 'target/release/deps/sftp_workflow-*.exe')|Sort-Object LastWriteTimeUtc -Descending)[0].FullName
    $ErrorActionPreference='Continue'
    & cargo test -p capstan-ssh --release --test sftp_performance --offline --locked --no-run *> (Join-Path $evidence 'rtt-build.txt')
    $rttBuildCode=$LASTEXITCODE;$ErrorActionPreference='Stop'
    if($rttBuildCode -ne 0){throw 'The authenticated SSH RTT probe did not build.'}
    $rttProbe=@(Get-ChildItem (Join-Path $repository 'target/release/deps/sftp_performance-*.exe')|Sort-Object LastWriteTimeUtc -Descending)[0].FullName
    $inputFile=Join-Path $workspaceRoot 'payload.bin'
    $random=[Random]::new(20261017);$buffer=[byte[]]::new(1MB)
    $file=[IO.File]::Open($inputFile,[IO.FileMode]::CreateNew,[IO.FileAccess]::Write,[IO.FileShare]::None)
    try{for($index=0;$index -lt $ProbeMiB;$index++){$random.NextBytes($buffer);$file.Write($buffer,0,$buffer.Length)}}finally{$file.Dispose()}
    $hash=(Get-FileHash -LiteralPath $inputFile).Hash
    $tree=$null
    if($Mode -eq 'tree'){
        # Construct Unicode independently of the Windows PowerShell source encoding.
        $unicodeGroup=([char]0x4E2D).ToString()+[char]0x6587+' space'
        $unicodeEmpty=([char]0x7A7A).ToString()+' '+[char]0x76EE+[char]0x5F55
        $directories=@('matrix-tree','matrix-tree/empty',"matrix-tree/$unicodeEmpty",'matrix-tree/nested','matrix-tree/nested/empty')
        $directories+=@(0..7|ForEach-Object{if($_ -eq 0){"matrix-tree/$unicodeGroup"}else{"matrix-tree/group-$_"}})
        foreach($directory in $directories){New-Item -ItemType Directory -Path (Join-Path $workspaceRoot $directory)|Out-Null}
        $files=@();$treeRandom=[Random]::new(20261020);$sizes=@(0,1,1024,32768,65536,131072)
        for($index=0;$index -lt 128;$index++){
            $group=if($index%8 -eq 0){$unicodeGroup}else{"group-$($index%8)"}
            $name=if($index%16 -eq 0){".hidden-$index.bin"}else{"file $index.bin"}
            $relative="matrix-tree/$group/$name";$bytes=[byte[]]::new($sizes[$index%6]);$treeRandom.NextBytes($bytes)
            $path=Join-Path $workspaceRoot $relative;[IO.File]::WriteAllBytes($path,$bytes)
            $files+=@{path=$relative;bytes=$bytes.Length;sha256=(Get-FileHash -LiteralPath $path).Hash}
        }
        $tree=[ordered]@{directories=$directories;files=$files};Save-Json (Join-Path $workspaceRoot 'matrix-tree-manifest.json') $tree
        Copy-Item -LiteralPath (Join-Path $workspaceRoot 'matrix-tree-manifest.json') -Destination (Join-Path $evidence 'tree-manifest.json')
        Reset-TreeOutput
    }
    Add-Type -Path (Join-Path $WinScpDirectory 'WinSCPnet.dll')
    $publicKey=& ssh-keygen -l -E sha256 -f $env:CAPSTAN_TEST_SSH_HOST_KEY_FILE
    if($LASTEXITCODE -ne 0 -or ($publicKey -split '\s+')[1] -notmatch '^SHA256:([A-Za-z0-9+/]{43})$'){throw 'Invalid pinned fixture key.'}
    $fingerprint="ssh-ed25519 255 $($Matches[1])"
    $session=New-Object WinSCP.Session;$session.ExecutablePath=Join-Path $WinScpDirectory 'WinSCP.exe';$session.SessionLogPath=Join-Path $evidence 'control.log'
    $session.Open((New-Options))
    $options=New-Object WinSCP.TransferOptions;$options.TransferMode=[WinSCP.TransferMode]::Binary;$options.PreserveTimestamp=$false;$options.ResumeSupport.State=[WinSCP.TransferResumeSupportState]::Off
    $environment=[ordered]@{schemaVersion=1;workload=$Mode;network=$NetworkScenario;perFileBytes=([long]$ProbeMiB*1MB).ToString();seed=20261017;sha256=$hash;policySha256=$policyHash;probeSha256=(Get-FileHash -LiteralPath $probe).Hash;winScpVersion=(Get-Item (Join-Path $WinScpDirectory 'WinSCP.exe')).VersionInfo.ProductVersion;winScpSha256=(Get-FileHash -LiteralPath (Join-Path $WinScpDirectory 'WinSCP.exe')).Hash;os=[Environment]::OSVersion.VersionString;powerScheme=(& powercfg /getactivescheme)-join ' ';requestsPerFile=64;productWorkers=3;productDataConnections=1;referenceDataConnections=3;idleControlSession='same unmeasured cleanup session in both phases';sampleDeadlineSeconds=$sampleDeadline;smoke=[bool]$Smoke;performanceAcceptance=$false;desktopAcceptance=$false}
    Save-Json (Join-Path $evidence 'input-environment.json') $environment
    $rttLog=Join-Path $evidence 'ssh-rtt-reference.txt'
    $env:CAPSTAN_TEST_SFTP_PERFORMANCE='1'
    $rttWorker=Start-OwnedMatrix $rttProbe 'configured_openssh_shell_rtt_reference --exact --nocapture' $rttLog ''
    Wait-OwnedMatrix @($rttWorker) 60 | Out-Null
    $rttRecord=[regex]::Match(([IO.File]::ReadAllText($rttLog)),'SSH_RTT_SAMPLE (\{[^\r\n]*\})')
    if(-not $rttRecord.Success){throw 'The matrix requires an actual authenticated SSH RTT reference.'}
    Save-Json (Join-Path $evidence 'ssh-rtt-reference.json') ($rttRecord.Groups[1].Value|ConvertFrom-Json)
    $env:CAPSTAN_TEST_TRANSFER_MATRIX='1';$env:CAPSTAN_TEST_MATRIX_MODE=$Mode;$env:CAPSTAN_TEST_SFTP_INPUT=$inputFile;$env:CAPSTAN_TEST_SFTP_SHA256=$hash
    if($Mode -eq 'resume'){
        $session.PutFiles($inputFile,"$env:CAPSTAN_TEST_SFTP_ROOT/resume-source.bin",$false,$options).Check()
        foreach($direction in @('upload','download')){foreach($cut in @(20,50,85)){
            if(Test-Path -LiteralPath $cancellationPath -PathType Leaf){throw 'Owned matrix measurement cancelled before the next interruption case.'}
            if(@(Get-Process -Name cargo,rustc,clippy-driver -ErrorAction SilentlyContinue).Count){throw 'No compilers may run during samples.'}
            $name="resume-$direction-$cut";Write-Output "M2 matrix: $name"
            if($direction -eq 'upload' -and $session.FileExists("$env:CAPSTAN_TEST_SFTP_ROOT/resume-target.bin")){$session.RemoveFile("$env:CAPSTAN_TEST_SFTP_ROOT/resume-target.bin")|Out-Null}
            Remove-OwnedFile 'resume-returned.bin';$env:CAPSTAN_TEST_SFTP_DIRECTION=$direction;$env:CAPSTAN_TEST_MATRIX_CUT_PERCENT=$cut.ToString()
            $worker=Start-OwnedMatrix $probe 'performance::matrix::configured_openssh_transfer_matrix --exact --nocapture' (Join-Path $evidence "$name.txt") ''
            $execution=@(Wait-OwnedMatrix @($worker) $sampleDeadline $cancellationPath)[0]
            $match=[regex]::Match($execution.text,'(?m)^MATRIX_SAMPLE (\{[^\r\n]*\})\r?$');if(-not $match.Success){throw 'Matrix resume has no execution record.'}
            $sample=$match.Groups[1].Value|ConvertFrom-Json
            if(-not $sample.hashMatched -or -not $sample.partialGoneAfterCommit -or -not $sample.newGenerationDifferent -or $sample.tcpConnections -ne 2){throw 'Incomplete real resume proof.'}
            $samples.Add($sample);Save-Json (Join-Path $evidence 'samples.json') $samples.ToArray()
        }}
    } else {
        $rounds=if($Smoke){0}else{3}
        foreach($candidate in @('winscp','product')){
            if($candidate -eq 'product' -and -not $Smoke){Save-Json (Join-Path $evidence 'frozen-reference.json') ([ordered]@{policySha256=$policyHash;frozenBeforeProductSamples=$true;scope='Three active files, same workload and per-file bytes';referenceSamples=@($samples|Where-Object{-not $_.warmup})})}
            for($round=0;$round -le $rounds;$round++){foreach($direction in @('upload','download')){
                if(@(Get-Process -Name cargo,rustc,clippy-driver -ErrorAction SilentlyContinue).Count){throw 'No compilers may run during samples.'}
                $name="$candidate-$round-$direction";Write-Output "M2 matrix: $Mode $name"
                if(Test-Path -LiteralPath $cancellationPath -PathType Leaf){throw 'Owned matrix measurement cancelled before the next sample.'}
                if($Mode -eq 'files'){
                    foreach($index in 0..2){
                        if($direction -eq 'upload'){$remote=if($candidate -eq 'winscp'){"$env:CAPSTAN_TEST_SFTP_ROOT/reference-file-$index.bin"}else{"$env:CAPSTAN_TEST_SFTP_ROOT/matrix-file-$index.bin"};if($session.FileExists($remote)){$session.RemoveFile($remote)|Out-Null}}
                        else{Remove-OwnedFile "matrix-returned-$index.bin"}
                    }
                } elseif($direction -eq 'upload'){
                    # Only this fixture's exact generated tree root is ever removed.
                    if($session.FileExists("$env:CAPSTAN_TEST_SFTP_ROOT/matrix-tree")){$session.RemoveFiles("$env:CAPSTAN_TEST_SFTP_ROOT/matrix-tree/").Check()}
                } elseif($candidate -eq 'winscp') {Reset-TreeOutput 'reference-tree-output'} else {Reset-TreeOutput}
                # Fixture reset is outside both timings; reference directory
                # creation remains preparation, just as in the product plan.
                $timer=[Diagnostics.Stopwatch]::StartNew();$startTicks=[Diagnostics.Stopwatch]::GetTimestamp()
                if($candidate -eq 'winscp'){
                    if($Mode -eq 'tree' -and $direction -eq 'upload'){foreach($directory in $tree.directories){$session.CreateDirectory("$env:CAPSTAN_TEST_SFTP_ROOT/$directory")}}
                    if($Mode -eq 'tree' -and $direction -eq 'download'){foreach($directory in $tree.directories){New-Item -ItemType Directory -Path (Join-Path $workspaceRoot "reference-tree-output/$directory")|Out-Null}}
                    $config=Join-Path $workspaceRoot ('secrets/matrix-'+[Guid]::NewGuid().ToString('N')+'.json')
                    Save-Json $config ([ordered]@{mode=$Mode;direction=$direction;sample=$name;remoteRoot=$env:CAPSTAN_TEST_SFTP_ROOT;username="cm1$token";port=$port;fingerprint=$fingerprint;winScpDirectory=$WinScpDirectory;payloadBytes=([long]$ProbeMiB*1MB).ToString();payloadSha256=$hash;evidence=$evidence})
                    $scriptPath=Join-Path $PSScriptRoot 'Invoke-M2WinScpJob.ps1';if($scriptPath.Contains('"')){throw 'Invalid fixed helper path.'}
                    $pending=@(foreach($index in 0..2){Start-OwnedMatrix (Join-Path $PSHOME 'powershell.exe') ('-NoProfile -NonInteractive -File "'+$scriptPath+'" -Index '+$index) (Join-Path $evidence "$name-worker-$index.txt") $config})
                    $executions=@(Wait-OwnedMatrix $pending $sampleDeadline $cancellationPath);$jobs=@()
                    foreach($execution in $executions){$match=[regex]::Match($execution.text,'(?m)^WIN_MATRIX_JOB (\{[^\r\n]*\})\r?$');if(-not $match.Success){throw 'WinSCP worker has no complete execution record.'};$jobs+=($match.Groups[1].Value|ConvertFrom-Json)}
                    $first=($jobs|ForEach-Object{[long]$_.dataStartTicks}|Measure-Object -Minimum).Minimum;$last=($jobs|ForEach-Object{[long]$_.dataEndTicks}|Measure-Object -Maximum).Maximum
                    $total=$timer.Elapsed.TotalMilliseconds
                    if(@($jobs|Where-Object{-not $_.hashMatched}).Count){throw 'WinSCP matrix hash failed.'}
                    foreach($directory in @($tree.directories)){
                        if($Mode -eq 'tree'){
                            $exists=if($direction -eq 'upload'){$session.FileExists("$env:CAPSTAN_TEST_SFTP_ROOT/$directory")}else{Test-Path -LiteralPath (Join-Path $workspaceRoot "reference-tree-output/$directory") -PathType Container}
                            if(-not $exists){throw 'WinSCP lost an empty directory.'}
                        }
                    }
                    $sample=[pscustomobject]@{implementation='winscp';workload=$Mode;direction=$direction;concurrency=3;files=($jobs|Measure-Object files -Sum).Sum;bytes=(($jobs|ForEach-Object{[long]$_.bytes}|Measure-Object -Sum).Sum).ToString('F0');prepareMs=([double]$first-$startTicks)/[Diagnostics.Stopwatch]::Frequency*1000;transferMs=([double]$last-$first)/[Diagnostics.Stopwatch]::Frequency*1000;totalMs=$total;hashMatched=$true;jobs=$jobs;emptyDirectoriesChecked=if($tree){$tree.directories.Count}else{0}}
                } else {
                    $env:CAPSTAN_TEST_SFTP_DIRECTION=$direction
                    $worker=Start-OwnedMatrix $probe 'performance::matrix::configured_openssh_transfer_matrix --exact --nocapture' (Join-Path $evidence "$name.txt") ''
                    $execution=@(Wait-OwnedMatrix @($worker) $sampleDeadline $cancellationPath)[0]
                    $match=[regex]::Match($execution.text,'(?m)^MATRIX_SAMPLE (\{[^\r\n]*\})\r?$');if(-not $match.Success){throw 'Product matrix has no execution record.'}
                    $sample=$match.Groups[1].Value|ConvertFrom-Json
                    if(-not $sample.hashMatched -or $sample.finalOwnedPayloadAdmissionBytes -ne 0 -or $sample.peakOwnedPayloadAdmissionBytes -gt $policy.transferResourceLimits.globalTransferBufferBytes){throw 'Incomplete product matrix integrity/admission evidence.'}
                    $extra=[Math]::Max(0.0,[double]$execution.elapsedMs-[double]$sample.wallMsIncludingIndependentOracle)
                    $sample|Add-Member helperOverheadMs $extra;$sample|Add-Member externalProcessMs ([double]$execution.elapsedMs)
                    $sample.totalMs=[double]$sample.totalMs+$extra;$sample.prepareMs=[double]$sample.prepareMs+$extra
                }
                $sample|Add-Member round $round;$sample|Add-Member warmup ($round -eq 0)
                $samples.Add($sample);Save-Json (Join-Path $evidence 'samples.json') $samples.ToArray()
            }}
        }
        if(-not $Smoke){
            $comparisons=@(foreach($direction in @('upload','download')){
                $win=@($samples|Where-Object{$_.implementation -eq 'winscp' -and $_.direction -eq $direction -and -not $_.warmup}|Sort-Object transferMs)
                $product=@($samples|Where-Object{$_.implementation -ne 'winscp' -and $_.direction -eq $direction -and -not $_.warmup}|Sort-Object transferMs)
                if($win.Count -ne 3 -or $product.Count -ne 3){throw 'Incomplete fixed rounds.'}
                $winTotal=@($win|Sort-Object totalMs);$productTotal=@($product|Sort-Object totalMs)
                $dataRatio=[double]$win[1].transferMs/[double]$product[1].transferMs;$durationRatio=[double]$productTotal[1].totalMs/[double]$winTotal[1].totalMs
                $absolute=if($direction -eq 'upload'){$policy.absoluteLimits.byNetwork.$NetworkScenario.minimumUploadBytesPerSecond}else{$policy.absoluteLimits.byNetwork.$NetworkScenario.minimumDownloadBytesPerSecond}
                $throughput=[double]$product[1].bytes/(([double]$product[1].transferMs+10)/1000)
                $passed=if($Mode -eq 'tree'){$durationRatio -le [double]$policy.smallTreeMaximumWinScpDurationRatio}else{$dataRatio -ge [double]$policy.largeFileMinimumWinScpThroughputRatio -and (1/$durationRatio) -ge [double]$policy.largeFileMinimumWinScpThroughputRatio -and $throughput -ge [double]$absolute}
                [ordered]@{direction=$direction;winScpMedianTransferMs=$win[1].transferMs;productMedianTransferMs=$product[1].transferMs;dataThroughputRatio=$dataRatio;winScpMedianTotalMs=$winTotal[1].totalMs;productMedianTotalMs=$productTotal[1].totalMs;totalDurationRatio=$durationRatio;productBytesPerSecondWithPhaseAllowance=$throughput;frozenAbsoluteMinimum=$absolute;passed=$passed}
            })
            Save-Json (Join-Path $evidence 'comparison.json') ([ordered]@{policySha256=$policyHash;workload=$Mode;network=$NetworkScenario;comparisons=$comparisons;passed=(@($comparisons|Where-Object{-not $_.passed}).Count -eq 0);performanceAcceptance=$false;desktopAcceptance=$false})
            if(@($comparisons|Where-Object{-not $_.passed}).Count){throw 'The workflow misses a frozen target. Preserve these samples; do not relax the target.'}
        }
    }
    if((Get-FileHash -LiteralPath $policyPath).Hash -ne $policyHash){throw 'Frozen policy changed during measurement.'}
} catch {$failure=$_.Exception.Message} finally {
    foreach($worker in $workers){
        try{Stop-OwnedMatrix $worker}catch{if(-not $failure){$failure='Owned process cleanup failed.'}}
        try{if(-not (Test-Path -LiteralPath $worker.log)){[IO.File]::WriteAllText($worker.log,$worker.output.GetAwaiter().GetResult()+"`n"+$worker.errors.GetAwaiter().GetResult(),$utf8)}}catch{}
        $worker.process.Dispose()
    }
    if($session){$session.Dispose()}
    if($priorExecutionState -ne 0){$awakeReleased=([Capstan.M2WorkflowPower]::SetThreadExecutionState($priorExecutionState) -ne 0)}
    foreach($variable in $variables){[Environment]::SetEnvironmentVariable($variable,$previous[$variable],'Process')}
    Pop-Location
    Save-Json (Join-Path $evidence 'report.json') ([ordered]@{schemaVersion=1;mode=$Mode;network=$NetworkScenario;perFileMiB=$ProbeMiB;smoke=[bool]$Smoke;sampleDeadlineSeconds=$sampleDeadline;retainedSamples=$samples.Count;passed=($null -eq $failure -and $awakeReleased);failure=$failure;temporaryAwakeRequestReleased=$awakeReleased;policySha256=$policyHash;performanceAcceptance=$false;desktopAcceptance=$false})
}
if($failure){throw $failure}
