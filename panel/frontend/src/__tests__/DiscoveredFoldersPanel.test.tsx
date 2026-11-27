import { describe, it, expect } from "vitest";
import React from "react";
import { renderToString } from "react-dom/server";
import DiscoveredFoldersPanel, { type DiscoveredFolder } from "../components/DiscoveredFoldersPanel";

describe("DiscoveredFoldersPanel Component", () => {
  const sampleFolders: DiscoveredFolder[] = [
    {
      name: "myshop.com",
      path: "C:\\Users\\dev\\DockPanel\\sites\\myshop.com",
      suggested_domain: "myshop.test",
      suggested_runtime: "php",
      detected_db_files: [
        {
          name: "backup.sql",
          relative_path: "app/sql/backup.sql",
          path: "C:\\Users\\dev\\DockPanel\\sites\\myshop.com\\app\\sql\\backup.sql",
          size_bytes: 204800,
          detected_prefix: "wp_",
          detected_site_url: "https://myshop.com",
          matches_site: true,
        },
        {
          name: "wrong_site.sql",
          relative_path: "wp-content/backups/wrong_site.sql",
          path: "C:\\Users\\dev\\DockPanel\\sites\\myshop.com\\wp-content\\backups\\wrong_site.sql",
          size_bytes: 102400,
          detected_prefix: "other_",
          detected_site_url: "https://unrelated.example",
          matches_site: false,
          match_reason: "Table prefix 'other_' differs from wp-config.php ('wp_')",
        },
      ],
      inspection: {
        path: "C:\\Users\\dev\\DockPanel\\sites\\myshop.com",
        exists: true,
        is_dir: true,
        is_empty: false,
        file_count: 24,
        detected_type: "wordpress",
        is_wordpress: true,
        has_wp_config: true,
        wp_version: "6.7.1",
        wp_table_prefix: "wp_",
        wp_db_name: "shop_live",
        wp_siteurl: "https://myshop.com",
      },
    },
    {
      name: "scratch-empty",
      path: "C:\\Users\\dev\\DockPanel\\sites\\scratch-empty",
      suggested_domain: "scratch-empty.test",
      suggested_runtime: "static",
      detected_db_files: [],
      inspection: {
        path: "C:\\Users\\dev\\DockPanel\\sites\\scratch-empty",
        exists: true,
        is_dir: true,
        is_empty: true,
        file_count: 0,
        detected_type: "empty",
        is_wordpress: false,
        has_wp_config: false,
      },
    },
  ];

  it("renders suggested folders with editable domain, database upload, and detected dumps", () => {
    const html = renderToString(
      React.createElement(DiscoveredFoldersPanel, {
        sitesDir: "C:\\Users\\dev\\DockPanel\\sites",
        folders: sampleFolders,
        scanning: false,
        onRescan: () => {},
        onImport: async () => {},
        onOpenFullForm: () => {},
      })
    );

    expect(html).toContain("Unregistered Folders Ready to Import");
    expect(html).toContain("myshop.com");
    expect(html).toContain("myshop.test");
    expect(html).toContain("WordPress 6.7.1");
    expect(html).toContain("DB: shop_live");
    expect(html).toContain("app/sql/backup.sql");
    expect(html).toContain("Suggested Match");
    expect(html).toContain("Different Site?");
    expect(html).toContain("Wrong database? Select from directories");
    expect(html).toContain("Upload Database");
    expect(html).toContain("Import Website");
    // Empty folder hidden behind "Show 1 Empty" toggle when non-empty sites exist
    expect(html).toContain("Show 1 Empty");
    expect(html).not.toContain("scratch-empty.test");
  });

  it("renders nothing when no folders exist and not scanning", () => {
    const html = renderToString(
      React.createElement(DiscoveredFoldersPanel, {
        sitesDir: "C:\\Users\\dev\\DockPanel\\sites",
        folders: [],
        scanning: false,
        onRescan: () => {},
        onImport: async () => {},
        onOpenFullForm: () => {},
      })
    );
    expect(html).toBe("");
  });

  it("renders failure recovery banner, workable memory limit suggestion, and retry controls", () => {
    const html = renderToString(
      React.createElement(DiscoveredFoldersPanel, {
        sitesDir: "C:\\Users\\dev\\DockPanel\\sites",
        folders: sampleFolders,
        scanning: false,
        failedImports: {
          "C:\\Users\\dev\\DockPanel\\sites\\myshop.com": {
            domain: "furniture-schools2.local",
            runtime: "php",
            appCommand: "",
            dbFile: null,
            localDbPath: "C:\\Users\\dev\\DockPanel\\sites\\myshop.com\\app\\sql\\backup.sql",
            sourceUrl: "https://myshop.com",
            dbName: "",
            memoryLimitMb: 2048,
            error: "Allowed memory size of 134217728 bytes exhausted",
            memorySuggestion: "Workable Fix: Bump Import Memory to 2048 MB and retry.",
          },
        },
        onRescan: () => {},
        onImport: async () => {},
        onOpenFullForm: () => {},
      })
    );

    expect(html).toContain("Previous import failed");
    expect(html).toContain("Workable Fix: Bump Import Memory to 2048 MB and retry.");
    expect(html).toContain("Retry Import");
    expect(html).toContain("Retry Now (2048 MB Limit)");
    expect(html).toContain("Retry with Unlimited Memory (-1)");
    expect(html).toContain("furniture-schools2.local");
  });
});
