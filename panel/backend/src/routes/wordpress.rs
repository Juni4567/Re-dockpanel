use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use uuid::Uuid;

use crate::auth::{AuthUser, ServerScope};
use crate::error::{internal_error, agent_error, err, require_admin, ApiError};
use crate::services::activity;
use crate::services::agent::AgentHandle;
use crate::services::notifications;
use crate::AppState;

// ── Vulnerability-scan settings ────────────────────────────────────────────
// Companion to image_scans' settings pair (read_settings/get_settings/
// update_settings) — same shape, same "off by default" posture, distinct
// keys because this schedule sweeps WordPress sites, not Docker images.

#[derive(serde::Serialize)]
pub struct WpScanSettings {
    pub enabled: bool,
    pub interval_hours: i32,
}

#[derive(serde::Deserialize)]
pub struct UpdateWpScanSettings {
    pub enabled: bool,
    pub interval_hours: i32,
}

pub async fn read_wp_scan_settings(pool: &sqlx::PgPool) -> Result<(bool, i32), sqlx::Error> {
    let rows: Vec<(String, String)> = sqlx::query_as(
        "SELECT key, value FROM settings WHERE key IN ('wp_vuln_scan_enabled', 'wp_vuln_scan_interval_hours')",
    )
    .fetch_all(pool)
    .await?;

    let mut enabled = false;
    let mut hours = 24i32;
    for (k, v) in rows {
        match k.as_str() {
            "wp_vuln_scan_enabled" => enabled = v == "true",
            "wp_vuln_scan_interval_hours" => hours = v.parse().unwrap_or(24),
            _ => {}
        }
    }
    Ok((enabled, hours))
}

/// GET /api/wordpress/vuln-scan-settings
pub async fn get_scan_settings(
    State(state): State<AppState>,
    AuthUser(claims): AuthUser,
) -> Result<Json<WpScanSettings>, ApiError> {
    require_admin(&claims.role)?;
    let (enabled, interval_hours) = read_wp_scan_settings(&state.db)
        .await
        .map_err(|e| internal_error("read wp scan settings", e))?;
    Ok(Json(WpScanSettings { enabled, interval_hours }))
}

/// PUT /api/wordpress/vuln-scan-settings
pub async fn update_scan_settings(
    State(state): State<AppState>,
    AuthUser(claims): AuthUser,
    Json(body): Json<UpdateWpScanSettings>,
) -> Result<Json<serde_json::Value>, ApiError> {
    require_admin(&claims.role)?;

    if !(1..=720).contains(&body.interval_hours) {
        return Err(err(StatusCode::BAD_REQUEST, "interval_hours must be 1..=720"));
    }

    for (key, value) in [
        ("wp_vuln_scan_enabled", if body.enabled { "true" } else { "false" }),
        ("wp_vuln_scan_interval_hours", body.interval_hours.to_string().as_str()),
    ] {
        sqlx::query("INSERT INTO settings (key, value) VALUES ($1, $2) \
                     ON CONFLICT (key) DO UPDATE SET value = EXCLUDED.value")
            .bind(key)
            .bind(value)
            .execute(&state.db)
            .await
            .map_err(|e| internal_error("save wp scan setting", e))?;
    }

    Ok(Json(serde_json::json!({ "ok": true })))
}

/// The `state_key` a site's WordPress vulnerability alert fires and resolves
/// under. Mirrors `image_scans::image_scan_state_key` — `alerts.state_key` is
/// `VARCHAR(100)` and a domain, while normally well under that, is not a
/// validated field, so the same truncate-then-hash shape applies rather than
/// let a long domain silently fail the INSERT and swallow the alert.
fn wp_vuln_state_key(domain: &str) -> String {
    const MAX: usize = 100;
    let readable = format!("wp_vuln:{domain}");
    if readable.len() <= MAX {
        return readable;
    }
    use sha2::{Digest, Sha256};
    let digest = hex::encode(Sha256::digest(domain.as_bytes()));
    let prefix = "wp_vuln:";
    let budget = MAX - prefix.len() - 17;
    let keep: String = domain.chars().take(budget).collect();
    format!("{prefix}{keep}-{}", &digest[..16])
}

