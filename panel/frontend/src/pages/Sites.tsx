import { useState, useEffect, useRef, FormEvent } from "react";
import { Link } from "react-router-dom";
import { api, ApiError } from "../api";
import { formatDate, formatSize } from "../utils/format";
import { statusColors, runtimeLabels } from "../constants";
import ProvisionLog, {
  type ProvisionStep,
  type ProvisionCompatMeta,
} from "../components/ProvisionLog";
import ConfirmDialog from "../components/ConfirmDialog";
import { PrereqCallout, useDnsPrereq } from "../components/Prerequisite";
import { FieldHelp, InfoTip } from "../components/FieldHelp";
import PhpVersionPicker from "../components/PhpVersionPicker";
import SiteThumbnail from "../components/SiteThumbnail";
import SiteFavicon from "../components/SiteFavicon";
import DesktopSiteActions from "../components/DesktopSiteActions";
import DiscoveredFoldersPanel, {
  type DiscoveredFolder,
  type FolderImportRequest,
  type FailedFolderImportState,
  type DetectedDbFile,
  MEMORY_LIMIT_OPTIONS,
} from "../components/DiscoveredFoldersPanel";
import DirectoryDbSelector from "../components/DirectoryDbSelector";
import InstallWordPressModal from "../components/InstallWordPressModal";
import LiveSyncModal from "../components/LiveSyncModal";
import PortConflictModal, {
  PortConflictInfo,
  buildSiteExternalUrl,
} from "../components/PortConflictModal";
import { isDesktopApp, formatDesktopDomain } from "../utils/desktop";
import { showGlobalNotification, buildAlertDetails } from "../context/NotificationContext";
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
  const [provisionJob, setProvisionJob] = useState<{
    siteId?: string;
    sseUrl?: string;
    title?: string;
    defaultShowLogs?: boolean;
    externalSteps?: ProvisionStep[];
  } | null>(null);
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
  const [browsingFolder, setBrowsingFolder] = useState(false);
  const [inspectingFolder, setInspectingFolder] = useState(false);
  const [folderInspection, setFolderInspection] = useState<{
    path: string;
    exists: boolean;
    is_dir: boolean;
    is_empty: boolean;
    file_count: number;
    detected_type: string;
    is_wordpress: boolean;
    has_wp_config: boolean;
    wp_version?: string | null;
    wp_table_prefix?: string | null;
    wp_db_name?: string | null;
    wp_siteurl?: string | null;
    detected_db_files?: DetectedDbFile[];
    other_directory_db_files?: DetectedDbFile[];
  } | null>(null);
  const [wpDbFile, setWpDbFile] = useState<File | null>(null);
  const [wpLocalDb, setWpLocalDb] = useState<DetectedDbFile | null>(null);
  const [wpSourceUrl, setWpSourceUrl] = useState("");
  const [wpMemoryLimitMb, setWpMemoryLimitMb] = useState<number>(1024);
  const [skipWpDbUpload, setSkipWpDbUpload] = useState(false);
  const [discoveredFolders, setDiscoveredFolders] = useState<DiscoveredFolder[]>([]);
  const [discoveredRootDbFiles, setDiscoveredRootDbFiles] = useState<DetectedDbFile[]>([]);
  const [scanningFolders, setScanningFolders] = useState(false);
  const [failedFolderImports, setFailedFolderImports] = useState<Record<string, FailedFolderImportState>>({});
  const [compatPrompt, setCompatPrompt] = useState<{
    error: string;
    fixes: string[];
    targetVersion?: string;
    retry: () => Promise<void>;
  } | null>(null);
  const activeImportRef = useRef<{
    siteId: string;
    domain: string;
    runtime: string;
    isWordPress: boolean;
    appCommand?: string;
    isConnectExisting: boolean;
    folderPath?: string;
    dbFile: File | null;
    localDbPath: string;
    localDbObj: DetectedDbFile | null;
    sourceUrl: string;
    dbName: string;
    memoryLimitMb: number;
  } | null>(null);
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

  const discoverFolders = () => {
    if (!isDesktopApp()) return;
    setScanningFolders(true);
    api
      .get<{
        sites_dir?: string;
        folders?: DiscoveredFolder[];
        root_db_files?: DetectedDbFile[];
      }>("/desktop/discover-folders")
      .then((res) => {
        if (res?.sites_dir) setDefaultSitesDir(res.sites_dir);
        setDiscoveredFolders(Array.isArray(res?.folders) ? res.folders : []);
        setDiscoveredRootDbFiles(Array.isArray(res?.root_db_files) ? res.root_db_files : []);
      })
      .catch(() => {})
      .finally(() => setScanningFolders(false));
  };

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
    discoverFolders();
  }, []);

  useEffect(() => {
    if (!isDesktopApp()) return;
    const trimmed = customRootPath.trim();
    if (!trimmed) {
      setInspectingFolder(false);
      setFolderInspection(null);
      setWpDbFile(null);
      setWpLocalDb(null);
      setWpSourceUrl("");
      setSkipWpDbUpload(false);
      return;
    }
    setInspectingFolder(true);
    const timer = setTimeout(() => {
      api
        .post<NonNullable<typeof folderInspection>>("/desktop/inspect-folder", { path: trimmed })
        .then((insp) => applyFolderInspection(insp))
        .catch(() => {})
        .finally(() => setInspectingFolder(false));
    }, 300);
    return () => {
      clearTimeout(timer);
      setInspectingFolder(false);
    };
  }, [customRootPath]);

  const applyFolderInspection = (insp: NonNullable<typeof folderInspection>) => {
    setFolderInspection(insp);
    const detectedList = Array.isArray(insp.detected_db_files) ? insp.detected_db_files : [];
    const suggestedDb =
      detectedList.find((d) => d.matches_site === true) ||
      detectedList.find((d) => d.matches_site !== false) ||
      null;
    if (!wpDbFile && !wpLocalDb && suggestedDb) {
      setWpLocalDb(suggestedDb);
    }
    if (insp.is_wordpress) {
      setCms("");
      setRuntime("php");
      setPhpPreset("wordpress");
      if ((insp.wp_siteurl || suggestedDb?.detected_site_url) && !wpSourceUrl) {
        setWpSourceUrl(insp.wp_siteurl || suggestedDb?.detected_site_url || "");
      }
    } else if (!insp.is_empty && insp.exists && insp.is_dir) {
      setCms("");
      if (insp.detected_type === "node") setRuntime("node");
      else if (insp.detected_type === "php") setRuntime("php");
      else if (insp.detected_type === "static") setRuntime("static");
    }
  };

  const handleBrowseFolder = async () => {
    setError("");
    setBrowsingFolder(true);
    try {
      const res = await api.post<{
        cancelled: boolean;
        path?: string;
        inspection?: NonNullable<typeof folderInspection>;
      }>("/desktop/pick-folder", {
        initial_dir: customRootPath.trim() || defaultSitesDir || undefined,
      });
      if (!res.cancelled && res.path) {
        setCustomRootPath(res.path);
        if (res.inspection) {
          applyFolderInspection(res.inspection);
        }
      }
    } catch (err) {
      setError(err instanceof Error ? err.message : "Failed opening folder picker");
    } finally {
      setBrowsingFolder(false);
    }
  };

  const fileToBase64 = (file: File, onReadProgress?: (pct: number) => void): Promise<string> =>
    new Promise((resolve, reject) => {
      const reader = new FileReader();
      reader.onprogress = (ev) => {
        if (ev.lengthComputable && onReadProgress) {
          onReadProgress(Math.round((ev.loaded / ev.total) * 100));
        }
      };
      reader.onload = () => {
        const result = typeof reader.result === "string" ? reader.result : "";
        const commaIdx = result.indexOf(",");
        onReadProgress?.(100);
        resolve(commaIdx >= 0 ? result.slice(commaIdx + 1) : result);
      };
      reader.onerror = () => reject(new Error("Failed reading selected database file"));
      reader.readAsDataURL(file);
    });

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

    const isExistingWpDir = isDesktopApp() && !!folderInspection?.is_wordpress;
    if (isExistingWpDir && !skipWpDbUpload && !wpDbFile && !wpLocalDb) {
      setError("Existing WordPress directory detected — please select or upload a database dump (.sql, .sql.gz, .zip, or .sqlite) or check 'Skip database upload for now'.");
      return;
    }

    setSubmitting(true);
    try {
      const effectiveRuntime = isExistingWpDir ? "php" : (cms ? "php" : runtime);
      const effectivePreset = isExistingWpDir ? "wordpress" : (cms || phpPreset);
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
        if (folderInspection && !folderInspection.is_empty) {
          body.connect_existing = true;
        }
      }
      if (cms && !isExistingWpDir) {
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
      const isConnectExisting = !!body.connect_existing;
      const canAttachDb = isDesktopApp() && !!folderInspection && !folderInspection.is_empty && folderInspection.is_dir;
      const selectedDbFile = canAttachDb && wpDbFile && !skipWpDbUpload ? wpDbFile : null;
      const selectedLocalDb = canAttachDb && !wpDbFile && !skipWpDbUpload ? wpLocalDb : null;
      const selectedFolderPath = customRootPath.trim();
      const selectedSourceUrl = wpSourceUrl.trim();

      setShowForm(false);
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
      setFolderInspection(null);
      setWpDbFile(null);
      setWpLocalDb(null);
      setWpSourceUrl("");
      setSkipWpDbUpload(false);
      setEnableBackup(false);
      setBackupFrequency("daily");
      setBackupRetention(7);
      fetchSites();
      discoverFolders();

      await runAfterCreateFlow(created, {
        isConnectExisting,
        isWordPress: isExistingWpDir,
        runtime: effectiveRuntime,
        appCommand: effectiveRuntime === "node" ? appCommand : undefined,
        folderPath: selectedFolderPath,
        dbFile: selectedDbFile,
        localDbFile: selectedLocalDb,
        sourceUrl: selectedSourceUrl,
        memoryLimitMb: wpMemoryLimitMb,
      });
    } catch (err) {
      setError(err instanceof Error ? err.message : "Failed to create site");
    } finally {
      setSubmitting(false);
    }
  };

  const suggestWorkableMemoryMb = (errMsg: string, curMb: number): { nextMb: number; suggestion?: string } => {
    const lower = errMsg.toLowerCase();
    const isMemory =
      lower.includes("allowed memory size") ||
      lower.includes("out of memory") ||
      lower.includes("memory_limit") ||
      lower.includes("max_allowed_packet") ||
      lower.includes("packet bigger") ||
      lower.includes("413") ||
      lower.includes("exhausted");
    if (!isMemory) {
      return { nextMb: curMb };
    }
    const nextMb = curMb < 0 ? -1 : curMb < 2048 ? 2048 : curMb < 4096 ? 4096 : -1;
    const label = nextMb <= 0 ? "Unlimited (-1)" : `${nextMb} MB`;
    return {
      nextMb,
      suggestion: `Workable Fix: Memory limit (${curMb <= 0 ? "Unlimited" : `${curMb} MB`}) was exhausted. Try bumping Import Memory to ${label} and click Retry.`,
    };
  };

  const rollbackFailedImport = async (errMsg: string) => {
    const ctx = activeImportRef.current;
    activeImportRef.current = null;
    if (!ctx) return;

    if (ctx.isConnectExisting) {
      await api.delete(`/sites/${ctx.siteId}?keep_files=true`).catch(() => {});
      if (ctx.folderPath) {
        const memAdvice = suggestWorkableMemoryMb(errMsg, ctx.memoryLimitMb);
        setFailedFolderImports((prev) => ({
          ...prev,
          [ctx.folderPath!]: {
            domain: ctx.domain,
            runtime: ctx.runtime,
            appCommand: ctx.appCommand || "",
            dbFile: ctx.dbFile,
            localDbPath: ctx.localDbPath,
            localDb: ctx.localDbObj,
            sourceUrl: ctx.sourceUrl,
            dbName: ctx.dbName,
            memoryLimitMb: memAdvice.nextMb,
            error: errMsg,
            memorySuggestion: memAdvice.suggestion,
          },
        }));
      }
    }
    fetchSites();
    discoverFolders();
  };

  const runAfterCreateFlow = async (
    created: Site,
    opts: {
      isConnectExisting: boolean;
      isWordPress: boolean;
      runtime?: string;
      appCommand?: string;
      folderPath?: string;
      dbFile: File | null;
      localDbFile: DetectedDbFile | null;
      sourceUrl?: string;
      dbName?: string;
      memoryLimitMb?: number;
      applyCompatFixes?: boolean;
    }
  ) => {
    const memLimitMb = opts.memoryLimitMb ?? 1024;
    activeImportRef.current = opts.isConnectExisting
      ? {
          siteId: created.id,
          domain: created.domain,
          runtime: opts.runtime || created.runtime || "php",
          isWordPress: opts.isWordPress,
          appCommand: opts.appCommand,
          isConnectExisting: true,
          folderPath: opts.folderPath,
          dbFile: opts.dbFile,
          localDbPath: opts.localDbFile?.path || "",
          localDbObj: opts.localDbFile,
          sourceUrl: opts.sourceUrl || "",
          dbName: opts.dbName || "",
          memoryLimitMb: memLimitMb,
        }
      : null;

    const targetUrl =
      sslPort && created.ssl_enabled
        ? buildSiteExternalUrl(created.domain, sslPort, true)
        : buildSiteExternalUrl(created.domain, conflictInfo?.active_port, false);
    const hasDbToImport = !!(opts.dbFile || opts.localDbFile);
    const importEndpoint = opts.isWordPress
      ? `/sites/${created.id}/wordpress/import-database`
      : `/sites/${created.id}/database/import`;

    if (hasDbToImport) {
      const ts = () => new Date().toTimeString().slice(0, 8);
      const fmtSize = (bytes: number) =>
        bytes < 1024 * 1024
          ? `${Math.max(1, Math.round(bytes / 1024))} KB`
          : `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
      const dbFilename = opts.dbFile ? opts.dbFile.name : opts.localDbFile!.name;
      const dbBytes = opts.dbFile ? opts.dbFile.size : opts.localDbFile!.size_bytes;
      const fileSizeLabel = fmtSize(dbBytes);
      const connectMsg = `[${ts()}] Folder: ${opts.folderPath || created.domain}\n[${ts()}] Registered ${created.domain} -> ${targetUrl}`;
      const initialUploadMsg = `[${ts()}] ${opts.dbFile ? `Reading ${dbFilename}` : `Using local file ${dbFilename}`} (${fileSizeLabel})...`;

      const updateUploadStep = (uploadStep: ProvisionStep, extraStep?: ProvisionStep) => {
        setProvisionJob((prev) => {
          const base = (prev?.externalSteps || [
            {
              step: "site_connect",
              label: `Connected existing folder for ${created.domain}`,
              status: "done",
              progress: 100,
              message: connectMsg,
            },
          ]).filter((s) => s.step !== "upload" && (!extraStep || s.step !== extraStep.step));
          const nextSteps = [...base, uploadStep];
          if (extraStep) nextSteps.push(extraStep);
          return {
            sseUrl: prev?.sseUrl,
            title: `Importing ${created.domain} & Database...`,
            defaultShowLogs: true,
            externalSteps: nextSteps,
          };
        });
      };

      setProvisionJob({
        title: `Importing ${created.domain} & Database...`,
        defaultShowLogs: true,
        externalSteps: [
          {
            step: "site_connect",
            label: `Connected existing folder for ${created.domain}`,
            status: "done",
            progress: 100,
            message: connectMsg,
          },
          {
            step: "upload",
            label: opts.dbFile
              ? `Reading ${dbFilename} (${fileSizeLabel})...`
              : `Preparing local ${dbFilename} (${fileSizeLabel})...`,
            status: "in_progress",
            progress: 5,
            message: initialUploadMsg,
          },
        ],
      });
      showGlobalNotification(
        "info",
        `Uploading & importing database for ${created.domain}...`,
        "Database Import Started",
        `Site: ${created.domain}\nDump: ${dbFilename} (${fileSizeLabel})\nMemory Limit: ${memLimitMb <= 0 ? "Unlimited" : `${memLimitMb} MB`}`,
        { category: "database", target: created.domain }
      );

      let uploadLog = initialUploadMsg;
      let lastLoggedBucket = -1;
      void (async () => {
        try {
          let payload: Record<string, unknown>;
          if (opts.dbFile) {
            const contentBase64 = await fileToBase64(opts.dbFile, (readPct) => {
              const overallPct = Math.max(5, Math.min(25, Math.round(readPct * 0.25)));
              updateUploadStep({
                step: "upload",
                label: `Preparing ${dbFilename} (${readPct}%)...`,
                status: "in_progress",
                progress: overallPct,
                message: uploadLog,
              });
            });
            uploadLog += `\n[${ts()}] Uploading ${dbFilename} (${fileSizeLabel}) to local server...`;
            updateUploadStep({
              step: "upload",
              label: `Uploading ${dbFilename} (0%)...`,
              status: "in_progress",
              progress: 25,
              message: uploadLog,
            });
            payload = {
              filename: dbFilename,
              content_base64: contentBase64,
              source_url: opts.sourceUrl || undefined,
              target_url: targetUrl,
              db_name: opts.dbName || undefined,
              memory_limit_mb: memLimitMb,
              rollback_on_failure: opts.isConnectExisting,
              apply_compat_fixes: !!opts.applyCompatFixes,
              stream: true,
            };
          } else {
            payload = {
              filename: dbFilename,
              local_path: opts.localDbFile!.path,
              source_url: opts.sourceUrl || undefined,
              target_url: targetUrl,
              db_name: opts.dbName || undefined,
              memory_limit_mb: memLimitMb,
              rollback_on_failure: opts.isConnectExisting,
              apply_compat_fixes: !!opts.applyCompatFixes,
              stream: true,
            };
          }

          const imp = await api.upload<{
            ok?: boolean;
            streaming?: boolean;
            job_id?: string;
            sse_url?: string;
            db_name?: string;
          }>(
            importEndpoint,
            payload,
            (loaded, total) => {
              const upPct = total > 0 ? Math.round((loaded / total) * 100) : 50;
              const combinedPct = Math.min(98, 25 + Math.round(upPct * 0.73));
              const bucket = Math.floor(upPct / 25);
              if (bucket > lastLoggedBucket && upPct < 100) {
                lastLoggedBucket = bucket;
                uploadLog += `\n[${ts()}] Uploaded ${fmtSize(loaded)} / ${fmtSize(total)} (${upPct}%)`;
              }
              updateUploadStep({
                step: "upload",
                label: `Uploading ${dbFilename}: ${fmtSize(loaded)} / ${fmtSize(total)} (${upPct}%)`,
                status: "in_progress",
                progress: combinedPct,
                message: uploadLog,
              });
            }
          );

          uploadLog += `\n[${ts()}] Upload complete — starting server-side import pipeline...`;
          if (imp.sse_url) {
            setProvisionJob((prev) => ({
              sseUrl: imp.sse_url,
              title: `Importing ${created.domain} & Database...`,
              defaultShowLogs: true,
              externalSteps: [
                ...(prev?.externalSteps || []).filter((s) => s.step !== "upload"),
                {
                  step: "upload",
                  label: `Uploaded ${dbFilename} (${fileSizeLabel})`,
                  status: "done",
                  progress: 100,
                  message: uploadLog,
                },
              ],
            }));
          } else {
            activeImportRef.current = null;
            updateUploadStep(
              {
                step: "upload",
                label: `Uploaded ${dbFilename} (${fileSizeLabel})`,
                status: "done",
                progress: 100,
                message: uploadLog,
              },
              {
                step: "complete",
                label: `Connected ${created.domain}${imp.db_name ? ` (${imp.db_name})` : ""}`,
                status: "done",
                progress: 100,
                message: `${uploadLog}\n[${ts()}] Database imported cleanly.`,
              }
            );
            if (opts.folderPath) {
              setFailedFolderImports((prev) => {
                const copy = { ...prev };
                delete copy[opts.folderPath!];
                return copy;
              });
            }
            fetchSites();
            discoverFolders();
          }
        } catch (impErr) {
          const snapshot = activeImportRef.current;
          const errBody = impErr instanceof ApiError ? (impErr.body as any) : null;
          const errMsg = (errBody?.error as string) || (impErr instanceof Error ? impErr.message : "Database import failed");
          await rollbackFailedImport(errMsg);
          if (errBody?.requires_compat_confirm && snapshot) {
            setCompatPrompt({
              error: errMsg,
              fixes: Array.isArray(errBody.compat_fixes) ? errBody.compat_fixes : [],
              targetVersion: errBody.target_version,
              retry: () => retryImportWithCompat(snapshot),
            });
          }
          uploadLog += `\n[${ts()}] ERROR: ${errMsg}`;
          if (opts.isConnectExisting) {
            uploadLog += `\n[${ts()}] Unregistered temporary site registration (website files kept on disk) so you can retry from "Unregistered Folders Ready to Import".`;
          }
          updateUploadStep(
            {
              step: "upload",
              label: `Failed uploading/importing ${dbFilename}`,
              status: "error",
              message: uploadLog,
            },
            {
              step: "complete",
              label: `Database import failed for ${created.domain}`,
              status: "error",
              message: errMsg,
            }
          );
          setError(errMsg);
          const info = buildAlertDetails(impErr, {
            context: `Import ${dbFilename} for ${created.domain}`,
            fallback: errMsg,
            category: "database",
            target: created.domain,
          });
          showGlobalNotification(
            "error",
            errMsg,
            "Database Import Failed",
            uploadLog ? `${info.details ? `${info.details}\n\n` : ""}── Upload Log ──\n${uploadLog}` : info.details,
            info.meta
          );
        }
      })();
    } else {
      activeImportRef.current = null;
      setProvisionJob({
        siteId: created.id,
        title: opts.isConnectExisting
          ? `Connecting Existing Site (${created.domain})...`
          : `Provisioning ${created.domain}...`,
        defaultShowLogs: opts.isConnectExisting,
      });
      showGlobalNotification(
        "info",
        opts.isConnectExisting
          ? `Connecting existing site ${created.domain}...`
          : `Creating local site ${created.domain}...`,
        opts.isConnectExisting ? "Importing Site" : "Provisioning Site",
        opts.folderPath ? `Domain: ${created.domain}\nFolder: ${opts.folderPath}` : `Domain: ${created.domain}`,
        { category: "site", target: created.domain }
      );
    }
  };

  const handleImportDiscoveredFolder = async (req: FolderImportRequest) => {
    setError("");
    const effectiveRuntime = req.folder.inspection.is_wordpress ? "php" : req.runtime || "php";
    const body: Record<string, unknown> = {
      domain: req.domain,
      runtime: effectiveRuntime,
      document_root: req.folder.path,
      connect_existing: true,
      stream: true,
    };
    if (effectiveRuntime === "php") {
      body.php_version = phpVersion;
      if (req.folder.inspection.is_wordpress) body.php_preset = "wordpress";
    } else if (effectiveRuntime === "node" && req.appCommand) {
      body.app_command = req.appCommand;
    }

    try {
      const created = await api.post<Site>("/sites", body);
      setFailedFolderImports((prev) => {
        const copy = { ...prev };
        delete copy[req.folder.path];
        return copy;
      });
      fetchSites();
      discoverFolders();
      await runAfterCreateFlow(created, {
        isConnectExisting: true,
        isWordPress: req.folder.inspection.is_wordpress,
        runtime: effectiveRuntime,
        appCommand: req.appCommand,
        folderPath: req.folder.path,
        dbFile: req.dbFile,
        localDbFile: req.localDbFile,
        sourceUrl: req.sourceUrl,
        dbName: req.dbName,
        memoryLimitMb: req.memoryLimitMb,
      });
    } catch (err) {
      const info = buildAlertDetails(err, {
        context: `Import folder ${req.folder.name} as ${req.domain}`,
        fallback: "Failed to import folder",
        category: "site",
        target: req.domain,
      });
      setError(info.message);
      showGlobalNotification("error", info.message, "Folder Import Failed", info.details, info.meta);
    }
  };

  const handleOpenFolderInFullForm = (
    folder: DiscoveredFolder,
    customDomain: string,
    dbFile: File | null,
    memoryLimitMb?: number,
    localDb?: DetectedDbFile | null
  ) => {
    setShowForm(true);
    setCms("");
    setCustomRootPath(folder.path);
    setFolderInspection({
      ...folder.inspection,
      detected_db_files: folder.detected_db_files,
      other_directory_db_files: discoveredRootDbFiles,
    });
    const suffix = customDomain.endsWith(".local") ? ".local" : ".test";
    setDomainSuffix(suffix);
    setDomain(customDomain);
    setRuntime(folder.suggested_runtime || "php");
    setPhpPreset(folder.inspection.is_wordpress ? "wordpress" : "generic");
    if (folder.inspection.wp_siteurl || localDb?.detected_site_url) {
      setWpSourceUrl(folder.inspection.wp_siteurl || localDb?.detected_site_url || "");
    }
    if (memoryLimitMb !== undefined) setWpMemoryLimitMb(memoryLimitMb);
    setWpLocalDb(dbFile ? null : localDb || null);
    if (dbFile) {
      setWpDbFile(dbFile);
      setSkipWpDbUpload(false);
    } else if (localDb) {
      setWpDbFile(null);
      setSkipWpDbUpload(false);
    }
  };

  const retryImportWithCompat = async (snapshot: NonNullable<typeof activeImportRef.current>) => {
    setError("");
    const body: Record<string, unknown> = {
      domain: snapshot.domain,
      runtime: snapshot.isWordPress ? "php" : snapshot.runtime || "php",
      document_root: snapshot.folderPath,
      connect_existing: snapshot.isConnectExisting,
      stream: true,
    };
    if (body.runtime === "php") {
      body.php_version = phpVersion;
      if (snapshot.isWordPress) body.php_preset = "wordpress";
    } else if (body.runtime === "node" && snapshot.appCommand) {
      body.app_command = snapshot.appCommand;
    }
    try {
      const recreated = await api.post<Site>("/sites", body);
      fetchSites();
      discoverFolders();
      await runAfterCreateFlow(recreated, {
        isConnectExisting: snapshot.isConnectExisting,
        isWordPress: snapshot.isWordPress,
        runtime: snapshot.runtime,
        appCommand: snapshot.appCommand,
        folderPath: snapshot.folderPath,
        dbFile: snapshot.dbFile,
        localDbFile: snapshot.localDbObj,
        sourceUrl: snapshot.sourceUrl || undefined,
        dbName: snapshot.dbName || undefined,
        memoryLimitMb: snapshot.memoryLimitMb,
        applyCompatFixes: true,
      });
    } catch (err) {
      setError(err instanceof Error ? err.message : "Failed retrying import with compatibility fixes");
    }
  };

  const handleProvisionComplete = async (
    ok?: boolean,
    errorDetail?: string,
    compatMeta?: ProvisionCompatMeta
  ) => {
    if (ok === false) {
      const snapshot = activeImportRef.current;
      await rollbackFailedImport(errorDetail || "Site import failed");
      if (compatMeta?.requiresCompatConfirm && snapshot) {
        setCompatPrompt({
          error: compatMeta.errorDetail || errorDetail || "Database compatibility fixes required",
          fixes: compatMeta.compatFixes || [],
          targetVersion: compatMeta.targetVersion,
          retry: () => retryImportWithCompat(snapshot),
        });
      }
      return;
    }
    const ctx = activeImportRef.current;
    activeImportRef.current = null;
    if (ctx?.folderPath) {
      setFailedFolderImports((prev) => {
        const copy = { ...prev };
        delete copy[ctx.folderPath!];
        return copy;
      });
    }
    fetchSites();
    discoverFolders();
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

      {/* Provisioning & database import progress log */}
      {provisionJob && (
        <ProvisionLog
          siteId={provisionJob.siteId}
          sseUrl={provisionJob.sseUrl}
          title={provisionJob.title}
          defaultShowLogs={provisionJob.defaultShowLogs}
          externalSteps={provisionJob.externalSteps}
          onComplete={handleProvisionComplete}
          onDismiss={() => setProvisionJob(null)}
        />
      )}

      {/* Discovered folders in Default Websites Directory */}
      {isDesktopApp() && !isClient && (
        <DiscoveredFoldersPanel
          sitesDir={defaultSitesDir}
          folders={discoveredFolders}
          rootDbFiles={discoveredRootDbFiles}
          scanning={scanningFolders}
          failedImports={failedFolderImports}
          onDismissFailure={(fp) =>
            setFailedFolderImports((prev) => {
              const copy = { ...prev };
              delete copy[fp];
              return copy;
            })
          }
          onRescan={discoverFolders}
          onImport={handleImportDiscoveredFolder}
          onOpenFullForm={handleOpenFolderInFullForm}
        />
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
            <div className="space-y-3">
              <div>
                <label htmlFor="site-custom-root" className="block text-sm font-medium text-dark-100 mb-1">
                  Website Folder Path <span className="text-xs text-dark-400 font-normal">(empty or existing site directory)</span>
                </label>
                <div className="flex items-stretch gap-2">
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
                    className="flex-1 px-3 py-2.5 border border-dark-500 rounded-lg focus:ring-2 focus:ring-accent-500 outline-none text-sm font-mono"
                  />
                  <button
                    type="button"
                    onClick={handleBrowseFolder}
                    disabled={browsingFolder}
                    className="px-4 py-2.5 bg-dark-700 hover:bg-dark-600 text-dark-100 border border-dark-500 rounded-lg text-sm font-medium transition-colors whitespace-nowrap disabled:opacity-50"
                  >
                    {browsingFolder ? "Opening..." : "Browse..."}
                  </button>
                  {customRootPath && (
                    <button
                      type="button"
                      onClick={() => {
                        setCustomRootPath("");
                        setFolderInspection(null);
                        setWpDbFile(null);
                        setWpLocalDb(null);
                      }}
                      className="px-3 py-2.5 bg-dark-800 hover:bg-dark-700 text-dark-300 border border-dark-600 rounded-lg text-xs"
                      title="Reset to default folder"
                    >
                      Clear
                    </button>
                  )}
                </div>
                <p className="text-xs text-dark-400 mt-1">
                  Browse for an empty folder or an existing website folder to connect it locally. Leave empty to use the default directory.
                </p>

                {inspectingFolder && customRootPath.trim() && (
                  <div className="mt-2 px-3 py-2 rounded-lg border border-dark-600 bg-dark-800/80 text-xs text-dark-300 flex items-center gap-2 font-mono">
                    <span className="w-2 h-2 rounded-full bg-accent-400 animate-pulse" />
                    Scanning folder contents &amp; WordPress configuration...
                  </div>
                )}

                {!inspectingFolder && folderInspection && customRootPath.trim() && (
                  <div
                    className={`mt-2 px-3 py-2 rounded-lg border text-xs flex items-center justify-between gap-2 ${
                      folderInspection.is_wordpress
                        ? "bg-accent-950/30 border-accent-700/50 text-accent-200"
                        : folderInspection.is_empty
                        ? "bg-dark-800 border-dark-600 text-dark-200"
                        : folderInspection.detected_type === "invalid"
                        ? "bg-red-950/40 border-red-800/50 text-red-300"
                        : "bg-emerald-950/30 border-emerald-800/40 text-emerald-300"
                    }`}
                  >
                    <span>
                      {folderInspection.detected_type === "invalid"
                        ? "Selected path is a file, not a folder."
                        : !folderInspection.exists
                        ? "New folder — will be created empty."
                        : folderInspection.is_empty
                        ? "Empty directory — ready for a new website."
                        : folderInspection.is_wordpress
                        ? `Existing WordPress directory detected${folderInspection.wp_version ? ` (v${folderInspection.wp_version})` : ""} — files will be connected without overwriting.`
                        : `Existing ${folderInspection.detected_type.toUpperCase()} website detected (${folderInspection.file_count} items) — files will be connected without overwriting.`}
                    </span>
                    <div className="flex items-center gap-2 font-mono text-[11px] text-dark-300 shrink-0">
                      {folderInspection.wp_db_name && (
                        <span>DB: {folderInspection.wp_db_name}</span>
                      )}
                      {folderInspection.wp_table_prefix && (
                        <span>Prefix: {folderInspection.wp_table_prefix}</span>
                      )}
                    </div>
                  </div>
                )}
              </div>

              {folderInspection && !folderInspection.is_empty && folderInspection.exists && folderInspection.is_dir && (
                <div className="p-4 rounded-lg bg-dark-850 border border-accent-700/50 space-y-3">
                  <div>
                    <h4 className="text-sm font-medium text-dark-50">
                      {folderInspection.is_wordpress
                        ? "Upload WordPress Database Dump"
                        : "Upload Website Database Dump (Optional)"}
                    </h4>
                    <p className="text-xs text-dark-300 mt-0.5">
                      {folderInspection.is_wordpress ? (
                        <>
                          Upload your <code className="text-dark-100">.sql</code>,{" "}
                          <code className="text-dark-100">.sql.gz</code>, <code className="text-dark-100">.zip</code>, or{" "}
                          <code className="text-dark-100">.sqlite</code> export. DockPanel will rewrite URLs for{" "}
                          <span className="font-mono text-accent-300">{effectiveDomain || "this site"}</span>, import it into local MariaDB, and update{" "}
                          <code className="text-dark-100">wp-config.php</code> credentials automatically.
                        </>
                      ) : (
                        <>
                          Upload a <code className="text-dark-100">.sql</code>,{" "}
                          <code className="text-dark-100">.sql.gz</code>, <code className="text-dark-100">.zip</code>, or{" "}
                          <code className="text-dark-100">.sqlite</code> dump to provision a local database and wire{" "}
                          <code className="text-dark-100">.env</code> credentials automatically.
                        </>
                      )}
                    </p>
                  </div>

                  <div className="grid grid-cols-1 md:grid-cols-2 gap-3">
                    <div className="md:col-span-2">
                      <label htmlFor="wp-db-upload" className="block text-xs font-medium text-dark-200 mb-1">
                        Database File
                      </label>
                      <div className="flex items-center gap-2 flex-wrap">
                        <input
                          id="wp-db-upload"
                          type="file"
                          accept=".sql,.gz,.sql.gz,.zip,.sqlite,.db"
                          disabled={skipWpDbUpload}
                          onChange={(e) => {
                            const f = e.target.files?.[0] || null;
                            setWpDbFile(f);
                            if (f) setWpLocalDb(null);
                          }}
                          className="text-xs text-dark-200 file:mr-3 file:py-1.5 file:px-3 file:rounded-md file:border-0 file:text-xs file:font-medium file:bg-accent-600 file:text-white hover:file:bg-accent-500 disabled:opacity-40"
                        />
                        {wpDbFile && (
                          <span className="text-[11px] text-emerald-300 font-mono">
                            Selected: {wpDbFile.name} ({Math.max(1, Math.round(wpDbFile.size / 1024))} KB)
                          </span>
                        )}
                        {(wpDbFile || wpLocalDb) && !skipWpDbUpload && (
                          <button
                            type="button"
                            onClick={() => {
                              setWpDbFile(null);
                              setWpLocalDb(null);
                            }}
                            className="text-[11px] text-dark-400 hover:text-dark-100 underline"
                          >
                            Clear DB
                          </button>
                        )}
                      </div>
                      {!skipWpDbUpload && (
                        <DirectoryDbSelector
                          detectedDbFiles={folderInspection.detected_db_files || []}
                          otherDirectoryDbFiles={
                            folderInspection.other_directory_db_files || discoveredRootDbFiles
                          }
                          selectedLocalDb={wpLocalDb}
                          uploadedFile={wpDbFile}
                          siteDir={folderInspection.path}
                          sitesRootDir={defaultSitesDir}
                          onSelectLocalDb={(dbItem) => {
                            setWpLocalDb(dbItem);
                            if (dbItem) {
                              setWpDbFile(null);
                              if (!wpSourceUrl && dbItem.detected_site_url) {
                                setWpSourceUrl(dbItem.detected_site_url);
                              }
                            }
                          }}
                        />
                      )}
                    </div>

                    {folderInspection.is_wordpress && (
                      <div>
                        <label htmlFor="wp-source-url" className="block text-xs font-medium text-dark-200 mb-1">
                          Source Site URL <span className="text-dark-400 font-normal">(auto-detected if blank)</span>
                        </label>
                        <input
                          id="wp-source-url"
                          type="url"
                          disabled={skipWpDbUpload}
                          value={wpSourceUrl}
                          onChange={(e) => setWpSourceUrl(e.target.value)}
                          placeholder="https://old-website.com"
                          className="w-full px-3 py-1.5 bg-dark-900 border border-dark-600 rounded-lg text-xs text-dark-100 font-mono focus:outline-none focus:border-accent-500 disabled:opacity-40"
                        />
                      </div>
                    )}
                    <div>
                      <label htmlFor="wp-memory-limit" className="block text-xs font-medium text-dark-200 mb-1">
                        Import PHP Memory Limit <span className="text-dark-400 font-normal">(for large SQL dumps)</span>
                      </label>
                      <select
                        id="wp-memory-limit"
                        disabled={skipWpDbUpload}
                        value={wpMemoryLimitMb}
                        onChange={(e) => setWpMemoryLimitMb(parseInt(e.target.value, 10))}
                        className="w-full px-3 py-1.5 bg-dark-900 border border-dark-600 rounded-lg text-xs text-dark-100 font-mono focus:outline-none focus:border-accent-500 disabled:opacity-40"
                      >
                        {MEMORY_LIMIT_OPTIONS.map((opt) => (
                          <option key={opt.value} value={opt.value}>
                            {opt.label}
                          </option>
                        ))}
                      </select>
                    </div>
                  </div>

                  {folderInspection.is_wordpress && (
                    <label className="inline-flex items-center gap-2 text-xs text-dark-300 cursor-pointer">
                      <input
                        type="checkbox"
                        checked={skipWpDbUpload}
                        onChange={(e) => setSkipWpDbUpload(e.target.checked)}
                        className="rounded border-dark-600 bg-dark-800 text-accent-500 focus:ring-0"
                      />
                      Skip database upload for now (connect existing WordPress files only)
                    </label>
                  )}
                </div>
              )}
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
                              <Link
                                to={`/mail?domain=${site.domain}`}
                                className={`p-1.5 rounded hover:bg-dark-700 transition-colors ${m?.has_mail ? "text-rust-400" : "text-dark-300 hover:text-dark-100"}`}
                                title={isDesktopApp() ? "Local Mail Catcher" : (m?.has_mail ? "Mail configured" : "Mail")}
                              >
                                <svg className="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                                  <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M3 8l7.89 5.26a2 2 0 002.22 0L21 8M5 19h14a2 2 0 002-2V7a2 2 0 00-2-2H5a2 2 0 00-2 2v10a2 2 0 002 2z" />
                                </svg>
                              </Link>
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
                              <Link
                                to={`/mail?domain=${site.domain}`}
                                className={`p-1.5 rounded hover:bg-dark-700 transition-colors ${m?.has_mail ? "text-rust-400" : "text-dark-300 hover:text-dark-100"}`}
                                title={isDesktopApp() ? "Local Mail Catcher" : (m?.has_mail ? "Mail configured" : "Mail")}
                              >
                                <svg className="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                                  <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M3 8l7.89 5.26a2 2 0 002.22 0L21 8M5 19h14a2 2 0 002-2V7a2 2 0 00-2-2H5a2 2 0 00-2 2v10a2 2 0 002 2z" />
                                </svg>
                              </Link>
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
          showGlobalNotification(
            "success",
            `Local site router switched to port ${newPort}`,
            "Router Port Updated",
            `Active HTTP Router: http://127.0.0.1:${newPort}`,
            { category: "router", target: `port ${newPort}` }
          );
        }}
      />
      {compatPrompt && (
        <ConfirmDialog
          tone="warn"
          title="Database Compatibility Confirmation Required"
          label={`Error on target server${compatPrompt.targetVersion ? ` (${compatPrompt.targetVersion})` : ""}: ${compatPrompt.error}`}
          consequences={[
            ...compatPrompt.fixes,
            "Your original local .sql dump file is never modified — fixes only apply during import into local MariaDB.",
          ]}
          confirmLabel="Apply Compatibility Fixes & Retry"
          onConfirm={() => {
            const runRetry = compatPrompt.retry;
            setCompatPrompt(null);
            void runRetry();
          }}
          onCancel={() => setCompatPrompt(null)}
        />
      )}
      </div>
    </div>
  );
}
