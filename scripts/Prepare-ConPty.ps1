param([string]$EvidenceDirectory)
$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot 'ConPty.ps1')
$repository = (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot '..')).Path
$lock = Get-ConPtyLock
$package = Join-Path $repository "target/conpty-source-$($lock.package_version).nupkg"
if (-not (Test-Path -LiteralPath $package)) {
    New-Item -ItemType Directory -Path (Split-Path -Parent $package) -Force | Out-Null
    Invoke-WebRequest -Uri $lock.package_url -OutFile $package
}
if ((Get-FileHash -LiteralPath $package -Algorithm SHA256).Hash -ne $lock.package_sha256) { throw 'Pinned NuGet package hash mismatch.' }
$runtime = Join-Path $repository "target/runtime/$($lock.runtime_architecture)"
New-Item -ItemType Directory -Path $runtime -Force | Out-Null
Add-Type -AssemblyName System.IO.Compression.FileSystem
$archive = [IO.Compression.ZipFile]::OpenRead($package)
try {
    foreach ($file in $lock.files) {
        if ($file.destination -notmatch '^(conpty\.dll|(x64|arm64)/OpenConsole\.exe)$') { throw 'Unexpected runtime destination.' }
        $entry = $archive.GetEntry($file.entry)
        if ($null -eq $entry -or $entry.Length -gt 16MB) { throw 'Missing or oversized runtime entry.' }
        $destination = Join-Path $runtime $file.destination
        New-Item -ItemType Directory -Path (Split-Path -Parent $destination) -Force | Out-Null
        [IO.Compression.ZipFileExtensions]::ExtractToFile($entry, $destination, $true)
    }
} finally { $archive.Dispose() }
Copy-Item -LiteralPath (Join-Path $repository 'resources/ConPTY-LICENSE.txt') -Destination (Join-Path $runtime 'LICENSE.txt')
$inventory = @(Assert-ConPtyRuntime $runtime -SkipManifest)
$manifest = [ordered]@{
    package = $lock.package; package_version = $lock.package_version
    package_url = $lock.package_url; package_sha256 = $lock.package_sha256
    architecture = $lock.runtime_architecture; license = $lock.license; license_source = $lock.license_source
    files = $inventory; redistribution_review = 'MIT license included; native NuGet host architecture layout retained'
}
[IO.File]::WriteAllText((Join-Path $runtime 'manifest.json'), ($manifest | ConvertTo-Json -Depth 6) + "`n", [Text.UTF8Encoding]::new($false))
foreach ($profile in @('debug','release')) {
    Copy-ConPtyRuntime $runtime (Join-Path $repository "target/$profile/runtime/$($lock.runtime_architecture)")
}
if ($EvidenceDirectory) {
    New-Item -ItemType Directory -Path $EvidenceDirectory -Force | Out-Null
    Copy-Item -LiteralPath (Join-Path $runtime 'manifest.json') -Destination (Join-Path $EvidenceDirectory 'conpty-package.json')
}
Write-Output "Verified Microsoft ConPTY $($lock.package_version) and native host companions: $runtime"
