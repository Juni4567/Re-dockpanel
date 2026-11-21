//! Systemd service management for Node.js/Python app processes.
//!
//! Creates a per-site systemd service that runs the app, proxied by nginx.

use crate::safe_cmd::safe_command_sync;

use super::command_filter;

const SERVICE_PREFIX: &str = "dockpanel-app-";

/// Service unit name for a domain.
fn service_name(domain: &str) -> String {
    format!("{SERVICE_PREFIX}{}", domain.replace('.', "-"))
}

/// The unit name to act on for `domain` — but only when the unit on disk says
/// it is the one running `domain`.
///
/// `service_name` collapses `.` to `-`, and `-` is legal inside a domain label,
/// so `a.b.com` and `a-b.com` are separately claimable domains that land on the
/// same unit. v2.53.0 guarded the create and remove paths with exactly this
/// question and left the *lifecycle* callers behind: disabling a site, enabling
/// it, or saving its `.env` each ran `systemctl stop`/`restart` on the collided
/// name. Stopping a neighbour's app is not as loud as deleting its unit file,
/// but it is the same outage, reachable by any tenant on their own site.
///
/// `None` when there is nothing to act on OR when the unit belongs to someone
/// else — the caller does nothing either way, which is why one return value
/// serves both.
pub fn owned_service_name(domain: &str) -> Option<String> {
    let svc = service_name(domain);
    let unit_path = format!("/etc/systemd/system/{svc}.service");
    if !std::path::Path::new(&unit_path).exists() {
        return None;
    }
    if !crate::services::ownership::systemd_unit(&unit_path, domain).may_delete() {
        tracing::warn!(
            "Not touching {svc}: {unit_path} does not run {domain}. The unit name collapses \
             '.' to '-', so this unit belongs to a different domain."
        );
        return None;
    }
    Some(svc)
}

