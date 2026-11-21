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
  w = 320,
  alt,
}: SiteThumbnailProps) {
  const [stage, setStage] = useState<"screenshot" | "favicon" | "placeholder">("screenshot");
  const [loaded, setLoaded] = useState(false);

  // thum.io works without an API key for basic screenshots
  const screenshotUrl = `https://image.thum.io/get/width/${w}/crop/200/noanimate/https://${domain}`;
  // Google favicon service as fallback - always works
  const faviconUrl = `https://www.google.com/s2/favicons?domain=${domain}&sz=128`;

  const handleScreenshotError = () => {
    setLoaded(false);
    setStage("favicon");
  };

  const handleFaviconError = () => {
    setStage("placeholder");
  };

  if (stage === "placeholder") {
    return (
      <div
        className={`rounded border border-dark-600/60 bg-gradient-to-br from-dark-800 to-dark-900 shrink-0 overflow-hidden relative shadow-xs flex items-center justify-center ${className}`}
        title={`${domain}`}
      >
        <svg className="w-4 h-4 text-dark-400/80" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={1.5} aria-hidden="true">
          <path strokeLinecap="round" strokeLinejoin="round" d="M12 21a9.004 9.004 0 008.716-6.747M12 21a9.004 9.004 0 01-8.716-6.747M12 21c2.485 0 4.5-4.03 4.5-9S14.485 3 12 3m0 18c-2.485 0-4.5-4.03-4.5-9S9.515 3 12 3m0 0a8.997 8.997 0 017.843 4.582M12 3a8.997 8.997 0 00-7.843 4.582m15.686 0A11.953 11.953 0 0112 10.5c-2.998 0-5.74-1.1-7.843-2.918m15.686 0A8.959 8.959 0 0121 12c0 .778-.099 1.533-.284 2.253m0 0A17.919 17.919 0 0112 16.5c-3.162 0-6.133-.815-8.716-2.247m0 0A9.015 9.015 0 013 12c0-1.605.42-3.113 1.157-4.418" />
        </svg>
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
          className="w-6 h-6 object-contain"
        />
      </div>
    );
  }

  return (
    <div
      className={`rounded border border-dark-600/60 bg-dark-800/90 shrink-0 overflow-hidden relative shadow-xs flex items-center justify-center ${className}`}
      title={`${domain} live preview`}
    >
      {!loaded && (
        <div className="absolute inset-0 bg-dark-700/50 animate-pulse flex items-center justify-center">
          <svg className="w-3.5 h-3.5 text-dark-500 animate-spin" fill="none" viewBox="0 0 24 24">
            <circle className="opacity-25" cx="12" cy="12" r="10" stroke="currentColor" strokeWidth="3" />
            <path className="opacity-75" fill="currentColor" d="M4 12a8 8 0 018-8v8H4z" />
          </svg>
        </div>
      )}
      <img
        src={screenshotUrl}
        alt={alt || `${domain} screenshot preview`}
        loading="lazy"
        onLoad={() => setLoaded(true)}
        onError={handleScreenshotError}
        className={`w-full h-full object-cover object-top transition-all duration-300 ${
          loaded ? "opacity-100 scale-100" : "opacity-0 scale-95"
        }`}
      />
    </div>
  );
}

  return (
    <div
      className={`rounded border border-dark-600/60 bg-dark-800/90 shrink-0 overflow-hidden relative shadow-xs flex items-center justify-center ${className}`}
      title={`${domain} live preview`}
    >
      {!hasError ? (
        <>
          {!loaded && (
            <div className="absolute inset-0 bg-dark-700/50 animate-pulse flex items-center justify-center">
              <svg
                className="w-3.5 h-3.5 text-dark-500 animate-spin"
                fill="none"
                viewBox="0 0 24 24"
              >
                <circle
                  className="opacity-25"
                  cx="12"
                  cy="12"
                  r="10"
                  stroke="currentColor"
                  strokeWidth="3"
                />
                <path
                  className="opacity-75"
                  fill="currentColor"
                  d="M4 12a8 8 0 018-8v8H4z"
                />
              </svg>
            </div>
          )}
          <img
            src={screenshotUrl}
            alt={alt || `${domain} screenshot preview`}
            loading="lazy"
            onLoad={() => setLoaded(true)}
            onError={() => setHasError(true)}
            className={`w-full h-full object-cover object-top transition-all duration-300 ${
              loaded ? "opacity-100 scale-100" : "opacity-0 scale-95"
            }`}
          />
        </>
      ) : (
        <div className="w-full h-full flex flex-col items-center justify-center text-dark-400 bg-gradient-to-br from-dark-800 to-dark-900 p-1">
          <svg
            className="w-4 h-4 text-dark-400/80"
            fill="none"
            viewBox="0 0 24 24"
            stroke="currentColor"
            strokeWidth={1.5}
            aria-hidden="true"
          >
            <path
              strokeLinecap="round"
              strokeLinejoin="round"
              d="M12 21a9.004 9.004 0 008.716-6.747M12 21a9.004 9.004 0 01-8.716-6.747M12 21c2.485 0 4.5-4.03 4.5-9S14.485 3 12 3m0 18c-2.485 0-4.5-4.03-4.5-9S9.515 3 12 3m0 0a8.997 8.997 0 017.843 4.582M12 3a8.997 8.997 0 00-7.843 4.582m15.686 0A11.953 11.953 0 0112 10.5c-2.998 0-5.74-1.1-7.843-2.918m15.686 0A8.959 8.959 0 0121 12c0 .778-.099 1.533-.284 2.253m0 0A17.919 17.919 0 0112 16.5c-3.162 0-6.133-.815-8.716-2.247m0 0A9.015 9.015 0 013 12c0-1.605.42-3.113 1.157-4.418"
            />
          </svg>
        </div>
      )}
    </div>
  );
}
