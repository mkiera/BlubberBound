param(
    [Parameter(Mandatory)][string]$AppPath,
    [Parameter(Mandatory)][string]$OutDir,
    # Wildcard on the check file name without .ps1, such as "close-*".
    [string]$Filter = '*'
)

$ErrorActionPreference = 'Stop'
New-Item -ItemType Directory -Force -Path $OutDir | Out-Null
$AppPath = (Resolve-Path $AppPath).Path
$OutDir = (Resolve-Path $OutDir).Path

$checks = @(Get-ChildItem (Join-Path $PSScriptRoot 'checks') -Filter '*.ps1' |
    Where-Object { $_.BaseName -like $Filter } | Sort-Object Name)
if ($checks.Count -eq 0) { throw "no checks match '$Filter'" }

$results = foreach ($check in $checks) {
    $dir = Join-Path $OutDir $check.BaseName
    New-Item -ItemType Directory -Force -Path $dir | Out-Null
    $log = Join-Path $dir 'log.txt'

    $env:BB_UI_APP = $AppPath
    $env:BB_UI_OUT = $dir
    $env:BB_UI_MODULE = Join-Path $PSScriptRoot 'Bb.psm1'

    Write-Host "::group::$($check.BaseName)"
    $start = Get-Date
    & pwsh -NoProfile -File $check.FullName *>&1 | Tee-Object -FilePath $log | Out-Host
    $code = $LASTEXITCODE
    $seconds = [int]((Get-Date) - $start).TotalSeconds

    if ($code -ne 0) {
        Import-Module $env:BB_UI_MODULE -Force
        try { Save-BbDesktopScreenshot -Name 'on-failure' | Out-Null } catch { Write-Host "no failure screenshot: $_" }
    }

    # A check that throws leaves the app open, and the single-instance lock would then hand the next launch to it.
    Get-Process blubberbound, msedgewebview2 -ErrorAction SilentlyContinue | Stop-Process -Force -ErrorAction SilentlyContinue
    Start-Sleep -Seconds 1
    Write-Host '::endgroup::'

    [pscustomobject]@{ Check = $check.BaseName; Passed = ($code -eq 0); Seconds = $seconds }
}

$summary = @('| Check | Result | Seconds |', '| --- | --- | ---: |')
$summary += $results | ForEach-Object { "| $($_.Check) | $(if ($_.Passed) { 'passed' } else { '**failed**' }) | $($_.Seconds) |" }
$summary | Set-Content -Encoding utf8 (Join-Path $OutDir 'summary.md')
if ($env:GITHUB_STEP_SUMMARY) { $summary | Add-Content -Encoding utf8 $env:GITHUB_STEP_SUMMARY }
$summary | Out-Host

if ($results | Where-Object { -not $_.Passed }) { exit 1 }
