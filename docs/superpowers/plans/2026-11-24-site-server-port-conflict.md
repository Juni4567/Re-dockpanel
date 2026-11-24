# Site Server, Start/Stop Controls & Port Conflict Resolution Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build a native in-process site router with static and PHP reverse-proxying, site start/stop lifecycle controls, automated Windows hosts registration, and interactive port conflict detection and resolution for DockPanel Desktop.

**Architecture:** An embedded Axum HTTP router running inside `dockpanel.exe` listens on the configured router port (default 80, auto-fallback to 8080 if 80 is occupied by WampServer/Apache). It resolves incoming `Host` headers to local sites in `%USERPROFILE%\DockPanel\sites\<domain>\`, directly serves static assets, proxies `.php` requests to a dedicated background `php.exe -S` worker, and renders a holding page for stopped sites. The frontend provides Start/Stop controls and a Port Conflict modal to toggle ports or view conflicting process details.

**Tech Stack:** Rust (Tokio, Axum, reqwest, tower-http, winapi/windows-sys), React, TypeScript, Tailwind CSS.

**Spec:** [docs/superpowers/specs/2026-11-24-site-server-port-conflict-design.md](file:///c:/Users/junai/Desktop/Re-dockpanel/docs/superpowers/specs/2026-11-24-site-server-port-conflict-design.md)

## Global Constraints
- Pure Rust architecture: Zero Tauri or WSL2 dependencies; standalone `dockpanel.exe`.
- Path conventions: Document root is `%USERPROFILE%\DockPanel\sites\<domain>\` using native Windows paths.
- Default ports: Preferred port is 80; fallback port is 8080; HTTPS alternative is 8443.
- Non-destructive execution: Third-party processes (like WampServer `httpd.exe`) must never be terminated without explicit user confirmation.
- PHP CLI discovery: Detect PHP CLI at `C:\wamp64\bin\php\php*\php.exe` or `%PATH%`.

## Review Focus
1. Port 80 busy at launch: Router must auto-bind port 8080 without panicking or failing to start.
2. Direct HTTP access to stopped site: Router returns HTTP 200 with branded "Site is Stopped" page, not a dropped connection or 500 error.
3. PHP worker lifecycle: Terminating `dockpanel.exe` or clicking "Stop Site" cleanly kills the child `php.exe` process without leaving orphaned processes.
4. Non-standard port URL formatting: Links generated in `Sites.tsx` must correctly append `:8080` when `active_port` is 8080 and omit port when 80.
5. Hosts file write resilience: If writing `hosts` fails due to permissions, the application logs a warning and continues running without crashing.

---

### Task 1: Port Conflict Detection & Router Config (`router.rs` & `state.rs`)

**Files:**
- Create: `desktop/dockpanel-app/src/router.rs`
- Modify: `desktop/dockpanel-app/src/state.rs`
- Test: Unit tests in `desktop/dockpanel-app/src/router.rs`

**Interfaces:**
- Produces:
  - `pub fn check_port_status(port: u16) -> PortStatus`
  - `pub struct PortStatus { pub port: u16, pub in_use: bool, pub process_name: Option<String>, pub pid: Option<u32> }`
  - `pub struct RouterConfig { pub preferred_port: u16, pub active_port: u16 }`
  - `AppState::get_router_config(&self) -> RouterConfig`
  - `AppState::set_router_port(&self, port: u16)`

- [ ] **Step 1: Write unit tests for port conflict detection and router config**
  Add unit tests in `desktop/dockpanel-app/src/router.rs`:
  ```rust
  #[cfg(test)]
  mod tests {
      use super::*;
      #[test]
      fn test_port_checker_available_port() {
          // Binding to port 0 checks random free port
          let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
          let port = listener.local_addr().unwrap().port();
          drop(listener);
          let status = check_port_status(port);
          assert!(!status.in_use);
      }
      #[test]
      fn test_port_checker_occupied_port() {
          let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
          let port = listener.local_addr().unwrap().port();
          let status = check_port_status(port);
          assert!(status.in_use);
          drop(listener);
      }
  }
  ```

- [ ] **Step 2: Run test to verify it fails**
  Run: `cargo test --manifest-path desktop/dockpanel-app/Cargo.toml test_port_checker`
  Expected: FAIL (module or function missing)

- [ ] **Step 3: Implement `check_port_status` and `RouterConfig`**
  - Implement TCP bind probe in `router.rs`. On error `AddrInUse`, inspect process via `netstat -ano -p tcp` or Windows socket APIs to retrieve PID and process image name.
  - Add `RouterConfig` load/save in `state.rs` pointing to `%APPDATA%\DockPanel\router_config.json`.

- [ ] **Step 4: Run test to verify it passes**
  Run: `cargo test --manifest-path desktop/dockpanel-app/Cargo.toml test_port_checker`
  Expected: PASS

- [ ] **Step 5: Commit**
  ```bash
  git add desktop/dockpanel-app/src/router.rs desktop/dockpanel-app/src/state.rs
  git commit -m "feat(desktop): add port conflict detection and router configuration"
  ```

---

### Task 2: Multi-Site Router Engine & PHP Reverse Proxy (`router.rs`)

**Files:**
- Modify: `desktop/dockpanel-app/src/router.rs`
- Modify: `desktop/dockpanel-app/src/state.rs`

**Interfaces:**
- Consumes: `AppState`, `PortStatus`, `LocalSite`
- Produces:
  - `pub fn start_site_router(state: Arc<AppState>, port: u16) -> tokio::task::JoinHandle<()>`
  - `pub async fn handle_router_request(req: Request<Body>, state: Arc<AppState>) -> Response`

- [ ] **Step 1: Write test for site routing and stopped page response**
  Add unit test in `router.rs`:
  ```rust
  #[tokio::test]
  async fn test_stopped_site_returns_holding_page() {
      let state = Arc::new(AppState::new());
      let mut site = state.create_site("test-stopped.test", Some("8.3"));
      site.status = "stopped".to_string();
      // Test router handler produces 200 OK with "Site is Stopped"
  }
  ```

- [ ] **Step 2: Run test to verify it fails**
  Run: `cargo test --manifest-path desktop/dockpanel-app/Cargo.toml test_stopped_site`
  Expected: FAIL

- [ ] **Step 3: Implement Axum router and request handler**
  - Extract `Host` header, trim port (e.g. `mytestdomain.local:8080` -> `mytestdomain.local`).
  - Lookup site in `AppState`.
  - If stopped: render HTML containing "Site is Stopped - Click Start Site in DockPanel to resume".
  - If active: check URI path. If path ends with `.php` or is root `/` and `index.php` exists, forward request via `reqwest` to `http://127.0.0.1:<worker_port>`. If static file exists in document root, stream file with MIME type.
  - If unknown domain: return 404 "Domain not configured in DockPanel".

