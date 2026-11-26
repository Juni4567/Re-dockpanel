import { describe, it, expect } from "vitest";
import React from "react";
import { renderToString } from "react-dom/server";
import ProvisionLog, { type ProvisionStep } from "../components/ProvisionLog";

describe("ProvisionLog Component", () => {
  it("renders externalSteps with live upload progress and expanded terminal logs", () => {
    const externalSteps: ProvisionStep[] = [
      {
        step: "site_connect",
        label: "Connected existing WordPress folder for shop.test",
        status: "done",
        progress: 100,
        message: "[01:40:00] Folder: C:\\Sites\\shop\n[01:40:00] Registered shop.test",
      },
      {
        step: "upload",
        label: "Uploading backup.sql.gz: 4.2 MB / 10.0 MB (42%)",
        status: "in_progress",
        progress: 42,
        message: "[01:40:01] Reading backup.sql.gz...\n[01:40:02] Uploaded 4.2 MB / 10.0 MB (42%)",
      },
    ];

    const html = renderToString(
      React.createElement(ProvisionLog, {
        title: "Connecting shop.test & Importing Database...",
        defaultShowLogs: true,
        externalSteps,
      }),
    );

    expect(html).toContain("Connecting shop.test &amp; Importing Database...");
    expect(html).toContain("Connected existing WordPress folder for shop.test");
    expect(html).toContain("Uploading backup.sql.gz: 4.2 MB / 10.0 MB (42%)");
    expect(html).toContain("Terminal Output");
    expect(html).toContain("[SITE_CONNECT]");
    expect(html).toContain("[UPLOAD]");
    expect(html).toContain("Uploaded 4.2 MB / 10.0 MB (42%)");
  });

  it("renders Dismiss button when complete step is present and onDismiss is passed", () => {
    const externalSteps: ProvisionStep[] = [
      {
        step: "db_import",
        label: "Imported SQL dump into shop_test_wp",
        status: "done",
        progress: 100,
        message: "[01:40:05] SQL dump imported cleanly via mariadb.exe",
      },
      {
        step: "complete",
        label: "Connected shop.test (shop_test_wp) — 42 URL references updated",
        status: "done",
        progress: 100,
        message: "[01:40:06] Import complete",
      },
    ];

    const html = renderToString(
      React.createElement(ProvisionLog, {
        defaultShowLogs: true,
        externalSteps,
        onDismiss: () => {},
      }),
    );

    expect(html).toContain("Connected shop.test (shop_test_wp)");
    expect(html).toContain("Dismiss");
    expect(html).toContain("100%");
  });
});
