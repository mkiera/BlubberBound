@echo off
setlocal
cd /d "%~dp0"
where node >nul 2>nul
if errorlevel 1 goto node_missing
where cargo >nul 2>nul
if errorlevel 1 goto rust_missing
if not exist "node_modules\.bin\tauri.cmd" (
    call npm ci
    if errorlevel 1 goto failed
)
powershell.exe -NoProfile -ExecutionPolicy Bypass -File "%~dp0prepare_tools.ps1"
if errorlevel 1 goto failed
node scripts\build-identity.mjs
if errorlevel 1 goto failed
call npm run build:frontend
if errorlevel 1 goto failed
call npm run tauri -- dev --no-watch --config src-tauri/build-config.json -- -- %*
if errorlevel 1 goto failed
exit /b 0

:node_missing
echo Install Node.js 22 or newer from https://nodejs.org, then reopen run.bat.
pause
exit /b 1

:rust_missing
echo Install Rust from https://rustup.rs and Visual Studio C++ Build Tools, then reopen run.bat.
pause
exit /b 1

:failed
echo The application could not start. The error is shown above.
pause
exit /b 1