- [ ] **Step 4: Run test to verify it passes**
  Run: `cargo test --manifest-path desktop/dockpanel-app/Cargo.toml test_stopped_site`
  Expected: PASS

- [ ] **Step 5: Commit**
  ```bash
  git add desktop/dockpanel-app/src/router.rs
  git commit -m "feat(desktop): implement multi-site router and reverse proxy engine"
  ```

---

### Task 3: Site Lifecycle (Start/Stop) & Active Worker Reconciliation (`state.rs` & `main.rs`)

**Files:**
- Modify: `desktop/dockpanel-app/src/state.rs`
- Modify: `desktop/dockpanel-app/src/main.rs`

**Interfaces:**
- Produces:
  - `AppState::start_site(&self, id_or_domain: &str) -> Result<LocalSite, String>`
  - `AppState::stop_site(&self, id_or_domain: &str) -> Result<LocalSite, String>`
  - `AppState::reconcile_active_sites(&self)`
  - API Routes:
    - `POST /api/sites/{id}/start`
    - `POST /api/sites/{id}/stop`
    - `GET /api/system/ports/conflict`
    - `POST /api/system/ports/resolve`

- [ ] **Step 1: Write integration tests for start/stop API routes**
  In `state.rs`:
  ```rust
  #[test]
  fn test_site_start_stop_lifecycle() {
      let state = AppState::new();
      let site = state.create_site("lifecycle-test.local", Some("8.3"));
      assert_eq!(site.status, "active");
      let stopped = state.stop_site(&site.id).unwrap();
      assert_eq!(stopped.status, "stopped");
      let started = state.start_site(&site.id).unwrap();
      assert_eq!(started.status, "active");
  }
  ```

