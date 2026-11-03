param(
    [Parameter(Mandatory = $true)][string]$EvidenceDirectory,
    [ValidateSet('all', 'rust', 'ui', 'native')][string]$Group = 'all'
)
$ErrorActionPreference = 'Stop'
$repository = (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot '..')).Path
$evidence = [IO.Path]::GetFullPath($EvidenceDirectory)
New-Item -ItemType Directory -Path $evidence -Force | Out-Null
$results = [Collections.Generic.List[object]]::new()
function Invoke-Check([string]$Name, [string]$Command, [scriptblock]$Action) {
    $timer = [Diagnostics.Stopwatch]::StartNew()
    $log = Join-Path $evidence "$Name.txt"
    $code = 1
    $global:LASTEXITCODE = 0
    try {
        $ErrorActionPreference = 'Continue'
        & $Action *> $log
        $code = $LASTEXITCODE
    } catch { $_ | Out-String | Add-Content -LiteralPath $log } finally { $ErrorActionPreference = 'Stop' }
    $timer.Stop()
    $results.Add([ordered]@{ name = $Name; command = $Command; exitCode = $code; elapsedMs = $timer.ElapsedMilliseconds; log = "$Name.txt" })
    Write-Output "$Name : exit $code"
}
Push-Location $repository
try {
    if ($Group -in @('all', 'rust')) {
        Invoke-Check 'fmt' 'cargo fmt --all -- --check' { cargo fmt --all -- --check }
        Invoke-Check 'clippy' 'cargo clippy --workspace --all-targets --offline --locked -- -D warnings' { cargo clippy --workspace --all-targets --offline --locked -- -D warnings }
        Invoke-Check 'clippy-all-features' 'cargo clippy --workspace --all-targets --all-features --offline --locked -- -D warnings' { cargo clippy --workspace --all-targets --all-features --offline --locked -- -D warnings }
        Invoke-Check 'workspace-tests' 'cargo test --workspace --offline --locked -- --nocapture' { cargo test --workspace --offline --locked -- --nocapture }
        Invoke-Check 'runtime-tests' 'cargo test -p capstan-app --features tauri-runtime --offline --locked' { cargo test -p capstan-app --features tauri-runtime --offline --locked }
        Invoke-Check 'all-feature-tests' 'cargo test --workspace --all-features --offline --locked -- --nocapture' { cargo test --workspace --all-features --offline --locked -- --nocapture }
        Invoke-Check 'runtime-check' 'cargo check -p capstan-app --features tauri-runtime --offline --locked' { cargo check -p capstan-app --features tauri-runtime --offline --locked }
        Invoke-Check 'cargo-deny' 'cargo deny check all' { cargo deny check all }
        Invoke-Check 'licenses' 'powershell -NoProfile -File scripts/Check-Licenses.ps1' { powershell -NoProfile -File scripts/Check-Licenses.ps1 }
    }
    if ($Group -in @('all', 'native')) {
        Invoke-Check 'native-fmt' 'cargo fmt --manifest-path vendor/capstan-native-windows/Cargo.toml -- --check' { cargo fmt --manifest-path vendor/capstan-native-windows/Cargo.toml -- --check }
        Invoke-Check 'native-clippy' 'cargo clippy --manifest-path vendor/capstan-native-windows/Cargo.toml --all-targets --offline --locked -- -D warnings' { cargo clippy --manifest-path vendor/capstan-native-windows/Cargo.toml --all-targets --offline --locked -- -D warnings }
        Invoke-Check 'native-tests' 'cargo test --manifest-path vendor/capstan-native-windows/Cargo.toml --offline --locked' { cargo test --manifest-path vendor/capstan-native-windows/Cargo.toml --offline --locked }
    }
    if ($Group -in @('all', 'ui')) {
        Invoke-Check 'ui-types' 'pnpm -C ui typecheck' { pnpm -C ui typecheck }
        Invoke-Check 'ui-tests' 'pnpm -C ui test' { pnpm -C ui test }
        Invoke-Check 'ui-build' 'pnpm -C ui build' { pnpm -C ui build }
        Invoke-Check 'ui-layout' 'pnpm -C ui test:layout' { pnpm -C ui test:layout }
        Invoke-Check 'ui-sftp-layout' 'pnpm -C ui test:sftp-layout' { pnpm -C ui test:sftp-layout }
        Invoke-Check 'ui-security' 'pnpm -C ui test:security' { pnpm -C ui test:security }
    }
} finally {
    Pop-Location
    [ordered]@{ schemaVersion = 1; recordedAtUtc = [DateTime]::UtcNow.ToString('o'); group = $Group; checks = $results.ToArray() } |
        ConvertTo-Json -Depth 5 | Set-Content -LiteralPath (Join-Path $evidence "checks-$Group.json") -Encoding utf8
}
if (@($results | Where-Object { $_.exitCode -ne 0 }).Count) { throw 'M2 engineering checks failed; inspect the preserved logs.' }
