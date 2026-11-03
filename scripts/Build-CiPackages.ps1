param([string]$OutputDirectory = (Join-Path $PSScriptRoot '../target/ci-packages'))

$ErrorActionPreference = 'Stop'
$repository = (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot '..')).Path
$releaseTools = Join-Path $PSScriptRoot 'release.py'

Push-Location $repository
try {
    $version = python $releaseTools version
    if ($LASTEXITCODE -ne 0) { throw 'Release version verification failed.' }

    pnpm tauri build --bundles nsis --features tauri-runtime,custom-protocol -- --locked
    if ($LASTEXITCODE -ne 0) { throw 'NSIS build failed.' }

    $executable = Get-Item -LiteralPath 'target/release/capstan-app.exe'
    if ($executable.VersionInfo.ProductVersion -cne $version) {
        throw 'Desktop executable version differs from the source version.'
    }
    $installer = "Capstan_${version}_x64-setup.exe"
    $packages = New-Item -ItemType Directory -Path $OutputDirectory
    Copy-Item -LiteralPath (Join-Path 'target/release/bundle/nsis' $installer) -Destination $packages.FullName
    $portable = Join-Path $packages.FullName "capstan-$version-win-x64.zip"
    & (Join-Path $PSScriptRoot 'Build-Portable.ps1') -ExecutablePath $executable.FullName -OutputPath $portable
    & (Join-Path $PSScriptRoot 'Test-M1Package.ps1') -ArchivePath $portable `
        -CandidateExecutable $executable.FullName -EvidenceDirectory 'target/ci-package-checks'

    python $releaseTools checksums $packages.FullName $version
    if ($LASTEXITCODE -ne 0) { throw 'Release checksum generation failed.' }
} finally {
    Pop-Location
}
