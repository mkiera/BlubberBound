# Closing during a compression must keep the running file queued for the next start, as the close prompt says.
Import-Module $env:BB_UI_MODULE -Force
$data = Initialize-BbProfile
@{ settings = @{ compression_mode = 'limit'; target_mb = 1; encoder = 'software' }; jobs = @() } | ConvertTo-Json | Set-Content -Encoding utf8 (Get-BbStatePath)
$clip = New-BbClip -Path (Join-Path $data.Media 'long.mp4') -Seconds 90 -Size '1920x1080'

$proc = Start-Bb -Files $clip
Wait-BbJob -Status pending -TimeoutSeconds 60 | Out-Null
Find-BbElement -Name 'Start compressing' -ControlType Button | Invoke-BbElement
Wait-BbJob -Status running -TimeoutSeconds 60 | Out-Null
Start-Sleep -Seconds 3
Save-BbScreenshot -Name 'compressing' | Out-Null

(Get-Process blubberbound | Where-Object MainWindowTitle -eq 'BlubberBound' | Select-Object -First 1).CloseMainWindow() | Out-Null
$prompt = Get-BbWindow -Title 'Stop compression?' -TimeoutSeconds 15
Save-BbDesktopScreenshot -Name 'close-prompt' | Out-Null
$buttons = foreach ($e in $prompt.FindAll([System.Windows.Automation.TreeScope]::Descendants, [System.Windows.Automation.Condition]::TrueCondition)) {
    Write-Host "prompt element: $($e.Current.ControlType.ProgrammaticName) '$($e.Current.Name)' patterns=$(($e.GetSupportedPatterns() | ForEach-Object ProgrammaticName) -join ',')"
    if ($e.Current.Name -eq 'OK') { $e }
}
Assert-Bb (@($buttons).Count -gt 0) 'close prompt has an OK control'
$invoked = $false
foreach ($b in $buttons) { try { $b | Invoke-BbElement; $invoked = $true; break } catch { } }
if (-not $invoked) {
    # OK is the prompt's default button.
    [BbWin32]::SetForegroundWindow([IntPtr]$prompt.Current.NativeWindowHandle) | Out-Null
    Start-Sleep -Milliseconds 400
    [System.Windows.Forms.SendKeys]::SendWait('{ENTER}')
}
Assert-Bb ($proc.WaitForExit(30000)) 'app exited after confirming the close prompt'

$jobs = Get-BbJobs
Assert-Bb ($jobs[0].status -eq 'pending') "running file saved as pending (status $($jobs[0].status), stage '$($jobs[0].stage)')"
Assert-Bb (-not (Get-ChildItem -Force $data.Media -Filter '.blubberbound-*')) 'no temp folder left beside the source'

Start-Bb | Out-Null
Assert-Bb (Test-BbElement -Like '*Ready to retry after restart*' -TimeoutSeconds 10) 'after restart the file shows Ready to retry after restart'
Save-BbScreenshot -Name 'after-restart' | Out-Null
Stop-Bb
