import { Link, NavLink } from "react-router-dom";
import { useLayoutState, badgeCount } from "../hooks/useLayoutState";
import { useBranding } from "../context/BrandingContext";
import { useServer } from "../context/ServerContext";
import { Icon } from "../data/icons";
import LayoutSwitcher from "./LayoutSwitcher";
import { PANEL_VERSION } from "../constants";
import { isDesktopApp, isServerOnlyPath } from "../utils/desktop";
import { useState, useRef, useEffect } from "react";

function ServerSelector() {
  const { servers, activeServer, setActiveServerId } = useServer();
  const [open, setOpen] = useState(false);
  const ref = useRef<HTMLDivElement>(null);

  useEffect(() => {
    function handleClick(e: MouseEvent) {
      if (ref.current && !ref.current.contains(e.target as Node)) setOpen(false);
    }
    document.addEventListener("mousedown", handleClick);
    return () => document.removeEventListener("mousedown", handleClick);
  }, []);

  if (servers.length <= 1) return null;

  return (
    <div className="px-3 pt-2" ref={ref}>
      <button
        onClick={() => setOpen(!open)}
        className="w-full flex items-center gap-2 px-3 py-2 rounded-lg bg-dark-800/50 border border-dark-600/50 text-sm text-dark-200 hover:text-dark-100 hover:border-dark-400 transition-colors"
      >
        <div className={`w-2 h-2 rounded-full shrink-0 ${activeServer?.status === "online" ? "bg-rust-500" : activeServer?.status === "offline" ? "bg-danger-500" : "bg-dark-400"}`} />
        <span className="flex-1 text-left truncate">{activeServer?.name || "Select server"}</span>
        <svg className={`w-4 h-4 transition-transform ${open ? "rotate-180" : ""}`} fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}>
          <path strokeLinecap="round" strokeLinejoin="round" d="M19.5 8.25l-7.5 7.5-7.5-7.5" />
        </svg>
      </button>
      {open && (
        <div className="mt-1 bg-dark-900 border border-dark-600 rounded-lg shadow-xl overflow-hidden">
          {servers.map((s) => (
            <button
              key={s.id}
              onClick={() => { setActiveServerId(s.id); setOpen(false); window.location.href = "/"; }}
              className={`w-full flex items-center gap-2 px-3 py-2 text-sm text-left hover:bg-dark-700/50 transition-colors ${s.id === activeServer?.id ? "bg-dark-800 text-dark-50" : "text-dark-300"}`}
            >
              <div className={`w-2 h-2 rounded-full shrink-0 ${s.status === "online" ? "bg-rust-500" : s.status === "offline" ? "bg-danger-500" : "bg-dark-400"}`} />
              <span className="flex-1 truncate">{s.name}</span>
              {s.is_local && <span className="text-[10px] text-dark-400 uppercase">local</span>}
            </button>
          ))}
        </div>
      )}
    </div>
  );
}

interface SidebarProps {
  showHeader?: boolean;
  flatNav?: boolean;
}

