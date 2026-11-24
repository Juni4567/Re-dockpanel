import React from "react";
import {
  isDesktopApp,
  openDesktopFolder,
  openDesktopVSCode,
} from "../utils/desktop";

export interface DesktopSiteActionsProps {
  domain: string;
  className?: string;
  onOpenExplorer?: () => void;
  onOpenVSCode?: () => void;
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
 * DesktopSiteActions
 *
 * Renders native desktop quick actions ("Open in Explorer" and "Open in VS Code")
 * for a given site domain. Only renders when running in desktop mode (Tauri/Vite desktop).
 */
export default function DesktopSiteActions({
  domain,
  className = "",
  onOpenExplorer,
  onOpenVSCode,
}: DesktopSiteActionsProps) {
  if (!isDesktopApp()) {
    return null;
  }

  const handleOpenExplorer = async (e: React.MouseEvent) => {
    e.preventDefault();
    e.stopPropagation();
    await openDesktopFolder(domain);
    onOpenExplorer?.();
  };

  const handleOpenVSCode = async (e: React.MouseEvent) => {
    e.preventDefault();
    e.stopPropagation();
    await openDesktopVSCode(domain);
    onOpenVSCode?.();
  };

  return (
    <div className={`inline-flex items-center gap-1 ${className}`}>
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
