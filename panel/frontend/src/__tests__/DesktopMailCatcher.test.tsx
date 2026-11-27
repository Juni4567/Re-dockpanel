import { describe, it, expect, beforeEach, afterEach } from "vitest";
import React from "react";
import { renderToString } from "react-dom/server";
import { MemoryRouter } from "react-router-dom";
import DesktopMailCatcher from "../components/DesktopMailCatcher";

if (typeof (globalThis as any).window === "undefined") {
  (globalThis as any).window = globalThis;
}

describe("DesktopMailCatcher Component", () => {
  const originalTauri = (window as any).__TAURI__;

  beforeEach(() => {
    (window as any).__TAURI__ = {};
  });

  afterEach(() => {
    if (originalTauri !== undefined) {
      (window as any).__TAURI__ = originalTauri;
    } else {
      delete (window as any).__TAURI__;
    }
  });

  it("renders local mail catcher header, test button, and inbox layout", () => {
    const html = renderToString(
      React.createElement(
        MemoryRouter,
        { initialEntries: ["/mail?domain=mysite.test"] },
        React.createElement(DesktopMailCatcher)
      )
    );

    expect(html).toContain("Local Mail Catcher");
    expect(html).toContain("Send Test Email");
    expect(html).toContain("Clear Inbox");
    expect(html).toContain("mysite.test");
  });
});
