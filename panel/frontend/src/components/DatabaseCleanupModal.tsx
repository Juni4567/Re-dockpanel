import { useState, useEffect, useCallback } from "react";
import { api } from "../api";
import { showGlobalNotification, buildAlertDetails } from "../context/NotificationContext";
import ConfirmDialog from "./ConfirmDialog";

interface OrphanDatabase {
  id: string;
  name: string;
  engine: string;
  size_bytes: number;
  table_count: number;
  reason: string;
}

interface MaintenanceSummary {
  total_databases: number;
  total_bytes: number;
  total_overhead_bytes: number;
  total_dumps: number;
  total_dump_bytes: number;
  orphan_databases: OrphanDatabase[];
}

function fmtBytes(bytes: number): string {
  if (!bytes || bytes <= 0) return "0 B";
  if (bytes >= 1_073_741_824) return `${(bytes / 1_073_741_824).toFixed(2)} GB`;
  if (bytes >= 1_048_576) return `${(bytes / 1_048_576).toFixed(2)} MB`;
  if (bytes >= 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${bytes} B`;
}

export default function DatabaseCleanupModal({
  onClose,
  onCompleted,
}: {
  onClose: () => void;
  onCompleted: () => void;
}) {
  const [summary, setSummary] = useState<MaintenanceSummary | null>(null);
  const [loading, setLoading] = useState(true);
  const [running, setRunning] = useState(false);
  const [optimizeAll, setOptimizeAll] = useState(true);
  const [pruneDumps, setPruneDumps] = useState(false);
  const [keepDumps, setKeepDumps] = useState(3);
  const [olderThanDays, setOlderThanDays] = useState(30);
  const [selectedOrphans, setSelectedOrphans] = useState<string[]>([]);
  const [confirmOrphanDelete, setConfirmOrphanDelete] = useState(false);

  const loadSummary = useCallback(async () => {
    setLoading(true);
    try {
      const data = await api.get<MaintenanceSummary>("/databases/maintenance/summary");
      setSummary(data);
    } catch (err) {
      const info = buildAlertDetails(err, {
        context: "Scan workspace database maintenance summary",
        fallback: "Failed scanning database summary",
        category: "database",
      });
      showGlobalNotification("error", info.message, "Maintenance Scan Failed", info.details, info.meta);
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    loadSummary();
  }, [loadSummary]);

  const executeCleanup = async () => {
    const startedAt = typeof performance !== "undefined" ? performance.now() : Date.now();
    setRunning(true);
    try {
      const payload: Record<string, unknown> = {
        optimize_all: optimizeAll,
        delete_orphan_ids: selectedOrphans,
      };
      if (pruneDumps) {
        payload.prune_dumps_keep = keepDumps;
        payload.prune_dumps_older_than_days = olderThanDays;
      }
      const res = await api.post<{
        ok: boolean;
        optimized_count: number;
        reclaimed_bytes: number;
        dumps_deleted: number;
        orphans_deleted: number;
        errors?: string[];
      }>("/databases/maintenance/cleanup", payload);

      const durationMs = Math.max(1, Math.round((typeof performance !== "undefined" ? performance.now() : Date.now()) - startedAt));
      showGlobalNotification(
        res.errors && res.errors.length > 0 ? "warn" : "success",
        `Optimized ${res.optimized_count} DB(s), pruned ${res.dumps_deleted} dump(s), removed ${res.orphans_deleted} orphan(s) — reclaimed ${fmtBytes(res.reclaimed_bytes)}`,
        "Database Cleanup Completed",
        `Optimized:       ${res.optimized_count}\nDumps Deleted:   ${res.dumps_deleted}\nOrphans Removed: ${res.orphans_deleted}\nReclaimed Space: ${fmtBytes(res.reclaimed_bytes)}\nElapsed:         ${durationMs}ms${res.errors?.length ? `\nNotes:\n${res.errors.join("\n")}` : ""}`,
        {
          category: "database",
          target: "workspace",
          endpoint: "/databases/maintenance/cleanup",
          method: "POST",
          statusCode: 200,
          durationMs,
        }
      );
      setSelectedOrphans([]);
      await loadSummary();
      onCompleted();
    } catch (err) {
      const info = buildAlertDetails(err, {
        context: "Run global database cleanup & optimization",
        fallback: "Cleanup failed",
        category: "database",
      });
      showGlobalNotification("error", info.message, "Cleanup Failed", info.details, info.meta);
    } finally {
      setRunning(false);
    }
  };

  return (
    <div className="fixed inset-0 bg-black/60 backdrop-blur-sm flex items-center justify-center z-50 p-4">
      <div className="bg-dark-800 rounded-xl border border-dark-600 w-full max-w-2xl max-h-[88vh] flex flex-col overflow-hidden shadow-2xl">
        <div className="px-5 py-4 border-b border-dark-700 flex items-center justify-between">
          <div>
            <h2 className="text-base font-semibold text-dark-50">Workspace Database Cleanup & Optimization</h2>
            <p className="text-xs text-dark-400 mt-0.5">
              Optimize all databases, prune stale dump archives, and clean up unlinked databases.
            </p>
          </div>
          <button
            type="button"
            onClick={onClose}
            className="text-dark-400 hover:text-dark-100 text-lg leading-none px-1"
            aria-label="Close"
          >
            &times;
          </button>
        </div>

        <div className="p-5 space-y-4 overflow-y-auto flex-1">
          {loading || !summary ? (
            <div className="py-10 text-center text-sm text-dark-400 animate-pulse">
              Scanning databases, tables, and dump files...
            </div>
          ) : (
            <>
              <div className="grid grid-cols-2 sm:grid-cols-4 gap-3">
                <div className="p-3 rounded-lg bg-dark-900 border border-dark-700">
                  <div className="text-[10px] font-mono uppercase text-dark-400">Databases</div>
                  <div className="text-base font-semibold text-dark-50 font-mono mt-0.5">{summary.total_databases}</div>
                </div>
                <div className="p-3 rounded-lg bg-dark-900 border border-dark-700">
                  <div className="text-[10px] font-mono uppercase text-dark-400">Total Size</div>
                  <div className="text-base font-semibold text-dark-50 font-mono mt-0.5">{fmtBytes(summary.total_bytes)}</div>
                </div>
                <div className="p-3 rounded-lg bg-dark-900 border border-dark-700">
                  <div className="text-[10px] font-mono uppercase text-dark-400">Overhead</div>
                  <div className="text-base font-semibold text-warn-400 font-mono mt-0.5">{fmtBytes(summary.total_overhead_bytes)}</div>
                </div>
                <div className="p-3 rounded-lg bg-dark-900 border border-dark-700">
                  <div className="text-[10px] font-mono uppercase text-dark-400">Saved Dumps</div>
                  <div className="text-base font-semibold text-dark-50 font-mono mt-0.5">
                    {summary.total_dumps} ({fmtBytes(summary.total_dump_bytes)})
                  </div>
                </div>
              </div>

              {/* Cleanup Options */}
              <div className="space-y-3">
                <label className="flex items-start gap-3 p-3.5 rounded-lg bg-dark-900/70 border border-dark-700 cursor-pointer">
                  <input
                    type="checkbox"
                    checked={optimizeAll}
                    onChange={(e) => setOptimizeAll(e.target.checked)}
                    className="mt-0.5 rounded border-dark-600 bg-dark-800 text-rust-500"
                  />
                  <div>
                    <div className="text-sm font-medium text-dark-100">Optimize & Compact All Databases</div>
                    <div className="text-xs text-dark-400 mt-0.5">
                      Runs <code className="text-dark-200">VACUUM</code> / <code className="text-dark-200">OPTIMIZE TABLE</code> across every database to reclaim free space and rebuild indexes.
                    </div>
                  </div>
                </label>

                <div className="p-3.5 rounded-lg bg-dark-900/70 border border-dark-700 space-y-2.5">
                  <label className="flex items-start gap-3 cursor-pointer">
                    <input
                      type="checkbox"
                      checked={pruneDumps}
                      onChange={(e) => setPruneDumps(e.target.checked)}
                      className="mt-0.5 rounded border-dark-600 bg-dark-800 text-accent-500"
                    />
                    <div>
                      <div className="text-sm font-medium text-dark-100">Prune Old Database Export Dumps</div>
                      <div className="text-xs text-dark-400 mt-0.5">
                        Remove older <code className="text-dark-200">.sql</code> / <code className="text-dark-200">.sql.gz</code> exports across all databases.
                      </div>
                    </div>
                  </label>

                  {pruneDumps && (
                    <div className="pl-7 flex items-center gap-4 flex-wrap text-xs text-dark-300">
                      <label className="flex items-center gap-1.5">
                        Keep newest per DB:
                        <input
                          type="number"
                          min={0}
                          max={100}
                          value={keepDumps}
                          onChange={(e) => setKeepDumps(Math.max(0, parseInt(e.target.value) || 0))}
                          className="w-16 px-2 py-1 bg-dark-800 border border-dark-600 rounded font-mono text-dark-100"
                        />
                      </label>
                      <label className="flex items-center gap-1.5">
                        Or older than (days):
                        <input
                          type="number"
                          min={1}
                          max={3650}
                          value={olderThanDays}
                          onChange={(e) => setOlderThanDays(Math.max(1, parseInt(e.target.value) || 1))}
                          className="w-16 px-2 py-1 bg-dark-800 border border-dark-600 rounded font-mono text-dark-100"
                        />
                      </label>
                    </div>
                  )}
                </div>

                {/* Orphaned Databases Audit */}
                <div className="rounded-lg bg-dark-900/70 border border-dark-700 overflow-hidden">
                  <div className="px-3.5 py-2.5 border-b border-dark-700 flex items-center justify-between">
                    <span className="text-xs font-semibold text-dark-200">
                      Unlinked / Orphaned Databases ({summary.orphan_databases.length})
                    </span>
                    <button
                      type="button"
                      onClick={loadSummary}
                      className="text-[11px] text-dark-400 hover:text-dark-100"
                    >
                      Rescan
                    </button>
                  </div>
                  {summary.orphan_databases.length === 0 ? (
                    <p className="px-3.5 py-3 text-xs text-dark-400">
                      All discovered databases are cleanly linked to active sites or stacks.
                    </p>
                  ) : (
                    <div className="divide-y divide-dark-800 max-h-48 overflow-y-auto">
                      {summary.orphan_databases.map((o) => {
                        const checked = selectedOrphans.includes(o.id);
                        return (
                          <label
                            key={o.id}
                            className="flex items-center justify-between gap-3 px-3.5 py-2.5 text-xs hover:bg-dark-800/50 cursor-pointer"
                          >
                            <div className="flex items-center gap-2.5 min-w-0">
                              <input
                                type="checkbox"
                                checked={checked}
                                onChange={() =>
                                  setSelectedOrphans((prev) =>
                                    checked ? prev.filter((x) => x !== o.id) : [...prev, o.id]
                                  )
                                }
                                className="rounded border-dark-600 bg-dark-800 text-danger-500"
                              />
                              <div className="min-w-0">
                                <div className="font-mono text-dark-100 truncate">{o.name}</div>
                                <div className="text-[11px] text-dark-400 truncate">{o.reason}</div>
                              </div>
                            </div>
                            <div className="text-right font-mono shrink-0">
                              <span className="text-dark-200">{fmtBytes(o.size_bytes)}</span>
                              <span className="ml-2 px-1.5 py-0.5 rounded bg-dark-800 border border-dark-600 text-[10px] uppercase text-dark-300">
                                {o.engine}
                              </span>
                            </div>
                          </label>
                        );
                      })}
                    </div>
                  )}
                </div>
              </div>
            </>
          )}
        </div>

        <div className="px-5 py-3.5 border-t border-dark-700 bg-dark-850 flex items-center justify-end gap-2">
          <button
            type="button"
            onClick={onClose}
            className="px-3.5 py-1.5 rounded-lg border border-dark-600 text-xs text-dark-300 hover:text-dark-100"
          >
            Close
          </button>
          <button
            type="button"
            disabled={loading || running || (!optimizeAll && !pruneDumps && selectedOrphans.length === 0)}
            onClick={() => {
              if (selectedOrphans.length > 0) {
                setConfirmOrphanDelete(true);
              } else {
                executeCleanup();
              }
            }}
            className="px-4 py-1.5 rounded-lg bg-rust-500 hover:bg-rust-600 text-white text-xs font-medium disabled:opacity-50"
          >
            {running ? "Running Cleanup..." : "Run Cleanup & Optimize"}
          </button>
        </div>
      </div>

      {confirmOrphanDelete && (
        <ConfirmDialog
          title="Delete Selected Unlinked Databases"
          label={`Permanently delete ${selectedOrphans.length} selected orphaned database(s)?`}
          consequences={[
            "Selected unlinked database files or containers will be permanently removed from disk.",
            "This cannot be undone unless you have an exported dump.",
          ]}
          confirmLabel="Delete Orphans & Run Cleanup"
          tone="danger"
          onConfirm={() => {
            setConfirmOrphanDelete(false);
            executeCleanup();
          }}
          onCancel={() => setConfirmOrphanDelete(false)}
        />
      )}
    </div>
  );
}
