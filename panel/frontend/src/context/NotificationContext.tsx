import {
  createContext,
  useContext,
  useState,
  useEffect,
  ReactNode,
  useCallback,
  useRef,
} from "react";
import { ApiError } from "../api";

export type NotificationType = "success" | "error" | "warn" | "info";

export interface NotificationMeta {
  source?: "live-sse" | "action" | "system" | string;
  category?: string;
  target?: string;
  endpoint?: string;
  method?: string;
  statusCode?: number;
  durationMs?: number;
  code?: string;
  dedupeKey?: string;
  autoDismissMs?: number;
  live?: boolean;
  timestamp?: string;
  confirmedByServer?: boolean;
}

export interface GlobalNotification {
  id: string;
  type: NotificationType;
  message: string;
  title?: string;
  details?: string;
  meta?: NotificationMeta;
  createdAt: number;
  updatedAt?: number;
  autoDismissMs: number;
  pinned?: boolean;
}

interface NotificationContextValue {
  notification: GlobalNotification | null;
  notifications: GlobalNotification[];
  notify: {
    success: (message: string, title?: string, details?: string, meta?: NotificationMeta) => void;
    error: (message: string, title?: string, details?: string, meta?: NotificationMeta) => void;
    warn: (message: string, title?: string, details?: string, meta?: NotificationMeta) => void;
    info: (message: string, title?: string, details?: string, meta?: NotificationMeta) => void;
    dismiss: (id: string) => void;
    pin: (id: string, pinned?: boolean) => void;
    clear: () => void;
  };
}

const NotificationContext = createContext<NotificationContextValue | null>(null);

const MAX_ACTIVE_ALERTS = 4;

export function defaultAutoDismissMs(type: NotificationType, meta?: NotificationMeta): number {
  if (meta?.autoDismissMs !== undefined) return meta.autoDismissMs;
  if (type === "error") return 0; // Errors remain pinned until user dismisses
  if (type === "warn") return 12000;
  return 8000;
}

/**
 * Extract structured message + technical diagnostic details from ApiError / Error.
 */
export function buildAlertDetails(
  err: unknown,
  extra?: Partial<NotificationMeta> & { context?: string; fallback?: string }
): { message: string; details?: string; meta: NotificationMeta } {
  const meta: NotificationMeta = {
    source: "action",
    timestamp: new Date().toISOString(),
    ...extra,
  };

  if (err instanceof ApiError || (err && typeof err === "object" && "status" in err && err instanceof Error)) {
    const apiErr = err as ApiError;
    meta.statusCode = apiErr.status || meta.statusCode;
    meta.code = apiErr.code || meta.code;
    meta.method = apiErr.method || meta.method;
    meta.endpoint = apiErr.endpoint || meta.endpoint;
    meta.durationMs = apiErr.durationMs ?? meta.durationMs;
    if (apiErr.timestamp) meta.timestamp = apiErr.timestamp;

    const lines: string[] = [];
    if (extra?.context) lines.push(`Operation: ${extra.context}`);
    if (meta.target) lines.push(`Target:    ${meta.target}`);
    if (meta.endpoint) {
      lines.push(
        `Request:   ${meta.method || "API"} /api${meta.endpoint}${
          meta.durationMs ? ` (${meta.durationMs}ms)` : ""
        }`
      );
    }
    if (meta.statusCode !== undefined) {
      lines.push(`HTTP Code: ${meta.statusCode}${meta.code ? ` [${meta.code}]` : ""}`);
    }
    lines.push(`Timestamp: ${meta.timestamp}`);
    if (apiErr.body && Object.keys(apiErr.body).length > 0) {
      try {
        lines.push(`Response:\n${JSON.stringify(apiErr.body, null, 2)}`);
      } catch {
        // Ignore serialization issues
      }
    }
    return {
      message: apiErr.message || extra?.fallback || "Request failed",
      details: lines.length > 0 ? lines.join("\n") : undefined,
      meta,
    };
  }

  if (err instanceof Error) {
    const lines: string[] = [];
    if (extra?.context) lines.push(`Operation: ${extra.context}`);
    if (meta.target) lines.push(`Target:    ${meta.target}`);
    lines.push(`Timestamp: ${meta.timestamp}`);
    lines.push(`Error:     ${err.name}: ${err.message}`);
    if (err.stack) {
      const topFrames = err.stack.split("\n").slice(1, 4).join("\n").trim();
      if (topFrames) lines.push(`Trace:\n${topFrames}`);
    }
    return {
      message: err.message || extra?.fallback || "Unexpected error",
      details: lines.join("\n"),
      meta,
    };
  }

  return {
    message: typeof err === "string" && err.trim() ? err : extra?.fallback || "Unexpected error",
    details: extra?.context ? `Operation: ${extra.context}\nTimestamp: ${meta.timestamp}` : undefined,
    meta,
  };
}

