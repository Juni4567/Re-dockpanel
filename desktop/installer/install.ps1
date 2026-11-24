#Requires -RunAsAdministrator
<#
.SYNOPSIS
    DockPanel Desktop - Native Standalone Windows Installer
.DESCRIPTION
    Installs DockPanel Desktop, imports the WSL2 micro-distro, registers the
    dockpanel-host-helper background service, installs mkcert root CA,
    and creates desktop/start menu shortcuts.
#>

$ErrorActionPreference = "Stop"

$AppName = "DockPanel Desktop"
$AppVersion = "2.248.5"
$InstallDir = "$env:ProgramFiles\DockPanel Desktop"
$ScriptDir = Split-Path -Parent $MyInvocation.MyCommand.Definition
$RootDir = Split-Path -Parent (Split-Path -Parent $ScriptDir)

Write-Host "==========================================================" -ForegroundColor Cyan
Write-Host "  Installing $AppName v$AppVersion" -ForegroundColor Cyan
Write-Host "==========================================================" -ForegroundColor Cyan
Write-Host "Destination: $InstallDir" -ForegroundColor Gray

# 1. Check & Ensure WSL2 is Available
Write-Host "`n[1/6] Checking WSL2 prerequisites..." -ForegroundColor Yellow
$wslStatus = wsl.exe --status 2>&1
if ($LASTEXITCODE -ne 0) {
    Write-Host "WSL2 is not active. Enabling WSL2 (no distribution)..." -ForegroundColor Yellow
    wsl.exe --install --no-distribution
    Write-Host "WSL2 installation command issued. If this is a first-time setup, a system reboot may be required." -ForegroundColor Yellow
} else {
    Write-Host "✓ WSL2 is installed and ready." -ForegroundColor Green
}

# 2. Create Target Directories
Write-Host "`n[2/6] Creating application directories..." -ForegroundColor Yellow
New-Item -ItemType Directory -Force -Path $InstallDir | Out-Null
New-Item -ItemType Directory -Force -Path "$InstallDir\runtime\distro" | Out-Null
New-Item -ItemType Directory -Force -Path "$InstallDir\assets" | Out-Null
Write-Host "✓ Directories created at $InstallDir" -ForegroundColor Green

# 3. Copy Application Binaries & Payloads
Write-Host "`n[3/6] Copying binaries and runtime assets..." -ForegroundColor Yellow

$SourceTauri = Join-Path $RootDir "desktop\src-tauri\target\release\DockPanel Desktop.exe"
$SourceHelper = Join-Path $RootDir "desktop\host-helper\target\release\dockpanel-host-helper.exe"
$SourceRuntime = Join-Path $RootDir "desktop\runtime\dockpanel-runtime.tar.gz"
$SourceMkcert = Join-Path $ScriptDir "bin\mkcert.exe"
$SourceAppIcon = Join-Path $ScriptDir "assets\app.ico"
$SourceBanner = Join-Path $ScriptDir "assets\banner.bmp"

Copy-Item -Force $SourceTauri "$InstallDir\DockPanel Desktop.exe"
Copy-Item -Force $SourceHelper "$InstallDir\dockpanel-host-helper.exe"
Copy-Item -Force $SourceRuntime "$InstallDir\runtime\dockpanel-runtime.tar.gz"
Copy-Item -Force $SourceMkcert "$InstallDir\mkcert.exe"
Copy-Item -Force $SourceAppIcon "$InstallDir\assets\app.ico"
Copy-Item -Force $SourceBanner "$InstallDir\assets\banner.bmp"
Write-Host "✓ Application files copied successfully." -ForegroundColor Green

# 4. Import WSL2 Micro-Distribution
Write-Host "`n[4/6] Setting up WSL2 micro-distribution (dockpanel-desktop)..." -ForegroundColor Yellow
$existingDistros = wsl.exe -l -q 2>&1
if ($existingDistros -match "dockpanel-desktop") {
    Write-Host "✓ 'dockpanel-desktop' distro already imported." -ForegroundColor Green
} else {
    Write-Host "Importing dockpanel-desktop container rootfs..." -ForegroundColor Gray
    wsl.exe --import dockpanel-desktop "$InstallDir\runtime\distro" "$InstallDir\runtime\dockpanel-runtime.tar.gz" --version 2
    if ($LASTEXITCODE -eq 0) {
        Write-Host "✓ 'dockpanel-desktop' imported successfully." -ForegroundColor Green
    } else {
        Write-Warning "WSL import exited with code $LASTEXITCODE. You can import manually later using: wsl --import dockpanel-desktop `"$InstallDir\runtime\distro`" `"$InstallDir\runtime\dockpanel-runtime.tar.gz`" --version 2"
    }
}

# 5. Install & Start Windows Host Helper Service
Write-Host "`n[5/6] Registering Zero-UAC Host Helper service..." -ForegroundColor Yellow
sc.exe stop DockPanelHostHelper 2>&1 | Out-Null
sc.exe delete DockPanelHostHelper 2>&1 | Out-Null
sc.exe create DockPanelHostHelper binPath= "`"$InstallDir\dockpanel-host-helper.exe`"" start= auto obj= "LocalSystem" | Out-Null
sc.exe start DockPanelHostHelper | Out-Null
Write-Host "✓ DockPanelHostHelper service registered and started." -ForegroundColor Green

# 6. Trust Local Root CA with mkcert
Write-Host "`n[6/6] Installing local Root CA for green-padlock HTTPS..." -ForegroundColor Yellow
& "$InstallDir\mkcert.exe" -install
Write-Host "✓ Local Root CA trusted in Windows system store." -ForegroundColor Green

# 7. Create Shortcuts (Desktop & Start Menu)
$WScriptShell = New-Object -ComObject WScript.Shell

# Start Menu Shortcut
$StartMenuDir = "$env:ProgramData\Microsoft\Windows\Start Menu\Programs\DockPanel"
New-Item -ItemType Directory -Force -Path $StartMenuDir | Out-Null
$StartMenuShortcut = $WScriptShell.CreateShortcut("$StartMenuDir\DockPanel Desktop.lnk")
$StartMenuShortcut.TargetPath = "$InstallDir\DockPanel Desktop.exe"
$StartMenuShortcut.IconLocation = "$InstallDir\assets\app.ico, 0"
$StartMenuShortcut.WorkingDirectory = $InstallDir
$StartMenuShortcut.Save()

# Desktop Shortcut
$DesktopShortcut = $WScriptShell.CreateShortcut("$env:PUBLIC\Desktop\DockPanel Desktop.lnk")
$DesktopShortcut.TargetPath = "$InstallDir\DockPanel Desktop.exe"
$DesktopShortcut.IconLocation = "$InstallDir\assets\app.ico, 0"
$DesktopShortcut.WorkingDirectory = $InstallDir
$DesktopShortcut.Save()

Write-Host "`n==========================================================" -ForegroundColor Green
Write-Host "  Installation Complete! DockPanel Desktop is ready to use." -ForegroundColor Green
Write-Host "==========================================================" -ForegroundColor Green
Write-Host "Location: $InstallDir\DockPanel Desktop.exe" -ForegroundColor White
Write-Host "Desktop shortcut created." -ForegroundColor White
