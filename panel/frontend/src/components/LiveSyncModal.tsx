import { useState } from "react";
import { api } from "../api";

export interface LiveSyncModalProps {
  site: { id: string; domain: string };
  isOpen: boolean;
  initialMode?: "push" | "pull";
  onClose: () => void;
}

interface PreflightStatus {
  checked: boolean;
  success: boolean;
  message: string;
  remoteDomain?: string;
  remoteSiteId?: string;
}

interface ProgressStep {
  step: string;
  label: string;
  status: string; // "pending" | "in_progress" | "done" | "error"
  message?: string;
}

export default function LiveSyncModal({
  site,
  isOpen,
  initialMode = "push",
  onClose,
}: LiveSyncModalProps) {
  if (!isOpen) return null;

  const [mode, setMode] = useState<"push" | "pull">(initialMode);
  const [remoteUrl, setRemoteUrl] = useState(() => {
    return typeof localStorage !== "undefined"
      ? localStorage.getItem("dockpanel-sync-url") || ""
      : "";
  });
  const [apiToken, setApiToken] = useState(() => {
    return typeof localStorage !== "undefined"
      ? localStorage.getItem("dockpanel-sync-token") || ""
      : "";
  });
  const [remoteSiteId, setRemoteSiteId] = useState<string | null>(null);

  const [syncDb, setSyncDb] = useState(true);
  const [syncFiles, setSyncFiles] = useState(true);
  const [dryRun, setDryRun] = useState(false);

  const [testingConnection, setTestingConnection] = useState(false);
  const [preflightStatus, setPreflightStatus] = useState<PreflightStatus | null>(null);

  const [syncing, setSyncing] = useState(false);
  const [progress, setProgress] = useState<ProgressStep[]>([]);
  const [syncResult, setSyncResult] = useState<{ success: boolean; message: string } | null>(null);
  const [error, setError] = useState<string | null>(null);

  const handleTestConnection = async () => {
    if (!remoteUrl.trim()) {
      setError("Please enter a remote panel URL");
      return;
    }
    if (!apiToken.trim()) {
      setError("Please enter an API token");
      return;
    }

    setError(null);
    setTestingConnection(true);

    try {
      const res = await api.post<{
        success: boolean;
        message: string;
        remote_site_found?: boolean;
        remote_domain?: string;
        remote_site_id?: string;
      }>(`/sites/${site.id}/sync/preflight`, {
        remote_url: remoteUrl.trim(),
        api_token: apiToken.trim(),
        remote_site_id: remoteSiteId || undefined,
      });

      if (res.remote_site_id) {
        setRemoteSiteId(res.remote_site_id);
      }

      setPreflightStatus({
        checked: true,
        success: res.success,
        message: res.message,
        remoteDomain: res.remote_domain,
        remoteSiteId: res.remote_site_id,
      });

      if (typeof localStorage !== "undefined") {
        localStorage.setItem("dockpanel-sync-url", remoteUrl.trim());
        localStorage.setItem("dockpanel-sync-token", apiToken.trim());
      }
    } catch (err: any) {
      setPreflightStatus({
        checked: true,
        success: false,
        message: err.message || "Failed to connect to remote server",
      });
    } finally {
      setTestingConnection(false);
    }
  };

  const handleStartSync = async () => {
    if (!remoteUrl.trim() || !apiToken.trim()) {
      setError("Remote Panel URL and API Token are required.");
      return;
    }
    if (!syncDb && !syncFiles) {
      setError("Please select at least one component to synchronize (Database or Files).");
      return;
    }

    setError(null);
    setSyncing(true);
    setProgress([
      {
        step: "init",
        label: mode === "push" ? "Starting push to live server..." : "Starting pull from live server...",
        status: "in_progress",
      },
    ]);
    setSyncResult(null);

    const endpoint = mode === "push" ? `/sites/${site.id}/sync/push` : `/sites/${site.id}/sync/pull`;

    try {
      const res = await api.post<{
        sync_id: string;
        message: string;
        status: string;
        progress?: ProgressStep[];
      }>(endpoint, {
        remote_url: remoteUrl.trim(),
        api_token: apiToken.trim(),
        sync_db: syncDb,
        sync_files: syncFiles,
        dry_run: dryRun,
        remote_site_id: remoteSiteId || undefined,
      });

      if (res.progress && res.progress.length > 0) {
        setProgress(res.progress);
      } else {
        setProgress([
          {
            step: "complete",
            label: res.message || "Sync operation completed successfully",
            status: "done",
          },
        ]);
      }

      setSyncResult({
        success: true,
        message: res.message || (dryRun ? "Dry run simulation finished" : "Sync completed successfully!"),
      });

      if (typeof localStorage !== "undefined") {
        localStorage.setItem("dockpanel-sync-url", remoteUrl.trim());
        localStorage.setItem("dockpanel-sync-token", apiToken.trim());
      }
    } catch (err: any) {
      setError(err.message || "Synchronization request failed");
      setSyncResult({
        success: false,
        message: err.message || "Sync operation failed",
      });
      setProgress((prev) => [
        ...prev,
        {
          step: "failed",
          label: "Operation failed",
          status: "error",
          message: err.message,
        },
      ]);
    } finally {
      setSyncing(false);
    }
  };

  return (
    <div className="fixed inset-0 bg-black/70 backdrop-blur-sm z-50 flex items-center justify-center p-4">
      <div className="bg-dark-900 border border-dark-700 rounded-xl max-w-xl w-full p-6 space-y-5 max-h-[90vh] overflow-y-auto shadow-2xl">
        {/* Header */}
        <div className="flex items-center justify-between border-b border-dark-700/60 pb-4">
          <div>
            <h2 className="text-lg font-semibold text-dark-100 flex items-center gap-2">
              <svg className="w-5 h-5 text-accent-400" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M4 4v5h.582m15.356 2A8.001 8.001 0 004.582 9m0 0H9m11 11v-5h-.581m0 0a8.003 8.003 0 01-15.357-2m15.357 2H15" />
              </svg>
              Live Server Synchronization
            </h2>
            <p className="text-xs text-dark-400 mt-0.5">
              Target site: <span className="font-mono text-dark-200">{site.domain}</span>
            </p>
          </div>
          <button
            onClick={onClose}
            className="text-dark-400 hover:text-dark-100 p-1.5 rounded-lg hover:bg-dark-800 transition-colors"
          >
            <svg className="w-5 h-5" fill="none" viewBox="0 0 24 24" stroke="currentColor">
              <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M6 18L18 6M6 6l12 12" />
            </svg>
          </button>
        </div>

        {/* Push / Pull Switcher */}
        <div className="grid grid-cols-2 gap-2 bg-dark-800 p-1 rounded-lg border border-dark-700">
          <button
            type="button"
            onClick={() => setMode("push")}
            className={`flex items-center justify-center gap-2 py-2 px-3 rounded-md text-xs font-medium transition-all ${
              mode === "push"
                ? "bg-accent-600 text-white shadow-sm"
                : "text-dark-300 hover:text-dark-100 hover:bg-dark-700/50"
            }`}
          >
            <svg className="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor">
              <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M4 16v1a3 3 0 003 3h10a3 3 0 003-3v-1m-4-8l-4-4m0 0L8 8m4-4v12" />
            </svg>
            Push to Live
          </button>
          <button
            type="button"
            onClick={() => setMode("pull")}
            className={`flex items-center justify-center gap-2 py-2 px-3 rounded-md text-xs font-medium transition-all ${
              mode === "pull"
                ? "bg-accent-600 text-white shadow-sm"
                : "text-dark-300 hover:text-dark-100 hover:bg-dark-700/50"
            }`}
          >
            <svg className="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor">
              <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M4 16v1a3 3 0 003 3h10a3 3 0 003-3v-1m-4-4l-4 4m0 0l-4-4m4 4V4" />
            </svg>
            Pull from Live
          </button>
        </div>

        {/* Safety Indicator Badge */}
        <div className="flex items-center gap-2 p-3 bg-emerald-950/30 border border-emerald-800/40 rounded-lg text-xs text-emerald-300">
          <svg className="w-4 h-4 flex-shrink-0 text-emerald-400" fill="none" viewBox="0 0 24 24" stroke="currentColor">
            <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M9 12l2 2 4-4m5.618-4.016A11.955 11.955 0 0112 2.944a11.955 11.955 0 01-8.618 3.04A12.02 12.02 0 003 9c0 5.591 3.824 10.29 9 11.622 5.176-1.332 9-6.03 9-11.622 0-1.042-.133-2.052-.382-3.016z" />
          </svg>
          <span>
            <strong>Automatic remote safety snapshot before overwrite:</strong> A snapshot backup is taken on the remote server prior to any deployment.
          </span>
        </div>

        {/* Remote Server Form */}
        <div className="space-y-3">
          <div>
            <label className="block text-xs font-medium text-dark-300 mb-1">Remote Panel URL</label>
            <input
              type="url"
              value={remoteUrl}
              onChange={(e) => setRemoteUrl(e.target.value)}
              placeholder="https://panel.example.com"
              className="w-full px-3 py-2 bg-dark-800 border border-dark-600 rounded-lg text-sm text-dark-100 placeholder-dark-500 focus:outline-none focus:border-accent-500"
            />
          </div>

          <div>
            <label className="block text-xs font-medium text-dark-300 mb-1">API Token</label>
            <input
              type="password"
              value={apiToken}
              onChange={(e) => setApiToken(e.target.value)}
              placeholder="dp_pat_..."
              className="w-full px-3 py-2 bg-dark-800 border border-dark-600 rounded-lg text-sm text-dark-100 placeholder-dark-500 focus:outline-none focus:border-accent-500 font-mono"
            />
          </div>

          <div className="flex items-center justify-between pt-1">
            <button
              type="button"
              onClick={handleTestConnection}
              disabled={testingConnection}
              className="inline-flex items-center gap-1.5 px-3 py-1.5 bg-dark-800 hover:bg-dark-700 text-dark-200 text-xs font-medium rounded border border-dark-600 transition-colors disabled:opacity-50"
            >
              {testingConnection ? (
                <>
                  <svg className="animate-spin w-3.5 h-3.5 text-accent-400" fill="none" viewBox="0 0 24 24">
                    <circle className="opacity-25" cx="12" cy="12" r="10" stroke="currentColor" strokeWidth="4" />
                    <path className="opacity-75" fill="currentColor" d="M4 12a8 8 0 018-8V0C5.373 0 0 5.373 0 12h4z" />
                  </svg>
                  Testing...
                </>
              ) : (
                "Test Connection"
              )}
            </button>

            {preflightStatus && (
              <span
                className={`text-xs inline-flex items-center gap-1 font-medium ${
                  preflightStatus.success ? "text-emerald-400" : "text-red-400"
                }`}
              >
                {preflightStatus.success ? "✓" : "✕"}{" "}
                {preflightStatus.remoteDomain
                  ? `Connected (Remote: ${preflightStatus.remoteDomain}${
                      preflightStatus.remoteSiteId ? ` / #${preflightStatus.remoteSiteId.slice(0, 8)}` : ""
                    })`
                  : preflightStatus.message}
              </span>
            )}
          </div>
        </div>

        {/* Options */}
        <div className="space-y-2 pt-2 border-t border-dark-700/60">
          <p className="text-xs font-medium text-dark-300">Synchronization Options</p>
          <div className="grid grid-cols-1 sm:grid-cols-3 gap-3">
            <label className="flex items-center gap-2 text-xs text-dark-200 cursor-pointer">
              <input
                type="checkbox"
                checked={syncDb}
                onChange={(e) => setSyncDb(e.target.checked)}
                className="rounded border-dark-600 bg-dark-800 text-accent-500 focus:ring-0"
              />
              Sync Database
            </label>

            <label className="flex items-center gap-2 text-xs text-dark-200 cursor-pointer">
              <input
                type="checkbox"
                checked={syncFiles}
                onChange={(e) => setSyncFiles(e.target.checked)}
                className="rounded border-dark-600 bg-dark-800 text-accent-500 focus:ring-0"
              />
              Sync Files
            </label>

            <label className="flex items-center gap-2 text-xs text-dark-200 cursor-pointer">
              <input
                type="checkbox"
                checked={dryRun}
                onChange={(e) => setDryRun(e.target.checked)}
                className="rounded border-dark-600 bg-dark-800 text-accent-500 focus:ring-0"
              />
              Dry Run
            </label>
          </div>
        </div>

        {/* Error message */}
        {error && (
          <div className="p-3 bg-red-950/40 border border-red-800/50 rounded-lg text-xs text-red-300">
            {error}
          </div>
        )}

        {/* Progress & Log */}
        {progress.length > 0 && (
          <div className="space-y-2 pt-2 border-t border-dark-700/60">
            <p className="text-xs font-medium text-dark-300">Execution Progress</p>
            <div className="bg-dark-950 border border-dark-800 rounded-lg p-3 space-y-1.5 font-mono text-xs max-h-40 overflow-y-auto">
              {progress.map((step, idx) => (
                <div key={idx} className="flex items-start gap-2">
                  <span
                    className={
                      step.status === "done"
                        ? "text-emerald-400"
                        : step.status === "in_progress"
                        ? "text-accent-400 animate-pulse"
                        : step.status === "error"
                        ? "text-red-400"
                        : "text-dark-500"
                    }
                  >
                    {step.status === "done" ? "✓" : step.status === "error" ? "✕" : "•"}
                  </span>
                  <div>
                    <span className="text-dark-200">{step.label}</span>
                    {step.message && (
                      <p className="text-[11px] text-dark-400">{step.message}</p>
                    )}
                  </div>
                </div>
              ))}
            </div>
          </div>
        )}

        {/* Success / Result alert */}
        {syncResult && (
          <div
            className={`p-3 rounded-lg text-xs font-medium ${
              syncResult.success
                ? "bg-emerald-950/40 border border-emerald-800/40 text-emerald-300"
                : "bg-red-950/40 border border-red-800/40 text-red-300"
            }`}
          >
            {syncResult.message}
          </div>
        )}

        {/* Footer Actions */}
        <div className="flex items-center justify-end gap-3 pt-3 border-t border-dark-700/60">
          <button
            type="button"
            onClick={onClose}
            disabled={syncing}
            className="px-4 py-2 text-xs font-medium text-dark-300 hover:text-dark-100 transition-colors disabled:opacity-50"
          >
            Close
          </button>
          <button
            type="button"
            onClick={handleStartSync}
            disabled={syncing}
            className="px-4 py-2 bg-accent-600 hover:bg-accent-500 text-white text-xs font-medium rounded-lg shadow-sm transition-colors disabled:opacity-50 inline-flex items-center gap-1.5"
          >
            {syncing && (
              <svg className="animate-spin w-3.5 h-3.5 text-white" fill="none" viewBox="0 0 24 24">
                <circle className="opacity-25" cx="12" cy="12" r="10" stroke="currentColor" strokeWidth="4" />
                <path className="opacity-75" fill="currentColor" d="M4 12a8 8 0 018-8V0C5.373 0 0 5.373 0 12h4z" />
              </svg>
            )}
            {dryRun
              ? "Run Dry Run"
              : mode === "push"
              ? "Start Push to Live"
              : "Start Pull from Live"}
          </button>
        </div>
      </div>
    </div>
  );
}
