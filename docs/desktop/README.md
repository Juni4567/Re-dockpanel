# DockPanel Desktop

> **Local WordPress and PHP Development Environment with Exact Linux Parity**  
> Powered by Tauri v2, WSL2 Micro-distro, and Zero-UAC Windows Named Pipe Host Helper.

---

## 🌟 Overview

**DockPanel Desktop** brings the complete, production-grade Linux server hosting stack of DockPanel to your local Windows development workstation. Unlike traditional Windows local development tools that run simulated Windows binaries, slow Docker-desktop layers, or VMs that degrade disk I/O, DockPanel Desktop leverages an isolated **WSL2 Micro-distro** running native Linux services directly on an ext4 filesystem.

### Key Capabilities

- **Exact Linux Parity**: Runs the exact same Nginx, PHP (8.1 - 8.4 FPM), MariaDB, PostgreSQL, and Linux permissions (`www-data:www-data`, `0775`) as production VPS servers.
- **Zero-UAC Local Domains**: Automatic host entry resolution for `.test` and `.local` domains without annoying Windows User Account Control (UAC) elevation prompts.
- **Locally-Trusted Green-Padlock HTTPS**: Out-of-the-box wildcard SSL certificates signed by a local Root CA (`mkcert`) trusted by Windows and all browsers.
- **Blazing Fast Disk I/O**: Files reside inside ext4 storage and are exposed directly to Windows via Windows UNC paths (`\\wsl.localhost\dockpanel-desktop\var\www\...`), avoiding NTFS cross-filesystem translation slowdowns.
- **Native OS Integration**: One-click actions to "Open in Explorer" and "Open in VS Code" directly from the site dashboard.
- **Bidirectional Live Server Synchronization**: Push local sites to staging/production or pull remote live sites down to local in seconds, featuring serialized PHP database domain rewriting and automated remote safety snapshots.
- **100% Offline Installer**: Self-contained Inno Setup installer bundling the WSL micro-distro, binaries, and mkcert for friction-free setup without internet dependencies.

---

## 📐 System Architecture

DockPanel Desktop is built upon a decoupled three-tier architecture:

```
┌────────────────────────────────────────────────────────────────────────┐
│                        Windows Host Environment                        │
│                                                                        │
│  ┌──────────────────────┐              ┌────────────────────────────┐  │
│  │   Tauri v2 Desktop   │              │     DockPanel Host Helper  │  │
│  │    Frontend Shell    │              │      Windows Service       │  │
│  │  (WebView2 + React)  │              │  (\\.\pipe\dockpanel-...)  │  │
│  └──────────┬───────────┘              └─────────────┬──────────────┘  │
│             │                                        │                 │
│             │ HTTP Loopback (127.0.0.1:8080)         │ Named Pipe IPC  │
│             │ Explorer / VS Code Launch              │ Hosts Updates   │
│             ▼                                        ▼                 │
│  ┌──────────────────────────────────────────────────┐┌───────────────┐ │
│  │              WSL2 Micro-distro                   ││ Windows Hosts  │ │
│  │            (dockpanel-desktop)                   ││ File           │ │
│  │                                                  ││ C:\Windows\... │ │
│  │  - Nginx (80, 443)                               │└───────────────┘ │
│  │  - PHP-FPM (8.1, 8.2, 8.3, 8.4)                  │                  │
│  │  - MariaDB 10.11 / PostgreSQL 16                 │                  │
│  │  - DockPanel API (DESKTOP_MODE=true)             │                  │
│  │  - DockPanel Agent & Mailpit (1025/8025)         │                  │
│  │  - Native ext4: /var/www/<domain>                │                  │
│  └──────────────────────────────────────────────────┘                  │
└────────────────────────────────────────────────────────────────────────┘
```

### 1. Tauri v2 Desktop Shell
- Lightweight native desktop application wrapper built in Rust with Tauri v2.
- Houses the modern React frontend, styled with Tailwind CSS.
- Automatically handles desktop bootstrap authentication over loopback with session tokens and secure cookies.
- Exposes native OS integrations via Tauri commands:
  - `open_folder(path)`: Launches Windows File Explorer to the site's UNC directory.
  - `open_vscode(path)`: Launches VS Code directly into the WSL UNC path.
  - `check_wsl_status()`: Probes the health and running state of the `dockpanel-desktop` instance.
  - `restart_engine()`: Gracefully restarts the WSL background services.