/**
 * Dispatch a notification from anywhere in the application (even outside React).
 */
export function showGlobalNotification(
  type: NotificationType,
  message: string,
  title?: string,
  details?: string,
  meta?: NotificationMeta
) {
  if (typeof window === "undefined") return;
  window.dispatchEvent(
    new CustomEvent("dp:global-notify", {
      detail: { type, message, title, details, meta },
    })
  );
}

function normalizeMatchText(s?: string): string {
  return (s || "").toLowerCase().replace(/[^a-z0-9]+/g, " ").trim();
}

function formatClockTime(ms: number): string {
  try {
    return new Date(ms).toLocaleTimeString([], {
      hour12: false,
      hour: "2-digit",
      minute: "2-digit",
      second: "2-digit",
    });
  } catch {
    return "";
  }
}

function formatRelativeAge(createdAt: number, nowMs: number): string {
  const diffSec = Math.max(0, Math.floor((nowMs - createdAt) / 1000));
  if (diffSec < 3) return "just now";
  if (diffSec < 60) return `${diffSec}s ago`;
  const mins = Math.floor(diffSec / 60);
  if (mins < 60) return `${mins}m ago`;
  return `${Math.floor(mins / 60)}h ago`;
}

export function NotificationProvider({ children }: { children: ReactNode }) {
  const [notifications, setNotifications] = useState<GlobalNotification[]>([]);

  const pushNotification = useCallback(
    (
      type: NotificationType,
      message: string,
      title?: string,
      details?: string,
      meta?: NotificationMeta
    ) => {
      const now = Date.now();
      const autoDismissMs = defaultAutoDismissMs(type, meta);

      setNotifications((prev) => {
        // 1. Exact dedupeKey match inside recent window (prevents SSE replay duplicates)
        if (meta?.dedupeKey) {
          const existingIdx = prev.findIndex((n) => n.meta?.dedupeKey === meta.dedupeKey);
          if (existingIdx >= 0) {
            const copy = [...prev];
            copy[existingIdx] = {
              ...copy[existingIdx],
              type,
              message,
              title: title || copy[existingIdx].title,
              details: details || copy[existingIdx].details,
              meta: { ...copy[existingIdx].meta, ...meta },
              updatedAt: now,
            };
            return copy;
          }
        }

        // 2. Real-time SSE event enrichment: if a manual toast for the same target/action
        // fired within the last 3.5 seconds, enrich it with server confirmation & backend details
        // rather than popping a confusing duplicate card.
        if (meta?.live) {
          const incomingTarget = normalizeMatchText(meta.target);
          const incomingMsg = normalizeMatchText(message);
          const matchIdx = prev.findIndex((n) => {
            if (now - n.createdAt > 3500) return false;
            if (n.type !== type && !(n.type === "info" && type === "success")) return false;
            const prevTarget = normalizeMatchText(n.meta?.target);
            if (incomingTarget && prevTarget && incomingTarget === prevTarget) return true;
            if (incomingTarget && normalizeMatchText(n.message).includes(incomingTarget)) return true;
            if (incomingMsg && normalizeMatchText(n.message) === incomingMsg) return true;
            return false;
          });

          if (matchIdx >= 0) {
            const existing = prev[matchIdx];
            const mergedDetails =
              details && existing.details && details !== existing.details
                ? `${existing.details}\n\n── Server Event Stream ──\n${details}`
                : details || existing.details;
            const copy = [...prev];
            copy[matchIdx] = {
              ...existing,
              type: existing.type === "info" && type === "success" ? "success" : existing.type,
              title: existing.title || title,
              details: mergedDetails,
              updatedAt: now,
              meta: {
                ...existing.meta,
                category: meta.category || existing.meta?.category,
                target: meta.target || existing.meta?.target,
                confirmedByServer: true,
                live: true,
              },
            };
            return copy;
          }
        }

        const next: GlobalNotification = {
          id: `${now.toString(36)}-${Math.random().toString(36).slice(2, 7)}`,
          type,
          message,
          title,
          details,
          meta: {
            timestamp: new Date(now).toISOString(),
            ...meta,
          },
          createdAt: now,
          autoDismissMs,
          pinned: autoDismissMs === 0,
        };

        return [next, ...prev].slice(0, MAX_ACTIVE_ALERTS);
      });
    },
    []
  );

  // Handle custom window events
  useEffect(() => {
    const handler = (e: Event) => {
      const custom = e as CustomEvent<{
        type: NotificationType;
        message: string;
        title?: string;
        details?: string;
        meta?: NotificationMeta;
      }>;
      if (custom.detail) {
        pushNotification(
          custom.detail.type,
          custom.detail.message,
          custom.detail.title,
          custom.detail.details,
          custom.detail.meta
        );
      }
    };
    window.addEventListener("dp:global-notify", handler);
    return () => window.removeEventListener("dp:global-notify", handler);
  }, [pushNotification]);

  const dismiss = useCallback((id: string) => {
    setNotifications((prev) => prev.filter((n) => n.id !== id));
  }, []);

  const pin = useCallback((id: string, pinned = true) => {
    setNotifications((prev) =>
      prev.map((n) => (n.id === id ? { ...n, pinned } : n))
    );
  }, []);

  const clear = useCallback(() => {
    setNotifications([]);
  }, []);

  const notify = {
    success: useCallback(
      (message: string, title?: string, details?: string, meta?: NotificationMeta) =>
        pushNotification("success", message, title, details, meta),
      [pushNotification]
    ),
    error: useCallback(
      (message: string, title?: string, details?: string, meta?: NotificationMeta) =>
        pushNotification("error", message, title, details, meta),
      [pushNotification]
    ),
    warn: useCallback(
      (message: string, title?: string, details?: string, meta?: NotificationMeta) =>
        pushNotification("warn", message, title, details, meta),
      [pushNotification]
    ),
    info: useCallback(
      (message: string, title?: string, details?: string, meta?: NotificationMeta) =>
        pushNotification("info", message, title, details, meta),
      [pushNotification]
    ),
    dismiss,
    pin,
    clear,
  };

  return (
    <NotificationContext.Provider
      value={{
        notification: notifications[0] ?? null,
        notifications,
        notify,
      }}
    >
      {children}
    </NotificationContext.Provider>
  );
}

