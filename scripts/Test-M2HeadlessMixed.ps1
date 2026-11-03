param(
    [Parameter(Mandatory=$true)][string]$EvidenceDirectory,
    [Parameter(Mandatory=$true)][string]$Workspace,
    [ValidateSet(64,1024)][int]$ProbeMiB = 64
)
$ErrorActionPreference='Stop'
$repository=(Resolve-Path -LiteralPath (Join-Path $PSScriptRoot '..')).Path
$workspaceRoot=(Resolve-Path -LiteralPath $Workspace).Path
$prefix=(Join-Path $repository 'target').TrimEnd('\')+'\'
if(-not $workspaceRoot.StartsWith($prefix,[StringComparison]::OrdinalIgnoreCase) -or [IO.Path]::GetFileName($workspaceRoot) -notmatch '^m1-sshd-[0-9a-f]{12}$'){throw 'Use the disposable mixed fixture workspace.'}
$token=[IO.Path]::GetFileName($workspaceRoot).Substring(8)
if($env:CAPSTAN_TEST_SFTP_ROOT -cne "/var/tmp/capstan-m1-ssh-$token/home" -or $env:CAPSTAN_TEST_SSHD -notmatch "^cm1$token@127\.0\.0\.1:([0-9]+)$"){throw 'Use the isolated OpenSSH runner.'}
$port=[int]$Matches[1]
$evidence=[IO.Path]::GetFullPath($EvidenceDirectory)
if(Test-Path -LiteralPath $evidence){throw 'Preserve every headless diagnostic attempt.'}
New-Item -ItemType Directory -Path $evidence | Out-Null
$utf8=[Text.UTF8Encoding]::new($false)
function Save-Json([string]$Path,$Value){[IO.File]::WriteAllText($Path,(ConvertTo-Json -InputObject $Value -Depth 16),$utf8)}
$workers=[Collections.Generic.List[object]]::new()
. (Join-Path $PSScriptRoot 'M2OwnedProcess.ps1')
$variables=@('CAPSTAN_TEST_HEADLESS_MIXED','CAPSTAN_M1_FIXTURE_CONFIG','CAPSTAN_M2_LOAD_CONFIG')
$previous=@{};foreach($variable in $variables){$previous[$variable]=[Environment]::GetEnvironmentVariable($variable,'Process')}
$session=$null;$failure=$null;$samples=@()
Push-Location $repository
try{
    $ErrorActionPreference='Continue'
    & cargo test -p capstan-core --release --test ssh_mixed_progress --features benchmarks --offline --locked --no-run *> (Join-Path $evidence 'build.txt')
    $buildCode=$LASTEXITCODE;$ErrorActionPreference='Stop'
    if($buildCode -ne 0){throw 'The headless progress probe did not build.'}
    $probe=@(Get-ChildItem (Join-Path $repository 'target/release/deps/ssh_mixed_progress-*.exe') | Sort-Object LastWriteTimeUtc -Descending)[0].FullName
    $payload=Join-Path $workspaceRoot 'payload.bin'
    $file=[IO.File]::Open($payload,[IO.FileMode]::CreateNew,[IO.FileAccess]::Write,[IO.FileShare]::None)
    $random=[Random]::new(20261017);$buffer=[byte[]]::new(1MB)
    try{for($index=0;$index -lt $ProbeMiB;$index++){$random.NextBytes($buffer);$file.Write($buffer,0,$buffer.Length)}}finally{$file.Dispose()}
    $hash=(Get-FileHash -LiteralPath $payload).Hash
    $winScp='C:/Program Files (x86)/WinSCP';Add-Type -Path (Join-Path $winScp 'WinSCPnet.dll')
    $publicKey=& ssh-keygen -l -E sha256 -f $env:CAPSTAN_TEST_SSH_HOST_KEY_FILE
    if($LASTEXITCODE -ne 0 -or ($publicKey -split '\s+')[1] -notmatch '^SHA256:([A-Za-z0-9+/]{43})$'){throw 'Invalid pinned fixture key.'}
    $options=New-Object WinSCP.SessionOptions;$options.Protocol=[WinSCP.Protocol]::Sftp;$options.HostName='127.0.0.1';$options.PortNumber=$port;$options.UserName="cm1$token"
    $options.Password=([IO.File]::ReadAllText($env:CAPSTAN_TEST_SSH_PASSWORD_FILE)).TrimEnd("`r","`n")
    $options.SshHostKeyFingerprint="ssh-ed25519 255 $($Matches[1])";$options.AddRawSettings('Compression','0')
    $session=New-Object WinSCP.Session;$session.ExecutablePath=Join-Path $winScp 'WinSCP.exe';$session.SessionLogPath=Join-Path $evidence 'seed.log';$session.Open($options)
    $transfer=New-Object WinSCP.TransferOptions;$transfer.TransferMode=[WinSCP.TransferMode]::Binary;$transfer.PreserveTimestamp=$false;$transfer.ResumeSupport.State=[WinSCP.TransferResumeSupportState]::Off
    $session.PutFiles($payload,"$env:CAPSTAN_TEST_SFTP_ROOT/visible-source.bin",$false,$transfer).Check();$session.Dispose();$session=$null
    $private=Join-Path $workspaceRoot 'secrets'
    $env:CAPSTAN_M1_FIXTURE_CONFIG=Join-Path $private 'headless.json'
    Save-Json $env:CAPSTAN_M1_FIXTURE_CONFIG ([ordered]@{port=$port;username="cm1$token";password_file=$env:CAPSTAN_TEST_SSH_PASSWORD_FILE;key_file=(Join-Path $private 'openssh');host_public_key_file=$env:CAPSTAN_TEST_SSH_HOST_KEY_FILE;vault_file=(Join-Path $private 'headless.capstan');remote_fixture="$env:CAPSTAN_TEST_SFTP_ROOT/benchmark.py"})
    $env:CAPSTAN_M2_LOAD_CONFIG=Join-Path $private 'm2-load.json'
    Save-Json $env:CAPSTAN_M2_LOAD_CONFIG ([ordered]@{input=$payload;sha256=$hash;remoteRoot=$env:CAPSTAN_TEST_SFTP_ROOT;round=0;smoke=($ProbeMiB -eq 64)})
    $env:CAPSTAN_TEST_HEADLESS_MIXED='1'
    Save-Json (Join-Path $evidence 'input-environment.json') ([ordered]@{schemaVersion=1;probeSha256=(Get-FileHash -LiteralPath $probe).Hash;testSourceSha256=(Get-FileHash -LiteralPath (Join-Path $repository 'crates/capstan-core/tests/ssh_mixed_progress.rs')).Hash;runnerSha256=(Get-FileHash -LiteralPath $PSCommandPath).Hash;bytesPerFile=([long]$ProbeMiB*1MB).ToString();smoke=($ProbeMiB -eq 64);sha256=$hash;seed=20261017;scope='Headless production terminal ACK/backpressure and three-transfer progress diagnosis; no WebView or rendering';foregroundRequired=$false;performanceAcceptance=$false;desktopAcceptance=$false})
    $worker=Start-OwnedMatrix $probe 'configured_openssh_mixed_progress_without_webview --exact --nocapture' (Join-Path $evidence 'progress.txt') ''
    $sampleDeadline = if($ProbeMiB -eq 64){420}else{1800}
    $execution=@(Wait-OwnedMatrix @($worker) $sampleDeadline (Join-Path $evidence 'cancel.request'))[0]
    $matches=[regex]::Matches($execution.text,'(?m)^HEADLESS_MIXED_SAMPLE (\{[^\r\n]*\})\r?$')
    if($matches.Count -ne 2 -or $execution.text.Contains('SKIP:')){throw 'The headless probe has no complete dual-direction evidence.'}
    foreach($match in $matches){
        $sample=$match.Groups[1].Value | ConvertFrom-Json
        if([long]$sample.transfer.bytesPerFile -ne ([long]$ProbeMiB*1MB) -or -not $sample.headless -or $sample.sshConnections -ne 1 -or $sample.poolEntriesDuring -ne 1 -or $sample.poolEntriesAfter -ne 0 -or $sample.registeredSessionsAfter -ne 0 -or $sample.inputProbes -ne 8 -or @($sample.transfer.hashes).Count -ne 3 -or $sample.transfer.confirmedAllRunningChecks -lt 16 -or $sample.transfer.finalOwnedPayloadAdmissionBytes -ne 0){throw 'Incomplete headless progress/cleanup proof.'}
        foreach($actual in $sample.transfer.hashes){if($actual -ine $hash){throw 'Headless fixture hash mismatch.'}}
        $samples+=@($sample)
    }
    $directions=@($samples | ForEach-Object {$_.transfer.direction} | Sort-Object -Unique)
    if($directions.Count -ne 2 -or $directions -notcontains 'upload' -or $directions -notcontains 'download'){throw 'Headless evidence must cover both transfer directions.'}
    Save-Json (Join-Path $evidence 'samples.json') $samples
}catch{$failure=$_.Exception.Message}finally{
    if($session){$session.Dispose()}
    foreach($worker in $workers){try{Stop-OwnedMatrix $worker}finally{$worker.process.Dispose()}}
    foreach($variable in $variables){[Environment]::SetEnvironmentVariable($variable,$previous[$variable],'Process')}
    Pop-Location
    Save-Json (Join-Path $evidence 'report.json') ([ordered]@{schemaVersion=1;passed=($null -eq $failure);failure=$failure;directions=$samples.Count;foregroundRequired=$false;scope='Headless progress diagnosis only; does not close visible mixed/M1/resource gates';performanceAcceptance=$false;desktopAcceptance=$false})
}
if($failure){throw $failure}
Write-Output 'Headless production mixed progress passed in both directions.'
