import { useState, useEffect } from "react";
import { formatDesktopDomain } from "../utils/desktop";
import DirectoryDbSelector, {
  type DetectedDbFile,
  formatDbBytesShort,
} from "./DirectoryDbSelector";

export type { DetectedDbFile };

export interface DiscoveredFolderInspection {
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
}

export interface DiscoveredFolder {
  name: string;
  path: string;
  suggested_domain: string;
  suggested_runtime: string;
  detected_db_files: DetectedDbFile[];
  inspection: DiscoveredFolderInspection;
}

export interface FolderImportRequest {
  folder: DiscoveredFolder;
  domain: string;
  runtime: string;
  appCommand?: string;
  dbFile: File | null;
  localDbFile: DetectedDbFile | null;
  sourceUrl?: string;
  dbName?: string;
  memoryLimitMb: number;
}

export interface FailedFolderImportState {
  domain: string;
  runtime: string;
  appCommand: string;
  dbFile: File | null;
  localDbPath: string;
  localDb?: DetectedDbFile | null;
  sourceUrl: string;
  dbName: string;
  memoryLimitMb: number;
  error: string;
  memorySuggestion?: string;
}

export const MEMORY_LIMIT_OPTIONS = [
  { value: 512, label: "512 MB" },
  { value: 1024, label: "1024 MB (Default)" },
  { value: 2048, label: "2048 MB (Large DB)" },
  { value: 4096, label: "4096 MB (Huge DB)" },
  { value: -1, label: "Unlimited (-1)" },
];

export function formatMemoryLimitLabel(mb: number): string {
  if (mb <= 0) return "Unlimited (-1)";
  return `${mb} MB`;
}

interface DiscoveredFoldersPanelProps {
  sitesDir: string;
  folders: DiscoveredFolder[];
  rootDbFiles?: DetectedDbFile[];
  scanning: boolean;
  failedImports?: Record<string, FailedFolderImportState>;
  onDismissFailure?: (folderPath: string) => void;
  onRescan: () => void;
  onImport: (req: FolderImportRequest) => Promise<void>;
  onOpenFullForm: (
    folder: DiscoveredFolder,
    domain: string,
    dbFile: File | null,
    memoryLimitMb?: number,
    localDb?: DetectedDbFile | null
  ) => void;
}

interface FolderDraftState {
  domain: string;
  runtime: string;
  appCommand: string;
  dbFile: File | null;
  localDbPath: string;
  localDb: DetectedDbFile | null;
  sourceUrl: string;
  dbName: string;
  memoryLimitMb: number;
}

function pickSuggestedDbForFolder(folder: DiscoveredFolder): DetectedDbFile | null {
  if (!folder.detected_db_files || folder.detected_db_files.length === 0) return null;
  const exact = folder.detected_db_files.find((d) => d.matches_site === true);
  if (exact) return exact;
  return folder.detected_db_files.find((d) => d.matches_site !== false) || null;
}

function detectBadgeClass(detectedType: string): string {
  switch (detectedType) {
    case "wordpress":
      return "bg-accent-500/15 text-accent-300 border-accent-500/30";
    case "php":
      return "bg-indigo-500/15 text-indigo-300 border-indigo-500/30";
    case "node":
      return "bg-emerald-500/15 text-emerald-300 border-emerald-500/30";
    case "static":
      return "bg-sky-500/15 text-sky-300 border-sky-500/30";
    default:
      return "bg-dark-700 text-dark-300 border-dark-600";
  }
}

function detectBadgeLabel(insp: DiscoveredFolderInspection): string {
  if (insp.is_wordpress) {
    return insp.wp_version ? `WordPress ${insp.wp_version}` : "WordPress";
  }
  switch (insp.detected_type) {
    case "php":
      return "PHP";
    case "node":
      return "Node.js";
    case "static":
      return "Static HTML";
    case "empty":
      return "Empty Folder";
    default:
      return insp.detected_type.toUpperCase();
  }
}

