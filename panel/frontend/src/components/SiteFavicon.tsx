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
  const [stage, setStage] = useState<"google" | "duckduckgo" | "monogram">("google");

  const cleanDomain = domain.trim().toLowerCase().replace(/^(https?:\/\/)?(www\.)?/, "");
  const initial = cleanDomain.charAt(0).toUpperCase() || "W";

  const googleUrl = `https://www.google.com/s2/favicons?domain=${cleanDomain}&sz=${size}`;
  const ddgUrl = `https://icons.duckduckgo.com/ip3/${cleanDomain}.ico`;

  if (stage === "monogram") {
    return (
      <div
        className={`bg-dark-700 text-dark-300 font-mono text-[10px] font-bold flex items-center justify-center select-none uppercase ${className}`}
        title={domain}
      >
        {initial}
      </div>
    );
  }

  const currentUrl = stage === "google" ? googleUrl : ddgUrl;

  return (
    <img
      src={currentUrl}
      alt=""
      loading="lazy"
      onError={() => {
        if (stage === "google") setStage("duckduckgo");
        else setStage("monogram");
      }}
      className={`object-contain ${className}`}
    />
  );
}
