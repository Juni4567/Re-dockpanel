import { useState, FormEvent } from "react";
import { api } from "../api";
import { showGlobalNotification } from "../context/NotificationContext";

export interface InstallWordPressModalProps {
  isOpen: boolean;
  onClose: () => void;
  siteId: string;
  domain: string;
  onSuccess?: () => void;
}

export function generateRandomPassword(length = 16): string {
  const chars = "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789!@#$%^&*()-_=+";
  let pass = "";
  for (let i = 0; i < length; i++) {
    pass += chars.charAt(Math.floor(Math.random() * chars.length));
  }
  return pass;
}

export default function InstallWordPressModal({
  isOpen,
  onClose,
  siteId,
  domain,
  onSuccess,
}: InstallWordPressModalProps) {
  const [title, setTitle] = useState("My WordPress Site");
  const [adminUser, setAdminUser] = useState("admin");
  const [adminPass, setAdminPass] = useState(() => generateRandomPassword(16));
  const [adminEmail, setAdminEmail] = useState(() => `admin@${domain}`);
  const [showPassword, setShowPassword] = useState(false);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState("");

  if (!isOpen) return null;

  const handleSubmit = async (e: FormEvent) => {
    e.preventDefault();
    if (!adminUser.trim()) {
      setError("Admin username is required.");
      return;
    }
    if (!adminPass) {
      setError("Admin password is required.");
      return;
    }

    setError("");
    setLoading(true);

    try {
      await api.post(`/sites/${siteId}/wordpress/install`, {
        title: title.trim() || "My WordPress Site",
        admin_user: adminUser.trim(),
        admin_pass: adminPass,
        admin_email: adminEmail.trim() || `admin@${domain}`,
        url: `http://${domain}`,
      });

      showGlobalNotification("success", `WordPress successfully installed for ${domain}!`);
      if (onSuccess) onSuccess();
      onClose();
    } catch (err) {
      setError(err instanceof Error ? err.message : "Failed to install WordPress");
    } finally {
      setLoading(false);
    }
  };

  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center bg-black/60 backdrop-blur-sm p-4 overflow-y-auto"
      role="dialog"
      aria-modal="true"
      aria-labelledby="install-wp-title"
    >
      <div className="bg-dark-900 border border-dark-600 rounded-xl shadow-2xl w-full max-w-md p-6 relative">
        <div className="flex items-center justify-between pb-4 border-b border-dark-700">
          <div className="flex items-center gap-3">
            <div className="w-9 h-9 rounded-lg bg-[#0073aa]/20 border border-[#0073aa]/40 flex items-center justify-center text-[#38bdf8]">
              <svg className="w-5 h-5" viewBox="0 0 24 24" fill="currentColor">
                <path d="M12 2C6.486 2 2 6.486 2 12s4.486 10 10 10 10-4.486 10-10S17.514 2 12 2zm0 2c1.67 0 3.214.52 4.488 1.401L5.401 16.488A7.957 7.957 0 0 1 4 12c0-4.411 3.589-8 8-8zm0 16c-1.67 0-3.214-.52-4.488-1.401L18.599 7.512A7.957 7.957 0 0 1 20 12c0 4.411-3.589 8-8 8z" />
              </svg>
            </div>
            <div>
              <h2 id="install-wp-title" className="text-lg font-semibold text-dark-50">
                Install WordPress
              </h2>
              <p className="text-xs text-dark-400">{domain}</p>
            </div>
          </div>
          <button
            onClick={onClose}
            disabled={loading}
            className="text-dark-400 hover:text-dark-200 p-1.5 rounded-lg hover:bg-dark-800 transition-colors"
            aria-label="Close"
          >
            <svg className="w-5 h-5" fill="none" viewBox="0 0 24 24" stroke="currentColor">
              <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M6 18L18 6M6 6l12 12" />
            </svg>
          </button>
        </div>

        {error && (
          <div className="mt-4 p-3 bg-danger-500/10 border border-danger-500/30 rounded-lg text-danger-400 text-xs">
            {error}
          </div>
        )}

        <form onSubmit={handleSubmit} className="mt-4 space-y-4">
          <div>
            <label className="block text-xs font-medium text-dark-300 mb-1">Site Title</label>
            <input
              type="text"
              value={title}
              onChange={(e) => setTitle(e.target.value)}
              className="w-full px-3 py-2 bg-dark-800 border border-dark-600 rounded-lg text-sm text-dark-50 focus:outline-none focus:border-accent-500"
              placeholder="My WordPress Site"
              disabled={loading}
            />
          </div>

          <div>
            <label className="block text-xs font-medium text-dark-300 mb-1">Admin Username</label>
            <input
              type="text"
              value={adminUser}
              onChange={(e) => setAdminUser(e.target.value)}
              className="w-full px-3 py-2 bg-dark-800 border border-dark-600 rounded-lg text-sm text-dark-50 focus:outline-none focus:border-accent-500"
              placeholder="admin"
              required
              disabled={loading}
            />
          </div>

          <div>
            <div className="flex items-center justify-between mb-1">
              <label className="block text-xs font-medium text-dark-300">Admin Password</label>
              <button
                type="button"
                onClick={() => setAdminPass(generateRandomPassword(16))}
                className="text-[11px] text-accent-400 hover:text-accent-300 transition-colors"
                disabled={loading}
              >
                Generate strong password
              </button>
            </div>
            <div className="relative">
              <input
                type={showPassword ? "text" : "password"}
                value={adminPass}
                onChange={(e) => setAdminPass(e.target.value)}
                className="w-full px-3 py-2 pr-10 bg-dark-800 border border-dark-600 rounded-lg text-sm text-dark-50 focus:outline-none focus:border-accent-500 font-mono"
                required
                disabled={loading}
              />
              <button
                type="button"
                onClick={() => setShowPassword(!showPassword)}
                className="absolute right-2 top-1/2 -translate-y-1/2 text-dark-400 hover:text-dark-200 text-xs p-1"
                aria-label={showPassword ? "Hide password" : "Show password"}
                disabled={loading}
              >
                {showPassword ? "Hide" : "Show"}
              </button>
            </div>
          </div>

          <div>
            <label className="block text-xs font-medium text-dark-300 mb-1">Admin Email</label>
            <input
              type="email"
              value={adminEmail}
              onChange={(e) => setAdminEmail(e.target.value)}
              className="w-full px-3 py-2 bg-dark-800 border border-dark-600 rounded-lg text-sm text-dark-50 focus:outline-none focus:border-accent-500"
              placeholder="admin@example.com"
              required
              disabled={loading}
            />
          </div>

          <div className="pt-2 flex justify-end gap-3 border-t border-dark-700">
            <button
              type="button"
              onClick={onClose}
              disabled={loading}
              className="px-4 py-2 rounded-lg text-sm font-medium text-dark-300 hover:bg-dark-800 transition-colors"
            >
              Cancel
            </button>
            <button
              type="submit"
              disabled={loading}
              className="px-4 py-2 rounded-lg text-sm font-medium bg-[#0073aa] hover:bg-[#005a87] text-white transition-colors flex items-center gap-2"
            >
              {loading ? (
                <>
                  <div className="w-4 h-4 border-2 border-white/30 border-t-white rounded-full animate-spin" />
                  <span>Installing...</span>
                </>
              ) : (
                <span>Install WordPress</span>
              )}
            </button>
          </div>
        </form>
      </div>
    </div>
  );
}
