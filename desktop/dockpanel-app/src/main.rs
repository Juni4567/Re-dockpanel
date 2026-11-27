#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod backups;
mod databases;
mod downloads;
mod files;
mod git_repo;
mod hosts_manager;
pub mod live_sync;
pub mod mail_catcher;
pub mod mariadb;
pub mod router;
pub mod ssl;
mod state;
mod tray;
mod updater;
pub mod wordpress;

use axum::{
    extract::{Path, Query, State},
    http::{header, StatusCode},
    response::{
        sse::{Event as SseEvent, KeepAlive, Sse},
        IntoResponse, Response,
    },
    routing::{delete, get, post, put},
    Json, Router,
};
use rust_embed::RustEmbed;
use state::AppState;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use tao::{
    dpi::LogicalSize,
    event::{Event, WindowEvent},
    event_loop::{ControlFlow, EventLoopBuilder},
    window::WindowBuilder,
};
use tower_http::cors::CorsLayer;
use wry::WebViewBuilder;

#[derive(RustEmbed)]
#[folder = "../../panel/frontend/dist/"]
struct Assets;

pub fn open_external_url(url: &str) -> bool {
    let trimmed = url.trim();
    if !trimmed.starts_with("http://") && !trimmed.starts_with("https://") {
        return false;
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x08000000;
        let res = std::process::Command::new("rundll32.exe")
            .args(["url.dll,FileProtocolHandler", trimmed])
            .creation_flags(CREATE_NO_WINDOW)
            .spawn();
        res.is_ok()
    }
    #[cfg(not(windows))]
    {
        let _ = trimmed;
        true
    }
}

fn resolve_desktop_path(raw: &str) -> String {
    if raw.contains("dockpanel-desktop") {
        if let Some(domain) = raw.split('\\').filter(|s| !s.is_empty()).last() {
            if let Some(user_profile) = std::env::var_os("USERPROFILE") {
                let native_dir = PathBuf::from(user_profile)
                    .join("DockPanel")
                    .join("sites")
                    .join(domain);
                if native_dir.exists() {
                    return native_dir.to_string_lossy().to_string();
                }
            }
        }
    }
    raw.to_string()
}

async fn static_handler(uri: axum::http::Uri) -> impl IntoResponse {
    let mut path = uri.path().trim_start_matches('/');
    if path.is_empty() {
        path = "index.html";
    }

    match Assets::get(path) {
        Some(content) => {
            let mime = mime_guess::from_path(path).first_or_octet_stream();
            (
                StatusCode::OK,
                [(header::CONTENT_TYPE, mime.as_ref())],
                content.data.to_vec(),
            )
                .into_response()
        }
        None => {
            // SPA client-side fallback
            match Assets::get("index.html") {
                Some(content) => (
                    StatusCode::OK,
                    [(header::CONTENT_TYPE, "text/html; charset=utf-8")],
                    content.data.to_vec(),
                )
                    .into_response(),
                None => (StatusCode::NOT_FOUND, "Not Found").into_response(),
            }
        }
    }
}

async fn list_sites_handler(State(state): State<Arc<AppState>>) -> Json<serde_json::Value> {
    let sites = state.list_sites();
    Json(serde_json::to_value(&sites).unwrap_or_else(|_| serde_json::json!([])))
}

async fn get_site_handler(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Response {
    if let Some(site) = state.get_site(&id) {
        Json(site).into_response()
    } else {
        (StatusCode::NOT_FOUND, Json(serde_json::json!({"error": "Site not found"}))).into_response()
    }
}

async fn create_site_handler(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<serde_json::Value>,
) -> Response {
    let domain = payload.get("domain").and_then(|v| v.as_str()).unwrap_or("site.test");
    let runtime = payload
        .get("runtime")
        .and_then(|v| v.as_str())
        .unwrap_or("php");
    let php = payload.get("php_version").and_then(|v| v.as_str());
    let app_command = payload.get("app_command").and_then(|v| v.as_str());
    let proxy_port = payload
        .get("proxy_port")
        .and_then(|v| v.as_u64())
        .and_then(|p| u16::try_from(p).ok());
    let cms = payload.get("cms").and_then(|v| v.as_str());
    let preset = payload.get("php_preset").and_then(|v| v.as_str());
    let connect_existing = payload
        .get("connect_existing")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let custom_root = payload
        .get("document_root")
        .or_else(|| payload.get("root_path"))
        .and_then(|v| v.as_str());
    let stream = payload.get("stream").and_then(|v| v.as_bool()).unwrap_or(false);
    let requested_db_engine = payload
        .get("db_engine")
        .and_then(|v| v.as_str())
        .unwrap_or("sqlite")
        .to_string();

    #[cfg(not(test))]
    if runtime == "node" && state::find_node_binary().is_none() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "error": "Node.js was not found on PATH or standard install folders. Install Node.js (https://nodejs.org) and try again."
            })),
        )
            .into_response();
    }

    match state.create_site_with_options(domain, Some(runtime), php, custom_root, app_command, proxy_port) {
        Ok(site) => {
            if let Some(p) = cms.or(preset).filter(|s| !s.is_empty() && *s != "generic") {
                state.update_site_preset(&site.id, p);
            }
            let site_dir = state.site_dir(&site);
            let existing_wp = connect_existing && wordpress::inspect_site_folder(&site_dir).is_wordpress;
            let wants_wp = !connect_existing
                && !existing_wp
                && site.runtime == "php"
                && (cms == Some("wordpress") || preset == Some("wordpress"));
            let prov_key = format!("site:{}", site.id);

            let wp_req = wordpress::InstallWpRequest {
                admin_user: payload.get("admin_user").and_then(|v| v.as_str()).map(|s| s.to_string()),
                admin_pass: payload
                    .get("admin_password")
                    .or_else(|| payload.get("admin_pass"))
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string()),
                admin_email: payload.get("admin_email").and_then(|v| v.as_str()).map(|s| s.to_string()),
                title: payload
                    .get("site_title")
                    .or_else(|| payload.get("title"))
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string()),
                url: Some(format!("http://{}", site.domain)),
                db_engine: Some(requested_db_engine.clone()),
                db_port: state.get_mariadb_port(),
            };

            if stream && wants_wp {
                state.push_install_progress(
                    &prov_key,
                    "dir",
                    &format!("Created local site folder: {}", site_dir.display()),
                    "done",
                    None,
                    None,
                    false,
                );
                state.push_install_progress(
                    &prov_key,
                    "hosts",
                    &format!("Registered {} in Windows hosts & local SSL", site.domain),
                    "done",
                    None,
                    None,
                    false,
                );
                state.push_install_progress(
                    &prov_key,
                    "cms",
                    &format!("Checking local WordPress archive cache for {}...", site.domain),
                    "in_progress",
                    Some(0),
                    None,
                    false,
                );

                let bg_state = state.clone();
                let bg_site_id = site.id.clone();
                let bg_domain = site.domain.clone();
                let bg_php = site.php_version.clone();
                let mut bg_wp_req = wp_req;
                let prov_key_bg = prov_key.clone();
                tokio::spawn(async move {
                    let use_mariadb = bg_wp_req.db_engine.as_deref() == Some("mariadb");
                    if use_mariadb {
                        if bg_state.get_mariadb_port().is_none() {
                            if let Err(e) = mariadb::install_mariadb_with_progress(|step, lbl, pct, msg| {
                                bg_state.push_install_progress(
                                    &prov_key_bg,
                                    step,
                                    lbl,
                                    "in_progress",
                                    pct,
                                    msg.as_deref(),
                                    false,
                                );
                            })
                            .await
                            {
                                bg_state.push_install_progress(
                                    &prov_key_bg,
                                    "complete",
                                    &format!("MariaDB install failed: {}", e),
                                    "error",
                                    None,
                                    Some(&e),
                                    true,
                                );
                                return;
                            }
                            let start_res = tokio::task::spawn_blocking({
                                let st = bg_state.clone();
                                move || st.start_mariadb()
                            })
                            .await;
                            match start_res {
                                Ok(Ok(p)) => bg_wp_req.db_port = Some(p),
                                Ok(Err(e)) => {
                                    bg_state.push_install_progress(
                                        &prov_key_bg,
                                        "complete",
                                        &format!("Could not start MariaDB: {}", e),
                                        "error",
                                        None,
                                        Some(&e),
                                        true,
                                    );
                                    return;
                                }
                                Err(e) => {
                                    let msg = e.to_string();
                                    bg_state.push_install_progress(
                                        &prov_key_bg,
                                        "complete",
                                        &format!("MariaDB task error: {}", msg),
                                        "error",
                                        None,
                                        Some(&msg),
                                        true,
                                    );
                                    return;
                                }
                            }
                        } else {
                            bg_wp_req.db_port = bg_state.get_mariadb_port();
                        }
                    }

                    if let Err(e) = wordpress::ensure_wp_archives_with_progress(!use_mariadb, |step, lbl, pct, msg| {
                        bg_state.push_install_progress(
                            &prov_key_bg,
                            step,
                            lbl,
                            "in_progress",
                            pct,
                            msg.as_deref(),
                            false,
                        );
                    })
                    .await
                    {
                        bg_state.push_install_progress(
                            &prov_key_bg,
                            "complete",
                            &format!("WordPress download failed: {}", e),
                            "error",
                            None,
                            Some(&e),
                            true,
                        );
                        return;
                    }

                    let db_label = if use_mariadb { "MariaDB" } else { "SQLite" };
                    bg_state.push_install_progress(
                        &prov_key_bg,
                        "cms",
                        "WordPress Core archive ready (cached locally)",
                        "done",
                        None,
                        None,
                        false,
                    );

                    let dom_cl = bg_domain.clone();
                    let req_cl = bg_wp_req.clone();
                    let sd_cl = site_dir.clone();
                    let st_cb = bg_state.clone();
                    let pk_cb = prov_key_bg.clone();
                    let install_res = tokio::task::spawn_blocking(move || {
                        wordpress::install_site_wordpress_with_progress(&sd_cl, &dom_cl, &req_cl, |step, lbl, status, pct| {
                            st_cb.push_install_progress(&pk_cb, step, lbl, status, pct, None, false);
                        })
                    })
                    .await;

                    match install_res {
                        Ok(Ok(_)) => {
                            let _ = bg_state.ensure_site_ssl(&bg_domain);
                            let _ = bg_state.start_site(&bg_site_id);
                            let port = bg_state.get_router_config().active_port;
                            let url = if port == 80 {
                                format!("http://{}", bg_domain)
                            } else {
                                format!("http://{}:{}", bg_domain, port)
                            };
                            bg_state.record_event(
                                "create_site",
                                "site",
                                &bg_domain,
                                "WordPress Site Created",
                                &format!("Created local WordPress site {} (PHP {}, {})", url, bg_php, db_label),
                                "success",
                            );
                            bg_state.push_install_progress(
                                &prov_key_bg,
                                "php",
                                &format!("Started local PHP {} worker & HTTP/HTTPS router", bg_php),
                                "done",
                                None,
                                None,
                                false,
                            );
                            bg_state.push_install_progress(
                                &prov_key_bg,
                                "complete",
                                &format!("WordPress is live at {}", url),
                                "done",
                                None,
                                None,
                                true,
                            );
                        }
                        Ok(Err(e)) => {
                            bg_state.record_event(
                                "install_wordpress_error",
                                "site",
                                &bg_domain,
                                "WordPress Install Failed",
                                &e,
                                "critical",
                            );
                            bg_state.push_install_progress(
                                &prov_key_bg,
                                "cms",
                                "WordPress setup failed",
                                "error",
                                None,
                                Some(&e),
                                false,
                            );
                            bg_state.push_install_progress(
                                &prov_key_bg,
                                "complete",
                                &format!("Site created, but WordPress install failed: {}", e),
                                "error",
                                None,
                                Some(&e),
                                true,
                            );
                        }
                        Err(e) => {
                            let msg = e.to_string();
                            bg_state.push_install_progress(
                                &prov_key_bg,
                                "complete",
                                &format!("WordPress worker panicked: {}", msg),
                                "error",
                                None,
                                Some(&msg),
                                true,
                            );
                        }
                    }
                });

                let fresh = state.get_site(&site.id).unwrap_or(site);
                return Json(fresh).into_response();
            }

            if stream && connect_existing && !wants_wp {
                let insp = wordpress::inspect_site_folder(&site_dir);
                if insp.is_wordpress {
                    let _ = wordpress::sanitize_plesk_maintenance_dropin(&site_dir, &site.domain);
                }
                state.append_install_progress(
                    &prov_key,
                    "folder",
                    &format!("Connected existing {} folder", insp.detected_type.to_uppercase()),
                    "done",
                    Some(100),
                    Some(&format!(
                        "Path: {} ({} items, type: {}{})",
                        site_dir.display(),
                        insp.file_count,
                        insp.detected_type,
                        insp.wp_version
                            .as_deref()
                            .map(|v| format!(", WordPress v{}", v))
                            .unwrap_or_default()
                    )),
                    false,
                );
                let ssl_ok = state.ensure_site_ssl(&site.domain).is_ok();
                state.append_install_progress(
                    &prov_key,
                    "hosts",
                    &format!("Registered {} in Windows hosts & local SSL", site.domain),
                    "done",
                    Some(100),
                    Some(&format!(
                        "Hosts entry 127.0.0.1 {} synced | Local SSL {}",
                        site.domain,
                        if ssl_ok { "ready" } else { "skipped" }
                    )),
                    false,
                );
                let started_site = state
                    .start_site(&site.id)
                    .unwrap_or_else(|_| state.get_site(&site.id).unwrap_or(site.clone()));
                let active_port = state.get_router_config().active_port;
                let ssl_port = state.get_ssl_port();
                let url = if started_site.ssl_enabled {
                    match ssl_port {
                        Some(443) => format!("https://{}", started_site.domain),
                        Some(p) => format!("https://{}:{}", started_site.domain, p),
                        None if active_port == 80 => format!("http://{}", started_site.domain),
                        None => format!("http://{}:{}", started_site.domain, active_port),
                    }
                } else if active_port == 80 {
                    format!("http://{}", started_site.domain)
                } else {
                    format!("http://{}:{}", started_site.domain, active_port)
                };
                let runtime_desc = match started_site.runtime.as_str() {
                    "node" => format!(
                        "Node.js ({})",
                        started_site.app_command.as_deref().unwrap_or("npm start")
                    ),
                    "static" => "Static HTTP".to_string(),
                    "proxy" => format!("Proxy (:{})", started_site.proxy_port.unwrap_or(0)),
                    _ => format!("PHP {}", started_site.php_version),
                };
                state.append_install_progress(
                    &prov_key,
                    "runtime",
                    &format!("Started {} worker & router", runtime_desc),
                    "done",
                    Some(100),
                    Some(&format!(
                        "Runtime: {} | Worker port: {}",
                        runtime_desc,
                        state
                            .get_site_worker_port(&started_site.domain)
                            .map(|p| p.to_string())
                            .unwrap_or_else(|| "router".into())
                    )),
                    false,
                );
                state.record_event(
                    "create_site",
                    "site",
                    &started_site.domain,
                    "Existing Site Connected",
                    &format!("Connected existing site {} ({})", url, runtime_desc),
                    "success",
                );
                state.append_install_progress(
                    &prov_key,
                    "complete",
                    &format!("{} is live at {}", started_site.domain, url),
                    "done",
                    Some(100),
                    Some(&format!("Connected existing website without overwriting files. Live URL: {}", url)),
                    true,
                );
                return Json(started_site).into_response();
            }

            let mut wp_failed = false;
            if wants_wp {
                let mut sync_req = wp_req;
                if requested_db_engine == "mariadb" && state.get_mariadb_port().is_none() {
                    let _ = state.start_mariadb();
                    sync_req.db_port = state.get_mariadb_port();
                }
                if let Err(e) = wordpress::install_site_wordpress(&site_dir, &site.domain, &sync_req) {
                    wp_failed = true;
                    state.record_event(
                        "install_wordpress_error",
                        "site",
                        domain,
                        "WordPress Install Failed",
                        &e,
                        "critical",
                    );
                }
            }
            if !wp_failed {
                let runtime_desc = match site.runtime.as_str() {
                    "node" => format!("Node.js ({})", site.app_command.as_deref().unwrap_or("npm start")),
                    "static" => "Static".to_string(),
                    "proxy" => format!("Proxy (:{} )", site.proxy_port.unwrap_or(0)),
                    _ => format!("PHP {}", site.php_version),
                };
                state.record_event(
                    "create_site",
                    "site",
                    domain,
                    "Site Created",
                    &format!("Created local site http://{} ({})", domain, runtime_desc),
                    "success",
                );
            }
            let site = state.start_site(&site.id).unwrap_or_else(|_| state.get_site(&site.id).unwrap_or(site));
            Json(site).into_response()
        }
        Err(err) => (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": err })),
        )
            .into_response(),
    }
}

async fn delete_site_handler(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Query(params): Query<std::collections::HashMap<String, String>>,
) -> Json<serde_json::Value> {
    let keep_files = params
        .get("keep_files")
        .map(|v| v == "true" || v == "1")
        .unwrap_or(false);
    let domain = state.get_site(&id).map(|s| s.domain).unwrap_or_else(|| id.clone());
    let ok = if keep_files {
        state.unregister_site_keep_files(&id)
    } else {
        state.delete_site(&id)
    };
    if ok {
        state.record_event(
            if keep_files { "unregister_site" } else { "delete_site" },
            "site",
            &domain,
            if keep_files { "Site Unregistered" } else { "Site Deleted" },
            &format!(
                "{} local site {}",
                if keep_files { "Unregistered" } else { "Deleted" },
                domain
            ),
            "info",
        );
    }
    Json(serde_json::json!({ "success": ok, "kept_files": keep_files }))
}

async fn start_site_handler(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Response {
    match state.start_site(&id) {
        Ok(site) => {
            state.record_event(
                "start_site",
                "site",
                &site.domain,
                "Site Started",
                &format!("Started PHP worker for http://{}", site.domain),
                "success",
            );
            Json(site).into_response()
        }
        Err(err) => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": err })),
        )
            .into_response(),
    }
}

async fn stop_site_handler(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Response {
    match state.stop_site(&id) {
        Ok(site) => {
            state.record_event(
                "stop_site",
                "site",
                &site.domain,
                "Site Stopped",
                &format!("Stopped PHP worker for {}", site.domain),
                "info",
            );
            Json(site).into_response()
        }
        Err(err) => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": err })),
        )
            .into_response(),
    }
}

async fn get_wordpress_info_handler(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Response {
    if let Some(site) = state.get_site(&id) {
        let site_dir = state.site_dir(&site);
        let dom = site.domain.clone();
        let php = site.php_version.clone();
        let info = tokio::task::spawn_blocking(move || {
            wordpress::get_site_wp_info(&site_dir, &dom, &php)
        })
        .await
        .unwrap_or_else(|_| serde_json::json!({"installed": false}));
        Json(info).into_response()
    } else {
        (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": "Site not found"})),
        )
            .into_response()
    }
}

async fn install_wordpress_handler(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(raw): Json<serde_json::Value>,
) -> Response {
    let stream = raw.get("stream").and_then(|v| v.as_bool()).unwrap_or(false);
    let db_engine = raw
        .get("db_engine")
        .and_then(|v| v.as_str())
        .unwrap_or("sqlite")
        .to_string();
    let payload = wordpress::InstallWpRequest {
        admin_user: raw
            .get("admin_user")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string()),
        admin_pass: raw
            .get("admin_pass")
            .or_else(|| raw.get("admin_password"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string()),
        admin_email: raw
            .get("admin_email")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string()),
        title: raw
            .get("title")
            .or_else(|| raw.get("site_title"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string()),
        url: raw
            .get("url")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string()),
        db_engine: Some(db_engine.clone()),
        db_port: state.get_mariadb_port(),
    };

    if let Some(site) = state.get_site(&id) {
        let site_dir = state.site_dir(&site);
        if stream {
            let install_id = uuid::Uuid::new_v4().to_string();
            let domain = site.domain.clone();
            let site_id = site.id.clone();
            state.push_install_progress(
                &install_id,
                "prepare",
                &format!("Preparing WordPress installation for {}...", domain),
                "in_progress",
                None,
                None,
                false,
            );

            let state_bg = state.clone();
            let iid_bg = install_id.clone();
            let mut bg_payload = payload;
            tokio::spawn(async move {
                state_bg.push_install_progress(
                    &iid_bg,
                    "prepare",
                    &format!("Target directory ready: {}", site_dir.display()),
                    "done",
                    None,
                    None,
                    false,
                );
                let use_mariadb = bg_payload.db_engine.as_deref() == Some("mariadb");
                if use_mariadb {
                    if state_bg.get_mariadb_port().is_none() {
                        if let Err(e) = mariadb::install_mariadb_with_progress(|step, lbl, pct, msg| {
                            state_bg.push_install_progress(
                                &iid_bg,
                                step,
                                lbl,
                                "in_progress",
                                pct,
                                msg.as_deref(),
                                false,
                            );
                        })
                        .await
                        {
                            state_bg.push_install_progress(
                                &iid_bg,
                                "complete",
                                &format!("MariaDB install failed: {}", e),
                                "error",
                                None,
                                Some(&e),
                                true,
                            );
                            return;
                        }
                        match tokio::task::spawn_blocking({
                            let st = state_bg.clone();
                            move || st.start_mariadb()
                        })
                        .await
                        {
                            Ok(Ok(p)) => bg_payload.db_port = Some(p),
                            Ok(Err(e)) => {
                                state_bg.push_install_progress(
                                    &iid_bg,
                                    "complete",
                                    &format!("Could not start MariaDB: {}", e),
                                    "error",
                                    None,
                                    Some(&e),
                                    true,
                                );
                                return;
                            }
                            Err(e) => {
                                state_bg.push_install_progress(
                                    &iid_bg,
                                    "complete",
                                    &format!("MariaDB start error: {}", e),
                                    "error",
                                    None,
                                    None,
                                    true,
                                );
                                return;
                            }
                        }
                    } else {
                        bg_payload.db_port = state_bg.get_mariadb_port();
                    }
                }

                if let Err(e) = wordpress::ensure_wp_archives_with_progress(!use_mariadb, |step, lbl, pct, msg| {
                    state_bg.push_install_progress(
                        &iid_bg,
                        step,
                        lbl,
                        "in_progress",
                        pct,
                        msg.as_deref(),
                        false,
                    );
                })
                .await
                {
                    state_bg.push_install_progress(
                        &iid_bg,
                        "complete",
                        &format!("WordPress download failed: {}", e),
                        "error",
                        None,
                        Some(&e),
                        true,
                    );
                    return;
                }

                state_bg.push_install_progress(
                    &iid_bg,
                    "cms",
                    "WordPress Core archive ready (cached locally)",
                    "done",
                    None,
                    None,
                    false,
                );

                let sd = site_dir.clone();
                let dm = domain.clone();
                let st_cb = state_bg.clone();
                let iid_cb = iid_bg.clone();
                let res = tokio::task::spawn_blocking(move || {
                    wordpress::install_site_wordpress_with_progress(&sd, &dm, &bg_payload, |step, lbl, status, pct| {
                        st_cb.push_install_progress(&iid_cb, step, lbl, status, pct, None, false);
                    })
                })
                .await;

                match res {
                    Ok(Ok(_)) => {
                        state_bg.update_site_preset(&site_id, "wordpress");
                        let _ = state_bg.ensure_site_ssl(&domain);
                        let _ = state_bg.start_site(&site_id);
                        state_bg.record_event(
                            "install_wordpress",
                            "site",
                            &domain,
                            "WordPress Installed",
                            &format!("WordPress installed and running on http://{}", domain),
                            "success",
                        );
                        state_bg.push_install_progress(
                            &iid_bg,
                            "complete",
                            &format!("WordPress is live at http://{}", domain),
                            "done",
                            None,
                            None,
                            true,
                        );
                    }
                    Ok(Err(e)) => {
                        state_bg.record_event(
                            "install_wordpress_error",
                            "site",
                            &domain,
                            "WordPress Install Failed",
                            &e,
                            "critical",
                        );
                        state_bg.push_install_progress(
                            &iid_bg,
                            "complete",
                            &format!("WordPress install failed: {}", e),
                            "error",
                            None,
                            Some(&e),
                            true,
                        );
                    }
                    Err(e) => {
                        state_bg.push_install_progress(
                            &iid_bg,
                            "complete",
                            &format!("WordPress setup failed: {}", e),
                            "error",
                            None,
                            None,
                            true,
                        );
                    }
                }
            });

            return (
                StatusCode::ACCEPTED,
                Json(serde_json::json!({
                    "ok": true,
                    "success": true,
                    "install_id": install_id
                })),
            )
                .into_response();
        }

        let mut sync_payload = payload;
        if db_engine == "mariadb" && state.get_mariadb_port().is_none() {
            let _ = state.start_mariadb();
            sync_payload.db_port = state.get_mariadb_port();
        }
        match wordpress::install_site_wordpress(&site_dir, &site.domain, &sync_payload) {
            Ok(res) => {
                state.update_site_preset(&site.id, "wordpress");
                let _ = state.start_site(&site.id);
                state.record_event(
                    "install_wordpress",
                    "site",
                    &site.domain,
                    "WordPress Installed",
                    &format!("WordPress installed on http://{}", site.domain),
                    "success",
                );
                (StatusCode::CREATED, Json(res)).into_response()
            }
            Err(e) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error": e})),
            )
                .into_response(),
        }
    } else {
        (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": "Site not found"})),
        )
            .into_response()
    }
}

async fn reinstall_wordpress_core_handler(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    payload: Option<Json<serde_json::Value>>,
) -> Response {
    if let Some(site) = state.get_site(&id) {
        let site_dir = state.site_dir(&site);
        let stream = payload
            .and_then(|Json(v)| v.get("stream").and_then(|b| b.as_bool()))
            .unwrap_or(false);

        if stream {
            let install_id = format!("wp-reinstall-{}", uuid::Uuid::new_v4());
            let iid_bg = install_id.clone();
            let state_bg = state.clone();
            let domain = site.domain.clone();
            state.push_install_progress(
                &install_id,
                "prepare",
                &format!("Checking local WordPress Core cache for {}", site.domain),
                "done",
                None,
                None,
                false,
            );
            tokio::spawn(async move {
                if let Err(e) = wordpress::ensure_wp_archives_with_progress(false, |step, lbl, pct, msg| {
                    state_bg.push_install_progress(
                        &iid_bg,
                        step,
                        lbl,
                        "in_progress",
                        pct,
                        msg.as_deref(),
                        false,
                    );
                })
                .await
                {
                    state_bg.push_install_progress(
                        &iid_bg,
                        "complete",
                        &format!("WordPress download failed: {}", e),
                        "error",
                        None,
                        Some(&e),
                        true,
                    );
                    return;
                }
                state_bg.push_install_progress(
                    &iid_bg,
                    "cms",
                    "WordPress Core archive ready (cached locally)",
                    "done",
                    None,
                    None,
                    false,
                );
                let sd = site_dir.clone();
                let st_cb = state_bg.clone();
                let iid_cb = iid_bg.clone();
                match tokio::task::spawn_blocking(move || {
                    wordpress::reinstall_core_with_progress(&sd, |step, lbl, status, pct| {
                        st_cb.push_install_progress(&iid_cb, step, lbl, status, pct, None, false);
                    })
                })
                .await
                {
                    Ok(Ok(val)) => {
                        let msg = val["message"]
                            .as_str()
                            .unwrap_or("WordPress core files re-installed successfully!");
                        state_bg.record_event(
                            "wp_reinstall_core",
                            "site",
                            &domain,
                            "WordPress Core Re-installed",
                            &format!("Refreshed core WordPress files for {}", domain),
                            "success",
                        );
                        state_bg.push_install_progress(
                            &iid_bg,
                            "complete",
                            msg,
                            "done",
                            None,
                            None,
                            true,
                        );
                    }
                    Ok(Err(e)) => {
                        state_bg.push_install_progress(
                            &iid_bg,
                            "complete",
                            &format!("Core reinstall failed: {}", e),
                            "error",
                            None,
                            Some(&e),
                            true,
                        );
                    }
                    Err(e) => {
                        let msg = e.to_string();
                        state_bg.push_install_progress(
                            &iid_bg,
                            "complete",
                            &format!("Core reinstall failed: {}", msg),
                            "error",
                            None,
                            Some(&msg),
                            true,
                        );
                    }
                }
            });
            return (
                StatusCode::ACCEPTED,
                Json(serde_json::json!({ "ok": true, "install_id": install_id })),
            )
                .into_response();
        }

        let res = tokio::task::spawn_blocking(move || wordpress::reinstall_core(&site_dir)).await;
        match res {
            Ok(Ok(val)) => {
                state.record_event(
                    "wp_reinstall_core",
                    "site",
                    &site.domain,
                    "WordPress Core Re-installed",
                    &format!("Refreshed core WordPress files for {}", site.domain),
                    "success",
                );
                Json(val).into_response()
            }
            Ok(Err(e)) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error": e})),
            )
                .into_response(),
            Err(e) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error": e.to_string()})),
            )
                .into_response(),
        }
    } else {
        (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": "Site not found"})),
        )
            .into_response()
    }
}

async fn wordpress_plugins_handler(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Response {
    if let Some(site) = state.get_site(&id) {
        let site_dir = state.site_dir(&site);
        let dom = site.domain.clone();
        let plugins = tokio::task::spawn_blocking(move || wordpress::list_plugins(&site_dir, &dom))
            .await
            .unwrap_or_default();
        Json(serde_json::json!(plugins)).into_response()
    } else {
        (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": "Site not found"})),
        )
            .into_response()
    }
}

async fn wordpress_themes_handler(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Response {
    if let Some(site) = state.get_site(&id) {
        let site_dir = state.site_dir(&site);
        let dom = site.domain.clone();
        let themes = tokio::task::spawn_blocking(move || wordpress::list_themes(&site_dir, &dom))
            .await
            .unwrap_or_default();
        Json(serde_json::json!(themes)).into_response()
    } else {
        (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": "Site not found"})),
        )
            .into_response()
    }
}

async fn wordpress_db_info_handler(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Response {
    if let Some(site) = state.get_site(&id) {
        let site_dir = state.site_dir(&site);
        Json(wordpress::get_wp_db_info(
            &site_dir,
            &site.id,
            &site.domain,
            state.get_mariadb_port(),
        ))
        .into_response()
    } else {
        (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": "Site not found"})),
        )
            .into_response()
    }
}

async fn wordpress_features_status_handler(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Response {
    if let Some(site) = state.get_site(&id) {
        let site_dir = state.site_dir(&site);
        let dom = site.domain.clone();
        let pass_protect = state.get_site_pass_protect(&site.domain).is_some();
        let res = tokio::task::spawn_blocking(move || {
            wordpress::get_wp_features_status(&site_dir, &dom, pass_protect)
        })
        .await
        .unwrap_or_else(|_| serde_json::json!({}));
        Json(res).into_response()
    } else {
        (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": "Site not found"})),
        )
            .into_response()
    }
}

async fn php_versions_handler(State(state): State<Arc<AppState>>) -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "versions": state.list_php_versions()
    }))
}

async fn switch_site_php_handler(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(payload): Json<serde_json::Value>,
) -> Response {
    let version = payload
        .get("version")
        .or_else(|| payload.get("php_version"))
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|v| !v.is_empty());

    let Some(ver) = version else {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "PHP version is required"})),
        )
            .into_response();
    };

    if state::find_php_binary_exact(ver).is_none() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": format!("PHP {} is not installed. Click Install PHP {} first.", ver, ver)})),
        )
            .into_response();
    }

    match state.update_site_php_version(&id, ver) {
        Ok(site) => {
            state.record_event(
                "switch_php",
                "site",
                &site.domain,
                "PHP Version Updated",
                &format!("Switched {} to PHP {}", site.domain, ver),
                "info",
            );
            Json(site).into_response()
        }
        Err(err) => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": err})),
        )
            .into_response(),
    }
}

async fn site_app_status_handler(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Response {
    let Some(site) = state.get_site(&id) else {
        return (StatusCode::NOT_FOUND, Json(serde_json::json!({"error": "Site not found"}))).into_response();
    };
    let ports = state.get_site_worker_ports(&site.domain);
    let port = ports.first().copied();
    let alive = {
        let mut procs = state.worker_processes.lock().unwrap();
        if let Some(children) = procs.get_mut(&site.domain.to_lowercase()) {
            children
                .iter_mut()
                .any(|child| matches!(child.try_wait(), Ok(None)))
        } else {
            false
        }
    };
    let active = if site.runtime == "proxy" {
        port.is_some()
    } else {
        alive && port.is_some()
    };
    Json(serde_json::json!({
        "active": active,
        "status": if active { "running" } else { "stopped" },
        "port": port,
        "worker_ports": ports,
        "worker_count": ports.len(),
        "runtime": site.runtime,
        "app_command": site.app_command,
    }))
    .into_response()
}

async fn site_app_restart_handler(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Response {
    let Some(site) = state.get_site(&id) else {
        return (StatusCode::NOT_FOUND, Json(serde_json::json!({"error": "Site not found"}))).into_response();
    };
    let _ = state.stop_site(&site.id);
    match state.start_site(&site.id) {
        Ok(updated) => Json(serde_json::json!({
            "ok": true,
            "status": "running",
            "port": state.get_site_worker_port(&updated.domain),
        }))
        .into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({"error": e}))).into_response(),
    }
}

async fn site_app_logs_handler(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Response {
    let Some(site) = state.get_site(&id) else {
        return (StatusCode::NOT_FOUND, Json(serde_json::json!({"error": "Site not found"}))).into_response();
    };
    let log_path = state.app_log_path(&site.domain);
    let content = std::fs::read_to_string(&log_path).unwrap_or_default();
    let lines: Vec<&str> = content.lines().rev().take(200).collect();
    let tail: String = lines.into_iter().rev().collect::<Vec<_>>().join("\n");
    Json(serde_json::json!({ "logs": tail })).into_response()
}

async fn switch_site_runtime_handler(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(payload): Json<serde_json::Value>,
) -> Response {
    let runtime = payload
        .get("runtime")
        .and_then(|v| v.as_str())
        .unwrap_or("php");
    let php_version = payload.get("php_version").and_then(|v| v.as_str());

    if runtime == "php" {
        if let Some(ver) = php_version {
            if state::find_php_binary_exact(ver).is_none() {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(serde_json::json!({"error": format!("PHP {} is not installed. Click Install PHP {} first.", ver, ver)})),
                )
                    .into_response();
            }
        }
    }

    match state.update_site_runtime(&id, runtime, php_version) {
        Ok(site) => {
            state.record_event(
                "switch_runtime",
                "site",
                &site.domain,
                "Runtime Updated",
                &format!("Updated {} runtime to {}", site.domain, runtime),
                "info",
            );
            Json(site).into_response()
        }
        Err(err) => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": err})),
        )
            .into_response(),
    }
}

async fn php_install_handler(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<serde_json::Value>,
) -> Response {
    let version = payload
        .get("version")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .unwrap_or("")
        .to_string();

    const ALLOWED: &[&str] = &["8.6", "8.5", "8.4", "8.3", "8.2", "8.1", "8.0", "7.4"];
    if !ALLOWED.contains(&version.as_str()) {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": format!("Unsupported PHP version: {}", version)})),
        )
            .into_response();
    }

    let install_id = uuid::Uuid::new_v4().to_string();
    state.push_php_install_step(
        &install_id,
        serde_json::json!({
            "step": "resolve",
            "label": format!("Preparing PHP {} installation for Windows...", version),
            "status": "in_progress"
        }),
        false,
    );

    let bg_state = state.clone();
    let bg_id = install_id.clone();
    let bg_ver = version.clone();

    tokio::spawn(async move {
        if let Some(existing) = state::find_php_binary_exact(&bg_ver) {
            bg_state.push_php_install_step(
                &bg_id,
                serde_json::json!({
                    "step": "resolve",
                    "label": format!("Found existing PHP {} binary", bg_ver),
                    "status": "done",
                    "message": existing.to_string_lossy().to_string()
                }),
                false,
            );
            bg_state.push_php_install_step(
                &bg_id,
                serde_json::json!({
                    "step": "complete",
                    "label": format!("PHP {} is ready", bg_ver),
                    "status": "done"
                }),
                true,
            );
            return;
        }

        let Some(base_dir) = state::dockpanel_php_dir() else {
            bg_state.push_php_install_step(
                &bg_id,
                serde_json::json!({
                    "step": "complete",
                    "label": "Could not determine USERPROFILE directory",
                    "status": "error"
                }),
                true,
            );
            return;
        };

        let target_dir = base_dir.join(format!("php{}", bg_ver));
        bg_state.push_php_install_step(
            &bg_id,
            serde_json::json!({
                "step": "resolve",
                "label": format!("Querying windows.php.net for PHP {} release...", bg_ver),
                "status": "done"
            }),
            false,
        );
        bg_state.push_php_install_step(
            &bg_id,
            serde_json::json!({
                "step": "download",
                "label": format!("Downloading & extracting PHP {} x64 package...", bg_ver),
                "status": "in_progress"
            }),
            false,
        );

        let target_str = target_dir.to_string_lossy().replace('\'', "''");
        let ver_escaped = bg_ver.replace('\'', "''");
        let ps_script = format!(
            r#"
$ErrorActionPreference = 'Stop'
[Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12
$ver = '{ver}'
$dest = '{dest}'
$zipPath = Join-Path $env:TEMP "dockpanel_php_$ver.zip"

$downloadUrl = $null
try {{
    $releases = Invoke-RestMethod -Uri 'https://windows.php.net/downloads/releases/releases.json' -TimeoutSec 20
    $entry = $releases.$ver
    if ($entry) {{
        $buildProp = $entry.PSObject.Properties | Where-Object {{ $_.Name -match '^(nts|ts)-vs\d+-x64$' }} | Sort-Object Name | Select-Object -First 1
        if ($buildProp -and $buildProp.Value.zip.path) {{
            $downloadUrl = 'https://windows.php.net/downloads/releases/' + $buildProp.Value.zip.path
        }}
    }}
}} catch {{}}

if (-not $downloadUrl) {{
    $archHtml = (Invoke-WebRequest -Uri 'https://windows.php.net/downloads/releases/archives/' -TimeoutSec 20 -UseBasicParsing).Content
    $escapedVer = [regex]::Escape($ver)
    $matches = [regex]::Matches($archHtml, "php-$escapedVer\.\d+(?:-nts)?-Win32-v[sc]\d+-x64\.zip") | ForEach-Object {{ $_.Value }} | Sort-Object -Descending -Unique
    $picked = $matches | Select-Object -First 1
    if ($picked) {{
        $downloadUrl = 'https://windows.php.net/downloads/releases/archives/' + $picked
    }}
}}

if (-not $downloadUrl) {{
    throw "Could not find an official Windows x64 release package for PHP $ver on windows.php.net."
}}

Invoke-WebRequest -Uri $downloadUrl -OutFile $zipPath -TimeoutSec 180 -UseBasicParsing
New-Item -ItemType Directory -Force -Path $dest | Out-Null
Expand-Archive -Path $zipPath -DestinationPath $dest -Force
Remove-Item -Force $zipPath -ErrorAction SilentlyContinue

$devIni = Join-Path $dest 'php.ini-development'
$targetIni = Join-Path $dest 'php.ini'
if ((Test-Path $devIni) -and -not (Test-Path $targetIni)) {{
    $ini = Get-Content -Raw $devIni
    $ini = $ini -replace ';extension_dir = "ext"', 'extension_dir = "ext"'
    foreach ($ext in @('curl','fileinfo','gd','mbstring','mysqli','openssl','pdo_mysql','pdo_sqlite','sqlite3','zip')) {{
        $ini = $ini -replace ";extension=$ext", "extension=$ext"
    }}
    Set-Content -Path $targetIni -Value $ini -Encoding ASCII
}}

$phpExe = Join-Path $dest 'php.exe'
if (-not (Test-Path $phpExe)) {{
    throw "php.exe was not found after extraction in $dest"
}}
Write-Output $phpExe
"#,
            ver = ver_escaped,
            dest = target_str
        );

        let result = tokio::task::spawn_blocking(move || {
            #[cfg(windows)]
            use std::os::windows::process::CommandExt;
            #[cfg(windows)]
            const CREATE_NO_WINDOW: u32 = 0x08000000;

            let mut cmd = std::process::Command::new("powershell.exe");
            cmd.args(["-NoProfile", "-ExecutionPolicy", "Bypass", "-Command", &ps_script]);
            #[cfg(windows)]
            cmd.creation_flags(CREATE_NO_WINDOW);
            cmd.output()
        })
        .await;

        match result {
            Ok(Ok(out)) if out.status.success() => {
                let exe_out = String::from_utf8_lossy(&out.stdout).trim().to_string();
                bg_state.record_event(
                    "install_php",
                    "runtime",
                    &format!("php{}", bg_ver),
                    "PHP Runtime Installed",
                    &format!("Installed PHP {} ({})", bg_ver, exe_out),
                    "success",
                );
                bg_state.push_php_install_step(
                    &bg_id,
                    serde_json::json!({
                        "step": "download",
                        "label": format!("PHP {} installed to {}", bg_ver, exe_out),
                        "status": "done"
                    }),
                    false,
                );
                bg_state.push_php_install_step(
                    &bg_id,
                    serde_json::json!({
                        "step": "complete",
                        "label": format!("PHP {} installed and ready for local sites!", bg_ver),
                        "status": "done"
                    }),
                    true,
                );
            }
            Ok(Ok(out)) => {
                let stderr = String::from_utf8_lossy(&out.stderr).trim().to_string();
                let stdout = String::from_utf8_lossy(&out.stdout).trim().to_string();
                let msg = if !stderr.is_empty() { stderr } else { stdout };
                bg_state.record_event(
                    "install_php_error",
                    "runtime",
                    &format!("php{}", bg_ver),
                    "PHP Install Failed",
                    &msg,
                    "critical",
                );
                bg_state.push_php_install_step(
                    &bg_id,
                    serde_json::json!({
                        "step": "download",
                        "label": format!("Failed to install PHP {}", bg_ver),
                        "status": "error",
                        "message": msg
                    }),
                    false,
                );
                bg_state.push_php_install_step(
                    &bg_id,
                    serde_json::json!({
                        "step": "complete",
                        "label": format!("PHP {} installation failed", bg_ver),
                        "status": "error",
                        "message": msg
                    }),
                    true,
                );
            }
            _ => {
                bg_state.push_php_install_step(
                    &bg_id,
                    serde_json::json!({
                        "step": "complete",
                        "label": format!("Failed to spawn PHP {} installer process", bg_ver),
                        "status": "error"
                    }),
                    true,
                );
            }
        }
    });

    Json(serde_json::json!({ "install_id": install_id })).into_response()
}

async fn php_install_log_handler(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let (tx, rx) = tokio::sync::mpsc::channel::<Result<SseEvent, std::convert::Infallible>>(64);

    tokio::spawn(async move {
        let mut last_sent: Vec<String> = Vec::new();
        for _ in 0..12000 {
            let (snapshot, done) = {
                if let Ok(jobs) = state.php_installs.lock() {
                    if let Some(job) = jobs.get(&id) {
                        (job.steps.clone(), job.done)
                    } else {
                        (
                            vec![serde_json::json!({
                                "step": "complete",
                                "label": "Install job not found",
                                "status": "error"
                            })],
                            true,
                        )
                    }
                } else {
                    (Vec::new(), true)
                }
            };

            for (i, step) in snapshot.iter().enumerate() {
                if let Ok(data) = serde_json::to_string(step) {
                    let changed = last_sent.get(i).map(|prev| prev != &data).unwrap_or(true);
                    if changed {
                        if i < last_sent.len() {
                            last_sent[i] = data.clone();
                        } else {
                            last_sent.push(data.clone());
                        }
                        if tx.send(Ok(SseEvent::default().data(data))).await.is_err() {
                            return;
                        }
                    }
                }
            }

            if done {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(150)).await;
        }
    });

    Sse::new(tokio_stream::wrappers::ReceiverStream::new(rx))
        .keep_alive(KeepAlive::default())
}

fn php_ext_directive(ext: &str) -> &'static str {
    if ext.eq_ignore_ascii_case("opcache") || ext.eq_ignore_ascii_case("xdebug") {
        "zend_extension"
    } else {
        "extension"
    }
}

fn scan_php_dll_extensions(php_exe: &std::path::Path) -> Vec<String> {
    let mut exts = Vec::new();
    if let Some(ext_dir) = php_exe.parent().map(|d| d.join("ext")) {
        if let Ok(entries) = std::fs::read_dir(ext_dir) {
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().to_string();
                let lower = name.to_ascii_lowercase();
                if lower.starts_with("php_") && lower.ends_with(".dll") && lower.len() > 8 {
                    let ext_name = lower[4..lower.len() - 4].to_string();
                    if !ext_name.is_empty() {
                        exts.push(ext_name);
                    }
                }
            }
        }
    }
    exts.sort();
    exts.dedup();
    exts
}

fn toggle_php_extension_in_ini_text(ini: &str, ext: &str, enable: bool) -> String {
    let ext_lower = ext.trim().to_ascii_lowercase();
    let directive = php_ext_directive(&ext_lower);
    let dll_name = format!("php_{}.dll", ext_lower);
    let mut lines: Vec<String> = Vec::new();
    let mut matched = false;
    let mut has_ext_dir = false;

    for raw_line in ini.lines() {
        let trimmed = raw_line.trim();
        let uncommented = trimmed.trim_start_matches(';').trim();

        if enable && uncommented.eq_ignore_ascii_case("extension_dir = \"ext\"") {
            lines.push("extension_dir = \"ext\"".to_string());
            has_ext_dir = true;
            continue;
        }
        if uncommented.to_ascii_lowercase().starts_with("extension_dir")
            && !trimmed.starts_with(';')
        {
            has_ext_dir = true;
        }

        let is_match = if let Some((lhs, rhs)) = uncommented.split_once('=') {
            let lhs_clean = lhs.trim().to_ascii_lowercase();
            let rhs_clean = rhs
                .split(';')
                .next()
                .unwrap_or("")
                .trim()
                .trim_matches('"')
                .trim_matches('\'')
                .to_ascii_lowercase();
            (lhs_clean == "extension" || lhs_clean == "zend_extension")
                && (rhs_clean == ext_lower || rhs_clean == dll_name)
        } else {
            false
        };

        if is_match {
            matched = true;
            if enable {
                lines.push(format!("{}={}", directive, ext_lower));
            } else {
                lines.push(format!(";{}={}", directive, ext_lower));
            }
        } else {
            lines.push(raw_line.to_string());
        }
    }

    if enable && !matched {
        if !has_ext_dir {
            lines.push("extension_dir = \"ext\"".to_string());
        }
        lines.push(format!("{}={}", directive, ext_lower));
    }

    let mut out = lines.join("\r\n");
    if ini.ends_with('\n') && !out.ends_with('\n') {
        out.push_str("\r\n");
    }
    out
}

fn set_php_extension_enabled(version: &str, ext: &str, enable: bool) -> Result<String, String> {
    let clean_ext = ext.trim().to_ascii_lowercase();
    if clean_ext.is_empty()
        || clean_ext.len() > 64
        || !clean_ext
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    {
        return Err("Invalid PHP extension name".to_string());
    }

    const CORE_BUILTINS: &[&str] = &[
        "core",
        "standard",
        "pcre",
        "spl",
        "reflection",
        "date",
        "json",
        "hash",
        "filter",
        "ctype",
        "tokenizer",
        "session",
        "random",
    ];
    if !enable && CORE_BUILTINS.contains(&clean_ext.as_str()) {
        return Err(format!("'{}' is a built-in PHP core module and cannot be disabled", clean_ext));
    }

    let exe = state::find_php_binary_exact(version)
        .ok_or_else(|| format!("PHP {} is not installed", version))?;
    let php_dir = exe
        .parent()
        .ok_or_else(|| "Invalid PHP binary directory".to_string())?;
    let ini_path = php_dir.join("php.ini");

    if !ini_path.is_file() {
        let dev_ini = php_dir.join("php.ini-development");
        let prod_ini = php_dir.join("php.ini-production");
        if dev_ini.is_file() {
            std::fs::copy(&dev_ini, &ini_path)
                .map_err(|e| format!("Failed creating php.ini: {}", e))?;
        } else if prod_ini.is_file() {
            std::fs::copy(&prod_ini, &ini_path)
                .map_err(|e| format!("Failed creating php.ini: {}", e))?;
        } else {
            std::fs::write(&ini_path, "extension_dir = \"ext\"\r\n")
                .map_err(|e| format!("Failed creating php.ini: {}", e))?;
        }
    }

    if enable {
        let dll_path = php_dir.join("ext").join(format!("php_{}.dll", clean_ext));
        if !dll_path.is_file() {
            return Err(format!(
                "Extension DLL '{}' was not found in {}",
                dll_path.display(),
                php_dir.join("ext").display()
            ));
        }
    }

    let current = std::fs::read_to_string(&ini_path)
        .map_err(|e| format!("Failed reading {}: {}", ini_path.display(), e))?;
    let updated = toggle_php_extension_in_ini_text(&current, &clean_ext, enable);
    std::fs::write(&ini_path, updated)
        .map_err(|e| format!("Failed writing {}: {}", ini_path.display(), e))?;
    Ok(ini_path.to_string_lossy().to_string())
}

async fn php_extensions_handler(Path(version): Path<String>) -> Json<serde_json::Value> {
    let mut installed = Vec::new();
    let mut dll_available = Vec::new();
    let mut ini_path_str = String::new();
    if let Some(exe) = state::find_php_binary_exact(&version) {
        if let Some(dir) = exe.parent() {
            ini_path_str = dir.join("php.ini").to_string_lossy().to_string();
        }
        dll_available = scan_php_dll_extensions(&exe);
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x08000000;
            if let Ok(out) = std::process::Command::new(&exe)
                .arg("-m")
                .creation_flags(CREATE_NO_WINDOW)
                .output()
            {
                if out.status.success() {
                    for line in String::from_utf8_lossy(&out.stdout).lines() {
                        let t = line.trim();
                        if !t.is_empty() && !t.starts_with('[') {
                            let lower = t.to_lowercase();
                            if !installed.iter().any(|x| x == &lower) {
                                installed.push(lower);
                            }
                        }
                    }
                }
            }
        }
    }
    let common = [
        "curl", "exif", "fileinfo", "gd", "intl", "mbstring", "mysqli", "opcache", "openssl",
        "pdo_mysql", "pdo_sqlite", "soap", "sockets", "sodium", "sqlite3", "xsl", "zip",
    ];
    let mut available: Vec<String> = dll_available
        .into_iter()
        .filter(|ext| !installed.iter().any(|i| i == ext))
        .collect();
    if available.is_empty() {
        available = common
            .iter()
            .filter(|ext| !installed.iter().any(|i| i == *ext))
            .map(|s| s.to_string())
            .collect();
    }
    Json(serde_json::json!({
        "installed": installed,
        "available": available,
        "ini_path": ini_path_str,
    }))
}

async fn php_extension_install_handler(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<serde_json::Value>,
) -> Response {
    let version = payload
        .get("version")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .unwrap_or("");
    let extension = payload
        .get("extension")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .unwrap_or("");
    if version.is_empty() || extension.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "Both version and extension are required"})),
        )
            .into_response();
    }
    match set_php_extension_enabled(version, extension, true) {
        Ok(ini_path) => {
            state.restart_sites_for_php_version(version);
            state.record_event(
                "php_extension_enable",
                "php",
                version,
                "PHP Extension Enabled",
                &format!("Enabled {} in PHP {} ({})", extension, version, ini_path),
                "success",
            );
            Json(serde_json::json!({"ok": true, "extension": extension, "ini_path": ini_path}))
                .into_response()
        }
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": e})),
        )
            .into_response(),
    }
}

async fn php_extension_disable_handler(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<serde_json::Value>,
) -> Response {
    let version = payload
        .get("version")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .unwrap_or("");
    let extension = payload
        .get("extension")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .unwrap_or("");
    if version.is_empty() || extension.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "Both version and extension are required"})),
        )
            .into_response();
    }
    match set_php_extension_enabled(version, extension, false) {
        Ok(ini_path) => {
            state.restart_sites_for_php_version(version);
            state.record_event(
                "php_extension_disable",
                "php",
                version,
                "PHP Extension Disabled",
                &format!("Disabled {} in PHP {} ({})", extension, version, ini_path),
                "info",
            );
            Json(serde_json::json!({"ok": true, "extension": extension, "ini_path": ini_path}))
                .into_response()
        }
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": e})),
        )
            .into_response(),
    }
}

async fn site_provision_log_handler(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let prov_key = format!("site:{}", id);
    let has_async_job = state
        .php_installs
        .lock()
        .map(|jobs| jobs.contains_key(&prov_key))
        .unwrap_or(false);

    if has_async_job {
        return php_install_log_handler(State(state), Path(prov_key))
            .await
            .into_response();
    }

    let (tx, rx) = tokio::sync::mpsc::channel::<Result<SseEvent, std::convert::Infallible>>(8);
    let site = state.get_site(&id);
    tokio::spawn(async move {
        let domain = site
            .as_ref()
            .map(|s| s.domain.as_str())
            .unwrap_or("site");
        let is_wp = site
            .as_ref()
            .and_then(|s| s.php_preset.as_deref())
            == Some("wordpress");

        let mut steps = vec![
            serde_json::json!({"step": "dir", "label": format!("Created local site root for {}", domain), "status": "done"}),
            serde_json::json!({"step": "hosts", "label": format!("Registered {} in Windows hosts", domain), "status": "done"}),
            serde_json::json!({"step": "php", "label": "Started local PHP-CGI worker & HTTP router", "status": "done"}),
        ];
        if is_wp {
            steps.push(serde_json::json!({"step": "cms", "label": "Configured WordPress starter environment", "status": "done"}));
        }
        steps.push(serde_json::json!({"step": "complete", "label": format!("{} is live on DockPanel Desktop", domain), "status": "done"}));

        for step in steps {
            if let Ok(data) = serde_json::to_string(&step) {
                if tx.send(Ok(SseEvent::default().data(data))).await.is_err() {
                    return;
                }
            }
        }
    });
    Sse::new(tokio_stream::wrappers::ReceiverStream::new(rx))
        .keep_alive(KeepAlive::default())
        .into_response()
}

async fn metrics_overview_handler(State(state): State<Arc<AppState>>) -> Json<serde_json::Value> {
    let mut map = serde_json::Map::new();
    for site in state.list_sites() {
        let site_dir = state.site_dir(&site);
        let disk = state::AppState::dir_size_bytes(&site_dir);
        let has_db = site_dir.join("wp-content").join("database").join(".ht.sqlite").is_file()
            || wordpress::read_wp_mysql_db_name(&site_dir).is_some();
        map.insert(
            site.id,
            serde_json::json!({
                "disk_usage_bytes": disk,
                "traffic_bytes_this_month": 0,
                "server_location": site.document_root,
                "has_database": has_db,
                "has_mail": state.mail_catcher.is_running(),
            }),
        );
    }
    Json(serde_json::Value::Object(map))
}

async fn site_health_handler(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Response {
    let Some(site) = state.get_site(&id) else {
        return (StatusCode::NOT_FOUND, Json(serde_json::json!({"error": "Site not found"}))).into_response();
    };
    let port = state.get_router_config().active_port;
    let start = std::time::Instant::now();
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(3))
        .build()
        .unwrap_or_default();
    let res = client
        .get(format!("http://127.0.0.1:{}/", port))
        .header("Host", &site.domain)
        .send()
        .await;
    let ms = start.elapsed().as_millis() as u64;
    match res {
        Ok(r) => Json(serde_json::json!({
            "healthy": r.status().is_success() || r.status().as_u16() < 500,
            "status": r.status().as_u16(),
            "response_time_ms": ms
        }))
        .into_response(),
        Err(_) => Json(serde_json::json!({
            "healthy": false,
            "status": 0,
            "response_time_ms": ms
        }))
        .into_response(),
    }
}

async fn site_health_summary_handler(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Response {
    if let Some(site) = state.get_site(&id) {
        let up = site.status == "active";
        Json(serde_json::json!({
            "ssl_status": { "enabled": site.ssl_enabled, "days_until_expiry": 365 },
            "backup_freshness": { "last_backup": null, "hours_since": null },
            "uptime": { "status": if up { "up" } else { "down" }, "response_time_ms": 5, "monitor_enabled": true },
            "score": if up { 100 } else { 50 }
        }))
        .into_response()
    } else {
        (StatusCode::NOT_FOUND, Json(serde_json::json!({"error": "Site not found"}))).into_response()
    }
}

async fn toggle_site_handler(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(payload): Json<serde_json::Value>,
) -> Response {
    let enabled = payload.get("enabled").and_then(|v| v.as_bool()).unwrap_or(true);
    match state.toggle_site_enabled(&id, enabled) {
        Ok(site) => {
            state.record_event(
                "toggle_site",
                "site",
                &site.domain,
                if enabled { "Site Enabled" } else { "Site Disabled" },
                &format!("{} {}", if enabled { "Enabled" } else { "Disabled" }, site.domain),
                "info",
            );
            Json(site).into_response()
        }
        Err(e) => (StatusCode::BAD_REQUEST, Json(serde_json::json!({"error": e}))).into_response(),
    }
}

async fn rename_site_handler(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(payload): Json<serde_json::Value>,
) -> Response {
    let new_domain = payload.get("new_domain").and_then(|v| v.as_str()).unwrap_or("");
    match state.rename_site_domain(&id, new_domain) {
        Ok(site) => {
            state.record_event(
                "rename_site",
                "site",
                &site.domain,
                "Domain Renamed",
                &format!("Renamed site to {}", site.domain),
                "info",
            );
            Json(site).into_response()
        }
        Err(e) => (StatusCode::BAD_REQUEST, Json(serde_json::json!({"error": e}))).into_response(),
    }
}

async fn clone_site_handler(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(payload): Json<serde_json::Value>,
) -> Response {
    let new_domain = payload.get("domain").and_then(|v| v.as_str()).unwrap_or("");
    match state.clone_site(&id, new_domain) {
        Ok(site) => {
            state.record_event(
                "clone_site",
                "site",
                &site.domain,
                "Site Cloned",
                &format!("Cloned site to {}", site.domain),
                "success",
            );
            Json(site).into_response()
        }
        Err(e) => (StatusCode::BAD_REQUEST, Json(serde_json::json!({"error": e}))).into_response(),
    }
}

async fn update_site_limits_handler(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(payload): Json<serde_json::Value>,
) -> Response {
    let max_upload = payload.get("max_upload_mb").and_then(|v| v.as_i64()).map(|n| n as i32);
    let php_memory = payload.get("php_memory_mb").and_then(|v| v.as_i64()).map(|n| n as i32);
    let php_workers = payload.get("php_max_workers").and_then(|v| v.as_i64()).map(|n| n as i32);
    match state.update_site_limits(&id, max_upload, php_memory, php_workers) {
        Ok(site) => Json(site).into_response(),
        Err(e) => (StatusCode::NOT_FOUND, Json(serde_json::json!({"error": e}))).into_response(),
    }
}

async fn site_env_get_handler(State(state): State<Arc<AppState>>, Path(id): Path<String>) -> Response {
    let Some(site) = state.get_site(&id) else {
        return (StatusCode::NOT_FOUND, Json(serde_json::json!({"error": "Site not found"}))).into_response();
    };
    let env_path = state.site_dir(&site).join(".env");
    let mut vars = Vec::new();
    if let Ok(content) = std::fs::read_to_string(env_path) {
        for line in content.lines() {
            let trimmed = line.trim();
            if trimmed.is_empty() || trimmed.starts_with('#') {
                continue;
            }
            if let Some((k, v)) = trimmed.split_once('=') {
                vars.push(serde_json::json!({"key": k.trim(), "value": v.trim()}));
            }
        }
    }
    Json(serde_json::json!({"vars": vars})).into_response()
}

async fn site_env_put_handler(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(payload): Json<serde_json::Value>,
) -> Response {
    let Some(site) = state.get_site(&id) else {
        return (StatusCode::NOT_FOUND, Json(serde_json::json!({"error": "Site not found"}))).into_response();
    };
    let env_path = state.site_dir(&site).join(".env");
    let mut lines = Vec::new();
    if let Some(vars) = payload.get("vars").and_then(|v| v.as_array()) {
        for item in vars {
            if let Some(k) = item.get("key").and_then(|v| v.as_str()).filter(|s| !s.is_empty()) {
                let v = item.get("value").and_then(|v| v.as_str()).unwrap_or("");
                lines.push(format!("{}={}", k, v));
            }
        }
    }
    let _ = std::fs::write(env_path, lines.join("\n"));
    if site.status.eq_ignore_ascii_case("active")
        && (site.runtime == "php" || site.runtime == "node")
    {
        state.stop_domain_workers(&site.domain);
        state.spawn_site_worker(&site);
    }
    Json(serde_json::json!({"ok": true})).into_response()
}

async fn site_redirects_get_handler(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Response {
    let Some(site) = state.get_site(&id) else {
        return (StatusCode::NOT_FOUND, Json(serde_json::json!({"error": "Site not found"}))).into_response();
    };
    Json(serde_json::json!({"redirects": site.redirects})).into_response()
}

async fn site_redirects_add_handler(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(payload): Json<serde_json::Value>,
) -> Response {
    let source = payload.get("source").and_then(|v| v.as_str()).unwrap_or("");
    let target = payload.get("target").and_then(|v| v.as_str()).unwrap_or("");
    let redirect_type = payload
        .get("redirect_type")
        .or_else(|| payload.get("type"))
        .and_then(|v| v.as_str())
        .unwrap_or("301");
    match state.add_site_redirect(&id, source, target, redirect_type) {
        Ok(redirects) => Json(serde_json::json!({"ok": true, "redirects": redirects})).into_response(),
        Err(e) => (StatusCode::BAD_REQUEST, Json(serde_json::json!({"error": e}))).into_response(),
    }
}

async fn site_redirects_remove_handler(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(payload): Json<serde_json::Value>,
) -> Response {
    let source = payload.get("source").and_then(|v| v.as_str()).unwrap_or("");
    match state.remove_site_redirect(&id, source) {
        Ok(redirects) => Json(serde_json::json!({"ok": true, "redirects": redirects})).into_response(),
        Err(e) => (StatusCode::BAD_REQUEST, Json(serde_json::json!({"error": e}))).into_response(),
    }
}

async fn site_aliases_get_handler(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Response {
    let Some(site) = state.get_site(&id) else {
        return (StatusCode::NOT_FOUND, Json(serde_json::json!({"error": "Site not found"}))).into_response();
    };
    Json(serde_json::json!({"aliases": site.aliases})).into_response()
}

async fn site_aliases_add_handler(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(payload): Json<serde_json::Value>,
) -> Response {
    let alias = payload.get("alias").and_then(|v| v.as_str()).unwrap_or("");
    match state.add_site_alias(&id, alias) {
        Ok(aliases) => Json(serde_json::json!({"ok": true, "aliases": aliases})).into_response(),
        Err(e) => (StatusCode::BAD_REQUEST, Json(serde_json::json!({"error": e}))).into_response(),
    }
}

async fn site_aliases_remove_handler(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(payload): Json<serde_json::Value>,
) -> Response {
    let alias = payload.get("alias").and_then(|v| v.as_str()).unwrap_or("");
    match state.remove_site_alias(&id, alias) {
        Ok(aliases) => Json(serde_json::json!({"ok": true, "aliases": aliases})).into_response(),
        Err(e) => (StatusCode::BAD_REQUEST, Json(serde_json::json!({"error": e}))).into_response(),
    }
}

async fn site_pass_protect_get_handler(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Response {
    let Some(site) = state.get_site(&id) else {
        return (StatusCode::NOT_FOUND, Json(serde_json::json!({"error": "Site not found"}))).into_response();
    };
    if let Some((user, _hash)) = state.get_site_pass_protect(&site.domain) {
        Json(serde_json::json!({"paths": ["/"], "users": [user]})).into_response()
    } else {
        Json(serde_json::json!({"paths": [], "users": []})).into_response()
    }
}

async fn site_pass_protect_add_handler(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(payload): Json<serde_json::Value>,
) -> Response {
    let Some(site) = state.get_site(&id) else {
        return (StatusCode::NOT_FOUND, Json(serde_json::json!({"error": "Site not found"}))).into_response();
    };
    let user = payload.get("username").and_then(|v| v.as_str()).unwrap_or("admin");
    let pass = payload.get("password").and_then(|v| v.as_str()).unwrap_or("");
    match state.set_site_pass_protect(&site.domain, true, Some(user), Some(pass)) {
        Ok(()) => Json(serde_json::json!({"ok": true})).into_response(),
        Err(e) => (StatusCode::BAD_REQUEST, Json(serde_json::json!({"error": e}))).into_response(),
    }
}

async fn site_pass_protect_remove_handler(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Response {
    let Some(site) = state.get_site(&id) else {
        return (StatusCode::NOT_FOUND, Json(serde_json::json!({"error": "Site not found"}))).into_response();
    };
    match state.set_site_pass_protect(&site.domain, false, None, None) {
        Ok(()) => Json(serde_json::json!({"ok": true})).into_response(),
        Err(e) => (StatusCode::BAD_REQUEST, Json(serde_json::json!({"error": e}))).into_response(),
    }
}

#[derive(serde::Deserialize)]
pub struct FilePathQuery {
    pub path: Option<String>,
    #[serde(rename = "type")]
    pub entry_type: Option<String>,
}

async fn files_list_handler(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Query(q): Query<FilePathQuery>,
) -> Response {
    let Some(site) = state.get_site(&id) else {
        return (StatusCode::NOT_FOUND, Json(serde_json::json!({"error": "Site not found"}))).into_response();
    };
    let site_dir = state.site_dir(&site);
    match files::list_dir(&site_dir, q.path.as_deref().unwrap_or(".")) {
        Ok(entries) => Json(entries).into_response(),
        Err(e) => (StatusCode::BAD_REQUEST, Json(serde_json::json!({"error": e}))).into_response(),
    }
}

async fn files_tree_handler(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Query(q): Query<FilePathQuery>,
) -> Response {
    let Some(site) = state.get_site(&id) else {
        return (StatusCode::NOT_FOUND, Json(serde_json::json!({"error": "Site not found"}))).into_response();
    };
    let site_dir = state.site_dir(&site);
    match files::list_tree(&site_dir, q.path.as_deref().unwrap_or(".")) {
        Ok(items) => Json(items).into_response(),
        Err(e) => (StatusCode::BAD_REQUEST, Json(serde_json::json!({"error": e}))).into_response(),
    }
}

async fn files_read_handler(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Query(q): Query<FilePathQuery>,
) -> Response {
    let Some(site) = state.get_site(&id) else {
        return (StatusCode::NOT_FOUND, Json(serde_json::json!({"error": "Site not found"}))).into_response();
    };
    let site_dir = state.site_dir(&site);
    match files::read_file_content(&site_dir, q.path.as_deref().unwrap_or("")) {
        Ok(content) => Json(serde_json::json!({"content": content})).into_response(),
        Err(e) => (StatusCode::BAD_REQUEST, Json(serde_json::json!({"error": e}))).into_response(),
    }
}

async fn files_write_handler(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(payload): Json<serde_json::Value>,
) -> Response {
    let Some(site) = state.get_site(&id) else {
        return (StatusCode::NOT_FOUND, Json(serde_json::json!({"error": "Site not found"}))).into_response();
    };
    let path = payload.get("path").and_then(|v| v.as_str()).unwrap_or("");
    let content = payload.get("content").and_then(|v| v.as_str()).unwrap_or("");
    let site_dir = state.site_dir(&site);
    match files::write_file_content(&site_dir, path, content) {
        Ok(()) => Json(serde_json::json!({"ok": true})).into_response(),
        Err(e) => (StatusCode::BAD_REQUEST, Json(serde_json::json!({"error": e}))).into_response(),
    }
}

async fn files_create_handler(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Query(q): Query<FilePathQuery>,
) -> Response {
    let Some(site) = state.get_site(&id) else {
        return (StatusCode::NOT_FOUND, Json(serde_json::json!({"error": "Site not found"}))).into_response();
    };
    let site_dir = state.site_dir(&site);
    match files::create_entry(
        &site_dir,
        q.path.as_deref().unwrap_or(""),
        q.entry_type.as_deref().unwrap_or("file"),
    ) {
        Ok(()) => Json(serde_json::json!({"ok": true})).into_response(),
        Err(e) => (StatusCode::BAD_REQUEST, Json(serde_json::json!({"error": e}))).into_response(),
    }
}

async fn files_rename_handler(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(payload): Json<serde_json::Value>,
) -> Response {
    let Some(site) = state.get_site(&id) else {
        return (StatusCode::NOT_FOUND, Json(serde_json::json!({"error": "Site not found"}))).into_response();
    };
    let from = payload.get("from").and_then(|v| v.as_str()).unwrap_or("");
    let to = payload.get("to").and_then(|v| v.as_str()).unwrap_or("");
    let site_dir = state.site_dir(&site);
    match files::rename_entry(&site_dir, from, to) {
        Ok(()) => Json(serde_json::json!({"ok": true})).into_response(),
        Err(e) => (StatusCode::BAD_REQUEST, Json(serde_json::json!({"error": e}))).into_response(),
    }
}

async fn files_delete_handler(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Query(q): Query<FilePathQuery>,
) -> Response {
    let Some(site) = state.get_site(&id) else {
        return (StatusCode::NOT_FOUND, Json(serde_json::json!({"error": "Site not found"}))).into_response();
    };
    let site_dir = state.site_dir(&site);
    match files::delete_entry(&site_dir, q.path.as_deref().unwrap_or("")) {
        Ok(()) => Json(serde_json::json!({"ok": true})).into_response(),
        Err(e) => (StatusCode::BAD_REQUEST, Json(serde_json::json!({"error": e}))).into_response(),
    }
}

async fn files_delete_batch_handler(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(payload): Json<serde_json::Value>,
) -> Response {
    let Some(site) = state.get_site(&id) else {
        return (StatusCode::NOT_FOUND, Json(serde_json::json!({"error": "Site not found"}))).into_response();
    };
    let site_dir = state.site_dir(&site);
    if let Some(paths) = payload.get("paths").and_then(|v| v.as_array()) {
        for p in paths.iter().filter_map(|v| v.as_str()) {
            let _ = files::delete_entry(&site_dir, p);
        }
    }
    Json(serde_json::json!({"ok": true})).into_response()
}

async fn files_copy_or_move_handler(
    state: Arc<AppState>,
    id: String,
    payload: serde_json::Value,
    move_items: bool,
) -> Response {
    let Some(site) = state.get_site(&id) else {
        return (StatusCode::NOT_FOUND, Json(serde_json::json!({"error": "Site not found"}))).into_response();
    };
    let site_dir = state.site_dir(&site);
    let dest = payload
        .get("destination_dir")
        .or_else(|| payload.get("destination"))
        .and_then(|v| v.as_str())
        .unwrap_or(".");
    let sources: Vec<String> = payload
        .get("sources")
        .or_else(|| payload.get("paths"))
        .and_then(|v| v.as_array())
        .map(|arr| arr.iter().filter_map(|x| x.as_str().map(String::from)).collect())
        .unwrap_or_default();
    match files::copy_or_move(&site_dir, &sources, dest, move_items) {
        Ok(()) => Json(serde_json::json!({"ok": true})).into_response(),
        Err(e) => (StatusCode::BAD_REQUEST, Json(serde_json::json!({"error": e}))).into_response(),
    }
}

async fn files_compress_handler(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(payload): Json<serde_json::Value>,
) -> Response {
    let Some(site) = state.get_site(&id) else {
        return (StatusCode::NOT_FOUND, Json(serde_json::json!({"error": "Site not found"}))).into_response();
    };
    let site_dir = state.site_dir(&site);
    let sources: Vec<String> = payload
        .get("sources")
        .and_then(|v| v.as_array())
        .map(|arr| arr.iter().filter_map(|x| x.as_str().map(String::from)).collect())
        .unwrap_or_default();
    let dest = payload
        .get("destination")
        .and_then(|v| v.as_str())
        .unwrap_or("archive.zip");
    match files::compress_paths(&site_dir, &sources, dest) {
        Ok(archive) => {
            state.record_event(
                "files_compress",
                "file",
                &site.domain,
                "Files Compressed",
                &format!("Created {} in {}", archive, site.domain),
                "info",
            );
            Json(serde_json::json!({ "ok": true, "archive": archive })).into_response()
        }
        Err(e) => (StatusCode::BAD_REQUEST, Json(serde_json::json!({"error": e}))).into_response(),
    }
}

async fn files_extract_handler(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(payload): Json<serde_json::Value>,
) -> Response {
    let Some(site) = state.get_site(&id) else {
        return (StatusCode::NOT_FOUND, Json(serde_json::json!({"error": "Site not found"}))).into_response();
    };
    let site_dir = state.site_dir(&site);
    let archive = payload.get("archive").and_then(|v| v.as_str()).unwrap_or("");
    let dest = payload.get("destination").and_then(|v| v.as_str()).unwrap_or(".");
    match files::extract_archive(&site_dir, archive, dest) {
        Ok(()) => {
            state.record_event(
                "files_extract",
                "file",
                &site.domain,
                "Archive Extracted",
                &format!("Extracted {} to {} in {}", archive, dest, site.domain),
                "info",
            );
            Json(serde_json::json!({ "ok": true })).into_response()
        }
        Err(e) => (StatusCode::BAD_REQUEST, Json(serde_json::json!({"error": e}))).into_response(),
    }
}

async fn files_download_handler(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Query(q): Query<FilePathQuery>,
) -> Response {
    let Some(site) = state.get_site(&id) else {
        return (StatusCode::NOT_FOUND, Json(serde_json::json!({"error": "Site not found"}))).into_response();
    };
    let site_dir = state.site_dir(&site);
    let rel = q.path.as_deref().unwrap_or("");
    let Ok(target) = files::resolve_safe_path(&site_dir, rel) else {
        return (StatusCode::BAD_REQUEST, Json(serde_json::json!({"error": "Invalid path"}))).into_response();
    };
    match std::fs::read(&target) {
        Ok(bytes) => {
            let filename = target
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("download.bin");
            (
                StatusCode::OK,
                [
                    (header::CONTENT_TYPE, "application/octet-stream".to_string()),
                    (
                        header::CONTENT_DISPOSITION,
                        format!("attachment; filename=\"{}\"", filename),
                    ),
                ],
                bytes,
            )
                .into_response()
        }
        Err(_) => (StatusCode::NOT_FOUND, Json(serde_json::json!({"error": "File not found"}))).into_response(),
    }
}

async fn wordpress_sites_handler(State(state): State<Arc<AppState>>) -> Json<serde_json::Value> {
    let mut rows = Vec::new();
    for site in state.list_sites() {
        let site_dir = state.site_dir(&site);
        if wordpress::is_wp_installed(&site_dir) || site.php_preset.as_deref() == Some("wordpress") {
            let ver = wordpress::read_wp_version(&site_dir);
            let (vulns, critical_vulns) = wordpress::get_cached_vuln_counts(&site_dir);
            rows.push(serde_json::json!({
                "site_id": site.id,
                "domain": site.domain,
                "wp_version": ver,
                "version": ver,
                "status": site.status,
                "ssl_enabled": site.ssl_enabled,
                "auto_update": false,
                "update_available": false,
                "vulns": vulns,
                "critical_vulns": critical_vulns,
                "vuln_count": vulns
            }));
        }
    }
    Json(serde_json::Value::Array(rows))
}

async fn wordpress_toggle_feature_handler(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(payload): Json<serde_json::Value>,
) -> Response {
    let Some(site) = state.get_site(&id) else {
        return (StatusCode::NOT_FOUND, Json(serde_json::json!({"error": "Site not found"}))).into_response();
    };
    let site_dir = state.site_dir(&site);
    let dom = site.domain.clone();
    let feature = payload
        .get("feature")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let enabled = payload.get("enabled").and_then(|v| v.as_bool()).unwrap_or(false);
    let params = payload.get("params").cloned();
    let st = state.clone();
    let feat_cl = feature.clone();
    let dom_cl = dom.clone();
    let res = tokio::task::spawn_blocking(move || {
        wordpress::toggle_feature(&st, &site_dir, &dom_cl, &feat_cl, enabled, params.as_ref())
    })
    .await;
    match res {
        Ok(Ok(val)) => {
            state.record_event(
                "wp_toggle_feature",
                "site",
                &dom,
                "WordPress Feature Updated",
                &format!("Set {}={} on {}", feature, enabled, dom),
                "info",
            );
            Json(val).into_response()
        }
        Ok(Err(e)) => (StatusCode::BAD_REQUEST, Json(serde_json::json!({"error": e}))).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({"error": e.to_string()}))).into_response(),
    }
}

async fn wordpress_maintenance_handler(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(payload): Json<serde_json::Value>,
) -> Response {
    let Some(site) = state.get_site(&id) else {
        return (StatusCode::NOT_FOUND, Json(serde_json::json!({"error": "Site not found"}))).into_response();
    };
    let site_dir = state.site_dir(&site);
    let enabled = payload.get("enabled").and_then(|v| v.as_bool()).unwrap_or(false);
    match wordpress::set_maintenance(&site_dir, enabled) {
        Ok(res) => {
            state.record_event(
                "wp_maintenance",
                "site",
                &site.domain,
                "Maintenance Mode Updated",
                &format!("{} maintenance mode for {}", if enabled { "Enabled" } else { "Disabled" }, site.domain),
                "info",
            );
            Json(res).into_response()
        }
        Err(e) => (StatusCode::BAD_REQUEST, Json(serde_json::json!({"error": e}))).into_response(),
    }
}

async fn wordpress_setup_handler(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(payload): Json<serde_json::Value>,
) -> Response {
    let Some(site) = state.get_site(&id) else {
        return (StatusCode::NOT_FOUND, Json(serde_json::json!({"error": "Site not found"}))).into_response();
    };
    let site_dir = state.site_dir(&site);
    let dom = site.domain.clone();
    let admin_user = payload.get("admin_user").and_then(|v| v.as_str()).map(String::from);
    let admin_email = payload.get("admin_email").and_then(|v| v.as_str()).map(String::from);
    let admin_pass = payload
        .get("admin_pass")
        .or_else(|| payload.get("admin_password"))
        .and_then(|v| v.as_str())
        .map(String::from);
    let site_title = payload
        .get("site_title")
        .or_else(|| payload.get("title"))
        .and_then(|v| v.as_str())
        .map(String::from);

    let dom_cl = dom.clone();
    let res = tokio::task::spawn_blocking(move || {
        wordpress::setup_wp(
            &site_dir,
            &dom_cl,
            admin_user.as_deref(),
            admin_email.as_deref(),
            admin_pass.as_deref(),
            site_title.as_deref(),
        )
    })
    .await;

    match res {
        Ok(Ok(val)) => {
            state.record_event(
                "wp_setup",
                "site",
                &dom,
                "WordPress Setup Updated",
                &format!("Updated administrator access and title for {}", dom),
                "success",
            );
            Json(val).into_response()
        }
        Ok(Err(e)) => (StatusCode::BAD_REQUEST, Json(serde_json::json!({"error": e}))).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({"error": e.to_string()}))).into_response(),
    }
}

async fn wordpress_clear_cache_handler(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Response {
    let Some(site) = state.get_site(&id) else {
        return (StatusCode::NOT_FOUND, Json(serde_json::json!({"error": "Site not found"}))).into_response();
    };
    let site_dir = state.site_dir(&site);
    let dom = site.domain.clone();
    let dom_cl = dom.clone();
    let res = tokio::task::spawn_blocking(move || wordpress::clear_wp_cache(&site_dir, &dom_cl)).await;
    match res {
        Ok(Ok(val)) => {
            state.record_event(
                "wp_clear_cache",
                "site",
                &dom,
                "WordPress Cache Purged",
                &format!("Purged cache for {}", dom),
                "info",
            );
            Json(val).into_response()
        }
        Ok(Err(e)) => (StatusCode::BAD_REQUEST, Json(serde_json::json!({"error": e}))).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({"error": e.to_string()}))).into_response(),
    }
}

#[derive(serde::Deserialize)]
pub struct PortConflictQuery {
    pub port: Option<u16>,
}

async fn port_conflict_handler(
    State(state): State<Arc<AppState>>,
    axum::extract::Query(query): axum::extract::Query<PortConflictQuery>,
) -> Json<serde_json::Value> {
    let cfg = state.get_router_config();
    let port = query.port.unwrap_or(cfg.preferred_port);
    let mut status = router::check_port_status(port);

    let my_pid = std::process::id();
    if status.pid == Some(my_pid) {
        status.in_use = false;
        status.process_name = None;
        status.pid = None;
    }

    let display_name = if !status.in_use {
        None
    } else {
        status.process_name.as_ref().map(|name| {
            let lower = name.to_lowercase();
            if lower.contains("httpd") || lower.contains("apache") {
                "Apache HTTP Server (WampServer)".to_string()
            } else if lower.contains("nginx") {
                "Nginx Web Server".to_string()
            } else if lower == "system" || status.pid == Some(4) {
                "Windows HTTP.sys (IIS / System)".to_string()
            } else {
                name.clone()
            }
        })
    };

    Json(serde_json::json!({
        "preferred_port": cfg.preferred_port,
        "active_port": cfg.active_port,
        "in_use": status.in_use,
        "process_name": status.process_name,
        "display_name": display_name,
        "pid": status.pid,
        "can_auto_stop": false,
        "alternatives": [8080, 8443, 3000]
    }))
}

#[derive(serde::Deserialize)]
pub struct PortResolvePayload {
    pub action: String,
    pub port: Option<u16>,
    pub pid: Option<u32>,
}

async fn port_resolve_handler(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<PortResolvePayload>,
) -> Response {
    match payload.action.as_str() {
        "use_port" => {
            let target_port = payload.port.unwrap_or(8080);
            let active = state.get_router_config().active_port;

            if target_port == active {
                return Json(serde_json::json!({
                    "success": true,
                    "active_port": target_port
                }))
                .into_response();
            }

            if !router::is_port_bindable(target_port) {
                return Json(serde_json::json!({
                    "success": false,
                    "message": "Port is already in use"
                }))
                .into_response();
            }

            if state.restart_site_router(target_port).await {
                state.set_router_port(target_port);
                state.record_event(
                    "port_resolve",
                    "system",
                    "router",
                    "Router Port Updated",
                    &format!("Switched site router to port {}", target_port),
                    "info",
                );
                Json(serde_json::json!({
                    "success": true,
                    "active_port": target_port,
                    "message": format!("Switched router port to {}", target_port)
                }))
                .into_response()
            } else {
                let _ = state.restart_site_router(active).await;
                Json(serde_json::json!({
                    "success": false,
                    "active_port": active,
                    "message": format!("Failed to bind site router on port {}", target_port)
                }))
                .into_response()
            }
        }
        "stop_process" => {
            let pid = match payload.pid {
                Some(p) => p,
                None => {
                    return (
                        StatusCode::BAD_REQUEST,
                        Json(serde_json::json!({
                            "success": false,
                            "error": "PID is required to stop process"
                        })),
                    )
                        .into_response();
                }
            };

            let my_pid = std::process::id();
            if pid == my_pid || pid == 0 || pid == 4 {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(serde_json::json!({
                        "success": false,
                        "message": "Cannot terminate current process or system process"
                    })),
                )
                    .into_response();
            }

            let port = payload.port.unwrap_or(80);
            let current_status = router::check_port_status(port);
            if current_status.pid != Some(pid) {
                return (
                    StatusCode::CONFLICT,
                    Json(serde_json::json!({
                        "success": false,
                        "message": format!("PID {} does not own port {}", pid, port)
                    })),
                )
                    .into_response();
            }

            #[cfg(windows)]
            let success = {
                use std::os::windows::process::CommandExt;
                const CREATE_NO_WINDOW: u32 = 0x08000000;
                let res = std::process::Command::new("taskkill")
                    .args(["/F", "/PID", &pid.to_string(), "/T"])
                    .creation_flags(CREATE_NO_WINDOW)
                    .output();
                match res {
                    Ok(out) => out.status.success(),
                    Err(_) => false,
                }
            };
            #[cfg(not(windows))]
            let success = false;

            if success {
                tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
                let status = router::check_port_status(port);
                if !status.in_use {
                    if state.restart_site_router(port).await {
                        state.set_router_port(port);
                        Json(serde_json::json!({
                            "success": true,
                            "active_port": port,
                            "message": format!("Successfully stopped conflicting process and bound port {}", port)
                        }))
                        .into_response()
                    } else {
                        Json(serde_json::json!({
                            "success": false,
                            "message": format!("Process was terminated but failed to bind port {}", port)
                        }))
                        .into_response()
                    }
                } else {
                    Json(serde_json::json!({
                        "success": false,
                        "message": format!("Process was terminated but port {} is still in use", port)
                    }))
                    .into_response()
                }
            } else {
                Json(serde_json::json!({
                    "success": false,
                    "message": "Failed to stop process. It may require Administrator privileges or is running as a Windows Service. Consider selecting Port 8080."
                }))
                .into_response()
            }
        }
        other => (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "success": false,
                "error": format!("Unknown action '{}'", other)
            })),
        )
            .into_response(),
    }
}

fn validate_site_domain_param(domain: &str) -> Result<String, String> {
    let trimmed = domain.trim();
    if trimmed.is_empty()
        || trimmed.contains("..")
        || trimmed.contains('/')
        || trimmed.contains('\\')
        || trimmed.contains(':')
    {
        return Err("Invalid site domain".to_string());
    }
    Ok(trimmed.to_string())
}

fn format_uptime(secs: u64) -> String {
    let days = secs / 86400;
    let hours = (secs % 86400) / 3600;
    let mins = (secs % 3600) / 60;
    let rem = secs % 60;
    if days > 0 {
        format!("{}d {}h {}m", days, hours, mins)
    } else if hours > 0 {
        format!("{}h {}m {}s", hours, mins, rem)
    } else if mins > 0 {
        format!("{}m {}s", mins, rem)
    } else {
        format!("{}s", rem)
    }
}

fn validate_sqlite_ident(name: &str) -> Result<&str, String> {
    let trimmed = name.trim();
    if trimmed.is_empty()
        || trimmed.len() > 128
        || !trimmed
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    {
        return Err("Invalid identifier".to_string());
    }
    Ok(trimmed)
}

fn json_val_to_string(v: &serde_json::Value) -> String {
    match v {
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Bool(b) => b.to_string(),
        serde_json::Value::Number(n) => n.to_string(),
        serde_json::Value::Null => String::new(),
        other => other.to_string(),
    }
}

fn format_notification_json(n: &state::LocalNotification) -> serde_json::Value {
    serde_json::json!({
        "id": n.id,
        "title": n.title,
        "message": n.message,
        "severity": n.severity,
        "category": "desktop",
        "link": serde_json::Value::Null,
        "read": n.read,
        "read_at": if n.read {
            serde_json::Value::String(n.created_at.clone())
        } else {
            serde_json::Value::Null
        },
        "created_at": n.created_at,
    })
}

fn read_desktop_log_lines(
    state: &AppState,
    log_type: &str,
    limit: usize,
    filter: Option<&str>,
    site_domain: Option<&str>,
) -> Vec<String> {
    let mut candidates: Vec<PathBuf> = if let Some(dom) = site_domain {
        let wp_debug_log = state
            .get_site(dom)
            .map(|s| state.site_dir(&s).join("wp-content").join("debug.log"))
            .filter(|p| p.is_file());
        match log_type {
            "php" | "php_errors" => {
                let mut v = vec![
                    state.php_error_log_path(dom),
                    state.php_worker_log_path(dom),
                ];
                if let Some(dbg) = wp_debug_log {
                    v.push(dbg);
                }
                v.push(state.config_dir.join("php_errors.log"));
                v
            }
            "error" | "nginx_error" | "php_fpm" => {
                let mut v = vec![
                    state.php_error_log_path(dom),
                    state.php_worker_log_path(dom),
                ];
                if let Some(dbg) = wp_debug_log {
                    v.push(dbg);
                }
                v.push(state.config_dir.join("php_errors.log"));
                v
            }
            "access" | "nginx_access" => {
                vec![
                    state.access_log_path(dom),
                    state.config_dir.join("desktop_activity.log"),
                ]
            }
            _ => {
                let mut v = vec![
                    state.access_log_path(dom),
                    state.php_error_log_path(dom),
                    state.php_worker_log_path(dom),
                ];
                if let Some(dbg) = wp_debug_log {
                    v.push(dbg);
                }
                v.push(state.config_dir.join("desktop_activity.log"));
                v
            }
        }
    } else {
        match log_type {
            "error" | "nginx_error" | "php_fpm" | "php_errors" | "php" => {
                vec![
                    state.config_dir.join("php_errors.log"),
                    state.config_dir.join("webview_console.log"),
                ]
            }
            "access" | "nginx_access" | "syslog" | "auth" | "service" => {
                vec![state.config_dir.join("desktop_activity.log")]
            }
            _ => vec![
                state.config_dir.join("desktop_activity.log"),
                state.config_dir.join("php_errors.log"),
                state.config_dir.join("webview_console.log"),
            ],
        }
    };

    // WebView IPC handler also writes directly into %APPDATA%\DockPanel\webview_console.log
    if site_domain.is_none() && (log_type == "error" || log_type == "nginx_error" || log_type == "php_fpm") {
        if let Ok(app_data) = std::env::var("APPDATA") {
            let global_webview = PathBuf::from(app_data)
                .join("DockPanel")
                .join("webview_console.log");
            if !candidates.contains(&global_webview) {
                candidates.push(global_webview);
            }
        }
    }

    let filter_lower = filter.map(|s| s.trim().to_lowercase()).filter(|s| !s.is_empty());
    let domain_lower = site_domain.map(|d| d.to_lowercase());

    let mut collected = Vec::new();
    for path in candidates {
        // Only fall back to legacy global php_errors.log when per-site PHP logs have nothing yet
        if site_domain.is_some()
            && path.ends_with("php_errors.log")
            && path.parent() == Some(state.config_dir.as_path())
            && !collected.is_empty()
        {
            continue;
        }
        // Only fall back to desktop_activity.log for access logs if no per-site HTTP access lines exist yet
        if site_domain.is_some()
            && path.ends_with("desktop_activity.log")
            && (log_type == "access" || log_type == "nginx_access")
            && !collected.is_empty()
        {
            continue;
        }

        if let Ok(content) = std::fs::read_to_string(&path) {
            for line in content.lines() {
                let trimmed = line.trim();
                if trimmed.is_empty() {
                    continue;
                }
                let lower = trimmed.to_lowercase();
                if let Some(ref d) = domain_lower {
                    if !lower.contains(d) && path.ends_with("desktop_activity.log") {
                        continue;
                    }
                }
                if let Some(ref f) = filter_lower {
                    if !lower.contains(f) {
                        continue;
                    }
                }
                collected.push(trimmed.to_string());
            }
        }
    }

    if collected.len() > limit {
        collected.split_off(collected.len() - limit)
    } else {
        collected
    }
}

async fn desktop_open_folder_handler(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<serde_json::Value>,
) -> Response {
    let raw_domain = payload
        .get("domain")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let domain = match validate_site_domain_param(raw_domain) {
        Ok(d) => d,
        Err(e) => {
            return (StatusCode::BAD_REQUEST, Json(serde_json::json!({"error": e}))).into_response();
        }
    };

    let site_dir = state
        .get_site(&domain)
        .map(|s| state.site_dir(&s))
        .unwrap_or_else(|| state.get_default_sites_dir().join(&domain));
    if !site_dir.exists() {
        let _ = std::fs::create_dir_all(&site_dir);
    }

    #[cfg(all(windows, not(test)))]
    {
        if let Err(e) = std::process::Command::new("explorer.exe")
            .arg(&site_dir)
            .spawn()
        {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error": format!("Failed to launch Explorer: {}", e)})),
            )
                .into_response();
        }
    }

    state.record_event(
        "open_folder",
        "site",
        &domain,
        "Opened Site Folder",
        &format!("Opened {} in File Explorer", site_dir.display()),
        "info",
    );
    Json(serde_json::json!({
        "ok": true,
        "path": site_dir.to_string_lossy().to_string()
    }))
    .into_response()
}

async fn desktop_open_vscode_handler(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<serde_json::Value>,
) -> Response {
    let raw_domain = payload
        .get("domain")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let domain = match validate_site_domain_param(raw_domain) {
        Ok(d) => d,
        Err(e) => {
            return (StatusCode::BAD_REQUEST, Json(serde_json::json!({"error": e}))).into_response();
        }
    };

    let site_dir = state
        .get_site(&domain)
        .map(|s| state.site_dir(&s))
        .unwrap_or_else(|| state.get_default_sites_dir().join(&domain));
    if !site_dir.exists() {
        let _ = std::fs::create_dir_all(&site_dir);
    }

    #[cfg(all(windows, not(test)))]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x08000000;
        let found = std::process::Command::new("where.exe")
            .arg("code")
            .creation_flags(CREATE_NO_WINDOW)
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false);
        if !found {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({
                    "error": "VS Code CLI ('code') was not found on PATH. Open VS Code and enable 'Add to PATH'."
                })),
            )
                .into_response();
        }
        if let Err(e) = std::process::Command::new("cmd.exe")
            .args(["/c", "code", &site_dir.to_string_lossy()])
            .creation_flags(CREATE_NO_WINDOW)
            .spawn()
        {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error": format!("Failed to open VS Code: {}", e)})),
            )
                .into_response();
        }
    }

    state.record_event(
        "open_vscode",
        "site",
        &domain,
        "Opened in VS Code",
        &format!("Opened {} in VS Code", site_dir.display()),
        "info",
    );
    Json(serde_json::json!({
        "ok": true,
        "path": site_dir.to_string_lossy().to_string()
    }))
    .into_response()
}

async fn desktop_clear_cache_handler(State(state): State<Arc<AppState>>) -> Json<serde_json::Value> {
    let app_data = std::env::var("APPDATA")
        .unwrap_or_else(|_| std::env::var("USERPROFILE").unwrap_or_else(|_| ".".into()));
    let cache_dir = PathBuf::from(app_data).join("DockPanel").join("cache");
    let mut cleared_bytes: u64 = 0;
    if let Ok(rd) = std::fs::read_dir(&cache_dir) {
        for entry in rd.flatten() {
            let p = entry.path();
            if p.is_file() {
                if let Ok(m) = entry.metadata() {
                    cleared_bytes += m.len();
                }
                let _ = std::fs::remove_file(&p);
            }
        }
    }
    state.record_event(
        "clear_cache",
        "system",
        "cache",
        "Download Cache Cleared",
        &format!("Cleared {} KB from DockPanel download cache", cleared_bytes / 1024),
        "info",
    );
    Json(serde_json::json!({ "ok": true, "cleared_bytes": cleared_bytes }))
}

async fn desktop_pick_folder_handler(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<Option<serde_json::Value>>,
) -> Response {
    let initial_dir = payload
        .as_ref()
        .and_then(|p| p.get("initial_dir"))
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(String::from)
        .unwrap_or_else(|| state.get_default_sites_dir().to_string_lossy().to_string());

    let dialog_title = payload
        .as_ref()
        .and_then(|p| p.get("description"))
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(String::from)
        .unwrap_or_else(|| "Select website folder (empty or existing website)".to_string());

    let pick_res = pick_windows_folder_blocking(initial_dir, dialog_title).await;

    match pick_res {
        Ok(Some(path)) => {
            let folder_path = std::path::PathBuf::from(&path);
            let st_clone = state.clone();
            let fp_clone = folder_path.clone();
            let (inspection, detected_db_files, other_directory_db_files) =
                tokio::task::spawn_blocking(move || {
                    st_clone.approve_db_scan_dir(&fp_clone);
                    let insp = wordpress::inspect_site_folder(&fp_clone);
                    let name_hint = fp_clone
                        .file_name()
                        .map(|n| n.to_string_lossy().to_string())
                        .unwrap_or_default();
                    let db_files = scan_folder_db_files(&fp_clone, Some(&name_hint), Some(&insp));
                    let root_dir = st_clone.get_default_sites_dir();
                    st_clone.approve_db_scan_dir(&root_dir);
                    let other_files = scan_other_directory_db_files(
                        &root_dir,
                        Some(&fp_clone),
                        Some(&name_hint),
                        Some(&insp),
                    );
                    (insp, db_files, other_files)
                })
                .await
                .unwrap_or_else(|_| {
                    (
                        wordpress::inspect_site_folder(std::path::Path::new(&path)),
                        Vec::new(),
                        Vec::new(),
                    )
                });
            Json(serde_json::json!({
                "cancelled": false,
                "path": path,
                "inspection": inspection,
                "detected_db_files": detected_db_files,
                "other_directory_db_files": other_directory_db_files,
            }))
            .into_response()
        }
        Ok(None) => Json(serde_json::json!({ "cancelled": true })).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e })),
        )
            .into_response(),
    }
}

async fn pick_windows_folder_blocking(
    initial_dir: String,
    description: String,
) -> Result<Option<String>, String> {
    tokio::task::spawn_blocking(move || {
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x08000000;
            let escaped = initial_dir.replace('\'', "''");
            let escaped_desc = description.replace('\'', "''");
            let ps = format!(
                r#"
Add-Type -AssemblyName System.Windows.Forms
Add-Type -AssemblyName System.Drawing
$owner = New-Object System.Windows.Forms.Form
$owner.TopMost = $true
$owner.ShowInTaskbar = $false
$owner.FormBorderStyle = 'None'
$owner.StartPosition = 'Manual'
$owner.Location = New-Object System.Drawing.Point(-32000, -32000)
$owner.Size = New-Object System.Drawing.Size(1, 1)
$owner.Show()
$dlg = New-Object System.Windows.Forms.FolderBrowserDialog
$dlg.Description = '{escaped_desc}'
$dlg.UseDescriptionForTitle = $true
$dlg.ShowNewFolderButton = $true
$init = '{escaped}'
if ($init -and (Test-Path -LiteralPath $init)) {{
    $dlg.SelectedPath = $init
}}
$res = $dlg.ShowDialog($owner)
$owner.Close()
$owner.Dispose()
if ($res -eq [System.Windows.Forms.DialogResult]::OK) {{
    [Console]::OutputEncoding = [System.Text.Encoding]::UTF8
    Write-Output $dlg.SelectedPath
}}
"#
            );
            let out = std::process::Command::new("powershell.exe")
                .args(["-NoProfile", "-STA", "-ExecutionPolicy", "Bypass", "-Command", &ps])
                .creation_flags(CREATE_NO_WINDOW)
                .output()
                .map_err(|e| format!("Failed opening Windows folder dialog: {}", e))?;
            if !out.status.success() {
                let err = String::from_utf8_lossy(&out.stderr).trim().to_string();
                return Err(if err.is_empty() {
                    "Folder picker failed".to_string()
                } else {
                    err
                });
            }
            let selected = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if selected.is_empty() {
                Ok(None)
            } else {
                Ok(Some(selected))
            }
        }
        #[cfg(not(windows))]
        {
            let _ = (initial_dir, description);
            Ok(None)
        }
    })
    .await
    .map_err(|e| e.to_string())
    .and_then(|r| r)
}

#[derive(Debug, Clone, serde::Serialize)]
struct DetectedDbFile {
    name: String,
    relative_path: String,
    path: String,
    size_bytes: u64,
    source_scope: String,
    source_dir: String,
    detected_prefix: Option<String>,
    detected_site_url: Option<String>,
    detected_db_name: Option<String>,
    matches_site: Option<bool>,
    match_reason: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize)]
struct DiscoveredFolderCandidate {
    name: String,
    path: String,
    suggested_domain: String,
    suggested_runtime: String,
    detected_db_files: Vec<DetectedDbFile>,
    inspection: wordpress::FolderInspection,
}

fn normalize_dir_key(path: &std::path::Path) -> String {
    let raw = std::fs::canonicalize(path)
        .unwrap_or_else(|_| path.to_path_buf())
        .to_string_lossy()
        .to_string();
    let no_unc = raw.strip_prefix(r"\\?\").unwrap_or(&raw);
    no_unc
        .trim_end_matches(['\\', '/'])
        .replace('/', "\\")
        .to_ascii_lowercase()
}

fn suggest_domain_for_folder(
    folder_name: &str,
    taken_domains: &std::collections::HashSet<String>,
) -> String {
    let lower = folder_name.trim().to_ascii_lowercase();
    let (base_stem, preferred_suffix) = if let Some(s) = lower.strip_suffix(".test") {
        (s, ".test")
    } else if let Some(s) = lower.strip_suffix(".local") {
        (s, ".local")
    } else {
        let mut stripped = lower.as_str();
        for ext in [".com", ".net", ".org", ".io", ".dev", ".co", ".app", ".shop", ".store"] {
            if let Some(s) = stripped.strip_suffix(ext) {
                if !s.is_empty() {
                    stripped = s;
                    break;
                }
            }
        }
        (stripped, ".test")
    };

    let mut clean: String = base_stem
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' {
                c
            } else {
                '-'
            }
        })
        .collect();
    while clean.contains("--") {
        clean = clean.replace("--", "-");
    }
    let clean = clean.trim_matches('-').to_string();
    let stem = if clean.is_empty() {
        "site".to_string()
    } else {
        clean.chars().take(48).collect()
    };

    let first = format!("{}{}", stem, preferred_suffix);
    if !taken_domains.contains(&first) {
        return first;
    }
    for n in 2..=99 {
        let next = format!("{}-{}{}", stem, n, preferred_suffix);
        if !taken_domains.contains(&next) {
            return next;
        }
    }
    format!("{}-{}{}", stem, uuid::Uuid::new_v4().simple(), preferred_suffix)
}

fn is_supported_db_dump_filename(lower_name: &str, rel_lower: &str) -> bool {
    if lower_name.ends_with(".sql")
        || lower_name.ends_with(".sql.gz")
        || lower_name.ends_with(".sqlite")
        || lower_name.ends_with(".db")
    {
        return true;
    }
    if lower_name.ends_with(".zip") {
        let in_db_subdir = ["db", "database", "sql", "backup", "backups", "dump", "dumps", "ai1wm-backups", "updraft"]
            .iter()
            .any(|seg| rel_lower.contains(&format!("{}/", seg)));
        return in_db_subdir
            || lower_name.contains("db")
            || lower_name.contains("sql")
            || lower_name.contains("dump")
            || lower_name.contains("backup")
            || lower_name.contains("database");
    }
    false
}

fn should_skip_scan_subdir(lower_name: &str) -> bool {
    lower_name.starts_with('.')
        || matches!(
            lower_name,
            "node_modules"
                | "__pycache__"
                | "vendor"
                | "wp-includes"
                | "wp-admin"
                | ".git"
                | ".svn"
                | ".hg"
                | ".next"
                | "dist"
                | "build"
                | "target"
                | "$recycle.bin"
                | "system volume information"
        )
}

fn url_host_stem(url_or_domain: &str) -> String {
    let no_scheme = url_or_domain
        .trim()
        .to_ascii_lowercase()
        .replace("https://", "")
        .replace("http://", "");
    let host = no_scheme.split('/').next().unwrap_or("").trim_start_matches("www.");
    let stem = host.split('.').next().unwrap_or(host);
    stem.chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .collect()
}

fn evaluate_db_file_match(
    folder_name_hint: Option<&str>,
    inspection: Option<&wordpress::FolderInspection>,
    filename: &str,
    relative_path: &str,
    peek: &wordpress::SqlDumpPeek,
    _is_in_site_folder: bool,
) -> (Option<bool>, Option<String>) {
    let mut matches: Option<bool> = None;
    let mut reason: Option<String> = None;

    if let Some(insp) = inspection {
        if let (Some(cfg_prefix), Some(dump_prefix)) =
            (insp.wp_table_prefix.as_deref(), peek.prefix.as_deref())
        {
            if cfg_prefix.eq_ignore_ascii_case(dump_prefix) {
                matches = Some(true);
                reason = Some(format!("Table prefix matches wp-config.php ({})", dump_prefix));
            } else {
                return (
                    Some(false),
                    Some(format!(
                        "Table prefix '{}' differs from wp-config.php ('{}')",
                        dump_prefix, cfg_prefix
                    )),
                );
            }
        }

        if matches.is_none() {
            if let (Some(cfg_db), Some(dump_db)) =
                (insp.wp_db_name.as_deref(), peek.db_name.as_deref())
            {
                if cfg_db.eq_ignore_ascii_case(dump_db) {
                    matches = Some(true);
                    reason = Some(format!("Database name matches wp-config.php ({})", dump_db));
                }
            }
        }

        if let (Some(cfg_url), Some(dump_url)) =
            (insp.wp_siteurl.as_deref(), peek.site_url.as_deref())
        {
            let cfg_stem = url_host_stem(cfg_url);
            let dump_stem = url_host_stem(dump_url);
            if !cfg_stem.is_empty() && !dump_stem.is_empty() {
                if cfg_stem == dump_stem {
                    if matches.is_none() {
                        matches = Some(true);
                        reason = Some(format!("Site URL matches config ({})", dump_url));
                    }
                } else if matches.is_none() {
                    return (
                        Some(false),
                        Some(format!(
                            "Dump URL ({}) appears to belong to a different site ({})",
                            dump_url, cfg_url
                        )),
                    );
                }
            }
        }
    }

    if matches.is_none() {
        if let Some(folder_name) = folder_name_hint {
            let folder_stem = url_host_stem(folder_name);
            if folder_stem.len() >= 3 {
                if let Some(dump_url) = peek.site_url.as_deref() {
                    let dump_stem = url_host_stem(dump_url);
                    if !dump_stem.is_empty() && (dump_stem.contains(&folder_stem) || folder_stem.contains(&dump_stem)) {
                        matches = Some(true);
                        reason = Some(format!("Dump URL ({}) matches folder '{}'", dump_url, folder_name));
                    }
                }
                if matches.is_none() {
                    let target_text = format!("{} {}", relative_path, filename)
                        .to_ascii_lowercase()
                        .chars()
                        .filter(|c| c.is_ascii_alphanumeric())
                        .collect::<String>();
                    if target_text.contains(&folder_stem) {
                        matches = Some(true);
                        reason = Some(format!("Filename/path matches '{}'", folder_name));
                    }
                }
            }
        }
    }

    (matches, reason)
}

fn scan_directory_db_files_recursive(
    scan_root: &std::path::Path,
    source_scope: &str,
    exclude_dir: Option<&std::path::Path>,
    folder_name_hint: Option<&str>,
    inspection: Option<&wordpress::FolderInspection>,
    max_depth: usize,
    max_results: usize,
) -> Vec<DetectedDbFile> {
    if !scan_root.is_dir() {
        return Vec::new();
    }

    let exclude_key = exclude_dir.map(normalize_dir_key);
    let source_dir_label = scan_root.to_string_lossy().to_string();
    let is_in_site_folder = source_scope == "folder";
    let mut found = Vec::new();
    let mut queue: std::collections::VecDeque<(PathBuf, String, usize)> =
        std::collections::VecDeque::new();
    queue.push_back((scan_root.to_path_buf(), String::new(), 0));
    let mut visited_dirs = 0usize;

    while let Some((cur_dir, rel_prefix, depth)) = queue.pop_front() {
        visited_dirs += 1;
        if visited_dirs > 600 || found.len() >= max_results {
            break;
        }
        if depth > 0 {
            if let Some(ref ex) = exclude_key {
                if normalize_dir_key(&cur_dir) == *ex {
                    continue;
                }
            }
        }

        let Ok(entries) = std::fs::read_dir(&cur_dir) else {
            continue;
        };

        let mut entry_list: Vec<std::fs::DirEntry> = entries.flatten().collect();
        entry_list.sort_by_key(|e| e.file_name().to_string_lossy().to_ascii_lowercase());

        for entry in entry_list {
            let p = entry.path();
            let name = entry.file_name().to_string_lossy().to_string();
            let lower_name = name.to_ascii_lowercase();

            if p.is_dir() {
                if depth < max_depth && !should_skip_scan_subdir(&lower_name) {
                    if let Some(ref ex) = exclude_key {
                        if normalize_dir_key(&p) == *ex {
                            continue;
                        }
                    }
                    queue.push_back((p, format!("{}{}/", rel_prefix, name), depth + 1));
                }
                continue;
            }

            if !p.is_file() {
                continue;
            }
            let relative_path = format!("{}{}", rel_prefix, name);
            let rel_lower = relative_path.to_ascii_lowercase();
            if !is_supported_db_dump_filename(&lower_name, &rel_lower) {
                continue;
            }

            let size_bytes = entry.metadata().map(|m| m.len()).unwrap_or(0);
            let peek = wordpress::peek_db_file_metadata(&p);
            let (matches_site, match_reason) = evaluate_db_file_match(
                folder_name_hint,
                inspection,
                &name,
                &relative_path,
                &peek,
                is_in_site_folder,
            );

            found.push(DetectedDbFile {
                name,
                relative_path,
                path: p.to_string_lossy().to_string(),
                size_bytes,
                source_scope: source_scope.to_string(),
                source_dir: source_dir_label.clone(),
                detected_prefix: peek.prefix,
                detected_site_url: peek.site_url,
                detected_db_name: peek.db_name,
                matches_site,
                match_reason,
            });
            if found.len() >= max_results {
                break;
            }
        }
    }

    found.sort_by(|a, b| {
        let rank = |item: &DetectedDbFile| -> (u8, usize) {
            let match_rank = match item.matches_site {
                Some(true) => 0,
                None => 1,
                Some(false) => 2,
            };
            let depth_rank = item.relative_path.matches('/').count();
            (match_rank, depth_rank)
        };
        rank(a)
            .cmp(&rank(b))
            .then_with(|| a.relative_path.cmp(&b.relative_path))
    });
    found
}

fn scan_folder_db_files(
    dir: &std::path::Path,
    folder_name_hint: Option<&str>,
    inspection: Option<&wordpress::FolderInspection>,
) -> Vec<DetectedDbFile> {
    scan_directory_db_files_recursive(
        dir,
        "folder",
        None,
        folder_name_hint,
        inspection,
        6,
        40,
    )
}

fn scan_other_directory_db_files(
    root_dir: &std::path::Path,
    exclude_dir: Option<&std::path::Path>,
    folder_name_hint: Option<&str>,
    inspection: Option<&wordpress::FolderInspection>,
) -> Vec<DetectedDbFile> {
    scan_directory_db_files_recursive(
        root_dir,
        "sites_dir",
        exclude_dir,
        folder_name_hint,
        inspection,
        4,
        40,
    )
}

fn discover_unregistered_folders(state: &AppState) -> (PathBuf, Vec<DiscoveredFolderCandidate>, Vec<DetectedDbFile>) {
    let root_dir = state.get_default_sites_dir();
    state.approve_db_scan_dir(&root_dir);
    let sites = state.list_sites();
    let mut registered_dirs = std::collections::HashSet::new();
    let mut taken_domains = std::collections::HashSet::new();

    for site in &sites {
        registered_dirs.insert(normalize_dir_key(&state.site_dir(site)));
        taken_domains.insert(site.domain.trim().to_ascii_lowercase());
        for alias in &site.aliases {
            taken_domains.insert(alias.trim().to_ascii_lowercase());
        }
    }

    let mut out = Vec::new();
    if !root_dir.is_dir() {
        return (root_dir, out, Vec::new());
    }

    let Ok(entries) = std::fs::read_dir(&root_dir) else {
        return (root_dir, out, Vec::new());
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().to_string();
        let lower_name = name.to_ascii_lowercase();
        if name.starts_with('.')
            || matches!(
                lower_name.as_str(),
                "node_modules" | "__pycache__" | "vendor" | "$recycle.bin" | "system volume information"
            )
        {
            continue;
        }

        if registered_dirs.contains(&normalize_dir_key(&path)) {
            continue;
        }

        let inspection = wordpress::inspect_site_folder(&path);
        let suggested_domain = suggest_domain_for_folder(&name, &taken_domains);
        taken_domains.insert(suggested_domain.clone());

        let suggested_runtime = match inspection.detected_type.as_str() {
            "wordpress" | "php" => "php",
            "node" => "node",
            _ => "static",
        }
        .to_string();

        let detected_db_files = scan_folder_db_files(&path, Some(&name), Some(&inspection));

        out.push(DiscoveredFolderCandidate {
            name,
            path: path.to_string_lossy().to_string(),
            suggested_domain,
            suggested_runtime,
            detected_db_files,
            inspection,
        });
    }

    out.sort_by(|a, b| {
        let rank = |c: &DiscoveredFolderCandidate| -> (u8, u8) {
            let empty_rank = if c.inspection.is_empty { 1 } else { 0 };
            let type_rank = match c.inspection.detected_type.as_str() {
                "wordpress" => 0,
                "php" => 1,
                "node" => 2,
                "static" => 3,
                _ => 4,
            };
            (empty_rank, type_rank)
        };
        rank(a)
            .cmp(&rank(b))
            .then_with(|| a.name.to_ascii_lowercase().cmp(&b.name.to_ascii_lowercase()))
    });

    let root_db_files = scan_other_directory_db_files(&root_dir, None, None, None);

    (root_dir, out, root_db_files)
}

async fn desktop_discover_folders_handler(State(state): State<Arc<AppState>>) -> Response {
    let (sites_dir, folders, root_db_files) =
        tokio::task::spawn_blocking(move || discover_unregistered_folders(&state))
            .await
            .unwrap_or_else(|_| (PathBuf::new(), Vec::new(), Vec::new()));
    Json(serde_json::json!({
        "sites_dir": sites_dir.to_string_lossy().to_string(),
        "folders": folders,
        "root_db_files": root_db_files,
    }))
    .into_response()
}

async fn desktop_inspect_folder_handler(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<serde_json::Value>,
) -> Response {
    let raw = payload
        .get("path")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim()
        .to_string();
    if raw.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": "path is required" })),
        )
            .into_response();
    }
    let st_clone = state.clone();
    let result = tokio::task::spawn_blocking(move || {
        let target = std::path::PathBuf::from(&raw);
        st_clone.approve_db_scan_dir(&target);
        let inspection = wordpress::inspect_site_folder(&target);
        let name_hint = target
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();
        let detected_db_files = scan_folder_db_files(&target, Some(&name_hint), Some(&inspection));
        let root_dir = st_clone.get_default_sites_dir();
        st_clone.approve_db_scan_dir(&root_dir);
        let other_directory_db_files = scan_other_directory_db_files(
            &root_dir,
            Some(&target),
            Some(&name_hint),
            Some(&inspection),
        );
        let mut val = serde_json::to_value(&inspection).unwrap_or_default();
        if let Some(obj) = val.as_object_mut() {
            obj.insert(
                "detected_db_files".to_string(),
                serde_json::to_value(&detected_db_files).unwrap_or_default(),
            );
            obj.insert(
                "other_directory_db_files".to_string(),
                serde_json::to_value(&other_directory_db_files).unwrap_or_default(),
            );
        }
        val
    })
    .await
    .unwrap_or_else(|_| serde_json::json!({ "error": "Inspection failed" }));
    Json(result).into_response()
}

async fn desktop_scan_db_files_handler(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<Option<serde_json::Value>>,
) -> Response {
    let payload_val = payload.unwrap_or_else(|| serde_json::json!({}));
    let pick = payload_val
        .get("pick")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let requested_dir = payload_val
        .get("dir")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(String::from);
    let site_id = payload_val
        .get("site_id")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(String::from);
    let requested_site_dir = payload_val
        .get("site_dir")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(String::from);

    let site_dir_path = site_id
        .as_deref()
        .and_then(|id| state.get_site(id))
        .map(|s| state.site_dir(&s))
        .or_else(|| requested_site_dir.map(PathBuf::from));

    let root_sites_dir = state.get_default_sites_dir();
    let initial_dir = requested_dir
        .clone()
        .or_else(|| {
            site_dir_path
                .as_ref()
                .map(|p| p.to_string_lossy().to_string())
        })
        .unwrap_or_else(|| root_sites_dir.to_string_lossy().to_string());

    let scan_dir_str = if pick {
        match pick_windows_folder_blocking(
            initial_dir,
            "Select directory to scan (including subdirectories) for .sql / database dumps"
                .to_string(),
        )
        .await
        {
            Ok(Some(chosen)) => chosen,
            Ok(None) => return Json(serde_json::json!({ "cancelled": true })).into_response(),
            Err(e) => {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(serde_json::json!({ "error": e })),
                )
                    .into_response();
            }
        }
    } else {
        initial_dir
    };

    let target_dir = PathBuf::from(&scan_dir_str);
    if !target_dir.is_dir() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "error": format!("Directory '{}' does not exist", scan_dir_str)
            })),
        )
            .into_response();
    }

    let st_clone = state.clone();
    let res = tokio::task::spawn_blocking(move || {
        st_clone.approve_db_scan_dir(&target_dir);
        let root_dir = st_clone.get_default_sites_dir();
        st_clone.approve_db_scan_dir(&root_dir);

        let (inspection, folder_name_hint) = if let Some(ref sd) = site_dir_path {
            let insp = wordpress::inspect_site_folder(sd);
            let name = sd
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_default();
            (Some(insp), Some(name))
        } else {
            let insp = wordpress::inspect_site_folder(&target_dir);
            let name = target_dir
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_default();
            (
                if insp.is_wordpress || !insp.is_empty {
                    Some(insp)
                } else {
                    None
                },
                Some(name),
            )
        };

        let is_same_as_site = site_dir_path
            .as_ref()
            .is_some_and(|sd| normalize_dir_key(sd) == normalize_dir_key(&target_dir));
        let scope = if is_same_as_site { "folder" } else { "directory" };
        let files = scan_directory_db_files_recursive(
            &target_dir,
            scope,
            None,
            folder_name_hint.as_deref(),
            inspection.as_ref(),
            6,
            60,
        );
        let sites_dir_files = scan_other_directory_db_files(
            &root_dir,
            Some(&target_dir),
            folder_name_hint.as_deref(),
            inspection.as_ref(),
        );

        serde_json::json!({
            "cancelled": false,
            "dir": target_dir.to_string_lossy().to_string(),
            "sites_dir": root_dir.to_string_lossy().to_string(),
            "files": files,
            "sites_dir_files": sites_dir_files,
        })
    })
    .await
    .unwrap_or_else(|e| serde_json::json!({ "error": e.to_string() }));

    Json(res).into_response()
}

fn read_uploaded_or_local_db_bytes(
    state: &AppState,
    site: &state::LocalSite,
    payload: &serde_json::Value,
) -> Result<(String, Vec<u8>), (StatusCode, String)> {
    use base64::{engine::general_purpose::STANDARD as B64, Engine};

    let local_path = payload
        .get("local_path")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty());

    let mut filename = payload
        .get("filename")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim()
        .to_string();

    if let Some(lp) = local_path {
        let p = PathBuf::from(lp);
        if !p.is_file() {
            return Err((StatusCode::BAD_REQUEST, "Selected local database file not found".to_string()));
        }
        let canonical = std::fs::canonicalize(&p).map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;
        if !state.is_approved_db_path(site, &canonical) {
            return Err((
                StatusCode::FORBIDDEN,
                "Local database path must reside inside the site, default websites directory, or a scanned directory".to_string(),
            ));
        }
        if filename.is_empty() {
            filename = p
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| "database.sql".to_string());
        }
        let bytes = std::fs::read(&canonical)
            .map_err(|e| (StatusCode::BAD_REQUEST, format!("Failed reading local database file: {}", e)))?;
        return Ok((filename, bytes));
    }

    if filename.is_empty() {
        filename = "database.sql".to_string();
    }
    let b64_raw = payload
        .get("content_base64")
        .or_else(|| payload.get("content"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim();

    if b64_raw.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            "Database file content is required".to_string(),
        ));
    }

    let cleaned_b64: String = b64_raw.chars().filter(|c| !c.is_whitespace()).collect();
    let raw_bytes = B64.decode(&cleaned_b64).map_err(|e| {
        (
            StatusCode::BAD_REQUEST,
            format!("Invalid base64 database payload: {}", e),
        )
    })?;
    Ok((filename, raw_bytes))
}

fn upsert_site_env_kv(site_dir: &std::path::Path, pairs: &[(&str, &str)]) {
    let env_path = site_dir.join(".env");
    if !env_path.is_file() {
        return;
    }
    let Ok(raw) = std::fs::read_to_string(&env_path) else {
        return;
    };
    let mut lines: Vec<String> = raw.lines().map(|l| l.to_string()).collect();
    for (k, v) in pairs {
        let mut updated = false;
        for line in &mut lines {
            let trimmed = line.trim_start();
            if !trimmed.starts_with('#') && trimmed.starts_with(&format!("{}=", k)) {
                *line = format!("{}={}", k, v);
                updated = true;
                break;
            }
        }
        if !updated {
            lines.push(format!("{}={}", k, v));
        }
    }
    let _ = std::fs::write(env_path, lines.join("\n") + "\n");
}

async fn wordpress_import_database_handler(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(payload): Json<serde_json::Value>,
) -> Response {
    let Some(site) = state.get_site(&id) else {
        return (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": "Site not found" })),
        )
            .into_response();
    };

    let (filename, raw_bytes) = match read_uploaded_or_local_db_bytes(&state, &site, &payload) {
        Ok(v) => v,
        Err((status, msg)) => return (status, Json(serde_json::json!({ "error": msg }))).into_response(),
    };

    let lower_fn = filename.to_ascii_lowercase();
    let looks_sqlite = lower_fn.ends_with(".sqlite")
        || lower_fn.ends_with(".db")
        || raw_bytes.starts_with(b"SQLite format 3\0");

    let active_port = state.get_router_config().active_port;
    let ssl_port = state.get_ssl_port();
    let default_target_url = if site.ssl_enabled {
        match ssl_port {
            Some(443) => format!("https://{}", site.domain),
            Some(p) => format!("https://{}:{}", site.domain, p),
            None if active_port == 80 => format!("http://{}", site.domain),
            None => format!("http://{}:{}", site.domain, active_port),
        }
    } else if active_port == 80 {
        format!("http://{}", site.domain)
    } else {
        format!("http://{}:{}", site.domain, active_port)
    };

    let target_url = payload
        .get("target_url")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(String::from)
        .unwrap_or(default_target_url);

    let source_url_override = payload
        .get("source_url")
        .and_then(|v| v.as_str())
        .map(String::from);

    let memory_limit_mb = payload
        .get("memory_limit_mb")
        .and_then(|v| v.as_i64())
        .map(|n| n as i32);

    let rollback_on_failure = payload
        .get("rollback_on_failure")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    let apply_compat_fixes = payload
        .get("apply_compat_fixes")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    let stream = payload
        .get("stream")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    let site_dir = state.site_dir(&site);
    let domain = site.domain.clone();

    if stream {
        let job_id = format!("wp-import-{}", uuid::Uuid::new_v4());
        let sse_url = format!("/api/sites/{}/wordpress/import-log/{}", site.id, job_id);

        state.append_install_progress(
            &job_id,
            "site_connect",
            &format!("Connected existing WordPress folder for {}", site.domain),
            "done",
            Some(100),
            Some(&format!(
                "Folder: {} | Target URL: {}",
                site_dir.display(),
                target_url
            )),
            false,
        );
        state.append_install_progress(
            &job_id,
            "upload",
            &format!(
                "Uploaded {} ({})",
                filename,
                downloads::format_mb(raw_bytes.len() as u64)
            ),
            "done",
            Some(100),
            Some(&format!(
                "Server received {} bytes for '{}'",
                raw_bytes.len(),
                filename
            )),
            false,
        );
        state.append_install_progress(
            &job_id,
            "unpack",
            &format!("Preparing to process {}...", filename),
            "in_progress",
            Some(5),
            None,
            false,
        );

        let bg_state = state.clone();
        let bg_site_id = site.id.clone();
        let bg_domain = site.domain.clone();
        let bg_php = site.php_version.clone();
        let bg_job_id = job_id.clone();
        let bg_filename = filename.clone();
        let bg_target_url = target_url.clone();
        let bg_source_override = source_url_override.clone();

        tokio::spawn(async move {
            if !looks_sqlite && bg_state.get_mariadb_port().is_none() {
                bg_state.append_install_progress(
                    &bg_job_id,
                    "mariadb",
                    "Checking local MariaDB server...",
                    "in_progress",
                    Some(5),
                    Some("MariaDB is not running; preparing local database engine..."),
                    false,
                );
                if mariadb::find_mariadbd_binary().is_none() {
                    if let Err(e) = mariadb::install_mariadb_with_progress(|_, lbl, pct, msg| {
                        bg_state.append_install_progress(
                            &bg_job_id,
                            "mariadb",
                            lbl,
                            "in_progress",
                            pct,
                            msg.as_deref().or(Some(lbl)),
                            false,
                        );
                    })
                    .await
                    {
                        bg_state.append_install_progress(
                            &bg_job_id,
                            "mariadb",
                            "MariaDB auto-install failed",
                            "error",
                            None,
                            Some(&e),
                            false,
                        );
                        if rollback_on_failure {
                            bg_state.unregister_site_keep_files(&bg_site_id);
                        }
                        bg_state.append_install_progress(
                            &bg_job_id,
                            "complete",
                            &format!("MariaDB auto-install failed: {}", e),
                            "error",
                            None,
                            Some(&e),
                            true,
                        );
                        return;
                    }
                }
                bg_state.append_install_progress(
                    &bg_job_id,
                    "mariadb",
                    "Starting local MariaDB server...",
                    "in_progress",
                    Some(90),
                    Some("Launching mariadbd.exe on loopback port..."),
                    false,
                );
                let start_res = tokio::task::spawn_blocking({
                    let st = bg_state.clone();
                    move || st.start_mariadb()
                })
                .await;
                match start_res {
                    Ok(Ok(port)) => {
                        bg_state.append_install_progress(
                            &bg_job_id,
                            "mariadb",
                            &format!("MariaDB ready on 127.0.0.1:{}", port),
                            "done",
                            Some(100),
                            Some(&format!("MariaDB listening on 127.0.0.1:{}", port)),
                            false,
                        );
                    }
                    Ok(Err(e)) => {
                        bg_state.append_install_progress(
                            &bg_job_id,
                            "mariadb",
                            "Failed starting MariaDB",
                            "error",
                            None,
                            Some(&e),
                            false,
                        );
                        if rollback_on_failure {
                            bg_state.unregister_site_keep_files(&bg_site_id);
                        }
                        bg_state.append_install_progress(
                            &bg_job_id,
                            "complete",
                            &format!("Failed starting MariaDB: {}", e),
                            "error",
                            None,
                            Some(&e),
                            true,
                        );
                        return;
                    }
                    Err(e) => {
                        let msg = e.to_string();
                        if rollback_on_failure {
                            bg_state.unregister_site_keep_files(&bg_site_id);
                        }
                        bg_state.append_install_progress(
                            &bg_job_id,
                            "complete",
                            &format!("MariaDB task error: {}", msg),
                            "error",
                            None,
                            Some(&msg),
                            true,
                        );
                        return;
                    }
                }
            } else if !looks_sqlite {
                if let Some(p) = bg_state.get_mariadb_port() {
                    bg_state.append_install_progress(
                        &bg_job_id,
                        "mariadb",
                        &format!("Using running MariaDB on 127.0.0.1:{}", p),
                        "done",
                        Some(100),
                        Some(&format!("Detected active MariaDB on 127.0.0.1:{}", p)),
                        false,
                    );
                }
            }

            let mdb_port = bg_state.get_mariadb_port();
            let st_cb = bg_state.clone();
            let jid_cb = bg_job_id.clone();
            let dom_cb = bg_domain.clone();
            let sd_cb = site_dir.clone();
            let fn_cb = bg_filename.clone();
            let tgt_cb = bg_target_url.clone();
            let src_cb = bg_source_override.clone();

            let res = tokio::task::spawn_blocking(move || {
                wordpress::import_existing_wp_database_with_compat(
                    &sd_cb,
                    &dom_cb,
                    &tgt_cb,
                    mdb_port,
                    &fn_cb,
                    &raw_bytes,
                    src_cb.as_deref(),
                    memory_limit_mb,
                    apply_compat_fixes,
                    |step, lbl, status, pct, log_line| {
                        st_cb.append_install_progress(
                            &jid_cb,
                            step,
                            lbl,
                            status,
                            pct,
                            log_line,
                            false,
                        );
                    },
                )
            })
            .await;

            let rollback_site_msg = |state_ref: &AppState, sid: &str, dom: &str| -> String {
                if rollback_on_failure {
                    if !looks_sqlite {
                        if let Some(p) = state_ref.get_mariadb_port() {
                            let partial_db = format!("{}_wp", dom.replace('.', "_").replace('-', "_"));
                            let _ = mariadb::drop_database(p, &partial_db);
                        }
                    }
                    if state_ref.unregister_site_keep_files(sid) {
                        return format!(
                            "\n\nRolled back temporary site '{}' (website files preserved on disk) — it has been returned to \"Unregistered Folders Ready to Import\" so you can adjust settings and retry.",
                            dom
                        );
                    }
                }
                String::new()
            };

            match res {
                Ok(Ok(val)) => {
                    bg_state.append_install_progress(
                        &bg_job_id,
                        "ssl_start",
                        &format!("Starting PHP {} worker & local HTTPS router...", bg_php),
                        "in_progress",
                        Some(50),
                        Some(&format!("Provisioning SSL certificate & starting site '{}'...", bg_domain)),
                        false,
                    );
                    bg_state.update_site_preset(&bg_site_id, "wordpress");
                    let ssl_ok = bg_state.ensure_site_ssl(&bg_domain).is_ok();
                    let start_ok = bg_state.start_site(&bg_site_id).is_ok();
                    let db_name = val
                        .get("db_name")
                        .and_then(|v| v.as_str())
                        .unwrap_or("database");
                    let db_engine = val
                        .get("db_engine")
                        .and_then(|v| v.as_str())
                        .unwrap_or("mariadb");
                    let replacements = val
                        .get("replacements")
                        .and_then(|v| v.as_u64())
                        .unwrap_or(0);

                    bg_state.append_install_progress(
                        &bg_job_id,
                        "ssl_start",
                        &format!("Site live at {} (PHP {}, {})", bg_target_url, bg_php, db_engine),
                        "done",
                        Some(100),
                        Some(&format!(
                            "SSL={} | Worker started={} | Live URL={}",
                            ssl_ok, start_ok, bg_target_url
                        )),
                        false,
                    );
                    bg_state.record_event(
                        "wp_import_db",
                        "site",
                        &bg_domain,
                        "WordPress Database Imported",
                        &format!(
                            "Connected {} to {} and updated wp-config.php",
                            bg_domain, db_name
                        ),
                        "success",
                    );
                    bg_state.append_install_progress(
                        &bg_job_id,
                        "complete",
                        &format!(
                            "Connected {} ({}) — {} URL references updated",
                            bg_domain, db_name, replacements
                        ),
                        "done",
                        Some(100),
                        Some(&format!(
                            "Import complete | Engine: {} | Database: {} | URL replacements: {} | Live at {}",
                            db_engine, db_name, replacements, bg_target_url
                        )),
                        true,
                    );
                }
                Ok(Err(imp_err)) => {
                    let rb_note = rollback_site_msg(&bg_state, &bg_site_id, &bg_domain);
                    let err_msg = imp_err.message().to_string();
                    let full_err = format!("{}{}", err_msg, rb_note);
                    bg_state.record_event(
                        "wp_import_db_error",
                        "site",
                        &bg_domain,
                        "WordPress Database Import Failed",
                        &full_err,
                        "critical",
                    );
                    let extra_meta = match &imp_err {
                        mariadb::SqlImportError::CompatConfirm(c) => Some(serde_json::json!({
                            "requires_compat_confirm": true,
                            "compat_fixes": c.fixes,
                            "target_version": c.target_version,
                            "error_detail": c.error,
                        })),
                        mariadb::SqlImportError::Fatal(_) => None,
                    };
                    bg_state.push_install_progress_with_meta(
                        &bg_job_id,
                        "complete",
                        &format!("Database import failed: {}", err_msg),
                        "error",
                        None,
                        Some(&full_err),
                        extra_meta,
                        true,
                    );
                }
                Err(e) => {
                    let rb_note = rollback_site_msg(&bg_state, &bg_site_id, &bg_domain);
                    let msg = format!("{}{}", e, rb_note);
                    bg_state.append_install_progress(
                        &bg_job_id,
                        "complete",
                        &format!("Database import worker error: {}", e),
                        "error",
                        None,
                        Some(&msg),
                        true,
                    );
                }
            }
        });

        return Json(serde_json::json!({
            "ok": true,
            "streaming": true,
            "job_id": job_id,
            "sse_url": sse_url,
            "target_url": target_url,
        }))
        .into_response();
    }

    // Auto-install and start MariaDB when importing a MySQL/SQL dump
    if !looks_sqlite && state.get_mariadb_port().is_none() {
        if mariadb::find_mariadbd_binary().is_none() {
            if let Err(e) = mariadb::install_mariadb_with_progress(|_, _, _, _| {}).await {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(serde_json::json!({ "error": format!("MariaDB auto-install failed: {}", e) })),
                )
                    .into_response();
            }
        }
        let start_res = tokio::task::spawn_blocking({
            let st = state.clone();
            move || st.start_mariadb()
        })
        .await;
        match start_res {
            Ok(Ok(_)) => {}
            Ok(Err(e)) => {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(serde_json::json!({ "error": format!("Failed starting MariaDB: {}", e) })),
                )
                    .into_response();
            }
            Err(e) => {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(serde_json::json!({ "error": e.to_string() })),
                )
                    .into_response();
            }
        }
    }

    let mdb_port = state.get_mariadb_port();
    let rb_site_id = site.id.clone();

    let res = tokio::task::spawn_blocking(move || {
        wordpress::import_existing_wp_database_with_compat(
            &site_dir,
            &domain,
            &target_url,
            mdb_port,
            &filename,
            &raw_bytes,
            source_url_override.as_deref(),
            memory_limit_mb,
            apply_compat_fixes,
            |_, _, _, _, _| {},
        )
    })
    .await;

    match res {
        Ok(Ok(val)) => {
            state.update_site_preset(&site.id, "wordpress");
            let _ = state.start_site(&site.id);
            state.record_event(
                "wp_import_db",
                "site",
                &site.domain,
                "WordPress Database Imported",
                &format!(
                    "Connected {} to {} and updated wp-config.php",
                    site.domain,
                    val.get("db_name").and_then(|v| v.as_str()).unwrap_or("database")
                ),
                "success",
            );
            Json(val).into_response()
        }
        Ok(Err(imp_err)) => {
            if rollback_on_failure {
                if !looks_sqlite {
                    if let Some(p) = state.get_mariadb_port() {
                        let partial_db = format!("{}_wp", site.domain.replace('.', "_").replace('-', "_"));
                        let _ = mariadb::drop_database(p, &partial_db);
                    }
                }
                state.unregister_site_keep_files(&rb_site_id);
            }
            match imp_err {
                mariadb::SqlImportError::CompatConfirm(c) => (
                    StatusCode::CONFLICT,
                    Json(serde_json::json!({
                        "error": c.error,
                        "requires_compat_confirm": true,
                        "compat_fixes": c.fixes,
                        "target_version": c.target_version,
                        "rolled_back": rollback_on_failure,
                    })),
                )
                    .into_response(),
                mariadb::SqlImportError::Fatal(e) => (
                    StatusCode::BAD_REQUEST,
                    Json(serde_json::json!({ "error": e, "rolled_back": rollback_on_failure })),
                )
                    .into_response(),
            }
        }
        Err(e) => {
            if rollback_on_failure {
                state.unregister_site_keep_files(&rb_site_id);
            }
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": e.to_string(), "rolled_back": rollback_on_failure })),
            )
                .into_response()
        }
    }
}

async fn site_import_database_handler(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(payload): Json<serde_json::Value>,
) -> Response {
    let Some(site) = state.get_site(&id) else {
        return (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": "Site not found" })),
        )
            .into_response();
    };

    let (filename, raw_bytes) = match read_uploaded_or_local_db_bytes(&state, &site, &payload) {
        Ok(v) => v,
        Err((status, msg)) => return (status, Json(serde_json::json!({ "error": msg }))).into_response(),
    };

    let requested_name = payload
        .get("db_name")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(String::from)
        .unwrap_or_else(|| site.domain.clone());

    let memory_limit_mb = payload
        .get("memory_limit_mb")
        .and_then(|v| v.as_i64())
        .map(|n| n as i32);

    let rollback_on_failure = payload
        .get("rollback_on_failure")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    let apply_compat_fixes = payload
        .get("apply_compat_fixes")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    let stream = payload.get("stream").and_then(|v| v.as_bool()).unwrap_or(false);
    let site_dir = state.site_dir(&site);
    let domain = site.domain.clone();
    let site_id = site.id.clone();
    let rollback_id = site.id.clone();

    let (is_sqlite, data_bytes, unpack_msg) = match wordpress::unpack_uploaded_db_bytes(&filename, &raw_bytes) {
        Ok(v) => v,
        Err(e) => {
            if rollback_on_failure {
                state.unregister_site_keep_files(&rollback_id);
            }
            return (StatusCode::BAD_REQUEST, Json(serde_json::json!({ "error": e }))).into_response();
        }
    };

    let import_work = move |st: Arc<AppState>, job_opt: Option<String>| -> Result<(String, String, Option<u16>), mariadb::SqlImportError> {
        let log = |step: &str, lbl: &str, status: &str, pct: Option<u8>, msg: Option<&str>| {
            if let Some(ref jid) = job_opt {
                st.append_install_progress(jid, step, lbl, status, pct, msg, false);
            }
        };

        log("unpack", &unpack_msg, "done", Some(35), Some(&unpack_msg));
        let normalized_req = requested_name.replace(['.', '-'], "_");
        let clean_name = databases::sanitize_db_name(&normalized_req).map_err(mariadb::SqlImportError::Fatal)?;

        if is_sqlite {
            let db_dir = site_dir.join("database");
            let _ = std::fs::create_dir_all(&db_dir);
            let target_sqlite = if site_dir.join(".env").is_file() {
                db_dir.join("database.sqlite")
            } else {
                db_dir.join(format!("{}.sqlite", clean_name))
            };
            std::fs::write(&target_sqlite, &data_bytes)
                .map_err(|e| mariadb::SqlImportError::Fatal(format!("Failed saving SQLite database: {}", e)))?;
            upsert_site_env_kv(
                &site_dir,
                &[
                    ("DB_CONNECTION", "sqlite"),
                    ("DB_DATABASE", &target_sqlite.to_string_lossy()),
                ],
            );
            let _ = st.start_site(&site_id);
            log(
                "db_import",
                &format!("Imported SQLite database to {}", target_sqlite.display()),
                "done",
                Some(90),
                Some(&format!("Saved SQLite file at {}", target_sqlite.display())),
            );
            return Ok((clean_name, "sqlite".to_string(), None));
        }

        let port = st
            .get_mariadb_port()
            .ok_or_else(|| mariadb::SqlImportError::Fatal("MariaDB daemon is not running".to_string()))?;
        log("db_create", &format!("Preparing clean MariaDB database '{}'", clean_name), "in_progress", Some(50), None);
        let actual_name = mariadb::reset_database(port, &clean_name).map_err(mariadb::SqlImportError::Fatal)?;

        let dump_path = std::env::temp_dir().join(format!("dockpanel-site-db-{}.sql", uuid::Uuid::new_v4()));
        std::fs::write(&dump_path, &data_bytes)
            .map_err(|e| mariadb::SqlImportError::Fatal(format!("Failed staging SQL dump: {}", e)))?;
        let imp_res = mariadb::import_mysql_dump_with_compat(
            port,
            &actual_name,
            &dump_path,
            memory_limit_mb,
            apply_compat_fixes,
            |_, _, _| {},
        );
        let _ = std::fs::remove_file(&dump_path);
        if let Err(e) = imp_res {
            if rollback_on_failure {
                let _ = mariadb::drop_database(port, &actual_name);
            }
            return Err(e);
        }

        let port_str = port.to_string();
        upsert_site_env_kv(
            &site_dir,
            &[
                ("DB_CONNECTION", "mysql"),
                ("DB_HOST", "127.0.0.1"),
                ("DB_PORT", &port_str),
                ("DB_DATABASE", &actual_name),
                ("DB_USERNAME", "root"),
                ("DB_PASSWORD", ""),
            ],
        );
        let _ = st.start_site(&site_id);
        log(
            "db_import",
            &format!("Imported SQL dump into MariaDB database '{}'", actual_name),
            "done",
            Some(95),
            Some(&format!("Engine: MariaDB 127.0.0.1:{} | Database: {}", port, actual_name)),
        );
        Ok((actual_name, "mariadb".to_string(), Some(port)))
    };

    if stream {
        let job_id = format!("site-db-import-{}", uuid::Uuid::new_v4());
        let sse_url = format!("/api/sites/{}/wordpress/import-log/{}", site.id, job_id);
        state.append_install_progress(
            &job_id,
            "site_connect",
            &format!("Connected site folder for {}", domain),
            "done",
            Some(100),
            Some(&format!("Preparing database import for '{}'...", domain)),
            false,
        );

        let bg_state = state.clone();
        let bg_job = job_id.clone();
        let bg_domain = domain.clone();
        let bg_site_id = rollback_id.clone();
        tokio::spawn(async move {
            let rollback_note = || {
                if rollback_on_failure && bg_state.unregister_site_keep_files(&bg_site_id) {
                    format!(
                        "\n\nRolled back temporary site '{}' (website files preserved on disk) — returned to \"Unregistered Folders Ready to Import\".",
                        bg_domain
                    )
                } else {
                    String::new()
                }
            };
            if !is_sqlite && bg_state.get_mariadb_port().is_none() {
                if mariadb::find_mariadbd_binary().is_none() {
                    if let Err(e) = mariadb::install_mariadb_with_progress(|_, lbl, pct, msg| {
                        bg_state.append_install_progress(&bg_job, "mariadb", lbl, "in_progress", pct, msg.as_deref(), false);
                    })
                    .await
                    {
                        let full = format!("{}{}", e, rollback_note());
                        bg_state.append_install_progress(&bg_job, "complete", &format!("MariaDB install failed: {}", e), "error", None, Some(&full), true);
                        return;
                    }
                }
                let st_start = bg_state.clone();
                if let Err(e) = tokio::task::spawn_blocking(move || st_start.start_mariadb())
                    .await
                    .map_err(|e| e.to_string())
                    .and_then(|r| r.map(|_| ()))
                {
                    let full = format!("{}{}", e, rollback_note());
                    bg_state.append_install_progress(&bg_job, "complete", &format!("Failed starting MariaDB: {}", e), "error", None, Some(&full), true);
                    return;
                }
            }

            let st_work = bg_state.clone();
            let job_work = bg_job.clone();
            let res = tokio::task::spawn_blocking(move || import_work(st_work, Some(job_work))).await;
            match res {
                Ok(Ok((db_name, engine, port))) => {
                    bg_state.record_event(
                        "import_database",
                        "site",
                        &bg_domain,
                        "Site Database Imported",
                        &format!("Imported {} database '{}' for {}", engine, db_name, bg_domain),
                        "success",
                    );
                    bg_state.append_install_progress(
                        &bg_job,
                        "complete",
                        &format!("Imported '{}' ({}) for {}", db_name, engine, bg_domain),
                        "done",
                        Some(100),
                        Some(&format!(
                            "Database: {} | Engine: {}{}",
                            db_name,
                            engine,
                            port.map(|p| format!(" (127.0.0.1:{})", p)).unwrap_or_default()
                        )),
                        true,
                    );
                }
                Ok(Err(imp_err)) => {
                    let err_msg = imp_err.message().to_string();
                    let full = format!("{}{}", err_msg, rollback_note());
                    bg_state.record_event(
                        "import_database_error",
                        "site",
                        &bg_domain,
                        "Database Import Failed",
                        &full,
                        "critical",
                    );
                    let extra_meta = match &imp_err {
                        mariadb::SqlImportError::CompatConfirm(c) => Some(serde_json::json!({
                            "requires_compat_confirm": true,
                            "compat_fixes": c.fixes,
                            "target_version": c.target_version,
                            "error_detail": c.error,
                        })),
                        mariadb::SqlImportError::Fatal(_) => None,
                    };
                    bg_state.push_install_progress_with_meta(
                        &bg_job,
                        "complete",
                        &format!("Database import failed: {}", err_msg),
                        "error",
                        None,
                        Some(&full),
                        extra_meta,
                        true,
                    );
                }
                Err(e) => {
                    let msg = format!("{}{}", e, rollback_note());
                    bg_state.append_install_progress(&bg_job, "complete", &format!("Worker error: {}", e), "error", None, Some(&msg), true);
                }
            }
        });

        return Json(serde_json::json!({
            "ok": true,
            "streaming": true,
            "job_id": job_id,
            "sse_url": sse_url,
        }))
        .into_response();
    }

    if !is_sqlite && state.get_mariadb_port().is_none() {
        if mariadb::find_mariadbd_binary().is_none() {
            if let Err(e) = mariadb::install_mariadb_with_progress(|_, _, _, _| {}).await {
                return (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({ "error": e }))).into_response();
            }
        }
        let st_start = state.clone();
        if let Err(e) = tokio::task::spawn_blocking(move || st_start.start_mariadb())
            .await
            .map_err(|e| e.to_string())
            .and_then(|r| r.map(|_| ()))
        {
            return (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({ "error": e }))).into_response();
        }
    }

    let st_work = state.clone();
    match tokio::task::spawn_blocking(move || import_work(st_work, None)).await {
        Ok(Ok((db_name, engine, port))) => {
            state.record_event(
                "import_database",
                "site",
                &domain,
                "Site Database Imported",
                &format!("Imported {} database '{}' for {}", engine, db_name, domain),
                "success",
            );
            Json(serde_json::json!({
                "ok": true,
                "db_name": db_name,
                "engine": engine,
                "port": port,
            }))
            .into_response()
        }
        Ok(Err(imp_err)) => {
            if rollback_on_failure {
                state.unregister_site_keep_files(&rollback_id);
            }
            match imp_err {
                mariadb::SqlImportError::CompatConfirm(c) => (
                    StatusCode::CONFLICT,
                    Json(serde_json::json!({
                        "error": c.error,
                        "requires_compat_confirm": true,
                        "compat_fixes": c.fixes,
                        "target_version": c.target_version,
                        "rolled_back": rollback_on_failure,
                    })),
                )
                    .into_response(),
                mariadb::SqlImportError::Fatal(e) => (
                    StatusCode::BAD_REQUEST,
                    Json(serde_json::json!({ "error": e, "rolled_back": rollback_on_failure })),
                )
                    .into_response(),
            }
        }
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({ "error": e.to_string() }))).into_response(),
    }
}

async fn site_git_status_handler(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Response {
    let Some(site) = state.get_site(&id) else {
        return (StatusCode::NOT_FOUND, Json(serde_json::json!({ "error": "Site not found" }))).into_response();
    };
    let site_dir = state.site_dir(&site);
    let st = tokio::task::spawn_blocking(move || git_repo::get_status(&site_dir))
        .await
        .unwrap_or_else(|e| git_repo::GitRepoStatus {
            git_available: false,
            git_version: None,
            initialized: false,
            doc_root: String::new(),
            branch: None,
            branches: Vec::new(),
            remote_name: None,
            remote_url: None,
            clean: true,
            changed_files: 0,
            recent_commits: Vec::new(),
            has_gitignore: false,
            error: Some(e.to_string()),
        });
    Json(st).into_response()
}

async fn site_git_init_handler(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(req): Json<git_repo::GitInitRequest>,
) -> Response {
    let Some(site) = state.get_site(&id) else {
        return (StatusCode::NOT_FOUND, Json(serde_json::json!({ "error": "Site not found" }))).into_response();
    };
    let site_dir = state.site_dir(&site);
    let dom = site.domain.clone();
    match tokio::task::spawn_blocking(move || git_repo::init_repo(&site_dir, &req)).await {
        Ok(Ok((status, output))) => {
            state.record_event("git_init", "site", &dom, "Initialized Git Repository", &output, "success");
            Json(serde_json::json!({ "ok": true, "status": status, "output": output })).into_response()
        }
        Ok(Err(e)) => (StatusCode::BAD_REQUEST, Json(serde_json::json!({ "error": e }))).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({ "error": e.to_string() }))).into_response(),
    }
}

async fn site_git_connect_handler(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(req): Json<git_repo::GitConnectRequest>,
) -> Response {
    let Some(site) = state.get_site(&id) else {
        return (StatusCode::NOT_FOUND, Json(serde_json::json!({ "error": "Site not found" }))).into_response();
    };
    let site_dir = state.site_dir(&site);
    let dom = site.domain.clone();
    match tokio::task::spawn_blocking(move || git_repo::connect_repo(&site_dir, &req)).await {
        Ok(Ok((status, output))) => {
            state.record_event("git_connect", "site", &dom, "Connected Git Repository", &output, "success");
            Json(serde_json::json!({ "ok": true, "status": status, "output": output })).into_response()
        }
        Ok(Err(e)) => (StatusCode::BAD_REQUEST, Json(serde_json::json!({ "error": e }))).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({ "error": e.to_string() }))).into_response(),
    }
}

async fn site_git_pull_handler(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(req): Json<git_repo::GitPullRequest>,
) -> Response {
    let Some(site) = state.get_site(&id) else {
        return (StatusCode::NOT_FOUND, Json(serde_json::json!({ "error": "Site not found" }))).into_response();
    };
    let site_dir = state.site_dir(&site);
    let dom = site.domain.clone();
    match tokio::task::spawn_blocking(move || git_repo::pull_repo(&site_dir, &req)).await {
        Ok(Ok((status, output))) => {
            state.record_event("git_pull", "site", &dom, "Git Pull Completed", &output, "success");
            Json(serde_json::json!({ "ok": true, "status": status, "output": output })).into_response()
        }
        Ok(Err(e)) => (StatusCode::BAD_REQUEST, Json(serde_json::json!({ "error": e }))).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({ "error": e.to_string() }))).into_response(),
    }
}

async fn site_git_commit_push_handler(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(req): Json<git_repo::GitCommitPushRequest>,
) -> Response {
    let Some(site) = state.get_site(&id) else {
        return (StatusCode::NOT_FOUND, Json(serde_json::json!({ "error": "Site not found" }))).into_response();
    };
    let site_dir = state.site_dir(&site);
    let dom = site.domain.clone();
    match tokio::task::spawn_blocking(move || git_repo::commit_and_push(&site_dir, &req)).await {
        Ok(Ok((status, output))) => {
            state.record_event("git_commit_push", "site", &dom, "Git Commit/Push Completed", &output, "success");
            Json(serde_json::json!({ "ok": true, "status": status, "output": output })).into_response()
        }
        Ok(Err(e)) => (StatusCode::BAD_REQUEST, Json(serde_json::json!({ "error": e }))).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({ "error": e.to_string() }))).into_response(),
    }
}

async fn site_git_disconnect_handler(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Response {
    let Some(site) = state.get_site(&id) else {
        return (StatusCode::NOT_FOUND, Json(serde_json::json!({ "error": "Site not found" }))).into_response();
    };
    let site_dir = state.site_dir(&site);
    let dom = site.domain.clone();
    match tokio::task::spawn_blocking(move || git_repo::disconnect_remote(&site_dir)).await {
        Ok(Ok(status)) => {
            state.record_event("git_disconnect", "site", &dom, "Disconnected Git Remote", "Removed origin remote", "info");
            Json(serde_json::json!({ "ok": true, "status": status })).into_response()
        }
        Ok(Err(e)) => (StatusCode::BAD_REQUEST, Json(serde_json::json!({ "error": e }))).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({ "error": e.to_string() }))).into_response(),
    }
}

async fn wordpress_reset_admin_password_handler(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(payload): Json<serde_json::Value>,
) -> Response {
    let Some(site) = state.get_site(&id) else {
        return (StatusCode::NOT_FOUND, Json(serde_json::json!({"error": "Site not found"}))).into_response();
    };
    let new_pw = payload
        .get("password")
        .or_else(|| payload.get("new_password"))
        .or_else(|| payload.get("admin_pass"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let site_dir = state.site_dir(&site);
    let dom = site.domain.clone();
    let dom_cl = dom.clone();
    let res = tokio::task::spawn_blocking(move || {
        wordpress::reset_admin_password(&site_dir, &dom_cl, &new_pw)
    })
    .await;
    match res {
        Ok(Ok(val)) => {
            state.record_event(
                "wp_reset_password",
                "site",
                &dom,
                "WordPress Admin Password Reset",
                &format!("Reset admin password for {}", dom),
                "success",
            );
            Json(val).into_response()
        }
        Ok(Err(e)) => (StatusCode::BAD_REQUEST, Json(serde_json::json!({"error": e}))).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({"error": e.to_string()}))).into_response(),
    }
}

async fn wordpress_optimize_db_handler(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Response {
    let Some(site) = state.get_site(&id) else {
        return (StatusCode::NOT_FOUND, Json(serde_json::json!({"error": "Site not found"}))).into_response();
    };
    let site_dir = state.site_dir(&site);
    let mdb_port = state.get_mariadb_port();
    let dom = site.domain.clone();
    let res = tokio::task::spawn_blocking(move || wordpress::optimize_wp_db(&site_dir, mdb_port)).await;
    match res {
        Ok(Ok(val)) => {
            state.record_event(
                "wp_optimize_db",
                "site",
                &dom,
                "WordPress Database Optimized",
                &format!("Optimized database for {}", dom),
                "success",
            );
            Json(val).into_response()
        }
        Ok(Err(e)) => (StatusCode::BAD_REQUEST, Json(serde_json::json!({"error": e}))).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({"error": e.to_string()}))).into_response(),
    }
}

async fn wordpress_plugin_action_handler(
    State(state): State<Arc<AppState>>,
    Path((id, action)): Path<(String, String)>,
    Json(payload): Json<serde_json::Value>,
) -> Response {
    let Some(site) = state.get_site(&id) else {
        return (StatusCode::NOT_FOUND, Json(serde_json::json!({"error": "Site not found"}))).into_response();
    };
    let name = payload.get("name").and_then(|v| v.as_str()).unwrap_or("").to_string();
    let site_dir = state.site_dir(&site);
    let dom = site.domain.clone();
    let dom_cl = dom.clone();
    let act_cl = action.clone();
    let name_cl = name.clone();
    let res = tokio::task::spawn_blocking(move || {
        wordpress::plugin_or_theme_action(&site_dir, &dom_cl, "plugin", &act_cl, &name_cl)
    })
    .await;
    match res {
        Ok(Ok(val)) => {
            state.record_event(
                "wp_plugin_action",
                "site",
                &dom,
                "WordPress Plugin Action",
                &format!("Plugin {} -> {} on {}", name, action, dom),
                "info",
            );
            Json(val).into_response()
        }
        Ok(Err(e)) => (StatusCode::BAD_REQUEST, Json(serde_json::json!({"error": e}))).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({"error": e.to_string()}))).into_response(),
    }
}

async fn wordpress_theme_action_handler(
    State(state): State<Arc<AppState>>,
    Path((id, action)): Path<(String, String)>,
    Json(payload): Json<serde_json::Value>,
) -> Response {
    let Some(site) = state.get_site(&id) else {
        return (StatusCode::NOT_FOUND, Json(serde_json::json!({"error": "Site not found"}))).into_response();
    };
    let name = payload.get("name").and_then(|v| v.as_str()).unwrap_or("").to_string();
    let site_dir = state.site_dir(&site);
    let dom = site.domain.clone();
    let dom_cl = dom.clone();
    let act_cl = action.clone();
    let name_cl = name.clone();
    let res = tokio::task::spawn_blocking(move || {
        wordpress::plugin_or_theme_action(&site_dir, &dom_cl, "theme", &act_cl, &name_cl)
    })
    .await;
    match res {
        Ok(Ok(val)) => {
            state.record_event(
                "wp_theme_action",
                "site",
                &dom,
                "WordPress Theme Action",
                &format!("Theme {} -> {} on {}", name, action, dom),
                "info",
            );
            Json(val).into_response()
        }
        Ok(Err(e)) => (StatusCode::BAD_REQUEST, Json(serde_json::json!({"error": e}))).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({"error": e.to_string()}))).into_response(),
    }
}

async fn wordpress_harden_handler(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    payload: Option<Json<serde_json::Value>>,
) -> Response {
    let Some(site) = state.get_site(&id) else {
        return (StatusCode::NOT_FOUND, Json(serde_json::json!({"error": "Site not found"}))).into_response();
    };
    let fixes: Vec<String> = payload
        .and_then(|Json(v)| v.get("fixes").and_then(|a| a.as_array()).cloned())
        .map(|arr| {
            arr.into_iter()
                .filter_map(|x| x.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();
    let site_dir = state.site_dir(&site);
    let dom = site.domain.clone();
    match wordpress::apply_wp_hardening(&state, &site_dir, &dom, &fixes) {
        Ok(results) => {
            let applied_cnt = results.iter().filter(|r| r["applied"] == true).count();
            state.record_event(
                "wp_harden",
                "site",
                &site.domain,
                "WordPress Hardened",
                &format!("Applied {} hardening fix(es) on {}", applied_cnt, site.domain),
                "success",
            );
            Json(serde_json::Value::Array(results)).into_response()
        }
        Err(e) => (StatusCode::BAD_REQUEST, Json(serde_json::json!({"error": e}))).into_response(),
    }
}

async fn wordpress_vuln_scan_handler(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Response {
    let Some(site) = state.get_site(&id) else {
        return (StatusCode::NOT_FOUND, Json(serde_json::json!({"error": "Site not found"}))).into_response();
    };
    let site_dir = state.site_dir(&site);
    let dom = site.domain.clone();
    let res = tokio::task::spawn_blocking(move || wordpress::scan_wp_vulnerabilities(&site_dir, &dom)).await;
    match res {
        Ok(Ok(val)) => {
            state.record_event(
                "wp_vuln_scan",
                "site",
                &site.domain,
                "WordPress Vulnerability Scan",
                &format!("Found {} vulnerabilities on {}", val["total_vulns"], site.domain),
                if val["critical_count"].as_u64().unwrap_or(0) > 0 { "warning" } else { "info" },
            );
            Json(val).into_response()
        }
        Ok(Err(e)) => (StatusCode::BAD_REQUEST, Json(serde_json::json!({"error": e}))).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({"error": e.to_string()}))).into_response(),
    }
}

async fn wordpress_security_check_handler(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Response {
    let Some(site) = state.get_site(&id) else {
        return (StatusCode::NOT_FOUND, Json(serde_json::json!({"error": "Site not found"}))).into_response();
    };
    let site_dir = state.site_dir(&site);
    match wordpress::check_wp_security(&site_dir) {
        Ok(val) => Json(val).into_response(),
        Err(e) => (StatusCode::BAD_REQUEST, Json(serde_json::json!({"error": e}))).into_response(),
    }
}

async fn wordpress_hardening_history_handler(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Response {
    let Some(site) = state.get_site(&id) else {
        return (StatusCode::NOT_FOUND, Json(serde_json::json!({"error": "Site not found"}))).into_response();
    };
    let site_dir = state.site_dir(&site);
    Json(serde_json::Value::Array(wordpress::get_hardening_history(&site_dir))).into_response()
}

async fn wordpress_update_handler(
    State(state): State<Arc<AppState>>,
    Path((id, target)): Path<(String, String)>,
) -> Response {
    let Some(site) = state.get_site(&id) else {
        return (StatusCode::NOT_FOUND, Json(serde_json::json!({"error": "Site not found"}))).into_response();
    };
    let site_dir = state.site_dir(&site);
    let dom = site.domain.clone();
    let tgt = target.clone();
    let res = tokio::task::spawn_blocking(move || wordpress::update_wp_target(&site_dir, &dom, &tgt)).await;
    match res {
        Ok(Ok(val)) => {
            state.record_event(
                "wp_update",
                "site",
                &site.domain,
                "WordPress Updated",
                val["message"].as_str().unwrap_or("Updated WordPress"),
                "success",
            );
            Json(val).into_response()
        }
        Ok(Err(e)) => (StatusCode::BAD_REQUEST, Json(serde_json::json!({"error": e}))).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({"error": e.to_string()}))).into_response(),
    }
}

async fn wordpress_update_safe_handler(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    payload: Option<Json<serde_json::Value>>,
) -> Response {
    let Some(site) = state.get_site(&id) else {
        return (StatusCode::NOT_FOUND, Json(serde_json::json!({"error": "Site not found"}))).into_response();
    };
    let target = payload
        .and_then(|Json(v)| v.get("target").and_then(|s| s.as_str()).map(String::from))
        .unwrap_or_else(|| "all".to_string());
    let site_dir = state.site_dir(&site);
    let dom = site.domain.clone();
    let res = tokio::task::spawn_blocking(move || wordpress::smart_update_wp(&site_dir, &dom, &target)).await;
    match res {
        Ok(Ok(val)) => {
            state.record_event(
                "wp_smart_update",
                "site",
                &site.domain,
                "WordPress Smart Update",
                val["message"].as_str().unwrap_or("Smart Update completed"),
                "success",
            );
            Json(val).into_response()
        }
        Ok(Err(e)) => (StatusCode::BAD_REQUEST, Json(serde_json::json!({"error": e}))).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({"error": e.to_string()}))).into_response(),
    }
}

async fn wordpress_bulk_update_handler(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<serde_json::Value>,
) -> Response {
    let site_ids: Vec<String> = payload
        .get("site_ids")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|x| x.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();
    let target = payload
        .get("target")
        .and_then(|v| v.as_str())
        .unwrap_or("all")
        .to_string();

    let mut results = Vec::new();
    for sid in site_ids {
        let Some(site) = state.get_site(&sid) else {
            results.push(serde_json::json!({
                "site_id": sid,
                "ok": false,
                "message": "Site not found",
            }));
            continue;
        };
        let site_dir = state.site_dir(&site);
        let dom = site.domain.clone();
        let tgt = target.clone();
        let outcome = tokio::task::spawn_blocking(move || wordpress::update_wp_target(&site_dir, &dom, &tgt)).await;
        match outcome {
            Ok(Ok(val)) => results.push(serde_json::json!({
                "site_id": site.id,
                "domain": site.domain,
                "ok": true,
                "updated": val["updated"],
                "message": val["message"],
            })),
            Ok(Err(e)) => results.push(serde_json::json!({
                "site_id": site.id,
                "domain": site.domain,
                "ok": false,
                "message": e,
            })),
            Err(e) => results.push(serde_json::json!({
                "site_id": site.id,
                "domain": site.domain,
                "ok": false,
                "message": e.to_string(),
            })),
        }
    }
    Json(serde_json::json!({ "results": results })).into_response()
}

async fn site_stats_handler(State(state): State<Arc<AppState>>, Path(id): Path<String>) -> Response {
    let Some(site) = state.get_site(&id) else {
        return (StatusCode::NOT_FOUND, Json(serde_json::json!({"error": "Site not found"}))).into_response();
    };
    Json(state.get_site_traffic_stats(&site.domain)).into_response()
}

async fn site_bandwidth_handler(State(state): State<Arc<AppState>>, Path(id): Path<String>) -> Response {
    let Some(site) = state.get_site(&id) else {
        return (StatusCode::NOT_FOUND, Json(serde_json::json!({"error": "Site not found"}))).into_response();
    };
    let stats = state.get_site_traffic_stats(&site.domain);
    let bytes = stats["bytes_sent"].as_u64().unwrap_or(0);
    let ym = chrono::Local::now().format("%Y-%m").to_string();
    Json(serde_json::json!({
        "used_bytes_this_month": bytes,
        "year_month": ym,
    }))
    .into_response()
}

async fn wordpress_integrity_handler(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Response {
    let Some(site) = state.get_site(&id) else {
        return (StatusCode::NOT_FOUND, Json(serde_json::json!({"error": "Site not found"}))).into_response();
    };
    let site_dir = state.site_dir(&site);
    let result = wordpress::check_wp_integrity(&site_dir).await;
    Json(result).into_response()
}

async fn wordpress_cli_handler(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(payload): Json<serde_json::Value>,
) -> Response {
    let Some(site) = state.get_site(&id) else {
        return (StatusCode::NOT_FOUND, Json(serde_json::json!({"error": "Site not found"}))).into_response();
    };
    let command = payload
        .get("command")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim()
        .to_string();
    if command.is_empty() {
        return (StatusCode::BAD_REQUEST, Json(serde_json::json!({"error": "Command is required"}))).into_response();
    }
    let site_dir = state.site_dir(&site);
    let dom = site.domain.clone();
    let cmd_cl = command.clone();
    let dom_cl = dom.clone();
    let res = tokio::task::spawn_blocking(move || wordpress::run_wp_cli_command(&site_dir, &dom_cl, &cmd_cl)).await;
    state.record_event(
        "wp_cli",
        "site",
        &dom,
        "WordPress CLI Command",
        &format!("Ran '{}' on {}", command, dom),
        "info",
    );
    match res {
        Ok(Ok(output)) => Json(serde_json::json!({ "ok": true, "output": output })).into_response(),
        Ok(Err(e)) => (StatusCode::BAD_REQUEST, Json(serde_json::json!({ "ok": false, "output": e, "error": e }))).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({ "ok": false, "error": e.to_string() }))).into_response(),
    }
}

async fn list_backups_handler(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Response {
    let Some(site) = state.get_site(&id) else {
        return (StatusCode::NOT_FOUND, Json(serde_json::json!({"error": "Site not found"}))).into_response();
    };
    Json(backups::list_site_backups(&site.id, &site.domain)).into_response()
}

async fn create_backup_handler(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Response {
    let Some(site) = state.get_site(&id) else {
        return (StatusCode::NOT_FOUND, Json(serde_json::json!({"error": "Site not found"}))).into_response();
    };
    let site_dir = state.site_dir(&site);
    let backup_id = uuid::Uuid::new_v4().to_string();
    let domain = site.domain.clone();
    state.push_php_install_step(
        &backup_id,
        serde_json::json!({
            "step": "prepare",
            "label": format!("Preparing ZIP archive for {}...", domain),
            "status": "in_progress"
        }),
        false,
    );

    let state_bg = state.clone();
    let bid_bg = backup_id.clone();
    tokio::task::spawn_blocking(move || {
        state_bg.push_php_install_step(
            &bid_bg,
            serde_json::json!({
                "step": "archive",
                "label": format!("Compressing files & SQLite databases in {}...", site_dir.display()),
                "status": "in_progress"
            }),
            false,
        );
        match backups::create_archive_sync(&site_dir, &domain) {
            Ok(entry) => {
                state_bg.record_event(
                    "create_backup",
                    "backup",
                    &domain,
                    "Backup Created",
                    &format!("Created {} ({} KB)", entry.filename, entry.size_bytes / 1024),
                    "success",
                );
                state_bg.push_php_install_step(
                    &bid_bg,
                    serde_json::json!({
                        "step": "complete",
                        "label": format!("Backup complete: {}", entry.filename),
                        "status": "done"
                    }),
                    true,
                );
            }
            Err(e) => {
                state_bg.record_event(
                    "create_backup_error",
                    "backup",
                    &domain,
                    "Backup Failed",
                    &e,
                    "critical",
                );
                state_bg.push_php_install_step(
                    &bid_bg,
                    serde_json::json!({
                        "step": "complete",
                        "label": format!("Backup failed: {}", e),
                        "status": "error"
                    }),
                    true,
                );
            }
        }
    });

    (
        StatusCode::ACCEPTED,
        Json(serde_json::json!({ "ok": true, "backup_id": backup_id })),
    )
        .into_response()
}

async fn restore_backup_handler(
    State(state): State<Arc<AppState>>,
    Path((id, bid)): Path<(String, String)>,
) -> Response {
    let Some(site) = state.get_site(&id) else {
        return (StatusCode::NOT_FOUND, Json(serde_json::json!({"error": "Site not found"}))).into_response();
    };
    let archive_path = match backups::resolve_backup_file(&site.domain, &bid) {
        Ok(p) => p,
        Err(e) => return (StatusCode::NOT_FOUND, Json(serde_json::json!({"error": e}))).into_response(),
    };
    let site_dir = state.site_dir(&site);
    let restore_id = uuid::Uuid::new_v4().to_string();
    let domain = site.domain.clone();
    state.push_php_install_step(
        &restore_id,
        serde_json::json!({
            "step": "prepare",
            "label": format!("Restoring backup {} to {}...", bid, domain),
            "status": "in_progress"
        }),
        false,
    );

    let state_bg = state.clone();
    let rid_bg = restore_id.clone();
    tokio::task::spawn_blocking(move || {
        match backups::restore_archive_sync(&site_dir, &archive_path) {
            Ok(()) => {
                state_bg.record_event(
                    "restore_backup",
                    "backup",
                    &domain,
                    "Backup Restored",
                    &format!("Restored {} from archive {}", domain, bid),
                    "success",
                );
                state_bg.push_php_install_step(
                    &rid_bg,
                    serde_json::json!({
                        "step": "complete",
                        "label": format!("Restore completed for {}", domain),
                        "status": "done"
                    }),
                    true,
                );
            }
            Err(e) => {
                state_bg.record_event(
                    "restore_backup_error",
                    "backup",
                    &domain,
                    "Restore Failed",
                    &e,
                    "critical",
                );
                state_bg.push_php_install_step(
                    &rid_bg,
                    serde_json::json!({
                        "step": "complete",
                        "label": format!("Restore failed: {}", e),
                        "status": "error"
                    }),
                    true,
                );
            }
        }
    });

    (
        StatusCode::ACCEPTED,
        Json(serde_json::json!({ "ok": true, "restore_id": restore_id })),
    )
        .into_response()
}

async fn download_backup_handler(
    State(state): State<Arc<AppState>>,
    Path((id, bid)): Path<(String, String)>,
) -> Response {
    let Some(site) = state.get_site(&id) else {
        return (StatusCode::NOT_FOUND, Json(serde_json::json!({"error": "Site not found"}))).into_response();
    };
    let archive_path = match backups::resolve_backup_file(&site.domain, &bid) {
        Ok(p) => p,
        Err(e) => return (StatusCode::NOT_FOUND, Json(serde_json::json!({"error": e}))).into_response(),
    };
    let bytes = match std::fs::read(&archive_path) {
        Ok(b) => b,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error": e.to_string()})),
            )
                .into_response()
        }
    };
    let fname = archive_path
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("backup.zip");
    (
        [
            (header::CONTENT_TYPE, "application/zip".to_string()),
            (
                header::CONTENT_DISPOSITION,
                format!("attachment; filename=\"{}\"", fname),
            ),
        ],
        bytes,
    )
        .into_response()
}

async fn delete_backup_handler(
    State(state): State<Arc<AppState>>,
    Path((id, bid)): Path<(String, String)>,
) -> Response {
    let Some(site) = state.get_site(&id) else {
        return (StatusCode::NOT_FOUND, Json(serde_json::json!({"error": "Site not found"}))).into_response();
    };
    let archive_path = match backups::resolve_backup_file(&site.domain, &bid) {
        Ok(p) => p,
        Err(e) => return (StatusCode::NOT_FOUND, Json(serde_json::json!({"error": e}))).into_response(),
    };
    match std::fs::remove_file(&archive_path) {
        Ok(()) => {
            state.record_event(
                "delete_backup",
                "backup",
                &site.domain,
                "Backup Deleted",
                &format!("Deleted backup {} for {}", bid, site.domain),
                "info",
            );
            Json(serde_json::json!({"ok": true})).into_response()
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

fn find_database_by_id(state: &AppState, id: &str) -> Option<databases::LocalDatabase> {
    let sites = state.list_sites();
    databases::discover_databases_with_mariadb(
        &sites,
        &state.get_default_sites_dir(),
        state.get_mariadb_port(),
    )
    .into_iter()
    .find(|d| d.id == id)
}

async fn list_databases_handler(State(state): State<Arc<AppState>>) -> Json<serde_json::Value> {
    let sites = state.list_sites();
    let dbs = databases::discover_databases_with_mariadb(
        &sites,
        &state.get_default_sites_dir(),
        state.get_mariadb_port(),
    );
    Json(serde_json::json!(dbs))
}

async fn create_database_handler(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<serde_json::Value>,
) -> Response {
    let name = payload.get("name").and_then(|v| v.as_str()).unwrap_or("");
    let engine = payload
        .get("engine")
        .and_then(|v| v.as_str())
        .unwrap_or("sqlite");

    if engine.eq_ignore_ascii_case("mariadb") || engine.eq_ignore_ascii_case("mysql") {
        let port = match state.get_mariadb_port().ok_or(()).or_else(|_| state.start_mariadb().map_err(|_| ())) {
            Ok(p) => p,
            Err(_) => {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(serde_json::json!({
                        "error": "MariaDB is not installed or running. Install MariaDB in Settings -> PHP & Services or use SQLite."
                    })),
                )
                    .into_response();
            }
        };
        let nm = name.to_string();
        let res = tokio::task::spawn_blocking(move || mariadb::create_database(port, &nm)).await;
        return match res {
            Ok(Ok(clean)) => {
                state.record_event(
                    "create_database",
                    "database",
                    &clean,
                    "MariaDB Database Created",
                    &format!("Created local MariaDB database '{}'", clean),
                    "success",
                );
                let db = databases::LocalDatabase {
                    id: format!("mariadb:{}", clean),
                    site_id: None,
                    stack_id: None,
                    name: clean,
                    engine: "mariadb".to_string(),
                    db_user: "root".to_string(),
                    container_id: None,
                    port: Some(port as i32),
                    created_at: chrono::Utc::now().to_rfc3339(),
                    file_path: mariadb::mariadb_data_dir(),
                };
                (StatusCode::CREATED, Json(db)).into_response()
            }
            Ok(Err(e)) => (StatusCode::BAD_REQUEST, Json(serde_json::json!({"error": e}))).into_response(),
            Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({"error": e.to_string()}))).into_response(),
        };
    }

    match databases::create_local_sqlite_db(name) {
        Ok(db) => {
            state.record_event(
                "create_database",
                "database",
                &db.name,
                "SQLite Database Created",
                &format!("Created local SQLite database '{}'", db.name),
                "success",
            );
            (StatusCode::CREATED, Json(db)).into_response()
        }
        Err(e) => (StatusCode::BAD_REQUEST, Json(serde_json::json!({"error": e}))).into_response(),
    }
}

async fn delete_database_handler(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Response {
    let Some(db) = find_database_by_id(&state, &id) else {
        return (StatusCode::NOT_FOUND, Json(serde_json::json!({"error": "Database not found"}))).into_response();
    };
    if id.starts_with("wp:") {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "error": "Site WordPress SQLite databases cannot be deleted independently; delete the site instead."
            })),
        )
            .into_response();
    }
    if db.engine == "mariadb" {
        let Some(port) = state.get_mariadb_port() else {
            return (StatusCode::BAD_REQUEST, Json(serde_json::json!({"error": "MariaDB daemon is not running"}))).into_response();
        };
        let db_name = db.name.clone();
        let nm_cl = db_name.clone();
        let res = tokio::task::spawn_blocking(move || mariadb::drop_database(port, &nm_cl)).await;
        return match res {
            Ok(Ok(())) => {
                state.record_event(
                    "delete_database",
                    "database",
                    &db_name,
                    "Database Deleted",
                    &format!("Dropped MariaDB database '{}'", db_name),
                    "info",
                );
                Json(serde_json::json!({"ok": true})).into_response()
            }
            Ok(Err(e)) => (StatusCode::BAD_REQUEST, Json(serde_json::json!({"error": e}))).into_response(),
            Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({"error": e.to_string()}))).into_response(),
        };
    }
    match std::fs::remove_file(&db.file_path) {
        Ok(()) => {
            state.record_event(
                "delete_database",
                "database",
                &db.name,
                "Database Deleted",
                &format!("Deleted local SQLite database '{}'", db.name),
                "info",
            );
            Json(serde_json::json!({"ok": true})).into_response()
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

async fn databases_sizes_handler(State(state): State<Arc<AppState>>) -> Json<serde_json::Value> {
    let sites = state.list_sites();
    let mdb_port = state.get_mariadb_port();
    let dbs = databases::discover_databases_with_mariadb(
        &sites,
        &state.get_default_sites_dir(),
        mdb_port,
    );
    let mut sizes = serde_json::Map::new();
    for db in dbs {
        if db.engine == "mariadb" {
            let sz = mdb_port
                .and_then(|p| {
                    let sql = format!(
                        "SELECT COALESCE(SUM(data_length + index_length), 0) FROM information_schema.TABLES WHERE table_schema='{}'",
                        db.name.replace('\'', "''")
                    );
                    mariadb::run_mysql_query(p, None, &sql).ok()
                })
                .and_then(|v| v["rows"][0][0].as_str().and_then(|s| s.parse::<u64>().ok()))
                .unwrap_or(0);
            sizes.insert(db.id, serde_json::json!(sz));
        } else {
            let len = std::fs::metadata(&db.file_path).map(|m| m.len()).unwrap_or(0);
            sizes.insert(db.id, serde_json::json!(len));
            sizes.insert(db.name, serde_json::json!(len));
        }
    }
    Json(serde_json::json!({ "sizes": sizes }))
}

async fn database_credentials_handler(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Response {
    let Some(db) = find_database_by_id(&state, &id) else {
        return (StatusCode::NOT_FOUND, Json(serde_json::json!({"error": "Database not found"}))).into_response();
    };
    if db.engine == "mariadb" {
        let port = db.port.unwrap_or(3306);
        return Json(serde_json::json!({
            "host": "127.0.0.1",
            "port": port,
            "database": db.name,
            "username": "root",
            "password": "",
            "engine": "mariadb",
            "connection_string": format!("mysql://root@127.0.0.1:{}/{}", port, db.name),
            "internal_host": "127.0.0.1",
        }))
        .into_response();
    }
    let path_str = db.file_path.to_string_lossy().to_string();
    Json(serde_json::json!({
        "host": "localhost",
        "port": 0,
        "database": db.name,
        "username": "local",
        "password": "",
        "engine": "sqlite",
        "connection_string": format!("sqlite:{}", path_str),
        "internal_host": path_str,
    }))
    .into_response()
}

async fn database_schema_overview_handler(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Response {
    let Some(db) = find_database_by_id(&state, &id) else {
        return (StatusCode::NOT_FOUND, Json(serde_json::json!({"error": "Database not found"}))).into_response();
    };
    if db.engine == "mariadb" {
        let Some(port) = state.get_mariadb_port() else {
            return (StatusCode::BAD_REQUEST, Json(serde_json::json!({"error": "MariaDB daemon is not running"}))).into_response();
        };
        let db_name = db.name.clone();
        let res = tokio::task::spawn_blocking(move || {
            let sql = format!(
                "SELECT table_name, engine, table_rows, ROUND((data_length + index_length)/1024, 1) AS size_kb FROM information_schema.tables WHERE table_schema='{}' ORDER BY table_name",
                db_name.replace('\'', "''")
            );
            mariadb::run_mysql_query(port, Some(&db_name), &sql)
        })
        .await;
        return match res {
            Ok(Ok(qr)) => Json(serde_json::json!({
                "tables": qr,
                "foreign_keys": {
                    "columns": ["table_name", "column_name", "referenced_table", "referenced_column"],
                    "rows": [],
                    "row_count": 0,
                    "execution_time_ms": 0,
                    "truncated": false
                }
            }))
            .into_response(),
            Ok(Err(e)) => (StatusCode::BAD_REQUEST, Json(serde_json::json!({"error": e}))).into_response(),
            Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({"error": e.to_string()}))).into_response(),
        };
    }

    let res = tokio::task::spawn_blocking(move || {
        databases::run_sqlite_query(
            &db.file_path,
            "SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%' AND name NOT LIKE '_wp_sqlite_%' ORDER BY name",
        )
    })
    .await;

    match res {
        Ok(Ok(qr)) => {
            let empty = Vec::new();
            let src_rows = qr["rows"].as_array().unwrap_or(&empty);
            let mapped: Vec<serde_json::Value> = src_rows
                .iter()
                .filter_map(|r| r.as_array()?.first()?.as_str())
                .map(|name| serde_json::json!([name, "SQLite", "—", "—"]))
                .collect();
            let count = mapped.len();
            Json(serde_json::json!({
                "tables": {
                    "columns": ["table_name", "engine", "rows", "size_kb"],
                    "rows": mapped,
                    "row_count": count,
                    "execution_time_ms": qr["execution_time_ms"],
                    "truncated": false
                },
                "foreign_keys": {
                    "columns": ["table_name", "column_name", "referenced_table", "referenced_column"],
                    "rows": [],
                    "row_count": 0,
                    "execution_time_ms": 0,
                    "truncated": false
                }
            }))
            .into_response()
        }
        Ok(Err(e)) => (StatusCode::BAD_REQUEST, Json(serde_json::json!({"error": e}))).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({"error": e.to_string()}))).into_response(),
    }
}

async fn database_tables_handler(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Response {
    let Some(db) = find_database_by_id(&state, &id) else {
        return (StatusCode::NOT_FOUND, Json(serde_json::json!({"error": "Database not found"}))).into_response();
    };
    let mdb_port = state.get_mariadb_port();
    let res = tokio::task::spawn_blocking(move || {
        if db.engine == "mariadb" {
            let port = mdb_port.ok_or_else(|| "MariaDB daemon is not running".to_string())?;
            mariadb::run_mysql_query(port, Some(&db.name), "SHOW TABLES")
        } else {
            databases::run_sqlite_query(
                &db.file_path,
                "SELECT name AS table_name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%' AND name NOT LIKE '_wp_sqlite_%' ORDER BY name",
            )
        }
    })
    .await;
    match res {
        Ok(Ok(v)) => Json(v).into_response(),
        Ok(Err(e)) => (StatusCode::BAD_REQUEST, Json(serde_json::json!({"error": e}))).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({"error": e.to_string()}))).into_response(),
    }
}

async fn database_table_structure_handler(
    State(state): State<Arc<AppState>>,
    Path((id, table)): Path<(String, String)>,
) -> Response {
    let Some(db) = find_database_by_id(&state, &id) else {
        return (StatusCode::NOT_FOUND, Json(serde_json::json!({"error": "Database not found"}))).into_response();
    };
    let clean = match validate_sqlite_ident(&table) {
        Ok(t) => t.to_string(),
        Err(e) => return (StatusCode::BAD_REQUEST, Json(serde_json::json!({"error": e}))).into_response(),
    };
    let mdb_port = state.get_mariadb_port();
    let res = tokio::task::spawn_blocking(move || {
        if db.engine == "mariadb" {
            let port = mdb_port.ok_or_else(|| "MariaDB daemon is not running".to_string())?;
            mariadb::run_mysql_query(port, Some(&db.name), &format!("DESCRIBE `{}`", clean))
        } else {
            let raw = databases::run_sqlite_query(&db.file_path, &format!("PRAGMA table_info(\"{}\")", clean))?;
            let empty = Vec::new();
            let src_rows = raw["rows"].as_array().unwrap_or(&empty);
            let mapped: Vec<serde_json::Value> = src_rows
                .iter()
                .filter_map(|r| {
                    let arr = r.as_array()?;
                    let col_name = arr.get(1)?.as_str().unwrap_or("");
                    let col_type = arr.get(2).and_then(|v| v.as_str()).unwrap_or("");
                    let notnull = arr.get(3).and_then(|v| v.as_str()).unwrap_or("0");
                    let dflt = arr.get(4).and_then(|v| v.as_str()).unwrap_or("");
                    let pk = arr.get(5).and_then(|v| v.as_str()).unwrap_or("0");
                    let nullable = if notnull == "1" { "NO" } else { "YES" };
                    let key = if pk != "0" && !pk.is_empty() { "PRI" } else { "" };
                    let def_str = if dflt == "\\N" { "" } else { dflt };
                    Some(serde_json::json!([col_name, col_type, nullable, key, def_str, ""]))
                })
                .collect();
            let rc = mapped.len();
            Ok(serde_json::json!({
                "columns": ["Field", "Type", "Null", "Key", "Default", "Extra"],
                "rows": mapped,
                "row_count": rc,
                "execution_time_ms": raw["execution_time_ms"],
                "truncated": false
            }))
        }
    })
    .await;
    match res {
        Ok(Ok(v)) => Json(v).into_response(),
        Ok(Err(e)) => (StatusCode::BAD_REQUEST, Json(serde_json::json!({"error": e}))).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({"error": e.to_string()}))).into_response(),
    }
}

async fn database_table_indexes_handler(
    State(state): State<Arc<AppState>>,
    Path((id, table)): Path<(String, String)>,
) -> Response {
    let Some(db) = find_database_by_id(&state, &id) else {
        return (StatusCode::NOT_FOUND, Json(serde_json::json!({"error": "Database not found"}))).into_response();
    };
    let clean = match validate_sqlite_ident(&table) {
        Ok(t) => t.to_string(),
        Err(e) => return (StatusCode::BAD_REQUEST, Json(serde_json::json!({"error": e}))).into_response(),
    };
    let mdb_port = state.get_mariadb_port();
    let res = tokio::task::spawn_blocking(move || {
        if db.engine == "mariadb" {
            let port = mdb_port.ok_or_else(|| "MariaDB daemon is not running".to_string())?;
            mariadb::run_mysql_query(port, Some(&db.name), &format!("SHOW INDEX FROM `{}`", clean))
        } else {
            databases::run_sqlite_query(&db.file_path, &format!("PRAGMA index_list(\"{}\")", clean))
        }
    })
    .await;
    match res {
        Ok(Ok(v)) => Json(v).into_response(),
        Ok(Err(e)) => (StatusCode::BAD_REQUEST, Json(serde_json::json!({"error": e}))).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({"error": e.to_string()}))).into_response(),
    }
}

async fn database_query_handler(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(payload): Json<serde_json::Value>,
) -> Response {
    let Some(db) = find_database_by_id(&state, &id) else {
        return (StatusCode::NOT_FOUND, Json(serde_json::json!({"error": "Database not found"}))).into_response();
    };
    let sql = payload
        .get("sql")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    if sql.trim().is_empty() {
        return (StatusCode::BAD_REQUEST, Json(serde_json::json!({"error": "SQL query is empty"}))).into_response();
    }
    let mdb_port = state.get_mariadb_port();
    let res = tokio::task::spawn_blocking(move || {
        if db.engine == "mariadb" {
            let port = mdb_port.ok_or_else(|| "MariaDB daemon is not running".to_string())?;
            mariadb::run_mysql_query(port, Some(&db.name), &sql)
        } else {
            databases::run_sqlite_query(&db.file_path, &sql)
        }
    })
    .await;
    match res {
        Ok(Ok(v)) => Json(v).into_response(),
        Ok(Err(e)) => (StatusCode::BAD_REQUEST, Json(serde_json::json!({"error": e}))).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({"error": e.to_string()}))).into_response(),
    }
}

async fn database_list_dumps_handler(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Response {
    let Some(db) = find_database_by_id(&state, &id) else {
        return (StatusCode::NOT_FOUND, Json(serde_json::json!({"error": "Database not found"}))).into_response();
    };
    Json(serde_json::json!(databases::list_dumps(&db))).into_response()
}

async fn database_export_handler(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Response {
    let Some(db) = find_database_by_id(&state, &id) else {
        return (StatusCode::NOT_FOUND, Json(serde_json::json!({"error": "Database not found"}))).into_response();
    };
    let db_name = db.name.clone();
    let mdb_port = state.get_mariadb_port();
    let res = tokio::task::spawn_blocking(move || {
        if db.engine == "mariadb" {
            let port = mdb_port.ok_or_else(|| "MariaDB daemon is not running".to_string())?;
            let dir = databases::database_dumps_dir(&db);
            let stamp = chrono::Utc::now().format("%Y%m%d-%H%M%S");
            let fname = format!("{}_{}.sql", db.name, stamp);
            mariadb::export_mysql_dump(port, &db.name, &dir.join(&fname))?;
            Ok(fname)
        } else {
            databases::export_sqlite_dump(&db)
        }
    })
    .await;
    match res {
        Ok(Ok(filename)) => {
            state.record_event(
                "export_database",
                "database",
                &db_name,
                "Database Exported",
                &format!("Exported {} to {}", db_name, filename),
                "success",
            );
            Json(serde_json::json!({ "ok": true, "filename": filename })).into_response()
        }
        Ok(Err(e)) => (StatusCode::BAD_REQUEST, Json(serde_json::json!({"error": e}))).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({"error": e.to_string()}))).into_response(),
    }
}

async fn database_upload_dump_handler(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(payload): Json<serde_json::Value>,
) -> Response {
    let Some(db) = find_database_by_id(&state, &id) else {
        return (StatusCode::NOT_FOUND, Json(serde_json::json!({"error": "Database not found"}))).into_response();
    };
    let filename = payload.get("filename").and_then(|v| v.as_str()).unwrap_or("").to_string();
    let content = payload.get("content").and_then(|v| v.as_str()).unwrap_or("").to_string();
    let auto_import = payload.get("auto_import").and_then(|v| v.as_bool()).unwrap_or(false);
    let db_name = db.name.clone();
    let res = tokio::task::spawn_blocking(move || databases::upload_dump(&db, &filename, &content, auto_import)).await;
    match res {
        Ok(Ok(fname)) => {
            state.record_event(
                "upload_dump",
                "database",
                &db_name,
                "Database Dump Uploaded",
                &format!("Uploaded '{}'{}", fname, if auto_import { " and restored" } else { "" }),
                "success",
            );
            Json(serde_json::json!({ "ok": true, "filename": fname })).into_response()
        }
        Ok(Err(e)) => (StatusCode::BAD_REQUEST, Json(serde_json::json!({"error": e}))).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({"error": e.to_string()}))).into_response(),
    }
}

async fn database_import_dump_handler(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(payload): Json<serde_json::Value>,
) -> Response {
    let Some(db) = find_database_by_id(&state, &id) else {
        return (StatusCode::NOT_FOUND, Json(serde_json::json!({"error": "Database not found"}))).into_response();
    };
    let filename = payload.get("filename").and_then(|v| v.as_str()).unwrap_or("").to_string();
    let fname_log = filename.clone();
    let db_name = db.name.clone();
    let mdb_port = state.get_mariadb_port();
    let res = tokio::task::spawn_blocking(move || {
        if db.engine == "mariadb" {
            let port = mdb_port.ok_or_else(|| "MariaDB daemon is not running".to_string())?;
            let path = databases::database_dumps_dir(&db).join(&filename);
            mariadb::import_mysql_dump(port, &db.name, &path)
        } else {
            databases::import_dump(&db, &filename)
        }
    })
    .await;
    match res {
        Ok(Ok(())) => {
            state.record_event(
                "import_database",
                "database",
                &db_name,
                "Database Restored",
                &format!("Restored {} from dump '{}'", db_name, fname_log),
                "success",
            );
            Json(serde_json::json!({ "ok": true })).into_response()
        }
        Ok(Err(e)) => (StatusCode::BAD_REQUEST, Json(serde_json::json!({"error": e}))).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({"error": e.to_string()}))).into_response(),
    }
}

async fn database_delete_dump_handler(
    State(state): State<Arc<AppState>>,
    Path((id, filename)): Path<(String, String)>,
) -> Response {
    let Some(db) = find_database_by_id(&state, &id) else {
        return (StatusCode::NOT_FOUND, Json(serde_json::json!({"error": "Database not found"}))).into_response();
    };
    match databases::delete_dump(&db, &filename) {
        Ok(()) => Json(serde_json::json!({ "ok": true })).into_response(),
        Err(e) => (StatusCode::BAD_REQUEST, Json(serde_json::json!({"error": e}))).into_response(),
    }
}

async fn database_reset_password_handler(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Response {
    let Some(db) = find_database_by_id(&state, &id) else {
        return (StatusCode::NOT_FOUND, Json(serde_json::json!({"error": "Database not found"}))).into_response();
    };
    Json(serde_json::json!({
        "ok": true,
        "password": if db.engine == "mariadb" { "(empty local root password)" } else { "(local file-based SQLite — no password required)" }
    }))
    .into_response()
}

async fn database_download_dump_handler(
    State(state): State<Arc<AppState>>,
    Path((id, filename)): Path<(String, String)>,
) -> Response {
    let Some(db) = find_database_by_id(&state, &id) else {
        return (StatusCode::NOT_FOUND, Json(serde_json::json!({"error": "Database not found"}))).into_response();
    };
    if filename.trim().is_empty()
        || filename.contains("..")
        || filename.contains('/')
        || filename.contains('\\')
        || filename.contains(':')
    {
        return (StatusCode::BAD_REQUEST, Json(serde_json::json!({"error": "Invalid dump filename"}))).into_response();
    }
    let path = databases::database_dumps_dir(&db).join(filename.trim());
    let Ok(bytes) = tokio::fs::read(&path).await else {
        return (StatusCode::NOT_FOUND, Json(serde_json::json!({"error": "Dump file not found"}))).into_response();
    };
    let disp = format!("attachment; filename=\"{}\"", filename.replace('"', ""));
    Response::builder()
        .status(StatusCode::OK)
        .header(axum::http::header::CONTENT_TYPE, "application/octet-stream")
        .header(axum::http::header::CONTENT_DISPOSITION, disp)
        .body(axum::body::Body::from(bytes))
        .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response())
}

async fn database_health_handler(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Response {
    let Some(db) = find_database_by_id(&state, &id) else {
        return (StatusCode::NOT_FOUND, Json(serde_json::json!({"error": "Database not found"}))).into_response();
    };
    let res = tokio::task::spawn_blocking(move || databases::inspect_database_health(&db)).await;
    match res {
        Ok(Ok(report)) => Json(report).into_response(),
        Ok(Err(e)) => (StatusCode::BAD_REQUEST, Json(serde_json::json!({"error": e}))).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({"error": e.to_string()}))).into_response(),
    }
}

async fn database_optimize_handler(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    payload: Option<Json<databases::DbOptimizeRequest>>,
) -> Response {
    let Some(db) = find_database_by_id(&state, &id) else {
        return (StatusCode::NOT_FOUND, Json(serde_json::json!({"error": "Database not found"}))).into_response();
    };
    let db_name = db.name.clone();
    let req = payload.map(|Json(v)| v).unwrap_or_default();
    let res = tokio::task::spawn_blocking(move || databases::optimize_database(&db, &req)).await;
    match res {
        Ok(Ok(report)) => {
            state.record_event(
                "optimize_database",
                "database",
                &db_name,
                "Database Optimized",
                &format!(
                    "Ran '{}' on {} ({} table(s), reclaimed {} bytes)",
                    report.action, db_name, report.tables_processed, report.reclaimed_bytes
                ),
                "success",
            );
            Json(report).into_response()
        }
        Ok(Err(e)) => (StatusCode::BAD_REQUEST, Json(serde_json::json!({"error": e}))).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({"error": e.to_string()}))).into_response(),
    }
}

async fn database_prune_handler(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    payload: Option<Json<databases::DbPruneRequest>>,
) -> Response {
    let Some(db) = find_database_by_id(&state, &id) else {
        return (StatusCode::NOT_FOUND, Json(serde_json::json!({"error": "Database not found"}))).into_response();
    };
    let db_name = db.name.clone();
    let req = payload.map(|Json(v)| v).unwrap_or_default();
    let res = tokio::task::spawn_blocking(move || databases::prune_database(&db, &req)).await;
    match res {
        Ok(Ok(report)) => {
            if !report.dry_run {
                state.record_event(
                    "prune_database",
                    "database",
                    &db_name,
                    "Database Pruned",
                    &format!(
                        "Pruned {} row(s) and {} dump(s) from {} (reclaimed {} bytes)",
                        report.rows_deleted, report.dumps_deleted, db_name, report.reclaimed_bytes
                    ),
                    "success",
                );
            }
            Json(report).into_response()
        }
        Ok(Err(e)) => (StatusCode::BAD_REQUEST, Json(serde_json::json!({"error": e}))).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({"error": e.to_string()}))).into_response(),
    }
}

async fn databases_maintenance_summary_handler(State(state): State<Arc<AppState>>) -> Response {
    let sites = state.list_sites();
    let sites_dir = state.get_default_sites_dir();
    let mdb_port = state.get_mariadb_port();

    let summary = tokio::task::spawn_blocking(move || {
        let dbs = databases::discover_databases_with_mariadb(&sites, &sites_dir, mdb_port);
        let (linked_names, linked_ids) = databases::linked_database_names_and_ids(&sites, &sites_dir);

        let mut total_bytes = 0u64;
        let mut total_overhead_bytes = 0u64;
        let mut total_dumps = 0usize;
        let mut total_dump_bytes = 0u64;
        let mut orphan_databases = Vec::new();
        let mut items = Vec::new();

        for db in &dbs {
            let health = databases::inspect_database_health(db).ok();
            let size_bytes = health
                .as_ref()
                .map(|h| h.total_bytes)
                .unwrap_or_else(|| databases::database_disk_size_bytes(db));
            let overhead_bytes = health.as_ref().map(|h| h.overhead_bytes).unwrap_or(0);
            let table_count = health.as_ref().map(|h| h.table_count).unwrap_or(0);
            let (dump_count, dump_bytes) = databases::database_dump_stats(db);

            total_bytes += size_bytes;
            total_overhead_bytes += overhead_bytes;
            total_dumps += dump_count;
            total_dump_bytes += dump_bytes;

            let is_linked = db.site_id.is_some()
                || linked_ids.contains(&db.id)
                || linked_names.contains(&db.name.to_ascii_lowercase());

            if !is_linked {
                orphan_databases.push(serde_json::json!({
                    "id": db.id,
                    "name": db.name,
                    "engine": db.engine,
                    "size_bytes": size_bytes,
                    "table_count": table_count,
                    "reason": "Not referenced by any registered site (.env / wp-config.php)"
                }));
            }

            items.push(serde_json::json!({
                "id": db.id,
                "name": db.name,
                "engine": db.engine,
                "linked": is_linked,
                "size_bytes": size_bytes,
                "overhead_bytes": overhead_bytes,
                "table_count": table_count,
                "dump_count": dump_count,
                "dump_bytes": dump_bytes
            }));
        }

        serde_json::json!({
            "total_databases": dbs.len(),
            "total_bytes": total_bytes,
            "total_overhead_bytes": total_overhead_bytes,
            "total_dumps": total_dumps,
            "total_dump_bytes": total_dump_bytes,
            "orphan_databases": orphan_databases,
            "databases": items,
        })
    })
    .await
    .unwrap_or_else(|_| serde_json::json!({ "total_databases": 0, "orphan_databases": [], "databases": [] }));

    Json(summary).into_response()
}

#[derive(serde::Deserialize, Default)]
struct GlobalMaintenanceCleanupReq {
    #[serde(default)]
    optimize_all: bool,
    #[serde(default)]
    prune_dumps_keep: Option<usize>,
    #[serde(default)]
    prune_dumps_older_than_days: Option<u64>,
    #[serde(default)]
    delete_orphan_ids: Vec<String>,
}

async fn databases_maintenance_cleanup_handler(
    State(state): State<Arc<AppState>>,
    Json(req): Json<GlobalMaintenanceCleanupReq>,
) -> Response {
    let sites = state.list_sites();
    let sites_dir = state.get_default_sites_dir();
    let mdb_port = state.get_mariadb_port();

    let result = tokio::task::spawn_blocking(move || {
        let dbs = databases::discover_databases_with_mariadb(&sites, &sites_dir, mdb_port);
        let (linked_names, linked_ids) = databases::linked_database_names_and_ids(&sites, &sites_dir);
        let mut optimized_count = 0usize;
        let mut reclaimed_bytes = 0u64;
        let mut dumps_deleted = 0usize;
        let mut orphans_deleted = 0usize;
        let mut errors = Vec::new();

        for target_id in &req.delete_orphan_ids {
            if let Some(db) = dbs.iter().find(|d| d.id == *target_id) {
                let is_linked = db.site_id.is_some()
                    || linked_ids.contains(&db.id)
                    || linked_names.contains(&db.name.to_ascii_lowercase());
                if is_linked {
                    errors.push(format!("Skipped '{}': database is linked to an active site", db.name));
                    continue;
                }
                let sz = databases::database_disk_size_bytes(db);
                match databases::delete_local_database(db) {
                    Ok(()) => {
                        orphans_deleted += 1;
                        reclaimed_bytes += sz;
                    }
                    Err(e) => errors.push(format!("{}: {}", db.name, e)),
                }
            }
        }

        let surviving_dbs = databases::discover_databases_with_mariadb(&sites, &sites_dir, mdb_port);
        for db in &surviving_dbs {
            if req.prune_dumps_keep.is_some() || req.prune_dumps_older_than_days.is_some() {
                match databases::prune_dumps(db, req.prune_dumps_keep, req.prune_dumps_older_than_days) {
                    Ok((cnt, bytes)) => {
                        dumps_deleted += cnt;
                        reclaimed_bytes += bytes;
                    }
                    Err(e) => errors.push(format!("{} dumps: {}", db.name, e)),
                }
            }
            if req.optimize_all {
                match databases::optimize_database(
                    db,
                    &databases::DbOptimizeRequest {
                        action: Some("optimize".to_string()),
                        tables: None,
                    },
                ) {
                    Ok(rep) => {
                        optimized_count += 1;
                        reclaimed_bytes += rep.reclaimed_bytes;
                    }
                    Err(e) => errors.push(format!("{} optimize: {}", db.name, e)),
                }
            }
        }

        serde_json::json!({
            "ok": errors.is_empty(),
            "optimized_count": optimized_count,
            "reclaimed_bytes": reclaimed_bytes,
            "dumps_deleted": dumps_deleted,
            "orphans_deleted": orphans_deleted,
            "errors": errors,
        })
    })
    .await
    .unwrap_or_else(|e| serde_json::json!({ "ok": false, "errors": [e.to_string()] }));

    state.record_event(
        "cleanup_databases",
        "database",
        "workspace",
        "Database Maintenance Completed",
        &format!(
            "Optimized {} database(s), pruned {} dump(s), removed {} orphan(s), reclaimed {} KB",
            result["optimized_count"].as_u64().unwrap_or(0),
            result["dumps_deleted"].as_u64().unwrap_or(0),
            result["orphans_deleted"].as_u64().unwrap_or(0),
            result["reclaimed_bytes"].as_u64().unwrap_or(0) / 1024
        ),
        "success",
    );

    Json(result).into_response()
}

#[derive(serde::Deserialize, Default)]
struct LogQueryParams {
    #[serde(rename = "type")]
    log_type: Option<String>,
    lines: Option<usize>,
    filter: Option<String>,
    pattern: Option<String>,
    max: Option<usize>,
}

async fn logs_read_handler(
    State(state): State<Arc<AppState>>,
    Query(q): Query<LogQueryParams>,
) -> Json<serde_json::Value> {
    let lt = q.log_type.as_deref().unwrap_or("syslog");
    let limit = q.lines.or(q.max).unwrap_or(200).clamp(1, 5000);
    let pat = q.pattern.as_deref().or(q.filter.as_deref());
    let lines = read_desktop_log_lines(&state, lt, limit, pat, None);
    Json(serde_json::json!(lines))
}

async fn site_logs_read_handler(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Query(q): Query<LogQueryParams>,
) -> Response {
    let Some(site) = state.get_site(&id) else {
        return (StatusCode::NOT_FOUND, Json(serde_json::json!({"error": "Site not found"}))).into_response();
    };
    let lt = q.log_type.as_deref().unwrap_or("access");
    let limit = q.lines.or(q.max).unwrap_or(200).clamp(1, 5000);
    let pat = q.pattern.as_deref().or(q.filter.as_deref());
    let lines = read_desktop_log_lines(&state, lt, limit, pat, Some(&site.domain));
    Json(serde_json::json!(lines)).into_response()
}

async fn site_log_text_handler(
    state: Arc<AppState>,
    id: String,
    log_type: &str,
    limit: usize,
) -> Response {
    let Some(site) = state.get_site(&id) else {
        return (StatusCode::NOT_FOUND, Json(serde_json::json!({"error": "Site not found"}))).into_response();
    };
    let lines = read_desktop_log_lines(&state, log_type, limit.clamp(1, 5000), None, Some(&site.domain));
    Json(serde_json::json!({ "logs": lines.join("\n") })).into_response()
}

async fn site_logs_stream_handler(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Query(q): Query<LogQueryParams>,
) -> Response {
    let Some(site) = state.get_site(&id) else {
        return (StatusCode::NOT_FOUND, Json(serde_json::json!({"error": "Site not found"}))).into_response();
    };
    let lt = q.log_type.unwrap_or_else(|| "php".to_string());
    let limit = q.lines.or(q.max).unwrap_or(200).clamp(1, 2000);
    let pat = q.pattern.or(q.filter);
    let domain = site.domain.clone();

    let (tx, rx) = tokio::sync::mpsc::channel::<Result<SseEvent, std::convert::Infallible>>(64);
    tokio::spawn(async move {
        let initial = read_desktop_log_lines(&state, &lt, limit, pat.as_deref(), Some(&domain));
        let snap_json = serde_json::json!({
            "mode": "snapshot",
            "type": lt,
            "lines": initial,
        })
        .to_string();
        if tx.send(Ok(SseEvent::default().data(snap_json))).await.is_err() {
            return;
        }
        let mut prev_lines = initial;

        for _ in 0..7200 {
            tokio::time::sleep(std::time::Duration::from_millis(450)).await;
            let current = read_desktop_log_lines(&state, &lt, limit, pat.as_deref(), Some(&domain));
            if current == prev_lines {
                continue;
            }
            // Find overlap suffix of prev_lines matching prefix of current to emit only appended lines
            let mut overlap = 0usize;
            let max_check = prev_lines.len().min(current.len());
            for k in (1..=max_check).rev() {
                if prev_lines[prev_lines.len() - k..] == current[..k] {
                    overlap = k;
                    break;
                }
            }
            let payload = if overlap > 0 && overlap < current.len() {
                serde_json::json!({
                    "mode": "append",
                    "type": lt,
                    "lines": &current[overlap..],
                })
            } else if current.len() > prev_lines.len() && prev_lines.is_empty() {
                serde_json::json!({
                    "mode": "append",
                    "type": lt,
                    "lines": &current,
                })
            } else {
                serde_json::json!({
                    "mode": "snapshot",
                    "type": lt,
                    "lines": &current,
                })
            };
            prev_lines = current;
            if tx.send(Ok(SseEvent::default().data(payload.to_string()))).await.is_err() {
                return;
            }
        }
    });

    Sse::new(tokio_stream::wrappers::ReceiverStream::new(rx))
        .keep_alive(KeepAlive::default())
        .into_response()
}

async fn logs_stats_handler(State(state): State<Arc<AppState>>) -> Json<serde_json::Value> {
    let activity = state
        .activity_log
        .lock()
        .map(|a| a.len())
        .unwrap_or(0);
    let err_lines = read_desktop_log_lines(&state, "error", 5000, None, None).len();
    Json(serde_json::json!({
        "requests_total": activity + err_lines,
        "errors_5xx": err_lines,
        "status_breakdown": {
            "200": activity,
            "500": err_lines
        },
        "top_urls": []
    }))
}

async fn logs_check_errors_handler(State(state): State<Arc<AppState>>) -> Json<serde_json::Value> {
    let activity = state.activity_log.lock().map(|a| a.len()).unwrap_or(0);
    let err_lines = read_desktop_log_lines(&state, "error", 5000, None, None).len();
    let total = (activity + err_lines).max(1);
    let pct = (err_lines as f64 / total as f64) * 100.0;
    Json(serde_json::json!({
        "status": if err_lines == 0 { "healthy" } else { "warnings" },
        "error_rate_percent": (pct * 100.0).round() / 100.0,
        "error_5xx": err_lines,
        "total_requests": activity + err_lines
    }))
}

fn known_log_files(state: &AppState) -> Vec<(&'static str, PathBuf)> {
    let mut files = vec![
        ("Desktop Activity Log", state.config_dir.join("desktop_activity.log")),
        ("PHP Error Log", state.config_dir.join("php_errors.log")),
        ("WebView Console Log", state.config_dir.join("webview_console.log")),
    ];
    if let Ok(app_data) = std::env::var("APPDATA") {
        let global_wv = PathBuf::from(app_data).join("DockPanel").join("webview_console.log");
        if global_wv != state.config_dir.join("webview_console.log") {
            files.push(("Global WebView Log", global_wv));
        }
    }
    files
}

async fn logs_sizes_handler(State(state): State<Arc<AppState>>) -> Json<serde_json::Value> {
    let mut total_bytes: u64 = 0;
    let mut entries = Vec::new();
    for (label, path) in known_log_files(&state) {
        let size_bytes = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
        total_bytes += size_bytes;
        let size_mb = ((size_bytes as f64 / 1_048_576.0) * 100.0).round() / 100.0;
        entries.push(serde_json::json!({
            "path": path.to_string_lossy().to_string(),
            "label": label,
            "size_mb": size_mb,
        }));
    }
    let total_mb = ((total_bytes as f64 / 1_048_576.0) * 100.0).round() / 100.0;
    Json(serde_json::json!({
        "total_mb": total_mb,
        "files": entries,
        "logrotate": false
    }))
}

async fn logs_truncate_handler(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<serde_json::Value>,
) -> Response {
    let target = payload.get("path").and_then(|v| v.as_str()).unwrap_or("");
    let matched = known_log_files(&state)
        .into_iter()
        .map(|(_, p)| p)
        .find(|p| p.to_string_lossy() == target);

    let Some(path) = matched else {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "Only DockPanel desktop log files may be truncated"})),
        )
            .into_response();
    };
    if let Err(e) = std::fs::write(&path, "") {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": e.to_string()})),
        )
            .into_response();
    }
    state.record_event(
        "truncate_log",
        "logs",
        &path.file_name().and_then(|s| s.to_str()).unwrap_or("log"),
        "Log Truncated",
        &format!("Cleared {}", path.display()),
        "info",
    );
    Json(serde_json::json!({ "ok": true })).into_response()
}

#[derive(serde::Deserialize, Default)]
struct ActivityQuery {
    action: Option<String>,
    limit: Option<usize>,
    offset: Option<usize>,
}

async fn activity_list_handler(
    State(state): State<Arc<AppState>>,
    Query(q): Query<ActivityQuery>,
) -> Json<serde_json::Value> {
    let filter = q.action.as_deref().map(str::to_lowercase).filter(|s| !s.is_empty());
    let limit = q.limit.unwrap_or(50).clamp(1, 200);
    let offset = q.offset.unwrap_or(0);
    let items: Vec<serde_json::Value> = state
        .activity_log
        .lock()
        .map(|guard| {
            guard
                .iter()
                .filter(|a| {
                    filter
                        .as_ref()
                        .map(|f| a.target_type.to_lowercase().contains(f) || a.action.to_lowercase().contains(f))
                        .unwrap_or(true)
                })
                .skip(offset)
                .take(limit)
                .map(|a| {
                    serde_json::json!({
                        "id": a.id,
                        "user_email": a.user_email,
                        "action": a.action,
                        "target_type": a.target_type,
                        "target_name": a.target_name,
                        "details": a.details,
                        "ip_address": "127.0.0.1",
                        "created_at": a.created_at,
                        "server_id": serde_json::Value::Null,
                        "server_name": "Local Desktop"
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    Json(serde_json::Value::Array(items))
}

#[derive(serde::Deserialize, Default)]
struct SystemLogsQuery {
    level: Option<String>,
    limit: Option<usize>,
    offset: Option<usize>,
}

async fn system_logs_handler(
    State(state): State<Arc<AppState>>,
    Query(q): Query<SystemLogsQuery>,
) -> Json<serde_json::Value> {
    let level_filter = q.level.as_deref().map(str::to_lowercase).filter(|s| !s.is_empty());
    let limit = q.limit.unwrap_or(50).clamp(1, 200);
    let offset = q.offset.unwrap_or(0);
    let rows: Vec<serde_json::Value> = state
        .activity_log
        .lock()
        .map(|guard| {
            guard
                .iter()
                .map(|a| {
                    let lvl = if a.action.contains("error") || a.action.contains("fail") {
                        "error"
                    } else if a.action.contains("warn") {
                        "warning"
                    } else {
                        "info"
                    };
                    (lvl, a)
                })
                .filter(|(lvl, _)| {
                    level_filter
                        .as_ref()
                        .map(|f| lvl == &f.as_str())
                        .unwrap_or(true)
                })
                .skip(offset)
                .take(limit)
                .map(|(lvl, a)| {
                    serde_json::json!({
                        "id": a.id,
                        "level": lvl,
                        "source": "desktop",
                        "message": a.details,
                        "details": format!("{} ({})", a.action, a.target_name),
                        "created_at": a.created_at
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    Json(serde_json::Value::Array(rows))
}

async fn system_logs_count_handler(State(state): State<Arc<AppState>>) -> Json<serde_json::Value> {
    let (mut error, mut warning, mut info) = (0usize, 0usize, 0usize);
    if let Ok(guard) = state.activity_log.lock() {
        for a in guard.iter() {
            if a.action.contains("error") || a.action.contains("fail") {
                error += 1;
            } else if a.action.contains("warn") {
                warning += 1;
            } else {
                info += 1;
            }
        }
    }
    Json(serde_json::json!({ "error": error, "warning": warning, "info": info }))
}

#[derive(serde::Deserialize, Default)]
struct NotificationsQuery {
    unread: Option<bool>,
    limit: Option<usize>,
}

async fn notifications_list_handler(
    State(state): State<Arc<AppState>>,
    Query(q): Query<NotificationsQuery>,
) -> Json<serde_json::Value> {
    let unread_only = q.unread.unwrap_or(false);
    let limit = q.limit.unwrap_or(50).clamp(1, 200);
    let items: Vec<serde_json::Value> = state
        .notifications
        .lock()
        .map(|guard| {
            guard
                .iter()
                .filter(|n| !unread_only || !n.read)
                .take(limit)
                .map(format_notification_json)
                .collect()
        })
        .unwrap_or_default();
    Json(serde_json::Value::Array(items))
}

async fn notifications_unread_count_handler(State(state): State<Arc<AppState>>) -> Json<serde_json::Value> {
    let count = state
        .notifications
        .lock()
        .map(|guard| guard.iter().filter(|n| !n.read).count())
        .unwrap_or(0);
    Json(serde_json::json!({ "count": count }))
}

async fn notifications_summary_handler(State(state): State<Arc<AppState>>) -> Json<serde_json::Value> {
    let (total, unread) = state
        .notifications
        .lock()
        .map(|guard| (guard.len(), guard.iter().filter(|n| !n.read).count()))
        .unwrap_or((0, 0));
    Json(serde_json::json!({
        "total": total,
        "unread": unread,
        "categories": [{
            "category": "desktop",
            "total": total,
            "unread": unread
        }]
    }))
}

async fn notifications_read_one_handler(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Json<serde_json::Value> {
    if let Ok(mut guard) = state.notifications.lock() {
        for n in guard.iter_mut() {
            if n.id == id {
                n.read = true;
            }
        }
    }
    Json(serde_json::json!({ "ok": true }))
}

async fn notifications_read_all_handler(State(state): State<Arc<AppState>>) -> Json<serde_json::Value> {
    if let Ok(mut guard) = state.notifications.lock() {
        for n in guard.iter_mut() {
            n.read = true;
        }
    }
    Json(serde_json::json!({ "ok": true }))
}

async fn notifications_delete_one_handler(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Json<serde_json::Value> {
    if let Ok(mut guard) = state.notifications.lock() {
        guard.retain(|n| n.id != id);
    }
    Json(serde_json::json!({ "ok": true }))
}

async fn notifications_delete_read_handler(State(state): State<Arc<AppState>>) -> Json<serde_json::Value> {
    if let Ok(mut guard) = state.notifications.lock() {
        guard.retain(|n| !n.read);
    }
    Json(serde_json::json!({ "ok": true }))
}

async fn notifications_stream_handler(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let mut rx_bcast = state.notif_tx.subscribe();
    let (tx, rx) = tokio::sync::mpsc::channel::<Result<SseEvent, std::convert::Infallible>>(16);
    tokio::spawn(async move {
        while let Ok(val) = rx_bcast.recv().await {
            if let Ok(data) = serde_json::to_string(&val) {
                if tx.send(Ok(SseEvent::default().data(data))).await.is_err() {
                    break;
                }
            }
        }
    });
    Sse::new(tokio_stream::wrappers::ReceiverStream::new(rx))
        .keep_alive(KeepAlive::default())
}

async fn get_settings_handler(State(state): State<Arc<AppState>>) -> Json<serde_json::Value> {
    Json(serde_json::json!(state.get_settings()))
}

async fn update_settings_handler(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<serde_json::Value>,
) -> Response {
    let Some(obj) = payload.as_object() else {
        return (StatusCode::BAD_REQUEST, Json(serde_json::json!({"error": "Invalid JSON object"}))).into_response();
    };
    let mut updates = std::collections::HashMap::new();
    for (k, v) in obj {
        updates.insert(k.clone(), json_val_to_string(v));
    }
    if let Some(startup_val) = updates.get("launch_at_startup") {
        let enabled = startup_val.eq_ignore_ascii_case("true") || startup_val == "1";
        #[cfg(not(test))]
        if let Err(e) = tray::set_launch_at_startup(enabled) {
            return (StatusCode::BAD_REQUEST, Json(serde_json::json!({"error": e}))).into_response();
        }
        let _ = enabled;
    }
    let updated = state.update_settings(updates);
    state.record_event(
        "update_settings",
        "settings",
        "desktop",
        "Settings Saved",
        "Updated DockPanel Desktop configuration",
        "info",
    );
    Json(serde_json::json!(updated)).into_response()
}

async fn telemetry_update_status_handler(
    State(state): State<Arc<AppState>>,
    Query(q): Query<std::collections::HashMap<String, String>>,
) -> Json<serde_json::Value> {
    let force = q
        .get("force")
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false);
    Json(updater::check_for_desktop_update(&state, force).await)
}

async fn desktop_update_install_handler(State(state): State<Arc<AppState>>) -> Response {
    match updater::download_and_launch_update(&state).await {
        Ok(val) => {
            state.record_event(
                "desktop_update",
                "system",
                "desktop",
                "Desktop Update Launched",
                &format!("Launched installer for v{}", val["version"].as_str().unwrap_or("")),
                "success",
            );
            Json(val).into_response()
        }
        Err(e) => (StatusCode::BAD_REQUEST, Json(serde_json::json!({ "error": e }))).into_response(),
    }
}

async fn export_settings_handler(State(state): State<Arc<AppState>>) -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "exported_at": chrono::Utc::now().to_rfc3339(),
        "settings": state.get_settings()
    }))
}

async fn health_handler(State(state): State<Arc<AppState>>) -> Json<serde_json::Value> {
    let uptime = format_uptime(state.started_at.elapsed().as_secs());
    let cfg = state.get_router_config();
    let ssl_port = state.get_ssl_port();
    let mdb_port = state.get_mariadb_port();
    Json(serde_json::json!({
        "status": "ok",
        "db": "ok",
        "agent": "ok",
        "database": true,
        "agentOk": true,
        "uptime": uptime,
        "active_port": cfg.active_port,
        "preferred_port": cfg.preferred_port,
        "ssl_port": ssl_port,
        "ssl_enabled": ssl_port.is_some(),
        "mariadb_installed": mariadb::find_mariadbd_binary().is_some(),
        "mariadb_running": mdb_port.is_some(),
        "mariadb_port": mdb_port,
        "default_sites_dir": state.get_default_sites_dir().to_string_lossy().to_string(),
        "version": env!("CARGO_PKG_VERSION")
    }))
}

async fn site_ssl_get_handler(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Response {
    let Some(site) = state.get_site(&id) else {
        return (StatusCode::NOT_FOUND, Json(serde_json::json!({"error": "Site not found"}))).into_response();
    };
    let clean = site.domain.trim().to_lowercase();
    let cert_exists = state.certs_dir().join(&clean).join("cert.pem").is_file();
    let mkcert_ready = ssl::find_mkcert_binary().is_some();
    Json(serde_json::json!({
        "provenance": "dockpanel",
        "issuer": "mkcert Local CA",
        "enabled": cert_exists && state.get_ssl_port().is_some(),
        "cert_exists": cert_exists,
        "mkcert_installed": mkcert_ready,
        "ssl_port": state.get_ssl_port(),
    }))
    .into_response()
}

async fn site_ssl_provision_handler(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Response {
    let Some(site) = state.get_site(&id) else {
        return (StatusCode::NOT_FOUND, Json(serde_json::json!({"error": "Site not found"}))).into_response();
    };
    let st = state.clone();
    let dom = site.domain.clone();
    let res = tokio::task::spawn_blocking(move || st.ensure_site_ssl(&dom)).await;
    match res {
        Ok(Ok(())) => {
            if state.get_ssl_port().is_none() {
                for p in [443u16, 8443] {
                    if router::is_port_bindable(p) && state.restart_ssl_router(p).await {
                        break;
                    }
                }
            }
            state.record_event(
                "provision_ssl",
                "ssl",
                &site.domain,
                "Local SSL Enabled",
                &format!("Issued local mkcert certificate for {}", site.domain),
                "success",
            );
            Json(serde_json::json!({
                "ok": true,
                "ssl_enabled": true,
                "ssl_port": state.get_ssl_port(),
                "message": format!("Trusted local SSL certificate issued for {}", site.domain),
            }))
            .into_response()
        }
        Ok(Err(e)) => (StatusCode::BAD_REQUEST, Json(serde_json::json!({"error": e}))).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({"error": e.to_string()}))).into_response(),
    }
}

async fn mariadb_status_handler(State(state): State<Arc<AppState>>) -> Json<serde_json::Value> {
    let bin = mariadb::find_mariadbd_binary();
    let port = state.get_mariadb_port();
    Json(serde_json::json!({
        "installed": bin.is_some() || port.is_some(),
        "running": port.is_some(),
        "port": port.unwrap_or(3306),
        "bin_path": bin.map(|p| p.to_string_lossy().to_string()),
        "data_dir": mariadb::mariadb_data_dir().to_string_lossy().to_string(),
    }))
}

async fn mariadb_install_handler(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let install_id = uuid::Uuid::new_v4().to_string();
    state.push_install_progress(
        &install_id,
        "mariadb_resolve",
        "Preparing portable MariaDB installation...",
        "in_progress",
        Some(2),
        None,
        false,
    );

    let bg_state = state.clone();
    let iid = install_id.clone();
    tokio::spawn(async move {
        match mariadb::install_mariadb_with_progress(|step, lbl, pct, msg| {
            bg_state.push_install_progress(&iid, step, lbl, "in_progress", pct, msg.as_deref(), false);
        })
        .await
        {
            Ok(ver) => {
                bg_state.push_install_progress(
                    &iid,
                    "mariadb_start",
                    "Starting local MariaDB service...",
                    "in_progress",
                    Some(95),
                    None,
                    false,
                );
                let start_res = tokio::task::spawn_blocking({
                    let st = bg_state.clone();
                    move || st.start_mariadb()
                })
                .await;
                match start_res {
                    Ok(Ok(port)) => {
                        bg_state.record_event(
                            "install_mariadb",
                            "service",
                            "mariadb",
                            "MariaDB Installed & Running",
                            &format!("MariaDB {} started locally on 127.0.0.1:{}", ver, port),
                            "success",
                        );
                        bg_state.push_install_progress(
                            &iid,
                            "complete",
                            &format!("MariaDB ready on 127.0.0.1:{}", port),
                            "done",
                            Some(100),
                            None,
                            true,
                        );
                    }
                    Ok(Err(e)) => {
                        bg_state.push_install_progress(
                            &iid,
                            "complete",
                            &format!("MariaDB extracted, but startup failed: {}", e),
                            "error",
                            None,
                            Some(&e),
                            true,
                        );
                    }
                    Err(e) => {
                        let msg = e.to_string();
                        bg_state.push_install_progress(
                            &iid,
                            "complete",
                            &format!("MariaDB startup error: {}", msg),
                            "error",
                            None,
                            Some(&msg),
                            true,
                        );
                    }
                }
            }
            Err(e) => {
                bg_state.record_event(
                    "install_mariadb_error",
                    "service",
                    "mariadb",
                    "MariaDB Install Failed",
                    &e,
                    "critical",
                );
                bg_state.push_install_progress(
                    &iid,
                    "complete",
                    &format!("MariaDB install failed: {}", e),
                    "error",
                    None,
                    Some(&e),
                    true,
                );
            }
        }
    });

    (
        StatusCode::ACCEPTED,
        Json(serde_json::json!({ "ok": true, "install_id": install_id })),
    )
}

async fn mariadb_start_handler(State(state): State<Arc<AppState>>) -> Response {
    let st = state.clone();
    match tokio::task::spawn_blocking(move || st.start_mariadb()).await {
        Ok(Ok(port)) => {
            state.record_event(
                "start_mariadb",
                "service",
                "mariadb",
                "MariaDB Started",
                &format!("MariaDB listening on 127.0.0.1:{}", port),
                "success",
            );
            Json(serde_json::json!({ "ok": true, "running": true, "port": port })).into_response()
        }
        Ok(Err(e)) => (StatusCode::BAD_REQUEST, Json(serde_json::json!({ "error": e }))).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({ "error": e.to_string() }))).into_response(),
    }
}

async fn mariadb_stop_handler(State(state): State<Arc<AppState>>) -> Json<serde_json::Value> {
    state.stop_mariadb();
    state.record_event(
        "stop_mariadb",
        "service",
        "mariadb",
        "MariaDB Stopped",
        "Stopped local MariaDB server",
        "info",
    );
    Json(serde_json::json!({ "ok": true, "running": false }))
}

async fn services_install_status_handler(State(state): State<Arc<AppState>>) -> Json<serde_json::Value> {
    let php_versions = state.list_php_versions();
    let installed_php = php_versions
        .iter()
        .find(|p| p.get("installed").and_then(|v| v.as_bool()) == Some(true));
    let helper_installed = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.join("dockpanel-host-helper.exe").is_file()))
        .unwrap_or(false)
        || std::path::Path::new(r"C:\Program Files\DockPanel Desktop\dockpanel-host-helper.exe").is_file();
    let workers_running = state
        .worker_processes
        .lock()
        .map(|w| w.values().any(|v| !v.is_empty()))
        .unwrap_or(false);
    let mdb_installed = mariadb::find_mariadbd_binary().is_some();
    let mdb_port = state.get_mariadb_port();

    let node_ver = state::get_node_version();
    let node_running = state.list_sites().iter().any(|s| {
        s.runtime == "node"
            && s.status == "active"
            && state.get_site_worker_port(&s.domain).is_some()
    });

    Json(serde_json::json!({
        "php": {
            "installed": installed_php.is_some(),
            "running": workers_running || installed_php.is_some(),
            "version": installed_php
                .and_then(|p| p.get("version").and_then(|v| v.as_str()))
                .unwrap_or(""),
        },
        "mariadb": {
            "installed": mdb_installed,
            "running": mdb_port.is_some(),
            "port": mdb_port.unwrap_or(3306),
        },
        "host_helper": {
            "installed": helper_installed,
            "running": helper_installed,
        },
        "nodejs": {
            "installed": node_ver.is_some(),
            "running": node_running,
            "version": node_ver.unwrap_or_default(),
        },
        "composer": { "installed": false, "running": false },
        "redis": { "installed": false, "running": false },
        "certbot": { "installed": false },
        "ufw": { "installed": false, "active": false },
        "fail2ban": { "installed": false, "running": false },
        "powerdns": { "installed": false, "running": false },
        "waf": { "installed": false },
        "cloudflared": { "installed": false, "running": false },
        "sshpass": { "installed": false }
    }))
}

async fn desktop_mail_status_handler(State(state): State<Arc<AppState>>) -> Json<serde_json::Value> {
    Json(state.mail_catcher.status())
}

async fn desktop_mail_list_handler(
    State(state): State<Arc<AppState>>,
    Query(q): Query<std::collections::HashMap<String, String>>,
) -> Json<serde_json::Value> {
    let search = q.get("q").map(|s| s.as_str());
    let domain = q.get("domain").map(|s| s.as_str());
    let messages = state.mail_catcher.list_summaries(search, domain);
    Json(serde_json::json!({
        "messages": messages,
        "status": state.mail_catcher.status(),
    }))
}

async fn desktop_mail_get_handler(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Response {
    match state.mail_catcher.get_message(&id) {
        Some(msg) => {
            state.mail_catcher.mark_read(&id, true);
            let mut updated = msg;
            updated.read = true;
            Json(updated).into_response()
        }
        None => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": "Message not found" })),
        )
            .into_response(),
    }
}

async fn desktop_mail_mark_read_handler(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(payload): Json<Option<serde_json::Value>>,
) -> Response {
    let read = payload
        .as_ref()
        .and_then(|p| p.get("read"))
        .and_then(|v| v.as_bool())
        .unwrap_or(true);
    if state.mail_catcher.mark_read(&id, read) {
        Json(serde_json::json!({ "ok": true, "read": read })).into_response()
    } else {
        (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": "Message not found" })),
        )
            .into_response()
    }
}

async fn desktop_mail_read_all_handler(State(state): State<Arc<AppState>>) -> Json<serde_json::Value> {
    state.mail_catcher.mark_all_read();
    Json(serde_json::json!({ "ok": true }))
}

async fn desktop_mail_delete_handler(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Response {
    if state.mail_catcher.delete_message(&id) {
        Json(serde_json::json!({ "ok": true })).into_response()
    } else {
        (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": "Message not found" })),
        )
            .into_response()
    }
}

async fn desktop_mail_clear_handler(State(state): State<Arc<AppState>>) -> Json<serde_json::Value> {
    let cleared = state.mail_catcher.clear_messages();
    Json(serde_json::json!({ "ok": true, "cleared": cleared }))
}

async fn desktop_mail_test_handler(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<Option<serde_json::Value>>,
) -> Response {
    let Some(port) = state.mail_catcher.smtp_port() else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(serde_json::json!({ "error": "Local SMTP catcher is not running" })),
        )
            .into_response();
    };
    let site_domain = payload
        .as_ref()
        .and_then(|p| p.get("site_domain"))
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(String::from)
        .or_else(|| state.list_sites().into_iter().next().map(|s| s.domain));
    let domain_label = site_domain.as_deref().unwrap_or("dockpanel.desktop");
    let from = payload
        .as_ref()
        .and_then(|p| p.get("from"))
        .and_then(|v| v.as_str())
        .filter(|s| !s.trim().is_empty())
        .map(String::from)
        .unwrap_or_else(|| format!("wordpress@{}", domain_label));
    let to = payload
        .as_ref()
        .and_then(|p| p.get("to"))
        .and_then(|v| v.as_str())
        .filter(|s| !s.trim().is_empty())
        .unwrap_or("developer@localhost");
    let subject = payload
        .as_ref()
        .and_then(|p| p.get("subject"))
        .and_then(|v| v.as_str())
        .filter(|s| !s.trim().is_empty())
        .map(String::from)
        .unwrap_or_else(|| format!("Test email from {}", domain_label));
    let text = payload
        .as_ref()
        .and_then(|p| p.get("text"))
        .and_then(|v| v.as_str())
        .filter(|s| !s.trim().is_empty())
        .map(String::from)
        .unwrap_or_else(|| {
            format!(
                "This test email was captured locally by DockPanel Desktop on 127.0.0.1:{}.\nSite: {}",
                port, domain_label
            )
        });
    let html = format!(
        "<div style=\"font-family:sans-serif;padding:16px;\"><h2>DockPanel Local Mail Catcher</h2><p>Captured locally on <code>127.0.0.1:{}</code>.</p><p><strong>Site:</strong> {}</p></div>",
        port,
        domain_label
    );
    match mail_catcher::send_test_email_smtp(port, &from, to, &subject, &text, Some(&html)).await {
        Ok(()) => Json(serde_json::json!({ "ok": true, "smtp_port": port })).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e })),
        )
            .into_response(),
    }
}

#[cfg(windows)]
mod win_metrics {
    #[repr(C)]
    #[allow(non_snake_case)]
    struct MEMORYSTATUSEX {
        dwLength: u32,
        dwMemoryLoad: u32,
        ullTotalPhys: u64,
        ullAvailPhys: u64,
        ullTotalPageFile: u64,
        ullAvailPageFile: u64,
        ullTotalVirtual: u64,
        ullAvailVirtual: u64,
        ullAvailExtendedVirtual: u64,
    }

    #[repr(C)]
    #[allow(non_snake_case)]
    #[derive(Clone, Copy, Default)]
    struct FILETIME {
        dwLowDateTime: u32,
        dwHighDateTime: u32,
    }

    #[repr(C)]
    #[allow(non_snake_case)]
    struct PROCESSENTRY32W {
        dwSize: u32,
        cntUsage: u32,
        th32ProcessID: u32,
        th32DefaultHeapID: usize,
        th32ModuleID: u32,
        cntThreads: u32,
        th32ParentProcessID: u32,
        pcPriClassBase: i32,
        dwFlags: u32,
        szExeFile: [u16; 260],
    }

    #[repr(C)]
    #[allow(non_snake_case)]
    #[derive(Default)]
    struct PROCESS_MEMORY_COUNTERS {
        cb: u32,
        PageFaultCount: u32,
        PeakWorkingSetSize: usize,
        WorkingSetSize: usize,
        QuotaPeakPagedPoolUsage: usize,
        QuotaPagedPoolUsage: usize,
        QuotaPeakNonPagedPoolUsage: usize,
        QuotaNonPagedPoolUsage: usize,
        PagefileUsage: usize,
        PeakPagefileUsage: usize,
    }

    #[repr(C)]
    #[allow(non_snake_case)]
    struct MIB_IFROW {
        wszName: [u16; 256],
        dwIndex: u32,
        dwType: u32,
        dwMtu: u32,
        dwSpeed: u32,
        dwPhysAddrLen: u32,
        bPhysAddr: [u8; 8],
        dwAdminStatus: u32,
        dwOperStatus: u32,
        dwLastChange: u32,
        dwInOctets: u32,
        dwInUcastPkts: u32,
        dwInNUcastPkts: u32,
        dwInDiscards: u32,
        dwInErrors: u32,
        dwInUnknownProtos: u32,
        dwOutOctets: u32,
        dwOutUcastPkts: u32,
        dwOutNUcastPkts: u32,
        dwOutDiscards: u32,
        dwOutErrors: u32,
        dwOutQLen: u32,
        dwDescrLen: u32,
        bDescr: [u8; 256],
    }

    #[link(name = "kernel32")]
    extern "system" {
        fn GlobalMemoryStatusEx(lpBuffer: *mut MEMORYSTATUSEX) -> i32;
        fn GetDiskFreeSpaceExW(
            lpDirectoryName: *const u16,
            lpFreeBytesAvailableToCaller: *mut u64,
            lpTotalNumberOfBytes: *mut u64,
            lpTotalNumberOfFreeBytes: *mut u64,
        ) -> i32;
        fn GetTickCount64() -> u64;
        fn GetSystemTimes(
            lpIdleTime: *mut FILETIME,
            lpKernelTime: *mut FILETIME,
            lpUserTime: *mut FILETIME,
        ) -> i32;
        fn CreateToolhelp32Snapshot(dwFlags: u32, th32ProcessID: u32) -> isize;
        fn Process32FirstW(hSnapshot: isize, lppe: *mut PROCESSENTRY32W) -> i32;
        fn Process32NextW(hSnapshot: isize, lppe: *mut PROCESSENTRY32W) -> i32;
        fn OpenProcess(dwDesiredAccess: u32, bInheritHandle: i32, dwProcessId: u32) -> isize;
        fn CloseHandle(hObject: isize) -> i32;
        fn K32GetProcessMemoryInfo(
            Process: isize,
            ppsmemCounters: *mut PROCESS_MEMORY_COUNTERS,
            cb: u32,
        ) -> i32;
    }

    #[link(name = "iphlpapi")]
    extern "system" {
        fn GetIfTable(pIfTable: *mut u8, pdwSize: *mut u32, bOrder: i32) -> u32;
    }

    static PREV_CPU_TIMES: std::sync::Mutex<Option<(u64, u64, u64)>> = std::sync::Mutex::new(None);
    static PREV_PROC_TIMES: std::sync::Mutex<Option<std::collections::HashMap<u32, u64>>> =
        std::sync::Mutex::new(None);

    fn ft_to_u64(ft: FILETIME) -> u64 {
        ((ft.dwHighDateTime as u64) << 32) | (ft.dwLowDateTime as u64)
    }

    pub fn collect_cpu_usage_pct() -> f64 {
        let mut idle = FILETIME::default();
        let mut kernel = FILETIME::default();
        let mut user = FILETIME::default();
        if unsafe { GetSystemTimes(&mut idle, &mut kernel, &mut user) } == 0 {
            return 0.0;
        }
        let cur = (ft_to_u64(idle), ft_to_u64(kernel), ft_to_u64(user));
        let mut prev_guard = PREV_CPU_TIMES.lock().unwrap();
        let prev = match *prev_guard {
            Some(p) => {
                *prev_guard = Some(cur);
                p
            }
            None => {
                *prev_guard = Some(cur);
                drop(prev_guard);
                std::thread::sleep(std::time::Duration::from_millis(80));
                if unsafe { GetSystemTimes(&mut idle, &mut kernel, &mut user) } == 0 {
                    return 0.0;
                }
                let cur2 = (ft_to_u64(idle), ft_to_u64(kernel), ft_to_u64(user));
                *PREV_CPU_TIMES.lock().unwrap() = Some(cur2);
                let idle_d = cur2.0.saturating_sub(cur.0);
                let total_d = cur2.1.saturating_sub(cur.1) + cur2.2.saturating_sub(cur.2);
                return if total_d > 0 {
                    (((total_d.saturating_sub(idle_d)) as f64 / total_d as f64) * 100.0)
                        .clamp(0.0, 100.0)
                } else {
                    0.0
                };
            }
        };
        let idle_d = cur.0.saturating_sub(prev.0);
        let total_d = cur.1.saturating_sub(prev.1) + cur.2.saturating_sub(prev.2);
        if total_d > 0 {
            (((total_d.saturating_sub(idle_d)) as f64 / total_d as f64) * 100.0).clamp(0.0, 100.0)
        } else {
            0.0
        }
    }

    pub fn list_processes(limit: usize) -> (Vec<serde_json::Value>, usize) {
        const TH32CS_SNAPPROCESS: u32 = 0x00000002;
        const INVALID_HANDLE: isize = -1;
        const PROCESS_QUERY_LIMITED_INFORMATION: u32 = 0x1000;
        const PROCESS_VM_READ: u32 = 0x0010;

        let snap = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) };
        if snap == INVALID_HANDLE || snap == 0 {
            return (Vec::new(), 0);
        }

        let mut pe = PROCESSENTRY32W {
            dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32,
            cntUsage: 0,
            th32ProcessID: 0,
            th32DefaultHeapID: 0,
            th32ModuleID: 0,
            cntThreads: 0,
            th32ParentProcessID: 0,
            pcPriClassBase: 0,
            dwFlags: 0,
            szExeFile: [0; 260],
        };

        let mut rows: Vec<(u32, String, f64, f64)> = Vec::new();
        let mut total_count = 0usize;
        let mut new_times = std::collections::HashMap::new();
        let prev_times = PREV_PROC_TIMES.lock().unwrap().clone().unwrap_or_default();

        if unsafe { Process32FirstW(snap, &mut pe) } != 0 {
            loop {
                total_count += 1;
                let pid = pe.th32ProcessID;
                let end = pe
                    .szExeFile
                    .iter()
                    .position(|&c| c == 0)
                    .unwrap_or(pe.szExeFile.len());
                let name = String::from_utf16_lossy(&pe.szExeFile[..end]);
                if pid > 0 && !name.is_empty() {
                    let h = unsafe {
                        OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_VM_READ, 0, pid)
                    };
                    let mem_mb = if h != 0 {
                        let mut pmc = PROCESS_MEMORY_COUNTERS {
                            cb: std::mem::size_of::<PROCESS_MEMORY_COUNTERS>() as u32,
                            ..Default::default()
                        };
                        let ok = unsafe {
                            K32GetProcessMemoryInfo(
                                h,
                                &mut pmc,
                                std::mem::size_of::<PROCESS_MEMORY_COUNTERS>() as u32,
                            )
                        };
                        unsafe { CloseHandle(h) };
                        if ok != 0 {
                            (pmc.WorkingSetSize as f64) / (1024.0 * 1024.0)
                        } else {
                            0.0
                        }
                    } else {
                        0.0
                    };
                    let _ = prev_times.get(&pid);
                    new_times.insert(pid, 0u64);
                    if mem_mb > 0.1 {
                        rows.push((pid, name, 0.0, mem_mb));
                    }
                }
                if unsafe { Process32NextW(snap, &mut pe) } == 0 {
                    break;
                }
            }
        }
        unsafe { CloseHandle(snap) };
        *PREV_PROC_TIMES.lock().unwrap() = Some(new_times);

        rows.sort_by(|a, b| b.3.partial_cmp(&a.3).unwrap_or(std::cmp::Ordering::Equal));
        let top = rows
            .into_iter()
            .take(limit)
            .map(|(pid, name, cpu_pct, mem_mb)| {
                serde_json::json!({
                    "pid": pid,
                    "name": name,
                    "cpu_pct": (cpu_pct * 10.0).round() / 10.0,
                    "mem_mb": (mem_mb * 10.0).round() / 10.0,
                })
            })
            .collect();
        (top, total_count)
    }

    pub fn list_network() -> Vec<serde_json::Value> {
        const ERROR_INSUFFICIENT_BUFFER: u32 = 122;
        const NO_ERROR: u32 = 0;

        let mut size: u32 = 0;
        let rc = unsafe { GetIfTable(std::ptr::null_mut(), &mut size, 1) };
        if rc != ERROR_INSUFFICIENT_BUFFER && rc != NO_ERROR {
            return Vec::new();
        }
        if size < 4 {
            return Vec::new();
        }
        let mut buf = vec![0u8; size as usize];
        let rc2 = unsafe { GetIfTable(buf.as_mut_ptr(), &mut size, 1) };
        if rc2 != NO_ERROR {
            return Vec::new();
        }
        let num_entries = u32::from_le_bytes([buf[0], buf[1], buf[2], buf[3]]) as usize;
        let row_size = std::mem::size_of::<MIB_IFROW>();
        let mut out = Vec::new();
        let mut seen = std::collections::HashSet::new();
        for i in 0..num_entries {
            let offset = 4 + i * row_size;
            if offset + row_size > buf.len() {
                break;
            }
            let row = unsafe { std::ptr::read_unaligned(buf.as_ptr().add(offset) as *const MIB_IFROW) };
            // Skip software loopback (dwType == 24) if zero traffic, or show active ethernet/wifi/loopback
            let dlen = (row.dwDescrLen as usize).min(row.bDescr.len());
            let descr = String::from_utf8_lossy(&row.bDescr[..dlen])
                .trim_matches('\0')
                .trim()
                .to_string();
            if descr.is_empty() || (row.dwInOctets == 0 && row.dwOutOctets == 0) {
                continue;
            }
            if !seen.insert(descr.clone()) {
                continue;
            }
            out.push(serde_json::json!({
                "name": descr,
                "rx_bytes": row.dwInOctets as u64,
                "tx_bytes": row.dwOutOctets as u64,
            }));
        }
        out
    }

    pub struct HostMetrics {
        pub cpu_usage: f64,
        pub mem_total_mb: u64,
        pub mem_used_mb: u64,
        pub mem_usage_pct: f64,
        pub disk_total_gb: f64,
        pub disk_used_gb: f64,
        pub disk_usage_pct: f64,
        pub os_uptime_secs: u64,
    }

    pub fn collect(drive_root: &std::path::Path) -> HostMetrics {
        let cpu_usage = collect_cpu_usage_pct();
        let mut mem_status = MEMORYSTATUSEX {
            dwLength: std::mem::size_of::<MEMORYSTATUSEX>() as u32,
            dwMemoryLoad: 0,
            ullTotalPhys: 0,
            ullAvailPhys: 0,
            ullTotalPageFile: 0,
            ullAvailPageFile: 0,
            ullTotalVirtual: 0,
            ullAvailVirtual: 0,
            ullAvailExtendedVirtual: 0,
        };
        let (mem_total_mb, mem_used_mb, mem_usage_pct) = unsafe {
            if GlobalMemoryStatusEx(&mut mem_status) != 0 && mem_status.ullTotalPhys > 0 {
                let total = mem_status.ullTotalPhys / (1024 * 1024);
                let avail = mem_status.ullAvailPhys / (1024 * 1024);
                let used = total.saturating_sub(avail);
                let pct = (used as f64 / total as f64) * 100.0;
                (total, used, pct)
            } else {
                (16384, 4096, 25.0)
            }
        };

        let root_str = drive_root
            .components()
            .next()
            .map(|c| format!("{}\\", c.as_os_str().to_string_lossy()))
            .unwrap_or_else(|| "C:\\".to_string());
        let wide: Vec<u16> = root_str.encode_utf16().chain(std::iter::once(0)).collect();
        let mut free_caller: u64 = 0;
        let mut total_bytes: u64 = 0;
        let mut free_total: u64 = 0;
        let (disk_total_gb, disk_used_gb, disk_usage_pct) = unsafe {
            if GetDiskFreeSpaceExW(
                wide.as_ptr(),
                &mut free_caller,
                &mut total_bytes,
                &mut free_total,
            ) != 0
                && total_bytes > 0
            {
                let gb = 1024.0 * 1024.0 * 1024.0;
                let total_gb = total_bytes as f64 / gb;
                let used_gb = total_bytes.saturating_sub(free_total) as f64 / gb;
                let pct = (used_gb / total_gb) * 100.0;
                (total_gb, used_gb, pct)
            } else {
                (256.0, 64.0, 25.0)
            }
        };

        let os_uptime_secs = unsafe { GetTickCount64() / 1000 };
        HostMetrics {
            cpu_usage,
            mem_total_mb,
            mem_used_mb,
            mem_usage_pct,
            disk_total_gb,
            disk_used_gb,
            disk_usage_pct,
            os_uptime_secs,
        }
    }
}

async fn system_info_handler(State(state): State<Arc<AppState>>) -> Json<serde_json::Value> {
    let cpu_count = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4);
    let hostname = std::env::var("COMPUTERNAME")
        .or_else(|_| std::env::var("HOSTNAME"))
        .unwrap_or_else(|_| "localhost".into());
    let worker_count: usize = state
        .worker_processes
        .lock()
        .map(|w| w.values().map(|v| v.len()).sum())
        .unwrap_or(0);

    #[cfg(windows)]
    let (cpu_usage, mem_total_mb, mem_used_mb, mem_usage_pct, disk_total_gb, disk_used_gb, disk_usage_pct, uptime_secs, proc_total) = {
        let m = win_metrics::collect(&state.sites_dir);
        let (_top, total_procs) = win_metrics::list_processes(1);
        (
            (m.cpu_usage * 10.0).round() / 10.0,
            m.mem_total_mb,
            m.mem_used_mb,
            m.mem_usage_pct,
            m.disk_total_gb,
            m.disk_used_gb,
            m.disk_usage_pct,
            m.os_uptime_secs.max(state.started_at.elapsed().as_secs()),
            total_procs.max(worker_count + 1),
        )
    };
    #[cfg(not(windows))]
    let (cpu_usage, mem_total_mb, mem_used_mb, mem_usage_pct, disk_total_gb, disk_used_gb, disk_usage_pct, uptime_secs, proc_total) = (
        0.0f64,
        16384u64,
        4096u64,
        25.0f64,
        256.0f64,
        64.0f64,
        25.0f64,
        state.started_at.elapsed().as_secs(),
        worker_count + 1,
    );

    Json(serde_json::json!({
        "cpu_count": cpu_count,
        "cpu_usage": cpu_usage,
        "cpu_model": "Windows Host CPU",
        "cpu_temp": null,
        "mem_total_mb": mem_total_mb,
        "mem_used_mb": mem_used_mb,
        "mem_usage_pct": mem_usage_pct,
        "swap_total_mb": 0,
        "swap_used_mb": 0,
        "disk_total_gb": disk_total_gb,
        "disk_used_gb": disk_used_gb,
        "disk_usage_pct": disk_usage_pct,
        "load_avg_1": 0.0,
        "load_avg_5": 0.0,
        "load_avg_15": 0.0,
        "uptime_secs": uptime_secs,
        "hostname": hostname,
        "os": "Windows",
        "kernel": std::env::consts::ARCH,
        "process_count": proc_total
    }))
}

async fn metrics_handler(State(state): State<Arc<AppState>>) -> Json<serde_json::Value> {
    #[cfg(windows)]
    {
        let m = win_metrics::collect(&state.sites_dir);
        Json(serde_json::json!({
            "cpu_percent": (m.cpu_usage * 10.0).round() / 10.0,
            "memory_mb": m.mem_used_mb,
            "disk_gb": (m.disk_used_gb * 10.0).round() / 10.0,
        }))
    }
    #[cfg(not(windows))]
    {
        let _ = state;
        Json(serde_json::json!({"cpu_percent": 0.0, "memory_mb": 0, "disk_gb": 0.0}))
    }
}

async fn system_processes_handler() -> Json<serde_json::Value> {
    #[cfg(windows)]
    {
        let (procs, _total) = tokio::task::spawn_blocking(|| win_metrics::list_processes(20))
            .await
            .unwrap_or_default();
        Json(serde_json::Value::Array(procs))
    }
    #[cfg(not(windows))]
    {
        Json(serde_json::json!([]))
    }
}

async fn system_network_handler() -> Json<serde_json::Value> {
    #[cfg(windows)]
    {
        let ifaces = tokio::task::spawn_blocking(win_metrics::list_network)
            .await
            .unwrap_or_default();
        Json(serde_json::Value::Array(ifaces))
    }
    #[cfg(not(windows))]
    {
        Json(serde_json::json!([]))
    }
}

pub fn build_api_router(state: Arc<AppState>) -> Router {
    Router::new()
        .route(
            "/auth/setup-status",
            get(|| async { Json(serde_json::json!({"needs_setup": false})) }),
        )
        .route(
            "/auth/me",
            get(|| async {
                Json(serde_json::json!({
                    "id": "00000000-0000-0000-0000-000000000001",
                    "email": "local@dockpanel.desktop",
                    "role": "admin"
                }))
            }),
        )
        .route(
            "/auth/desktop-bootstrap",
            post(|| async {
                Json(serde_json::json!({
                    "token": "desktop-local-token",
                    "user": {
                        "id": "00000000-0000-0000-0000-000000000001",
                        "email": "local@dockpanel.desktop",
                        "role": "admin"
                    }
                }))
            }),
        )
        .route(
            "/auth/2fa/status",
            get(|| async { Json(serde_json::json!({"enabled": false})) }),
        )
        .route(
            "/auth/passkeys",
            get(|| async { Json(serde_json::json!({"passkeys": []})) }),
        )
        .route(
            "/auth/sessions",
            get(|| async { Json(serde_json::json!([])) }),
        )
        .route(
            "/branding",
            get({
                let s = state.clone();
                move || async move {
                    let cfg = s.get_settings();
                    let panel_name = cfg
                        .get("panel_name")
                        .cloned()
                        .unwrap_or_else(|| "DockPanel Desktop".to_string());
                    let logo_url = cfg.get("logo_url").cloned().unwrap_or_default();
                    let accent_color = cfg.get("accent_color").cloned().unwrap_or_default();
                    let hide_branding = cfg.get("hide_branding").map(|v| v == "true").unwrap_or(false);
                    Json(serde_json::json!({
                        "panel_name": panel_name,
                        "logo_url": logo_url,
                        "accent_color": accent_color,
                        "hide_branding": hide_branding,
                        "oauth_providers": []
                    }))
                }
            }),
        )
        .route(
            "/servers",
            get(|| async {
                Json(serde_json::json!([{
                    "id": "local",
                    "name": "Local Desktop",
                    "ip_address": "127.0.0.1",
                    "agent_url": null,
                    "status": "online",
                    "is_local": true,
                    "os_info": "Windows",
                    "cpu_cores": 8,
                    "ram_mb": 16384,
                    "disk_gb": 512,
                    "agent_version": env!("CARGO_PKG_VERSION"),
                    "cpu_usage": 5.0,
                    "mem_used_mb": 2048,
                    "uptime_secs": 3600,
                    "last_seen_at": "2026-11-24T00:00:00Z",
                    "cert_fingerprint": null,
                    "created_at": "2026-11-24T00:00:00Z"
                }]))
            }),
        )
        .route("/sites", get(list_sites_handler).post(create_site_handler))
        .route("/admin/sites", get(list_sites_handler))
        .route(
            "/sites/{id}",
            get(get_site_handler).delete(delete_site_handler),
        )
        .route("/sites/{id}/start", post(start_site_handler))
        .route("/sites/{id}/stop", post(stop_site_handler))
        .route("/sites/{id}/php", put(switch_site_php_handler))
        .route("/sites/{id}/runtime", put(switch_site_runtime_handler))
        .route("/sites/{id}/app/status", get(site_app_status_handler))
        .route("/sites/{id}/app/restart", post(site_app_restart_handler))
        .route("/sites/{id}/app/logs", get(site_app_logs_handler))
        .route("/sites/{id}/provision-log", get(site_provision_log_handler))
        .route("/sites/{id}/health", get(site_health_handler))
        .route("/sites/{id}/health-summary", get(site_health_summary_handler))
        .route("/sites/{id}/toggle", put(toggle_site_handler))
        .route("/sites/{id}/domain", put(rename_site_handler))
        .route("/sites/{id}/clone", post(clone_site_handler))
        .route("/sites/{id}/limits", put(update_site_limits_handler))
        .route("/sites/{id}/env", get(site_env_get_handler).put(site_env_put_handler))
        .route("/sites/{id}/staging", get(|| async { Json(serde_json::json!({"exists": false})) }))
        .route(
            "/sites/{id}/redirects",
            get(site_redirects_get_handler).post(site_redirects_add_handler),
        )
        .route(
            "/sites/{id}/redirects/remove",
            post(site_redirects_remove_handler),
        )
        .route(
            "/sites/{id}/aliases",
            get(site_aliases_get_handler).post(site_aliases_add_handler),
        )
        .route(
            "/sites/{id}/aliases/remove",
            post(site_aliases_remove_handler),
        )
        .route(
            "/sites/{id}/password-protect",
            get(site_pass_protect_get_handler).post(site_pass_protect_add_handler),
        )
        .route(
            "/sites/{id}/password-protect/remove",
            post(site_pass_protect_remove_handler),
        )
        .route("/sites/{id}/stats", get(site_stats_handler))
        .route("/sites/{id}/bandwidth-usage", get(site_bandwidth_handler))
        .route(
            "/sites/{id}/access-logs",
            get(|State(s): State<Arc<AppState>>, Path(id): Path<String>, Query(q): Query<LogQueryParams>| async move {
                let lt = q.log_type.as_deref().unwrap_or("access");
                let limit = q.lines.or(q.max).unwrap_or(200);
                site_log_text_handler(s, id, lt, limit).await
            }),
        )
        .route(
            "/sites/{id}/php-errors",
            get(|State(s): State<Arc<AppState>>, Path(id): Path<String>, Query(q): Query<LogQueryParams>| async move {
                let limit = q.lines.or(q.max).unwrap_or(200);
                site_log_text_handler(s, id, "php", limit).await
            }),
        )
        .route("/sites/{id}/logs", get(site_logs_read_handler))
        .route("/sites/{id}/logs/search", get(site_logs_read_handler))
        .route("/sites/{id}/logs/stream", get(site_logs_stream_handler))
        .route("/sites/{id}/ssl", get(site_ssl_get_handler))
        .route("/sites/{id}/ssl/provision", post(site_ssl_provision_handler))
        .route("/sites/{id}/ssl/renew", post(site_ssl_provision_handler))
        .route("/sites/{id}/crons", get(|| async { Json(serde_json::json!([])) }))
        .route(
            "/sites/{id}/backups",
            get(list_backups_handler).post(create_backup_handler),
        )
        .route(
            "/sites/{id}/backups/{bid}/restore",
            post(restore_backup_handler),
        )
        .route(
            "/sites/{id}/backups/{bid}/download",
            get(download_backup_handler),
        )
        .route("/sites/{id}/backups/{bid}", delete(delete_backup_handler))
        .route("/backup-destinations/selectable", get(|| async { Json(serde_json::json!([])) }))
        .route(
            "/sites/{id}/backup-schedule",
            get(|| async { Json(serde_json::Value::Null) })
                .put(|| async { Json(serde_json::json!({"ok": true})) })
                .delete(|| async { Json(serde_json::json!({"ok": true})) }),
        )
        .route("/sites/{id}/files", get(files_list_handler).delete(files_delete_handler))
        .route("/sites/{id}/files/tree", get(files_tree_handler))
        .route("/sites/{id}/files/read", get(files_read_handler))
        .route("/sites/{id}/files/write", put(files_write_handler))
        .route("/sites/{id}/files/create", post(files_create_handler))
        .route("/sites/{id}/files/rename", post(files_rename_handler))
        .route("/sites/{id}/files/delete-batch", post(files_delete_batch_handler))
        .route("/sites/{id}/files/download", get(files_download_handler))
        .route(
            "/sites/{id}/files/copy",
            post(|State(s): State<Arc<AppState>>, Path(id): Path<String>, Json(p): Json<serde_json::Value>| async move {
                files_copy_or_move_handler(s, id, p, false).await
            }),
        )
        .route(
            "/sites/{id}/files/move",
            post(|State(s): State<Arc<AppState>>, Path(id): Path<String>, Json(p): Json<serde_json::Value>| async move {
                files_copy_or_move_handler(s, id, p, true).await
            }),
        )
        .route("/sites/{id}/files/compress", post(files_compress_handler))
        .route("/sites/{id}/files/extract", post(files_extract_handler))
        .route("/wordpress/sites", get(wordpress_sites_handler))
        .route("/sites/{id}/wordpress", get(get_wordpress_info_handler))
        .route("/sites/{id}/wordpress/info", get(get_wordpress_info_handler))
        .route("/sites/{id}/wordpress/install", post(install_wordpress_handler))
        .route(
            "/sites/{id}/wordpress/reinstall-core",
            post(reinstall_wordpress_core_handler),
        )
        .route(
            "/sites/{id}/wordpress/plugins",
            get(wordpress_plugins_handler),
        )
        .route(
            "/sites/{id}/wordpress/themes",
            get(wordpress_themes_handler),
        )
        .route(
            "/sites/{id}/wordpress/db-info",
            get(wordpress_db_info_handler),
        )
        .route(
            "/sites/{id}/wordpress/features-status",
            get(wordpress_features_status_handler),
        )
        .route(
            "/sites/{id}/wordpress/toggle-feature",
            post(wordpress_toggle_feature_handler),
        )
        .route(
            "/sites/{id}/wordpress/maintenance",
            post(wordpress_maintenance_handler),
        )
        .route(
            "/sites/{id}/wordpress/setup",
            post(wordpress_setup_handler),
        )
        .route(
            "/sites/{id}/wordpress/clear-cache",
            post(wordpress_clear_cache_handler),
        )
        .route("/wordpress/bulk-update", post(wordpress_bulk_update_handler))
        .route(
            "/sites/{id}/wordpress/vuln-scan",
            post(wordpress_vuln_scan_handler),
        )
        .route(
            "/sites/{id}/wordpress/security-check",
            get(wordpress_security_check_handler),
        )
        .route(
            "/sites/{id}/wordpress/hardening-history",
            get(wordpress_hardening_history_handler),
        )
        .route(
            "/sites/{id}/wordpress/reset-admin-password",
            post(wordpress_reset_admin_password_handler),
        )
        .route(
            "/sites/{id}/wordpress/optimize-db",
            post(wordpress_optimize_db_handler),
        )
        .route(
            "/sites/{id}/wordpress/plugin/{action}",
            post(wordpress_plugin_action_handler),
        )
        .route(
            "/sites/{id}/wordpress/theme/{action}",
            post(wordpress_theme_action_handler),
        )
        .route(
            "/sites/{id}/wordpress/harden",
            post(wordpress_harden_handler),
        )
        .route(
            "/sites/{id}/wordpress/integrity",
            get(wordpress_integrity_handler).post(wordpress_integrity_handler),
        )
        .route(
            "/sites/{id}/wordpress/cli",
            post(wordpress_cli_handler),
        )
        .route(
            "/sites/{id}/wordpress/update/{target}",
            post(wordpress_update_handler),
        )
        .route(
            "/sites/{id}/wordpress/update-safe",
            post(wordpress_update_safe_handler),
        )
        .route("/desktop/open-folder", post(desktop_open_folder_handler))
        .route("/desktop/open-vscode", post(desktop_open_vscode_handler))
        .route("/desktop/pick-folder", post(desktop_pick_folder_handler))
        .route("/desktop/inspect-folder", post(desktop_inspect_folder_handler))
        .route("/desktop/discover-folders", get(desktop_discover_folders_handler))
        .route("/desktop/scan-db-files", post(desktop_scan_db_files_handler))
        .route("/desktop/clear-cache", post(desktop_clear_cache_handler))
        .route(
            "/sites/{id}/wordpress/import-database",
            post(wordpress_import_database_handler)
                .layer(axum::extract::DefaultBodyLimit::max(1024 * 1024 * 1024)),
        )
        .route(
            "/sites/{id}/database/import",
            post(site_import_database_handler)
                .layer(axum::extract::DefaultBodyLimit::max(1024 * 1024 * 1024)),
        )
        .route("/sites/{id}/git/status", get(site_git_status_handler))
        .route("/sites/{id}/git/init", post(site_git_init_handler))
        .route("/sites/{id}/git/connect", post(site_git_connect_handler))
        .route("/sites/{id}/git/pull", post(site_git_pull_handler))
        .route("/sites/{id}/git/commit-push", post(site_git_commit_push_handler))
        .route("/sites/{id}/git/disconnect", post(site_git_disconnect_handler))
        .route(
            "/sites/{id}/wordpress/import-log/{job_id}",
            get(|State(s): State<Arc<AppState>>, Path((_id, job_id)): Path<(String, String)>| async move {
                php_install_log_handler(State(s), Path(job_id)).await
            }),
        )
        .route("/desktop/mail/status", get(desktop_mail_status_handler))
        .route(
            "/desktop/mail/messages",
            get(desktop_mail_list_handler).delete(desktop_mail_clear_handler),
        )
        .route("/desktop/mail/messages/read-all", post(desktop_mail_read_all_handler))
        .route(
            "/desktop/mail/messages/{id}",
            get(desktop_mail_get_handler).delete(desktop_mail_delete_handler),
        )
        .route("/desktop/mail/messages/{id}/read", post(desktop_mail_mark_read_handler))
        .route("/desktop/mail/test", post(desktop_mail_test_handler))
        .route("/system/ports/conflict", get(port_conflict_handler))
        .route("/system/ports/resolve", post(port_resolve_handler))
        .route("/sites/metrics-overview", get(metrics_overview_handler))
        .route(
            "/sites/{id}/sync/preflight",
            post(|State(s): State<Arc<AppState>>, Path(id): Path<String>, Json(req): Json<live_sync::PreflightCheckRequest>| async move {
                let local_domain = s.get_site(&id).map(|site| site.domain);
                Json(live_sync::check_preflight(&req, local_domain.as_deref()).await)
            }),
        )
        .route(
            "/sites/{id}/sync/push",
            post(|State(s): State<Arc<AppState>>, Path(id): Path<String>, Json(req): Json<live_sync::SyncRequest>| async move {
                match live_sync::execute_push(s, &id, &req).await {
                    Ok(resp) => (StatusCode::OK, Json(serde_json::to_value(resp).unwrap())).into_response(),
                    Err((code, msg)) => {
                        let st = StatusCode::from_u16(code).unwrap_or(StatusCode::BAD_GATEWAY);
                        (st, Json(serde_json::json!({ "error": msg }))).into_response()
                    }
                }
            }),
        )
        .route(
            "/sites/{id}/sync/pull",
            post(|State(s): State<Arc<AppState>>, Path(id): Path<String>, Json(req): Json<live_sync::SyncRequest>| async move {
                match live_sync::execute_pull(s, &id, &req).await {
                    Ok(resp) => (StatusCode::OK, Json(serde_json::to_value(resp).unwrap())).into_response(),
                    Err((code, msg)) => {
                        let st = StatusCode::from_u16(code).unwrap_or(StatusCode::BAD_GATEWAY);
                        (st, Json(serde_json::json!({ "error": msg }))).into_response()
                    }
                }
            }),
        )
        .route(
            "/users",
            get(|| async {
                Json(serde_json::json!([{
                    "id": "00000000-0000-0000-0000-000000000001",
                    "email": "local@dockpanel.desktop",
                    "role": "admin"
                }]))
            }),
        )
        .route(
            "/alerts/summary",
            get(|| async { Json(serde_json::json!({"firing": 0, "open": 0})) }),
        )
        .route(
            "/incidents/summary",
            get(|| async { Json(serde_json::json!({"open": 0})) }),
        )
        .route(
            "/notifications/unread-count",
            get(notifications_unread_count_handler),
        )
        .route(
            "/notifications/summary",
            get(notifications_summary_handler),
        )
        .route(
            "/notifications/stream",
            get(notifications_stream_handler),
        )
        .route(
            "/notifications/read-all",
            post(notifications_read_all_handler),
        )
        .route(
            "/notifications/read",
            delete(notifications_delete_read_handler),
        )
        .route(
            "/notifications/{id}/read",
            post(notifications_read_one_handler),
        )
        .route(
            "/notifications/{id}",
            delete(notifications_delete_one_handler),
        )
        .route(
            "/notifications",
            get(notifications_list_handler),
        )
        .route("/php/versions", get(php_versions_handler))
        .route("/php/install", post(php_install_handler))
        .route("/php/extensions/{version}", get(php_extensions_handler))
        .route("/php/extensions/install", post(php_extension_install_handler))
        .route("/php/extensions/disable", post(php_extension_disable_handler))
        .route("/services/install/{id}/log", get(php_install_log_handler))
        .route("/services/install-status", get(services_install_status_handler))
        .route("/mariadb/status", get(mariadb_status_handler))
        .route("/mariadb/install", post(mariadb_install_handler))
        .route("/mariadb/start", post(mariadb_start_handler))
        .route("/mariadb/stop", post(mariadb_stop_handler))
        .route(
            "/databases",
            get(list_databases_handler).post(create_database_handler),
        )
        .route("/databases/sizes", get(databases_sizes_handler))
        .route("/databases/maintenance/summary", get(databases_maintenance_summary_handler))
        .route("/databases/maintenance/cleanup", post(databases_maintenance_cleanup_handler))
        .route("/databases/{id}", delete(delete_database_handler))
        .route("/databases/{id}/health", get(database_health_handler))
        .route("/databases/{id}/optimize", post(database_optimize_handler))
        .route("/databases/{id}/prune", post(database_prune_handler))
        .route("/databases/{id}/credentials", get(database_credentials_handler))
        .route("/databases/{id}/schema-overview", get(database_schema_overview_handler))
        .route("/databases/{id}/tables", get(database_tables_handler))
        .route("/databases/{id}/tables/{table}", get(database_table_structure_handler))
        .route("/databases/{id}/indexes/{table}", get(database_table_indexes_handler))
        .route("/databases/{id}/query", post(database_query_handler))
        .route("/databases/{id}/dumps", get(database_list_dumps_handler))
        .route("/databases/{id}/dumps/{filename}", delete(database_delete_dump_handler))
        .route(
            "/databases/{id}/dumps/{filename}/download",
            get(database_download_dump_handler),
        )
        .route("/databases/{id}/export", post(database_export_handler))
        .route("/databases/{id}/upload-dump", post(database_upload_dump_handler))
        .route("/databases/{id}/import", post(database_import_dump_handler))
        .route(
            "/databases/{id}/reset-password",
            post(database_reset_password_handler),
        )
        .route("/stacks", get(|| async { Json(serde_json::json!([])) }))
        .route("/activity", get(activity_list_handler))
        .route("/system-logs", get(system_logs_handler))
        .route("/system-logs/count", get(system_logs_count_handler))
        .route("/logs", get(logs_read_handler))
        .route("/logs/search", get(logs_read_handler))
        .route("/logs/stats", get(logs_stats_handler))
        .route("/logs/sizes", get(logs_sizes_handler))
        .route("/logs/check-errors", post(logs_check_errors_handler))
        .route("/logs/truncate", post(logs_truncate_handler))
        .route("/logs/docker", get(|| async { Json(serde_json::json!({ "containers": [] })) }))
        .route(
            "/settings",
            get(get_settings_handler).put(update_settings_handler),
        )
        .route("/settings/export", get(export_settings_handler))
        .route(
            "/settings/import",
            post(|State(s): State<Arc<AppState>>, Json(payload): Json<serde_json::Value>| async move {
                let obj = payload
                    .get("settings")
                    .and_then(|v| v.as_object())
                    .or_else(|| payload.as_object());
                let Some(map) = obj else {
                    return (StatusCode::BAD_REQUEST, Json(serde_json::json!({"error": "Invalid config"}))).into_response();
                };
                let mut updates = std::collections::HashMap::new();
                for (k, v) in map {
                    updates.insert(k.clone(), json_val_to_string(v));
                }
                s.update_settings(updates);
                Json(serde_json::json!({ "ok": true })).into_response()
            }),
        )
        .route("/apps/gpu-info", get(|| async { Json(serde_json::json!({ "gpus": [] })) }))
        .route("/dashboard/gpu-metrics-history", get(|| async { Json(serde_json::json!({ "points": [] })) }))
        .route("/settings/health", get(health_handler))
        .route("/health", get(health_handler))
        .route(
            "/settings/oauth-redirects",
            get(|| async {
                Json(serde_json::json!({
                    "base_url": "http://127.0.0.1",
                    "base_url_configured": false,
                    "redirect_uris": {}
                }))
            }),
        )
        .route(
            "/settings/recording-coverage",
            get(|| async { Json(serde_json::json!({ "lagging": [] })) }),
        )
        .route("/api-keys", get(|| async { Json(serde_json::json!([])) }))
        .route(
            "/security/panel-jail/status",
            get(|| async { Json(serde_json::json!({ "jailed": false })) }),
        )
        .route("/alert-rules", get(|| async { Json(serde_json::json!([])) }))
        .route("/escalation-policies", get(|| async { Json(serde_json::json!([])) }))
        .route("/metrics", get(metrics_handler))
        .route(
            "/system/status",
            get(|| async {
                Json(serde_json::json!({
                    "status": "running",
                    "mode": "desktop",
                    "os": "windows",
                    "version": env!("CARGO_PKG_VERSION")
                }))
            }),
        )
        .route("/system/info", get(system_info_handler))
        .route("/system/processes", get(system_processes_handler))
        .route("/system/network", get(system_network_handler))
        .route("/telemetry/update-status", get(telemetry_update_status_handler))
        .route("/desktop/update/install", post(desktop_update_install_handler))
        .route(
            "/tls-certificates",
            get(|| async { Json(serde_json::json!([])) }),
        )
        .route(
            "/dashboard/intelligence",
            get({
                let s = state.clone();
                move || async move {
                    let sites = s.list_sites();
                    let mut ssl_countdowns = Vec::new();
                    for site in &sites {
                        if site.ssl_enabled {
                            ssl_countdowns.push(serde_json::json!({
                                "domain": site.domain,
                                "days_left": 90
                            }));
                        }
                    }
                    Json(serde_json::json!({
                        "health_score": 100,
                        "grade": "A+",
                        "firing_alerts": 0,
                        "acknowledged_alerts": 0,
                        "open_incidents": 0,
                        "stale_backups": 0,
                        "scan_critical": 0,
                        "scan_warnings": 0,
                        "ssl_countdowns": ssl_countdowns,
                        "top_issues": [],
                        "recommendations": []
                    }))
                }
            }),
        )
        .route(
            "/dashboard/fleet",
            get({
                let s = state.clone();
                move || async move {
                    let total_sites = s.list_sites().len();
                    Json(serde_json::json!({
                        "servers": [],
                        "total_servers": 1,
                        "total_firing": 0,
                        "total_incidents": 0,
                        "total_sites": total_sites
                    }))
                }
            }),
        )
        .route(
            "/backup-setup-status",
            get(|| async {
                Json(serde_json::json!({"has_schedule": true, "has_backup": true}))
            }),
        )
        .route(
            "/dashboard/docker",
            get(|| async {
                Json(serde_json::json!({"total": 0, "running": 0, "stopped": 0}))
            }),
        )
        .route(
            "/dashboard/timeline",
            get(|| async { Json(serde_json::json!([])) }),
        )
        .route("/apps/images", get(|| async { Json(serde_json::json!([])) }))
        .route(
            "/system/disk-io",
            get(|| async {
                Json(serde_json::json!({"read_bytes_sec": 0, "write_bytes_sec": 0}))
            }),
        )
        .route(
            "/mail/queue",
            get({
                let s = state.clone();
                move || async move {
                    let queue = s.mail_catcher.list_summaries(None, None);
                    let count = queue.len();
                    let preview: Vec<_> = queue.into_iter().take(10).collect();
                    Json(serde_json::json!({"count": count, "queue": preview}))
                }
            }),
        )
        .route(
            "/on-call/whoami",
            get(|| async { Json(serde_json::json!([])) }),
        )
        .route("/apps", get(|| async { Json(serde_json::json!([])) }))
        .route(
            "/system/updates/count",
            get(|| async {
                Json(serde_json::json!({"count": 0, "security": 0, "reboot_required": false}))
            }),
        )
        .route(
            "/dashboard/metrics-history",
            get({
                let s = state.clone();
                move || async move {
                    Json(serde_json::json!({"points": s.get_metrics_history()}))
                }
            }),
        )
        .fallback(|| async {
            (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({"error": "Not found"})),
            )
        })
        .with_state(state)
}

fn select_startup_port_with<F>(cfg: &router::RouterConfig, mut is_bindable: F) -> u16
where
    F: FnMut(u16) -> bool,
{
    if is_bindable(cfg.preferred_port) {
        return cfg.preferred_port;
    }

    if cfg.active_port != 0
        && cfg.active_port != cfg.preferred_port
        && is_bindable(cfg.active_port)
    {
        return cfg.active_port;
    }

    for candidate in [8080u16, 8443, 3000, 0] {
        if is_bindable(candidate) {
            if candidate == 0 {
                if let Some(ephemeral) = std::net::TcpListener::bind(("0.0.0.0", 0))
                    .or_else(|_| std::net::TcpListener::bind(("127.0.0.1", 0)))
                    .ok()
                    .and_then(|l| l.local_addr().ok().map(|a| a.port()))
                {
                    return ephemeral;
                }
            } else {
                return candidate;
            }
        }
    }

    8080
}

fn start_embedded_server(state: Arc<AppState>) -> u16 {
    let (port_tx, port_rx) = std::sync::mpsc::channel::<u16>();

    std::thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .expect("Failed to create tokio runtime");

        rt.block_on(async move {
            // Honor preferred_port (80), then persisted active_port, then free alternatives
            let cfg = state.get_router_config();
            let mut active_port = select_startup_port_with(&cfg, router::is_port_bindable);
            if !state.restart_site_router(active_port).await {
                if let Some(fallback_port) = state::find_available_loopback_port() {
                    if state.restart_site_router(fallback_port).await {
                        active_port = fallback_port;
                    }
                }
            }
            state.set_router_port(active_port);

            // Start local HTTPS TLS router on 443 (or fallback 8443) for mkcert SSL sites
            for ssl_candidate in [443u16, 8443] {
                if ssl_candidate != active_port
                    && router::is_port_bindable(ssl_candidate)
                    && state.restart_ssl_router(ssl_candidate).await
                {
                    break;
                }
            }

            let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
                .await
                .expect("Failed to bind loopback TCP listener");
            let port = listener.local_addr().unwrap().port();

            let api_routes = build_api_router(state.clone());

            let app = Router::new()
                .nest("/api", api_routes)
                .fallback(static_handler)
                .layer(CorsLayer::permissive());

            let _ = port_tx.send(port);

            // Sample local Windows CPU/RAM/disk metrics every 10s for Dashboard history charts
            {
                let metrics_state = state.clone();
                tokio::spawn(async move {
                    loop {
                        #[cfg(windows)]
                        let point = {
                            let s_dir = metrics_state.sites_dir.clone();
                            tokio::task::spawn_blocking(move || {
                                let m = win_metrics::collect(&s_dir);
                                serde_json::json!({
                                    "cpu": (m.cpu_usage * 10.0).round() / 10.0,
                                    "mem": (m.mem_usage_pct * 10.0).round() / 10.0,
                                    "disk": (m.disk_usage_pct * 10.0).round() / 10.0,
                                    "time": chrono::Local::now().format("%H:%M:%S").to_string(),
                                })
                            })
                            .await
                            .ok()
                        };
                        #[cfg(not(windows))]
                        let point: Option<serde_json::Value> = None;

                        if let Some(pt) = point {
                            metrics_state.push_metrics_point(pt);
                        }
                        tokio::time::sleep(std::time::Duration::from_secs(10)).await;
                    }
                });
            }

            // Start local SMTP Mail Catcher (127.0.0.1:1025 or fallback free port) before
            // spawning PHP/Node site workers so every worker receives the bound SMTP port.
            {
                let domains_state = state.clone();
                let event_state = state.clone();
                match mail_catcher::start_smtp_server(
                    state.mail_catcher.clone(),
                    1025,
                    move || {
                        domains_state
                            .list_sites()
                            .into_iter()
                            .map(|s| s.domain)
                            .collect()
                    },
                    move |email| {
                        event_state.record_event(
                            "mail_captured",
                            "mail",
                            email.site_domain.as_deref().unwrap_or("localhost"),
                            &format!("Email captured: {}", email.subject),
                            &format!("From {} to {}", email.from, email.to.join(", ")),
                            "info",
                        );
                    },
                )
                .await
                {
                    Ok(smtp_port) => {
                        println!("DockPanel Local Mail Catcher listening on 127.0.0.1:{}", smtp_port);
                    }
                    Err(e) => {
                        eprintln!("Local Mail Catcher failed to start: {}", e);
                    }
                }
            }

            // Reconcile and start PHP workers for all active sites in the background
            // so GUI window creation is not delayed. Clean up any orphaned PHP processes
            // left over from crashed/killed sessions first.
            let reconcile_state = state.clone();
            tokio::task::spawn_blocking(move || {
                reconcile_state.cleanup_orphan_php_workers();
                reconcile_state.reconcile_active_sites();
                if mariadb::find_mariadbd_binary().is_some() {
                    let _ = reconcile_state.start_mariadb();
                }
            });

            axum::serve(
                listener,
                app.into_make_service_with_connect_info::<SocketAddr>(),
            )
            .await
            .unwrap();
        });
    });

    port_rx.recv().expect("Failed to receive port from server thread")
}

fn main() {
    std::panic::set_hook(Box::new(|info| {
        let msg = format!("PANIC: {:?}\n", info);
        let _ = std::fs::write("dockpanel_panic.log", msg);
    }));

    let args: Vec<String> = std::env::args().collect();
    if args.iter().any(|a| a == "--server-only") {
        let state = Arc::new(AppState::new());
        let port = start_embedded_server(state);
        println!("DOCKPANEL_EMBEDDED_PORT={}", port);
        std::thread::park();
        return;
    }

    if let Err(e) = run_app() {
        let _ = std::fs::write("dockpanel_error.log", format!("RUN_APP_ERROR: {:?}\n", e));
    }
}

fn run_app() -> Result<(), Box<dyn std::error::Error>> {
    let state = Arc::new(AppState::new());

    // Start in-memory embedded HTTP server
    let port = start_embedded_server(state.clone());

    // Ensure the embedded server is actively accepting connections before creating the webview
    let test_addr = format!("127.0.0.1:{}", port);
    for _ in 0..50 {
        if std::net::TcpStream::connect(&test_addr).is_ok() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }

    let event_loop = EventLoopBuilder::<tray::TrayAction>::with_user_event().build();
    let tray_handle = tray::start_tray(event_loop.create_proxy());
    let window = WindowBuilder::new()
        .with_title("DockPanel Desktop")
        .with_inner_size(LogicalSize::new(1280.0, 800.0))
        .with_min_inner_size(LogicalSize::new(1024.0, 600.0))
        .build(&event_loop)?;

    // JavaScript shim injecting window.__DOCKPANEL__ and window.__TAURI__ for desktop detection
    let init_script = r#"
        window.__DOCKPANEL__ = true;
        window.__TAURI__ = {
            invoke: function(cmd, args) {
                return new Promise(function(resolve, reject) {
                    try {
                        if (window.ipc) {
                            window.ipc.postMessage(JSON.stringify({ cmd: cmd, args: args || {} }));
                        }
                        resolve("ok");
                    } catch (e) {
                        reject(e);
                    }
                });
            },
            core: {
                invoke: function(cmd, args) {
                    return window.__TAURI__.invoke(cmd, args);
                }
            }
        };

        window.addEventListener('error', function(e) {
            try {
                if (window.ipc) {
                    window.ipc.postMessage(JSON.stringify({
                        cmd: "log_error",
                        args: { message: "WINDOW ERROR: " + (e.message || "unknown") + " at " + (e.filename || "unknown") + ":" + e.lineno + ":" + e.colno + " stack: " + (e.error ? e.error.stack : "") }
                    }));
                }
            } catch (_) {}
        });

        window.addEventListener('unhandledrejection', function(e) {
            try {
                if (window.ipc) {
                    window.ipc.postMessage(JSON.stringify({
                        cmd: "log_error",
                        args: { message: "UNHANDLED REJECTION: " + (e.reason ? (e.reason.stack || e.reason.message || e.reason) : e) }
                    }));
                }
            } catch (_) {}
        });

        var _origConsoleError = console.error;
        console.error = function() {
            try {
                var args = Array.prototype.slice.call(arguments);
                _origConsoleError.apply(console, args);
                if (window.ipc) {
                    window.ipc.postMessage(JSON.stringify({
                        cmd: "log_error",
                        args: { message: "CONSOLE ERROR: " + args.join(" ") }
                    }));
                }
            } catch (_) {}
        };

        console.log("DockPanel Desktop (Pure Rust Edition) connected to embedded engine.");
    "#;

    let target_url = format!("http://127.0.0.1:{}/", port);

    let data_dir = state.config_dir.join("webview2");
    std::fs::create_dir_all(&data_dir).ok();
    let mut web_context = wry::WebContext::new(Some(data_dir));

    let _webview = WebViewBuilder::with_web_context(&mut web_context)
        .with_devtools(true)
        .with_initialization_script(init_script)
        .with_new_window_req_handler(|uri| {
            open_external_url(&uri);
            false
        })
        .with_ipc_handler(move |request| {
            if let Ok(val) = serde_json::from_str::<serde_json::Value>(&request.body()) {
                let cmd = val.get("cmd").and_then(|v| v.as_str()).unwrap_or("");
                let args = val.get("args");
                match cmd {
                    "log_error" | "log_info" => {
                        if let Some(msg) = args.and_then(|a| a.get("message")).and_then(|m| m.as_str()) {
                            use std::io::Write;
                            let app_data = std::env::var("APPDATA")
                                .unwrap_or_else(|_| std::env::var("USERPROFILE").unwrap_or_else(|_| ".".into()));
                            let log_file = std::path::PathBuf::from(app_data).join("DockPanel").join("webview_console.log");
                            if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(log_file) {
                                let _ = writeln!(f, "[{}] {}", cmd, msg);
                            }
                        }
                    }
                    "open_url" | "open_external" => {
                        if let Some(url) = args.and_then(|a| a.get("url")).and_then(|u| u.as_str()) {
                            open_external_url(url);
                        }
                    }
                    "open_folder" | "open_site_folder" => {
                        if let Some(path) = args.and_then(|a| a.get("path")).and_then(|p| p.as_str()) {
                            let resolved = resolve_desktop_path(path);
                            let _ = std::process::Command::new("explorer.exe").arg(&resolved).spawn();
                        }
                    }
                    "open_vscode" => {
                        if let Some(path) = args.and_then(|a| a.get("path")).and_then(|p| p.as_str()) {
                            let resolved = resolve_desktop_path(path);
                            let _ = std::process::Command::new("cmd.exe").args(["/c", "code", &resolved]).spawn();
                        }
                    }
                    _ => {}
                }
            }
        })
        .with_url(&target_url)
        .build(&window)?;

    let state_exit = state.clone();
    event_loop.run(move |event, _, control_flow| {
        let _ = &_webview;
        *control_flow = ControlFlow::Wait;
        match event {
            Event::WindowEvent {
                event: WindowEvent::CloseRequested,
                ..
            } => {
                let minimize = state_exit
                    .get_settings()
                    .get("minimize_to_tray")
                    .map(|v| v != "false")
                    .unwrap_or(true);
                if minimize && tray_handle.is_some() {
                    window.set_visible(false);
                } else {
                    state_exit.stop_all_workers();
                    if let Some(ref th) = tray_handle {
                        th.shutdown();
                    }
                    *control_flow = ControlFlow::Exit;
                }
            }
            Event::UserEvent(action) => match action {
                tray::TrayAction::ShowWindow => {
                    window.set_visible(true);
                    window.set_focus();
                }
                tray::TrayAction::OpenSitesFolder => {
                    let dir = state_exit.get_default_sites_dir();
                    let _ = std::fs::create_dir_all(&dir);
                    #[cfg(windows)]
                    let _ = std::process::Command::new("explorer.exe").arg(&dir).spawn();
                }
                tray::TrayAction::QuitApp => {
                    state_exit.stop_all_workers();
                    if let Some(ref th) = tray_handle {
                        th.shutdown();
                    }
                    *control_flow = ControlFlow::Exit;
                }
            },
            _ => {}
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_open_external_url_validation() {
        assert!(!open_external_url("file:///C:/Windows/System32/calc.exe"));
        assert!(!open_external_url("javascript:alert(1)"));
        assert!(!open_external_url(""));
        assert!(!open_external_url("   "));
        assert!(!open_external_url("cmd.exe /c calc"));
    }

    #[tokio::test]
    async fn test_static_handler_serves_assets() {
        let resp = static_handler("/".parse().unwrap()).await.into_response();
        assert_eq!(resp.status(), StatusCode::OK);
        let ct = resp.headers().get(header::CONTENT_TYPE).unwrap().to_str().unwrap();
        assert!(ct.contains("text/html"));

        // Extract the JS bundle path dynamically from index.html
        let body = axum::body::to_bytes(resp.into_body(), 1024 * 64).await.unwrap();
        let html = String::from_utf8_lossy(&body);
        let js_match = html
            .lines()
            .find(|l| l.contains("src=\"/assets/"))
            .and_then(|l| {
                let start = l.find("src=\"")? + 5;
                let end = l[start..].find('"')? + start;
                Some(&l[start..end])
            })
            .expect("index.html must reference a JS asset in /assets/");

        let uri: axum::http::Uri = js_match.parse().unwrap();
        let resp_js = static_handler(uri).await.into_response();
        assert_eq!(resp_js.status(), StatusCode::OK);
        let ct_js = resp_js.headers().get(header::CONTENT_TYPE).unwrap().to_str().unwrap();
        assert!(ct_js.contains("javascript"));
    }

    #[tokio::test]
    async fn test_api_sites_start_stop() {
        let temp_dir = std::env::temp_dir().join(format!("dockpanel_api_test_{}", uuid::Uuid::new_v4()));
        let config_dir = temp_dir.join("config");
        let sites_dir = temp_dir.join("sites");
        let state = Arc::new(AppState::new_with_dirs(config_dir, sites_dir));

        let site = state.create_site("api-lifecycle-test.local", Some("8.3")).unwrap();

        // Stop site via stop_site_handler
        let resp = stop_site_handler(State(state.clone()), Path(site.id.clone())).await;
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(state.get_site(&site.id).unwrap().status, "stopped");

        // Start site via start_site_handler
        let resp = start_site_handler(State(state.clone()), Path(site.id.clone())).await;
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(state.get_site(&site.id).unwrap().status, "active");

        let _ = state.stop_site(&site.id);
        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[tokio::test]
    async fn test_create_site_handler_rejects_invalid_domain() {
        let temp_dir = std::env::temp_dir().join(format!("dockpanel_bad_site_{}", uuid::Uuid::new_v4()));
        let config_dir = temp_dir.join("config");
        let sites_dir = temp_dir.join("sites");
        let state = Arc::new(AppState::new_with_dirs(config_dir, sites_dir));

        let resp = create_site_handler(
            State(state.clone()),
            Json(serde_json::json!({ "domain": "../evil", "php_version": "8.3" })),
        )
        .await;
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
        assert!(state.list_sites().is_empty());

        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_select_startup_port_logic() {
        let cfg = router::RouterConfig {
            preferred_port: 80,
            active_port: 8443,
        };
        // 1. Preferred port free -> chooses 80
        assert_eq!(select_startup_port_with(&cfg, |p| p == 80), 80);
        // 2. Preferred occupied, persisted active_port bindable -> keeps active_port
        assert_eq!(select_startup_port_with(&cfg, |p| p == 8443 || p == 8080), 8443);
        // 3. Both occupied -> probes [8080, 8443, 3000, 0] and picks first available
        assert_eq!(select_startup_port_with(&cfg, |p| p == 3000), 3000);
        // 4. All static alternatives occupied -> probes 0 and returns non-zero ephemeral port
        let eph = select_startup_port_with(&cfg, |p| p == 0);
        assert_ne!(eph, 0);
    }

    #[tokio::test]
    async fn test_api_port_conflict_and_resolve() {
        let temp_dir = std::env::temp_dir().join(format!("dockpanel_port_test_{}", uuid::Uuid::new_v4()));
        let config_dir = temp_dir.join("config");
        let sites_dir = temp_dir.join("sites");
        let state = Arc::new(AppState::new_with_dirs(config_dir, sites_dir));

        // GET conflict
        let conflict_json = port_conflict_handler(
            State(state.clone()),
            axum::extract::Query(PortConflictQuery { port: Some(80) }),
        )
        .await;
        assert_eq!(conflict_json["preferred_port"], 80);
        assert!(conflict_json.get("alternatives").is_some());

        // Pick a guaranteed free port for testing use_port & no-op guard
        let free_probe = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let free_port = free_probe.local_addr().unwrap().port();
        drop(free_probe);

        // POST resolve use_port
        let resolve_resp = port_resolve_handler(
            State(state.clone()),
            Json(PortResolvePayload {
                action: "use_port".to_string(),
                port: Some(free_port),
                pid: None,
            }),
        )
        .await;
        assert_eq!(resolve_resp.status(), StatusCode::OK);
        assert_eq!(state.get_router_config().active_port, free_port);

        // No-op guard: calling use_port with already-active port succeeds without restart
        let noop_resp = port_resolve_handler(
            State(state.clone()),
            Json(PortResolvePayload {
                action: "use_port".to_string(),
                port: Some(free_port),
                pid: None,
            }),
        )
        .await;
        assert_eq!(noop_resp.status(), StatusCode::OK);
        let noop_body = axum::body::to_bytes(noop_resp.into_body(), 1024 * 64).await.unwrap();
        let noop_val: serde_json::Value = serde_json::from_slice(&noop_body).unwrap();
        assert_eq!(noop_val["success"], true);
        assert_eq!(noop_val["active_port"], free_port);

        // POST resolve use_port on an occupied port returns success: false
        let occupied_listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let occupied_port = occupied_listener.local_addr().unwrap().port();

        let occ_resp = port_resolve_handler(
            State(state.clone()),
            Json(PortResolvePayload {
                action: "use_port".to_string(),
                port: Some(occupied_port),
                pid: None,
            }),
        )
        .await;
        assert_eq!(occ_resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(occ_resp.into_body(), 1024 * 64).await.unwrap();
        let val: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(val["success"], false);
        assert!(val["message"].as_str().unwrap().contains("already in use"));
        drop(occupied_listener);

        // POST resolve stop_process with mismatched PID returns 409 Conflict
        let mismatch_resp = port_resolve_handler(
            State(state.clone()),
            Json(PortResolvePayload {
                action: "stop_process".to_string(),
                port: Some(free_port),
                pid: Some(999_999),
            }),
        )
        .await;
        assert_eq!(mismatch_resp.status(), StatusCode::CONFLICT);

        // POST resolve stop_process self PID returns 400 Bad Request
        let self_pid_resp = port_resolve_handler(
            State(state.clone()),
            Json(PortResolvePayload {
                action: "stop_process".to_string(),
                port: Some(80),
                pid: Some(std::process::id()),
            }),
        )
        .await;
        assert_eq!(self_pid_resp.status(), StatusCode::BAD_REQUEST);

        // POST resolve stop_process system PID 4 returns 400 Bad Request
        let sys_pid_resp = port_resolve_handler(
            State(state.clone()),
            Json(PortResolvePayload {
                action: "stop_process".to_string(),
                port: Some(80),
                pid: Some(4),
            }),
        )
        .await;
        assert_eq!(sys_pid_resp.status(), StatusCode::BAD_REQUEST);

        // POST resolve unknown action returns 400 Bad Request
        let bad_resp = port_resolve_handler(
            State(state.clone()),
            Json(PortResolvePayload {
                action: "invalid_action".to_string(),
                port: None,
                pid: None,
            }),
        )
        .await;
        assert_eq!(bad_resp.status(), StatusCode::BAD_REQUEST);

        // When checking a port held by current process, port_conflict_handler reports in_use == false
        let self_listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let self_port = self_listener.local_addr().unwrap().port();
        let self_status = router::check_port_status(self_port);
        if self_status.pid == Some(std::process::id()) {
            let conflict_self = port_conflict_handler(
                State(state.clone()),
                axum::extract::Query(PortConflictQuery {
                    port: Some(self_port),
                }),
            )
            .await;
            assert_eq!(conflict_self["in_use"], false);
            assert!(conflict_self["process_name"].is_null());
        }
        drop(self_listener);

        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[tokio::test]
    async fn test_api_wordpress_endpoints_lifecycle() {
        let temp_dir = std::env::temp_dir().join(format!("dockpanel_wp_api_test_{}", uuid::Uuid::new_v4()));
        let config_dir = temp_dir.join("config");
        let sites_dir = temp_dir.join("sites");
        let state = Arc::new(AppState::new_with_dirs(config_dir, sites_dir));

        let site = state.create_site("wp-lifecycle.test", Some("8.3")).unwrap();

        // 1. Initial info: uninstalled
        let resp = get_wordpress_info_handler(State(state.clone()), Path(site.id.clone())).await;
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), 1024 * 64).await.unwrap();
        let val: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(val["installed"], false);

        // 2. Install WordPress (verifies duplicate admin_pass + admin_password aliases do not 422)
        let req = serde_json::json!({
            "admin_user": "wpadmin",
            "admin_pass": "wpsecret999",
            "admin_password": "wpsecret999",
            "admin_email": "wpadmin@wp-lifecycle.test",
            "title": "Lifecycle Site",
            "site_title": "Lifecycle Site",
            "url": "http://wp-lifecycle.test"
        });
        let install_resp = install_wordpress_handler(State(state.clone()), Path(site.id.clone()), Json(req)).await;
        assert_eq!(install_resp.status(), StatusCode::CREATED);
        assert_eq!(state.get_site(&site.id).unwrap().php_preset.as_deref(), Some("wordpress"));

        // 3. Post-install info: installed
        let resp2 = get_wordpress_info_handler(State(state.clone()), Path(site.id.clone())).await;
        let body2 = axum::body::to_bytes(resp2.into_body(), 1024 * 64).await.unwrap();
        let val2: serde_json::Value = serde_json::from_slice(&body2).unwrap();
        assert_eq!(val2["installed"], true);
        assert_eq!(val2["version"], "6.7.1");

        // 4. Reinstall core
        let re_resp = reinstall_wordpress_core_handler(State(state.clone()), Path(site.id.clone()), None).await;
        assert_eq!(re_resp.status(), StatusCode::OK);

        // 5. DB & Features & Plugins & Themes
        let db_resp = wordpress_db_info_handler(State(state.clone()), Path(site.id.clone())).await;
        assert_eq!(db_resp.status(), StatusCode::OK);

        let feat_resp = wordpress_features_status_handler(State(state.clone()), Path(site.id.clone())).await;
        assert_eq!(feat_resp.status(), StatusCode::OK);

        let pl_resp = wordpress_plugins_handler(State(state.clone()), Path(site.id.clone())).await;
        assert_eq!(pl_resp.status(), StatusCode::OK);

        let th_resp = wordpress_themes_handler(State(state.clone()), Path(site.id.clone())).await;
        assert_eq!(th_resp.status(), StatusCode::OK);

        let _ = state.stop_site(&site.id);
        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[tokio::test]
    async fn test_php_versions_and_switch_endpoints() {
        let temp_dir = std::env::temp_dir().join(format!("dockpanel_php_api_test_{}", uuid::Uuid::new_v4()));
        let config_dir = temp_dir.join("config");
        let sites_dir = temp_dir.join("sites");
        let state = Arc::new(AppState::new_with_dirs(config_dir, sites_dir));

        let Json(versions_val) = php_versions_handler(State(state.clone())).await;
        let versions = versions_val["versions"].as_array().unwrap();
        assert!(versions.len() >= 8);
        for v in versions {
            // fpm_running must match installed so PhpVersionPicker never claims PHP-FPM is stopped when PHP is ready.
            assert_eq!(v["installed"], v["fpm_running"]);
        }

        // Unsupported PHP version should return 400 BAD_REQUEST
        let bad_install = php_install_handler(
            State(state.clone()),
            Json(serde_json::json!({ "version": "5.2" })),
        )
        .await;
        assert_eq!(bad_install.status(), StatusCode::BAD_REQUEST);

        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[tokio::test]
    async fn test_full_desktop_e2e_workflows() {
        let temp_dir = std::env::temp_dir().join(format!("dockpanel_e2e_test_{}", uuid::Uuid::new_v4()));
        let config_dir = temp_dir.join("config");
        let sites_dir = temp_dir.join("sites");
        let state = Arc::new(AppState::new_with_dirs(config_dir, sites_dir));

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let app = Router::new()
            .nest("/api", build_api_router(state.clone()))
            .fallback(static_handler);
        let server = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });

        let client = reqwest::Client::new();
        let base = format!("http://127.0.0.1:{}/api", addr.port());

        // 1. Create site
        let created_text = client
            .post(format!("{}/sites", base))
            .header("content-type", "application/json")
            .body(serde_json::json!({"domain": "e2e-wp.test", "php_version": "8.3"}).to_string())
            .send()
            .await
            .unwrap()
            .text()
            .await
            .unwrap();
        let created: serde_json::Value = serde_json::from_str(&created_text).unwrap();
        let id = created["id"].as_str().unwrap().to_string();
        assert_eq!(created["runtime"], "php");
        assert_eq!(created["enabled"], true);

        // 2. Provision log SSE returns complete event without closing empty
        let sse_res = client
            .get(format!("{}/sites/{}/provision-log", base, id))
            .send()
            .await
            .unwrap();
        assert_eq!(sse_res.status(), StatusCode::OK);
        let sse_text = sse_res.text().await.unwrap();
        assert!(sse_text.contains("\"step\":\"complete\""), "SSE output: {}", sse_text);

        // 3. Metrics overview + health summary + env
        let metrics_text = client
            .get(format!("{}/sites/metrics-overview", base))
            .send()
            .await
            .unwrap()
            .text()
            .await
            .unwrap();
        let metrics: serde_json::Value = serde_json::from_str(&metrics_text).unwrap();
        assert!(metrics[&id]["disk_usage_bytes"].as_u64().is_some());

        let summary_text = client
            .get(format!("{}/sites/{}/health-summary", base, id))
            .send()
            .await
            .unwrap()
            .text()
            .await
            .unwrap();
        let summary: serde_json::Value = serde_json::from_str(&summary_text).unwrap();
        assert_eq!(summary["score"], 100);

        // 4. WordPress 1-click install (even with duplicate aliases) + features + toolkit list
        let wp_res = client
            .post(format!("{}/sites/{}/wordpress/install", base, id))
            .header("content-type", "application/json")
            .body(
                serde_json::json!({
                    "title": "E2E WP",
                    "admin_user": "admin",
                    "admin_pass": "pass1234",
                    "admin_password": "pass1234",
                    "admin_email": "admin@e2e-wp.test"
                })
                .to_string(),
            )
            .send()
            .await
            .unwrap();
        assert_eq!(wp_res.status(), StatusCode::CREATED);

        let wp_sites_text = client
            .get(format!("{}/wordpress/sites", base))
            .send()
            .await
            .unwrap()
            .text()
            .await
            .unwrap();
        let wp_sites: serde_json::Value = serde_json::from_str(&wp_sites_text).unwrap();
        assert_eq!(wp_sites.as_array().unwrap().len(), 1);

        let maint_res = client
            .post(format!("{}/sites/{}/wordpress/maintenance", base, id))
            .header("content-type", "application/json")
            .body(serde_json::json!({"enabled": true}).to_string())
            .send()
            .await
            .unwrap();
        assert_eq!(maint_res.status(), StatusCode::OK);

        // 5. File manager create, write, read, delete
        let f_create = client
            .post(format!("{}/sites/{}/files/create?path=notes.txt&type=file", base, id))
            .send()
            .await
            .unwrap();
        assert_eq!(f_create.status(), StatusCode::OK);

        let f_write = client
            .put(format!("{}/sites/{}/files/write", base, id))
            .header("content-type", "application/json")
            .body(serde_json::json!({"path": "notes.txt", "content": "hello dockpanel"}).to_string())
            .send()
            .await
            .unwrap();
        assert_eq!(f_write.status(), StatusCode::OK);

        let f_read_text = client
            .get(format!("{}/sites/{}/files/read?path=notes.txt", base, id))
            .send()
            .await
            .unwrap()
            .text()
            .await
            .unwrap();
        let f_read: serde_json::Value = serde_json::from_str(&f_read_text).unwrap();
        assert_eq!(f_read["content"], "hello dockpanel");

        // 6. Env get/put
        let env_put = client
            .put(format!("{}/sites/{}/env", base, id))
            .header("content-type", "application/json")
            .body(serde_json::json!({"vars": [{"key": "APP_ENV", "value": "local"}]}).to_string())
            .send()
            .await
            .unwrap();
        assert_eq!(env_put.status(), StatusCode::OK);
        let env_get_text = client
            .get(format!("{}/sites/{}/env", base, id))
            .send()
            .await
            .unwrap()
            .text()
            .await
            .unwrap();
        let env_get: serde_json::Value = serde_json::from_str(&env_get_text).unwrap();
        assert_eq!(env_get["vars"][0]["key"], "APP_ENV");

        let _ = state.stop_site(&id);
        server.abort();
        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[tokio::test]
    async fn test_audited_desktop_endpoints_and_events() {
        async fn req_json(
            client: &reqwest::Client,
            method: reqwest::Method,
            url: &str,
            body: Option<serde_json::Value>,
        ) -> (reqwest::StatusCode, serde_json::Value) {
            let mut builder = client.request(method, url);
            if let Some(b) = body {
                builder = builder
                    .header("content-type", "application/json")
                    .body(b.to_string());
            }
            let resp = builder.send().await.unwrap();
            let status = resp.status();
            let txt = resp.text().await.unwrap();
            let val = serde_json::from_str(&txt).unwrap_or(serde_json::json!(null));
            (status, val)
        }

        let temp_dir = std::env::temp_dir().join(format!("dockpanel_audit_e2e_{}", uuid::Uuid::new_v4()));
        let config_dir = temp_dir.join("config");
        let sites_dir = temp_dir.join("sites");
        let state = Arc::new(AppState::new_with_dirs(config_dir.clone(), sites_dir));

        let app = build_api_router(state.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });

        let client = reqwest::Client::new();
        let base = format!("http://127.0.0.1:{}", port);
        let test_domain = format!("audit-{}.test", uuid::Uuid::new_v4().simple());

        // 1. Health, System Info, Settings & Branding
        let (st, health) = req_json(&client, reqwest::Method::GET, &format!("{}/settings/health", base), None).await;
        assert_eq!(st, StatusCode::OK);
        assert_eq!(health["db"], "ok");
        assert_eq!(health["database"], true);

        let (st, sys_info) = req_json(&client, reqwest::Method::GET, &format!("{}/system/info", base), None).await;
        assert_eq!(st, StatusCode::OK);
        assert_eq!(sys_info["os"], "Windows");

        let (st, _) = req_json(
            &client,
            reqwest::Method::PUT,
            &format!("{}/settings", base),
            Some(serde_json::json!({"panel_name": "Studio Local"})),
        )
        .await;
        assert_eq!(st, StatusCode::OK);
        let (_, branding) = req_json(&client, reqwest::Method::GET, &format!("{}/branding", base), None).await;
        assert_eq!(branding["panel_name"], "Studio Local");

        // 2. Create Site -> Activity + Notifications populated
        let (st, created) = req_json(
            &client,
            reqwest::Method::POST,
            &format!("{}/sites", base),
            Some(serde_json::json!({ "domain": test_domain })),
        )
        .await;
        assert_eq!(st, StatusCode::OK);
        let site_id = created["id"].as_str().unwrap().to_string();

        let (_, unread_before) = req_json(&client, reqwest::Method::GET, &format!("{}/notifications/unread-count", base), None).await;
        assert!(unread_before["count"].as_u64().unwrap_or(0) >= 1);

        let (st, _) = req_json(&client, reqwest::Method::POST, &format!("{}/notifications/read-all", base), None).await;
        assert_eq!(st, StatusCode::OK);
        let (_, unread_after) = req_json(&client, reqwest::Method::GET, &format!("{}/notifications/unread-count", base), None).await;
        assert_eq!(unread_after["count"], 0);

        let (_, activity) = req_json(&client, reqwest::Method::GET, &format!("{}/activity", base), None).await;
        assert!(!activity.as_array().unwrap().is_empty());

        // 3. Desktop open-folder domain validation
        let (bad_st, _) = req_json(
            &client,
            reqwest::Method::POST,
            &format!("{}/desktop/open-folder", base),
            Some(serde_json::json!({"domain": "../evil"})),
        )
        .await;
        assert_eq!(bad_st, StatusCode::BAD_REQUEST);

        let (ok_st, _) = req_json(
            &client,
            reqwest::Method::POST,
            &format!("{}/desktop/open-folder", base),
            Some(serde_json::json!({"domain": test_domain})),
        )
        .await;
        assert_eq!(ok_st, StatusCode::OK);

        // 4. Streaming WordPress install -> poll state.php_installs until done
        let (wp_st, wp_body) = req_json(
            &client,
            reqwest::Method::POST,
            &format!("{}/sites/{}/wordpress/install", base, site_id),
            Some(serde_json::json!({
                "title": "Audit WP",
                "admin_user": "admin",
                "admin_pass": "StrongPass!123",
                "admin_email": format!("admin@{}", test_domain),
                "stream": true
            })),
        )
        .await;
        assert_eq!(wp_st, StatusCode::ACCEPTED);
        let iid = wp_body["install_id"].as_str().unwrap().to_string();

        let mut wp_done = false;
        for _ in 0..120 {
            if let Ok(jobs) = state.php_installs.lock() {
                if jobs.get(&iid).map(|j| j.done).unwrap_or(false) {
                    wp_done = true;
                    break;
                }
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
        assert!(wp_done, "Streaming WordPress install job must complete");

        let (_, wp_info) = req_json(&client, reqwest::Method::GET, &format!("{}/sites/{}/wordpress", base, site_id), None).await;
        assert_eq!(wp_info["installed"], true);

        // 5. Local SQLite database lifecycle & query
        let db_name = format!("auditdb_{}", uuid::Uuid::new_v4().simple());
        let (db_st, db_obj) = req_json(
            &client,
            reqwest::Method::POST,
            &format!("{}/databases", base),
            Some(serde_json::json!({ "name": db_name })),
        )
        .await;
        assert_eq!(db_st, StatusCode::CREATED);
        let db_id = db_obj["id"].as_str().unwrap().to_string();

        let (cred_st, _) = req_json(&client, reqwest::Method::GET, &format!("{}/databases/{}/credentials", base, db_id), None).await;
        assert_eq!(cred_st, StatusCode::OK);

        if state::find_php_binary(None).is_some() {
            let (ct_st, ct_body) = req_json(
                &client,
                reqwest::Method::POST,
                &format!("{}/databases/{}/query", base, db_id),
                Some(serde_json::json!({ "sql": "CREATE TABLE items (id INTEGER PRIMARY KEY, name TEXT)" })),
            )
            .await;
            assert_eq!(ct_st, StatusCode::OK, "CREATE TABLE failed: {ct_body}");

            let (ins_st, _) = req_json(
                &client,
                reqwest::Method::POST,
                &format!("{}/databases/{}/query", base, db_id),
                Some(serde_json::json!({ "sql": "INSERT INTO items (name) VALUES ('hello')" })),
            )
            .await;
            assert_eq!(ins_st, StatusCode::OK);

            let (sel_st, sel) = req_json(
                &client,
                reqwest::Method::POST,
                &format!("{}/databases/{}/query", base, db_id),
                Some(serde_json::json!({ "sql": "SELECT name FROM items" })),
            )
            .await;
            assert_eq!(sel_st, StatusCode::OK);
            assert_eq!(sel["rows"][0][0], "hello");

            let (_, schema) = req_json(&client, reqwest::Method::GET, &format!("{}/databases/{}/schema-overview", base, db_id), None).await;
            assert_eq!(schema["tables"]["rows"][0][0], "items");

            let (exp_st, exp_body) = req_json(
                &client,
                reqwest::Method::POST,
                &format!("{}/databases/{}/export", base, db_id),
                Some(serde_json::json!({})),
            )
            .await;
            assert_eq!(exp_st, StatusCode::OK, "export failed: {exp_body}");
            let dump_name = exp_body["filename"].as_str().unwrap().to_string();

            let (dmps_st, dmps) = req_json(&client, reqwest::Method::GET, &format!("{}/databases/{}/dumps", base, db_id), None).await;
            assert_eq!(dmps_st, StatusCode::OK);
            assert!(!dmps.as_array().unwrap().is_empty());

            let (imp_st, _) = req_json(
                &client,
                reqwest::Method::POST,
                &format!("{}/databases/{}/import", base, db_id),
                Some(serde_json::json!({ "filename": dump_name })),
            )
            .await;
            assert_eq!(imp_st, StatusCode::OK);

            let (dd_st, _) = req_json(
                &client,
                reqwest::Method::DELETE,
                &format!("{}/databases/{}/dumps/{}", base, db_id, dump_name),
                None,
            )
            .await;
            assert_eq!(dd_st, StatusCode::OK);
        }

        // File compress/extract/copy test
        let (comp_st, comp_body) = req_json(
            &client,
            reqwest::Method::POST,
            &format!("{}/sites/{}/files/compress", base, site_id),
            Some(serde_json::json!({ "sources": ["wp-config.php"], "destination": "cfg.zip" })),
        )
        .await;
        assert_eq!(comp_st, StatusCode::OK, "compress failed: {comp_body}");

        let (ext_st, _) = req_json(
            &client,
            reqwest::Method::POST,
            &format!("{}/sites/{}/files/extract", base, site_id),
            Some(serde_json::json!({ "archive": "cfg.zip", "destination": "extracted" })),
        )
        .await;
        assert_eq!(ext_st, StatusCode::OK);

        let (db_del_st, _) = req_json(&client, reqwest::Method::DELETE, &format!("{}/databases/{}", base, db_id), None).await;
        assert_eq!(db_del_st, StatusCode::OK);

        // 6. Site backups lifecycle (create -> SSE done -> list -> download -> delete)
        let (bk_st, bk_body) = req_json(&client, reqwest::Method::POST, &format!("{}/sites/{}/backups", base, site_id), None).await;
        assert_eq!(bk_st, StatusCode::ACCEPTED);
        let bid = bk_body["backup_id"].as_str().unwrap().to_string();
        for _ in 0..120 {
            if let Ok(jobs) = state.php_installs.lock() {
                if jobs.get(&bid).map(|j| j.done).unwrap_or(false) {
                    break;
                }
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }

        let (_, bk_list) = req_json(&client, reqwest::Method::GET, &format!("{}/sites/{}/backups", base, site_id), None).await;
        let bk_arr = bk_list.as_array().unwrap();
        assert_eq!(bk_arr.len(), 1);
        let zip_id = bk_arr[0]["id"].as_str().unwrap().to_string();

        let dl = client
            .get(format!("{}/sites/{}/backups/{}/download", base, site_id, zip_id))
            .send()
            .await
            .unwrap();
        assert_eq!(dl.status(), StatusCode::OK);
        let dl_bytes = dl.bytes().await.unwrap();
        assert!(dl_bytes.starts_with(b"PK"), "Backup download must be a valid ZIP archive");

        let (bk_del_st, _) = req_json(&client, reqwest::Method::DELETE, &format!("{}/sites/{}/backups/{}", base, site_id, zip_id), None).await;
        assert_eq!(bk_del_st, StatusCode::OK);
        let _ = std::fs::remove_dir_all(backups::site_backups_dir(&test_domain));

        // 7. Logs & system logs endpoints
        let (_, log_sizes) = req_json(&client, reqwest::Method::GET, &format!("{}/logs/sizes", base), None).await;
        assert!(log_sizes["files"].as_array().unwrap().len() >= 3);

        let (_, sys_logs) = req_json(&client, reqwest::Method::GET, &format!("{}/system-logs", base), None).await;
        assert!(!sys_logs.as_array().unwrap().is_empty());

        let (del_site_st, _) = req_json(&client, reqwest::Method::DELETE, &format!("{}/sites/{}", base, site_id), None).await;
        assert_eq!(del_site_st, StatusCode::OK);
        server.abort();
        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[tokio::test]
    async fn test_wordpress_setup_integrity_ssl_and_single_active_theme() {
        let temp_dir = std::env::temp_dir().join(format!("dockpanel_wp_toolkit_{}", uuid::Uuid::new_v4()));
        let config_dir = temp_dir.join("config");
        let sites_dir = temp_dir.join("sites");
        let state = Arc::new(AppState::new_with_dirs(config_dir, sites_dir));

        let app = build_api_router(state.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });

        let client = reqwest::Client::new();
        let base = format!("http://127.0.0.1:{}", port);
        let custom_dir = temp_dir.join("custom_wp").join("wptoolkit.test");

        // Create site with custom document_root
        let create_resp = client
            .post(format!("{}/sites", base))
            .json(&serde_json::json!({
                "domain": "wptoolkit.test",
                "document_root": custom_dir.to_string_lossy().to_string()
            }))
            .send()
            .await
            .unwrap();
        assert_eq!(create_resp.status(), StatusCode::OK);
        let site_json: serde_json::Value = create_resp.json().await.unwrap();
        let site_id = site_json["id"].as_str().unwrap().to_string();

        // Install WordPress synchronously in temp custom dir
        let install_resp = client
            .post(format!("{}/sites/{}/wordpress/install", base, site_id))
            .json(&serde_json::json!({
                "title": "Toolkit Test",
                "admin_user": "admin",
                "admin_pass": "InitialPass!123",
                "admin_email": "admin@wptoolkit.test"
            }))
            .send()
            .await
            .unwrap();
        assert_eq!(install_resp.status(), StatusCode::CREATED);

        // Setup endpoint should succeed
        let setup_resp = client
            .post(format!("{}/sites/{}/wordpress/setup", base, site_id))
            .json(&serde_json::json!({
                "site_title": "Updated Title",
                "admin_user": "admin",
                "admin_password": "UpdatedPass!456",
                "admin_email": "admin@wptoolkit.test"
            }))
            .send()
            .await
            .unwrap();
        assert_eq!(setup_resp.status(), StatusCode::OK);

        // POST /wordpress/integrity must work and return { clean, issues, message }
        let int_resp = client
            .post(format!("{}/sites/{}/wordpress/integrity", base, site_id))
            .send()
            .await
            .unwrap();
        assert_eq!(int_resp.status(), StatusCode::OK);
        let int_json: serde_json::Value = int_resp.json().await.unwrap();
        assert!(int_json.get("issues").and_then(|v| v.as_array()).is_some());

        // Themes list must never have more than 1 active theme
        let themes_resp = client
            .get(format!("{}/sites/{}/wordpress/themes", base, site_id))
            .send()
            .await
            .unwrap();
        assert_eq!(themes_resp.status(), StatusCode::OK);
        let themes_json: serde_json::Value = themes_resp.json().await.unwrap();
        let active_count = themes_json
            .as_array()
            .unwrap()
            .iter()
            .filter(|t| t["status"] == "active")
            .count();
        assert!(active_count <= 1, "Only 1 WordPress theme can be active at a time, got {}", active_count);

        // SSL status + provision endpoint should respond cleanly
        let ssl_resp = client
            .post(format!("{}/sites/{}/ssl/provision", base, site_id))
            .send()
            .await
            .unwrap();
        let ssl_st = ssl_resp.status();
        let ssl_txt = ssl_resp.text().await.unwrap_or_default();
        assert_eq!(ssl_st, StatusCode::OK, "SSL provision failed: {}", ssl_txt);

        // Delete site removes custom folder on disk too
        let del_resp = client
            .delete(format!("{}/sites/{}", base, site_id))
            .send()
            .await
            .unwrap();
        assert_eq!(del_resp.status(), StatusCode::OK);
        assert!(!custom_dir.exists());

        server.abort();
        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[tokio::test]
    async fn test_sse_emits_inplace_download_progress_updates() {
        let temp_dir = std::env::temp_dir().join(format!("dockpanel_sse_prog_{}", uuid::Uuid::new_v4()));
        let state = Arc::new(AppState::new_with_dirs(temp_dir.join("config"), temp_dir.join("sites")));
        let app = build_api_router(state.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });

        let job_id = "wp-prog-test";
        state.push_install_progress(job_id, "cms", "Starting download...", "in_progress", Some(0), None, false);

        let st_bg = state.clone();
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(80)).await;
            st_bg.push_install_progress(job_id, "cms", "Downloading WordPress Core: 13.0 MB / 26.0 MB (50%)", "in_progress", Some(50), None, false);
            tokio::time::sleep(std::time::Duration::from_millis(200)).await;
            st_bg.push_install_progress(job_id, "cms", "WordPress Core ready", "done", Some(100), None, false);
            st_bg.push_install_progress(job_id, "complete", "Done", "done", Some(100), None, true);
        });

        let body = reqwest::Client::new()
            .get(format!("http://127.0.0.1:{}/services/install/{}/log", port, job_id))
            .send()
            .await
            .unwrap()
            .text()
            .await
            .unwrap();

        assert!(body.contains("50%") || body.contains("\"progress\":50"), "SSE missed in-place 50% progress update: {}", body);
        assert!(body.contains("\"progress\":100"), "SSE missed 100% update: {}", body);

        let c = reqwest::Client::new();
        assert_eq!(c.get(format!("http://127.0.0.1:{}/stacks", port)).send().await.unwrap().status(), StatusCode::OK);
        assert_eq!(c.get(format!("http://127.0.0.1:{}/databases", port)).send().await.unwrap().status(), StatusCode::OK);
        assert_eq!(c.get(format!("http://127.0.0.1:{}/databases/sizes", port)).send().await.unwrap().status(), StatusCode::OK);

        server.abort();
        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[tokio::test]
    async fn test_inspect_folder_and_connect_existing_wordpress_db_import() {
        use base64::{engine::general_purpose::STANDARD as B64, Engine};

        let temp_dir = std::env::temp_dir().join(format!("dockpanel_wp_connect_{}", uuid::Uuid::new_v4()));
        let existing_wp = temp_dir.join("my_existing_wp");
        std::fs::create_dir_all(existing_wp.join("wp-includes")).unwrap();
        std::fs::write(
            existing_wp.join("wp-includes").join("version.php"),
            "<?php $wp_version = '6.6.2';",
        )
        .unwrap();
        std::fs::write(
            existing_wp.join("wp-config.php"),
            "<?php\ndefine( 'DB_NAME', 'old_prod' );\n$table_prefix = 'wp_prod_';\n",
        )
        .unwrap();

        let state = Arc::new(AppState::new_with_dirs(temp_dir.join("config"), temp_dir.join("sites")));
        let app = build_api_router(state.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });

        let c = reqwest::Client::new();
        let base = format!("http://127.0.0.1:{}", port);

        // 1. Inspect existing WP folder
        let insp_res: serde_json::Value = c
            .post(format!("{}/desktop/inspect-folder", base))
            .json(&serde_json::json!({ "path": existing_wp.to_string_lossy() }))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(insp_res["is_wordpress"], true);
        assert_eq!(insp_res["wp_version"], "6.6.2");
        assert_eq!(insp_res["wp_table_prefix"], "wp_prod_");

        // 2. Connect existing WP folder without overwriting wp-config.php
        let create_res: serde_json::Value = c
            .post(format!("{}/sites", base))
            .json(&serde_json::json!({
                "domain": "connected-wp.test",
                "runtime": "php",
                "php_preset": "wordpress",
                "document_root": existing_wp.to_string_lossy(),
                "connect_existing": true
            }))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        let site_id = create_res["id"].as_str().unwrap().to_string();
        assert_eq!(
            std::fs::read_to_string(existing_wp.join("wp-config.php")).unwrap(),
            "<?php\ndefine( 'DB_NAME', 'old_prod' );\n$table_prefix = 'wp_prod_';\n"
        );

        // 3. Import uploaded database and verify wp-config.php credentials/URL update
        let imp_res = c
            .post(format!("{}/sites/{}/wordpress/import-database", base, site_id))
            .json(&serde_json::json!({
                "filename": "backup.sqlite",
                "content_base64": B64.encode(b"SQLite format 3\0testdb"),
                "target_url": "https://connected-wp.test"
            }))
            .send()
            .await
            .unwrap();
        assert_eq!(imp_res.status(), StatusCode::OK);
        let updated_cfg = std::fs::read_to_string(existing_wp.join("wp-config.php")).unwrap();
        assert!(updated_cfg.contains("define( 'WP_HOME', 'https://connected-wp.test' )"));
        assert!(updated_cfg.contains("define( 'WP_SITEURL', 'https://connected-wp.test' )"));
        assert!(updated_cfg.contains("$table_prefix = 'wp_prod_';"));

        server.abort();
        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[tokio::test]
    async fn test_streaming_connect_existing_and_import_database_sse() {
        use base64::{engine::general_purpose::STANDARD as B64, Engine};

        let temp_dir = std::env::temp_dir().join(format!("dockpanel_wp_stream_{}", uuid::Uuid::new_v4()));
        let existing_wp = temp_dir.join("stream_wp");
        std::fs::create_dir_all(existing_wp.join("wp-includes")).unwrap();
        std::fs::write(
            existing_wp.join("wp-includes").join("version.php"),
            "<?php $wp_version = '6.7.0';",
        )
        .unwrap();
        std::fs::write(
            existing_wp.join("wp-config.php"),
            "<?php\ndefine( 'DB_NAME', 'old_db' );\n$table_prefix = 'wp_old_';\n",
        )
        .unwrap();

        let state = Arc::new(AppState::new_with_dirs(temp_dir.join("config"), temp_dir.join("sites")));
        let app = build_api_router(state.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });

        let c = reqwest::Client::new();
        let base = format!("http://127.0.0.1:{}", port);

        // 1. Connect existing folder with stream=true and verify provision-log emits folder/hosts/runtime/complete
        let create_res: serde_json::Value = c
            .post(format!("{}/sites", base))
            .json(&serde_json::json!({
                "domain": "stream-wp.test",
                "runtime": "php",
                "php_preset": "wordpress",
                "document_root": existing_wp.to_string_lossy(),
                "connect_existing": true,
                "stream": true
            }))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        let site_id = create_res["id"].as_str().unwrap().to_string();

        let prov_sse = c
            .get(format!("{}/sites/{}/provision-log", base, site_id))
            .send()
            .await
            .unwrap()
            .text()
            .await
            .unwrap();
        assert!(prov_sse.contains("\"step\":\"folder\""), "missing folder step: {}", prov_sse);
        assert!(prov_sse.contains("\"step\":\"hosts\""), "missing hosts step: {}", prov_sse);
        assert!(prov_sse.contains("\"step\":\"runtime\""), "missing runtime step: {}", prov_sse);
        assert!(prov_sse.contains("\"step\":\"complete\""), "missing complete step: {}", prov_sse);

        // 2. Import DB with stream=true and verify SSE import-log streams all steps & logs
        let imp_start: serde_json::Value = c
            .post(format!("{}/sites/{}/wordpress/import-database", base, site_id))
            .json(&serde_json::json!({
                "filename": "backup.sqlite",
                "content_base64": B64.encode(b"SQLite format 3\0streamdb"),
                "target_url": "https://stream-wp.test",
                "stream": true
            }))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(imp_start["streaming"], true);
        let sse_path = imp_start["sse_url"].as_str().unwrap();
        assert!(sse_path.starts_with("/api/sites/"));
        let stripped = sse_path.strip_prefix("/api").unwrap();

        let imp_sse = c
            .get(format!("{}{}", base, stripped))
            .send()
            .await
            .unwrap()
            .text()
            .await
            .unwrap();
        assert!(imp_sse.contains("\"step\":\"site_connect\""), "missing site_connect: {}", imp_sse);
        assert!(imp_sse.contains("\"step\":\"upload\""), "missing upload: {}", imp_sse);
        assert!(imp_sse.contains("\"step\":\"unpack\""), "missing unpack: {}", imp_sse);
        assert!(imp_sse.contains("\"step\":\"db_import\""), "missing db_import: {}", imp_sse);
        assert!(imp_sse.contains("\"step\":\"wp_config\""), "missing wp_config: {}", imp_sse);
        assert!(imp_sse.contains("\"step\":\"ssl_start\""), "missing ssl_start: {}", imp_sse);
        assert!(imp_sse.contains("\"step\":\"complete\""), "missing complete: {}", imp_sse);

        // 3. Verify per-site PHP errors, access logs, and real-time logs/stream SSE
        std::fs::create_dir_all(state.config_dir.join("logs")).unwrap();
        std::fs::write(
            state.php_error_log_path("stream-wp.test"),
            "[27-Nov-2026 01:45:00 UTC] PHP Warning: test warning in index.php\n",
        )
        .unwrap();
        state.append_site_access_log("stream-wp.test", "HTTPS GET / -> 301 (Location: https://stream-wp.test/) [4ms]");

        let php_logs_res: serde_json::Value = c
            .get(format!("{}/sites/{}/php-errors", base, site_id))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert!(php_logs_res["logs"].as_str().unwrap().contains("PHP Warning: test warning"));

        let acc_logs_res: serde_json::Value = c
            .get(format!("{}/sites/{}/access-logs?type=access", base, site_id))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert!(acc_logs_res["logs"].as_str().unwrap().contains("301 (Location: https://stream-wp.test/)"));

        // Stream initial snapshot and ensure connection closes cleanly when aborted/timed out
        let mut stream_resp = c
            .get(format!("{}/sites/{}/logs/stream?type=php", base, site_id))
            .send()
            .await
            .unwrap();
        assert_eq!(stream_resp.status(), StatusCode::OK);
        let first_chunk = tokio::time::timeout(
            std::time::Duration::from_secs(2),
            stream_resp.chunk(),
        )
        .await
        .unwrap()
        .unwrap()
        .unwrap();
        let sse_txt = String::from_utf8_lossy(&first_chunk);
        assert!(sse_txt.contains("PHP Warning: test warning"), "SSE stream missed PHP log line: {}", sse_txt);

        server.abort();
        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_toggle_php_extension_in_ini_text() {
        let base_ini = ";extension_dir = \"ext\"\n;extension=intl\nextension=curl\n;zend_extension=opcache\n";
        let enabled_intl = toggle_php_extension_in_ini_text(base_ini, "intl", true);
        assert!(enabled_intl.contains("extension_dir = \"ext\""));
        assert!(enabled_intl.contains("\nextension=intl") || enabled_intl.contains("\r\nextension=intl"));

        let enabled_opcache = toggle_php_extension_in_ini_text(&enabled_intl, "opcache", true);
        assert!(enabled_opcache.contains("zend_extension=opcache"));

        let disabled_curl = toggle_php_extension_in_ini_text(&enabled_opcache, "curl", false);
        assert!(disabled_curl.contains(";extension=curl"));

        let appended_soap = toggle_php_extension_in_ini_text(&disabled_curl, "soap", true);
        assert!(appended_soap.contains("extension=soap"));
    }

    #[test]
    fn test_set_php_extension_rejects_invalid_and_builtins() {
        assert!(set_php_extension_enabled("8.3", "../bad", true).is_err());
        assert!(set_php_extension_enabled("8.3", "", true).is_err());
        assert!(set_php_extension_enabled("8.3", "core", false).is_err());
        assert!(set_php_extension_enabled("8.3", "standard", false).is_err());
    }

    #[cfg(windows)]
    #[test]
    fn test_win_metrics_cpu_processes_and_network() {
        let m = win_metrics::collect(std::path::Path::new("C:\\"));
        assert!(m.cpu_usage >= 0.0 && m.cpu_usage <= 100.0);
        assert!(m.mem_total_mb > 0);
        let (procs, total) = win_metrics::list_processes(10);
        assert!(total > 0);
        assert!(!procs.is_empty());
        assert!(procs[0]["mem_mb"].as_f64().unwrap_or(0.0) > 0.0);
        let _ifaces = win_metrics::list_network();
    }

    #[tokio::test]
    async fn test_batch3_wp_endpoints_and_telemetry_api() {
        let temp_dir = std::env::temp_dir().join(format!("dockpanel_b3_{}", uuid::Uuid::new_v4()));
        let state = Arc::new(AppState::new_with_dirs(temp_dir.join("config"), temp_dir.join("sites")));
        let site = state.create_site("b3wp.test", Some("8.3")).unwrap();
        let site_dir = state.site_dir(&site);
        wordpress::install_site_wordpress(
            &site_dir,
            "b3wp.test",
            &wordpress::InstallWpRequest {
                admin_user: Some("admin".to_string()),
                admin_pass: Some("pass1234".to_string()),
                admin_email: Some("a@b3wp.test".to_string()),
                title: Some("B3".to_string()),
                url: Some("http://b3wp.test".to_string()),
                db_engine: Some("sqlite".to_string()),
                db_port: None,
            },
        )
        .unwrap();

        state.record_site_traffic("b3wp.test", "/", 200, 4096, "127.0.0.1");
        state.push_metrics_point(serde_json::json!({ "cpu": 12.3, "mem": 45.6, "disk": 30.0, "time": "12:00:00" }));

        let app = build_api_router(state.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let srv = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let base = format!("http://127.0.0.1:{}", port);
        let c = reqwest::Client::new();

        let health: serde_json::Value = c.get(format!("{}/health", base)).send().await.unwrap().json().await.unwrap();
        assert_eq!(health["version"], env!("CARGO_PKG_VERSION"));

        let stats: serde_json::Value = c.get(format!("{}/sites/{}/stats", base, site.id)).send().await.unwrap().json().await.unwrap();
        assert_eq!(stats["requests"], 1);
        assert_eq!(stats["bytes_sent"], 4096);

        let bw: serde_json::Value = c.get(format!("{}/sites/{}/bandwidth-usage", base, site.id)).send().await.unwrap().json().await.unwrap();
        assert_eq!(bw["used_bytes_this_month"], 4096);

        let hist: serde_json::Value = c.get(format!("{}/dashboard/metrics-history", base)).send().await.unwrap().json().await.unwrap();
        assert_eq!(hist["points"].as_array().unwrap().len(), 1);

        let harden: serde_json::Value = c
            .post(format!("{}/sites/{}/wordpress/harden", base, site.id))
            .json(&serde_json::json!({ "fixes": ["disable-file-editing", "force-ssl"] }))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(harden.as_array().unwrap().len(), 2);

        let sec: serde_json::Value = c
            .get(format!("{}/sites/{}/wordpress/security-check", base, site.id))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert!(sec["checks"].as_array().unwrap().len() >= 6);

        let scan: serde_json::Value = c
            .post(format!("{}/sites/{}/wordpress/vuln-scan", base, site.id))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(scan["domain"], "b3wp.test");

        srv.abort();
        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[tokio::test]
    async fn test_discover_folders_and_site_database_import() {
        let temp_dir = std::env::temp_dir().join(format!(
            "dockpanel_discover_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let config_dir = temp_dir.join("cfg");
        let sites_dir = temp_dir.join("sites");
        std::fs::create_dir_all(&config_dir).unwrap();
        std::fs::create_dir_all(&sites_dir).unwrap();

        // Registered site folder (should be skipped by discovery)
        let reg_dir = sites_dir.join("already.test");
        std::fs::create_dir_all(&reg_dir).unwrap();
        std::fs::write(reg_dir.join("index.html"), "ok").unwrap();

        // Unregistered WordPress folder with an .sql dump inside
        let wp_dir = sites_dir.join("myshop.com");
        std::fs::create_dir_all(&wp_dir).unwrap();
        std::fs::write(
            wp_dir.join("wp-config.php"),
            "<?php define('DB_NAME', 'shop_db'); $table_prefix = 'wp_';",
        )
        .unwrap();
        std::fs::write(wp_dir.join("backup.sql"), "CREATE TABLE wp_options (id INT);").unwrap();

        // Unregistered Node folder
        let node_dir = sites_dir.join("api_backend");
        std::fs::create_dir_all(&node_dir).unwrap();
        std::fs::write(node_dir.join("package.json"), r#"{"name":"api"}"#).unwrap();

        // Hidden folder (should be skipped)
        std::fs::create_dir_all(sites_dir.join(".git")).unwrap();

        let state = Arc::new(AppState::new_with_dirs(config_dir, sites_dir.clone()));
        let reg_site = state
            .create_site_with_options("already.test", Some("static"), None, None, None, None)
            .unwrap();
        assert_eq!(reg_site.domain, "already.test");

        let app = build_api_router(state.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let srv = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let base = format!("http://127.0.0.1:{}", port);
        let c = reqwest::Client::new();

        let disc: serde_json::Value = c
            .get(format!("{}/desktop/discover-folders", base))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();

        let folders = disc["folders"].as_array().unwrap();
        assert_eq!(folders.len(), 2);
        assert_eq!(folders[0]["name"], "myshop.com");
        assert_eq!(folders[0]["suggested_domain"], "myshop.test");
        assert_eq!(folders[0]["suggested_runtime"], "php");
        assert_eq!(folders[0]["inspection"]["is_wordpress"], true);
        assert_eq!(folders[0]["detected_db_files"][0]["name"], "backup.sql");

        assert_eq!(folders[1]["name"], "api_backend");
        assert_eq!(folders[1]["suggested_domain"], "api-backend.test");
        assert_eq!(folders[1]["suggested_runtime"], "node");

        // Import api_backend and upload a SQLite DB payload to /sites/{id}/database/import
        let imported: serde_json::Value = c
            .post(format!("{}/sites", base))
            .json(&serde_json::json!({
                "domain": "api-backend.test",
                "runtime": "node",
                "document_root": node_dir.to_string_lossy().to_string(),
                "connect_existing": true
            }))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        let imported_id = imported["id"].as_str().unwrap();

        use base64::{engine::general_purpose::STANDARD as B64, Engine};
        let fake_sqlite = b"SQLite format 3\0test-db-bytes";
        let imp_db: serde_json::Value = c
            .post(format!("{}/sites/{}/database/import", base, imported_id))
            .json(&serde_json::json!({
                "filename": "local.sqlite",
                "content_base64": B64.encode(fake_sqlite)
            }))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(imp_db["ok"], true);
        assert_eq!(imp_db["engine"], "sqlite");

        // Re-discovering should now only show myshop.com
        let disc2: serde_json::Value = c
            .get(format!("{}/desktop/discover-folders", base))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(disc2["folders"].as_array().unwrap().len(), 1);

        srv.abort();
        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[tokio::test]
    async fn test_database_maintenance_health_optimize_and_prune() {
        let temp_dir = std::env::temp_dir().join(format!(
            "dockpanel_db_maint_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let config_dir = temp_dir.join("cfg");
        let sites_dir = temp_dir.join("sites");
        std::fs::create_dir_all(&config_dir).unwrap();
        std::fs::create_dir_all(&sites_dir).unwrap();

        let state = Arc::new(AppState::new_with_dirs(config_dir, sites_dir.clone()));

        // Create a temporary WP SQLite database inside a registered site
        let site = state
            .create_site_with_options("wpclean.test", Some("php"), None, None, None, None)
            .unwrap();
        let db_dir = state.site_dir(&site).join("wp-content").join("database");
        std::fs::create_dir_all(&db_dir).unwrap();
        let sqlite_path = db_dir.join(".ht.sqlite");

        if state::find_php_binary(None).is_none() {
            let _ = std::fs::remove_dir_all(&temp_dir);
            return;
        }

        // Seed WP tables + bloat + free pages
        std::fs::write(&sqlite_path, b"").unwrap();
        databases::run_sqlite_query(
            &sqlite_path,
            "CREATE TABLE wp_posts (ID INTEGER PRIMARY KEY, post_type TEXT, post_status TEXT, post_title TEXT)",
        )
        .unwrap();
        databases::run_sqlite_query(
            &sqlite_path,
            "CREATE TABLE wp_postmeta (meta_id INTEGER PRIMARY KEY, post_id INTEGER, meta_key TEXT)",
        )
        .unwrap();
        databases::run_sqlite_query(
            &sqlite_path,
            "CREATE TABLE wp_options (option_id INTEGER PRIMARY KEY, option_name TEXT, option_value TEXT, autoload TEXT)",
        )
        .unwrap();
        databases::run_sqlite_query(
            &sqlite_path,
            "INSERT INTO wp_posts (ID, post_type, post_status, post_title) VALUES (1, 'post', 'publish', 'Hello'), (2, 'revision', 'inherit', 'Rev1'), (3, 'post', 'auto-draft', 'Draft')",
        )
        .unwrap();
        databases::run_sqlite_query(
            &sqlite_path,
            "INSERT INTO wp_postmeta (meta_id, post_id, meta_key) VALUES (1, 1, 'valid'), (2, 9999, 'orphan')",
        )
        .unwrap();
        databases::run_sqlite_query(
            &sqlite_path,
            "INSERT INTO wp_options (option_id, option_name, option_value, autoload) VALUES (1, '_transient_timeout_old', '100', 'no'), (2, '_transient_old', 'payload', 'no')",
        )
        .unwrap();

        let db_id = format!("wp:{}", site.id);
        let app = build_api_router(state.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let srv = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let base = format!("http://127.0.0.1:{}", port);
        let c = reqwest::Client::new();

        // 1. Health endpoint detects WP bloat
        let health: serde_json::Value = c
            .get(format!("{}/databases/{}/health", base, db_id))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(health["wp_bloat"]["detected"], true);
        assert_eq!(health["wp_bloat"]["revisions"], 1);
        assert_eq!(health["wp_bloat"]["auto_drafts"], 1);
        assert_eq!(health["wp_bloat"]["orphaned_meta"], 1);
        assert_eq!(health["wp_bloat"]["expired_transients"], 1);

        // 2. Dry-run prune leaves data intact
        let dry: serde_json::Value = c
            .post(format!("{}/databases/{}/prune", base, db_id))
            .json(&serde_json::json!({
                "dry_run": true,
                "categories": ["revisions", "auto_drafts", "orphaned_meta", "expired_transients"]
            }))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(dry["dry_run"], true);
        assert_eq!(dry["rows_deleted"], 4);

        // 3. Apply prune + optimize
        let pruned: serde_json::Value = c
            .post(format!("{}/databases/{}/prune", base, db_id))
            .json(&serde_json::json!({
                "dry_run": false,
                "categories": ["revisions", "auto_drafts", "orphaned_meta", "expired_transients"],
                "optimize_after": true
            }))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(pruned["ok"], true);
        assert_eq!(pruned["optimized"], true);

        let health_after: serde_json::Value = c
            .get(format!("{}/databases/{}/health", base, db_id))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(health_after["wp_bloat"]["revisions"], 0);
        assert_eq!(health_after["wp_bloat"]["orphaned_meta"], 0);
        assert_eq!(health_after["wp_bloat"]["expired_transients"], 0);

        // 4. Direct optimize endpoint
        let opt: serde_json::Value = c
            .post(format!("{}/databases/{}/optimize", base, db_id))
            .json(&serde_json::json!({ "action": "optimize" }))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(opt["ok"], true);
        assert_eq!(opt["integrity_status"], "ok");

        // 5. Maintenance summary
        let summary: serde_json::Value = c
            .get(format!("{}/databases/maintenance/summary", base))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert!(summary["total_databases"].as_u64().unwrap() >= 1);

        srv.abort();
        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[tokio::test]
    async fn test_failed_import_rollback_preserves_files_and_returns_to_discover_folders() {
        let temp_dir = std::env::temp_dir().join(format!(
            "dockpanel_rollback_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let config_dir = temp_dir.join("cfg");
        let sites_dir = temp_dir.join("sites");
        let folder_dir = sites_dir.join("client-shop");
        std::fs::create_dir_all(&config_dir).unwrap();
        std::fs::create_dir_all(&folder_dir).unwrap();
        std::fs::write(folder_dir.join("index.php"), "<?php echo 'shop ok';").unwrap();

        let state = Arc::new(AppState::new_with_dirs(config_dir, sites_dir.clone()));
        let app = build_api_router(state.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let srv = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let base = format!("http://127.0.0.1:{}", port);
        let c = reqwest::Client::new();

        // 1. Register folder as a site (simulating initial step of import)
        let site: serde_json::Value = c
            .post(format!("{}/sites", base))
            .json(&serde_json::json!({
                "domain": "client-shop.local",
                "runtime": "php",
                "document_root": folder_dir.to_string_lossy().to_string(),
                "connect_existing": true
            }))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        let site_id = site["id"].as_str().unwrap();

        // Discover should NOT list it while registered
        let disc1: serde_json::Value = c
            .get(format!("{}/desktop/discover-folders", base))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(disc1["folders"].as_array().unwrap().len(), 0);

        // 2. Database import fails (invalid ZIP / payload) with rollback_on_failure=true
        use base64::{engine::general_purpose::STANDARD as B64, Engine};
        let fail_resp = c
            .post(format!("{}/sites/{}/database/import", base, site_id))
            .json(&serde_json::json!({
                "filename": "broken.zip",
                "content_base64": B64.encode(b"not-a-valid-zip"),
                "rollback_on_failure": true
            }))
            .send()
            .await
            .unwrap();
        assert_eq!(fail_resp.status(), StatusCode::BAD_REQUEST);

        // 3. Site registration was rolled back, files STILL EXIST on disk,
        // and discover-folders returns "client-shop" ready for retry!
        assert!(folder_dir.join("index.php").is_file());
        let disc2: serde_json::Value = c
            .get(format!("{}/desktop/discover-folders", base))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        let folders = disc2["folders"].as_array().unwrap();
        assert_eq!(folders.len(), 1);
        assert_eq!(folders[0]["name"], "client-shop");

        // 4. Explicit keep_files=true delete also preserves disk files
        let site2: serde_json::Value = c
            .post(format!("{}/sites", base))
            .json(&serde_json::json!({
                "domain": "client-shop.local",
                "runtime": "php",
                "document_root": folder_dir.to_string_lossy().to_string(),
                "connect_existing": true
            }))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        let del: serde_json::Value = c
            .delete(format!("{}/sites/{}?keep_files=true", base, site2["id"].as_str().unwrap()))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(del["success"], true);
        assert_eq!(del["kept_files"], true);
        assert!(folder_dir.join("index.php").is_file());

        // 5. Verify workable memory limit suggestion message
        let hint = mariadb::suggest_memory_limit_hint(
            "Allowed memory size of 134217728 bytes exhausted",
            Some(512),
        )
        .unwrap();
        assert!(hint.contains("2048 MB"));

        srv.abort();
        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[tokio::test]
    async fn test_recursive_subdir_sql_discovery_and_directory_db_selection() {
        let temp_dir = std::env::temp_dir().join(format!(
            "dockpanel_subdir_sql_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let config_dir = temp_dir.join("cfg");
        let sites_dir = temp_dir.join("sites");
        let site_folder = sites_dir.join("furniture-schools2");
        let external_dir = temp_dir.join("external_backups");
        std::fs::create_dir_all(&config_dir).unwrap();
        std::fs::create_dir_all(site_folder.join("app").join("sql")).unwrap();
        std::fs::create_dir_all(site_folder.join("wp-content").join("exports")).unwrap();
        std::fs::create_dir_all(sites_dir.join("_shared_backups").join("nested")).unwrap();
        std::fs::create_dir_all(external_dir.join("client_archives")).unwrap();

        std::fs::write(
            site_folder.join("wp-config.php"),
            "<?php define('DB_NAME', 'furniture_db'); $table_prefix = 'wpl8_'; define('WP_SITEURL', 'https://furnitureforschools.co.uk');",
        )
        .unwrap();

        // Matching SQL dump inside nested subdirectory app/sql/
        std::fs::write(
            site_folder.join("app").join("sql").join("local.sql"),
            "-- Database: `furniture_db`\nCREATE TABLE `wpl8_options` (id INT);\nINSERT INTO `wpl8_options` VALUES (1, 'siteurl', 'https://furnitureforschools.co.uk');\n",
        )
        .unwrap();

        // Mismatched SQL dump (belongs to another site) inside wp-content/exports/
        std::fs::write(
            site_folder.join("wp-content").join("exports").join("other_blog.sql"),
            "-- Database: `unrelated_blog`\nCREATE TABLE `other_options` (id INT);\nINSERT INTO `other_options` VALUES (1, 'siteurl', 'https://unrelated-blog.com');\n",
        )
        .unwrap();

        // Sibling directory SQL dump under Default Websites Directory
        std::fs::write(
            sites_dir.join("_shared_backups").join("nested").join("furniture_alt.sql"),
            "CREATE TABLE `wpl8_options` (id INT);\nINSERT INTO `wpl8_options` VALUES (1, 'siteurl', 'https://furnitureforschools.co.uk');\n",
        )
        .unwrap();

        // External backup directory SQLite dump outside sites_dir
        let ext_sqlite = external_dir.join("client_archives").join("chosen_from_dir.sqlite");
        std::fs::write(&ext_sqlite, b"SQLite format 3\0dir-selected").unwrap();

        let state = Arc::new(AppState::new_with_dirs(config_dir, sites_dir.clone()));
        let app = build_api_router(state.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let srv = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let base = format!("http://127.0.0.1:{}", port);
        let c = reqwest::Client::new();

        // 1. Discover folders searches arbitrary nested subdirectories recursively & scores match/mismatch
        let disc: serde_json::Value = c
            .get(format!("{}/desktop/discover-folders", base))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        let folders = disc["folders"].as_array().unwrap();
        let folder = folders.iter().find(|f| f["name"] == "furniture-schools2").unwrap();
        let detected = folder["detected_db_files"].as_array().unwrap();
        assert_eq!(detected.len(), 2);
        // Matching SQL dump should rank first with matches_site = true and prefix/url populated
        assert_eq!(detected[0]["relative_path"], "app/sql/local.sql");
        assert_eq!(detected[0]["matches_site"], true);
        assert_eq!(detected[0]["detected_prefix"], "wpl8_");
        assert_eq!(detected[0]["detected_site_url"], "https://furnitureforschools.co.uk");
        // Mismatched dump from another website should be flagged matches_site = false
        assert_eq!(detected[1]["relative_path"], "wp-content/exports/other_blog.sql");
        assert_eq!(detected[1]["matches_site"], false);

        // Root/sibling directory scan also surfaces _shared_backups/nested/furniture_alt.sql
        let root_dumps = disc["root_db_files"].as_array().unwrap();
        assert!(root_dumps.iter().any(|d| d["name"] == "furniture_alt.sql"));

        // 2. Inspect folder endpoint also returns recursive detected_db_files & other_directory_db_files
        let insp: serde_json::Value = c
            .post(format!("{}/desktop/inspect-folder", base))
            .json(&serde_json::json!({ "path": site_folder.to_string_lossy().to_string() }))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(insp["detected_db_files"].as_array().unwrap().len(), 2);
        assert!(insp["other_directory_db_files"]
            .as_array()
            .unwrap()
            .iter()
            .any(|d| d["name"] == "furniture_alt.sql" && d["matches_site"] == true));

        // 3. Scanning an external directory via /desktop/scan-db-files finds nested dumps & approves dir
        let scan: serde_json::Value = c
            .post(format!("{}/desktop/scan-db-files", base))
            .json(&serde_json::json!({
                "dir": external_dir.to_string_lossy().to_string(),
                "site_dir": site_folder.to_string_lossy().to_string()
            }))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        let ext_files = scan["files"].as_array().unwrap();
        assert_eq!(ext_files.len(), 1);
        assert_eq!(ext_files[0]["relative_path"], "client_archives/chosen_from_dir.sqlite");

        // 4. Site can import the database file selected from that scanned external directory using local_path
        let created: serde_json::Value = c
            .post(format!("{}/sites", base))
            .json(&serde_json::json!({
                "domain": "furniture-schools2.test",
                "runtime": "php",
                "document_root": site_folder.to_string_lossy().to_string(),
                "connect_existing": true
            }))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        let imp: serde_json::Value = c
            .post(format!("{}/sites/{}/database/import", base, created["id"].as_str().unwrap()))
            .json(&serde_json::json!({
                "filename": "chosen_from_dir.sqlite",
                "local_path": ext_files[0]["path"]
            }))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(imp["ok"], true);
        assert_eq!(imp["engine"], "sqlite");

        srv.abort();
        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[tokio::test]
    async fn test_site_git_endpoints_lifecycle() {
        if git_repo::find_git_binary().is_none() {
            return;
        }
        let temp_dir = std::env::temp_dir().join(format!(
            "dockpanel_site_git_api_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let config_dir = temp_dir.join("cfg");
        let sites_dir = temp_dir.join("sites");
        std::fs::create_dir_all(&config_dir).unwrap();
        std::fs::create_dir_all(&sites_dir).unwrap();

        let state = Arc::new(AppState::new_with_dirs(config_dir, sites_dir));
        let site = state
            .create_site_with_options("git-admin.test", Some("php"), None, None, None, None)
            .unwrap();
        let app = build_api_router(state.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let srv = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let base = format!("http://127.0.0.1:{}", port);
        let c = reqwest::Client::new();

        let st0: serde_json::Value = c
            .get(format!("{}/sites/{}/git/status", base, site.id))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(st0["git_available"], true);
        assert_eq!(st0["initialized"], false);

        let init_res: serde_json::Value = c
            .post(format!("{}/sites/{}/git/init", base, site.id))
            .json(&serde_json::json!({
                "branch": "main",
                "gitignore_preset": "php",
                "initial_commit": true,
                "commit_message": "Admin initial commit"
            }))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(init_res["ok"], true);
        assert_eq!(init_res["status"]["initialized"], true);
        assert_eq!(init_res["status"]["branch"], "main");
        assert_eq!(init_res["status"]["has_gitignore"], true);

        let bad_url_resp = c
            .post(format!("{}/sites/{}/git/connect", base, site.id))
            .json(&serde_json::json!({ "repo_url": "file:///invalid/scheme" }))
            .send()
            .await
            .unwrap();
        assert_eq!(bad_url_resp.status(), StatusCode::BAD_REQUEST);

        srv.abort();
        let _ = std::fs::remove_dir_all(&temp_dir);
    }
}
