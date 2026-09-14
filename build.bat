@echo off
setlocal
cd /d "%~dp0"
powershell.exe -NoProfile -ExecutionPolicy Bypass -File "%~dp0build-package.ps1" -Local
if errorlevel 1 goto failed
echo Packages ready in dist_installer.
exit /b 0

:failed
echo Build failed. The error is shown above.
pause
exit /b 1
