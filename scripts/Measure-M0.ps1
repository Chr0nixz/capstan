param(
    [string]$Executable = (Join-Path $PSScriptRoot '../target/debug/capstan-app.exe'),
    [Parameter(Mandatory = $true)][string]$OutputDirectory,
    [ValidateRange(100, 5000)][int]$IntervalMilliseconds = 1000,
    [ValidateRange(60, 7200)][int]$TimeoutSeconds = 3600,
    [switch]$Visible,
    [switch]$TransferLoad,
    [ValidateSet('loopback','controlled_high_rtt')][string]$NetworkScenario='loopback',
    [switch]$Smoke
)
function Get-M0ProcessResourceSample($Entry, [long]$Started, [scriptblock]$Lookup = { param($Id) Get-Process -Id $Id -ErrorAction Stop }) {
    $process = $null
    try {
        $process = & $Lookup ([int]$Entry.ProcessId)
        $null = $process.SafeHandle
        if ($process.StartTime.ToUniversalTime().Ticks -ne $Started) { return [ordered]@{ id = [int]$Entry.ProcessId; name = $Entry.Name; departed = 'identity_changed' } }
        $row = [ordered]@{
            id = [int]$Entry.ProcessId; name = $process.ProcessName
            working_set_bytes = $process.WorkingSet64; private_bytes = $process.PrivateMemorySize64
            handles = $process.HandleCount; cpu_ms = $process.TotalProcessorTime.TotalMilliseconds
        }
        foreach ($counter in @('working_set_bytes','private_bytes','handles','cpu_ms')) {
            if ($null -eq $row[$counter]) { throw [InvalidOperationException]::new('A live resource counter is unavailable.') }
        }
        # Version metadata is not a resource counter and can disappear after
        # the counters were captured. Never discard valid counters for it.
        try { $row.version = $process.MainModule.FileVersionInfo.FileVersion } catch { $row.version = $null; $row.version_unavailable = $true }
        if ([string]::IsNullOrEmpty([string]$row.version)) { $row.version_unavailable = $true }
        return $row
    } catch {
        if ($null -ne $process -and $process.HasExited) { return [ordered]@{ id = [int]$Entry.ProcessId; name = $Entry.Name; departed = 'exited_during_sample' } }
        try {
            $current = & $Lookup ([int]$Entry.ProcessId)
            if ($current.HasExited -or $current.StartTime.ToUniversalTime().Ticks -ne $Started) { return [ordered]@{ id = [int]$Entry.ProcessId; name = $Entry.Name; departed = 'exited_or_reused_during_sample' } }
        } catch [ArgumentException] {
            return [ordered]@{ id = [int]$Entry.ProcessId; name = $Entry.Name; departed = 'exited_during_sample' }
        } catch {
            if ($_.FullyQualifiedErrorId -like 'NoProcessFoundForGivenId,*') { return [ordered]@{ id = [int]$Entry.ProcessId; name = $Entry.Name; departed = 'exited_during_sample' } }
        }
        return [ordered]@{ id = [int]$Entry.ProcessId; name = $Entry.Name; unmeasured = $true }
    }
}
$ErrorActionPreference = 'Stop'
$executablePath = (Resolve-Path -LiteralPath $Executable).Path
$capability = & $executablePath --m0-check
if ($LASTEXITCODE -ne 0 -or $capability -ne 'capstan-m0-benchmarks-v1') { throw 'Build the application with m0-benchmarks and custom-protocol before measurement.' }
$outputPath = [IO.Path]::GetFullPath($OutputDirectory)
if (Test-Path -LiteralPath $outputPath) { throw 'Use a new output directory; earlier evidence is never overwritten.' }
New-Item -ItemType Directory -Path $outputPath | Out-Null
$utf8 = [Text.UTF8Encoding]::new($false)
$previousDirectory = $env:CAPSTAN_M0_REPORT_DIR
$previousVisible = $env:CAPSTAN_M0_VISIBLE
$application = $null
$samples = [Collections.Generic.List[object]]::new()
$ownedIds = [Collections.Generic.HashSet[int]]::new()
$births = [Collections.Generic.Dictionary[int,long]]::new()
$clock = [Diagnostics.Stopwatch]::StartNew()
try {
    $env:CAPSTAN_M0_REPORT_DIR = $outputPath
    $env:CAPSTAN_M0_VISIBLE = if ($Visible) { '1' } else { '0' }
    # Hidden is the default; -Visible explicitly requests the device baseline UI.
    $windowStyle = if ($Visible) { 'Normal' } else { 'Hidden' }
    $application = Start-Process -FilePath $executablePath -WorkingDirectory (Split-Path $executablePath) -WindowStyle $windowStyle -RedirectStandardOutput (Join-Path $outputPath 'application-stdout.txt') -RedirectStandardError (Join-Path $outputPath 'application-stderr.txt') -PassThru
    $ownedIds.Add($application.Id) | Out-Null
    $births.Add($application.Id, $application.StartTime.ToUniversalTime().Ticks)
    $completedSamples = 0
    do {
        $tree = @(Get-CimInstance Win32_Process -Property ProcessId, ParentProcessId, Name)
        # Expired identities cannot authorize sampling or stopping a reused PID.
        foreach ($knownId in @($ownedIds)) {
            try {
                $known = Get-Process -Id $knownId -ErrorAction Stop
                if ($known.StartTime.ToUniversalTime().Ticks -ne $births[$knownId]) { $ownedIds.Remove($knownId) | Out-Null }
            } catch { $ownedIds.Remove($knownId) | Out-Null }
        }
        do {
            $added = $false
            foreach ($entry in $tree) {
                if ($ownedIds.Contains([int]$entry.ParentProcessId) -and !$ownedIds.Contains([int]$entry.ProcessId)) {
                    try {
                        $child = Get-Process -Id $entry.ProcessId -ErrorAction Stop
                        $births[[int]$entry.ProcessId] = $child.StartTime.ToUniversalTime().Ticks
                        $ownedIds.Add([int]$entry.ProcessId) | Out-Null
                        $added = $true
                    } catch { }
                }
            }
        } while ($added)
        $processes = @()
        $departed = @()
        foreach ($entry in $tree) {
            if (!$ownedIds.Contains([int]$entry.ProcessId)) { continue }
            $row = Get-M0ProcessResourceSample $entry $births[[int]$entry.ProcessId]
            if ($row.departed) { $departed += $row } else { $processes += $row }
        }
        $samples.Add([ordered]@{ elapsed_ms = $clock.Elapsed.TotalMilliseconds; processes = $processes; departed_processes = $departed })
        $application.Refresh()
        if ($application.HasExited) { throw 'Application exited before completing measurement.' }
        if (Test-Path -LiteralPath (Join-Path $outputPath 'terminal-report.json')) { $completedSamples++ }
        if ($clock.Elapsed.TotalSeconds -ge $TimeoutSeconds) { throw 'Benchmark timed out; retained process samples are incomplete evidence.' }
        Start-Sleep -Milliseconds $IntervalMilliseconds
    } while ($completedSamples -lt 3)
    if (-not $TransferLoad) {
        & (Join-Path $PSScriptRoot 'Verify-M0Report.ps1') -ReportPath (Join-Path $outputPath 'terminal-report.json')
    }
} finally {
    $env:CAPSTAN_M0_REPORT_DIR = $previousDirectory
    $env:CAPSTAN_M0_VISIBLE = $previousVisible
    try {
        [IO.File]::WriteAllText((Join-Path $outputPath 'process-samples.json'), (ConvertTo-Json -InputObject $samples.ToArray() -Depth 8), $utf8)
        $os = Get-CimInstance Win32_OperatingSystem -Property Caption, Version, BuildNumber, OSArchitecture
        $cpu = Get-CimInstance Win32_Processor -Property Name, NumberOfLogicalProcessors
        $computer = Get-CimInstance Win32_ComputerSystem -Property TotalPhysicalMemory
        $video = Get-CimInstance Win32_VideoController -Property Name, DriverVersion
        $environment = [ordered]@{
            schema = 1; measured_at_utc = [DateTime]::UtcNow.ToString('o'); requested_visible = [bool]$Visible
            sample_interval_ms = $IntervalMilliseconds
            os = [ordered]@{ caption = $os.Caption; version = $os.Version; build = $os.BuildNumber; architecture = $os.OSArchitecture }
            cpu = @($cpu | Select-Object Name, NumberOfLogicalProcessors)
            memory_bytes = $computer.TotalPhysicalMemory
            video = @($video | Select-Object Name, DriverVersion)
            executable_sha256 = (Get-FileHash -LiteralPath $executablePath -Algorithm SHA256).Hash
            fixture_sha256 = (Get-FileHash -LiteralPath (Join-Path (Split-Path $executablePath) 'capstan-m0-fixture.exe') -Algorithm SHA256).Hash
            cargo_lock_sha256 = (Get-FileHash -LiteralPath (Join-Path $PSScriptRoot '../Cargo.lock') -Algorithm SHA256).Hash
            ui_lock_sha256 = (Get-FileHash -LiteralPath (Join-Path $PSScriptRoot '../ui/pnpm-lock.yaml') -Algorithm SHA256).Hash
        }
        [IO.File]::WriteAllText((Join-Path $outputPath 'environment.json'), (ConvertTo-Json -InputObject $environment -Depth 8), $utf8)
    } finally {
        # Cleanup still runs if metadata or evidence persistence fails.
        # Only terminate this measurement's process tree, never a name-wide match.
        foreach ($processId in $ownedIds) {
            if ($null -ne $application -and $processId -ne $application.Id) {
                try {
                    $owned = Get-Process -Id $processId -ErrorAction Stop
                    $null = $owned.SafeHandle
                    if ($owned.StartTime.ToUniversalTime().Ticks -eq $births[$processId]) { Stop-Process -InputObject $owned -ErrorAction SilentlyContinue }
                } catch { }
            }
        }
        if ($null -ne $application -and !$application.HasExited) { Stop-Process -InputObject $application -ErrorAction SilentlyContinue }
    }
}
if ($TransferLoad) {
    & (Join-Path $PSScriptRoot 'Verify-M2Mixed.ps1') -RoundDirectory $outputPath -NetworkScenario $NetworkScenario -Smoke:$Smoke
}
