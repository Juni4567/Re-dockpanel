import { describe, it, expect } from "vitest";
import React from "react";
import { renderToString } from "react-dom/server";
import DatabaseMaintenanceTab from "../components/DatabaseMaintenanceTab";
import DatabaseCleanupModal from "../components/DatabaseCleanupModal";

describe("Database Maintenance UI Components", () => {
  it("renders initial loading skeleton cleanly in DatabaseMaintenanceTab", () => {
    const html = renderToString(
      React.createElement(DatabaseMaintenanceTab, {
        databaseId: "wp:site-1",
        databaseName: "wp_shop",
        engine: "sqlite",
      })
    );
    expect(html).toContain("animate-pulse");
  });

  it("renders Workspace Database Cleanup & Optimization modal cleanly", () => {
    const html = renderToString(
      React.createElement(DatabaseCleanupModal, {
        onClose: () => {},
        onCompleted: () => {},
      })
    );
    expect(html).toContain("Workspace Database Cleanup &amp; Optimization");
    expect(html).toContain("Run Cleanup &amp; Optimize");
  });
});
