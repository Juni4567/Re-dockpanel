# Design Document: DockPanel Desktop for Windows

**Date:** 2026-11-24  
**Author:** Antigravity Team  
**Status:** In Review (Pending Approval)  
**Target Delivery:** Distributable Windows Installer & Desktop Application  

---

## 1. Executive Summary & Intent

**DockPanel Desktop** is a specialized, distributable local Windows application that brings the full power of DockPanel's website management, WordPress Toolkit, and Database Manager to local development workstations. 

Developers can spin up local sites instantly on custom domains (e.g., `project.test` or `project.local`) with locally-trusted green-padlock HTTPS, test theme and plugin updates with zero risk, and **bidirectionally synchronize files and databases between the local workstation and live production servers** with a single click.

---

## 2. Goals, Non-Goals & Anti-Goals

### 2.1 Goals
- **1-Click Distributable Windows Installer:** An `.exe` installer wizard (Inno Setup) that configures the application, sets up desktop/start-menu shortcuts, and prepares the runtime.
- **Tauri Native GUI:** A fast, low-memory Windows application shell (<30 MB RAM) wrapping the React frontend using Windows WebView2.
- **100% Linux Engine Parity (WSL2 Micro-Distro):** Nginx, PHP-FPM, MariaDB/MySQL, and DockPanel's agent run inside a lightweight WSL2 micro-instance, guaranteeing exact behavioral parity with production Linux VPS hosting.
- **Blazing Local Filesystem Performance:** Site files live natively in WSL2 ext4 (`/var/www/<domain>`) for maximum PHP-FPM/WordPress I/O throughput, exposed seamlessly to Windows via native UNC paths (`\\wsl.localhost\dockpanel-desktop\var\www\<domain>`).
- **Dual Domain TLD Support:** Native support for both `.test` (RFC 6761 standard) and `.local` domain extensions with automated, zero-UAC Windows `hosts` file management via a background helper service.
- **Zero-Warning Local HTTPS:** Built-in `mkcert` integration that provisions locally-trusted CA certificates directly into Windows Certificate Store, Edge, Chrome, and Firefox.
- **Native OS Hooks:** Replaces web file managers with 1-click **"Open in File Explorer"** and **"Open in VS Code"**.
- **Bidirectional Live Server Sync:** Connect any local site to a live DockPanel remote server to **Push** (deploy to live) or **Pull** (import from live) files, media, and database tables with automated URL search-and-replace.
- **Local Mail Catcher:** Intercepts outgoing PHP `mail()` and displays messages in a local inbox via Mailpit.
- **Frictionless Local Authentication:** Zero login prompts on local launch; automatically bootstraps and stores a local loopback administrative session.

### 2.2 Non-Goals
- **Public Hosting on Residential Windows:** DockPanel Desktop is not an Internet-facing production server. It does not handle public DNS glue records, dynamic DNS, or Let's Encrypt public ACME challenges.
- **Multi-Tenant Reseller / Client Billing:** No subscription billing, Stripe webhooks, client role quotas, or reseller isolation. The local app operates as a single super-user workstation.
- **Production Server Hardening:** No UFW, Fail2ban, SSH brute-force protection, or 24/7 uptime monitor alerts.

### 2.3 Anti-Goals
- **Native Win32 Service Re-architecture:** We will *not* attempt to rewrite the agent to control Windows Services (`sc.exe`), Windows Named Pipes, or Windows IIS/Apache. That would sacrifice parity with live Linux VPS environments, break WP-CLI shell hooks, and create separate codebases.
- **Docker Desktop Dependency:** We will *not* require the user to install or license Docker Desktop.

---

## 3. Approaches Considered & Selected Architecture

