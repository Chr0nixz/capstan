param([string]$EvidenceDirectory)
$ErrorActionPreference='Stop'
. (Join-Path $PSScriptRoot 'ConPty.ps1')
$repository=(Resolve-Path -LiteralPath (Join-Path $PSScriptRoot '..')).Path
$source=Join-Path $repository 'target/runtime/x86_64'
$target=(Resolve-Path -LiteralPath (Join-Path $repository 'target')).Path
$stage=Join-Path $target ('conpty-negative-'+[Guid]::NewGuid().ToString('N'))
$passed=[Collections.Generic.List[string]]::new()
function Reject([string]$Name, [scriptblock]$Action) {
    try { & $Action | Out-Null } catch { $passed.Add($Name); return }
    throw "Runtime validation unexpectedly accepted: $Name"
}
try {
    Copy-ConPtyRuntime $source $stage
    $passed.Add('complete-signed-package')
    $dll=Join-Path $stage 'conpty.dll'
    [IO.File]::WriteAllBytes($dll,@(0,1,2))
    Reject 'changed-dll' { Assert-ConPtyRuntime $stage }
    Copy-Item -LiteralPath (Join-Path $source 'conpty.dll') -Destination $dll
    $companion=Join-Path $stage 'arm64/OpenConsole.exe'
    Remove-Item -LiteralPath $companion
    Reject 'missing-host-companion' { Assert-ConPtyRuntime $stage }
    Copy-Item -LiteralPath (Join-Path $source 'arm64/OpenConsole.exe') -Destination $companion
    $manifest=Join-Path $stage 'manifest.json'
    [IO.File]::WriteAllText($manifest,'{"package":"unknown"}')
    Reject 'changed-manifest' { Assert-ConPtyRuntime $stage }
    Copy-Item -LiteralPath (Join-Path $source 'manifest.json') -Destination $manifest
    Remove-Item -LiteralPath (Join-Path $stage 'LICENSE.txt')
    Reject 'missing-license' { Assert-ConPtyRuntime $stage }
} finally {
    $resolved=[IO.Path]::GetFullPath($stage)
    if (!$resolved.StartsWith($target.TrimEnd('\')+'\',[StringComparison]::OrdinalIgnoreCase)) { throw 'Invalid runtime test cleanup path.' }
    if (Test-Path -LiteralPath $stage) { Remove-Item -LiteralPath $stage -Recurse -Force }
}
$report=@{ schema=1; passed=$true; scenarios=$passed.ToArray(); package_version=(Get-ConPtyLock).package_version }
if ($EvidenceDirectory) {
    New-Item -ItemType Directory -Path $EvidenceDirectory -Force | Out-Null
    [IO.File]::WriteAllText((Join-Path $EvidenceDirectory 'validation-tests.json'),($report|ConvertTo-Json),[Text.UTF8Encoding]::new($false))
}
Write-Output "ConPTY runtime validation: $($passed.Count) scenarios passed."
