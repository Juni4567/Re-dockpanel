import { useState, useEffect, FormEvent } from "react";
import { api } from "../api";
import { isDesktopApp } from "../utils/desktop";

export interface PortConflictInfo {
  preferred_port: number;
  active_port: number;
  in_use: boolean;
  process_name: string | null;
  display_name: string | null;
  pid: number | null;
  alternatives: number[];
}

export interface PortConflictModalProps {
  isOpen: boolean;
  onClose: () => void;
  conflictInfo: PortConflictInfo | null;
  onResolved: (newPort: number) => void;
}

interface PortResolveResponse {
  success?: boolean;
  active_port?: number;
  message?: string;
  error?: string;
}

const DEFAULT_ALTERNATIVES = [8080, 8443, 3000];

export function isKillablePid(pid: number | null | undefined): boolean {
  return typeof pid === "number" && pid > 4;
}

export function buildSiteExternalUrl(
  domain: string,
  activePort?: number | null,
  sslEnabled?: boolean
): string {
  const protocol = sslEnabled ? "https" : "http";
  const port = activePort || (sslEnabled ? 443 : 80);
  const desktop = isDesktopApp();
  const portSuffix = desktop && port !== 80 && port !== 443 ? `:${port}` : "";
  return `${protocol}://${domain}${portSuffix}`;
}

export async function resolvePortConflict(
  payload:
    | { action: "use_port"; port: number }
    | { action: "stop_process"; pid: number; port: number }
): Promise<number> {
  const res = await api.post<PortResolveResponse>("/system/ports/resolve", payload);
  if (res && res.success === false) {
    throw new Error(
      res.message ||
        res.error ||
        (payload.action === "use_port"
          ? "Failed to switch router port."
          : "Failed to stop conflicting process.")
    );
  }
  return res?.active_port ?? payload.port;
}

