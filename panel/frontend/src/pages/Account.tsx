import { useAuth } from "../context/AuthContext";
import { isDesktopApp } from "../utils/desktop";
import {
  TwoFactorCard,
  PasskeysCard,
  ChangePasswordCard,
  SessionsCard,
  ApiKeysCard,
  ExportMyDataCard,
} from "../components/AccountSecurity";

export default function Account() {
  const { user } = useAuth();
  const desktop = isDesktopApp();

  return (
    <div>
      <div className="page-header">
        <div>
          <h1 className="page-header-title">My Account</h1>
          <p className="page-header-subtitle">
            {user?.email || "Your sign-in, security and API access"}
            {user?.role && (
              <span className="ml-2 px-1.5 py-0.5 bg-dark-700 border border-dark-600 rounded text-[10px] font-medium uppercase tracking-wider text-dark-300">
                {user.role}
              </span>
            )}
          </p>
        </div>
      </div>

      <div className="p-6 lg:p-8">
        {desktop ? (
          <div className="bg-dark-800 border border-dark-500 rounded-lg p-6 max-w-2xl space-y-3">
            <h2 className="text-sm font-semibold text-dark-50 font-mono uppercase tracking-wider">
              Local Workstation Session
            </h2>
            <p className="text-sm text-dark-200 leading-relaxed">
              You are signed in automatically as{" "}
              <span className="font-mono text-rust-400">{user?.email || "local@dockpanel.desktop"}</span>{" "}
              in DockPanel Desktop. Multi-user authentication, 2FA, passkeys, SSH sessions, and API
              tokens are only used on remote Linux server panels.
            </p>
          </div>
        ) : (
          <div className="space-y-6">
            <TwoFactorCard />
            <PasskeysCard />
            <ChangePasswordCard />
            <SessionsCard />
            <ApiKeysCard />
            <ExportMyDataCard />
          </div>
        )}
      </div>
    </div>
  );
}
