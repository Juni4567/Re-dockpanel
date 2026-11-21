use axum::{
    extract::Path,
    http::StatusCode,
    routing::{get, post},
    Json, Router,
};

use crate::routes::{is_valid_domain, AppState};
use crate::services::wordpress;

#[derive(serde::Deserialize)]
pub struct InstallRequest {
    url: String,
    title: String,
    admin_user: String,
    admin_pass: String,
    admin_email: String,
    db_name: String,
    db_user: String,
    db_pass: String,
    db_host: String,
}

#[derive(serde::Deserialize)]
pub struct NameRequest {
    name: String,
}

#[derive(serde::Deserialize)]
pub struct AutoUpdateRequest {
    enabled: bool,
}

fn validate_domain(domain: &str) -> Result<(), (StatusCode, Json<serde_json::Value>)> {
    if !is_valid_domain(domain) {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "Invalid domain"})),
        ));
    }
    Ok(())
}

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/wordpress/{domain}/detect", get(detect))
        .route("/wordpress/{domain}/info", get(info))
        .route("/wordpress/{domain}/plugins", get(plugins))
        .route("/wordpress/{domain}/themes", get(themes))
        .route("/wordpress/{domain}/install", post(install))
        .route("/wordpress/{domain}/promote-https", post(promote_https))
        .route(
            "/wordpress/{domain}/update/{target}",
            post(update),
        )
        .route(
            "/wordpress/{domain}/plugin/{action}",
            post(plugin_action),
        )
        .route(
            "/wordpress/{domain}/theme/{action}",
            post(theme_action),
        )
        .route(
            "/wordpress/{domain}/auto-update",
            get(get_auto_update).post(set_auto_update),
        )
        .route("/wordpress/{domain}/vuln-scan", post(vuln_scan))
        .route("/wordpress/{domain}/security-check", get(security_check))
        .route("/wordpress/{domain}/harden", post(harden))
        .route("/wordpress/{domain}/update-with-rollback", post(update_with_rollback))
        .route("/wordpress/{domain}/clear-cache", post(clear_cache))
        .route("/wordpress/{domain}/optimize-db", post(optimize_db))
        .route("/wordpress/{domain}/search-replace", post(search_replace_route))
        .route("/wordpress/{domain}/reset-admin-password", post(reset_admin_password))
}

async fn detect(
    Path(domain): Path<String>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    validate_domain(&domain)?;
    Ok(Json(serde_json::json!({
        "detected": wordpress::detect(&domain),
    })))
}

async fn info(
    Path(domain): Path<String>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    validate_domain(&domain)?;
    if !wordpress::detect(&domain) {
        return Ok(Json(serde_json::json!({ "installed": false })));
    }
    wordpress::info(&domain).await.map(Json).map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": e})),
        )
    })
}

async fn plugins(
    Path(domain): Path<String>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    validate_domain(&domain)?;
    wordpress::plugins(&domain).await.map(Json).map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": e})),
        )
    })
}

async fn themes(
    Path(domain): Path<String>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    validate_domain(&domain)?;
    wordpress::themes(&domain).await.map(Json).map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": e})),
        )
    })
}

/// POST /wordpress/{domain}/promote-https — move `siteurl`/`home` to HTTPS.
///
/// `enable_ssl_for_site` already does this for every site that exists when its
/// certificate arrives. This covers the other order: a certificate that lands
/// while WordPress is still being installed has nothing to promote yet, so the
/// panel calls back here once the install finishes.
///
/// The target URL is derived from the path domain, never accepted from the
/// caller — a settable `siteurl` is a site-takeover primitive, since WordPress
/// loads its own scripts from it.
async fn promote_https(
    Path(domain): Path<String>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    validate_domain(&domain)?;
    let outcome = wordpress::promote_site_url_to_https(&domain).await;
    if let Some(err) = outcome.failure() {
        return Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": err })),
        ));
    }
    Ok(Json(serde_json::json!({ "canonical_url": outcome })))
}

async fn install(
    Path(domain): Path<String>,
    Json(body): Json<InstallRequest>,
) -> Result<(StatusCode, Json<serde_json::Value>), (StatusCode, Json<serde_json::Value>)> {
    validate_domain(&domain)?;
    wordpress::install(
        &domain,
        &body.url,
        &body.title,
        &body.admin_user,
        &body.admin_pass,
        &body.admin_email,
        &body.db_name,
        &body.db_user,
        &body.db_pass,
        &body.db_host,
    )
    .await
    .map(|msg| {
        (
            StatusCode::CREATED,
            Json(serde_json::json!({ "ok": true, "message": msg })),
        )
    })
    .map_err(|e| {
        (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(serde_json::json!({"error": e})),
        )
    })
}

