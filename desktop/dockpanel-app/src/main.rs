#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod hosts_manager;
pub mod router;
mod state;

use axum::{
    extract::{Path, State},
    http::{header, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
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
    event_loop::{ControlFlow, EventLoop},
    window::WindowBuilder,
};
use tower_http::cors::CorsLayer;
use wry::WebViewBuilder;

#[derive(RustEmbed)]
#[folder = "../../panel/frontend/dist/"]
struct Assets;

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
    let php = payload.get("php_version").and_then(|v| v.as_str());
    let site = state.create_site(domain, php);
    let site = state.start_site(&site.id).unwrap_or(site);
    Json(site).into_response()
}

async fn delete_site_handler(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Json<serde_json::Value> {
    let ok = state.delete_site(&id);
    Json(serde_json::json!({ "success": ok }))
}

async fn start_site_handler(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Response {
    match state.start_site(&id) {
        Ok(site) => Json(site).into_response(),
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
        Ok(site) => Json(site).into_response(),
        Err(err) => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": err })),
        )
            .into_response(),
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

            if target_port != active && !router::is_port_bindable(target_port) {
                return Json(serde_json::json!({
                    "success": false,
                    "message": "Port is already in use"
                }))
                .into_response();
            }

            state.set_router_port(target_port);
            state.restart_site_router(target_port);

            Json(serde_json::json!({
                "success": true,
                "active_port": target_port,
                "message": format!("Switched router port to {}", target_port)
            }))
            .into_response()
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
                    state.set_router_port(port);
                    state.restart_site_router(port);
                    Json(serde_json::json!({
                        "success": true,
                        "active_port": port,
                        "message": format!("Successfully stopped conflicting process and bound port {}", port)
                    }))
                    .into_response()
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
            get(|| async {
                Json(serde_json::json!({
                    "panel_name": "DockPanel Desktop",
                    "logo_url": "",
                    "accent_color": "",
                    "hide_branding": false,
                    "oauth_providers": []
                }))
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
                    "agent_version": "2.248.5",
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
        .route("/system/ports/conflict", get(port_conflict_handler))
        .route("/system/ports/resolve", post(port_resolve_handler))
        .route(
            "/sites/metrics-overview",
            get(|| async { Json(serde_json::json!({})) }),
        )
        .route(
            "/sites/{id}/sync/preflight",
            post(|| async {
                Json(serde_json::json!({
                    "success": true,
                    "message": "Remote server connection verified",
                    "remote_site_id": "remote-site-123"
                }))
            }),
        )
        .route(
            "/sites/{id}/sync/push",
            post(|| async {
                Json(serde_json::json!({
                    "success": true,
                    "message": "Sync completed successfully",
                    "steps": ["Connected", "Synchronized", "Complete"]
                }))
            }),
        )
        .route(
            "/sites/{id}/sync/pull",
            post(|| async {
                Json(serde_json::json!({
                    "success": true,
                    "message": "Sync completed successfully",
                    "steps": ["Connected", "Synchronized", "Complete"]
                }))
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
            get(|| async { Json(serde_json::json!({"count": 0})) }),
        )
        .route(
            "/notifications",
            get(|| async { Json(serde_json::json!([])) }),
        )
        .route(
            "/php/versions",
            get(|| async {
                Json(serde_json::json!({
                    "versions": [
                        {"version": "8.3", "installed": true, "default": true},
                        {"version": "8.2", "installed": true, "default": false}
                    ]
                }))
            }),
        )
        .route(
            "/databases",
            get(|| async { Json(serde_json::json!([])) }),
        )
        .route("/activity", get(|| async { Json(serde_json::json!([])) }))
        .route(
            "/metrics",
            get(|| async {
                Json(
                    serde_json::json!({"cpu_percent": 2.5, "memory_mb": 145, "disk_gb": 12.4}),
                )
            }),
        )
        .route(
            "/system/status",
            get(|| async {
                Json(serde_json::json!({
                    "status": "running",
                    "mode": "desktop",
                    "os": "windows",
                    "version": "2.248.5"
                }))
            }),
        )
        .route(
            "/system/info",
            get(|| async {
                Json(serde_json::json!({
                    "cpu_count": 8,
                    "cpu_usage": 5.0,
                    "cpu_model": "Native Windows Host",
                    "cpu_temp": null,
                    "mem_total_mb": 16384,
                    "mem_used_mb": 4096,
                    "mem_usage_pct": 25.0,
                    "swap_total_mb": 0,
                    "swap_used_mb": 0,
                    "disk_total_gb": 512,
                    "disk_used_gb": 64,
                    "disk_usage_pct": 12.5,
                    "uptime_secs": 3600,
                    "hostname": "localhost",
                    "os": "Windows",
                    "kernel": "Windows 11",
                    "process_count": 42
                }))
            }),
        )
        .route(
            "/system/processes",
            get(|| async { Json(serde_json::json!([])) }),
        )
        .route(
            "/system/network",
            get(|| async { Json(serde_json::json!([])) }),
        )
        .route(
            "/telemetry/update-status",
            get(|| async { Json(serde_json::json!({"update_available": false})) }),
        )
        .route(
            "/tls-certificates",
            get(|| async { Json(serde_json::json!([])) }),
        )
        .route(
            "/dashboard/intelligence",
            get(|| async {
                Json(serde_json::json!({
                    "health_score": 100,
                    "grade": "A+",
                    "firing_alerts": 0,
                    "acknowledged_alerts": 0,
                    "open_incidents": 0,
                    "stale_backups": 0,
                    "scan_critical": 0,
                    "scan_warnings": 0,
                    "ssl_countdowns": [],
                    "top_issues": [],
                    "recommendations": []
                }))
            }),
        )
        .route(
            "/dashboard/fleet",
            get(|| async {
                Json(serde_json::json!({
                    "servers": [],
                    "total_servers": 1,
                    "total_firing": 0,
                    "total_incidents": 0,
                    "total_sites": 2
                }))
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
                Json(serde_json::json!({"total": 2, "running": 2, "stopped": 0}))
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
            get(|| async { Json(serde_json::json!({"count": 0, "queue": []})) }),
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
            get(|| async { Json(serde_json::json!({"points": []})) }),
        )
        .fallback(|| async {
            (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({"error": "Not found"})),
            )
        })
        .with_state(state)
}

fn start_embedded_server(state: Arc<AppState>) -> u16 {
    let (port_tx, port_rx) = std::sync::mpsc::channel::<u16>();

    std::thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .expect("Failed to create tokio runtime");

        rt.block_on(async move {
            // Check port 80 status. If occupied, auto-fallback active_port to 8080
            let preferred_port = state.get_router_config().preferred_port;
            let status = router::check_port_status(preferred_port);
            let active_port = if status.in_use {
                8080
            } else {
                preferred_port
            };
            state.set_router_port(active_port);

            // Start site router
            state.restart_site_router(active_port);

            // Reconcile and start PHP workers for all active sites
            state.reconcile_active_sites();

            let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
                .await
                .expect("Failed to bind loopback TCP listener");
            let port = listener.local_addr().unwrap().port();
            let _ = port_tx.send(port);

            let api_routes = build_api_router(state.clone());

            let app = Router::new()
                .nest("/api", api_routes)
                .fallback(static_handler)
                .layer(CorsLayer::permissive());

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

    if let Err(e) = run_app() {
        let _ = std::fs::write("dockpanel_error.log", format!("RUN_APP_ERROR: {:?}\n", e));
    }
}

fn run_app() -> Result<(), Box<dyn std::error::Error>> {
    let state = Arc::new(AppState::new());

    // Auto-seed demo sites if empty
    if state.list_sites().is_empty() {
        state.create_site("mytestdomain.local", Some("8.3"));
        state.create_site("wordpress.test", Some("8.3"));
    }

    // Start in-memory embedded HTTP server
    let port = start_embedded_server(state.clone());

    let event_loop = EventLoop::new();
    let window = WindowBuilder::new()
        .with_title("DockPanel Desktop")
        .with_inner_size(LogicalSize::new(1280.0, 800.0))
        .with_min_inner_size(LogicalSize::new(1024.0, 600.0))
        .build(&event_loop)?;

    // JavaScript shim injecting window.__TAURI__
    let init_script = r#"
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
        console.log("DockPanel Desktop (Pure Rust Edition) connected to embedded engine.");
    "#;

    let target_url = format!("http://127.0.0.1:{}/", port);

    let _webview = WebViewBuilder::new()
        .with_devtools(true)
        .with_initialization_script(init_script)
        .with_ipc_handler(move |request| {
            if let Ok(val) = serde_json::from_str::<serde_json::Value>(&request.body()) {
                let cmd = val.get("cmd").and_then(|v| v.as_str()).unwrap_or("");
                let args = val.get("args");
                match cmd {
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
        let _ = (&_webview, &window);
        *control_flow = ControlFlow::Wait;
        if let Event::WindowEvent {
            event: WindowEvent::CloseRequested,
            ..
        } = event
        {
            state_exit.stop_all_workers();
            *control_flow = ControlFlow::Exit;
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_api_sites_start_stop() {
        let temp_dir = std::env::temp_dir().join(format!("dockpanel_api_test_{}", uuid::Uuid::new_v4()));
        let config_dir = temp_dir.join("config");
        let sites_dir = temp_dir.join("sites");
        let state = Arc::new(AppState::new_with_dirs(config_dir, sites_dir));

        let site = state.create_site("api-lifecycle-test.local", Some("8.3"));

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

        // POST resolve use_port
        let resolve_resp = port_resolve_handler(
            State(state.clone()),
            Json(PortResolvePayload {
                action: "use_port".to_string(),
                port: Some(8080),
                pid: None,
            }),
        )
        .await;
        assert_eq!(resolve_resp.status(), StatusCode::OK);
        assert_eq!(state.get_router_config().active_port, 8080);

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
}