export default function Sidebar({ showHeader = false, flatNav = false }: SidebarProps) {
  const state = useLayoutState();
  const branding = useBranding();
  const isLight = state.theme === "clean" || state.theme === "arctic";
  const isDesktop = isDesktopApp();

  // Prune server-only navigation items when in desktop mode
  const visibleGroups = state.visibleGroups
    .map((g) => ({
      ...g,
      items: g.items.filter((item) => !(isDesktop && (item.serverOnly || isServerOnlyPath(item.to)))),
    }))
    .filter((g) => g.items.length > 0);

  const navItems = flatNav ? visibleGroups.flatMap((g) => g.items) : null;

  return (
    <aside
      className={`fixed inset-y-0 left-0 z-50 w-64 bg-dark-950 border-r border-dark-600 text-dark-50 flex flex-col shrink-0 transform transition-transform duration-200 ease-in-out md:relative md:translate-x-0 ${
        state.sidebarOpen ? "translate-x-0" : "-translate-x-full"
      }`}
    >
      {/* Logo */}
      <div className={`px-6 border-b border-dark-600 flex items-center justify-between ${showHeader ? "h-16" : "py-5"}`}>
        <Link to="/" className="flex items-center gap-3 hover:opacity-90 transition-opacity">
          {branding.logoUrl ? (
            <img src={branding.logoUrl} alt={branding.panelName} className="h-10 w-auto max-w-[160px] object-contain" />
          ) : (
            <>
              <div className="w-8 h-8 bg-rust-500 rounded-lg flex items-center justify-center logo-icon-glow">
                <svg className="w-5 h-5 text-dark-950" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2.5" aria-hidden="true">
                  <path d="M5 16h4" strokeLinecap="square" />
                  <path d="M5 12h8" strokeLinecap="square" />
                  <path d="M5 8h6" strokeLinecap="square" />
                  <rect x="16" y="7" width="4" height="4" fill="currentColor" stroke="none" />
                  <rect x="16" y="13" width="4" height="4" fill="currentColor" stroke="none" />
                </svg>
              </div>
              {!branding.hideBranding && (
                <span className="text-lg font-bold logo-glow">
                  {branding.panelName === "DockPanel" ? (
                    <><span className="text-rust-400">Re-</span><span className="text-rust-400">Dock</span><span className="text-dark-50">Panel</span></>
                  ) : (
                    <span className="text-dark-50">{branding.panelName}</span>
                  )}
                </span>
              )}
            </>
          )}
        </Link>
        <button
          onClick={() => state.setSidebarOpen(false)}
          className="p-1.5 text-dark-200 hover:text-dark-50 md:hidden rounded-lg"
          aria-label="Close sidebar"
        >
          <svg className="w-5 h-5" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2} aria-hidden="true">
            <path strokeLinecap="round" strokeLinejoin="round" d="M6 18L18 6M6 6l12 12" />
          </svg>
        </button>
      </div>

      {/* Search shortcut (hide if header has search) */}
      {!showHeader && (
        <div className="px-3 pt-3 pb-1">
          <button
            onClick={() => window.dispatchEvent(new KeyboardEvent("keydown", { key: "k", ctrlKey: true }))}
            className="w-full flex items-center gap-2 px-3 py-2 rounded-lg bg-dark-800/30 border border-dark-600/50 text-sm text-dark-300 hover:text-dark-100 hover:border-dark-400 transition-colors outline-none"
          >
            <Icon name="search" className="w-[19px] h-[19px]" />
            <span className="flex-1 text-left">Search...</span>
            <kbd className="text-[10px] px-1.5 py-0.5 border border-dark-500 rounded bg-dark-700/50">Ctrl K</kbd>
          </button>
        </div>
      )}

      {!isDesktop && <ServerSelector />}

      {/* Nav */}
      <nav className="flex-1 px-3 pt-4 overflow-y-auto sidebar-scroll-fade">
        {flatNav && navItems ? (
          /* Flat nav — no group labels */
          <div className="space-y-1">
            {navItems.map((item) => (
              <NavLink
                key={item.to}
                to={item.to}
                end
                onClick={() => state.setSidebarOpen(false)}
                className={({ isActive }) =>
                  `flex items-center gap-3 px-3 py-2.5 text-sm font-medium rounded-lg transition-colors ${
                    isActive
                      ? "bg-rust-500/10 text-rust-400"
                      : "text-dark-300 hover:text-dark-100 hover:bg-dark-700/30"
                  }`
                }
              >
                {({ isActive }) => (
                  <>
                    <span className={isActive ? "text-rust-400" : "text-dark-400"}><Icon name={item.iconName} className="w-5 h-5" /></span>
                    <span>{item.label}</span>
                    {item.to === "/monitoring" && (state.firingCount > 0 || state.incidentCount > 0) && (
                      <span className="ml-auto flex items-center gap-1">
                        {state.firingCount > 0 && (
                          <span
                            title={`${state.firingCount} firing alert${state.firingCount === 1 ? "" : "s"}`}
                            className="px-1.5 py-0.5 text-xs font-bold bg-danger-500 text-white rounded-full min-w-[20px] text-center"
                          >
                            {badgeCount(state.firingCount)}
                          </span>
                        )}
                        {state.incidentCount > 0 && (
                          <span
                            title={`${state.incidentCount} open incident${state.incidentCount === 1 ? "" : "s"}`}
                            className="px-1.5 py-0.5 text-xs font-bold bg-warn-500 text-dark-900 rounded-full min-w-[20px] text-center"
                          >
                            {badgeCount(state.incidentCount)}
                          </span>
                        )}
                      </span>
                    )}
                  </>
                )}
              </NavLink>
            ))}
          </div>
        ) : (
          /* Grouped nav — with labels */
          visibleGroups.map((group, gi) => (
            <div key={group.label} className={gi > 0 ? "mt-5" : ""}>
              {gi > 0 && (
                <div className="px-4 pb-1.5 text-[10px] text-dark-400 uppercase tracking-widest font-medium">
                  {group.label}
                </div>
              )}
              <div className="space-y-1">
                {group.items.map((item) => (
                  <NavLink
                    key={item.label}
                    to={item.to}
                    end
                    onClick={() => state.setSidebarOpen(false)}
                    className={({ isActive }) =>
                      `flex items-center gap-3 px-4 py-2 transition-colors text-sm ${
                        isActive
                          ? "bg-rust-500/10 text-rust-400 font-bold border-l-2 border-rust-500"
                          : "text-dark-300 hover:text-dark-100 hover:bg-dark-700/30"
                      }`
                    }
                  >
                    {({ isActive }) => (
                      <>
                        <Icon name={item.iconName} />
                        <span>{item.label}</span>
                        {item.to === "/monitoring" && (state.firingCount > 0 || state.incidentCount > 0) ? (
                          <span className="ml-auto flex items-center gap-1">
                            {state.firingCount > 0 && (
                              <span
                                title={`${state.firingCount} firing alert${state.firingCount === 1 ? "" : "s"}`}
                                className="px-1.5 py-0.5 text-xs font-bold bg-danger-500 text-white rounded-full min-w-[20px] text-center"
                              >
                                {badgeCount(state.firingCount)}
                              </span>
                            )}
                            {state.incidentCount > 0 && (
                              <span
                                title={`${state.incidentCount} open incident${state.incidentCount === 1 ? "" : "s"}`}
                                className="px-1.5 py-0.5 text-xs font-bold bg-warn-500 text-dark-900 rounded-full min-w-[20px] text-center"
                              >
                                {badgeCount(state.incidentCount)}
                              </span>
                            )}
                          </span>
                        ) : isActive ? (
                          <span className="ml-auto blinking-cursor text-xs">_</span>
                        ) : null}
                      </>
                    )}
                  </NavLink>
                ))}
              </div>
            </div>
          ))
        )}
      </nav>

      {/* User + Status (in sidebar footer when no header) */}
      {!showHeader && (
        <div className="px-3 py-3 border-t border-dark-600/50">
          <div className="flex items-center gap-3 px-3 py-2 rounded-lg hover:bg-dark-800/50 transition-colors group">
            <div className="w-8 h-8 rounded-full bg-rust-500/15 flex items-center justify-center shrink-0">
              <span className="text-xs font-bold text-rust-400 uppercase">{state.user.email?.charAt(0) || "?"}</span>
            </div>
            <div className="flex-1 min-w-0">
              <p className="text-sm font-medium truncate">{state.user.email}</p>
              <p className="text-[11px] text-dark-400 capitalize">{state.user.role}</p>
            </div>
            <button
              onClick={state.logout}
              className="p-1.5 text-dark-400 hover:text-dark-100 rounded-lg transition-colors opacity-0 group-hover:opacity-100"
              title="Logout"
              aria-label="Logout"
            >
              <Icon name="logout" className="w-4 h-4" />
            </button>
          </div>
          <div className="flex items-center justify-between px-3 mt-2">
            <div className="flex items-center gap-2">
              {state.canSeeHealth && (
                <>
                  <div className={`w-1.5 h-1.5 rounded-full shrink-0 ${state.apiHealthy === null ? "bg-dark-400" : state.apiHealthy ? "bg-rust-500" : "bg-danger-500 animate-pulse"}`} />
                  <span className="text-[10px] text-dark-400">{state.apiHealthy === null ? "Checking..." : state.apiHealthy ? "Connected" : "Disconnected"}</span>
                </>
              )}
            </div>
            <div className="flex items-center gap-1">
              <Link to="/notifications" className="relative p-1.5 text-dark-400 hover:text-dark-200 transition-colors rounded" title={state.notifCount > 0 ? `${state.notifCount} unread notification${state.notifCount === 1 ? "" : "s"}` : "Notifications"} aria-label="Notifications">
                <svg className="w-3.5 h-3.5" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={1.5}>
                  <path strokeLinecap="round" strokeLinejoin="round" d="M14.857 17.082a23.848 23.848 0 005.454-1.31A8.967 8.967 0 0118 9.75v-.7V9A6 6 0 006 9v.75a8.967 8.967 0 01-2.312 6.022c1.733.64 3.56 1.085 5.455 1.31m5.714 0a24.255 24.255 0 01-5.714 0m5.714 0a3 3 0 11-5.714 0" />
                </svg>
                {state.notifCount > 0 && (
                  <span className="absolute -top-0.5 -right-0.5 w-4 h-4 bg-danger-500 text-white text-[10px] font-bold rounded-full flex items-center justify-center">
                    {badgeCount(state.notifCount)}
                  </span>
                )}
              </Link>
              <LayoutSwitcher variant={isLight ? "light" : "dark"} />
              <button
                onClick={state.cycleTheme}
                className="p-1.5 text-dark-400 hover:text-dark-200 transition-colors rounded"
                title={`Theme: ${state.theme}`}
                aria-label="Cycle theme"
              >
                <svg className="w-3.5 h-3.5" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={1.5}>
                  <path strokeLinecap="round" strokeLinejoin="round" d="M4.098 19.902a3.75 3.75 0 0 0 5.304 0l6.401-6.402M6.75 21A3.75 3.75 0 0 1 3 17.25V4.125C3 3.504 3.504 3 4.125 3h5.25c.621 0 1.125.504 1.125 1.125v4.072M6.75 21a3.75 3.75 0 0 0 3.75-3.75V8.197M6.75 21h13.125c.621 0 1.125-.504 1.125-1.125v-5.25c0-.621-.504-1.125-1.125-1.125h-4.072M10.5 8.197l2.88-2.88c.438-.439 1.15-.439 1.59 0l3.712 3.713c.44.44.44 1.152 0 1.59l-2.88 2.88M6.75 17.25h.008v.008H6.75v-.008Z" />
                </svg>
              </button>
            </div>
          </div>

          {/* Version & Update Indicator */}
          <div className="px-3 pt-2 mt-2 border-t border-dark-700/40 flex items-center justify-between text-[11px]">
            <span className="font-mono text-dark-300 font-medium">Re-dockpanel v{PANEL_VERSION}</span>
            <span className="inline-flex items-center gap-1 text-[10px] text-rust-400 bg-rust-500/10 px-1.5 py-0.5 rounded font-mono">
              <span className="w-1.5 h-1.5 rounded-full bg-rust-400" />
              Up to date
            </span>
          </div>
        </div>
      )}

      {/* Minimal sidebar footer when header is shown */}
      {showHeader && (
        <div className="px-4 py-2.5 border-t border-dark-600/50 flex items-center justify-between">
          {state.canSeeHealth && (
            <div className="flex items-center gap-2">
              <div className={`w-2 h-2 rounded-full shrink-0 ${state.apiHealthy === null ? "bg-dark-400" : state.apiHealthy ? "bg-rust-500" : "bg-danger-500 animate-pulse"}`} />
              <span className="text-[10px] text-dark-400">{state.apiHealthy === null ? "Checking..." : state.apiHealthy ? "All Systems OK" : "Issues Detected"}</span>
            </div>
          )}
          <span className="text-[10px] font-mono text-dark-400">v{PANEL_VERSION}</span>
        </div>
      )}
    </aside>
  );
}
