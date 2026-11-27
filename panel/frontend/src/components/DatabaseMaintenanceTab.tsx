import { useState, useEffect, useCallback } from "react";
import { api } from "../api";
import { showGlobalNotification, buildAlertDetails } from "../context/NotificationContext";
import ConfirmDialog from "./ConfirmDialog";

export interface DbTableHealth {
  name: string;
  rows: number;
  data_bytes: number;
  index_bytes: number;
  overhead_bytes: number;
  engine: string;
  dead_tuples: number;
  needs_optimize: boolean;
}

export interface WpBloatStats {
  detected: boolean;
  table_prefix: string;
  revisions: number;
  auto_drafts: number;
  trashed_posts: number;
  spam_comments: number;
  trashed_comments: number;
  orphaned_meta: number;
  expired_transients: number;
  total_transients: number;
  autoload_bytes: number;
  autoload_count: number;
}

export interface DbHealthReport {
  database_id: string;
  database_name: string;
  engine: string;
  total_bytes: number;
  data_bytes: number;
  index_bytes: number;
  overhead_bytes: number;
  table_count: number;
  fragmented_table_count: number;
  dump_count: number;
  dump_bytes: number;
  integrity: string;
  tables: DbTableHealth[];
  wp_bloat: WpBloatStats;
}

interface OptimizeReport {
  ok: boolean;
  action: string;
  engine: string;
  tables_processed: number;
  size_before_bytes: number;
  size_after_bytes: number;
  reclaimed_bytes: number;
  integrity_status: string;
  results: Array<{ table: string; status: string; message: string }>;
}

interface PruneReport {
  ok: boolean;
  dry_run: boolean;
  rows_deleted: number;
  dumps_deleted: number;
  reclaimed_bytes: number;
  optimized: boolean;
  breakdown: Record<string, number>;
}

