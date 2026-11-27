import { useAuth } from "../context/AuthContext";
import { Navigate } from "react-router-dom";
import { useState, useEffect, useCallback } from "react";
import { api, ApiError } from "../api";
import { Link } from "react-router-dom";
import { timeAgo } from "../utils/format";
import SiteThumbnail from "../components/SiteThumbnail";
import SiteFavicon from "../components/SiteFavicon";
import { showGlobalNotification } from "../context/NotificationContext";
import ConfirmDialog from "../components/ConfirmDialog";

// ─── Interfaces ───────────────────────────────────────────────────────────────

interface WPSite {
  site_id: string;
  domain: string;
  wp_version: string;
  update_available: boolean;
  vulns: number;
  critical_vulns: number;
}

interface SecurityItem {
  name: string;
  label: string;
  description?: string;
  status: string;
  auto_fixable: boolean;
  severity?: string;
}

interface KnownVuln {
  title: string;
  severity: string;
  fixed_in?: string | null;
  vuln_type: string;
}

interface PluginVulnResult {
  slug: string;
  name: string;
  installed_version: string;
  latest_version?: string | null;
  outdated: boolean;
  vulnerabilities: KnownVuln[];
}

interface VulnItem {
  severity: string;
  title: string;
  plugin: string;
}

interface VulnScanResult {
  domain: string;
  plugin_vulns: PluginVulnResult[];
  total_vulns: number;
  critical_count: number;
  high_count: number;
  scanned_at: string;
}

function flattenVulns(result: VulnScanResult): VulnItem[] {
  return (result.plugin_vulns ?? []).flatMap((p) =>
    (p.vulnerabilities ?? []).map((v) => ({
      severity: v.severity,
      title: v.title,
      plugin: p.name || p.slug,
    }))
  );
}

interface SecurityCheckResponse {
  checks?: SecurityItem[];
}

interface HardeningHistoryRow {
  check_name: string;
  status: string;
  details?: string | null;
  checked_at: string;
}

interface BulkUpdateResult {
  site_id: string;
  domain?: string;
  ok: boolean;
  updated?: number;
  message?: string;
}

interface HardenResult {
  name: string;
  applied: boolean;
  message: string;
}

interface WPPlugin {
  name: string;
  slug: string;
  version: string;
  description?: string;
  status: "active" | "inactive";
  update_available: boolean;
  latest_version?: string;
  author?: string;
  plugin_uri?: string;
}

interface WPTheme {
  name: string;
  slug: string;
  version: string;
  description?: string;
  status: "active" | "inactive";
  update_available: boolean;
  latest_version?: string;
  author?: string;
}

// ─── Spinner ──────────────────────────────────────────────────────────────────

function Spinner({ className = "w-4 h-4" }: { className?: string }) {
  return (
    <svg className={`${className} animate-spin`} fill="none" viewBox="0 0 24 24">
      <circle className="opacity-25" cx="12" cy="12" r="10" stroke="currentColor" strokeWidth="4" />
      <path className="opacity-75" fill="currentColor" d="M4 12a8 8 0 018-8v4a4 4 0 00-4 4H4z" />
    </svg>
  );
}

// ─── SiteSelector ─────────────────────────────────────────────────────────────

function SiteSelector({
  sites,
  value,
  onChange,
}: {
  sites: WPSite[];
  value: string;
  onChange: (id: string) => void;
}) {
  return (
    <select
      value={value}
      onChange={(e) => onChange(e.target.value)}
      className="bg-dark-700 border border-dark-500 text-dark-100 text-sm rounded-lg px-3 py-2 focus:outline-none focus:ring-1 focus:ring-rust-500 focus:border-rust-500"
    >
      <option value="">— Select a site —</option>
      {sites.map((s) => (
        <option key={s.site_id} value={s.site_id}>
          {s.domain}
        </option>
      ))}
    </select>
  );
}

// ─── PluginsTab ───────────────────────────────────────────────────────────────

