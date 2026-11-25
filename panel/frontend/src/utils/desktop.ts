/**
 * Desktop Environment Detection & Path Utilities
 *
 * DockPanel Desktop runs as a pure-Rust native Windows workstation app (Tao + Wry + Axum).
 */

declare global {
  interface Window {
    __DOCKPANEL__?: unknown;
    __TAURI__?: unknown;
    ipc?: { postMessage: (msg: string) => void };
  }
}

/**
 * Detects whether the panel is running inside the desktop application:
 * - Inside the native desktop webview wrapper (window.__DOCKPANEL__, window.ipc, or window.__TAURI__)
 * - Under desktop Vite development/preview mode (VITE_APP_MODE === 'desktop')
 */
export function isDesktopApp(): boolean {
  if (typeof window !== "undefined") {
    const win = window as any;
    if (win.__DOCKPANEL__ !== undefined || win.ipc !== undefined || win.__TAURI__ !== undefined) {
      return true;
    }
  }
  if (
    typeof import.meta !== "undefined" &&
    import.meta.env &&
    import.meta.env.VITE_APP_MODE === "desktop"
  ) {
    return true;
  }
  return false;
}

/**
 * Constructs the Windows WSL2 UNC path for a site's document root in the
 * dockpanel-desktop micro-instance:
 * \\wsl.localhost\dockpanel-desktop\var\www\<domain>
 */
export function getDesktopSitePath(domain: string): string {
  return `\\\\wsl.localhost\\dockpanel-desktop\\var\\www\\${domain}`;
}

/**
 * Invokes a Tauri command via window.__TAURI__ with graceful fallback.
 */
export async function invokeDesktop(cmd: string, args?: Record<string, unknown>): Promise<unknown> {
  if (typeof window !== "undefined") {
    const tauri = (window as any).__TAURI__;
    if (tauri) {
      if (typeof tauri.invoke === "function") {
        return await tauri.invoke(cmd, args);
      }
      if (tauri.core && typeof tauri.core.invoke === "function") {
        return await tauri.core.invoke(cmd, args);
      }
    }
  }
  return null;
}

/**
 * Opens a local site folder in Windows File Explorer via Tauri WSL UNC path.
 */
export async function openDesktopFolder(domain: string): Promise<boolean> {
  const path = getDesktopSitePath(domain);
  try {
    const tauri = typeof window !== "undefined" ? (window as any).__TAURI__ : undefined;
    if (!tauri) {
      console.log(`[Desktop Fallback] Open folder: ${path}`);
      return true;
    }
    await invokeDesktop("open_folder", { path });
    return true;
  } catch (primaryErr) {
    try {
      await invokeDesktop("open_site_folder", { path });
      return true;
    } catch {
      console.warn("Failed to open folder in desktop environment:", primaryErr);
      return false;
    }
  }
}

/**
 * Opens a local site in VS Code via Tauri WSL UNC path.
 */
export async function openDesktopVSCode(domain: string): Promise<boolean> {
  const path = getDesktopSitePath(domain);
  try {
    const tauri = typeof window !== "undefined" ? (window as any).__TAURI__ : undefined;
    if (!tauri) {
      console.log(`[Desktop Fallback] Open VS Code: ${path}`);
      return true;
    }
    await invokeDesktop("open_vscode", { path });
    return true;
  } catch (err) {
    console.warn("Failed to open VS Code in desktop environment:", err);
    return false;
  }
}

/**
 * Formats a local domain name ensuring the desired suffix (.test or .local) is present
 * without accidental duplication.
 */
export function formatDesktopDomain(name: string, suffix: string = ".test"): string {
  const trimmed = name.trim();
  if (!trimmed) return "";
  const base = trimmed.replace(/\.(test|local)$/i, "");
  return `${base}${suffix}`;
}

/**
 * Navigation paths that only make sense on a multi-tenant or internet-facing VPS
 * and are pruned from local desktop navigation:
 * - DNS (local hosts helper handles .test/.local)
 * - Security/Firewall (local developer workstation, no public attack surface)
 * - Monitoring (remote server uptime pinging not applicable locally)
 * - Reseller / My Users / Resellers (no multi-tenant client hosting)
 * - Fleet Servers (runs only on local WSL2 instance)
 */
export const SERVER_ONLY_NAV_PATHS: readonly string[] = [
  "/dns",
  "/security",
  "/monitoring",
  "/servers",
  "/reseller",
  "/reseller/users",
  "/resellers",
];

export function isServerOnlyPath(path: string): boolean {
  const base = path.split("?")[0];
  return SERVER_ONLY_NAV_PATHS.includes(base);
}
