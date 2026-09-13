param([switch]$Local)
$ErrorActionPreference = 'Stop'
Set-Location -LiteralPath $PSScriptRoot
& npm.cmd ci
if ($LASTEXITCODE -ne 0) { throw 'Dependency installation failed.' }
if ($Local -or -not (Test-Path -LiteralPath 'build_info.json')) {
    & node scripts/build-identity.mjs
    if ($LASTEXITCODE -ne 0) { throw 'Build identity generation failed.' }
}
& "$PSScriptRoot\prepare_tools.ps1"
& npm.cmd run build:frontend
if ($LASTEXITCODE -ne 0) { throw 'Frontend build failed.' }
& cargo test --manifest-path src-tauri/Cargo.toml --locked
if ($LASTEXITCODE -ne 0) { throw 'Rust tests failed.' }
foreach ($script in @('script.js', 'updates.js', 'notes.js', 'desktop.js')) {
    if (Test-Path -LiteralPath $script) {
        & node --check $script
        if ($LASTEXITCODE -ne 0) { throw 'JavaScript validation failed.' }
    }
}
& npm.cmd test
if ($LASTEXITCODE -ne 0) { throw 'UI or version tests failed.' }
& npm.cmd run tauri -- build --no-bundle --config src-tauri/build-config.json -- --locked
if ($LASTEXITCODE -ne 0) { throw 'Application build failed.' }
$identity = Get-Content build_info.json -Raw | ConvertFrom-Json
$profile = Get-Content app_profile.json -Raw | ConvertFrom-Json
$numeric = & node --input-type=module -e "import {numericVersion} from './scripts/versioning.mjs'; import fs from 'node:fs'; console.log(numericVersion(JSON.parse(fs.readFileSync('build_info.json')).version));"
if ($LASTEXITCODE -ne 0) { throw 'Numeric version generation failed.' }
$executableInfo = [Diagnostics.FileVersionInfo]::GetVersionInfo((Join-Path $PSScriptRoot 'src-tauri\target\release\blubberbound.exe'))
if ($executableInfo.ProductVersion.Trim() -ne $identity.version -or $executableInfo.FileVersion.Trim() -ne $identity.version) {
    throw 'The executable version does not match its build identity.'
}
$exeStem = [IO.Path]::GetFileNameWithoutExtension($profile.executable)
$payload = Join-Path $PSScriptRoot "build\payload-$([Guid]::NewGuid().ToString('N'))\$exeStem"
& node scripts/stage-payload.mjs $payload
if ($LASTEXITCODE -ne 0) { throw 'Payload preparation failed.' }
$iscc = (Get-Command ISCC.exe -ErrorAction SilentlyContinue).Source
if (-not $iscc) {
    $iscc = Get-ChildItem "${env:ProgramFiles(x86)}\Inno Setup *\ISCC.exe", "$env:LOCALAPPDATA\Programs\Inno Setup *\ISCC.exe" -ErrorAction SilentlyContinue | Select-Object -First 1 -ExpandProperty FullName
}
if (-not $iscc -and $env:GITHUB_ACTIONS -eq 'true') {
    choco install innosetup -y --no-progress
    if ($LASTEXITCODE -ne 0) { throw 'Inno Setup installation failed.' }
    $iscc = Get-ChildItem "${env:ProgramFiles(x86)}\Inno Setup *\ISCC.exe" | Select-Object -First 1 -ExpandProperty FullName
}
if (-not $iscc) { throw 'Install Inno Setup 6 before packaging.' }
& "$PSScriptRoot\scripts\test-installer.ps1" -Compiler $iscc
& "$PSScriptRoot\scripts\test-installer.ps1" -Compiler $iscc -HoldOpen
$installerStem = [IO.Path]::GetFileNameWithoutExtension($profile.installer_asset)
$repoUrl = "https://github.com/$($profile.owner)/$($profile.repository)"
Write-Host 'Building the Windows installer...'
& $iscc /Q "/DAppVersion=$($identity.version)" "/DVersionNumeric=$numeric" "/DAppName=$($profile.display_name)" "/DPackageId={$($profile.package_id)" "/DExecutable=$($profile.executable)" "/DPayloadRoot=$payload" "/DInstallerName=$installerStem" "/DStorageId=$($profile.storage_id)" "/DPublisher=$($profile.owner)" "/DRepositoryUrl=$repoUrl" installer.iss
if ($LASTEXITCODE -ne 0) { throw 'Installer build failed.' }
$installerInfo = [Diagnostics.FileVersionInfo]::GetVersionInfo((Join-Path $PSScriptRoot "dist_installer\$($profile.installer_asset)"))
if ($installerInfo.ProductVersion.Trim() -ne $identity.version -or $installerInfo.FileVersion.Trim() -ne $numeric) {
    throw 'The installer version does not match its build identity.'
}