export function useNotify() {
  const ctx = useContext(NotificationContext);
  if (!ctx) {
    return {
      notification: null,
      notifications: [] as GlobalNotification[],
      notify: {
        success: (msg: string, title?: string, details?: string, meta?: NotificationMeta) =>
          showGlobalNotification("success", msg, title, details, meta),
        error: (msg: string, title?: string, details?: string, meta?: NotificationMeta) =>
          showGlobalNotification("error", msg, title, details, meta),
        warn: (msg: string, title?: string, details?: string, meta?: NotificationMeta) =>
          showGlobalNotification("warn", msg, title, details, meta),
        info: (msg: string, title?: string, details?: string, meta?: NotificationMeta) =>
          showGlobalNotification("info", msg, title, details, meta),
        dismiss: () => {},
        pin: () => {},
        clear: () => {},
      },
    };
  }
  return ctx;
}

function getTypeStyle(type: NotificationType) {
  switch (type) {
    case "success":
      return {
        border: "border-l-rust-400 border-rust-500/35",
        badge: "bg-rust-500/20 text-rust-300 border-rust-500/40",
        bar: "bg-rust-400",
        iconColor: "text-rust-400",
        dot: "bg-rust-400",
        defaultTitle: "Action Successful",
        icon: (
          <svg className="w-5 h-5" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth="2.2">
            <path strokeLinecap="round" strokeLinejoin="round" d="M9 12.75 11.25 15 15 9.75M21 12a9 9 0 1 1-18 0 9 9 0 0 1 18 0Z" />
          </svg>
        ),
      };
    case "error":
      return {
        border: "border-l-danger-400 border-danger-500/45",
        badge: "bg-danger-500/20 text-danger-300 border-danger-500/40",
        bar: "bg-danger-400",
        iconColor: "text-danger-400",
        dot: "bg-danger-400",
        defaultTitle: "Action Failed",
        icon: (
          <svg className="w-5 h-5" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth="2.2">
            <path strokeLinecap="round" strokeLinejoin="round" d="M12 9v3.75m9-.75a9 9 0 1 1-18 0 9 9 0 0 1 18 0Zm-9 3.75h.008v.008H12v-.008Z" />
          </svg>
        ),
      };
    case "warn":
      return {
        border: "border-l-warn-400 border-warn-500/40",
        badge: "bg-warn-500/20 text-warn-400 border-warn-500/40",
        bar: "bg-warn-400",
        iconColor: "text-warn-400",
        dot: "bg-warn-400",
        defaultTitle: "Warning",
        icon: (
          <svg className="w-5 h-5" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth="2.2">
            <path strokeLinecap="round" strokeLinejoin="round" d="M12 9v3.75m-9.303 3.376c-.866 1.5.217 3.374 1.948 3.374h14.71c1.73 0 2.813-1.874 1.948-3.374L13.949 3.378c-.866-1.5-3.032-1.5-3.898 0L2.697 16.126ZM12 15.75h.007v.008H12v-.008Z" />
          </svg>
        ),
      };
    case "info":
    default:
      return {
        border: "border-l-accent-400 border-accent-500/40",
        badge: "bg-accent-500/20 text-accent-300 border-accent-500/40",
        bar: "bg-accent-400",
        iconColor: "text-accent-400",
        dot: "bg-accent-400",
        defaultTitle: "Live Notice",
        icon: (
          <svg className="w-5 h-5" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth="2.2">
            <path strokeLinecap="round" strokeLinejoin="round" d="m11.25 11.25.041-.02a.75.75 0 0 1 1.063.852l-.708 2.836a.75.75 0 0 0 1.063.853l.041-.021M21 12a9 9 0 1 1-18 0 9 9 0 0 1 18 0Zm-9-3.75h.008v.008H12V8.25Z" />
          </svg>
        ),
      };
  }
}

