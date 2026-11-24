# DockPanel Desktop Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build and package DockPanel Desktop—an installable, distributable local Windows workstation developer application with WSL2 engine parity, dual `.test` and `.local` domain support, trusted local `mkcert` HTTPS, and bidirectional live server synchronization.

**Architecture:** A native Tauri v2 desktop application wrapping DockPanel's React frontend with native Windows OS hooks ("Open in Explorer", "Open in VS Code"), communicating with an isolated WSL2 micro-instance (`dockpanel-desktop`) running Nginx, PHP-FPM, MariaDB, PostgreSQL, and DockPanel API/agent, backed by a zero-UAC Windows named pipe host helper (`dockpanel-host-helper`) and an offline Inno Setup installer wizard.

**Tech Stack:** Rust (Tauri v2, Axum, dockpanel-api, dockpanel-agent), TypeScript/React (Vite, Tailwind CSS), WSL2 (Ubuntu micro-distro), mkcert, Inno Setup 6 (Pascal Script).

**Spec:** `docs/superpowers/specs/2026-11-24-dockpanel-desktop-design.md`

## Global Constraints

- **Exact Linux Engine Parity:** Website files and runtime execution must happen inside WSL2 ext4 (`/var/www/<domain>`), never across Windows NTFS 9P mounts, ensuring full speed and parity with VPS hosting.
- **Offline Installer Delivery:** All-in-one Inno Setup `.exe` (~250–350 MB) containing Tauri binaries and the compressed WSL2 rootfs image (`dockpanel-runtime.tar.gz`), with zero network dependency during installation.
- **Zero-UAC Domain Changes:** Local domain additions and removals must update `C:\Windows\System32\drivers\etc\hosts` via the background `DockPanelHostHelper` Windows service over named pipe `\\.\pipe\dockpanel-host-helper` without triggering UAC elevation popups.
- **Bidirectional Sync Safety:** Pushing changes to live servers must enforce pre-flight remote snapshots and automatic domain search-and-replace (`.test`/`.local` ↔ live domain).
- **Frictionless Desktop UX:** Automatic loopback authentication bypasses login prompts on local desktop launch.

## Review Focus

1. **WSL2 UNC Path Compatibility on Windows Explorer/VS Code:** Ensure paths like `\\wsl.localhost\dockpanel-desktop\var\www\<domain>` open correctly in Windows File Explorer and VS Code even when WSL2 is freshly booted.
2. **Hosts File Concurrency & Locking:** Ensure named pipe requests to update `hosts` handle Windows Defender locks gracefully with retries and atomic writes without corrupting existing host entries.
3. **mkcert Certificate Expiry & Trust:** Ensure root CA registration in Windows Certificate Store is verified before issuing per-site certificates so browsers show a valid green lock without warnings.
4. **Database URL Search-Replace Integrity:** Ensure SQL dump search-and-replace handles serialized PHP objects (updating byte length strings like `s:21:"https://mysite.test"`) to prevent corrupted WordPress options on push/pull.
5. **Port Conflict Detection on Windows:** Ensure installer and runtime detect if Windows ports 80 or 443 are occupied (e.g. by IIS or Skype) and provide actionable remediation or alternative loopback port configuration.

---

### Task 1: Desktop Environment Mode & UI Pruning in Frontend

**Files:**
- Create: `panel/frontend/src/utils/desktop.ts`
- Modify: `panel/frontend/src/components/Sidebar.tsx`
- Modify: `panel/frontend/src/pages/Sites.tsx`
- Modify: `panel/frontend/src/pages/Settings.tsx`
- Test: `panel/frontend/src/__tests__/desktop.test.ts`

**Interfaces:**
- Produces: `isDesktopApp(): boolean`
- Produces: `getDesktopSitePath(domain: string): string`

- [ ] **Step 1: Write test for desktop mode detection and desktop-specific route filtering**

```typescript
import { isDesktopApp, getDesktopSitePath } from "../utils/desktop";

describe("Desktop Mode Utils", () => {
  it("detects desktop environment correctly", () => {
    (window as any).__TAURI__ = {};
    expect(isDesktopApp()).toBe(true);
    delete (window as any).__TAURI__;
    expect(isDesktopApp()).toBe(false);
  });

  it("constructs valid Windows WSL UNC path for local sites", () => {
    const path = getDesktopSitePath("mysite.test");
    expect(path).toBe("\\\\wsl.localhost\\dockpanel-desktop\\var\\www\\mysite.test");
  });
});
```

