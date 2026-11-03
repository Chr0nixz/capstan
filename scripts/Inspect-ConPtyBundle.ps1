param([Parameter(Mandatory = $true)][string]$RuntimeDirectory)
$ErrorActionPreference = 'Stop'
$root = (Resolve-Path -LiteralPath $RuntimeDirectory).Path
$files = @(Get-ChildItem -LiteralPath $root -File -Recurse | Where-Object { $_.Extension -in @('.dll', '.exe') })
if ($files.Count -eq 0) { throw 'No runtime binaries found. Bundle acceptance remains blocked.' }
foreach ($file in $files) {
    $reader = [IO.BinaryReader]::new([IO.File]::OpenRead($file.FullName))
    try {
        if ($reader.BaseStream.Length -lt 64 -or $reader.ReadUInt16() -ne 0x5a4d) { throw 'Invalid DOS header.' }
        $reader.BaseStream.Position = 60
        $offset = $reader.ReadUInt32()
        if ($offset -gt $reader.BaseStream.Length - 6) { throw 'Invalid PE offset.' }
        $reader.BaseStream.Position = $offset
        if ($reader.ReadUInt32() -ne 0x00004550) { throw 'Invalid PE signature.' }
        $machine = $reader.ReadUInt16()
    } finally { $reader.Dispose() }
    $signature = Get-AuthenticodeSignature -LiteralPath $file.FullName
    [pscustomobject][ordered]@{
        file = $file.Name; architecture = switch ($machine) { 0x8664 { 'x64' } 0xaa64 { 'arm64' } default { 'unsupported' } }
        bytes = $file.Length; sha256 = (Get-FileHash -LiteralPath $file.FullName -Algorithm SHA256).Hash
        file_version = $file.VersionInfo.FileVersion; signature_status = [string]$signature.Status
        signer = if ($null -ne $signature.SignerCertificate) { $signature.SignerCertificate.Subject } else { $null }
        redistribution_review = 'required'; package_version_review = 'required'; companion_files_review = 'required'
    }
}
