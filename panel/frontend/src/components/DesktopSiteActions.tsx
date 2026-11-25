import React, { useState } from "react";
import { api } from "../api";
import { showGlobalNotification } from "../context/NotificationContext";
import {
  isDesktopApp,
  openDesktopFolder,
  openDesktopVSCode,
} from "../utils/desktop";

export interface DesktopSiteActionsProps {
  domain: string;
  siteId?: string;
  status?: string;
  className?: string;
  onOpenExplorer?: () => void;
  onOpenVSCode?: () => void;
  onStatusChange?: (newStatus: string) => void;
  onError?: (message: string) => void;
}

/**
 * Lucide-style Play icon
 */
export function PlayIcon({ className = "w-4 h-4" }: { className?: string }) {
  return (
    <svg
      className={className}
      fill="none"
      viewBox="0 0 24 24"
      stroke="currentColor"
      strokeWidth={2}
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
    >
      <polygon points="6 3 20 12 6 21 6 3" />
    </svg>
  );
}

/**
 * Lucide-style Square (Stop) icon
 */
export function SquareIcon({ className = "w-4 h-4" }: { className?: string }) {
  return (
    <svg
      className={className}
      fill="none"
      viewBox="0 0 24 24"
      stroke="currentColor"
      strokeWidth={2}
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
    >
      <rect width="14" height="14" x="5" y="5" rx="2" />
    </svg>
  );
}

/**
 * Lucide-style FolderOpen icon
 */
export function FolderOpenIcon({ className = "w-4 h-4" }: { className?: string }) {
  return (
    <svg
      className={className}
      fill="none"
      viewBox="0 0 24 24"
      stroke="currentColor"
      strokeWidth={2}
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
    >
      <path d="m6 14 1.45-2.9A2 2 0 0 1 9.24 10H20a2 2 0 0 1 1.94 2.5l-1.55 6a2 2 0 0 1-1.94 1.5H4a2 2 0 0 1-2-2V5c0-1.1.9-2 2-2h3.93a2 2 0 0 1 1.66.9l.82 1.2a2 2 0 0 0 1.66.9H18a2 2 0 0 1 2 2v2" />
    </svg>
  );
}

/**
 * Lucide-style Code icon
 */
export function CodeIcon({ className = "w-4 h-4" }: { className?: string }) {
  return (
    <svg
      className={className}
      fill="none"
      viewBox="0 0 24 24"
      stroke="currentColor"
      strokeWidth={2}
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
    >
      <polyline points="16 18 22 12 16 6" />
      <polyline points="8 6 2 12 8 18" />
    </svg>
  );
}

/**
 * Calls the desktop site lifecycle endpoint to start or stop a site and returns
 * the resulting status string.
 */
export async function toggleDesktopSiteStatus(
  siteId: string,
  currentStatus?: string
): Promise<string> {
  const isRunning = currentStatus === "active";
  const endpoint = isRunning ? `/sites/${siteId}/stop` : `/sites/${siteId}/start`;
  const fallbackStatus = isRunning ? "stopped" : "active";
  const resp = await api.post<{ status?: string }>(endpoint, {});
  return resp?.status || fallbackStatus;
}

/**
 * Stateful Start/Stop toggle button kept in its own component so parent
 * DesktopSiteActions remains hook-free when returning null outside desktop mode.
 */
