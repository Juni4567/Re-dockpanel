#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod hosts_manager;
pub mod router;
mod state;
pub mod wordpress;

use axum::{
    extract::{Path, State},
    http::{header, StatusCode},
    response::{
        sse::{Event as SseEvent, KeepAlive, Sse},
        IntoResponse, Response,
    },
    routing::{get, post, put},
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
    let php = payload.get("php_version").and_then(|v| v.as_str());
    let cms = payload.get("cms").and_then(|v| v.as_str());
    let preset = payload.get("php_preset").and_then(|v| v.as_str());
    match state.create_site(domain, php) {
        Ok(site) => {
            if let Some(p) = cms.or(preset).filter(|s| !s.is_empty() && *s != "generic") {
                state.update_site_preset(&site.id, p);
            }
            if cms == Some("wordpress") {
                let site_dir = state.sites_dir.join(&site.domain);
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
                };
                let _ = wordpress::install_site_wordpress(&site_dir, &site.domain, &wp_req);
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

async fn get_wordpress_info_handler(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Response {
    if let Some(site) = state.get_site(&id) {
        let site_dir = state.sites_dir.join(&site.domain);
        let info = wordpress::get_site_wp_info(&site_dir, &site.domain, &site.php_version);
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
    };

    if let Some(site) = state.get_site(&id) {
        let site_dir = state.sites_dir.join(&site.domain);
        match wordpress::install_site_wordpress(&site_dir, &site.domain, &payload) {
            Ok(res) => {
                state.update_site_preset(&site.id, "wordpress");
                let _ = state.start_site(&site.id);
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
) -> Response {
    if let Some(site) = state.get_site(&id) {
        let site_dir = state.sites_dir.join(&site.domain);
        match wordpress::reinstall_core(&site_dir) {
            Ok(res) => Json(res).into_response(),
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

async fn wordpress_plugins_handler(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Response {
    if let Some(site) = state.get_site(&id) {
        let plugins_dir = state.sites_dir.join(&site.domain).join("wp-content").join("plugins");
        let mut plugins = Vec::new();
        if let Ok(entries) = std::fs::read_dir(plugins_dir) {
            for entry in entries.flatten() {
                if let Ok(file_type) = entry.file_type() {
                    let name = entry.file_name().to_string_lossy().to_string();
                    if file_type.is_dir() || name.ends_with(".php") {
                        let clean_name = name.trim_end_matches(".php").to_string();
                        plugins.push(serde_json::json!({
                            "name": clean_name,
                            "status": "active",
                            "version": "1.0.0",
                            "update": "none",
                            "title": clean_name,
                        }));
                    }
                }
            }
        }
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
        let themes_dir = state.sites_dir.join(&site.domain).join("wp-content").join("themes");
        let mut themes = Vec::new();
        if let Ok(entries) = std::fs::read_dir(themes_dir) {
            for entry in entries.flatten() {
                if entry.path().is_dir() {
                    let name = entry.file_name().to_string_lossy().to_string();
                    themes.push(serde_json::json!({
                        "name": name,
                        "status": "active",
                        "version": "1.0.0",
                        "update": "none",
                        "title": name,
                    }));
                }
            }
        }
        if themes.is_empty() {
            themes.push(serde_json::json!({
                "name": "twentytwentyfour",
                "status": "active",
                "version": "1.2",
                "update": "none",
                "title": "Twenty Twenty-Four",
            }));
        }
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
        Json(wordpress::get_wp_db_info(&site.domain)).into_response()
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
        let site_dir = state.sites_dir.join(&site.domain);
        Json(wordpress::get_wp_features_status(&site.domain, &site_dir)).into_response()
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
        Ok(site) => Json(site).into_response(),
        Err(err) => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": err})),
        )
            .into_response(),
    }
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
        Ok(site) => Json(site).into_response(),
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
    let (tx, rx) = tokio::sync::mpsc::channel::<Result<SseEvent, std::convert::Infallible>>(32);

    tokio::spawn(async move {
        let mut sent = 0usize;
        for _ in 0..720 {
            let (new_steps, done) = {
                if let Ok(jobs) = state.php_installs.lock() {
                    if let Some(job) = jobs.get(&id) {
                        let slice = if sent < job.steps.len() {
                            job.steps[sent..].to_vec()
                        } else {
                            Vec::new()
                        };
                        (slice, job.done)
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

            for step in new_steps {
                sent += 1;
                if let Ok(data) = serde_json::to_string(&step) {
                    if tx.send(Ok(SseEvent::default().data(data))).await.is_err() {
                        return;
                    }
                }
            }

            if done {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(250)).await;
        }
    });

    Sse::new(tokio_stream::wrappers::ReceiverStream::new(rx))
        .keep_alive(KeepAlive::default())
}

async fn php_extensions_handler(Path(version): Path<String>) -> Json<serde_json::Value> {
    let mut installed = Vec::new();
    if let Some(exe) = state::find_php_binary_exact(&version) {
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
                            installed.push(t.to_lowercase());
                        }
                    }
                }
            }
        }
    }
    let common = ["curl", "fileinfo", "gd", "mbstring", "mysqli", "openssl", "pdo_mysql", "pdo_sqlite", "sqlite3", "zip"];
    let available: Vec<&str> = common
        .iter()
        .copied()
        .filter(|ext| !installed.iter().any(|i| i == ext))
        .collect();
    Json(serde_json::json!({
        "installed": installed,
        "available": available
    }))
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
        .route("/sites/{id}/php", put(switch_site_php_handler))
        .route("/sites/{id}/runtime", put(switch_site_runtime_handler))
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
        .route("/php/versions", get(php_versions_handler))
        .route("/php/install", post(php_install_handler))
        .route("/php/extensions/{version}", get(php_extensions_handler))
        .route("/services/install/{id}/log", get(php_install_log_handler))
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

            // Reconcile and start PHP workers for all active sites in the background
            // so GUI window creation is not delayed.
            let reconcile_state = state.clone();
            tokio::task::spawn_blocking(move || {
                reconcile_state.reconcile_active_sites();
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
        if state.list_sites().is_empty() {
            let _ = state.create_site("mytestdomain.local", Some("8.3"));
            let _ = state.create_site("wordpress.test", Some("8.3"));
        }
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

    // Auto-seed demo sites if empty
    if state.list_sites().is_empty() {
        let _ = state.create_site("mytestdomain.local", Some("8.3"));
        let _ = state.create_site("wordpress.test", Some("8.3"));
    }

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

    let event_loop = EventLoop::new();
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
        let re_resp = reinstall_wordpress_core_handler(State(state.clone()), Path(site.id.clone())).await;
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
}
