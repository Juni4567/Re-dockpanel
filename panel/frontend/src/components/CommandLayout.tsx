import { Navigate, Outlet, NavLink, Link, useNavigate } from "react-router-dom";
import { useLayoutState, badgeCount } from "../hooks/useLayoutState";
import { useServer } from "../context/ServerContext";
import { useBranding } from "../context/BrandingContext";
import { Icon } from "../data/icons";
import CommandPalette from "./CommandPalette";
import LayoutSwitcher from "./LayoutSwitcher";
import Sidebar from "./Sidebar";
import { PANEL_VERSION } from "../constants";
import { useState, useRef, useEffect } from "react";
import { GlobalNotificationBanner } from "../context/NotificationContext";

export default function CommandLayout() {
  const state = useLayoutState();
  const branding = useBranding();
  const isLight = state.theme === "clean" || state.theme === "arctic";

  // Layout options (persisted in localStorage)
  const [showHeader, setShowHeader] = useState(() => localStorage.getItem("dp-show-header") === "true");
  const [flatNav, setFlatNav] = useState(() => localStorage.getItem("dp-flat-nav") === "true");

  // Expose setters for Settings page
  useEffect(() => {
    const handler = () => {
      setShowHeader(localStorage.getItem("dp-show-header") === "true");
      setFlatNav(localStorage.getItem("dp-flat-nav") === "true");
    };
    window.addEventListener("dp-layout-options-change", handler);
    return () => window.removeEventListener("dp-layout-options-change", handler);
  }, []);

  if (state.loading) {
    return (
      <div className="flex items-center justify-center h-screen bg-dark-900">
        <div className="w-8 h-8 border-4 border-rust-500 border-t-transparent rounded-full animate-spin" />
      </div>
    );
  }

  if (!state.user.email) return <Navigate to="/login" replace />;

  return (
    <div className="flex h-screen bg-dark-900">
      <CommandPalette />

      {/* Skip to content */}
      <a href="#main-content" className="sr-only focus:not-sr-only focus:absolute focus:z-[100] focus:top-2 focus:left-2 focus:px-4 focus:py-2 focus:bg-accent-600 focus:text-dark-50 focus:rounded-lg">
        Skip to main content
      </a>

      {/* Mobile overlay */}
      {state.sidebarOpen && (
        <div
          className="fixed inset-0 bg-black/50 z-40 md:hidden"
          role="presentation"
          onClick={() => state.setSidebarOpen(false)}
        />
      )}

      {/* Sidebar */}
      <Sidebar showHeader={showHeader} flatNav={flatNav} />

      {/* Main content */}
      <div className={`flex-1 min-w-0 flex flex-col ${showHeader ? "h-screen overflow-hidden" : "overflow-hidden"}`}>
        {/* Top header bar (optional, replaces controls from sidebar footer) */}
        {showHeader && (
          <header className="h-16 border-b flex items-center justify-between px-6 shrink-0 bg-dark-900 border-dark-700">
            {/* Mobile hamburger */}
            <button
              className="p-2 md:hidden rounded-lg text-dark-400 hover:text-dark-50"
              onClick={() => state.setSidebarOpen(true)}
              aria-label="Open menu"
            >
              <svg className="w-5 h-5" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}><path strokeLinecap="round" strokeLinejoin="round" d="M3.75 6.75h16.5M3.75 12h16.5m-16.5 5.25h16.5" /></svg>
            </button>

            {/* Search */}
            <div className="flex-1 flex items-center max-w-md">
              <button
                onClick={() => window.dispatchEvent(new KeyboardEvent("keydown", { key: "k", ctrlKey: true }))}
                className="w-full flex items-center gap-2 pl-3 pr-4 py-2 border rounded-lg text-sm transition-all bg-dark-950 border-dark-700 text-dark-400 hover:border-rust-500/50"
              >
                <svg className="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}><circle cx="11" cy="11" r="8" /><line x1="21" y1="21" x2="16.65" y2="16.65" /></svg>
                <span className="flex-1 text-left">Search...</span>
                <kbd className="text-[10px] px-1.5 py-0.5 border rounded border-dark-700 bg-dark-800 text-dark-400">Ctrl K</kbd>
              </button>
            </div>

            {/* Right side */}
            <div className="flex items-center gap-3">
              <Link to="/notifications" className="relative p-2 rounded-lg transition-colors text-dark-400 hover:text-dark-50 hover:bg-dark-800" title={state.notifCount > 0 ? `${state.notifCount} unread notification${state.notifCount === 1 ? "" : "s"}` : "Notifications"} aria-label="Notifications">
                <svg className="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={1.5}>
                  <path strokeLinecap="round" strokeLinejoin="round" d="M14.857 17.082a23.848 23.848 0 005.454-1.31A8.967 8.967 0 0118 9.75v-.7V9A6 6 0 006 9v.75a8.967 8.967 0 01-2.312 6.022c1.733.64 3.56 1.085 5.455 1.31m5.714 0a24.255 24.255 0 01-5.714 0m5.714 0a3 3 0 11-5.714 0" />
                </svg>
                {state.notifCount > 0 && (
                  <span className="absolute top-0.5 right-0.5 w-4 h-4 bg-danger-500 text-white text-[10px] font-bold rounded-full flex items-center justify-center">
                    {badgeCount(state.notifCount)}
                  </span>
                )}
              </Link>
              {/* This pill sat next to the notification bell drawing the SAME
                  bell glyph over a different number, so the header showed two
                  identical icons meaning unrelated things. A firing alert is not
                  a notification; it gets the alert glyph and says what it is. */}
              {state.firingCount > 0 && (
                <Link
                  to="/monitoring?tab=alerts"
                  title={`${state.firingCount} firing alert${state.firingCount === 1 ? "" : "s"}`}
                  className="flex items-center gap-1.5 px-2.5 py-1.5 text-xs font-bold bg-danger-500/15 text-danger-400 rounded-lg hover:bg-danger-500/25 transition-colors"
                >
                  <svg className="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={1.5}><path strokeLinecap="round" strokeLinejoin="round" d="M12 9v3.75m-9.303 3.376c-.866 1.5.217 3.374 1.948 3.374h14.71c1.73 0 2.813-1.874 1.948-3.374L13.949 3.378c-.866-1.5-3.032-1.5-3.898 0L2.697 16.126zM12 15.75h.007v.008H12v-.008z" /></svg>
                  {badgeCount(state.firingCount)} firing
                </Link>
              )}
              <div className="h-6 w-px hidden sm:block bg-dark-700" />
              <button onClick={state.cycleTheme} className="p-2 rounded-lg transition-colors text-dark-400 hover:text-dark-50 hover:bg-dark-800" title={`Theme: ${state.theme}`}>
                <svg className="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={1.5}>
                  <path strokeLinecap="round" strokeLinejoin="round" d="M4.098 19.902a3.75 3.75 0 0 0 5.304 0l6.401-6.402M6.75 21A3.75 3.75 0 0 1 3 17.25V4.125C3 3.504 3.504 3 4.125 3h5.25c.621 0 1.125.504 1.125 1.125v4.072M6.75 21a3.75 3.75 0 0 0 3.75-3.75V8.197M6.75 21h13.125c.621 0 1.125-.504 1.125-1.125v-5.25c0-.621-.504-1.125-1.125-1.125h-4.072M10.5 8.197l2.88-2.88c.438-.439 1.15-.439 1.59 0l3.712 3.713c.44.44.44 1.152 0 1.59l-2.88 2.88M6.75 17.25h.008v.008H6.75v-.008Z" />
                </svg>
              </button>
              <div className="hidden sm:block"><LayoutSwitcher variant={isLight ? "light" : "dark"} /></div>
              <div className="hidden sm:flex items-center gap-2">
                <div className="w-8 h-8 rounded-full flex items-center justify-center font-semibold text-sm bg-rust-500/15 text-rust-400">
                  {state.user.email[0]?.toUpperCase()}
                </div>
                <div className="text-left">
                  <p className="text-sm font-medium leading-none text-dark-100">{state.user.email.split("@")[0]}</p>
                  <p className="text-xs mt-0.5 text-dark-400">{state.user.role}</p>
                </div>
              </div>
              <button onClick={state.logout} className="p-1.5 text-dark-400 hover:text-dark-100 rounded-lg" title="Logout"><Icon name="logout" className="w-4 h-4" /></button>
            </div>
          </header>
        )}

        {/* Mobile header (only when no top header) */}
        {!showHeader && (
          <div className="sticky top-0 z-30 flex items-center gap-3 px-4 py-3 bg-dark-900/80 backdrop-blur-lg border-b border-dark-600 md:hidden">
            <button onClick={() => state.setSidebarOpen(true)} className="p-2 text-dark-200 hover:text-dark-50 hover:bg-dark-700 rounded-lg" aria-label="Open sidebar">
              <svg className="w-6 h-6" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}><path strokeLinecap="round" strokeLinejoin="round" d="M3.75 6.75h16.5M3.75 12h16.5m-16.5 5.25h16.5" /></svg>
            </button>
            <span className="text-base font-bold logo-glow">
              {branding.hideBranding ? "" : branding.panelName === "DockPanel" ? <><span className="text-rust-400">Dock</span><span className="text-dark-50">Panel</span></> : <span className="text-dark-50">{branding.panelName}</span>}
            </span>
          </div>
        )}

        {/* 2FA warning */}
        {state.twoFaEnforced && !state.twoFaEnabled && (
          <div className={`border-b px-4 py-3 flex items-center justify-between ${isLight ? "bg-warn-500/5 border-warn-500/20" : "bg-warn-500/10 border-warn-500/20"}`}>
            <div className="flex items-center gap-2">
              <svg className="w-5 h-5 text-warn-400" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={1.5}>
                <path strokeLinecap="round" strokeLinejoin="round" d="M12 9v3.75m-9.303 3.376c-.866 1.5.217 3.374 1.948 3.374h14.71c1.73 0 2.813-1.874 1.948-3.374L13.949 3.378c-.866-1.5-3.032-1.5-3.898 0L2.697 16.126ZM12 15.75h.007v.008H12v-.008Z" />
              </svg>
              <span className="text-sm text-warn-400 font-medium">Two-factor authentication is required. Enable it from My Account.</span>
            </div>
            <Link to="/account" className="px-3 py-1.5 bg-warn-500 text-dark-900 rounded text-xs font-bold hover:bg-warn-400 transition-colors">Set Up 2FA</Link>
          </div>
        )}

        <main id="main-content" className={`flex-1 ${showHeader ? "overflow-y-auto bg-dark-950" : "overflow-auto"}`}>
          <GlobalNotificationBanner />
          <Outlet />
        </main>
      </div>
    </div>
  );
}