- [ ] **Step 2: Run test to verify it fails**
  Run: `cargo test --manifest-path desktop/dockpanel-app/Cargo.toml test_site_start_stop`
  Expected: FAIL

- [ ] **Step 3: Implement PHP worker manager and API routes**
  - Implement PHP executable finder scanning `C:\wamp64\bin\php\php*\php.exe` and `PATH`.
  - Implement `start_site`: spawns `php.exe -S 127.0.0.1:<port> -t <doc_root>`, stores child handle in a thread-safe worker map, sets status to `"active"`, saves `sites.json`.
  - Implement `stop_site`: kills child process, removes from worker map, sets status to `"stopped"`, saves `sites.json`.
  - Wire Axum routes in `main.rs`:
    - `/api/sites/{id}/start` -> `start_site`
    - `/api/sites/{id}/stop` -> `stop_site`
    - `/api/system/ports/conflict` -> returns port 80 status and alternatives
    - `/api/system/ports/resolve` -> switches active router port
  - Call `state.reconcile_active_sites()` in `run_app()` on startup.

- [ ] **Step 4: Run test to verify it passes**
  Run: `cargo test --manifest-path desktop/dockpanel-app/Cargo.toml test_site_start_stop`
  Expected: PASS

- [ ] **Step 5: Commit**
  ```bash
  git add desktop/dockpanel-app/src/state.rs desktop/dockpanel-app/src/main.rs
  git commit -m "feat(desktop): add site start/stop lifecycle and worker reconciliation"
  ```

---

### Task 4: Windows Hosts File Synchronization (`hosts_manager.rs`)

**Files:**
- Modify: `desktop/dockpanel-app/src/hosts_manager.rs`
- Modify: `desktop/dockpanel-app/src/state.rs`

**Interfaces:**
- Produces:
  - `pub fn sync_domain_to_hosts(domain: &str) -> Result<(), String>`
  - `pub fn remove_domain_from_hosts(domain: &str) -> Result<(), String>`

- [ ] **Step 1: Write unit tests for hosts file manipulation**
  Test adding and removing `# dockpanel-desktop` tagged entries in `hosts_manager.rs`.

- [ ] **Step 2: Run test to verify it passes**
  Run: `cargo test --manifest-path desktop/dockpanel-app/Cargo.toml hosts_manager`
  Expected: PASS

- [ ] **Step 3: Implement direct sync fallback**
  If `dockpanel-host-helper` pipe is unavailable or returns Access Denied, attempt direct write if running with permissions or use PowerShell elevated runner.

- [ ] **Step 4: Commit**
  ```bash
  git add desktop/dockpanel-app/src/hosts_manager.rs desktop/dockpanel-app/src/state.rs
  git commit -m "feat(desktop): enhance hosts file synchronization with fallback"
  ```

---

### Task 5: Frontend Start/Stop Site Actions & Status Badges (`DesktopSiteActions.tsx`)

**Files:**
- Modify: `panel/frontend/src/components/DesktopSiteActions.tsx`
- Modify: `panel/frontend/src/pages/Sites.tsx`

**Interfaces:**
- Consumes:
  - `site.id`, `site.domain`, `site.status`
  - `POST /api/sites/{id}/start`
  - `POST /api/sites/{id}/stop`
- Produces:
  - Start / Stop button (▶️ Start Site / ⏹️ Stop Site)
  - Live status indicator badge (🟢 Running vs ⚪ Stopped)
  - Callback `onStatusChange?: (newStatus: string) => void`

- [ ] **Step 1: Update `DesktopSiteActionsProps` with status and callbacks**
  Add `siteId: string`, `status: string`, `onStatusChange?: (status: string) => void`.