export default function DiscoveredFoldersPanel({
  sitesDir,
  folders,
  rootDbFiles = [],
  scanning,
  failedImports,
  onDismissFailure,
  onRescan,
  onImport,
  onOpenFullForm,
}: DiscoveredFoldersPanelProps) {
  const [collapsed, setCollapsed] = useState(false);
  const [showEmpty, setShowEmpty] = useState(false);
  const [importingPath, setImportingPath] = useState<string | null>(null);
  const [drafts, setDrafts] = useState<Record<string, FolderDraftState>>({});

  useEffect(() => {
    setDrafts((prev) => {
      const next: Record<string, FolderDraftState> = {};
      for (const f of folders) {
        const restored = failedImports?.[f.path];
        if (restored) {
          const restoredDb =
            restored.localDb ||
            f.detected_db_files.find((d) => d.path === restored.localDbPath) ||
            rootDbFiles.find((d) => d.path === restored.localDbPath) ||
            null;
          next[f.path] = {
            domain: restored.domain,
            runtime: restored.runtime,
            appCommand: restored.appCommand,
            dbFile: restored.dbFile,
            localDbPath: restored.localDbPath,
            localDb: restoredDb,
            sourceUrl: restored.sourceUrl,
            dbName: restored.dbName,
            memoryLimitMb: restored.memoryLimitMb,
          };
        } else if (prev[f.path]) {
          next[f.path] = prev[f.path];
        } else {
          const suggested = pickSuggestedDbForFolder(f);
          next[f.path] = {
            domain: f.suggested_domain,
            runtime: f.suggested_runtime || "php",
            appCommand: f.suggested_runtime === "node" ? "npm start" : "",
            dbFile: null,
            localDbPath: suggested?.path || "",
            localDb: suggested,
            sourceUrl: f.inspection.wp_siteurl || suggested?.detected_site_url || "",
            dbName: suggested?.detected_db_name || "",
            memoryLimitMb: 1024,
          };
        }
      }
      return next;
    });
  }, [folders, failedImports, rootDbFiles]);

  const nonEmptyFolders = folders.filter((f) => !f.inspection.is_empty);
  const emptyFolders = folders.filter((f) => f.inspection.is_empty);
  const visibleFolders = showEmpty || nonEmptyFolders.length === 0 ? folders : nonEmptyFolders;

  if (folders.length === 0 && !scanning) {
    return null;
  }

  const updateDraft = (path: string, patch: Partial<FolderDraftState>, fallback: DiscoveredFolder) => {
    setDrafts((prev) => {
      const suggested = pickSuggestedDbForFolder(fallback);
      const base: FolderDraftState = prev[path] || {
        domain: fallback.suggested_domain,
        runtime: fallback.suggested_runtime || "php",
        appCommand: fallback.suggested_runtime === "node" ? "npm start" : "",
        dbFile: null,
        localDbPath: suggested?.path || "",
        localDb: suggested,
        sourceUrl: fallback.inspection.wp_siteurl || suggested?.detected_site_url || "",
        dbName: suggested?.detected_db_name || "",
        memoryLimitMb: 1024,
      };
      return {
        ...prev,
        [path]: {
          ...base,
          ...patch,
        },
      };
    });
  };

  return (
    <section
      aria-label="Discovered folders in Default Websites Directory"
      className="mb-6 rounded-xl border border-accent-500/30 bg-dark-800/90 overflow-hidden shadow-lg"
    >
      <div className="px-4 py-3 bg-dark-850/90 border-b border-dark-700 flex items-center justify-between gap-3 flex-wrap">
        <div className="flex items-center gap-2.5 min-w-0">
          <span className="inline-flex items-center justify-center w-7 h-7 rounded-lg bg-accent-500/15 border border-accent-500/30 text-accent-300 shrink-0">
            <svg className="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={1.8}>
              <path strokeLinecap="round" strokeLinejoin="round" d="M3.75 9.776c.112-.017.227-.026.344-.026h15.812c.117 0 .232.009.344.026m-16.5 0a2.25 2.25 0 0 0-1.883 2.542l.857 6a2.25 2.25 0 0 0 2.227 1.932H19.05a2.25 2.25 0 0 0 2.227-1.932l.857-6a2.25 2.25 0 0 0-1.883-2.542m-16.5 0V6A2.25 2.25 0 0 1 6 3.75h3.879a1.5 1.5 0 0 1 1.06.44l2.122 2.12a1.5 1.5 0 0 0 1.06.44H18A2.25 2.25 0 0 1 20.25 9v.776" />
            </svg>
          </span>
          <div className="min-w-0">
            <div className="flex items-center gap-2 flex-wrap">
              <h2 className="text-sm font-semibold text-dark-50">
                Unregistered Folders Ready to Import
              </h2>
              <span className="px-2 py-0.5 rounded-full text-[11px] font-mono bg-accent-500/20 text-accent-300 border border-accent-500/30">
                {nonEmptyFolders.length > 0 ? `${nonEmptyFolders.length} detected` : `${folders.length} empty`}
              </span>
            </div>
            {sitesDir && (
              <p className="text-[11px] font-mono text-dark-400 truncate mt-0.5" title={sitesDir}>
                Scanning: {sitesDir}
              </p>
            )}
          </div>
        </div>

        <div className="flex items-center gap-2 flex-wrap">
          {emptyFolders.length > 0 && nonEmptyFolders.length > 0 && (
            <button
              type="button"
              onClick={() => setShowEmpty((v) => !v)}
              className="px-2.5 py-1 rounded-md border border-dark-600 bg-dark-800 hover:bg-dark-700 text-xs text-dark-300 transition-colors"
            >
              {showEmpty ? "Hide Empty" : `Show ${emptyFolders.length} Empty`}
            </button>
          )}
          <button
            type="button"
            onClick={onRescan}
            disabled={scanning}
            className="inline-flex items-center gap-1.5 px-2.5 py-1 rounded-md border border-dark-600 bg-dark-800 hover:bg-dark-700 text-xs text-dark-200 transition-colors disabled:opacity-50"
          >
            <svg className={`w-3.5 h-3.5 ${scanning ? "animate-spin" : ""}`} fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}>
              <path strokeLinecap="round" strokeLinejoin="round" d="M16.023 9.348h4.992v-.001M2.985 19.644v-4.992m0 0h4.992m-4.993 0 3.181 3.183a8.25 8.25 0 0 0 13.803-3.7M4.031 9.865a8.25 8.25 0 0 1 13.803-3.7l3.181 3.182" />
            </svg>
            {scanning ? "Scanning..." : "Rescan"}
          </button>
          <button
            type="button"
            onClick={() => setCollapsed((v) => !v)}
            className="px-2.5 py-1 rounded-md border border-dark-600 bg-dark-800 hover:bg-dark-700 text-xs text-dark-300 transition-colors"
          >
            {collapsed ? "Show" : "Hide"}
          </button>
        </div>
      </div>

      {!collapsed && (
        <div className="divide-y divide-dark-700/70">
          {visibleFolders.map((folder) => {
            const failure = failedImports?.[folder.path];
            const suggestedDefault = pickSuggestedDbForFolder(folder);
            const draft = drafts[folder.path] || {
              domain: failure?.domain || folder.suggested_domain,
              runtime: failure?.runtime || folder.suggested_runtime || "php",
              appCommand: failure?.appCommand || (folder.suggested_runtime === "node" ? "npm start" : ""),
              dbFile: failure?.dbFile || null,
              localDbPath: failure?.localDbPath || suggestedDefault?.path || "",
              localDb:
                failure?.localDb ||
                (failure ? null : suggestedDefault),
              sourceUrl: failure?.sourceUrl || folder.inspection.wp_siteurl || suggestedDefault?.detected_site_url || "",
              dbName: failure?.dbName || suggestedDefault?.detected_db_name || "",
              memoryLimitMb: failure?.memoryLimitMb ?? 1024,
            };
            const selectedLocalDb =
              draft.localDb ||
              folder.detected_db_files.find((d) => d.path === draft.localDbPath) ||
              rootDbFiles.find((d) => d.path === draft.localDbPath) ||
              (draft.localDbPath
                ? {
                    name: draft.localDbPath.split(/[\\/]/).pop() || draft.localDbPath,
                    relative_path: draft.localDbPath,
                    path: draft.localDbPath,
                    size_bytes: 0,
                  }
                : null);
            const isImporting = importingPath === folder.path;
            const normalizedDomain = formatDesktopDomain(
              draft.domain.trim(),
              draft.domain.trim().endsWith(".local") ? ".local" : ".test"
            );

            const triggerImport = async (overrideMemMb?: number) => {
              setImportingPath(folder.path);
              try {
                await onImport({
                  folder,
                  domain: normalizedDomain,
                  runtime: draft.runtime,
                  appCommand: draft.runtime === "node" ? draft.appCommand || "npm start" : undefined,
                  dbFile: draft.dbFile,
                  localDbFile: selectedLocalDb,
                  sourceUrl: draft.sourceUrl.trim() || undefined,
                  dbName: draft.dbName.trim() || undefined,
                  memoryLimitMb: overrideMemMb ?? draft.memoryLimitMb,
                });
              } finally {
                setImportingPath(null);
              }
            };

            return (
              <div key={folder.path} className="p-4 space-y-3 bg-dark-800/50">
                <div className="flex items-start justify-between gap-3 flex-wrap">
                  <div className="min-w-0">
                    <div className="flex items-center gap-2 flex-wrap">
                      <span className="font-mono text-sm font-semibold text-dark-50">
                        {folder.name}
                      </span>
                      <span
                        className={`text-[10px] font-semibold uppercase tracking-wide px-2 py-0.5 rounded-full border ${detectBadgeClass(
                          folder.inspection.detected_type
                        )}`}
                      >
                        {detectBadgeLabel(folder.inspection)}
                      </span>
                      <span className="text-xs text-dark-400">
                        {folder.inspection.file_count} item{folder.inspection.file_count === 1 ? "" : "s"}
                      </span>
                      {folder.inspection.wp_db_name && (
                        <span className="text-[11px] font-mono text-dark-300 bg-dark-900 px-2 py-0.5 rounded border border-dark-700">
                          {`DB: ${folder.inspection.wp_db_name}`}
                        </span>
                      )}
                    </div>
                    <p className="text-[11px] font-mono text-dark-400 truncate mt-0.5" title={folder.path}>
                      {folder.path}
                    </p>
                  </div>

                  <div className="flex items-center gap-2 shrink-0">
                    <button
                      type="button"
                      disabled={!!importingPath}
                      onClick={() =>
                        onOpenFullForm(
                          folder,
                          normalizedDomain,
                          draft.dbFile,
                          draft.memoryLimitMb,
                          selectedLocalDb
                        )
                      }
                      className="px-3 py-1.5 rounded-lg border border-dark-600 bg-dark-900 hover:bg-dark-700 text-xs text-dark-200 transition-colors disabled:opacity-50"
                    >
                      Full Options
                    </button>
                    <button
                      type="button"
                      disabled={!!importingPath || !draft.domain.trim()}
                      onClick={() => triggerImport()}
                      className="inline-flex items-center gap-1.5 px-3.5 py-1.5 rounded-lg bg-rust-500 hover:bg-rust-600 text-white text-xs font-medium transition-colors disabled:opacity-50"
                    >
                      {isImporting ? (
                        <>
                          <span className="w-3 h-3 border-2 border-white/30 border-t-white rounded-full animate-spin" />
                          Importing...
                        </>
                      ) : failure ? (
                        "Retry Import"
                      ) : (
                        "Import Website"
                      )}
                    </button>
                  </div>
                </div>

                {/* Rollback / Recovery & Memory Limit Recommendation Banner */}
                {failure && (
                  <div className="p-3 rounded-lg bg-warn-500/10 border border-warn-500/35 text-xs space-y-2">
                    <div className="flex items-start justify-between gap-2">
                      <div className="min-w-0">
                        <p className="font-semibold text-warn-300">
                          Previous import failed — folder restored cleanly so you can retry
                        </p>
                        <p className="text-dark-200 mt-0.5 break-words font-mono text-[11px]">
                          {failure.error}
                        </p>
                        {failure.memorySuggestion && (
                          <p className="text-amber-300 mt-1 font-medium">
                            {failure.memorySuggestion}
                          </p>
                        )}
                      </div>
                      {onDismissFailure && (
                        <button
                          type="button"
                          onClick={() => onDismissFailure(folder.path)}
                          className="text-dark-400 hover:text-dark-100 text-sm px-1 shrink-0"
                          title="Dismiss hint"
                        >
                          &times;
                        </button>
                      )}
                    </div>
                    {(draft.dbFile || selectedLocalDb) && (
                      <div className="flex items-center gap-2 flex-wrap">
                        <button
                          type="button"
                          disabled={!!importingPath || !draft.domain.trim()}
                          onClick={() => triggerImport(draft.memoryLimitMb)}
                          className="px-3 py-1 rounded bg-warn-500/20 hover:bg-warn-500/30 border border-warn-500/40 text-warn-300 font-medium transition-colors"
                        >
                          {`Retry Now (${formatMemoryLimitLabel(draft.memoryLimitMb)} Limit)`}
                        </button>
                        <button
                          type="button"
                          disabled={!!importingPath || !draft.domain.trim()}
                          onClick={() => {
                            updateDraft(folder.path, { memoryLimitMb: -1 }, folder);
                            triggerImport(-1);
                          }}
                          className="px-3 py-1 rounded bg-dark-900 hover:bg-dark-700 border border-dark-600 text-dark-200 transition-colors"
                        >
                          Retry with Unlimited Memory (-1)
                        </button>
                      </div>
                    )}
                  </div>
                )}

                {/* Domain, Runtime, Database upload & Memory Limit configuration */}
                <div className="grid grid-cols-1 lg:grid-cols-12 gap-3 pt-1">
                  <div className="lg:col-span-4">
                    <label className="block text-[11px] font-medium text-dark-300 mb-1">
                      Local Domain
                    </label>
                    <div className="flex items-stretch">
                      <input
                        type="text"
                        value={draft.domain}
                        disabled={!!importingPath}
                        onChange={(e) => updateDraft(folder.path, { domain: e.target.value }, folder)}
                        placeholder="mysite.test"
                        aria-label={`Domain for ${folder.name}`}
                        className="flex-1 min-w-0 px-2.5 py-1.5 bg-dark-900 border border-dark-600 rounded-l-lg text-xs font-mono text-dark-100 focus:outline-none focus:border-accent-500"
                      />
                      <button
                        type="button"
                        onClick={() =>
                          updateDraft(
                            folder.path,
                            { domain: formatDesktopDomain(draft.domain || folder.name, ".test") },
                            folder
                          )
                        }
                        className="px-2 border border-l-0 border-dark-600 bg-dark-800 hover:bg-dark-700 text-[11px] font-mono text-dark-300"
                        title="Use .test suffix"
                      >
                        .test
                      </button>
                      <button
                        type="button"
                        onClick={() =>
                          updateDraft(
                            folder.path,
                            { domain: formatDesktopDomain(draft.domain || folder.name, ".local") },
                            folder
                          )
                        }
                        className="px-2 border border-l-0 border-dark-600 rounded-r-lg bg-dark-800 hover:bg-dark-700 text-[11px] font-mono text-dark-300"
                        title="Use .local suffix"
                      >
                        .local
                      </button>
                    </div>
                  </div>

                  <div className="lg:col-span-2">
                    <label className="block text-[11px] font-medium text-dark-300 mb-1">
                      Runtime
                    </label>
                    <select
                      value={draft.runtime}
                      disabled={!!importingPath || folder.inspection.is_wordpress}
                      onChange={(e) => updateDraft(folder.path, { runtime: e.target.value }, folder)}
                      aria-label={`Runtime for ${folder.name}`}
                      className="w-full px-2.5 py-1.5 bg-dark-900 border border-dark-600 rounded-lg text-xs text-dark-100 focus:outline-none focus:border-accent-500 disabled:opacity-60"
                    >
                      <option value="php">PHP</option>
                      <option value="node">Node.js</option>
                      <option value="static">Static</option>
                    </select>
                  </div>

                  <div className="lg:col-span-6">
                    <label className="block text-[11px] font-medium text-dark-300 mb-1">
                      Upload Database{" "}
                      <span className="text-dark-500 font-normal">
                        (.sql, .sql.gz, .zip, .sqlite — optional)
                      </span>
                    </label>
                    <div className="flex items-center gap-2 flex-wrap">
                      <input
                        type="file"
                        accept=".sql,.gz,.sql.gz,.zip,.sqlite,.db"
                        disabled={!!importingPath}
                        aria-label={`Database file for ${folder.name}`}
                        onChange={(e) => {
                          const file = e.target.files?.[0] || null;
                          updateDraft(
                            folder.path,
                            {
                              dbFile: file,
                              localDbPath: file ? "" : draft.localDbPath,
                              localDb: file ? null : draft.localDb,
                            },
                            folder
                          );
                        }}
                        className="text-xs text-dark-300 file:mr-2.5 file:py-1 file:px-2.5 file:rounded-md file:border file:border-dark-600 file:text-xs file:font-medium file:bg-dark-700 file:text-dark-100 hover:file:bg-dark-600 max-w-full"
                      />
                      {draft.dbFile && (
                        <span className="text-[11px] font-mono text-emerald-300">
                          {`${draft.dbFile.name} (${formatDbBytesShort(draft.dbFile.size)})`}
                        </span>
                      )}
                      {(draft.dbFile || selectedLocalDb) && (
                        <button
                          type="button"
                          onClick={() =>
                            updateDraft(
                              folder.path,
                              { dbFile: null, localDbPath: "", localDb: null },
                              folder
                            )
                          }
                          className="text-[11px] text-dark-400 hover:text-dark-100 underline"
                        >
                          Clear DB
                        </button>
                      )}
                    </div>

                    <DirectoryDbSelector
                      detectedDbFiles={folder.detected_db_files}
                      otherDirectoryDbFiles={rootDbFiles}
                      selectedLocalDb={selectedLocalDb}
                      uploadedFile={draft.dbFile}
                      siteDir={folder.path}
                      sitesRootDir={sitesDir}
                      disabled={!!importingPath}
                      onSelectLocalDb={(dbItem) =>
                        updateDraft(
                          folder.path,
                          {
                            localDbPath: dbItem?.path || "",
                            localDb: dbItem,
                            dbFile: null,
                            sourceUrl:
                              folder.inspection.wp_siteurl ||
                              dbItem?.detected_site_url ||
                              draft.sourceUrl,
                            dbName: dbItem?.detected_db_name || draft.dbName,
                          },
                          folder
                        )
                      }
                    />

                    {(draft.dbFile || selectedLocalDb) && (
                      <div className="mt-2 grid grid-cols-1 sm:grid-cols-2 gap-2">
                        {folder.inspection.is_wordpress ? (
                          <input
                            type="url"
                            value={draft.sourceUrl}
                            onChange={(e) =>
                              updateDraft(folder.path, { sourceUrl: e.target.value }, folder)
                            }
                            placeholder="Source URL for rewrite (optional)"
                            className="w-full px-2.5 py-1 bg-dark-900 border border-dark-600 rounded text-[11px] font-mono text-dark-100 focus:outline-none focus:border-accent-500"
                          />
                        ) : (
                          <input
                            type="text"
                            value={draft.dbName}
                            onChange={(e) =>
                              updateDraft(folder.path, { dbName: e.target.value }, folder)
                            }
                            placeholder="Target DB name (defaults to domain)"
                            className="w-full px-2.5 py-1 bg-dark-900 border border-dark-600 rounded text-[11px] font-mono text-dark-100 focus:outline-none focus:border-accent-500"
                          />
                        )}
                        <select
                          aria-label={`Import memory limit for ${folder.name}`}
                          value={draft.memoryLimitMb}
                          onChange={(e) =>
                            updateDraft(
                              folder.path,
                              { memoryLimitMb: parseInt(e.target.value, 10) },
                              folder
                            )
                          }
                          className="w-full px-2.5 py-1 bg-dark-900 border border-dark-600 rounded text-[11px] font-mono text-dark-100 focus:outline-none focus:border-accent-500"
                          title="PHP/SQL Memory Limit for large imports"
                        >
                          {MEMORY_LIMIT_OPTIONS.map((opt) => (
                            <option key={opt.value} value={opt.value}>
                              {`Import Memory: ${opt.label}`}
                            </option>
                          ))}
                        </select>
                      </div>
                    )}
                  </div>
                </div>
              </div>
            );
          })}
        </div>
      )}
    </section>
  );
}
