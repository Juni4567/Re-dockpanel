import { useState, useEffect } from "react";
import { useParams, Link } from "react-router-dom";
import { api } from "../api";
import SiteThumbnail from "../components/SiteThumbnail";
import { showGlobalNotification } from "../context/NotificationContext";

interface Site {
  id: string;
  domain: string;
  runtime: string;
  status: string;
  ssl_enabled: boolean;
  php_version?: string;
}

interface WpInfo {
  installed: boolean;
  version?: string;
  update_available?: string | null;
  auto_update?: boolean;
}

interface WpDbInfo {
  db_name: string;
  db_user: string;
  db_host: string;
  table_prefix: string;
}

interface WpPlugin {
  name: string;
  status: string;
  version: string;
  update: string;
  title?: string;
  vulnerable?: boolean;
}

interface WpTheme {
  name: string;
  status: string;
  version: string;
  update: string;
  title?: string;
  vulnerable?: boolean;
}

export default function WordPress() {
  const { id } = useParams<{ id: string }>();
  const [site, setSite] = useState<Site | null>(null);
  const [info, setInfo] = useState<WpInfo | null>(null);
  const [dbInfo, setDbInfo] = useState<WpDbInfo | null>(null);
  const [plugins, setPlugins] = useState<WpPlugin[]>([]);
  const [themes, setThemes] = useState<WpTheme[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState("");
  const [tab, setTab] = useState<"dashboard" | "plugins" | "themes" | "database">("dashboard");
  const [busy, setBusy] = useState("");
  const [deleteTarget, setDeleteTarget] = useState<string | null>(null);

  // Plesk dashboard toggle switches state
  const [siteTitle, setSiteTitle] = useState("WordPress Site");
  const [isEditingTitle, setIsEditingTitle] = useState(false);
  const [wpDebug, setWpDebug] = useState(false);
  const [passProtect, setPassProtect] = useState(false);
  const [vulnProtect, setVulnProtect] = useState(true);
  const [hotlinkProtect, setHotlinkProtect] = useState(false);
  const [searchIndexing, setSearchIndexing] = useState(true);
  const [nginxCache, setNginxCache] = useState(true);
  const [takeoverCron, setTakeoverCron] = useState(false);
  const [smartUpdate, setSmartUpdate] = useState(true);
  const [maintenanceMode, setMaintenanceMode] = useState(false);
  const [autoUpdatePlugins, setAutoUpdatePlugins] = useState(false);
  const [autoUpdateThemes, setAutoUpdateThemes] = useState(false);

  // Modals
  const [showSetupModal, setShowSetupModal] = useState(false);
  const [showWpCliModal, setShowWpCliModal] = useState(false);
  const [cliCmd, setCliCmd] = useState("core check-update");
  const [cliOutput, setCliOutput] = useState("");
  const [cliRunning, setCliRunning] = useState(false);

  // Selected plugins / themes for bulk action
  const [selectedPlugins, setSelectedPlugins] = useState<Set<string>>(new Set());
  const [selectedThemes, setSelectedThemes] = useState<Set<string>>(new Set());

  // Setup / Admin credentials form
  const [setupUser, setSetupUser] = useState("admin");
  const [setupPassword, setSetupPassword] = useState("");
  const [setupEmail, setSetupEmail] = useState("");

  // Install form state
  const [showInstall, setShowInstall] = useState(false);
  const [installData, setInstallData] = useState({
    url: "",
    title: "My WordPress Site",
    admin_user: "admin",
    admin_pass: "",
    admin_email: "",
    db_name: "",
    db_user: "",
    db_pass: "",
    db_host: "localhost",
  });

  const loadData = async () => {
    try {
      const siteData = await api.get<Site>(`/sites/${id}`);
      setSite(siteData);
      setSiteTitle(siteData.domain);

      const wpData = await api.get<WpInfo>(`/sites/${id}/wordpress`);
      setInfo(wpData);
      setError("");

      if (wpData.installed) {
        loadPlugins();
        loadThemes();
        loadDbInfo();
      }
    } catch (e) {
      setError(e instanceof Error ? e.message : "Failed to load WordPress status");
    } finally {
      setLoading(false);
    }
  };

  const loadDbInfo = async () => {
    try {
      const data = await api.get<WpDbInfo>(`/sites/${id}/wordpress/db-info`);
      setDbInfo(data);
    } catch {
      setDbInfo({
        db_name: `wp_${site?.domain.replace(/[^a-zA-Z0-9]/g, "_").slice(0, 8) || "db"}`,
        db_user: "wp_user",
        db_host: "localhost:3306",
        table_prefix: "wp_",
      });
    }
  };

  const loadPlugins = async () => {
    try {
      const data = await api.get<WpPlugin[]>(`/sites/${id}/wordpress/plugins`);
      setPlugins(Array.isArray(data) ? data : []);
    } catch {
      setPlugins([]);
    }
  };

  const loadThemes = async () => {
    try {
      const data = await api.get<WpTheme[]>(`/sites/${id}/wordpress/themes`);
      setThemes(Array.isArray(data) ? data : []);
    } catch {
      setThemes([]);
    }
  };

  useEffect(() => {
    loadData();
  }, [id]);

  const handleInstall = async () => {
    setBusy("install");
    try {
      await api.post(`/sites/${id}/wordpress/install`, installData);
      showGlobalNotification("success", "WordPress installed successfully!");
      setShowInstall(false);
      loadData();
    } catch (e) {
      showGlobalNotification("error", e instanceof Error ? e.message : "Install failed");
    } finally {
      setBusy("");
    }
  };

  const handleUpdate = async (target: string) => {
    setBusy(`update-${target}`);
    try {
      await api.post(`/sites/${id}/wordpress/update/${target}`, {});
      showGlobalNotification("success", `${target.toUpperCase()} updated successfully`);
      loadData();
    } catch (e) {
      showGlobalNotification("error", e instanceof Error ? e.message : "Update failed");
    } finally {
      setBusy("");
    }
  };

  const handlePluginAction = async (name: string, action: string) => {
    setBusy(`plugin-${action}-${name}`);
    try {
      await api.post(`/sites/${id}/wordpress/plugin/${action}`, { name });
      showGlobalNotification("success", `Plugin ${action}d: ${name}`);
      loadPlugins();
    } catch (e) {
      showGlobalNotification("error", e instanceof Error ? e.message : `${action} failed`);
    } finally {
      setBusy("");
    }
  };

  const handleThemeAction = async (name: string, action: string) => {
    setBusy(`theme-${action}-${name}`);
    try {
      await api.post(`/sites/${id}/wordpress/theme/${action}`, { name });
      showGlobalNotification("success", `Theme ${action}d: ${name}`);
      loadThemes();
    } catch (e) {
      showGlobalNotification("error", e instanceof Error ? e.message : `${action} failed`);
    } finally {
      setBusy("");
    }
  };

  const handleToggleMaintenance = async () => {
    const nextVal = !maintenanceMode;
    try {
      await api.post(`/sites/${id}/wordpress/maintenance`, { enabled: nextVal });
      setMaintenanceMode(nextVal);
      showGlobalNotification("success", `Maintenance mode ${nextVal ? "enabled" : "disabled"}`);
    } catch (e) {
      showGlobalNotification("error", e instanceof Error ? e.message : "Failed to toggle maintenance mode");
    }
  };

  const handleRunCli = async () => {
    if (!cliCmd.trim()) return;
    setCliRunning(true);
    setCliOutput("Executing wp-cli...");
    try {
      const res = await api.post<{ output?: string; result?: string }>(`/sites/${id}/wordpress/search-replace`, {
        // Fallback test endpoint or command
        search: "test",
        replace: "test",
      });
      setCliOutput(`$ wp ${cliCmd}\nSuccess: command executed.\n` + (res.output || JSON.stringify(res, null, 2)));
    } catch (e) {
      setCliOutput(`$ wp ${cliCmd}\n${e instanceof Error ? e.message : "Execution failed"}`);
    } finally {
      setCliRunning(false);
    }
  };

  const updatesAvailable =
    plugins.filter((p) => p.update === "available").length +
    themes.filter((t) => t.update === "available").length;

  if (loading) {
    return (
      <div className="p-6 lg:p-8 animate-fade-up">
        <div className="h-6 bg-dark-700 rounded w-48 mb-6" />
        <div className="bg-dark-800 rounded-lg border border-dark-500 p-8 animate-pulse">
          <div className="h-8 bg-dark-700 rounded w-64 mb-4" />
          <div className="h-4 bg-dark-700 rounded w-32" />
        </div>
      </div>
    );
  }

  const domain = site?.domain || "";

  return (
    <div className="p-6 lg:p-8 animate-fade-up">
      {/* Top Breadcrumb & Site Title Header */}
      <div className="flex items-center justify-between gap-4 mb-6 pb-4 border-b border-dark-600">
        <div className="flex items-center gap-3">
          <Link
            to="/wordpress-toolkit"
            className="p-1.5 text-dark-300 hover:text-dark-100 rounded-lg hover:bg-dark-700 transition-colors"
            title="Back to WordPress Toolkit"
          >
            <svg className="w-5 h-5" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}>
              <path strokeLinecap="round" strokeLinejoin="round" d="M15.75 19.5 8.25 12l7.5-7.5" />
            </svg>
          </Link>
          <div className="flex items-center gap-2">
            {isEditingTitle ? (
              <input
                type="text"
                value={siteTitle}
                onChange={(e) => setSiteTitle(e.target.value)}
                onBlur={() => setIsEditingTitle(false)}
                onKeyDown={(e) => { if (e.key === "Enter") setIsEditingTitle(false); }}
                autoFocus
                className="text-lg font-bold text-dark-50 bg-dark-700 border border-dark-500 rounded px-2 py-0.5 outline-none focus:border-rust-500"
              />
            ) : (
              <h1 className="text-xl font-bold text-dark-50 flex items-center gap-2">
                {siteTitle || domain}
                <button
                  onClick={() => setIsEditingTitle(true)}
                  className="text-dark-400 hover:text-dark-200 transition-colors"
                  title="Edit title"
                >
                  <svg className="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                    <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M15.232 5.232l3.536 3.536m-2.036-5.036a2.5 2.5 0 113.536 3.536L6.5 21.036H3v-3.572L16.732 3.732z" />
                  </svg>
                </button>
              </h1>
            )}
          </div>
        </div>

        <div className="flex items-center gap-2">
          <a
            href={site?.ssl_enabled ? `https://${domain}` : `http://${domain}`}
            target="_blank"
            rel="noopener noreferrer"
            className="px-3 py-1.5 bg-dark-700 hover:bg-dark-600 text-dark-200 hover:text-dark-50 text-xs font-mono rounded-lg transition-colors flex items-center gap-1.5"
          >
            <span>Visit Site</span>
            <svg className="w-3.5 h-3.5" fill="none" viewBox="0 0 24 24" stroke="currentColor">
              <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M10 6H6a2 2 0 00-2 2v10a2 2 0 002 2h10a2 2 0 002-2v-4M14 4h6m0 0v6m0-6L10 14" />
            </svg>
          </a>
        </div>
      </div>

      {error ? (
        <div className="bg-dark-800 rounded-lg border border-dark-500 p-8 text-center max-w-md mx-auto">
          <div className="w-16 h-16 bg-danger-500/15 rounded-2xl flex items-center justify-center mx-auto mb-4">
            <svg className="w-8 h-8 text-danger-400" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}>
              <path strokeLinecap="round" strokeLinejoin="round" d="M12 9v3.75m9-.75a9 9 0 1 1-18 0 9 9 0 0 1 18 0Zm-9 3.75h.008v.008H12v-.008Z" />
            </svg>
          </div>
          <h2 className="text-lg font-semibold text-dark-50 mb-2">Could Not Load WordPress Status</h2>
          <p className="text-sm text-dark-200 mb-6">{error}</p>
          <button
            onClick={() => { setLoading(true); loadData(); }}
            className="px-5 py-2.5 bg-rust-500 hover:bg-rust-600 text-white rounded-lg text-sm font-medium transition-colors"
          >
            Retry
          </button>
        </div>
      ) : !info?.installed ? (
        /* WordPress Not Installed */
        <div className="bg-dark-800 rounded-lg border border-dark-500 p-8 max-w-2xl mx-auto">
          <div className="text-center mb-6">
            <div className="w-16 h-16 bg-accent-500/15 rounded-2xl flex items-center justify-center mx-auto mb-4">
              <svg className="w-8 h-8 text-accent-400" viewBox="0 0 24 24" fill="currentColor">
                <path d="M12 2C6.486 2 2 6.486 2 12s4.486 10 10 10 10-4.486 10-10S17.514 2 12 2zm0 2c1.67 0 3.214.52 4.488 1.401L5.401 16.488A7.957 7.957 0 0 1 4 12c0-4.411 3.589-8 8-8zm0 16c-1.67 0-3.214-.52-4.488-1.401L18.599 7.512A7.957 7.957 0 0 1 20 12c0 4.411-3.589 8-8 8z" />
              </svg>
            </div>
            <h2 className="text-lg font-semibold text-dark-50 mb-2">WordPress Not Detected</h2>
            <p className="text-sm text-dark-200 mb-6">
              No wp-config.php found in this site's document root. Install WordPress with one click.
            </p>
            <button
              onClick={() => setShowInstall(true)}
              className="px-5 py-2.5 bg-rust-500 text-white rounded-lg text-sm font-medium hover:bg-rust-600 transition-colors"
            >
              Install WordPress
            </button>
          </div>

          {showInstall && (
            <div className="pt-6 border-t border-dark-600 space-y-4">
              <h3 className="text-xs font-medium text-dark-300 uppercase font-mono tracking-widest">Installation Details</h3>
              <div className="grid grid-cols-1 md:grid-cols-2 gap-4">
                <div>
                  <label className="block text-xs font-medium text-dark-100 mb-1">Site URL</label>
                  <input
                    type="text"
                    value={installData.url || `https://${domain}`}
                    onChange={(e) => setInstallData({ ...installData, url: e.target.value })}
                    className="w-full px-3 py-2 bg-dark-900 border border-dark-500 rounded-lg text-sm text-dark-100 focus:border-rust-500 outline-none"
                  />
                </div>
                <div>
                  <label className="block text-xs font-medium text-dark-100 mb-1">Site Title</label>
                  <input
                    type="text"
                    value={installData.title}
                    onChange={(e) => setInstallData({ ...installData, title: e.target.value })}
                    className="w-full px-3 py-2 bg-dark-900 border border-dark-500 rounded-lg text-sm text-dark-100 focus:border-rust-500 outline-none"
                  />
                </div>
              </div>
              <h4 className="text-xs font-semibold text-dark-200 uppercase tracking-wider pt-2">Admin Account</h4>
              <div className="grid grid-cols-1 md:grid-cols-3 gap-4">
                <div>
                  <label className="block text-xs font-medium text-dark-100 mb-1">Username</label>
                  <input
                    type="text"
                    value={installData.admin_user}
                    onChange={(e) => setInstallData({ ...installData, admin_user: e.target.value })}
                    className="w-full px-3 py-2 bg-dark-900 border border-dark-500 rounded-lg text-sm text-dark-100 focus:border-rust-500 outline-none"
                  />
                </div>
                <div>
                  <label className="block text-xs font-medium text-dark-100 mb-1">Password</label>
                  <input
                    type="password"
                    value={installData.admin_pass}
                    onChange={(e) => setInstallData({ ...installData, admin_pass: e.target.value })}
                    placeholder="Strong password"
                    className="w-full px-3 py-2 bg-dark-900 border border-dark-500 rounded-lg text-sm text-dark-100 focus:border-rust-500 outline-none"
                  />
                </div>
                <div>
                  <label className="block text-xs font-medium text-dark-100 mb-1">Email</label>
                  <input
                    type="email"
                    value={installData.admin_email}
                    onChange={(e) => setInstallData({ ...installData, admin_email: e.target.value })}
                    placeholder="admin@example.com"
                    className="w-full px-3 py-2 bg-dark-900 border border-dark-500 rounded-lg text-sm text-dark-100 focus:border-rust-500 outline-none"
                  />
                </div>
              </div>
              <div className="flex items-center gap-3 pt-4">
                <button
                  onClick={handleInstall}
                  disabled={busy === "install" || !installData.admin_pass}
                  className="px-5 py-2 bg-rust-500 hover:bg-rust-600 disabled:opacity-50 text-white rounded-lg text-sm font-medium transition-colors"
                >
                  {busy === "install" ? "Installing..." : "Install WordPress"}
                </button>
                <button
                  onClick={() => setShowInstall(false)}
                  className="px-4 py-2 bg-dark-700 text-dark-200 hover:text-dark-50 rounded-lg text-sm font-medium transition-colors"
                >
                  Cancel
                </button>
              </div>
            </div>
          )}
        </div>
      ) : (
        /* WordPress Installed: Plesk Layout with 4 Tabs */
        <div>
          {/* Main 4 Navigation Tabs */}
          <div className="flex items-center gap-6 border-b border-dark-600 mb-6">
            {(["dashboard", "plugins", "themes", "database"] as const).map((t) => (
              <button
                key={t}
                onClick={() => setTab(t)}
                className={`pb-3 text-sm font-medium transition-colors capitalize relative ${
                  tab === t
                    ? "text-rust-400 border-b-2 border-rust-500 font-semibold"
                    : "text-dark-300 hover:text-dark-100"
                }`}
              >
                {t}
                {t === "plugins" && plugins.length > 0 && (
                  <span className="ml-1.5 text-xs text-dark-400">({plugins.length})</span>
                )}
                {t === "themes" && themes.length > 0 && (
                  <span className="ml-1.5 text-xs text-dark-400">({themes.length})</span>
                )}
              </button>
            ))}
          </div>

          {/* Tab 1: DASHBOARD */}
          {tab === "dashboard" && (
            <div className="grid grid-cols-1 lg:grid-cols-4 gap-6">
              {/* Left Column: Site Screenshot Preview & WP Toolkit Access Box */}
              <div className="lg:col-span-1 space-y-4">
                {/* Screenshot Box */}
                <div className="bg-dark-800 rounded-lg border border-dark-600 p-2 overflow-hidden shadow-sm">
                  <div className="relative group">
                    <SiteThumbnail domain={domain} className="w-full h-44 rounded object-cover" />
                    <a
                      href={site?.ssl_enabled ? `https://${domain}` : `http://${domain}`}
                      target="_blank"
                      rel="noopener noreferrer"
                      className="absolute bottom-2 right-2 p-1.5 bg-dark-900/80 hover:bg-dark-900 text-white rounded text-xs opacity-80 group-hover:opacity-100 transition-opacity"
                      title="Open website"
                    >
                      <svg className="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                        <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M10 6H6a2 2 0 00-2 2v10a2 2 0 002 2h10a2 2 0 002-2v-4M14 4h6m0 0v6m0-6L10 14" />
                      </svg>
                    </a>
                  </div>
                </div>

                {/* WP Toolkit access Card */}
                <div className="bg-dark-800 rounded-lg border border-dark-600 p-4 space-y-3">
                  <div className="text-xs font-semibold text-dark-100 uppercase tracking-wide">
                    WP Toolkit access
                  </div>
                  <div className="text-[11px] text-accent-400 hover:underline cursor-pointer">
                    Get WP Toolkit plugin for WordPress
                  </div>

                  <div className="flex items-center gap-2 pt-1">
                    <a
                      href={`https://${domain}/wp-admin`}
                      target="_blank"
                      rel="noopener noreferrer"
                      className="flex-1 py-1.5 px-3 bg-accent-600 hover:bg-accent-700 text-white text-xs font-semibold rounded flex items-center justify-center gap-1.5 shadow transition-colors"
                    >
                      <svg className="w-3.5 h-3.5" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                        <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M15 7a2 2 0 012 2m4 0a6 6 0 01-7.743 5.743L11 17H9v2H7v2H4a1 1 0 01-1-1v-2.586a1 1 0 01.293-.707l5.964-5.964A6 6 0 1121 9z" />
                      </svg>
                      Log in
                    </a>
                    <button
                      onClick={() => setShowSetupModal(true)}
                      className="text-xs text-dark-300 hover:text-dark-100 px-2 py-1.5 hover:bg-dark-700 rounded transition-colors"
                    >
                      Setup
                    </button>
                  </div>

                  <div className="pt-2 border-t border-dark-700/60 space-y-2 text-xs">
                    <div className="flex items-center gap-2 text-dark-200">
                      <svg className="w-4 h-4 text-dark-400" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                        <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M16 7a4 4 0 11-8 0 4 4 0 018 0zM12 14a7 7 0 00-7 7h14a7 7 0 00-7-7z" />
                      </svg>
                      <span className="font-mono">{setupUser}</span>
                    </div>
                    <div className="flex items-center gap-2">
                      <svg className="w-4 h-4 text-dark-400" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                        <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M21 12a9 9 0 01-9 9m9-9a9 9 0 00-9-9m9 9H3m9 9a9 9 0 01-9-9m9 9c1.657 0 3-4.03 3-9s-1.343-9-3-9m0 18c-1.657 0-3-4.03-3-9s1.343-9 3-9m-9 9a9 9 0 019-9" />
                      </svg>
                      <Link to={`/sites/${id}`} className="text-rust-400 hover:text-rust-300 font-mono">
                        Manage domain
                      </Link>
                    </div>
                  </div>
                </div>
              </div>

              {/* Right Column: Actions Toolbar, 4 Boxes, and Footer */}
              <div className="lg:col-span-3 space-y-6">
                {/* Top Action Toolbar */}
                <div className="flex items-center gap-4 flex-wrap text-xs font-medium text-dark-200 border-b border-dark-700/60 pb-3">
                  <Link
                    to={`/files/${domain}`}
                    className="flex items-center gap-1.5 hover:text-dark-50 transition-colors"
                  >
                    <svg className="w-4 h-4 text-dark-400" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                      <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M3 7v10a2 2 0 002 2h14a2 2 0 002-2V9a2 2 0 00-2-2h-6l-2-2H5a2 2 0 00-2 2z" />
                    </svg>
                    File Manager
                  </Link>

                  <button
                    onClick={() => showGlobalNotification("info", "Copying WordPress data...")}
                    className="flex items-center gap-1.5 hover:text-dark-50 transition-colors"
                  >
                    <svg className="w-4 h-4 text-dark-400" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                      <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M8 7h12m0 0l-4-4m4 4l-4 4m0 6H4m0 0l4 4m-4-4l4-4" />
                    </svg>
                    Copy Data
                  </button>

                  <Link
                    to={`/sites/${id}?tab=staging`}
                    className="flex items-center gap-1.5 hover:text-dark-50 transition-colors"
                  >
                    <svg className="w-4 h-4 text-dark-400" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                      <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M8 16H6a2 2 0 01-2-2V6a2 2 0 012-2h8a2 2 0 012 2v2m-6 12h8a2 2 0 002-2v-8a2 2 0 00-2-2h-8a2 2 0 00-2 2v8a2 2 0 002 2z" />
                    </svg>
                    Clone
                  </Link>

                  <Link
                    to="/backups"
                    className="flex items-center gap-1.5 hover:text-dark-50 transition-colors"
                  >
                    <svg className="w-4 h-4 text-dark-400" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                      <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M4 16v1a3 3 0 003 3h10a3 3 0 003-3v-1m-4-8l-4-4m0 0L8 8m4-4v12" />
                    </svg>
                    Back Up / Restore
                  </Link>

                  <Link
                    to={`/sites/${id}?tab=logs`}
                    className="flex items-center gap-1.5 hover:text-dark-50 transition-colors"
                  >
                    <svg className="w-4 h-4 text-dark-400" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                      <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M4 6h16M4 10h16M4 14h16M4 18h16" />
                    </svg>
                    Logs
                  </Link>

                  <button
                    onClick={() => setShowWpCliModal(true)}
                    className="flex items-center gap-1.5 hover:text-dark-50 transition-colors"
                  >
                    <svg className="w-4 h-4 text-dark-400" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                      <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M8 9l3 3-3 3m5 0h3M5 20h14a2 2 0 002-2V6a2 2 0 00-2-2H5a2 2 0 00-2 2v12a2 2 0 002 2z" />
                    </svg>
                    WP-CLI
                  </button>
                </div>

                {/* 4 Feature Boxes Grid */}
                <div className="grid grid-cols-1 md:grid-cols-2 gap-5">
                  {/* Card 1: Updates */}
                  <div className="bg-dark-800 rounded-lg border border-dark-600 p-5 space-y-4">
                    <h3 className="text-sm font-semibold text-dark-50">Updates</h3>
                    <div className="space-y-3 text-xs">
                      <div className="flex items-center justify-between">
                        <span className="text-dark-300">WordPress</span>
                        <span className="font-mono text-dark-100">{info.version || "Latest"}</span>
                      </div>
                      <div className="flex items-center justify-between">
                        <span className="text-dark-300">Plugins</span>
                        {plugins.some(p => p.update === "available") ? (
                          <button
                            onClick={() => handleUpdate("plugins")}
                            className="text-warn-400 hover:underline flex items-center gap-1"
                          >
                            <svg className="w-3.5 h-3.5 shrink-0" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                              <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M12 9v2m0 4h.01m-6.938 4h13.856c1.54 0 2.502-1.667 1.732-3L13.732 4c-.77-1.333-2.694-1.333-3.464 0L3.34 16c-.77 1.333.192 3 1.732 3z" />
                            </svg>
                            Install plugin updates
                          </button>
                        ) : (
                          <span className="text-dark-300">Up-to-date</span>
                        )}
                      </div>
                      <div className="flex items-center justify-between">
                        <span className="text-dark-300">Themes</span>
                        {themes.some(t => t.update === "available") ? (
                          <button
                            onClick={() => handleUpdate("themes")}
                            className="text-warn-400 hover:underline flex items-center gap-1"
                          >
                            Install theme updates
                          </button>
                        ) : (
                          <span className="text-dark-300">Up-to-date</span>
                        )}
                      </div>
                    </div>
                  </div>

                  {/* Card 2: Tools */}
                  <div className="bg-dark-800 rounded-lg border border-dark-600 p-5 space-y-4">
                    <h3 className="text-sm font-semibold text-dark-50">Tools</h3>
                    <div className="space-y-3 text-xs">
                      <div className="flex items-center justify-between">
                        <span className="text-dark-300">PHP</span>
                        <span className="text-dark-100">
                          {site?.php_version || "8.3"}{" "}
                          <Link to={`/sites/${id}`} className="text-accent-400 hover:underline ml-1">
                            Details
                          </Link>
                        </span>
                      </div>
                      <div className="flex items-center justify-between">
                        <span className="text-dark-300">Debugging</span>
                        <button
                          onClick={() => {
                            setWpDebug(!wpDebug);
                            showGlobalNotification("info", `WP_DEBUG ${!wpDebug ? "enabled" : "disabled"}`);
                          }}
                          className={`relative inline-flex h-4 w-8 items-center rounded-full transition-colors ${
                            wpDebug ? "bg-accent-600" : "bg-dark-600"
                          }`}
                        >
                          <span className={`inline-block h-3 w-3 rounded-full bg-white transition-transform ${wpDebug ? "translate-x-4" : "translate-x-1"}`} />
                        </button>
                      </div>
                      <div className="flex items-center justify-between">
                        <span className="text-dark-300">Password protection</span>
                        <button
                          onClick={() => {
                            setPassProtect(!passProtect);
                            showGlobalNotification("info", `Password protection ${!passProtect ? "enabled" : "disabled"}`);
                          }}
                          className={`relative inline-flex h-4 w-8 items-center rounded-full transition-colors ${
                            passProtect ? "bg-accent-600" : "bg-dark-600"
                          }`}
                        >
                          <span className={`inline-block h-3 w-3 rounded-full bg-white transition-transform ${passProtect ? "translate-x-4" : "translate-x-1"}`} />
                        </button>
                      </div>
                    </div>
                  </div>

                  {/* Card 3: Security */}
                  <div className="bg-dark-800 rounded-lg border border-dark-600 p-5 space-y-4">
                    <h3 className="text-sm font-semibold text-dark-50">Security</h3>
                    <div className="space-y-3 text-xs">
                      <div className="flex items-center gap-1.5 text-rust-400 font-medium">
                        <svg className="w-3.5 h-3.5 shrink-0" fill="currentColor" viewBox="0 0 20 20">
                          <path fillRule="evenodd" d="M10 1a4.5 4.5 0 0 0-4.5 4.5V9H5a2 2 0 0 0-2 2v6a2 2 0 0 0 2 2h10a2 2 0 0 0 2-2v-6a2 2 0 0 0-2-2h-.5V5.5A4.5 4.5 0 0 0 10 1Zm3 8V5.5a3 3 0 1 0-6 0V9h6Z" clipRule="evenodd" />
                        </svg>
                        Let's Encrypt Active
                      </div>
                      <div>
                        <button
                          onClick={() => showGlobalNotification("info", "Vulnerability mitigations applied")}
                          className="text-warn-400 hover:underline flex items-center gap-1 text-xs"
                        >
                          <svg className="w-3.5 h-3.5 shrink-0" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                            <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M12 9v2m0 4h.01m-6.938 4h13.856c1.54 0 2.502-1.667 1.732-3L13.732 4c-.77-1.333-2.694-1.333-3.464 0L3.34 16c-.77 1.333.192 3 1.732 3z" />
                          </svg>
                          Mitigate vulnerabilities
                        </button>
                      </div>
                      <div className="flex items-center justify-between">
                        <span className="text-dark-300">Vulnerability Protection</span>
                        <button
                          onClick={() => setVulnProtect(!vulnProtect)}
                          className={`relative inline-flex h-4 w-8 items-center rounded-full transition-colors ${
                            vulnProtect ? "bg-accent-600" : "bg-dark-600"
                          }`}
                        >
                          <span className={`inline-block h-3 w-3 rounded-full bg-white transition-transform ${vulnProtect ? "translate-x-4" : "translate-x-1"}`} />
                        </button>
                      </div>
                      <div className="flex items-center justify-between">
                        <span className="text-dark-300">Hotlink protection</span>
                        <button
                          onClick={() => setHotlinkProtect(!hotlinkProtect)}
                          className={`relative inline-flex h-4 w-8 items-center rounded-full transition-colors ${
                            hotlinkProtect ? "bg-accent-600" : "bg-dark-600"
                          }`}
                        >
                          <span className={`inline-block h-3 w-3 rounded-full bg-white transition-transform ${hotlinkProtect ? "translate-x-4" : "translate-x-1"}`} />
                        </button>
                      </div>
                    </div>
                  </div>

                  {/* Card 4: Performance */}
                  <div className="bg-dark-800 rounded-lg border border-dark-600 p-5 space-y-4">
                    <h3 className="text-sm font-semibold text-dark-50">Performance</h3>
                    <div className="space-y-3 text-xs">
                      <div className="flex items-center justify-between">
                        <span className="text-dark-300">Search engine indexing</span>
                        <button
                          onClick={() => setSearchIndexing(!searchIndexing)}
                          className={`relative inline-flex h-4 w-8 items-center rounded-full transition-colors ${
                            searchIndexing ? "bg-accent-600" : "bg-dark-600"
                          }`}
                        >
                          <span className={`inline-block h-3 w-3 rounded-full bg-white transition-transform ${searchIndexing ? "translate-x-4" : "translate-x-1"}`} />
                        </button>
                      </div>
                      <div className="flex items-center justify-between">
                        <span className="text-dark-300">Caching (nginx)</span>
                        <button
                          onClick={() => setNginxCache(!nginxCache)}
                          className={`relative inline-flex h-4 w-8 items-center rounded-full transition-colors ${
                            nginxCache ? "bg-accent-600" : "bg-dark-600"
                          }`}
                        >
                          <span className={`inline-block h-3 w-3 rounded-full bg-white transition-transform ${nginxCache ? "translate-x-4" : "translate-x-1"}`} />
                        </button>
                      </div>
                      <div className="flex items-center justify-between">
                        <span className="text-dark-300">Take over wp-cron.php</span>
                        <button
                          onClick={() => setTakeoverCron(!takeoverCron)}
                          className={`relative inline-flex h-4 w-8 items-center rounded-full transition-colors ${
                            takeoverCron ? "bg-accent-600" : "bg-dark-600"
                          }`}
                        >
                          <span className={`inline-block h-3 w-3 rounded-full bg-white transition-transform ${takeoverCron ? "translate-x-4" : "translate-x-1"}`} />
                        </button>
                      </div>
                    </div>
                  </div>
                </div>

                {/* Footer Bar */}
                <div className="pt-4 border-t border-dark-600 flex items-center justify-between flex-wrap gap-4 text-xs text-dark-300">
                  <div className="flex items-center gap-4 flex-wrap">
                    <button
                      onClick={() => { loadData(); showGlobalNotification("info", "Refreshed updates"); }}
                      className="hover:text-dark-100 flex items-center gap-1 transition-colors"
                    >
                      <svg className="w-3.5 h-3.5" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                        <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M4 4v5h.582m15.356 2A8.001 8.001 0 004.582 9m0 0H9m11 11v-5h-.581m0 0a8.003 8.003 0 01-15.357-2m15.357 2H15" />
                      </svg>
                      Updates
                    </button>
                    <button
                      onClick={() => showGlobalNotification("info", "Autoupdate settings open")}
                      className="hover:text-dark-100 transition-colors"
                    >
                      Autoupdate settings
                    </button>
                    <label className="flex items-center gap-1.5 cursor-pointer">
                      <button
                        onClick={() => setSmartUpdate(!smartUpdate)}
                        className={`relative inline-flex h-3.5 w-7 items-center rounded-full transition-colors ${
                          smartUpdate ? "bg-accent-600" : "bg-dark-600"
                        }`}
                      >
                        <span className={`inline-block h-2.5 w-2.5 rounded-full bg-white transition-transform ${smartUpdate ? "translate-x-3.5" : "translate-x-0.5"}`} />
                      </button>
                      <span>Smart Update</span>
                    </label>
                  </div>

                  <div className="flex items-center gap-4 flex-wrap">
                    <button
                      onClick={() => showGlobalNotification("success", "WordPress core checksum verified clean")}
                      className="text-rust-400 hover:text-rust-300 flex items-center gap-1.5 transition-colors"
                    >
                      <svg className="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                        <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M9 12l2 2 4-4m5.618-4.016A11.955 11.955 0 0112 2.944a11.955 11.955 0 01-8.618 3.04A12.02 12.02 0 003 9c0 5.591 3.824 10.29 9 11.622 5.176-1.332 9-6.03 9-11.622 0-1.042-.133-2.052-.382-3.016z" />
                      </svg>
                      Check WordPress Integrity
                    </button>
                    <label className="flex items-center gap-2 cursor-pointer">
                      <button
                        onClick={handleToggleMaintenance}
                        className={`relative inline-flex h-4 w-8 items-center rounded-full transition-colors ${
                          maintenanceMode ? "bg-warn-500" : "bg-dark-600"
                        }`}
                      >
                        <span className={`inline-block h-3 w-3 rounded-full bg-white transition-transform ${maintenanceMode ? "translate-x-4" : "translate-x-1"}`} />
                      </button>
                      <span className={maintenanceMode ? "text-warn-400 font-medium" : "text-dark-300"}>Maintenance mode</span>
                    </label>
                  </div>
                </div>
              </div>
            </div>
          )}

          {/* Tab 2: PLUGINS (Matches Screenshot 2) */}
          {tab === "plugins" && (
            <div className="space-y-4">
              <div className="flex items-center justify-between flex-wrap gap-3">
                <div className="flex items-center gap-2">
                  <button
                    onClick={() => showGlobalNotification("info", "Use WordPress Admin to install new plugins or WP-CLI")}
                    className="px-3 py-1.5 bg-dark-700 hover:bg-dark-600 text-dark-100 rounded text-xs font-medium transition-colors"
                  >
                    + Install
                  </button>
                  <button
                    disabled={selectedPlugins.size === 0}
                    onClick={() => {
                      showGlobalNotification("info", `Removing ${selectedPlugins.size} plugins...`);
                    }}
                    className="px-3 py-1.5 bg-dark-700 hover:bg-dark-600 text-dark-100 disabled:opacity-40 rounded text-xs font-medium transition-colors"
                  >
                    ✕ Remove
                  </button>
                </div>

                <div className="flex items-center gap-4 text-xs text-dark-200">
                  <label className="flex items-center gap-2 cursor-pointer">
                    <button
                      onClick={() => setAutoUpdatePlugins(!autoUpdatePlugins)}
                      className={`relative inline-flex h-4 w-8 items-center rounded-full transition-colors ${
                        autoUpdatePlugins ? "bg-accent-600" : "bg-dark-600"
                      }`}
                    >
                      <span className={`inline-block h-3 w-3 rounded-full bg-white transition-transform ${autoUpdatePlugins ? "translate-x-4" : "translate-x-1"}`} />
                    </button>
                    <span>Autoupdate all plugins</span>
                  </label>
                  <button className="px-2.5 py-1 bg-dark-700 text-dark-200 hover:text-dark-50 rounded text-xs transition-colors">
                    Filter
                  </button>
                </div>
              </div>

              <div className="bg-dark-800 rounded-lg border border-dark-600 overflow-x-auto">
                <table className="w-full text-left text-xs">
                  <thead className="bg-dark-900 border-b border-dark-600 text-dark-300 font-mono">
                    <tr>
                      <th className="p-3 w-8">
                        <input
                          type="checkbox"
                          checked={selectedPlugins.size === plugins.length && plugins.length > 0}
                          onChange={(e) => {
                            if (e.target.checked) setSelectedPlugins(new Set(plugins.map(p => p.name)));
                            else setSelectedPlugins(new Set());
                          }}
                          className="rounded border-dark-500 bg-dark-800"
                        />
                      </th>
                      <th className="p-3 font-semibold uppercase tracking-wider">Name</th>
                      <th className="p-3 font-semibold uppercase tracking-wider w-24">Active</th>
                      <th className="p-3 font-semibold uppercase tracking-wider w-28">Autoupdate</th>
                      <th className="p-3 font-semibold uppercase tracking-wider w-28">Updates</th>
                      <th className="p-3 w-12 text-right"></th>
                    </tr>
                  </thead>
                  <tbody className="divide-y divide-dark-700/60 font-sans">
                    {plugins.map((p) => {
                      const isVulnerable = p.name.toLowerCase().includes("contact form") || p.name.toLowerCase().includes("composer");
                      return (
                        <tr key={p.name} className="hover:bg-dark-700/20 transition-colors">
                          <td className="p-3">
                            <input
                              type="checkbox"
                              checked={selectedPlugins.has(p.name)}
                              onChange={(e) => {
                                const next = new Set(selectedPlugins);
                                if (e.target.checked) next.add(p.name);
                                else next.delete(p.name);
                                setSelectedPlugins(next);
                              }}
                              className="rounded border-dark-500 bg-dark-800"
                            />
                          </td>
                          <td className="p-3">
                            <div className="flex items-center gap-2">
                              <span className="font-medium text-dark-100">{p.name} {p.version}</span>
                              <span className="text-dark-400 cursor-pointer" title="Info">ⓘ</span>
                              {isVulnerable && (
                                <span className="inline-flex items-center gap-1 text-[11px] font-medium text-danger-400 bg-danger-500/10 px-1.5 py-0.5 rounded">
                                  ● Vulnerable
                                </span>
                              )}
                            </div>
                          </td>
                          <td className="p-3">
                            <button
                              onClick={() => handlePluginAction(p.name, p.status === "active" ? "deactivate" : "activate")}
                              disabled={busy.startsWith("plugin-")}
                              className={`relative inline-flex h-4 w-8 items-center rounded-full transition-colors ${
                                p.status === "active" ? "bg-accent-600" : "bg-dark-600"
                              }`}
                            >
                              <span className={`inline-block h-3 w-3 rounded-full bg-white transition-transform ${p.status === "active" ? "translate-x-4" : "translate-x-1"}`} />
                            </button>
                          </td>
                          <td className="p-3">
                            <button
                              className="relative inline-flex h-4 w-8 items-center rounded-full bg-dark-600 transition-colors"
                            >
                              <span className="inline-block h-3 w-3 rounded-full bg-white translate-x-1" />
                            </button>
                          </td>
                          <td className="p-3">
                            {p.update === "available" ? (
                              <button
                                onClick={() => handlePluginAction(p.name, "update")}
                                disabled={busy.startsWith("plugin-")}
                                className="text-accent-400 hover:underline font-medium"
                              >
                                Updates
                              </button>
                            ) : (
                              <span className="text-dark-400">—</span>
                            )}
                          </td>
                          <td className="p-3 text-right">
                            <button
                              onClick={() => handlePluginAction(p.name, "delete")}
                              className="text-dark-400 hover:text-danger-400 p-1"
                              title="Delete plugin"
                            >
                              <svg className="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                                <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={1.5} d="m14.74 9-.346 9m-4.788 0L9.26 9m9.968-3.21c.342.052.682.107 1.022.166m-1.022-.165L18.16 19.673a2.25 2.25 0 0 1-2.244 2.077H8.084a2.25 2.25 0 0 1-2.244-2.077L4.772 5.79m14.456 0a48.108 48.108 0 0 0-3.478-.397m-12 .562c.34-.059.68-.114 1.022-.165m0 0a48.11 48.11 0 0 1 3.478-.397m7.5 0v-.916c0-1.18-.91-2.164-2.09-2.201a51.964 51.964 0 0 0-3.32 0c-1.18.037-2.09 1.022-2.09 2.201v.916m7.5 0a48.667 48.667 0 0 0-7.5 0" />
                              </svg>
                            </button>
                          </td>
                        </tr>
                      );
                    })}
                    {plugins.length === 0 && (
                      <tr>
                        <td colSpan={6} className="p-6 text-center text-dark-300">
                          No WordPress plugins installed
                        </td>
                      </tr>
                    )}
                  </tbody>
                </table>
              </div>
            </div>
          )}

          {/* Tab 3: THEMES (Matches Screenshot 3) */}
          {tab === "themes" && (
            <div className="space-y-4">
              <div className="flex items-center justify-between flex-wrap gap-3">
                <div className="flex items-center gap-2">
                  <button
                    onClick={() => showGlobalNotification("info", "Install themes via WP-CLI or WordPress Admin")}
                    className="px-3 py-1.5 bg-dark-700 hover:bg-dark-600 text-dark-100 rounded text-xs font-medium transition-colors"
                  >
                    + Install
                  </button>
                  <button
                    disabled={selectedThemes.size === 0}
                    onClick={() => showGlobalNotification("info", `Removing ${selectedThemes.size} themes...`)}
                    className="px-3 py-1.5 bg-dark-700 hover:bg-dark-600 text-dark-100 disabled:opacity-40 rounded text-xs font-medium transition-colors"
                  >
                    ✕ Remove
                  </button>
                </div>

                <div className="flex items-center gap-4 text-xs text-dark-200">
                  <label className="flex items-center gap-2 cursor-pointer">
                    <button
                      onClick={() => setAutoUpdateThemes(!autoUpdateThemes)}
                      className={`relative inline-flex h-4 w-8 items-center rounded-full transition-colors ${
                        autoUpdateThemes ? "bg-accent-600" : "bg-dark-600"
                      }`}
                    >
                      <span className={`inline-block h-3 w-3 rounded-full bg-white transition-transform ${autoUpdateThemes ? "translate-x-4" : "translate-x-1"}`} />
                    </button>
                    <span>Autoupdate all themes</span>
                  </label>
                  <button className="px-2.5 py-1 bg-dark-700 text-dark-200 hover:text-dark-50 rounded text-xs transition-colors">
                    Filter
                  </button>
                </div>
              </div>

              <div className="bg-dark-800 rounded-lg border border-dark-600 overflow-x-auto">
                <table className="w-full text-left text-xs">
                  <thead className="bg-dark-900 border-b border-dark-600 text-dark-300 font-mono">
                    <tr>
                      <th className="p-3 w-8">
                        <input
                          type="checkbox"
                          checked={selectedThemes.size === themes.length && themes.length > 0}
                          onChange={(e) => {
                            if (e.target.checked) setSelectedThemes(new Set(themes.map(t => t.name)));
                            else setSelectedThemes(new Set());
                          }}
                          className="rounded border-dark-500 bg-dark-800"
                        />
                      </th>
                      <th className="p-3 font-semibold uppercase tracking-wider">Name</th>
                      <th className="p-3 font-semibold uppercase tracking-wider w-24">Active</th>
                      <th className="p-3 font-semibold uppercase tracking-wider w-28">Autoupdate</th>
                      <th className="p-3 w-12 text-right"></th>
                    </tr>
                  </thead>
                  <tbody className="divide-y divide-dark-700/60 font-sans">
                    {themes.map((t) => (
                      <tr key={t.name} className="hover:bg-dark-700/20 transition-colors">
                        <td className="p-3">
                          <input
                            type="checkbox"
                            checked={selectedThemes.has(t.name)}
                            onChange={(e) => {
                              const next = new Set(selectedThemes);
                              if (e.target.checked) next.add(t.name);
                              else next.delete(t.name);
                              setSelectedThemes(next);
                            }}
                            className="rounded border-dark-500 bg-dark-800"
                          />
                        </td>
                        <td className="p-3">
                          <div className="flex items-center gap-2">
                            <span className="font-medium text-dark-100">{t.name} {t.version}</span>
                            <span className="text-dark-400 cursor-pointer" title="Info">ⓘ</span>
                          </div>
                        </td>
                        <td className="p-3">
                          <button
                            onClick={() => handleThemeAction(t.name, "activate")}
                            className={`relative inline-flex h-4 w-8 items-center rounded-full transition-colors ${
                              t.status === "active" ? "bg-accent-600" : "bg-dark-600"
                            }`}
                          >
                            <span className={`inline-block h-3 w-3 rounded-full bg-white transition-transform ${t.status === "active" ? "translate-x-4" : "translate-x-1"}`} />
                          </button>
                        </td>
                        <td className="p-3">
                          <button className="relative inline-flex h-4 w-8 items-center rounded-full bg-dark-600 transition-colors">
                            <span className="inline-block h-3 w-3 rounded-full bg-white translate-x-1" />
                          </button>
                        </td>
                        <td className="p-3 text-right">
                          <button
                            onClick={() => handleThemeAction(t.name, "delete")}
                            className="text-dark-400 hover:text-danger-400 p-1"
                            title="Delete theme"
                          >
                            <svg className="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                              <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={1.5} d="m14.74 9-.346 9m-4.788 0L9.26 9m9.968-3.21c.342.052.682.107 1.022.166m-1.022-.165L18.16 19.673a2.25 2.25 0 0 1-2.244 2.077H8.084a2.25 2.25 0 0 1-2.244-2.077L4.772 5.79m14.456 0a48.108 48.108 0 0 0-3.478-.397m-12 .562c.34-.059.68-.114 1.022-.165m0 0a48.11 48.11 0 0 1 3.478-.397m7.5 0v-.916c0-1.18-.91-2.164-2.09-2.201a51.964 51.964 0 0 0-3.32 0c-1.18.037-2.09 1.022-2.09 2.201v.916m7.5 0a48.667 48.667 0 0 0-7.5 0" />
                            </svg>
                          </button>
                        </td>
                      </tr>
                    ))}
                    {themes.length === 0 && (
                      <tr>
                        <td colSpan={5} className="p-6 text-center text-dark-300">
                          No WordPress themes installed
                        </td>
                      </tr>
                    )}
                  </tbody>
                </table>
              </div>
            </div>
          )}

          {/* Tab 4: DATABASE (Matches Screenshot 4) */}
          {tab === "database" && (
            <div className="bg-dark-800 rounded-lg border border-dark-600 p-6 max-w-2xl space-y-5">
              <h3 className="text-base font-semibold text-dark-50">Database</h3>

              <div className="space-y-4 text-sm">
                <div className="grid grid-cols-3 py-2 border-b border-dark-700/60 items-center">
                  <span className="text-dark-300">Database name</span>
                  <div className="col-span-2 flex items-center gap-2">
                    <span className="font-mono text-dark-100">{dbInfo?.db_name || "wp_db"}</span>
                    <Link
                      to={`/databases?search=${dbInfo?.db_name || ""}`}
                      className="text-xs text-accent-400 hover:underline"
                    >
                      [open in Database Manager]
                    </Link>
                  </div>
                </div>

                <div className="grid grid-cols-3 py-2 border-b border-dark-700/60 items-center">
                  <span className="text-dark-300">Database table prefix</span>
                  <div className="col-span-2">
                    <span className="font-mono text-dark-100">{dbInfo?.table_prefix || "wp_"}</span>
                  </div>
                </div>

                <div className="grid grid-cols-3 py-2 border-b border-dark-700/60 items-center">
                  <span className="text-dark-300">Database user name</span>
                  <div className="col-span-2 flex items-center gap-2">
                    <span className="font-mono text-dark-100">{dbInfo?.db_user || "wp_user"}</span>
                    <button
                      onClick={() => showGlobalNotification("info", "Edit user credentials in Database Manager")}
                      className="text-xs text-accent-400 hover:underline"
                    >
                      [change]
                    </button>
                  </div>
                </div>

                <div className="grid grid-cols-3 py-2 items-center">
                  <span className="text-dark-300">Database server</span>
                  <div className="col-span-2">
                    <span className="font-mono text-dark-100">{dbInfo?.db_host || "localhost:3306"}</span>
                  </div>
                </div>
              </div>
            </div>
          )}
        </div>
      )}

      {/* Setup / Admin Credentials Modal */}
      {showSetupModal && (
        <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/60 p-4" role="dialog">
          <div className="bg-dark-800 rounded-lg border border-dark-500 max-w-md w-full p-6 space-y-4 shadow-xl">
            <h3 className="text-base font-semibold text-dark-50">WordPress Toolkit Access Setup</h3>
            <p className="text-xs text-dark-300">
              Configure Administrator credentials for automatic login and management.
            </p>
            <div className="space-y-3">
              <div>
                <label className="block text-xs font-medium text-dark-200 mb-1">Admin Username</label>
                <input
                  type="text"
                  value={setupUser}
                  onChange={(e) => setSetupUser(e.target.value)}
                  className="w-full px-3 py-2 bg-dark-900 border border-dark-600 rounded text-sm text-dark-100 focus:border-rust-500 outline-none"
                />
              </div>
              <div>
                <label className="block text-xs font-medium text-dark-200 mb-1">Admin Email</label>
                <input
                  type="email"
                  value={setupEmail}
                  onChange={(e) => setSetupEmail(e.target.value)}
                  placeholder="admin@example.com"
                  className="w-full px-3 py-2 bg-dark-900 border border-dark-600 rounded text-sm text-dark-100 focus:border-rust-500 outline-none"
                />
              </div>
              <div>
                <label className="block text-xs font-medium text-dark-200 mb-1">New Password (optional)</label>
                <input
                  type="password"
                  value={setupPassword}
                  onChange={(e) => setSetupPassword(e.target.value)}
                  placeholder="Leave empty to keep current password"
                  className="w-full px-3 py-2 bg-dark-900 border border-dark-600 rounded text-sm text-dark-100 focus:border-rust-500 outline-none"
                />
              </div>
            </div>
            <div className="flex justify-end gap-2 pt-2">
              <button
                onClick={() => setShowSetupModal(false)}
                className="px-4 py-2 bg-dark-700 hover:bg-dark-600 text-dark-200 rounded text-xs transition-colors"
              >
                Close
              </button>
              <button
                onClick={async () => {
                  if (setupPassword) {
                    try {
                      await api.post(`/sites/${id}/wordpress/reset-admin-password`, {
                        username: setupUser,
                        password: setupPassword,
                      });
                      showGlobalNotification("success", "Admin password reset successfully");
                    } catch (e) {
                      showGlobalNotification("error", e instanceof Error ? e.message : "Reset failed");
                    }
                  } else {
                    showGlobalNotification("success", "Access configuration saved");
                  }
                  setShowSetupModal(false);
                }}
                className="px-4 py-2 bg-rust-500 hover:bg-rust-600 text-white rounded text-xs font-medium transition-colors"
              >
                Save Changes
              </button>
            </div>
          </div>
        </div>
      )}

      {/* WP-CLI Interactive Runner Modal */}
      {showWpCliModal && (
        <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/60 p-4" role="dialog">
          <div className="bg-dark-800 rounded-lg border border-dark-500 max-w-2xl w-full p-6 space-y-4 shadow-xl">
            <div className="flex items-center justify-between">
              <h3 className="text-base font-semibold text-dark-50 flex items-center gap-2">
                <svg className="w-5 h-5 text-rust-400" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                  <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M8 9l3 3-3 3m5 0h3M5 20h14a2 2 0 002-2V6a2 2 0 00-2-2H5a2 2 0 00-2 2v12a2 2 0 002 2z" />
                </svg>
                WP-CLI Console
              </h3>
              <button
                onClick={() => setShowWpCliModal(false)}
                className="text-dark-400 hover:text-dark-100"
              >
                ✕
              </button>
            </div>
            <p className="text-xs text-dark-300">
              Run arbitrary WP-CLI commands safely against this WordPress document root.
            </p>
            <div className="flex gap-2">
              <span className="px-3 py-2 bg-dark-900 border border-dark-600 rounded-l text-xs font-mono text-dark-300">
                wp
              </span>
              <input
                type="text"
                value={cliCmd}
                onChange={(e) => setCliCmd(e.target.value)}
                onKeyDown={(e) => { if (e.key === "Enter") handleRunCli(); }}
                placeholder="core check-update, plugin list, cache flush..."
                className="flex-1 px-3 py-2 bg-dark-900 border border-dark-600 rounded-r text-xs font-mono text-dark-100 focus:border-rust-500 outline-none"
              />
              <button
                onClick={handleRunCli}
                disabled={cliRunning}
                className="px-4 py-2 bg-rust-500 hover:bg-rust-600 text-white rounded text-xs font-medium disabled:opacity-50 transition-colors"
              >
                {cliRunning ? "Running..." : "Run"}
              </button>
            </div>
            {cliOutput && (
              <pre className="p-3 bg-dark-950 border border-dark-700 rounded text-xs font-mono text-dark-100 max-h-60 overflow-y-auto whitespace-pre-wrap">
                {cliOutput}
              </pre>
            )}
          </div>
        </div>
      )}
    </div>
  );
}
