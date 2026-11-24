#!/usr/bin/env bash
#
# DockPanel Desktop - First-Boot Initialization Script
# Initializes local databases, permissions, wildcard SSL, and desktop services.
#
set -euo pipefail

# ── Color Output & Helpers ──────────────────────────────────────────────
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

# ── Safety Checks ─────────────────────────────────────────────────────────
if [ "$(id -u)" -ne 0 ]; then
    error "setup-desktop-services.sh must be run as root."
    exit 1
fi

FORCE=0
for arg in "$@"; do
    case "$arg" in
        --force|-f) FORCE=1 ;;
        --help|-h)
            echo "Usage: $0 [--force]"
            echo "Initializes DockPanel Desktop WSL2 micro-distro local services."
            exit 0
            ;;
    esac
done

INIT_FLAG="/etc/dockpanel/.desktop-initialized"
if [ -f "$INIT_FLAG" ] && [ "$FORCE" -eq 0 ]; then
    info "DockPanel Desktop services were already initialized ($INIT_FLAG exists)."
    info "Pass --force to re-run initialization."
    exit 0
fi

echo -e "\n${BOLD}${GREEN}==============================================${NC}"
echo -e "${BOLD}${GREEN}  DockPanel Desktop — Local Engine Initialization${NC}"
echo -e "${BOLD}${GREEN}==============================================${NC}\n"

# ── 1. Filesystem & Permissions Setup ─────────────────────────────────────
info "Setting up local directories and permissions..."

mkdir -p /var/www /var/www/html
mkdir -p /etc/dockpanel /etc/dockpanel/ssl /etc/dockpanel/vhosts
mkdir -p /var/run/dockpanel /var/lib/dockpanel /var/backups/dockpanel /run/dockpanel
mkdir -p /run/php /var/log/dockpanel /var/log/nginx

# Ensure www-data group exists
if ! getent group www-data >/dev/null 2>&1; then
    groupadd -r www-data || true
fi
if ! getent passwd www-data >/dev/null 2>&1; then
    useradd -r -g www-data -d /var/www -s /usr/sbin/nologin www-data || true
fi

# Set native ext4 permissions for /var/www (accessible via Windows UNC redirector)
chown -R www-data:www-data /var/www
chmod -R 775 /var/www
chmod g+s /var/www

# Default welcome landing page for catch-all desktop requests
if [ ! -f /var/www/html/index.html ]; then
    cat > /var/www/html/index.html << 'EOF'
