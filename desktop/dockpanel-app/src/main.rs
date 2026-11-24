#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod hosts_manager;
mod state;

use axum::{
    extract::{Path, State},
    http::{header, StatusCode},
    response::{IntoResponse, Response},
    routing::{delete, get, post},
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
    Json(site).into_response()
}

async fn delete_site_handler(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Json<serde_json::Value> {
    let ok = state.delete_site(&id);
    Json(serde_json::json!({ "success": ok }))
}

fn start_embedded_server(state: Arc<AppState>) -> u16 {
    let (port_tx, port_rx) = std::sync::mpsc::channel::<u16>();

    std::thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .expect("Failed to create tokio runtime");

        rt.block_on(async move {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
                .await
                .expect("Failed to bind loopback TCP listener");
            let port = listener.local_addr().unwrap().port();
            let _ = port_tx.send(port);

            let api_routes = Router::new()
                .route("/auth/setup-status", get(|| async { Json(serde_json::json!({"needs_setup": false})) }))
                .route("/auth/me", get(|| async {
                    Json(serde_json::json!({
                        "id": "00000000-0000-0000-0000-000000000001",
                        "email": "local@dockpanel.desktop",
                        "role": "admin"
                    }))
                }))
                .route("/auth/desktop-bootstrap", post(|| async {
                    Json(serde_json::json!({
                        "token": "desktop-local-token",
                        "user": {
                            "id": "00000000-0000-0000-0000-000000000001",
                            "email": "local@dockpanel.desktop",
                            "role": "admin"
                        }
                    }))
                }))
                .route("/auth/2fa/status", get(|| async { Json(serde_json::json!({"enabled": false})) }))
                .route("/auth/passkeys", get(|| async { Json(serde_json::json!({"passkeys": []})) }))
                .route("/auth/sessions", get(|| async { Json(serde_json::json!([])) }))
                .route("/branding", get(|| async {
                    Json(serde_json::json!({
                        "panel_name": "DockPanel Desktop",
                        "logo_url": "",
                        "accent_color": "",
                        "hide_branding": false,
                        "oauth_providers": []
                    }))
                }))
                .route("/servers", get(|| async {
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
                }))
                .route("/sites", get(list_sites_handler).post(create_site_handler))
                .route("/admin/sites", get(list_sites_handler))
                .route("/sites/{id}", get(get_site_handler).delete(delete_site_handler))
                .route("/sites/metrics-overview", get(|| async { Json(serde_json::json!({})) }))
                .route("/sites/{id}/sync/preflight", post(|| async {
                    Json(serde_json::json!({
                        "success": true,
                        "message": "Remote server connection verified",
                        "remote_site_id": "remote-site-123"
                    }))
                }))
                .route("/sites/{id}/sync/push", post(|| async {
                    Json(serde_json::json!({
                        "success": true,
                        "message": "Sync completed successfully",
                        "steps": ["Connected", "Synchronized", "Complete"]
                    }))
                }))
                .route("/sites/{id}/sync/pull", post(|| async {
                    Json(serde_json::json!({
                        "success": true,
                        "message": "Sync completed successfully",
                        "steps": ["Connected", "Synchronized", "Complete"]
                    }))
                }))
                .route("/users", get(|| async {
                    Json(serde_json::json!([{
                        "id": "00000000-0000-0000-0000-000000000001",
                        "email": "local@dockpanel.desktop",
                        "role": "admin"
                    }]))
                }))
                .route("/alerts/summary", get(|| async { Json(serde_json::json!({"firing": 0, "open": 0})) }))
                .route("/incidents/summary", get(|| async { Json(serde_json::json!({"open": 0})) }))
                .route("/notifications/unread-count", get(|| async { Json(serde_json::json!({"count": 0})) }))
                .route("/notifications", get(|| async { Json(serde_json::json!([])) }))
                .route("/php/versions", get(|| async {
                    Json(serde_json::json!({
                        "versions": [
                            {"version": "8.3", "installed": true, "default": true},
                            {"version": "8.2", "installed": true, "default": false}
                        ]
                    }))
                }))
                .route("/databases", get(|| async { Json(serde_json::json!([])) }))
                .route("/activity", get(|| async { Json(serde_json::json!([])) }))
                .route("/metrics", get(|| async { Json(serde_json::json!({"cpu_percent": 2.5, "memory_mb": 145, "disk_gb": 12.4})) }))
                .route("/system/status", get(|| async {
                    Json(serde_json::json!({
                        "status": "running",
                        "mode": "desktop",
                        "os": "windows",
                        "version": "2.248.5"
                    }))
                }))
                .route("/system/info", get(|| async {
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
                }))
                .route("/system/processes", get(|| async { Json(serde_json::json!([])) }))
                .route("/system/network", get(|| async { Json(serde_json::json!([])) }))
                .route("/telemetry/update-status", get(|| async { Json(serde_json::json!({"update_available": false})) }))
                .route("/tls-certificates", get(|| async { Json(serde_json::json!([])) }))
                .route("/dashboard/intelligence", get(|| async {
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
                }))
                .route("/dashboard/fleet", get(|| async {
                    Json(serde_json::json!({
                        "servers": [],
                        "total_servers": 1,
                        "total_firing": 0,
                        "total_incidents": 0,
                        "total_sites": 2
                    }))
                }))
                .route("/backup-setup-status", get(|| async {
                    Json(serde_json::json!({"has_schedule": true, "has_backup": true}))
                }))
                .route("/dashboard/docker", get(|| async {
                    Json(serde_json::json!({"total": 2, "running": 2, "stopped": 0}))
                }))
                .route("/dashboard/timeline", get(|| async { Json(serde_json::json!([])) }))
                .route("/apps/images", get(|| async { Json(serde_json::json!([])) }))
                .route("/system/disk-io", get(|| async {
                    Json(serde_json::json!({"read_bytes_sec": 0, "write_bytes_sec": 0}))
                }))
                .route("/mail/queue", get(|| async {
                    Json(serde_json::json!({"count": 0, "queue": []}))
                }))
                .route("/on-call/whoami", get(|| async { Json(serde_json::json!([])) }))
                .route("/apps", get(|| async { Json(serde_json::json!([])) }))
                .route("/system/updates/count", get(|| async {
                    Json(serde_json::json!({"count": 0, "security": 0, "reboot_required": false}))
                }))
                .route("/dashboard/metrics-history", get(|| async {
                    Json(serde_json::json!({"points": []}))
                }))
                .fallback(|| async { (StatusCode::NOT_FOUND, Json(serde_json::json!({"error": "Not found"}))) })
                .with_state(state);

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
    let state = AppState::new();

    // Auto-seed demo sites if empty
    if state.list_sites().is_empty() {
        state.create_site("mytestdomain.local", Some("8.3"));
        state.create_site("wordpress.test", Some("8.3"));
    }

    // Start in-memory embedded HTTP server
    let port = start_embedded_server(Arc::new(state));

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

    event_loop.run(move |event, _, control_flow| {
        let _ = (&_webview, &window);
        *control_flow = ControlFlow::Wait;
        if let Event::WindowEvent {
            event: WindowEvent::CloseRequested,
            ..
        } = event
        {
            *control_flow = ControlFlow::Exit;
        }
    });
}
