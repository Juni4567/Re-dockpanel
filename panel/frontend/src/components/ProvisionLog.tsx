import { useState, useEffect, useRef } from "react";
import { showGlobalNotification } from "../context/NotificationContext";

export interface ProvisionStep {
  step: string;
  label: string;
  status: "pending" | "in_progress" | "done" | "error";
  progress?: number;
  message?: string;
  requires_compat_confirm?: boolean;
  compat_fixes?: string[];
  target_version?: string;
  error_detail?: string;
}

export interface ProvisionCompatMeta {
  requiresCompatConfirm?: boolean;
  compatFixes?: string[];
  targetVersion?: string;
  errorDetail?: string;
}

interface Props {
  siteId?: string;
  sseUrl?: string;
  title?: string;
  autoClose?: boolean;
  defaultShowLogs?: boolean;
  externalSteps?: ProvisionStep[];
  onComplete?: (ok?: boolean, errorDetail?: string, compatMeta?: ProvisionCompatMeta) => void;
  onDismiss?: () => void;
}

export default function ProvisionLog({
  siteId,
  sseUrl,
  title,
  autoClose = false,
  defaultShowLogs = false,
  externalSteps,
  onComplete,
  onDismiss,
}: Props) {
  const [steps, setSteps] = useState<ProvisionStep[]>([]);
  const [done, setDone] = useState(false);
  const [severed, setSevered] = useState(false);
  const [showLogs, setShowLogs] = useState(defaultShowLogs);
  const [copied, setCopied] = useState(false);
  const receivedRef = useRef(0);
  const logContainerRef = useRef<HTMLPreElement | null>(null);

  const isDeploy = !!(title?.toLowerCase().includes("deploy") || sseUrl?.includes("install") || sseUrl?.includes("deploy"));
  const defaultTitle = isDeploy ? "Deploying..." : "Provisioning...";

  const url = sseUrl || (siteId ? `/api/sites/${siteId}/provision-log` : "");

  useEffect(() => {
    if (defaultShowLogs) {
      setShowLogs(true);
    }
  }, [defaultShowLogs, url]);

  useEffect(() => {
    if (!url) return;

    receivedRef.current = 0;
    setSteps([]);
    setDone(false);
    setSevered(false);

    const es = new EventSource(url);

    es.onmessage = (event) => {
      try {
        const step: ProvisionStep = JSON.parse(event.data);
        receivedRef.current += 1;
        setSteps((prev) => {
          const idx = prev.findIndex((s) => s.step === step.step);
          if (idx >= 0) {
            const next = [...prev];
            next[idx] = step;
            return next;
          }
          return [...prev, step];
        });

        // Auto-expand logs if an error occurred or long output received
        if (step.status === "error" || (step.message && step.message.length > 200)) {
          setShowLogs(true);
        }

        if (step.step === "complete") {
          es.close();
          setDone(true);
          const ok = step.status !== "error";
          const errDetail = !ok ? (step.error_detail || step.message || step.label || "Operation failed") : undefined;
          const compatMeta: ProvisionCompatMeta | undefined = step.requires_compat_confirm
            ? {
                requiresCompatConfirm: true,
                compatFixes: step.compat_fixes || [],
                targetVersion: step.target_version,
                errorDetail: step.error_detail || errDetail,
              }
            : undefined;
          showGlobalNotification(
            ok ? "success" : "error",
            step.label || (ok ? (isDeploy ? "Deployment completed successfully" : "Provisioning complete") : (isDeploy ? "Deployment failed" : "Provisioning failed")),
            isDeploy ? (ok ? "Deploy Success" : "Deploy Failed") : undefined,
            step.message || undefined
          );

          if (autoClose) {
            setTimeout(() => onComplete?.(ok, errDetail, compatMeta), 3000);
          } else {
            // Signal completion to parent for background data refresh
            onComplete?.(ok, errDetail, compatMeta);
          }
        }
      } catch {
        // ignore parse errors
      }
    };

    es.onerror = () => {
      es.close();
      if (receivedRef.current > 0) return;
      setDone(true);
      setSevered(true);
      showGlobalNotification(
        "warn",
        "The progress stream closed before reporting status. Check the history or site status for current state."
      );
      if (autoClose) {
        setTimeout(() => onComplete?.(false, "The progress stream closed unexpectedly."), 4000);
      } else {
        onComplete?.(false, "The progress stream closed unexpectedly.");
      }
    };

    return () => es.close();
  }, [url]);

  const mergedSteps = (() => {
    if (!externalSteps || externalSteps.length === 0) return steps;
    const out = [...externalSteps];
    for (const s of steps) {
      const idx = out.findIndex((x) => x.step === s.step);
      if (idx >= 0) out[idx] = s;
      else out.push(s);
    }
    return out;
  })();

  // Auto-scroll logs to bottom as they update
  useEffect(() => {
    if (showLogs && logContainerRef.current) {
      logContainerRef.current.scrollTop = logContainerRef.current.scrollHeight;
    }
  }, [mergedSteps, showLogs]);

  const visibleSteps = mergedSteps.filter((s) => s.step !== "complete");
  const completeStep = mergedSteps.find((s) => s.step === "complete");

  const hasError = completeStep?.status === "error" || mergedSteps.some((s) => s.status === "error");
  const effectiveDone = done || Boolean(completeStep);
  const isComplete = effectiveDone && !hasError && !severed;

  const headline = !effectiveDone
    ? (title || defaultTitle)
    : severed
    ? "Progress unavailable"
    : completeStep?.label ?? (isComplete ? (isDeploy ? "Deployment complete" : "Provisioning complete") : (isDeploy ? "Deployment failed" : "Provisioning failed"));

  const overallPercent = (() => {
    if (isComplete) return 100;
    if (visibleSteps.length === 0) return 5;
    const total = Math.max(visibleSteps.length, 4);
    let score = 0;
    for (const s of visibleSteps) {
      if (s.status === "done") score += 1;
      else if (s.status === "in_progress") {
        score += typeof s.progress === "number" ? Math.min(99, Math.max(5, s.progress)) / 100 : 0.4;
      }
    }
    return Math.min(98, Math.max(5, Math.round((score / total) * 100)));
  })();

  // Collect all step messages into a combined execution log
  const allLogs = mergedSteps
    .filter((s) => s.message && s.message.trim().length > 0)
    .map((s) => `[${s.step.toUpperCase()}] ${s.label}\n${s.message}`)
    .join("\n\n");

  const handleCopyLogs = () => {
    if (!allLogs) return;
    navigator.clipboard.writeText(allLogs);
    setCopied(true);
    setTimeout(() => setCopied(false), 2000);
  };

  return (
    <div className={`bg-dark-800 rounded-xl border p-5 mb-6 animate-fade-up transition-colors shadow-lg ${
      hasError ? "border-danger-500/40 bg-danger-950/10" : isComplete ? "border-rust-500/30" : "border-dark-500"
    }`}>
      {/* Header */}
      <div className="flex items-center justify-between gap-3 mb-4">
        <div className="flex items-center gap-3">
          {!effectiveDone ? (
            <svg className="w-5 h-5 text-rust-400 animate-spin" fill="none" viewBox="0 0 24 24">
              <circle className="opacity-25" cx="12" cy="12" r="10" stroke="currentColor" strokeWidth="4" />
              <path className="opacity-75" fill="currentColor" d="M4 12a8 8 0 018-8V0C5.373 0 0 5.373 0 12h4zm2 5.291A7.962 7.962 0 014 12H0c0 3.042 1.135 5.824 3 7.938l3-2.647z" />
            </svg>
          ) : isComplete ? (
            <div className="w-5 h-5 rounded-full bg-rust-500/20 text-rust-400 flex items-center justify-center">
              <svg className="w-3.5 h-3.5" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth="3">
                <path strokeLinecap="round" strokeLinejoin="round" d="m4.5 12.75 6 6 9-13.5" />
              </svg>
            </div>
          ) : (
            <div className="w-5 h-5 rounded-full bg-danger-500/20 text-danger-400 flex items-center justify-center">
              <svg className="w-3.5 h-3.5" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth="2.5">
                <path strokeLinecap="round" strokeLinejoin="round" d="M12 9v3.75m-9.303 3.376c-.866 1.5.217 3.374 1.948 3.374h14.71c1.73 0 2.813-1.874 1.948-3.374L13.949 3.378c-.866-1.5-3.032-1.5-3.898 0L2.697 16.126ZM12 15.75h.007v.008H12v-.008Z" />
              </svg>
            </div>
          )}
          <div>
            <span className="text-sm font-semibold text-dark-50 font-mono tracking-wide">
              {headline}
            </span>
            {!effectiveDone && (
              <span className="ml-2 text-xs text-dark-300 font-mono">
                {visibleSteps.length > 0 ? `(Step ${visibleSteps.length}: ${visibleSteps[visibleSteps.length - 1].label})` : "Initializing..."}
              </span>
            )}
          </div>
        </div>

        {/* Right controls: Overall % + View Logs toggle + Dismiss */}
        <div className="flex items-center gap-2">
          {!severed && (
            <span className="text-xs font-mono font-semibold text-dark-200 tabular-nums">
              {overallPercent}%
            </span>
          )}
          {allLogs && (
            <button
              type="button"
              onClick={() => setShowLogs(!showLogs)}
              className="px-2.5 py-1 text-xs font-mono font-medium rounded-md border border-dark-600 bg-dark-700/60 hover:bg-dark-700 text-dark-200 hover:text-dark-50 transition-colors flex items-center gap-1.5"
            >
              <span>{showLogs ? "▲ Hide Logs" : "▼ Expand Full Logs"}</span>
              <span className="px-1.5 py-0.2 rounded bg-dark-800 text-[10px] text-dark-300 border border-dark-600">
                {allLogs.split("\n").length} lines
              </span>
            </button>
          )}
          {effectiveDone && onDismiss && (
            <button
              type="button"
              onClick={onDismiss}
              className="px-2.5 py-1 text-xs font-mono font-medium rounded-md border border-dark-600 bg-dark-700/60 hover:bg-dark-700 text-dark-200 hover:text-dark-50 transition-colors"
              aria-label="Dismiss progress log"
            >
              Dismiss
            </button>
          )}
        </div>
      </div>

      {!severed && (
        <div className="w-full h-1.5 bg-dark-700 rounded-full overflow-hidden mb-3.5">
          <div
            className={`h-full transition-all duration-200 ${
              hasError ? "bg-danger-500" : isComplete ? "bg-rust-500" : "bg-accent-500"
            }`}
            style={{ width: `${overallPercent}%` }}
          />
        </div>
      )}

      {severed && (
        <p className="text-xs mb-3 text-warn-400">
          The progress stream closed before reporting anything. The job may still
          be running, or may have finished before this page attached — check the
          list below for its current state.
        </p>
      )}

      {hasError &&
        /allowed memory size|out of memory|memory_limit|max_allowed_packet/i.test(allLogs) && (
          <div className="mb-3 px-3 py-2.5 rounded-lg bg-warn-500/10 border border-warn-500/35 text-xs text-warn-300 space-y-1">
            <p className="font-semibold">
              Workable Fix: Memory limit reached during SQL import
            </p>
            <p className="text-dark-200">
              Increase <strong>Import PHP Memory Limit</strong> to <strong>2048 MB</strong> or <strong>Unlimited (-1)</strong> and click Retry. Your folder has been safely preserved on disk and returned to <strong>Unregistered Folders Ready to Import</strong>.
            </p>
          </div>
        )}

      {/* Step List */}
      <div className="space-y-1.5 mb-3">
        {visibleSteps.map((step) => (
          <div key={step.step} className="flex items-start gap-3 py-1 px-2 rounded hover:bg-dark-700/30 transition-colors">
            {/* Status icon */}
            <div className="mt-0.5 flex-shrink-0">
              {step.status === "in_progress" ? (
                <svg className="w-3.5 h-3.5 text-warn-400 animate-spin" fill="none" viewBox="0 0 24 24">
                  <circle className="opacity-25" cx="12" cy="12" r="10" stroke="currentColor" strokeWidth="4" />
                  <path className="opacity-75" fill="currentColor" d="M4 12a8 8 0 018-8V0C5.373 0 0 5.373 0 12h4zm2 5.291A7.962 7.962 0 014 12H0c0 3.042 1.135 5.824 3 7.938l3-2.647z" />
                </svg>
              ) : step.status === "done" ? (
                <svg className="w-3.5 h-3.5 text-rust-400" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth="3">
                  <path strokeLinecap="round" strokeLinejoin="round" d="m4.5 12.75 6 6 9-13.5" />
                </svg>
              ) : step.status === "error" ? (
                <svg className="w-3.5 h-3.5 text-danger-400" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth="3">
                  <path strokeLinecap="round" strokeLinejoin="round" d="M6 18 18 6M6 6l12 12" />
                </svg>
              ) : (
                <div className="w-3.5 h-3.5 rounded-full border-2 border-dark-500" />
              )}
            </div>

            {/* Label */}
            <div className="min-w-0 flex-1">
              <span className={`text-sm font-mono ${
                step.status === "done" ? "text-dark-200" :
                step.status === "in_progress" ? "text-dark-50 font-medium" :
                step.status === "error" ? "text-danger-400 font-medium" :
                "text-dark-400"
              }`}>
                {step.label}
              </span>
              {step.status === "in_progress" && typeof step.progress === "number" && (
                <div className="mt-1.5 flex items-center gap-2">
                  <div className="flex-1 h-1.5 bg-dark-700 rounded-full overflow-hidden">
                    <div
                      className="h-full transition-all duration-200 bg-accent-500"
                      style={{ width: `${Math.min(100, Math.max(0, step.progress))}%` }}
                    />
                  </div>
                  <span className="text-[11px] font-mono text-dark-300 tabular-nums w-9 text-right">
                    {step.progress}%
                  </span>
                </div>
              )}
              {step.message && !showLogs && (
                <p className="text-xs text-dark-300 mt-0.5 line-clamp-2 font-mono">
                  {step.message.length > 300 ? `${step.message.slice(0, 300)}...` : step.message}
                </p>
              )}
            </div>
          </div>
        ))}
      </div>

      {/* Expandable Terminal Log Viewer */}
      {showLogs && allLogs && (
        <div className="mt-3 border border-dark-600 rounded-lg overflow-hidden bg-dark-950 shadow-inner">
          <div className="flex items-center justify-between px-3 py-1.5 bg-dark-900 border-b border-dark-700/80">
            <div className="flex items-center gap-2">
              <span className="w-2.5 h-2.5 rounded-full bg-danger-500/60 inline-block" />
              <span className="w-2.5 h-2.5 rounded-full bg-warn-500/60 inline-block" />
              <span className="w-2.5 h-2.5 rounded-full bg-rust-500/60 inline-block" />
              <span className="text-[11px] font-mono text-dark-300 ml-2 font-medium">Terminal Output</span>
            </div>
            <button
              type="button"
              onClick={handleCopyLogs}
              className="text-xs text-dark-300 hover:text-dark-100 font-mono transition-colors"
            >
              {copied ? "Copied!" : "Copy Output"}
            </button>
          </div>
          <pre
            ref={logContainerRef}
            className="p-3 text-xs font-mono text-dark-200 max-h-72 overflow-y-auto whitespace-pre-wrap break-words leading-relaxed select-text"
          >
            {allLogs}
          </pre>
        </div>
      )}
    </div>
  );
}
