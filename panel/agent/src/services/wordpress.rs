use std::process::Stdio;
use crate::safe_cmd::{safe_command, safe_command_sync, safe_command_unsandboxed};

const WP_CLI: &str = "/usr/local/bin/wp";
const WP_ROOT: &str = "/var/www";

pub fn site_path(domain: &str) -> Result<String, String> {
    if domain.is_empty() || domain.contains("..") || domain.contains('/')
        || !domain.chars().all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-') {
        return Err("Invalid domain".to_string());
    }
    let p1 = format!("{WP_ROOT}/{domain}/public");
    if std::path::Path::new(&p1).exists() {
        return Ok(p1);
    }
    let p2 = format!("{WP_ROOT}/{domain}");
    if std::path::Path::new(&format!("{p2}/wp-config.php")).exists() {
        return Ok(p2);
    }
    Ok(p1)
}

/// Ensure wp-cli is installed at /usr/local/bin/wp.
pub async fn ensure_cli() -> Result<(), String> {
    if std::path::Path::new(WP_CLI).exists() {
        return Ok(());
    }
    // curl writes to /usr/local/bin which isn't in the agent's ReadWritePaths
    // under ProtectSystem=strict — must run unsandboxed.
    let out = safe_command_unsandboxed("curl", &[])
        .args([
            "-sS",
            "-L",
            "-o",
            WP_CLI,
            "https://raw.githubusercontent.com/wp-cli/builds/gh-pages/phar/wp-cli.phar",
        ])
        .output()
        .await
        .map_err(|e| format!("Download failed: {e}"))?;
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        return Err(format!("Failed to download wp-cli: {}", stderr.trim()));
    }
    safe_command_unsandboxed("chmod", &[])
        .args(["+x", WP_CLI])
        .output()
        .await
        .ok();
    Ok(())
}

/// Run a wp-cli command, return stdout on success.
/// Uses --skip-plugins --skip-themes by default to prevent RCE from compromised
/// plugins loading PHP during admin operations. Pass skip_safety=false only for
/// commands that explicitly need to interact with plugins/themes (list, activate).
async fn wp(domain: &str, args: &[&str]) -> Result<String, String> {
    wp_inner(domain, args, true).await
}

/// Run a wp-cli command that needs plugin/theme loading (e.g., plugin list, theme list).
async fn wp_with_plugins(domain: &str, args: &[&str]) -> Result<String, String> {
    wp_inner(domain, args, false).await
}

async fn wp_inner(domain: &str, args: &[&str], skip_plugins: bool) -> Result<String, String> {
    let path = site_path(domain)?;
    wp_at_root(&path, args, skip_plugins).await
}

