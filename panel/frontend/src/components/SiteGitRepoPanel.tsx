import { useState, useEffect, useCallback } from "react";
import { api } from "../api";
import { isDesktopApp } from "../utils/desktop";
import { showGlobalNotification } from "../context/NotificationContext";

export interface GitCommitSummary {
  hash: string;
  author: string;
  date: string;
  message: string;
}

export interface SiteGitStatus {
  git_available: boolean;
  git_version?: string | null;
  initialized: boolean;
  doc_root: string;
  branch?: string | null;
  branches: string[];
  remote_name?: string | null;
  remote_url?: string | null;
  clean: boolean;
  changed_files: number;
  recent_commits: GitCommitSummary[];
  has_gitignore: boolean;
  error?: string | null;
}

interface SiteGitRepoPanelProps {
  siteId: string;
  siteDomain?: string;
  runtime?: string;
  phpPreset?: string | null;
  isAdmin?: boolean;
  onRepoChanged?: () => void;
}

export default function SiteGitRepoPanel({
  siteId,
  siteDomain,
  runtime,
  phpPreset,
  isAdmin = true,
  onRepoChanged,
}: SiteGitRepoPanelProps) {
  const desktop = isDesktopApp();
  const [status, setStatus] = useState<SiteGitStatus | null>(null);
  const [loading, setLoading] = useState(true);
  const [busy, setBusy] = useState<string | null>(null);
  const [error, setError] = useState("");
  const [opOutput, setOpOutput] = useState("");

  const defaultPreset =
    phpPreset === "wordpress"
      ? "wordpress"
      : runtime === "node"
      ? "node"
      : runtime === "php"
      ? "php"
      : "static";

  // Init form state
  const [initBranch, setInitBranch] = useState("main");
  const [gitignorePreset, setGitignorePreset] = useState(defaultPreset);
  const [initialCommit, setInitialCommit] = useState(true);
  const [initCommitMsg, setInitCommitMsg] = useState("Initial site commit");

  // Connect existing remote form state
  const [repoUrl, setRepoUrl] = useState("");
  const [remoteBranch, setRemoteBranch] = useState("main");
  const [token, setToken] = useState("");
  const [connectMode, setConnectMode] = useState<"link" | "link_and_pull" | "clone">("link");

  // Commit & Push state
  const [commitMessage, setCommitMessage] = useState("");

  const loadStatus = useCallback(async () => {
    if (!siteId) return;
    setLoading(true);
    try {
      const st = await api.get<SiteGitStatus>(`/sites/${siteId}/git/status`);
      setStatus(st);
      if (st.branch) {
        setInitBranch(st.branch);
        setRemoteBranch(st.branch);
      }
      if (st.remote_url && !repoUrl) {
        setRepoUrl(st.remote_url);
      }
    } catch (err) {
      setError(err instanceof Error ? err.message : "Failed checking Git status");
    } finally {
      setLoading(false);
    }
  }, [siteId]);

  useEffect(() => {
    if (desktop || isAdmin) {
      loadStatus();
    } else {
      setLoading(false);
    }
  }, [desktop, isAdmin, loadStatus]);

  if (!desktop && !isAdmin) {
    return (
      <div className="bg-dark-800 rounded-lg border border-dark-600 p-5 text-xs text-dark-300">
        Repository initialization and remote connection are restricted to Admin accounts.
      </div>
    );
  }

  const handleInit = async () => {
    setError("");
    setOpOutput("");
    setBusy("init");
    try {
      const res = await api.post<{ ok: boolean; status: SiteGitStatus; output: string }>(
        `/sites/${siteId}/git/init`,
        {
          branch: initBranch.trim() || "main",
          gitignore_preset: gitignorePreset === "none" ? undefined : gitignorePreset,
          initial_commit: initialCommit,
          commit_message: initCommitMsg.trim() || "Initial commit",
        }
      );
      if (res?.status) setStatus(res.status);
      if (res?.output) setOpOutput(res.output);
      showGlobalNotification("success", `Initialized Git repository for ${siteDomain || "site"}`);
      onRepoChanged?.();
    } catch (err) {
      const msg = err instanceof Error ? err.message : "Git init failed";
      setError(msg);
      showGlobalNotification("error", msg, "Git Init Failed");
    } finally {
      setBusy(null);
    }
  };

  const handleConnect = async () => {
    if (!repoUrl.trim()) {
      setError("Please enter an existing Git repository URL (HTTPS or SSH).");
      return;
    }
    setError("");
    setOpOutput("");
    setBusy("connect");
    try {
      const res = await api.post<{ ok: boolean; status: SiteGitStatus; output: string }>(
        `/sites/${siteId}/git/connect`,
        {
          repo_url: repoUrl.trim(),
          branch: remoteBranch.trim() || "main",
          token: token.trim() || undefined,
          mode: connectMode,
          gitignore_preset: gitignorePreset === "none" ? undefined : gitignorePreset,
        }
      );
      if (res?.status) setStatus(res.status);
      if (res?.output) setOpOutput(res.output);
      setToken("");
      showGlobalNotification("success", `Connected repository to ${siteDomain || "site"}`);
      onRepoChanged?.();
    } catch (err) {
      const msg = err instanceof Error ? err.message : "Failed connecting repository";
      setError(msg);
      showGlobalNotification("error", msg, "Git Connect Failed");
    } finally {
      setBusy(null);
    }
  };

  const handlePull = async () => {
    setError("");
    setOpOutput("");
    setBusy("pull");
    try {
      const res = await api.post<{ ok: boolean; status: SiteGitStatus; output: string }>(
        `/sites/${siteId}/git/pull`,
        {
          branch: status?.branch || remoteBranch.trim() || "main",
          token: token.trim() || undefined,
        }
      );
      if (res?.status) setStatus(res.status);
      if (res?.output) setOpOutput(res.output);
      showGlobalNotification("success", "Pulled latest changes from remote repository");
    } catch (err) {
      const msg = err instanceof Error ? err.message : "Git pull failed";
      setError(msg);
      showGlobalNotification("error", msg, "Git Pull Failed");
    } finally {
      setBusy(null);
    }
  };

  const handleCommitPush = async (push: boolean) => {
    if (!commitMessage.trim()) {
      setError("Enter a commit message first.");
      return;
    }
    setError("");
    setOpOutput("");
    setBusy(push ? "push" : "commit");
    try {
      const res = await api.post<{ ok: boolean; status: SiteGitStatus; output: string }>(
        `/sites/${siteId}/git/commit-push`,
        {
          message: commitMessage.trim(),
          branch: status?.branch || remoteBranch.trim() || "main",
          token: token.trim() || undefined,
          push,
        }
      );
      if (res?.status) setStatus(res.status);
      if (res?.output) setOpOutput(res.output);
      setCommitMessage("");
      showGlobalNotification("success", push ? "Committed & pushed to origin" : "Created Git commit");
    } catch (err) {
      const msg = err instanceof Error ? err.message : "Commit/push failed";
      setError(msg);
      showGlobalNotification("error", msg, "Git Commit/Push Failed");
    } finally {
      setBusy(null);
    }
  };

  const handleDisconnect = async () => {
    setError("");
    setOpOutput("");
    setBusy("disconnect");
    try {
      const res = await api.post<{ ok: boolean; status: SiteGitStatus }>(
        `/sites/${siteId}/git/disconnect`,
        {}
      );
      if (res?.status) setStatus(res.status);
      setRepoUrl("");
      setOpOutput("Removed remote 'origin'. Local .git repository and files remain intact.");
      showGlobalNotification("info", "Disconnected remote repository");
      onRepoChanged?.();
    } catch (err) {
      setError(err instanceof Error ? err.message : "Failed removing remote");
    } finally {
      setBusy(null);
    }
  };

  return (
    <div className="bg-dark-800 rounded-lg border border-dark-500 overflow-hidden">
      <div className="px-5 py-4 border-b border-dark-600 flex items-center justify-between gap-3 flex-wrap">
        <div>
          <div className="flex items-center gap-2 flex-wrap">
            <h2 className="text-xs font-medium text-dark-300 uppercase font-mono tracking-widest">
              Site Git Repository
            </h2>
            {!loading && status && (
              <span
                className={`px-2 py-0.5 rounded-full text-[11px] font-medium border ${
                  !status.git_available
                    ? "bg-danger-500/15 text-danger-300 border-danger-500/30"
                    : status.initialized
                    ? "bg-emerald-500/15 text-emerald-300 border-emerald-500/30"
                    : "bg-dark-700 text-dark-300 border-dark-600"
                }`}
              >
                {!status.git_available
                  ? "Git Not Found"
                  : status.initialized
                  ? `Initialized (${status.branch || "HEAD"})`
                  : "Not Initialized"}
              </span>
            )}
          </div>
          <p className="text-xs text-dark-300 mt-1">
            Initialize version control inside the site folder or connect an existing Git repository from Admin.
          </p>
        </div>

        <button
          type="button"
          onClick={loadStatus}
          disabled={loading || !!busy}
          className="px-2.5 py-1 rounded border border-dark-600 bg-dark-900 hover:bg-dark-700 text-xs text-dark-200 disabled:opacity-50"
        >
          {loading ? "Checking..." : "Refresh"}
        </button>
      </div>

      <div className="p-5 space-y-5">
        {error && (
          <div className="px-3 py-2.5 rounded-lg bg-danger-500/10 border border-danger-500/30 text-xs text-danger-300 font-mono">
            {error}
          </div>
        )}

        {status?.error && (
          <div className="px-3 py-2.5 rounded-lg bg-warn-500/10 border border-warn-500/30 text-xs text-warn-300">
            {status.error}
          </div>
        )}

        {status?.doc_root && (
          <div className="grid grid-cols-1 sm:grid-cols-3 gap-3 text-xs">
            <div className="p-3 rounded-lg bg-dark-900 border border-dark-700">
              <span className="text-dark-400 block mb-0.5">Site Path</span>
              <span className="font-mono text-dark-100 truncate block" title={status.doc_root}>
                {status.doc_root}
              </span>
            </div>
            <div className="p-3 rounded-lg bg-dark-900 border border-dark-700">
              <span className="text-dark-400 block mb-0.5">Connected Remote (origin)</span>
              <span className="font-mono text-dark-100 truncate block" title={status.remote_url || ""}>
                {status.remote_url || "None configured"}
              </span>
            </div>
            <div className="p-3 rounded-lg bg-dark-900 border border-dark-700">
              <span className="text-dark-400 block mb-0.5">Working Tree</span>
              <span className="font-mono text-dark-100 block">
                {!status.initialized
                  ? "No .git repository"
                  : status.clean
                  ? "Clean (0 uncommitted changes)"
                  : `${status.changed_files} uncommitted change(s)`}
              </span>
            </div>
          </div>
        )}

        {/* Section 1: Initialize Git Repo */}
        {status?.git_available && !status.initialized && (
          <div className="p-4 rounded-lg bg-dark-900/80 border border-dark-600 space-y-3">
            <div>
              <h3 className="text-sm font-semibold text-dark-100">
                1. Initialize New Git Repository
              </h3>
              <p className="text-xs text-dark-400 mt-0.5">
                Runs <code className="text-dark-200">git init</code> in this website&apos;s folder without touching live files.
              </p>
            </div>
            <div className="grid grid-cols-1 sm:grid-cols-3 gap-3">
              <div>
                <label className="block text-[11px] font-medium text-dark-300 mb-1">
                  Default Branch
                </label>
                <input
                  type="text"
                  value={initBranch}
                  onChange={(e) => setInitBranch(e.target.value)}
                  className="w-full px-2.5 py-1.5 bg-dark-950 border border-dark-600 rounded text-xs font-mono text-dark-100"
                />
              </div>
              <div>
                <label className="block text-[11px] font-medium text-dark-300 mb-1">
                  .gitignore Preset
                </label>
                <select
                  value={gitignorePreset}
                  onChange={(e) => setGitignorePreset(e.target.value)}
                  className="w-full px-2.5 py-1.5 bg-dark-950 border border-dark-600 rounded text-xs text-dark-100"
                >
                  <option value="wordpress">WordPress (ignores uploads, cache, .sql, wp-config.php)</option>
                  <option value="php">PHP / Laravel (ignores vendor, .env, logs)</option>
                  <option value="node">Node.js (ignores node_modules, dist, .env)</option>
                  <option value="static">Static HTML (ignores .env, logs)</option>
                  <option value="none">None</option>
                </select>
              </div>
              <div>
                <label className="block text-[11px] font-medium text-dark-300 mb-1">
                  Initial Commit Message
                </label>
                <input
                  type="text"
                  value={initCommitMsg}
                  disabled={!initialCommit}
                  onChange={(e) => setInitCommitMsg(e.target.value)}
                  className="w-full px-2.5 py-1.5 bg-dark-950 border border-dark-600 rounded text-xs text-dark-100 disabled:opacity-40"
                />
              </div>
            </div>
            <div className="flex items-center justify-between gap-3 flex-wrap">
              <label className="inline-flex items-center gap-2 text-xs text-dark-200 cursor-pointer">
                <input
                  type="checkbox"
                  checked={initialCommit}
                  onChange={(e) => setInitialCommit(e.target.checked)}
                  className="rounded border-dark-600 bg-dark-950 text-rust-500"
                />
                Stage files and create initial commit
              </label>
              <button
                type="button"
                disabled={!!busy}
                onClick={handleInit}
                className="px-4 py-1.5 rounded-lg bg-rust-500 hover:bg-rust-600 text-white text-xs font-medium disabled:opacity-50"
              >
                {busy === "init" ? "Initializing..." : "Initialize Git Repository"}
              </button>
            </div>
          </div>
        )}

        {/* Section 2: Connect Existing Git Repository */}
        {status?.git_available && (
          <div className="p-4 rounded-lg bg-dark-900/80 border border-dark-600 space-y-3">
            <div className="flex items-center justify-between gap-2 flex-wrap">
              <div>
                <h3 className="text-sm font-semibold text-dark-100">
                  {status.remote_url
                    ? "Connected Remote Repository"
                    : status.initialized
                    ? "Connect Existing Remote Repository"
                    : "2. Or Connect Existing Git Repository"}
                </h3>
                <p className="text-xs text-dark-400 mt-0.5">
                  Link this site folder to an existing GitHub, GitLab, Bitbucket, or private SSH/HTTPS repository.
                </p>
              </div>
              {status.remote_url && (
                <button
                  type="button"
                  disabled={!!busy}
                  onClick={handleDisconnect}
                  className="px-2.5 py-1 rounded border border-danger-500/30 bg-danger-500/10 text-danger-300 hover:bg-danger-500/20 text-xs disabled:opacity-50"
                >
                  {busy === "disconnect" ? "Disconnecting..." : "Disconnect Remote"}
                </button>
              )}
            </div>

            <div className="grid grid-cols-1 md:grid-cols-12 gap-3">
              <div className="md:col-span-6">
                <label className="block text-[11px] font-medium text-dark-300 mb-1">
                  Repository URL
                </label>
                <input
                  type="text"
                  value={repoUrl}
                  onChange={(e) => setRepoUrl(e.target.value)}
                  placeholder="https://github.com/your-org/website.git or git@github.com:..."
                  className="w-full px-2.5 py-1.5 bg-dark-950 border border-dark-600 rounded text-xs font-mono text-dark-100"
                />
              </div>
              <div className="md:col-span-3">
                <label className="block text-[11px] font-medium text-dark-300 mb-1">
                  Branch
                </label>
                <input
                  type="text"
                  value={remoteBranch}
                  onChange={(e) => setRemoteBranch(e.target.value)}
                  placeholder="main"
                  className="w-full px-2.5 py-1.5 bg-dark-950 border border-dark-600 rounded text-xs font-mono text-dark-100"
                />
              </div>
              <div className="md:col-span-3">
                <label className="block text-[11px] font-medium text-dark-300 mb-1">
                  Mode
                </label>
                <select
                  value={connectMode}
                  onChange={(e) => setConnectMode(e.target.value as "link" | "link_and_pull" | "clone")}
                  className="w-full px-2.5 py-1.5 bg-dark-950 border border-dark-600 rounded text-xs text-dark-100"
                >
                  <option value="link">Link &amp; Fetch (Keep existing files)</option>
                  <option value="link_and_pull">Link &amp; Merge Pull</option>
                  {!status.initialized && (
                    <option value="clone">Clone Fresh (Empty folders only)</option>
                  )}
                </select>
              </div>
            </div>

            <div className="flex items-center gap-3 flex-wrap justify-between">
              <input
                type="password"
                value={token}
                onChange={(e) => setToken(e.target.value)}
                placeholder="Optional Personal Access Token (for private HTTPS repos)"
                className="flex-1 min-w-[240px] px-2.5 py-1.5 bg-dark-950 border border-dark-600 rounded text-xs font-mono text-dark-100"
              />
              <button
                type="button"
                disabled={!!busy || !repoUrl.trim()}
                onClick={handleConnect}
                className="px-4 py-1.5 rounded-lg bg-accent-600 hover:bg-accent-700 text-white text-xs font-medium disabled:opacity-50"
              >
                {busy === "connect" ? "Connecting..." : "Connect Repository"}
              </button>
            </div>
          </div>
        )}

        {/* Section 3: Commit, Pull & Push when Initialized */}
        {status?.git_available && status.initialized && (
          <div className="p-4 rounded-lg bg-dark-900/60 border border-dark-700 space-y-3">
            <div className="flex items-center justify-between gap-2 flex-wrap">
              <h3 className="text-xs font-semibold uppercase tracking-wider text-dark-300 font-mono">
                Sync &amp; Commit Changes
              </h3>
              {status.remote_url && (
                <button
                  type="button"
                  disabled={!!busy}
                  onClick={handlePull}
                  className="px-3 py-1 rounded bg-dark-700 hover:bg-dark-600 text-xs text-dark-100 disabled:opacity-50"
                >
                  {busy === "pull" ? "Pulling..." : "Pull Latest"}
                </button>
              )}
            </div>
            <div className="flex items-center gap-2 flex-wrap">
              <input
                type="text"
                value={commitMessage}
                onChange={(e) => setCommitMessage(e.target.value)}
                placeholder="Commit message (stages all modified/new files)..."
                className="flex-1 min-w-[220px] px-2.5 py-1.5 bg-dark-950 border border-dark-600 rounded text-xs text-dark-100"
              />
              <button
                type="button"
                disabled={!!busy || !commitMessage.trim()}
                onClick={() => handleCommitPush(false)}
                className="px-3 py-1.5 rounded bg-dark-700 hover:bg-dark-600 text-xs text-dark-100 disabled:opacity-50"
              >
                {busy === "commit" ? "Committing..." : "Commit"}
              </button>
              {status.remote_url && (
                <button
                  type="button"
                  disabled={!!busy || !commitMessage.trim()}
                  onClick={() => handleCommitPush(true)}
                  className="px-3.5 py-1.5 rounded bg-rust-500 hover:bg-rust-600 text-xs text-white font-medium disabled:opacity-50"
                >
                  {busy === "push" ? "Pushing..." : "Commit & Push"}
                </button>
              )}
            </div>

            {status.recent_commits.length > 0 && (
              <div className="pt-2 border-t border-dark-800 space-y-1">
                <span className="text-[11px] font-medium text-dark-400 block">
                  Recent Commits
                </span>
                {status.recent_commits.map((c) => (
                  <div key={c.hash} className="text-xs font-mono flex items-center gap-2 text-dark-200">
                    <span className="px-1.5 py-0.2 rounded bg-dark-800 text-accent-300 border border-dark-700">
                      {c.hash}
                    </span>
                    <span className="truncate flex-1">{c.message}</span>
                    <span className="text-[11px] text-dark-400 shrink-0">{c.date}</span>
                  </div>
                ))}
              </div>
            )}
          </div>
        )}

        {opOutput && (
          <pre className="p-3 rounded-lg bg-dark-950 border border-dark-700 text-[11px] font-mono text-dark-200 whitespace-pre-wrap max-h-40 overflow-y-auto">
            {opOutput}
          </pre>
        )}
      </div>
    </div>
  );
}
