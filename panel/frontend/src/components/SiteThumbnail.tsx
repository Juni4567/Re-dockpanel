import { useState } from "react";

interface SiteThumbnailProps {
  domain: string;
  className?: string;
  w?: number;
  alt?: string;
  showRefresh?: boolean;
}

export default function SiteThumbnail({
  domain,
  className = "w-12 h-8",
  w = 600,
  alt,
  showRefresh = false,
}: SiteThumbnailProps) {
  const [stage, setStage] = useState<"thum" | "mshots" | "favicon" | "placeholder">("thum");
  const [loaded, setLoaded] = useState(false);
  const [cacheBuster, setCacheBuster] = useState(Date.now());

  const cleanDomain = domain.trim().toLowerCase().replace(/^(https?:\/\/)?(www\.)?/, "");
  const thumUrl = `https://image.thum.io/get/width/${w}/crop/400/noanimate/https://${cleanDomain}?t=${cacheBuster}`;
  const mshotsUrl = `https://s0.wp.com/mshots/v1/https%3A%2F%2F${cleanDomain}?w=${w}&t=${cacheBuster}`;
  const faviconUrl = `https://www.google.com/s2/favicons?domain=${cleanDomain}&sz=128`;

  const handleRefresh = (e: React.MouseEvent) => {
    e.preventDefault();
    e.stopPropagation();
    setLoaded(false);
    setStage("thum");
    setCacheBuster(Date.now());
  };

  const handleThumError = () => {
    setLoaded(false);
    setStage("mshots");
  };

  const handleMshotsError = () => {
    setLoaded(false);
    setStage("favicon");
  };

  const handleFaviconError = () => {
    setStage("placeholder");
  };

  const initial = cleanDomain.charAt(0).toUpperCase() || "W";

  const refreshBtn = showRefresh ? (
    <button
      onClick={handleRefresh}
      className="absolute bottom-2 right-2 p-1.5 bg-dark-900/80 hover:bg-dark-900 text-white rounded shadow-md opacity-80 hover:opacity-100 transition-opacity z-10"
      title="Refresh thumbnail preview"
    >
      <svg className="w-3.5 h-3.5" fill="none" viewBox="0 0 24 24" stroke="currentColor">
        <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M4 4v5h.582m15.356 2A8.001 8.001 0 004.582 9m0 0H9m11 11v-5h-.581m0 0a8.003 8.003 0 01-15.357-2m15.357 2H15" />
      </svg>
    </button>
  ) : null;

  if (stage === "placeholder") {
    return (
      <div
        className={`rounded border border-dark-600/60 bg-gradient-to-br from-dark-800 to-dark-900 shrink-0 overflow-hidden relative shadow-xs flex items-center justify-center font-bold text-dark-400 select-none ${className}`}
        title={`${domain}`}
      >
        <span className="text-xl font-mono uppercase">{initial}</span>
        {refreshBtn}
      </div>
    );
  }

  if (stage === "favicon") {
    return (
      <div
        className={`rounded border border-dark-600/60 bg-dark-800/90 shrink-0 overflow-hidden relative shadow-xs flex items-center justify-center ${className}`}
        title={`${domain}`}
      >
        <img
          src={faviconUrl}
          alt={alt || `${domain} favicon`}
          loading="lazy"
          onError={handleFaviconError}
          className="w-10 h-10 object-contain"
        />
        {refreshBtn}
      </div>
    );
  }

  const currentUrl = stage === "thum" ? thumUrl : mshotsUrl;
  const errorHandler = stage === "thum" ? handleMshotsError : handleFaviconError;

  return (
    <div
      className={`rounded border border-dark-600/60 bg-dark-800/90 shrink-0 overflow-hidden relative shadow-xs flex items-center justify-center group ${className}`}
      title={`${domain} live preview`}
    >
      {!loaded && (
        <div className="absolute inset-0 bg-dark-700/50 animate-pulse flex items-center justify-center">
          <svg className="w-4 h-4 text-dark-500 animate-spin" fill="none" viewBox="0 0 24 24">
            <circle className="opacity-25" cx="12" cy="12" r="10" stroke="currentColor" strokeWidth="3" />
            <path className="opacity-75" fill="currentColor" d="M4 12a8 8 0 018-8v8H4z" />
          </svg>
        </div>
      )}
      <img
        src={currentUrl}
        alt={alt || `${domain} screenshot preview`}
        loading="lazy"
        onLoad={() => setLoaded(true)}
        onError={errorHandler}
        className={`w-full h-full object-cover object-top transition-all duration-300 ${
          loaded ? "opacity-100 scale-100" : "opacity-0 scale-95"
        }`}
      />
      {refreshBtn}
    </div>
  );
}
