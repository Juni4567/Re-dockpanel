import { useEffect, useState } from "react";

interface SiteThumbnailProps {
  domain: string;
  className?: string;
  w?: number;
  alt?: string;
  showRefresh?: boolean;
}

const THUMB_VERSION_PREFIX = "dp:site-thumb-ver:v1:";
const THUMB_REFRESH_EVENT = "dp:site-thumb-refresh";

function readCachedThumbVersion(cleanDomain: string): string {
  try {
    return localStorage.getItem(`${THUMB_VERSION_PREFIX}${cleanDomain}`) || "1";
  } catch {
    return "1";
  }
}

export function warmSiteThumbnailCache(domain: string, forceRefresh = false): void {
  const cleanDomain = domain.trim().toLowerCase().replace(/^(https?:\/\/)?(www\.)?/, "");
  if (!cleanDomain) return;
  const ver = forceRefresh ? String(Date.now()) : readCachedThumbVersion(cleanDomain);
  if (forceRefresh) {
    try {
      localStorage.setItem(`${THUMB_VERSION_PREFIX}${cleanDomain}`, ver);
    } catch {
      // Ignore storage quota issues
    }
    if (typeof window !== "undefined") {
      window.dispatchEvent(
        new CustomEvent(THUMB_REFRESH_EVENT, { detail: { domain: cleanDomain, version: ver } })
      );
    }
  }
  const url = `/api/sites/thumbnail?domain=${encodeURIComponent(cleanDomain)}${
    forceRefresh ? "&refresh=1" : ""
  }&v=${encodeURIComponent(ver)}`;
  if (typeof fetch === "function") {
    fetch(url, { credentials: "same-origin" }).catch(() => {});
  }
}

export default function SiteThumbnail({
  domain,
  className = "w-12 h-8",
  alt,
  showRefresh = false,
}: SiteThumbnailProps) {
  const cleanDomain = domain.trim().toLowerCase().replace(/^(https?:\/\/)?(www\.)?/, "");
  const [stage, setStage] = useState<"backend" | "placeholder">("backend");
  const [loaded, setLoaded] = useState(false);
  const [refreshing, setRefreshing] = useState(false);
  const [forceRefreshOnce, setForceRefreshOnce] = useState(false);
  const [version, setVersion] = useState<string>(() => readCachedThumbVersion(cleanDomain));

  useEffect(() => {
    setVersion(readCachedThumbVersion(cleanDomain));
    setStage("backend");
    setLoaded(false);
    setForceRefreshOnce(false);
  }, [cleanDomain]);

  useEffect(() => {
    const onRefresh = (e: Event) => {
      const detail = (e as CustomEvent<{ domain?: string; version?: string }>).detail;
      if (detail?.domain === cleanDomain && detail.version) {
        setVersion(detail.version);
        setStage("backend");
        setLoaded(false);
      }
    };
    window.addEventListener(THUMB_REFRESH_EVENT, onRefresh);
    return () => window.removeEventListener(THUMB_REFRESH_EVENT, onRefresh);
  }, [cleanDomain]);

  const backendUrl = `/api/sites/thumbnail?domain=${encodeURIComponent(cleanDomain)}${
    forceRefreshOnce ? "&refresh=1" : ""
  }&v=${encodeURIComponent(version)}`;

  const handleRefresh = (e: React.MouseEvent) => {
    e.preventDefault();
    e.stopPropagation();
    const nextVer = String(Date.now());
    try {
      localStorage.setItem(`${THUMB_VERSION_PREFIX}${cleanDomain}`, nextVer);
    } catch {
      // Ignore storage errors
    }
    setRefreshing(true);
    setLoaded(false);
    setForceRefreshOnce(true);
    setStage("backend");
    setVersion(nextVer);
    window.dispatchEvent(
      new CustomEvent(THUMB_REFRESH_EVENT, { detail: { domain: cleanDomain, version: nextVer } })
    );
  };

  const handleImageError = () => {
    setLoaded(false);
    setRefreshing(false);
    setStage("placeholder");
  };

  const initial = cleanDomain.charAt(0).toUpperCase() || "W";
  const colors = [
    "from-blue-600 to-indigo-700",
    "from-emerald-600 to-teal-700",
    "from-amber-600 to-orange-700",
    "from-purple-600 to-pink-700",
    "from-rose-600 to-red-700",
    "from-cyan-600 to-blue-700",
  ];
  const colorIndex = (cleanDomain.charCodeAt(0) + cleanDomain.length) % colors.length;
  const bgGradient = colors[colorIndex];

  const refreshBtn = showRefresh ? (
    <button
      type="button"
      onClick={handleRefresh}
      disabled={refreshing}
      className="absolute bottom-1.5 right-1.5 p-1.5 bg-dark-900/85 hover:bg-dark-900 text-white rounded-md shadow-md opacity-85 hover:opacity-100 transition-opacity z-20 disabled:opacity-60"
      title="Refresh cached website preview"
      aria-label={`Refresh preview for ${cleanDomain}`}
    >
      <svg
        className={`w-3.5 h-3.5 ${refreshing ? "animate-spin" : ""}`}
        fill="none"
        viewBox="0 0 24 24"
        stroke="currentColor"
      >
        <path
          strokeLinecap="round"
          strokeLinejoin="round"
          strokeWidth={2}
          d="M4 4v5h.582m15.356 2A8.001 8.001 0 004.582 9m0 0H9m11 11v-5h-.581m0 0a8.003 8.003 0 01-15.357-2m15.357 2H15"
        />
      </svg>
    </button>
  ) : null;

  if (stage === "placeholder") {
    return (
      <div
        className={`rounded border border-dark-600/60 bg-gradient-to-br ${bgGradient} shrink-0 overflow-hidden relative shadow-xs flex items-center justify-center font-bold text-white select-none ${className}`}
        title={`${domain}`}
      >
        <span className="text-xl font-mono uppercase drop-shadow">{initial}</span>
        {refreshBtn}
      </div>
    );
  }

  return (
    <div
      className={`rounded border border-dark-600/60 bg-dark-800/90 shrink-0 overflow-hidden relative shadow-xs flex items-center justify-center group ${className}`}
      title={`${domain} cached preview`}
    >
      {!loaded && (
        <div className="absolute inset-0 bg-dark-700/50 animate-pulse flex items-center justify-center">
          <svg className="w-4 h-4 text-dark-500 animate-spin" fill="none" viewBox="0 0 24 24">
            <circle className="opacity-25" cx="12" cy="12" r="10" stroke="currentColor" strokeWidth={3} />
            <path className="opacity-75" fill="currentColor" d="M4 12a8 8 0 018-8v8H4z" />
          </svg>
        </div>
      )}
      <img
        src={backendUrl}
        alt={alt || `${domain} website preview`}
        loading="lazy"
        onLoad={() => {
          setLoaded(true);
          setRefreshing(false);
          setForceRefreshOnce(false);
        }}
        onError={handleImageError}
        className={`w-full h-full object-cover object-top transition-all duration-300 ${
          loaded ? "opacity-100 scale-100" : "opacity-0 scale-95"
        }`}
      />
      {refreshBtn}
    </div>
  );
}
