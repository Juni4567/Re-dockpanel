# Re-dockpanel Feature Roadmap & Batch Implementation Log

This document tracks the multi-batch implementation plan derived from the full codebase audit on **2026-11-27**.
Any engineer or AI agent picking up this repo should **read this file first**, check the **Current Status** and **Next Pickup Instructions**, verify tests, and update this log after completing each batch.

---

## Baseline Verified State (Start of Roadmap)
- **Branch:** `feat/dockpanel-desktop` (latest tag/release: `desktop-v2.253.0` on `Juni4567/Re-dockpanel`)
- **Baseline Tests:**
  - `cargo test --manifest-path desktop/dockpanel-app/Cargo.toml` -> **65 passed, 0 failed**
  - `npm test` in `panel/frontend` -> **7 test files, 37 passed, 0 failed**

---

## Phased Implementation Plan

### Batch 1 — Desktop PHP Engine, Resource Limits, & Extension Management
- **Goal:** Enforce stored per-site PHP limits (`php_memory_mb`, `max_upload_mb`, `php_max_workers`), replace single-worker blocking `php -S` with a multi-worker round-robin pool, inject `.env` vars into PHP/Node workers, enable real `php.ini` / `ext/*.dll` extension toggling, and ungate Resource Limits + PHP Extensions in `SiteDetail.tsx` for Desktop.
- **Status:** `DONE` (2026-11-27)
- **Target Files:**
  - `desktop/dockpanel-app/src/state.rs`
  - `desktop/dockpanel-app/src/main.rs`
  - `panel/frontend/src/pages/SiteDetail.tsx`

### Batch 2 — Desktop Router Upgrade: WebSockets, Streaming, Domain Aliases, & Redirects
- **Goal:** Upgrade `proxy_to_worker()` in `desktop/dockpanel-app/src/router.rs` to support WebSocket upgrades (`101 Switching Protocols` + bidirectional tunneling for Vite/Next.js HMR, Socket.IO, Laravel Reverb) and response streaming (removing full-body buffering & 30s timeout limits for SSE/large streams). Implement real `aliases` (`www.` / custom domains synced to hosts + `mkcert` SANs) and `redirects` (HTTP→HTTPS + custom 301/302 rules) on `/sites/{id}/aliases` and `/sites/{id}/redirects`.
- **Status:** `DONE` (2026-11-27)
- **Target Files:**
  - `desktop/dockpanel-app/src/router.rs`
  - `desktop/dockpanel-app/src/ssl.rs`
  - `desktop/dockpanel-app/src/state.rs`
  - `desktop/dockpanel-app/src/main.rs`

### Batch 3 — Desktop WordPress Parity & Live Windows Telemetry
- **Goal:** Replace Desktop stubs for `/sites/{id}/wordpress/update/{target}`, `/update-safe`, `/vuln-scan`, and `/security-check` with real WP-CLI / checksum / security checks. Replace hardcoded `/metrics`, `/system/processes`, `/system/network`, `/dashboard/metrics-history`, `cpu_usage`, and `/sites/{id}/stats` + `/bandwidth-usage` with real Windows host metrics and per-site router traffic counters.
- **Status:** `DONE` (2026-11-27)
- **Target Files:**
  - `desktop/dockpanel-app/src/wordpress.rs`
  - `desktop/dockpanel-app/src/state.rs`
  - `desktop/dockpanel-app/src/router.rs`
  - `desktop/dockpanel-app/src/main.rs`
  - `panel/frontend/src/pages/Dashboard.tsx`

### Batch 4 — Desktop System Tray, Background Mode, & In-App Auto-Updater
- **Goal:** Add Windows system tray support (minimize-to-tray keeping sites, MariaDB, and Mail Catcher alive; quick site toggles; Quit action; optional launch at Windows startup). Wire `/telemetry/update-status` to check GitHub Releases (`Juni4567/Re-dockpanel` `desktop-v*`), compare semver, verify SHA-256, and trigger installer download/launch.
- **Status:** `DONE` (2026-11-27)
- **Target Files:**
  - `desktop/dockpanel-app/src/tray.rs`
  - `desktop/dockpanel-app/src/updater.rs`
  - `desktop/dockpanel-app/src/state.rs`
  - `desktop/dockpanel-app/src/main.rs`
  - `panel/frontend/src/pages/Dashboard.tsx`
  - `panel/frontend/src/pages/Settings.tsx`

