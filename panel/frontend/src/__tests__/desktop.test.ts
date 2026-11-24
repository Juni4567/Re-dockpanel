import { describe, it, expect, beforeEach, afterEach } from "vitest";
import { isDesktopApp, getDesktopSitePath } from "../utils/desktop";
import { isNavVisible, navGroups } from "../data/navItems";

// Ensure window is defined for Node-based test runners
if (typeof (globalThis as any).window === "undefined") {
  (globalThis as any).window = globalThis;
}

describe("Desktop Mode Utils", () => {
  const originalTauri = (window as any).__TAURI__;
  const originalEnvMode = import.meta.env.VITE_APP_MODE;

  beforeEach(() => {
    delete (window as any).__TAURI__;
    (import.meta.env as any).VITE_APP_MODE = undefined;
  });

  afterEach(() => {
    if (originalTauri !== undefined) {
      (window as any).__TAURI__ = originalTauri;
    } else {
      delete (window as any).__TAURI__;
    }
    (import.meta.env as any).VITE_APP_MODE = originalEnvMode;
  });

  it("detects desktop environment correctly", () => {
    (window as any).__TAURI__ = {};
    expect(isDesktopApp()).toBe(true);
    delete (window as any).__TAURI__;
    expect(isDesktopApp()).toBe(false);
  });

  it("detects desktop environment via VITE_APP_MODE", () => {
    (import.meta.env as any).VITE_APP_MODE = "desktop";
    expect(isDesktopApp()).toBe(true);
  });

  it("constructs valid Windows WSL UNC path for local sites", () => {
    const path = getDesktopSitePath("mysite.test");
    expect(path).toBe("\\\\wsl.localhost\\dockpanel-desktop\\var\\www\\mysite.test");
  });

  it("prunes server-only navigation items when in desktop mode", () => {
    (window as any).__TAURI__ = {};
    expect(isDesktopApp()).toBe(true);

    const serverOnlyPaths = [
      "/dns",
      "/security",
      "/monitoring",
      "/servers",
      "/reseller",
      "/reseller/users",
      "/resellers",
    ];
    for (const group of navGroups) {
      for (const item of group.items) {
        if (serverOnlyPaths.includes(item.to)) {
          expect(isNavVisible(item, "admin")).toBe(false);
        }
      }
    }
  });

  it("preserves standard developer navigation items in desktop mode", () => {
    (window as any).__TAURI__ = {};
    expect(isDesktopApp()).toBe(true);

    const devPaths = ["/", "/sites", "/databases", "/wordpress-toolkit", "/account"];
    for (const group of navGroups) {
      for (const item of group.items) {
        if (devPaths.includes(item.to)) {
          expect(isNavVisible(item, "admin")).toBe(true);
        }
      }
    }
  });
});
