/**
 * Desktop Environment Detection & Path Utilities
 *
 * DockPanel Desktop runs as a local workstation app powered by Tauri v2
 * and an isolated WSL2 micro-instance (dockpanel-desktop).
 */

declare global {
  interface Window {
    __TAURI__?: unknown;
  }
}

/**
 * Detects whether the panel is running inside the desktop application:
 * - Inside the Tauri webview wrapper (window.__TAURI__ is defined)
 * - Under desktop Vite development/preview mode (VITE_APP_MODE === 'desktop')
 */
export function isDesktopApp(): boolean {
  if (typeof window !== "undefined" && (window as any).__TAURI__ !== undefined) {
    return true;
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
