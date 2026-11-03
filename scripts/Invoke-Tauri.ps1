param(
    [Parameter(ValueFromRemainingArguments = $true)]
    [string[]]$TauriArguments
)

$ErrorActionPreference = 'Stop'
$repositoryRoot = (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot '..')).Path
$cliPath = Join-Path $repositoryRoot 'ui/node_modules/.bin/tauri.cmd'
if (-not (Test-Path -LiteralPath $cliPath -PathType Leaf)) {
    throw 'Install the frozen UI dependencies before invoking Tauri CLI.'
}

Push-Location $repositoryRoot
try {
    & $cliPath @TauriArguments
    exit $LASTEXITCODE
} finally {
    Pop-Location
}
