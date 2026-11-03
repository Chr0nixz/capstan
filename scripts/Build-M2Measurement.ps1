param([Parameter(Mandatory=$true)][string]$EvidenceDirectory)
$ErrorActionPreference='Stop'
$repository=(Resolve-Path -LiteralPath (Join-Path $PSScriptRoot '..')).Path
. (Join-Path $PSScriptRoot 'ConPty.ps1')
$evidence=[IO.Path]::GetFullPath($EvidenceDirectory)
if(Test-Path -LiteralPath $evidence){throw 'Preserve earlier measurement builds; use a new evidence directory.'}
New-Item -ItemType Directory -Path $evidence|Out-Null
$utf8=[Text.UTF8Encoding]::new($false)
$checks=[Collections.Generic.List[object]]::new()
$directory=Join-Path $repository ('target/m2-benchmark-'+[Guid]::NewGuid().ToString('N').Substring(0,12))
$failure=$null
function Build-Step([string]$Name,[scriptblock]$Run){
    $global:LASTEXITCODE=0;$ErrorActionPreference='Continue'
    try{& $Run *> (Join-Path $evidence "$Name.txt");$code=$LASTEXITCODE}finally{$ErrorActionPreference='Stop'}
    $checks.Add([ordered]@{name=$Name;exitCode=$code;log="$Name.txt"})
    if($code -ne 0){throw "Measurement build failed: $Name"}
    Write-Output "$Name : exit $code"
}
Push-Location $repository
try{
    Build-Step 'fixture' {cargo build -p capstan-pty --bin capstan-m0-fixture --features m0-benchmarks --release --offline --locked}
    Build-Step 'measurement-desktop' {powershell -NoProfile -File scripts/Invoke-Tauri.ps1 build --no-bundle --features m0-benchmarks,custom-protocol --config src-tauri/tauri.m0-benchmark.conf.json}
    New-Item -ItemType Directory -Path $directory|Out-Null
    Copy-Item -LiteralPath target/release/capstan-app.exe -Destination (Join-Path $directory 'Capstan.exe')
    Copy-Item -LiteralPath target/release/capstan-m0-fixture.exe -Destination (Join-Path $directory 'capstan-m0-fixture.exe')
    Copy-ConPtyRuntime (Join-Path $repository 'target/runtime/x86_64') (Join-Path $directory 'runtime/x86_64')
    [IO.File]::WriteAllText((Join-Path $directory 'capstan.portable'),'CAPSTAN_PORTABLE_V1',[Text.Encoding]::ASCII)
    foreach($name in @('LICENSE','THIRD_PARTY_NOTICES.md')){Copy-Item -LiteralPath (Join-Path $repository $name) -Destination $directory}
    $manifest=[ordered]@{
        schemaVersion=1;purpose='M2-B-quantitative-measurement';directory=$directory
        features=@('m0-benchmarks','custom-protocol');instrumentation=$true
        benchmarkUrl='index.html#/m0-benchmark'
        benchmarkConfigSha256=(Get-FileHash -LiteralPath src-tauri/tauri.m0-benchmark.conf.json).Hash
        executableSha256=(Get-FileHash -LiteralPath (Join-Path $directory 'Capstan.exe')).Hash
        fixtureSha256=(Get-FileHash -LiteralPath (Join-Path $directory 'capstan-m0-fixture.exe')).Hash
        planSha256=(Get-FileHash -LiteralPath docs/PLAN.md).Hash
        candidate=$false;performanceAcceptance=$false;desktopAcceptance=$false
    }
    [IO.File]::WriteAllText((Join-Path $directory 'm2-measurement.json'),($manifest|ConvertTo-Json -Depth 5),$utf8)
    $capability=& (Join-Path $directory 'Capstan.exe') --m0-check
    if($LASTEXITCODE -ne 0 -or $capability -cne 'capstan-m0-benchmarks-v1'){throw 'Measurement capability preflight failed.'}
    Write-Output "Measurement build: $directory"
}catch{$failure=$_.Exception.Message}finally{
    Pop-Location
    $report=[ordered]@{checks=$checks.ToArray();directory=$directory;passed=($null -eq $failure);failure=$failure;candidate=$false;performanceAcceptance=$false;desktopAcceptance=$false}
    if(Test-Path -LiteralPath (Join-Path $directory 'm2-measurement.json')){$report.manifestSha256=(Get-FileHash -LiteralPath (Join-Path $directory 'm2-measurement.json')).Hash}
    [IO.File]::WriteAllText((Join-Path $evidence 'report.json'),($report|ConvertTo-Json -Depth 5),$utf8)
}
if($failure){throw $failure}