function NotificationCard({
  item,
  nowMs,
  onDismiss,
  onTogglePin,
}: {
  item: GlobalNotification;
  nowMs: number;
  onDismiss: (id: string) => void;
  onTogglePin: (id: string, pinned: boolean) => void;
}) {
  const [showDetails, setShowDetails] = useState(item.type === "error" && Boolean(item.details));
  const [hovered, setHovered] = useState(false);
  const [copied, setCopied] = useState(false);
  const [remainingMs, setRemainingMs] = useState<number>(item.autoDismissMs);
  const lastTickRef = useRef<number>(Date.now());

  const isSticky = Boolean(item.pinned || showDetails || item.autoDismissMs <= 0);
  const styles = getTypeStyle(item.type);

  // Reset countdown when card is updated in-place by SSE confirmation
  useEffect(() => {
    setRemainingMs(item.autoDismissMs);
    lastTickRef.current = Date.now();
  }, [item.updatedAt, item.autoDismissMs]);

  useEffect(() => {
    if (isSticky || hovered) {
      lastTickRef.current = Date.now();
      return;
    }

    lastTickRef.current = Date.now();
    const timer = setInterval(() => {
      const now = Date.now();
      const delta = now - lastTickRef.current;
      lastTickRef.current = now;
      setRemainingMs((prev) => {
        const next = prev - delta;
        if (next <= 0) {
          clearInterval(timer);
          onDismiss(item.id);
          return 0;
        }
        return next;
      });
    }, 100);

    return () => clearInterval(timer);
  }, [isSticky, hovered, item.id, onDismiss]);

  const progressPct =
    item.autoDismissMs > 0
      ? Math.max(0, Math.min(100, (remainingMs / item.autoDismissMs) * 100))
      : 100;

  const diagnosticDump = (() => {
    const metaLines: string[] = [];
    metaLines.push(`Title:     ${item.title || styles.defaultTitle}`);
    metaLines.push(`Status:    ${item.type.toUpperCase()}`);
    metaLines.push(`Message:   ${item.message}`);
    metaLines.push(`Time:      ${item.meta?.timestamp || new Date(item.createdAt).toISOString()}`);
    if (item.meta?.target) metaLines.push(`Target:    ${item.meta.target}`);
    if (item.meta?.endpoint) {
      metaLines.push(
        `Endpoint:  ${item.meta.method || "API"} /api${item.meta.endpoint}${
          item.meta.statusCode !== undefined ? ` -> HTTP ${item.meta.statusCode}` : ""
        }${item.meta.durationMs !== undefined ? ` (${item.meta.durationMs}ms)` : ""}`
      );
    } else if (item.meta?.statusCode !== undefined) {
      metaLines.push(`HTTP Code: ${item.meta.statusCode}`);
    }
    if (item.meta?.confirmedByServer) {
      metaLines.push(`Stream:    Confirmed via real-time SSE event`);
    } else if (item.meta?.live) {
      metaLines.push(`Stream:    Real-time SSE event`);
    }
    if (item.details) {
      metaLines.push("");
      metaLines.push(item.details);
    }
    return metaLines.join("\n");
  })();

  const hasRichDetails = Boolean(
    item.details ||
      item.meta?.endpoint ||
      item.meta?.statusCode !== undefined ||
      item.meta?.durationMs !== undefined
  );

  const handleCopy = async () => {
    try {
      await navigator.clipboard.writeText(diagnosticDump);
      setCopied(true);
      setTimeout(() => setCopied(false), 1800);
    } catch {
      // Clipboard API may be unavailable in headless tests
    }
  };

  const ageLabel = formatRelativeAge(item.updatedAt || item.createdAt, nowMs);
  const clockLabel = formatClockTime(item.updatedAt || item.createdAt);
  const isFresh = nowMs - (item.updatedAt || item.createdAt) < 4000;

  return (
    <div
      role={item.type === "error" ? "alert" : "status"}
      onMouseEnter={() => setHovered(true)}
      onMouseLeave={() => setHovered(false)}
      className={`pointer-events-auto relative overflow-hidden rounded-xl border border-l-4 bg-dark-800/95 text-dark-100 backdrop-blur-md shadow-2xl transition-all animate-alert-in ${styles.border}`}
    >
      <div className="p-3.5 pr-3">
        <div className="flex items-start justify-between gap-2.5">
          <div className="flex items-start gap-2.5 min-w-0 flex-1">
            <div className={`mt-0.5 shrink-0 ${styles.iconColor}`}>{styles.icon}</div>

            <div className="min-w-0 flex-1">
              <div className="flex items-center justify-between gap-2 flex-wrap">
                <div className="flex items-center gap-1.5 flex-wrap">
                  <span
                    className={`inline-flex items-center gap-1.5 text-[10px] uppercase font-bold tracking-wider px-2 py-0.5 rounded-full border ${styles.badge}`}
                  >
                    {isFresh && (
                      <span className={`w-1.5 h-1.5 rounded-full animate-ping ${styles.dot}`} />
                    )}
                    {item.title || styles.defaultTitle}
                  </span>

                  {item.meta?.live && (
                    <span className="inline-flex items-center gap-1 text-[10px] font-mono uppercase px-1.5 py-0.5 rounded bg-emerald-500/15 text-emerald-300 border border-emerald-500/30">
                      <span className="w-1.5 h-1.5 rounded-full bg-emerald-400 animate-pulse" />
                      {item.meta.confirmedByServer ? "Verified" : "Live"}
                    </span>
                  )}

                  {item.meta?.category && (
                    <span className="text-[10px] font-mono uppercase px-1.5 py-0.5 rounded bg-dark-700/80 text-dark-300 border border-dark-600/60">
                      {item.meta.category}
                    </span>
                  )}
                </div>

                <span
                  className="text-[10px] font-mono text-dark-400 shrink-0 tabular-nums"
                  title={item.meta?.timestamp}
                >
                  {clockLabel} · {ageLabel}
                </span>
              </div>

              <p className="text-[13px] leading-snug font-medium text-dark-50 mt-1.5 break-words">
                {item.message}
              </p>

              {/* Inline telemetry / metadata chips */}
              {(item.meta?.target ||
                item.meta?.endpoint ||
                item.meta?.statusCode !== undefined ||
                item.meta?.durationMs !== undefined) && (
                <div className="mt-2 flex items-center gap-1.5 flex-wrap text-[11px] font-mono">
                  {item.meta?.target && (
                    <span className="px-1.5 py-0.5 rounded bg-dark-900/80 border border-dark-700 text-dark-200 truncate max-w-[200px]">
                      {item.meta.target}
                    </span>
                  )}
                  {item.meta?.statusCode !== undefined && (
                    <span
                      className={`px-1.5 py-0.5 rounded border ${
                        item.meta.statusCode >= 400
                          ? "bg-danger-500/15 border-danger-500/30 text-danger-300"
                          : "bg-rust-500/15 border-rust-500/30 text-rust-300"
                      }`}
                    >
                      HTTP {item.meta.statusCode}
                    </span>
                  )}
                  {item.meta?.durationMs !== undefined && (
                    <span className="px-1.5 py-0.5 rounded bg-dark-900/80 border border-dark-700 text-dark-300 tabular-nums">
                      {item.meta.durationMs}ms
                    </span>
                  )}
                  {item.meta?.endpoint && (
                    <span className="px-1.5 py-0.5 rounded bg-dark-900/80 border border-dark-700 text-dark-300 truncate max-w-[240px]">
                      {item.meta.method ? `${item.meta.method} ` : ""}
                      {item.meta.endpoint}
                    </span>
                  )}
                </div>
              )}

              {/* Actions bar: Details toggle, Copy, Pin */}
              <div className="mt-2 flex items-center gap-3 flex-wrap">
                {hasRichDetails && (
                  <button
                    type="button"
                    onClick={() => setShowDetails((v) => !v)}
                    className="text-[11px] font-mono text-dark-300 hover:text-dark-50 underline underline-offset-2 transition-colors"
                  >
                    {showDetails ? "Hide Details" : "Inspect Details"}
                  </button>
                )}

                {hasRichDetails && (
                  <button
                    type="button"
                    onClick={handleCopy}
                    className="text-[11px] font-mono text-dark-400 hover:text-dark-100 transition-colors"
                  >
                    {copied ? "Copied ✓" : "Copy Log"}
                  </button>
                )}

                {item.autoDismissMs > 0 && (
                  <button
                    type="button"
                    onClick={() => onTogglePin(item.id, !item.pinned)}
                    className="text-[11px] font-mono text-dark-400 hover:text-dark-100 transition-colors"
                  >
                    {item.pinned ? "Unpin" : hovered ? "Paused" : "Keep Open"}
                  </button>
                )}
              </div>

              {showDetails && hasRichDetails && (
                <pre className="mt-2 p-2.5 bg-dark-950/90 text-dark-200 border border-dark-700/80 rounded-lg text-[11px] leading-relaxed font-mono overflow-x-auto max-h-48 whitespace-pre-wrap break-words select-text">
                  {item.details
                    ? item.meta?.endpoint && !item.details.includes("Request:")
                      ? `${item.details}\n\n── Request Telemetry ──\n${item.meta.method || "API"} /api${item.meta.endpoint}${item.meta.statusCode !== undefined ? ` -> HTTP ${item.meta.statusCode}` : ""}${item.meta.durationMs !== undefined ? ` (${item.meta.durationMs}ms)` : ""}`
                      : item.details
                    : diagnosticDump}
                </pre>
              )}
            </div>
          </div>

          <button
            type="button"
            onClick={() => onDismiss(item.id)}
            className="p-1 rounded-lg text-dark-400 hover:text-dark-50 hover:bg-dark-700/60 transition-colors shrink-0"
            title="Dismiss notification"
            aria-label="Dismiss notification"
          >
            <svg className="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth="2.2">
              <path strokeLinecap="round" strokeLinejoin="round" d="M6 18 18 6M6 6l12 12" />
            </svg>
          </button>
        </div>
      </div>

      {/* Live auto-dismiss countdown progress strip */}
      {!isSticky && item.autoDismissMs > 0 && (
        <div className="h-0.5 w-full bg-dark-700/60">
          <div
            className={`h-full transition-[width] duration-100 ease-linear ${styles.bar}`}
            style={{ width: `${progressPct}%` }}
          />
        </div>
      )}
    </div>
  );
}