- [ ] **Step 2: Run test to verify it fails**

Run: `npx --prefix panel/frontend vitest run panel/frontend/src/__tests__/desktop.test.ts`
Expected: FAIL (modules not defined)

- [ ] **Step 3: Implement `isDesktopApp` and `getDesktopSitePath` in `panel/frontend/src/utils/desktop.ts`**

Export `isDesktopApp` checking `window.__TAURI__ || import.meta.env.VITE_APP_MODE === 'desktop'` and `getDesktopSitePath(domain)`.

- [ ] **Step 4: Prune server-only navigation items in `Sidebar.tsx` when `isDesktopApp() === true`**

Hide `DNS`, `Security/Firewall`, `Uptime Monitoring`, `Resellers`, and `Fleet Servers` when in desktop mode.

- [ ] **Step 5: Run tests and verify they pass**

Run: `npx --prefix panel/frontend vitest run panel/frontend/src/__tests__/desktop.test.ts`
Expected: PASS

- [ ] **Step 6: Commit**

```bash
git add panel/frontend/src/utils/desktop.ts panel/frontend/src/__tests__/desktop.test.ts panel/frontend/src/components/Sidebar.tsx
git commit -m "feat(desktop): add desktop detection and prune server-only navigation"
```

---

### Task 2: Native OS Hooks & Dual `.test`/`.local` Domain Switcher

**Files:**
- Create: `panel/frontend/src/components/DesktopSiteActions.tsx`
- Modify: `panel/frontend/src/pages/Sites.tsx`
- Test: `panel/frontend/src/__tests__/DesktopSiteActions.test.tsx`

**Interfaces:**
- Consumes: `isDesktopApp()`
- Produces: `<DesktopSiteActions domain={domain} />` with "Open in Explorer" and "Open in VS Code"

- [ ] **Step 1: Write tests for DesktopSiteActions component and domain validator**

Verify that clicking "Open in Explorer" calls the Tauri shell command or protocol handler, and that domain picker defaults to `.test` while allowing `.local`.

- [ ] **Step 2: Run test to verify it fails**

Run: `npx --prefix panel/frontend vitest run panel/frontend/src/__tests__/DesktopSiteActions.test.tsx`
Expected: FAIL

- [ ] **Step 3: Implement `DesktopSiteActions.tsx`**

Implement buttons:
- `Open in Explorer`: invokes Tauri command `open_folder` with WSL UNC path.
- `Open in VS Code`: invokes Tauri command `open_vscode` with WSL UNC path.
- In `Sites.tsx` site creation modal, add domain suffix selector (`.test` / `.local`) when `isDesktopApp()` is true.

- [ ] **Step 4: Run tests to verify they pass**

