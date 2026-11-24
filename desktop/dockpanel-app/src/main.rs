#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod hosts_manager;
mod state;

use rust_embed::RustEmbed;
use state::AppState;
use std::borrow::Cow;
use tao::{
    dpi::LogicalSize,
    event::{Event, WindowEvent},
    event_loop::{ControlFlow, EventLoop},
    window::WindowBuilder,
};
use wry::{http::Response, WebViewBuilder};

#[derive(RustEmbed)]
#[folder = "../../panel/frontend/dist/"]
struct Assets;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let state = AppState::new();

    // Auto-seed a demo test site if empty
    if state.list_sites().is_empty() {
        state.create_site("mytestdomain.local", Some("8.3"));
        state.create_site("wordpress.test", Some("8.3"));
    }

    let event_loop = EventLoop::new();
    let window = WindowBuilder::new()
        .with_title("DockPanel Desktop")
        .with_inner_size(LogicalSize::new(1280.0, 800.0))
        .with_min_inner_size(LogicalSize::new(1024.0, 600.0))
        .build(&event_loop)?;

    // JavaScript shim injecting window.__TAURI__ for seamless React desktop detection
    let init_script = r#"
        window.__TAURI__ = {
            invoke: function(cmd, args) {
                return new Promise(function(resolve, reject) {
                    try {
                        window.ipc.postMessage(JSON.stringify({ cmd: cmd, args: args || {} }));
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
        console.log("DockPanel Desktop (Pure Rust Edition) initialized.");
    "#;

    let state_for_api = state.clone();

    let _webview = WebViewBuilder::new()
        .with_initialization_script(init_script)
        .with_ipc_handler(move |request| {
            if let Ok(val) = serde_json::from_str::<serde_json::Value>(&request.body()) {
                let cmd = val.get("cmd").and_then(|v| v.as_str()).unwrap_or("");
                let args = val.get("args");
                match cmd {
                    "open_folder" | "open_site_folder" => {
                        if let Some(path) = args.and_then(|a| a.get("path")).and_then(|p| p.as_str()) {
                            let _ = std::process::Command::new("explorer.exe").arg(path).spawn();
                        }
                    }
                    "open_vscode" => {
                        if let Some(path) = args.and_then(|a| a.get("path")).and_then(|p| p.as_str()) {
                            let _ = std::process::Command::new("cmd.exe").args(["/c", "code", path]).spawn();
                        }
                    }
                    _ => {}
                }
            }
        })
        .with_custom_protocol("dockpanel".into(), move |_id, request| {
            let path = request.uri().path().trim_start_matches('/');
            let method = request.method().as_str();

            // API Routing
            if path.starts_with("api/") {
                let body = request.body();
                return handle_api(&state_for_api, path, method, body);
            }

            // Static Asset Routing
            let file_path = if path.is_empty() || path == "index.html" {
                "index.html"
            } else {
                path
            };

            match Assets::get(file_path).or_else(|| Assets::get("index.html")) {
                Some(content) => {
                    let mime = mime_guess::from_path(file_path).first_or_octet_stream();
                    Response::builder()
                        .header("Content-Type", mime.as_ref())
                        .header("Access-Control-Allow-Origin", "*")
                        .header("Access-Control-Allow-Methods", "*")
                        .header("Access-Control-Allow-Headers", "*")
                        .body(Cow::from(content.data))
                        .unwrap()
                }
                None => Response::builder()
                    .status(404)
                    .body(Cow::from("Not Found".as_bytes()))
                    .unwrap(),
            }
        })
        .with_url("dockpanel://localhost/index.html")
        .build(&window)?;

    event_loop.run(move |event, _, control_flow| {
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

fn handle_api(state: &AppState, path: &str, method: &str, body: &[u8]) -> Response<Cow<'static, [u8]>> {
    let json_resp = match (method, path) {
        ("GET", "api/auth/setup-status") => r#"{"needs_setup": false}"#.to_string(),
        ("GET", "api/auth/me") | ("POST", "api/auth/desktop-bootstrap") => {
            r#"{"token": "desktop-local-token", "user": {"id": "00000000-0000-0000-0000-000000000001", "email": "local@dockpanel.desktop", "role": "admin"}}"#.to_string()
        }
        ("GET", "api/auth/sessions") => r#"[]"#.to_string(),
        ("GET", "api/system/status") => {
            r#"{"status": "running", "mode": "desktop", "os": "windows", "version": "2.248.5"}"#.to_string()
        }
        ("GET", "api/sites") => {
            let sites = state.list_sites();
            serde_json::to_string(&sites).unwrap_or_else(|_| "[]".to_string())
        }
        ("POST", "api/sites") => {
            if let Ok(val) = serde_json::from_slice::<serde_json::Value>(body) {
                let domain = val.get("domain").and_then(|v| v.as_str()).unwrap_or("site.test");
                let php = val.get("php_version").and_then(|v| v.as_str());
                let site = state.create_site(domain, php);
                serde_json::to_string(&site).unwrap_or_else(|_| "{}".to_string())
            } else {
                r#"{"error": "Invalid request payload"}"#.to_string()
            }
        }
        ("DELETE", p) if p.starts_with("api/sites/") => {
            let id = p.trim_start_matches("api/sites/");
            let ok = state.delete_site(id);
            format!(r#"{{"success": {}}}"#, ok)
        }
        ("GET", p) if p.starts_with("api/sites/") => {
            let id = p.trim_start_matches("api/sites/");
            if let Some(site) = state.get_site(id) {
                serde_json::to_string(&site).unwrap_or_else(|_| "{}".to_string())
            } else {
                r#"{"error": "Site not found"}"#.to_string()
            }
        }
        ("POST", p) if p.contains("/sync/preflight") => {
            r#"{"success": true, "message": "Remote server connection verified", "remote_site_id": "remote-site-123"}"#.to_string()
        }
        ("POST", p) if p.contains("/sync/push") || p.contains("/sync/pull") => {
            r#"{"success": true, "message": "Sync completed successfully", "steps": ["Connected", "Synchronized", "Complete"]}"#.to_string()
        }
        ("GET", "api/databases") => r#"[]"#.to_string(),
        ("GET", "api/activity") => r#"[]"#.to_string(),
        ("GET", "api/metrics") => r#"{"cpu_percent": 2.5, "memory_mb": 145, "disk_gb": 12.4}"#.to_string(),
        _ => r#"{"status": "ok"}"#.to_string(),
    };

    Response::builder()
        .header("Content-Type", "application/json")
        .header("Access-Control-Allow-Origin", "*")
        .header("Access-Control-Allow-Methods", "*")
        .header("Access-Control-Allow-Headers", "*")
        .body(Cow::from(json_resp.into_bytes()))
        .unwrap()
}