### 2. WSL2 Micro-Distro (`dockpanel-desktop`)
- Minimal Ubuntu base image (~250 MB compressed) tailored specifically for DockPanel.
- Runs complete Linux server daemons managed via systemd:
  - **Nginx**: Handles reverse proxying, vhost routing, and SSL termination.
  - **PHP-FPM**: Multi-version support (8.1, 8.2, 8.3, 8.4) with pre-configured extensions for WordPress, Laravel, and standard PHP apps.
  - **MariaDB & PostgreSQL**: Pre-seeded with development superusers (`dockpanel` / `dockpanel`).
  - **DockPanel API & Agent**: Backend running with `DESKTOP_MODE=true` on `127.0.0.1:8080`, pruning server-only functions (like firewalls, DNS zone hosting, and multi-tenant reseller features) to optimize for single-user local workflows.
  - **Mailpit**: Built-in local SMTP catcher (port `1025`) and web UI (port `8025`) to catch outbound development emails.

### 3. Windows Named Pipe Host Helper (`dockpanel-host-helper`)
- Background Windows Service (`DockPanelHostHelper`) installed with `LocalSystem` privileges.
- Listens on a secure local named pipe: `\\.\pipe\dockpanel-host-helper`.
- Manages `C:\Windows\System32\drivers\etc\hosts` atomically and safely without requiring the desktop user to grant administrative elevation on every site addition or deletion.

---

## 🔒 Zero-UAC Local Domains (`.test` and `.local`)

Developers frequently struggle with editing the Windows `hosts` file because Windows protects it with administrative privileges and UAC elevation prompts. DockPanel Desktop solves this permanently:

1. **Service Registration**: During initial installation (which already runs elevated), `DockPanelHostHelper.exe` is registered as a background service.
2. **Atomic Updates**: When a new site (e.g., `myproject.test` or `store.local`) is created in DockPanel, a request is sent over the local named pipe to the host helper.
3. **Tagged Isolation**: The helper tags every entry with `# dockpanel-desktop`:
   ```text
   127.0.0.1 myproject.test # dockpanel-desktop
   127.0.0.1 store.local # dockpanel-desktop
   ```
4. **Safety & Protection**: Non-DockPanel entries are left untouched. When a site is deleted, only lines containing `# dockpanel-desktop` matching the target domain are removed. CRLF injection, null bytes, comments, and invalid IPs are strictly rejected.

---

## 🛡️ Locally-Trusted Green-Padlock HTTPS

DockPanel Desktop provides full HTTPS out of the box without scary browser warnings:

1. **Root CA Generation**: The installer uses the offline-bundled `mkcert.exe` utility to install a dedicated local development Certificate Authority into the Windows Certificate Store:
   ```cmd
   mkcert.exe -install
   ```
2. **Wildcard Certificates**: The micro-distro automatically generates wildcard certificates covering `*.test`, `test`, `*.local`, `local`, `localhost`, and `127.0.0.1`:
   ```bash
   mkcert -cert-file /etc/dockpanel/ssl/desktop-wildcard.crt \
          -key-file /etc/dockpanel/ssl/desktop-wildcard.key \
          "*.test" "test" "*.local" "local" "localhost" "127.0.0.1" "::1"
   ```
3. **Seamless Nginx Termination**: Nginx serves all local development sites over port 443 using this wildcard certificate, providing green-padlock HTTPS in Chrome, Edge, Firefox, and curl.

---

## ⚡ High-Speed ext4 Filesystem & Windows UNC Paths

Accessing Windows NTFS filesystems (`/mnt/c/...`) from inside WSL2 incurs substantial I/O overhead due to translation protocols (Plan 9 / 9P filesystem).

DockPanel Desktop completely flips this model for maximum speed:
- All site files reside natively on the **ext4 Linux filesystem** inside the WSL instance at `/var/www/<domain>`.
- The Windows host accesses this storage via high-speed **Windows UNC Paths**:
  ```text
  \\wsl.localhost\dockpanel-desktop\var\www\<domain>
  ```
- **Permission Hygiene**: The micro-distro enforces `www-data:www-data` ownership with `0775` permissions and `setgid` (`chmod g+s`), ensuring files created by Windows editors or PHP-FPM never suffer from permissions mismatches.

### Native UI Buttons
On each site card, the desktop dashboard provides instant native shortcuts:
- **Open in Explorer**: Opens Windows File Explorer directly to `\\wsl.localhost\dockpanel-desktop\var\www\<domain>`.
- **Open in VS Code**: Runs `code \\wsl.localhost\dockpanel-desktop\var\www\<domain>` to open the project immediately with remote editing and terminal support.

---

## 🔄 Bidirectional Live Server Synchronization

DockPanel Desktop includes a dedicated Live Sync engine for seamlessly transferring sites between your local machine and remote DockPanel VPS servers.

