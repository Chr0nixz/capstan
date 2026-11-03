# Shared by disposable M2 measurement runners. The caller owns the workers
# collection and supplies repository/utf8; these functions never select PIDs by name.
function Start-OwnedMatrix([string]$Executable,[string]$Arguments,[string]$Log,[string]$ConfigPath){
    $process=[Diagnostics.Process]::new();$process.StartInfo=[Diagnostics.ProcessStartInfo]::new()
    $process.StartInfo.FileName=$Executable;$process.StartInfo.Arguments=$Arguments;$process.StartInfo.WorkingDirectory=$repository
    $process.StartInfo.UseShellExecute=$false;$process.StartInfo.CreateNoWindow=$true;$process.StartInfo.WindowStyle=[Diagnostics.ProcessWindowStyle]::Hidden
    $process.StartInfo.RedirectStandardOutput=$true;$process.StartInfo.RedirectStandardError=$true
    $process.StartInfo.StandardOutputEncoding=$utf8;$process.StartInfo.StandardErrorEncoding=$utf8
    if($ConfigPath){$process.StartInfo.EnvironmentVariables['CAPSTAN_M2_MATRIX_WORKER_CONFIG']=$ConfigPath}
    $clock=[Diagnostics.Stopwatch]::StartNew()
    if(-not $process.Start()){throw 'Owned matrix worker failed to start.'}
    # Retain the process handle until cleanup; PID ownership is never inferred by name.
    $null=$process.SafeHandle
    $worker=[pscustomobject]@{process=$process;clock=$clock;log=$Log;output=$process.StandardOutput.ReadToEndAsync();errors=$process.StandardError.ReadToEndAsync()}
    $workers.Add($worker);return $worker
}
function Stop-OwnedMatrix($Worker){
    if(-not $Worker.process.HasExited){
        # The retained handle pins this exact PID. Stop only its child tree.
        # The child may exit between HasExited and taskkill. Its observed exit
        # is authoritative; a stale termination error must not hide a deadline.
        try { & taskkill.exe /PID $Worker.process.Id /T /F *> $null } catch {
            if(-not $Worker.process.WaitForExit(1000)){throw}
        }
        if(-not $Worker.process.WaitForExit(10000)){throw 'Owned matrix worker tree did not terminate.'}
    }
}
function Wait-OwnedMatrix([object[]]$Pending,[int]$TimeoutSeconds,[string]$CancellationPath=''){
    $results=@()
    foreach($worker in $Pending){
        $timedOut=$false;$cancelled=$false;$cleanupError=$null
        try {
            while(-not $worker.process.WaitForExit(250)){
                if($CancellationPath -and (Test-Path -LiteralPath $CancellationPath -PathType Leaf)){$cancelled=$true;Stop-OwnedMatrix $worker;break}
                if($worker.clock.Elapsed.TotalSeconds -ge $TimeoutSeconds){$timedOut=$true;Stop-OwnedMatrix $worker;break}
            }
        } catch { $cleanupError=$_.Exception.Message }
        $exited=$worker.process.WaitForExit(1000)
        $worker.clock.Stop()
        if(-not $cancelled -and $worker.clock.Elapsed.TotalSeconds -ge $TimeoutSeconds){$timedOut=$true}
        $drained=$false
        try { $drained=[Threading.Tasks.Task]::WhenAll([Threading.Tasks.Task[]]@($worker.output,$worker.errors)).Wait(5000) } catch {
            if(-not $cleanupError){$cleanupError='Owned process output failed before it could be drained.'}
        }
        if($drained){
            $output=$worker.output.GetAwaiter().GetResult();$errors=$worker.errors.GetAwaiter().GetResult()
        }else{
            $output='';$errors='Owned process output did not close before its cleanup deadline.'
            if(-not $cleanupError){$cleanupError=$errors}
        }
        [IO.File]::WriteAllText($worker.log,$output+"`n"+$errors,$utf8)
        $code=if($exited){$worker.process.ExitCode}else{$null}
        Save-Json "$($worker.log).execution.json" ([ordered]@{exitCode=$code;timedOut=$timedOut;cancelled=$cancelled;cleanupError=$cleanupError;processExited=$exited;outputDrained=$drained;elapsedMs=$worker.clock.Elapsed.TotalMilliseconds;timeoutSeconds=$TimeoutSeconds;scope='Owned helper process; not visible application resource acceptance'})
        if($cleanupError){throw "Owned matrix cleanup failed: $cleanupError. timedOut=$timedOut; cancelled=$cancelled"}
        if($cancelled){throw 'Owned matrix measurement cancelled; preserved output and stopped its own process tree.'}
        if($timedOut){throw 'Owned matrix worker exceeded its wall-clock deadline.'}
        if($code -ne 0){throw 'Matrix worker failed; inspect the preserved stdout/stderr record.'}
        $results+=@{text=$output;elapsedMs=$worker.clock.Elapsed.TotalMilliseconds}
    }
    return $results
}
