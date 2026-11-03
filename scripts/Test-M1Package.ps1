param(
    [Parameter(Mandatory=$true)][string]$ArchivePath,
    [Parameter(Mandatory=$true)][string]$CandidateExecutable,
    [Parameter(Mandatory=$true)][string]$EvidenceDirectory
)
$ErrorActionPreference='Stop'
. (Join-Path $PSScriptRoot 'ConPty.ps1')
$repository=(Resolve-Path -LiteralPath (Join-Path $PSScriptRoot '..')).Path
$target=(Resolve-Path -LiteralPath (Join-Path $repository 'target')).Path
$stage=Join-Path $target ('m1-package-check-'+[Guid]::NewGuid().ToString('N'))
$archiveFile=(Resolve-Path -LiteralPath $ArchivePath).Path
$candidate=(Resolve-Path -LiteralPath $CandidateExecutable).Path
$required=@('Capstan.exe','capstan.portable','LICENSE','THIRD_PARTY_NOTICES.md','PORTABLE-README.md','runtime/x86_64/conpty.dll','runtime/x86_64/x64/OpenConsole.exe','runtime/x86_64/arm64/OpenConsole.exe','runtime/x86_64/LICENSE.txt','runtime/x86_64/manifest.json')
New-Item -ItemType Directory -Path $stage | Out-Null
Add-Type -AssemblyName System.IO.Compression.FileSystem
$archive=[IO.Compression.ZipFile]::OpenRead($archiveFile)
$inventory=@()
try {
    foreach ($entry in $archive.Entries) {
        $name=$entry.FullName.Replace('\','/')
        if ($name.EndsWith('/')) {
            if ($name -notin @('runtime/','runtime/x86_64/','runtime/x86_64/x64/','runtime/x86_64/arm64/')) { throw 'Unexpected archive directory.' }
            continue
        }
        if ($name -notin $required -or $name -in @($inventory|ForEach-Object { $_.file }) -or $entry.Length -gt 40MB) { throw 'Unexpected, duplicate or oversized archive entry.' }
        $destination=Join-Path $stage $name
        New-Item -ItemType Directory -Path (Split-Path $destination -Parent) -Force | Out-Null
        [IO.Compression.ZipFileExtensions]::ExtractToFile($entry,$destination,$false)
        $inventory+=@{file=$name;bytes=$entry.Length;sha256=(Get-FileHash -LiteralPath $destination).Hash}
    }
    if ($inventory.Count -ne $required.Count) { throw 'Incomplete portable payload.' }
    if ((Get-FileHash -LiteralPath (Join-Path $stage 'Capstan.exe')).Hash -ne (Get-FileHash -LiteralPath $candidate).Hash) { throw 'Archive executable differs from the candidate.' }
    if ([IO.File]::ReadAllText((Join-Path $stage 'capstan.portable')) -cne 'CAPSTAN_PORTABLE_V1') { throw 'Portable marker differs.' }
    Assert-ConPtyRuntime (Join-Path $stage 'runtime/x86_64') | Out-Null
    foreach ($name in @('LICENSE','THIRD_PARTY_NOTICES.md')) {
        if ((Get-FileHash -LiteralPath (Join-Path $stage $name)).Hash -ne (Get-FileHash -LiteralPath (Join-Path $repository $name)).Hash) { throw 'Archive legal notices differ.' }
    }
    $process=Start-Process -FilePath (Join-Path $stage 'Capstan.exe') -ArgumentList '--m0-check' -WindowStyle Hidden -PassThru
    if (!$process.WaitForExit(10000) -or $process.ExitCode -ne 2) { throw 'Production artifact unexpectedly exposes measurement capability.' }
    $report=@{
        schema=1;passed=$true;archive_sha256=(Get-FileHash -LiteralPath $archiveFile).Hash
        compressed_bytes=(Get-Item -LiteralPath $archiveFile).Length;uncompressed_file_bytes=($inventory|ForEach-Object { $_.bytes }|Measure-Object -Sum).Sum
        files=$inventory;production_measurement_preflight_exit_code=$process.ExitCode
    }
    New-Item -ItemType Directory -Path $EvidenceDirectory -Force | Out-Null
    [IO.File]::WriteAllText((Join-Path $EvidenceDirectory 'report.json'),($report|ConvertTo-Json -Depth 6),[Text.UTF8Encoding]::new($false))
    Write-Output "Portable package verified: $($inventory.Count) files, signed runtime and companions, no measurement capability."
} finally {
    $archive.Dispose()
    $resolved=[IO.Path]::GetFullPath($stage)
    if (!$resolved.StartsWith($target.TrimEnd('\')+'\',[StringComparison]::OrdinalIgnoreCase)) { throw 'Invalid archive test cleanup path.' }
    Remove-Item -LiteralPath $stage -Recurse -Force
}
