<#
.SYNOPSIS
    Validation suite for DockPanel Desktop Inno Setup offline installer.
.DESCRIPTION
    Validates setup.iss and wsl_prereq.pas for:
    1. Structure and required section headers.
    2. Metadata and configuration properties (AppId, version, admin privileges, etc.).
    3. File existence for all bundled payloads and assets.
    4. Pascal syntax integrity (matching begin/end pairs, required procedures, WSL2/service commands).
    5. Clean compilation via ISCC if compiler is available.
#>

[CmdletBinding()]
param()

$ErrorActionPreference = "Stop"
$ScriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$InstallerDir = Split-Path -Parent $ScriptDir

Write-Host "==========================================================" -ForegroundColor Cyan
Write-Host "DockPanel Desktop - Inno Setup Validation Suite" -ForegroundColor Cyan
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
# Test 1: File Existence
# -------------------------------------------------------------------
Write-Host "`nTest 1: Verifying installer source files..." -ForegroundColor Yellow
$SetupIssPath = Join-Path $InstallerDir "setup.iss"
$WslPasPath = Join-Path $InstallerDir "wsl_prereq.pas"

Assert-Condition (Test-Path $SetupIssPath) "setup.iss exists"
Assert-Condition (Test-Path $WslPasPath) "wsl_prereq.pas exists"

$SetupContent = Get-Content $SetupIssPath -Raw
$WslContent = Get-Content $WslPasPath -Raw

# -------------------------------------------------------------------
# Test 2: Section Headers
# -------------------------------------------------------------------
Write-Host "`nTest 2: Verifying required section headers..." -ForegroundColor Yellow
$RequiredSections = @("[Setup]", "[Languages]", "[Tasks]", "[Files]", "[Icons]", "[Run]", "[UninstallRun]", "[Code]")
foreach ($sec in $RequiredSections) {
    $found = $SetupContent -match [regex]::Escape($sec)
    Assert-Condition $found "Section header '$sec' is present"
}

# -------------------------------------------------------------------
# Test 3: Application Metadata & Setup Properties
# -------------------------------------------------------------------
Write-Host "`nTest 3: Verifying metadata and setup parameters..." -ForegroundColor Yellow

Assert-Condition ($SetupContent -match 'AppId=\{\{C17E4BD3-40FD-4B07-B91F-30A5845B2598\}\}') "AppId matches {{C17E4BD3-40FD-4B07-B91F-30A5845B2598}}"
Assert-Condition ($SetupContent -match 'MyAppName "DockPanel Desktop"') "AppName is 'DockPanel Desktop'"
Assert-Condition ($SetupContent -match 'MyAppVersion "2\.248\.5"') "AppVersion is '2.248.5'"
Assert-Condition ($SetupContent -match 'MyAppPublisher "DockPanel Team"') "AppPublisher is 'DockPanel Team'"
Assert-Condition ($SetupContent -match 'DefaultDirName=\{autopf\}\\') "DefaultDirName uses {autopf}"
Assert-Condition ($SetupContent -match 'OutputBaseFilename=DockPanel-Desktop-Setup-v') "OutputBaseFilename matches convention"
Assert-Condition ($SetupContent -match 'Compression=lzma2/ultra64') "Compression is lzma2/ultra64"
Assert-Condition ($SetupContent -match 'SolidCompression=yes') "SolidCompression is enabled"
Assert-Condition ($SetupContent -match 'PrivilegesRequired=admin') "PrivilegesRequired is admin"
Assert-Condition ($SetupContent -match 'ArchitecturesAllowed=x64compatible') "ArchitecturesAllowed is x64compatible"
Assert-Condition ($SetupContent -match 'ArchitecturesInstallIn64BitMode=x64compatible') "ArchitecturesInstallIn64BitMode is x64compatible"

# -------------------------------------------------------------------
# Test 4: Bundled Files Existence
# -------------------------------------------------------------------
Write-Host "`nTest 4: Verifying all bundled source files exist on disk..." -ForegroundColor Yellow

$Macros = @{
    '{#MyAppName}' = 'DockPanel Desktop'
    '{#MyAppVersion}' = '2.248.5'
    '{#MyAppExeName}' = 'DockPanel Desktop.exe'
}

$FilesSectionMatch = [regex]::Match($SetupContent, '(?s)\[Files\](.*?)(?=\[\w+\]|\z)')
if ($FilesSectionMatch.Success) {
    $FilesBlock = $FilesSectionMatch.Groups[1].Value
    $FileLines = $FilesBlock -split "`r?`n" | Where-Object { $_ -match '^\s*Source:\s*"([^"]+)"' }

    Assert-Condition ($FileLines.Count -ge 5) "Found $($FileLines.Count) bundled file definitions (expected >= 5)"

    foreach ($line in $FileLines) {
        if ($line -match '^\s*Source:\s*"([^"]+)"') {
            $rawSource = $Matches[1]
            $resolvedSource = $rawSource
            foreach ($key in $Macros.Keys) {
                $resolvedSource = $resolvedSource.Replace($key, $Macros[$key])
            }

            # Resolve relative to installer directory
            $fullPath = [System.IO.Path]::GetFullPath([System.IO.Path]::Combine($InstallerDir, $resolvedSource))
            $exists = Test-Path $fullPath
            Assert-Condition $exists "Payload exists: $resolvedSource ($fullPath)"
        }
    }
} else {
    Assert-Condition $false "Failed to locate [Files] section"
}

# -------------------------------------------------------------------
# Test 5: Assets Verification
# -------------------------------------------------------------------
Write-Host "`nTest 5: Verifying asset images and icons..." -ForegroundColor Yellow
$IconPath = Join-Path $InstallerDir "assets\app.ico"
$BannerPath = Join-Path $InstallerDir "assets\banner.bmp"