### Batch 5 — Desktop Local DX: Integrated Site Shell, Composer, Local Crons, & Local Staging
- **Goal:** Auto-download/wire `composer.phar`; add Laravel/Symfony presets; implement local task scheduler (`/sites/{id}/crons` & `/sites/{id}/backup-schedule`); implement local staging clones (`/sites/{id}/staging` with `staging.<domain>` clone + sync); add interactive site terminal / ConPTY shell pre-seeded with site PHP/Node/Composer/WP-CLI PATH; optional `cloudflared` quick preview tunnels.
- **Status:** `PENDING`

### Batch 6 — Desktop Python Runtime & Local Redis Daemon
- **Goal:** Add `python` runtime detection (`python`, `venv`, `uv`, `uvicorn`/`gunicorn`/`manage.py runserver`) and optional portable Redis daemon management for local queues/object cache.
- **Status:** `PENDING`

### Batch 7 — Cloud Platform Completeness (`panel/`)
- **Goal:** Wire `routes/teams.rs` RBAC into site/server authorization + build Teams UI; implement Wake-on-Request for auto-slept Docker containers; implement real DB PITR (WAL/binlog archiving + restore); expand `panel/mcp` with scoped write tools; add Desktop ↔ Cloud fleet switching & 1-click remote deploy.
- **Status:** `PENDING`

---

## Detailed Execution Log

### [2026-11-27] Batch 1 — Completed
- **Multi-Worker PHP Pool & Resource Limit Enforcement (`desktop/dockpanel-app/src/state.rs`, `main.rs`):**
  - Updated `AppState::worker_ports` (`HashMap<String, Vec<u16>>`) and `worker_processes` (`HashMap<String, Vec<Child>>`) with atomic round-robin selection (`worker_rr_cursor`), `stop_domain_workers()`, and `restart_sites_for_php_version()`.
  - Updated `spawn_php_worker()` to spawn `1..=8` `php -S` workers per site (based on `php_max_workers`) and pass `-d memory_limit=<php_memory_mb>M`, `-d upload_max_filesize=<max_upload_mb>M`, `-d post_max_size=<max_upload_mb + 2>M`, and `-d max_execution_time=120`.
  - Fixed `update_site_php_version()` and `update_site_limits()` to clamp limits (`max_upload_mb` `1..=10240`, `php_memory_mb` `32..=4096`, `php_max_workers` `1..=8`) and restart active PHP workers so changes apply immediately.
  - Added `AppState::load_site_env_vars()` and injected `<site_dir>/.env` variables into both `spawn_php_worker()` and `spawn_node_worker()`, restarting active workers in `site_env_put_handler()`.
- **PHP Extension Management (`desktop/dockpanel-app/src/main.rs`, `panel/frontend/src/pages/SiteDetail.tsx`):**
  - Enhanced `GET /api/php/extensions/{version}` to scan `<php_dir>/ext/php_*.dll` for real available Windows DLL extensions.
  - Added `POST /api/php/extensions/install` and `POST /api/php/extensions/disable` (`set_php_extension_enabled()` + `toggle_php_extension_in_ini_text()`) supporting both `extension=` and `zend_extension=` (`opcache`/`xdebug`), protecting PHP core built-ins, and restarting active sites on that PHP version.
  - Ungated **Resource Limits** (showing Desktop-relevant fields: Max Upload, PHP Memory, PHP Workers `1..=8`) and **PHP Extensions** (enable/disable) in `SiteDetail.tsx` for Desktop mode.

### [2026-11-27] Batch 2 — Completed
- **WebSocket Upgrades & Streaming HTTP Proxy (`desktop/dockpanel-app/src/router.rs`):**
  - Added `is_websocket_upgrade()` and `proxy_websocket_upgrade()` to tunnel `Upgrade: websocket` (`101 Switching Protocols`) connections bidirectionally via `hyper::upgrade::on` + `tokio::io::copy_bidirectional`.
  - Converted `proxy_to_worker()` to stream request and response bodies (`reqwest::Body::wrap_stream(body.into_data_stream())` and `axum::body::Body::from_stream(resp.bytes_stream())`) and removed the 30s total request timeout so SSE and large streams work uninterrupted.
