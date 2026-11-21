import { createContext, useContext, useState, useEffect, ReactNode, useCallback } from "react";
import { useLocation } from "react-router-dom";

export type NotificationType = "success" | "error" | "warn" | "info";

export interface GlobalNotification {
  id: string;
  type: NotificationType;
  message: string;
  title?: string;
  details?: string;
}

interface NotificationContextValue {
  notification: GlobalNotification | null;
  notify: {
    success: (message: string, title?: string, details?: string) => void;
    error: (message: string, title?: string, details?: string) => void;
    warn: (message: string, title?: string, details?: string) => void;
    info: (message: string, title?: string, details?: string) => void;
    clear: () => void;
  };
}

const NotificationContext = createContext<NotificationContextValue | null>(null);

/**
 * Dispatch a notification from anywhere in the application (even outside React).
 */
export function showGlobalNotification(type: NotificationType, message: string, title?: string, details?: string) {
  window.dispatchEvent(
    new CustomEvent("dp:global-notify", {
      detail: { type, message, title, details },
    })
  );
}

export function NotificationProvider({ children }: { children: ReactNode }) {
  const [notification, setNotification] = useState<GlobalNotification | null>(null);
  const location = useLocation();

  // Permanent notification: automatically dismissed when navigating to other pages
  useEffect(() => {
    setNotification(null);
  }, [location.pathname]);

  // Handle custom window events
  useEffect(() => {
    const handler = (e: Event) => {
      const custom = e as CustomEvent<{ type: NotificationType; message: string; title?: string; details?: string }>;
      if (custom.detail) {
        setNotification({
          id: Math.random().toString(36).slice(2),
          type: custom.detail.type,
          message: custom.detail.message,
          title: custom.detail.title,
          details: custom.detail.details,
        });
      }
    };
    window.addEventListener("dp:global-notify", handler);
    return () => window.removeEventListener("dp:global-notify", handler);
  }, []);

  const notify = {
    success: useCallback((message: string, title?: string, details?: string) => {
      setNotification({
        id: Math.random().toString(36).slice(2),
        type: "success",
        message,
        title,
        details,
      });
    }, []),
    error: useCallback((message: string, title?: string, details?: string) => {
      setNotification({
        id: Math.random().toString(36).slice(2),
        type: "error",
        message,
        title,
        details,
      });
    }, []),
    warn: useCallback((message: string, title?: string, details?: string) => {
      setNotification({
        id: Math.random().toString(36).slice(2),
        type: "warn",
        message,
        title,
        details,
      });
    }, []),
    info: useCallback((message: string, title?: string, details?: string) => {
      setNotification({
        id: Math.random().toString(36).slice(2),
        type: "info",
        message,
        title,
        details,
      });
    }, []),
    clear: useCallback(() => {
      setNotification(null);
    }, []),
  };

  return (
    <NotificationContext.Provider value={{ notification, notify }}>
      {children}
    </NotificationContext.Provider>
  );
}

export function useNotify() {
  const ctx = useContext(NotificationContext);
  if (!ctx) {
    return {
      notification: null,
      notify: {
        success: (msg: string, title?: string, details?: string) => showGlobalNotification("success", msg, title, details),
        error: (msg: string, title?: string, details?: string) => showGlobalNotification("error", msg, title, details),
        warn: (msg: string, title?: string, details?: string) => showGlobalNotification("warn", msg, title, details),
        info: (msg: string, title?: string, details?: string) => showGlobalNotification("info", msg, title, details),
        clear: () => {},
      },
    };
  }
  return ctx;
}

/**
 * The user-facing Notification Banner rendered at the top of page layouts.
 * Stays permanently until explicitly closed by clicking [X] or until navigating away.
 */