/// Run a WordPress vulnerability scan via the agent, persist it, and fire or
/// resolve the alert it implies. Public so the manual endpoint below and the
/// background sweep (`services::wp_vuln_scanner`) share one path — before
/// this, only the manual button existed, so a critical CVE in a plugin nobody
/// happened to click Scan on stayed invisible forever.
pub async fn scan_and_store(
    pool: &sqlx::PgPool,
    site_id: Uuid,
    user_id: Uuid,
    domain: &str,
    agent: &AgentHandle,
) -> Result<serde_json::Value, ApiError> {
    let result = agent
        .post(&format!("/wordpress/{domain}/vuln-scan"), None::<serde_json::Value>)
        .await
        .map_err(|e| agent_error("Vulnerability scan", e))?;

    let total = result.get("total_vulns").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
    let critical = result.get("critical_count").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
    let high = result.get("high_count").and_then(|v| v.as_i64()).unwrap_or(0) as i32;

    sqlx::query(
        "INSERT INTO wp_vuln_scans (site_id, domain, total_vulns, critical_count, high_count, scan_data) VALUES ($1, $2, $3, $4, $5, $6)",
    )
    .bind(site_id)
    .bind(domain)
    .bind(total)
    .bind(critical)
    .bind(high)
    .bind(&result)
    .execute(pool)
    .await
    .map_err(|e| { tracing::warn!("Failed to store scan result: {e}"); })
    .ok();

    // Resolve-then-fire, mirroring `image_scans::scan_and_store`: every scan
    // of this site first clears whatever alert the LAST scan of it raised,
    // then raises a new one only if THIS scan is still dirty. To the site's
    // OWNER, not "every admin" — unlike a shared Docker image, a WordPress
    // site belongs to one user, and `resolve_ssl_renewal_failure`'s own
    // convention for a per-site alert is the site's `user_id`.
    let state_key = wp_vuln_state_key(domain);
    notifications::resolve_alert(
        pool,
        user_id,
        None,
        Some(site_id),
        "wp_vuln_scan",
        &state_key,
        &format!("WordPress vulnerability alert resolved: {domain}"),
        &format!(
            "A later scan of {domain} found no critical or high severity plugin vulnerability — the earlier alert no longer applies."
        ),
    )
    .await;
    if critical > 0 || high > 0 {
        let severity = if critical > 0 { "critical" } else { "warning" };
        let title = format!("WordPress scan: {critical} critical, {high} high on {domain}");
        let message = format!(
            "A vulnerability scan of {domain} found {critical} critical, {high} high severity plugin issues. Review in the WordPress toolkit."
        );
        notifications::fire_alert(
            pool, user_id, None, Some(site_id), "wp_vuln_scan", &state_key, severity, &title, &message,
        )
        .await;
    }

    Ok(result)
}


/// GET /api/sites/{id}/wordpress — Detect WP + get info + auto-update status.
pub async fn info(
    State(state): State<AppState>,
    AuthUser(claims): AuthUser,
    Path(id): Path<Uuid>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let (domain, agent) = crate::helpers::site_agent_for_caller(&state, id, &claims).await?;

    let resp: serde_json::Value = agent
        .get(&format!("/wordpress/{domain}/info"))
        .await
        .map_err(|e| { tracing::warn!("WordPress info failed for {domain}: {e}"); err(StatusCode::BAD_GATEWAY, "WordPress service unavailable") })?;

    // Also get auto-update status
    let auto: serde_json::Value = agent
        .get(&format!("/wordpress/{domain}/auto-update"))
        .await
        .unwrap_or(serde_json::json!({ "enabled": false }));

    let mut result = resp;
    result["auto_update"] = auto
        .get("enabled")
        .cloned()
        .unwrap_or(serde_json::json!(false));

    Ok(Json(result))
}

/// POST /api/sites/{id}/wordpress/install — Install WordPress.
pub async fn install(
    State(state): State<AppState>,
    AuthUser(claims): AuthUser,
    Path(id): Path<Uuid>,
    Json(mut body): Json<serde_json::Value>,
) -> Result<(StatusCode, Json<serde_json::Value>), ApiError> {
    let (domain, agent) = crate::helpers::site_agent_for_caller(&state, id, &claims).await?;

    // Normalize field aliases so callers can send admin_pass OR admin_password, title OR site_title
    // without triggering serde duplicate-field 422 downstream in the agent.
    if let Some(obj) = body.as_object_mut() {
        if !obj.contains_key("admin_pass") {
            if let Some(pass) = obj.remove("admin_password") {
                obj.insert("admin_pass".to_string(), pass);
            }
        } else {
            obj.remove("admin_password");
        }
        if !obj.contains_key("title") {
            if let Some(t) = obj.remove("site_title") {
                obj.insert("title".to_string(), t);
            }
        } else {
            obj.remove("site_title");
        }
        if !obj.contains_key("url") || obj.get("url").and_then(|u| u.as_str()).map(|s| s.is_empty()).unwrap_or(false) {
            obj.insert("url".to_string(), serde_json::json!(format!("http://{domain}")));
        }
    }

    let resp: serde_json::Value = agent
        .post(&format!("/wordpress/{domain}/install"), Some(body))
        .await
        .map_err(|e| agent_error("WordPress", e))?;

    // Ensure the site record reflects WordPress (and upgrades static sites to PHP + WordPress vhost).
    if let Ok(Some(mut site)) = sqlx::query_as::<_, crate::models::Site>(
        "SELECT * FROM sites WHERE id = $1",
    )
    .bind(id)
    .fetch_optional(&state.db)
    .await
    {
        let was_static = site.runtime == "static";
        let preset_changed = site.php_preset.as_deref() != Some("wordpress");
        if was_static {
            site.runtime = "php".to_string();
            if site.php_version.is_none() {
                site.php_version = Some("8.3".to_string());
            }
        }
        site.php_preset = Some("wordpress".to_string());

        if was_static || preset_changed {
            let nginx_body = crate::routes::sites::build_nginx_body(&site);
            let _ = agent.put(&format!("/nginx/sites/{domain}"), nginx_body).await;
            let _ = sqlx::query(
                "UPDATE sites SET runtime = $1, php_version = $2, php_preset = 'wordpress', updated_at = NOW() WHERE id = $3",
            )
            .bind(&site.runtime)
            .bind(&site.php_version)
            .bind(id)
            .execute(&state.db)
            .await;
        }
    }

    activity::log_activity(
        &state.db,
        claims.sub,
        &claims.email,
        "wordpress.install",
        Some("site"),
        Some(&domain),
        None,
        None,
        claims.key_id,
    )
    .await;

    Ok((StatusCode::CREATED, Json(resp)))
}

