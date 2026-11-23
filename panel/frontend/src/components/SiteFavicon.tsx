import { useState } from "react";

interface SiteFaviconProps {
  domain: string;
  className?: string;
  size?: number;
}

export default function SiteFavicon({
  domain,
  className = "w-4 h-4 rounded-sm flex-shrink-0",
  size = 32,
}: SiteFaviconProps) {
  const [stage, setStage] = useState<"unavatar" | "direct" | "google" | "monogram">("unavatar");

  const cleanDomain = domain.trim().toLowerCase().replace(/^(https?:\/\/)?(www\.)?/, "");
  const initial = cleanDomain.charAt(0).toUpperCase() || "W";

  // Stage 1: Unavatar (aggregates high-res touch-icons, web manifests, and favicons)
  const unavatarUrl = `https://unavatar.io/${cleanDomain}?fallback=false`;
  // Stage 2: Direct site /favicon.ico
  const directUrl = `https://${cleanDomain}/favicon.ico`;
  // Stage 3: Google Favicon service
  const googleUrl = `https://www.google.com/s2/favicons?domain=${cleanDomain}&sz=${size}`;

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

  if (stage === "monogram") {
    return (
      <div
        className={`bg-gradient-to-br ${bgGradient} text-white font-mono text-[9px] font-bold flex items-center justify-center select-none uppercase shadow-xs ${className}`}
        title={domain}
      >
        {initial}
      </div>
    );
  }

  let currentUrl = unavatarUrl;
  if (stage === "direct") currentUrl = directUrl;
  else if (stage === "google") currentUrl = googleUrl;

  const handleNextStage = () => {
    if (stage === "unavatar") setStage("direct");
    else if (stage === "direct") setStage("google");
    else setStage("monogram");
  };

  return (
    <img
      src={currentUrl}
      alt=""
      loading="lazy"
      onError={handleNextStage}
      className={`object-contain ${className}`}
    />
  );
}
