param([string]$Workspace)
$ErrorActionPreference = 'Stop'
$repository = (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot '..')).Path
$target = (Resolve-Path -LiteralPath (Join-Path $repository 'target')).Path
if ([string]::IsNullOrWhiteSpace($Workspace)) {
    $current = Get-Content -LiteralPath (Join-Path $target 'm2-manual-current.json') -Raw | ConvertFrom-Json
    if ($current.purpose -ne 'M2-A-manual-review') { throw 'Invalid current fixture pointer.' }
    $Workspace = $current.workspace
}
$root = (Resolve-Path -LiteralPath $Workspace).Path
if ([IO.Path]::GetDirectoryName($root) -ne $target -or [IO.Path]::GetFileName($root) -notmatch '^m1-sshd-[0-9a-f]{12}$' -or
    ((Get-Item -LiteralPath $root -Force).Attributes -band [IO.FileAttributes]::ReparsePoint)) { throw 'Invalid manual fixture root.' }
$state = Get-Content -LiteralPath (Join-Path $root 'manual-state.json') -Raw | ConvertFrom-Json
if ($state.purpose -ne 'M2-A-manual-review' -or $state.token -ne [IO.Path]::GetFileName($root).Substring(8)) { throw 'Invalid manual ownership record.' }
$manifest = Get-Content -LiteralPath (Join-Path $root 'file-manifest.json') -Raw | ConvertFrom-Json
if ($manifest.Count -ne 4) { throw 'The manual fixture requires exactly four manifest entries.' }
$results = foreach ($item in $manifest) {
    if ([IO.Path]::GetFileName($item.name) -ne $item.name -or $item.name -in @('.', '..') -or $item.sha256 -notmatch '^[0-9A-F]{64}$') { throw 'Invalid manual manifest entry.' }
    $source = Join-Path (Join-Path $root 'files') $item.name
    $download = Join-Path (Join-Path $root 'downloads') $item.name
    $sourceMatched = (Get-FileHash -LiteralPath $source).Hash -eq $item.sha256
    $downloadMatched = (Test-Path -LiteralPath $download -PathType Leaf) -and
        (Get-FileHash -LiteralPath $download).Hash -eq $item.sha256 -and (Get-Item -LiteralPath $download).Length -eq $item.bytes
    [pscustomobject][ordered]@{ name = $item.name; bytes = $item.bytes; sourceMatched = $sourceMatched; downloadMatched = $downloadMatched }
}
$passed = $results.Count -eq 4 -and @($results | Where-Object { -not $_.sourceMatched -or -not $_.downloadMatched }).Count -eq 0
[ordered]@{ schemaVersion = 1; files = @($results); completeHashesMatched = $passed; desktopAcceptance = $false } |
    ConvertTo-Json -Depth 5 | Set-Content -LiteralPath (Join-Path $root 'manual-file-check.json') -Encoding utf8
$results | Format-Table name,bytes,sourceMatched,downloadMatched
if (-not $passed) { throw 'Download all four fixture files with their original names, then rerun. Preserve the report on a mismatch.' }