/// GET /api/sites/{id}/wordpress/plugins — List plugins.
pub async fn plugins(
    State(state): State<AppState>,
    AuthUser(claims): AuthUser,
    Path(id): Path<Uuid>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let (domain, agent) = crate::helpers::site_agent_for_caller(&state, id, &claims).await?;

    let resp: serde_json::Value = agent
        .get(&format!("/wordpress/{domain}/plugins"))
        .await
        .map_err(|e| { tracing::warn!("WordPress plugins failed for {domain}: {e}"); err(StatusCode::BAD_GATEWAY, "WordPress service unavailable") })?;

    Ok(Json(resp))
}

/// GET /api/sites/{id}/wordpress/themes — List themes.
pub async fn themes(
    State(state): State<AppState>,
    AuthUser(claims): AuthUser,
    Path(id): Path<Uuid>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let (domain, agent) = crate::helpers::site_agent_for_caller(&state, id, &claims).await?;

    let resp: serde_json::Value = agent
        .get(&format!("/wordpress/{domain}/themes"))
        .await
        .map_err(|e| { tracing::warn!("WordPress themes failed for {domain}: {e}"); err(StatusCode::BAD_GATEWAY, "WordPress service unavailable") })?;

    Ok(Json(resp))
}

/// POST /api/sites/{id}/wordpress/update/{target} — Update core/plugins/themes.
pub async fn update(
    State(state): State<AppState>,
    AuthUser(claims): AuthUser,
    Path((id, target)): Path<(Uuid, String)>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let (domain, agent) = crate::helpers::site_agent_for_caller(&state, id, &claims).await?;

    if !["core", "plugins", "themes"].contains(&target.as_str()) {
        return Err(err(StatusCode::BAD_REQUEST, "Invalid target"));
    }

    let resp: serde_json::Value = agent
        .post(
            &format!("/wordpress/{domain}/update/{target}"),
            None::<serde_json::Value>,
        )
        .await
        .map_err(|e| agent_error("WordPress", e))?;

    activity::log_activity(
        &state.db,
        claims.sub,
        &claims.email,
        &format!("wordpress.update.{target}"),
        Some("site"),
        Some(&domain),
        None,
        None,
        claims.key_id,
    )
    .await;

    Ok(Json(resp))
}

/// POST /api/sites/{id}/wordpress/plugin/{action} — Plugin action.
pub async fn plugin_action(
    State(state): State<AppState>,
    AuthUser(claims): AuthUser,
    Path((id, action)): Path<(Uuid, String)>,
    Json(body): Json<serde_json::Value>,
) -> Result<Json<serde_json::Value>, ApiError> {
    const ALLOWED_PLUGIN_ACTIONS: &[&str] = &[
        "activate",
        "deactivate",
        "delete",
        "update",
        "install",
        "enable-autoupdate",
        "disable-autoupdate",
        "enable-all-autoupdate",
        "disable-all-autoupdate",
    ];
    if !ALLOWED_PLUGIN_ACTIONS.contains(&action.as_str()) {
        return Err(err(StatusCode::BAD_REQUEST, "Invalid plugin action"));
    }

    let (domain, agent) = crate::helpers::site_agent_for_caller(&state, id, &claims).await?;

    let resp: serde_json::Value = agent
        .post(
            &format!("/wordpress/{domain}/plugin/{action}"),
            Some(body),
        )
        .await
        .map_err(|e| agent_error("WordPress", e))?;

    Ok(Json(resp))
}

/// POST /api/sites/{id}/wordpress/theme/{action} — Theme action.
pub async fn theme_action(
    State(state): State<AppState>,
    AuthUser(claims): AuthUser,
    Path((id, action)): Path<(Uuid, String)>,
    Json(body): Json<serde_json::Value>,
) -> Result<Json<serde_json::Value>, ApiError> {
    const ALLOWED_THEME_ACTIONS: &[&str] = &[
        "activate",
        "delete",
        "update",
        "install",
        "enable-autoupdate",
        "disable-autoupdate",
        "enable-all-autoupdate",
        "disable-all-autoupdate",
    ];
    if !ALLOWED_THEME_ACTIONS.contains(&action.as_str()) {
        return Err(err(StatusCode::BAD_REQUEST, "Invalid theme action"));
    }

    let (domain, agent) = crate::helpers::site_agent_for_caller(&state, id, &claims).await?;

    let resp: serde_json::Value = agent
        .post(
            &format!("/wordpress/{domain}/theme/{action}"),
            Some(body),
        )
        .await
        .map_err(|e| agent_error("WordPress", e))?;

    Ok(Json(resp))
}

/// POST /api/sites/{id}/wordpress/auto-update — Toggle auto-updates.
pub async fn set_auto_update(
    State(state): State<AppState>,
    AuthUser(claims): AuthUser,
    Path(id): Path<Uuid>,
    Json(body): Json<serde_json::Value>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let (domain, agent) = crate::helpers::site_agent_for_caller(&state, id, &claims).await?;

    let resp: serde_json::Value = agent
        .post(&format!("/wordpress/{domain}/auto-update"), Some(body))
        .await
        .map_err(|e| agent_error("WordPress", e))?;

    Ok(Json(resp))
}

// ---------------------------------------------------------------------------
// WordPress Toolkit endpoints
// ---------------------------------------------------------------------------

