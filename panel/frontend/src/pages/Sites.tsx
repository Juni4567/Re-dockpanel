import { useState, useEffect, FormEvent } from "react";
import { Link } from "react-router-dom";
import { api } from "../api";
import { formatDate, formatSize } from "../utils/format";
import { statusColors, runtimeLabels } from "../constants";
import ProvisionLog from "../components/ProvisionLog";
import { PrereqCallout, useDnsPrereq } from "../components/Prerequisite";
import { FieldHelp, InfoTip } from "../components/FieldHelp";
import PhpVersionPicker from "../components/PhpVersionPicker";
import SiteThumbnail from "../components/SiteThumbnail";
import SiteFavicon from "../components/SiteFavicon";
import DesktopSiteActions from "../components/DesktopSiteActions";
import InstallWordPressModal from "../components/InstallWordPressModal";
import LiveSyncModal from "../components/LiveSyncModal";
import PortConflictModal, {
  PortConflictInfo,
  buildSiteExternalUrl,
} from "../components/PortConflictModal";
import { isDesktopApp, formatDesktopDomain } from "../utils/desktop";
import { showGlobalNotification } from "../context/NotificationContext";
import { useAuth } from "../context/AuthContext";

interface SiteMetrics {
  disk_usage_bytes: number;
  traffic_bytes_this_month: number;
  server_location: string;
  has_database: boolean;
  has_mail: boolean;
}

interface Site {
  id: string;
  domain: string;
  runtime?: string;
  php_preset?: string | null;
  status: string;
  ssl_enabled: boolean;
  enabled: boolean;
  parent_site_id: string | null;
  created_at: string;
  // Present only on the admin all-sites view (GET /api/admin/sites). The
  // owner-scoped list cannot carry them, and would not need to: every row it
  // returns belongs to the person reading it.
  user_id?: string;
  owner_email?: string | null;
  owner_role?: string | null;
}

interface PanelUser {
  id: string;
  email: string;
  role: string;
}

