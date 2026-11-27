import { describe, it, expect, beforeEach } from "vitest";
import React from "react";
import { renderToString } from "react-dom/server";
import { ApiError } from "../api";
import {
  NotificationProvider,
  GlobalNotificationBanner,
  buildAlertDetails,
  defaultAutoDismissMs,
} from "../context/NotificationContext";

describe("NotificationContext & Real-Time Alerts", () => {
  beforeEach(() => {
    if (typeof (globalThis as any).window === "undefined") {
      (globalThis as any).window = globalThis;
    }
  });

  it("extracts rich HTTP diagnostic metadata from ApiError via buildAlertDetails", () => {
    const err = new ApiError(
      422,
      "SQL import failed on line 18",
      "invalid_sql",
      { error: "SQL import failed on line 18", hint: "Check charset" },
      "POST",
      "/databases/db-1/import",
      184,
      "2026-11-27T09:00:00.000Z"
    );

    const parsed = buildAlertDetails(err, {
      context: "Restore dump",
      category: "database",
      target: "wp_shop",
    });

    expect(parsed.message).toBe("SQL import failed on line 18");
    expect(parsed.meta.statusCode).toBe(422);
    expect(parsed.meta.method).toBe("POST");
    expect(parsed.meta.endpoint).toBe("/databases/db-1/import");
    expect(parsed.meta.durationMs).toBe(184);
    expect(parsed.meta.code).toBe("invalid_sql");
    expect(parsed.meta.target).toBe("wp_shop");
    expect(parsed.details).toContain("Operation: Restore dump");
    expect(parsed.details).toContain("POST /api/databases/db-1/import (184ms)");
    expect(parsed.details).toContain("HTTP Code: 422 [invalid_sql]");
    expect(parsed.details).toContain("Check charset");
  });

  it("extracts stack summary from generic Error instances", () => {
    const err = new Error("Connection refused");
    const parsed = buildAlertDetails(err, {
      context: "Connect live sync",
      target: "shop.test",
    });

    expect(parsed.message).toBe("Connection refused");
    expect(parsed.meta.target).toBe("shop.test");
    expect(parsed.details).toContain("Operation: Connect live sync");
    expect(parsed.details).toContain("Target:    shop.test");
    expect(parsed.details).toContain("Error:     Error: Connection refused");
  });

  it("computes severity-aware auto-dismiss durations and respects overrides", () => {
    expect(defaultAutoDismissMs("error")).toBe(0);
    expect(defaultAutoDismissMs("warn")).toBe(12000);
    expect(defaultAutoDismissMs("success")).toBe(8000);
    expect(defaultAutoDismissMs("info")).toBe(8000);
    expect(defaultAutoDismissMs("success", { autoDismissMs: 5000 })).toBe(5000);
    expect(defaultAutoDismissMs("error", { autoDismissMs: 4000 })).toBe(4000);
  });

  it("renders NotificationProvider with empty GlobalNotificationBanner cleanly", () => {
    const html = renderToString(
      React.createElement(
        NotificationProvider,
        null,
        React.createElement(GlobalNotificationBanner, null)
      )
    );
    expect(html).toBe("");
  });
});