export function GlobalNotificationBanner() {
  const { notification, notify } = useNotify();
  const [showDetails, setShowDetails] = useState(false);

  if (!notification) return null;

  const colorStyles = {
    success: {
      container: "bg-rust-950/80 border-rust-500/50 text-rust-200 shadow-rust-900/20",
      iconColor: "text-rust-400",
      badge: "bg-rust-500/20 text-rust-300 border-rust-500/30",
      icon: (
        <svg className="w-5 h-5 flex-shrink-0" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth="2.5">
          <path strokeLinecap="round" strokeLinejoin="round" d="m4.5 12.75 6 6 9-13.5" />
        </svg>
      ),
    },
    error: {
      container: "bg-danger-950/80 border-danger-500/50 text-danger-200 shadow-danger-900/20",
      iconColor: "text-danger-400",
      badge: "bg-danger-500/20 text-danger-300 border-danger-500/30",
      icon: (
        <svg className="w-5 h-5 flex-shrink-0" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth="2">
          <path strokeLinecap="round" strokeLinejoin="round" d="M12 9v3.75m-9.303 3.376c-.866 1.5.217 3.374 1.948 3.374h14.71c1.73 0 2.813-1.874 1.948-3.374L13.949 3.378c-.866-1.5-3.032-1.5-3.898 0L2.697 16.126ZM12 15.75h.007v.008H12v-.008Z" />
        </svg>
      ),
    },
    warn: {
      container: "bg-warn-950/80 border-warn-500/50 text-warn-200 shadow-warn-900/20",
      iconColor: "text-warn-400",
      badge: "bg-warn-500/20 text-warn-300 border-warn-500/30",
      icon: (
        <svg className="w-5 h-5 flex-shrink-0" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth="2">
          <path strokeLinecap="round" strokeLinejoin="round" d="M12 9v3.75m9-.75a9 9 0 1 1-18 0 9 9 0 0 1 18 0Zm-9 3.75h.008v.008H12v-.008Z" />
        </svg>
      ),
    },
    info: {
      container: "bg-sky-950/80 border-sky-500/50 text-sky-200 shadow-sky-900/20",
      iconColor: "text-sky-400",
      badge: "bg-sky-500/20 text-sky-300 border-sky-500/30",
      icon: (
        <svg className="w-5 h-5 flex-shrink-0" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth="2">
          <path strokeLinecap="round" strokeLinejoin="round" d="m11.25 11.25.041-.02a.75.75 0 0 1 1.063.852l-.708 2.836a.75.75 0 0 0 1.063.853l.041-.021M21 12a9 9 0 1 1-18 0 9 9 0 0 1 18 0Zm-9-3.75h.008v.008H12V8.25Z" />
        </svg>
      ),
    },
  }[notification.type];

  return (
    <div className="mx-6 lg:mx-8 mt-4 mb-2 animate-fade-up">
      <div className={`flex items-start justify-between gap-3 p-4 rounded-xl border backdrop-blur-md shadow-lg ${colorStyles.container}`}>
        <div className="flex items-start gap-3 min-w-0 flex-1">
          <div className={`mt-0.5 ${colorStyles.iconColor}`}>
            {colorStyles.icon}
          </div>
          <div className="min-w-0 flex-1">
            <div className="flex items-center gap-2 flex-wrap">
              <span className={`text-[10px] uppercase font-bold tracking-wider px-2 py-0.5 rounded-full border ${colorStyles.badge}`}>
                {notification.title || (notification.type === "success" ? "Action Successful" : notification.type === "error" ? "Action Failed" : "Notice")}
              </span>
            </div>
            <p className="text-sm font-medium mt-1 break-words">
              {notification.message}
            </p>
            {notification.details && (
              <div className="mt-2">
                <button
                  type="button"
                  onClick={() => setShowDetails(!showDetails)}
                  className="text-xs font-mono underline opacity-80 hover:opacity-100 transition-opacity"
                >
                  {showDetails ? "▲ Hide Technical Details" : "▼ Show Technical Details"}
                </button>
                {showDetails && (
                  <pre className="mt-2 p-3 bg-dark-900/90 text-dark-200 border border-dark-700/60 rounded-lg text-xs font-mono overflow-x-auto max-h-48 whitespace-pre-wrap">
                    {notification.details}
                  </pre>
                )}
              </div>
            )}
          </div>
        </div>
        <button
          type="button"
          onClick={() => notify.clear()}
          className="p-1 rounded-lg text-current opacity-70 hover:opacity-100 hover:bg-white/10 transition-all flex-shrink-0"
          title="Dismiss notification"
          aria-label="Dismiss notification"
        >
          <svg className="w-5 h-5" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth="2">
            <path strokeLinecap="round" strokeLinejoin="round" d="M6 18 18 6M6 6l12 12" />
          </svg>
        </button>
      </div>
    </div>
  );
}