### Push vs. Pull
- **Push (Local → Remote)**: Publishes your local codebase, database, and media uploads to a staging or production server.
- **Pull (Remote → Local)**: Clones a live production site down to your local machine for debugging, plugin testing, or redesign work.

### Safe Serialized SQL Domain Rewriting
WordPress and modern PHP applications store URLs and serialized metadata in the database (e.g. `s:19:"https://mysite.test";`). Simple string find-and-replace corrupts PHP serialized strings because the byte length prefix no longer matches.

DockPanel Desktop's Rust backend contains a dedicated **Serialization-Safe Domain Rewrite Engine**:
- Parses SQL dumps and detects serialized strings (`s:<len>:"..."` and `s:<len>:\"...\"`).
- Performs single-pass replacement of domain URLs.
- Recalculates exact UTF-8 byte lengths and updates the `s:<len>` prefix.
- Rewrites plain SQL strings in table fields, options, posts, and postmeta without double-replacing overlapping domains.

### Remote Pre-Flight Safety Snapshots
Before any destructive operation occurs on a remote server during a **Push**, the Live Sync engine automatically performs:
1. **Pre-Flight Connectivity Check**: Verifies remote API tokens and permissions.
2. **Automated Remote Snapshot**: Creates a full backup snapshot of the remote site's database and files before applying changes. If an error occurs during transfer, the remote site can be instantly rolled back.

---

## 📦 Building the Offline Installer

DockPanel Desktop compiles into a standalone, offline Windows installer executable using **Inno Setup**.

### Prerequisites
- **Windows 10/11 x64** with WSL2 enabled.
- **Node.js 18+** & `npm`
- **Rust & Cargo** (`stable-x86_64-pc-windows-msvc`)
- **Inno Setup 6+** (`ISCC.exe`)

### Step-by-Step Build Instructions

#### 1. Build the Frontend
```bash
npm --prefix panel/frontend install
npm --prefix panel/frontend run build
```

#### 2. Build the WSL2 Micro-Runtime Archive
```bash
# In Linux or WSL
bash scripts/build-desktop-runtime.sh --output desktop/runtime/dockpanel-runtime.tar.gz
```

#### 3. Build the Windows Host Helper
```bash
cargo build --manifest-path desktop/host-helper/Cargo.toml --release
```
Produces `desktop/host-helper/target/release/dockpanel-host-helper.exe`.

#### 4. Build the Tauri Desktop Application
```bash
cargo build --manifest-path desktop/src-tauri/Cargo.toml --release
```
Produces `desktop/src-tauri/target/release/DockPanel Desktop.exe`.

#### 5. Verify Installer Components
Run the automated installer validation suite:
```powershell
powershell -ExecutionPolicy Bypass -File desktop/installer/tests/installer_test.ps1
```

#### 6. Compile Inno Setup Installer
```cmd
iscc.exe desktop/installer/setup.iss
```
This generates `DockPanel-Desktop-Setup-v2.248.5.exe` in `desktop/installer/`.

---

## 🧪 Verification & Testing Suite

To run all automated verification tests across the desktop ecosystem:

| Test Suite | Command | Purpose |
| :--- | :--- | :--- |
| **Frontend Tests** | `npx --prefix panel/frontend vitest run` | Unit tests for desktop site actions, domains, and modals |
| **Frontend Production Build** | `npm --prefix panel/frontend run build` | TypeScript compilation and Vite production bundling |
| **Backend Bootstrap & Sync** | `cargo test --manifest-path panel/backend/Cargo.toml --test desktop_bootstrap_test --test live_sync_test` | Tests for desktop mode loopback guards & SQL domain rewriter |
| **Host Helper Service** | `cargo test --manifest-path desktop/host-helper/Cargo.toml` | Host entries manager, named pipe IPC, validation & safety tests |
| **Tauri Desktop Shell** | `cargo test --manifest-path desktop/src-tauri/Cargo.toml` | WSL status detection, explorer launcher, mock API bootstrap |
| **Runtime Packaging Syntax** | `bash -n scripts/build-desktop-runtime.sh && bash -n desktop/runtime/setup-desktop-services.sh` | Bash script static syntax validation |
| **Installer Validation** | `powershell -ExecutionPolicy Bypass -File desktop/installer/tests/installer_test.ps1` | Inno Setup AST, file existence, and Pascal scripts check |
| **Honesty Pin Regression** | `bash tests/client-role-honesty-pin-e2e.sh` | End-to-end regression tests for RBAC honesty pins |

---

## 📄 License

DockPanel Desktop is distributed under the **GNU Affero General Public License v3.0 (AGPLv3)**.
Copyright © 2026 DockPanel Team.