async fn update(
    Path((domain, target)): Path<(String, String)>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    validate_domain(&domain)?;
    let result = match target.as_str() {
        "core" => wordpress::update_core(&domain).await,
        "plugins" => wordpress::update_all_plugins(&domain).await,
        "themes" => wordpress::update_all_themes(&domain).await,
        _ => Err("Invalid target. Use: core, plugins, themes".into()),
    };
    result
        .map(|msg| Json(serde_json::json!({ "ok": true, "message": msg })))
        .map_err(|e| {
            (
                StatusCode::UNPROCESSABLE_ENTITY,
                Json(serde_json::json!({"error": e})),
            )
        })
}

async fn plugin_action(
    Path((domain, action)): Path<(String, String)>,
    Json(body): Json<NameRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    validate_domain(&domain)?;
    wordpress::plugin_action(&domain, &body.name, &action)
        .await
        .map(|msg| Json(serde_json::json!({ "ok": true, "message": msg })))
        .map_err(|e| {
            (
                StatusCode::UNPROCESSABLE_ENTITY,
                Json(serde_json::json!({"error": e})),
            )
        })
}

async fn theme_action(
    Path((domain, action)): Path<(String, String)>,
    Json(body): Json<NameRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    validate_domain(&domain)?;
    wordpress::theme_action(&domain, &body.name, &action)
        .await
        .map(|msg| Json(serde_json::json!({ "ok": true, "message": msg })))
        .map_err(|e| {
            (
                StatusCode::UNPROCESSABLE_ENTITY,
                Json(serde_json::json!({"error": e})),
            )
        })
}

async fn get_auto_update(
    Path(domain): Path<String>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    validate_domain(&domain)?;
    Ok(Json(serde_json::json!({
        "enabled": wordpress::is_auto_update_enabled(&domain),
    })))
}

async fn set_auto_update(
    Path(domain): Path<String>,
    Json(body): Json<AutoUpdateRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    validate_domain(&domain)?;
    wordpress::set_auto_update(&domain, body.enabled)
        .await
        .map(|_| Json(serde_json::json!({ "ok": true })))
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error": e})),
            )
        })
}

/// POST /wordpress/{domain}/vuln-scan — Scan for plugin vulnerabilities
async fn vuln_scan(
    Path(domain): Path<String>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    validate_domain(&domain)?;
    crate::services::wp_vulnerability::scan_site(&domain)
        .await
        .map(|result| Json(serde_json::to_value(result).unwrap_or_default()))
        .map_err(|e| {
            // 422, like every other route in this file, and for the reason the
            // panel enforces: `error.rs` passes an agent's sentence through on a
            // 4xx and replaces anything else with "Operation failed. Reference:
            // <uuid>". This arm was unreachable until `scan_site` learned to
            // fail instead of reporting zero vulnerabilities, so the collapse it
            // would have caused had never been observed.
            (
                StatusCode::UNPROCESSABLE_ENTITY,
                Json(serde_json::json!({"error": e})),
            )
        })
}

/// GET /wordpress/{domain}/security-check — Check security hardening status
async fn security_check(
    Path(domain): Path<String>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    validate_domain(&domain)?;
    crate::services::wp_vulnerability::check_security(&domain)
        .await
        .map(|checks| Json(serde_json::to_value(checks).unwrap_or_default()))
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error": e})),
            )
        })
}

/// POST /wordpress/{domain}/harden — Apply security hardening fixes
async fn harden(
    Path(domain): Path<String>,
    Json(body): Json<serde_json::Value>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    validate_domain(&domain)?;
    let fixes: Vec<String> = body["fixes"]
        .as_array()
        .map(|arr| arr.iter().filter_map(|v| v.as_str().map(|s| s.to_string())).collect())
        .unwrap_or_default();
    if fixes.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "No fixes specified"})),
        ));
    }
    crate::services::wp_vulnerability::apply_hardening(&domain, &fixes)
        .await
        .map(|results| Json(serde_json::to_value(results).unwrap_or_default()))
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error": e})),
            )
        })
}

/// POST /wordpress/{domain}/update-with-rollback — Update WP with snapshot + auto-rollback.
async fn update_with_rollback(
    Path(domain): Path<String>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    validate_domain(&domain)?;
    wordpress::update_with_rollback(&domain)
        .await
        .map(Json)
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error": e})),
            )
        })
}

