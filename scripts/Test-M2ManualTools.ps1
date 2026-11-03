param([Parameter(Mandatory = $true)][string]$EvidenceDirectory)
$ErrorActionPreference = 'Stop'
$repository = (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot '..')).Path
$evidence = [IO.Path]::GetFullPath($EvidenceDirectory)
New-Item -ItemType Directory -Path $evidence -Force | Out-Null
$checks = [Collections.Generic.List[object]]::new()
function Run-Tool([string]$Name, [string]$File, [string[]]$Arguments, [bool]$ExpectedSuccess) {
    $ErrorActionPreference = 'Continue'
    try {
        & powershell -NoProfile -File (Join-Path $repository $File) @Arguments *> (Join-Path $evidence "$Name.txt")
        $code = $LASTEXITCODE
    } finally { $ErrorActionPreference = 'Stop' }
    $passed = if ($ExpectedSuccess) { $code -eq 0 } else { $code -ne 0 }
    $checks.Add([ordered]@{ name = $Name; actualExitCode = $code; expectedSuccess = $ExpectedSuccess; passed = $passed })
    if (-not $passed) { throw "Manual tool check failed: $Name" }
}
Push-Location $repository
$failure = $null
$workspace = $null
try {
    Run-Tool 'prepare-smoke' 'scripts/Prepare-M2Manual.ps1' @('-Smoke', '-NoLaunch') $true
    $workspace = (Get-Content -LiteralPath target/m2-manual-current.json -Raw | ConvertFrom-Json).workspace
    $statePath = Join-Path $workspace 'manual-state.json'
    $state = Get-Content -LiteralPath $statePath -Raw | ConvertFrom-Json
    if ($state.active -or (Test-Path -LiteralPath (Join-Path $workspace 'secrets')) -or
        -not (Test-Path -LiteralPath (Join-Path $workspace 'desktop/data/vault.capstan'))) { throw 'Smoke cleanup or encrypted Vault creation failed.' }
    $state | Add-Member -NotePropertyName automatedToolingSelfTest -NotePropertyValue $true
    $state | ConvertTo-Json -Depth 5 | Set-Content -LiteralPath $statePath -Encoding utf8
    Run-Tool 'missing-downloads-rejected' 'scripts/Test-M2ManualFiles.ps1' @('-Workspace', $workspace) $false
    Copy-Item -LiteralPath (Join-Path $workspace 'manual-file-check.json') -Destination (Join-Path $evidence 'missing-files.json')
    foreach ($file in Get-ChildItem -LiteralPath (Join-Path $workspace 'files') -File) {
        Copy-Item -LiteralPath $file.FullName -Destination (Join-Path $workspace 'downloads')
    }
    # This is deliberately a local verifier test, not a fabricated SFTP round trip.
    Run-Tool 'local-copy-verifier-passes' 'scripts/Test-M2ManualFiles.ps1' @('-Workspace', $workspace) $true
    Copy-Item -LiteralPath (Join-Path $workspace 'manual-file-check.json') -Destination (Join-Path $evidence 'local-copy-only.json')
    [IO.File]::WriteAllText((Join-Path $workspace 'downloads/sample.txt'), 'deliberately corrupted verifier input')
    Run-Tool 'corrupted-download-rejected' 'scripts/Test-M2ManualFiles.ps1' @('-Workspace', $workspace) $false
    Copy-Item -LiteralPath (Join-Path $workspace 'manual-file-check.json') -Destination (Join-Path $evidence 'corruption.json')
    Run-Tool 'stop-idempotent' 'scripts/Prepare-M2Manual.ps1' @('-Action', 'Stop', '-Workspace', $workspace) $true
    Run-Tool 'outside-cleanup-rejected' 'scripts/Prepare-M2Manual.ps1' @('-Action', 'Stop', '-Workspace', $repository) $false
    Run-Tool 'native-data-object-setup' 'scripts/Test-M2NativeDrag.ps1' @('-SelfTest') $true
} catch { $failure = $_.Exception.Message } finally {
    Pop-Location
    [ordered]@{
        schemaVersion = 1; checks = $checks.ToArray(); passed = ($null -eq $failure); failure = $failure
        workspace = $workspace; scope = 'Automated fixture/cleanup/verifier/data-object setup only; local copies are not SFTP evidence.'
        desktopAcceptance = $false; platformDragAcceptance = $false; performanceAcceptance = $false
    } | ConvertTo-Json -Depth 6 | Set-Content -LiteralPath (Join-Path $evidence 'report.json') -Encoding utf8
}
if ($failure) { throw $failure }
