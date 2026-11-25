import { describe, it, expect, vi, beforeEach } from "vitest";
import React from "react";
import { renderToString } from "react-dom/server";
import InstallWordPressModal, { generateRandomPassword } from "../components/InstallWordPressModal";

if (typeof (globalThis as any).window === "undefined") {
  (globalThis as any).window = globalThis;
}

describe("InstallWordPressModal Component", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it("renders nothing when isOpen is false", () => {
    const html = renderToString(
      React.createElement(InstallWordPressModal, {
        isOpen: false,
        onClose: () => {},
        siteId: "test-site-123",
        domain: "mysite.local",
      })
    );
    expect(html).toBe("");
  });

  it("renders form elements when isOpen is true", () => {
    const html = renderToString(
      React.createElement(InstallWordPressModal, {
        isOpen: true,
        onClose: () => {},
        siteId: "test-site-123",
        domain: "mysite.local",
      })
    );
    expect(html).toContain("Install WordPress");
    expect(html).toContain("mysite.local");
    expect(html).toContain("Site Title");
    expect(html).toContain("Admin Username");
    expect(html).toContain("Admin Password");
    expect(html).toContain("Admin Email");
    expect(html).toContain("Generate strong password");
  });

  it("generates random password of specified length", () => {
    const pass = generateRandomPassword(20);
    expect(pass.length).toBe(20);
    expect(typeof pass).toBe("string");
  });

  it("exports valid component with proper props signature", () => {
    expect(typeof InstallWordPressModal).toBe("function");
  });
});
