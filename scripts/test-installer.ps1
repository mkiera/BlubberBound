param([Parameter(Mandatory = $true)][string]$Compiler, [switch]$HoldOpen)
$ErrorActionPreference = 'Stop'
$project = Split-Path -Parent $PSScriptRoot
$root = Join-Path ([IO.Path]::GetTempPath()) ('BlubberBound-install-test-' + [Guid]::NewGuid().ToString('N'))
$payload = Join-Path $root 'payload'
$install = Join-Path $root 'installed'
$oldPayload = Join-Path $install 'app'
$name = 'UpdateFixture-' + [Guid]::NewGuid().ToString('N')
New-Item -ItemType Directory -Path $payload, $oldPayload | Out-Null
$app = Join-Path $payload ($name + '.exe')
& rustc --edition 2021 (Join-Path $project 'tests\fixtures\update_app.rs') -o $app
if ($LASTEXITCODE -ne 0) { throw 'Installer fixture compilation failed.' }
Copy-Item -LiteralPath $app -Destination $oldPayload
[IO.File]::WriteAllText((Join-Path $oldPayload 'version.txt'), $(if ($HoldOpen) { 'blocked' } else { 'old' }))
[IO.File]::WriteAllText((Join-Path $payload 'version.txt'), 'new')
[IO.File]::WriteAllText((Join-Path $oldPayload 'obsolete.txt'), 'old-only file')
[IO.File]::WriteAllText((Join-Path $install 'settings.json'), '{"preserve":true}')
$packageId = '{{' + [Guid]::NewGuid().ToString() + '}'
& $Compiler /Q "/O$root" '/FUpdateFixture-Setup' '/DAppVersion=1.0.0' '/DVersionNumeric=1.0.0.0' "/DAppName=$name" "/DPackageId=$packageId" "/DExecutable=$name.exe" "/DStorageId=$name" "/DPayloadRoot=$payload" (Join-Path $project 'installer.iss')
if ($LASTEXITCODE -ne 0) { throw 'Installer fixture packaging failed.' }
$old = Start-Process -FilePath (Join-Path $oldPayload ($name + '.exe')) -WindowStyle Hidden -PassThru
$setup = $null
try {
    $deadline = [DateTime]::UtcNow.AddSeconds(10)
    while (-not (Test-Path -LiteralPath (Join-Path $install 'started.txt'))) {
        if ([DateTime]::UtcNow -gt $deadline) { throw 'Old application did not start.' }
        Start-Sleep -Milliseconds 20
    }
    $arguments = @('/VERYSILENT', '/SUPPRESSMSGBOXES', '/CLOSEAPPLICATIONS', '/NORESTARTAPPLICATIONS', '/SP-', '/NOICONS', '/TASKS=', ('/DIR="' + $install + '"'), ('/LOG="' + (Join-Path $install 'update.log') + '"'))
    $setup = Start-Process -FilePath (Join-Path $root 'UpdateFixture-Setup.exe') -ArgumentList $arguments -WindowStyle Hidden -PassThru
    if (-not $setup.WaitForExit(60000)) { throw 'Installer did not finish within 60 seconds.' }
    if ($HoldOpen) {
        if ($setup.ExitCode -eq 0) { throw 'Installer replaced an application that was still running.' }
        if (-not (Test-Path -LiteralPath (Join-Path $oldPayload 'obsolete.txt')) -or
            (Get-Content -LiteralPath (Join-Path $oldPayload 'version.txt') -Raw) -ne 'blocked') {
            throw 'Failed upgrade did not preserve the old application.'
        }
        if (Test-Path -LiteralPath (Join-Path $install 'app.old')) { throw 'Failed upgrade moved the old application.' }
        Write-Output 'Blocked upgrade passed: the previous application remains intact.'
        return
    }
    if ($setup.ExitCode -ne 0) { throw "Installer failed with code $($setup.ExitCode). Log: $install\update.log" }
    if (-not $old.WaitForExit(5000)) { throw 'Old application is still running.' }
    $deadline = [DateTime]::UtcNow.AddSeconds(10)
    while (-not (Test-Path -LiteralPath (Join-Path $install 'relaunched.txt'))) {
        if ([DateTime]::UtcNow -gt $deadline) { throw 'New application was not relaunched.' }
        Start-Sleep -Milliseconds 20
    }
    if ((Get-Content -LiteralPath (Join-Path $install 'relaunched.txt') -Raw) -ne 'new') { throw 'The old application was relaunched.' }
    if (Test-Path -LiteralPath (Join-Path $oldPayload 'obsolete.txt')) { throw 'Old payload files were left behind.' }
    if (Test-Path -LiteralPath (Join-Path $install 'app.old')) { throw 'The completed upgrade left its backup behind.' }
    if ((Get-Content -LiteralPath (Join-Path $install 'settings.json') -Raw) -ne '{"preserve":true}') { throw 'Upgrade changed user settings.' }
    Write-Output 'Running-app upgrade passed: delayed exit, payload replacement, and relaunch.'
} finally {
    if ($setup -and -not $setup.HasExited) { Stop-Process -Id $setup.Id }
    if (-not $old.HasExited) { Stop-Process -Id $old.Id }
    $uninstaller = Join-Path $install 'unins000.exe'
    if (Test-Path -LiteralPath $uninstaller) {
        $cleanup = Start-Process -FilePath $uninstaller -ArgumentList '/VERYSILENT', '/SUPPRESSMSGBOXES', '/NORESTART' -WindowStyle Hidden -PassThru
        if (-not $cleanup.WaitForExit(30000)) { Stop-Process -Id $cleanup.Id }
    }
    Write-Output "Installer test files: $root"
}