function PluginsTab({ sites }: { sites: WPSite[] }) {
  const [siteId, setSiteId] = useState(sites[0]?.site_id ?? "");
  const [plugins, setPlugins] = useState<WPPlugin[]>([]);
  const [loading, setLoading] = useState(false);
  const [actionLoading, setActionLoading] = useState<string | null>(null);
  const [filter, setFilter] = useState<"all" | "active" | "inactive">("all");
  const [confirmDelete, setConfirmDelete] = useState<WPPlugin | null>(null);

  const fetchPlugins = useCallback(async (id: string) => {
    if (!id) return;
    setLoading(true);
    try {
      const data = await api.get<WPPlugin[]>(`/sites/${id}/wordpress/plugins`);
      setPlugins(Array.isArray(data) ? data : []);
    } catch (e) {
      showGlobalNotification("error", e instanceof ApiError ? e.message : "Failed to load plugins");
      setPlugins([]);
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    if (siteId) fetchPlugins(siteId);
  }, [siteId, fetchPlugins]);

  const pluginAction = async (action: "activate" | "deactivate" | "update" | "delete", slug: string) => {
    setActionLoading(`${action}:${slug}`);
    try {
      await api.post(`/sites/${siteId}/wordpress/plugin/${action}`, { name: slug });
      showGlobalNotification("success", `Plugin ${action}d successfully`);
      await fetchPlugins(siteId);
    } catch (e) {
      showGlobalNotification("error", e instanceof ApiError ? e.message : `Failed to ${action} plugin`);
    } finally {
      setActionLoading(null);
    }
  };

  const updateAll = async () => {
    setActionLoading("update-all");
    try {
      await api.post(`/sites/${siteId}/wordpress/update/plugins`, {});
      showGlobalNotification("success", "All plugins updated successfully");
      await fetchPlugins(siteId);
    } catch (e) {
      showGlobalNotification("error", e instanceof ApiError ? e.message : "Failed to update all plugins");
    } finally {
      setActionLoading(null);
    }
  };

  const visible = plugins.filter((p) =>
    filter === "all" ? true : filter === "active" ? p.status === "active" : p.status === "inactive"
  );

  const updatableCount = plugins.filter((p) => p.update_available).length;

  return (
    <div>
      {/* Controls */}
      <div className="flex flex-wrap items-center gap-3 mb-5">
        <SiteSelector sites={sites} value={siteId} onChange={setSiteId} />
        <button
          onClick={() => fetchPlugins(siteId)}
          disabled={!siteId || loading}
          className="px-3 py-2 bg-dark-700 text-dark-100 rounded-lg text-sm font-medium hover:bg-dark-600 disabled:opacity-50 flex items-center gap-2 transition-colors"
        >
          {loading ? <Spinner /> : (
            <svg className="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}>
              <path strokeLinecap="round" strokeLinejoin="round" d="M16.023 9.348h4.992v-.001M2.985 19.644v-4.992m0 0h4.992m-4.993 0 3.181 3.183a8.25 8.25 0 0 0 13.803-3.7M4.031 9.865a8.25 8.25 0 0 1 13.803-3.7l3.181 3.182m0-4.991v4.99" />
            </svg>
          )}
          Refresh
        </button>

        {/* Filter */}
        <div className="flex gap-1 ml-auto">
          {(["all", "active", "inactive"] as const).map((f) => (
            <button
              key={f}
              onClick={() => setFilter(f)}
              className={`px-3 py-1.5 rounded text-xs font-medium font-mono capitalize transition-colors ${
                filter === f
                  ? "bg-rust-500 text-white"
                  : "bg-dark-700 text-dark-300 hover:bg-dark-600"
              }`}
            >
              {f}
            </button>
          ))}
        </div>

        {updatableCount > 0 && (
          <button
            onClick={updateAll}
            disabled={actionLoading === "update-all"}
            className="px-3 py-2 bg-warn-500 text-white rounded-lg text-sm font-medium hover:bg-warn-600 disabled:opacity-50 flex items-center gap-2 transition-colors"
          >
            {actionLoading === "update-all" ? <Spinner /> : null}
            Update All Plugins ({updatableCount})
          </button>
        )}
      </div>

      {/* Table */}
      {!siteId ? (
        <div className="bg-dark-800 rounded-lg border border-dark-500 p-10 text-center text-sm text-dark-300">
          Select a site above to manage its plugins.
        </div>
      ) : loading ? (
        <div className="bg-dark-800 rounded-lg border border-dark-500 p-10 flex items-center justify-center gap-3 text-dark-300 text-sm">
          <Spinner className="w-5 h-5" /> Loading plugins…
        </div>
      ) : visible.length === 0 ? (
        <div className="bg-dark-800 rounded-lg border border-dark-500 p-10 text-center text-sm text-dark-300">
          No plugins found{filter !== "all" ? ` with status "${filter}"` : ""}.
        </div>
      ) : (
        <div className="bg-dark-800 rounded-lg border border-dark-500 overflow-hidden">
          <table className="w-full text-sm">
            <thead>
              <tr className="border-b border-dark-600">
                <th className="px-4 py-3 text-left text-xs text-dark-300 font-mono uppercase tracking-widest font-medium">Status</th>
                <th className="px-4 py-3 text-left text-xs text-dark-300 font-mono uppercase tracking-widest font-medium">Plugin</th>
                <th className="px-4 py-3 text-left text-xs text-dark-300 font-mono uppercase tracking-widest font-medium hidden md:table-cell">Version</th>
                <th className="px-4 py-3 text-left text-xs text-dark-300 font-mono uppercase tracking-widest font-medium hidden lg:table-cell">Author</th>
                <th className="px-4 py-3 text-right text-xs text-dark-300 font-mono uppercase tracking-widest font-medium">Actions</th>
              </tr>
            </thead>
            <tbody className="divide-y divide-dark-700">
              {visible.map((plugin) => {
                const isActing = actionLoading?.endsWith(`:${plugin.slug}`);
                return (
                  <tr key={plugin.slug} className="hover:bg-dark-700/40 transition-colors">
                    <td className="px-4 py-3">
                      <span
                        className={`inline-flex px-2 py-0.5 rounded text-[10px] font-mono font-medium ${
                          plugin.status === "active"
                            ? "bg-rust-500/15 text-rust-400"
                            : "bg-dark-600 text-dark-300"
                        }`}
                      >
                        {plugin.status}
                      </span>
                    </td>
                    <td className="px-4 py-3">
                      <div className="text-dark-50 font-medium">{plugin.name}</div>
                      {plugin.description && (
                        <div className="text-xs text-dark-400 mt-0.5 max-w-xs truncate">{plugin.description}</div>
                      )}
                    </td>
                    <td className="px-4 py-3 hidden md:table-cell">
                      <span className="font-mono text-xs text-dark-200">{plugin.version}</span>
                      {plugin.update_available && plugin.latest_version && (
                        <span className="ml-1.5 font-mono text-xs text-warn-400">→ {plugin.latest_version}</span>
                      )}
                    </td>
                    <td className="px-4 py-3 hidden lg:table-cell text-xs text-dark-300 font-mono">{plugin.author ?? "—"}</td>
                    <td className="px-4 py-3">
                      <div className="flex items-center justify-end gap-1.5">
                        {/* Toggle active */}
                        <button
                          onClick={() => pluginAction(plugin.status === "active" ? "deactivate" : "activate", plugin.slug)}
                          disabled={!!isActing}
                          className="px-2.5 py-1 bg-dark-700 text-dark-100 rounded text-xs font-medium hover:bg-dark-600 disabled:opacity-50 transition-colors"
                        >
                          {isActing ? <Spinner className="w-3 h-3" /> : plugin.status === "active" ? "Deactivate" : "Activate"}
                        </button>
                        {/* Update */}
                        {plugin.update_available && (
                          <button
                            onClick={() => pluginAction("update", plugin.slug)}
                            disabled={!!isActing}
                            className="px-2.5 py-1 bg-warn-500/15 text-warn-400 border border-warn-500/30 rounded text-xs font-medium hover:bg-warn-500/25 disabled:opacity-50 transition-colors"
                          >
                            {actionLoading === `update:${plugin.slug}` ? <Spinner className="w-3 h-3" /> : "Update"}
                          </button>
                        )}
                        {/* Delete */}
                        <button
                          onClick={() => setConfirmDelete(plugin)}
                          disabled={!!isActing}
                          className="px-2.5 py-1 bg-danger-500/10 text-danger-400 border border-danger-500/20 rounded text-xs font-medium hover:bg-danger-500/20 disabled:opacity-50 transition-colors"
                        >
                          Delete
                        </button>
                      </div>
                    </td>
                  </tr>
                );
              })}
            </tbody>
          </table>
        </div>
      )}

      {/* Confirm Delete */}
      {confirmDelete && (
        <ConfirmDialog
          title="Delete Plugin"
          label={`Are you sure you want to delete "${confirmDelete.name}"? This will permanently remove the plugin files.`}
          consequences={[
            "Plugin files will be permanently deleted from the server.",
            "Any functionality provided by this plugin will stop working immediately.",
            "This action cannot be undone.",
          ]}
          confirmLabel="Delete Plugin"
          onConfirm={async () => {
            const slug = confirmDelete.slug;
            setConfirmDelete(null);
            await pluginAction("delete", slug);
          }}
          onCancel={() => setConfirmDelete(null)}
        />
      )}
    </div>
  );
}

// ─── ThemesTab ────────────────────────────────────────────────────────────────

function ThemesTab({ sites }: { sites: WPSite[] }) {
  const [siteId, setSiteId] = useState(sites[0]?.site_id ?? "");
  const [themes, setThemes] = useState<WPTheme[]>([]);
  const [loading, setLoading] = useState(false);
  const [actionLoading, setActionLoading] = useState<string | null>(null);
  const [confirmDelete, setConfirmDelete] = useState<WPTheme | null>(null);

  const fetchThemes = useCallback(async (id: string) => {
    if (!id) return;
    setLoading(true);
    try {
      const data = await api.get<WPTheme[]>(`/sites/${id}/wordpress/themes`);
      setThemes(Array.isArray(data) ? data : []);
    } catch (e) {
      showGlobalNotification("error", e instanceof ApiError ? e.message : "Failed to load themes");
      setThemes([]);
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    if (siteId) fetchThemes(siteId);
  }, [siteId, fetchThemes]);

  const themeAction = async (action: "activate" | "deactivate" | "update" | "delete", slug: string) => {
    setActionLoading(`${action}:${slug}`);
    try {
      const res = await api.post<{ message?: string }>(`/sites/${siteId}/wordpress/theme/${action}`, { name: slug });
      showGlobalNotification("success", res?.message || `Theme ${action}d successfully`);
      await fetchThemes(siteId);
    } catch (e) {
      showGlobalNotification("error", e instanceof ApiError ? e.message : `Failed to ${action} theme`);
    } finally {
      setActionLoading(null);
    }
  };

  const updateAll = async () => {
    setActionLoading("update-all");
    try {
      await api.post(`/sites/${siteId}/wordpress/update/themes`, {});
      showGlobalNotification("success", "All themes updated successfully");
      await fetchThemes(siteId);
    } catch (e) {
      showGlobalNotification("error", e instanceof ApiError ? e.message : "Failed to update all themes");
    } finally {
      setActionLoading(null);
    }
  };

  const updatableCount = themes.filter((t) => t.update_available).length;

  return (
    <div>
      {/* Controls */}
      <div className="flex flex-wrap items-center gap-3 mb-5">
        <SiteSelector sites={sites} value={siteId} onChange={setSiteId} />
        <button
          onClick={() => fetchThemes(siteId)}
          disabled={!siteId || loading}
          className="px-3 py-2 bg-dark-700 text-dark-100 rounded-lg text-sm font-medium hover:bg-dark-600 disabled:opacity-50 flex items-center gap-2 transition-colors"
        >
          {loading ? <Spinner /> : (
            <svg className="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}>
              <path strokeLinecap="round" strokeLinejoin="round" d="M16.023 9.348h4.992v-.001M2.985 19.644v-4.992m0 0h4.992m-4.993 0 3.181 3.183a8.25 8.25 0 0 0 13.803-3.7M4.031 9.865a8.25 8.25 0 0 1 13.803-3.7l3.181 3.182m0-4.991v4.99" />
            </svg>
          )}
          Refresh
        </button>
        {updatableCount > 0 && (
          <button
            onClick={updateAll}
            disabled={actionLoading === "update-all"}
            className="ml-auto px-3 py-2 bg-warn-500 text-white rounded-lg text-sm font-medium hover:bg-warn-600 disabled:opacity-50 flex items-center gap-2 transition-colors"
          >
            {actionLoading === "update-all" ? <Spinner /> : null}
            Update All Themes ({updatableCount})
          </button>
        )}
      </div>

      {/* Table */}
      {!siteId ? (
        <div className="bg-dark-800 rounded-lg border border-dark-500 p-10 text-center text-sm text-dark-300">
          Select a site above to manage its themes.
        </div>
      ) : loading ? (
        <div className="bg-dark-800 rounded-lg border border-dark-500 p-10 flex items-center justify-center gap-3 text-dark-300 text-sm">
          <Spinner className="w-5 h-5" /> Loading themes…
        </div>
      ) : themes.length === 0 ? (
        <div className="bg-dark-800 rounded-lg border border-dark-500 p-10 text-center text-sm text-dark-300">
          No themes found.
        </div>
      ) : (
        <div className="bg-dark-800 rounded-lg border border-dark-500 overflow-hidden">
          <table className="w-full text-sm">
            <thead>
              <tr className="border-b border-dark-600">
                <th className="px-4 py-3 text-left text-xs text-dark-300 font-mono uppercase tracking-widest font-medium">Status</th>
                <th className="px-4 py-3 text-left text-xs text-dark-300 font-mono uppercase tracking-widest font-medium">Theme</th>
                <th className="px-4 py-3 text-left text-xs text-dark-300 font-mono uppercase tracking-widest font-medium hidden md:table-cell">Version</th>
                <th className="px-4 py-3 text-left text-xs text-dark-300 font-mono uppercase tracking-widest font-medium hidden lg:table-cell">Author</th>
                <th className="px-4 py-3 text-right text-xs text-dark-300 font-mono uppercase tracking-widest font-medium">Actions</th>
              </tr>
            </thead>
            <tbody className="divide-y divide-dark-700">
              {themes.map((theme) => {
                const isActing = actionLoading?.endsWith(`:${theme.slug}`);
                const isActive = theme.status === "active";
                return (
                  <tr key={theme.slug} className="hover:bg-dark-700/40 transition-colors">
                    <td className="px-4 py-3">
                      {isActive ? (
                        <span className="inline-flex px-2 py-0.5 rounded text-[10px] font-mono font-medium bg-rust-500/15 text-rust-400">
                          active
                        </span>
                      ) : (
                        <span className="inline-flex px-2 py-0.5 rounded text-[10px] font-mono font-medium bg-dark-600 text-dark-300">
                          inactive
                        </span>
                      )}
                    </td>
                    <td className="px-4 py-3">
                      <div className="text-dark-50 font-medium">{theme.name}</div>
                      {theme.description && (
                        <div className="text-xs text-dark-400 mt-0.5 max-w-xs truncate">{theme.description}</div>
                      )}
                    </td>
                    <td className="px-4 py-3 hidden md:table-cell">
                      <span className="font-mono text-xs text-dark-200">{theme.version}</span>
                      {theme.update_available && theme.latest_version && (
                        <span className="ml-1.5 font-mono text-xs text-warn-400">→ {theme.latest_version}</span>
                      )}
                    </td>
                    <td className="px-4 py-3 hidden lg:table-cell text-xs text-dark-300 font-mono">{theme.author ?? "—"}</td>
                    <td className="px-4 py-3">
                      <div className="flex items-center justify-end gap-1.5">
                        {!isActive ? (
                          <button
                            onClick={() => themeAction("activate", theme.slug)}
                            disabled={!!isActing}
                            className="px-2.5 py-1 bg-dark-700 text-dark-100 rounded text-xs font-medium hover:bg-dark-600 disabled:opacity-50 transition-colors"
                          >
                            {actionLoading === `activate:${theme.slug}` ? <Spinner className="w-3 h-3" /> : "Activate"}
                          </button>
                        ) : themes.length > 1 ? (
                          <button
                            onClick={() => themeAction("deactivate", theme.slug)}
                            disabled={!!isActing}
                            title="Switch away from active theme directly in the database"
                            className="px-2.5 py-1 bg-dark-700 text-dark-100 rounded text-xs font-medium hover:bg-dark-600 disabled:opacity-50 transition-colors"
                          >
                            {actionLoading === `deactivate:${theme.slug}` ? <Spinner className="w-3 h-3" /> : "Disable"}
                          </button>
                        ) : null}
                        {theme.update_available && (
                          <button
                            onClick={() => themeAction("update", theme.slug)}
                            disabled={!!isActing}
                            className="px-2.5 py-1 bg-warn-500/15 text-warn-400 border border-warn-500/30 rounded text-xs font-medium hover:bg-warn-500/25 disabled:opacity-50 transition-colors"
                          >
                            {actionLoading === `update:${theme.slug}` ? <Spinner className="w-3 h-3" /> : "Update"}
                          </button>
                        )}
                        <button
                          onClick={() => setConfirmDelete(theme)}
                          disabled={!!isActing || isActive}
                          title={isActive ? "Cannot delete the active theme" : "Delete theme"}
                          className="px-2.5 py-1 bg-danger-500/10 text-danger-400 border border-danger-500/20 rounded text-xs font-medium hover:bg-danger-500/20 disabled:opacity-30 disabled:cursor-not-allowed transition-colors"
                        >
                          Delete
                        </button>
                      </div>
                    </td>
                  </tr>
                );
              })}
            </tbody>
          </table>
        </div>
      )}

      {/* Confirm Delete */}
      {confirmDelete && (
        <ConfirmDialog
          title="Delete Theme"
          label={`Are you sure you want to delete "${confirmDelete.name}"? This will permanently remove the theme files.`}
          consequences={[
            "Theme files will be permanently deleted from the server.",
            "If this theme is a parent theme for another theme, that child theme may break.",
            "This action cannot be undone.",
          ]}
          confirmLabel="Delete Theme"
          onConfirm={async () => {
            const slug = confirmDelete.slug;
            setConfirmDelete(null);
            await themeAction("delete", slug);
          }}
          onCancel={() => setConfirmDelete(null)}
        />
      )}
    </div>
  );
}

// ─── ToolsTab ─────────────────────────────────────────────────────────────────

interface SearchReplaceResult {
  matches?: number;
  replacements?: number;
  dry_run?: boolean;
  message?: string;
}

interface OptimizeResult {
  tables_optimized?: number;
  message?: string;
}

function ToolsTab({ sites }: { sites: WPSite[] }) {
  const [siteId, setSiteId] = useState(sites[0]?.site_id ?? "");

  // Maintenance mode
  const [maintenanceEnabled, setMaintenanceEnabled] = useState<boolean | null>(null);
  const [maintenanceLoading, setMaintenanceLoading] = useState(false);

  // Clear cache
  const [cacheLoading, setCacheLoading] = useState(false);
  const [lastClearedAt, setLastClearedAt] = useState<string | null>(null);

  // Optimize DB
  const [optimizeLoading, setOptimizeLoading] = useState(false);
  const [optimizeResult, setOptimizeResult] = useState<OptimizeResult | null>(null);

  // Search & Replace
  const [srSearch, setSrSearch] = useState("");
  const [srReplace, setSrReplace] = useState("");
  const [srDryRun, setSrDryRun] = useState(true);
  const [srLoading, setSrLoading] = useState(false);
  const [srResult, setSrResult] = useState<SearchReplaceResult | null>(null);

  // Reset admin password
  const [newPassword, setNewPassword] = useState("");
  const [showPassword, setShowPassword] = useState(false);
  const [resetLoading, setResetLoading] = useState(false);
  const [confirmResetPassword, setConfirmResetPassword] = useState(false);

  // Reset per-site state when site changes
  useEffect(() => {
    setMaintenanceEnabled(null);
    setLastClearedAt(null);
    setOptimizeResult(null);
    setSrResult(null);
    setNewPassword("");
  }, [siteId]);

  const toggleMaintenance = async () => {
    const newState = !maintenanceEnabled;
    setMaintenanceLoading(true);
    try {
      await api.post(`/sites/${siteId}/wordpress/maintenance`, { enabled: newState });
      setMaintenanceEnabled(newState);
      showGlobalNotification("success", `Maintenance mode ${newState ? "enabled" : "disabled"}`);
    } catch (e) {
      showGlobalNotification("error", e instanceof ApiError ? e.message : "Failed to toggle maintenance mode");
    } finally {
      setMaintenanceLoading(false);
    }
  };

  const clearCache = async () => {
    setCacheLoading(true);
    try {
      await api.post(`/sites/${siteId}/wordpress/clear-cache`, {});
      const now = new Date().toISOString();
      setLastClearedAt(now);
      showGlobalNotification("success", "All caches cleared successfully");
    } catch (e) {
      showGlobalNotification("error", e instanceof ApiError ? e.message : "Failed to clear cache");
    } finally {
      setCacheLoading(false);
    }
  };

  const optimizeDb = async () => {
    setOptimizeLoading(true);
    setOptimizeResult(null);
    try {
      const res = await api.post<OptimizeResult>(`/sites/${siteId}/wordpress/optimize-db`, {});
      setOptimizeResult(res);
      showGlobalNotification("success", res.message ?? `Database optimized (${res.tables_optimized ?? 0} tables)`);
    } catch (e) {
      showGlobalNotification("error", e instanceof ApiError ? e.message : "Failed to optimize database");
    } finally {
      setOptimizeLoading(false);
    }
  };

  const runSearchReplace = async () => {
    if (!srSearch.trim()) {
      showGlobalNotification("warn", "Please enter a search term");
      return;
    }
    setSrLoading(true);
    setSrResult(null);
    try {
      const res = await api.post<SearchReplaceResult>(`/sites/${siteId}/wordpress/search-replace`, {
        search: srSearch,
        replace: srReplace,
        dry_run: srDryRun,
      });
      setSrResult(res);
      if (srDryRun) {
        showGlobalNotification("info", `Dry run complete — ${res.matches ?? 0} match(es) found`);
      } else {
        showGlobalNotification("success", `Search & replace complete — ${res.replacements ?? 0} replacement(s) made`);
      }
    } catch (e) {
      showGlobalNotification("error", e instanceof ApiError ? e.message : "Search & replace failed");
    } finally {
      setSrLoading(false);
    }
  };

  const passwordStrength = (pw: string): { label: string; color: string; width: string } => {
    if (pw.length === 0) return { label: "", color: "bg-dark-600", width: "0%" };
    if (pw.length < 8) return { label: "Weak", color: "bg-danger-500", width: "25%" };
    if (pw.length < 12) return { label: "Fair", color: "bg-warn-500", width: "50%" };
    if (pw.length < 16) return { label: "Good", color: "bg-rust-500", width: "75%" };
    return { label: "Strong", color: "bg-rust-400", width: "100%" };
  };

  const strength = passwordStrength(newPassword);

  const doResetPassword = async () => {
    setConfirmResetPassword(false);
    if (!newPassword) return;
    setResetLoading(true);
    try {
      await api.post(`/sites/${siteId}/wordpress/reset-admin-password`, { new_password: newPassword });
      showGlobalNotification("success", "Admin password reset successfully");
      setNewPassword("");
    } catch (e) {
      showGlobalNotification("error", e instanceof ApiError ? e.message : "Failed to reset admin password");
    } finally {
      setResetLoading(false);
    }
  };

  if (!siteId) {
    return (
      <div>
        <div className="mb-5">
          <SiteSelector sites={sites} value={siteId} onChange={setSiteId} />
        </div>
        <div className="bg-dark-800 rounded-lg border border-dark-500 p-10 text-center text-sm text-dark-300">
          Select a site above to access tools.
        </div>
      </div>
    );
  }

  return (
    <div>
      {/* Site selector */}
      <div className="mb-6">
        <SiteSelector sites={sites} value={siteId} onChange={setSiteId} />
      </div>

      <div className="grid grid-cols-1 md:grid-cols-2 gap-4">

        {/* Card 1: Maintenance Mode */}
        <div className="bg-dark-800 rounded-lg border border-dark-500 p-5">
          <div className="flex items-center gap-3 mb-3">
            <div className="p-2 bg-dark-700 rounded-lg">
              <svg className="w-5 h-5 text-dark-200" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={1.5}>
                <path strokeLinecap="round" strokeLinejoin="round" d="M11.42 15.17 17.25 21A2.652 2.652 0 0 0 21 17.25l-5.877-5.877M11.42 15.17l2.496-3.03c.317-.384.74-.626 1.208-.766M11.42 15.17l-4.655 5.653a2.548 2.548 0 1 1-3.586-3.586l6.837-5.63m5.108-.233c.55-.164 1.163-.188 1.743-.14a4.5 4.5 0 0 0 4.486-6.336l-3.276 3.277a3.004 3.004 0 0 1-2.25-2.25l3.276-3.276a4.5 4.5 0 0 0-6.336 4.486c.091 1.076-.071 2.264-.904 2.95l-.102.085m-1.745 1.437L5.909 7.5H4.5L2.25 3.75l1.5-1.5L7.5 4.5v1.409l4.26 4.26m-1.745 1.437 1.745-1.437m6.615 8.206L15.75 15.75M4.867 19.125h.008v.008h-.008v-.008Z" />
              </svg>
            </div>
            <div>
              <h3 className="text-sm font-semibold text-dark-50">Maintenance Mode</h3>
              <p className="text-xs text-dark-400 mt-0.5">Safely put your site offline for updates</p>
            </div>
            {maintenanceEnabled !== null && (
              <span
                className={`ml-auto px-2 py-0.5 rounded text-[10px] font-mono font-medium ${
                  maintenanceEnabled
                    ? "bg-warn-500/15 text-warn-400"
                    : "bg-dark-600 text-dark-300"
                }`}
              >
                {maintenanceEnabled ? "ENABLED" : "DISABLED"}
              </span>
            )}
          </div>
          <p className="text-xs text-dark-400 mb-4">
            Put your site in maintenance mode to safely perform updates. Visitors will see a "Site under maintenance" page.
          </p>
          <button
            onClick={toggleMaintenance}
            disabled={maintenanceLoading}
            className={`w-full px-4 py-2 rounded-lg text-sm font-medium flex items-center justify-center gap-2 transition-colors ${
              maintenanceEnabled
                ? "bg-dark-700 text-dark-100 hover:bg-dark-600"
                : "bg-rust-500 text-white hover:bg-rust-600"
            } disabled:opacity-50`}
          >
            {maintenanceLoading ? <Spinner /> : null}
            {maintenanceEnabled === null
              ? "Enable Maintenance Mode"
              : maintenanceEnabled
              ? "Disable Maintenance Mode"
              : "Enable Maintenance Mode"}
          </button>
        </div>

        {/* Card 2: Clear Cache */}
        <div className="bg-dark-800 rounded-lg border border-dark-500 p-5">
          <div className="flex items-center gap-3 mb-3">
            <div className="p-2 bg-dark-700 rounded-lg">
              <svg className="w-5 h-5 text-dark-200" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={1.5}>
                <path strokeLinecap="round" strokeLinejoin="round" d="M12 16.5V9.75m0 0 3 3m-3-3-3 3M6.75 19.5a4.5 4.5 0 0 1-1.41-8.775 5.25 5.25 0 0 1 10.233-2.33 3 3 0 0 1 3.758 3.848A3.752 3.752 0 0 1 18 19.5H6.75Z" />
              </svg>
            </div>
            <div>
              <h3 className="text-sm font-semibold text-dark-50">Clear Cache</h3>
              <p className="text-xs text-dark-400 mt-0.5">Flush object cache, transients & plugin caches</p>
            </div>
          </div>
          {lastClearedAt && (
            <p className="text-xs text-dark-400 mb-3 font-mono">Last cleared: {timeAgo(lastClearedAt)}</p>
          )}
          <p className="text-xs text-dark-400 mb-4">
            Clears WordPress object cache, database transients, and any caching plugin caches (WP Super Cache, W3TC, etc.).
          </p>
          <button
            onClick={clearCache}
            disabled={cacheLoading}
            className="w-full px-4 py-2 bg-rust-500 text-white rounded-lg text-sm font-medium hover:bg-rust-600 disabled:opacity-50 flex items-center justify-center gap-2 transition-colors"
          >
            {cacheLoading ? <Spinner /> : null}
            Clear All Caches
          </button>
        </div>

        {/* Card 3: Optimize Database */}
        <div className="bg-dark-800 rounded-lg border border-dark-500 p-5">
          <div className="flex items-center gap-3 mb-3">
            <div className="p-2 bg-dark-700 rounded-lg">
              <svg className="w-5 h-5 text-dark-200" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={1.5}>
                <path strokeLinecap="round" strokeLinejoin="round" d="M20.25 6.375c0 2.278-3.694 4.125-8.25 4.125S3.75 8.653 3.75 6.375m16.5 0c0-2.278-3.694-4.125-8.25-4.125S3.75 4.097 3.75 6.375m16.5 0v11.25c0 2.278-3.694 4.125-8.25 4.125s-8.25-1.847-8.25-4.125V6.375m16.5 0v3.75m-16.5-3.75v3.75m16.5 0v3.75C20.25 16.153 16.556 18 12 18s-8.25-1.847-8.25-4.125v-3.75m16.5 0c0 2.278-3.694 4.125-8.25 4.125s-8.25-1.847-8.25-4.125" />
              </svg>
            </div>
            <div>
              <h3 className="text-sm font-semibold text-dark-50">Optimize Database</h3>
              <p className="text-xs text-dark-400 mt-0.5">Run OPTIMIZE TABLE on all WordPress tables</p>
            </div>
          </div>
          {optimizeResult && (
            <div className="mb-3 px-3 py-2 bg-rust-500/10 border border-rust-500/20 rounded text-xs font-mono text-dark-200">
              {optimizeResult.message ?? `${optimizeResult.tables_optimized ?? 0} table(s) optimized`}
            </div>
          )}
          <p className="text-xs text-dark-400 mb-4">
            Reclaims wasted space and defragments table data. Runs via WP-CLI against all WordPress database tables.
          </p>
          <button
            onClick={optimizeDb}
            disabled={optimizeLoading}
            className="w-full px-4 py-2 bg-rust-500 text-white rounded-lg text-sm font-medium hover:bg-rust-600 disabled:opacity-50 flex items-center justify-center gap-2 transition-colors"
          >
            {optimizeLoading ? <Spinner /> : null}
            Optimize Database Tables
          </button>
        </div>

        {/* Card 4: Search & Replace */}
        <div className="bg-dark-800 rounded-lg border border-dark-500 p-5">
          <div className="flex items-center gap-3 mb-3">
            <div className="p-2 bg-dark-700 rounded-lg">
              <svg className="w-5 h-5 text-dark-200" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={1.5}>
                <path strokeLinecap="round" strokeLinejoin="round" d="m15.75 15.75-2.489-2.489m0 0a3.375 3.375 0 1 0-4.773-4.773 3.375 3.375 0 0 0 4.774 4.774ZM21 12a9 9 0 1 1-18 0 9 9 0 0 1 18 0Z" />
              </svg>
            </div>
            <div>
              <h3 className="text-sm font-semibold text-dark-50">Search & Replace</h3>
              <p className="text-xs text-dark-400 mt-0.5">Find and replace content in the database</p>
            </div>
          </div>

          <div className="p-3 mb-3 bg-warn-500/10 border border-warn-500/20 rounded text-xs text-warn-400">
            ⚠ This modifies database content. Use dry run first to preview changes before applying them.
          </div>

          <div className="space-y-3 mb-4">
            <div>
              <label className="block text-xs text-dark-300 mb-1">Search for</label>
              <input
                type="text"
                value={srSearch}
                onChange={(e) => setSrSearch(e.target.value)}
                placeholder="e.g. http://old-domain.com"
                className="w-full bg-dark-700 border border-dark-500 text-dark-100 text-sm rounded px-3 py-2 focus:outline-none focus:ring-1 focus:ring-rust-500 focus:border-rust-500 placeholder:text-dark-500"
              />
            </div>
            <div>
              <label className="block text-xs text-dark-300 mb-1">Replace with</label>
              <input
                type="text"
                value={srReplace}
                onChange={(e) => setSrReplace(e.target.value)}
                placeholder="e.g. https://new-domain.com"
                className="w-full bg-dark-700 border border-dark-500 text-dark-100 text-sm rounded px-3 py-2 focus:outline-none focus:ring-1 focus:ring-rust-500 focus:border-rust-500 placeholder:text-dark-500"
              />
            </div>
            <label className="flex items-center gap-2 cursor-pointer select-none">
              <input
                type="checkbox"
                checked={srDryRun}
                onChange={(e) => setSrDryRun(e.target.checked)}
                className="w-4 h-4 rounded border-dark-500 bg-dark-700 text-rust-500 focus:ring-rust-500 focus:ring-offset-0"
              />
              <span className="text-xs text-dark-200">Dry Run</span>
              <span className="text-xs text-dark-400">(preview matches without making changes)</span>
            </label>
          </div>

          {srResult && (
            <div className="mb-3 px-3 py-2 bg-dark-700 rounded text-xs font-mono text-dark-200">
              {srResult.dry_run
                ? `Dry run: ${srResult.matches ?? 0} match(es) found`
                : `${srResult.replacements ?? 0} replacement(s) made`}
              {srResult.message && <span className="ml-2 text-dark-400">— {srResult.message}</span>}
            </div>
          )}

          <button
            onClick={runSearchReplace}
            disabled={srLoading || !srSearch.trim()}
            className="w-full px-4 py-2 bg-rust-500 text-white rounded-lg text-sm font-medium hover:bg-rust-600 disabled:opacity-50 flex items-center justify-center gap-2 transition-colors"
          >
            {srLoading ? <Spinner /> : null}
            {srDryRun ? "Preview (Dry Run)" : "Run Search & Replace"}
          </button>
        </div>

        {/* Card 5: Reset Admin Password */}
        <div className="bg-dark-800 rounded-lg border border-danger-500/20 p-5 md:col-span-2">
          <div className="flex items-center gap-3 mb-3">
            <div className="p-2 bg-danger-500/10 border border-danger-500/20 rounded-lg">
              <svg className="w-5 h-5 text-danger-400" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={1.5}>
                <path strokeLinecap="round" strokeLinejoin="round" d="M15.75 5.25a3 3 0 0 1 3 3m3 0a6 6 0 0 1-7.029 5.912c-.563-.097-1.159.026-1.563.43L10.5 17.25H8.25v2.25H6v2.25H2.25v-2.818c0-.597.237-1.17.659-1.591l6.499-6.499c.404-.404.527-1 .43-1.563A6 6 0 0 1 21.75 8.25Z" />
              </svg>
            </div>
            <div>
              <h3 className="text-sm font-semibold text-dark-50">Reset Admin Password</h3>
              <p className="text-xs text-dark-400 mt-0.5">Change the WordPress admin account password</p>
            </div>
          </div>

          <div className="p-3 mb-4 bg-danger-500/10 border border-danger-500/20 rounded text-xs text-danger-400">
            ⚠ Dangerous: resetting the admin password will immediately invalidate all active admin sessions. Ensure you save the new password before proceeding.
          </div>

          <div className="flex flex-col sm:flex-row gap-3 items-end">
            <div className="flex-1">
              <label className="block text-xs text-dark-300 mb-1">New Password</label>
              <div className="relative">
                <input
                  type={showPassword ? "text" : "password"}
                  value={newPassword}
                  onChange={(e) => setNewPassword(e.target.value)}
                  placeholder="Enter new admin password"
                  className="w-full bg-dark-700 border border-dark-500 text-dark-100 text-sm rounded px-3 py-2 pr-10 focus:outline-none focus:ring-1 focus:ring-rust-500 focus:border-rust-500 placeholder:text-dark-500"
                />
                <button
                  type="button"
                  onClick={() => setShowPassword((v) => !v)}
                  className="absolute right-2 top-1/2 -translate-y-1/2 text-dark-400 hover:text-dark-200 transition-colors"
                >
                  {showPassword ? (
                    <svg className="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}>
                      <path strokeLinecap="round" strokeLinejoin="round" d="M3.98 8.223A10.477 10.477 0 0 0 1.934 12C3.226 16.338 7.244 19.5 12 19.5c.993 0 1.953-.138 2.863-.395M6.228 6.228A10.451 10.451 0 0 1 12 4.5c4.756 0 8.773 3.162 10.065 7.498a10.522 10.522 0 0 1-4.293 5.774M6.228 6.228 3 3m3.228 3.228 3.65 3.65m7.894 7.894L21 21m-3.228-3.228-3.65-3.65m0 0a3 3 0 1 0-4.243-4.243m4.242 4.242L9.88 9.88" />
                    </svg>
                  ) : (
                    <svg className="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}>
                      <path strokeLinecap="round" strokeLinejoin="round" d="M2.036 12.322a1.012 1.012 0 0 1 0-.639C3.423 7.51 7.36 4.5 12 4.5c4.638 0 8.573 3.007 9.963 7.178.07.207.07.431 0 .639C20.577 16.49 16.64 19.5 12 19.5c-4.638 0-8.573-3.007-9.963-7.178Z" />
                      <path strokeLinecap="round" strokeLinejoin="round" d="M15 12a3 3 0 1 1-6 0 3 3 0 0 1 6 0Z" />
                    </svg>
                  )}
                </button>
              </div>
              {/* Strength bar */}
              {newPassword.length > 0 && (
                <div className="mt-2">
                  <div className="h-1.5 bg-dark-600 rounded-full overflow-hidden">
                    <div
                      className={`h-full rounded-full transition-all ${strength.color}`}
                      style={{ width: strength.width }}
                    />
                  </div>
                  <span className="text-[10px] font-mono text-dark-400 mt-0.5 block">
                    Strength: {strength.label}
                  </span>
                </div>
              )}
            </div>
            <button
              onClick={() => setConfirmResetPassword(true)}
              disabled={resetLoading || !newPassword || newPassword.length < 8}
              className="px-4 py-2 bg-danger-500 text-white rounded-lg text-sm font-medium hover:bg-danger-600 disabled:opacity-50 flex items-center gap-2 transition-colors whitespace-nowrap"
            >
              {resetLoading ? <Spinner /> : null}
              Reset Password
            </button>
          </div>
        </div>
      </div>

      {/* Confirm reset password */}
      {confirmResetPassword && (
        <ConfirmDialog
          title="Reset Admin Password"
          label={`Are you sure you want to reset the WordPress admin password for ${sites.find((s) => s.site_id === siteId)?.domain ?? "this site"}?`}
          consequences={[
            "The admin password will be immediately changed on the server.",
            "All active admin sessions will be invalidated — anyone currently logged in as admin will be signed out.",
            "Make sure you have saved the new password before confirming.",
          ]}
          confirmLabel="Reset Password"
          onConfirm={doResetPassword}
          onCancel={() => setConfirmResetPassword(false)}
        />
      )}
    </div>
  );
}

// ─── Main Component ───────────────────────────────────────────────────────────

export default function WordPressToolkit() {
  const { user } = useAuth();
  if (!user) return <Navigate to="/login" replace />;

  const [tab, setTab] = useState<"overview" | "plugins" | "themes" | "tools" | "security">("overview");
  const [sites, setSites] = useState<WPSite[]>([]);
  const [loading, setLoading] = useState(true);
  const [selected, setSelected] = useState<Set<string>>(new Set());
  const [updating, setUpdating] = useState(false);
  const [scanning, setScanning] = useState<string | null>(null);
  const [scanningAll, setScanningAll] = useState(false);
  const [search, setSearch] = useState("");
  const [sortBy, setSortBy] = useState<"domain" | "version" | "vulns" | "updates">("domain");
  const [sortOrder, setSortOrder] = useState<"asc" | "desc">("asc");
  const [viewMode, setViewMode] = useState<"grid" | "table">("grid");
  const [scanResults, setScanResults] = useState<Record<string, VulnScanResult>>({});
  const [securityChecks, setSecurityChecks] = useState<Record<string, SecurityItem[]>>({});
  const [hardeningHistory, setHardeningHistory] = useState<Record<string, HardeningHistoryRow[]>>({});
  const [checkingAll, setCheckingAll] = useState(false);
  const [hardening, setHardening] = useState<string | null>(null);
  const [error, setError] = useState("");
  const [success, setSuccess] = useState("");

  const handleScanAll = async () => {
    setScanningAll(true);
    try {
      const res = await api.post<{ scanned: number; total_vulns: number; critical_count: number }>("/wordpress/scan-all");
      showGlobalNotification(
        "success",
        `Scanned ${res.scanned} WordPress site${res.scanned !== 1 ? "s" : ""}. Detected ${res.total_vulns} vulnerabilities (${res.critical_count} critical).`
      );
      fetchSites();
    } catch (e) {
      showGlobalNotification("error", e instanceof Error ? e.message : "Failed to scan all WordPress sites");
    } finally {
      setScanningAll(false);
    }
  };

  // Fetch WP sites
  const fetchSites = useCallback(async () => {
    try {
      const data = await api.get<WPSite[]>("/wordpress/sites");
      setSites(data);
    } catch (e) {
      setError(e instanceof ApiError ? e.message : "Failed to load WordPress sites");
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    fetchSites();
  }, [fetchSites]);

  // Fetch hardening history for all sites
  useEffect(() => {
    let cancelled = false;
    (async () => {
      const entries = await Promise.all(
        sites.map(async (site) => {
          try {
            const rows = await api.get<HardeningHistoryRow[]>(
              `/sites/${site.site_id}/wordpress/hardening-history`
            );
            return [site.site_id, rows] as const;
          } catch {
            return [site.site_id, []] as const;
          }
        })
      );
      if (!cancelled) {
        setHardeningHistory(Object.fromEntries(entries));
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [sites]);

  // Bulk update
  const handleBulkUpdate = async (target: string) => {
    if (selected.size === 0) return;
    setUpdating(true);
    setError("");
    try {
      const res = await api.post<{ results?: BulkUpdateResult[] }>("/wordpress/bulk-update", {
        site_ids: Array.from(selected),
        target,
      });
      await fetchSites();
      setSelected(new Set());

      const results = res.results ?? [];
      const failed = results.filter((r) => !r.ok);
      const succeeded = results.length - failed.length;

      if (results.length === 0) {
        setSuccess("");
        setError("The server accepted the request but reported on no sites.");
      } else if (failed.length === 0) {
        setError("");
        setSuccess(`Updated ${succeeded} site(s) successfully`);
      } else {
        setSuccess("");
        setError(
          `Updated ${succeeded} of ${results.length} site(s). Failed: ` +
            failed
              .map((r) => `${r.domain ?? r.site_id} (${r.message || "unknown error"})`)
              .join("; ")
        );
      }
    } catch (e) {
      setError(e instanceof ApiError ? e.message : "Bulk update failed");
    } finally {
      setUpdating(false);
    }
  };

  // Vuln scan
  const handleScan = async (siteId: string) => {
    setScanning(siteId);
    setError("");
    try {
      const result = await api.post<VulnScanResult>(`/sites/${siteId}/wordpress/vuln-scan`);
      setScanResults((prev) => ({ ...prev, [siteId]: result }));
      await fetchSites();
    } catch (e) {
      setError(e instanceof ApiError ? e.message : "Vulnerability scan failed");
    } finally {
      setScanning(null);
    }
  };

  // Security check for one site
  const handleSecurityCheck = async (siteId: string) => {
    try {
      const result = await api.get<SecurityCheckResponse>(`/sites/${siteId}/wordpress/security-check`);
      const checks: SecurityItem[] = Array.isArray(result?.checks) ? result.checks : [];
      setSecurityChecks((prev) => ({ ...prev, [siteId]: checks }));
      try {
        const history = await api.get<HardeningHistoryRow[]>(
          `/sites/${siteId}/wordpress/hardening-history`
        );
        setHardeningHistory((prev) => ({ ...prev, [siteId]: history }));
      } catch {
        // Non-fatal
      }
    } catch (e) {
      setError(e instanceof ApiError ? e.message : "Security check failed");
    }
  };

  // Check all sites
  const handleCheckAll = async () => {
    setCheckingAll(true);
    for (const site of sites) {
      await handleSecurityCheck(site.site_id);
    }
    setCheckingAll(false);
  };

  // Apply hardening
  const handleHarden = async (siteId: string, fixes: string[]) => {
    setHardening(siteId);
    setError("");
    try {
      const res = await api.post<HardenResult[]>(`/sites/${siteId}/wordpress/harden`, { fixes });
      const results = Array.isArray(res) ? res : [];
      const failed = results.filter((r) => !r.applied);

      if (results.length === 0) {
        setSuccess("");
        setError("The server accepted the request but reported no fixes applied.");
      } else if (failed.length === 0) {
        setError("");
        setSuccess(`Hardening applied (${results.length} fix(es))`);
      } else {
        setSuccess("");
        setError(
          `Applied ${results.length - failed.length} of ${results.length} fix(es). Not applied: ` +
            failed.map((r) => `${r.name} (${r.message || "no reason given"})`).join("; ")
        );
      }
      await handleSecurityCheck(siteId);
    } catch (e) {
      setError(e instanceof ApiError ? e.message : "Hardening failed");
    } finally {
      setHardening(null);
    }
  };

  // Toggle selection
  const toggleSelect = (siteId: string) => {
    setSelected((prev) => {
      const next = new Set(prev);
      if (next.has(siteId)) next.delete(siteId);
      else next.add(siteId);
      return next;
    });
  };

  const toggleSelectAll = () => {
    if (selected.size === sites.length) {
      setSelected(new Set());
    } else {
      setSelected(new Set(sites.map((s) => s.site_id)));
    }
  };

  if (loading) {
    return (
      <div className="p-6 lg:p-8">
        <h1 className="text-sm font-medium text-dark-300 uppercase font-mono tracking-widest mb-6">
          WordPress Toolkit
        </h1>
        <div className="bg-dark-800 rounded-lg border border-dark-500 p-6 animate-pulse">
          <div className="h-6 bg-dark-700 rounded w-48 mb-4" />
          <div className="h-4 bg-dark-700 rounded w-32" />
        </div>
      </div>
    );
  }

  const totalVulns = sites.reduce((acc, s) => acc + s.vulns, 0);
  const criticalVulns = sites.reduce((acc, s) => acc + s.critical_vulns, 0);
  const updatesAvailable = sites.filter((s) => s.update_available).length;

  const tabs: { key: typeof tab; label: string }[] = [
    { key: "overview", label: "Overview" },
    { key: "plugins", label: "Plugins" },
    { key: "themes", label: "Themes" },
    { key: "tools", label: "Tools" },
    { key: "security", label: "Security" },
  ];

  return (
    <div className="p-6 lg:p-8">
      {/* Header */}
      <div className="flex items-center justify-between gap-3 mb-6 pb-4 border-b border-dark-600 flex-wrap">
        <div>
          <h1 className="text-sm font-medium text-dark-300 uppercase font-mono tracking-widest">
            WordPress Toolkit
          </h1>
          <p className="text-sm text-dark-200 font-mono mt-0.5">
            Manage all WordPress sites from one place
          </p>
        </div>
        <div className="flex items-center gap-3 flex-wrap text-xs">
          <button
            onClick={handleScanAll}
            disabled={scanningAll}
            className="inline-flex items-center gap-1.5 px-3 py-1.5 bg-rust-500 hover:bg-rust-600 disabled:opacity-50 text-white rounded-lg font-medium transition-colors shadow-sm"
          >
            {scanningAll ? (
              <>
                <Spinner className="w-3.5 h-3.5" />
                Scanning All Sites...
              </>
            ) : (
              <>
                <svg className="w-3.5 h-3.5" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                  <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M21 21l-6-6m2-5a7 7 0 11-14 0 7 7 0 0114 0z" />
                </svg>
                Scan All Websites
              </>
            )}
          </button>

          <input
            type="text"
            value={search}
            onChange={(e) => setSearch(e.target.value)}
            placeholder="Search WP sites..."
            className="px-3 py-1.5 bg-dark-800 border border-dark-600 rounded-lg text-xs text-dark-100 placeholder-dark-400 focus:outline-none focus:border-dark-400 w-36 sm:w-44"
          />

          <select
            value={sortBy}
            onChange={(e) => setSortBy(e.target.value as any)}
            className="px-2.5 py-1.5 bg-dark-800 border border-dark-600 rounded-lg text-xs text-dark-100 focus:outline-none font-mono"
            aria-label="Sort WordPress sites"
          >
            <option value="domain">Domain (A-Z)</option>
            <option value="version">WP Version</option>
            <option value="vulns">Vulnerabilities</option>
            <option value="updates">Updates Available</option>
          </select>

          <button
            onClick={() => setSortOrder(o => o === "asc" ? "desc" : "asc")}
            className="p-1.5 bg-dark-800 border border-dark-600 rounded-lg text-dark-200 hover:text-dark-50 text-xs font-mono"
            title={sortOrder === "asc" ? "Ascending order" : "Descending order"}
          >
            {sortOrder === "asc" ? "↑" : "↓"}
          </button>

          <div className="flex items-center bg-dark-800 border border-dark-600 rounded-lg p-0.5">
            <button
              onClick={() => setViewMode("grid")}
              className={`p-1.5 rounded ${viewMode === "grid" ? "bg-dark-700 text-dark-50" : "text-dark-400 hover:text-dark-200"}`}
              title="Grid / Card view"
            >
              <svg className="w-3.5 h-3.5" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M4 6a2 2 0 012-2h2a2 2 0 012 2v2a2 2 0 01-2 2H6a2 2 0 01-2-2V6zM14 6a2 2 0 012-2h2a2 2 0 012 2v2a2 2 0 01-2 2h-2a2 2 0 01-2-2V6zM4 16a2 2 0 012-2h2a2 2 0 012 2v2a2 2 0 01-2 2H6a2 2 0 01-2-2v-2zM14 16a2 2 0 012-2h2a2 2 0 012 2v2a2 2 0 01-2 2h-2a2 2 0 01-2-2v-2z" />
              </svg>
            </button>
            <button
              onClick={() => setViewMode("table")}
              className={`p-1.5 rounded ${viewMode === "table" ? "bg-dark-700 text-dark-50" : "text-dark-400 hover:text-dark-200"}`}
              title="Table / List view"
            >
              <svg className="w-3.5 h-3.5" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M4 6h16M4 12h16M4 18h16" />
              </svg>
            </button>
          </div>
        </div>
      </div>

      {/* Success */}
      {success && (
        <div className="mb-4 px-4 py-3 rounded-lg text-sm border bg-rust-500/10 text-rust-400 border-rust-500/20 flex items-center justify-between">
          <span>{success}</span>
          <button onClick={() => setSuccess("")} className="text-rust-400 hover:text-rust-300 ml-4">
            <svg className="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}>
              <path strokeLinecap="round" strokeLinejoin="round" d="M6 18 18 6M6 6l12 12" />
            </svg>
          </button>
        </div>
      )}
      {/* Error */}
      {error && (
        <div className="mb-4 px-4 py-3 rounded-lg text-sm border bg-danger-500/10 text-danger-400 border-danger-500/20 flex items-center justify-between">
          <span>{error}</span>
          <button onClick={() => setError("")} className="text-danger-400 hover:text-danger-300 ml-4">
            <svg className="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}>
              <path strokeLinecap="round" strokeLinejoin="round" d="M6 18 18 6M6 6l12 12" />
            </svg>
          </button>
        </div>
      )}

      {/* Tabs */}
      <div className="flex gap-1 mb-6 flex-wrap">
        {tabs.map((t) => (
          <button
            key={t.key}
            onClick={() => setTab(t.key)}
            className={`px-4 py-2 rounded-lg text-sm font-medium transition-colors ${
              tab === t.key
                ? "bg-rust-500 text-white"
                : "bg-dark-700 text-dark-200 hover:bg-dark-600"
            }`}
          >
            {t.label}
          </button>
        ))}
      </div>

      {/* No sites */}
      {sites.length === 0 && (
        <div className="bg-dark-800 rounded-lg border border-dark-500 p-12 text-center">
          <div className="w-16 h-16 bg-dark-700 rounded-2xl flex items-center justify-center mx-auto mb-4">
            <svg className="w-8 h-8 text-dark-300" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={1.5}>
              <path strokeLinecap="round" strokeLinejoin="round" d="M12 21a9.004 9.004 0 0 0 8.716-6.747M12 21a9.004 9.004 0 0 1-8.716-6.747M12 21c2.485 0 4.5-4.03 4.5-9S14.485 3 12 3m0 18c-2.485 0-4.5-4.03-4.5-9S9.515 3 12 3m0 0a8.997 8.997 0 0 1 7.843 4.582M12 3a8.997 8.997 0 0 0-7.843 4.582m15.686 0A11.953 11.953 0 0 1 12 10.5c-2.998 0-5.74-1.1-7.843-2.918m15.686 0A8.959 8.959 0 0 1 21 12c0 .778-.099 1.533-.284 2.253m0 0A17.919 17.919 0 0 1 12 16.5a17.92 17.92 0 0 1-8.716-2.247m0 0A8.966 8.966 0 0 1 3 12c0-1.264.26-2.466.732-3.558" />
            </svg>
          </div>
          <h2 className="text-lg font-semibold text-dark-50 mb-2">No WordPress Sites</h2>
          <p className="text-sm text-dark-200 mb-4">
            No WordPress installations detected on your sites. Install WordPress on a site to see it here.
          </p>
          <Link
            to="/sites"
            className="inline-flex px-4 py-2 bg-rust-500 text-white rounded-lg text-sm font-medium hover:bg-rust-600"
          >
            Go to Sites
          </Link>
        </div>
      )}

      {/* Overview Tab */}
      {tab === "overview" && sites.length > 0 && (
        <>
          {/* Summary cards */}
          <div className="grid grid-cols-1 sm:grid-cols-3 gap-4 mb-6">
            <div className="bg-dark-800 rounded-lg border border-dark-500 p-4">
              <div className="text-xs text-dark-300 uppercase font-mono tracking-widest mb-1">
                WordPress Sites
              </div>
              <div className="text-2xl font-bold text-dark-50 font-mono">{sites.length}</div>
            </div>
            <div className="bg-dark-800 rounded-lg border border-dark-500 p-4">
              <div className="text-xs text-dark-300 uppercase font-mono tracking-widest mb-1">
                Updates Available
              </div>
              <div className={`text-2xl font-bold font-mono ${updatesAvailable > 0 ? "text-warn-400" : "text-dark-50"}`}>
                {updatesAvailable}
              </div>
            </div>
            <div className="bg-dark-800 rounded-lg border border-dark-500 p-4">
              <div className="text-xs text-dark-300 uppercase font-mono tracking-widest mb-1">
                Vulnerabilities
              </div>
              <div className={`text-2xl font-bold font-mono ${criticalVulns > 0 ? "text-danger-400" : totalVulns > 0 ? "text-warn-400" : "text-dark-50"}`}>
                {totalVulns}
                {criticalVulns > 0 && (
                  <span className="text-sm text-danger-400 ml-2">({criticalVulns} critical)</span>
                )}
              </div>
            </div>
          </div>

          {/* Bulk actions */}
          {selected.size > 0 && (
            <div className="bg-dark-800 rounded-lg border border-rust-500/30 p-3 mb-4 flex items-center gap-3 flex-wrap">
              <span className="text-sm text-dark-200 font-mono">
                {selected.size} selected
              </span>
              <div className="flex items-center gap-2">
                <button
                  onClick={() => handleBulkUpdate("plugins")}
                  disabled={updating}
                  className="px-3 py-1.5 bg-rust-500 text-white rounded-lg text-xs font-medium hover:bg-rust-600 disabled:opacity-50"
                >
                  {updating ? "Updating..." : "Update Plugins"}
                </button>
                <button
                  onClick={() => handleBulkUpdate("themes")}
                  disabled={updating}
                  className="px-3 py-1.5 bg-rust-500 text-white rounded-lg text-xs font-medium hover:bg-rust-600 disabled:opacity-50"
                >
                  Update Themes
                </button>
                <button
                  onClick={() => handleBulkUpdate("core")}
                  disabled={updating}
                  className="px-3 py-1.5 bg-warn-500 text-white rounded-lg text-xs font-medium hover:bg-warn-600 disabled:opacity-50"
                >
                  Update Core
                </button>
                <button
                  onClick={() => handleBulkUpdate("all")}
                  disabled={updating}
                  className="px-3 py-1.5 bg-accent-600 text-white rounded-lg text-xs font-medium hover:bg-accent-700 disabled:opacity-50"
                >
                  Update All
                </button>
              </div>
              <button
                onClick={() => setSelected(new Set())}
                className="ml-auto text-xs text-dark-300 hover:text-dark-100"
              >
                Clear
              </button>
            </div>
          )}

          {/* Site list: Table or Grid View */}
          {(() => {
            const filteredSites = sites
              .filter((s) => s.domain.toLowerCase().includes(search.toLowerCase()) || s.wp_version.toLowerCase().includes(search.toLowerCase()))
              .sort((a, b) => {
                let cmp = 0;
                if (sortBy === "domain") cmp = a.domain.localeCompare(b.domain);
                else if (sortBy === "version") cmp = a.wp_version.localeCompare(b.wp_version);
                else if (sortBy === "vulns") cmp = a.vulns - b.vulns;
                else if (sortBy === "updates") cmp = (a.update_available ? 1 : 0) - (b.update_available ? 1 : 0);
                return sortOrder === "asc" ? cmp : -cmp;
              });

            if (viewMode === "table") {
              return (
                <div className="bg-dark-800 rounded-lg border border-dark-600 overflow-x-auto">
                  <table className="w-full text-left text-xs font-mono">
                    <thead className="bg-dark-900 border-b border-dark-600 text-dark-300">
                      <tr>
                        <th className="p-3 w-8">
                          <input
                            type="checkbox"
                            checked={selected.size === filteredSites.length && filteredSites.length > 0}
                            onChange={toggleSelectAll}
                            className="rounded border-dark-500 bg-dark-800"
                          />
                        </th>
                        <th className="p-3 font-semibold uppercase tracking-wider">Site / Domain</th>
                        <th className="p-3 font-semibold uppercase tracking-wider w-28">WordPress</th>
                        <th className="p-3 font-semibold uppercase tracking-wider w-28">Updates</th>
                        <th className="p-3 font-semibold uppercase tracking-wider w-36">Vulnerabilities</th>
                        <th className="p-3 font-semibold uppercase tracking-wider text-right w-36">Actions</th>
                      </tr>
                    </thead>
                    <tbody className="divide-y divide-dark-700/60 font-sans">
                      {filteredSites.map((site) => (
                        <tr key={site.site_id} className="hover:bg-dark-700/20 transition-colors">
                          <td className="p-3">
                            <input
                              type="checkbox"
                              checked={selected.has(site.site_id)}
                              onChange={() => toggleSelect(site.site_id)}
                              className="rounded border-dark-500 bg-dark-800"
                            />
                          </td>
                          <td className="p-3">
                            <div className="flex items-center gap-2.5">
                              <SiteFavicon domain={site.domain} />
                              <SiteThumbnail domain={site.domain} className="w-8 h-6 rounded border border-dark-600 shrink-0" />
                              <Link
                                to={`/sites/${site.site_id}/wordpress`}
                                className="font-semibold text-rust-400 hover:text-rust-300 font-mono text-sm"
                              >
                                {site.domain}
                              </Link>
                            </div>
                          </td>
                          <td className="p-3 font-mono text-dark-100">
                            v{site.wp_version}
                          </td>
                          <td className="p-3">
                            {site.update_available ? (
                              <span className="px-1.5 py-0.5 bg-warn-500/15 text-warn-400 rounded text-[11px] font-medium">
                                Update Available
                              </span>
                            ) : (
                              <span className="text-dark-300">Up-to-date</span>
                            )}
                          </td>
                          <td className="p-3 font-mono">
                            {site.vulns > 0 ? (
                              <span className={site.critical_vulns > 0 ? "text-danger-400 font-semibold" : "text-warn-400 font-semibold"}>
                                {site.vulns} vuln{site.vulns !== 1 ? "s" : ""}
                                {site.critical_vulns > 0 && ` (${site.critical_vulns} crit)`}
                              </span>
                            ) : (
                              <span className="text-dark-400">Clean</span>
                            )}
                          </td>
                          <td className="p-3 text-right">
                            <div className="flex items-center justify-end gap-1.5">
                              <Link
                                to={`/sites/${site.site_id}/wordpress`}
                                className="px-2.5 py-1 bg-rust-500/20 text-rust-400 hover:bg-rust-500/30 rounded text-xs font-medium transition-colors"
                              >
                                Manage
                              </Link>
                              <a
                                href={`https://${site.domain}`}
                                target="_blank"
                                rel="noopener noreferrer"
                                className="p-1 text-dark-400 hover:text-dark-200"
                                title="Open in new tab"
                              >
                                <svg className="w-3.5 h-3.5" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                                  <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M10 6H6a2 2 0 00-2 2v10a2 2 0 002 2h10a2 2 0 002-2v-4M14 4h6m0 0v6m0-6L10 14" />
                                </svg>
                              </a>
                            </div>
                          </td>
                        </tr>
                      ))}
                    </tbody>
                  </table>
                </div>
              );
            }

            return (
              <div className="grid grid-cols-1 md:grid-cols-2 xl:grid-cols-3 gap-4">
                {/* Select all header */}
                <div className="col-span-full flex items-center gap-2 mb-1">
                  <label className="flex items-center gap-2 cursor-pointer">
                    <input
                      type="checkbox"
                      checked={selected.size === filteredSites.length && filteredSites.length > 0}
                      onChange={toggleSelectAll}
                      className="w-4 h-4 rounded border-dark-500 bg-dark-700 text-rust-500 focus:ring-rust-500 focus:ring-offset-0"
                    />
                    <span className="text-xs text-dark-300 font-mono">Select all</span>
                  </label>
                </div>

                {filteredSites.map((site) => (
                  <div
                    key={site.site_id}
                    className={`bg-dark-800 rounded-lg border p-4 transition-colors flex flex-col justify-between ${
                      selected.has(site.site_id)
                        ? "border-rust-500/50"
                        : "border-dark-500 hover:border-dark-400"
                    }`}
                  >
                    <div>
                      <div className="flex items-start gap-3">
                        <input
                          type="checkbox"
                          checked={selected.has(site.site_id)}
                          onChange={() => toggleSelect(site.site_id)}
                          className="mt-1 w-4 h-4 rounded border-dark-500 bg-dark-700 text-rust-500 focus:ring-rust-500 focus:ring-offset-0"
                        />
                        <div className="flex-1 min-w-0">
                          <div className="flex items-center gap-2 mb-1">
                            <SiteFavicon domain={site.domain} />
                            <h3 className="text-sm font-semibold text-dark-50 truncate font-mono">
                              {site.domain}
                            </h3>
                            {site.update_available && (
                              <span className="shrink-0 px-1.5 py-0.5 bg-warn-500/15 text-warn-400 rounded text-[10px] font-medium">
                                UPDATE
                              </span>
                            )}
                          </div>
                          <div className="mb-2">
                            <SiteThumbnail domain={site.domain} className="w-full h-28 rounded border border-dark-600 object-cover" />
                          </div>
                          <div className="flex items-center gap-3 text-xs text-dark-200 font-mono mb-2">
                            <span>WP {site.wp_version}</span>
                            {site.vulns > 0 ? (
                              <span className={site.critical_vulns > 0 ? "text-danger-400" : "text-warn-400"}>
                                {site.vulns} vuln{site.vulns !== 1 ? "s" : ""}
                              </span>
                            ) : (
                              <span className="text-dark-400">No vulns</span>
                            )}
                          </div>
                        </div>
                      </div>
                    </div>

                    <div className="pt-3 border-t border-dark-700/60 flex items-center justify-between">
                      <a
                        href={`https://${site.domain}`}
                        target="_blank"
                        rel="noopener noreferrer"
                        className="text-xs text-dark-400 hover:text-dark-200 flex items-center gap-1"
                      >
                        Visit
                        <svg className="w-3 h-3" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                          <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M10 6H6a2 2 0 00-2 2v10a2 2 0 002 2h10a2 2 0 002-2v-4M14 4h6m0 0v6m0-6L10 14" />
                        </svg>
                      </a>
                      <Link
                        to={`/sites/${site.site_id}/wordpress`}
                        className="px-3 py-1.5 bg-rust-500 hover:bg-rust-600 text-white rounded text-xs font-medium transition-colors"
                      >
                        Manage Toolkit
                      </Link>
                    </div>
                  </div>
                ))}
              </div>
            );
          })()}
        </>
      )}

      {/* Plugins Tab */}
      {tab === "plugins" && sites.length > 0 && <PluginsTab sites={sites} />}

      {/* Themes Tab */}
      {tab === "themes" && sites.length > 0 && <ThemesTab sites={sites} />}

      {/* Tools Tab */}
      {tab === "tools" && sites.length > 0 && <ToolsTab sites={sites} />}

      {/* Security Tab */}
      {tab === "security" && sites.length > 0 && (
        <>
          <div className="flex items-center justify-between mb-4">
            <p className="text-sm text-dark-200">
              Run security checks against your WordPress sites and apply hardening fixes.
            </p>
            <button
              onClick={handleCheckAll}
              disabled={checkingAll}
              className="px-4 py-2 bg-rust-500 text-white rounded-lg text-sm font-medium hover:bg-rust-600 disabled:opacity-50"
            >
              {checkingAll ? (
                <span className="flex items-center gap-2">
                  <Spinner />
                  Checking...
                </span>
              ) : (
                "Check All Sites"
              )}
            </button>
          </div>

          <div className="space-y-4">
            {sites.map((site) => {
              const checks = securityChecks[site.site_id];
              const failedChecks = checks?.filter((c) => c.status === "fail") || [];
              const openChecks = checks?.filter((c) => c.status !== "pass") || [];
              const fixableIds =
                checks?.filter((c) => c.status !== "pass" && c.auto_fixable).map((c) => c.name) || [];
              const history = hardeningHistory[site.site_id] || [];
              const lastChecked = history.reduce<string | null>(
                (latest, row) => (!latest || row.checked_at > latest ? row.checked_at : latest),
                null
              );

              return (
                <div
                  key={site.site_id}
                  className="bg-dark-800 rounded-lg border border-dark-500"
                >
                  <div className="flex items-center justify-between p-4 border-b border-dark-600">
                    <div className="flex items-center gap-3">
                      <h3 className="text-sm font-semibold text-dark-50">{site.domain}</h3>
                      <span className="text-xs text-dark-300 font-mono">WP {site.wp_version}</span>
                      {checks && (
                        <span
                          className={`px-2 py-0.5 rounded text-[10px] font-medium ${
                            openChecks.length === 0
                              ? "bg-rust-500/15 text-rust-400"
                              : failedChecks.length > 0
                              ? "bg-danger-500/15 text-danger-400"
                              : "bg-warn-500/15 text-warn-400"
                          }`}
                        >
                          {openChecks.length === 0
                            ? "All Passed"
                            : `${openChecks.length} Issue${openChecks.length !== 1 ? "s" : ""}`}
                        </span>
                      )}
                      <span
                        className="text-xs text-dark-400 font-mono"
                        title={lastChecked ? new Date(lastChecked).toLocaleString() : undefined}
                      >
                        {lastChecked ? `Checked ${timeAgo(lastChecked)}` : "Never checked"}
                      </span>
                    </div>
                    <div className="flex items-center gap-2">
                      {fixableIds.length > 0 && (
                        <button
                          onClick={() => handleHarden(site.site_id, fixableIds)}
                          disabled={hardening === site.site_id}
                          className="px-3 py-1.5 bg-accent-600 text-white rounded-lg text-xs font-medium hover:bg-accent-700 disabled:opacity-50"
                        >
                          {hardening === site.site_id ? "Fixing..." : `Fix ${fixableIds.length} Issue${fixableIds.length !== 1 ? "s" : ""}`}
                        </button>
                      )}
                      <button
                        onClick={() => handleSecurityCheck(site.site_id)}
                        className="px-3 py-1.5 bg-dark-700 text-dark-100 rounded-lg text-xs font-medium hover:bg-dark-600"
                      >
                        {checks ? "Re-check" : "Check"}
                      </button>
                    </div>
                  </div>

                  {checks && checks.length > 0 && (
                    <div className="divide-y divide-dark-600">
                      {checks.map((check) => (
                        <div
                          key={check.name}
                          className="flex items-center gap-3 px-4 py-2.5 hover:bg-dark-700/30 transition-colors"
                        >
                          {/* Status icon */}
                          {check.status === "pass" ? (
                            <svg className="w-4 h-4 text-rust-400 shrink-0" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}>
                              <path strokeLinecap="round" strokeLinejoin="round" d="m4.5 12.75 6 6 9-13.5" />
                            </svg>
                          ) : check.status === "warning" ? (
                            <svg className="w-4 h-4 text-warn-400 shrink-0" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}>
                              <path strokeLinecap="round" strokeLinejoin="round" d="M12 9v3.75m-9.303 3.376c-.866 1.5.217 3.374 1.948 3.374h14.71c1.73 0 2.813-1.874 1.948-3.374L13.949 3.378c-.866-1.5-3.032-1.5-3.898 0L2.697 16.126ZM12 15.75h.007v.008H12v-.008Z" />
                            </svg>
                          ) : (
                            <svg className="w-4 h-4 text-danger-400 shrink-0" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}>
                              <path strokeLinecap="round" strokeLinejoin="round" d="M6 18 18 6M6 6l12 12" />
                            </svg>
                          )}

                          <div className="flex-1 min-w-0">
                            <div className="text-sm text-dark-100">{check.label}</div>
                            {check.description && (
                              <div className="text-xs text-dark-400 mt-0.5">{check.description}</div>
                            )}
                          </div>

                          {check.status !== "pass" && check.auto_fixable && (
                            <button
                              onClick={() => handleHarden(site.site_id, [check.name])}
                              disabled={hardening === site.site_id}
                              className="shrink-0 px-2.5 py-1 bg-accent-600 text-white rounded text-xs font-medium hover:bg-accent-700 disabled:opacity-50"
                            >
                              Fix
                            </button>
                          )}
                        </div>
                      ))}
                    </div>
                  )}

                  {checks && checks.length === 0 && (
                    <div className="px-4 py-6 text-center text-sm text-dark-300">
                      No security checks returned for this site.
                    </div>
                  )}

                  {!checks && (
                    <div className="px-4 py-6 text-center text-sm text-dark-400">
                      Click "Check" to run a security audit on this site.
                    </div>
                  )}
                </div>
              );
            })}
          </div>
        </>
      )}
    </div>
  );
}