| Approach | Architecture | Feasibility | Trade-Offs | Decision |
| :--- | :--- | :--- | :--- | :--- |
| **Path A: Tauri + WSL2 Micro-Instance** *(Selected)* | Tauri v2 Windows GUI + Minimal WSL2 Ubuntu background engine | **High** (Native performance, 100% Linux parity) | Requires WSL2 enabled on Windows 10/11 (automated by installer) | **SELECTED** by user: Fast, robust, exact VPS parity, clean uninstall. |
| **Path B: Native Win32 Port** | Win32 Rust Daemon + Windows Nginx + FastCGI TCP + Windows MariaDB | **Low** (Massive engineering overhead) | Huge ongoing maintenance, Windows ACL issues, broken WP-CLI scripts | **REJECTED**: High risk of behavioral divergence from live servers. |
| **Path C: Heavyweight VM (Hyper-V / VirtualBox)** | Full Linux OS in standard VM | **Medium** | Heavy disk/RAM consumption (4GB+ RAM), slow boot | **REJECTED**: Sluggish user experience compared to WSL2/Tauri. |

---

## 4. System Architecture & Builder Specifications

```
┌─────────────────────────────────────────────────────────────────────────────┐
│                           WINDOWS HOST SYSTEM                               │
│                                                                             │
│   ┌─────────────────────────────────────────────────────────────────────┐   │
│   │               DockPanel Desktop GUI (Tauri v2)                      │   │
│   │  • Site Grid & Creation Modal (.test / .local)                      │   │
│   │  • WordPress Toolkit, Database Manager, Table Browser               │   │
│   │  • Live Server Sync Dialog (Push / Pull, Diff Preview)              │   │
│   │  • Native OS Hooks: [Open in Explorer] [Open in VS Code]            │   │
│   └───────────────┬─────────────────────────────────┬───────────────────┘   │
│                   │ IPC (Tauri Rust Backend)        │                       │
│                   ▼                                 ▼                       │
│   ┌───────────────────────────────┐ ┌───────────────────────────────────┐   │
│   │  DockPanel Host Helper Service│ │  mkcert Local CA Helper           │   │
│   │  LocalSystem Windows Service  │ │  Installs Root CA into Windows    │   │
│   │  Named pipe: \\.\pipe\dockhost│ │  Cert Store & Browser Stores      │   │
│   │  Updates hosts (127.0.0.1)    │ │                                   │   │
│   └───────────────────────────────┘ └───────────────────────────────────┘   │
└───────────────────────────────────┬─────────────────────────────────────────┘
                                    │ localhost:80, 443, 3080, 8025
                                    ▼
┌─────────────────────────────────────────────────────────────────────────────┐
│                    WSL2 MICRO-INSTANCE (dockpanel-desktop)                  │
│                                                                             │
│   ┌─────────────────────────────────────────────────────────────────────┐   │
│   │  DockPanel API (Port 3080) & Agent (/run/dockpanel/agent.sock)      │   │
│   ├─────────────────────────────────────────────────────────────────────┤   │
│   │  Nginx (HTTP: 80, HTTPS: 443 with mkcert vhosts)                    │   │
│   ├─────────────────────────────────────────────────────────────────────┤   │
│   │  PHP-FPM (8.1, 8.2, 8.3, 8.4 via Unix domain sockets)               │   │
│   ├─────────────────────────────────────────────────────────────────────┤   │
│   │  Databases (MariaDB / MySQL 3306, PostgreSQL 5432)                  │   │
│   ├─────────────────────────────────────────────────────────────────────┤   │
│   │  Mailpit (Local SMTP on 1025, Web UI on 8025)                       │   │
│   ├─────────────────────────────────────────────────────────────────────┤   │
│   │  Native ext4 Storage: /var/www/<domain>                             │   │
│   │  Accessible on Windows via:                                         │   │
│   │  \\wsl.localhost\dockpanel-desktop\var\www\<domain>                 │   │
│   └─────────────────────────────────────────────────────────────────────┘   │
└─────────────────────────────────────────────────────────────────────────────┘
                                    │
                                    │ Encrypted Wire / API / SSH (Push/Pull)
                                    ▼
┌─────────────────────────────────────────────────────────────────────────────┐
│                         LIVE SERVER (REMOTE VPS)                            │
│  DockPanel API (https://panel.mydomain.com) + Live Site Files & DB          │
└─────────────────────────────────────────────────────────────────────────────┘
```