Assert-Condition (Test-Path $IconPath) "Installer app icon exists at assets\app.ico"
Assert-Condition (Test-Path $BannerPath) "Wizard banner exists at assets\banner.bmp"

if (Test-Path $IconPath) {
    $iconLen = (Get-Item $IconPath).Length
    Assert-Condition ($iconLen -gt 1000) "App icon file size is valid ($iconLen bytes)"
}

if (Test-Path $BannerPath) {
    $bannerLen = (Get-Item $BannerPath).Length
    Assert-Condition ($bannerLen -gt 1000) "Banner BMP file size is valid ($bannerLen bytes)"
}

# -------------------------------------------------------------------
# Test 6: Pascal Script & Procedure Verification
# -------------------------------------------------------------------
Write-Host "`nTest 6: Verifying Pascal script routines..." -ForegroundColor Yellow

$RequiredProcedures = @(
    "CheckAndEnableWSL",
    "ImportWSLDistro",
    "InstallHostHelperService",
    "InstallRootCA",
    "PerformUninstallCleanup"
)

foreach ($proc in $RequiredProcedures) {
    $foundInPas = $WslContent -match "(?i)procedure\s+$proc"
    $foundInIss = $SetupContent -match "(?i)procedure\s+$proc"
    Assert-Condition ($foundInPas -or $foundInIss) "Procedure '$proc' defined"
}

# Verify setup event hooks
Assert-Condition ($SetupContent -match 'procedure\s+CurStepChanged') "Event hook 'CurStepChanged' defined in setup.iss"
Assert-Condition ($SetupContent -match 'procedure\s+CurUninstallStepChanged') "Event hook 'CurUninstallStepChanged' defined in setup.iss"

# Verify required command executions are present in script
Assert-Condition ($WslContent -match '--status') "WSL status probe present"
Assert-Condition ($WslContent -match '--install\s+--no-distribution') "WSL installation command present"
Assert-Condition ($WslContent -match '--import\s+dockpanel-desktop') "WSL distro import command present"
Assert-Condition ($WslContent -match 'create\s+DockPanelHostHelper') "Host helper service creation command present"
Assert-Condition ($WslContent -match 'start\s+DockPanelHostHelper') "Host helper service start command present"
Assert-Condition ($WslContent -match '-install') "Root CA install via mkcert present"
Assert-Condition ($WslContent -match 'stop\s+DockPanelHostHelper') "Host helper service stop command present"
Assert-Condition ($WslContent -match 'delete\s+DockPanelHostHelper') "Host helper service delete command present"
Assert-Condition ($WslContent -match '--unregister\s+dockpanel-desktop') "WSL distro unregister command present"

# Verify Pascal begin/end block integrity
function Test-PascalBlockBalance {
    param([string]$Source)

    # Strip single line comments //
    $clean = $Source -replace '//.*$', ''
    # Strip multi-line comments (* *) and { }
    $clean = $clean -replace '\{[^}]*\}', ''
    $clean = $clean -replace '\(\*.*?\*\)', ''
    # Strip string literals
    $clean = $clean -replace "'[^']*'", "''"

    $begins = ([regex]::Matches($clean, '(?i)\bbegin\b')).Count
    # 'end' or 'end;' or 'end.'
    $ends = ([regex]::Matches($clean, '(?i)\bend\b')).Count

    return @{
        Begins = $begins
        Ends = $ends
        Balanced = ($begins -eq $ends)
    }
}

$WslBalance = Test-PascalBlockBalance $WslContent
Assert-Condition ($WslBalance.Balanced) "wsl_prereq.pas begin/end blocks balanced (begins: $($WslBalance.Begins), ends: $($WslBalance.Ends))"

$IssCodeSection = [regex]::Match($SetupContent, '(?s)\[Code\](.*?)\z')
if ($IssCodeSection.Success) {
    $IssCode = $IssCodeSection.Groups[1].Value
    $IssBalance = Test-PascalBlockBalance $IssCode
    Assert-Condition ($IssBalance.Balanced) "setup.iss [Code] section begin/end blocks balanced (begins: $($IssBalance.Begins), ends: $($IssBalance.Ends))"
}

# -------------------------------------------------------------------
# Test 7: Compiler Dry-Run (if ISCC available)
# -------------------------------------------------------------------
Write-Host "`nTest 7: Inno Setup compiler check..." -ForegroundColor Yellow
$Iscc = Get-Command "iscc.exe" -ErrorAction SilentlyContinue
if (-not $Iscc) {
    $PotentialPaths = @(
        "C:\Program Files (x86)\Inno Setup 6\ISCC.exe",
        "C:\Program Files\Inno Setup 6\ISCC.exe"
    )
    foreach ($p in $PotentialPaths) {
        if (Test-Path $p) {
            $Iscc = $p
            break
        }
    }
}

if ($Iscc) {
    Write-Host "  ISCC found at: $Iscc. Running dry-run compilation..." -ForegroundColor Cyan
    & $Iscc "/Q" $SetupIssPath
    $CompileExit = $LASTEXITCODE
    Assert-Condition ($CompileExit -eq 0) "ISCC compile succeeded with exit code 0"
} else {
    Write-Host "  [INFO] ISCC not installed on runner. Standalone static AST & resource validation completed." -ForegroundColor Cyan
}

# -------------------------------------------------------------------
# Summary
# -------------------------------------------------------------------
Write-Host "`n==========================================================" -ForegroundColor Cyan
Write-Host "Validation Results: Passed=$Passed, Failed=$Failed" -ForegroundColor $(if ($Failed -eq 0) { "Green" } else { "Red" })
Write-Host "==========================================================" -ForegroundColor Cyan

if ($Failed -gt 0) {
    exit 1
} else {
    exit 0
}
