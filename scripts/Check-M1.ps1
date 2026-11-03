param([Parameter(Mandatory=$true)][string]$EvidenceDirectory, [string]$WslDistribution)
$ErrorActionPreference='Stop'
$repository=(Resolve-Path -LiteralPath (Join-Path $PSScriptRoot '..')).Path
$evidence=[IO.Path]::GetFullPath($EvidenceDirectory)
New-Item -ItemType Directory -Path $evidence -Force | Out-Null
$checks=[Collections.Generic.List[object]]::new()
$previousWsl=$env:CAPSTAN_TEST_WSL
$previousRuntime=$env:CAPSTAN_TEST_CONPTY_DIR
function Invoke-Check([string]$Name,[scriptblock]$Action) {
    $log=Join-Path $evidence "$Name.txt"
    $timer=[Diagnostics.Stopwatch]::StartNew()
    $exitCode=1
    $global:LASTEXITCODE=0
    try {
        $ErrorActionPreference='Continue'
        & $Action *> $log
        $exitCode=$LASTEXITCODE
    } catch { $exitCode=1; $_ | Out-String | Add-Content -LiteralPath $log } finally { $ErrorActionPreference='Stop' }
    $checks.Add(@{name=$Name;exit_code=$exitCode;elapsed_ms=$timer.ElapsedMilliseconds;log="$Name.txt"})
    Write-Output "$Name : exit $exitCode"
    if ($exitCode -ne 0) { throw "Check failed: $Name; inspect its saved log." }
}
Push-Location $repository
try {
    Invoke-Check 'fmt' { cargo fmt --all -- --check }
    Invoke-Check 'vendor-fmt' { cargo fmt --manifest-path vendor/ssh-key/Cargo.toml -- --check }
    Invoke-Check 'clippy-default' { cargo clippy --workspace --all-targets --offline -- -D warnings }
    Invoke-Check 'clippy-all-features' { cargo clippy --workspace --all-targets --all-features --offline -- -D warnings }
    Invoke-Check 'workspace-tests' { cargo test --workspace --offline }
    Invoke-Check 'runtime-tests' { cargo test -p capstan-app --features tauri-runtime --offline }
    Invoke-Check 'all-feature-tests' { cargo test --workspace --all-features --offline }
    Invoke-Check 'ui-types' { pnpm -C ui typecheck }
    Invoke-Check 'ui-tests' { pnpm -C ui test }
    Invoke-Check 'ui-security' { pnpm -C ui test:security }
    Invoke-Check 'cargo-deny' { cargo deny check all }
    Invoke-Check 'licenses' { & ./scripts/Check-Licenses.ps1 }
    Invoke-Check 'm0-tools' { & ./scripts/Test-M0Tools.ps1 }
    if ($WslDistribution) {
        $env:CAPSTAN_TEST_WSL=$WslDistribution
        $env:CAPSTAN_TEST_CONPTY_DIR=Join-Path $repository 'target/runtime/x86_64'
        Invoke-Check 'bundled-wsl-powershell' { cargo test -p capstan-pty --offline --test local_profiles -- --nocapture }
    }
} finally {
    if ($null -eq $previousWsl) { Remove-Item Env:CAPSTAN_TEST_WSL -ErrorAction SilentlyContinue } else { $env:CAPSTAN_TEST_WSL=$previousWsl }
    if ($null -eq $previousRuntime) { Remove-Item Env:CAPSTAN_TEST_CONPTY_DIR -ErrorAction SilentlyContinue } else { $env:CAPSTAN_TEST_CONPTY_DIR=$previousRuntime }
    Pop-Location
    [IO.File]::WriteAllText((Join-Path $evidence 'checks.json'),(@{schema=1;recorded_at_utc=[DateTime]::UtcNow.ToString('o');checks=$checks.ToArray()}|ConvertTo-Json -Depth 5),[Text.UTF8Encoding]::new($false))
}
