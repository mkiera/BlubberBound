$ErrorActionPreference = 'Stop'
$taskRoot = $PSScriptRoot
$toolsDir = Join-Path $taskRoot 'tools'
$downloadDir = Join-Path $taskRoot '.downloads'
$archive = Join-Path $downloadDir 'ffmpeg-9.0-essentials_build.zip'
$expectedHash = 'e6b54767a6065919048f1a098eb27211ca4e12b4348a05d88777a5855d0b6e71'
$marker = Join-Path $toolsDir 'source-sha256.txt'

if ((Test-Path -LiteralPath $marker) -and
    (Test-Path -LiteralPath (Join-Path $toolsDir 'ffmpeg.exe')) -and
    (Test-Path -LiteralPath (Join-Path $toolsDir 'ffprobe.exe')) -and
    ((Get-Content -LiteralPath $marker -Raw).Trim() -eq $expectedHash)) {
    Write-Host 'FFmpeg 9.0 is ready.'
    exit 0
}

New-Item -ItemType Directory -Path $downloadDir -Force | Out-Null
if (-not (Test-Path -LiteralPath $archive)) {
    Write-Host 'Downloading FFmpeg 9.0 (111 MB)...'
    Invoke-WebRequest -Uri 'https://github.com/GyanD/codexffmpeg/releases/download/9.0/ffmpeg-9.0-essentials_build.zip' -OutFile $archive
}
$actualHash = (Get-FileHash -LiteralPath $archive -Algorithm SHA256).Hash.ToLowerInvariant()
if ($actualHash -ne $expectedHash) {
    throw "FFmpeg checksum mismatch. Remove $archive and run this script again."
}

$extractDir = Join-Path $downloadDir 'ffmpeg-9.0'
Expand-Archive -LiteralPath $archive -DestinationPath $extractDir -Force
$sourceDir = Join-Path $extractDir 'ffmpeg-9.0-essentials_build'
New-Item -ItemType Directory -Path $toolsDir -Force | Out-Null
Copy-Item -LiteralPath (Join-Path $sourceDir 'bin\ffmpeg.exe') -Destination $toolsDir -Force
Copy-Item -LiteralPath (Join-Path $sourceDir 'bin\ffprobe.exe') -Destination $toolsDir -Force
Copy-Item -LiteralPath (Join-Path $sourceDir 'LICENSE') -Destination (Join-Path $toolsDir 'FFMPEG-LICENSE.txt') -Force
Copy-Item -LiteralPath (Join-Path $sourceDir 'README.txt') -Destination (Join-Path $toolsDir 'FFMPEG-README.txt') -Force
Set-Content -LiteralPath $marker -Value $expectedHash -Encoding ASCII
Write-Host 'FFmpeg 9.0 downloaded and verified.'

