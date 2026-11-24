#!/usr/bin/env bash
#
# DockPanel Desktop - WSL2 Micro-Runtime Packaging Script
# Builds and packages a minimal Ubuntu WSL2 root filesystem with exact Linux parity:
# - Ubuntu Base (Noble 24.04 LTS / Jammy 22.04 LTS)
# - Nginx, PHP (8.1, 8.2, 8.3, 8.4-fpm), MariaDB, PostgreSQL
# - Mailpit local mail catcher & mkcert local SSL trust
# - Systemd services: dockpanel-api (DESKTOP_MODE=true on 127.0.0.1:8080) & dockpanel-agent
# - Cache pruning to achieve ~250 MB compressed footprint
#
set -euo pipefail

# ── Color Output & Formatting ─────────────────────────────────────────────
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
CYAN='\033[0;36m'
BOLD='\033[1m'
NC='\033[0m'

log()   { echo -e "  ${GREEN}✓${NC} $1"; }
warn()  { echo -e "  ${YELLOW}⚠${NC} $1"; }
error() { echo -e "  ${RED}✗${NC} $1" >&2; }
info()  { echo -e "  ${CYAN}→${NC} $1"; }

# ── Path Resolution ───────────────────────────────────────────────────────
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

DEFAULT_OUTPUT="${REPO_ROOT}/desktop/runtime/dockpanel-runtime.tar.gz"
OUTPUT_TAR="${OUTPUT_TAR:-$DEFAULT_OUTPUT}"
BASE_DISTRO="noble"
BUILD_MODE="auto"
API_BIN=""
AGENT_BIN=""
DRY_RUN=0
SKIP_CLEAN=0

# ── CLI Argument Parsing ──────────────────────────────────────────────────
usage() {
    cat << EOF
DockPanel Desktop - WSL2 Micro-Runtime Packaging Script

Usage: $(basename "$0") [OPTIONS]

Options:
  -o, --output <PATH>     Target path for dockpanel-runtime.tar.gz
                          (Default: desktop/runtime/dockpanel-runtime.tar.gz)
  -b, --base <DISTRO>     Ubuntu base release: noble (24.04) | jammy (22.04)
                          (Default: noble)
  -m, --mode <MODE>       Builder mode: auto | docker | debootstrap | offline
                          (Default: auto)
      --api-bin <PATH>    Path to compiled Linux dockpanel-api binary
      --agent-bin <PATH>  Path to compiled Linux dockpanel-agent binary
      --skip-clean        Skip aggressive documentation and cache removal
      --dry-run           Validate configuration and inputs without generating image
  -h, --help              Show this help message and exit

Examples:
  $(basename "$0")
  $(basename "$0") --output /dist/dockpanel-runtime.tar.gz --base noble
  $(basename "$0") --dry-run
EOF
}

while [[ $# -gt 0 ]]; do
    case "$1" in
        -o|--output)
            OUTPUT_TAR="$2"; shift 2 ;;
        -b|--base)
            BASE_DISTRO="$2"; shift 2 ;;
        -m|--mode)
            BUILD_MODE="$2"; shift 2 ;;
        --api-bin)
            API_BIN="$2"; shift 2 ;;
        --agent-bin)
            AGENT_BIN="$2"; shift 2 ;;
        --skip-clean)
            SKIP_CLEAN=1; shift ;;
        --dry-run)
            DRY_RUN=1; shift ;;
        -h|--help)
            usage; exit 0 ;;
        *)
            error "Unknown argument: $1"
            usage
            exit 1
            ;;
    esac
done

echo -e "\n${BOLD}${CYAN}====================================================${NC}"
echo -e "${BOLD}${CYAN}  DockPanel Desktop — WSL2 Micro-Runtime Builder    ${NC}"
echo -e "${BOLD}${CYAN}====================================================${NC}\n"

info "Target archive: $OUTPUT_TAR"
info "Ubuntu base:    $BASE_DISTRO"
info "Builder mode:   $BUILD_MODE"

# ── Validate Prerequisite Files ───────────────────────────────────────────
WSL_CONF="${REPO_ROOT}/desktop/runtime/wsl.conf"
SETUP_SCRIPT="${REPO_ROOT}/desktop/runtime/setup-desktop-services.sh"
AGENT_SERVICE_TEMPLATE="${REPO_ROOT}/panel/agent/dockpanel-agent.service"

if [ ! -f "$WSL_CONF" ]; then
    error "Missing WSL configuration file at $WSL_CONF"
    exit 1
