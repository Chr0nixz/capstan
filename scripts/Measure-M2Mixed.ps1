param(
    [Parameter(Mandatory=$true)][string]$EvidenceDirectory,
    [Parameter(Mandatory=$true)][string]$Workspace,
    [Parameter(Mandatory=$true)][string]$Executable,
    [ValidateSet('loopback','controlled_high_rtt')][string]$NetworkScenario='loopback',
    [ValidateSet(64,1024)][int]$ProbeMiB=1024,
    [switch]$Smoke,
    [switch]$DiagnosticTrace
)
$ErrorActionPreference='Stop'
if($DiagnosticTrace -and -not $Smoke){throw 'Diagnostic tracing is excluded from formal measurements.'}
$repository=(Resolve-Path -LiteralPath (Join-Path $PSScriptRoot '..')).Path
$workspaceRoot=(Resolve-Path -LiteralPath $Workspace).Path
$prefix=(Join-Path $repository 'target').TrimEnd('\')+'\'
if(-not $workspaceRoot.StartsWith($prefix,[StringComparison]::OrdinalIgnoreCase) -or [IO.Path]::GetFileName($workspaceRoot) -notmatch '^m1-sshd-[0-9a-f]{12}$'){throw 'Use the disposable fixture workspace.'}
$token=[IO.Path]::GetFileName($workspaceRoot).Substring(8)
if($env:CAPSTAN_TEST_SFTP_ROOT -cne "/var/tmp/capstan-m1-ssh-$token/home" -or $env:CAPSTAN_TEST_SSHD -notmatch "^cm1$token@127\.0\.0\.1:([0-9]+)$"){throw 'Use the isolated visible SFTP probe runner.'}
$port=[int]$Matches[1]
if(($Smoke -and $ProbeMiB -ne 64) -or (-not $Smoke -and $ProbeMiB -ne 1024)){throw 'Smoke uses 64 MiB; formal visible load uses exact 1 GiB files.'}
$application=(Resolve-Path -LiteralPath $Executable).Path
$applicationDirectory=Split-Path $application -Parent
function Assert-M2MeasurementBuild([string]$Directory){
    $manifestPath=Join-Path $Directory 'm2-measurement.json'
    if(-not (Test-Path -LiteralPath $manifestPath -PathType Leaf) -or (Get-Item -LiteralPath $manifestPath).Length -gt 32768){throw 'Use Build-M2Measurement.ps1 to bind the dedicated benchmark entry configuration.'}
    $manifest=Get-Content -LiteralPath $manifestPath -Raw -Encoding UTF8|ConvertFrom-Json
    if($manifest.schemaVersion -ne 1 -or $manifest.purpose -cne 'M2-B-quantitative-measurement' -or -not $manifest.instrumentation -or $manifest.candidate -or $manifest.benchmarkUrl -cne 'index.html#/m0-benchmark' -or $manifest.benchmarkConfigSha256 -ine (Get-FileHash -LiteralPath (Join-Path $repository 'src-tauri/tauri.m0-benchmark.conf.json')).Hash -or $manifest.executableSha256 -ine (Get-FileHash -LiteralPath (Join-Path $Directory 'Capstan.exe')).Hash -or $manifest.fixtureSha256 -ine (Get-FileHash -LiteralPath (Join-Path $Directory 'capstan-m0-fixture.exe')).Hash){throw 'The measurement entry configuration or binary identity does not match its manifest.'}
}
Assert-M2MeasurementBuild $applicationDirectory
if(-not (Test-Path -LiteralPath (Join-Path $applicationDirectory 'capstan.portable')) -or [IO.File]::ReadAllText((Join-Path $applicationDirectory 'capstan.portable')) -cne 'CAPSTAN_PORTABLE_V1' -or (Test-Path -LiteralPath (Join-Path $applicationDirectory 'data/vault.capstan'))){throw 'Use an isolated portable measurement directory without user data.'}
$capability=& $application --m0-check
if($LASTEXITCODE -ne 0 -or $capability -cne 'capstan-m0-benchmarks-v1'){throw 'Use the opt-in measurement build.'}
$evidence=[IO.Path]::GetFullPath($EvidenceDirectory)
if(Test-Path -LiteralPath $evidence){throw 'Preserve every earlier visible mixed attempt.'}
New-Item -ItemType Directory -Path $evidence|Out-Null
$utf8=[Text.UTF8Encoding]::new($false)
function Save-Json([string]$Path,$Value){[IO.File]::WriteAllText($Path,(ConvertTo-Json -InputObject $Value -Depth 12),$utf8)}
function Get-M2P95([double[]]$Values){
    if($Values.Count -eq 0 -or @($Values|Where-Object {[double]::IsNaN($_) -or [double]::IsInfinity($_) -or $_ -lt 0 -or $_ -gt 3600000}).Count){throw 'Invalid bounded input-response samples.'}
    $ordered=@($Values|Sort-Object);return $ordered[[Math]::Max(0,[Math]::Ceiling($ordered.Count*.95)-1)]
}
function Get-M2LatencyComparison([object[]]$Reports,[string]$Network){
    if($Reports.Count -ne 3){throw 'Visible latency requires all three measured rounds.'}
    if(@($Reports|Where-Object {$_.diagnostic_mode}).Count){throw 'Diagnostic tracing cannot enter a formal timing comparison.'}
    $rounds=@($Reports|ForEach-Object {$_.receiver.runs[0].round}|Sort-Object -Unique)
    if(($rounds -join ',') -cne '1,2,3'){throw 'Warmup, duplicate or missing measured rounds cannot enter the latency gate.'}
    $frozen=Get-Content -LiteralPath (Join-Path $repository 'docs/agent/baselines/M2/performance-policy.json') -Raw -Encoding UTF8|ConvertFrom-Json
    foreach($index in 0..1){
        $local=@();$ssh=@();$isolation=@()
        foreach($report in $Reports){
            $run=$report.receiver.runs[$index]
            foreach($terminal in $run.terminals){$source=$report.sources.PSObject.Properties[$terminal.id].Value;if($source.backend -eq 'local'){$local+=@($terminal.input_response_ms)}else{$ssh+=@($terminal.input_response_ms)}}
            $isolation+=@($run.isolation_input_response_ms)
        }
        if($local.Count -lt 33 -or $ssh.Count -lt 36 -or $isolation.Count -ne 30){throw 'Incomplete measured input sample topology.'}
        $localP95=Get-M2P95 $local;$sshP95=Get-M2P95 $ssh;$isolationP95=Get-M2P95 $isolation
        [ordered]@{direction=@('upload','download')[$index];localSamples=$local.Count;sshSamples=$ssh.Count;heldAckSamples=$isolation.Count;localInputP95Ms=$localP95;sshInputP95Ms=$sshP95;heldAckSiblingInputP95Ms=$isolationP95;maximumLocalInputP95Ms=$frozen.mixedTerminalLoadLimits.maximumLocalInputP95Ms;maximumSshInputP95Ms=$frozen.mixedTerminalLoadLimits.byNetwork.$Network.maximumInputP95Ms;maximumHeldAckSiblingInputP95Ms=$frozen.mixedTerminalLoadLimits.maximumSlowAckSiblingInputP95Ms;passed=($localP95 -le $frozen.mixedTerminalLoadLimits.maximumLocalInputP95Ms -and $sshP95 -le $frozen.mixedTerminalLoadLimits.byNetwork.$Network.maximumInputP95Ms -and $isolationP95 -le $frozen.mixedTerminalLoadLimits.maximumSlowAckSiblingInputP95Ms)}
    }
}
$policy=Join-Path $repository 'docs/agent/baselines/M2/performance-policy.json'
$policyHash=(Get-FileHash -LiteralPath $policy).Hash
Copy-Item -LiteralPath $policy -Destination (Join-Path $evidence 'premeasurement-policy.json')
$previousM1=$env:CAPSTAN_M1_FIXTURE_CONFIG;$previousM2=$env:CAPSTAN_M2_LOAD_CONFIG
$previousRtt=$env:CAPSTAN_TEST_SFTP_PERFORMANCE
$previousDiagnostic=$env:CAPSTAN_M2_DIAGNOSTIC
$env:CAPSTAN_M2_DIAGNOSTIC=if($DiagnosticTrace){'1'}else{$null}
$session=$null;$failure=$null;$awake=[uint32]0;$released=$false;$results=@()
$workers=[Collections.Generic.List[object]]::new()
. (Join-Path $PSScriptRoot 'M2OwnedProcess.ps1')
try {
    if(-not ('Capstan.M2MixedPower' -as [type])){
        Add-Type -TypeDefinition @'
using System.Runtime.InteropServices;
namespace Capstan { public static class M2MixedPower { [DllImport("kernel32.dll")] public static extern uint SetThreadExecutionState(uint flags); } }
'@
    }
    $awake=[Capstan.M2MixedPower]::SetThreadExecutionState([uint32]2147483649)
    if($awake -eq 0){throw 'Could not hold a temporary helper sleep requirement.'}
    $ErrorActionPreference='Continue'
    & cargo test -p capstan-ssh --release --test sftp_performance --offline --locked --no-run *> (Join-Path $evidence 'rtt-build.txt')
    $code=$LASTEXITCODE;$ErrorActionPreference='Stop'
    if($code -ne 0){throw 'The authenticated RTT probe failed to build.'}
    $rtt=@(Get-ChildItem (Join-Path $repository 'target/release/deps/sftp_performance-*.exe')|Sort-Object LastWriteTimeUtc -Descending)[0].FullName
    $env:CAPSTAN_TEST_SFTP_PERFORMANCE='1'
    $rttWorker=Start-OwnedMatrix $rtt 'configured_openssh_shell_rtt_reference --exact --nocapture' (Join-Path $evidence 'ssh-rtt-reference.txt') ''
    Wait-OwnedMatrix @($rttWorker) 60 | Out-Null
    $rttMatch=[regex]::Match(([IO.File]::ReadAllText((Join-Path $evidence 'ssh-rtt-reference.txt'))),'SSH_RTT_SAMPLE (\{[^\r\n]*\})')
    if(-not $rttMatch.Success){throw 'No authenticated SSH RTT execution record.'}
    Save-Json (Join-Path $evidence 'ssh-rtt-reference.json') ($rttMatch.Groups[1].Value|ConvertFrom-Json)
    $payloadPath=Join-Path $workspaceRoot 'payload.bin';$file=[IO.File]::Open($payloadPath,[IO.FileMode]::CreateNew,[IO.FileAccess]::Write,[IO.FileShare]::None)
    $random=[Random]::new(20261017);$buffer=[byte[]]::new(1MB)
    try{for($index=0;$index -lt $ProbeMiB;$index++){$random.NextBytes($buffer);$file.Write($buffer,0,$buffer.Length)}}finally{$file.Dispose()}
    $hash=(Get-FileHash -LiteralPath $payloadPath).Hash
    $winScp='C:/Program Files (x86)/WinSCP';Add-Type -Path (Join-Path $winScp 'WinSCPnet.dll')
    $publicKey=& ssh-keygen -l -E sha256 -f $env:CAPSTAN_TEST_SSH_HOST_KEY_FILE
    if($LASTEXITCODE -ne 0 -or ($publicKey -split '\s+')[1] -notmatch '^SHA256:([A-Za-z0-9+/]{43})$'){throw 'Invalid pinned fixture host key.'}
    $options=New-Object WinSCP.SessionOptions;$options.Protocol=[WinSCP.Protocol]::Sftp;$options.HostName='127.0.0.1';$options.PortNumber=$port;$options.UserName="cm1$token"
    $options.Password=([IO.File]::ReadAllText($env:CAPSTAN_TEST_SSH_PASSWORD_FILE)).TrimEnd("`r","`n");$options.SshHostKeyFingerprint="ssh-ed25519 255 $($Matches[1])";$options.AddRawSettings('Compression','0')
    $session=New-Object WinSCP.Session;$session.ExecutablePath=Join-Path $winScp 'WinSCP.exe';$session.SessionLogPath=Join-Path $evidence 'seed.log';$session.Open($options)
    $transfer=New-Object WinSCP.TransferOptions;$transfer.TransferMode=[WinSCP.TransferMode]::Binary;$transfer.PreserveTimestamp=$false;$transfer.ResumeSupport.State=[WinSCP.TransferResumeSupportState]::Off
    $session.PutFiles($payloadPath,"$env:CAPSTAN_TEST_SFTP_ROOT/visible-source.bin",$false,$transfer).Check();$session.Dispose();$session=$null
    Save-Json (Join-Path $evidence 'input-environment.json') ([ordered]@{schemaVersion=1;network=$NetworkScenario;bytesPerFile=([long]$ProbeMiB*1MB).ToString();sha256=$hash;seed=20261017;policySha256=$policyHash;executableSha256=(Get-FileHash -LiteralPath $application).Hash;fixtureSha256=(Get-FileHash -LiteralPath (Join-Path $applicationDirectory 'capstan-m0-fixture.exe')).Hash;measurementManifestSha256=(Get-FileHash -LiteralPath (Join-Path $applicationDirectory 'm2-measurement.json')).Hash;smoke=[bool]$Smoke;method='Visible Tauri raw Channel/controller/xterm parser response; one local terminal, two same-connection SSH shells and three native production transfers; held ACK isolation and tab switching';powerPlanChanged=$false;performanceAcceptance=$false;desktopAcceptance=$false})
    $last=if($Smoke){0}else{3}
    foreach($round in 0..$last){
        if(@(Get-Process -Name cargo,rustc,clippy-driver -ErrorAction SilentlyContinue).Count){throw 'No concurrent compilers/checks during visible samples.'}
        $private=Join-Path $workspaceRoot 'secrets'
        $env:CAPSTAN_M1_FIXTURE_CONFIG=Join-Path $private 'measurement.json'
        Save-Json $env:CAPSTAN_M1_FIXTURE_CONFIG ([ordered]@{port=$port;username="cm1$token";password_file=$env:CAPSTAN_TEST_SSH_PASSWORD_FILE;key_file=(Join-Path $private 'openssh');host_public_key_file=$env:CAPSTAN_TEST_SSH_HOST_KEY_FILE;vault_file=(Join-Path $private "visible-round-$round.capstan");remote_fixture="$env:CAPSTAN_TEST_SFTP_ROOT/benchmark.py"})
        $env:CAPSTAN_M2_LOAD_CONFIG=Join-Path $private 'm2-load.json'
        Save-Json $env:CAPSTAN_M2_LOAD_CONFIG ([ordered]@{input=$payloadPath;sha256=$hash;remoteRoot=$env:CAPSTAN_TEST_SFTP_ROOT;round=$round;smoke=[bool]$Smoke})
        Write-Output "M2 visible mixed: round $round (both directions)"
        $roundDirectory=Join-Path $evidence "round-$round"
        $deadline=if($Smoke){300}else{3600}
        & (Join-Path $PSScriptRoot 'Measure-M0.ps1') -Executable $application -OutputDirectory $roundDirectory -Visible -TransferLoad -NetworkScenario $NetworkScenario -Smoke:$Smoke -TimeoutSeconds $deadline
        $results+=Get-Content -LiteralPath (Join-Path $roundDirectory 'm2-verification.json') -Raw -Encoding UTF8|ConvertFrom-Json
    }
    if(-not $Smoke){
        $measured=@(foreach($round in 1..3){Get-Content -LiteralPath (Join-Path $evidence "round-$round/terminal-report.json") -Raw -Encoding UTF8|ConvertFrom-Json})
        $comparisons=@(Get-M2LatencyComparison $measured $NetworkScenario)
        Save-Json (Join-Path $evidence 'comparison.json') ([ordered]@{passed=(@($comparisons|Where-Object {-not $_.passed}).Count -eq 0);comparisons=$comparisons;policySha256=$policyHash;warmupExcluded=$true;measuredRounds=3;performanceAcceptance=$false;desktopAcceptance=$false})
        if(@($comparisons|Where-Object {-not $_.passed}).Count){throw 'Visible mixed input exceeds the unchanged frozen targets.'}
    }
    if((Get-FileHash -LiteralPath $policy).Hash -ne $policyHash){throw 'The frozen policy changed.'}
} catch {$failure=$_.Exception.Message} finally {
    foreach($worker in $workers){
        try{Stop-OwnedMatrix $worker}catch{if(-not $failure){$failure='Owned RTT process cleanup failed.'}}
        try{if(-not (Test-Path -LiteralPath $worker.log)){[IO.File]::WriteAllText($worker.log,$worker.output.GetAwaiter().GetResult()+"`n"+$worker.errors.GetAwaiter().GetResult(),$utf8)}}catch{}
        $worker.process.Dispose()
    }
    if($session){$session.Dispose()}
    if($awake -ne 0){$released=([Capstan.M2MixedPower]::SetThreadExecutionState($awake) -ne 0)}
    [Environment]::SetEnvironmentVariable('CAPSTAN_M1_FIXTURE_CONFIG',$previousM1,'Process');[Environment]::SetEnvironmentVariable('CAPSTAN_M2_LOAD_CONFIG',$previousM2,'Process')
    [Environment]::SetEnvironmentVariable('CAPSTAN_TEST_SFTP_PERFORMANCE',$previousRtt,'Process')
    [Environment]::SetEnvironmentVariable('CAPSTAN_M2_DIAGNOSTIC',$previousDiagnostic,'Process')
    Save-Json (Join-Path $evidence 'report.json') ([ordered]@{schemaVersion=1;passed=($null -eq $failure -and $released);failure=$failure;network=$NetworkScenario;smoke=[bool]$Smoke;rounds=$results;policySha256=$policyHash;temporaryAwakeRequestReleased=$released;performanceAcceptance=$false;desktopAcceptance=$false})
}
if($failure){throw $failure}
