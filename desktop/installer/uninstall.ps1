#Requires -RunAsAdministrator
$ErrorActionPreference = "Continue"

Write-Host "==========================================================" -ForegroundColor Cyan
Write-Host "  Uninstalling DockPanel Desktop..." -ForegroundColor Cyan
Write-Host "==========================================================" -ForegroundColor Cyan

# 1. Stop and remove Host Helper Windows service
Write-Host "`n[1/4] Stopping and removing Host Helper service..." -ForegroundColor Yellow
sc.exe stop DockPanelHostHelper 2>&1 | Out-Null
sc.exe delete DockPanelHostHelper 2>&1 | Out-Null
Write-Host "✓ Host Helper service removed." -ForegroundColor Green

# 2. Unregister WSL2 distro
Write-Host "`n[2/4] Unregistering WSL2 micro-distribution (dockpanel-desktop)..." -ForegroundColor Yellow
wsl.exe --unregister dockpanel-desktop 2>&1 | Out-Null
Write-Host "✓ WSL2 distro unregistered." -ForegroundColor Green

# 3. Remove shortcuts
Write-Host "`n[3/4] Removing shortcuts..." -ForegroundColor Yellow
Remove-Item -Force "$env:PUBLIC\Desktop\DockPanel Desktop.lnk" -ErrorAction SilentlyContinue
Remove-Item -Recurse -Force "$env:ProgramData\Microsoft\Windows\Start Menu\Programs\DockPanel" -ErrorAction SilentlyContinue
Write-Host "✓ Shortcuts removed." -ForegroundColor Green

# 4. Remove installation files
Write-Host "`n[4/4] Removing program files..." -ForegroundColor Yellow
$InstallDir = "$env:ProgramFiles\DockPanel Desktop"
if (Test-Path $InstallDir) {
    Remove-Item -Recurse -Force $InstallDir -ErrorAction SilentlyContinue
}
Write-Host "✓ Program files removed." -ForegroundColor Green

Write-Host "`n==========================================================" -ForegroundColor Green
Write-Host "  Uninstallation Complete." -ForegroundColor Green
Write-Host "==========================================================" -ForegroundColor Green