- **Domain Aliases, SSL SANs, Redirect Rules, & Password-Protect API (`desktop/dockpanel-app/src/ssl.rs`, `state.rs`, `router.rs`, `main.rs`):**
  - Added `SiteRedirect` and `aliases`/`redirects` fields to `LocalSite`, `AppState::resolve_site_by_host()`, `add_site_alias()`, `remove_site_alias()`, `add_site_redirect()`, and `remove_site_redirect()`.
  - Updated `ssl::ensure_site_certificate_with_aliases()` and `AppState::refresh_site_ssl_with_aliases()` so adding/removing aliases updates Windows `hosts`, regenerates `mkcert` SAN certificates, and registers all alias keys in `DynamicCertResolver`.
  - Added `match_site_redirect()` in `router.rs` supporting exact path and wildcard (`/*`) 301/302 redirect rules before serving/proxying.
  - Replaced stubs in `main.rs` with real handlers for `/sites/{id}/redirects`, `/sites/{id}/redirects/remove`, `/sites/{id}/aliases`, `/sites/{id}/aliases/remove`, `/sites/{id}/password-protect`, and `/sites/{id}/password-protect/remove`.
  - *Note:* An explicit per-site `force_https` (HTTP→HTTPS auto-redirect) toggle can be added to `LocalSite` + UI in a follow-up if desired.

### [2026-11-27] Batch 3 — Completed
- **Desktop WordPress Parity (`desktop/dockpanel-app/src/wordpress.rs`, `main.rs`):**
  - Replaced canned stubs with real `scan_wp_vulnerabilities()` (14-plugin CVE database + outdated check + cached counts in `.dockpanel_wp.json`), `check_wp_security()` (7 checks: `disable-file-editing`, `debug-disabled`, `db-prefix`, `block-php-uploads`, `force-ssl`, `auto-updates`, `wp-config-perms`), `apply_wp_hardening()` (returning `HardenResult[]` + persisting `hardening_history`), `get_hardening_history()`, `update_wp_target()` (`core`/`plugins`/`themes`/`all`), `smart_update_wp()` (ZIP snapshot + rollback on failure), and `/wordpress/bulk-update`.
  - Updated `wordpress_sites_handler` to surface `wp_version`, `vulns`, `critical_vulns`, `vuln_count`, and `update_available`.
- **Live Windows Telemetry & Per-Site Traffic Accounting (`desktop/dockpanel-app/src/main.rs`, `state.rs`, `router.rs`, `panel/frontend/src/pages/Dashboard.tsx`):**
  - Upgraded `win_metrics` with Win32 `GetSystemTimes` host CPU %, `CreateToolhelp32Snapshot` + `K32GetProcessMemoryInfo` top running processes, and `GetIfTable` (`iphlpapi`) network adapter byte counters.
  - Added 10s background metrics sampler pushing `{ cpu, mem, disk, time }` into `AppState::metrics_history` (capped at 120 points) for `/dashboard/metrics-history` and wired `/metrics`, `/system/info`, `/system/processes`, `/system/network`, `/dashboard/intelligence`, `/dashboard/fleet`, `/dashboard/docker`, and dynamic `env!("CARGO_PKG_VERSION")` across version fields.
  - Added `SiteTrafficStats` + `record_site_traffic()` in `router.rs` + `Content-Length` headers on static files, wiring `/sites/{id}/stats` (`requests`, `unique_ips`, `bandwidth_mb`, `top_pages`, `status_codes`) and `/sites/{id}/bandwidth-usage`.
  - Ungated `/system/processes` and `/system/network` fetches in `Dashboard.tsx` for Desktop mode.

### [2026-11-27] Batch 4 — Completed
- **Windows System Tray, Background Mode, & Startup Toggle (`desktop/dockpanel-app/src/tray.rs`, `state.rs`, `main.rs`, `panel/frontend/src/pages/Settings.tsx`):**
  - Added pure-Win32 `Shell_NotifyIconW` tray thread (`tray::start_tray`) with native popup menu (`Show DockPanel Desktop`, `Open Sites Folder`, `Quit DockPanel Desktop`) and left-click restore via `EventLoopBuilder::<TrayAction>::with_user_event()`.
  - Updated `WindowEvent::CloseRequested` to hide the WebView2 window and keep PHP/Node workers, MariaDB, and Mail Catcher alive when `minimize_to_tray != "false"`, stopping all workers only on explicit Quit.
  - Added `HKCU\Software\Microsoft\Windows\CurrentVersion\Run` integration (`is_launch_at_startup_enabled` / `set_launch_at_startup`) wired through `/api/settings` (`launch_at_startup`) and `Settings.tsx` (**Background Mode & App Updates** card).
