param(
    [string]$ExecutablePath = (Join-Path $PSScriptRoot "../target/release/capstan-app.exe"),
    [string]$OutputPath
)

$ErrorActionPreference = 'Stop'
$repositoryRoot = (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot '..')).Path
. (Join-Path $PSScriptRoot 'ConPty.ps1')
$runtimeLock = Get-ConPtyLock
$targetRoot = [System.IO.Path]::GetFullPath((Join-Path $repositoryRoot 'target'))
$executable = [System.IO.Path]::GetFullPath($ExecutablePath)

if (-not (Test-Path -LiteralPath $executable -PathType Leaf)) {
    throw "Build the desktop executable first; expected: $executable"
}

$stream = [System.IO.File]::OpenRead($executable)
try {
    $reader = [System.IO.BinaryReader]::new($stream)
    if ($stream.Length -lt 256) { throw 'Executable is too small to be a valid PE file.' }
    $stream.Position = 0x3C
    $peOffset = $reader.ReadInt32()
    if ($peOffset -lt 64 -or $peOffset -gt $stream.Length - 6) { throw 'Executable has an invalid PE header offset.' }
    $stream.Position = $peOffset
    if ($reader.ReadUInt32() -ne 0x00004550) { throw 'Executable has an invalid PE signature.' }
    $architecture = switch ($reader.ReadUInt16()) {
        0x8664 { 'x64' }
        0xAA64 { 'arm64' }
        default { throw 'Portable packaging supports only verified Windows x64 or ARM64 executables.' }
    }
} finally {
    if ($reader) { $reader.Dispose() } else { $stream.Dispose() }
}

if ([string]::IsNullOrWhiteSpace($OutputPath)) {
    $version = (Get-Content -LiteralPath (Join-Path $repositoryRoot 'src-tauri/tauri.conf.json') -Raw | ConvertFrom-Json).version
    if ($version -cnotmatch '\A(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\z') { throw 'Expected a stable Tauri package version.' }
    $OutputPath = Join-Path $targetRoot "bundle/portable/capstan-$version-win-$architecture.zip"
}
$output = [System.IO.Path]::GetFullPath($OutputPath)
if (Test-Path -LiteralPath $output) {
    throw "Refusing to overwrite an existing portable archive: $output"
}

$stageRoot = Join-Path $targetRoot 'm1-portable-stage'
New-Item -ItemType Directory -Path $stageRoot -Force | Out-Null
$targetRootResolved = (Resolve-Path -LiteralPath $targetRoot).Path.TrimEnd('\') + '\'
$stageRootResolved = (Resolve-Path -LiteralPath $stageRoot).Path
if (-not $stageRootResolved.StartsWith($targetRootResolved, [System.StringComparison]::OrdinalIgnoreCase)) {
    throw 'Portable staging directory resolved outside the repository target directory.'
}
$stage = Join-Path $stageRoot ([Guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $stage | Out-Null

try {
    Copy-Item -LiteralPath $executable -Destination (Join-Path $stage 'Capstan.exe')
    [System.IO.File]::WriteAllBytes(
        (Join-Path $stage 'capstan.portable'),
        [System.Text.Encoding]::ASCII.GetBytes('CAPSTAN_PORTABLE_V1')
    )
    Copy-Item -LiteralPath (Join-Path $repositoryRoot 'docs/PORTABLE.md') -Destination (Join-Path $stage 'PORTABLE-README.md')
    Copy-Item -LiteralPath (Join-Path $repositoryRoot 'THIRD_PARTY_NOTICES.md') -Destination (Join-Path $stage 'THIRD_PARTY_NOTICES.md')
    Copy-Item -LiteralPath (Join-Path $repositoryRoot 'LICENSE') -Destination (Join-Path $stage 'LICENSE')
    $runtimeArchitecture = if ($architecture -eq 'x64') { 'x86_64' } else { 'aarch64' }
    if ($runtimeArchitecture -ne $runtimeLock.runtime_architecture) { throw 'Prepare and verify a runtime for this architecture before packaging.' }
    Copy-ConPtyRuntime (Join-Path $repositoryRoot "target/runtime/$runtimeArchitecture") (Join-Path $stage "runtime/$runtimeArchitecture")

    $outputDirectory = Split-Path -Parent $output
    New-Item -ItemType Directory -Path $outputDirectory -Force | Out-Null
    Compress-Archive -Path (Join-Path $stage '*') -DestinationPath $output -CompressionLevel Optimal
    $sha256 = [System.Security.Cryptography.SHA256]::Create()
    $archiveStream = [System.IO.File]::OpenRead($output)
    try {
        $hash = ([System.BitConverter]::ToString($sha256.ComputeHash($archiveStream))).Replace('-', '')
    } finally {
        $archiveStream.Dispose()
        $sha256.Dispose()
    }
    Write-Output "Portable archive: $output"
    Write-Output "SHA-256: $hash"
}
finally {
    if (Test-Path -LiteralPath $stage -PathType Container) {
        $stageFullPath = (Resolve-Path -LiteralPath $stage).Path
        $stageRootFullPath = (Resolve-Path -LiteralPath $stageRoot).Path.TrimEnd('\') + '\'
        if ($stageFullPath.StartsWith($stageRootFullPath, [System.StringComparison]::OrdinalIgnoreCase)) {
            Remove-Item -LiteralPath $stageFullPath -Recurse -Force
        }
    }
}
