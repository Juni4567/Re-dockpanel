import { useState } from "react";
import { api } from "../api";

export interface DetectedDbFile {
  name: string;
  relative_path: string;
  path: string;
  size_bytes: number;
  source_scope?: string;
  source_dir?: string;
  detected_prefix?: string | null;
  detected_site_url?: string | null;
  detected_db_name?: string | null;
  matches_site?: boolean | null;
  match_reason?: string | null;
}

export function formatDbBytesShort(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${Math.max(1, Math.round(bytes / 1024))} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
}

interface DirectoryDbSelectorProps {
  detectedDbFiles: DetectedDbFile[];
  otherDirectoryDbFiles?: DetectedDbFile[];
  selectedLocalDb: DetectedDbFile | null;
  uploadedFile: File | null;
  siteId?: string;
  siteDir?: string;
  sitesRootDir?: string;
  disabled?: boolean;
  onSelectLocalDb: (db: DetectedDbFile | null) => void;
}

export default function DirectoryDbSelector({
  detectedDbFiles,
  otherDirectoryDbFiles = [],
  selectedLocalDb,
  uploadedFile,
  siteId,
  siteDir,
  sitesRootDir,
  disabled = false,
  onSelectLocalDb,
}: DirectoryDbSelectorProps) {
  const [showDirectoryBrowser, setShowDirectoryBrowser] = useState(false);
  const [customDirInput, setCustomDirInput] = useState(sitesRootDir || siteDir || "");
  const [scannedDir, setScannedDir] = useState("");
  const [scannedFiles, setScannedFiles] = useState<DetectedDbFile[]>([]);
  const [scanning, setScanning] = useState(false);
  const [scanError, setScanError] = useState("");

  if (uploadedFile) {
    return null;
  }

  const folderPaths = new Set(detectedDbFiles.map((f) => f.path));
  const extraSitesDirFiles = otherDirectoryDbFiles.filter((f) => !folderPaths.has(f.path));

  // Include selectedLocalDb if it came from a previously scanned directory no longer in lists
  const allCustomPool = scannedFiles.filter((f) => !folderPaths.has(f.path));

  const runScan = async (opts: { pick?: boolean; dir?: string }) => {
    setScanError("");
    setScanning(true);
    setShowDirectoryBrowser(true);
    try {
      const res = await api.post<{
        cancelled?: boolean;
        dir?: string;
        files?: DetectedDbFile[];
        sites_dir_files?: DetectedDbFile[];
        error?: string;
      }>("/desktop/scan-db-files", {
        pick: !!opts.pick,
        dir: opts.pick ? undefined : opts.dir?.trim() || undefined,
        site_id: siteId || undefined,
        site_dir: siteDir || undefined,
      });
      if (res?.cancelled) {
        return;
      }
      if (res?.dir) {
        setScannedDir(res.dir);
        setCustomDirInput(res.dir);
      }
      setScannedFiles(Array.isArray(res?.files) ? res.files : []);
    } catch (err) {
      setScanError(err instanceof Error ? err.message : "Failed scanning directory for .sql files");
    } finally {
      setScanning(false);
    }
  };

  const renderDbButton = (dbItem: DetectedDbFile, showSourceDir = false) => {
    const active = selectedLocalDb?.path === dbItem.path;
    const isMatch = dbItem.matches_site === true;
    const isMismatch = dbItem.matches_site === false;

    const baseClasses = active
      ? isMismatch
        ? "bg-amber-500/20 border-amber-500/50 text-amber-200"
        : "bg-emerald-500/20 border-emerald-500/50 text-emerald-200"
      : isMismatch
      ? "bg-amber-500/5 border-amber-500/30 text-amber-300/90 hover:border-amber-400"
      : isMatch
      ? "bg-emerald-500/10 border-emerald-500/35 text-emerald-200 hover:border-emerald-400"
      : "bg-dark-900 border-dark-600 text-dark-200 hover:border-dark-400";

    const metaParts: string[] = [];
    if (dbItem.detected_prefix) metaParts.push(`prefix:${dbItem.detected_prefix}`);
    if (dbItem.detected_site_url) metaParts.push(dbItem.detected_site_url);
    if (dbItem.detected_db_name && !dbItem.detected_site_url) metaParts.push(`db:${dbItem.detected_db_name}`);

    return (
      <button
        key={dbItem.path}
        type="button"
        disabled={disabled}
        onClick={() => onSelectLocalDb(active ? null : dbItem)}
        title={[
          dbItem.path,
          dbItem.match_reason || "",
          metaParts.join(" | "),
        ]
          .filter(Boolean)
          .join("\n")}
        className={`text-left px-2.5 py-1 rounded text-[11px] font-mono border transition-colors flex items-center gap-1.5 flex-wrap disabled:opacity-50 ${baseClasses}`}
      >
        <span>{active ? "✓" : "•"}</span>
        <span className="font-semibold">{dbItem.relative_path}</span>
        <span className="text-dark-400">({formatDbBytesShort(dbItem.size_bytes)})</span>
        {showSourceDir && dbItem.source_dir && (
          <span className="text-[10px] px-1 py-0.2 rounded bg-dark-950/60 text-dark-400 truncate max-w-[160px]">
            {dbItem.source_dir.split(/[\\/]/).filter(Boolean).pop()}
          </span>
        )}
        {isMatch && (
          <span className="px-1.5 py-0.2 rounded text-[10px] font-sans font-semibold bg-emerald-500/20 text-emerald-300 border border-emerald-500/30">
            Suggested Match
          </span>
        )}
        {isMismatch && (
          <span className="px-1.5 py-0.2 rounded text-[10px] font-sans font-semibold bg-amber-500/20 text-amber-300 border border-amber-500/30">
            Different Site?
          </span>
        )}
        {metaParts.length > 0 && (
          <span className="text-[10px] text-dark-400 font-normal">
            [{metaParts.join(" • ")}]
          </span>
        )}
      </button>
    );
  };

  return (
    <div className="mt-2 space-y-2">
      {/* Subdirectory database suggestions for this website folder */}
      {detectedDbFiles.length > 0 && (
        <div className="space-y-1.5">
          <div className="flex items-center justify-between gap-2 flex-wrap">
            <span className="text-[11px] text-dark-300">
              Found in folder &amp; subdirectories ({detectedDbFiles.length}):
            </span>
            <button
              type="button"
              disabled={disabled}
              onClick={() => setShowDirectoryBrowser((v) => !v)}
              className="text-[11px] text-accent-400 hover:text-accent-300 underline disabled:opacity-50"
            >
              {showDirectoryBrowser
                ? "Hide directory picker"
                : selectedLocalDb
                ? "Wrong database? Select from directories..."
                : "Or choose from another directory..."}
            </button>
          </div>
          <div className="flex items-center gap-1.5 flex-wrap">
            {detectedDbFiles.map((dbItem) => renderDbButton(dbItem, false))}
          </div>
        </div>
      )}

      {detectedDbFiles.length === 0 && (
        <div className="flex items-center gap-2 flex-wrap">
          <span className="text-[11px] text-dark-400">
            No .sql files inside this folder&apos;s subdirectories.
          </span>
          <button
            type="button"
            disabled={disabled}
            onClick={() => setShowDirectoryBrowser((v) => !v)}
            className="text-[11px] text-accent-400 hover:text-accent-300 underline disabled:opacity-50"
          >
            {showDirectoryBrowser
              ? "Hide directory picker"
              : `Select database from ${extraSitesDirFiles.length > 0 ? `${extraSitesDirFiles.length} found / ` : ""}other directories...`}
          </button>
        </div>
      )}

      {/* Mismatch Alert Banner if selected DB appears to belong to a different site */}
      {selectedLocalDb && selectedLocalDb.matches_site === false && (
        <div className="px-2.5 py-2 rounded-md bg-amber-500/10 border border-amber-500/35 text-[11px] text-amber-200 flex items-center justify-between gap-2 flex-wrap">
          <div>
            <span className="font-semibold">Heads up:</span>{" "}
            <span>
              {selectedLocalDb.match_reason ||
                "This SQL dump may belong to a different website."}
            </span>
          </div>
          <div className="flex items-center gap-2 shrink-0">
            <button
              type="button"
              onClick={() => {
                onSelectLocalDb(null);
                setShowDirectoryBrowser(true);
              }}
              className="px-2 py-0.5 rounded bg-amber-500/20 hover:bg-amber-500/30 border border-amber-500/40 text-amber-200 font-medium"
            >
              Choose Another Database
            </button>
          </div>
        </div>
      )}

      {/* Selected external / directory dump readout if not already visible in above list */}
      {selectedLocalDb && !folderPaths.has(selectedLocalDb.path) && (
        <div className="px-2.5 py-1.5 rounded bg-emerald-500/10 border border-emerald-500/35 text-[11px] font-mono text-emerald-200 flex items-center justify-between gap-2 flex-wrap">
          <span className="truncate" title={selectedLocalDb.path}>
            Selected from directory: {selectedLocalDb.relative_path} ({formatDbBytesShort(selectedLocalDb.size_bytes)})
          </span>
          <button
            type="button"
            onClick={() => onSelectLocalDb(null)}
            className="text-dark-300 hover:text-dark-100 underline font-sans shrink-0"
          >
            Clear
          </button>
        </div>
      )}

      {/* Expandable Directory Browser & Custom Subdirectory Scanner */}
      {showDirectoryBrowser && (
        <div className="p-2.5 rounded-lg bg-dark-900/90 border border-dark-600 space-y-2.5">
          <div className="flex items-center justify-between gap-2 flex-wrap">
            <span className="text-[11px] font-medium text-dark-200">
              Select Database (.sql / .gz / .zip / .sqlite) from Directories
            </span>
            <div className="flex items-center gap-1.5 flex-wrap">
              {sitesRootDir && (
                <button
                  type="button"
                  disabled={disabled || scanning}
                  onClick={() => runScan({ dir: sitesRootDir })}
                  className="px-2 py-1 rounded bg-dark-800 hover:bg-dark-700 border border-dark-600 text-[11px] text-dark-200 transition-colors disabled:opacity-50"
                >
                  Scan Websites Root
                </button>
              )}
              <button
                type="button"
                disabled={disabled || scanning}
                onClick={() => runScan({ pick: true })}
                className="px-2.5 py-1 rounded bg-accent-600/20 hover:bg-accent-600/30 border border-accent-500/40 text-[11px] text-accent-200 font-medium transition-colors disabled:opacity-50"
              >
                {scanning ? "Scanning..." : "Browse Folder..."}
              </button>
            </div>
          </div>

          <div className="flex items-center gap-1.5">
            <input
              type="text"
              value={customDirInput}
              disabled={disabled || scanning}
              onChange={(e) => setCustomDirInput(e.target.value)}
              placeholder="Paste directory path to scan subdirectories for .sql..."
              aria-label="Directory path to scan for SQL files"
              className="flex-1 min-w-0 px-2.5 py-1 bg-dark-950 border border-dark-600 rounded text-[11px] font-mono text-dark-100 focus:outline-none focus:border-accent-500"
            />
            <button
              type="button"
              disabled={disabled || scanning || !customDirInput.trim()}
              onClick={() => runScan({ dir: customDirInput })}
              className="px-2.5 py-1 rounded bg-dark-700 hover:bg-dark-600 border border-dark-500 text-[11px] text-dark-100 transition-colors disabled:opacity-50"
            >
              {scanning ? "Scanning..." : "Scan Path"}
            </button>
          </div>

          {scanError && (
            <p className="text-[11px] text-danger-400 font-mono">{scanError}</p>
          )}

          {scannedDir && (
            <div className="space-y-1">
              <p className="text-[11px] text-dark-300 font-mono truncate" title={scannedDir}>
                Scanned {scannedDir} ({allCustomPool.length} found in subdirectories):
              </p>
              {allCustomPool.length === 0 ? (
                <p className="text-[11px] text-dark-400">
                  No .sql, .sql.gz, .sqlite, or backup .zip files found in that directory tree.
                </p>
              ) : (
                <div className="flex items-center gap-1.5 flex-wrap max-h-40 overflow-y-auto py-0.5">
                  {allCustomPool.map((dbItem) => renderDbButton(dbItem, true))}
                </div>
              )}
            </div>
          )}

          {extraSitesDirFiles.length > 0 && (
            <div className="space-y-1 pt-1 border-t border-dark-700/80">
              <p className="text-[11px] text-dark-300">
                Other databases in Websites Directory ({extraSitesDirFiles.length}):
              </p>
              <div className="flex items-center gap-1.5 flex-wrap max-h-40 overflow-y-auto py-0.5">
                {extraSitesDirFiles.map((dbItem) => renderDbButton(dbItem, true))}
              </div>
            </div>
          )}
        </div>
      )}
    </div>
  );
}
