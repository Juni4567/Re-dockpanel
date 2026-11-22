<#
.SYNOPSIS
    Deploys DockPanel (Re-dockpanel) changes to your Ubuntu 26.x server.

.DESCRIPTION
    Automates deployment to your production Ubuntu server.
    Supports Git-based automated deployment (smart auto-detecting what changed),
    specific component targets (frontend, backend, agent, all),
    direct local sync without git commits, and live log streaming.

.PARAMETER Target
    Component to deploy: 'auto' (default), 'frontend', 'backend', 'agent', 'cli', 'all'

.PARAMETER DirectSync
    Sync local files directly to the server via SCP/tar without needing to git commit & push first.

.PARAMETER Logs
    Tail live logs from the server (e.g. 'api', 'agent', 'all').

.PARAMETER Status
    Check the current health and status of DockPanel services on the server.

.EXAMPLE
    .\deploy.ps1
    # Pushes to GitHub and updates only changed components on the server

.EXAMPLE
    .\deploy.ps1 -Target backend
    # Deploys backend API and restarts dockpanel-api in ~4 seconds

.EXAMPLE
    .\deploy.ps1 -DirectSync -Target frontend
    # Directly syncs local frontend changes to server without git commit
#>
[CmdletBinding()]
param (
    [ValidateSet('auto', 'frontend', 'backend', 'agent', 'cli', 'all')]
    [string]$Target = 'auto',

    [switch]$DirectSync,

    [string]$Logs,

    [switch]$Status
)

$ErrorActionPreference = "Stop"
$ServerHost = "dockpanel"
$RemoteDir = "/opt/dockpanel"

# 1. Status Check
if ($Status) {
    Write-Host "`n=== DockPanel Server Status ($ServerHost) ===" -ForegroundColor Cyan
    ssh.exe $ServerHost "
        echo '--- System Health ---'
        curl -s http://127.0.0.1:3080/api/health || echo 'API NOT RESPONDING'
        echo ''
        echo '--- Services Status ---'
        systemctl is-active dockpanel-api dockpanel-agent nginx docker
        echo '--- Docker Containers ---'
        docker ps --format 'table {{.Names}}\t{{.Status}}\t{{.Ports}}'
        echo '--- Git Status ---'
        git -C $RemoteDir log -1 --oneline
    "
    exit 0
}

# 2. Live Log Streaming
if ($PSBoundParameters.ContainsKey('Logs')) {
    $unit = switch ($Logs.ToLower()) {
        "api"   { "dockpanel-api" }
        "agent" { "dockpanel-agent" }
        default { "dockpanel-api -u dockpanel-agent" }
    }
    Write-Host "`n=== Streaming live logs for $unit (Ctrl+C to exit) ===" -ForegroundColor Cyan
    ssh.exe -t $ServerHost "journalctl -u $unit -f --no-hostname -o cat"
    exit 0
}

# 3. Direct Local Sync (Bypasses Git for rapid testing)
if ($DirectSync) {
    Write-Host "`n[+] Direct Sync mode active: sending local files to $ServerHost..." -ForegroundColor Yellow
    
    # Determine which folders to sync based on Target
    $syncPaths = switch ($Target) {
        "frontend" { @("panel/frontend") }
        "backend"  { @("panel/backend") }
        "agent"    { @("panel/agent") }
        "cli"      { @("panel/cli") }
        default    { @("panel", "scripts") }
    }

    foreach ($path in $syncPaths) {
        if (Test-Path $path) {
            Write-Host "  -> Syncing $path..." -ForegroundColor Gray
            tar -czf "$env:TEMP\dockpanel_sync.tar.gz" --exclude="node_modules" --exclude="target" --exclude="dist" -C . $path
            scp.exe "$env:TEMP\dockpanel_sync.tar.gz" "${ServerHost}:${RemoteDir}/"
            ssh.exe $ServerHost "cd $RemoteDir && tar -xzf dockpanel_sync.tar.gz && rm -f dockpanel_sync.tar.gz"
            Remove-Item "$env:TEMP\dockpanel_sync.tar.gz" -Force
        }
    }

    Write-Host "[+] Local sync completed. Triggering build for target: $Target..." -ForegroundColor Green
    ssh.exe $ServerHost "$RemoteDir/deploy.sh $Target --no-pull"
    exit 0
}

# 4. Standard Git-based Deployment
Write-Host "`n=== DockPanel Deployment: Target [$Target] ===" -ForegroundColor Cyan

# Check if there are unpushed local git commits
$unpushed = git log origin/main..HEAD --oneline 2>$null
if ($unpushed) {
    Write-Host "[!] Found unpushed local commits:" -ForegroundColor Yellow
    $unpushed | ForEach-Object { Write-Host "    $_" -ForegroundColor DarkGray }
    Write-Host "[+] Pushing to origin/main..." -ForegroundColor Green
    git push origin main
} else {
    # Check if working directory has uncommitted changes
    $uncommitted = git status -s
    if ($uncommitted) {
        Write-Host "[!] Notice: You have uncommitted local changes:" -ForegroundColor Yellow
        $uncommitted | ForEach-Object { Write-Host "    $_" -ForegroundColor DarkGray }
        Write-Host "    (Tip: Use '.\deploy.ps1 -DirectSync' to test uncommitted changes directly)" -ForegroundColor Gray
    }
}

# Trigger remote deployment script
Write-Host "[+] Triggering remote build on server..." -ForegroundColor Green
ssh.exe $ServerHost "$RemoteDir/deploy.sh $Target"
