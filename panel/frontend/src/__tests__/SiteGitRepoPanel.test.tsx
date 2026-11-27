import { describe, it, expect } from "vitest";
import React from "react";
import { renderToString } from "react-dom/server";
import SiteGitRepoPanel from "../components/SiteGitRepoPanel";

describe("SiteGitRepoPanel Component", () => {
  it("renders Git repository panel for admin user", () => {
    const html = renderToString(
      React.createElement(SiteGitRepoPanel, {
        siteId: "site-123",
        siteDomain: "example.test",
        runtime: "php",
        phpPreset: "wordpress",
        isAdmin: true,
      })
    );
    expect(html).toContain("Site Git Repository");
    expect(html).toContain("Initialize version control inside the site folder");
  });

  it("restricts Git initialization/connection in non-admin Cloud view", () => {
    const html = renderToString(
      React.createElement(SiteGitRepoPanel, {
        siteId: "site-123",
        siteDomain: "example.test",
        isAdmin: false,
      })
    );
    expect(html).toContain("restricted to Admin accounts");
  });
});
