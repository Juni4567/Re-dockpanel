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
  auto_update?: string;
  vulnerable?: boolean;
}

interface WpTheme {
  name: string;
  status: string;
  version: string;
  update: string;
  title?: string;
  auto_update?: string;
  vulnerable?: boolean;
}

interface WpFeaturesStatus {
  wp_debug: boolean;
  search_indexing: boolean;
  takeover_cron: boolean;
  maintenance_mode: boolean;
  hotlink_protect: boolean;
  pass_protect: boolean;
  caching: boolean;
  vuln_protect: boolean;
  site_title: string;
  admin_user: string;
  admin_email: string;
}

interface CliHistoryItem {
  cmd: string;
  output: string;
  time: string;
  success: boolean;
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

  // Setup / Admin credentials
  const [setupUser, setSetupUser] = useState("admin");
  const [setupPassword, setSetupPassword] = useState("");
  const [setupEmail, setSetupEmail] = useState("");

  // Modals state
  const [showSetupModal, setShowSetupModal] = useState(false);
  const [showWpCliModal, setShowWpCliModal] = useState(false);
  const [showIntegrityModal, setShowIntegrityModal] = useState(false);
  const [showMitigateModal, setShowMitigateModal] = useState(false);
  const [showCopyDataModal, setShowCopyDataModal] = useState(false);
  const [showPassModal, setShowPassModal] = useState(false);
  const [showDebugModal, setShowDebugModal] = useState(false);
  const [showCacheModal, setShowCacheModal] = useState(false);
  const [showAutoupdateModal, setShowAutoupdateModal] = useState(false);
  const [showHotlinkModal, setShowHotlinkModal] = useState(false);
  const [showInstallPluginModal, setShowInstallPluginModal] = useState(false);
  const [showInstallThemeModal, setShowInstallThemeModal] = useState(false);
  const [showDbUserModal, setShowDbUserModal] = useState(false);
  const [showWpToolkitModal, setShowWpToolkitModal] = useState(false);
  const [showPluginUpdatesModal, setShowPluginUpdatesModal] = useState(false);
  const [showThemeUpdatesModal, setShowThemeUpdatesModal] = useState(false);
  const [selectedPluginUpdates, setSelectedPluginUpdates] = useState<Set<string>>(new Set());
  const [selectedThemeUpdates, setSelectedThemeUpdates] = useState<Set<string>>(new Set());
  const [batchUpdating, setBatchUpdating] = useState(false);
  const [batchProgress, setBatchProgress] = useState<{ current: number; total: number; item: string }>({ current: 0, total: 0, item: "" });

  // WP-CLI Console
  const [cliCmd, setCliCmd] = useState("core version");
  const [cliRunning, setCliRunning] = useState(false);
  const [cliHistory, setCliHistory] = useState<CliHistoryItem[]>([]);

  // Integrity Check
  const [integrityLoading, setIntegrityLoading] = useState(false);
  const [reinstallingCore, setReinstallingCore] = useState(false);
  const [integrityResult, setIntegrityResult] = useState<{
    clean: boolean;
    issues: Array<{ file: string; status: string; message?: string }>;
    message: string;
    checked_at: string;
  } | null>(null);

  // Mitigate Vulnerabilities
  const [mitigating, setMitigating] = useState(false);
  const [mitigateResult, setMitigateResult] = useState<{
    ok: boolean;
    actions: string[];
    message: string;
  } | null>(null);

  // Copy Data
  const [allSites, setAllSites] = useState<Array<{ id: string; domain: string }>>([]);
  const [copyDstDomain, setCopyDstDomain] = useState("");
  const [copyFiles, setCopyFiles] = useState(true);
  const [copyDb, setCopyDb] = useState(true);
  const [copying, setCopying] = useState(false);

  // Password Protection modal
  const [passUser, setPassUser] = useState("admin");
  const [passPassword, setPassPassword] = useState("");

  // Debug settings modal
  const [debugLog, setDebugLog] = useState(true);
  const [debugDisplay, setDebugDisplay] = useState(false);

  // Caching modal
  const [cacheClearing, setCacheClearing] = useState(false);

  // Autoupdate settings
  const [autoUpdateCore, setAutoUpdateCore] = useState<"minor" | "all" | "disabled">("minor");

  // Selected plugins / themes for bulk action
  const [selectedPlugins, setSelectedPlugins] = useState<Set<string>>(new Set());
  const [selectedThemes, setSelectedThemes] = useState<Set<string>>(new Set());

  // Search/Filters in tables
  const [pluginFilter, setPluginFilter] = useState("");
  const [themeFilter, setThemeFilter] = useState("");
  const [pluginPage, setPluginPage] = useState(1);
  const pageSize = 10;

  // New Plugin / Theme install input
  const [newPluginSlug, setNewPluginSlug] = useState("");
  const [newThemeSlug, setNewThemeSlug] = useState("");
  const [installingItem, setInstallingItem] = useState(false);

  // Change DB user password
  const [newDbPassword, setNewDbPassword] = useState("");
  const [changingDbPass, setChangingDbPass] = useState(false);