export default function Sites() {
  const { user } = useAuth();
  const [sites, setSites] = useState<Site[]>([]);
  const [metrics, setMetrics] = useState<Record<string, SiteMetrics>>({});
  const [loading, setLoading] = useState(true);
  const [showForm, setShowForm] = useState(false);
  const [error, setError] = useState("");
  const [provisioningSiteId, setProvisioningSiteId] = useState<string | null>(null);
  const [search, setSearch] = useState("");
  const [displayCount, setDisplayCount] = useState(25);
  const [viewMode, setViewMode] = useState<"table" | "grid">("table");
  const [sortBy, setSortBy] = useState<"domain" | "created" | "disk" | "traffic">("domain");
  const [sortOrder, setSortOrder] = useState<"asc" | "desc">("asc");
  const [copiedDomain, setCopiedDomain] = useState<string | null>(null);

  // Admin all-sites view. Ownership is exclusive, so a site handed to a client
  // leaves the admin's own list entirely — this is how they find it again, and
  // the only place from which a transfer can be undone.
  const isAdmin = user?.role === "admin";
  /** The one role that may hold sites but never claim a new domain. */
  const isClient = user?.role === "client";
  const [allSites, setAllSites] = useState(false);
  const [users, setUsers] = useState<PanelUser[]>([]);
  const [transferFor, setTransferFor] = useState<Site | null>(null);
  const [transferEmail, setTransferEmail] = useState("");
  const [transferring, setTransferring] = useState(false);
  const [notice, setNotice] = useState("");
  const [syncSite, setSyncSite] = useState<Site | null>(null);
  const [wpInstallSite, setWpInstallSite] = useState<Site | null>(null);
  const [conflictInfo, setConflictInfo] = useState<PortConflictInfo | null>(null);
  const [showPortModal, setShowPortModal] = useState(false);

  // Form state
  const [domain, setDomain] = useState("");
  const [domainSuffix, setDomainSuffix] = useState<".test" | ".local">(".test");
  const [runtime, setRuntime] = useState("static");
  const [proxyPort, setProxyPort] = useState("");
  const [phpVersion, setPhpVersion] = useState("8.3");
  const [phpPreset, setPhpPreset] = useState("generic");
  const [appCommand, setAppCommand] = useState("");
  const [submitting, setSubmitting] = useState(false);
  const [cms, setCms] = useState("");
  const [siteTitle, setSiteTitle] = useState("");
  const [adminEmail, setAdminEmail] = useState("");
  const [adminUser, setAdminUser] = useState("admin");
  const [adminPassword, setAdminPassword] = useState("");
  const [wpDbEngine, setWpDbEngine] = useState<"sqlite" | "mariadb">("sqlite");
  const [customRootPath, setCustomRootPath] = useState("");
  const [defaultSitesDir, setDefaultSitesDir] = useState("");
  const [sslPort, setSslPort] = useState<number | null>(null);
  const [enableBackup, setEnableBackup] = useState(false);
  const [backupFrequency, setBackupFrequency] = useState("daily");
  const [backupRetention, setBackupRetention] = useState(7);

  const copyDomain = (d: string) => {
    navigator.clipboard.writeText(d);
    setCopiedDomain(d);
    setTimeout(() => setCopiedDomain(null), 2000);
  };

  const getSiteExternalUrl = (site: Site) => {
    if (isDesktopApp()) {
      return sslPort && site.ssl_enabled
        ? buildSiteExternalUrl(site.domain, sslPort, true)
        : buildSiteExternalUrl(site.domain, conflictInfo?.active_port, false);
    }
    return buildSiteExternalUrl(site.domain, conflictInfo?.active_port, site.ssl_enabled);
  };

  const handleSiteStatusChange = (siteId: string, newStatus: string) => {
    setSites((prev) =>
      prev.map((s) => (s.id === siteId ? { ...s, status: newStatus } : s))
    );
  };

  const renderStatusBadge = (site: Site) => {
    const badge =
      site.enabled === false
        ? { classes: "bg-warn-500/10 text-warn-400", dot: "bg-warn-400", label: "disabled" }
        : site.status === "active"
        ? { classes: statusColors.active, dot: "bg-emerald-400", label: "Running" }
        : site.status === "stopped"
        ? { classes: statusColors.stopped, dot: "bg-dark-400", label: "Stopped" }
        : { classes: statusColors[site.status] || "bg-dark-700 text-dark-200", dot: "bg-current", label: site.status };

    return (
      <span className={`inline-flex items-center gap-1.5 px-2 py-0.5 rounded-full text-xs font-medium ${badge.classes}`}>
        <span className={`w-1.5 h-1.5 rounded-full ${badge.dot}`} />
        {badge.label}
      </span>
    );
  };

  const effectiveDomain = isDesktopApp() ? formatDesktopDomain(domain, domainSuffix) : domain;

  // Preflight the domain as it is typed. Nothing here BLOCKS creation — a site
  // is perfectly valid before its DNS exists, and the brief is explicit that a
  // not-yet-propagated record must not be presented as an error. The point is
  // that the user learns about the prerequisite here, rather than discovering it
  // later when SSL silently fails to appear.
  const { prereq: dnsPrereq, checking: dnsChecking, recheck: recheckDns } =
    useDnsPrereq(showForm ? effectiveDomain : "");

  const fetchSites = () => {
    api
      .get<Site[]>(allSites ? "/admin/sites" : "/sites")
      .then(setSites)
      .catch((e) => setError(e.message))
      .finally(() => setLoading(false));

    api
      .get<Record<string, SiteMetrics>>("/sites/metrics-overview")
      .then(setMetrics)
      .catch(() => {});
  };

  useEffect(fetchSites, [allSites]);

  useEffect(() => {
    if (!isDesktopApp()) return;
    api.get<PortConflictInfo>("/system/ports/conflict").then(setConflictInfo).catch(() => {});
    api.get<{ ssl_port?: number | null }>("/health")
      .then((h) => { if (typeof h?.ssl_port === "number") setSslPort(h.ssl_port); })
      .catch(() => {});
    api.get<{ default_php_version?: string; default_sites_dir?: string }>("/settings")
      .then((s) => {
        if (s?.default_php_version) setPhpVersion(s.default_php_version);
        if (s?.default_sites_dir) setDefaultSitesDir(s.default_sites_dir);
      })
      .catch(() => {});
  }, []);

  // The transfer recipient is picked, not typed. A free-text email could only
  // ever fail late — `transfer` answers 404 "No account with that email" — and
  // an operator does not carry their clients' addresses in their head.
  useEffect(() => {
    if (!isAdmin) return;
    api.get<PanelUser[]>("/users").then(setUsers).catch(() => setUsers([]));
  }, [isAdmin]);

  const handleCreate = async (e: FormEvent) => {
    e.preventDefault();
    setError("");
    setSubmitting(true);
    try {
      const effectiveRuntime = cms ? "php" : runtime;
      const effectivePreset = cms || phpPreset;
      const effectiveDomainToCreate = isDesktopApp() ? formatDesktopDomain(domain, domainSuffix) : domain;
      const body: Record<string, unknown> = {
        domain: effectiveDomainToCreate,
        runtime: effectiveRuntime,
        ...(isDesktopApp() ? { stream: true } : {}),
      };
      if (effectiveRuntime === "proxy") body.proxy_port = parseInt(proxyPort);
      if (effectiveRuntime === "node" || effectiveRuntime === "python") {
        body.app_command = appCommand;
      }
      if (effectiveRuntime === "php") {
        body.php_version = phpVersion;
        body.php_preset = effectivePreset;
      }
      if (isDesktopApp() && customRootPath.trim()) {
        body.document_root = customRootPath.trim();
      }
      if (cms) {
        body.cms = cms;
        if (cms === "wordpress" && isDesktopApp()) body.db_engine = wpDbEngine;
        if (siteTitle) body.site_title = siteTitle;
        if (adminEmail) body.admin_email = adminEmail;
        if (adminUser) body.admin_user = adminUser;
        if (adminPassword) body.admin_password = adminPassword;
      }
      if (!isDesktopApp()) {
        body.enable_backup_schedule = enableBackup;
        if (enableBackup) {
          body.backup_frequency = backupFrequency;
          body.backup_retention = backupRetention;
        }
      }

      const created = await api.post<Site>("/sites", body);
      setShowForm(false);
      setProvisioningSiteId(created.id);
      showGlobalNotification("info", `Creating local site ${created.domain}...`);
      setDomain("");
      setDomainSuffix(".test");
      setRuntime("static");
      setProxyPort("");
      setCms("");
      setSiteTitle("");
      setAdminEmail("");
      setAdminUser("admin");
      setAdminPassword("");
      setWpDbEngine("sqlite");
      setCustomRootPath("");
      setEnableBackup(false);
      setBackupFrequency("daily");
      setBackupRetention(7);
      fetchSites();
    } catch (err) {
      setError(err instanceof Error ? err.message : "Failed to create site");
    } finally {
      setSubmitting(false);
    }
  };

  const handleProvisionComplete = () => {
    setProvisioningSiteId(null);
    showGlobalNotification("success", "Site provisioning complete");
    fetchSites();
  };

  return (
    <div className="animate-fade-up">
      <div className="page-header">
        <div>
          <h1 className="page-header-title">Sites</h1>
          <p className="page-header-subtitle">Manage your websites and applications</p>
        </div>
        <div className="flex items-center gap-2 flex-wrap">
          {isAdmin && (
            <label className="flex items-center gap-2 text-sm text-dark-200 select-none cursor-pointer">
              <input
                type="checkbox"
                checked={allSites}
                onChange={(e) => { setLoading(true); setAllSites(e.target.checked); }}
                className="rounded border-dark-500 bg-dark-800 text-rust-500 focus:ring-rust-500"
              />
              All sites
            </label>
          )}
          <input
            type="text"
            value={search}
            onChange={(e) => setSearch(e.target.value)}
            placeholder="Search sites..."
            className="px-3 py-1.5 bg-dark-800 border border-dark-600 rounded-lg text-sm text-dark-100 placeholder-dark-400 focus:outline-none focus:border-dark-400 w-36 sm:w-48"
          />
          <select
            value={sortBy}
            onChange={(e) => setSortBy(e.target.value as any)}
            className="px-2.5 py-1.5 bg-dark-800 border border-dark-600 rounded-lg text-xs text-dark-100 focus:outline-none"
            aria-label="Sort sites"
          >
            <option value="domain">Domain (A-Z)</option>
            <option value="created">Created Date</option>
            <option value="disk">Disk Usage</option>
            <option value="traffic">Traffic</option>
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
              onClick={() => setViewMode("table")}
              className={`p-1.5 rounded ${viewMode === "table" ? "bg-dark-700 text-dark-50" : "text-dark-400 hover:text-dark-200"}`}
              title="Table view"
              aria-label="Table view"
            >
              <svg className="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M4 6h16M4 12h16M4 18h16" />
              </svg>
            </button>
            <button
              onClick={() => setViewMode("grid")}
              className={`p-1.5 rounded ${viewMode === "grid" ? "bg-dark-700 text-dark-50" : "text-dark-400 hover:text-dark-200"}`}
              title="Grid / Card view"
              aria-label="Grid view"
            >
              <svg className="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M4 6a2 2 0 012-2h2a2 2 0 012 2v2a2 2 0 01-2 2H6a2 2 0 01-2-2V6zM14 6a2 2 0 012-2h2a2 2 0 012 2v2a2 2 0 01-2 2h-2a2 2 0 01-2-2V6zM4 16a2 2 0 012-2h2a2 2 0 012 2v2a2 2 0 01-2 2H6a2 2 0 01-2-2v-2zM14 16a2 2 0 012-2h2a2 2 0 012 2v2a2 2 0 01-2 2h-2a2 2 0 01-2-2v-2z" />
              </svg>
            </button>
          </div>
          {!isClient && (
            <button
              onClick={() => setShowForm(!showForm)}
              className="px-4 py-2 bg-rust-500 text-white rounded-lg text-sm font-medium hover:bg-rust-600 transition-colors"
            >
              {showForm ? "Cancel" : "Create Site"}
            </button>
          )}
        </div>
      </div>

      <div className="p-6 lg:p-8">

      {isDesktopApp() && conflictInfo?.in_use && (
        <div className="mb-6 p-4 bg-amber-500/10 border border-amber-500/20 rounded-xl flex items-center justify-between gap-4">
          <div className="flex items-center gap-3">
            <span className="p-2 bg-amber-500/20 text-amber-400 rounded-lg shrink-0" aria-hidden="true">⚠️</span>
            <div>
              <p className="text-sm font-medium text-amber-200">
                Port {conflictInfo.preferred_port} in use by {conflictInfo.display_name || conflictInfo.process_name || "another app"}.
              </p>
              <p className="text-xs text-amber-400/80">
                Sites are currently being served on port {conflictInfo.active_port}.
              </p>
            </div>
          </div>
          <button
            type="button"
            onClick={() => setShowPortModal(true)}
            className="px-3 py-1.5 bg-amber-500/20 hover:bg-amber-500/30 text-amber-300 text-xs font-semibold rounded-lg transition-colors shrink-0"
          >
            Change Port
          </button>
        </div>
      )}

      {error && (
        <div role="alert" className="bg-danger-500/10 text-danger-400 text-sm px-4 py-3 rounded-lg border border-danger-500/20 mb-4">
          {error}
          <button onClick={() => setError("")} className="float-right font-bold" aria-label="Close error">&times;</button>
        </div>
      )}

      {/* Provisioning log */}
      {provisioningSiteId && (
        <ProvisionLog siteId={provisioningSiteId} onComplete={handleProvisionComplete} />
      )}

      {/* Create form */}
      {showForm && (
        <div className="mb-6">
        <form
          onSubmit={handleCreate}
          className="bg-dark-800 rounded-lg border border-dark-500 p-5 space-y-4"
        >
          {/* Quick CMS Install */}
          <div>
            <label className="block text-xs font-medium text-dark-200 mb-2">Quick Install</label>
            <div className="flex gap-2 overflow-x-auto pb-2 -mx-1 px-1">
              {[
                { id: "", label: "Custom Site", desc: "" },
                { id: "wordpress", label: "WordPress", desc: "Blog & CMS" },
                { id: "laravel", label: "Laravel", desc: "PHP Framework" },
                { id: "drupal", label: "Drupal", desc: "Enterprise CMS" },
                { id: "joomla", label: "Joomla", desc: "CMS" },
                { id: "symfony", label: "Symfony", desc: "PHP Framework" },
                { id: "codeigniter", label: "CodeIgniter", desc: "PHP Framework" },
              ].map((c) => (
                <button key={c.id} type="button" onClick={() => { setCms(c.id); if (c.id) { setRuntime("php"); setPhpPreset(c.id || "generic"); } else { setRuntime("static"); } }}
                  className={`flex-shrink-0 px-3 py-2 border text-sm transition-colors ${cms === c.id ? "border-dark-50/30 bg-dark-50/5 text-dark-50" : "border-dark-500 bg-dark-900/50 text-dark-300 hover:border-dark-400"}`}
                >
                  {c.label}
                </button>
              ))}
            </div>
          </div>

          <div className="grid grid-cols-1 md:grid-cols-2 gap-4">
            <div>
              <div className="flex items-center justify-between mb-1">
                <label htmlFor="site-domain" className="block text-sm font-medium text-dark-100">Domain</label>
                {isDesktopApp() && (
                  <div className="flex items-center gap-1 text-xs">
                    <span className="text-dark-400">Suffix:</span>
                    <button
                      type="button"
                      onClick={() => setDomainSuffix(".test")}
                      className={`px-1.5 py-0.5 rounded font-mono transition-colors ${
                        domainSuffix === ".test"
                          ? "bg-rust-500 text-white font-medium"
                          : "bg-dark-700 text-dark-300 hover:text-dark-100"
                      }`}
                    >
                      .test
                    </button>
                    <button
                      type="button"
                      onClick={() => setDomainSuffix(".local")}
                      className={`px-1.5 py-0.5 rounded font-mono transition-colors ${
                        domainSuffix === ".local"
                          ? "bg-rust-500 text-white font-medium"
                          : "bg-dark-700 text-dark-300 hover:text-dark-100"
                      }`}
                    >
                      .local
                    </button>
                  </div>
                )}
              </div>
              <div className="flex">
                <input
                  id="site-domain"
                  type="text"
                  value={domain}
                  onChange={(e) => {
                    const val = e.target.value;
                    if (val.endsWith(".test")) {
                      setDomainSuffix(".test");
                    } else if (val.endsWith(".local")) {
                      setDomainSuffix(".local");
                    }
                    setDomain(val);
                  }}
                  required
                  placeholder={isDesktopApp() ? "mysite" : "example.com"}
                  className={`w-full px-3 py-2.5 border border-dark-500 ${
                    isDesktopApp() ? "rounded-l-lg border-r-0" : "rounded-lg"
                  } focus:ring-2 focus:ring-accent-500 focus:border-accent-500 outline-none text-sm`}
                />
                {isDesktopApp() && (
                  <select
                    id="site-domain-suffix"
                    aria-label="Domain suffix"
                    value={domainSuffix}
                    onChange={(e) => setDomainSuffix(e.target.value as ".test" | ".local")}
                    className="px-3 py-2.5 bg-dark-700 border border-dark-500 rounded-r-lg text-sm font-mono text-dark-100 outline-none focus:ring-2 focus:ring-accent-500 cursor-pointer"
                  >
                    <option value=".test">.test</option>
                    <option value=".local">.local</option>
                  </select>
                )}
              </div>
              <FieldHelp id="sites.create.domain" />
              <PrereqCallout
                prereq={dnsPrereq}
                onRecheck={recheckDns}
                checking={dnsChecking}
                showSatisfied
                className="mt-2"
              />
            </div>
            {!cms ? (
              <div>
                {/* The (i) sits beside the label, never inside it: a button
                    inside a <label> activates the labelled control, so opening
                    the tooltip would also drop the select's list over it. */}
                <div className="flex items-center mb-1">
                  <label htmlFor="site-runtime" className="block text-sm font-medium text-dark-100">Runtime</label>
                  <InfoTip id="sites.create.runtime" />
                </div>
                <select
                  id="site-runtime"
                  value={runtime}
                  onChange={(e) => setRuntime(e.target.value)}
                  className="w-full px-3 py-2.5 border border-dark-500 rounded-lg focus:ring-2 focus:ring-accent-500 focus:border-accent-500 outline-none text-sm bg-dark-800"
                >
                  <option value="static">Static (HTML/CSS/JS)</option>
                  <option value="php">PHP</option>
                  <option value="node">Node.js</option>
                  {!isDesktopApp() && <option value="python">Python</option>}
                  {!isDesktopApp() && <option value="proxy">Reverse Proxy</option>}
                </select>
                <p className="text-xs text-dark-400 mt-1.5">
                  {runtime === "node" ? (isDesktopApp() ? "Node.js app with managed local process (auto-assigned $PORT)" : "Node.js app with managed process (systemd + nginx reverse proxy)") :
                   runtime === "python" ? "Python app with managed process (systemd + nginx reverse proxy)" :
                   runtime === "proxy" ? "Reverse proxy to a port (Docker container or external service)" :
                   runtime === "php" ? "PHP with PHP-FPM — WordPress, Laravel, Drupal, etc." :
                   "Static HTML/CSS/JS files served by nginx"}
                </p>
              </div>
            ) : (
              <div>
                <label className="block text-sm font-medium text-dark-100 mb-1">Site Title</label>
                <input type="text" value={siteTitle} onChange={(e) => setSiteTitle(e.target.value)} placeholder={`My ${cms.charAt(0).toUpperCase() + cms.slice(1)} Site`} className="w-full px-3 py-2.5 border border-dark-500 rounded-lg focus:ring-2 focus:ring-accent-500 outline-none text-sm" />
                <p className="text-xs text-dark-400 mt-1.5">The title for your {cms.charAt(0).toUpperCase() + cms.slice(1)} site</p>
              </div>
            )}
          </div>

          {/* CMS Admin Fields */}
          {cms && (
            <>
            <div className="col-span-2 border-t border-dark-700 pt-3 mt-1">
              <span className="text-xs font-medium text-dark-400 uppercase tracking-wider">{cms.charAt(0).toUpperCase() + cms.slice(1)} Configuration</span>
            </div>
            <div className="grid grid-cols-1 md:grid-cols-3 gap-4">
              <div>
                <label className="block text-sm font-medium text-dark-100 mb-1">Admin Email</label>
                <input type="email" value={adminEmail} onChange={(e) => setAdminEmail(e.target.value)} placeholder="you@example.com" className="w-full px-3 py-2.5 border border-dark-500 rounded-lg focus:ring-2 focus:ring-accent-500 outline-none text-sm" />
                <p className="text-xs text-dark-400 mt-1.5">{cms.charAt(0).toUpperCase() + cms.slice(1)} admin email address</p>
              </div>
              <div>
                <label className="block text-sm font-medium text-dark-100 mb-1">Admin Username</label>
                <input type="text" value={adminUser} onChange={(e) => setAdminUser(e.target.value)} placeholder="admin" className="w-full px-3 py-2.5 border border-dark-500 rounded-lg focus:ring-2 focus:ring-accent-500 outline-none text-sm" />
              </div>
              <div>
                <label className="block text-sm font-medium text-dark-100 mb-1">Admin Password</label>
                <input type="password" value={adminPassword} onChange={(e) => setAdminPassword(e.target.value)} placeholder="Auto-generated if blank" className="w-full px-3 py-2.5 border border-dark-500 rounded-lg focus:ring-2 focus:ring-accent-500 outline-none text-sm" />
                <FieldHelp id="sites.create.admin_password" />
              </div>
            </div>
            {cms === "wordpress" && isDesktopApp() && (
              <div>
                <label htmlFor="site-wp-db" className="block text-sm font-medium text-dark-100 mb-1">Database Engine</label>
                <select
                  id="site-wp-db"
                  value={wpDbEngine}
                  onChange={(e) => setWpDbEngine(e.target.value as "sqlite" | "mariadb")}
                  className="w-full px-3 py-2.5 border border-dark-500 rounded-lg focus:ring-2 focus:ring-accent-500 outline-none text-sm bg-dark-800 max-w-md"
                >
                  <option value="sqlite">SQLite (Instant — zero database setup)</option>
                  <option value="mariadb">MariaDB (Open-source MySQL server — auto-downloads if needed)</option>
                </select>
                <p className="text-xs text-dark-400 mt-1">Choose SQLite for zero-config speed or MariaDB for full MySQL compatibility.</p>
              </div>
            )}
            </>
          )}

          {isDesktopApp() && (
            <div>
              <label htmlFor="site-custom-root" className="block text-sm font-medium text-dark-100 mb-1">
                Website Folder Path <span className="text-xs text-dark-400 font-normal">(optional custom location)</span>
              </label>
              <input
                id="site-custom-root"
                type="text"
                value={customRootPath}
                onChange={(e) => setCustomRootPath(e.target.value)}
                placeholder={
                  defaultSitesDir
                    ? `${defaultSitesDir}\\${effectiveDomain || "mysite.test"}`
                    : `C:\\Users\\...\\DockPanel\\sites\\${effectiveDomain || "mysite.test"}`
                }
                className="w-full px-3 py-2.5 border border-dark-500 rounded-lg focus:ring-2 focus:ring-accent-500 outline-none text-sm font-mono"
              />
              <p className="text-xs text-dark-400 mt-1">
                Leave empty to use your default websites directory.
              </p>
            </div>
          )}

          {runtime === "proxy" && (
            <div>
              <label htmlFor="site-proxy-port" className="block text-sm font-medium text-dark-100 mb-1">Proxy Port</label>
              <input
                id="site-proxy-port"
                type="number"
                value={proxyPort}
                onChange={(e) => setProxyPort(e.target.value)}
                required
                placeholder="3000"
                min="1"
                max="65535"
                className="w-full px-3 py-2.5 border border-dark-500 rounded-lg focus:ring-2 focus:ring-accent-500 focus:border-accent-500 outline-none text-sm max-w-xs"
              />
              <FieldHelp id="sites.create.proxy_port" />
            </div>
          )}

          {(runtime === "node" || runtime === "python") && (
            <div>
              <label htmlFor="site-app-command" className="block text-sm font-medium text-dark-100 mb-1">Start Command</label>
              <input
                id="site-app-command"
                type="text"
                value={appCommand}
                onChange={(e) => setAppCommand(e.target.value)}
                required
                placeholder={runtime === "node" ? "npm start" : "gunicorn app:app"}
                className="w-full px-3 py-2.5 border border-dark-500 rounded-lg focus:ring-2 focus:ring-accent-500 focus:border-accent-500 outline-none text-sm font-mono"
              />
              <p className="text-xs text-dark-400 mt-1.5">
                {runtime === "node"
                  ? "e.g., npm start, node server.js, npx next start"
                  : "e.g., gunicorn app:app, uvicorn main:app, flask run"}
                {" "}— port auto-allocated via $PORT env var
              </p>
            </div>
          )}

          {/* Shown for a CMS too. `handleCreate` sends `php_version` for every
              CMS install — a one-click WordPress is a PHP site — so gating this
              on `!cms` left the most common route to a PHP site submitting the
              form's untouched default with nothing on screen to say which
              versions the server has. That is the same failure the picker exists
              to prevent, on the path most people take. */}
          {(runtime === "php" || cms) && (
            <div className="grid grid-cols-1 md:grid-cols-2 gap-4">
              <div>
                <label htmlFor="site-php-version" className="block text-sm font-medium text-dark-100 mb-1">PHP Version</label>
                {/* The agent refuses to write a PHP vhost when the version's FPM
                    socket is missing, so an uninstalled version failed the whole
                    site creation with a message pointing somewhere that could not
                    fix it. The picker now says which versions this server has and
                    offers to install one that it does not. */}
                <PhpVersionPicker
                  id="site-php-version"
                  value={phpVersion}
                  onChange={(v) => setPhpVersion(v)}
                  canInstall={user?.role === "admin"}
                />
              </div>
              <div className={cms ? "hidden" : undefined}>
                <label htmlFor="site-php-preset" className="block text-sm font-medium text-dark-100 mb-1">Framework</label>
                <select
                  id="site-php-preset"
                  value={phpPreset}
                  onChange={(e) => setPhpPreset(e.target.value)}
                  className="w-full px-3 py-2.5 border border-dark-500 rounded-lg focus:ring-2 focus:ring-accent-500 focus:border-accent-500 outline-none text-sm bg-dark-800"
                >
                  <option value="generic">Generic PHP</option>
                  <option value="laravel">Laravel</option>
                  <option value="wordpress">WordPress</option>
                  <option value="drupal">Drupal</option>
                  <option value="joomla">Joomla</option>
                  <option value="symfony">Symfony</option>
                  <option value="codeigniter">CodeIgniter</option>
                  <option value="magento">Magento</option>
                </select>
                <p className="text-xs text-dark-400 mt-1.5">Nginx configuration preset for your PHP framework</p>
              </div>
            </div>
          )}

          {/* Scheduled Backups Option (server-only) */}
          {!isDesktopApp() && (
          <div className="bg-dark-850 p-4 rounded-lg border border-dark-600/70 space-y-3">
            <div className="flex items-center justify-between">
              <div>
                <span className="text-sm font-medium text-dark-100 flex items-center gap-2">
                  <svg className="w-4 h-4 text-rust-400" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                    <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M4 16v1a3 3 0 003 3h10a3 3 0 003-3v-1m-4-8l-4-4m0 0L8 8m4-4v12" />
                  </svg>
                  Configure Scheduled Backups
                </span>
                <p className="text-xs text-dark-400 mt-0.5">
                  Automatically take recurring backups of files and databases. Optional — leave disabled if not required.
                </p>
              </div>
              <button
                type="button"
                onClick={() => setEnableBackup(!enableBackup)}
                className={`relative inline-flex h-5 w-9 shrink-0 items-center rounded-full transition-colors ${
                  enableBackup ? "bg-accent-600" : "bg-dark-600"
                }`}
              >
                <span className={`inline-block h-4 w-4 rounded-full bg-white transition-transform ${enableBackup ? "translate-x-4" : "translate-x-0.5"}`} />
              </button>
            </div>

            {enableBackup && (
              <div className="pt-3 border-t border-dark-700/60 grid grid-cols-1 sm:grid-cols-2 gap-4 animate-fade-in text-xs">
                <div>
                  <label htmlFor="site-backup-frequency" className="block font-medium text-dark-200 mb-1">Backup Frequency</label>
                  <select
                    id="site-backup-frequency"
                    value={backupFrequency}
                    onChange={(e) => setBackupFrequency(e.target.value)}
                    className="w-full px-3 py-2 bg-dark-900 border border-dark-600 rounded-lg text-dark-100 outline-none focus:border-rust-500"
                  >
                    <option value="daily">Daily at 3:00 AM (Recommended)</option>
                    <option value="hourly">Hourly</option>
                    <option value="weekly">Weekly (Sundays at 3:00 AM)</option>
                    <option value="monthly">Monthly (1st at 3:00 AM)</option>
                  </select>
                </div>
                <div>
                  <label htmlFor="site-backup-retention" className="block font-medium text-dark-200 mb-1">Retention Count (Max Backups Kept)</label>
                  <input
                    id="site-backup-retention"
                    type="number"
                    min="1"
                    max="100"
                    value={backupRetention}
                    onChange={(e) => setBackupRetention(Math.max(1, parseInt(e.target.value) || 1))}
                    className="w-full px-3 py-2 bg-dark-900 border border-dark-600 rounded-lg text-dark-100 outline-none focus:border-rust-500"
                  />
                </div>
              </div>
            )}
          </div>
          )}

          <div className="flex items-center gap-3 pt-2">
            <button
              type="submit"
              disabled={submitting}
              className="inline-flex items-center gap-2 px-6 py-2.5 bg-rust-500 text-white rounded-lg text-sm font-medium hover:bg-rust-600 disabled:opacity-50 transition-colors"
            >
              {submitting ? (
                <>
                  <span className="w-4 h-4 border-2 border-white/30 border-t-white rounded-full animate-spin" />
                  Creating...
                </>
              ) : "Create Site"}
            </button>
            <button
              type="button"
              onClick={() => setShowForm(false)}
              className="px-4 py-2 text-sm text-dark-300 border border-dark-600 rounded-lg hover:text-dark-100 hover:border-dark-400 transition-colors"
            >
              Cancel
            </button>
          </div>
        </form>
        </div>
      )}

      {/* Sites list */}
      {loading ? (
        <div className="bg-dark-800 rounded-lg border border-dark-500 animate-pulse">
          {[...Array(3)].map((_, i) => (
            <div key={i} className="px-5 py-4 border-b border-dark-600 last:border-0">
              <div className="h-5 bg-dark-700 rounded w-48" />
            </div>
          ))}
        </div>
      ) : !showForm && sites.length === 0 ? (
        <div className="bg-dark-800 rounded-lg border border-dark-500 p-12 text-center">
          <svg className="w-12 h-12 mx-auto text-dark-300 mb-4" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={1}>
            <path strokeLinecap="round" strokeLinejoin="round" d="M12 21a9.004 9.004 0 0 0 8.716-6.747M12 21a9.004 9.004 0 0 1-8.716-6.747M12 21c2.485 0 4.5-4.03 4.5-9S14.485 3 12 3m0 18c-2.485 0-4.5-4.03-4.5-9S9.515 3 12 3m0 0a8.997 8.997 0 0 1 7.843 4.582M12 3a8.997 8.997 0 0 0-7.843 4.582m15.686 0A11.953 11.953 0 0 1 12 10.5c-2.998 0-5.74-1.1-7.843-2.918m15.686 0A8.959 8.959 0 0 1 21 12c0 .778-.099 1.533-.284 2.253m0 0A17.919 17.919 0 0 1 12 16.5a17.92 17.92 0 0 1-8.716-2.247m0 0A9 9 0 0 1 3 12c0-1.47.353-2.856.978-4.082" />
          </svg>
          <p className="text-dark-200 font-medium text-lg">No sites yet</p>
          {/* The empty state has to say something different to the one role that
              cannot act on it. Inviting a client to "create your first site" is
              the same broken promise as the button above, and it is worse here:
              this is the screen they reach when a site they DO own has not been
              transferred yet, so the panel answered "you have nothing, make one"
              to someone whose only route is to ask their administrator. */}
          {isClient ? (
            <p className="text-dark-300 text-sm mt-2 max-w-md mx-auto">
              No sites have been assigned to your account yet. Sites are created and handed
              over by your administrator — once one is, it appears here and you can manage it
              fully.
            </p>
          ) : (
            <>
              <p className="text-dark-300 text-sm mt-2 max-w-md mx-auto">Deploy static, PHP, Node.js, or Python sites with automatic SSL certificates, nginx configuration, and one-click CMS installs.</p>
              <button onClick={() => setShowForm(true)} className="mt-3 px-4 py-2 bg-rust-500 text-white rounded-lg text-sm font-medium hover:bg-rust-600 transition-colors">
                Create your first site
              </button>
            </>
          )}
        </div>
      ) : sites.length > 0 ? (
        <div>
          {(() => {
            const filtered = sites
              .filter((s) => !s.parent_site_id && s.domain.toLowerCase().includes(search.toLowerCase()))
              .sort((a, b) => {
                let cmp = 0;
                if (sortBy === "domain") {
                  cmp = a.domain.localeCompare(b.domain);
                } else if (sortBy === "created") {
                  cmp = new Date(a.created_at).getTime() - new Date(b.created_at).getTime();
                } else if (sortBy === "disk") {
                  cmp = (metrics[a.id]?.disk_usage_bytes || 0) - (metrics[b.id]?.disk_usage_bytes || 0);
                } else if (sortBy === "traffic") {
                  cmp = (metrics[a.id]?.traffic_bytes_this_month || 0) - (metrics[b.id]?.traffic_bytes_this_month || 0);
                }
                return sortOrder === "asc" ? cmp : -cmp;
              });
            const displayed = filtered.slice(0, displayCount);
            const remaining = filtered.length - displayed.length;

            if (viewMode === "grid") {
              return (
                <>
                  <div className="grid grid-cols-1 md:grid-cols-2 lg:grid-cols-3 gap-4">
                    {displayed.map((site) => {
                      const m = metrics[site.id];
                      return (
                        <div
                          key={site.id}
                          className="bg-dark-800 rounded-lg border border-dark-500 hover:border-dark-400 p-4 transition-all flex flex-col justify-between"
                        >
                          <div>
                            <div className="flex items-start justify-between gap-2 mb-3">
                              <div className="flex items-center gap-2 min-w-0">
                                <SiteFavicon domain={site.domain} />
                                <Link
                                  to={`/sites/${site.id}`}
                                  className="text-sm font-semibold text-rust-400 hover:text-rust-300 font-mono truncate"
                                  title={site.domain}
                                >
                                  {site.domain}
                                </Link>
                              </div>
                              <div className="flex items-center gap-1 shrink-0">
                                <button
                                  onClick={() => copyDomain(site.domain)}
                                  className="p-1 rounded text-dark-300 hover:text-dark-100 hover:bg-dark-700 transition-colors"
                                  title={copiedDomain === site.domain ? "Copied!" : "Copy domain"}
                                >
                                  {copiedDomain === site.domain ? (
                                    <svg className="w-3.5 h-3.5 text-rust-400" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                                      <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M5 13l4 4L19 7" />
                                    </svg>
                                  ) : (
                                    <svg className="w-3.5 h-3.5" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                                      <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M8 16H6a2 2 0 01-2-2V6a2 2 0 012-2h8a2 2 0 012 2v2m-6 12h8a2 2 0 002-2v-8a2 2 0 00-2-2h-8a2 2 0 00-2 2v8a2 2 0 002 2z" />
                                    </svg>
                                  )}
                                </button>
                                <a
                                  href={getSiteExternalUrl(site)}
                                  target="_blank"
                                  rel="noopener noreferrer"
                                  className="p-1 rounded text-dark-300 hover:text-dark-100 hover:bg-dark-700 transition-colors"
                                  title="Open in new tab"
                                >
                                  <svg className="w-3.5 h-3.5" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                                    <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M10 6H6a2 2 0 00-2 2v10a2 2 0 002 2h10a2 2 0 002-2v-4M14 4h6m0 0v6m0-6L10 14" />
                                  </svg>
                                </a>
                              </div>
                            </div>

                            <div className="mb-3">
                              <SiteThumbnail domain={site.domain} className="w-full h-28 rounded border border-dark-600 object-cover" />
                            </div>

                            <div className="space-y-1.5 text-xs text-dark-200 mb-4">
                              <div className="flex items-center justify-between">
                                <span className="text-dark-300">Status:</span>
                                {renderStatusBadge(site)}
                              </div>
                              <div className="flex items-center justify-between">
                                <span className="text-dark-300">Runtime:</span>
                                <span className="font-mono text-dark-100">{runtimeLabels[site.runtime || "php"] || site.runtime || "PHP"}</span>
                              </div>
                              <div className="flex items-center justify-between">
                                <span className="text-dark-300">Disk Usage:</span>
                                <span className="font-mono text-dark-100">{formatSize(m?.disk_usage_bytes || 0)}</span>
                              </div>
                              <div className="flex items-center justify-between">
                                <span className="text-dark-300">Traffic / mo:</span>
                                <span className="font-mono text-dark-100">
                                  {m?.traffic_bytes_this_month ? `${formatSize(m.traffic_bytes_this_month)} / mo` : "0 B / mo"}
                                </span>
                              </div>
                              <div className="pt-1 border-t border-dark-700/60 truncate" title={m?.server_location || `/var/www/${site.domain}/public`}>
                                <span className="text-[11px] text-dark-400 font-mono flex items-center gap-1 truncate">
                                  <svg className="w-3 h-3 shrink-0" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                                    <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M3 7v10a2 2 0 002 2h14a2 2 0 002-2V9a2 2 0 00-2-2h-6l-2-2H5a2 2 0 00-2 2z" />
                                  </svg>
                                  {m?.server_location || `/var/www/${site.domain}/public`}
                                </span>
                              </div>
                            </div>
                          </div>

                          <div className="pt-3 border-t border-dark-600 flex items-center justify-between">
                            <div className="flex items-center gap-1">
                              <Link
                                to={`/files/${site.domain}`}
                                className="p-1.5 rounded text-dark-300 hover:text-dark-100 hover:bg-dark-700 transition-colors"
                                title="File Manager"
                              >
                                <svg className="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                                  <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M3 7v10a2 2 0 002 2h14a2 2 0 002-2V9a2 2 0 00-2-2h-6l-2-2H5a2 2 0 00-2 2z" />
                                </svg>
                              </Link>
                              {!isDesktopApp() && (
                              <Link
                                to={`/mail?domain=${site.domain}`}
                                className={`p-1.5 rounded hover:bg-dark-700 transition-colors ${m?.has_mail ? "text-rust-400" : "text-dark-300 hover:text-dark-100"}`}
                                title={m?.has_mail ? "Mail configured" : "Mail"}
                              >
                                <svg className="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                                  <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M3 8l7.89 5.26a2 2 0 002.22 0L21 8M5 19h14a2 2 0 002-2V7a2 2 0 00-2-2H5a2 2 0 00-2 2v10a2 2 0 002 2z" />
                                </svg>
                              </Link>
                              )}
                              <Link
                                to={`/databases?search=${site.domain}`}
                                className={`p-1.5 rounded hover:bg-dark-700 transition-colors ${m?.has_database ? "text-accent-400" : "text-dark-300 hover:text-dark-100"}`}
                                title={m?.has_database ? "Database connected" : "Databases"}
                              >
                                <svg className="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                                  <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M4 7v10c0 2 1.5 3 3.5 3s3.5-1 3.5-3V7M4 7c0-2 1.5-3 3.5-3s3.5 1 3.5 3M4 7c0 2 1.5 3 3.5 3s3.5-1 3.5-3m7-3v10c0 2 1.5 3 3.5 3s3.5-1 3.5-3V4m0 0c0-2 1.5-3 3.5-3s3.5 1 3.5 3m0 0c0 2 1.5 3 3.5 3s3.5-1 3.5-3" />
                                </svg>
                              </Link>
                              {(!site.runtime || site.runtime === "php" || site.runtime === "static") && (
                                site.php_preset === "wordpress" ? (
                                  <Link
                                    to={`/sites/${site.id}/wordpress`}
                                    className="p-1.5 rounded hover:bg-dark-700 text-[#38bdf8] transition-colors"
                                    title="Manage WordPress"
                                  >
                                    <svg className="w-4 h-4" viewBox="0 0 24 24" fill="currentColor">
                                      <path d="M12 2C6.486 2 2 6.486 2 12s4.486 10 10 10 10-4.486 10-10S17.514 2 12 2zm0 2c1.67 0 3.214.52 4.488 1.401L5.401 16.488A7.957 7.957 0 0 1 4 12c0-4.411 3.589-8 8-8zm0 16c-1.67 0-3.214-.52-4.488-1.401L18.599 7.512A7.957 7.957 0 0 1 20 12c0 4.411-3.589 8-8 8z" />
                                    </svg>
                                  </Link>
                                ) : (
                                  <button
                                    onClick={() => setWpInstallSite(site)}
                                    className="p-1.5 rounded hover:bg-dark-700 text-dark-300 hover:text-[#38bdf8] transition-colors"
                                    title="Install WordPress"
                                  >
                                    <svg className="w-4 h-4" viewBox="0 0 24 24" fill="currentColor">
                                      <path d="M12 2C6.486 2 2 6.486 2 12s4.486 10 10 10 10-4.486 10-10S17.514 2 12 2zm0 2c1.67 0 3.214.52 4.488 1.401L5.401 16.488A7.957 7.957 0 0 1 4 12c0-4.411 3.589-8 8-8zm0 16c-1.67 0-3.214-.52-4.488-1.401L18.599 7.512A7.957 7.957 0 0 1 20 12c0 4.411-3.589 8-8 8z" />
                                    </svg>
                                  </button>
                                )
                              )}
                              {isDesktopApp() && (
                                <>
                                  <button
                                    onClick={() => setSyncSite(site)}
                                    className="p-1.5 rounded hover:bg-dark-700 text-dark-300 hover:text-dark-100 transition-colors"
                                    title="Live Server Sync (Push / Pull)"
                                  >
                                    <svg className="w-4 h-4 text-accent-400" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                                      <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M4 4v5h.582m15.356 2A8.001 8.001 0 004.582 9m0 0H9m11 11v-5h-.581m0 0a8.003 8.003 0 01-15.357-2m15.357 2H15" />
                                    </svg>
                                  </button>
                                  <DesktopSiteActions
                                    domain={site.domain}
                                    siteId={site.id}
                                    status={site.status}
                                    onStatusChange={(newStatus) => handleSiteStatusChange(site.id, newStatus)}
                                    onError={setError}
                                  />
                                </>
                              )}
                            </div>
                            <Link
                              to={`/sites/${site.id}`}
                              className="px-3 py-1 bg-dark-700 hover:bg-dark-600 text-dark-100 text-xs font-medium rounded transition-colors"
                            >
                              Manage
                            </Link>
                          </div>
                        </div>
                      );
                    })}
                  </div>
                  {remaining > 0 && (
                    <button
                      onClick={() => setDisplayCount((c) => c + 25)}
                      className="w-full mt-4 py-2 text-sm text-dark-300 hover:text-dark-100 bg-dark-800 rounded-lg border border-dark-600 hover:bg-dark-700/30 transition-colors"
                    >
                      Show more ({remaining} remaining)
                    </button>
                  )}
                </>
              );
            }

            return (
              <div className="bg-dark-800 rounded-lg border border-dark-500 overflow-x-auto elevation-1">
                <table className="w-full">
                  <thead>
                    <tr className="border-b border-dark-500 bg-dark-900">
                      <th scope="col" className="text-left text-xs font-medium text-dark-200 uppercase tracking-widest font-mono px-5 py-3">Domain</th>
                      <th scope="col" className="text-left text-xs font-medium text-dark-200 uppercase tracking-widest font-mono px-5 py-3 hidden xl:table-cell">Server Location</th>
                      <th scope="col" className="text-left text-xs font-medium text-dark-200 uppercase tracking-widest font-mono px-4 py-3">Disk Usage</th>
                      <th scope="col" className="text-left text-xs font-medium text-dark-200 uppercase tracking-widest font-mono px-4 py-3 hidden md:table-cell">Traffic / mo</th>
                      <th scope="col" className="text-left text-xs font-medium text-dark-200 uppercase tracking-widest font-mono px-4 py-3 hidden sm:table-cell">Runtime</th>
                      <th scope="col" className="text-left text-xs font-medium text-dark-200 uppercase tracking-widest font-mono px-4 py-3">Status</th>
                      <th scope="col" className="text-center text-xs font-medium text-dark-200 uppercase tracking-widest font-mono px-4 py-3">Quick Actions</th>
                      {allSites && (
                        <th scope="col" className="text-left text-xs font-medium text-dark-200 uppercase tracking-widest font-mono px-4 py-3 hidden lg:table-cell">Owner</th>
                      )}
                    </tr>
                  </thead>
                  <tbody className="divide-y divide-dark-600">
                    {displayed.map((site) => {
                      const m = metrics[site.id];
                      return (
                        <tr key={site.id} className="hover:bg-dark-700/30 transition-colors">
                          <td className="px-5 py-4">
                            <div className="flex items-center gap-3">
                              <SiteFavicon domain={site.domain} />
                              <SiteThumbnail domain={site.domain} className="w-10 h-7 rounded border border-dark-600 hidden sm:block shrink-0" />
                              <div className="min-w-0">
                                <div className="flex items-center gap-1.5">
                                  <Link
                                    to={`/sites/${site.id}`}
                                    className="text-sm font-medium text-rust-400 hover:text-rust-300 font-mono truncate"
                                  >
                                    {site.domain}
                                  </Link>
                                  <button
                                    onClick={() => copyDomain(site.domain)}
                                    className="p-1 rounded text-dark-300 hover:text-dark-100 hover:bg-dark-700 transition-colors"
                                    title={copiedDomain === site.domain ? "Copied!" : "Copy domain"}
                                  >
                                    {copiedDomain === site.domain ? (
                                      <svg className="w-3.5 h-3.5 text-rust-400" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                                        <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M5 13l4 4L19 7" />
                                      </svg>
                                    ) : (
                                      <svg className="w-3.5 h-3.5" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                                        <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M8 16H6a2 2 0 01-2-2V6a2 2 0 012-2h8a2 2 0 012 2v2m-6 12h8a2 2 0 002-2v-8a2 2 0 00-2-2h-8a2 2 0 00-2 2v8a2 2 0 002 2z" />
                                      </svg>
                                    )}
                                  </button>
                                  <a
                                    href={getSiteExternalUrl(site)}
                                    target="_blank"
                                    rel="noopener noreferrer"
                                    className="p-1 rounded text-dark-300 hover:text-dark-100 hover:bg-dark-700 transition-colors"
                                    title="Open in new tab"
                                  >
                                    <svg className="w-3.5 h-3.5" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                                      <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M10 6H6a2 2 0 00-2 2v10a2 2 0 002 2h10a2 2 0 002-2v-4M14 4h6m0 0v6m0-6L10 14" />
                                    </svg>
                                  </a>
                                </div>
                                <div className="text-[11px] text-dark-400 font-mono xl:hidden truncate">
                                  {m?.server_location || `/var/www/${site.domain}/public`}
                                </div>
                              </div>
                            </div>
                          </td>
                          <td className="px-5 py-4 text-xs font-mono text-dark-300 hidden xl:table-cell max-w-xs truncate">
                            <span className="flex items-center gap-1.5 truncate" title={m?.server_location || `/var/www/${site.domain}/public`}>
                              <svg className="w-3.5 h-3.5 text-dark-400 shrink-0" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                                <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M3 7v10a2 2 0 002 2h14a2 2 0 002-2V9a2 2 0 00-2-2h-6l-2-2H5a2 2 0 00-2 2z" />
                              </svg>
                              <span className="truncate">{m?.server_location || `/var/www/${site.domain}/public`}</span>
                            </span>
                          </td>
                          <td className="px-4 py-4 text-xs font-mono text-dark-100 whitespace-nowrap">
                            {formatSize(m?.disk_usage_bytes || 0)}
                          </td>
                          <td className="px-4 py-4 text-xs font-mono text-dark-200 hidden md:table-cell whitespace-nowrap">
                            {m?.traffic_bytes_this_month ? `${formatSize(m.traffic_bytes_this_month)} / mo` : "0 B / mo"}
                          </td>
                          <td className="px-4 py-4 text-sm text-dark-200 hidden sm:table-cell">
                            {runtimeLabels[site.runtime || "php"] || site.runtime || "PHP"}
                          </td>
                          <td className="px-4 py-4 whitespace-nowrap">
                            <div className="flex items-center gap-1.5">
                              {renderStatusBadge(site)}
                              {site.ssl_enabled ? (
                                <span className="inline-flex items-center text-rust-400" title="SSL Active">
                                  <svg className="w-3.5 h-3.5" fill="currentColor" viewBox="0 0 20 20">
                                    <path fillRule="evenodd" d="M10 1a4.5 4.5 0 0 0-4.5 4.5V9H5a2 2 0 0 0-2 2v6a2 2 0 0 0 2 2h10a2 2 0 0 0 2-2v-6a2 2 0 0 0-2-2h-.5V5.5A4.5 4.5 0 0 0 10 1Zm3 8V5.5a3 3 0 1 0-6 0V9h6Z" clipRule="evenodd" />
                                  </svg>
                                </span>
                              ) : (
                                <span className="text-[10px] text-dark-400">no SSL</span>
                              )}
                            </div>
                          </td>
                          <td className="px-4 py-4 text-center whitespace-nowrap">
                            <div className="flex items-center justify-center gap-1">
                              <Link
                                to={`/files/${site.domain}`}
                                className="p-1.5 rounded hover:bg-dark-700 text-dark-300 hover:text-dark-100 transition-colors"
                                title="File Manager"
                              >
                                <svg className="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                                  <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M3 7v10a2 2 0 002 2h14a2 2 0 002-2V9a2 2 0 00-2-2h-6l-2-2H5a2 2 0 00-2 2z" />
                                </svg>
                              </Link>
                              {!isDesktopApp() && (
                              <Link
                                to={`/mail?domain=${site.domain}`}
                                className={`p-1.5 rounded hover:bg-dark-700 transition-colors ${m?.has_mail ? "text-rust-400" : "text-dark-300 hover:text-dark-100"}`}
                                title={m?.has_mail ? "Mail configured" : "Mail"}
                              >
                                <svg className="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                                  <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M3 8l7.89 5.26a2 2 0 002.22 0L21 8M5 19h14a2 2 0 002-2V7a2 2 0 00-2-2H5a2 2 0 00-2 2v10a2 2 0 002 2z" />
                                </svg>
                              </Link>
                              )}
                              <Link
                                to={`/databases?search=${site.domain}`}
                                className={`p-1.5 rounded hover:bg-dark-700 transition-colors ${m?.has_database ? "text-accent-400" : "text-dark-300 hover:text-dark-100"}`}
                                title={m?.has_database ? "Database connected" : "Databases"}
                              >
                                <svg className="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                                  <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M4 7v10c0 2 1.5 3 3.5 3s3.5-1 3.5-3V7M4 7c0-2 1.5-3 3.5-3s3.5 1 3.5 3M4 7c0 2 1.5 3 3.5 3s3.5-1 3.5-3m7-3v10c0 2 1.5 3 3.5 3s3.5-1 3.5-3V4m0 0c0-2 1.5-3 3.5-3s3.5 1 3.5 3m0 0c0 2 1.5 3 3.5 3s3.5-1 3.5-3" />
                                </svg>
                              </Link>
                              {(!site.runtime || site.runtime === "php" || site.runtime === "static") && (
                                site.php_preset === "wordpress" ? (
                                  <Link
                                    to={`/sites/${site.id}/wordpress`}
                                    className="p-1.5 rounded hover:bg-dark-700 text-[#38bdf8] transition-colors"
                                    title="Manage WordPress"
                                  >
                                    <svg className="w-4 h-4" viewBox="0 0 24 24" fill="currentColor">
                                      <path d="M12 2C6.486 2 2 6.486 2 12s4.486 10 10 10 10-4.486 10-10S17.514 2 12 2zm0 2c1.67 0 3.214.52 4.488 1.401L5.401 16.488A7.957 7.957 0 0 1 4 12c0-4.411 3.589-8 8-8zm0 16c-1.67 0-3.214-.52-4.488-1.401L18.599 7.512A7.957 7.957 0 0 1 20 12c0 4.411-3.589 8-8 8z" />
                                    </svg>
                                  </Link>
                                ) : (
                                  <button
                                    onClick={() => setWpInstallSite(site)}
                                    className="p-1.5 rounded hover:bg-dark-700 text-dark-300 hover:text-[#38bdf8] transition-colors"
                                    title="Install WordPress"
                                  >
                                    <svg className="w-4 h-4" viewBox="0 0 24 24" fill="currentColor">
                                      <path d="M12 2C6.486 2 2 6.486 2 12s4.486 10 10 10 10-4.486 10-10S17.514 2 12 2zm0 2c1.67 0 3.214.52 4.488 1.401L5.401 16.488A7.957 7.957 0 0 1 4 12c0-4.411 3.589-8 8-8zm0 16c-1.67 0-3.214-.52-4.488-1.401L18.599 7.512A7.957 7.957 0 0 1 20 12c0 4.411-3.589 8-8 8z" />
                                    </svg>
                                  </button>
                                )
                              )}
                              {isDesktopApp() && (
                                <>
                                  <button
                                    onClick={() => setSyncSite(site)}
                                    className="p-1.5 rounded hover:bg-dark-700 text-dark-300 hover:text-dark-100 transition-colors"
                                    title="Live Server Sync (Push / Pull)"
                                  >
                                    <svg className="w-4 h-4 text-accent-400" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                                      <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M4 4v5h.582m15.356 2A8.001 8.001 0 004.582 9m0 0H9m11 11v-5h-.581m0 0a8.003 8.003 0 01-15.357-2m15.357 2H15" />
                                    </svg>
                                  </button>
                                  <DesktopSiteActions
                                    domain={site.domain}
                                    siteId={site.id}
                                    status={site.status}
                                    onStatusChange={(newStatus) => handleSiteStatusChange(site.id, newStatus)}
                                    onError={setError}
                                  />
                                </>
                              )}
                            </div>
                          </td>
                          {allSites && (
                            <td className="px-4 py-4 text-sm hidden lg:table-cell">
                              <span className="text-dark-100 font-mono text-xs">{site.owner_email ?? "—"}</span>
                              {site.owner_role && site.owner_role !== "admin" && (
                                <span className="ml-2 inline-flex px-2 py-0.5 rounded-full text-[10px] font-medium bg-dark-700 text-dark-200 uppercase tracking-wide">
                                  {site.owner_role}
                                </span>
                              )}
                            </td>
                          )}
                        </tr>
                      );
                    })}
                  </tbody>
                </table>
                {remaining > 0 && (
                  <button
                    onClick={() => setDisplayCount((c) => c + 25)}
                    className="w-full py-2 text-sm text-dark-300 hover:text-dark-100 border-t border-dark-600 hover:bg-dark-700/30 transition-colors"
                  >
                    Show more ({remaining} remaining)
                  </button>
                )}
              </div>
            );
          })()}
        </div>
      ) : null}

      {notice && (
        <div role="status" className="mt-4 bg-dark-700 text-dark-100 text-sm px-4 py-3 rounded-lg border border-dark-500">
          {notice}
        </div>
      )}

      {/* Transfer, from the all-sites list rather than the site's own page.
          It has to live here: once a site belongs to somebody else its detail
          page answers 404 to the admin, so a control rendered only there can
          hand a site away and never take it back. */}
      {transferFor && (
        <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/60 p-4" role="dialog" aria-modal="true" aria-labelledby="transfer-title">
          <div className="bg-dark-800 border border-dark-500 rounded-lg elevation-2 w-full max-w-md p-5">
            <h2 id="transfer-title" className="text-base font-medium text-dark-100">Transfer {transferFor.domain}</h2>
            <p className="text-xs text-dark-300 mt-1">
              Ownership is exclusive. The account you choose becomes the owner and the current
              one — {transferFor.owner_email ?? "the current owner"} — stops being it.
            </p>
            <label htmlFor="transfer-to" className="block text-sm font-medium text-dark-100 mt-4 mb-1">Transfer to</label>
            <select
              id="transfer-to"
              value={transferEmail}
              onChange={(e) => setTransferEmail(e.target.value)}
              className="w-full px-3 py-2.5 border border-dark-500 rounded-lg focus:ring-2 focus:ring-accent-500 outline-none text-sm bg-dark-800"
            >
              <option value="">Choose an account…</option>
              {users
                /* `suspended` is refused by the endpoint with a 409 — leaving it
                   out of the list is the same rule stated earlier. Every other
                   role is a legitimate destination, including admin, which is
                   how a transfer is undone. */
                .filter((u) => u.role !== "suspended" && u.email !== transferFor.owner_email)
                .map((u) => (
                  <option key={u.id} value={u.email}>{u.email} ({u.role})</option>
                ))}
            </select>
            <div className="flex items-center gap-2 mt-5">
              <button
                disabled={!transferEmail || transferring}
                onClick={() => {
                  setTransferring(true);
                  api.post(`/sites/${transferFor.id}/transfer`, { email: transferEmail })
                    .then(() => {
                      setNotice(`${transferFor.domain} transferred to ${transferEmail}.`);
                      setTransferFor(null);
                      fetchSites();
                    })
                    .catch((e) => setError(e instanceof Error ? e.message : "Transfer failed"))
                    .finally(() => setTransferring(false));
                }}
                className="px-4 py-2 bg-rust-500 text-white rounded-lg text-sm font-medium hover:bg-rust-600 disabled:opacity-50 transition-colors"
              >
                {transferring ? "Transferring…" : "Transfer"}
              </button>
              <button
                onClick={() => setTransferFor(null)}
                className="px-4 py-2 text-sm text-dark-300 border border-dark-600 rounded-lg hover:text-dark-100 hover:border-dark-400 transition-colors"
              >
                Cancel
              </button>
            </div>
          </div>
        </div>
      )}
      {syncSite && (
        <LiveSyncModal
          site={syncSite}
          isOpen={!!syncSite}
          onClose={() => setSyncSite(null)}
        />
      )}
      {wpInstallSite && (
        <InstallWordPressModal
          isOpen={!!wpInstallSite}
          onClose={() => setWpInstallSite(null)}
          siteId={wpInstallSite.id}
          domain={wpInstallSite.domain}
          onSuccess={fetchSites}
        />
      )}
      <PortConflictModal
        isOpen={showPortModal}
        onClose={() => setShowPortModal(false)}
        conflictInfo={conflictInfo}
        onResolved={(newPort) => {
          setConflictInfo((prev) =>
            prev
              ? {
                  ...prev,
                  active_port: newPort,
                  in_use: newPort === prev.preferred_port ? false : prev.in_use,
                }
              : null
          );
          showGlobalNotification("success", `Local site router switched to port ${newPort}`);
        }}
      />
      </div>
    </div>
  );
}