/// Create and start a systemd service for an app.
pub fn create_app_service(
    domain: &str,
    command: &str,
    port: u16,
    runtime: &str,
) -> Result<(), String> {
    // Validate the command before doing anything else
    command_filter::is_safe_exec_start(command, runtime)?;

    let svc = service_name(domain);
    let site_dir = format!("/var/www/{domain}");

    // A managed process has to start where its code is. `document_root_for` is the
    // one place that knows where that is per runtime — node/python/proxy keep the
    // site root, static and php nest under `public/` — and it is what the vhost,
    // the git clone and `EnvironmentFile` below all already agree on.
    //
    // This used to hardcode `{site_dir}/public` for every runtime, and this
    // function only ever runs for node and python (`routes/nginx.rs`, the
    // node|python branch). So every managed process was started in a directory the
    // panel itself created EMPTY two lines before `systemctl enable --now`.
    // `node server.js` (rc=1, MODULE_NOT_FOUND) and `python3 app.py` (rc=2) both
    // resolve their entry point against the cwd, and `Restart=always` below turns
    // that into a permanent crash loop. `npm start` is the exception that hid it
    // for so long: npm walks up to the nearest package.json and re-roots the cwd
    // there, and it is the first of the three examples the site form offers.
    let working_dir = crate::services::nginx::document_root_for(&site_dir, runtime);

    // Determine the ExecStart based on runtime
    let exec_start = match runtime {
        "node" => {
            // Check if it looks like a bare command (e.g., "server.js") vs full command
            if command.starts_with("node ")
                || command.starts_with("npm ")
                || command.starts_with("npx ")
                || command.starts_with("yarn ")
                || command.starts_with("pnpm ")
                || command.starts_with("/")
            {
                command.to_string()
            } else {
                format!("node {command}")
            }
        }
        "python" => {
            if command.starts_with("python")
                || command.starts_with("gunicorn")
                || command.starts_with("uvicorn")
                || command.starts_with("flask")
                || command.starts_with("django")
                || command.starts_with("/")
            {
                command.to_string()
            } else {
                format!("python3 {command}")
            }
        }
        _ => command.to_string(),
    };

    let unit = format!(
        r#"[Unit]
Description=DockPanel App: {domain}
After=network.target

[Service]
Type=simple
User=www-data
Group=www-data
WorkingDirectory={working_dir}
ExecStart={exec_start}
Restart=always
RestartSec=5
Environment=PORT={port}
Environment=NODE_ENV=production
Environment=HOST=0.0.0.0
EnvironmentFile=-/var/www/{domain}/.env

# Resource limits
MemoryMax=512M
CPUQuota=100%
LimitNOFILE=65536
TasksMax=512

# Security hardening
NoNewPrivileges=true
ProtectSystem=strict
ProtectHome=true
PrivateTmp=true
PrivateDevices=true
ProtectKernelTunables=true
ProtectKernelModules=true
ProtectControlGroups=true
RestrictSUIDSGID=true
RestrictNamespaces=true
RestrictRealtime=true
LockPersonality=true
SystemCallArchitectures=native
CapabilityBoundingSet=CAP_NET_BIND_SERVICE
SystemCallFilter=@system-service
SystemCallErrorNumber=EPERM
ProtectProc=invisible
RestrictAddressFamilies=AF_UNIX AF_INET AF_INET6 AF_NETLINK
ReadWritePaths=/var/www/{domain}

# Logging
StandardOutput=journal
StandardError=journal
SyslogIdentifier={svc}

[Install]
WantedBy=multi-user.target
"#
    );

    let unit_path = format!("/etc/systemd/system/{svc}.service");

    // The mirror of the delete-side collision: because `service_name` collapses
    // '.' to '-', writing this unit for `a-b.com` would silently replace the one
    // running `a.b.com` — pointing its ExecStart and WorkingDirectory at another
    // tenant's docroot. Latent rather than immediate (`enable --now` does not
    // restart an already-active unit), which is worse: it surfaces at the next
    // reboot, running the wrong process for the victim's vhost.
    if std::path::Path::new(&unit_path).exists()
        && crate::services::ownership::systemd_unit(&unit_path, domain)
            == crate::services::ownership::Owner::Theirs
    {
        return Err(format!(
            "The systemd unit name for {domain} is already taken by another domain \
             ({unit_path}). Unit names collapse '.' to '-', so these two domains \
             share one name; rename one of them."
        ));
    }

    std::fs::write(&unit_path, &unit)
        .map_err(|e| format!("Failed to write service unit: {e}"))?;

    // Create working directory if it doesn't exist
    std::fs::create_dir_all(&working_dir).ok();
    // Set ownership — everything except `.git`, which stays root's. A git-deployed
    // Node/Python app runs this on every (re)create; handing `.git` to www-data
    // gives the app `config`/`hooks/` that `deploy.rs::clone_or_pull` runs as root
    // on the next deploy. Mirrors `deploy.rs::hand_tree_to_web_user`.
    let site_dir = format!("/var/www/{domain}");
    if let Ok(entries) = std::fs::read_dir(&site_dir) {
        for entry in entries.flatten() {
            if entry.file_name() == std::ffi::OsStr::new(".git") {
                continue;
            }
            let _ = safe_command_sync("chown")
                .args(["-R", "www-data:www-data", &entry.path().to_string_lossy()])
                .output();
        }
    }
    let git_dir = format!("{site_dir}/.git");
    if std::path::Path::new(&git_dir).exists() {
        let _ = safe_command_sync("chown").args(["-R", "root:root", &git_dir]).output();
        let _ = safe_command_sync("chmod").args(["-R", "go-rwx", &git_dir]).output();
    }

    // Reload systemd and enable+start the service
    safe_command_sync("systemctl")
        .args(["daemon-reload"])
        .output()
        .map_err(|e| format!("daemon-reload failed: {e}"))?;

    safe_command_sync("systemctl")
        .args(["enable", "--now", &svc])
        .output()
        .map_err(|e| format!("Failed to start service: {e}"))?;

    tracing::info!("App service created and started: {svc} (port={port}, runtime={runtime})");
    Ok(())
}