function fmtBytes(bytes: number): string {
  if (!bytes || bytes <= 0) return "0 B";
  if (bytes >= 1_073_741_824) return `${(bytes / 1_073_741_824).toFixed(2)} GB`;
  if (bytes >= 1_048_576) return `${(bytes / 1_048_576).toFixed(2)} MB`;
  if (bytes >= 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${bytes} B`;
}

export default function DatabaseMaintenanceTab({
  databaseId,
  databaseName,
  engine,
  onChanged,
}: {
  databaseId: string;
  databaseName: string;
  engine: string;
  onChanged?: () => void;
}) {
  const [health, setHealth] = useState<DbHealthReport | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState("");
  const [runningAction, setRunningAction] = useState<string | null>(null);
  const [selectedTables, setSelectedTables] = useState<string[]>([]);
  const [lastOptReport, setLastOptReport] = useState<OptimizeReport | null>(null);
  const [lastPruneReport, setLastPruneReport] = useState<PruneReport | null>(null);

  const [wpCategories, setWpCategories] = useState<string[]>([
    "revisions",
    "auto_drafts",
    "trashed_posts",
    "spam_comments",
    "orphaned_meta",
    "expired_transients",
  ]);
  const [optimizeAfterPrune, setOptimizeAfterPrune] = useState(true);
  const [keepDumps, setKeepDumps] = useState(3);
  const [olderThanDays, setOlderThanDays] = useState(30);
  const [confirmTruncate, setConfirmTruncate] = useState(false);

  const loadHealth = useCallback(async () => {
    setLoading(true);
    setError("");
    try {
      const data = await api.get<DbHealthReport>(`/databases/${databaseId}/health`);
      setHealth(data);
    } catch (err) {
      const msg = err instanceof Error ? err.message : "Failed inspecting database health";
      setError(msg);
    } finally {
      setLoading(false);
    }
  }, [databaseId]);

  useEffect(() => {
    loadHealth();
  }, [loadHealth]);

  const runOptimize = async (action: "optimize" | "analyze" | "repair" | "check", onlySelected = false) => {
    const startedAt = typeof performance !== "undefined" ? performance.now() : Date.now();
    setRunningAction(action);
    try {
      const body: { action: string; tables?: string[] } = { action };
      if (onlySelected && selectedTables.length > 0) {
        body.tables = selectedTables;
      }
      const res = await api.post<OptimizeReport>(`/databases/${databaseId}/optimize`, body);
      const durationMs = Math.max(1, Math.round((typeof performance !== "undefined" ? performance.now() : Date.now()) - startedAt));
      setLastOptReport(res);
      showGlobalNotification(
        "success",
        `Completed '${action}' on ${databaseName} (${res.tables_processed} target${res.tables_processed === 1 ? "" : "s"}, reclaimed ${fmtBytes(res.reclaimed_bytes)})`,
        "Database Optimized",
        `Database:   ${databaseName} (${engine})\nAction:     ${action.toUpperCase()}\nBefore:     ${fmtBytes(res.size_before_bytes)}\nAfter:      ${fmtBytes(res.size_after_bytes)}\nReclaimed:  ${fmtBytes(res.reclaimed_bytes)}\nIntegrity:  ${res.integrity_status}\nElapsed:    ${durationMs}ms`,
        {
          category: "database",
          target: databaseName,
          endpoint: `/databases/${databaseId}/optimize`,
          method: "POST",
          statusCode: 200,
          durationMs,
        }
      );
      await loadHealth();
      onChanged?.();
    } catch (err) {
      const info = buildAlertDetails(err, {
        context: `Optimize database ${databaseName} (${action})`,
        fallback: "Optimization failed",
        category: "database",
        target: databaseName,
      });
      showGlobalNotification("error", info.message, "Optimization Failed", info.details, info.meta);
    } finally {
      setRunningAction(null);
    }
  };

  const runPrune = async (dryRun: boolean, opts?: { includeDumps?: boolean; truncateSelected?: boolean }) => {
    const startedAt = typeof performance !== "undefined" ? performance.now() : Date.now();
    setRunningAction(dryRun ? "dry-run" : "prune");
    try {
      const payload: Record<string, unknown> = {
        dry_run: dryRun,
        categories: opts?.truncateSelected ? [] : wpCategories,
        optimize_after: !dryRun && !opts?.truncateSelected ? optimizeAfterPrune : false,
      };
      if (opts?.includeDumps) {
        payload.categories = [];
        payload.keep_dumps = keepDumps;
        payload.older_than_days = olderThanDays;
        payload.optimize_after = false;
      }
      if (opts?.truncateSelected) {
        payload.truncate_tables = selectedTables;
      }

      const res = await api.post<PruneReport>(`/databases/${databaseId}/prune`, payload);
      const durationMs = Math.max(1, Math.round((typeof performance !== "undefined" ? performance.now() : Date.now()) - startedAt));
      setLastPruneReport(res);
      showGlobalNotification(
        dryRun ? "info" : "success",
        dryRun
          ? `Dry-run: ${res.rows_deleted} row(s) eligible for pruning in ${databaseName}`
          : `Pruned ${res.rows_deleted} row(s) & ${res.dumps_deleted} dump(s) in ${databaseName} (reclaimed ${fmtBytes(res.reclaimed_bytes)})`,
        dryRun ? "Prune Preview" : "Database Pruned",
        `Database:  ${databaseName}\nRows:      ${res.rows_deleted}\nDumps:     ${res.dumps_deleted}\nReclaimed: ${fmtBytes(res.reclaimed_bytes)}\nOptimized: ${res.optimized ? "Yes" : "No"}\nBreakdown: ${JSON.stringify(res.breakdown)}\nElapsed:   ${durationMs}ms`,
        {
          category: "database",
          target: databaseName,
          endpoint: `/databases/${databaseId}/prune`,
          method: "POST",
          statusCode: 200,
          durationMs,
        }
      );
      if (!dryRun) {
        if (opts?.truncateSelected) setSelectedTables([]);
        await loadHealth();
        onChanged?.();
      }
    } catch (err) {
      const info = buildAlertDetails(err, {
        context: `Prune database ${databaseName}`,
        fallback: "Prune operation failed",
        category: "database",
        target: databaseName,
      });
      showGlobalNotification("error", info.message, "Prune Failed", info.details, info.meta);
    } finally {
      setRunningAction(null);
    }
  };

  if (loading && !health) {
    return (
      <div className="p-6 space-y-4 animate-pulse">
        <div className="grid grid-cols-2 lg:grid-cols-4 gap-3">
          {[...Array(4)].map((_, i) => (
            <div key={i} className="h-20 bg-dark-700/60 rounded-lg" />
          ))}
        </div>
        <div className="h-48 bg-dark-700/40 rounded-lg" />
      </div>
    );
  }

  if (error && !health) {
    return (
      <div className="p-6">
        <div className="p-4 rounded-lg bg-danger-500/10 border border-danger-500/30 text-sm text-danger-300 flex items-center justify-between">
          <span>{error}</span>
          <button
            type="button"
            onClick={loadHealth}
            className="px-3 py-1 rounded bg-dark-800 border border-dark-600 text-xs text-dark-100 hover:bg-dark-700"
          >
            Retry
          </button>
        </div>
      </div>
    );
  }

  if (!health) return null;

  const toggleCategory = (cat: string) => {
    setWpCategories((prev) =>
      prev.includes(cat) ? prev.filter((c) => c !== cat) : [...prev, cat]
    );
  };

  const wp = health.wp_bloat;
  const wpBloatItems = [
    { id: "revisions", label: "Post Revisions", count: wp.revisions },
    { id: "auto_drafts", label: "Auto-Drafts", count: wp.auto_drafts },
    { id: "trashed_posts", label: "Trashed Posts", count: wp.trashed_posts },
    { id: "spam_comments", label: "Spam & Trash Comments", count: wp.spam_comments + wp.trashed_comments },
    { id: "orphaned_meta", label: "Orphaned Post/Comment Meta", count: wp.orphaned_meta },
    { id: "expired_transients", label: "Expired Transients", count: wp.expired_transients },
    { id: "all_transients", label: "All Transients (Full Flush)", count: wp.total_transients },
  ];

  const allSelected =
    health.tables.length > 0 && selectedTables.length === health.tables.length;

  return (
    <div className="p-5 space-y-5 overflow-y-auto flex-1">
      {/* Health Summary Cards */}
      <div className="grid grid-cols-2 lg:grid-cols-4 gap-3">
        <div className="p-3.5 rounded-lg bg-dark-900/70 border border-dark-700">
          <div className="text-[11px] font-mono uppercase text-dark-400">Total Storage</div>
          <div className="text-lg font-semibold text-dark-50 mt-1 font-mono">{fmtBytes(health.total_bytes)}</div>
          <div className="text-[11px] text-dark-400 mt-0.5">
            Data {fmtBytes(health.data_bytes)} · Index {fmtBytes(health.index_bytes)}
          </div>
        </div>

        <div className="p-3.5 rounded-lg bg-dark-900/70 border border-dark-700">
          <div className="text-[11px] font-mono uppercase text-dark-400">Reclaimable Overhead</div>
          <div className={`text-lg font-semibold mt-1 font-mono ${health.overhead_bytes > 65536 ? "text-warn-400" : "text-rust-400"}`}>
            {fmtBytes(health.overhead_bytes)}
          </div>
          <div className="text-[11px] text-dark-400 mt-0.5">
            {health.fragmented_table_count} table{health.fragmented_table_count === 1 ? "" : "s"} need compacting
          </div>
        </div>

        <div className="p-3.5 rounded-lg bg-dark-900/70 border border-dark-700">
          <div className="text-[11px] font-mono uppercase text-dark-400">Tables & Integrity</div>
          <div className="text-lg font-semibold text-dark-50 mt-1 font-mono">
            {health.table_count} <span className="text-xs font-normal text-emerald-400">({health.integrity})</span>
          </div>
          <div className="text-[11px] text-dark-400 mt-0.5 uppercase font-mono">{health.engine}</div>
        </div>

        <div className="p-3.5 rounded-lg bg-dark-900/70 border border-dark-700">
          <div className="text-[11px] font-mono uppercase text-dark-400">Saved Dump Files</div>
          <div className="text-lg font-semibold text-dark-50 mt-1 font-mono">{health.dump_count}</div>
          <div className="text-[11px] text-dark-400 mt-0.5">{fmtBytes(health.dump_bytes)} on disk</div>
        </div>
      </div>

      {/* Engine Optimization Actions */}
      <div className="p-4 rounded-lg bg-dark-900/60 border border-dark-700 space-y-3">
        <div className="flex items-center justify-between gap-3 flex-wrap">
          <div>
            <h3 className="text-sm font-semibold text-dark-100">Engine Optimization & Compaction</h3>
            <p className="text-xs text-dark-400">
              Reclaim fragmented pages, rebuild table indexes, and refresh query optimizer statistics.
            </p>
          </div>
          <div className="flex items-center gap-2 flex-wrap">
            <button
              type="button"
              disabled={!!runningAction}
              onClick={() => runOptimize("optimize", false)}
              className="px-3.5 py-1.5 rounded-lg bg-rust-500 hover:bg-rust-600 text-white text-xs font-medium disabled:opacity-50"
            >
              {runningAction === "optimize"
                ? "Optimizing..."
                : engine === "sqlite"
                ? "Vacuum & Compact"
                : engine === "postgres"
                ? "Vacuum & Analyze"
                : "Optimize All Tables"}
            </button>
            <button
              type="button"
              disabled={!!runningAction}
              onClick={() => runOptimize("analyze", false)}
              className="px-3 py-1.5 rounded-lg bg-dark-800 hover:bg-dark-700 border border-dark-600 text-dark-200 text-xs disabled:opacity-50"
            >
              {runningAction === "analyze" ? "Analyzing..." : "Analyze"}
            </button>
            <button
              type="button"
              disabled={!!runningAction}
              onClick={() => runOptimize(engine === "sqlite" ? "check" : "repair", false)}
              className="px-3 py-1.5 rounded-lg bg-dark-800 hover:bg-dark-700 border border-dark-600 text-dark-200 text-xs disabled:opacity-50"
            >
              {engine === "sqlite" ? "Integrity Check" : "Repair Tables"}
            </button>
            <button
              type="button"
              onClick={loadHealth}
              disabled={loading || !!runningAction}
              className="px-2.5 py-1.5 rounded-lg bg-dark-800 hover:bg-dark-700 border border-dark-600 text-dark-300 text-xs disabled:opacity-50"
            >
              Refresh
            </button>
          </div>
        </div>

        {lastOptReport && (
          <div className="p-2.5 rounded bg-emerald-500/10 border border-emerald-500/30 text-xs text-emerald-300 font-mono flex items-center justify-between gap-2 flex-wrap">
            <span>
              Last {lastOptReport.action.toUpperCase()}: {lastOptReport.tables_processed} target(s) · {fmtBytes(lastOptReport.size_before_bytes)} → {fmtBytes(lastOptReport.size_after_bytes)} (Reclaimed {fmtBytes(lastOptReport.reclaimed_bytes)})
            </span>
            <span>Status: {lastOptReport.integrity_status}</span>
          </div>
        )}
      </div>

      {/* WordPress Bloat Pruner */}
      {wp.detected && (
        <div className="p-4 rounded-lg bg-dark-900/60 border border-accent-500/30 space-y-3">
          <div className="flex items-start justify-between gap-3 flex-wrap">
            <div>
              <div className="flex items-center gap-2">
                <h3 className="text-sm font-semibold text-dark-100">WordPress Bloat Cleanup & Pruner</h3>
                <span className="text-[10px] font-mono px-2 py-0.5 rounded bg-accent-500/20 text-accent-300 border border-accent-500/30">
                  Prefix: {wp.table_prefix}
                </span>
              </div>
              <p className="text-xs text-dark-400 mt-0.5">
                Autoloaded options: <span className="font-mono text-dark-200">{wp.autoload_count}</span> ({fmtBytes(wp.autoload_bytes)})
                {wp.autoload_bytes > 512 * 1024 && (
                  <span className="ml-2 text-warn-400 font-medium">High autoload footprint</span>
                )}
              </p>
            </div>
            <div className="flex items-center gap-2 flex-wrap">
              <button
                type="button"
                disabled={!!runningAction || wpCategories.length === 0}
                onClick={() => runPrune(true)}
                className="px-3 py-1.5 rounded-lg bg-dark-800 hover:bg-dark-700 border border-dark-600 text-dark-200 text-xs disabled:opacity-50"
              >
                {runningAction === "dry-run" ? "Scanning..." : "Preview Dry-Run"}
              </button>
              <button
                type="button"
                disabled={!!runningAction || wpCategories.length === 0}
                onClick={() => runPrune(false)}
                className="px-3.5 py-1.5 rounded-lg bg-accent-600 hover:bg-accent-500 text-white text-xs font-medium disabled:opacity-50"
              >
                {runningAction === "prune" ? "Pruning..." : "Prune Selected"}
              </button>
            </div>
          </div>

          <div className="grid grid-cols-1 sm:grid-cols-2 lg:grid-cols-3 gap-2">
            {wpBloatItems.map((item) => {
              const checked = wpCategories.includes(item.id);
              return (
                <label
                  key={item.id}
                  className={`flex items-center justify-between gap-2 px-3 py-2 rounded-lg border text-xs cursor-pointer transition-colors ${
                    checked
                      ? "bg-dark-800 border-accent-500/40 text-dark-100"
                      : "bg-dark-900/70 border-dark-700 text-dark-400"
                  }`}
                >
                  <span className="flex items-center gap-2 min-w-0">
                    <input
                      type="checkbox"
                      checked={checked}
                      onChange={() => toggleCategory(item.id)}
                      className="rounded border-dark-600 bg-dark-800 text-accent-500"
                    />
                    <span className="truncate">{item.label}</span>
                  </span>
                  <span
                    className={`font-mono px-1.5 py-0.5 rounded text-[11px] ${
                      item.count > 0 ? "bg-warn-500/15 text-warn-400" : "bg-dark-700 text-dark-400"
                    }`}
                  >
                    {item.count}
                  </span>
                </label>
              );
            })}
          </div>

          <label className="inline-flex items-center gap-2 text-xs text-dark-300 cursor-pointer">
            <input
              type="checkbox"
              checked={optimizeAfterPrune}
              onChange={(e) => setOptimizeAfterPrune(e.target.checked)}
              className="rounded border-dark-600 bg-dark-800 text-rust-500"
            />
            Automatically Vacuum / Optimize tables immediately after pruning to reclaim disk space
          </label>

          {lastPruneReport && (
            <div className="p-2.5 rounded bg-dark-800 border border-dark-600 text-xs font-mono text-dark-200">
              {lastPruneReport.dry_run ? "[DRY RUN] " : ""}
              Rows: {lastPruneReport.rows_deleted} · Dumps pruned: {lastPruneReport.dumps_deleted} · Reclaimed: {fmtBytes(lastPruneReport.reclaimed_bytes)}
            </div>
          )}
        </div>
      )}

      {/* Dump File Retention Pruner */}
      <div className="p-4 rounded-lg bg-dark-900/60 border border-dark-700 flex items-center justify-between gap-4 flex-wrap">
        <div>
          <h3 className="text-sm font-semibold text-dark-100">Export Dump Retention Pruner</h3>
          <p className="text-xs text-dark-400">
            Keep only recent backup dumps for <span className="font-mono text-dark-200">{databaseName}</span> ({health.dump_count} files, {fmtBytes(health.dump_bytes)}).
          </p>
        </div>
        <div className="flex items-center gap-2 flex-wrap text-xs">
          <label className="flex items-center gap-1.5 text-dark-300">
            Keep newest:
            <input
              type="number"
              min={0}
              max={100}
              value={keepDumps}
              onChange={(e) => setKeepDumps(Math.max(0, parseInt(e.target.value) || 0))}
              className="w-16 px-2 py-1 bg-dark-800 border border-dark-600 rounded text-dark-100 font-mono"
            />
          </label>
          <label className="flex items-center gap-1.5 text-dark-300">
            Or older than (days):
            <input
              type="number"
              min={1}
              max={3650}
              value={olderThanDays}
              onChange={(e) => setOlderThanDays(Math.max(1, parseInt(e.target.value) || 1))}
              className="w-16 px-2 py-1 bg-dark-800 border border-dark-600 rounded text-dark-100 font-mono"
            />
          </label>
          <button
            type="button"
            disabled={!!runningAction || health.dump_count === 0}
            onClick={() => runPrune(false, { includeDumps: true })}
            className="px-3 py-1.5 rounded-lg bg-dark-700 hover:bg-dark-600 text-dark-100 font-medium disabled:opacity-40"
          >
            Prune Old Dumps
          </button>
        </div>
      </div>

      {/* Per-Table Bloat & Overhead Table */}
      <div className="rounded-lg bg-dark-900/60 border border-dark-700 overflow-hidden">
        <div className="px-4 py-3 border-b border-dark-700 flex items-center justify-between gap-2 flex-wrap">
          <span className="text-xs font-semibold uppercase font-mono text-dark-300">
            Table Storage & Overhead ({health.tables.length})
          </span>
          {selectedTables.length > 0 && (
            <div className="flex items-center gap-2">
              <button
                type="button"
                disabled={!!runningAction}
                onClick={() => runOptimize("optimize", true)}
                className="px-2.5 py-1 rounded bg-rust-500 hover:bg-rust-600 text-white text-xs font-medium disabled:opacity-50"
              >
                Optimize Selected ({selectedTables.length})
              </button>
              <button
                type="button"
                disabled={!!runningAction}
                onClick={() => setConfirmTruncate(true)}
                className="px-2.5 py-1 rounded bg-danger-500/20 hover:bg-danger-500/30 border border-danger-500/40 text-danger-300 text-xs disabled:opacity-50"
              >
                Empty Selected ({selectedTables.length})
              </button>
            </div>
          )}
        </div>

        <div className="overflow-x-auto max-h-80">
          <table className="w-full text-xs">
            <thead className="bg-dark-850 text-dark-400 border-b border-dark-700 text-left sticky top-0">
              <tr>
                <th className="px-3 py-2 w-8">
                  <input
                    type="checkbox"
                    checked={allSelected}
                    onChange={() =>
                      setSelectedTables(allSelected ? [] : health.tables.map((t) => t.name))
                    }
                    className="rounded border-dark-600 bg-dark-800"
                    aria-label="Select all tables"
                  />
                </th>
                <th className="px-3 py-2 font-mono">Table</th>
                <th className="px-3 py-2 font-mono text-right">Rows</th>
                <th className="px-3 py-2 font-mono text-right">Data</th>
                <th className="px-3 py-2 font-mono text-right">Indexes</th>
                <th className="px-3 py-2 font-mono text-right">Overhead</th>
                <th className="px-3 py-2 font-mono">Status</th>
              </tr>
            </thead>
            <tbody className="divide-y divide-dark-800">
              {health.tables.map((t) => {
                const isSel = selectedTables.includes(t.name);
                return (
                  <tr key={t.name} className="hover:bg-dark-800/40">
                    <td className="px-3 py-2">
                      <input
                        type="checkbox"
                        checked={isSel}
                        onChange={() =>
                          setSelectedTables((prev) =>
                            isSel ? prev.filter((x) => x !== t.name) : [...prev, t.name]
                          )
                        }
                        className="rounded border-dark-600 bg-dark-800"
                      />
                    </td>
                    <td className="px-3 py-2 font-mono text-dark-100">{t.name}</td>
                    <td className="px-3 py-2 font-mono text-right text-dark-300">{t.rows.toLocaleString()}</td>
                    <td className="px-3 py-2 font-mono text-right text-dark-300">{fmtBytes(t.data_bytes)}</td>
                    <td className="px-3 py-2 font-mono text-right text-dark-300">{fmtBytes(t.index_bytes)}</td>
                    <td className="px-3 py-2 font-mono text-right text-dark-300">
                      {t.dead_tuples > 0 ? `${t.dead_tuples} dead` : fmtBytes(t.overhead_bytes)}
                    </td>
                    <td className="px-3 py-2">
                      {t.needs_optimize ? (
                        <span className="px-1.5 py-0.5 rounded bg-warn-500/15 text-warn-400 border border-warn-500/30 text-[10px] font-mono">
                          Fragmented
                        </span>
                      ) : (
                        <span className="px-1.5 py-0.5 rounded bg-emerald-500/15 text-emerald-300 border border-emerald-500/30 text-[10px] font-mono">
                          Healthy
                        </span>
                      )}
                    </td>
                  </tr>
                );
              })}
              {health.tables.length === 0 && (
                <tr>
                  <td colSpan={7} className="px-4 py-6 text-center text-dark-400">
                    No user tables found in this database.
                  </td>
                </tr>
              )}
            </tbody>
          </table>
        </div>
      </div>

      {confirmTruncate && (
        <ConfirmDialog
          title="Empty Selected Tables"
          label={`Truncate ${selectedTables.length} table(s) in ${databaseName}?`}
          consequences={[
            `All rows inside: ${selectedTables.join(", ")} will be permanently deleted.`,
            "Table schema and indexes remain intact.",
          ]}
          confirmLabel="Empty Selected Tables"
          tone="danger"
          onConfirm={() => {
            setConfirmTruncate(false);
            runPrune(false, { truncateSelected: true });
          }}
          onCancel={() => setConfirmTruncate(false)}
        />
      )}
    </div>
  );
}