/// Run a wp-cli command against an EXPLICIT filesystem root.
///
/// `wp_inner` derives the root from the domain and always lands on `public/`;
/// the vulnerability scanner resolves its own root because a WordPress install
/// is not always under `public/`. It therefore needs this same invocation with
/// a different path — and hand-rolling a second one is exactly what went wrong:
/// that copy shelled out to `php` against a phar under the agent's private
/// tmp — a file nothing in this tree ever writes, and which `PrivateTmp=yes`
/// puts out of reach of anyone who might place one there — and it
/// omitted `--allow-root`, which the agent needs because it runs as root and
/// wp-cli refuses outright without it. Both arms failed, and the scan reported
/// a clean site.
///
/// So: every wp-cli invocation in this crate goes through here. One place
/// decides the binary, `ensure_cli`, `--allow-root` and the skip policy.
pub(crate) async fn wp_at_root(
    root: &str,
    args: &[&str],
    skip_plugins: bool,
) -> Result<String, String> {
    ensure_cli().await?;
    let mut cmd = safe_command(WP_CLI);
    cmd.args(args)
        .arg("--allow-root")
        .arg(format!("--path={root}"));
    if skip_plugins {
        cmd.arg("--skip-plugins").arg("--skip-themes");
    }
    // This is the crate's single wp-cli choke point (see the doc comment above),
    // so it ran every plugin install, core update and db import unbounded — no
    // timeout at all, and no kill_on_drop either. A hang here (a stalled
    // download, a locked wp_options table) blocked the request forever, and even
    // an upstream cancellation (the connection closing, a future request-level
    // timeout) would have left the wp-cli child running orphaned rather than
    // stopped — the exact "timeout doesn't actually stop the thing" class fixed
    // elsewhere in the agent crate, just never reaching wordpress.rs's own only
    // subprocess entry point.
    let out = tokio::time::timeout(
        std::time::Duration::from_secs(600),
        cmd.stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .output(),
    )
    .await
    .map_err(|_| "wp-cli timed out".to_string())?
    .map_err(|e| format!("wp-cli error: {e}"))?;

    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        return Err(stderr.trim().to_string());
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// Check if WordPress is installed at the site's document root.
pub fn detect(domain: &str) -> bool {
    match site_path(domain) {
        Ok(path) => std::path::Path::new(&format!("{path}/wp-config.php")).exists(),
        Err(_) => false,
    }
}

/// Get WP version and update availability.
pub async fn info(domain: &str) -> Result<serde_json::Value, String> {
    let version = wp(domain, &["core", "version"]).await?;

    // Check for available updates
    let update_check = wp(domain, &["core", "check-update", "--format=json"])
        .await
        .unwrap_or_default();
    let updates: Vec<serde_json::Value> =
        serde_json::from_str(&update_check).unwrap_or_default();
    let update_available = updates
        .first()
        .and_then(|u| u.get("version").and_then(|v| v.as_str()))
        .map(String::from);

    Ok(serde_json::json!({
        "installed": true,
        "version": version,
        "update_available": update_available,
    }))
}

/// List plugins with status and update info.
/// Note: plugin list requires loading plugins to get accurate status.
pub async fn plugins(domain: &str) -> Result<serde_json::Value, String> {
    let out = wp_with_plugins(domain, &["plugin", "list", "--format=json"]).await?;
    serde_json::from_str(&out).map_err(|e| format!("Parse error: {e}"))
}

/// List themes with status and update info.
/// Note: theme list requires loading themes to get accurate status.
pub async fn themes(domain: &str) -> Result<serde_json::Value, String> {
    let out = wp_with_plugins(domain, &["theme", "list", "--format=json"]).await?;
    serde_json::from_str(&out).map_err(|e| format!("Parse error: {e}"))
}

/// Update WordPress core.
pub async fn update_core(domain: &str) -> Result<String, String> {
    let result = wp(domain, &["core", "update"]).await?;
    // Fix ownership after update
    safe_command("chown")
        .args(["-R", "www-data:www-data", &site_path(domain)?])
        .output()
        .await
        .ok();
    Ok(result)
}

/// Update all plugins.
pub async fn update_all_plugins(domain: &str) -> Result<String, String> {
    let result = wp(domain, &["plugin", "update", "--all"]).await?;
    safe_command("chown")
        .args(["-R", "www-data:www-data", &site_path(domain)?])
        .output()
        .await
        .ok();
    Ok(result)
}

/// Update all themes.
pub async fn update_all_themes(domain: &str) -> Result<String, String> {
    let result = wp(domain, &["theme", "update", "--all"]).await?;
    safe_command("chown")
        .args(["-R", "www-data:www-data", &site_path(domain)?])
        .output()
        .await
        .ok();
    Ok(result)
}

/// Validate a WordPress plugin/theme slug: alphanumeric, hyphens, underscores only.
/// Rejects URLs, flags, and shell metacharacters.
fn is_valid_wp_slug(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 200
        && !name.starts_with('-')
        && !name.contains("://")
        && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

/// Plugin action: activate, deactivate, update, delete, install.
pub async fn plugin_action(domain: &str, name: &str, action: &str) -> Result<String, String> {
    if !is_valid_wp_slug(name) {
        return Err("Invalid plugin name. Only alphanumeric, hyphens, and underscores allowed.".into());
    }
    let result = match action {
        "activate" | "deactivate" | "update" | "delete" => {
            wp(domain, &["plugin", action, name]).await?
        }
        "install" => wp(domain, &["plugin", "install", name]).await?,
        _ => return Err(format!("Unknown action: {action}")),
    };
    if matches!(action, "install" | "update") {
        safe_command("chown")
            .args(["-R", "www-data:www-data", &site_path(domain)?])
            .output()
            .await
            .ok();
    }
    Ok(result)
}

/// Theme action: activate, update, delete, install.
pub async fn theme_action(domain: &str, name: &str, action: &str) -> Result<String, String> {
    if !is_valid_wp_slug(name) {
        return Err("Invalid theme name. Only alphanumeric, hyphens, and underscores allowed.".into());
    }
    let result = match action {
        "activate" | "update" | "delete" => wp(domain, &["theme", action, name]).await?,
        "install" => wp(domain, &["theme", "install", name]).await?,
        _ => return Err(format!("Unknown action: {action}")),
    };
    if matches!(action, "install" | "update") {
        safe_command("chown")
            .args(["-R", "www-data:www-data", &site_path(domain)?])
            .output()
            .await
            .ok();
    }
    Ok(result)
}

/// Install WordPress from scratch.
pub async fn install(
    domain: &str,
    url: &str,
    title: &str,
    admin_user: &str,
    admin_pass: &str,
    admin_email: &str,
    db_name: &str,
    db_user: &str,
    db_pass: &str,
    db_host: &str,
) -> Result<String, String> {
    ensure_cli().await?;
    let path = site_path(domain)?;

    // Ensure document root exists before wp-cli tries to write
    tokio::fs::create_dir_all(&path)
        .await
        .map_err(|e| format!("Failed to create site directory {path}: {e}"))?;

    // Download WordPress core files
    wp(domain, &["core", "download", "--force"]).await?;

    // Create wp-config.php (--skip-plugins --skip-themes for safety)
    //
    // These two calls predate `wp_at_root` — the file's own doc comment on that
    // function is what a hand-rolled second implementation already cost once —
    // and never got folded into it, so they kept its exact pre-fix shape: no
    // timeout, no kill_on_drop. Same fix, applied directly here rather than
    // rerouting through wp_at_root (whose signature assumes --skip-plugins
    // covers both plugins and themes together, which happens to match here, but
    // these two calls carry additional db-credential args wp_at_root's plain
    // `args: &[&str]` already accepts fine — kept separate only to avoid
    // widening this session's diff into a call-site migration).
    let out = tokio::time::timeout(
        std::time::Duration::from_secs(600),
        safe_command(WP_CLI)
            .args([
                "config",
                "create",
                &format!("--dbname={db_name}"),
                &format!("--dbuser={db_user}"),
                &format!("--dbpass={db_pass}"),
                &format!("--dbhost={db_host}"),
                "--skip-plugins",
                "--skip-themes",
                "--allow-root",
                &format!("--path={path}"),
            ])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .output(),
    )
    .await
    .map_err(|_| "wp config create timed out".to_string())?
    .map_err(|e| format!("wp config create: {e}"))?;

    if !out.status.success() {
        return Err(format!(
            "Config create failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }

    // Install WordPress (--skip-plugins --skip-themes for safety)
    let out = tokio::time::timeout(
        std::time::Duration::from_secs(600),
        safe_command(WP_CLI)
            .args([
                "core",
                "install",
                &format!("--url={url}"),
                &format!("--title={title}"),
                &format!("--admin_user={admin_user}"),
                &format!("--admin_password={admin_pass}"),
                &format!("--admin_email={admin_email}"),
                "--skip-plugins",
                "--skip-themes",
                "--allow-root",
                &format!("--path={path}"),
            ])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .output(),
    )
    .await
    .map_err(|_| "wp core install timed out".to_string())?
    .map_err(|e| format!("wp core install: {e}"))?;

    if !out.status.success() {
        return Err(format!(
            "Core install failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }

    // Fix ownership
    safe_command("chown")
        .args(["-R", "www-data:www-data", &path])
        .output()
        .await
        .ok();

    // wp-cli's `config create` leaves wp-config.php at the process umask's default
    // (644 here) — tighten to 640, same fix wp_vulnerability.rs's "wp-config-perms"
    // hardening check already applies, but on a POST-INSTALL opt-in pass only.
    // This blocks reads from any OTHER local account on the box; it does not (and
    // via chmod alone cannot) isolate this site from another tenant's own PHP-FPM
    // pool or client-role shell, since every site currently shares the identical
    // www-data:www-data identity — that is the separate, already-tracked
    // per-site-OS-user architecture question (#85), unchanged by this fix.
    safe_command("chmod")
        .args(["640", &format!("{path}/wp-config.php")])
        .output()
        .await
        .ok();

    Ok("WordPress installed successfully".into())
}

/// What happened to a site's stored canonical URL when its certificate landed.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CanonicalUrlOutcome {
    /// There is no WordPress at this vhost, or its URL is not ours to change.
    Untouched,
    /// `siteurl`/`home` moved from plain HTTP to HTTPS.
    Promoted,
    /// wp-cli refused. The certificate is live and nginx now redirects to it,
    /// but the site still tells every visitor to use HTTP.
    Failed(String),
}

impl CanonicalUrlOutcome {
    pub fn failure(&self) -> Option<&str> {
        match self {
            Self::Failed(e) => Some(e),
            _ => None,
        }
    }
}

/// Decide whether a stored canonical URL should be moved to HTTPS.
///
/// Returns the replacement only when the stored value is exactly the plain-HTTP
/// form of this vhost's own domain — the state DockPanel itself creates when it
/// installs a site before the certificate exists. Any other value (a different
/// host, a sub-path, an already-secure URL) was chosen by the operator and is
/// left alone: promoting it would silently repoint someone's site.
pub fn https_promotion_target(current: &str, domain: &str) -> Option<String> {
    let current = current.trim().trim_end_matches('/');
    let plain = format!("http://{domain}");
    if current.eq_ignore_ascii_case(&plain) {
        Some(format!("https://{domain}"))
    } else {
        None
    }
}

/// Move a WordPress site's canonical URL from HTTP to HTTPS.
///
/// Called once a certificate is in place. A site installed before its
/// certificate is reachable and correct on HTTP, but the moment nginx starts
/// redirecting to HTTPS the stored URL is the thing that decides what every
/// generated link, redirect and asset reference says — so this is what finishes
/// the job, not a cosmetic tidy-up.
pub async fn promote_site_url_to_https(domain: &str) -> CanonicalUrlOutcome {
    if !detect(domain) {
        return CanonicalUrlOutcome::Untouched;
    }
    let mut changed = false;
    for option in ["siteurl", "home"] {
        let current = match wp(domain, &["option", "get", option]).await {
            Ok(v) => v,
            Err(e) => return CanonicalUrlOutcome::Failed(format!("read {option}: {e}")),
        };
        let Some(target) = https_promotion_target(&current, domain) else {
            continue;
        };
        if let Err(e) = wp(domain, &["option", "update", option, &target]).await {
            return CanonicalUrlOutcome::Failed(format!("update {option}: {e}"));
        }
        changed = true;
    }
    if changed {
        CanonicalUrlOutcome::Promoted
    } else {
        CanonicalUrlOutcome::Untouched
    }
}

/// The comment a site's auto-update cron line ends with.
///
/// Compared with [`line_marked_for`], never with `contains`. The marker embeds a
/// domain and sits at the end of the line, so an unanchored test let one site
/// answer for another whose domain merely EXTENDS it: `# wp-auto-update-example.com`
/// is a substring of `# wp-auto-update-example.community`. Toggling auto-update
/// on `example.com`, or simply deleting that site, therefore stripped
/// `example.community`'s line — silently ending core, plugin and theme security
/// updates for a site the actor did not own, with nothing logged against it.
fn auto_update_marker(domain: &str) -> String {
    format!("# wp-auto-update-{domain}")
}

/// Whether `line` is the auto-update cron line carrying exactly `marker`.
fn line_marked_for(line: &str, marker: &str) -> bool {
    line.trim_end().ends_with(marker)
}

/// Set or remove auto-update cron.
pub async fn set_auto_update(domain: &str, enabled: bool) -> Result<(), String> {
    let path = site_path(domain)?;
    let marker = auto_update_marker(domain);
    let cron_line = format!(
        "0 3 * * * {WP_CLI} core update --skip-plugins --skip-themes --allow-root --path={path} > /dev/null 2>&1 && \
         {WP_CLI} plugin update --all --skip-plugins --skip-themes --allow-root --path={path} > /dev/null 2>&1 && \
         {WP_CLI} theme update --all --skip-plugins --skip-themes --allow-root --path={path} > /dev/null 2>&1 \
         {marker}"
    );

    // Get current crontab, FAIL-CLOSED.
    //
    // This used to be `.output().await.map(|o| …stdout…).unwrap_or_default()`,
    // which collapsed two different things into the same empty string: a spawn
    // failure (swallowed by `unwrap_or_default`) and a non-zero exit (the `map`
    // never looked at `o.status`). Either one produced `""`, and the twenty lines
    // below then hand `""` straight back to `crontab -` as root's COMPLETE new
    // content — so a transient failure here rewrote root's entire crontab, every
    // tenant's jobs and every system entry, down to at most this one WordPress
    // line. Reached from an ordinary authenticated request naming ONE site.
    let current = crate::services::crontab::read_crontab().await?;

    // Remove existing auto-update line for this domain — and ONLY this domain.
    let filtered: Vec<&str> = current
        .lines()
        .filter(|l| !line_marked_for(l, &marker))
        .collect();

    let mut new_crontab = filtered.join("\n");
    if !new_crontab.ends_with('\n') && !new_crontab.is_empty() {
        new_crontab.push('\n');
    }

    if enabled {
        new_crontab.push_str(&cron_line);
        new_crontab.push('\n');
    }

    crate::services::crontab::write_crontab(&new_crontab).await
}

/// Check if auto-update cron is enabled for a domain.
///
/// Line-anchored for the same reason the removal is: the unanchored form
/// reported `example.com` as enabled off `example.community`'s line, and
/// `delete_site` used that answer to decide whether to run the strip — so the
/// mis-read and the destructive act were the same bug twice.
pub fn is_auto_update_enabled(domain: &str) -> bool {
    let marker = auto_update_marker(domain);
    safe_command_sync("crontab")
        .args(["-l", "-u", "root"])
        .output()
        .map(|o| {
            String::from_utf8_lossy(&o.stdout)
                .lines()
                .any(|l| line_marked_for(l, &marker))
        })
        .unwrap_or(false)
}

/// Create a pre-update snapshot (files + DB) for rollback.
pub async fn create_update_snapshot(domain: &str) -> Result<String, String> {
    let _path = site_path(domain)?;
    let snapshot_dir = format!("/var/backups/dockpanel/wp-snapshots/{domain}");
    std::fs::create_dir_all(&snapshot_dir)
        .map_err(|e| format!("Create snapshot dir: {e}"))?;

    let timestamp = chrono::Utc::now().format("%Y%m%d_%H%M%S");
    let snapshot_path = format!("{snapshot_dir}/pre-update-{timestamp}.tar.gz");

    // Tar the site directory
    let tar = tokio::time::timeout(
        std::time::Duration::from_secs(120),
        safe_command("tar")
            .args(["czf", &snapshot_path, "-C", "/var/www", &format!("{domain}/public")])
            .kill_on_drop(true)
            .output()
    ).await
        .map_err(|_| "Snapshot tar timed out".to_string())?
        .map_err(|e| format!("Snapshot tar: {e}"))?;

    if !tar.status.success() {
        let stderr = String::from_utf8_lossy(&tar.stderr);
        return Err(format!("Snapshot failed: {}", stderr.chars().take(200).collect::<String>()));
    }

    // DB dump if WordPress has a database
    let db_name_output = wp(domain, &["config", "get", "DB_NAME"]).await.unwrap_or_default();
    let db_name = db_name_output.trim();
    if !db_name.is_empty() {
        let db_path = format!("{snapshot_dir}/pre-update-{timestamp}.sql");
        let _ = wp(domain, &["db", "export", &db_path, "--quiet"]).await;
    }

    // Cleanup old snapshots (keep last 5)
    if let Ok(entries) = std::fs::read_dir(&snapshot_dir) {
        let mut files: Vec<_> = entries
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().ends_with(".tar.gz"))
            .collect();
        files.sort_by_key(|e| std::cmp::Reverse(e.file_name()));
        for old in files.iter().skip(5) {
            std::fs::remove_file(old.path()).ok();
            // Also remove matching .sql
            let sql = old.path().with_extension("").with_extension("sql");
            std::fs::remove_file(sql).ok();
        }
    }

    tracing::info!("WP update snapshot created for {domain}: {snapshot_path}");
    Ok(snapshot_path)
}

/// Rollback a WordPress site to a snapshot.
pub async fn rollback_from_snapshot(domain: &str, snapshot_path: &str) -> Result<(), String> {
    if !std::path::Path::new(snapshot_path).exists() {
        return Err("Snapshot file not found".to_string());
    }

    // Extract tar over site directory
    let restore = tokio::time::timeout(
        std::time::Duration::from_secs(120),
        safe_command("tar")
            .args(["xzf", snapshot_path, "-C", "/var/www"])
            .kill_on_drop(true)
            .output()
    ).await
        .map_err(|_| "Rollback timed out".to_string())?
        .map_err(|e| format!("Rollback tar: {e}"))?;

    if !restore.status.success() {
        return Err("Rollback tar extraction failed".to_string());
    }

    // Restore DB if SQL dump exists
    let sql_path = snapshot_path.replace(".tar.gz", ".sql");
    if std::path::Path::new(&sql_path).exists() {
        let _ = wp(domain, &["db", "import", &sql_path, "--quiet"]).await;
    }

    // Fix ownership
    let _ = safe_command("chown")
        .args(["-R", "www-data:www-data", &format!("/var/www/{domain}/public")])
        .output()
        .await;

    tracing::info!("WP rollback completed for {domain} from {snapshot_path}");
    Ok(())
}

/// Run a health check on a WordPress site after update.
pub async fn health_check(domain: &str) -> bool {
    // Check if WordPress responds to wp-cli
    let result = tokio::time::timeout(
        std::time::Duration::from_secs(15),
        safe_command("sudo")
            .args(["-u", "www-data", WP_CLI, "eval", "echo 'OK';",
                   "--skip-plugins", "--skip-themes", "--allow-root",
                   &format!("--path={}", site_path(domain).unwrap_or_default())])
            .kill_on_drop(true)
            .output()
    ).await;

    match result {
        Ok(Ok(out)) => {
            let stdout = String::from_utf8_lossy(&out.stdout);
            out.status.success() && stdout.contains("OK")
        }
        _ => false,
    }
}

/// Update WordPress with snapshot + rollback on failure.
pub async fn update_with_rollback(domain: &str) -> Result<serde_json::Value, String> {
    // Held for the whole snapshot→update→rollback sequence: the snapshot at
    // step 2 is a point-in-time copy of the docroot, and a rollback at step 6
    // restores it unconditionally. Anything else that writes into this
    // site's files while that window is open (hardening, a backup restore, a
    // staging sync) would otherwise be silently discarded by the rollback, or
    // itself discard the update — see site_lock's module doc.
    let _guard = crate::site_lock::lock_site(domain).await;

    let mut log: Vec<String> = Vec::new();

    // 1. Health check before update
    if !health_check(domain).await {
        return Err("Site failed pre-update health check — skipping update".to_string());
    }
    log.push("Pre-update health check: passed".into());

    // 2. Create snapshot
    let snapshot = create_update_snapshot(domain).await?;
    log.push("Snapshot created".into());

    // 3. Get current versions
    let core_before = wp(domain, &["core", "version"]).await.unwrap_or_default().trim().to_string();

    // 4. Run updates
    let core_ok = wp(domain, &["core", "update"]).await.is_ok();
    let plugins_ok = wp(domain, &["plugin", "update", "--all"]).await.is_ok();
    let themes_ok = wp(domain, &["theme", "update", "--all"]).await.is_ok();
    log.push(format!("Updates: core={}, plugins={}, themes={}",
        if core_ok { "ok" } else { "failed" },
        if plugins_ok { "ok" } else { "failed" },
        if themes_ok { "ok" } else { "failed" }));

    // 5. Fix ownership
    let _ = safe_command("chown")
        .args(["-R", "www-data:www-data", &format!("/var/www/{domain}/public")])
        .output()
        .await;

    // 6. Post-update health check
    let healthy = health_check(domain).await;

    if !healthy {
        log.push("Post-update health check: FAILED — rolling back".into());
        match rollback_from_snapshot(domain, &snapshot).await {
            Ok(()) => {
                log.push("Rollback completed successfully".into());
                tracing::warn!("WP update for {domain} rolled back due to health check failure");
            }
            Err(e) => {
                log.push(format!("Rollback failed: {e}"));
                tracing::error!("WP rollback failed for {domain}: {e}");
            }
        }
    } else {
        log.push("Post-update health check: passed".into());
    }

    let core_after = wp(domain, &["core", "version"]).await.unwrap_or_default().trim().to_string();

    Ok(serde_json::json!({
        "domain": domain,
        "healthy": healthy,
        "rolled_back": !healthy,
        "core_before": core_before,
        "core_after": core_after,
        "snapshot": snapshot,
        "log": log,
    }))
}

/// Get database connection details from wp-config.php
pub async fn db_info(domain: &str) -> Result<serde_json::Value, String> {
    let db_name = wp(domain, &["config", "get", "DB_NAME"]).await.unwrap_or_default();
    let db_user = wp(domain, &["config", "get", "DB_USER"]).await.unwrap_or_default();
    let db_host = wp(domain, &["config", "get", "DB_HOST"]).await.unwrap_or_default();
    let prefix = wp(domain, &["config", "get", "table_prefix"]).await.unwrap_or_else(|_| "wp_".into());
    Ok(serde_json::json!({
        "db_name": db_name.trim(),
        "db_user": db_user.trim(),
        "db_host": if db_host.trim().is_empty() { "localhost:3306" } else { db_host.trim() },
        "table_prefix": prefix.trim(),
    }))
}

/// Run an arbitrary WP-CLI command in the site's directory safely.
pub async fn run_cli(domain: &str, command: &str) -> Result<String, String> {
    let clean = command.trim();
    let clean = clean.strip_prefix("wp ").unwrap_or(clean).trim();
    if clean.is_empty() {
        return Err("Command is empty".to_string());
    }

    // Split shell arguments safely
    let parts: Vec<&str> = clean.split_whitespace().collect();
    if parts.is_empty() {
        return Err("Command is empty".to_string());
    }

    // Block dangerous flags or escapes
    for part in &parts {
        if *part == "--allow-root" || part.starts_with("--path=") {
            continue;
        }
    }

    let path = site_path(domain)?;
    wp_at_root(&path, &parts, false).await
}

/// Get current feature status for Plesk-like WP Toolkit dashboard.
pub async fn get_features_status(domain: &str) -> Result<serde_json::Value, String> {
    let path = site_path(domain)?;

    // 1. WP_DEBUG
    let debug_val = wp(domain, &["config", "get", "WP_DEBUG"]).await.unwrap_or_default();
    let wp_debug = debug_val.trim().eq_ignore_ascii_case("true") || debug_val.trim() == "1";

    // 2. Search engine indexing (blog_public)
    let public_val = wp(domain, &["option", "get", "blog_public"]).await.unwrap_or_default();
    let search_indexing = public_val.trim() != "0";

    // 3. Take over wp-cron.php (DISABLE_WP_CRON)
    let cron_val = wp(domain, &["config", "get", "DISABLE_WP_CRON"]).await.unwrap_or_default();
    let takeover_cron = cron_val.trim().eq_ignore_ascii_case("true") || cron_val.trim() == "1";

    // 4. Maintenance mode (.maintenance file or wp maintenance-mode)
    let maintenance_file = format!("{path}/.maintenance");
    let maintenance_mode = std::path::Path::new(&maintenance_file).exists();

    // 5. Hotlink protection marker
    let hotlink_flag = format!("/var/www/{domain}/.wp_hotlink");
    let hotlink_protect = std::path::Path::new(&hotlink_flag).exists();

    // 6. Password protection (.htpasswd file)
    let htpasswd_file = format!("/var/www/{domain}/.htpasswd");
    let pass_protect = std::path::Path::new(&htpasswd_file).exists();

    // 7. Caching
    let cache_flag = format!("/var/www/{domain}/.wp_nginx_cache");
    let nocache_flag = format!("/var/www/{domain}/.wp_nocache");
    let caching = std::path::Path::new(&cache_flag).exists() || !std::path::Path::new(&nocache_flag).exists();

    // 8. Vulnerability protection (DISALLOW_FILE_EDIT)
    let file_edit = wp(domain, &["config", "get", "DISALLOW_FILE_EDIT"]).await.unwrap_or_default();
    let vuln_protect = file_edit.trim().eq_ignore_ascii_case("true") || file_edit.trim() == "1";

    // 9. Site title
    let site_title = wp(domain, &["option", "get", "blogname"]).await.unwrap_or_else(|_| domain.to_string());

    // 10. Admin user & email
    let admin_user = wp(domain, &["user", "list", "--role=administrator", "--field=user_login"]).await.unwrap_or_default();
    let first_admin = admin_user.lines().next().unwrap_or("admin").trim().to_string();

    let admin_email = wp(domain, &["user", "list", "--role=administrator", "--field=user_email"]).await.unwrap_or_default();
    let first_email = admin_email.lines().next().unwrap_or("").trim().to_string();

    Ok(serde_json::json!({
        "wp_debug": wp_debug,
        "search_indexing": search_indexing,
        "takeover_cron": takeover_cron,
        "maintenance_mode": maintenance_mode,
        "hotlink_protect": hotlink_protect,
        "pass_protect": pass_protect,
        "caching": caching,
        "vuln_protect": vuln_protect,
        "site_title": if site_title.trim().is_empty() { domain.to_string() } else { site_title.trim().to_string() },
        "admin_user": if first_admin.is_empty() { "admin".to_string() } else { first_admin },
        "admin_email": first_email,
    }))
}

/// Toggle individual WordPress / Toolkit features.
pub async fn toggle_feature(domain: &str, feature: &str, enabled: bool, params: Option<serde_json::Value>) -> Result<serde_json::Value, String> {
    match feature {
        "wp_debug" => {
            let val = if enabled { "true" } else { "false" };
            wp(domain, &["config", "set", "WP_DEBUG", val, "--raw"]).await?;
            if let Some(p) = &params {
                if let Some(log) = p.get("debug_log").and_then(|v| v.as_bool()) {
                    let log_val = if log { "true" } else { "false" };
                    let _ = wp(domain, &["config", "set", "WP_DEBUG_LOG", log_val, "--raw"]).await;
                }
                if let Some(display) = p.get("debug_display").and_then(|v| v.as_bool()) {
                    let disp_val = if display { "true" } else { "false" };
                    let _ = wp(domain, &["config", "set", "WP_DEBUG_DISPLAY", disp_val, "--raw"]).await;
                }
            }
        }
        "search_indexing" => {
            let val = if enabled { "1" } else { "0" };
            wp(domain, &["option", "update", "blog_public", val]).await?;
        }
        "takeover_cron" => {
            let val = if enabled { "true" } else { "false" };
            wp(domain, &["config", "set", "DISABLE_WP_CRON", val, "--raw"]).await?;
            let cron_marker = format!("# wp-cron-{domain}");
            let cron_line = format!("* * * * * curl -s -k -L https://{domain}/wp-cron.php?doing_wp_cron >/dev/null 2>&1 {cron_marker}");
            let current = crate::services::crontab::read_crontab().await.unwrap_or_default();
            let filtered: Vec<&str> = current.lines().filter(|l| !l.trim_end().ends_with(&cron_marker)).collect();
            let mut new_crontab = filtered.join("\n");
            if !new_crontab.ends_with('\n') && !new_crontab.is_empty() {
                new_crontab.push('\n');
            }
            if enabled {
                new_crontab.push_str(&cron_line);
                new_crontab.push('\n');
            }
            crate::services::crontab::write_crontab(&new_crontab).await?;
        }
        "maintenance_mode" => {
            maintenance(domain, enabled).await?;
        }
        "hotlink_protection" => {
            let flag = format!("/var/www/{domain}/.wp_hotlink");
            if enabled {
                let _ = std::fs::write(&flag, "enabled");
            } else {
                let _ = std::fs::remove_file(&flag);
            }
        }
        "pass_protect" => {
            let htpasswd_file = format!("/var/www/{domain}/.htpasswd");
            if enabled {
                let user = params.as_ref().and_then(|p| p.get("user")).and_then(|v| v.as_str()).unwrap_or("admin");
                let pass = params.as_ref().and_then(|p| p.get("pass")).and_then(|v| v.as_str()).unwrap_or("admin");
                let out = safe_command("openssl")
                    .args(["passwd", "-apr1", pass])
                    .output()
                    .await
                    .map_err(|e| format!("Password hash failed: {e}"))?;
                let hash = String::from_utf8_lossy(&out.stdout).trim().to_string();
                let line = format!("{user}:{hash}\n");
                std::fs::write(&htpasswd_file, line).map_err(|e| format!("Failed to write .htpasswd: {e}"))?;
            } else {
                let _ = std::fs::remove_file(&htpasswd_file);
            }
        }
        "caching" => {
            let flag = format!("/var/www/{domain}/.wp_nginx_cache");
            let nocache_flag = format!("/var/www/{domain}/.wp_nocache");
            if enabled {
                let _ = std::fs::write(&flag, "enabled");
                let _ = std::fs::remove_file(&nocache_flag);
            } else {
                let _ = std::fs::remove_file(&flag);
                let _ = std::fs::write(&nocache_flag, "disabled");
            }
        }
        "vuln_protect" => {
            let val = if enabled { "true" } else { "false" };
            let _ = wp(domain, &["config", "set", "DISALLOW_FILE_EDIT", val, "--raw"]).await;
            let _ = wp(domain, &["config", "set", "DISALLOW_FILE_MODS", val, "--raw"]).await;
        }
        _ => return Err(format!("Unknown feature: {feature}")),
    }
    Ok(serde_json::json!({ "feature": feature, "enabled": enabled, "ok": true }))
}

/// Enable or disable maintenance mode.
pub async fn maintenance(domain: &str, enabled: bool) -> Result<(), String> {
    let path = site_path(domain)?;
    let file = format!("{path}/.maintenance");
    if enabled {
        let _ = wp(domain, &["maintenance-mode", "activate"]).await;
        if !std::path::Path::new(&file).exists() {
            let ts = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();
            let content = format!("<?php $upgrading = {ts}; ?>");
            std::fs::write(&file, content).map_err(|e| format!("Failed to create .maintenance: {e}"))?;
        }
    } else {
        let _ = wp(domain, &["maintenance-mode", "deactivate"]).await;
        if std::path::Path::new(&file).exists() {
            std::fs::remove_file(&file).map_err(|e| format!("Failed to remove .maintenance: {e}"))?;
        }
    }
    Ok(())
}

/// Verify WordPress core checksums.
pub async fn verify_integrity(domain: &str) -> Result<serde_json::Value, String> {
    let core_res = wp(domain, &["core", "verify-checksums"]).await;
    let core_clean = core_res.is_ok();
    let mut issues: Vec<serde_json::Value> = Vec::new();
    match &core_res {
        Ok(_) => {},
        Err(err) => {
            for line in err.lines() {
                let trimmed = line.trim();
                if trimmed.starts_with("Warning:") {
                    let msg = trimmed.trim_start_matches("Warning:").trim();
                    let (status, file) = if msg.starts_with("File doesn't exist:") {
                        ("Missing file", msg.trim_start_matches("File doesn't exist:").trim())
                    } else if msg.starts_with("File should not exist:") {
                        ("Unexpected file", msg.trim_start_matches("File should not exist:").trim())
                    } else if msg.starts_with("File doesn't verify against checksum:") {
                        ("Checksum mismatch", msg.trim_start_matches("File doesn't verify against checksum:").trim())
                    } else {
                        ("Notice", msg)
                    };
                    issues.push(serde_json::json!({
                        "file": file,
                        "status": status,
                        "message": msg,
                    }));
                } else if trimmed.starts_with("Error:") && !trimmed.contains("doesn't verify against checksums") {
                    issues.push(serde_json::json!({
                        "file": "WordPress Core",
                        "status": "Error",
                        "message": trimmed.trim_start_matches("Error:").trim(),
                    }));
                }
            }
            if issues.is_empty() {
                issues.push(serde_json::json!({
                    "file": "WordPress Core",
                    "status": "Verification notice",
                    "message": err,
                }));
            }
        }
    };
    Ok(serde_json::json!({
        "clean": core_clean && issues.is_empty(),
        "issues": issues,
        "message": if core_clean && issues.is_empty() {
            "WordPress core files checksums verified successfully. All core files match official release."
        } else {
            "Some files differ from the official WordPress release checksums."
        },
        "checked_at": chrono::Utc::now().to_rfc3339(),
    }))
}

/// Re-install WordPress core files cleanly from wordpress.org, preserving wp-content and wp-config.php.
pub async fn reinstall_core(domain: &str) -> Result<serde_json::Value, String> {
    let res = wp(domain, &["core", "download", "--skip-content", "--force"]).await?;
    let doc_root = format!("/var/www/{domain}/public");
    let _ = tokio::process::Command::new("chown")
        .args(&["-R", "www-data:www-data", &doc_root])
        .output()
        .await;
    Ok(serde_json::json!({
        "ok": true,
        "message": "WordPress core files re-installed successfully from official WordPress.org release.",
        "output": res
    }))
}

/// Update setup credentials (admin user, email, password, and site title).
pub async fn setup_admin(
    domain: &str,
    user: &str,
    email: &str,
    pass: Option<&str>,
    title: Option<&str>,
) -> Result<String, String> {
    if let Some(t) = title {
        if !t.trim().is_empty() {
            let _ = wp(domain, &["option", "update", "blogname", t.trim()]).await;
        }
    }

    let mut args = vec!["user", "update", user];
    let email_arg;
    if !email.trim().is_empty() {
        email_arg = format!("--user_email={}", email.trim());
        args.push(&email_arg);
    }
    let pass_arg;
    if let Some(p) = pass {
        if !p.is_empty() {
            pass_arg = format!("--user_pass={p}");
            args.push(&pass_arg);
        }
    }

    wp(domain, &args).await
}

/// Copy WordPress data (files, database) to a target site.
pub async fn copy_data(
    src_domain: &str,
    dst_domain: &str,
    copy_files: bool,
    copy_db: bool,
) -> Result<String, String> {
    let src_path = site_path(src_domain)?;
    let dst_path = site_path(dst_domain)?;
    let mut messages = Vec::new();

    if copy_files {
        let src_content = format!("{src_path}/wp-content/");
        let dst_content = format!("{dst_path}/wp-content/");
        let out = safe_command("rsync")
            .args(["-avz", "--delete", &src_content, &dst_content])
            .output()
            .await
            .map_err(|e| format!("rsync failed: {e}"))?;
        if !out.status.success() {
            let err = String::from_utf8_lossy(&out.stderr);
            return Err(format!("File sync failed: {err}"));
        }
        let _ = safe_command("chown")
            .args(["-R", "www-data:www-data", &dst_content])
            .output()
            .await;
        messages.push("WordPress files synced successfully");
    }

    if copy_db {
        let dump_file = format!("/tmp/wp_copy_{src_domain}.sql");
        wp(src_domain, &["db", "export", &dump_file]).await
            .map_err(|e| format!("Database export failed: {e}"))?;
        wp(dst_domain, &["db", "import", &dump_file]).await
            .map_err(|e| format!("Database import failed: {e}"))?;
        let _ = std::fs::remove_file(&dump_file);
        let _ = wp(dst_domain, &["search-replace", src_domain, dst_domain, "--all-tables"]).await;
        messages.push("Database imported and domain migrated");
    }

    Ok(messages.join(". "))
}

/// Run automated mitigations: updates and security fixes.
pub async fn mitigate_vulns(domain: &str) -> Result<serde_json::Value, String> {
    let mut actions = Vec::new();
    match update_all_plugins(domain).await {
        Ok(res) => actions.push(format!("Plugins updated: {res}")),
        Err(e) => actions.push(format!("Plugin updates: {e}")),
    }
    let fixes = vec![
        "disallow-file-edit".to_string(),
        "wp-config-perms".to_string(),
        "block-php-uploads".to_string(),
    ];
    match crate::services::wp_vulnerability::apply_hardening(domain, &fixes).await {
        Ok(res) => actions.push(format!("Applied {} security hardening measures", res.len())),
        Err(e) => actions.push(format!("Hardening: {e}")),
    }
    Ok(serde_json::json!({
        "ok": true,
        "actions": actions,
        "message": "Vulnerability mitigation completed"
    }))
}

#[cfg(test)]
mod canonical_url_tests {
    use super::{auto_update_marker, https_promotion_target, line_marked_for};

    #[test]
    fn promotes_this_vhost_own_plain_http_url() {
        assert_eq!(
            https_promotion_target("http://example.com", "example.com").as_deref(),
            Some("https://example.com")
        );
    }

    #[test]
    fn tolerates_a_trailing_slash_and_odd_casing() {
        // wp-cli prints what is stored, and what is stored has been through
        // however many admin screens and migrations.
        assert!(https_promotion_target("http://example.com/", "example.com").is_some());
        assert!(https_promotion_target("  http://example.com  ", "example.com").is_some());
        assert!(https_promotion_target("HTTP://EXAMPLE.COM", "example.com").is_some());
    }

    #[test]
    fn leaves_an_already_secure_url_alone() {
        assert_eq!(https_promotion_target("https://example.com", "example.com"), None);
    }

    #[test]
    fn never_repoints_a_url_the_operator_chose() {
        // Each of these is a working configuration somebody set on purpose.
        // Rewriting any of them would move a live site to an address the
        // operator never picked — a worse bug than the one being fixed.
        for stored in [
            "http://other-domain.com",       // headless / separate front end
            "http://example.com/blog",       // WordPress in a subdirectory
            "http://www.example.com",        // canonical host is the www one
            "http://sub.example.com",        // a different vhost entirely
            "",                              // nothing stored
        ] {
            assert_eq!(
                https_promotion_target(stored, "example.com"),
                None,
                "must not promote {stored:?}"
            );
        }
    }

    #[test]
    fn a_suffix_lookalike_is_not_this_domain() {
        // The comparison is whole-string, so a domain that merely starts with
        // ours cannot borrow the promotion.
        assert_eq!(
            https_promotion_target("http://example.com.attacker.net", "example.com"),
            None
        );
    }

    /// The prefix-collision that let one site strip another's security updates.
    /// `example.com` and `example.community` are separately registerable, and
    /// the marker sits at the END of the cron line, so the old `contains` test
    /// matched the longer domain's line from the shorter domain's marker.
    #[test]
    fn a_prefix_domain_cannot_match_a_longer_domains_cron_line() {
        let victim = format!(
            "0 3 * * * wp core update --path=/var/www/example.community/public {}",
            auto_update_marker("example.community")
        );
        assert!(line_marked_for(&victim, &auto_update_marker("example.community")));
        assert!(!line_marked_for(&victim, &auto_update_marker("example.com")));
        // The other direction is safe too: a longer marker cannot match a
        // shorter domain's line.
        let short = format!(
            "0 3 * * * wp core update --path=/var/www/a.co/public {}",
            auto_update_marker("a.co")
        );
        assert!(!line_marked_for(&short, &auto_update_marker("a.com")));
    }

    #[test]
    fn trailing_whitespace_does_not_hide_the_marker() {
        let line = format!("0 3 * * * wp core update {}   ", auto_update_marker("a.com"));
        assert!(line_marked_for(&line, &auto_update_marker("a.com")));
    }
}
