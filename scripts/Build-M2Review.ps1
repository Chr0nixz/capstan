param([string]$EvidenceDirectory)
$ErrorActionPreference = 'Stop'
$repository = (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot '..')).Path
. (Join-Path $PSScriptRoot 'ConPty.ps1')
if ([string]::IsNullOrWhiteSpace($EvidenceDirectory)) {
    $EvidenceDirectory = Join-Path $repository ('docs/agent/baselines/M2/raw/' + (Get-Date -Format 'yyyy-MM-dd-HHmmss') + '-review-build')
}
$evidence = [IO.Path]::GetFullPath($EvidenceDirectory)
New-Item -ItemType Directory -Path $evidence -Force | Out-Null
$checks = [Collections.Generic.List[object]]::new()
function Build-Step([string]$Name, [scriptblock]$Run) {
    $global:LASTEXITCODE = 0
    $log = Join-Path $evidence "$Name.txt"
    $ErrorActionPreference = 'Continue'
    try { & $Run *> $log; $code = $LASTEXITCODE } finally { $ErrorActionPreference = 'Stop' }
    $checks.Add([ordered]@{ name = $Name; exitCode = $code; log = "$Name.txt" })
    if ($code -ne 0) { throw "Review build failed: $Name; inspect $log" }
    Write-Output "$Name : exit $code"
}
$directory = Join-Path $repository ('target/m2-review-' + [Guid]::NewGuid().ToString('N').Substring(0,12))
$failure = $null
Push-Location $repository
try {
    Build-Step 'manual-helper' { cargo build -p capstan-core --example m2_manual_fixture --features benchmarks --release --offline --locked }
    Build-Step 'native-probe' { cargo build --manifest-path vendor/capstan-native-windows/Cargo.toml --example m2_native_drag --release --offline --locked }
    Build-Step 'production-desktop' { powershell -NoProfile -File scripts/Invoke-Tauri.ps1 build --no-bundle --features tauri-runtime,custom-protocol }
    $desktop = Join-Path $directory 'desktop'
    $tools = Join-Path $directory 'tools'
    New-Item -ItemType Directory -Path $desktop,$tools | Out-Null
    Copy-Item -LiteralPath (Join-Path $repository 'target/release/capstan-app.exe') -Destination (Join-Path $desktop 'Capstan.exe')
    Copy-ConPtyRuntime (Join-Path $repository 'target/runtime/x86_64') (Join-Path $desktop 'runtime/x86_64')
    [IO.File]::WriteAllBytes((Join-Path $desktop 'capstan.portable'), [Text.Encoding]::ASCII.GetBytes('CAPSTAN_PORTABLE_V1'))
    foreach ($name in @('LICENSE', 'THIRD_PARTY_NOTICES.md')) { Copy-Item -LiteralPath (Join-Path $repository $name) -Destination (Join-Path $desktop $name) }
    Copy-Item -LiteralPath (Join-Path $repository 'docs/PORTABLE.md') -Destination (Join-Path $desktop 'PORTABLE-README.md')
    Copy-Item -LiteralPath (Join-Path $repository 'target/release/examples/m2_manual_fixture.exe') -Destination $tools
    Copy-Item -LiteralPath (Join-Path $repository 'vendor/capstan-native-windows/target/release/examples/m2_native_drag.exe') -Destination $tools
    $files = foreach ($file in Get-ChildItem -LiteralPath $directory -File -Recurse) {
        [ordered]@{ path = $file.FullName.Substring($directory.Length + 1).Replace('\','/'); sha256 = (Get-FileHash -LiteralPath $file.FullName).Hash }
    }
    [ordered]@{
        schemaVersion = 1; purpose = 'M2-A-manual-review'; directory = $directory
        productionDesktopFeatures = @('tauri-runtime','custom-protocol'); instrumentationInDesktop = $false
        files = @($files); planSha256 = (Get-FileHash -LiteralPath docs/PLAN.md).Hash
        candidate = $false; desktopAcceptance = $false; performanceAcceptance = $false
    } | ConvertTo-Json -Depth 6 | Set-Content -LiteralPath (Join-Path $directory 'manifest.json') -Encoding utf8
    [ordered]@{ directory = $directory; manifestSha256 = (Get-FileHash -LiteralPath (Join-Path $directory 'manifest.json')).Hash } |
        ConvertTo-Json | Set-Content -LiteralPath (Join-Path $repository 'target/m2-review-current.json') -Encoding utf8
    Write-Output "Review build: $directory"
} catch { $failure = $_.Exception.Message } finally {
    Pop-Location
    [ordered]@{ checks = $checks.ToArray(); directory = $directory; passed = ($null -eq $failure); failure = $failure; candidate = $false } |
        ConvertTo-Json -Depth 5 | Set-Content -LiteralPath (Join-Path $evidence 'report.json') -Encoding utf8
}
if ($failure) { throw $failure }
