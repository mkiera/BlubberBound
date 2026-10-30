# A saved output folder that no longer exists must not block other settings, and Start must refuse once.
Import-Module $env:BB_UI_MODULE -Force
$data = Initialize-BbProfile
$missing = Join-Path $data.Root 'removed drive folder'
@{ settings = @{ compression_mode = 'limit'; target_mb = 25; output_dir = $missing }; jobs = @() } | ConvertTo-Json | Set-Content -Encoding utf8 (Get-BbStatePath)
$clips = 1..2 | ForEach-Object { New-BbClip -Path (Join-Path $data.Media "sample$_.mp4") }

Start-Bb -Files $clips | Out-Null
Wait-BbJob -Status pending -Count 2 -TimeoutSeconds 60 | Out-Null
Find-BbElement -Name '50 MB' -ControlType Button | Invoke-BbElement
Start-Sleep -Seconds 2
Assert-Bb ((Get-BbSettings).target_mb -eq 50) 'size limit change saved while the output folder is missing'
Assert-Bb (-not (Test-BbElement -Like '*output folder is unavailable*')) 'no unavailable-folder error after the size change'
Save-BbScreenshot -Name 'size-limit-saved' | Out-Null

Find-BbElement -Name 'Start compressing' -ControlType Button | Invoke-BbElement
Assert-Bb (Test-BbElement -Like 'The output folder is unavailable*' -TimeoutSeconds 10) 'Start shows the unavailable-folder message'
Start-Sleep -Seconds 3
$jobs = Get-BbJobs
Assert-Bb (-not ($jobs | Where-Object { $_.status -ne 'pending' })) "queued files stay pending ($(($jobs.status) -join ', '))"
Save-BbScreenshot -Name 'start-refused' | Out-Null
Stop-Bb