- **In-App Auto-Updater (`desktop/dockpanel-app/src/updater.rs`, `main.rs`, `panel/frontend/src/pages/Dashboard.tsx`, `Settings.tsx`):**
  - Implemented `check_for_desktop_update()` (15-minute cached GitHub Releases feed lookup with `?force=1`, semver comparison, `DockPanel-Desktop-Setup.exe` asset discovery, and SHA-256 parsing from release notes) on `GET /api/telemetry/update-status`.
  - Implemented `download_and_launch_update()` on `POST /api/desktop/update/install` with SHA-256 verification before launching the installer.
  - Enabled the Desktop update banner in `Dashboard.tsx` and manual **Check for Updates** / **Download & Install** controls in `Settings.tsx`.

### Verification Snapshot (After Batch 4)
- Batch 3 release binary (`dockpanel.exe`) built and live smoke-tested (`--server-only`): verified `version=2.253.0`, live CPU (`34.4%`), 20 processes, 6 network interfaces, WP security score (`14 -> 57`), vuln scan (`1`), and router request/bandwidth tracking (`requests=1`, `bw_bytes=1100`).
- `cargo test --manifest-path desktop/dockpanel-app/Cargo.toml`: **79 passed, 0 failed** (includes new `tray` and `updater` semver/SHA-256 unit tests)
- `npm test` in `panel/frontend`: **7 test files, 37 passed, 0 failed**
- `npm run build` in `panel/frontend`: **passed** (`tsc -b && vite build`)

---

- **Active Batch:** **Batches 1–4 + Dynamic Real-Time Alerts Shipped in `v2.254.0` — Next Up: Batch 5**

### [2026-11-27] Real-Time Dynamic Alerts & Production Release `2.254.0` — Completed
- **Frontend Alert HUD & SSE Integration (`NotificationContext.tsx`, `useLayoutState.ts`, `api.ts`, `index.css`):**
  - Replaced single top banner with floating real-time alert stack (`MAX_ACTIVE_ALERTS = 4`), `alertSlideIn` animation, live relative age ticker, hover-pausing countdown bar, error auto-pin, copyable structured diagnostic dumps, and `Dismiss All`.
  - Connected `/api/notifications/stream` SSE events into live `NotificationCard` toasts with smart 3.5s target/message enrichment (`confirmedByServer`).
  - Added `method`, `endpoint`, `durationMs`, and `timestamp` to `ApiError` and `buildAlertDetails()` helper; enriched `DesktopSiteActions`, `SiteDetail`, `Sites`, and `Dashboard` notifications; added `NotificationContext.test.tsx`.
- **Backend SSE Payload & Build Warning Cleanup (`state.rs`, `tray.rs`):**
  - Enriched `record_event()` `notif_tx` JSON with `action`, `target_type`, `target_name`, `category`, and formatted `details`.
  - Cleaned unused `TrayHandle` re-export warning in `tray.rs`.

## Next Pickup Instructions (For Next Agent)
1. Read this file (`docs/ROADMAP_IMPLEMENTATION_LOG.md`) and run `git status -s` to review uncommitted Batch 1–4 changes (consider committing or building & publishing a new release when ready).
2. Pick up **Batch 5 — Desktop Local DX: Integrated Site Shell, Composer, Local Crons, & Local Staging**:
   - Auto-download/wire `composer.phar` alongside PHP and add Laravel/Symfony presets.
   - Implement local task scheduler (`/sites/{id}/crons` & `/sites/{id}/backup-schedule`).
   - Implement local staging clones (`/sites/{id}/staging` with `staging.<domain>` clone + sync) and interactive site terminal / ConPTY shell pre-seeded with site PHP/Node/Composer/WP-CLI PATH, plus optional `cloudflared` quick preview tunnels.
3. After completing each batch:
   - Run `npm test` and `npm run build` in `panel/frontend`
   - Run `cargo test --manifest-path desktop/dockpanel-app/Cargo.toml`
   - Update this log (`Status`, `Detailed Execution Log`, `Verification Snapshot`, and `Next Pickup Instructions`).
