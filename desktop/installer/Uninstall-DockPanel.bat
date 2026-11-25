@echo off
setlocal
echo ========================================================
echo   DockPanel Desktop - Uninstall Launcher
echo ========================================================
echo.

:: Check for administrative privileges
net session >nul 2>&1
if %errorLevel% neq 0 (
    echo Administrative privileges required. Requesting elevation...
    powershell -NoProfile -Command "Start-Process powershell.exe -ArgumentList '-NoProfile -ExecutionPolicy Bypass -File \"\"%~dp0uninstall.ps1\"\"' -Verb RunAs"
    exit /b
)

cd /d "%~dp0"
powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0uninstall.ps1"

echo.
echo Press any key to exit...
pause >nul
