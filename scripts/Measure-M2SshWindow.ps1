param([Parameter(Mandatory=$true)][string]$EvidenceDirectory,[Parameter(Mandatory=$true)][string]$Workspace)
$ErrorActionPreference='Stop'
$repository=(Resolve-Path -LiteralPath (Join-Path $PSScriptRoot '..')).Path
$workspaceRoot=(Resolve-Path -LiteralPath $Workspace).Path
$prefix=(Join-Path $repository 'target').TrimEnd('\')+'\'
if(-not $workspaceRoot.StartsWith($prefix,[StringComparison]::OrdinalIgnoreCase) -or [IO.Path]::GetFileName($workspaceRoot) -notmatch '^m1-sshd-[0-9a-f]{12}$'){throw 'Use the isolated fixture workspace.'}
$token=[IO.Path]::GetFileName($workspaceRoot).Substring(8)
if($env:CAPSTAN_TEST_SFTP_ROOT -cne "/var/tmp/capstan-m1-ssh-$token/home" -or $env:CAPSTAN_TEST_SSHD -notmatch "^cm1$token@127\.0\.0\.1:([0-9]+)$"){throw 'Use the isolated SFTP probe runner.'}
$port=[int]$Matches[1]
$evidence=[IO.Path]::GetFullPath($EvidenceDirectory)
if(Test-Path -LiteralPath $evidence){throw 'Preserve previous channel window probes.'}
New-Item -ItemType Directory -Path $evidence|Out-Null
$utf8=[Text.UTF8Encoding]::new($false)
function Save-Json([string]$Path,$Value){[IO.File]::WriteAllText($Path,($Value|ConvertTo-Json -Depth 8),$utf8)}
$workers=[Collections.Generic.List[object]]::new()
. (Join-Path $PSScriptRoot 'M2OwnedProcess.ps1')
$failure=$null;$reference=$null
try{
    $key=Join-Path $workspaceRoot 'secrets/openssh'
    $hostKey=([IO.File]::ReadAllText($env:CAPSTAN_TEST_SSH_HOST_KEY_FILE)).Trim() -split '\s+'
    if($hostKey.Count -lt 2 -or $hostKey[0] -cne 'ssh-ed25519' -or $hostKey[1] -notmatch '^[A-Za-z0-9+/=]+$'){throw 'Invalid pinned fixture host key.'}
    $knownHosts=Join-Path $workspaceRoot 'secrets/window-known-hosts'
    [IO.File]::WriteAllText($knownHosts,"[127.0.0.1]:$port $($hostKey[0]) $($hostKey[1])`n",$utf8)
    $ssh=(Get-Command ssh.exe -ErrorAction Stop).Source
    # -n supplies EOF without input. Only the SFTP subsystem is opened; there
    # is no shell, remote command, password argument or persistent SSH config.
    $arguments='-vv -n -F NUL -i "'+$key+'" -o BatchMode=yes -o IdentitiesOnly=yes -o StrictHostKeyChecking=yes -o "UserKnownHostsFile='+$knownHosts+'" -o GlobalKnownHostsFile=NUL -o PreferredAuthentications=publickey -p '+$port+' -s cm1'+$token+'@127.0.0.1 sftp'
    $log=Join-Path $evidence 'ssh-window-reference.txt'
    $worker=Start-OwnedMatrix $ssh $arguments $log ''
    Wait-OwnedMatrix @($worker) 30|Out-Null
    $windowMatches=[regex]::Matches(([IO.File]::ReadAllText($log)),'(?m)^debug2: channel \d+: open confirm rwindow (\d+) rmax (\d+)\r?$')
    if($windowMatches.Count -ne 1){throw 'No unique actual SSH channel-open window observation.'}
    $window=[uint32]::Parse($windowMatches[0].Groups[1].Value)
    $packet=[uint32]::Parse($windowMatches[0].Groups[2].Value)
    $adjustments=@([regex]::Matches(([IO.File]::ReadAllText($log)),'(?m)^debug2: channel \d+: rcvd adjust (\d+)\r?$')|ForEach-Object {[uint32]::Parse($_.Groups[1].Value)})
    $activated=[long]$window;foreach($adjustment in $adjustments){$activated+=$adjustment}
    if($activated -gt [uint32]::MaxValue){throw 'Invalid cumulative peer channel window.'}
    $reference=[ordered]@{schemaVersion=1;peerInitialChannelWindowBytes=$window;peerActivatedChannelWindowBytes=$activated;peerWindowAdjustments=$adjustments;peerMaximumPacketBytes=$packet;applicationPayloadSentBytes=0;client='Windows OpenSSH ssh.exe';clientSha256=(Get-FileHash -LiteralPath $ssh).Hash;subsystem='sftp';pinning='StrictHostKeyChecking=yes with a private generated fixture known_hosts file';method='Actual SSH channel-open confirmation and window adjustments before application data, reported by the independent native client';scope='Server-advertised flow-control reference, not a product throughput measurement';performanceAcceptance=$false;desktopAcceptance=$false}
    [IO.File]::WriteAllText((Join-Path $evidence 'channel-window.json'),($reference|ConvertTo-Json -Depth 5),$utf8)
}catch{$failure=$_.Exception.Message}finally{
    foreach($worker in $workers){try{Stop-OwnedMatrix $worker}finally{$worker.process.Dispose()}}
    [IO.File]::WriteAllText((Join-Path $evidence 'report.json'),([ordered]@{passed=($null -eq $failure);failure=$failure;scope='Actual channel window reference only';performanceAcceptance=$false;desktopAcceptance=$false}|ConvertTo-Json),$utf8)
}
if($failure){throw $failure}
Write-Output "Server SFTP activated channel window: $activated bytes; packet: $packet bytes."
