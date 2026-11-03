param(
    [ValidateRange(3, 100)][int]$Samples = 7,
    [string]$EvidenceDirectory
)

$ErrorActionPreference = 'Stop'
$repository = (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot '..')).Path
if ([string]::IsNullOrWhiteSpace($EvidenceDirectory)) {
    $EvidenceDirectory = Join-Path $repository ('docs/agent/baselines/M1/raw/' + (Get-Date -Format 'yyyy-MM-dd-HHmmss') + '-kdf')
}
$evidence = [IO.Path]::GetFullPath($EvidenceDirectory)
New-Item -ItemType Directory -Path $evidence -Force | Out-Null
Push-Location $repository
try {
    & cargo build -p capstan-vault --release --offline --example measure_kdf
    if ($LASTEXITCODE -ne 0) { throw 'Could not build the release KDF measurement example.' }
    $binary = Join-Path $repository 'target/release/examples/measure_kdf.exe'
    $profiles = @(
        @{ name = 'minimum'; memory = 19456; iterations = 2; parallelism = 1 },
        @{ name = 'default'; memory = 65536; iterations = 3; parallelism = 1 },
        @{ name = 'maximum-memory'; memory = 262144; iterations = 4; parallelism = 1 },
        @{ name = 'maximum-iterations'; memory = 65536; iterations = 6; parallelism = 1 },
        @{ name = 'maximum-parallelism'; memory = 65536; iterations = 3; parallelism = 4 }
    )
    $measurements = @()
    foreach ($profile in $profiles) {
        $info = [Diagnostics.ProcessStartInfo]::new()
        $info.FileName = $binary
        $info.Arguments = "$($profile.memory) $($profile.iterations) $($profile.parallelism) $Samples"
        $info.UseShellExecute = $false
        $info.CreateNoWindow = $true
        $info.RedirectStandardOutput = $true
        $info.RedirectStandardError = $true
        $process = [Diagnostics.Process]::new()
        $process.StartInfo = $info
        $peak = 0L
        try {
            $process.Start() | Out-Null
            $stdout = $process.StandardOutput.ReadToEndAsync()
            $stderr = $process.StandardError.ReadToEndAsync()
            do {
                $process.Refresh()
                try { $peak = [Math]::Max($peak, $process.PeakWorkingSet64) } catch [InvalidOperationException] {}
            } until ($process.WaitForExit(10))
            $output = $stdout.GetAwaiter().GetResult()
            $errorOutput = $stderr.GetAwaiter().GetResult()
            if ($process.ExitCode -ne 0) { throw "KDF measurement failed: $errorOutput" }
        } finally { $process.Dispose() }
        $measurement = $output | ConvertFrom-Json
        if ($measurement.elapsed_ms.Count -ne $Samples -or $peak -le 0) { throw 'Incomplete KDF measurement.' }
        $ordered = @($measurement.elapsed_ms | Sort-Object)
        $summary = [ordered]@{
            profile = $profile.name
            measurement = $measurement
            median_ms = $ordered[[int][Math]::Floor($Samples / 2)]
            p95_ms = $ordered[[int][Math]::Ceiling($Samples * 0.95) - 1]
            peak_working_set_bytes = $peak
        }
        $summary | ConvertTo-Json -Depth 6 | Set-Content -LiteralPath (Join-Path $evidence "$($profile.name).json") -Encoding UTF8
        $measurements += $summary
        Write-Output "$($profile.name): median $([Math]::Round($summary.median_ms, 2)) ms; peak $([Math]::Round($peak / 1MB, 2)) MiB"
    }
    $report = [ordered]@{
        schema_version = 1
        measured_at = (Get-Date).ToString('o')
        cpu = @(Get-CimInstance Win32_Processor | Select-Object Name,NumberOfCores,NumberOfLogicalProcessors)
        os = (Get-CimInstance Win32_OperatingSystem | Select-Object Version,BuildNumber)
        memory_kib = (Get-CimInstance Win32_OperatingSystem).TotalVisibleMemorySize
        binary_sha256 = (Get-FileHash -LiteralPath $binary -Algorithm SHA256).Hash
        build_profile = 'release'
        memory_measurement = 'OS peak working set queried every 10ms while the isolated process runs; includes initialization and warmup'
        measurements = $measurements
    }
    $report | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath (Join-Path $evidence 'report.json') -Encoding UTF8
    Write-Output "Evidence directory: $evidence"
} finally { Pop-Location }
