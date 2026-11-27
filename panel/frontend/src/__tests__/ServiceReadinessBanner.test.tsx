import { describe, it, expect } from "vitest";
import React from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { MemoryRouter } from "react-router-dom";
import { ServiceReadinessView, type ServicesStatusPayload } from "../components/ServiceReadinessBanner";

describe("ServiceReadinessBanner Component", () => {
  const startingPayload: ServicesStatusPayload = {
    ready: false,
    initializing: true,
    summary: "Starting MariaDB database server — wait a moment before opening database sites...",
    services: [
      { id: "mariadb", name: "MariaDB", status: "starting", detail: "Starting MariaDB daemon...", port: null },
      { id: "php", name: "PHP & Workers", status: "starting", detail: "Starting workers (1/2)...", active_workers: 1, active_sites: 2 },
      { id: "http_router", name: "HTTP Router", status: "ready", detail: "Listening on 127.0.0.1:80", port: 80 },
      { id: "mail_catcher", name: "Mail Catcher", status: "ready", detail: "SMTP listening on 127.0.0.1:1025", port: 1025 },
    ],
  };

  const readyPayload: ServicesStatusPayload = {
    ready: true,
    initializing: false,
    summary: "All services ready (MariaDB :3306 · HTTP :80 · Mail :1025)",
    services: [
      { id: "mariadb", name: "MariaDB", status: "ready", detail: "Listening on 127.0.0.1:3306", port: 3306 },
      { id: "php", name: "PHP & Workers", status: "ready", detail: "PHP 8.3 · 2 active site worker(s)", active_workers: 2, active_sites: 2 },
      { id: "http_router", name: "HTTP Router", status: "ready", detail: "Listening on 127.0.0.1:80", port: 80 },
      { id: "mail_catcher", name: "Mail Catcher", status: "ready", detail: "SMTP listening on 127.0.0.1:1025", port: 1025 },
    ],
  };

  it("renders initializing state prominently with MariaDB & PHP Starting status", () => {
    const html = renderToStaticMarkup(
      React.createElement(
        MemoryRouter,
        null,
        React.createElement(ServiceReadinessView, {
          data: startingPayload,
          collapsed: false,
          onToggleCollapse: () => {},
        })
      )
    );
    expect(html).toContain("Starting Services");
    expect(html).toContain("MariaDB");
    expect(html).toContain("Starting...");
    expect(html).toContain("HTTP Router");
    expect(html).toContain("Ready :80");
  });

  it("renders ready state and collapsed pill cleanly", () => {
    const htmlExpanded = renderToStaticMarkup(
      React.createElement(
        MemoryRouter,
        null,
        React.createElement(ServiceReadinessView, {
          data: readyPayload,
          collapsed: false,
          onToggleCollapse: () => {},
        })
      )
    );
    expect(htmlExpanded).toContain("System Ready");
    expect(htmlExpanded).toContain("Ready :3306");
    expect(htmlExpanded).toContain("Ready (2 active)");

    const htmlCollapsed = renderToStaticMarkup(
      React.createElement(
        MemoryRouter,
        null,
        React.createElement(ServiceReadinessView, {
          data: readyPayload,
          collapsed: true,
          onToggleCollapse: () => {},
        })
      )
    );
    expect(htmlCollapsed).toContain("System Ready (4/4)");
  });
});
