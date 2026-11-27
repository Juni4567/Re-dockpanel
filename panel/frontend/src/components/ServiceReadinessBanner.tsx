import { useState, useEffect, useCallback } from "react";
import { Link } from "react-router-dom";
import { api } from "../api";
import { isDesktopApp } from "../utils/desktop";
import { showGlobalNotification, buildAlertDetails } from "../context/NotificationContext";

export interface SystemServiceItem {
  id: string;
  name: string;
  status: "ready" | "starting" | "stopped" | "not_installed" | "standby" | "error" | string;
  detail: string;
  port?: number | null;
  versions?: string[];
  active_workers?: number;
  active_sites?: number;
}

export interface ServicesStatusPayload {
  ready: boolean;
  initializing: boolean;
  uptime_secs?: number;
  summary: string;
  services: SystemServiceItem[];
}

function statusPillClasses(status: string): {
  wrap: string;
  dot: string;
  label: string;
} {
  switch (status) {
    case "ready":
      return {
        wrap: "bg-emerald-500/10 border-emerald-500/30 text-emerald-300",
        dot: "bg-emerald-400",
        label: "Ready",
      };
    case "starting":
      return {
        wrap: "bg-amber-500/15 border-amber-500/35 text-amber-200",
        dot: "bg-amber-400 animate-ping",
        label: "Starting...",
      };
    case "error":
      return {
        wrap: "bg-danger-500/15 border-danger-500/35 text-danger-300",
        dot: "bg-danger-400",
        label: "Error",
      };
    case "stopped":
      return {
        wrap: "bg-warn-500/15 border-warn-500/35 text-warn-300",
        dot: "bg-warn-400",
        label: "Stopped",
      };
    case "standby":
      return {
        wrap: "bg-dark-800 border-dark-600 text-dark-300",
        dot: "bg-dark-400",
        label: "Standby",
      };
    case "not_installed":
    default:
      return {
        wrap: "bg-dark-800/70 border-dark-700 text-dark-400",
        dot: "bg-dark-500",
        label: "Not Installed",
      };
  }
}

export function ServiceReadinessView({
  data,
  collapsed,
  startingMariaDb,
  onToggleCollapse,
  onStartMariaDb,
  onRefresh,
}: {
  data: ServicesStatusPayload;
  collapsed: boolean;
  startingMariaDb?: boolean;
  onToggleCollapse: () => void;
  onStartMariaDb?: () => void;
  onRefresh?: () => void;
}) {
  const mdb = data.services.find((s) => s.id === "mariadb");
  const needsMariaDbStart = Boolean(mdb && (mdb.status === "stopped" || mdb.status === "error"));

  if (collapsed && !data.initializing && !needsMariaDbStart) {
    return (
      <div className="px-4 lg:px-6 pt-2 flex justify-end">
        <button
          type="button"
          onClick={onToggleCollapse}
          className="inline-flex items-center gap-1.5 px-2.5 py-1 rounded-full text-[11px] font-mono bg-emerald-500/10 border border-emerald-500/30 text-emerald-300 hover:bg-emerald-500/20 transition-colors"
          title={data.summary}
        >
          <span className="w-1.5 h-1.5 rounded-full bg-emerald-400" />
          System Ready ({data.services.filter((s) => s.status === "ready").length}/{data.services.length})
        </button>
      </div>
    );
  }

  return (
    <div
      role="status"
      aria-live="polite"
      className={`border-b px-4 lg:px-6 py-2.5 transition-colors ${
        data.initializing
          ? "bg-amber-500/10 border-amber-500/30"
          : needsMariaDbStart
          ? "bg-warn-500/10 border-warn-500/30"
          : "bg-dark-900/70 border-dark-700/70"
      }`}
    >
      <div className="flex items-center justify-between gap-3 flex-wrap">
        <div className="flex items-center gap-2 min-w-0">
          {data.initializing ? (
            <span className="w-4 h-4 border-2 border-amber-400 border-t-transparent rounded-full animate-spin shrink-0" />
          ) : needsMariaDbStart ? (
            <span className="w-2.5 h-2.5 rounded-full bg-warn-400 shrink-0" />
          ) : (
            <span className="w-2.5 h-2.5 rounded-full bg-emerald-400 shrink-0" />
          )}
          <span
            className={`text-xs font-semibold ${
              data.initializing
                ? "text-amber-200"
                : needsMariaDbStart
                ? "text-warn-300"
                : "text-emerald-300"
            }`}
          >
            {data.initializing
              ? "Starting Services"
              : needsMariaDbStart
              ? "Database Offline"
              : "System Ready"}
          </span>
          <span className="text-xs text-dark-300 truncate">{data.summary}</span>
        </div>

        <div className="flex items-center gap-2 flex-wrap">
          {needsMariaDbStart && onStartMariaDb && (
            <button
              type="button"
              onClick={onStartMariaDb}
              disabled={startingMariaDb}
              className="px-2.5 py-1 rounded bg-rust-500 hover:bg-rust-600 text-white text-[11px] font-semibold transition-colors disabled:opacity-50"
            >
              {startingMariaDb ? "Starting MariaDB..." : "Start MariaDB"}
            </button>
          )}
          {onRefresh && (
            <button
              type="button"
              onClick={onRefresh}
              className="px-2 py-1 rounded border border-dark-600 bg-dark-800 hover:bg-dark-700 text-[11px] font-mono text-dark-300 transition-colors"
              title="Refresh service status"
            >
              Refresh
            </button>
          )}
          {!data.initializing && (
            <button
              type="button"
              onClick={onToggleCollapse}
              className="px-2 py-1 rounded text-[11px] font-mono text-dark-400 hover:text-dark-100 transition-colors"
            >
              Hide
            </button>
          )}
        </div>
      </div>

      <div className="mt-2 flex items-center gap-1.5 flex-wrap">
        {data.services.map((svc) => {
          const st = statusPillClasses(svc.status);
          const portLabel = svc.port ? ` :${svc.port}` : "";
          const badge = (
            <span
              key={svc.id}
              title={`${svc.name}: ${svc.detail}`}
              className={`inline-flex items-center gap-1.5 px-2 py-0.5 rounded-md border text-[11px] font-mono ${st.wrap}`}
            >
              <span className={`w-1.5 h-1.5 rounded-full shrink-0 ${st.dot}`} />
              <span className="font-semibold">{svc.name}</span>
              <span className="opacity-80">
                {svc.status === "ready" && portLabel
                  ? `Ready${portLabel}`
                  : svc.status === "ready" && svc.active_workers !== undefined
                  ? `Ready (${svc.active_workers} active)`
                  : st.label}
              </span>
            </span>
          );

          if (svc.id === "mariadb") {
            return (
              <Link key={svc.id} to="/databases" className="hover:opacity-90 transition-opacity">
                {badge}
              </Link>
            );
          }
          if (svc.id === "php") {
            return (
              <Link key={svc.id} to="/sites" className="hover:opacity-90 transition-opacity">
                {badge}
              </Link>
            );
          }
          if (svc.id === "mail_catcher") {
            return (
              <Link key={svc.id} to="/mail" className="hover:opacity-90 transition-opacity">
                {badge}
              </Link>
            );
          }
          return badge;
        })}
      </div>
    </div>
  );
}