/// Stop and remove the systemd service for an app.
pub fn remove_app_service(domain: &str) -> Result<(), String> {
    let svc = service_name(domain);
    let unit_path = format!("/etc/systemd/system/{svc}.service");

    if !std::path::Path::new(&unit_path).exists() {
        return Ok(()); // No service to remove
    }

    // `service_name` maps `.` to `-`, and `-` is legal inside a domain label, so
    // it is NOT injective: `a.b.com` and `a-b.com` are separately claimable
    // domains that both land on `dockpanel-app-a-b-com`. Deleting a site used to
    // stop, disable and unlink the unit at that name whoever wrote it, and no
    // ordinary panel action re-sends `app_command`, so the victim's site stayed
    // down until it was deleted and recreated.
    //
    // Renaming the unit would strand every unit already on disk. The unit itself
    // records who it runs, so ask it.
    if !crate::services::ownership::systemd_unit(&unit_path, domain).may_delete() {
        tracing::warn!(
            "Leaving {unit_path} in place: it does not run {domain}. The unit name \
             collapses '.' to '-', so this file belongs to a different domain."
        );
        return Ok(());
    }

    // Stop and disable
    safe_command_sync("systemctl")
        .args(["stop", &svc])
        .output()
        .ok();
    safe_command_sync("systemctl")
        .args(["disable", &svc])
        .output()
        .ok();

    // Remove unit file
    std::fs::remove_file(&unit_path).ok();

    // Reload systemd
    safe_command_sync("systemctl")
        .args(["daemon-reload"])
        .output()
        .ok();

    tracing::info!("App service removed: {svc}");
    Ok(())
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct AppServiceStatus {
    pub exists: bool,
    pub service_name: Option<String>,
    pub active_state: Option<String>,
    pub sub_state: Option<String>,
    pub pid: Option<u32>,
    pub exit_code: Option<i32>,
    pub memory_bytes: Option<u64>,
    pub cpu_usage_ns: Option<u64>,
    pub started_at: Option<String>,
}

/// Query systemd for the status of an app process
pub fn get_app_service_status(domain: &str) -> AppServiceStatus {
    let svc = match owned_service_name(domain) {
        Some(s) => s,
        None => {
            return AppServiceStatus {
                exists: false,
                service_name: None,
                active_state: None,
                sub_state: None,
                pid: None,
                exit_code: None,
                memory_bytes: None,
                cpu_usage_ns: None,
                started_at: None,
            };
        }
    };

    let output = safe_command_sync("systemctl")
        .args([
            "show",
            &svc,
            "--property=ActiveState,SubState,MainPID,ExecMainStatus,ActiveEnterTimestamp,MemoryCurrent,CPUUsageNSec",
        ])
        .output();

    let text = match output {
        Ok(o) => String::from_utf8_lossy(&o.stdout).to_string(),
        Err(_) => {
            return AppServiceStatus {
                exists: true,
                service_name: Some(svc),
                active_state: Some("unknown".into()),
                sub_state: None,
                pid: None,
                exit_code: None,
                memory_bytes: None,
                cpu_usage_ns: None,
                started_at: None,
            };
        }
    };

    let mut active_state = None;
    let mut sub_state = None;
    let mut pid = None;
    let mut exit_code = None;
    let mut started_at = None;
    let mut memory_bytes = None;
    let mut cpu_usage_ns = None;

    for line in text.lines() {
        if let Some((k, v)) = line.split_once('=') {
            let k = k.trim();
            let v = v.trim();
            match k {
                "ActiveState" => active_state = Some(v.to_string()),
                "SubState" => sub_state = Some(v.to_string()),
                "MainPID" => {
                    if let Ok(p) = v.parse::<u32>() {
                        if p > 0 {
                            pid = Some(p);
                        }
                    }
                }
                "ExecMainStatus" => {
                    if let Ok(code) = v.parse::<i32>() {
                        exit_code = Some(code);
                    }
                }
                "ActiveEnterTimestamp" => {
                    if !v.is_empty() && v != "[not set]" {
                        started_at = Some(v.to_string());
                    }
                }
                "MemoryCurrent" => {
                    if let Ok(m) = v.parse::<u64>() {
                        if m < u64::MAX - 1000 {
                            memory_bytes = Some(m);
                        }
                    }
                }
                "CPUUsageNSec" => {
                    if let Ok(c) = v.parse::<u64>() {
                        cpu_usage_ns = Some(c);
                    }
                }
                _ => {}
            }
        }
    }

    AppServiceStatus {
        exists: true,
        service_name: Some(svc),
        active_state,
        sub_state,
        pid,
        exit_code,
        memory_bytes,
        cpu_usage_ns,
        started_at,
    }
}

/// Control an app service: start, stop, restart
pub fn control_app_service(domain: &str, action: &str) -> Result<AppServiceStatus, String> {
    if !["start", "stop", "restart"].contains(&action) {
        return Err(format!("Invalid action: {action}. Allowed: start, stop, restart"));
    }

    let svc = owned_service_name(domain)
        .ok_or_else(|| format!("No managed app service found for {domain}"))?;

    let output = safe_command_sync("systemctl")
        .args([action, &svc])
        .output()
        .map_err(|e| format!("systemctl {action} failed: {e}"))?;

    if !output.status.success() {
        let err = String::from_utf8_lossy(&output.stderr);
        return Err(format!("Failed to {action} {svc}: {err}"));
    }

    // Return the updated status
    Ok(get_app_service_status(domain))
}

/// Get recent logs for an app service using journalctl
pub fn get_app_service_logs(domain: &str, lines: u32) -> Result<String, String> {
    let svc = owned_service_name(domain)
        .ok_or_else(|| format!("No managed app service found for {domain}"))?;

    let line_count = lines.clamp(1, 500);

    let output = safe_command_sync("journalctl")
        .args(["-u", &svc, "-n", &line_count.to_string(), "--no-pager"])
        .output()
        .map_err(|e| format!("journalctl failed: {e}"))?;

    let logs = String::from_utf8_lossy(&output.stdout).to_string();
    Ok(logs)
}