/// POST /wordpress/{domain}/clear-cache — Clear WP object cache and transients.
async fn clear_cache(
    Path(domain): Path<String>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    validate_domain(&domain)?;
    // Try wp-cli cache flush and delete expired transients
    let cache_result = tokio::process::Command::new("wp")
        .args([
            "cache", "flush",
            &format!("--path=/var/www/{domain}/wordpress"),
            "--allow-root",
        ])
        .output()
        .await;

    let transient_result = tokio::process::Command::new("wp")
        .args([
            "transient", "delete", "--expired",
            &format!("--path=/var/www/{domain}/wordpress"),
            "--allow-root",
        ])
        .output()
        .await;

    let cache_ok = cache_result.map(|o| o.status.success()).unwrap_or(false);
    let transient_ok = transient_result.map(|o| o.status.success()).unwrap_or(false);

    Ok(Json(serde_json::json!({
        "ok": true,
        "cache_flushed": cache_ok,
        "transients_cleared": transient_ok,
        "message": "Cache cleared successfully",
    })))
}

/// POST /wordpress/{domain}/optimize-db — Optimize WP database tables.
async fn optimize_db(
    Path(domain): Path<String>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    validate_domain(&domain)?;
    let output = tokio::process::Command::new("wp")
        .args([
            "db", "optimize",
            &format!("--path=/var/www/{domain}/wordpress"),
            "--allow-root",
        ])
        .output()
        .await;

    match output {
        Ok(o) => {
            let stdout = String::from_utf8_lossy(&o.stdout).to_string();
            Ok(Json(serde_json::json!({
                "ok": o.status.success(),
                "message": if o.status.success() { "Database optimized successfully" } else { "Optimization completed with warnings" },
                "details": stdout,
            })))
        }
        Err(e) => Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": format!("Failed to run db optimize: {e}")})),
        )),
    }
}

#[derive(serde::Deserialize)]
struct SearchReplaceBody {
    search: String,
    replace: String,
    #[serde(default)]
    dry_run: bool,
}

/// POST /wordpress/{domain}/search-replace — WP database search & replace.
async fn search_replace_route(
    Path(domain): Path<String>,
    Json(body): Json<SearchReplaceBody>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    validate_domain(&domain)?;
    if body.search.trim().is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "Search string cannot be empty"})),
        ));
    }

    let mut args = vec![
        "search-replace".to_string(),
        body.search.clone(),
        body.replace.clone(),
        format!("--path=/var/www/{domain}/wordpress"),
        "--allow-root".to_string(),
        "--report-changed-only".to_string(),
    ];
    if body.dry_run {
        args.push("--dry-run".to_string());
    }

    let output = tokio::process::Command::new("wp")
        .args(&args)
        .output()
        .await;

    match output {
        Ok(o) => {
            let stdout = String::from_utf8_lossy(&o.stdout).to_string();
            let stderr = String::from_utf8_lossy(&o.stderr).to_string();
            Ok(Json(serde_json::json!({
                "ok": o.status.success(),
                "dry_run": body.dry_run,
                "output": stdout,
                "stderr": if stderr.is_empty() { serde_json::Value::Null } else { serde_json::Value::String(stderr) },
                "message": if body.dry_run { "Dry run complete — no changes made" } else { "Search & replace completed" },
            })))
        }
        Err(e) => Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": format!("Failed to run search-replace: {e}")})),
        )),
    }
}

#[derive(serde::Deserialize)]
struct ResetPasswordBody {
    new_password: String,
}

/// POST /wordpress/{domain}/reset-admin-password — Reset the WP admin user password.
async fn reset_admin_password(
    Path(domain): Path<String>,
    Json(body): Json<ResetPasswordBody>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    validate_domain(&domain)?;
    if body.new_password.len() < 8 {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "Password must be at least 8 characters"})),
        ));
    }

    // Get the first admin user login
    let get_admin = tokio::process::Command::new("wp")
        .args([
            "user", "list",
            "--role=administrator",
            "--fields=user_login",
            "--format=csv",
            "--limit=1",
            &format!("--path=/var/www/{domain}/wordpress"),
            "--allow-root",
        ])
        .output()
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({"error": format!("Could not list users: {e}")}))))?;

    let output_str = String::from_utf8_lossy(&get_admin.stdout);
    // CSV output: first line is header "user_login", second is the value
    let admin_login = output_str
        .lines()
        .nth(1)
        .unwrap_or("")
        .trim()
        .to_string();

    if admin_login.is_empty() {
        return Err((
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": "No administrator user found on this WordPress site"})),
        ));
    }

    let result = tokio::process::Command::new("wp")
        .args([
            "user", "update", &admin_login,
            &format!("--user_pass={}", body.new_password),
            &format!("--path=/var/www/{domain}/wordpress"),
            "--allow-root",
        ])
        .output()
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({"error": format!("Failed to reset password: {e}")}))))?;

    if result.status.success() {
        Ok(Json(serde_json::json!({
            "ok": true,
            "admin_login": admin_login,
            "message": format!("Password reset successfully for user '{admin_login}'"),
        })))
    } else {
        let stderr = String::from_utf8_lossy(&result.stderr).to_string();
        Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": format!("Failed to reset password: {stderr}")})),
        ))
    }
}