---

## 5. Detailed Component Specifications (Vetted with Builder Review)

### 5.1 Filesystem Architecture (Builder Call)
- **Native ext4 Location:** All website document roots reside at `/var/www/<domain>` inside the WSL2 ext4 virtual disk. This avoids the severe 5x–20x I/O performance penalty associated with WSL2 9P/drvfs mounts across Windows NTFS drives.
- **Windows Access Path:** The Tauri desktop shell exposes the site folder to Windows using the official WSL2 network redirector UNC path:
  ```text
  \\wsl.localhost\dockpanel-desktop\var\www\<domain>
  ```
- **"Open in Explorer":** Spawns `explorer.exe \\wsl.localhost\dockpanel-desktop\var\www\<domain>`.
- **"Open in VS Code":** Spawns `code.cmd --remote wsl+dockpanel-desktop /var/www/<domain>` (or opens the UNC path directly if remote extension is not installed).

### 5.2 Zero-UAC Hosts & SSL Helper Service (Builder Call)
- To eliminate repetitive Windows UAC elevation prompts every time a user creates or deletes a site:
  - During the initial Inno Setup installation (which already runs elevated with Administrator rights), an auxiliary background service **`DockPanelHostHelper.exe`** is installed as a Windows Service (`LocalSystem`).
  - The service listens on a secure local named pipe (`\\.\pipe\dockpanel-host-helper`).
  - When the user adds a `.test` or `.local` site, Tauri sends a request through the named pipe to safely insert or remove the host line in `C:\Windows\System32\drivers\etc\hosts`.
- **Local HTTPS:** `mkcert` runs during installation to register the root CA into the Windows Root Certificate Store. Per-site certs are generated and mounted into Nginx inside WSL2.

### 5.3 Desktop Authentication & Frictionless Startup (Builder Call)
- When the WSL2 micro-instance boots, `dockpanel-api` automatically initializes a pre-seeded local administrative key if none exists.
- The Tauri desktop client automatically exchanges this local key with the API on `http://127.0.0.1:3080/api/auth/desktop-bootstrap` to acquire a persistent JWT session.
- The user is never presented with a login screen on their local desktop machine.

### 5.4 Live Server Synchronization (Bidirectional Push / Pull)
- **Configuration (Settings → Remote Sync):**
  - Store remote server connection profile: Remote Panel URL, API Token, and optional SSH credentials (Host, Port, User, SSH Key).
- **Transport Mechanisms:**
  - **Preferred (High-Speed):** Rsync over SSH for diff-based file synchronization.
  - **Fallback (API-Only):** If SSH credentials are not provided, uses the DockPanel API backup & archive deployment pipeline (`/sites/{id}/backup/stream` and `/sites/{id}/restore/archive`) to sync files without requiring direct server shell access.
- **Database Search-and-Replace:**
  - Automatically replaces domains (e.g. `https://myproject.test` ↔ `https://livesite.com`) in the SQL stream using WP-CLI `search-replace` or deterministic regex replacement across serialized PHP options.
- **Pre-Flight Safety Snapshot:**
  - A push operation *always* creates a remote snapshot before writing changes to the live server.

### 5.5 Packaging Model Decision (For User Approval)
- **Standalone Offline Installer (~250–350 MB):** All-in-one Inno Setup `.exe` that bundles the pre-packaged WSL2 micro-distro (`dockpanel-runtime.tar.gz`). Works completely offline, installs in under 60 seconds without network dependencies (Recommended, matching LocalWP / Laragon).
- **Stub Web Installer (~15 MB):** Downloads the 250MB micro-distro from a GitHub release CDN during installation.

---

## 6. What is Left to the Builder
- Selection of specific Inno Setup visual theme.
- Exact styling of the "Sync to Live" visual diff review modal.
- Packaging script to generate the minimal rootfs archive (`dockpanel-runtime.tar.gz`).
