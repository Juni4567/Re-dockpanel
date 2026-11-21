#!/usr/bin/env bash
#
# DockPanel Deployment Script
# Efficiently deploys updates from Git or local sync to the live server
# Usage:
#   /opt/dockpanel/deploy.sh [component] [options]
# Components:
#   auto      - (Default) Pulls latest git and builds ONLY components that changed
#   frontend  - Build and deploy frontend (React/Vite)
#   backend   - Build and deploy backend (Rust API daemon)
#   agent     - Build and deploy agent (Rust Agent daemon)
#   cli       - Build and deploy CLI tool
#   all       - Build and deploy everything
# Options:
#   --no-pull - Do not run git pull before building
#
set -euo pipefail

BASE_DIR="/opt/dockpanel"
API_BIN="/usr/local/bin/dockpanel-api"
AGENT_BIN="/usr/local/bin/dockpanel-agent"
CLI_BIN="/usr/local/bin/dockpanel"
FRONTEND_DIST="$BASE_DIR/panel/frontend/dist"

GREEN='\033[0;32m'
BLUE='\033[0;34m'
YELLOW='\033[1;33m'
RED='\033[0;31m'
BOLD='\033[1m'
NC='\033[0m'

TARGET="${1:-auto}"
PULL=true

for arg in "$@"; do
    if [ "$arg" = "--no-pull" ]; then
        PULL=false
    fi
done

cd "$BASE_DIR"

echo -e "${BLUE}${BOLD}==> DockPanel Deploy: [Target: $TARGET]${NC}"
START_TIME=$(date +%s)

CHANGED_FILES=""
if [ "$PULL" = true ]; then
    echo -e "${GREEN}[+]${NC} Pulling latest commits from origin/main..."
    PREV_HEAD=$(git rev-parse HEAD 2>/dev/null || echo "")
    git fetch origin main --quiet
    git reset --hard origin/main --quiet
    NEW_HEAD=$(git rev-parse HEAD)
    if [ -n "$PREV_HEAD" ] && [ "$PREV_HEAD" != "$NEW_HEAD" ]; then
        CHANGED_FILES=$(git diff --name-only "$PREV_HEAD" "$NEW_HEAD")
        echo -e "${GREEN}[+]${NC} Updated to $(git log -1 --oneline)"
    else
        echo -e "${GREEN}[+]${NC} Already up to date at $(git log -1 --oneline)"
    fi
fi

should_build() {
    local comp="$1"
    if [ "$TARGET" = "all" ] || [ "$TARGET" = "$comp" ]; then
        return 0
    fi
    if [ "$TARGET" = "auto" ]; then
        if [ -z "$CHANGED_FILES" ]; then
            return 1
        fi
        case "$comp" in
            frontend)
                echo "$CHANGED_FILES" | grep -qE '^panel/frontend/' && return 0
                ;;
            backend)
                echo "$CHANGED_FILES" | grep -qE '^panel/backend/' && return 0
                ;;
            agent)
                echo "$CHANGED_FILES" | grep -qE '^panel/agent/' && return 0
                ;;
            cli)
                echo "$CHANGED_FILES" | grep -qE '^panel/cli/' && return 0
                ;;
        esac
    fi
    return 1
}

# 1. Build & Deploy Frontend
if should_build frontend; then
    echo -e "${GREEN}[+]${NC} Building Frontend..."
    (cd "$BASE_DIR/panel/frontend" && npm run build)
    echo -e "${GREEN}[✓]${NC} Frontend updated in $FRONTEND_DIST"
fi

# 2. Build & Deploy Backend (API)
if should_build backend; then
    echo -e "${GREEN}[+]${NC} Building Backend (dockpanel-api)..."
    (cd "$BASE_DIR/panel/backend" && cargo build --release)
    echo -e "${GREEN}[+]${NC} Restarting dockpanel-api service..."
    systemctl stop dockpanel-api
    cp "$BASE_DIR/panel/backend/target/release/dockpanel-api" "$API_BIN"
    chmod +x "$API_BIN"
    systemctl start dockpanel-api
    # Wait for API health
    sleep 1
    if curl -sf http://127.0.0.1:3080/api/health >/dev/null 2>&1; then
        echo -e "${GREEN}[✓]${NC} dockpanel-api restarted and healthy!"
    else
        echo -e "${YELLOW}[!]${NC} dockpanel-api started, verifying status:"
        systemctl status dockpanel-api --no-pager | head -n 8
    fi
fi

# 3. Build & Deploy Agent
if should_build agent; then
    echo -e "${GREEN}[+]${NC} Building Agent (dockpanel-agent)..."
    (cd "$BASE_DIR/panel/agent" && cargo build --release)
    echo -e "${GREEN}[+]${NC} Restarting dockpanel-agent service..."
    systemctl stop dockpanel-agent
    cp "$BASE_DIR/panel/agent/target/release/dockpanel-agent" "$AGENT_BIN"
    chmod +x "$AGENT_BIN"
    systemctl start dockpanel-agent
    sleep 1
    if systemctl is-active --quiet dockpanel-agent; then
        echo -e "${GREEN}[✓]${NC} dockpanel-agent restarted and active!"
    else
        echo -e "${RED}[x]${NC} dockpanel-agent failed to start:"
        systemctl status dockpanel-agent --no-pager | head -n 8
    fi
fi

# 4. Build & Deploy CLI
if should_build cli; then
    echo -e "${GREEN}[+]${NC} Building CLI (dockpanel)..."
    (cd "$BASE_DIR/panel/cli" && cargo build --release)
    cp "$BASE_DIR/panel/cli/target/release/dockpanel" "$CLI_BIN"
    chmod +x "$CLI_BIN"
    echo -e "${GREEN}[✓]${NC} dockpanel CLI updated!"
fi

ELAPSED=$(( $(date +%s) - START_TIME ))
echo -e "${GREEN}${BOLD}==> Deployment finished successfully in ${ELAPSED}s!${NC}"
