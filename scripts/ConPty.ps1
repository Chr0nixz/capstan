# Shared validation for downloaded, installed and portable runtime payloads.
function Get-ConPtyLock {
    $repository = (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot '..')).Path
    return Get-Content -LiteralPath (Join-Path $repository 'resources/conpty.lock.json') -Raw | ConvertFrom-Json
}

function Assert-ConPtyRuntime([string]$Directory, [switch]$SkipManifest) {
    $lock = Get-ConPtyLock
    $directoryPath = (Resolve-Path -LiteralPath $Directory).Path
    if (!$SkipManifest) {
        $manifest = Get-Content -LiteralPath (Join-Path $directoryPath 'manifest.json') -Raw | ConvertFrom-Json
        if ($manifest.package -ne $lock.package -or $manifest.package_version -ne $lock.package_version -or $manifest.package_sha256 -ne $lock.package_sha256 -or $manifest.architecture -ne $lock.runtime_architecture) {
            throw 'ConPTY runtime manifest does not match the pinned package.'
        }
    }
    $results = @()
    foreach ($file in $lock.files) {
        if ($file.destination -notmatch '^(conpty\.dll|(x64|arm64)/OpenConsole\.exe)$') { throw 'Unexpected runtime destination.' }
        $path = Join-Path $directoryPath $file.destination
        if ((Get-FileHash -LiteralPath $path -Algorithm SHA256).Hash -ne $file.sha256) { throw 'ConPTY payload hash mismatch.' }
        $reader = [IO.BinaryReader]::new([IO.File]::OpenRead($path))
        try {
            if ($reader.BaseStream.Length -lt 64 -or $reader.ReadUInt16() -ne 0x5a4d) { throw 'Invalid runtime DOS header.' }
            $reader.BaseStream.Position = 60
            $offset = $reader.ReadUInt32()
            if ($offset -lt 64 -or $offset -gt $reader.BaseStream.Length - 6) { throw 'Invalid runtime PE offset.' }
            $reader.BaseStream.Position = $offset
            if ($reader.ReadUInt32() -ne 0x00004550) { throw 'Invalid runtime PE signature.' }
            $expected = if ($file.architecture -eq 'x64') { 0x8664 } else { 0xaa64 }
            if ($reader.ReadUInt16() -ne $expected) { throw 'Runtime architecture mismatch.' }
        } finally { $reader.Dispose() }
        $signature = Get-AuthenticodeSignature -LiteralPath $path
        if ($signature.Status -ne 'Valid' -or $signature.SignerCertificate.Subject -notmatch '^CN=Microsoft Corporation,') {
            throw 'Runtime requires a valid Microsoft signature.'
        }
        $item = Get-Item -LiteralPath $path
        if ($item.VersionInfo.FileVersion -ne $lock.file_version) { throw 'Runtime file version mismatch.' }
        $results += [ordered]@{
            file = $file.destination; architecture = $file.architecture; sha256 = $file.sha256
            bytes = $item.Length; file_version = $item.VersionInfo.FileVersion
            signature = [string]$signature.Status; signer = $signature.SignerCertificate.Subject
        }
    }
    $repository = (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot '..')).Path
    if ((Get-FileHash -LiteralPath (Join-Path $directoryPath 'LICENSE.txt')).Hash -ne (Get-FileHash -LiteralPath (Join-Path $repository 'resources/ConPTY-LICENSE.txt')).Hash) {
        throw 'Runtime MIT license text is missing or changed.'
    }
    return $results
}

function Copy-ConPtyRuntime([string]$Source, [string]$Destination) {
    Assert-ConPtyRuntime $Source | Out-Null
    $lock = Get-ConPtyLock
    foreach ($name in @($lock.files | ForEach-Object { $_.destination }) + @('LICENSE.txt', 'manifest.json')) {
        $targetFile = Join-Path $Destination $name
        New-Item -ItemType Directory -Path (Split-Path -Parent $targetFile) -Force | Out-Null
        $sourceFile = Join-Path $Source $name
        if (-not (Test-Path -LiteralPath $targetFile) -or (Get-FileHash -LiteralPath $sourceFile).Hash -ne (Get-FileHash -LiteralPath $targetFile).Hash) {
            Copy-Item -LiteralPath $sourceFile -Destination $targetFile
        }
    }
    Assert-ConPtyRuntime $Destination | Out-Null
}
