<#
.SYNOPSIS
    Validation suite for DockPanel Desktop native installer.
.DESCRIPTION
    Validates install.ps1, setup.iss, assets, and binaries for:
    1. Structure and required installation steps.
    2. Payload existence (dockpanel.exe, dockpanel-host-helper.exe, mkcert.exe).
    3. Asset images and icons (app.ico, banner.bmp).
    4. PowerShell syntax integrity.
#>

[CmdletBinding()]
param()

$ErrorActionPreference = "Stop"
$ScriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$InstallerDir = Split-Path -Parent $ScriptDir

Write-Host "==========================================================" -ForegroundColor Cyan
Write-Host "DockPanel Desktop - Installer Validation Suite" -ForegroundColor Cyan
Write-Host "Installer Directory: $InstallerDir" -ForegroundColor Cyan
Write-Host "==========================================================" -ForegroundColor Cyan

$Passed = 0
$Failed = 0

function Assert-Condition {
    param(
        [bool]$Condition,
        [string]$Message
    )
    if ($Condition) {
        Write-Host "  [PASS] $Message" -ForegroundColor Green
        $script:Passed++
    } else {
        Write-Host "  [FAIL] $Message" -ForegroundColor Red
        $script:Failed++
    }
}

# -------------------------------------------------------------------
# Test 1: Installer Script & Asset Files Existence
# -------------------------------------------------------------------
Write-Host "`nTest 1: Verifying installer files..." -ForegroundColor Yellow
$InstallPs1Path = Join-Path $InstallerDir "install.ps1"
$InstallBatPath = Join-Path $InstallerDir "Install-DockPanel.bat"
$UninstallPs1Path = Join-Path $InstallerDir "uninstall.ps1"
$UninstallBatPath = Join-Path $InstallerDir "Uninstall-DockPanel.bat"
$SetupIssPath = Join-Path $InstallerDir "setup.iss"

Assert-Condition (Test-Path $InstallPs1Path) "install.ps1 exists"
Assert-Condition (Test-Path $InstallBatPath) "Install-DockPanel.bat exists"
Assert-Condition (Test-Path $UninstallPs1Path) "uninstall.ps1 exists"
Assert-Condition (Test-Path $UninstallBatPath) "Uninstall-DockPanel.bat exists"
Assert-Condition (Test-Path $SetupIssPath) "setup.iss exists"

# -------------------------------------------------------------------
# Test 2: Binaries Existence in bin/
# -------------------------------------------------------------------
Write-Host "`nTest 2: Verifying bundled release binaries..." -ForegroundColor Yellow
$DockpanelExe = Join-Path $InstallerDir "bin\dockpanel.exe"
$HostHelperExe = Join-Path $InstallerDir "bin\dockpanel-host-helper.exe"
$MkcertExe = Join-Path $InstallerDir "bin\mkcert.exe"

Assert-Condition (Test-Path $DockpanelExe) "dockpanel.exe exists in bin/"
Assert-Condition (Test-Path $HostHelperExe) "dockpanel-host-helper.exe exists in bin/"
Assert-Condition (Test-Path $MkcertExe) "mkcert.exe exists in bin/"

if (Test-Path $DockpanelExe) {
    $len = (Get-Item $DockpanelExe).Length
    Assert-Condition ($len -gt 1000000) "dockpanel.exe size is valid ($len bytes)"
}

if (Test-Path $HostHelperExe) {
    $len = (Get-Item $HostHelperExe).Length
    Assert-Condition ($len -gt 100000) "dockpanel-host-helper.exe size is valid ($len bytes)"
}

# -------------------------------------------------------------------
# Test 3: Assets Verification
# -------------------------------------------------------------------
Write-Host "`nTest 3: Verifying asset images and icons..." -ForegroundColor Yellow
$IconPath = Join-Path $InstallerDir "assets\app.ico"
$BannerPath = Join-Path $InstallerDir "assets\banner.bmp"

Assert-Condition (Test-Path $IconPath) "Installer app icon exists at assets\app.ico"
Assert-Condition (Test-Path $BannerPath) "Wizard banner exists at assets\banner.bmp"

# -------------------------------------------------------------------
# Test 4: PowerShell Syntax Validation
# -------------------------------------------------------------------
# -------------------------------------------------------------------
# Test 4: PowerShell Syntax Validation
# -------------------------------------------------------------------
Write-Host "`nTest 4: Verifying PowerShell syntax integrity..." -ForegroundColor Yellow
$tokens = $null
$parseErrors = $null
$null = [System.Management.Automation.Language.Parser]::ParseFile($InstallPs1Path, [ref]$tokens, [ref]$parseErrors)
Assert-Condition ($parseErrors.Count -eq 0) "install.ps1 parsed with 0 syntax errors"

$tokens = $null
$parseErrors = $null
$null = [System.Management.Automation.Language.Parser]::ParseFile($UninstallPs1Path, [ref]$tokens, [ref]$parseErrors)
Assert-Condition ($parseErrors.Count -eq 0) "uninstall.ps1 parsed with 0 syntax errors"

# -------------------------------------------------------------------
# Summary
# -------------------------------------------------------------------
Write-Host "`n==========================================================" -ForegroundColor Cyan
Write-Host "Validation Summary: $Passed Passed, $Failed Failed" -ForegroundColor $(if ($Failed -eq 0) { "Green" } else { "Red" })
Write-Host "==========================================================" -ForegroundColor Cyan

if ($Failed -gt 0) {
    exit 1
}
