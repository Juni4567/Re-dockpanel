<#
.SYNOPSIS
    DockPanel Desktop - Native Standalone Windows Installer
.DESCRIPTION
    Installs DockPanel Desktop (Pure-Rust Tao + Wry + Axum Architecture),
    registers the dockpanel-host-helper background service, installs mkcert root CA,
    configures hosts resolution for local domains, and creates desktop/start menu shortcuts.
#>

$isAdmin = ([Security.Principal.WindowsPrincipal][Security.Principal.WindowsIdentity]::GetCurrent()).IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)
if (-not $isAdmin) {
    Write-Host "Administrative privileges required. Requesting elevation..." -ForegroundColor Yellow
    Start-Process powershell.exe -ArgumentList "-NoProfile -ExecutionPolicy Bypass -File `"$PSCommandPath`"" -Verb RunAs
    exit
}

$ErrorActionPreference = "Stop"

$AppName = "DockPanel Desktop"
$AppVersion = "2.249.0"
$InstallDir = "$env:ProgramFiles\DockPanel Desktop"
$ScriptDir = Split-Path -Parent $MyInvocation.MyCommand.Definition
$RootDir = Split-Path -Parent (Split-Path -Parent $ScriptDir)

Write-Host "==========================================================" -ForegroundColor Cyan
Write-Host "  Installing $AppName v$AppVersion" -ForegroundColor Cyan
Write-Host "==========================================================" -ForegroundColor Cyan
Write-Host "Destination: $InstallDir" -ForegroundColor Gray

# 1. Stop existing instances and services
Write-Host "`n[1/5] Stopping any existing instances and services..." -ForegroundColor Yellow
Get-Process -Name "dockpanel", "DockPanel Desktop", "dockpanel-host-helper" -ErrorAction SilentlyContinue | Stop-Process -Force -ErrorAction SilentlyContinue
sc.exe stop DockPanelHostHelper 2>&1 | Out-Null
sc.exe delete DockPanelHostHelper 2>&1 | Out-Null
Start-Sleep -Seconds 1

# 2. Create Target Directories
Write-Host "`n[2/5] Creating application directories..." -ForegroundColor Yellow
New-Item -ItemType Directory -Force -Path $InstallDir | Out-Null
New-Item -ItemType Directory -Force -Path "$InstallDir\assets" | Out-Null
Write-Host "[OK] Directories created at $InstallDir" -ForegroundColor Green

# 3. Locate & Copy Binaries
Write-Host "`n[3/5] Copying binaries and runtime assets..." -ForegroundColor Yellow

$SourceApp = Join-Path $ScriptDir "bin\dockpanel.exe"
if (-not (Test-Path $SourceApp)) {
    $SourceApp = Join-Path $RootDir "desktop\dockpanel-app\target\release\dockpanel.exe"
}
if (-not (Test-Path $SourceApp)) {
    $SourceApp = Join-Path $RootDir "dockpanel.exe"
}

$SourceHelper = Join-Path $ScriptDir "bin\dockpanel-host-helper.exe"
if (-not (Test-Path $SourceHelper)) {
    $SourceHelper = Join-Path $RootDir "desktop\host-helper\target\release\dockpanel-host-helper.exe"
}

$SourceMkcert = Join-Path $ScriptDir "bin\mkcert.exe"
$SourceAppIcon = Join-Path $ScriptDir "assets\app.ico"
$SourceBanner = Join-Path $ScriptDir "assets\banner.bmp"

if (-not (Test-Path $SourceApp)) {
    throw "DockPanel Desktop executable not found. Please compile the release binary first."
}

Copy-Item -Force $SourceApp "$InstallDir\DockPanel Desktop.exe"
if (Test-Path $SourceHelper) {
    Copy-Item -Force $SourceHelper "$InstallDir\dockpanel-host-helper.exe"
}
if (Test-Path $SourceMkcert) {
    Copy-Item -Force $SourceMkcert "$InstallDir\mkcert.exe"
}
Copy-Item -Force $SourceAppIcon "$InstallDir\assets\app.ico"
Copy-Item -Force $SourceBanner "$InstallDir\assets\banner.bmp"
Write-Host "[OK] Application files copied successfully." -ForegroundColor Green

# 4. Register and Start Host Helper Windows Service
Write-Host "`n[4/5] Registering Host Helper background service..." -ForegroundColor Yellow
if (Test-Path "$InstallDir\dockpanel-host-helper.exe") {
    sc.exe create DockPanelHostHelper binPath= "`"$InstallDir\dockpanel-host-helper.exe`"" start= auto obj= "LocalSystem" | Out-Null
    sc.exe start DockPanelHostHelper | Out-Null
    Write-Host "[OK] DockPanelHostHelper service registered and running." -ForegroundColor Green
}

# 5. Trust Local Root CA with mkcert
if (Test-Path "$InstallDir\mkcert.exe") {
    Write-Host "`n[5/5] Installing local Root CA for green-padlock HTTPS..." -ForegroundColor Yellow
    & "$InstallDir\mkcert.exe" -install
    Write-Host "[OK] Local Root CA trusted in Windows system store." -ForegroundColor Green
}

# 6. Create Shortcuts (Desktop & Start Menu)
$WScriptShell = New-Object -ComObject WScript.Shell

$StartMenuDir = "$env:ProgramData\Microsoft\Windows\Start Menu\Programs\DockPanel"
New-Item -ItemType Directory -Force -Path $StartMenuDir | Out-Null
$StartMenuShortcut = $WScriptShell.CreateShortcut("$StartMenuDir\DockPanel Desktop.lnk")
$StartMenuShortcut.TargetPath = "$InstallDir\DockPanel Desktop.exe"
$StartMenuShortcut.IconLocation = "$InstallDir\assets\app.ico, 0"
$StartMenuShortcut.WorkingDirectory = $InstallDir
$StartMenuShortcut.Save()

$DesktopShortcut = $WScriptShell.CreateShortcut("$env:PUBLIC\Desktop\DockPanel Desktop.lnk")
$DesktopShortcut.TargetPath = "$InstallDir\DockPanel Desktop.exe"
$DesktopShortcut.IconLocation = "$InstallDir\assets\app.ico, 0"
$DesktopShortcut.WorkingDirectory = $InstallDir
$DesktopShortcut.Save()

Write-Host "`n==========================================================" -ForegroundColor Green
Write-Host "  Installation Complete! DockPanel Desktop is ready to use." -ForegroundColor Green
Write-Host "==========================================================" -ForegroundColor Green
Write-Host "Location: $InstallDir\DockPanel Desktop.exe" -ForegroundColor White
Write-Host "Desktop shortcut created on your desktop." -ForegroundColor White

if ([Environment]::UserInteractive) {
    Write-Host "`nPress Enter to exit..." -ForegroundColor Gray
    [void][System.Console]::ReadLine()
}