function SiteLifecycleToggle({
  siteId,
  status,
  onStatusChange,
  onError,
}: {
  siteId: string;
  status?: string;
  onStatusChange?: (newStatus: string) => void;
  onError?: (message: string) => void;
}) {
  const [loading, setLoading] = useState(false);
  const isRunning = status === "active";

  const handleToggleStatus = async (e: React.MouseEvent) => {
    e.preventDefault();
    e.stopPropagation();
    if (loading) return;

    setLoading(true);
    try {
      const nextStatus = await toggleDesktopSiteStatus(siteId, status);
      showGlobalNotification(
        "success",
        nextStatus === "active" ? "Site PHP worker started" : "Site PHP worker stopped",
      );
      onStatusChange?.(nextStatus);
    } catch (err) {
      const message =
        err instanceof Error
          ? err.message
          : `Failed to ${isRunning ? "stop" : "start"} site`;
      showGlobalNotification("error", message);
      onError?.(message);
    } finally {
      setLoading(false);
    }
  };

  return (
    <button
      type="button"
      onClick={handleToggleStatus}
      disabled={loading}
      className={`p-1.5 rounded hover:bg-dark-700 transition-colors disabled:opacity-50 ${
        isRunning
          ? "text-red-400 hover:text-red-300"
          : "text-emerald-400 hover:text-emerald-300"
      }`}
      title={isRunning ? "Stop Site" : "Start Site"}
      aria-label={isRunning ? "Stop Site" : "Start Site"}
    >
      {loading ? (
        <svg
          className="w-4 h-4 animate-spin"
          fill="none"
          viewBox="0 0 24 24"
          aria-hidden="true"
        >
          <circle
            className="opacity-25"
            cx="12"
            cy="12"
            r="10"
            stroke="currentColor"
            strokeWidth="4"
          />
          <path
            className="opacity-75"
            fill="currentColor"
            d="M4 12a8 8 0 018-8V0C5.373 0 0 5.373 0 12h4z"
          />
        </svg>
      ) : isRunning ? (
        <SquareIcon className="w-4 h-4" />
      ) : (
        <PlayIcon className="w-4 h-4" />
      )}
    </button>
  );
}

/**
 * DesktopSiteActions
 *
 * Renders native desktop quick actions ("Start/Stop Site", "Open in Explorer",
 * and "Open in VS Code") for a given site. Only renders when running in desktop
 * mode (Tauri/Vite desktop).
 */
export default function DesktopSiteActions({
  domain,
  siteId,
  status,
  className = "",
  onOpenExplorer,
  onOpenVSCode,
  onStatusChange,
  onError,
}: DesktopSiteActionsProps) {
  if (!isDesktopApp()) {
    return null;
  }

  const handleOpenExplorer = async (e: React.MouseEvent) => {
    e.preventDefault();
    e.stopPropagation();
    const ok = await openDesktopFolder(domain);
    if (ok) {
      showGlobalNotification("success", `Opened folder for ${domain}`);
      onOpenExplorer?.();
    } else {
      const msg = `Failed to open folder for ${domain}`;
      showGlobalNotification("error", msg);
      onError?.(msg);
    }
  };

  const handleOpenVSCode = async (e: React.MouseEvent) => {
    e.preventDefault();
    e.stopPropagation();
    const ok = await openDesktopVSCode(domain);
    if (ok) {
      showGlobalNotification("success", `Opening ${domain} in VS Code`);
      onOpenVSCode?.();
    } else {
      const msg = `Could not launch VS Code for ${domain}. Ensure 'code' CLI is in PATH.`;
      showGlobalNotification("error", msg);
      onError?.(msg);
    }
  };

  return (
    <div className={`inline-flex items-center gap-1 ${className}`}>
      {siteId && (
        <SiteLifecycleToggle
          siteId={siteId}
          status={status}
          onStatusChange={onStatusChange}
          onError={onError}
        />
      )}
      <button
        type="button"
        onClick={handleOpenExplorer}
        className="p-1.5 rounded hover:bg-dark-700 text-dark-300 hover:text-dark-100 transition-colors"
        title="Open in Explorer"
        aria-label="Open in Explorer"
      >
        <FolderOpenIcon className="w-4 h-4" />
      </button>
      <button
        type="button"
        onClick={handleOpenVSCode}
        className="p-1.5 rounded hover:bg-dark-700 text-dark-300 hover:text-dark-100 transition-colors"
        title="Open in VS Code"
        aria-label="Open in VS Code"
      >
        <CodeIcon className="w-4 h-4" />
      </button>
    </div>
  );
}

export { openDesktopFolder, openDesktopVSCode };