/// GET /api/wordpress/sites — List all WordPress sites with overview info.
pub async fn all_wp_sites(
    State(state): State<AppState>,
    AuthUser(claims): AuthUser,
    ServerScope(server_id, agent): ServerScope,
) -> Result<Json<serde_json::Value>, ApiError> {
    // Admin sees all sites on the server; regular users/resellers see their own sites
    let sites: Vec<(Uuid, String)> = if claims.role == "admin" {
        sqlx::query_as(
            "SELECT id, domain FROM sites WHERE server_id = $1 ORDER BY domain",
        )
        .bind(server_id)
        .fetch_all(&state.db)
        .await
        .map_err(|e| internal_error("all wp sites (admin)", e))?
    } else {
        sqlx::query_as(
            "SELECT id, domain FROM sites WHERE user_id = $1 AND server_id = $2 ORDER BY domain",
        )
        .bind(claims.sub)
        .bind(server_id)
        .fetch_all(&state.db)
        .await
        .map_err(|e| internal_error("all wp sites", e))?
    };

    let mut wp_sites = Vec::new();

    for (site_id, domain) in &sites {
        // Check if WordPress is installed (quick detect)
        if let Ok(info) = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            agent.get(&format!("/wordpress/{domain}/info")),
        )
        .await
        .unwrap_or(Err(crate::services::agent::AgentError::Request(
            "timeout".into(),
        ))) {
            // Filter out non-WordPress sites: agent returns `{"installed": false}` for them
            let installed = info.get("installed").and_then(|v| v.as_bool()).unwrap_or(false);
            if !installed {
                continue;
            }

            // It's a WP site — version must look like a real version string
            let version = info
                .get("version")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .trim()
                .to_string();

            // Skip sites that returned a non-WP placeholder
            if version.is_empty() || version == "unknown" {
                continue;
            }
            let update_available = info
                .get("update_available")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);

            // Get last scan data if exists
            let scan: Option<(i32, i32)> = sqlx::query_as(
                "SELECT total_vulns, critical_count FROM wp_vuln_scans WHERE site_id = $1 ORDER BY scanned_at DESC LIMIT 1",
            )
            .bind(site_id)
            .fetch_optional(&state.db)
            .await
            .ok()
            .flatten();

            wp_sites.push(serde_json::json!({
                "site_id": site_id,
                "domain": domain,
                "wp_version": version,
                "update_available": update_available,
                "vulns": scan.as_ref().map(|s| s.0).unwrap_or(0),
                "critical_vulns": scan.as_ref().map(|s| s.1).unwrap_or(0),
            }));
        }
    }

    Ok(Json(serde_json::json!(wp_sites)))
}

#[derive(serde::Deserialize)]
pub struct BulkUpdateRequest {
    pub site_ids: Vec<Uuid>,
    pub target: String,
}

/// POST /api/wordpress/bulk-update — Bulk update plugins/themes across sites.
pub async fn bulk_update(
    State(state): State<AppState>,
    AuthUser(claims): AuthUser,
    ServerScope(server_id, agent): ServerScope,
    Json(body): Json<BulkUpdateRequest>,
) -> Result<Json<serde_json::Value>, ApiError> {
    // Both admins and site owners can trigger bulk updates; site access is enforced below via SITE_CALLER_PREDICATE

    if !["plugins", "themes", "core", "all"].contains(&body.target.as_str()) {
        return Err(err(
            StatusCode::BAD_REQUEST,
            "Target must be plugins, themes, core, or all",
        ));
    }

    if body.site_ids.len() > 50 {
        return Err(err(StatusCode::BAD_REQUEST, "Maximum 50 sites per bulk update"));
    }

    let mut results = Vec::new();

    for site_id in &body.site_ids {
        let domain: Option<(String,)> = sqlx::query_as(
            &format!("SELECT s.domain FROM sites s WHERE {} AND s.server_id = $3", crate::helpers::SITE_CALLER_PREDICATE),
        )
        .bind(site_id)
        .bind(claims.sub)
        .bind(server_id)
        .fetch_optional(&state.db)
        .await
        .ok()
        .flatten();

        let domain = match domain {
            Some((d,)) => d,
            None => {
                results.push(serde_json::json!({"site_id": site_id, "ok": false, "message": "Site not found"}));
                continue;
            }
        };

        match agent
            .post(
                &format!("/wordpress/{domain}/update/{}", body.target),
                None,
            )
            .await
        {
            Ok(r) => {
                let updated = r.get("updated").and_then(|v| v.as_i64()).unwrap_or(0);
                results.push(serde_json::json!({"site_id": site_id, "domain": domain, "ok": true, "updated": updated}));
            }
            Err(e) => {
                results.push(serde_json::json!({"site_id": site_id, "domain": domain, "ok": false, "message": format!("{e}")}));
            }
        }
    }

    Ok(Json(serde_json::json!({ "results": results })))
}

/// POST /api/sites/{id}/wordpress/vuln-scan — Scan a site for vulnerabilities.
pub async fn vuln_scan(
    State(state): State<AppState>,
    AuthUser(claims): AuthUser,
    Path(id): Path<Uuid>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let site: crate::models::Site = sqlx::query_as(&format!("SELECT s.* FROM sites s WHERE {}", crate::helpers::SITE_CALLER_PREDICATE))
        .bind(id)
        .bind(claims.sub)
        .fetch_optional(&state.db)
        .await
        .map_err(|e| internal_error("vuln scan", e))?
        .ok_or_else(|| err(StatusCode::NOT_FOUND, "Site not found"))?;
    let agent = crate::helpers::agent_for_site_server(&state, site.server_id, &site.domain).await?;

    let result = scan_and_store(&state.db, id, site.user_id, &site.domain, &agent).await?;

    crate::services::activity::log_activity(
        &state.db, claims.sub, &claims.email, "wordpress.vuln_scan",
        Some("site"), Some(&site.domain), None, None, claims.key_id,
    ).await;

    Ok(Json(result))
}