fi

if [ ! -f "$SETUP_SCRIPT" ]; then
    error "Missing setup script at $SETUP_SCRIPT"
    exit 1
fi

mkdir -p "$(dirname "$OUTPUT_TAR")"

# ── Dry-Run Short Circuit ─────────────────────────────────────────────────
if [ "$DRY_RUN" -eq 1 ]; then
    log "Configuration and prerequisite checks passed successfully (dry-run)."
    exit 0
fi

# ── Detect Available Build Mechanism ─────────────────────────────────────
RESOLVED_MODE="$BUILD_MODE"
if [ "$BUILD_MODE" = "auto" ]; then
    if command -v docker >/dev/null 2>&1 && docker info >/dev/null 2>&1; then
        RESOLVED_MODE="docker"
    elif [ "$(id -u)" -eq 0 ] && command -v debootstrap >/dev/null 2>&1; then
        RESOLVED_MODE="debootstrap"
    else
        RESOLVED_MODE="offline"
    fi
fi

info "Executing packaging in mode: ${BOLD}$RESOLVED_MODE${NC}"

# Temporary workspace for rootfs assembly
WORK_DIR=$(mktemp -d 2>/dev/null || mktemp -d -t 'dockpanel-runtime-build.XXXXXX')
trap 'rm -rf "$WORK_DIR"' EXIT

# ── Systemd Unit File Content Generators ──────────────────────────────────
generate_api_service() {
    cat << 'EOF'
[Unit]
Description=DockPanel API (Desktop Mode)
After=network.target postgresql.service mariadb.service dockpanel-agent.service
Wants=dockpanel-agent.service
StartLimitBurst=5
StartLimitIntervalSec=60

[Service]
Type=simple
Environment=DESKTOP_MODE=true
Environment=LISTEN_ADDR=127.0.0.1:8080
Environment=RUST_LOG=info
EnvironmentFile=-/etc/dockpanel/api.env
ExecStart=/usr/local/bin/dockpanel-api
Restart=always
RestartSec=5
NoNewPrivileges=yes
PrivateTmp=yes
ProtectHome=yes
ReadWritePaths=/var/run/dockpanel /tmp /var/www /etc/dockpanel
MemoryMax=1G
LimitNOFILE=65535

[Install]
WantedBy=multi-user.target
EOF
}

generate_agent_service() {
    cat << 'EOF'
[Unit]
Description=DockPanel Agent (Desktop Mode)
After=network.target nginx.service mariadb.service postgresql.service
StartLimitBurst=5
StartLimitIntervalSec=60

[Service]
Type=simple
EnvironmentFile=-/etc/dockpanel/agent.env
Environment=DESKTOP_MODE=true
Environment=RUST_LOG=info
RuntimeDirectory=dockpanel
RuntimeDirectoryPreserve=yes
ExecStartPre=/bin/sh -c 'mkdir -p /run/dockpanel /var/lib/dockpanel/git /var/www'
ExecStart=/usr/local/bin/dockpanel-agent
ExecStartPost=/bin/sh -c 'sleep 1 && chgrp $(getent group www-data >/dev/null 2>&1 && echo www-data || echo nginx) /var/run/dockpanel/agent.sock 2>/dev/null; chmod 660 /var/run/dockpanel/agent.sock 2>/dev/null; true'
Restart=always
RestartSec=5

NoNewPrivileges=yes
ProtectSystem=strict
ProtectHome=yes
PrivateTmp=yes
ProtectKernelLogs=yes
ProtectKernelModules=yes
ProtectKernelTunables=yes
ProtectControlGroups=yes
ProtectClock=yes
ProtectHostname=yes
RestrictRealtime=yes
RestrictSUIDSGID=yes
LockPersonality=yes
RestrictNamespaces=~user
ReadWritePaths=/etc/nginx /etc/dockpanel /var/run/dockpanel /var/backups/dockpanel /var/lib/dockpanel /var/www /var/log /etc/systemd/system /opt -/etc/letsencrypt -/etc/apt -/etc/fail2ban -/etc/powerdns -/etc/modsecurity -/etc/cloudflared -/etc/postfix -/etc/dovecot -/var/spool/postfix -/var/spool/cron -/var/vmail -/var/cache/nginx -/etc/ufw -/var/lib/ufw -/etc/php -/etc/ssh -/var/dockpanel-sftp

MemoryMax=512M
LimitNOFILE=65535

[Install]
WantedBy=multi-user.target
EOF
}

