Set-StrictMode -Version 3
$ErrorActionPreference = 'Stop'

Add-Type -AssemblyName UIAutomationClient, UIAutomationTypes, System.Drawing, System.Windows.Forms

Add-Type -TypeDefinition @"
using System;
using System.Runtime.InteropServices;
public static class BbWin32 {
    [DllImport("user32.dll")] public static extern bool PrintWindow(IntPtr h, IntPtr dc, uint flags);
    [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
    [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
    [StructLayout(LayoutKind.Sequential)] public struct RECT { public int L, T, R, B; }
}
"@

$script:AE = [System.Windows.Automation.AutomationElement]
$script:Descendants = [System.Windows.Automation.TreeScope]::Descendants
$script:Children = [System.Windows.Automation.TreeScope]::Children
$script:Shot = 0

function Get-BbOutDir {
    $dir = $env:BB_UI_OUT
    if (-not $dir) { $dir = Join-Path ([IO.Path]::GetTempPath()) 'bb-ui-out' }
    New-Item -ItemType Directory -Force -Path $dir | Out-Null
    return $dir
}

function Get-BbStatePath { Join-Path $env:LOCALAPPDATA 'BlubberBound\state.json' }

function Initialize-BbProfile {
    # Deletes %LOCALAPPDATA%\BlubberBound, so it refuses to run outside CI.
    param([hashtable]$Settings = @{})
    if ($env:GITHUB_ACTIONS -ne 'true') { throw 'Initialize-BbProfile wipes the app data folder. It runs only on CI.' }
    $appData = Join-Path $env:LOCALAPPDATA 'BlubberBound'
    # WebView2 helpers of an app that was just stopped can hold the folder for a few seconds.
    for ($try = 0; (Test-Path $appData) -and $try -lt 30; $try++) {
        try { Remove-Item -Recurse -Force $appData -ErrorAction Stop } catch { Start-Sleep -Milliseconds 500 }
    }
    if (Test-Path $appData) { throw "could not clear $appData" }
    New-Item -ItemType Directory -Force -Path $appData | Out-Null
    $root = Join-Path ([IO.Path]::GetTempPath()) "bb-ui-data-$([Guid]::NewGuid().ToString('N').Substring(0, 8))"
    New-Item -ItemType Directory -Force -Path (Join-Path $root 'media') | Out-Null
    @{ settings = $Settings; jobs = @() } | ConvertTo-Json -Depth 5 | Set-Content -Encoding utf8 (Get-BbStatePath)
    [pscustomobject]@{ Root = $root; Media = (Join-Path $root 'media'); AppData = $appData }
}

function New-BbClip {
    param([Parameter(Mandatory)][string]$Path, [int]$Seconds = 4, [string]$Size = '640x360', [string]$Bitrate = '')
    $rate = if ($Bitrate) { @('-b:v', $Bitrate) } else { @() }
    & $env:BLUBBERBOUND_FFMPEG -v error -y -f lavfi -i "testsrc2=size=$($Size):rate=30" -f lavfi -i "sine=frequency=440" -t $Seconds -pix_fmt yuv420p -c:v libx264 @rate -c:a aac $Path
    if ($LASTEXITCODE -ne 0) { throw "ffmpeg could not make $Path" }
    return $Path
}

function Start-Bb {
    param([string[]]$Files = @(), [int]$TimeoutSeconds = 60)
    $exe = $env:BB_UI_APP
    if (-not $exe -or -not (Test-Path $exe)) { throw "BB_UI_APP does not point at the app: '$exe'" }
    $p = @{ FilePath = $exe; PassThru = $true }
    if ($Files.Count) { $p.ArgumentList = $Files | ForEach-Object { '"' + $_ + '"' } }
    $proc = Start-Process @p
    Get-BbWindow -TimeoutSeconds $TimeoutSeconds | Out-Null
    # The page loads after the window appears, and queued files are probed in the background.
    Find-BbElement -Name 'Start compressing' -ControlType Button -TimeoutSeconds 30 | Out-Null
    Start-Sleep -Seconds 3
    return $proc
}

function Stop-Bb {
    param([int]$TimeoutSeconds = 20)
    Get-Process blubberbound -ErrorAction SilentlyContinue | ForEach-Object {
        if (-not $_.WaitForExit(0)) { $_.Kill(); $_.WaitForExit($TimeoutSeconds * 1000) | Out-Null }
    }
    # FFmpeg and WebView2 children outlive a killed app and keep its data folder as their working directory.
    Get-Process msedgewebview2, ffmpeg, ffprobe -ErrorAction SilentlyContinue | Stop-Process -Force -ErrorAction SilentlyContinue
}

function Get-BbWindow {
    param([string]$Title = 'BlubberBound', [int]$TimeoutSeconds = 10)
    $deadline = (Get-Date).AddSeconds($TimeoutSeconds)
    do {
        foreach ($proc in @(Get-Process blubberbound -ErrorAction SilentlyContinue)) {
            $cond = New-Object System.Windows.Automation.PropertyCondition($AE::ProcessIdProperty, $proc.Id)
            foreach ($w in $AE::RootElement.FindAll($Children, $cond)) {
                if ($w.Current.Name -eq $Title) { return $w }
            }
        }
        Start-Sleep -Milliseconds 250
    } while ((Get-Date) -lt $deadline)
    throw "no window titled '$Title'"
}

function Find-BbElement {
    param([string]$Name, [string]$Like, [string]$ControlType, [string]$Window = 'BlubberBound', [int]$TimeoutSeconds = 10)
    $conds = @()
    if ($Name) { $conds += New-Object System.Windows.Automation.PropertyCondition($AE::NameProperty, $Name) }
    if ($ControlType) {
        $conds += New-Object System.Windows.Automation.PropertyCondition($AE::ControlTypeProperty, [System.Windows.Automation.ControlType]::$ControlType)
    }
    $cond = switch ($conds.Count) {
        0 { [System.Windows.Automation.Condition]::TrueCondition }
        1 { $conds[0] }
        default { New-Object System.Windows.Automation.AndCondition($conds) }
    }
    $deadline = (Get-Date).AddSeconds($TimeoutSeconds)
    $last = ''
    do {
        try {
            $scope = Get-BbWindow -Title $Window -TimeoutSeconds 1
            if (-not $Like) {
                $e = $scope.FindFirst($Descendants, $cond)
                if ($e) { return $e }
            } else {
                foreach ($e in $scope.FindAll($Descendants, $cond)) {
                    # The queue list redraws while it is polled, so an element can vanish mid-search.
                    try { if ($e.Current.Name -like $Like) { return $e } } catch { $last = "$_" }
                }
            }
        } catch { $last = "$_" }
        Start-Sleep -Milliseconds 250
    } while ((Get-Date) -lt $deadline)
    throw "no element name='$Name' like='$Like' type='$ControlType' in '$Window' (last error: $last)"
}

function Test-BbElement {
    param([string]$Like, [string]$Window = 'BlubberBound', [int]$TimeoutSeconds = 2)
    try { Find-BbElement -Like $Like -Window $Window -TimeoutSeconds $TimeoutSeconds | Out-Null; return $true } catch { return $false }
}

function Invoke-BbElement {
    param([Parameter(ValueFromPipeline)]$Element)
    process {
        $p = $null
        if ($Element.TryGetCurrentPattern([System.Windows.Automation.InvokePattern]::Pattern, [ref]$p)) { $p.Invoke() }
        # Buttons with aria-pressed, such as the size presets, are toggle buttons to UI Automation.
        elseif ($Element.TryGetCurrentPattern([System.Windows.Automation.TogglePattern]::Pattern, [ref]$p)) { $p.Toggle() }
        else { throw "element '$($Element.Current.Name)' cannot be invoked" }
        Start-Sleep -Milliseconds 600
    }
}

function Get-BbJobs { @((Get-Content (Get-BbStatePath) -Raw | ConvertFrom-Json).jobs) }
function Get-BbSettings { (Get-Content (Get-BbStatePath) -Raw | ConvertFrom-Json).settings }

function Wait-BbJob {
    # Waits until every job's status is in -Status, read from the saved state.
    param([Parameter(Mandatory)][string[]]$Status, [int]$Count = 1, [int]$TimeoutSeconds = 300)
    $deadline = (Get-Date).AddSeconds($TimeoutSeconds)
    do {
        try {
            $jobs = Get-BbJobs
            if ($jobs.Count -ge $Count -and -not ($jobs | Where-Object { $_.status -notin $Status })) { return $jobs }
        } catch { }
        Start-Sleep -Milliseconds 500
    } while ((Get-Date) -lt $deadline)
    throw "jobs did not reach '$($Status -join ', ')': $((Get-BbJobs | ForEach-Object { "$($_.name)=$($_.status) $($_.error)" }) -join '; ')"
}

function Save-BbScreenshot {
    # PrintWindow captures the window even when covered. 2 = PW_RENDERFULLCONTENT, needed for WebView2.
    param([Parameter(Mandatory)][string]$Name, [string]$Window = 'BlubberBound')
    $h = [IntPtr](Get-BbWindow -Title $Window).Current.NativeWindowHandle
    [BbWin32]::SetForegroundWindow($h) | Out-Null
    Start-Sleep -Milliseconds 600
    $r = New-Object BbWin32+RECT
    [BbWin32]::GetWindowRect($h, [ref]$r) | Out-Null
    $bmp = New-Object System.Drawing.Bitmap(($r.R - $r.L), ($r.B - $r.T))
    $g = [System.Drawing.Graphics]::FromImage($bmp)
    $dc = $g.GetHdc()
    [BbWin32]::PrintWindow($h, $dc, 2) | Out-Null
    $g.ReleaseHdc($dc); $g.Dispose()
    $script:Shot++
    $path = Join-Path (Get-BbOutDir) ('{0:D2}-{1}.png' -f $script:Shot, $Name)
    $bmp.Save($path, [System.Drawing.Imaging.ImageFormat]::Png); $bmp.Dispose()
    Write-Host "screenshot $path"
    return $path
}

function Save-BbDesktopScreenshot {
    param([Parameter(Mandatory)][string]$Name)
    $b = [System.Windows.Forms.SystemInformation]::VirtualScreen
    $bmp = New-Object System.Drawing.Bitmap($b.Width, $b.Height)
    $g = [System.Drawing.Graphics]::FromImage($bmp)
    $g.CopyFromScreen($b.Left, $b.Top, 0, 0, $bmp.Size); $g.Dispose()
    $script:Shot++
    $path = Join-Path (Get-BbOutDir) ('{0:D2}-{1}.png' -f $script:Shot, $Name)
    $bmp.Save($path, [System.Drawing.Imaging.ImageFormat]::Png); $bmp.Dispose()
    Write-Host "screenshot $path"
    return $path
}

function Assert-Bb {
    param([Parameter(Mandatory)][bool]$Condition, [Parameter(Mandatory)][string]$Message)
    if (-not $Condition) { throw "check failed: $Message" }
    Write-Host "ok: $Message"
}

Export-ModuleMember -Function *-Bb*