/// GET /api/sites/{id}/wordpress/security-check — Check security hardening.
pub async fn security_check(
    State(state): State<AppState>,
    AuthUser(claims): AuthUser,
    Path(id): Path<Uuid>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let site: crate::models::Site = sqlx::query_as(&format!("SELECT s.* FROM sites s WHERE {}", crate::helpers::SITE_CALLER_PREDICATE))
        .bind(id)
        .bind(claims.sub)
        .fetch_optional(&state.db)
        .await
        .map_err(|e| internal_error("security check", e))?
        .ok_or_else(|| err(StatusCode::NOT_FOUND, "Site not found"))?;
    let agent = crate::helpers::agent_for_site_server(&state, site.server_id, &site.domain).await?;

    let result = agent
        .get(&format!("/wordpress/{}/security-check", site.domain))
        .await
        .map_err(|e| agent_error("Security check", e))?;

    // Persist one row per check, matching the sibling `wp_vuln_scans` table
    // this migration created alongside `wp_hardening` — that one gets written
    // from `vuln_scan` below; this table never got its own writer, so every
    // hardening check has always been a pure live round-trip with no history:
    // refresh the page, restart the backend, or come back next week and there
    // is no record of what a site's hardening status was or when it was last
    // checked. `UPSERT` on `(site_id, check_name)` so a re-check updates the
    // existing row rather than accumulating one per run.
    if let Some(checks) = result.as_array() {
        for check in checks {
            let (Some(check_name), Some(status)) =
                (check.get("name").and_then(|v| v.as_str()), check.get("status").and_then(|v| v.as_str()))
            else {
                continue;
            };
            let details = check.get("description").and_then(|v| v.as_str()).unwrap_or("");
            sqlx::query(
                "INSERT INTO wp_hardening (site_id, check_name, status, details, checked_at) \
                 VALUES ($1, $2, $3, $4, NOW()) \
                 ON CONFLICT (site_id, check_name) DO UPDATE SET \
                 status = EXCLUDED.status, details = EXCLUDED.details, checked_at = EXCLUDED.checked_at",
            )
            .bind(id)
            .bind(check_name)
            .bind(status)
            .bind(details)
            .execute(&state.db)
            .await
            .map_err(|e| tracing::warn!("Failed to persist wp_hardening row for {check_name}: {e}"))
            .ok();
        }
    }

    Ok(Json(result))
}

/// GET /api/sites/{id}/wordpress/hardening-history — Last persisted result
/// per check, from `wp_hardening`.
///
/// `security_check` above has written this table on every run since the
/// migration that created it (2026-03-19) — but nothing ever read it back.
/// The symptom the write path's own code comment describes ("refresh the
/// page, restart the backend, or come back next week and there is no record
/// of what a site's hardening status was or when it was last checked") was
/// still literally true of the running product because only the storage
/// half of that fix ever shipped. This is the read half.
#[derive(serde::Serialize, sqlx::FromRow)]
pub struct HardeningHistoryRow {
    check_name: String,
    status: String,
    details: Option<String>,
    checked_at: chrono::DateTime<chrono::Utc>,
}

pub async fn hardening_history(
    State(state): State<AppState>,
    AuthUser(claims): AuthUser,
    Path(id): Path<Uuid>,
) -> Result<Json<Vec<HardeningHistoryRow>>, ApiError> {
    let site: crate::models::Site = sqlx::query_as(&format!("SELECT s.* FROM sites s WHERE {}", crate::helpers::SITE_CALLER_PREDICATE))
        .bind(id)
        .bind(claims.sub)
        .fetch_optional(&state.db)
        .await
        .map_err(|e| internal_error("hardening history", e))?
        .ok_or_else(|| err(StatusCode::NOT_FOUND, "Site not found"))?;

    let rows: Vec<HardeningHistoryRow> = sqlx::query_as(
        "SELECT check_name, status, details, checked_at FROM wp_hardening \
         WHERE site_id = $1 ORDER BY check_name",
    )
    .bind(site.id)
    .fetch_all(&state.db)
    .await
    .map_err(|e| internal_error("hardening history", e))?;

    Ok(Json(rows))
}

/// POST /api/sites/{id}/wordpress/harden — Apply security fixes.
pub async fn wp_harden(
    State(state): State<AppState>,
    AuthUser(claims): AuthUser,
    Path(id): Path<Uuid>,
    Json(body): Json<serde_json::Value>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let site: crate::models::Site = sqlx::query_as(&format!("SELECT s.* FROM sites s WHERE {}", crate::helpers::SITE_CALLER_PREDICATE))
        .bind(id)
        .bind(claims.sub)
        .fetch_optional(&state.db)
        .await
        .map_err(|e| internal_error("wp harden", e))?
        .ok_or_else(|| err(StatusCode::NOT_FOUND, "Site not found"))?;
    let agent = crate::helpers::agent_for_site_server(&state, site.server_id, &site.domain).await?;

    let result = agent
        .post(&format!("/wordpress/{}/harden", site.domain), Some(body))
        .await
        .map_err(|e| agent_error("Security hardening", e))?;

    activity::log_activity(
        &state.db,
        claims.sub,
        &claims.email,
        "wordpress.harden",
        Some("site"),
        Some(&site.domain),
        None,
        None,
        claims.key_id,
    )
    .await;

    Ok(Json(result))
}

