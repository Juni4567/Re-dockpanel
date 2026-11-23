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
  const [stage, setStage] = useState<"microlink" | "thum" | "unavatar" | "googleFavicon" | "placeholder">("microlink");
  const [loaded, setLoaded] = useState(false);
  const [cacheBuster, setCacheBuster] = useState(Date.now());

  const cleanDomain = domain.trim().toLowerCase().replace(/^(https?:\/\/)?(www\.)?/, "");
  
  // 1. High-resolution Headless Chrome Screenshot via Microlink API
  const microlinkUrl = `https://api.microlink.io?url=https%3A%2F%2F${cleanDomain}&screenshot=true&meta=false&embed=screenshot.url&t=${cacheBuster}`;
  // 2. Fast Fallback Screenshot via Thum.io
  const thumUrl = `https://image.thum.io/get/width/${w}/crop/400/noanimate/https://${cleanDomain}?t=${cacheBuster}`;
  // 3. Fallback High-Res Icon/Favicon via Unavatar
  const unavatarUrl = `https://unavatar.io/${cleanDomain}?fallback=false&t=${cacheBuster}`;
  // 4. Fallback Google Favicon
  const googleFaviconUrl = `https://www.google.com/s2/favicons?domain=${cleanDomain}&sz=128`;

  const handleRefresh = (e: React.MouseEvent) => {
    e.preventDefault();
    e.stopPropagation();
    setLoaded(false);
    setStage("microlink");
    setCacheBuster(Date.now());
  };

  const handleImageError = () => {
    setLoaded(false);
    if (stage === "microlink") {
      setStage("thum");
    } else if (stage === "thum") {
      setStage("unavatar");
    } else if (stage === "unavatar") {
      setStage("googleFavicon");
    } else {
      setStage("placeholder");
    }
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
        className={`rounded border border-dark-600/60 bg-gradient-to-br ${bgGradient} shrink-0 overflow-hidden relative shadow-xs flex items-center justify-center font-bold text-white select-none ${className}`}
        title={`${domain}`}
      >
        <span className="text-xl font-mono uppercase drop-shadow">{initial}</span>
        {refreshBtn}
      </div>
    );
  }

  if (stage === "unavatar" || stage === "googleFavicon") {
    const currentFaviconUrl = stage === "unavatar" ? unavatarUrl : googleFaviconUrl;
    return (
      <div
        className={`rounded border border-dark-600/60 bg-dark-800/90 shrink-0 overflow-hidden relative shadow-xs flex items-center justify-center ${className}`}
        title={`${domain}`}
      >
        <img
          src={currentFaviconUrl}
          alt={alt || `${domain} favicon`}
          loading="lazy"
          onError={handleImageError}
          className="w-10 h-10 object-contain drop-shadow"
        />
        {refreshBtn}
      </div>
    );
  }

  const currentUrl = stage === "microlink" ? microlinkUrl : thumUrl;

  return (
    <div
      className={`rounded border border-dark-600/60 bg-dark-800/90 shrink-0 overflow-hidden relative shadow-xs flex items-center justify-center group ${className}`}
      title={`${domain} live preview`}
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
        src={currentUrl}
        alt={alt || `${domain} screenshot preview`}
        loading="lazy"
        onLoad={(e) => {
          // If image is suspiciously tiny (e.g. 1x1 blank tracker or failed thumbnail), move to next stage
          const img = e.currentTarget;
          if (img.naturalWidth > 0 && img.naturalWidth < 40 && img.naturalHeight < 40) {
            handleImageError();
          } else {
            setLoaded(true);
          }
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
