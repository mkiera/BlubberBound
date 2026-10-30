# Auto quality must finish when the output folder path holds characters the FFmpeg filtergraph treats as syntax.
Import-Module $env:BB_UI_MODULE -Force
$data = Initialize-BbProfile
$out = Join-Path $data.Root "O'Brien, Ltd [4K]; clips"
New-Item -ItemType Directory -Force -Path $out | Out-Null
@{ settings = @{ compression_mode = 'auto'; output_dir = $out }; jobs = @() } | ConvertTo-Json | Set-Content -Encoding utf8 (Get-BbStatePath)
# A high bitrate gives Auto quality room for a smaller output, so its frame comparison has to run.
$clip = New-BbClip -Path (Join-Path $data.Media 'sample.mp4') -Bitrate '8M'

Start-Bb -Files $clip | Out-Null
Wait-BbJob -Status pending -TimeoutSeconds 60 | Out-Null
Save-BbScreenshot -Name 'queued' | Out-Null
Find-BbElement -Name 'Start compressing' -ControlType Button | Invoke-BbElement
$jobs = Wait-BbJob -Status completed, failed -TimeoutSeconds 600
Save-BbScreenshot -Name 'finished' | Out-Null
Assert-Bb ($jobs[0].status -eq 'completed') "Auto quality job completed (status $($jobs[0].status), error '$($jobs[0].error)')"
Assert-Bb ($jobs[0].preserved_original -ne $true -and $jobs[0].output_size -lt $jobs[0].original_size) "Auto quality saved a smaller output ($($jobs[0].original_size) -> $($jobs[0].output_size) bytes, stage '$($jobs[0].stage)')"
Assert-Bb ((Split-Path $jobs[0].output -Parent) -like "*O'Brien, Ltd [[]4K[]]; clips") "output saved in the special folder ($($jobs[0].output))"
Stop-Bb
