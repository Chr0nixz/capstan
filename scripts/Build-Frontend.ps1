$ErrorActionPreference = 'Stop'
# Tauri's nested PowerShell must resolve inbox modules from this PowerShell host.
Import-Module (Join-Path $PSHOME 'Modules/Microsoft.PowerShell.Utility') -ErrorAction Stop
Import-Module (Join-Path $PSHOME 'Modules/Microsoft.PowerShell.Security') -ErrorAction Stop
& (Join-Path $PSScriptRoot 'Prepare-ConPty.ps1')
$repository = (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot '..')).Path
Push-Location $repository
try {
    & pnpm -C ui build
    exit $LASTEXITCODE
} finally { Pop-Location }