export default function PortConflictModal({
  isOpen,
  onClose,
  conflictInfo,
  onResolved,
}: PortConflictModalProps) {
  const alternatives =
    conflictInfo?.alternatives && conflictInfo.alternatives.length > 0
      ? conflictInfo.alternatives
      : DEFAULT_ALTERNATIVES;

  const getDefaultPort = () => {
    if (conflictInfo?.active_port && alternatives.includes(conflictInfo.active_port)) {
      return conflictInfo.active_port;
    }
    return alternatives[0] ?? 8080;
  };

  const [action, setAction] = useState<"use_port" | "stop_process">("use_port");
  const [selectedPort, setSelectedPort] = useState<number>(getDefaultPort);
  const [submitting, setSubmitting] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (!isOpen) return;
    setAction("use_port");
    setError(null);
    const alts =
      conflictInfo?.alternatives && conflictInfo.alternatives.length > 0
        ? conflictInfo.alternatives
        : DEFAULT_ALTERNATIVES;
    if (conflictInfo?.active_port && alts.includes(conflictInfo.active_port)) {
      setSelectedPort(conflictInfo.active_port);
    } else {
      setSelectedPort(alts[0] ?? 8080);
    }
  }, [isOpen, conflictInfo]);

  if (!isOpen) return null;

  const preferredPort = conflictInfo?.preferred_port || 80;
  const processLabel =
    conflictInfo?.display_name || conflictInfo?.process_name || "another application";
  const pidLabel = conflictInfo?.pid || "Unknown";
  const killable = isKillablePid(conflictInfo?.pid);

  const handleSubmit = async (e: FormEvent) => {
    e.preventDefault();
    setError(null);
    setSubmitting(true);

    try {
      const newPort =
        action === "use_port"
          ? await resolvePortConflict({ action: "use_port", port: selectedPort })
          : await resolvePortConflict({
              action: "stop_process",
              pid: conflictInfo!.pid!,
              port: preferredPort,
            });
      onResolved(newPort);
      onClose();
    } catch (err) {
      setError(err instanceof Error ? err.message : "Failed to resolve port conflict.");
    } finally {
      setSubmitting(false);
    }
  };

  return (
    <div
      className="fixed inset-0 bg-black/70 backdrop-blur-sm z-50 flex items-center justify-center p-4"
      role="dialog"
      aria-modal="true"
      aria-labelledby="port-conflict-title"
    >
      <div className="bg-dark-900 border border-dark-700 rounded-xl max-w-lg w-full p-6 space-y-5 shadow-2xl">
        {/* Header */}
        <div className="flex items-center justify-between border-b border-dark-700/60 pb-4">
          <div>
            <h2 id="port-conflict-title" className="text-lg font-semibold text-dark-100">
              Site Router Port Settings
            </h2>
            <p className="text-xs text-dark-400 mt-0.5">
              Choose how DockPanel routes local sites on your machine
            </p>
          </div>
          <button
            type="button"
            onClick={onClose}
            disabled={submitting}
            aria-label="Close modal"
            className="text-dark-400 hover:text-dark-100 p-1.5 rounded-lg hover:bg-dark-800 transition-colors disabled:opacity-50"
          >
            <svg className="w-5 h-5" fill="none" viewBox="0 0 24 24" stroke="currentColor">
              <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M6 18L18 6M6 6l12 12" />
            </svg>
          </button>
        </div>

        {/* Warning Badge */}
        <div className="p-3.5 bg-amber-500/10 border border-amber-500/20 rounded-lg flex items-start gap-3">
          <span className="text-amber-400 text-base leading-none mt-0.5" aria-hidden="true">
            ⚠️
          </span>
          <p className="text-xs text-amber-200 leading-relaxed">
            Port {preferredPort} is currently in use by {processLabel} (PID {pidLabel}).
          </p>
        </div>

        <form onSubmit={handleSubmit} className="space-y-4">
          {/* Option 1: Alternative port */}
          <label
            className={`block p-4 rounded-lg border cursor-pointer transition-colors ${
              action === "use_port"
                ? "bg-dark-800 border-amber-500/50"
                : "bg-dark-800/50 border-dark-700 hover:border-dark-600"
            }`}
          >
            <div className="flex items-start gap-3">
              <input
                type="radio"
                name="port-resolution"
                value="use_port"
                checked={action === "use_port"}
                onChange={() => setAction("use_port")}
                className="mt-1 text-amber-500 focus:ring-amber-500 bg-dark-900 border-dark-600"
              />
              <div className="flex-1 space-y-2.5">
                <div>
                  <span className="text-sm font-medium text-dark-100 block">
                    Run site router on alternative port
                  </span>
                  <span className="text-xs text-dark-400">
                    Sites will be served at <code className="font-mono text-dark-200">http://&lt;domain&gt;:{selectedPort}</code> without disrupting other apps.
                  </span>
                </div>
                <div>
                  <label htmlFor="alternative-port-select" className="sr-only">
                    Alternative Port
                  </label>
                  <select
                    id="alternative-port-select"
                    value={selectedPort}
                    onChange={(e) => {
                      setAction("use_port");
                      setSelectedPort(Number(e.target.value));
                    }}
                    className="w-full px-3 py-2 bg-dark-900 border border-dark-600 rounded-lg text-sm text-dark-100 font-mono focus:outline-none focus:border-amber-500"
                  >
                    {alternatives.map((port) => (
                      <option key={port} value={port}>
                        Port {port}
                      </option>
                    ))}
                  </select>
                </div>
              </div>
            </div>
          </label>

          {/* Option 2: Stop conflicting process */}
          {killable && (
            <label
              className={`block p-4 rounded-lg border cursor-pointer transition-colors ${
                action === "stop_process"
                  ? "bg-dark-800 border-amber-500/50"
                  : "bg-dark-800/50 border-dark-700 hover:border-dark-600"
              }`}
            >
              <div className="flex items-start gap-3">
                <input
                  type="radio"
                  name="port-resolution"
                  value="stop_process"
                  checked={action === "stop_process"}
                  onChange={() => setAction("stop_process")}
                  className="mt-1 text-amber-500 focus:ring-amber-500 bg-dark-900 border-dark-600"
                />
                <div className="flex-1">
                  <span className="text-sm font-medium text-dark-100 block">
                    Stop conflicting process ({conflictInfo?.process_name || processLabel})
                  </span>
                  <span className="text-xs text-dark-400 mt-1 block">
                    Terminates PID {conflictInfo?.pid} and binds DockPanel to port {preferredPort}. This will interrupt {processLabel} and may require Administrator privileges if it runs as a Windows service.
                  </span>
                </div>
              </div>
            </label>
          )}

          {error && (
            <div
              role="alert"
              className="p-3 bg-red-950/40 border border-red-800/50 rounded-lg text-xs text-red-300"
            >
              {error}
            </div>
          )}

          {/* Footer Actions */}
          <div className="flex items-center justify-end gap-3 pt-3 border-t border-dark-700/60">
            <button
              type="button"
              onClick={onClose}
              disabled={submitting}
              className="px-4 py-2 text-xs font-medium text-dark-300 hover:text-dark-100 border border-dark-600 rounded-lg hover:border-dark-500 transition-colors disabled:opacity-50"
            >
              Cancel
            </button>
            <button
              type="submit"
              disabled={submitting}
              className="px-4 py-2 bg-rust-500 hover:bg-rust-600 text-white text-xs font-medium rounded-lg shadow-sm transition-colors disabled:opacity-50 inline-flex items-center gap-1.5"
            >
              {submitting && (
                <svg className="animate-spin w-3.5 h-3.5 text-white" fill="none" viewBox="0 0 24 24">
                  <circle className="opacity-25" cx="12" cy="12" r="10" stroke="currentColor" strokeWidth="4" />
                  <path className="opacity-75" fill="currentColor" d="M4 12a8 8 0 018-8V0C5.373 0 0 5.373 0 12h4z" />
                </svg>
              )}
              Save &amp; Apply
            </button>
          </div>
        </form>
      </div>
    </div>
  );
}