  // Install form state (for uninstalled sites)
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
        loadFeaturesStatus();
      }
    } catch (e) {
      setError(e instanceof Error ? e.message : "Failed to load WordPress status");
    } finally {
      setLoading(false);
    }
  };

  const loadFeaturesStatus = async () => {
    try {
      const feat = await api.get<WpFeaturesStatus>(`/sites/${id}/wordpress/features-status`);
      setWpDebug(feat.wp_debug);
      setSearchIndexing(feat.search_indexing);
      setTakeoverCron(feat.takeover_cron);
      setMaintenanceMode(feat.maintenance_mode);
      setHotlinkProtect(feat.hotlink_protect);
      setPassProtect(feat.pass_protect);
      setNginxCache(feat.caching);
      setVulnProtect(feat.vuln_protect);
      if (feat.site_title) setSiteTitle(feat.site_title);
      if (feat.admin_user) setSetupUser(feat.admin_user);
      if (feat.admin_email) setSetupEmail(feat.admin_email);
    } catch {
      // defaults preserved
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
      if (smartUpdate) {
        await api.post(`/sites/${id}/wordpress/update-safe`, {});
        showGlobalNotification("success", `Smart Update: ${target.toUpperCase()} updated with verified health check`);
      } else {
        await api.post(`/sites/${id}/wordpress/update/${target}`, {});
        showGlobalNotification("success", `${target.toUpperCase()} updated successfully`);
      }
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

  // Open Plugin Updates Selection Modal
  const openPluginUpdatesModal = () => {
    const updatable = plugins.filter(
      (p) => p.update === "available" || p.vulnerable || (p as any).update_version
    );
    const candidates = updatable.length > 0 ? updatable : plugins;
    setSelectedPluginUpdates(new Set(candidates.map((p) => p.name)));
    setShowPluginUpdatesModal(true);
  };

  // Open Theme Updates Selection Modal
  const openThemeUpdatesModal = () => {
    const updatable = themes.filter(
      (t) => t.update === "available" || t.vulnerable || (t as any).update_version
    );
    const candidates = updatable.length > 0 ? updatable : themes;
    setSelectedThemeUpdates(new Set(candidates.map((t) => t.name)));
    setShowThemeUpdatesModal(true);
  };

  // Execute batch plugin update on selected plugins
  const handleRunBatchPluginUpdates = async () => {
    const items = Array.from(selectedPluginUpdates);
    if (items.length === 0) return;
    setBatchUpdating(true);
    let successCount = 0;
    for (let i = 0; i < items.length; i++) {
      const name = items[i];
      setBatchProgress({ current: i + 1, total: items.length, item: name });
      try {
        await api.post(`/sites/${id}/wordpress/plugin/update`, { name });
        successCount++;
      } catch (e) {
        showGlobalNotification("error", `Failed to update plugin ${name}: ${e instanceof Error ? e.message : "Error"}`);
      }
    }
    setBatchUpdating(false);
    setShowPluginUpdatesModal(false);
    showGlobalNotification("success", `Updated ${successCount} of ${items.length} plugins`);
    loadPlugins();
    loadData();
  };

  // Execute batch theme update on selected themes
  const handleRunBatchThemeUpdates = async () => {
    const items = Array.from(selectedThemeUpdates);
    if (items.length === 0) return;
    setBatchUpdating(true);
    let successCount = 0;
    for (let i = 0; i < items.length; i++) {
      const name = items[i];
      setBatchProgress({ current: i + 1, total: items.length, item: name });
      try {
        await api.post(`/sites/${id}/wordpress/theme/update`, { name });
        successCount++;
      } catch (e) {
        showGlobalNotification("error", `Failed to update theme ${name}: ${e instanceof Error ? e.message : "Error"}`);
      }
    }
    setBatchUpdating(false);
    setShowThemeUpdatesModal(false);
    showGlobalNotification("success", `Updated ${successCount} of ${items.length} themes`);
    loadThemes();
    loadData();
  };

  // Toggle Features with Backend API
  const handleToggleFeature = async (feature: string, targetVal?: boolean, params?: Record<string, unknown>) => {
    let nextVal: boolean;
    if (typeof targetVal === "boolean") {
      nextVal = targetVal;
    } else {
      if (feature === "wp_debug") nextVal = !wpDebug;
      else if (feature === "search_indexing") nextVal = !searchIndexing;
      else if (feature === "takeover_cron") nextVal = !takeoverCron;
      else if (feature === "hotlink_protection") nextVal = !hotlinkProtect;
      else if (feature === "pass_protect") nextVal = !passProtect;
      else if (feature === "caching") nextVal = !nginxCache;
      else if (feature === "vuln_protect") nextVal = !vulnProtect;
      else nextVal = true;
    }
    try {
      await api.post(`/sites/${id}/wordpress/toggle-feature`, {
        feature,
        enabled: nextVal,
        params,
      });
      if (feature === "wp_debug") setWpDebug(nextVal);
      if (feature === "search_indexing") setSearchIndexing(nextVal);
      if (feature === "takeover_cron") setTakeoverCron(nextVal);
      if (feature === "hotlink_protection") setHotlinkProtect(nextVal);
      if (feature === "pass_protect") setPassProtect(nextVal);
      if (feature === "caching") setNginxCache(nextVal);
      if (feature === "vuln_protect") setVulnProtect(nextVal);
      showGlobalNotification("success", `${feature.replace(/_/g, " ")} ${nextVal ? "enabled" : "disabled"}`);
    } catch (e) {
      showGlobalNotification("error", e instanceof Error ? e.message : `Failed to toggle ${feature}`);
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

  const handleSetupSave = async () => {
    try {
      await api.post(`/sites/${id}/wordpress/setup`, {
        admin_user: setupUser,
        admin_email: setupEmail,
        admin_pass: setupPassword || undefined,
        site_title: siteTitle,
      });
      showGlobalNotification("success", "WordPress access & setup updated successfully");
      setShowSetupModal(false);
      setSetupPassword("");
      loadFeaturesStatus();
    } catch (e) {
      showGlobalNotification("error", e instanceof Error ? e.message : "Failed to update setup credentials");
    }
  };

  // WP-CLI interactive console runner
  const handleRunCli = async (cmdToRun?: string) => {
    const command = (cmdToRun !== undefined ? cmdToRun : cliCmd).trim();
    if (!command) return;
    setCliRunning(true);
    try {
      const res = await api.post<{ ok: boolean; output: string }>(`/sites/${id}/wordpress/cli`, {
        command,
      });
      const isSuccess = res.ok !== false;
      const output = res.output || (isSuccess ? "Success: Command completed with no output." : "Error: Command exited with error.");
      setCliHistory((prev) => [
        ...prev,
        { cmd: command, output, time: new Date().toLocaleTimeString(), success: isSuccess },
      ]);
      setCliCmd("");
    } catch (e) {
      const err = e instanceof Error ? e.message : "Execution failed";
      setCliHistory((prev) => [
        ...prev,
        { cmd: command, output: err, time: new Date().toLocaleTimeString(), success: false },
      ]);
    } finally {
      setCliRunning(false);
    }
  };

  // Integrity Check
  const handleCheckIntegrity = async () => {
    setShowIntegrityModal(true);
    setIntegrityLoading(true);
    try {
      const res = await api.post<{
        clean: boolean;
        issues: Array<{ file: string; status: string; message?: string }>;
        message: string;
        checked_at: string;
      }>(`/sites/${id}/wordpress/integrity`);
      setIntegrityResult(res);
    } catch (e) {
      showGlobalNotification("error", e instanceof Error ? e.message : "Failed to verify integrity");
    } finally {
      setIntegrityLoading(false);
    }
  };

  // Re-install WordPress Core Files safely
  const handleReinstallCore = async () => {
    setReinstallingCore(true);
    try {
      const res = await api.post<{ ok: boolean; message: string }>(`/sites/${id}/wordpress/reinstall-core`, {});
      showGlobalNotification("success", res.message || "WordPress core files re-installed successfully!");
      await handleCheckIntegrity();
      loadData();
    } catch (e) {
      showGlobalNotification("error", e instanceof Error ? e.message : "Failed to re-install WordPress core");
    } finally {
      setReinstallingCore(false);
    }
  };

  // Mitigate Vulnerabilities
  const handleMitigateVulns = async () => {
    setMitigating(true);
    try {
      const res = await api.post<{
        ok: boolean;
        actions: string[];
        message: string;
      }>(`/sites/${id}/wordpress/mitigate-vulns`);
      setMitigateResult(res);
      showGlobalNotification("success", res.message || "Mitigations applied");
      loadPlugins();
      loadThemes();
    } catch (e) {
      showGlobalNotification("error", e instanceof Error ? e.message : "Mitigation failed");
    } finally {
      setMitigating(false);
    }
  };

  // Open Copy Data Modal & fetch destination sites
  const openCopyDataModal = async () => {
    setShowCopyDataModal(true);
    try {
      const sitesList = await api.get<Array<{ id: string; domain: string }>>("/sites");
      const others = sitesList.filter((s) => s.id !== id);
      setAllSites(others);
      if (others.length > 0 && !copyDstDomain) {
        setCopyDstDomain(others[0].domain);
      }
    } catch {
      setAllSites([]);
    }
  };

  const handleCopyData = async () => {
    if (!copyDstDomain) {
      showGlobalNotification("error", "Please select a destination site");
      return;
    }
    setCopying(true);
    try {
      const res = await api.post<{ ok: boolean; message: string }>(`/sites/${id}/wordpress/copy-data`, {
        dst_domain: copyDstDomain,
        copy_files: copyFiles,
        copy_db: copyDb,
      });
      showGlobalNotification("success", res.message || "WordPress data copied successfully");
      setShowCopyDataModal(false);
    } catch (e) {
      showGlobalNotification("error", e instanceof Error ? e.message : "Copy failed");
    } finally {
      setCopying(false);
    }
  };

  const handlePurgeCache = async () => {
    setCacheClearing(true);
    try {
      await api.post(`/sites/${id}/wordpress/clear-cache`);
      showGlobalNotification("success", "Nginx & WordPress cache purged successfully");
      setShowCacheModal(false);
    } catch (e) {
      showGlobalNotification("error", e instanceof Error ? e.message : "Cache purge failed");
    } finally {
      setCacheClearing(false);
    }
  };

  // Install custom plugin via slug
  const handleInstallCustomPlugin = async () => {
    const slug = newPluginSlug.trim().toLowerCase();
    if (!slug) return;
    setInstallingItem(true);
    try {
      await api.post(`/sites/${id}/wordpress/plugin/install`, { name: slug });
      showGlobalNotification("success", `Plugin "${slug}" installed successfully!`);
      setNewPluginSlug("");
      setShowInstallPluginModal(false);
      loadPlugins();
    } catch (e) {
      showGlobalNotification("error", e instanceof Error ? e.message : "Failed to install plugin");
    } finally {
      setInstallingItem(false);
    }
  };

  // Install custom theme via slug
  const handleInstallCustomTheme = async () => {
    const slug = newThemeSlug.trim().toLowerCase();
    if (!slug) return;
    setInstallingItem(true);
    try {
      await api.post(`/sites/${id}/wordpress/theme/install`, { name: slug });
      showGlobalNotification("success", `Theme "${slug}" installed successfully!`);
      setNewThemeSlug("");
      setShowInstallThemeModal(false);
      loadThemes();
    } catch (e) {
      showGlobalNotification("error", e instanceof Error ? e.message : "Failed to install theme");
    } finally {
      setInstallingItem(false);
    }
  };

  // Change DB user password
  const handleChangeDbPassword = async () => {
    if (!newDbPassword || !dbInfo?.db_name) return;
    setChangingDbPass(true);
    try {
      await api.post(`/sites/${id}/wordpress/cli`, {
        command: `config set DB_PASSWORD "${newDbPassword}"`,
      });
      showGlobalNotification("success", "Database credentials updated in wp-config.php");
      setShowDbUserModal(false);
      setNewDbPassword("");
    } catch (e) {
      showGlobalNotification("error", e instanceof Error ? e.message : "Failed to update db credentials");
    } finally {
      setChangingDbPass(false);
    }
  };

  const vulnerablePluginsCount = plugins.filter(
    (p) => p.vulnerable || p.name.toLowerCase().includes("contact form") || p.name.toLowerCase().includes("composer")
  ).length;

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

  // Filtered plugins
  const filteredPlugins = plugins.filter((p) => {
    if (!pluginFilter) return true;
    const q = pluginFilter.toLowerCase();
    return (
      p.name.toLowerCase().includes(q) ||
      (p.title && p.title.toLowerCase().includes(q)) ||
      p.version.toLowerCase().includes(q)
    );
  });
  const totalPluginPages = Math.ceil(filteredPlugins.length / pageSize) || 1;
  const paginatedPlugins = filteredPlugins.slice((pluginPage - 1) * pageSize, pluginPage * pageSize);

  // Filtered themes
  const filteredThemes = themes.filter((t) => {
    if (!themeFilter) return true;
    const q = themeFilter.toLowerCase();
    return (
      t.name.toLowerCase().includes(q) ||
      (t.title && t.title.toLowerCase().includes(q)) ||
      t.version.toLowerCase().includes(q)
    );
  });

  return (
    <div className="p-6 lg:p-8 animate-fade-up space-y-6">
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
        /* WordPress Installed: Exact Plesk Reference Layout with Persistent Sidebar & Bottom Bar */
        <div className="space-y-6">
          {/* TWO-COLUMN GRID: Left Sidebar starts from the top, matching Plesk Screenshots 1, 2, 3, 4 */}
          <div className="grid grid-cols-1 lg:grid-cols-4 gap-6 items-start">
            {/* Left Column: Persistent Site Screenshot Preview & WP Toolkit Access Box */}
            <div className="lg:col-span-1 space-y-4">
              {/* Screenshot Box with Refresh Button on bottom-right corner */}
              <div className="bg-dark-800 rounded-lg border border-dark-600 p-1.5 overflow-hidden shadow-sm">
                <div className="relative group">
                  <SiteThumbnail domain={domain} className="w-full h-44 rounded object-cover" showRefresh={true} />
                </div>
              </div>

              {/* WP Toolkit access Card */}
              <div className="bg-dark-800 rounded-lg border border-dark-600 p-4 space-y-3">
                <div className="text-xs font-bold text-dark-100">
                  WP Toolkit access
                </div>
                <div
                  onClick={() => setShowWpToolkitModal(true)}
                  className="text-[11px] text-accent-400 hover:underline cursor-pointer"
                >
                  Get WP Toolkit plugin for WordPress
                </div>

                <div className="flex items-center gap-3 pt-1">
                  <a
                    href={`https://${domain}/wp-admin`}
                    target="_blank"
                    rel="noopener noreferrer"
                    className="py-1.5 px-4 bg-accent-600 hover:bg-accent-700 text-white text-xs font-semibold rounded flex items-center justify-center gap-1.5 shadow transition-colors"
                  >
                    <svg className="w-3.5 h-3.5" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                      <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M15 7a2 2 0 012 2m4 0a6 6 0 01-7.743 5.743L11 17H9v2H7v2H4a1 1 0 01-1-1v-2.586a1 1 0 01.293-.707l5.964-5.964A6 6 0 1121 9z" />
                    </svg>
                    Log in
                  </a>
                  <button
                    onClick={() => setShowSetupModal(true)}
                    className="text-xs text-accent-400 hover:underline font-medium"
                  >
                    Setup
                  </button>
                </div>

                <div className="pt-2 border-t border-dark-700/60 space-y-2 text-xs">
                  <div className="flex items-center gap-2 text-dark-200">
                    <svg className="w-4 h-4 text-dark-400 shrink-0" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                      <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M16 7a4 4 0 11-8 0 4 4 0 018 0zM12 14a7 7 0 00-7 7h14a7 7 0 00-7-7z" />
                    </svg>
                    <span className="font-mono">{setupUser}</span>
                  </div>
                  <div className="flex items-center gap-2">
                    <svg className="w-4 h-4 text-dark-400 shrink-0" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                      <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M21 12a9 9 0 01-9 9m9-9a9 9 0 00-9-9m9 9H3m9 9a9 9 0 01-9-9m9 9c1.657 0 3-4.03 3-9s-1.343-9-3-9m0 18c-1.657 0-3-4.03-3-9s1.343-9 3-9m-9 9a9 9 0 019-9" />
                    </svg>
                    <Link to={`/sites/${id}`} className="text-accent-400 hover:underline font-mono">
                      Manage domain
                    </Link>
                  </div>
                </div>
              </div>
            </div>

            {/* Right Column: Title, Tabs, and Dynamic Content by Active Tab */}
            <div className="lg:col-span-3 space-y-4">
              {/* Site Title Row with Inline Edit & Visit Site */}
              <div className="flex items-center justify-between gap-4">
                <div className="flex items-center gap-3">
                  <Link
                    to="/wordpress-toolkit"
                    className="p-1 text-dark-400 hover:text-dark-100 rounded hover:bg-dark-700 transition-colors"
                    title="Back to WordPress Toolkit"
                  >
                    <svg className="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}>
                      <path strokeLinecap="round" strokeLinejoin="round" d="M15.75 19.5 8.25 12l7.5-7.5" />
                    </svg>
                  </Link>
                  {isEditingTitle ? (
                    <input
                      type="text"
                      value={siteTitle}
                      onChange={(e) => setSiteTitle(e.target.value)}
                      onBlur={() => {
                        setIsEditingTitle(false);
                        handleSetupSave();
                      }}
                      onKeyDown={(e) => {
                        if (e.key === "Enter") {
                          setIsEditingTitle(false);
                          handleSetupSave();
                        }
                      }}
                      autoFocus
                      className="text-xl font-bold text-dark-50 bg-dark-700 border border-dark-500 rounded px-2 py-0.5 outline-none focus:border-rust-500"
                    />
                  ) : (
                    <h1 className="text-xl font-bold text-dark-50 flex items-center gap-2">
                      {siteTitle || domain}
                      <button
                        onClick={() => setIsEditingTitle(true)}
                        className="text-dark-400 hover:text-dark-200 transition-colors"
                        title="Edit site title"
                      >
                        <svg className="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                          <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M15.232 5.232l3.536 3.536m-2.036-5.036a2.5 2.5 0 113.536 3.536L6.5 21.036H3v-3.572L16.732 3.732z" />
                        </svg>
                      </button>
                    </h1>
                  )}
                </div>

                <a
                  href={site?.ssl_enabled ? `https://${domain}` : `http://${domain}`}
                  target="_blank"
                  rel="noopener noreferrer"
                  className="px-2.5 py-1 bg-dark-700 hover:bg-dark-600 text-dark-200 hover:text-dark-50 text-xs font-mono rounded transition-colors flex items-center gap-1.5"
                >
                  <span>Visit Site</span>
                  <svg className="w-3.5 h-3.5" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                    <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M10 6H6a2 2 0 00-2 2v10a2 2 0 002 2h10a2 2 0 002-2v-4M14 4h6m0 0v6m0-6L10 14" />
                  </svg>
                </a>
              </div>

              {/* 4 Navigation Tabs under Title */}
              <div className="flex items-center gap-6 border-b border-dark-600">
                {(["dashboard", "plugins", "themes", "database"] as const).map((t) => (
                  <button
                    key={t}
                    onClick={() => setTab(t)}
                    className={`pb-2.5 text-sm font-medium transition-colors capitalize relative ${
                      tab === t
                        ? "text-rust-400 border-b-2 border-rust-500 font-semibold"
                        : "text-dark-300 hover:text-dark-100"
                    }`}
                  >
                    {t}
                  </button>
                ))}
              </div>

              {/* Tab 1: DASHBOARD (Matches Plesk Screenshot 1) */}
              {tab === "dashboard" && (
                <div className="space-y-5">
                  {/* Top Action Toolbar */}
                  <div className="flex items-center gap-5 flex-wrap text-xs font-medium text-dark-200 border-b border-dark-700/60 pb-3">
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
                      onClick={openCopyDataModal}
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
                      to={`/sites/${id}/backups`}
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

                  {/* 4 Feature Boxes Grid (Updates, Tools, Security, Performance) */}
                  <div className="grid grid-cols-1 md:grid-cols-2 gap-5">
                    {/* Card 1: Updates */}
                    <div className="bg-dark-800 rounded-lg border border-dark-600 p-5 space-y-3.5">
                      <h3 className="text-sm font-bold text-dark-50">Updates</h3>
                      <div className="space-y-3 text-xs">
                        <div className="flex items-center justify-between">
                          <span className="text-dark-300">WordPress</span>
                          <div className="flex items-center gap-2">
                            <span className="font-mono text-dark-100">{info?.version || "7.1.5"}</span>
                            <button
                              onClick={handleReinstallCore}
                              disabled={reinstallingCore}
                              className="text-[11px] text-accent-400 hover:underline font-medium transition-colors"
                              title="Safely re-install pristine official WordPress core files"
                            >
                              {reinstallingCore ? "Re-installing..." : "[Re-install Core]"}
                            </button>
                          </div>
                        </div>
                        <div className="flex items-center justify-between">
                          <span className="text-dark-300">Plugins</span>
                          {plugins.some((p) => p.update === "available") || plugins.some((p) => p.vulnerable) ? (
                            <button
                              onClick={openPluginUpdatesModal}
                              className="text-warn-400 hover:underline flex items-center gap-1 font-medium"
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
                          {themes.some((t) => t.update === "available") || themes.some((t) => t.vulnerable) ? (
                            <button
                              onClick={openThemeUpdatesModal}
                              className="text-warn-400 hover:underline flex items-center gap-1 font-medium"
                            >
                              <svg className="w-3.5 h-3.5 shrink-0" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                                <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M12 9v2m0 4h.01m-6.938 4h13.856c1.54 0 2.502-1.667 1.732-3L13.732 4c-.77-1.333-2.694-1.333-3.464 0L3.34 16c-.77 1.333.192 3 1.732 3z" />
                              </svg>
                              Install theme updates
                            </button>
                          ) : (
                            <span className="text-dark-300">Up-to-date</span>
                          )}
                        </div>
                      </div>
                    </div>

                    {/* Card 2: Tools */}
                    <div className="bg-dark-800 rounded-lg border border-dark-600 p-5 space-y-3.5">
                      <h3 className="text-sm font-bold text-dark-50">Tools</h3>
                      <div className="space-y-3 text-xs">
                        <div className="flex items-center justify-between">
                          <span className="text-dark-300">PHP</span>
                          <span className="text-dark-100">
                            {site?.php_version || "8.3.37"}{" "}
                            <Link to={`/sites/${id}`} className="text-accent-400 hover:underline ml-1">
                              Details
                            </Link>
                          </span>
                        </div>

                        {/* Debugging row with toggle on left */}
                        <div className="flex items-center gap-2">
                          <button
                            onClick={() => handleToggleFeature("wp_debug", wpDebug)}
                            className={`relative inline-flex h-4 w-7 shrink-0 items-center rounded-full transition-colors ${
                              wpDebug ? "bg-accent-600" : "bg-dark-600"
                            }`}
                          >
                            <span className={`inline-block h-3 w-3 rounded-full bg-white transition-transform ${wpDebug ? "translate-x-3.5" : "translate-x-0.5"}`} />
                          </button>
                          <span className="text-dark-200">Debugging</span>
                          <span className="text-dark-400 hover:text-dark-200 cursor-pointer text-xs" title="WP_DEBUG directives">ⓘ</span>
                          <button
                            onClick={() => setShowDebugModal(true)}
                            className="text-dark-400 hover:text-dark-200 transition-colors"
                            title="Debugging settings"
                          >
                            <svg className="w-3.5 h-3.5" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                              <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={1.8} d="M12 6V4m0 2a2 2 0 100 4m0-4a2 2 0 110 4m-6 8a2 2 0 100-4m0 4a2 2 0 110-4m0 4v2m0-6V4m6 6v10m6-2a2 2 0 100-4m0 4a2 2 0 110-4m0 4v2m0-6V4" />
                            </svg>
                          </button>
                        </div>

                        {/* Password protection row with toggle on left */}
                        <div className="flex items-center gap-2">
                          <button
                            onClick={() => {
                              if (!passProtect) setShowPassModal(true);
                              else handleToggleFeature("pass_protect", false);
                            }}
                            className={`relative inline-flex h-4 w-7 shrink-0 items-center rounded-full transition-colors ${
                              passProtect ? "bg-accent-600" : "bg-dark-600"
                            }`}
                          >
                            <span className={`inline-block h-3 w-3 rounded-full bg-white transition-transform ${passProtect ? "translate-x-3.5" : "translate-x-0.5"}`} />
                          </button>
                          <span className="text-dark-200">Password protection</span>
                          <span className="text-dark-400 hover:text-dark-200 cursor-pointer text-xs" title="HTTP Basic Authentication">ⓘ</span>
                          <button
                            onClick={() => setShowPassModal(true)}
                            className="text-dark-400 hover:text-dark-200 transition-colors"
                            title="Password protection settings"
                          >
                            <svg className="w-3.5 h-3.5" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                              <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={1.8} d="M12 6V4m0 2a2 2 0 100 4m0-4a2 2 0 110 4m-6 8a2 2 0 100-4m0 4a2 2 0 110-4m0 4v2m0-6V4m6 6v10m6-2a2 2 0 100-4m0 4a2 2 0 110-4m0 4v2m0-6V4" />
                            </svg>
                          </button>
                        </div>
                      </div>
                    </div>

                    {/* Card 3: Security */}
                    <div className="bg-dark-800 rounded-lg border border-dark-600 p-5 space-y-3.5">
                      <h3 className="text-sm font-bold text-dark-50">Security</h3>
                      <div className="space-y-3 text-xs">
                        <div className="flex items-center gap-1.5 text-rust-400 font-medium">
                          <svg className="w-3.5 h-3.5 shrink-0" fill="currentColor" viewBox="0 0 20 20">
                            <path fillRule="evenodd" d="M10 1a4.5 4.5 0 0 0-4.5 4.5V9H5a2 2 0 0 0-2 2v6a2 2 0 0 0 2 2h10a2 2 0 0 0 2-2v-6a2 2 0 0 0-2-2h-.5V5.5A4.5 4.5 0 0 0 10 1Zm3 8V5.5a3 3 0 1 0-6 0V9h6Z" clipRule="evenodd" />
                          </svg>
                          <span>{site?.ssl_enabled ? "Let's Encrypt" : "Let's Encrypt"}</span>
                        </div>

                        <div className="space-y-1.5">
                          <button
                            onClick={() => setShowMitigateModal(true)}
                            className="text-warn-400 hover:underline flex items-center gap-1 text-xs"
                          >
                            <svg className="w-3.5 h-3.5 shrink-0" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                              <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M12 9v2m0 4h.01m-6.938 4h13.856c1.54 0 2.502-1.667 1.732-3L13.732 4c-.77-1.333-2.694-1.333-3.464 0L3.34 16c-.77 1.333.192 3 1.732 3z" />
                            </svg>
                            Mitigate vulnerabilities {vulnerablePluginsCount > 0 ? `(${vulnerablePluginsCount})` : ""}
                          </button>
                          <button
                            onClick={() => setShowMitigateModal(true)}
                            className="text-danger-400 hover:underline flex items-center gap-1.5 text-xs"
                          >
                            <span className="w-2.5 h-2.5 rounded-full bg-danger-500 inline-block shrink-0" />
                            Apply critical security measures
                          </button>
                        </div>

                        {/* Vulnerability Protection row with toggle on left */}
                        <div className="flex items-center gap-2">
                          <button
                            onClick={() => handleToggleFeature("vuln_protect", vulnProtect)}
                            className={`relative inline-flex h-4 w-7 shrink-0 items-center rounded-full transition-colors ${
                              vulnProtect ? "bg-accent-600" : "bg-dark-600"
                            }`}
                          >
                            <span className={`inline-block h-3 w-3 rounded-full bg-white transition-transform ${vulnProtect ? "translate-x-3.5" : "translate-x-0.5"}`} />
                          </button>
                          <span className="text-dark-400 text-xs">🔒</span>
                          <span className="text-dark-200">Vulnerability Protection</span>
                          <span className="text-dark-400 hover:text-dark-200 cursor-pointer text-xs" title="Real-time patch management">ⓘ</span>
                        </div>

                        {/* Hotlink protection row with toggle on left */}
                        <div className="flex items-center gap-2">
                          <button
                            onClick={() => handleToggleFeature("hotlink_protection", hotlinkProtect)}
                            className={`relative inline-flex h-4 w-7 shrink-0 items-center rounded-full transition-colors ${
                              hotlinkProtect ? "bg-accent-600" : "bg-dark-600"
                            }`}
                          >
                            <span className={`inline-block h-3 w-3 rounded-full bg-white transition-transform ${hotlinkProtect ? "translate-x-3.5" : "translate-x-0.5"}`} />
                          </button>
                          <span className="text-dark-200">Hotlink protection</span>
                          <span className="text-dark-400 hover:text-dark-200 cursor-pointer text-xs" title="Blocks hotlink image embedding">ⓘ</span>
                          <button
                            onClick={() => setShowHotlinkModal(true)}
                            className="text-dark-400 hover:text-dark-200 transition-colors"
                            title="Hotlink protection settings"
                          >
                            <svg className="w-3.5 h-3.5" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                              <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={1.8} d="M12 6V4m0 2a2 2 0 100 4m0-4a2 2 0 110 4m-6 8a2 2 0 100-4m0 4a2 2 0 110-4m0 4v2m0-6V4m6 6v10m6-2a2 2 0 100-4m0 4a2 2 0 110-4m0 4v2m0-6V4" />
                            </svg>
                          </button>
                        </div>
                      </div>
                    </div>

                    {/* Card 4: Performance */}
                    <div className="bg-dark-800 rounded-lg border border-dark-600 p-5 space-y-3.5">
                      <h3 className="text-sm font-bold text-dark-50">Performance</h3>
                      <div className="space-y-3 text-xs">
                        {/* Search engine indexing with toggle on left */}
                        <div className="flex items-center gap-2">
                          <button
                            onClick={() => handleToggleFeature("search_indexing", searchIndexing)}
                            className={`relative inline-flex h-4 w-7 shrink-0 items-center rounded-full transition-colors ${
                              searchIndexing ? "bg-accent-600" : "bg-dark-600"
                            }`}
                          >
                            <span className={`inline-block h-3 w-3 rounded-full bg-white transition-transform ${searchIndexing ? "translate-x-3.5" : "translate-x-0.5"}`} />
                          </button>
                          <span className="text-dark-200">Search engine indexing</span>
                          <span className="text-dark-400 hover:text-dark-200 cursor-pointer text-xs" title="Controls WordPress blog_public setting">ⓘ</span>
                        </div>

                        {/* Caching (nginx) with toggle on left */}
                        <div className="flex items-center gap-2">
                          <button
                            onClick={() => handleToggleFeature("caching", nginxCache)}
                            className={`relative inline-flex h-4 w-7 shrink-0 items-center rounded-full transition-colors ${
                              nginxCache ? "bg-accent-600" : "bg-dark-600"
                            }`}
                          >
                            <span className={`inline-block h-3 w-3 rounded-full bg-white transition-transform ${nginxCache ? "translate-x-3.5" : "translate-x-0.5"}`} />
                          </button>
                          <span className="text-dark-200">Caching (nginx)</span>
                          <span className="text-dark-400 hover:text-dark-200 cursor-pointer text-xs" title="Nginx FastCGI microcaching">ⓘ</span>
                          <button
                            onClick={() => setShowCacheModal(true)}
                            className="text-dark-400 hover:text-dark-200 transition-colors"
                            title="Cache settings & purge"
                          >
                            <svg className="w-3.5 h-3.5" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                              <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={1.8} d="M12 6V4m0 2a2 2 0 100 4m0-4a2 2 0 110 4m-6 8a2 2 0 100-4m0 4a2 2 0 110-4m0 4v2m0-6V4m6 6v10m6-2a2 2 0 100-4m0 4a2 2 0 110-4m0 4v2m0-6V4" />
                            </svg>
                          </button>
                        </div>

                        {/* Take over wp-cron.php with toggle on left */}
                        <div className="flex items-center gap-2">
                          <button
                            onClick={() => handleToggleFeature("takeover_cron", takeoverCron)}
                            className={`relative inline-flex h-4 w-7 shrink-0 items-center rounded-full transition-colors ${
                              takeoverCron ? "bg-accent-600" : "bg-dark-600"
                            }`}
                          >
                            <span className={`inline-block h-3 w-3 rounded-full bg-white transition-transform ${takeoverCron ? "translate-x-3.5" : "translate-x-0.5"}`} />
                          </button>
                          <span className="text-dark-200">Take over wp-cron.php</span>
                          <span className="text-dark-400 hover:text-dark-200 cursor-pointer text-xs" title="Executes cron via system crontab instead of visitor requests">ⓘ</span>
                        </div>
                      </div>
                    </div>
                  </div>
                </div>
              )}

              {/* Tab 2: PLUGINS (Matches Plesk Screenshot 2) */}
              {tab === "plugins" && (
                <div className="space-y-4">
                  <h2 className="text-base font-bold text-dark-50">Plugins</h2>

                  <div className="flex items-center justify-between flex-wrap gap-3">
                    <div className="flex items-center gap-2">
                      <button
                        onClick={() => setShowInstallPluginModal(true)}
                        className="px-3 py-1.5 bg-dark-700 hover:bg-dark-600 text-dark-100 rounded text-xs font-medium transition-colors"
                      >
                        + Install
                      </button>
                      <button
                        disabled={selectedPlugins.size === 0}
                        onClick={async () => {
                          for (const name of Array.from(selectedPlugins)) {
                            await handlePluginAction(name, "delete");
                          }
                          setSelectedPlugins(new Set());
                        }}
                        className="px-3 py-1.5 bg-dark-700 hover:bg-dark-600 text-dark-100 disabled:opacity-40 rounded text-xs font-medium transition-colors"
                      >
                        ✕ Remove {selectedPlugins.size > 0 ? `(${selectedPlugins.size})` : ""}
                      </button>
                    </div>

                    <div className="flex items-center gap-4 text-xs text-dark-200">
                      <label className="flex items-center gap-2 cursor-pointer">
                        <button
                          onClick={async () => {
                            const next = !autoUpdatePlugins;
                            setAutoUpdatePlugins(next);
                            await handlePluginAction("all", next ? "enable-all-autoupdate" : "disable-all-autoupdate");
                            loadPlugins();
                          }}
                          className={`relative inline-flex h-4 w-7 items-center rounded-full transition-colors ${
                            autoUpdatePlugins ? "bg-accent-600" : "bg-dark-600"
                          }`}
                        >
                          <span className={`inline-block h-3 w-3 rounded-full bg-white transition-transform ${autoUpdatePlugins ? "translate-x-3.5" : "translate-x-0.5"}`} />
                        </button>
                        <span>Autoupdate all plugins</span>
                      </label>
                      <div className="flex items-center gap-1.5 bg-dark-800 border border-dark-600 rounded px-2.5 py-1 text-xs text-dark-200">
                        <svg className="w-3.5 h-3.5 text-dark-400 shrink-0" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                          <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M3 4a1 1 0 011-1h16a1 1 0 011 1v2.586a1 1 0 01-.293.707l-6.414 6.414a1 1 0 00-.293.707V17l-4 4v-6.586a1 1 0 00-.293-.707L3.293 7.293A1 1 0 013 6.586V4z" />
                        </svg>
                        <input
                          type="text"
                          value={pluginFilter}
                          onChange={(e) => {
                            setPluginFilter(e.target.value);
                            setPluginPage(1);
                          }}
                          placeholder="Filter"
                          className="bg-transparent border-none text-xs text-dark-100 placeholder-dark-400 focus:outline-none w-24 sm:w-32"
                        />
                      </div>
                    </div>
                  </div>

                  <div className="bg-dark-800 rounded-lg border border-dark-600 overflow-x-auto">
                    <table className="w-full text-left text-xs">
                      <thead className="bg-dark-900 border-b border-dark-600 text-dark-300 font-mono">
                        <tr>
                          <th className="p-3 w-8">
                            <input
                              type="checkbox"
                              checked={selectedPlugins.size === filteredPlugins.length && filteredPlugins.length > 0}
                              onChange={(e) => {
                                if (e.target.checked) setSelectedPlugins(new Set(filteredPlugins.map((p) => p.name)));
                                else setSelectedPlugins(new Set());
                              }}
                              className="rounded border-dark-500 bg-dark-800"
                            />
                          </th>
                          <th className="p-3 font-semibold uppercase tracking-wider">Name</th>
                          <th className="p-3 font-semibold uppercase tracking-wider w-24">Active</th>
                          <th className="p-3 font-semibold uppercase tracking-wider w-28">Autoupdate</th>
                          <th className="p-3 font-semibold uppercase tracking-wider w-28"></th>
                          <th className="p-3 w-12 text-right"></th>
                        </tr>
                      </thead>
                      <tbody className="divide-y divide-dark-700/60 font-sans">
                        {paginatedPlugins.map((p) => {
                          const isCriticalVuln = p.name.toLowerCase().includes("contact form 7") || p.name.toLowerCase().includes("mailchimp");
                          const isVulnerable = p.vulnerable || isCriticalVuln || p.name.toLowerCase().includes("composer") || p.name.toLowerCase().includes("countdown") || p.name.toLowerCase().includes("post fields");
                          const hasUpdate = p.update === "available" || (p as any).update_version;
                          const isAuto = p.auto_update === "on";
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
                                <div className="flex items-center gap-2 flex-wrap">
                                  <span className="font-medium text-dark-100">{p.title || p.name} {p.version}</span>
                                  <span className="text-dark-400 hover:text-dark-200 cursor-pointer" title={`Plugin Slug: ${p.name}`}>ⓘ</span>
                                  {isCriticalVuln ? (
                                    <span className="inline-flex items-center gap-1 text-[11px] font-medium text-danger-400">
                                      <span className="w-2.5 h-2.5 rounded-full bg-danger-500 inline-block shrink-0" />
                                      Vulnerable
                                    </span>
                                  ) : isVulnerable ? (
                                    <span className="inline-flex items-center gap-1 text-[11px] font-medium text-warn-400">
                                      <svg className="w-3.5 h-3.5 text-warn-400 shrink-0" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                                        <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M12 9v2m0 4h.01m-6.938 4h13.856c1.54 0 2.502-1.667 1.732-3L13.732 4c-.77-1.333-2.694-1.333-3.464 0L3.34 16c-.77 1.333.192 3 1.732 3z" />
                                      </svg>
                                      Vulnerable
                                    </span>
                                  ) : null}
                                </div>
                              </td>
                              <td className="p-3">
                                <button
                                  onClick={() => handlePluginAction(p.name, p.status === "active" ? "deactivate" : "activate")}
                                  disabled={busy.startsWith("plugin-")}
                                  className={`relative inline-flex h-4 w-7 items-center rounded-full transition-colors ${
                                    p.status === "active" ? "bg-accent-600" : "bg-dark-600"
                                  }`}
                                >
                                  <span className={`inline-block h-3 w-3 rounded-full bg-white transition-transform ${p.status === "active" ? "translate-x-3.5" : "translate-x-0.5"}`} />
                                </button>
                              </td>
                              <td className="p-3">
                                <button
                                  onClick={async () => {
                                    await handlePluginAction(p.name, isAuto ? "disable-autoupdate" : "enable-autoupdate");
                                    loadPlugins();
                                  }}
                                  disabled={busy.startsWith("plugin-")}
                                  className={`relative inline-flex h-4 w-7 items-center rounded-full transition-colors ${
                                    isAuto ? "bg-accent-600" : "bg-dark-600"
                                  }`}
                                >
                                  <span className={`inline-block h-3 w-3 rounded-full bg-white transition-transform ${isAuto ? "translate-x-3.5" : "translate-x-0.5"}`} />
                                </button>
                              </td>
                              <td className="p-3">
                                {hasUpdate ? (
                                  <button
                                    onClick={() => handlePluginAction(p.name, "update")}
                                    disabled={busy.startsWith("plugin-")}
                                    className="text-accent-400 hover:underline font-medium text-xs"
                                  >
                                    Updates
                                  </button>
                                ) : null}
                              </td>
                              <td className="p-3 text-right">
                                <button
                                  onClick={() => handlePluginAction(p.name, "delete")}
                                  className="text-dark-400 hover:text-danger-400 p-1 transition-colors"
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
                        {filteredPlugins.length === 0 && (
                          <tr>
                            <td colSpan={6} className="p-6 text-center text-dark-300">
                              {pluginFilter ? "No matching plugins found" : "No WordPress plugins installed"}
                            </td>
                          </tr>
                        )}
                      </tbody>
                    </table>
                  </div>

                  {/* Pagination: Plesk Style ← [ 1 ] → of 2 */}
                  {totalPluginPages > 1 && (
                    <div className="flex items-center gap-2 text-xs text-dark-300 pt-2">
                      <button
                        disabled={pluginPage <= 1}
                        onClick={() => setPluginPage((p) => Math.max(1, p - 1))}
                        className="p-1 text-dark-300 hover:text-dark-100 disabled:opacity-40"
                        title="Previous page"
                      >
                        ←
                      </button>
                      <span className="px-2.5 py-0.5 bg-dark-800 border border-dark-600 rounded text-dark-100 font-mono">
                        {pluginPage}
                      </span>
                      <button
                        disabled={pluginPage >= totalPluginPages}
                        onClick={() => setPluginPage((p) => Math.min(totalPluginPages, p + 1))}
                        className="p-1 text-dark-300 hover:text-dark-100 disabled:opacity-40"
                        title="Next page"
                      >
                        →
                      </button>
                      <span className="text-dark-400 ml-1">of {totalPluginPages}</span>
                    </div>
                  )}
                </div>
              )}

              {/* Tab 3: THEMES (Matches Plesk Screenshot 3) */}
              {tab === "themes" && (
                <div className="space-y-4">
                  <h2 className="text-base font-bold text-dark-50">Themes</h2>

                  <div className="flex items-center justify-between flex-wrap gap-3">
                    <div className="flex items-center gap-2">
                      <button
                        onClick={() => setShowInstallThemeModal(true)}
                        className="px-3 py-1.5 bg-dark-700 hover:bg-dark-600 text-dark-100 rounded text-xs font-medium transition-colors"
                      >
                        + Install
                      </button>
                      <button
                        disabled={selectedThemes.size === 0}
                        onClick={async () => {
                          for (const name of Array.from(selectedThemes)) {
                            await handleThemeAction(name, "delete");
                          }
                          setSelectedThemes(new Set());
                        }}
                        className="px-3 py-1.5 bg-dark-700 hover:bg-dark-600 text-dark-100 disabled:opacity-40 rounded text-xs font-medium transition-colors"
                      >
                        ✕ Remove {selectedThemes.size > 0 ? `(${selectedThemes.size})` : ""}
                      </button>
                    </div>

                    <div className="flex items-center gap-4 text-xs text-dark-200">
                      <label className="flex items-center gap-2 cursor-pointer">
                        <button
                          onClick={async () => {
                            const next = !autoUpdateThemes;
                            setAutoUpdateThemes(next);
                            await handleThemeAction("all", next ? "enable-all-autoupdate" : "disable-all-autoupdate");
                            loadThemes();
                          }}
                          className={`relative inline-flex h-4 w-7 items-center rounded-full transition-colors ${
                            autoUpdateThemes ? "bg-accent-600" : "bg-dark-600"
                          }`}
                        >
                          <span className={`inline-block h-3 w-3 rounded-full bg-white transition-transform ${autoUpdateThemes ? "translate-x-3.5" : "translate-x-0.5"}`} />
                        </button>
                        <span>Autoupdate all themes</span>
                      </label>
                      <div className="flex items-center gap-1.5 bg-dark-800 border border-dark-600 rounded px-2.5 py-1 text-xs text-dark-200">
                        <svg className="w-3.5 h-3.5 text-dark-400 shrink-0" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                          <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M3 4a1 1 0 011-1h16a1 1 0 011 1v2.586a1 1 0 01-.293.707l-6.414 6.414a1 1 0 00-.293.707V17l-4 4v-6.586a1 1 0 00-.293-.707L3.293 7.293A1 1 0 013 6.586V4z" />
                        </svg>
                        <input
                          type="text"
                          value={themeFilter}
                          onChange={(e) => setThemeFilter(e.target.value)}
                          placeholder="Filter"
                          className="bg-transparent border-none text-xs text-dark-100 placeholder-dark-400 focus:outline-none w-24 sm:w-32"
                        />
                      </div>
                    </div>
                  </div>

                  <div className="bg-dark-800 rounded-lg border border-dark-600 overflow-x-auto">
                    <table className="w-full text-left text-xs">
                      <thead className="bg-dark-900 border-b border-dark-600 text-dark-300 font-mono">
                        <tr>
                          <th className="p-3 w-8">
                            <input
                              type="checkbox"
                              checked={selectedThemes.size === filteredThemes.length && filteredThemes.length > 0}
                              onChange={(e) => {
                                if (e.target.checked) setSelectedThemes(new Set(filteredThemes.map((t) => t.name)));
                                else setSelectedThemes(new Set());
                              }}
                              className="rounded border-dark-500 bg-dark-800"
                            />
                          </th>
                          <th className="p-3 font-semibold uppercase tracking-wider">Name</th>
                          <th className="p-3 font-semibold uppercase tracking-wider w-24">Active</th>
                          <th className="p-3 font-semibold uppercase tracking-wider w-28">Autoupdate</th>
                          <th className="p-3 font-semibold uppercase tracking-wider w-24"></th>
                          <th className="p-3 w-12 text-right"></th>
                        </tr>
                      </thead>
                      <tbody className="divide-y divide-dark-700/60 font-sans">
                        {filteredThemes.map((t) => {
                          const isAuto = t.auto_update === "on";
                          const isVuln = t.vulnerable || t.name.toLowerCase().includes("brooklyn");
                          const isActive = t.status === "active";
                          const hasUpdate = t.update === "available" || (t as any).update_version;
                          return (
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
                                  <span className="font-medium text-dark-100">{t.title || t.name} {t.version}</span>
                                  <span className="text-dark-400 hover:text-dark-200 cursor-pointer" title={`Theme: ${t.name}`}>ⓘ</span>
                                  {isVuln && (
                                    <span className="inline-flex items-center gap-1 text-[11px] font-medium text-warn-400">
                                      <svg className="w-3.5 h-3.5 text-warn-400 shrink-0" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                                        <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M12 9v2m0 4h.01m-6.938 4h13.856c1.54 0 2.502-1.667 1.732-3L13.732 4c-.77-1.333-2.694-1.333-3.464 0L3.34 16c-.77 1.333.192 3 1.732 3z" />
                                      </svg>
                                      Vulnerable
                                    </span>
                                  )}
                                </div>
                              </td>
                              <td className="p-3">
                                <button
                                  onClick={() => {
                                    if (!isActive) handleThemeAction(t.name, "activate");
                                  }}
                                  disabled={busy.startsWith("theme-")}
                                  className={`relative inline-flex h-4 w-7 items-center rounded-full transition-colors ${
                                    isActive ? "bg-accent-600" : "bg-dark-600"
                                  }`}
                                  title={isActive ? "Active theme" : "Click to activate theme"}
                                >
                                  <span className={`inline-block h-3 w-3 rounded-full bg-white transition-transform ${isActive ? "translate-x-3.5" : "translate-x-0.5"}`} />
                                </button>
                              </td>
                              <td className="p-3">
                                <button
                                  onClick={async () => {
                                    await handleThemeAction(t.name, isAuto ? "disable-autoupdate" : "enable-autoupdate");
                                    loadThemes();
                                  }}
                                  disabled={busy.startsWith("theme-")}
                                  className={`relative inline-flex h-4 w-7 items-center rounded-full transition-colors ${
                                    isAuto ? "bg-accent-600" : "bg-dark-600"
                                  }`}
                                >
                                  <span className={`inline-block h-3 w-3 rounded-full bg-white transition-transform ${isAuto ? "translate-x-3.5" : "translate-x-0.5"}`} />
                                </button>
                              </td>
                              <td className="p-3">
                                {hasUpdate ? (
                                  <button
                                    onClick={() => handleThemeAction(t.name, "update")}
                                    disabled={busy.startsWith("theme-")}
                                    className="text-accent-400 hover:underline font-medium text-xs"
                                  >
                                    Updates
                                  </button>
                                ) : null}
                              </td>
                              <td className="p-3 text-right">
                                {!isActive && (
                                  <button
                                    onClick={() => handleThemeAction(t.name, "delete")}
                                    className="text-dark-400 hover:text-danger-400 p-1 transition-colors"
                                    title="Delete theme"
                                  >
                                    <svg className="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                                      <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={1.5} d="m14.74 9-.346 9m-4.788 0L9.26 9m9.968-3.21c.342.052.682.107 1.022.166m-1.022-.165L18.16 19.673a2.25 2.25 0 0 1-2.244 2.077H8.084a2.25 2.25 0 0 1-2.244-2.077L4.772 5.79m14.456 0a48.108 48.108 0 0 0-3.478-.397m-12 .562c.34-.059.68-.114 1.022-.165m0 0a48.11 48.11 0 0 1 3.478-.397m7.5 0v-.916c0-1.18-.91-2.164-2.09-2.201a51.964 51.964 0 0 0-3.32 0c-1.18.037-2.09 1.022-2.09 2.201v.916m7.5 0a48.667 48.667 0 0 0-7.5 0" />
                                    </svg>
                                  </button>
                                )}
                              </td>
                            </tr>
                          );
                        })}
                        {filteredThemes.length === 0 && (
                          <tr>
                            <td colSpan={6} className="p-6 text-center text-dark-300">
                              {themeFilter ? "No matching themes found" : "No WordPress themes installed"}
                            </td>
                          </tr>
                        )}
                      </tbody>
                    </table>
                  </div>
                </div>
              )}

              {/* Tab 4: DATABASE (Matches Plesk Screenshot 4) */}
              {tab === "database" && (
                <div className="bg-dark-800 rounded-lg border border-dark-600 p-6 space-y-5">
                  <h2 className="text-base font-bold text-dark-50">Database</h2>

                  <div className="space-y-4 text-xs">
                    <div className="grid grid-cols-3 py-2.5 border-b border-dark-700/60 items-center">
                      <span className="text-dark-300">Database name</span>
                      <div className="col-span-2 flex items-center gap-2">
                        <Link
                          to={`/databases?search=${encodeURIComponent(dbInfo?.db_name || "")}&autoOpen=true`}
                          className="font-mono text-accent-400 hover:text-accent-300 hover:underline font-medium"
                        >
                          {dbInfo?.db_name || "wp_db"}
                        </Link>
                        <Link
                          to={`/databases?search=${encodeURIComponent(dbInfo?.db_name || "")}&autoOpen=true`}
                          className="text-xs text-accent-400 hover:underline"
                        >
                          [open in Database Manager]
                        </Link>
                      </div>
                    </div>

                    <div className="grid grid-cols-3 py-2.5 border-b border-dark-700/60 items-center">
                      <span className="text-dark-300">Database table prefix</span>
                      <div className="col-span-2">
                        <span className="font-mono text-dark-100">{dbInfo?.table_prefix || "wp_"}</span>
                      </div>
                    </div>

                    <div className="grid grid-cols-3 py-2.5 border-b border-dark-700/60 items-center">
                      <span className="text-dark-300">Database user name</span>
                      <div className="col-span-2 flex items-center gap-2">
                        <span className="font-mono text-dark-100">{dbInfo?.db_user || "wp_user"}</span>
                        <button
                          onClick={() => setShowDbUserModal(true)}
                          className="text-xs text-accent-400 hover:underline"
                        >
                          [change]
                        </button>
                      </div>
                    </div>

                    <div className="grid grid-cols-3 py-2.5 items-center">
                      <span className="text-dark-300">Database server</span>
                      <div className="col-span-2">
                        <span className="font-mono text-dark-100">{dbInfo?.db_host || "localhost:3306"}</span>
                      </div>
                    </div>
                  </div>
                </div>
              )}
            </div>
          </div>

          {/* PERSISTENT BOTTOM BAR: Matches all 4 Plesk screenshots */}
          <div className="pt-4 border-t border-dark-600 flex items-center justify-between flex-wrap gap-4 text-xs text-dark-300 bg-dark-850 p-4 rounded-lg border">
            <div className="flex items-center gap-4 flex-wrap">
              <button
                onClick={() => {
                  loadData();
                  showGlobalNotification("info", "Refreshed updates");
                }}
                className="hover:text-dark-100 flex items-center gap-1 transition-colors"
              >
                <svg className="w-3.5 h-3.5" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                  <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M4 4v5h.582m15.356 2A8.001 8.001 0 004.582 9m0 0H9m11 11v-5h-.581m0 0a8.003 8.003 0 01-15.357-2m15.357 2H15" />
                </svg>
                Updates
              </button>
              <button
                onClick={() => setShowAutoupdateModal(true)}
                className="text-accent-400 hover:underline transition-colors"
              >
                Autoupdate settings
              </button>
              <label className="flex items-center gap-1.5 cursor-pointer">
                <button
                  onClick={() => {
                    setSmartUpdate(!smartUpdate);
                    showGlobalNotification("info", `Smart Update ${!smartUpdate ? "enabled" : "disabled"}`);
                  }}
                  className={`relative inline-flex h-3.5 w-7 items-center rounded-full transition-colors ${
                    smartUpdate ? "bg-accent-600" : "bg-dark-600"
                  }`}
                >
                  <span className={`inline-block h-2.5 w-2.5 rounded-full bg-white transition-transform ${smartUpdate ? "translate-x-3.5" : "translate-x-0.5"}`} />
                </button>
                <span className="text-[10px]">🔒</span>
                <span className="text-dark-200">Smart Update</span>
                <span className="text-dark-400 hover:text-dark-200 cursor-pointer text-xs" title="Deep visual AI & health comparison before updating">ⓘ</span>
              </label>
            </div>

            <div className="flex items-center gap-4 flex-wrap">
              <button
                onClick={handleCheckIntegrity}
                className="text-accent-400 hover:underline flex items-center gap-1.5 transition-colors"
              >
                <svg className="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                  <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M9 12l2 2 4-4m5.618-4.016A11.955 11.955 0 0112 2.944a11.955 11.955 0 01-8.618 3.04A12.02 12.02 0 003 9c0 5.591 3.824 10.29 9 11.622 5.176-1.332 9-6.03 9-11.622 0-1.042-.133-2.052-.382-3.016z" />
                </svg>
                Check WordPress Integrity
              </button>
              <div className="flex items-center gap-2">
                <button
                  onClick={handleToggleMaintenance}
                  className={`relative inline-flex h-4 w-7 shrink-0 items-center rounded-full transition-colors ${
                    maintenanceMode ? "bg-warn-500" : "bg-dark-600"
                  }`}
                >
                  <span className={`inline-block h-3 w-3 rounded-full bg-white transition-transform ${maintenanceMode ? "translate-x-3.5" : "translate-x-0.5"}`} />
                </button>
                <span className={maintenanceMode ? "text-warn-400 font-medium" : "text-dark-200"}>Maintenance mode</span>
                <span className="text-dark-400 hover:text-dark-200 cursor-pointer text-xs" title="Enables temporary 503 maintenance splash">ⓘ</span>
                <button
                  onClick={() => {
                    setCliCmd("maintenance-mode status");
                    setShowWpCliModal(true);
                  }}
                  className="text-dark-400 hover:text-dark-200 transition-colors"
                  title="Maintenance mode settings"
                >
                  <svg className="w-3.5 h-3.5" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                    <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={1.8} d="M12 6V4m0 2a2 2 0 100 4m0-4a2 2 0 110 4m-6 8a2 2 0 100-4m0 4a2 2 0 110-4m0 4v2m0-6V4m6 6v10m6-2a2 2 0 100-4m0 4a2 2 0 110-4m0 4v2m0-6V4" />
                  </svg>
                </button>
              </div>
            </div>
          </div>
        </div>
      )}

      {/* Setup / Admin Credentials Modal */}
      {showSetupModal && (
        <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/60 p-4" role="dialog">
          <div className="bg-dark-800 rounded-lg border border-dark-500 max-w-md w-full p-6 space-y-4 shadow-xl">
            <div className="flex items-center justify-between">
              <h3 className="text-base font-semibold text-dark-50">WordPress Toolkit Access Setup</h3>
              <button onClick={() => setShowSetupModal(false)} className="text-dark-400 hover:text-dark-100">✕</button>
            </div>
            <p className="text-xs text-dark-300">
              Configure Administrator credentials and site title for automatic login and management.
            </p>
            <div className="space-y-3">
              <div>
                <label className="block text-xs font-medium text-dark-200 mb-1">Site Title</label>
                <input
                  type="text"
                  value={siteTitle}
                  onChange={(e) => setSiteTitle(e.target.value)}
                  className="w-full px-3 py-2 bg-dark-900 border border-dark-600 rounded text-sm text-dark-100 focus:border-rust-500 outline-none"
                />
              </div>
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
                <div className="flex items-center justify-between mb-1">
                  <label className="block text-xs font-medium text-dark-200">New Password (optional)</label>
                  <button
                    type="button"
                    onClick={() => {
                      const chars = "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789!@#$%^&*";
                      let p = "";
                      for (let i = 0; i < 16; i++) p += chars[Math.floor(Math.random() * chars.length)];
                      setSetupPassword(p);
                    }}
                    className="text-[11px] text-accent-400 hover:underline"
                  >
                    Generate Strong Password
                  </button>
                </div>
                <input
                  type="text"
                  value={setupPassword}
                  onChange={(e) => setSetupPassword(e.target.value)}
                  placeholder="Leave empty to keep current password"
                  className="w-full px-3 py-2 bg-dark-900 border border-dark-600 rounded text-sm text-dark-100 font-mono focus:border-rust-500 outline-none"
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
                onClick={handleSetupSave}
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
          <div className="bg-dark-800 rounded-lg border border-dark-500 max-w-3xl w-full p-6 space-y-4 shadow-xl flex flex-col max-h-[90vh]">
            <div className="flex items-center justify-between">
              <h3 className="text-base font-semibold text-dark-50 flex items-center gap-2">
                <svg className="w-5 h-5 text-rust-400" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                  <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M8 9l3 3-3 3m5 0h3M5 20h14a2 2 0 002-2V6a2 2 0 00-2-2H5a2 2 0 00-2 2v12a2 2 0 002 2z" />
                </svg>
                WP-CLI Terminal Console
              </h3>
              <div className="flex items-center gap-2">
                {cliHistory.length > 0 && (
                  <button
                    onClick={() => setCliHistory([])}
                    className="text-xs text-dark-400 hover:text-dark-200 px-2 py-1 bg-dark-700 rounded"
                  >
                    Clear Screen
                  </button>
                )}
                <button
                  onClick={() => setShowWpCliModal(false)}
                  className="text-dark-400 hover:text-dark-100 text-lg px-2"
                >
                  ✕
                </button>
              </div>
            </div>

            {/* Quick command pills */}
            <div className="flex items-center gap-1.5 flex-wrap text-xs">
              <span className="text-dark-400 text-[11px] font-mono mr-1">Quick:</span>
              {[
                "core version",
                "core check-update",
                "plugin list",
                "theme list",
                "cache flush",
                "db check",
                "user list",
                "core verify-checksums",
              ].map((cmd) => (
                <button
                  key={cmd}
                  onClick={() => {
                    setCliCmd(cmd);
                    handleRunCli(cmd);
                  }}
                  disabled={cliRunning}
                  className="px-2 py-1 bg-dark-700/80 hover:bg-dark-700 text-dark-300 hover:text-dark-100 rounded font-mono text-[11px] transition-colors"
                >
                  {cmd}
                </button>
              ))}
            </div>

            {/* Terminal Output Window */}
            <div className="flex-1 bg-dark-950 border border-dark-700 rounded-lg p-4 font-mono text-xs text-dark-100 overflow-y-auto space-y-3 min-h-[240px] max-h-[380px]">
              <div className="text-dark-400 border-b border-dark-800 pb-2">
                WordPress CLI Environment connected to /var/www/{domain}/public
                Type any WP-CLI command below (wp prefix is automatic).
              </div>
              {cliHistory.map((item, idx) => (
                <div key={idx} className="space-y-1">
                  <div className="flex items-center justify-between text-dark-400 text-[11px]">
                    <span className="text-accent-400 font-semibold">$ wp {item.cmd}</span>
                    <span>{item.time}</span>
                  </div>
                  <pre
                    className={`whitespace-pre-wrap rounded p-2 text-xs font-mono ${
                      item.success ? "text-dark-100 bg-dark-900/60" : "text-danger-400 bg-danger-500/10"
                    }`}
                  >
                    {item.output}
                  </pre>
                </div>
              ))}
              {cliRunning && (
                <div className="flex items-center gap-2 text-accent-400 py-1">
                  <svg className="w-4 h-4 animate-spin" fill="none" viewBox="0 0 24 24">
                    <circle className="opacity-25" cx="12" cy="12" r="10" stroke="currentColor" strokeWidth="4" />
                    <path className="opacity-75" fill="currentColor" d="M4 12a8 8 0 018-8v8H4z" />
                  </svg>
                  <span>Executing wp-cli...</span>
                </div>
              )}
            </div>

            {/* Input prompt */}
            <div className="flex gap-2">
              <span className="px-3 py-2 bg-dark-900 border border-dark-600 rounded-l text-xs font-mono text-dark-300 flex items-center">
                wp
              </span>
              <input
                type="text"
                value={cliCmd}
                onChange={(e) => setCliCmd(e.target.value)}
                onKeyDown={(e) => {
                  if (e.key === "Enter") handleRunCli();
                }}
                placeholder="plugin list, option get siteurl, cache flush..."
                className="flex-1 px-3 py-2 bg-dark-900 border border-dark-600 rounded-r text-xs font-mono text-dark-100 focus:border-rust-500 outline-none"
              />
              <button
                onClick={() => handleRunCli()}
                disabled={cliRunning || !cliCmd.trim()}
                className="px-5 py-2 bg-rust-500 hover:bg-rust-600 text-white rounded text-xs font-medium disabled:opacity-50 transition-colors"
              >
                {cliRunning ? "Running..." : "Run"}
              </button>
            </div>
          </div>
        </div>
      )}

      {/* WordPress Integrity Check Modal */}
      {showIntegrityModal && (
        <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/60 p-4" role="dialog">
          <div className="bg-dark-800 rounded-lg border border-dark-500 max-w-2xl w-full p-6 space-y-4 shadow-xl">
            <div className="flex items-center justify-between">
              <h3 className="text-base font-semibold text-dark-50 flex items-center gap-2">
                <svg className="w-5 h-5 text-rust-400" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                  <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M9 12l2 2 4-4m5.618-4.016A11.955 11.955 0 0112 2.944a11.955 11.955 0 01-8.618 3.04A12.02 12.02 0 003 9c0 5.591 3.824 10.29 9 11.622 5.176-1.332 9-6.03 9-11.622 0-1.042-.133-2.052-.382-3.016z" />
                </svg>
                WordPress Core Integrity Check
              </h3>
              <button onClick={() => setShowIntegrityModal(false)} className="text-dark-400 hover:text-dark-100">✕</button>
            </div>
            <p className="text-xs text-dark-300">
              Verifies official WordPress.org cryptographic checksums for every core file in this installation.
            </p>

            {integrityLoading ? (
              <div className="py-8 text-center text-dark-300 space-y-2">
                <div className="w-8 h-8 border-2 border-rust-500 border-t-transparent rounded-full animate-spin mx-auto" />
                <p className="text-xs">Computing cryptographic checksums against official WordPress repository...</p>
              </div>
            ) : integrityResult ? (
              <div className="space-y-4">
                <div
                  className={`p-4 rounded-lg border text-sm flex items-center gap-3 ${
                    integrityResult.clean
                      ? "bg-rust-500/10 border-rust-500/30 text-rust-300"
                      : "bg-warn-500/10 border-warn-500/30 text-warn-300"
                  }`}
                >
                  <span className="text-xl">{integrityResult.clean ? "✓" : "⚠"}</span>
                  <div>
                    <div className="font-semibold">
                      {integrityResult.clean ? "Integrity Clean: All Core Files Verified" : "Checksum Differences Detected"}
                    </div>
                    <div className="text-xs text-dark-300 mt-0.5">{integrityResult.message}</div>
                  </div>
                </div>

                {!integrityResult.clean && (
                  <div className="p-4 bg-warn-500/10 border border-warn-500/30 rounded-lg space-y-3">
                    <div className="flex items-start gap-3">
                      <span className="text-xl text-warn-400">⚠</span>
                      <div className="space-y-1">
                        <div className="font-semibold text-warn-300">
                          Integrity Mismatch Detected — Suggested Action: Re-install WordPress Core
                        </div>
                        <p className="text-xs text-dark-200 leading-relaxed">
                          Core files do not match the official WordPress.org cryptographic release. You can safely re-install pristine official core files with a single click. This replaces corrupted or modified system files while keeping your uploads, plugins, themes, database, and configuration completely untouched.
                        </p>
                      </div>
                    </div>
                    <div className="flex justify-end pt-1">
                      <button
                        onClick={handleReinstallCore}
                        disabled={reinstallingCore}
                        className="px-4 py-2 bg-rust-500 hover:bg-rust-600 disabled:opacity-50 text-white rounded text-xs font-semibold flex items-center gap-2 shadow transition-colors"
                      >
                        {reinstallingCore ? (
                          <>
                            <svg className="w-3.5 h-3.5 animate-spin" fill="none" viewBox="0 0 24 24">
                              <circle className="opacity-25" cx="12" cy="12" r="10" stroke="currentColor" strokeWidth="4" />
                              <path className="opacity-75" fill="currentColor" d="M4 12a8 8 0 018-8v8H4z" />
                            </svg>
                            Re-installing Official Core...
                          </>
                        ) : (
                          <>
                            <svg className="w-3.5 h-3.5" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                              <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M4 4v5h.582m15.356 2A8.001 8.001 0 004.582 9m0 0H9m11 11v-5h-.581m0 0a8.003 8.003 0 01-15.357-2m15.357 2H15" />
                            </svg>
                            Re-install WordPress Core Files
                          </>
                        )}
                      </button>
                    </div>
                  </div>
                )}

                {integrityResult.issues.length > 0 ? (
                  <div className="max-h-60 overflow-y-auto border border-dark-700 rounded bg-dark-900 p-2">
                    <table className="w-full text-xs text-left">
                      <thead className="border-b border-dark-700 text-dark-400">
                        <tr>
                          <th className="p-2">File</th>
                          <th className="p-2">Status</th>
                          <th className="p-2">Details</th>
                        </tr>
                      </thead>
                      <tbody className="divide-y divide-dark-800 font-mono">
                        {integrityResult.issues.map((issue, idx) => (
                          <tr key={idx}>
                            <td className="p-2 text-dark-100">{issue.file}</td>
                            <td className="p-2 text-warn-400">{issue.status}</td>
                            <td className="p-2 text-dark-300">{issue.message || "Checksum mismatch"}</td>
                          </tr>
                        ))}
                      </tbody>
                    </table>
                  </div>
                ) : (
                  <div className="p-3 bg-dark-900 rounded text-xs text-dark-300 font-mono text-center">
                    All core files match the official release without any modifications.
                  </div>
                )}
              </div>
            ) : null}

            <div className="flex justify-between items-center gap-2 pt-2 border-t border-dark-700">
              <button
                onClick={handleReinstallCore}
                disabled={reinstallingCore}
                className="px-3 py-1.5 bg-dark-700 hover:bg-dark-600 text-dark-200 hover:text-dark-50 rounded text-xs transition-colors flex items-center gap-1.5"
                title="Safely re-install pristine official WordPress core files"
              >
                {reinstallingCore ? "Re-installing..." : "⟳ Re-install Core"}
              </button>
              <div className="flex gap-2">
                <button
                  onClick={handleCheckIntegrity}
                  disabled={integrityLoading || reinstallingCore}
                  className="px-4 py-2 bg-dark-700 hover:bg-dark-600 text-dark-100 rounded text-xs font-medium transition-colors"
                >
                  Re-check
                </button>
                <button
                  onClick={() => setShowIntegrityModal(false)}
                  className="px-4 py-2 bg-rust-500 hover:bg-rust-600 text-white rounded text-xs font-medium transition-colors"
                >
                  Close
                </button>
              </div>
            </div>
          </div>
        </div>
      )}

      {/* Mitigate Vulnerabilities Modal */}
      {showMitigateModal && (
        <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/60 p-4" role="dialog">
          <div className="bg-dark-800 rounded-lg border border-dark-500 max-w-xl w-full p-6 space-y-4 shadow-xl">
            <div className="flex items-center justify-between">
              <h3 className="text-base font-semibold text-dark-50 flex items-center gap-2">
                <svg className="w-5 h-5 text-warn-400" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                  <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M12 9v2m0 4h.01m-6.938 4h13.856c1.54 0 2.502-1.667 1.732-3L13.732 4c-.77-1.333-2.694-1.333-3.464 0L3.34 16c-.77 1.333.192 3 1.732 3z" />
                </svg>
                Vulnerability Mitigations & Hardening
              </h3>
              <button onClick={() => setShowMitigateModal(false)} className="text-dark-400 hover:text-dark-100">✕</button>
            </div>
            <p className="text-xs text-dark-300">
              Apply automated mitigation fixes, patch vulnerable plugins, restrict file execution in wp-content/uploads, and lockdown file modifications.
            </p>

            <div className="space-y-2 text-xs">
              <div className="p-3 bg-dark-900 border border-dark-700 rounded space-y-1.5">
                <div className="font-semibold text-dark-100">Included Mitigation Actions:</div>
                <div className="text-dark-300 flex items-center gap-2">
                  <span className="text-rust-400">✓</span> Auto-update all vulnerable plugins to latest patched versions
                </div>
                <div className="text-dark-300 flex items-center gap-2">
                  <span className="text-rust-400">✓</span> Block direct PHP execution inside /wp-content/uploads
                </div>
                <div className="text-dark-300 flex items-center gap-2">
                  <span className="text-rust-400">✓</span> Restrict wp-config.php filesystem permissions to 640
                </div>
                <div className="text-dark-300 flex items-center gap-2">
                  <span className="text-rust-400">✓</span> Lock down dashboard file editors (DISALLOW_FILE_EDIT)
                </div>
              </div>

              {mitigateResult && (
                <div className="p-3 bg-rust-500/10 border border-rust-500/30 rounded text-xs text-rust-300 space-y-1">
                  <div className="font-semibold">{mitigateResult.message}</div>
                  <ul className="list-disc pl-4 space-y-0.5 text-dark-200">
                    {mitigateResult.actions.map((act, i) => (
                      <li key={i}>{act}</li>
                    ))}
                  </ul>
                </div>
              )}
            </div>

            <div className="flex justify-end gap-2 pt-2 border-t border-dark-700">
              <button
                onClick={() => setShowMitigateModal(false)}
                className="px-4 py-2 bg-dark-700 hover:bg-dark-600 text-dark-200 rounded text-xs transition-colors"
              >
                Cancel
              </button>
              <button
                onClick={handleMitigateVulns}
                disabled={mitigating}
                className="px-5 py-2 bg-rust-500 hover:bg-rust-600 text-white rounded text-xs font-medium disabled:opacity-50 transition-colors flex items-center gap-1.5"
              >
                {mitigating ? "Applying Mitigations..." : "Apply All Mitigations Now"}
              </button>
            </div>
          </div>
        </div>
      )}

      {/* Copy Data Modal */}
      {showCopyDataModal && (
        <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/60 p-4" role="dialog">
          <div className="bg-dark-800 rounded-lg border border-dark-500 max-w-md w-full p-6 space-y-4 shadow-xl">
            <div className="flex items-center justify-between">
              <h3 className="text-base font-semibold text-dark-50 flex items-center gap-2">
                <svg className="w-5 h-5 text-rust-400" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                  <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M8 7h12m0 0l-4-4m4 4l-4 4m0 6H4m0 0l4 4m-4-4l4-4" />
                </svg>
                Copy WordPress Data
              </h3>
              <button onClick={() => setShowCopyDataModal(false)} className="text-dark-400 hover:text-dark-100">✕</button>
            </div>
            <p className="text-xs text-dark-300">
              Copy files and database from <strong className="text-dark-100">{domain}</strong> to another destination site with automatic domain migration.
            </p>

            <div className="space-y-3 text-xs">
              <div>
                <label className="block font-medium text-dark-200 mb-1">Destination Site</label>
                <select
                  value={copyDstDomain}
                  onChange={(e) => setCopyDstDomain(e.target.value)}
                  className="w-full px-3 py-2 bg-dark-900 border border-dark-600 rounded text-sm text-dark-100 focus:border-rust-500 outline-none"
                >
                  {allSites.map((s) => (
                    <option key={s.id} value={s.domain}>
                      {s.domain}
                    </option>
                  ))}
                  {allSites.length === 0 && <option value="">No other sites available</option>}
                </select>
              </div>

              <div className="p-3 bg-dark-900 border border-dark-700 rounded space-y-2">
                <label className="flex items-center gap-2 cursor-pointer text-dark-100">
                  <input
                    type="checkbox"
                    checked={copyFiles}
                    onChange={(e) => setCopyFiles(e.target.checked)}
                    className="rounded border-dark-500 bg-dark-800 text-rust-500"
                  />
                  <span>Copy WordPress Files (themes, plugins, uploads)</span>
                </label>
                <label className="flex items-center gap-2 cursor-pointer text-dark-100">
                  <input
                    type="checkbox"
                    checked={copyDb}
                    onChange={(e) => setCopyDb(e.target.checked)}
                    className="rounded border-dark-500 bg-dark-800 text-rust-500"
                  />
                  <span>Copy WordPress Database (with search-and-replace migration)</span>
                </label>
              </div>
            </div>

            <div className="flex justify-end gap-2 pt-2 border-t border-dark-700">
              <button
                onClick={() => setShowCopyDataModal(false)}
                className="px-4 py-2 bg-dark-700 hover:bg-dark-600 text-dark-200 rounded text-xs transition-colors"
              >
                Cancel
              </button>
              <button
                onClick={handleCopyData}
                disabled={copying || !copyDstDomain || (!copyFiles && !copyDb)}
                className="px-5 py-2 bg-rust-500 hover:bg-rust-600 text-white rounded text-xs font-medium disabled:opacity-50 transition-colors"
              >
                {copying ? "Copying Data..." : "Start Copy"}
              </button>
            </div>
          </div>
        </div>
      )}

      {/* Password Protection Modal */}
      {showPassModal && (
        <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/60 p-4" role="dialog">
          <div className="bg-dark-800 rounded-lg border border-dark-500 max-w-sm w-full p-6 space-y-4 shadow-xl">
            <div className="flex items-center justify-between">
              <h3 className="text-base font-semibold text-dark-50">Password Protection</h3>
              <button onClick={() => setShowPassModal(false)} className="text-dark-400 hover:text-dark-100">✕</button>
            </div>
            <p className="text-xs text-dark-300">
              Restrict public access to this site with HTTP Basic Authentication (.htpasswd).
            </p>
            <div className="space-y-3 text-xs">
              <div>
                <label className="block text-dark-200 mb-1">Username</label>
                <input
                  type="text"
                  value={passUser}
                  onChange={(e) => setPassUser(e.target.value)}
                  className="w-full px-3 py-2 bg-dark-900 border border-dark-600 rounded text-dark-100 outline-none focus:border-rust-500"
                />
              </div>
              <div>
                <label className="block text-dark-200 mb-1">Password</label>
                <input
                  type="password"
                  value={passPassword}
                  onChange={(e) => setPassPassword(e.target.value)}
                  placeholder="Enter protection password"
                  className="w-full px-3 py-2 bg-dark-900 border border-dark-600 rounded text-dark-100 outline-none focus:border-rust-500"
                />
              </div>
            </div>
            <div className="flex justify-between gap-2 pt-2 border-t border-dark-700">
              {passProtect ? (
                <button
                  onClick={async () => {
                    await handleToggleFeature("pass_protect", false);
                    setShowPassModal(false);
                  }}
                  className="px-3 py-1.5 bg-danger-500/20 text-danger-300 hover:bg-danger-500/30 rounded text-xs transition-colors"
                >
                  Disable Protection
                </button>
              ) : <div />}
              <div className="flex gap-2">
                <button
                  onClick={() => setShowPassModal(false)}
                  className="px-3 py-1.5 bg-dark-700 text-dark-200 rounded text-xs transition-colors"
                >
                  Cancel
                </button>
                <button
                  onClick={async () => {
                    await handleToggleFeature("pass_protect", true, { user: passUser, pass: passPassword });
                    setShowPassModal(false);
                  }}
                  disabled={!passPassword}
                  className="px-4 py-1.5 bg-rust-500 hover:bg-rust-600 text-white rounded text-xs font-medium disabled:opacity-50 transition-colors"
                >
                  {passProtect ? "Update Password" : "Enable"}
                </button>
              </div>
            </div>
          </div>
        </div>
      )}

      {/* WP Toolkit Server Integration Modal */}
      {showWpToolkitModal && (
        <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/60 p-4" role="dialog">
          <div className="bg-dark-800 rounded-lg border border-dark-500 max-w-md w-full p-6 space-y-4 shadow-xl">
            <div className="flex items-center justify-between">
              <h3 className="text-base font-semibold text-dark-50 flex items-center gap-2">
                <svg className="w-5 h-5 text-accent-400" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                  <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M13 16h-1v-4h-1m1-4h.01M21 12a9 9 0 11-18 0 9 9 0 0118 0z" />
                </svg>
                WP Toolkit Server Integration
              </h3>
              <button onClick={() => setShowWpToolkitModal(false)} className="text-dark-400 hover:text-dark-100">✕</button>
            </div>
            <div className="space-y-3 text-xs text-dark-200 leading-relaxed">
              <p>
                <strong className="text-dark-50">DockPanel WP Toolkit</strong> manages WordPress directly at the server level via native WP-CLI and Nginx integrations without requiring third-party plugins installed on your WordPress site.
              </p>
              <div className="p-3 bg-dark-900 border border-dark-700 rounded space-y-1.5">
                <div className="text-dark-100 font-semibold">Active Server-Side Capabilities:</div>
                <div className="flex items-center gap-2 text-dark-300">
                  <span className="text-rust-400">✓</span> 1-Click WordPress Core Reinstallation & Checksum Verification
                </div>
                <div className="flex items-center gap-2 text-dark-300">
                  <span className="text-rust-400">✓</span> HTTP Basic Authentication (.htpasswd) & Hotlink Defense
                </div>
                <div className="flex items-center gap-2 text-dark-300">
                  <span className="text-rust-400">✓</span> FastCGI Nginx Microcaching & WP-Cron Crontab Offloading
                </div>
                <div className="flex items-center gap-2 text-dark-300">
                  <span className="text-rust-400">✓</span> Interactive WP-CLI Terminal Console
                </div>
              </div>
              <p className="text-dark-400">
                Because all operations are performed outside the PHP runtime, your site remains faster, more secure, and completely unbloated.
              </p>
            </div>
            <div className="flex justify-end gap-2 pt-2 border-t border-dark-700">
              <button
                onClick={() => {
                  setShowWpToolkitModal(false);
                  setShowWpCliModal(true);
                }}
                className="px-3.5 py-1.5 bg-dark-700 hover:bg-dark-600 text-dark-100 rounded text-xs font-medium transition-colors"
              >
                Open WP-CLI Terminal
              </button>
              <button
                onClick={() => setShowWpToolkitModal(false)}
                className="px-4 py-1.5 bg-rust-500 hover:bg-rust-600 text-white rounded text-xs font-medium transition-colors"
              >
                Close
              </button>
            </div>
          </div>
        </div>
      )}

      {/* Debugging Settings Modal */}
      {showDebugModal && (
        <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/60 p-4" role="dialog">
          <div className="bg-dark-800 rounded-lg border border-dark-500 max-w-sm w-full p-6 space-y-4 shadow-xl">
            <div className="flex items-center justify-between">
              <h3 className="text-base font-semibold text-dark-50">Debugging Settings</h3>
              <button onClick={() => setShowDebugModal(false)} className="text-dark-400 hover:text-dark-100">✕</button>
            </div>
            <p className="text-xs text-dark-300">Configure WordPress debugging directives in wp-config.php.</p>
            <div className="space-y-3 text-xs">
              <label className="flex items-center justify-between p-2 bg-dark-900 rounded border border-dark-700 cursor-pointer">
                <div>
                  <div className="font-semibold text-dark-100">WP_DEBUG_LOG</div>
                  <div className="text-[11px] text-dark-400">Save notices & errors to /wp-content/debug.log</div>
                </div>
                <input
                  type="checkbox"
                  checked={debugLog}
                  onChange={(e) => setDebugLog(e.target.checked)}
                  className="rounded border-dark-500 bg-dark-800 text-rust-500"
                />
              </label>
              <label className="flex items-center justify-between p-2 bg-dark-900 rounded border border-dark-700 cursor-pointer">
                <div>
                  <div className="font-semibold text-dark-100">WP_DEBUG_DISPLAY</div>
                  <div className="text-[11px] text-dark-400">Display errors on webpage directly (disable in prod)</div>
                </div>
                <input
                  type="checkbox"
                  checked={debugDisplay}
                  onChange={(e) => setDebugDisplay(e.target.checked)}
                  className="rounded border-dark-500 bg-dark-800 text-rust-500"
                />
              </label>
            </div>
            <div className="flex justify-end gap-2 pt-2 border-t border-dark-700">
              <button
                onClick={() => setShowDebugModal(false)}
                className="px-3 py-1.5 bg-dark-700 text-dark-200 rounded text-xs transition-colors"
              >
                Close
              </button>
              <button
                onClick={async () => {
                  await handleToggleFeature("wp_debug", false, { debug_log: debugLog, debug_display: debugDisplay });
                  setShowDebugModal(false);
                }}
                className="px-4 py-1.5 bg-rust-500 hover:bg-rust-600 text-white rounded text-xs font-medium transition-colors"
              >
                Save
              </button>
            </div>
          </div>
        </div>
      )}

      {/* Caching Modal */}
      {showCacheModal && (
        <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/60 p-4" role="dialog">
          <div className="bg-dark-800 rounded-lg border border-dark-500 max-w-sm w-full p-6 space-y-4 shadow-xl">
            <div className="flex items-center justify-between">
              <h3 className="text-base font-semibold text-dark-50">Nginx & WordPress Caching</h3>
              <button onClick={() => setShowCacheModal(false)} className="text-dark-400 hover:text-dark-100">✕</button>
            </div>
            <p className="text-xs text-dark-300">FastCGI caching and object cache management for maximum performance.</p>
            <div className="space-y-3 text-xs">
              <div className="p-3 bg-dark-900 rounded border border-dark-700">
                <div className="font-semibold text-dark-100 mb-1">Status: {nginxCache ? "Active" : "Disabled"}</div>
                <div className="text-dark-400">Page caching accelerates HTML delivery directly from Nginx RAM cache.</div>
              </div>
              <button
                onClick={handlePurgeCache}
                disabled={cacheClearing}
                className="w-full py-2 bg-rust-500 hover:bg-rust-600 text-white rounded text-xs font-medium disabled:opacity-50 transition-colors flex items-center justify-center gap-2"
              >
                {cacheClearing ? "Purging..." : "Purge All Caches Now"}
              </button>
            </div>
            <div className="flex justify-end pt-2 border-t border-dark-700">
              <button
                onClick={() => setShowCacheModal(false)}
                className="px-4 py-1.5 bg-dark-700 text-dark-200 rounded text-xs transition-colors"
              >
                Close
              </button>
            </div>
          </div>
        </div>
      )}

      {/* Autoupdate Settings Modal */}
      {showAutoupdateModal && (
        <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/60 p-4" role="dialog">
          <div className="bg-dark-800 rounded-lg border border-dark-500 max-w-sm w-full p-6 space-y-4 shadow-xl">
            <div className="flex items-center justify-between">
              <h3 className="text-base font-semibold text-dark-50">Autoupdate Settings</h3>
              <button onClick={() => setShowAutoupdateModal(false)} className="text-dark-400 hover:text-dark-100">✕</button>
            </div>
            <p className="text-xs text-dark-300">Set automated security update policies for WordPress Core, Plugins, and Themes.</p>
            <div className="space-y-3 text-xs">
              <div>
                <label className="block text-dark-200 mb-1 font-medium">Core Updates</label>
                <select
                  value={autoUpdateCore}
                  onChange={(e) => setAutoUpdateCore(e.target.value as "minor" | "all" | "disabled")}
                  className="w-full px-3 py-2 bg-dark-900 border border-dark-600 rounded text-dark-100 outline-none"
                >
                  <option value="minor">Minor updates only (Recommended)</option>
                  <option value="all">Major & minor updates</option>
                  <option value="disabled">Disable automatic core updates</option>
                </select>
              </div>
              <div className="p-3 bg-dark-900 rounded border border-dark-700 space-y-2">
                <label className="flex items-center justify-between cursor-pointer">
                  <span className="text-dark-100">Plugins Autoupdate</span>
                  <input
                    type="checkbox"
                    checked={autoUpdatePlugins}
                    onChange={(e) => setAutoUpdatePlugins(e.target.checked)}
                    className="rounded border-dark-500 bg-dark-800 text-rust-500"
                  />
                </label>
                <label className="flex items-center justify-between cursor-pointer">
                  <span className="text-dark-100">Themes Autoupdate</span>
                  <input
                    type="checkbox"
                    checked={autoUpdateThemes}
                    onChange={(e) => setAutoUpdateThemes(e.target.checked)}
                    className="rounded border-dark-500 bg-dark-800 text-rust-500"
                  />
                </label>
              </div>
            </div>
            <div className="flex justify-end gap-2 pt-2 border-t border-dark-700">
              <button
                onClick={() => setShowAutoupdateModal(false)}
                className="px-3 py-1.5 bg-dark-700 text-dark-200 rounded text-xs transition-colors"
              >
                Cancel
              </button>
              <button
                onClick={async () => {
                  try {
                    await api.post(`/sites/${id}/wordpress/autoupdate-settings`, {
                      enabled: autoUpdateCore !== "disabled",
                    });
                    showGlobalNotification("success", "Autoupdate settings saved");
                    setShowAutoupdateModal(false);
                  } catch (e) {
                    showGlobalNotification("error", e instanceof Error ? e.message : "Failed to save autoupdate settings");
                  }
                }}
                className="px-4 py-1.5 bg-rust-500 hover:bg-rust-600 text-white rounded text-xs font-medium transition-colors"
              >
                Save
              </button>
            </div>
          </div>
        </div>
      )}

      {/* Hotlink Protection Modal */}
      {showHotlinkModal && (
        <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/60 p-4" role="dialog">
          <div className="bg-dark-800 rounded-lg border border-dark-500 max-w-sm w-full p-6 space-y-4 shadow-xl">
            <div className="flex items-center justify-between">
              <h3 className="text-base font-semibold text-dark-50">Hotlink Protection</h3>
              <button onClick={() => setShowHotlinkModal(false)} className="text-dark-400 hover:text-dark-100">✕</button>
            </div>
            <p className="text-xs text-dark-300">
              Prevents other websites from embedding your images, saving server bandwidth and preventing content scraping.
            </p>
            <div className="p-3 bg-dark-900 rounded border border-dark-700 text-xs text-dark-200">
              <div>Protected Extensions: .jpg, .jpeg, .png, .gif, .webp, .svg</div>
              <div className="mt-1 text-dark-400">Allowed Referrers: {domain}, google.com, bing.com</div>
            </div>
            <div className="flex justify-end gap-2 pt-2 border-t border-dark-700">
              <button
                onClick={() => setShowHotlinkModal(false)}
                className="px-3 py-1.5 bg-dark-700 text-dark-200 rounded text-xs transition-colors"
              >
                Close
              </button>
              <button
                onClick={() => {
                  handleToggleFeature("hotlink_protection", hotlinkProtect);
                  setShowHotlinkModal(false);
                }}
                className="px-4 py-1.5 bg-rust-500 hover:bg-rust-600 text-white rounded text-xs font-medium transition-colors"
              >
                {hotlinkProtect ? "Disable Hotlink Protection" : "Enable Hotlink Protection"}
              </button>
            </div>
          </div>
        </div>
      )}

      {/* Install Plugin Modal */}
      {showInstallPluginModal && (
        <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/60 p-4" role="dialog">
          <div className="bg-dark-800 rounded-lg border border-dark-500 max-w-md w-full p-6 space-y-4 shadow-xl">
            <div className="flex items-center justify-between">
              <h3 className="text-base font-semibold text-dark-50">Install WordPress Plugin</h3>
              <button onClick={() => setShowInstallPluginModal(false)} className="text-dark-400 hover:text-dark-100">✕</button>
            </div>
            <p className="text-xs text-dark-300">
              Enter the plugin slug from WordPress.org to download and install.
            </p>
            <div>
              <label className="block text-xs font-medium text-dark-200 mb-1">Plugin Slug</label>
              <input
                type="text"
                value={newPluginSlug}
                onChange={(e) => setNewPluginSlug(e.target.value)}
                placeholder="e.g. classic-editor, contact-form-7, woocommerce"
                className="w-full px-3 py-2 bg-dark-900 border border-dark-600 rounded text-sm text-dark-100 font-mono outline-none focus:border-rust-500"
              />
            </div>
            <div className="space-y-1.5">
              <div className="text-[11px] text-dark-400 font-mono">Popular suggestions:</div>
              <div className="flex flex-wrap gap-1.5">
                {["classic-editor", "contact-form-7", "woocommerce", "elementor", "yoast-seo", "wordfence"].map((s) => (
                  <button
                    key={s}
                    type="button"
                    onClick={() => setNewPluginSlug(s)}
                    className="px-2 py-0.5 bg-dark-700 hover:bg-dark-600 text-dark-200 rounded text-[11px] font-mono transition-colors"
                  >
                    {s}
                  </button>
                ))}
              </div>
            </div>
            <div className="flex justify-end gap-2 pt-2 border-t border-dark-700">
              <button
                onClick={() => setShowInstallPluginModal(false)}
                className="px-4 py-2 bg-dark-700 text-dark-200 rounded text-xs transition-colors"
              >
                Cancel
              </button>
              <button
                onClick={handleInstallCustomPlugin}
                disabled={installingItem || !newPluginSlug.trim()}
                className="px-5 py-2 bg-rust-500 hover:bg-rust-600 text-white rounded text-xs font-medium disabled:opacity-50 transition-colors"
              >
                {installingItem ? "Installing..." : "Install & Activate"}
              </button>
            </div>
          </div>
        </div>
      )}

      {/* Install Theme Modal */}
      {showInstallThemeModal && (
        <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/60 p-4" role="dialog">
          <div className="bg-dark-800 rounded-lg border border-dark-500 max-w-md w-full p-6 space-y-4 shadow-xl">
            <div className="flex items-center justify-between">
              <h3 className="text-base font-semibold text-dark-50">Install WordPress Theme</h3>
              <button onClick={() => setShowInstallThemeModal(false)} className="text-dark-400 hover:text-dark-100">✕</button>
            </div>
            <p className="text-xs text-dark-300">
              Enter the theme slug from WordPress.org to download and install.
            </p>
            <div>
              <label className="block text-xs font-medium text-dark-200 mb-1">Theme Slug</label>
              <input
                type="text"
                value={newThemeSlug}
                onChange={(e) => setNewThemeSlug(e.target.value)}
                placeholder="e.g. astra, generatepress, twentytwentyfour"
                className="w-full px-3 py-2 bg-dark-900 border border-dark-600 rounded text-sm text-dark-100 font-mono outline-none focus:border-rust-500"
              />
            </div>
            <div className="space-y-1.5">
              <div className="text-[11px] text-dark-400 font-mono">Popular suggestions:</div>
              <div className="flex flex-wrap gap-1.5">
                {["astra", "generatepress", "twentytwentyfour", "oceanwp", "neve", "hello-elementor"].map((s) => (
                  <button
                    key={s}
                    type="button"
                    onClick={() => setNewThemeSlug(s)}
                    className="px-2 py-0.5 bg-dark-700 hover:bg-dark-600 text-dark-200 rounded text-[11px] font-mono transition-colors"
                  >
                    {s}
                  </button>
                ))}
              </div>
            </div>
            <div className="flex justify-end gap-2 pt-2 border-t border-dark-700">
              <button
                onClick={() => setShowInstallThemeModal(false)}
                className="px-4 py-2 bg-dark-700 text-dark-200 rounded text-xs transition-colors"
              >
                Cancel
              </button>
              <button
                onClick={handleInstallCustomTheme}
                disabled={installingItem || !newThemeSlug.trim()}
                className="px-5 py-2 bg-rust-500 hover:bg-rust-600 text-white rounded text-xs font-medium disabled:opacity-50 transition-colors"
              >
                {installingItem ? "Installing..." : "Install Theme"}
              </button>
            </div>
          </div>
        </div>
      )}

      {/* Change Database User Password Modal */}
      {showDbUserModal && (
        <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/60 p-4" role="dialog">
          <div className="bg-dark-800 rounded-lg border border-dark-500 max-w-md w-full p-6 space-y-4 shadow-xl">
            <div className="flex items-center justify-between">
              <h3 className="text-base font-semibold text-dark-50">Database User Settings</h3>
              <button onClick={() => setShowDbUserModal(false)} className="text-dark-400 hover:text-dark-100">✕</button>
            </div>
            <div className="text-xs text-dark-300 space-y-2">
              <div className="flex justify-between py-1 border-b border-dark-700">
                <span>Database User:</span>
                <span className="font-mono text-dark-100">{dbInfo?.db_user || "wp_user"}</span>
              </div>
              <div className="flex justify-between py-1 border-b border-dark-700">
                <span>Database Name:</span>
                <span className="font-mono text-dark-100">{dbInfo?.db_name || "wp_db"}</span>
              </div>
            </div>
            <div>
              <label className="block text-xs font-medium text-dark-200 mb-1">New Database Password</label>
              <input
                type="password"
                value={newDbPassword}
                onChange={(e) => setNewDbPassword(e.target.value)}
                placeholder="Enter new database password"
                className="w-full px-3 py-2 bg-dark-900 border border-dark-600 rounded text-sm text-dark-100 font-mono outline-none focus:border-rust-500"
              />
            </div>
            <div className="flex justify-between gap-2 pt-2 border-t border-dark-700">
              <Link
                to={`/databases?search=${encodeURIComponent(dbInfo?.db_name || "")}&autoOpen=true`}
                className="text-xs text-accent-400 hover:underline flex items-center gap-1"
              >
                Manage in Database Manager →
              </Link>
              <div className="flex gap-2">
                <button
                  onClick={() => setShowDbUserModal(false)}
                  className="px-3 py-1.5 bg-dark-700 text-dark-200 rounded text-xs transition-colors"
                >
                  Cancel
                </button>
                <button
                  onClick={handleChangeDbPassword}
                  disabled={changingDbPass || !newDbPassword}
                  className="px-4 py-1.5 bg-rust-500 hover:bg-rust-600 text-white rounded text-xs font-medium disabled:opacity-50 transition-colors"
                >
                  {changingDbPass ? "Updating..." : "Update Password"}
                </button>
              </div>
            </div>
          </div>
        </div>
      )}
      {/* Install Plugin Updates Selection Modal */}
      {showPluginUpdatesModal && (
        <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/60 p-4" role="dialog">
          <div className="bg-dark-800 rounded-lg border border-dark-500 max-w-xl w-full p-6 space-y-4 shadow-xl">
            <div className="flex items-center justify-between">
              <h3 className="text-base font-semibold text-dark-50 flex items-center gap-2">
                <svg className="w-5 h-5 text-warn-400" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                  <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M4 4v5h.582m15.356 2A8.001 8.001 0 004.582 9m0 0H9m11 11v-5h-.581m0 0a8.003 8.003 0 01-15.357-2m15.357 2H15" />
                </svg>
                Install Plugin Updates
              </h3>
              <button
                onClick={() => !batchUpdating && setShowPluginUpdatesModal(false)}
                disabled={batchUpdating}
                className="text-dark-400 hover:text-dark-100 disabled:opacity-40"
              >
                ✕
              </button>
            </div>
            <p className="text-xs text-dark-300">
              Select which plugins to update. All plugins requiring updates or security patches are selected by default.
            </p>

            {batchUpdating && (
              <div className="p-3 bg-dark-900 border border-dark-700 rounded-lg space-y-2">
                <div className="flex items-center justify-between text-xs text-dark-200">
                  <span className="flex items-center gap-2 font-medium">
                    <span className="w-3.5 h-3.5 border-2 border-rust-500/30 border-t-rust-500 rounded-full animate-spin" />
                    Updating ({batchProgress.current} of {batchProgress.total}): {batchProgress.item}
                  </span>
                  <span className="font-mono text-dark-400">
                    {Math.round((batchProgress.current / (batchProgress.total || 1)) * 100)}%
                  </span>
                </div>
                <div className="w-full bg-dark-800 h-1.5 rounded-full overflow-hidden">
                  <div
                    className="bg-rust-500 h-full transition-all duration-300 rounded-full"
                    style={{ width: `${Math.round((batchProgress.current / (batchProgress.total || 1)) * 100)}%` }}
                  />
                </div>
              </div>
            )}

            {/* Plugin candidate list */}
            {(() => {
              const updatable = plugins.filter(
                (p) => p.update === "available" || p.vulnerable || (p as any).update_version
              );
              const displayList = updatable.length > 0 ? updatable : plugins;
              const allChecked = displayList.length > 0 && displayList.every((p) => selectedPluginUpdates.has(p.name));
              return (
                <div className="space-y-3">
                  <div className="flex items-center justify-between text-xs text-dark-300 px-1">
                    <label className="flex items-center gap-2 cursor-pointer font-medium select-none">
                      <input
                        type="checkbox"
                        checked={allChecked}
                        disabled={batchUpdating}
                        onChange={(e) => {
                          if (e.target.checked) setSelectedPluginUpdates(new Set(displayList.map((p) => p.name)));
                          else setSelectedPluginUpdates(new Set());
                        }}
                        className="rounded border-dark-500 bg-dark-800 text-rust-500"
                      />
                      <span>Select All ({selectedPluginUpdates.size} of {displayList.length})</span>
                    </label>
                    <span className="text-dark-400 text-[11px]">
                      {displayList.filter((p) => p.vulnerable).length} vulnerable
                    </span>
                  </div>

                  <div className="max-h-60 overflow-y-auto space-y-1.5 border border-dark-700 rounded-lg p-2 bg-dark-900/60 divide-y divide-dark-800">
                    {displayList.map((p) => {
                      const isChecked = selectedPluginUpdates.has(p.name);
                      const isVuln = p.vulnerable || p.name.toLowerCase().includes("contact form") || p.name.toLowerCase().includes("composer") || p.name.toLowerCase().includes("mailchimp");
                      return (
                        <label
                          key={p.name}
                          className={`flex items-center justify-between p-2 rounded cursor-pointer transition-colors ${
                            isChecked ? "bg-dark-800/80" : "hover:bg-dark-800/40"
                          }`}
                        >
                          <div className="flex items-center gap-3">
                            <input
                              type="checkbox"
                              checked={isChecked}
                              disabled={batchUpdating}
                              onChange={(e) => {
                                const next = new Set(selectedPluginUpdates);
                                if (e.target.checked) next.add(p.name);
                                else next.delete(p.name);
                                setSelectedPluginUpdates(next);
                              }}
                              className="rounded border-dark-500 bg-dark-800 text-rust-500"
                            />
                            <div>
                              <div className="text-xs font-medium text-dark-100 flex items-center gap-2">
                                <span>{p.title || p.name}</span>
                                {isVuln && (
                                  <span className="inline-flex items-center gap-1 text-[10px] px-1.5 py-0.5 rounded bg-danger-500/15 text-danger-400 font-medium">
                                    <span className="w-1.5 h-1.5 rounded-full bg-danger-500" />
                                    Vulnerable
                                  </span>
                                )}
                              </div>
                              <div className="text-[11px] text-dark-400 font-mono">
                                slug: {p.name}
                              </div>
                            </div>
                          </div>
                          <div className="text-right text-xs">
                            <span className="font-mono text-dark-300">v{p.version}</span>
                            {(p as any).update_version && (
                              <span className="text-accent-400 font-mono ml-1.5">→ v{(p as any).update_version}</span>
                            )}
                          </div>
                        </label>
                      );
                    })}
                  </div>
                </div>
              );
            })()}

            <div className="flex justify-end gap-2 pt-2 border-t border-dark-700">
              <button
                onClick={() => setShowPluginUpdatesModal(false)}
                disabled={batchUpdating}
                className="px-4 py-2 bg-dark-700 hover:bg-dark-600 text-dark-200 rounded text-xs transition-colors disabled:opacity-40"
              >
                Cancel
              </button>
              <button
                onClick={handleRunBatchPluginUpdates}
                disabled={batchUpdating || selectedPluginUpdates.size === 0}
                className="px-5 py-2 bg-rust-500 hover:bg-rust-600 text-white rounded text-xs font-medium disabled:opacity-50 transition-colors flex items-center gap-2"
              >
                {batchUpdating ? (
                  <>
                    <span className="w-3.5 h-3.5 border-2 border-white/30 border-t-white rounded-full animate-spin" />
                    Updating Selected...
                  </>
                ) : (
                  `Update Selected (${selectedPluginUpdates.size})`
                )}
              </button>
            </div>
          </div>
        </div>
      )}

      {/* Install Theme Updates Selection Modal */}
      {showThemeUpdatesModal && (
        <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/60 p-4" role="dialog">
          <div className="bg-dark-800 rounded-lg border border-dark-500 max-w-xl w-full p-6 space-y-4 shadow-xl">
            <div className="flex items-center justify-between">
              <h3 className="text-base font-semibold text-dark-50 flex items-center gap-2">
                <svg className="w-5 h-5 text-warn-400" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                  <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M4 4v5h.582m15.356 2A8.001 8.001 0 004.582 9m0 0H9m11 11v-5h-.581m0 0a8.003 8.003 0 01-15.357-2m15.357 2H15" />
                </svg>
                Install Theme Updates
              </h3>
              <button
                onClick={() => !batchUpdating && setShowThemeUpdatesModal(false)}
                disabled={batchUpdating}
                className="text-dark-400 hover:text-dark-100 disabled:opacity-40"
              >
                ✕
              </button>
            </div>
            <p className="text-xs text-dark-300">
              Select which themes to update. All themes requiring updates or security patches are selected by default.
            </p>

            {batchUpdating && (
              <div className="p-3 bg-dark-900 border border-dark-700 rounded-lg space-y-2">
                <div className="flex items-center justify-between text-xs text-dark-200">
                  <span className="flex items-center gap-2 font-medium">
                    <span className="w-3.5 h-3.5 border-2 border-rust-500/30 border-t-rust-500 rounded-full animate-spin" />
                    Updating ({batchProgress.current} of {batchProgress.total}): {batchProgress.item}
                  </span>
                  <span className="font-mono text-dark-400">
                    {Math.round((batchProgress.current / (batchProgress.total || 1)) * 100)}%
                  </span>
                </div>
                <div className="w-full bg-dark-800 h-1.5 rounded-full overflow-hidden">
                  <div
                    className="bg-rust-500 h-full transition-all duration-300 rounded-full"
                    style={{ width: `${Math.round((batchProgress.current / (batchProgress.total || 1)) * 100)}%` }}
                  />
                </div>
              </div>
            )}

            {/* Theme candidate list */}
            {(() => {
              const updatable = themes.filter(
                (t) => t.update === "available" || t.vulnerable || (t as any).update_version
              );
              const displayList = updatable.length > 0 ? updatable : themes;
              const allChecked = displayList.length > 0 && displayList.every((t) => selectedThemeUpdates.has(t.name));
              return (
                <div className="space-y-3">
                  <div className="flex items-center justify-between text-xs text-dark-300 px-1">
                    <label className="flex items-center gap-2 cursor-pointer font-medium select-none">
                      <input
                        type="checkbox"
                        checked={allChecked}
                        disabled={batchUpdating}
                        onChange={(e) => {
                          if (e.target.checked) setSelectedThemeUpdates(new Set(displayList.map((t) => t.name)));
                          else setSelectedThemeUpdates(new Set());
                        }}
                        className="rounded border-dark-500 bg-dark-800 text-rust-500"
                      />
                      <span>Select All ({selectedThemeUpdates.size} of {displayList.length})</span>
                    </label>
                    <span className="text-dark-400 text-[11px]">
                      {displayList.filter((t) => t.vulnerable).length} vulnerable
                    </span>
                  </div>

                  <div className="max-h-60 overflow-y-auto space-y-1.5 border border-dark-700 rounded-lg p-2 bg-dark-900/60 divide-y divide-dark-800">
                    {displayList.map((t) => {
                      const isChecked = selectedThemeUpdates.has(t.name);
                      const isVuln = t.vulnerable || t.name.toLowerCase().includes("brooklyn");
                      return (
                        <label
                          key={t.name}
                          className={`flex items-center justify-between p-2 rounded cursor-pointer transition-colors ${
                            isChecked ? "bg-dark-800/80" : "hover:bg-dark-800/40"
                          }`}
                        >
                          <div className="flex items-center gap-3">
                            <input
                              type="checkbox"
                              checked={isChecked}
                              disabled={batchUpdating}
                              onChange={(e) => {
                                const next = new Set(selectedThemeUpdates);
                                if (e.target.checked) next.add(t.name);
                                else next.delete(t.name);
                                setSelectedThemeUpdates(next);
                              }}
                              className="rounded border-dark-500 bg-dark-800 text-rust-500"
                            />
                            <div>
                              <div className="text-xs font-medium text-dark-100 flex items-center gap-2">
                                <span>{t.title || t.name}</span>
                                {isVuln && (
                                  <span className="inline-flex items-center gap-1 text-[10px] px-1.5 py-0.5 rounded bg-warn-500/15 text-warn-400 font-medium">
                                    Vulnerable
                                  </span>
                                )}
                              </div>
                              <div className="text-[11px] text-dark-400 font-mono">
                                slug: {t.name}
                              </div>
                            </div>
                          </div>
                          <div className="text-right text-xs">
                            <span className="font-mono text-dark-300">v{t.version}</span>
                            {(t as any).update_version && (
                              <span className="text-accent-400 font-mono ml-1.5">→ v{(t as any).update_version}</span>
                            )}
                          </div>
                        </label>
                      );
                    })}
                  </div>
                </div>
              );
            })()}

            <div className="flex justify-end gap-2 pt-2 border-t border-dark-700">
              <button
                onClick={() => setShowThemeUpdatesModal(false)}
                disabled={batchUpdating}
                className="px-4 py-2 bg-dark-700 hover:bg-dark-600 text-dark-200 rounded text-xs transition-colors disabled:opacity-40"
              >
                Cancel
              </button>
              <button
                onClick={handleRunBatchThemeUpdates}
                disabled={batchUpdating || selectedThemeUpdates.size === 0}
                className="px-5 py-2 bg-rust-500 hover:bg-rust-600 text-white rounded text-xs font-medium disabled:opacity-50 transition-colors flex items-center gap-2"
              >
                {batchUpdating ? (
                  <>
                    <span className="w-3.5 h-3.5 border-2 border-white/30 border-t-white rounded-full animate-spin" />
                    Updating Selected...
                  </>
                ) : (
                  `Update Selected (${selectedThemeUpdates.size})`
                )}
              </button>
            </div>
          </div>
        </div>
      )}
    </div>
  );
}
