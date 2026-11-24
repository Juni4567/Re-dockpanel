# DockPanel Desktop: Native Site Server, Start/Stop Controls & Port Conflict Resolution

## 1. Intent & Why
DockPanel Desktop is a pure-Rust desktop application that manages local web development environments natively on Windows. Currently, the desktop app displays sites (e.g. `mytestdomain.local`, `wordpress.test`), but the sites are not accessible via web browser because:
1. There is no web server/router listening on the host's web port (80/8080) mapping incoming `Host` requests to site document roots (`%USERPROFILE%\DockPanel\sites\<domain>\`).
2. Standard web port 80 is occupied on the user's machine by WampServer's Apache (`httpd.exe`).
3. There are no controls in the panel UI to Start/Stop individual sites or view their runtime status.
4. The local domains are not registered in the Windows `hosts` file (`C:\Windows\System32\drivers\etc\hosts`).

The goal is to provide a seamless local development experience comparable to LocalWP or Laravel Herd, allowing users to start/stop sites, resolve port conflicts with an interactive dialog, and view their sites in any browser.

---

## 2. Goals, Non-Goals, Anti-Goals

### Goals
- **Native Site Router**: Listen on a user-chosen router port (default 80, fallback 8080) in the Rust application, routing `Host: <domain>` requests to the respective site's folder.
- **Static & PHP Serving**: Serve static files (`.html`, `.css`, `.js`, images) directly from the site directory. For PHP requests (`.php` files), proxy requests to a background PHP built-in server worker (`php.exe -S 127.0.0.1:<worker_port>`) using the installed host PHP CLI (`C:\wamp64\bin\php\php8.3.0\php.exe` or detected path).
- **Start / Stop Controls**: Provide individual site toggle controls (▶️ Start / ⏹️ Stop) in the desktop Sites table and grid view.
- **Stopped State Page**: When a stopped site is accessed via browser, render a clean, branded DockPanel holding page explaining the site is stopped and providing instructions to start it.
- **Port Conflict Detection**: Detect whether port 80 (or the chosen router port) is in use before binding, identify the conflicting process (e.g., `httpd.exe` / WampServer), and offer a resolution dialog in the UI.
- **Interactive Port Resolution**: Allow the user to choose between:
  - Switching to an alternative free port (e.g., port 8080, e.g. `http://mytestdomain.local:8080`).
  - Stopping the conflicting process (with explicit user confirmation).
- **Hosts File Synchronization**: Ensure site domains (`mytestdomain.local`, `wordpress.test`) are added to the Windows `hosts` file mapped to `127.0.0.1`.
- **Dynamic Site Links**: Update "Open in new tab" links in the frontend to include the custom port when a non-standard port is active (e.g. `http://mytestdomain.local:8080`).

### Non-Goals
- WSL2 or Docker dependencies: This is a 100% native Windows Rust implementation.
- Full Apache/Nginx configuration file generation: The router is an embedded, high-performance Axum/Tokio HTTP server.
- Multi-tenant cloud production serving: This is strictly for local desktop development.

### Anti-Goals
- Killing or modifying third-party software (like WampServer) automatically without explicit user confirmation.
- Modifying the user's global PHP configuration or WampServer installation files.

---