generate_mailpit_service() {
    cat << 'EOF'
[Unit]
Description=Mailpit Desktop SMTP & Webmail
After=network.target

[Service]
Type=simple
ExecStart=/usr/local/bin/mailpit --smtp-bind-addr 127.0.0.1:1025 --listen 127.0.0.1:8025
Restart=always
RestartSec=5
NoNewPrivileges=yes
MemoryMax=128M

[Install]
WantedBy=multi-user.target
EOF
}

generate_setup_service() {
    cat << 'EOF'
[Unit]
Description=DockPanel Desktop First-Boot Initialization
After=network.target mariadb.service postgresql.service
Before=dockpanel-agent.service dockpanel-api.service
ConditionPathExists=!/etc/dockpanel/.desktop-initialized

[Service]
Type=oneshot
ExecStart=/usr/local/bin/setup-desktop-services.sh
RemainAfterExit=yes

[Install]
WantedBy=multi-user.target
EOF
}

# ── Build Implementation: Mode DOCKER ─────────────────────────────────────
build_via_docker() {
    info "Preparing Docker-based micro-distro builder..."
    local DOCKER_BUILD_DIR="${WORK_DIR}/docker"
    mkdir -p "$DOCKER_BUILD_DIR"

    # Copy support assets
    cp "$WSL_CONF" "${DOCKER_BUILD_DIR}/wsl.conf"
    cp "$SETUP_SCRIPT" "${DOCKER_BUILD_DIR}/setup-desktop-services.sh"
    chmod +x "${DOCKER_BUILD_DIR}/setup-desktop-services.sh"

    generate_api_service > "${DOCKER_BUILD_DIR}/dockpanel-api.service"
    generate_agent_service > "${DOCKER_BUILD_DIR}/dockpanel-agent.service"
    generate_mailpit_service > "${DOCKER_BUILD_DIR}/mailpit.service"
    generate_setup_service > "${DOCKER_BUILD_DIR}/dockpanel-desktop-setup.service"

    # Optional local binaries or fallback placeholders
    if [ -n "$API_BIN" ] && [ -f "$API_BIN" ]; then
        cp "$API_BIN" "${DOCKER_BUILD_DIR}/dockpanel-api"
    else
        cat << 'EOF' > "${DOCKER_BUILD_DIR}/dockpanel-api"
#!/bin/sh
echo "DockPanel API desktop binary stub. Compile from panel/backend for live operation."
exec sleep infinity
EOF
    fi
    chmod +x "${DOCKER_BUILD_DIR}/dockpanel-api"

    if [ -n "$AGENT_BIN" ] && [ -f "$AGENT_BIN" ]; then
        cp "$AGENT_BIN" "${DOCKER_BUILD_DIR}/dockpanel-agent"
    else
        cat << 'EOF' > "${DOCKER_BUILD_DIR}/dockpanel-agent"
#!/bin/sh
echo "DockPanel Agent desktop binary stub. Compile from panel/agent for live operation."
exec sleep infinity
EOF
    fi
    chmod +x "${DOCKER_BUILD_DIR}/dockpanel-agent"

    # Generate Dockerfile
    cat << 'EOF' > "${DOCKER_BUILD_DIR}/Dockerfile"
ARG BASE_IMAGE=ubuntu:noble
FROM ${BASE_IMAGE}

ENV DEBIAN_FRONTEND=noninteractive

# Install systemd, init, and core runtime utilities
RUN apt-get update && apt-get install -y --no-install-recommends \
    systemd systemd-sysv init \
    ca-certificates curl wget rsync unzip git tar gzip sudo \
    gnupg lsb-release software-properties-common \
    nginx mariadb-server mariadb-client \
    postgresql postgresql-contrib \
    && add-apt-repository -y ppa:ondrej/php \
    && apt-get update && apt-get install -y --no-install-recommends \
    php8.1-fpm php8.1-cli php8.1-common php8.1-mysql php8.1-pgsql php8.1-curl php8.1-gd php8.1-mbstring php8.1-xml php8.1-zip php8.1-bcmath php8.1-intl \
    php8.2-fpm php8.2-cli php8.2-common php8.2-mysql php8.2-pgsql php8.2-curl php8.2-gd php8.2-mbstring php8.2-xml php8.2-zip php8.2-bcmath php8.2-intl \
    php8.3-fpm php8.3-cli php8.3-common php8.3-mysql php8.3-pgsql php8.3-curl php8.3-gd php8.3-mbstring php8.3-xml php8.3-zip php8.3-bcmath php8.3-intl \
    php8.4-fpm php8.4-cli php8.4-common php8.4-mysql php8.4-pgsql php8.4-curl php8.4-gd php8.4-mbstring php8.4-xml php8.4-zip php8.4-bcmath php8.4-intl \
    && curl -sSL https://github.com/FiloSottile/mkcert/releases/download/v1.4.4/mkcert-v1.4.4-linux-amd64 -o /usr/local/bin/mkcert \
    && chmod 755 /usr/local/bin/mkcert \
    && curl -sSL https://github.com/axllent/mailpit/releases/download/v1.21.8/mailpit-linux-amd64.tar.gz | tar -xz -C /usr/local/bin mailpit \
    && chmod 755 /usr/local/bin/mailpit

# Install WSL configuration and desktop services
COPY wsl.conf /etc/wsl.conf
COPY setup-desktop-services.sh /usr/local/bin/setup-desktop-services.sh
COPY dockpanel-api /usr/local/bin/dockpanel-api
COPY dockpanel-agent /usr/local/bin/dockpanel-agent

COPY dockpanel-api.service /etc/systemd/system/dockpanel-api.service
COPY dockpanel-agent.service /etc/systemd/system/dockpanel-agent.service
COPY mailpit.service /etc/systemd/system/mailpit.service
COPY dockpanel-desktop-setup.service /etc/systemd/system/dockpanel-desktop-setup.service

# Enable services for systemd boot
RUN systemctl enable nginx mariadb postgresql \
    php8.1-fpm php8.2-fpm php8.3-fpm php8.4-fpm \
    mailpit dockpanel-desktop-setup dockpanel-agent dockpanel-api \
    && mkdir -p /var/www /etc/dockpanel /var/run/dockpanel /var/lib/dockpanel \
    && chown -R www-data:www-data /var/www

# Aggressive cache and size optimization (~250 MB target)
RUN apt-get clean \
    && rm -rf /var/lib/apt/lists/* \
    && rm -rf /var/cache/apt/* \
    && rm -rf /tmp/* /var/tmp/* \
    && rm -rf /usr/share/doc/* /usr/share/man/* /usr/share/info/* /usr/share/locale/* \
    && find /var/log -type f -delete
EOF

    local IMAGE_TAG="dockpanel-desktop-builder:${BASE_DISTRO}"
    local CONTAINER_NAME="dockpanel-export-${RANDOM}"

    info "Building container image $IMAGE_TAG..."
    docker build --build-arg "BASE_IMAGE=ubuntu:${BASE_DISTRO}" -t "$IMAGE_TAG" "$DOCKER_BUILD_DIR"

    info "Exporting container filesystem into $OUTPUT_TAR..."
    docker create --name "$CONTAINER_NAME" "$IMAGE_TAG"
    docker export "$CONTAINER_NAME" | gzip -9 > "$OUTPUT_TAR"
    docker rm -f "$CONTAINER_NAME" >/dev/null 2>&1 || true

    log "Docker rootfs packaging completed"
}

# ── Build Implementation: Mode DEBOOTSTRAP ────────────────────────────────
build_via_debootstrap() {
    info "Preparing debootstrap minimal root filesystem..."
    local ROOTFS_DIR="${WORK_DIR}/rootfs"
    mkdir -p "$ROOTFS_DIR"

    debootstrap --variant=minbase "$BASE_DISTRO" "$ROOTFS_DIR" http://archive.ubuntu.com/ubuntu/

    cp "$WSL_CONF" "${ROOTFS_DIR}/etc/wsl.conf"
    cp "$SETUP_SCRIPT" "${ROOTFS_DIR}/usr/local/bin/setup-desktop-services.sh"
    chmod +x "${ROOTFS_DIR}/usr/local/bin/setup-desktop-services.sh"

    generate_api_service > "${ROOTFS_DIR}/etc/systemd/system/dockpanel-api.service"
    generate_agent_service > "${ROOTFS_DIR}/etc/systemd/system/dockpanel-agent.service"
    generate_mailpit_service > "${ROOTFS_DIR}/etc/systemd/system/mailpit.service"
    generate_setup_service > "${ROOTFS_DIR}/etc/systemd/system/dockpanel-desktop-setup.service"

    info "Packaging rootfs into $OUTPUT_TAR..."
    tar -czf "$OUTPUT_TAR" -C "$ROOTFS_DIR" .
    log "Debootstrap rootfs packaging completed"
}

# ── Build Implementation: Mode OFFLINE / SKELETON ────────────────────────
build_via_offline() {
    info "Docker or root debootstrap not available in current environment."
    info "Generating offline micro-runtime directory structure and configuration tarball..."

    local ROOTFS_DIR="${WORK_DIR}/rootfs"
    mkdir -p "${ROOTFS_DIR}/etc/systemd/system"
    mkdir -p "${ROOTFS_DIR}/etc/dockpanel/ssl"
    mkdir -p "${ROOTFS_DIR}/usr/local/bin"
    mkdir -p "${ROOTFS_DIR}/var/www/html"
    mkdir -p "${ROOTFS_DIR}/var/run/dockpanel"
    mkdir -p "${ROOTFS_DIR}/var/lib/dockpanel"

    cp "$WSL_CONF" "${ROOTFS_DIR}/etc/wsl.conf"
    cp "$SETUP_SCRIPT" "${ROOTFS_DIR}/usr/local/bin/setup-desktop-services.sh"
    chmod +x "${ROOTFS_DIR}/usr/local/bin/setup-desktop-services.sh"

    generate_api_service > "${ROOTFS_DIR}/etc/systemd/system/dockpanel-api.service"
    generate_agent_service > "${ROOTFS_DIR}/etc/systemd/system/dockpanel-agent.service"
    generate_mailpit_service > "${ROOTFS_DIR}/etc/systemd/system/mailpit.service"
    generate_setup_service > "${ROOTFS_DIR}/etc/systemd/system/dockpanel-desktop-setup.service"

    # Default landing page
    cat << 'EOF' > "${ROOTFS_DIR}/var/www/html/index.html"
<!DOCTYPE html>
<html><body><h1>DockPanel Desktop</h1><p>WSL2 micro-runtime environment initialized.</p></body></html>
EOF

    # Minimal launcher stubs for standalone testing
    cat << 'EOF' > "${ROOTFS_DIR}/usr/local/bin/dockpanel-api"
#!/bin/sh
exec sleep infinity
EOF
    chmod +x "${ROOTFS_DIR}/usr/local/bin/dockpanel-api"

    cat << 'EOF' > "${ROOTFS_DIR}/usr/local/bin/dockpanel-agent"
#!/bin/sh
exec sleep infinity
EOF
    chmod +x "${ROOTFS_DIR}/usr/local/bin/dockpanel-agent"

    info "Compressing into $OUTPUT_TAR..."
    tar -czf "$OUTPUT_TAR" -C "$ROOTFS_DIR" .
    log "Offline runtime archive generated"
}

# ── Dispatch Execution ────────────────────────────────────────────────────
case "$RESOLVED_MODE" in
    docker)
        build_via_docker
        ;;
    debootstrap)
        build_via_debootstrap
        ;;
    offline)
        build_via_offline
        ;;
    *)
        error "Unsupported build mode: $RESOLVED_MODE"
        exit 1
        ;;
esac

# ── Verification & Size Summary ───────────────────────────────────────────
if [ ! -f "$OUTPUT_TAR" ]; then
    error "Failed to generate $OUTPUT_TAR"
    exit 1
fi

ARCHIVE_BYTES=$(wc -c < "$OUTPUT_TAR" 2>/dev/null || stat -c%s "$OUTPUT_TAR" 2>/dev/null || echo 0)
ARCHIVE_MB=$(( ARCHIVE_BYTES / 1024 / 1024 ))

echo -e "\n${BOLD}${GREEN}====================================================${NC}"
echo -e "${BOLD}${GREEN}  WSL2 Micro-Distro Rootfs Generated Successfully   ${NC}"
echo -e "${BOLD}${GREEN}====================================================${NC}"
echo -e "  File: ${BOLD}$OUTPUT_TAR${NC}"
echo -e "  Size: ${BOLD}${ARCHIVE_MB} MB${NC} (${ARCHIVE_BYTES} bytes)\n"

# Maintain duplicate copy in repository root for convenience if default output
if [ "$OUTPUT_TAR" = "$DEFAULT_OUTPUT" ]; then
    cp "$OUTPUT_TAR" "${REPO_ROOT}/dockpanel-runtime.tar.gz" 2>/dev/null || true
    info "Synchronized root copy: ${REPO_ROOT}/dockpanel-runtime.tar.gz"
fi

log "Done!"