- [ ] **Step 2: Add Start / Stop button and status handler**
  Implement `toggleSiteStatus`:
  Calls `/api/sites/${siteId}/start` or `/api/sites/${siteId}/stop` based on current status.
  Renders play icon (▶️) when stopped and stop icon (⏹️) when active.

- [ ] **Step 3: Build frontend and verify no TypeScript errors**
  Run: `npm run build --prefix panel/frontend`
  Expected: Successful Vite build.

- [ ] **Step 4: Commit**
  ```bash
  git add panel/frontend/src/components/DesktopSiteActions.tsx panel/frontend/src/pages/Sites.tsx
  git commit -m "feat(frontend): add Start/Stop controls and status badges to site actions"
  ```

---

### Task 6: Frontend Port Conflict Banner, Modal & Dynamic URLs (`PortConflictModal.tsx` & `Sites.tsx`)

**Files:**
- Create: `panel/frontend/src/components/PortConflictModal.tsx`
- Modify: `panel/frontend/src/pages/Sites.tsx`

**Interfaces:**
- Consumes:
  - `GET /api/system/ports/conflict`
  - `POST /api/system/ports/resolve`
- Produces:
  - `PortConflictModal` component
  - Port status indicator banner in `Sites.tsx`
  - Dynamic URL generator taking `active_port` into account (e.g. `http://<domain>:8080`)

- [ ] **Step 1: Create `PortConflictModal.tsx`**
  Renders dialog with:
  - Alert: "Port 80 is in use by Apache (WampServer)."
  - Option 1 (Selected): Use Port 8080 (`http://<domain>:8080`)
  - Option 2: Stop Apache (WampServer)
  - Save action calling `/api/system/ports/resolve`.

- [ ] **Step 2: Integrate `PortConflictModal` and port status pill in `Sites.tsx`**
  - Add query for port status on mount.
  - When port 80 is occupied, display warning pill:
    *"Port 80 in use by Apache. Serving on port 8080. [Change]"*.
  - Update all "Open in new tab" links (`<a>` tags) in `Sites.tsx`:
    `href={activePort === 80 ? `http://${site.domain}` : `http://${site.domain}:${activePort}`}`.

- [ ] **Step 3: Build frontend and verify clean compile**
  Run: `npm run build --prefix panel/frontend`
  Expected: Successful Vite build.

- [ ] **Step 4: Commit**
  ```bash
  git add panel/frontend/src/components/PortConflictModal.tsx panel/frontend/src/pages/Sites.tsx
  git commit -m "feat(frontend): add Port Conflict modal, banner, and dynamic router port URLs"
  ```

---

### Task 7: End-to-End Build, Integration & Verification

**Files:**
- Modify: Project root `dockpanel.exe`
- Modify: Installer binary `desktop/installer/bin/dockpanel.exe`

- [ ] **Step 1: Compile full release binary**
  Run: `cargo build --manifest-path desktop/dockpanel-app/Cargo.toml --release`
  Expected: Successful binary compilation with 0 errors.

- [ ] **Step 2: Copy binary to root and installer bin**
  Copy `desktop/dockpanel-app/target/release/dockpanel.exe` to `dockpanel.exe` and `desktop/installer/bin/dockpanel.exe`.

- [ ] **Step 3: Verification**
  - Test port conflict endpoint: `curl http://127.0.0.1:<panel_port>/api/system/ports/conflict`.
  - Test site router: `curl -H "Host: mytestdomain.local" http://127.0.0.1:8080/`. Verify response serves the site's `index.html`.
  - Test stopped site: Stop site via API, curl again, verify "Site is Stopped" page returned.
  - Test start site: Start site, verify PHP/static site is returned.
  - Launch `dockpanel.exe` and take visual verification screenshot of Sites page with Start/Stop controls and Port banner.

- [ ] **Step 4: Final commit**
  ```bash
  git add dockpanel.exe desktop/installer/bin/dockpanel.exe
  git commit -m "chore(release): update dockpanel desktop binary with site router and port conflict management"
  ```