<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <title>DockPanel Desktop Engine Active</title>
    <style>
        body { font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, sans-serif; background: #0f172a; color: #f8fafc; display: flex; align-items: center; justify-content: center; height: 100vh; margin: 0; }
        .card { background: #1e293b; padding: 2.5rem; border-radius: 12px; box-shadow: 0 10px 25px rgba(0,0,0,0.5); max-width: 540px; text-align: center; border: 1px solid #334155; }
        h1 { color: #38bdf8; font-size: 1.8rem; margin-bottom: 0.5rem; }
        p { color: #94a3b8; font-size: 1rem; line-height: 1.6; }
        .badge { display: inline-block; background: #0284c7; color: #fff; font-weight: 600; font-size: 0.85rem; padding: 0.35rem 0.8rem; border-radius: 9999px; margin-bottom: 1rem; }
        .footer { font-size: 0.85rem; color: #64748b; margin-top: 2rem; border-top: 1px solid #334155; padding-top: 1rem; }
    </style>
</head>
<body>
    <div class="card">
        <span class="badge">WSL2 Engine Running</span>
        <h1>DockPanel Desktop</h1>
        <p>The local development micro-runtime is active with exact Linux parity. Websites are hosted inside native ext4 storage with trusted SSL and instant live sync.</p>
        <p>Open the <strong>DockPanel Desktop</strong> app to spin up a WordPress or PHP site on a <code>.test</code> or <code>.local</code> domain.</p>
        <div class="footer">Local Ports: Nginx (80, 443) • API (8080) • Mailpit (1025, 8025)</div>
    </div>
</body>
</html>
EOF
    chown www-data:www-data /var/www/html/index.html
    chmod 644 /var/www/html/index.html
fi

log "Filesystem directories and /var/www permissions configured"

# ── 2. Local Database Services Setup ──────────────────────────────────────
# PostgreSQL initialization
info "Configuring PostgreSQL local instance..."
if command -v systemctl >/dev/null 2>&1 && [ -d /run/systemd/system ]; then
    systemctl start postgresql 2>/dev/null || true
else
    service postgresql start 2>/dev/null || /etc/init.d/postgresql start 2>/dev/null || true
fi

# Ensure postgres cluster is ready
sleep 1
if command -v psql >/dev/null 2>&1; then
    su - postgres -c "psql -tc \"SELECT 1 FROM pg_roles WHERE rolname='dockpanel'\"" 2>/dev/null | grep -q 1 || \
        su - postgres -c "psql -c \"CREATE USER dockpanel WITH PASSWORD 'dockpanel' SUPERUSER;\"" 2>/dev/null || true

    su - postgres -c "psql -tc \"SELECT 1 FROM pg_database WHERE datname='dockpanel'\"" 2>/dev/null | grep -q 1 || \
        su - postgres -c "psql -c \"CREATE DATABASE dockpanel OWNER dockpanel;\"" 2>/dev/null || true

    su - postgres -c "psql -c \"GRANT ALL PRIVILEGES ON DATABASE dockpanel TO dockpanel;\"" 2>/dev/null || true
    log "PostgreSQL 'dockpanel' database and superuser initialized"
else
    warn "psql CLI not found; skipping PostgreSQL user seeding"
fi

# MariaDB initialization
info "Configuring MariaDB local instance..."
if command -v systemctl >/dev/null 2>&1 && [ -d /run/systemd/system ]; then
    systemctl start mariadb 2>/dev/null || systemctl start mysql 2>/dev/null || true
else
    service mariadb start 2>/dev/null || service mysql start 2>/dev/null || /etc/init.d/mariadb start 2>/dev/null || true
fi

sleep 1
SQL_CLIENT=""
if command -v mariadb >/dev/null 2>&1; then
    SQL_CLIENT="mariadb"
elif command -v mysql >/dev/null 2>&1; then
    SQL_CLIENT="mysql"
fi

if [ -n "$SQL_CLIENT" ]; then
    $SQL_CLIENT -e "CREATE DATABASE IF NOT EXISTS dockpanel;" 2>/dev/null || true
    $SQL_CLIENT -e "CREATE USER IF NOT EXISTS 'dockpanel'@'localhost' IDENTIFIED BY 'dockpanel';" 2>/dev/null || true
    $SQL_CLIENT -e "CREATE USER IF NOT EXISTS 'dockpanel'@'127.0.0.1' IDENTIFIED BY 'dockpanel';" 2>/dev/null || true
    $SQL_CLIENT -e "CREATE USER IF NOT EXISTS 'root'@'127.0.0.1' IDENTIFIED BY 'root';" 2>/dev/null || true
    $SQL_CLIENT -e "GRANT ALL PRIVILEGES ON *.* TO 'dockpanel'@'localhost' WITH GRANT OPTION; GRANT ALL PRIVILEGES ON *.* TO 'dockpanel'@'127.0.0.1' WITH GRANT OPTION; GRANT ALL PRIVILEGES ON *.* TO 'root'@'127.0.0.1' WITH GRANT OPTION; FLUSH PRIVILEGES;" 2>/dev/null || true
    log "MariaDB 'dockpanel' database and development credentials initialized"
else
    warn "MariaDB/MySQL client CLI not found; skipping SQL seeding"
fi

# ── 3. mkcert Wildcard SSL Certificate Generation ────────────────────────
info "Generating local development wildcard SSL certificates..."

CERT_FILE="/etc/dockpanel/ssl/desktop-wildcard.crt"
KEY_FILE="/etc/dockpanel/ssl/desktop-wildcard.key"

if command -v mkcert >/dev/null 2>&1; then
    # Install local root CA into Linux certificate trust store
    mkcert -install 2>/dev/null || true
    # Generate wildcard certificate supporting both .test and .local TLDs
    mkcert -cert-file "$CERT_FILE" -key-file "$KEY_FILE" \
        "*.test" "test" "*.local" "local" "localhost" "127.0.0.1" "::1" 2>/dev/null || true
    log "mkcert wildcard SSL certificate generated for *.test and *.local"
else
    warn "mkcert not found in PATH; generating OpenSSL fallback certificate"
    openssl req -x509 -nodes -days 3650 -newkey rsa:2048 \
        -keyout "$KEY_FILE" \
        -out "$CERT_FILE" \
        -subj "/CN=*.test/O=DockPanel Desktop" \
        -addext "subjectAltName=DNS:*.test,DNS:test,DNS:*.local,DNS:local,DNS:localhost,IP:127.0.0.1,IP:::1" 2>/dev/null || true
    log "OpenSSL self-signed wildcard fallback certificate generated"
fi

chmod 644 "$CERT_FILE" 2>/dev/null || true
chmod 600 "$KEY_FILE" 2>/dev/null || true
chown -R www-data:www-data /etc/dockpanel/ssl 2>/dev/null || true

# ── 4. Nginx Desktop Vhost Configuration ──────────────────────────────────
info "Configuring Nginx default desktop vhost..."

# Remove conflicting default site if present
rm -f /etc/nginx/sites-enabled/default 2>/dev/null || true

NGINX_CONF_TARGET="/etc/nginx/sites-available/000-dockpanel-desktop.conf"
if [ ! -d "/etc/nginx/sites-available" ]; then
    NGINX_CONF_TARGET="/etc/nginx/conf.d/000-dockpanel-desktop.conf"
fi

cat > "$NGINX_CONF_TARGET" << 'EOF'
# DockPanel Desktop - Catch-all Default Vhost
# Serves fallback info page and provides green-padlock SSL termination for local sites

server {
    listen 80 default_server;
    listen [::]:80 default_server;
    listen 443 ssl default_server;
    listen [::]:443 ssl default_server;
    server_name _;

    ssl_certificate /etc/dockpanel/ssl/desktop-wildcard.crt;
    ssl_certificate_key /etc/dockpanel/ssl/desktop-wildcard.key;
    ssl_protocols TLSv1.2 TLSv1.3;
    ssl_ciphers HIGH:!aNULL:!MD5;

    root /var/www/html;
    index index.html index.php;

    location / {
        try_files $uri $uri/ =404;
    }

    # Proxy to local Mailpit Web UI if accessed via /_mailpit/
    location /_mailpit/ {
        proxy_pass http://127.0.0.1:8025/;
        proxy_http_version 1.1;
        proxy_set_header Upgrade $http_upgrade;
        proxy_set_header Connection "upgrade";
        proxy_set_header Host $host;
    }
}
EOF

if [ -d "/etc/nginx/sites-enabled" ]; then
    ln -sf "$NGINX_CONF_TARGET" /etc/nginx/sites-enabled/000-dockpanel-desktop.conf
fi

if command -v nginx >/dev/null 2>&1; then
    if nginx -t 2>/dev/null; then
        if command -v systemctl >/dev/null 2>&1 && [ -d /run/systemd/system ]; then
            systemctl reload nginx 2>/dev/null || systemctl restart nginx 2>/dev/null || true
        else
            service nginx reload 2>/dev/null || service nginx restart 2>/dev/null || true
        fi
        log "Nginx default desktop vhost installed and reloaded"
    else
        warn "Nginx configuration test failed; leaving config in place for inspection"
    fi
fi

# ── 5. PHP-FPM Pools Verification ─────────────────────────────────────────
info "Verifying PHP-FPM pools..."
for ver in 8.1 8.2 8.3 8.4; do
    if command -v "php-fpm$ver" >/dev/null 2>&1 || [ -f "/etc/php/$ver/fpm/php-fpm.conf" ]; then
        if command -v systemctl >/dev/null 2>&1 && [ -d /run/systemd/system ]; then
            systemctl enable "php$ver-fpm" 2>/dev/null || true
            systemctl start "php$ver-fpm" 2>/dev/null || true
        fi
        log "PHP-FPM $ver service checked"
    fi
done

# ── 6. DockPanel Environment & Service Configuration ─────────────────────
info "Setting up DockPanel API and Agent desktop environment..."

API_ENV="/etc/dockpanel/api.env"
AGENT_ENV="/etc/dockpanel/agent.env"

JWT_SECRET=$(openssl rand -hex 32 2>/dev/null || echo "dockpanel-desktop-secret-key-$(date +%s)")
AGENT_TOKEN=$(openssl rand -hex 24 2>/dev/null || echo "dockpanel-desktop-agent-token-$(date +%s)")

if [ -f "$API_ENV" ]; then
    # Keep existing JWT_SECRET and AGENT_TOKEN if file already present
    EXISTING_JWT=$(grep '^JWT_SECRET=' "$API_ENV" 2>/dev/null | cut -d= -f2- || true)
    EXISTING_TOKEN=$(grep '^AGENT_TOKEN=' "$API_ENV" 2>/dev/null | cut -d= -f2- || true)
    [ -n "$EXISTING_JWT" ] && JWT_SECRET="$EXISTING_JWT"
    [ -n "$EXISTING_TOKEN" ] && AGENT_TOKEN="$EXISTING_TOKEN"
fi

cat > "$API_ENV" << EOF
DESKTOP_MODE=true
LISTEN_ADDR=127.0.0.1:8080
DATABASE_URL=postgresql://dockpanel:dockpanel@127.0.0.1:5432/dockpanel
JWT_SECRET=${JWT_SECRET}
AGENT_SOCKET=/var/run/dockpanel/agent.sock
AGENT_TOKEN=${AGENT_TOKEN}
BASE_URL=http://127.0.0.1:8080
CORS_ORIGINS=http://localhost:8080,http://127.0.0.1:8080,http://tauri.localhost,tauri://localhost
EOF
chmod 600 "$API_ENV"

cat > "$AGENT_ENV" << EOF
AGENT_TOKEN=${AGENT_TOKEN}
AGENT_SOCKET=/var/run/dockpanel/agent.sock
DESKTOP_MODE=true
EOF
chmod 600 "$AGENT_ENV"

log "DockPanel desktop environment configured (DESKTOP_MODE=true, LISTEN_ADDR=127.0.0.1:8080)"

# If running under active systemd, reload and restart services
if command -v systemctl >/dev/null 2>&1 && [ -d /run/systemd/system ]; then
    systemctl daemon-reload 2>/dev/null || true
    systemctl enable dockpanel-agent.service 2>/dev/null || true
    systemctl restart dockpanel-agent.service 2>/dev/null || true
    systemctl enable dockpanel-api.service 2>/dev/null || true
    systemctl restart dockpanel-api.service 2>/dev/null || true
    log "dockpanel-agent and dockpanel-api services restarted under systemd"
fi

# Write completion flag
date -u +"%Y-%m-%dT%H:%M:%SZ" > "$INIT_FLAG"
chmod 644 "$INIT_FLAG"

echo -e "\n${BOLD}${GREEN}✓ DockPanel Desktop initialization complete!${NC}\n"
