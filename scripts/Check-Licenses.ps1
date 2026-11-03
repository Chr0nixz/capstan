$ErrorActionPreference = 'Stop'
$repositoryRoot = (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot '..')).Path
Push-Location $repositoryRoot
try {
    $allowedLicenses = @(
        '0BSD', 'Apache-2.0', 'BSD-1-Clause', 'BSD-2-Clause', 'BSD-3-Clause', 'BlueOak-1.0.0',
        'CC-BY-4.0', 'CC0-1.0', 'GPL-3.0-only', 'GPL-3.0-or-later', 'ISC',
        'LGPL-2.1-or-later', 'MIT', 'MIT-0', 'MPL-2.0', 'Unicode-3.0', 'Unlicense', 'Zlib'
    )
    $allowedExceptions = @('LLVM-exception')
    $violations = [System.Collections.Generic.List[string]]::new()

    $projectLicensePath = Join-Path $repositoryRoot 'LICENSE'
    if (-not (Test-Path -LiteralPath $projectLicensePath -PathType Leaf)) {
        throw 'The GPL-3.0-or-later license text is missing.'
    }
    $licenseStream = [System.IO.File]::OpenRead($projectLicensePath)
    $sha256 = [System.Security.Cryptography.SHA256]::Create()
    try {
        $projectLicenseHash = ([System.BitConverter]::ToString($sha256.ComputeHash($licenseStream))).Replace('-', '')
    } finally {
        $licenseStream.Dispose()
        $sha256.Dispose()
    }
    if ($projectLicenseHash -ne 'FB981668C18A279E285FC4D83FBA1E836CC84DD4DAA73C9697D3CFD2D8ACA6E0') {
        throw 'The root LICENSE file does not match the reviewed SPDX GPL-3.0-or-later text.'
    }

    function Test-LicenseExpression([string]$name, [string]$version, [string]$expression, [string]$origin) {
        if ([string]::IsNullOrWhiteSpace($expression)) {
            $violations.Add("$origin ${name}@${version}: missing SPDX license")
            return
        }
        $tokens = [regex]::Split($expression, '\s+(?:OR|AND|WITH)\s+|/')
        foreach ($rawToken in $tokens) {
            $token = $rawToken.Trim().Trim('(', ')')
            if ($token -and $token -notin $allowedLicenses -and $token -notin $allowedExceptions) {
                $violations.Add("$origin ${name}@${version}: unreviewed license token '$token' in '$expression'")
            }
        }
    }

    $cargoJson = & cargo metadata --format-version 1 --locked
    if ($LASTEXITCODE -ne 0) {
        throw 'cargo metadata failed; dependency licenses were not audited.'
    }
    $cargoMetadata = ($cargoJson -join [Environment]::NewLine) | ConvertFrom-Json
    foreach ($package in $cargoMetadata.packages) {
        if ($package.license) {
            Test-LicenseExpression $package.name $package.version $package.license 'Cargo'
        } elseif ($package.license_file) {
            $violations.Add("Cargo $($package.name)@$($package.version): license_file requires manual SPDX review ($($package.license_file))")
        } else {
            $violations.Add("Cargo $($package.name)@$($package.version): missing license metadata")
        }
    }

    $pnpmDirectory = Join-Path $repositoryRoot 'ui/node_modules/.pnpm'
    if (-not (Test-Path -LiteralPath $pnpmDirectory -PathType Container)) {
        throw 'Install the frozen UI dependencies before running the frontend license audit.'
    }
    $frontendPackages = 0
    foreach ($directory in Get-ChildItem -LiteralPath $pnpmDirectory -Directory) {
        $packageFolder = ($directory.Name -split '_', 2)[0]
        if ($packageFolder.StartsWith('@')) {
            $packageName = '@' + (($packageFolder.Substring(1) -split '@', 2)[0].Replace('+', '/'))
        } else {
            $packageName = ($packageFolder -split '@', 2)[0]
        }
        if (-not $packageName) { continue }
        $manifest = Join-Path $directory.FullName "node_modules/$packageName/package.json"
        if (-not (Test-Path -LiteralPath $manifest -PathType Leaf)) { continue }
        $package = Get-Content -LiteralPath $manifest -Raw | ConvertFrom-Json
        $frontendPackages++
        $expression = if ($package.license -is [string]) {
            $package.license
        } elseif ($package.license.type) {
            $package.license.type
        } else {
            $null
        }
        Test-LicenseExpression $package.name $package.version $expression 'pnpm'
    }

    if ($violations.Count -gt 0) {
        $violations | ForEach-Object { Write-Error $_ }
        exit 1
    }
    Write-Output "License metadata reviewed: GPL-3.0-or-later project text plus $($cargoMetadata.packages.Count) Cargo packages and $frontendPackages frontend packages."
} finally {
    Pop-Location
}