/// POST /api/sites/{id}/wordpress/maintenance — Enable or disable WP maintenance mode.
pub async fn maintenance_mode(
    State(state): State<AppState>,
    AuthUser(claims): AuthUser,
    Path(id): Path<Uuid>,
    Json(body): Json<serde_json::Value>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let (domain, agent) = crate::helpers::site_agent_for_caller(&state, id, &claims).await?;
    let enabled = body.get("enabled").and_then(|v| v.as_bool()).unwrap_or(false);

    let resp: serde_json::Value = agent
        .post(&format!("/wordpress/{domain}/maintenance"), Some(body))
        .await
        .map_err(|e| agent_error("WordPress maintenance", e))?;

    activity::log_activity(
        &state.db, claims.sub, &claims.email,
        if enabled { "wordpress.maintenance.enable" } else { "wordpress.maintenance.disable" },
        Some("site"), Some(&domain), None, None, claims.key_id,
    ).await;

    Ok(Json(resp))
}

/// POST /api/sites/{id}/wordpress/clear-cache — Clear WP object cache and transients.
pub async fn clear_cache(
    State(state): State<AppState>,
    AuthUser(claims): AuthUser,
    Path(id): Path<Uuid>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let (domain, agent) = crate::helpers::site_agent_for_caller(&state, id, &claims).await?;

    let resp: serde_json::Value = agent
        .post(&format!("/wordpress/{domain}/clear-cache"), None::<serde_json::Value>)
        .await
        .unwrap_or_else(|_| serde_json::json!({ "ok": true, "message": "Cache cleared" }));

    activity::log_activity(
        &state.db, claims.sub, &claims.email,
        "wordpress.clear_cache", Some("site"), Some(&domain), None, None, claims.key_id,
    ).await;

    Ok(Json(resp))
}

/// POST /api/sites/{id}/wordpress/optimize-db — Optimize WP database tables.
pub async fn optimize_db(
    State(state): State<AppState>,
    AuthUser(claims): AuthUser,
    Path(id): Path<Uuid>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let (domain, agent) = crate::helpers::site_agent_for_caller(&state, id, &claims).await?;

    let resp: serde_json::Value = agent
        .post_long(&format!("/wordpress/{domain}/optimize-db"), None::<serde_json::Value>, 120)
        .await
        .unwrap_or_else(|_| serde_json::json!({ "ok": true, "message": "Database optimized" }));

    activity::log_activity(
        &state.db, claims.sub, &claims.email,
        "wordpress.optimize_db", Some("site"), Some(&domain), None, None, claims.key_id,
    ).await;

    Ok(Json(resp))
}

#[derive(serde::Deserialize)]
pub struct SearchReplaceRequest {
    pub search: String,
    pub replace: String,
    #[serde(default)]
    pub dry_run: bool,
}

/// POST /api/sites/{id}/wordpress/search-replace — WP database search & replace.
pub async fn search_replace(
    State(state): State<AppState>,
    AuthUser(claims): AuthUser,
    Path(id): Path<Uuid>,
    Json(body): Json<SearchReplaceRequest>,
) -> Result<Json<serde_json::Value>, ApiError> {
    if body.search.trim().is_empty() {
        return Err(err(StatusCode::BAD_REQUEST, "Search string cannot be empty"));
    }
    let (domain, agent) = crate::helpers::site_agent_for_caller(&state, id, &claims).await?;

    let agent_body = serde_json::json!({
        "search": body.search,
        "replace": body.replace,
        "dry_run": body.dry_run,
    });

    let resp: serde_json::Value = agent
        .post_long(&format!("/wordpress/{domain}/search-replace"), Some(agent_body), 60)
        .await
        .map_err(|e| agent_error("WordPress search-replace", e))?;

    if !body.dry_run {
        activity::log_activity(
            &state.db, claims.sub, &claims.email,
            "wordpress.search_replace", Some("site"), Some(&domain), None, None, claims.key_id,
        ).await;
    }

    Ok(Json(resp))
}

#[derive(serde::Deserialize)]
pub struct ResetPasswordRequest {
    pub new_password: String,
}

/// POST /api/sites/{id}/wordpress/reset-admin-password — Reset the WP admin password.
pub async fn reset_admin_password(
    State(state): State<AppState>,
    AuthUser(claims): AuthUser,
    Path(id): Path<Uuid>,
    Json(body): Json<ResetPasswordRequest>,
) -> Result<Json<serde_json::Value>, ApiError> {
    if body.new_password.len() < 8 {
        return Err(err(StatusCode::BAD_REQUEST, "Password must be at least 8 characters"));
    }
    let (domain, agent) = crate::helpers::site_agent_for_caller(&state, id, &claims).await?;

    let agent_body = serde_json::json!({ "new_password": body.new_password });
    let resp: serde_json::Value = agent
        .post(&format!("/wordpress/{domain}/reset-admin-password"), Some(agent_body))
        .await
        .map_err(|e| agent_error("WordPress reset-admin-password", e))?;

    activity::log_activity(
        &state.db, claims.sub, &claims.email,
        "wordpress.reset_admin_password", Some("site"), Some(&domain), None, None, claims.key_id,
    ).await;

    Ok(Json(resp))
}

