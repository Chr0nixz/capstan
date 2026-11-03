param([Parameter(Mandatory=$true)][ValidateRange(0,2)][int]$Index)
# Private, fixed-workload child of Measure-M2Workflow.ps1. No application IPC.
$ErrorActionPreference='Stop'
$session=$null
try {
    $repository=(Resolve-Path -LiteralPath (Join-Path $PSScriptRoot '..')).Path
    $configPath=[IO.Path]::GetFullPath($env:CAPSTAN_M2_MATRIX_WORKER_CONFIG)
    $private=Split-Path $configPath -Parent
    $workspace=Split-Path $private -Parent
    $prefix=(Join-Path $repository 'target').TrimEnd('\')+'\'
    if (-not $workspace.StartsWith($prefix,[StringComparison]::OrdinalIgnoreCase) -or
        [IO.Path]::GetFileName($workspace) -notmatch '^m1-sshd-[0-9a-f]{12}$' -or
        [IO.Path]::GetFileName($private) -ne 'secrets' -or
        [IO.Path]::GetFileName($configPath) -notmatch '^matrix-[0-9a-f]{32}\.json$') { throw 'worker_scope' }
    $token=[IO.Path]::GetFileName($workspace).Substring(8)
    $config=Get-Content -LiteralPath $configPath -Raw -Encoding UTF8 | ConvertFrom-Json
    if ($config.mode -notin @('files','tree') -or $config.direction -notin @('upload','download') -or
        $config.remoteRoot -cne "/var/tmp/capstan-m1-ssh-$token/home" -or $config.username -cne "cm1$token" -or
        $config.port -lt 1024 -or $config.port -gt 65535 -or $config.sample -notmatch '^winscp-[0-3]-(upload|download)$') { throw 'worker_config' }
    Add-Type -Path (Join-Path $config.winScpDirectory 'WinSCPnet.dll')
    $options=New-Object WinSCP.SessionOptions
    $options.Protocol=[WinSCP.Protocol]::Sftp
    $options.HostName='127.0.0.1'; $options.PortNumber=$config.port; $options.UserName=$config.username
    $options.Password=([IO.File]::ReadAllText((Join-Path $private 'password.txt'))).TrimEnd("`r","`n")
    $options.SshHostKeyFingerprint=$config.fingerprint
    $options.AddRawSettings('Compression','0')
    $session=New-Object WinSCP.Session
    $session.ExecutablePath=Join-Path $config.winScpDirectory 'WinSCP.exe'
    $session.SessionLogPath=Join-Path $config.evidence "$($config.sample)-worker-$Index.log"
    $timer=[Diagnostics.Stopwatch]::StartNew()
    $session.Open($options)
    $negotiated=Get-Content -LiteralPath $session.SessionLogPath -Raw
    if (-not $negotiated.Contains('Doing ECDH key exchange with curve Curve25519, using hash SHA-256') -or
        -not $negotiated.Contains('[aes256-gcm@openssh.com] outbound encryption') -or
        -not $negotiated.Contains('[aes256-gcm@openssh.com] inbound encryption') -or
        -not $negotiated.Contains('Compression: No')) { throw 'worker_algorithms' }
    $transfer=New-Object WinSCP.TransferOptions
    $transfer.TransferMode=[WinSCP.TransferMode]::Binary; $transfer.PreserveTimestamp=$false
    $transfer.ResumeSupport.State=[WinSCP.TransferResumeSupportState]::Off
    if ($config.mode -eq 'files') {
        $items=@([pscustomobject]@{ path="reference-file-$Index.bin"; source='payload.bin'; bytes=$config.payloadBytes; sha256=$config.payloadSha256 })
    } else {
        $manifest=Get-Content -LiteralPath (Join-Path $workspace 'matrix-tree-manifest.json') -Raw -Encoding UTF8 | ConvertFrom-Json
        $items=@(for($position=$Index;$position -lt $manifest.files.Count;$position+=3){
            $item=$manifest.files[$position]
            if (-not $item.path.StartsWith('matrix-tree/') -or $item.path.Split('/') -contains '..' -or $item.path.Contains('\')){throw 'worker_manifest'}
            [pscustomobject]@{path=$item.path;source=$item.path;bytes=$item.bytes;sha256=$item.sha256}
        })
    }
    $prepare=$timer.Elapsed.TotalMilliseconds
    $dataStart=$null; $dataEnd=$null; $transferMs=0.0; $verificationMs=0.0; $bytes=[long]0; $hashes=@()
    foreach($item in $items){
        $source=Join-Path $workspace $item.source
        $remote="$($config.remoteRoot)/$($item.path)"
        $returned=if($config.mode -eq 'tree' -and $config.direction -eq 'download'){Join-Path $workspace ("reference-tree-output/"+$item.path)}else{Join-Path $workspace ("reference-returned/$Index/"+$item.path)}
        New-Item -ItemType Directory -Path (Split-Path $returned -Parent) -Force | Out-Null
        if(Test-Path -LiteralPath $returned){Remove-Item -LiteralPath $returned -Force}
        $start=[Diagnostics.Stopwatch]::GetTimestamp()
        if($null -eq $dataStart){$dataStart=$start}
        $phase=[Diagnostics.Stopwatch]::StartNew()
        if($config.direction -eq 'upload'){$session.PutFiles($source,$remote,$false,$transfer).Check()}
        else{$session.GetFiles($remote,$returned,$false,$transfer).Check()}
        $transferMs+=$phase.Elapsed.TotalMilliseconds; $dataEnd=[Diagnostics.Stopwatch]::GetTimestamp()
        $phase.Restart()
        if($config.direction -eq 'upload'){$session.GetFiles($remote,$returned,$false,$transfer).Check()}
        $actual=(Get-FileHash -LiteralPath $returned -Algorithm SHA256).Hash
        if($actual -ine $item.sha256 -or (Get-Item -LiteralPath $returned).Length -ne [long]$item.bytes){throw 'worker_hash'}
        $verificationMs+=$phase.Elapsed.TotalMilliseconds; $bytes+=[long]$item.bytes; $hashes+=$actual
    }
    $close=[Diagnostics.Stopwatch]::StartNew(); $session.Dispose(); $session=$null
    $record=[ordered]@{index=$Index;files=$items.Count;bytes=$bytes.ToString();prepareMs=$prepare;transferMs=$transferMs;verificationMs=$verificationMs;connectionCloseMs=$close.Elapsed.TotalMilliseconds;totalMs=$timer.Elapsed.TotalMilliseconds;dataStartTicks=$dataStart.ToString();dataEndTicks=$dataEnd.ToString();hashMatched=$true;hashes=$hashes;negotiated=@{cipher='aes256-gcm@openssh.com';kex='curve25519-sha256'}}
    Write-Output ('WIN_MATRIX_JOB '+($record|ConvertTo-Json -Compress -Depth 6))
} catch {
    # A stable failure marker never includes credentials or library exception text.
    Write-Output 'WIN_MATRIX_JOB_FAILED'
    exit 1
} finally {if($session){$session.Dispose()}}
