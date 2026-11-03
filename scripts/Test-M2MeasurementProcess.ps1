param([Parameter(Mandatory = $true)][string]$EvidenceDirectory)
$ErrorActionPreference = 'Stop'
$repository = (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot '..')).Path
$evidence = [IO.Path]::GetFullPath($EvidenceDirectory)
New-Item -ItemType Directory -Path $evidence -Force | Out-Null
$tokens = $null
$parseErrors = $null
$source = [Management.Automation.Language.Parser]::ParseFile((Join-Path $PSScriptRoot 'Measure-M2SftpLibrary.ps1'), [ref]$tokens, [ref]$parseErrors)
if (@($parseErrors).Count) { throw 'Measurement helper syntax is invalid.' }
$function = $source.Find({ param($node) $node -is [Management.Automation.Language.FunctionDefinitionAst] -and $node.Name -eq 'Invoke-MeasurementProbe' }, $true)
if (-not $function) { throw 'Measurement process helper is missing.' }
. ([scriptblock]::Create($function.Extent.Text))
$executable = Join-Path $PSHOME 'powershell.exe'
$normal = [Convert]::ToBase64String([Text.Encoding]::Unicode.GetBytes('Write-Output "measurement-exit"; exit 7'))
$result = Invoke-MeasurementProbe $executable @('-NoProfile', '-NonInteractive', '-EncodedCommand', $normal) (Join-Path $evidence 'normal.txt') 10
if ($result.exitCode -ne 7 -or -not (Get-Content -LiteralPath (Join-Path $evidence 'normal.txt') -Raw).Contains('measurement-exit')) { throw 'The owned process exit/output was not preserved.' }
$stall = [Convert]::ToBase64String([Text.Encoding]::Unicode.GetBytes('Start-Sleep -Seconds 20; exit 0'))
$rejected = $false
try { Invoke-MeasurementProbe $executable @('-NoProfile', '-NonInteractive', '-EncodedCommand', $stall) (Join-Path $evidence 'timeout.txt') 1 | Out-Null }
catch { $rejected = $_.Exception.Message.Contains('exceeded its wall-clock deadline') }
$timeout = Get-Content -LiteralPath (Join-Path $evidence 'timeout.txt.execution.json') -Raw | ConvertFrom-Json
if (-not $rejected -or -not $timeout.timedOut -or $timeout.elapsedMs -ge 10000) { throw 'The owned process timeout was not enforced.' }
[ordered]@{ checks = 2; passed = $true; preservedNonzeroExitAndOutput = $true; terminatedOnlyOwnedTimeoutProcess = $true; scope = 'Measurement launcher self-check, no SFTP or product acceptance' } |
    ConvertTo-Json | Set-Content -LiteralPath (Join-Path $evidence 'report.json') -Encoding utf8
Write-Output 'Measurement process self-checks: 2 passed.'