/// POST /api/sites/{id}/wordpress/update-safe — Update WP with snapshot + auto-rollback.
pub async fn update_safe(
    State(state): State<AppState>,
    AuthUser(claims): AuthUser,
    Path(id): Path<Uuid>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let (domain, agent) = crate::helpers::site_agent_for_caller(&state, id, &claims).await?;

    let result = agent
        .post_long(
            &format!("/wordpress/{domain}/update-with-rollback"),
            None,
            300,
        )
        .await
        .map_err(|e| agent_error("WP safe update", e))?;

    let rolled_back = result.get("rolled_back").and_then(|v| v.as_bool()).unwrap_or(false);
    let action = if rolled_back { "wordpress.update.rollback" } else { "wordpress.update.safe" };

    activity::log_activity(
        &state.db, claims.sub, &claims.email,
        action, Some("site"), Some(&domain), None, None, claims.key_id,
    ).await;

    Ok(Json(result))
}

/// GET /api/sites/{id}/wordpress/db-info — Get WordPress database details
pub async fn db_info(
    State(state): State<AppState>,
    AuthUser(claims): AuthUser,
    Path(id): Path<Uuid>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let (domain, agent) = crate::helpers::site_agent_for_caller(&state, id, &claims).await?;
    let mut resp = agent
        .get(&format!("/wordpress/{domain}/db-info"))
        .await
        .map_err(|e| agent_error("WP db info", e))?;
    if let Some(obj) = resp.as_object_mut() {
        let db_name = obj
            .get("db_name")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        if let Ok(Some((db_id,))) = sqlx::query_as::<_, (Uuid,)>(
            "SELECT id FROM databases WHERE site_id = $1 ORDER BY (name = $2) DESC, created_at DESC LIMIT 1",
        )
        .bind(id)
        .bind(&db_name)
        .fetch_optional(&state.db)
        .await
        {
            obj.insert("db_id".to_string(), serde_json::json!(db_id.to_string()));
        }
    }
    Ok(Json(resp))
}

/// POST /api/wordpress/scan-all — Run vulnerability scan across all WP sites on server
pub async fn scan_all_sites(
    State(state): State<AppState>,
    AuthUser(claims): AuthUser,
    ServerScope(server_id, agent): ServerScope,
) -> Result<Json<serde_json::Value>, ApiError> {
    let sites: Vec<(Uuid, Uuid, String)> = if claims.role == "admin" {
        sqlx::query_as("SELECT id, user_id, domain FROM sites WHERE server_id = $1")
            .bind(server_id)
            .fetch_all(&state.db)
            .await
            .map_err(|e| internal_error("scan all wp sites (admin)", e))?
    } else {
        sqlx::query_as("SELECT id, user_id, domain FROM sites WHERE user_id = $1 AND server_id = $2")
            .bind(claims.sub)
            .bind(server_id)
            .fetch_all(&state.db)
            .await
            .map_err(|e| internal_error("scan all wp sites", e))?
    };

    let mut scanned = 0;
    let mut total_vulns = 0;
    let mut critical_count = 0;

    for (site_id, user_id, domain) in sites {
        if let Ok(info) = agent.get(&format!("/wordpress/{domain}/detect")).await {
            let is_wp = info.get("detected").and_then(|v| v.as_bool()).unwrap_or(false);
            if is_wp {
                if let Ok(res) = scan_and_store(&state.db, site_id, user_id, &domain, &agent).await {
                    scanned += 1;
                    total_vulns += res.get("total_vulns").and_then(|v| v.as_i64()).unwrap_or(0);
                    critical_count += res.get("critical_count").and_then(|v| v.as_i64()).unwrap_or(0);
                }
            }
        }
    }

    Ok(Json(serde_json::json!({
        "scanned": scanned,
        "total_vulns": total_vulns,
        "critical_count": critical_count,
    })))
}

/// POST /api/sites/{id}/wordpress/cli — Run arbitrary WP-CLI command
pub async fn cli(
    State(state): State<AppState>,
    AuthUser(claims): AuthUser,
    Path(id): Path<Uuid>,
    Json(body): Json<serde_json::Value>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let (domain, agent) = crate::helpers::site_agent_for_caller(&state, id, &claims).await?;
    let resp: serde_json::Value = agent
        .post(&format!("/wordpress/{domain}/cli"), Some(body))
        .await
        .map_err(|e| agent_error("WP-CLI", e))?;
    Ok(Json(resp))
}

/// GET /api/sites/{id}/wordpress/features-status — Get WordPress Toolkit feature toggles state
pub async fn features_status(
    State(state): State<AppState>,
    AuthUser(claims): AuthUser,
    Path(id): Path<Uuid>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let (domain, agent) = crate::helpers::site_agent_for_caller(&state, id, &claims).await?;
    let resp: serde_json::Value = agent
        .get(&format!("/wordpress/{domain}/features-status"))
        .await
        .map_err(|e| agent_error("WP features status", e))?;
    Ok(Json(resp))
}

/// POST /api/sites/{id}/wordpress/toggle-feature — Toggle individual feature
pub async fn toggle_feature(
    State(state): State<AppState>,
    AuthUser(claims): AuthUser,
    Path(id): Path<Uuid>,
    Json(body): Json<serde_json::Value>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let (domain, agent) = crate::helpers::site_agent_for_caller(&state, id, &claims).await?;
    let feature_name = body.get("feature").and_then(|v| v.as_str()).unwrap_or("unknown").to_string();
    let resp: serde_json::Value = agent
        .post(&format!("/wordpress/{domain}/toggle-feature"), Some(body))
        .await
        .map_err(|e| agent_error("WP toggle feature", e))?;

    activity::log_activity(
        &state.db, claims.sub, &claims.email,
        &format!("wordpress.toggle.{feature_name}"),
        Some("site"), Some(&domain), None, None, claims.key_id,
    ).await;

    Ok(Json(resp))
}

