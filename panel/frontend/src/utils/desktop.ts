import { api } from "../api";

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
 * Returns the default Windows folder path for a local site's document root.
 */
export function getDesktopSitePath(domain: string): string {
  return `%USERPROFILE%\\DockPanel\\sites\\${domain}`;
}

function isPureRustDesktop(): boolean {
  return (
    typeof window !== "undefined" &&
    ((window as any).__DOCKPANEL__ !== undefined ||
      (typeof import.meta !== "undefined" &&
        import.meta.env &&
        import.meta.env.VITE_APP_MODE === "desktop"))
  );
}

/**
 * Opens a local site folder in Windows File Explorer.
 */
export async function openDesktopFolder(domain: string): Promise<boolean> {
  const path = getDesktopSitePath(domain);
  if (isPureRustDesktop()) {
    try {
      await api.post("/desktop/open-folder", { domain });
      return true;
    } catch (err) {
      console.warn("Failed to open folder in DockPanel Desktop:", err);
      return false;
    }
  }
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
 * Opens a local site in VS Code.
 */
export async function openDesktopVSCode(domain: string): Promise<boolean> {
  const path = getDesktopSitePath(domain);
  if (isPureRustDesktop()) {
    try {
      await api.post("/desktop/open-vscode", { domain });
      return true;
    } catch (err) {
      console.warn("Failed to open VS Code in DockPanel Desktop:", err);
      return false;
    }
  }
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
 * Navigation paths that only make sense on a Linux VPS, Docker fleet, or multi-tenant server
 * and are pruned from local Windows DockPanel Desktop navigation.
 */
export const SERVER_ONLY_NAV_PATHS: readonly string[] = [
  "/dns",
  "/security",
  "/monitoring",
  "/servers",
  "/reseller",
  "/reseller/users",
  "/resellers",
  "/apps",
  "/git-deploys",
  "/migration",
  "/cdn",
  "/mail",
  "/backup-orchestrator",
  "/container-policies",
  "/integrations",
  "/secrets",
  "/telemetry",
  "/terminal",
  "/users",
  "/updates",
];

export function isServerOnlyPath(path: string): boolean {
  const base = path.split("?")[0];
  return SERVER_ONLY_NAV_PATHS.includes(base);
}
