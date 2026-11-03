param(
    [ValidateSet('en', 'zh-CN')][string]$Language = 'zh-CN',
    [switch]$SelfTest,
    [string]$ProbePath
)
$ErrorActionPreference = 'Stop'
$repository = (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot '..')).Path
if ([string]::IsNullOrWhiteSpace($ProbePath)) {
    $review = Get-Content -LiteralPath (Join-Path $repository 'target/m2-review-current.json') -Raw | ConvertFrom-Json
    $ProbePath = Join-Path $review.directory 'tools/m2_native_drag.exe'
}
$probe = (Resolve-Path -LiteralPath $ProbePath).Path
$workspace = Join-Path $repository ('target/m2-drag-' + [Guid]::NewGuid().ToString('N').Substring(0,12))
$staging = Join-Path $workspace 'staging'
$received = Join-Path $workspace 'received'
New-Item -ItemType Directory -Path $staging,$received -Force | Out-Null
foreach ($directory in @('folder with spaces/empty', 'received/single-file', 'received/folder', 'received/multiple')) {
    $parent = if ($directory.StartsWith('received/')) { $workspace } else { $staging }
    New-Item -ItemType Directory -Path (Join-Path $parent $directory) -Force | Out-Null
}
[IO.File]::WriteAllBytes((Join-Path $workspace 'probe.marker'), [Text.Encoding]::ASCII.GetBytes('CAPSTAN_M2_DRAG_PROBE_V1'))
[IO.File]::WriteAllText((Join-Path $staging 'sample.txt'), "Capstan native drag fixture`n", [Text.UTF8Encoding]::new($false))
$unicodeName = 'nested-' + [char]0x4e2d + [char]0x6587 + '.txt'
[IO.File]::WriteAllText((Join-Path $staging "folder with spaces/$unicodeName"), "Nested fixture`n", [Text.UTF8Encoding]::new($false))
$sourceManifest = @(
    [ordered]@{ relative = 'sample.txt'; sha256 = (Get-FileHash -LiteralPath (Join-Path $staging 'sample.txt')).Hash },
    [ordered]@{ relative = "folder with spaces/$unicodeName"; sha256 = (Get-FileHash -LiteralPath (Join-Path $staging "folder with spaces/$unicodeName")).Hash }
)
$sourceManifest | ConvertTo-Json -Depth 4 | Set-Content -LiteralPath (Join-Path $workspace 'source-manifest.json') -Encoding utf8
$log = Join-Path $workspace 'native-events.txt'
$code = 1
if ($SelfTest) {
    $ErrorActionPreference = 'Continue'
    try { & $probe $workspace $Language --self-test *> $log; $code = $LASTEXITCODE } finally { $ErrorActionPreference = 'Stop' }
} else {
    Write-Output "Drag probe workspace: $workspace"
    Write-Output 'In Explorer, open received. Drag each box to its matching folder, then perform one Escape cancellation. Close the probe to verify copied bytes.'
    # Explorer is visible because the maintainer must choose real drop targets.
    Start-Process explorer.exe -ArgumentList @($received) -WindowStyle Normal | Out-Null
    $ErrorActionPreference = 'Continue'
    try { & $probe $workspace $Language *> $log; $code = $LASTEXITCODE } finally { $ErrorActionPreference = 'Stop' }
}
$events = @(Get-Content -LiteralPath $log | Where-Object { $_.StartsWith('{') } | ForEach-Object { $_ | ConvertFrom-Json })
$checks = [Collections.Generic.List[object]]::new()
if ($SelfTest) {
    $checks.Add([ordered]@{ check = 'three-data-objects-prepared'; passed = ($events.Count -eq 1 -and $events[0].dataObjectsPrepared -eq 3 -and $events[0].interactiveDragExecuted -eq $false) })
}
foreach ($item in $sourceManifest) {
    $checks.Add([ordered]@{ check = "source:$($item.relative)"; passed = ((Get-FileHash -LiteralPath (Join-Path $staging $item.relative)).Hash -eq $item.sha256) })
}
if (-not $SelfTest) {
    foreach ($choice in 0..2) {
        $checks.Add([ordered]@{ check = "native-copy-event:$choice"; passed = @($events | Where-Object { $_.choice -eq $choice -and $_.outcome -eq 'copied' }).Count -gt 0 })
    }
    $expected = @(
        @{ target = 'single-file/sample.txt'; source = 'sample.txt' },
        @{ target = "folder/folder with spaces/$unicodeName"; source = "folder with spaces/$unicodeName" },
        @{ target = 'multiple/sample.txt'; source = 'sample.txt' },
        @{ target = "multiple/folder with spaces/$unicodeName"; source = "folder with spaces/$unicodeName" }
    )
    $deadline = [DateTime]::UtcNow.AddSeconds(10)
    do {
        $allPresent = @($expected | Where-Object { -not (Test-Path -LiteralPath (Join-Path $received $_.target) -PathType Leaf) }).Count -eq 0
        if (-not $allPresent) { Start-Sleep -Milliseconds 100 }
    } while (-not $allPresent -and [DateTime]::UtcNow -lt $deadline)
    foreach ($item in $expected) {
        $path = Join-Path $received $item.target
        $hashMatched = (Test-Path -LiteralPath $path -PathType Leaf) -and
            ((Get-FileHash -LiteralPath $path).Hash -eq (Get-FileHash -LiteralPath (Join-Path $staging $item.source)).Hash)
        $checks.Add([ordered]@{ check = "copied:$($item.target)"; passed = $hashMatched })
    }
    foreach ($directory in @('folder/folder with spaces/empty', 'multiple/folder with spaces/empty')) {
        $checks.Add([ordered]@{ check = "empty-directory:$directory"; passed = (Test-Path -LiteralPath (Join-Path $received $directory) -PathType Container) })
    }
    $checks.Add([ordered]@{ check = 'escape-cancellation-recorded'; passed = @($events | Where-Object outcome -eq 'canceled').Count -gt 0 })
}
$passed = $code -eq 0 -and @($checks | Where-Object { -not $_.passed }).Count -eq 0
[ordered]@{
    schemaVersion = 1; probeSha256 = (Get-FileHash -LiteralPath $probe).Hash; exitCode = $code
    selfTestOnly = [bool]$SelfTest; interactiveDragExecuted = -not [bool]$SelfTest
    checks = $checks.ToArray(); platformProbePassed = ($passed -and -not $SelfTest)
    automaticSetupPassed = $passed; desktopAcceptance = $false; performanceAcceptance = $false
    limitation = 'Staged local-copy feasibility only. No SFTP transfer, export grants or production staging lifecycle are implemented by this probe.'
} | ConvertTo-Json -Depth 6 | Set-Content -LiteralPath (Join-Path $workspace 'report.json') -Encoding utf8
Write-Output "Probe evidence: $workspace"
if (-not $passed) { throw 'Native drag probe checks failed; retain its report and events.' }
