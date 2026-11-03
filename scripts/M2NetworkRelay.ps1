# Explicit Windows loopback -> assigned WSL private address. This helper is
# fixture infrastructure; product code and the frozen limits do not depend on it.
function Get-M2NetworkProfile([string]$Scenario, [string]$Relay) {
    if ($Scenario -notin @('loopback','controlled_high_rtt') -or $Relay -notin @('wsl_localhost','windows_private')) { throw 'Unknown measurement network profile.' }
    $delay = if ($Scenario -eq 'controlled_high_rtt') { 50 } else { 0 }
    return [pscustomobject]@{ useNamespace = ($delay -gt 0 -or $Relay -eq 'windows_private'); oneWayDelayMs = $delay; expectedRttMs = 2 * $delay }
}

function Get-M2WslPrivateAddress([string]$Distribution) {
    $route = & wsl.exe -d $Distribution -- ip -j -4 route get 1.1.1.1
    if ($LASTEXITCODE -ne 0) { throw 'Could not inspect the WSL route.' }
    $address = [string]@($route | ConvertFrom-Json)[0].prefsrc
    $parsed = $null
    if (-not [Net.IPAddress]::TryParse($address, [ref]$parsed) -or $parsed.AddressFamily -ne [Net.Sockets.AddressFamily]::InterNetwork) { throw 'WSL has no usable private IPv4 address.' }
    $bytes = $parsed.GetAddressBytes()
    if (-not ($bytes[0] -eq 10 -or ($bytes[0] -eq 172 -and $bytes[1] -ge 16 -and $bytes[1] -le 31) -or ($bytes[0] -eq 192 -and $bytes[1] -eq 168))) { throw 'The WSL relay requires an assigned private address.' }
    return $parsed.ToString()
}

function Stop-M2WindowsRelay($Relay) {
    if ($Relay.closed) { return }
    $Relay.closed = $true
    $forced = $false
    $failure = $null
    $report = $null
    try {
        if (-not $Relay.process.HasExited) {
            [IO.File]::WriteAllText($Relay.stopPath, $Relay.token, [Text.Encoding]::ASCII)
            if (-not $Relay.process.WaitForExit(5000)) {
                $forced = $true
                $Relay.process.Kill()
                if (-not $Relay.process.WaitForExit(5000)) { throw 'The owned relay did not exit.' }
            }
        }
        $output = $Relay.output.GetAwaiter().GetResult()
        $errors = $Relay.errors.GetAwaiter().GetResult()
        [IO.File]::WriteAllText((Join-Path $Relay.evidence 'windows-relay.txt'), $output + $errors, [Text.UTF8Encoding]::new($false))
        $match = [regex]::Match($output, '(?m)^M2_RELAY (\{[^\r\n]*\})\r?$')
        if ($match.Success) { $report = $match.Groups[1].Value | ConvertFrom-Json }
        if ($forced -or $Relay.process.ExitCode -ne 0 -or -not $report -or $report.activeClients -ne 0 -or $report.ownedWriters -ne 0 -or $report.peakClients -gt 16 -or $report.socketConfigurationFailures -ne 0) { throw 'The owned relay did not report a complete graceful shutdown.' }
    } catch { $failure = $_.Exception.Message } finally {
        $identity = [ordered]@{ schemaVersion = 1; owner = $Relay.token; forcedStop = $forced; passed = ($null -eq $failure); failure = $failure; counters = $report; performanceAcceptance = $false }
        [IO.File]::WriteAllText((Join-Path $Relay.evidence 'windows-relay-cleanup.json'), ($identity | ConvertTo-Json -Depth 6), [Text.UTF8Encoding]::new($false))
        $Relay.process.Dispose()
    }
    if ($failure) { throw $failure }
}

function Start-M2WindowsRelay([string]$Address, [int]$Port, [string]$Token, [string]$Workspace, [string]$Evidence) {
    if ($Token -cnotmatch '^[0-9a-f]{12}$' -or $Port -lt 1024 -or $Port -gt 65535) { throw 'Invalid relay fixture identity.' }
    $repository = (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot '..')).Path
    $workspacePath = (Resolve-Path -LiteralPath $Workspace).Path
    if ($workspacePath -ine (Join-Path $repository "target/m1-sshd-$Token")) { throw 'Use the owned disposable relay workspace.' }
    $private = Join-Path $workspacePath 'secrets'
    $ready = Join-Path $private "m2-relay-$Token.ready"
    $stop = Join-Path $private "m2-relay-$Token.stop"
    $python = @(Get-Command python -CommandType Application -ErrorAction Stop)[0].Source
    $proxy = Join-Path $repository 'scripts/m2-net-proxy.py'
    $arguments = @($proxy, $Port.ToString(), $Address, '--owner', $Token, '--ready-file', $ready, '--stop-file', $stop)
    $quoted = @($arguments | ForEach-Object {
        if ($_ -match '["\r\n]') { throw 'Invalid literal relay argument.' }
        '"' + $_ + '"'
    }) -join ' '
    $process = [Diagnostics.Process]::new()
    $process.StartInfo = [Diagnostics.ProcessStartInfo]::new()
    $process.StartInfo.FileName = $python
    $process.StartInfo.Arguments = $quoted
    $process.StartInfo.UseShellExecute = $false
    $process.StartInfo.CreateNoWindow = $true
    $process.StartInfo.RedirectStandardOutput = $true
    $process.StartInfo.RedirectStandardError = $true
    $relay = $null
    try {
        if (-not $process.Start()) { throw 'The owned Windows relay did not start.' }
        $null = $process.SafeHandle
        $relay = [pscustomobject]@{ process = $process; output = $process.StandardOutput.ReadToEndAsync(); errors = $process.StandardError.ReadToEndAsync(); stopPath = $stop; token = $Token; evidence = $Evidence; closed = $false }
        $timer = [Diagnostics.Stopwatch]::StartNew()
        while (-not (Test-Path -LiteralPath $ready -PathType Leaf)) {
            if ($process.HasExited -or $timer.Elapsed.TotalSeconds -ge 10) { throw 'The owned Windows relay failed readiness.' }
            Start-Sleep -Milliseconds 25
        }
        if ([IO.File]::ReadAllText($ready) -cne $Token) { throw 'The Windows relay readiness owner differs.' }
        $identity = [ordered]@{ schemaVersion = 1; route = 'Windows loopback -> WSL assigned private IPv4 -> owned netem namespace'; address = $Address; port = $Port; pid = $process.Id; startedAtUtc = $process.StartTime.ToUniversalTime().ToString('o'); python = $python; pythonVersion = (& $python --version) -join ''; proxySha256 = (Get-FileHash -LiteralPath $proxy).Hash; maximumClients = 16; chunkBytes = 65536; performanceAcceptance = $false }
        [IO.File]::WriteAllText((Join-Path $Evidence 'windows-relay.json'), ($identity | ConvertTo-Json -Depth 5), [Text.UTF8Encoding]::new($false))
        return $relay
    } catch {
        if ($relay) { Stop-M2WindowsRelay $relay } else { $process.Dispose() }
        throw
    }
}
