import { describe, it, expect, beforeEach, afterEach, vi } from "vitest";
import React from "react";
import { renderToString } from "react-dom/server";
import DesktopSiteActions, {
  toggleDesktopSiteStatus,
} from "../components/DesktopSiteActions";
import { api } from "../api";
import {
  openDesktopFolder,
  openDesktopVSCode,
  formatDesktopDomain,
} from "../utils/desktop";

// Ensure window is defined for Node-based test runners
if (typeof (globalThis as any).window === "undefined") {
  (globalThis as any).window = globalThis;
}

describe("DesktopSiteActions Component & Native OS Hooks", () => {
  const originalTauri = (window as any).__TAURI__;
  const originalEnvMode = import.meta.env.VITE_APP_MODE;

  beforeEach(() => {
    delete (window as any).__TAURI__;
    (import.meta.env as any).VITE_APP_MODE = undefined;
    vi.clearAllMocks();
  });

  afterEach(() => {
    if (originalTauri !== undefined) {
      (window as any).__TAURI__ = originalTauri;
    } else {
      delete (window as any).__TAURI__;
    }
    (import.meta.env as any).VITE_APP_MODE = originalEnvMode;
  });

  it("renders nothing when not running in desktop mode", () => {
    const rendered = DesktopSiteActions({ domain: "myproject.test" });
    expect(rendered).toBeNull();
  });

  it("renders Open in Explorer and Open in VS Code buttons in desktop mode", () => {
    (window as any).__TAURI__ = {};
    const element = DesktopSiteActions({ domain: "myproject.test" });
    expect(element).not.toBeNull();

    const html = renderToString(React.createElement(DesktopSiteActions, { domain: "myproject.test" }));
    expect(html).toContain("Open in Explorer");
    expect(html).toContain("Open in VS Code");
    expect(html).toContain("<svg");
  });

  it("invokes Tauri open_folder with WSL UNC path when opening folder", async () => {
    const mockInvoke = vi.fn().mockResolvedValue(true);
    (window as any).__TAURI__ = { invoke: mockInvoke };

    const success = await openDesktopFolder("myproject.test");
    expect(success).toBe(true);
    expect(mockInvoke).toHaveBeenCalledWith("open_folder", {
      path: "\\\\wsl.localhost\\dockpanel-desktop\\var\\www\\myproject.test",
    });
  });

  it("invokes Tauri open_vscode with WSL UNC path when opening in VS Code", async () => {
    const mockInvoke = vi.fn().mockResolvedValue(true);
    (window as any).__TAURI__ = { invoke: mockInvoke };

    const success = await openDesktopVSCode("myproject.test");
    expect(success).toBe(true);
    expect(mockInvoke).toHaveBeenCalledWith("open_vscode", {
      path: "\\\\wsl.localhost\\dockpanel-desktop\\var\\www\\myproject.test",
    });
  });

  it("supports Tauri v2 window.__TAURI__.core.invoke bridge", async () => {
    const mockInvoke = vi.fn().mockResolvedValue(true);
    (window as any).__TAURI__ = { core: { invoke: mockInvoke } };

    await openDesktopFolder("store.local");
    expect(mockInvoke).toHaveBeenCalledWith("open_folder", {
      path: "\\\\wsl.localhost\\dockpanel-desktop\\var\\www\\store.local",
    });

    await openDesktopVSCode("store.local");
    expect(mockInvoke).toHaveBeenCalledWith("open_vscode", {
      path: "\\\\wsl.localhost\\dockpanel-desktop\\var\\www\\store.local",
    });
  });

  it("falls back gracefully when Tauri invoke throws an error", async () => {
    const consoleWarnSpy = vi.spyOn(console, "warn").mockImplementation(() => {});
    (window as any).__TAURI__ = {
      invoke: vi.fn().mockRejectedValue(new Error("IPC connection failed")),
    };

    const folderRes = await openDesktopFolder("fail.test");
    expect(folderRes).toBe(false);

    const vscodeRes = await openDesktopVSCode("fail.test");
    expect(vscodeRes).toBe(false);

    consoleWarnSpy.mockRestore();
  });

  it("formats desktop domains with default .test suffix", () => {
    expect(formatDesktopDomain("mysite")).toBe("mysite.test");
    expect(formatDesktopDomain("mysite", ".test")).toBe("mysite.test");
  });

  it("formats desktop domains with .local suffix when selected", () => {
    expect(formatDesktopDomain("mysite", ".local")).toBe("mysite.local");
  });

  it("avoids duplicate suffixes if user types domain with .test or .local", () => {
    expect(formatDesktopDomain("mysite.test", ".test")).toBe("mysite.test");
    expect(formatDesktopDomain("mysite.test", ".local")).toBe("mysite.local");
    expect(formatDesktopDomain("mysite.local", ".test")).toBe("mysite.test");
    expect(formatDesktopDomain("mysite.local", ".local")).toBe("mysite.local");
  });

  it("handles empty or whitespace strings cleanly", () => {
    expect(formatDesktopDomain("")).toBe("");
    expect(formatDesktopDomain("   ")).toBe("");
  });

  it("renders Stop Site button when siteId is provided and status is active", () => {
    (window as any).__TAURI__ = {};
    const html = renderToString(
      React.createElement(DesktopSiteActions, {
        domain: "myproject.test",
        siteId: "site-123",
        status: "active",
      })
    );
    expect(html).toContain("Stop Site");
    expect(html).toContain("text-red-400");
    expect(html).not.toContain("Start Site");
  });

  it("renders Start Site button when siteId is provided and status is stopped", () => {
    (window as any).__TAURI__ = {};
    const html = renderToString(
      React.createElement(DesktopSiteActions, {
        domain: "myproject.test",
        siteId: "site-123",
        status: "stopped",
      })
    );
    expect(html).toContain("Start Site");
    expect(html).toContain("text-emerald-400");
    expect(html).not.toContain("Stop Site");
  });

  it("omits Start/Stop button when siteId is not provided", () => {
    (window as any).__TAURI__ = {};
    const html = renderToString(
      React.createElement(DesktopSiteActions, {
        domain: "myproject.test",
      })
    );
    expect(html).not.toContain("Start Site");
    expect(html).not.toContain("Stop Site");
  });

  it("calls /sites/:id/stop and /sites/:id/start via toggleDesktopSiteStatus", async () => {
    const postSpy = vi
      .spyOn(api, "post")
      .mockResolvedValueOnce({ status: "stopped" })
      .mockResolvedValueOnce({ status: "active" });

    const stoppedStatus = await toggleDesktopSiteStatus("site-123", "active");
    expect(postSpy).toHaveBeenNthCalledWith(1, "/sites/site-123/stop", {});
    expect(stoppedStatus).toBe("stopped");

    const activeStatus = await toggleDesktopSiteStatus("site-123", "stopped");
    expect(postSpy).toHaveBeenNthCalledWith(2, "/sites/site-123/start", {});
    expect(activeStatus).toBe("active");

    postSpy.mockRestore();
  });
});
