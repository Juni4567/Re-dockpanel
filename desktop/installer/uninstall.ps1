$isAdmin = ([Security.Principal.WindowsPrincipal][Security.Principal.WindowsIdentity]::GetCurrent()).IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)
if (-not $isAdmin) {
    Write-Host "Administrative privileges required. Requesting elevation..." -ForegroundColor Yellow
    Start-Process powershell.exe -ArgumentList "-NoProfile -ExecutionPolicy Bypass -File `"$PSCommandPath`"" -Verb RunAs
    exit
}

$ErrorActionPreference = "Continue"

Write-Host "==========================================================" -ForegroundColor Cyan
Write-Host "  Uninstalling DockPanel Desktop..." -ForegroundColor Cyan
Write-Host "==========================================================" -ForegroundColor Cyan

# 1. Stop and remove Host Helper Windows service
Write-Host "`n[1/3] Stopping and removing Host Helper service..." -ForegroundColor Yellow
Get-Process -Name "dockpanel", "DockPanel Desktop", "dockpanel-host-helper" -ErrorAction SilentlyContinue | Stop-Process -Force -ErrorAction SilentlyContinue
sc.exe stop DockPanelHostHelper 2>&1 | Out-Null
sc.exe delete DockPanelHostHelper 2>&1 | Out-Null
Write-Host "✓ Host Helper service removed." -ForegroundColor Green

# 2. Clean up DockPanel domains from hosts file
Write-Host "`n[2/3] Cleaning up local domain mappings in hosts file..." -ForegroundColor Yellow
$hostsPath = "$env:SystemRoot\System32\drivers\etc\hosts"
if (Test-Path $hostsPath) {
    $lines = Get-Content $hostsPath
    $cleaned = $lines | Where-Object { $_ -notmatch "# dockpanel-desktop" }
    $cleaned | Set-Content $hostsPath
    Write-Host "✓ Hosts file cleaned." -ForegroundColor Green
}

# 3. Remove shortcuts & program files
Write-Host "`n[3/3] Removing shortcuts and program files..." -ForegroundColor Yellow
Remove-Item -Force "$env:PUBLIC\Desktop\DockPanel Desktop.lnk" -ErrorAction SilentlyContinue
Remove-Item -Recurse -Force "$env:ProgramData\Microsoft\Windows\Start Menu\Programs\DockPanel" -ErrorAction SilentlyContinue
$InstallDir = "$env:ProgramFiles\DockPanel Desktop"
if (Test-Path $InstallDir) {
    Remove-Item -Recurse -Force $InstallDir -ErrorAction SilentlyContinue
}
Write-Host "✓ Program files and shortcuts removed." -ForegroundColor Green

Write-Host "`n==========================================================" -ForegroundColor Green
Write-Host "  Uninstallation Complete." -ForegroundColor Green
Write-Host "==========================================================" -ForegroundColor Green
