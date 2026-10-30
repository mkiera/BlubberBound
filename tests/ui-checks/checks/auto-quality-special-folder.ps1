# Auto quality must finish when the output folder path holds characters the FFmpeg filtergraph treats as syntax.
Import-Module $env:BB_UI_MODULE -Force
$data = Initialize-BbProfile
$out = Join-Path $data.Root "O'Brien, Ltd [4K]; clips"
New-Item -ItemType Directory -Force -Path $out | Out-Null
@{ settings = @{ compression_mode = 'auto'; output_dir = $out }; jobs = @() } | ConvertTo-Json | Set-Content -Encoding utf8 (Get-BbStatePath)
$clip = New-BbClip -Path (Join-Path $data.Media 'sample.mp4')

Start-Bb -Files $clip | Out-Null
Wait-BbJob -Status pending -TimeoutSeconds 60 | Out-Null
Save-BbScreenshot -Name 'queued' | Out-Null
Find-BbElement -Name 'Start compressing' -ControlType Button | Invoke-BbElement
$jobs = Wait-BbJob -Status completed, failed -TimeoutSeconds 600
Save-BbScreenshot -Name 'finished' | Out-Null
Assert-Bb ($jobs[0].status -eq 'completed') "Auto quality job completed (status $($jobs[0].status), error '$($jobs[0].error)')"
Stop-Bb