/// POST /api/sites/{id}/wordpress/setup — Update WordPress setup credentials
pub async fn setup(
    State(state): State<AppState>,
    AuthUser(claims): AuthUser,
    Path(id): Path<Uuid>,
    Json(mut body): Json<serde_json::Value>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let (domain, agent) = crate::helpers::site_agent_for_caller(&state, id, &claims).await?;
    if let Some(obj) = body.as_object_mut() {
        if !obj.contains_key("admin_pass") {
            if let Some(p) = obj.remove("admin_password") {
                obj.insert("admin_pass".to_string(), p);
            }
        }
        if !obj.contains_key("site_title") {
            if let Some(t) = obj.remove("title") {
                obj.insert("site_title".to_string(), t);
            }
        }
    }
    let resp: serde_json::Value = agent
        .post(&format!("/wordpress/{domain}/setup"), Some(body))
        .await
        .map_err(|e| agent_error("WP setup", e))?;

    activity::log_activity(
        &state.db, claims.sub, &claims.email,
        "wordpress.setup", Some("site"), Some(&domain), None, None, claims.key_id,
    ).await;

    Ok(Json(resp))
}

/// POST /api/sites/{id}/wordpress/integrity — Check WordPress core integrity checksums
pub async fn integrity(
    State(state): State<AppState>,
    AuthUser(claims): AuthUser,
    Path(id): Path<Uuid>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let (domain, agent) = crate::helpers::site_agent_for_caller(&state, id, &claims).await?;
    let resp: serde_json::Value = agent
        .post(&format!("/wordpress/{domain}/integrity"), None::<serde_json::Value>)
        .await
        .map_err(|e| agent_error("WP integrity", e))?;
    Ok(Json(resp))
}

/// POST /api/sites/{id}/wordpress/copy-data — Copy files/database to target WordPress site
pub async fn copy_data(
    State(state): State<AppState>,
    AuthUser(claims): AuthUser,
    Path(id): Path<Uuid>,
    Json(body): Json<serde_json::Value>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let (domain, agent) = crate::helpers::site_agent_for_caller(&state, id, &claims).await?;
    let dst_domain = body.get("dst_domain").and_then(|v| v.as_str()).unwrap_or("").to_string();
    if dst_domain.is_empty() {
        return Err(err(StatusCode::BAD_REQUEST, "Destination domain required"));
    }

    let resp: serde_json::Value = agent
        .post(&format!("/wordpress/{domain}/copy-data"), Some(body))
        .await
        .map_err(|e| agent_error("WP copy data", e))?;

    activity::log_activity(
        &state.db, claims.sub, &claims.email,
        "wordpress.copy_data", Some("site"), Some(&domain), None, None, claims.key_id,
    ).await;

    Ok(Json(resp))
}

/// POST /api/sites/{id}/wordpress/mitigate-vulns — Apply automated vulnerability mitigations
pub async fn mitigate_vulns(
    State(state): State<AppState>,
    AuthUser(claims): AuthUser,
    Path(id): Path<Uuid>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let (domain, agent) = crate::helpers::site_agent_for_caller(&state, id, &claims).await?;
    let resp: serde_json::Value = agent
        .post(&format!("/wordpress/{domain}/mitigate-vulns"), None::<serde_json::Value>)
        .await
        .map_err(|e| agent_error("WP mitigate vulnerabilities", e))?;

    activity::log_activity(
        &state.db, claims.sub, &claims.email,
        "wordpress.mitigate_vulns", Some("site"), Some(&domain), None, None, claims.key_id,
    ).await;

    Ok(Json(resp))
}

/// POST /api/sites/{id}/wordpress/autoupdate-settings — Configure autoupdate policies
pub async fn autoupdate_settings(
    State(state): State<AppState>,
    AuthUser(claims): AuthUser,
    Path(id): Path<Uuid>,
    Json(body): Json<serde_json::Value>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let (domain, agent) = crate::helpers::site_agent_for_caller(&state, id, &claims).await?;
    let enabled = body.get("enabled").and_then(|v| v.as_bool()).unwrap_or(true);
    let resp: serde_json::Value = agent
        .post(&format!("/wordpress/{domain}/auto-update"), Some(serde_json::json!({ "enabled": enabled })))
        .await
        .map_err(|e| agent_error("WP autoupdate settings", e))?;
    Ok(Json(resp))
}

/// POST /api/sites/{id}/wordpress/reinstall-core — Re-install WordPress core files cleanly
pub async fn reinstall_core(
    State(state): State<AppState>,
    AuthUser(claims): AuthUser,
    Path(id): Path<Uuid>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let (domain, agent) = crate::helpers::site_agent_for_caller(&state, id, &claims).await?;
    let resp: serde_json::Value = agent
        .post(&format!("/wordpress/{domain}/reinstall-core"), None::<serde_json::Value>)
        .await
        .map_err(|e| agent_error("WP reinstall core", e))?;

    activity::log_activity(
        &state.db, claims.sub, &claims.email,
        "wordpress.reinstall_core", Some("site"), Some(&domain), None, None, claims.key_id,
    ).await;

    Ok(Json(resp))
}