Run: `npx --prefix panel/frontend vitest run panel/frontend/src/__tests__/DesktopSiteActions.test.tsx`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add panel/frontend/src/components/DesktopSiteActions.tsx panel/frontend/src/pages/Sites.tsx
git commit -m "feat(desktop): add native OS hooks and dual domain selector"
```

---

### Task 3: Zero-UAC Windows Named Pipe Host Helper (`dockpanel-host-helper`)

**Files:**
- Create: `desktop/host-helper/Cargo.toml`
- Create: `desktop/host-helper/src/main.rs`
- Create: `desktop/host-helper/src/hosts_manager.rs`
- Test: `desktop/host-helper/tests/hosts_test.rs`

**Interfaces:**
- Produces: Windows service binary `dockpanel-host-helper.exe` listening on `\\.\pipe\dockpanel-host-helper`
- Protocol: JSON messages: `{"action": "add"|"remove", "domain": "example.test", "ip": "127.0.0.1"}`

- [ ] **Step 1: Write integration tests for hosts file manipulation**

```rust
#[test]
fn test_add_and_remove_host_entry() {
    let original = "127.0.0.1 localhost\n::1 localhost\n";
    let updated = hosts_manager::add_entry(original, "mysite.test", "127.0.0.1");
    assert!(updated.contains("127.0.0.1 mysite.test # dockpanel-desktop"));
    let removed = hosts_manager::remove_entry(&updated, "mysite.test");
    assert_eq!(removed.trim(), original.trim());
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --manifest-path desktop/host-helper/Cargo.toml`
Expected: FAIL

- [ ] **Step 3: Implement `hosts_manager.rs` and Windows Named Pipe Server in `main.rs`**

- Read/write `C:\Windows\System32\drivers\etc\hosts` atomically with comments `# dockpanel-desktop`.
- Implement Windows Service entrypoint (`windows-service` crate) and Named Pipe listener accepting local IPC from Tauri client.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test --manifest-path desktop/host-helper/Cargo.toml`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add desktop/host-helper/
git commit -m "feat(desktop): implement zero-UAC Windows named pipe hosts helper service"
```

---

### Task 4: Frictionless Desktop Authentication Bootstrap in `dockpanel-api`

**Files:**
- Modify: `panel/backend/src/routes/auth.rs`
- Modify: `panel/backend/src/main.rs`
- Test: `panel/backend/tests/desktop_bootstrap_test.rs`

**Interfaces:**
- Produces: `POST /api/auth/desktop-bootstrap` (only allowed from `127.0.0.1` when `DESKTOP_MODE=true`)
- Returns: `{ "token": "<jwt>", "user": { "role": "admin", "email": "local@dockpanel.desktop" } }`

- [ ] **Step 1: Write integration test for desktop bootstrap endpoint**

```rust
#[tokio::test]
async fn test_desktop_bootstrap_local_loopback() {
    // Verifies 127.0.0.1 loopback receives admin token without password when desktop mode enabled
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --manifest-path panel/backend/Cargo.toml --test desktop_bootstrap_test`
Expected: FAIL

- [ ] **Step 3: Implement `desktop_bootstrap` handler in `panel/backend/src/routes/auth.rs`**

Check client IP is loopback (`127.0.0.1`), verify environment flag `DESKTOP_MODE=true`, generate or fetch pre-seeded local admin JWT, and return session token.

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test --manifest-path panel/backend/Cargo.toml --test desktop_bootstrap_test`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add panel/backend/src/routes/auth.rs panel/backend/src/main.rs
git commit -m "feat(desktop): add loopback desktop bootstrap authentication endpoint"
```

---

### Task 5: Bidirectional Live Server Synchronization Engine (Push & Pull)

**Files:**
- Create: `panel/backend/src/services/live_sync.rs`
- Create: `panel/backend/src/routes/live_sync.rs`
- Create: `panel/frontend/src/components/LiveSyncModal.tsx`
- Test: `panel/backend/tests/live_sync_test.rs`

**Interfaces:**
- Produces: `POST /api/sites/:id/sync/pull` and `POST /api/sites/:id/sync/push`
- Request body: `{ "remote_url": string, "api_token": string, "sync_files": bool, "sync_db": bool, "dry_run": bool }`
- Produces: `<LiveSyncModal site={site} isOpen={isOpen} onClose={onClose} />`

- [ ] **Step 1: Write tests for database URL search-and-replace during sync**

```rust
#[test]
fn test_database_domain_rewrite_serialized() {
    let sql = r#"INSERT INTO wp_options VALUES (1, 'siteurl', 'https://project.test', 'yes');"#;
    let rewritten = live_sync::rewrite_domains(sql, "https://project.test", "https://example.com");
    assert!(rewritten.contains("https://example.com"));
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --manifest-path panel/backend/Cargo.toml --test live_sync_test`
Expected: FAIL

- [ ] **Step 3: Implement `live_sync.rs` service and routes**

- Pull: Download remote DB dump via API, rewrite domains, restore into local DB. Pull files via rsync/SSH or archive stream.
- Push: Trigger remote safety backup, stream local DB dump with rewritten domains to remote restore endpoint, push modified files.

- [ ] **Step 4: Implement `LiveSyncModal.tsx` in frontend**

Add "Push to Live" and "Pull from Live" dialog with checklist: Files, Database, Dry-Run preview, and real-time sync progress bar.

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test --manifest-path panel/backend/Cargo.toml --test live_sync_test`
Expected: PASS

- [ ] **Step 6: Commit**

```bash
git add panel/backend/src/services/live_sync.rs panel/backend/src/routes/live_sync.rs panel/frontend/src/components/LiveSyncModal.tsx
git commit -m "feat(desktop): implement bidirectional live server push/pull sync engine"
```

---

### Task 6: Tauri v2 Desktop Wrapper Project

**Files:**
- Create: `desktop/src-tauri/Cargo.toml`
- Create: `desktop/src-tauri/tauri.conf.json`
- Create: `desktop/src-tauri/src/main.rs`
- Create: `desktop/src-tauri/src/wsl_manager.rs`
- Create: `desktop/package.json`

**Interfaces:**
- Produces: `DockPanel Desktop.exe`
- Produces Tauri commands: `open_site_folder`, `open_vscode`, `check_wsl_status`, `restart_engine`

- [ ] **Step 1: Scaffold Tauri v2 desktop project in `desktop/`**

Initialize Tauri configuration pointing to built frontend `../../panel/frontend/dist`.

- [ ] **Step 2: Implement `wsl_manager.rs`**

Add status checks for WSL2 instance `wsl.exe -d dockpanel-desktop -u root -- systemctl is-active dockpanel-api`.

- [ ] **Step 3: Verify local Tauri build compiles**

Run: `cargo check --manifest-path desktop/src-tauri/Cargo.toml`
Expected: Exit code 0 (clean build)

- [ ] **Step 4: Commit**

```bash
git add desktop/
git commit -m "feat(desktop): scaffold Tauri v2 desktop application shell and WSL2 manager"
```

---

### Task 7: WSL2 Micro-Runtime Packaging Script

**Files:**
- Create: `scripts/build-desktop-runtime.sh`
- Create: `desktop/runtime/wsl.conf`
- Create: `desktop/runtime/setup-desktop-services.sh`

**Interfaces:**
- Produces: `dockpanel-runtime.tar.gz` (~250 MB compressed rootfs)
- Contains: Ubuntu base, Nginx, PHP 8.1–8.4 FPM, MariaDB, PostgreSQL, Mailpit, mkcert, and pre-compiled dockpanel-api & dockpanel-agent.

- [ ] **Step 1: Write `build-desktop-runtime.sh`**

Script runs on Linux builder or CI: exports minimal Ubuntu rootfs, installs packages, copies DockPanel binaries, and packages into `dockpanel-runtime.tar.gz`.

- [ ] **Step 2: Add `wsl.conf` with `[boot] systemd=true`**

Ensure systemd runs automatically inside the imported WSL2 instance.

- [ ] **Step 3: Test rootfs generation script syntax and permissions**

Run: `bash -n scripts/build-desktop-runtime.sh`
Expected: Syntax OK

- [ ] **Step 4: Commit**

```bash
git add scripts/build-desktop-runtime.sh desktop/runtime/
git commit -m "feat(desktop): add WSL2 micro-distro packaging scripts"
```

---

### Task 8: Inno Setup Standalone Offline Installer Wizard

**Files:**
- Create: `desktop/installer/setup.iss`
- Create: `desktop/installer/wsl_prereq.pas`
- Create: `desktop/installer/assets/banner.bmp`
- Create: `desktop/installer/assets/app.ico`

**Interfaces:**
- Produces: `DockPanel-Desktop-Setup-v2.248.5.exe` (Standalone Offline Installer ~280 MB)

- [ ] **Step 1: Implement `setup.iss` Inno Setup script**

- Embeds `DockPanel Desktop.exe`, `dockpanel-runtime.tar.gz`, `dockpanel-host-helper.exe`, and `mkcert.exe`.
- Checks for WSL2 presence (`wsl.exe --status`). If missing, invokes `wsl.exe --install --no-distribution`.
- Imports WSL2 distro: `wsl.exe --import dockpanel-desktop "{app}\runtime" "{app}\dockpanel-runtime.tar.gz" --version 2`.
- Installs and starts `DockPanelHostHelper` Windows service.
- Executes `mkcert -install` to trust the root CA.

- [ ] **Step 2: Verify installer script compilation**

Test compile `setup.iss` with ISCC (Inno Setup Command line Compiler).
Expected: Clean compilation producing installer `.exe`.

- [ ] **Step 3: Commit**

```bash
git add desktop/installer/
git commit -m "feat(desktop): add Inno Setup offline installer wizard script"
```

---

### Task 9: End-to-End Verification and Installer Dry Run

- [ ] **Step 1: Run end-to-end unit and integration tests across frontend, backend, and desktop**
- [ ] **Step 2: Test sample site creation with `.test` and `.local` domain suffixes**
- [ ] **Step 3: Verify bidirectional sync mock (Push & Pull) with remote test site**
- [ ] **Step 4: Final commit and documentation update**
