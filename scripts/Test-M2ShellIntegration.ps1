param([Parameter(Mandatory = $true)][string]$EvidenceDirectory)
$ErrorActionPreference = 'Stop'
$repository = (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot '..')).Path
$evidence = [IO.Path]::GetFullPath($EvidenceDirectory)
if (Test-Path -LiteralPath (Join-Path $evidence 'shells/report.json')) { throw 'Do not overwrite shell evidence.' }
$fixtures = Join-Path $evidence 'fixtures'
New-Item -ItemType Directory -Path $fixtures -Force | Out-Null
$previous = $env:CAPSTAN_TEST_SHELL_INTEGRATION
Push-Location $repository
try {
    $env:CAPSTAN_TEST_SHELL_INTEGRATION = $fixtures
    $ErrorActionPreference = 'Continue'
    cargo test -p capstan-core --lib shell_integration::tests::export_configured_shell_probe_fixtures_or_explicit_skip --offline --locked -- --nocapture *> (Join-Path $evidence 'fixture-export.txt')
    $code = $LASTEXITCODE
    $ErrorActionPreference = 'Stop'
    if ($code -ne 0) { throw 'Generated shell fixture failed.' }
} finally {
    $env:CAPSTAN_TEST_SHELL_INTEGRATION = $previous
    Pop-Location
}
function Wsl-Path([string]$Path) {
    $result = (& wsl.exe -d Ubuntu --exec wslpath -u $Path.Replace('\', '/') | Out-String).Trim()
    if ($LASTEXITCODE -ne 0) { throw 'WSL path conversion failed.' }
    return $result
}
$script = Wsl-Path (Join-Path $repository 'scripts/test-m2-shell-integration.py')
$fixturePath = Wsl-Path $fixtures
$outputPath = Wsl-Path (Join-Path $evidence 'shells')
$zsh = Wsl-Path (Join-Path $repository 'target/m2-shell-tools/root/bin/zsh')
$ErrorActionPreference = 'Continue'
& wsl.exe -d Ubuntu -- python3 $script --fixtures $fixturePath --evidence $outputPath --zsh $zsh *> (Join-Path $evidence 'shells.txt')
$code = $LASTEXITCODE
$ErrorActionPreference = 'Stop'
if ($code -ne 0) { throw 'Real shell cases failed; inspect preserved evidence.' }
Write-Output "Shell evidence: $evidence"