/**
 * Live floating real-time alert stack rendered across page layouts.
 */
export function GlobalNotificationBanner() {
  const { notifications, notify } = useNotify();
  const [nowMs, setNowMs] = useState<number>(() => Date.now());

  useEffect(() => {
    if (notifications.length === 0) return;
    setNowMs(Date.now());
    const id = setInterval(() => setNowMs(Date.now()), 1000);
    return () => clearInterval(id);
  }, [notifications.length]);

  if (notifications.length === 0) return null;

  return (
    <div
      aria-live="polite"
      className="fixed bottom-5 right-5 z-[60] w-[min(94vw,430px)] flex flex-col gap-2.5 pointer-events-none"
    >
      {notifications.length > 1 && (
        <div className="pointer-events-auto flex items-center justify-between px-3 py-1.5 rounded-lg bg-dark-900/90 border border-dark-700/80 backdrop-blur-md text-[11px] font-mono text-dark-300 shadow-lg">
          <span className="inline-flex items-center gap-1.5">
            <span className="w-2 h-2 rounded-full bg-rust-400 animate-pulse" />
            {notifications.length} active alerts
          </span>
          <button
            type="button"
            onClick={() => notify.clear()}
            className="text-dark-200 hover:text-dark-50 underline underline-offset-2"
          >
            Dismiss All
          </button>
        </div>
      )}

      {notifications.map((item) => (
        <NotificationCard
          key={item.id}
          item={item}
          nowMs={nowMs}
          onDismiss={notify.dismiss}
          onTogglePin={notify.pin}
        />
      ))}
    </div>
  );
}
