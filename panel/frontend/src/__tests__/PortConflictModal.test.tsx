import { describe, it, expect, beforeEach, afterEach, vi } from "vitest";
import React from "react";
import { renderToString } from "react-dom/server";
import PortConflictModal, {
  PortConflictInfo,
  isKillablePid,
  buildSiteExternalUrl,
  resolvePortConflict,
} from "../components/PortConflictModal";
import { api } from "../api";

if (typeof (globalThis as any).window === "undefined") {
  (globalThis as any).window = globalThis;
}

describe("PortConflictModal & Dynamic URL Helpers", () => {
  const originalTauri = (window as any).__TAURI__;
  const originalEnvMode = import.meta.env.VITE_APP_MODE;

  const sampleConflict: PortConflictInfo = {
    preferred_port: 80,
    active_port: 8080,
    in_use: true,
    process_name: "httpd.exe",
    display_name: "Apache HTTP Server (WampServer)",
    pid: 4512,
    alternatives: [8080, 8443, 3000],
  };

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

  it("renders nothing when isOpen is false", () => {
    const html = renderToString(
      React.createElement(PortConflictModal, {
        isOpen: false,
        onClose: () => {},
        conflictInfo: sampleConflict,
        onResolved: () => {},
      })
    );
    expect(html).toBe("");
  });

  it("renders warning badge, alternative ports, and kill option for killable PID", () => {
    const html = renderToString(
      React.createElement(PortConflictModal, {
        isOpen: true,
        onClose: () => {},
        conflictInfo: sampleConflict,
        onResolved: () => {},
      })
    );
    const text = html.replace(/<!-- -->/g, "");

    expect(text).toContain(
      "Port 80 is currently in use by Apache HTTP Server (WampServer) (PID 4512)."
    );
    expect(text).toContain("Run site router on alternative port");
    expect(text).toContain("Port 8080");
    expect(text).toContain("Port 8443");
    expect(text).toContain("Port 3000");
    expect(text).toContain("Stop conflicting process (httpd.exe)");
    expect(text).toContain("Cancel");
    expect(text).toContain("Save &amp; Apply");
  });

  it("hides stop conflicting process option for non-killable system PIDs", () => {
    for (const pid of [null, 0, 4]) {
      const html = renderToString(
        React.createElement(PortConflictModal, {
          isOpen: true,
          onClose: () => {},
          conflictInfo: {
            ...sampleConflict,
            process_name: "System",
            display_name: "Windows HTTP.sys (IIS / System)",
            pid,
          },
          onResolved: () => {},
        })
      );

      expect(html).toContain("Run site router on alternative port");
      expect(html).not.toContain("Stop conflicting process");
    }
  });

  it("validates killable PID logic", () => {
    expect(isKillablePid(null)).toBe(false);
    expect(isKillablePid(undefined)).toBe(false);
    expect(isKillablePid(0)).toBe(false);
    expect(isKillablePid(4)).toBe(false);
    expect(isKillablePid(1234)).toBe(true);
  });

  it("builds dynamic external site URLs with active port in desktop mode", () => {
    (window as any).__TAURI__ = {};
    expect(buildSiteExternalUrl("mytestdomain.local", 8080, false)).toBe(
      "http://mytestdomain.local:8080"
    );
    expect(buildSiteExternalUrl("mytestdomain.local", 3000, true)).toBe(
      "http://mytestdomain.local:3000"
    );
    expect(buildSiteExternalUrl("mytestdomain.local", 80, false)).toBe(
      "http://mytestdomain.local"
    );
    expect(buildSiteExternalUrl("mytestdomain.local", 443, true)).toBe(
      "http://mytestdomain.local"
    );
  });

  it("builds standard URLs without desktop port suffix in VPS mode", () => {
    expect(buildSiteExternalUrl("example.com", 8080, false)).toBe("http://example.com");
    expect(buildSiteExternalUrl("example.com", 8080, true)).toBe("https://example.com");
  });

  it("resolves port conflicts via /system/ports/resolve and surfaces backend failures", async () => {
    const postSpy = vi
      .spyOn(api, "post")
      .mockResolvedValueOnce({ success: true, active_port: 8443 })
      .mockResolvedValueOnce({ success: true, active_port: 80 })
      .mockResolvedValueOnce({
        success: false,
        message: "Port is already in use",
      });

    const switched = await resolvePortConflict({ action: "use_port", port: 8443 });
    expect(switched).toBe(8443);
    expect(postSpy).toHaveBeenNthCalledWith(1, "/system/ports/resolve", {
      action: "use_port",
      port: 8443,
    });

    const stopped = await resolvePortConflict({
      action: "stop_process",
      pid: 4512,
      port: 80,
    });
    expect(stopped).toBe(80);
    expect(postSpy).toHaveBeenNthCalledWith(2, "/system/ports/resolve", {
      action: "stop_process",
      pid: 4512,
      port: 80,
    });

    await expect(
      resolvePortConflict({ action: "use_port", port: 80 })
    ).rejects.toThrow("Port is already in use");

    postSpy.mockRestore();
  });
});
