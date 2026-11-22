import { useState } from "react";

interface SiteThumbnailProps {
  domain: string;
  className?: string;
  w?: number;
  alt?: string;
}

export default function SiteThumbnail({
  domain,
  className = "w-12 h-8",
  w = 400,
  alt,
}: SiteThumbnailProps) {
  const [stage, setStage] = useState<"microlink" | "thum" | "favicon" | "placeholder">("microlink");
  const [loaded, setLoaded] = useState(false);

  const cleanDomain = domain.trim().toLowerCase();
  const microlinkUrl = `https://api.microlink.io?url=https://${cleanDomain}&screenshot=true&meta=false&embed=screenshot.url`;
  const thumUrl = `https://image.thum.io/get/width/${w}/crop/200/noanimate/https://${cleanDomain}`;
  const faviconUrl = `https://www.google.com/s2/favicons?domain=${cleanDomain}&sz=128`;

  const handleMicrolinkError = () => {
    setLoaded(false);
    setStage("thum");
  };

  const handleThumError = () => {
    setLoaded(false);
    setStage("favicon");
  };

  const handleFaviconError = () => {
    setStage("placeholder");
  };

  const initial = cleanDomain.replace(/^(https?:\/\/)?(www\.)?/, "").charAt(0).toUpperCase() || "W";

  if (stage === "placeholder") {
    return (
      <div
        className={`rounded border border-dark-600/60 bg-gradient-to-br from-dark-800 to-dark-900 shrink-0 overflow-hidden relative shadow-xs flex items-center justify-center font-bold text-dark-400 select-none ${className}`}
        title={`${domain}`}
      >
        <span className="text-sm font-mono uppercase">{initial}</span>
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
          className="w-5 h-5 object-contain"
        />
      </div>
    );
  }

  const currentUrl = stage === "microlink" ? microlinkUrl : thumUrl;
  const errorHandler = stage === "microlink" ? handleMicrolinkError : handleThumError;

  return (
    <div
      className={`rounded border border-dark-600/60 bg-dark-800/90 shrink-0 overflow-hidden relative shadow-xs flex items-center justify-center ${className}`}
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
    </div>
  );
}
