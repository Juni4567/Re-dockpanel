import { describe, it, expect, vi, beforeEach } from "vitest";
import React from "react";
import { renderToString } from "react-dom/server";
import LiveSyncModal from "../components/LiveSyncModal";

// Ensure window is defined for Node-based test runners
if (typeof (globalThis as any).window === "undefined") {
  (globalThis as any).window = globalThis;
}

describe("LiveSyncModal Component", () => {
  const mockSite = {
    id: "site-12345",
    domain: "mysite.test",
  };

  beforeEach(() => {
    vi.clearAllMocks();
  });

  it("renders nothing when isOpen is false", () => {
    const html = renderToString(
      React.createElement(LiveSyncModal, {
        site: mockSite,
        isOpen: false,
        onClose: () => {},
      })
    );
    expect(html).toBe("");
  });

  it("renders modal with site domain, inputs, and toggles when isOpen is true", () => {
    const html = renderToString(
      React.createElement(LiveSyncModal, {
        site: mockSite,
        isOpen: true,
        onClose: () => {},
      })
    );

    // Site domain
    expect(html).toContain("mysite.test");
    expect(html).toContain("Live Server Synchronization");

    // Push / Pull toggle options
    expect(html).toContain("Push to Live");
    expect(html).toContain("Pull from Live");

    // Form inputs
    expect(html).toContain("Remote Panel URL");
    expect(html).toContain("API Token");

    // Sync options
    expect(html).toContain("Sync Database");
    expect(html).toContain("Sync Files");
    expect(html).toContain("Dry Run");

    // Safety badge for Push mode
    expect(html).toContain("Automatic remote safety snapshot before overwrite");

    // Preflight button
    expect(html).toContain("Test Connection");
  });

  it("renders pull mode and displays appropriate pull descriptions", () => {
    const html = renderToString(
      React.createElement(LiveSyncModal, {
        site: mockSite,
        isOpen: true,
        initialMode: "pull",
        onClose: () => {},
      })
    );

    expect(html).toContain("Pull from Live");
    expect(html).toContain("Remote Panel URL");
  });
});
