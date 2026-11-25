@echo off
setlocal
echo ========================================================
echo   DockPanel Desktop - Setup Launcher
echo ========================================================
echo.

if exist "%~dp0DockPanel-Desktop-Setup.exe" (
    echo Launching DockPanel Desktop Setup Wizard...
    start "" "%~dp0DockPanel-Desktop-Setup.exe"
    exit /b
)

:: Check for administrative privileges
net session >nul 2>&1
if %errorLevel% neq 0 (
    echo Administrative privileges required. Requesting elevation...
    powershell -NoProfile -Command "Start-Process powershell.exe -ArgumentList '-NoProfile -ExecutionPolicy Bypass -File \"\"%~dp0install.ps1\"\"' -Verb RunAs"
    exit /b
)

cd /d "%~dp0"
powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0install.ps1"

echo.
echo Press any key to exit...
pause >nul