## 3. Constraints
- **Windows OS**: Paths must support Windows delimiters (`\` and `/`), Windows paths like `%USERPROFILE%\DockPanel\sites`, and Windows host naming.
- **Pure Rust Architecture**: The desktop app must remain a pure Rust binary (`dockpanel.exe`) without Tauri dependencies or external background runtime daemons.
- **Port 80 Privileges / Conflicts**: Port 80 is currently held by PID 20408 (`httpd.exe`). The solution must gracefully handle port binding errors and allow port 8080 or other alternatives.

---

## 4. Approaches Considered
1. **Approach 1: Native In-Process Site Router + Port Conflict Dialog (Selected)**
   - Embedded Tokio/Axum HTTP server running inside `dockpanel.exe` alongside the panel server.
   - Detects port 80 occupation, queries process name via Windows APIs, and provides an interactive modal in the frontend with alternatives (Port 8080 default, or kill process).
   - Serves static assets, proxies PHP, and returns a "Site is Stopped" page when inactive.
   - *Rationale*: Zero dependencies, works seamlessly alongside or independently of WampServer, completely portable.

2. **Approach 2: Per-Site Dedicated Ports (e.g. localhost:3001, localhost:3002) (Rejected)**
   - Binds a distinct port per site without domain routing.
   - *Why rejected*: Users cannot use clean domain names like `http://mytestdomain.local`; poor developer experience.

3. **Approach 3: WampServer VirtualHost Integration (Rejected)**
   - Writes Apache vhost configs directly into WampServer's `httpd-vhosts.conf`.
   - *Why rejected*: Ties DockPanel to WampServer being installed and running; breaks standalone operation.

---

## 5. System Architecture & Detailed Design

### 5.1 Port Conflict Detection & Router Configuration
- **Configuration Storage**: `%APPDATA%\DockPanel\router_config.json`:
  ```json
  {
    "preferred_port": 80,
    "active_port": 8080,
    "fallback_ports": [8080, 8443, 3000]
  }
  ```
- **Port Checker**:
  - `check_port_status(port: u16) -> PortStatus`:
    Attempts a TCP bind on `127.0.0.1:port`. If `AddrInUse`, queries the PID using Windows `GetExtendedTcpTable` (or `netstat` lookup) and retrieves the executable name (e.g. `httpd.exe (WampServer)`).
- **API Endpoints**:
  - `GET /api/system/ports/conflict?port=80`:
    Returns:
    ```json
    {
      "preferred_port": 80,
      "active_port": 8080,
      "in_use": true,
      "process_name": "httpd.exe",
      "display_name": "Apache HTTP Server (WampServer)",
      "pid": 20408,
      "can_auto_stop": false,
      "alternatives": [8080, 8443, 3000]
    }
    ```
  - `POST /api/system/ports/resolve`:
    Payload: `{ "action": "use_port", "port": 8080 }` OR `{ "action": "stop_process", "pid": 20408, "port": 80 }`
    Updates router configuration and restarts the site router on the selected port.

### 5.2 Multi-Site Router Engine
- **Listener & Startup Flow**:
  - On launch, the router checks the preferred port (default 80). If free, binds port 80.
  - If occupied, the router **automatically falls back** to the first free alternative (8080) and immediately starts serving, setting `active_port: 8080`. This prevents `ERR_CONNECTION_REFUSED` and ensures sites are reachable immediately.
  - The UI displays an info indicator ("Port 80 in use by Apache. Sites served on port 8080. [Change]") allowing users to open the resolution dialog anytime.
- **Request Pipeline**:
  1. Extract `Host` header (strip port if present, e.g. `mytestdomain.local:8080` -> `mytestdomain.local`).
  2. Query `AppState::get_site_by_domain(domain)`.
  3. If site not found: return HTTP 404 with a DockPanel "Domain Not Registered" page.
  4. If site status is `"stopped"`: return HTTP 200 with branded "Site is Stopped" page (including a button or hint to start it in DockPanel).
  5. If site status is `"active"`:
     - Check requested file path relative to site's `document_root`.
     - If path points to a `.php` file or a directory containing `index.php`: proxy HTTP request via `reqwest` to the site's dedicated PHP worker (`http://127.0.0.1:<worker_port>`).
     - If path points to an existing static file (`.html`, `.css`, `.js`, images, fonts): serve with correct MIME type.
     - If directory with `index.html`: serve `index.html`.

### 5.3 PHP Worker Lifecycle Management & Startup Reconciliation
- **Runtime Detection**: Scans known Windows PHP locations:
  1. `C:\wamp64\bin\php\php*\php.exe`
  2. `%PATH%` (`where.exe php`)
  3. `%USERPROFILE%\DockPanel\bin\php\php.exe`
- **Worker Spawning**:
  - When site is started: allocate an ephemeral loopback port (e.g. `127.0.0.1:49152+`), spawn `php.exe -S 127.0.0.1:<worker_port> -t <doc_root>`, and track the child `Child` handle and port in `AppState`.
  - When site is stopped: terminate the child process and free the worker port.
- **Launch Reconciliation**:
  - On application startup, `AppState` reads `sites.json`. For any site with `"status": "active"`, it immediately spawns a background PHP worker so the site is instantly ready without manual restarts.

### 5.4 Process Termination Privileges (Builder Call)
- If the user selects "Stop Conflicting Process":
  - `dockpanel.exe` attempts standard process termination (`taskkill /PID <pid> /T`).
  - If the process is a Windows Service or elevated (e.g. `wampapache64`), standard user `taskkill` will return Access Denied.
  - In that case, `POST /api/system/ports/resolve` triggers an elevated UAC prompt (`powershell -Command "Start-Process cmd -ArgumentList '/c net stop wampapache64' -Verb RunAs"`) or returns `{ "success": false, "message": "Service is elevated. Please stop WampServer from its tray icon or select Port 8080." }`.
  - Option 1 (Port 8080) is strongly recommended in the UI as the non-destructive, zero-privilege choice.

### 5.5 Site Start / Stop API
- `POST /api/sites/{id}/start`:
  - Sets site status to `"active"`.
  - Spawns PHP worker if site uses PHP runtime.
  - Ensures Windows hosts entry exists for `<domain>`.
  - Saves state to `sites.json`.
  - Returns updated site object.
- `POST /api/sites/{id}/stop`:
  - Sets site status to `"stopped"`.
  - Terminates PHP worker process.
  - Saves state to `sites.json`.
  - Returns updated site object.

### 5.6 Hosts File Synchronization
- Updates `C:\Windows\System32\drivers\etc\hosts`:
  - Directly adds entries formatted as:
    ```
    127.0.0.1 mytestdomain.local # dockpanel-desktop
    127.0.0.1 wordpress.test # dockpanel-desktop
    ```
  - Also attempts notifying `\\.\pipe\dockpanel-host-helper`.

### 5.7 Frontend UI Components
- **`DesktopSiteActions.tsx`**:
  - Adds Start / Stop button (▶️ Start Site / ⏹️ Stop Site) with loading spinner.
  - Displays site status badge: 🟢 Active (Running) vs ⚪ Stopped.
- **Port Conflict Modal (`PortConflictModal.tsx`)**:
  - Triggered by clicking the port warning badge or selecting "Change Port".
  - Shows clear visual alert:
    *"Port 80 is in use by Apache (WampServer). Choose how to proceed:"*
    - **Option 1**: Use Port 8080 (`http://<domain>:8080`) [Recommended / Pre-selected].
    - **Option 2**: Stop Apache (WampServer) [Requires confirmation / UAC].
  - Remembers user choice in router configuration.
- **`Sites.tsx`**:
  - Dynamic link generation: Computes external URL taking the router port into account (e.g. `http://${site.domain}:8080` if router port is 8080, or `http://${site.domain}` if port 80).
  - Listens for status updates and updates UI optimistically.

---

## 6. What's Left to the Builder
- Exact layout styling of the Port Conflict Modal adhering to Tailwind dark theme (`bg-dark-800`, `text-dark-100`).
- Buffer size and timeout settings for PHP reverse proxy requests.