export default function ServiceReadinessBanner() {
  const desktop = isDesktopApp();
  const [data, setData] = useState<ServicesStatusPayload | null>(null);
  const [startingMariaDb, setStartingMariaDb] = useState(false);
  const [collapsed, setCollapsed] = useState<boolean>(() => {
    try {
      return sessionStorage.getItem("dp:services-bar-collapsed") === "true";
    } catch {
      return false;
    }
  });

  const fetchStatus = useCallback(async () => {
    if (!desktop) return;
    try {
      const res = await api.get<ServicesStatusPayload>("/system/services-status");
      if (res && Array.isArray(res.services)) {
        setData(res);
      }
    } catch {
      // Ignore transient poll errors while backend initializes
    }
  }, [desktop]);

  useEffect(() => {
    if (!desktop) return;
    void fetchStatus();
  }, [desktop, fetchStatus]);

  useEffect(() => {
    if (!desktop || !data) return;
    const intervalMs = data.initializing ? 1200 : 15000;
    const id = setInterval(() => {
      void fetchStatus();
    }, intervalMs);
    return () => clearInterval(id);
  }, [desktop, data, fetchStatus]);

  useEffect(() => {
    if (!desktop) return;
    const onNotify = (e: Event) => {
      const cat = (e as CustomEvent<{ meta?: { category?: string } }>).detail?.meta?.category;
      if (cat === "system" || cat === "mariadb" || cat === "site" || cat === "php") {
        void fetchStatus();
      }
    };
    window.addEventListener("dp:global-notify", onNotify);
    return () => window.removeEventListener("dp:global-notify", onNotify);
  }, [desktop, fetchStatus]);

  const handleToggleCollapse = () => {
    setCollapsed((prev) => {
      const next = !prev;
      try {
        sessionStorage.setItem("dp:services-bar-collapsed", next ? "true" : "false");
      } catch {
        // Ignore storage errors
      }
      return next;
    });
  };

  const handleStartMariaDb = async () => {
    setStartingMariaDb(true);
    try {
      const res = await api.post<{ ok?: boolean; port?: number }>("/mariadb/start", {});
      showGlobalNotification(
        "success",
        `MariaDB started cleanly on 127.0.0.1:${res?.port || 3306}`,
        "MariaDB Started",
        `Port: ${res?.port || 3306}`,
        { category: "mariadb", target: "MariaDB", dedupeKey: "mariadb-start" }
      );
      await fetchStatus();
    } catch (err) {
      const info = buildAlertDetails(err, {
        context: "Start MariaDB service",
        category: "mariadb",
        target: "MariaDB",
        dedupeKey: "mariadb-start",
      });
      showGlobalNotification("error", info.message, "MariaDB Start Failed", info.details, info.meta);
    } finally {
      setStartingMariaDb(false);
    }
  };

  if (!desktop || !data) return null;

  return (
    <ServiceReadinessView
      data={data}
      collapsed={collapsed}
      startingMariaDb={startingMariaDb}
      onToggleCollapse={handleToggleCollapse}
      onStartMariaDb={handleStartMariaDb}
      onRefresh={() => void fetchStatus()}
    />
  );
}
