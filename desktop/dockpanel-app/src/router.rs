use base64::Engine;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use axum::http::StatusCode;
use axum::response::Response;
use crate::state::AppState;

#[derive(Clone, Copy, Default)]
pub struct IsHttps(pub bool);

fn default_port_80() -> u16 {
    80
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PortStatus {
    pub port: u16,
    pub in_use: bool,
    pub process_name: Option<String>,
    pub pid: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RouterConfig {
    #[serde(default = "default_port_80")]
    pub preferred_port: u16,
    #[serde(default = "default_port_80")]
    pub active_port: u16,
}

impl Default for RouterConfig {
    fn default() -> Self {
        Self {
            preferred_port: default_port_80(),
            active_port: default_active_port(),
        }
    }
}

fn default_active_port() -> u16 {
    80
}

/// Checks if a port is bindable on 127.0.0.1 and 0.0.0.0.
pub fn is_port_bindable(port: u16) -> bool {
    // Probe 127.0.0.1 first
    let l1 = match std::net::TcpListener::bind(("127.0.0.1", port)) {
        Ok(l) => l,
        Err(_) => return false,
    };
    drop(l1);

    // Probe 0.0.0.0 as well (e.g. Apache binding wildcard)
    match std::net::TcpListener::bind(("0.0.0.0", port)) {
        Ok(l2) => {
            drop(l2);
            true
        }
        Err(_) => false,
    }
}

/// Parses netstat -ano output to find the PID listening or bound to the target port.
pub fn parse_netstat_output(output: &str, target_port: u16) -> Option<u32> {
    let mut listening_pid = None;
    let mut other_pid = None;

    let port_str = target_port.to_string();

    for line in output.lines() {
        let line = line.trim();
        if !line.starts_with("TCP") && !line.starts_with("tcp") {
            continue;
        }

        let parts: Vec<&str> = line.split_whitespace().collect();
        // Standard TCP row: Proto, Local Address, Foreign Address, State, PID
        if parts.len() < 5 {
            continue;
        }

        let local_addr = parts[1];
        // Match address ending in :<target_port>
        let has_matching_port = local_addr
            .rsplit(':')
            .next()
            .map(|p| p == port_str)
            .unwrap_or(false);

        if !has_matching_port {
            continue;
        }

        let state = parts[3];
        if let Ok(pid) = parts[parts.len() - 1].parse::<u32>() {
            if state.eq_ignore_ascii_case("LISTENING") {
                listening_pid = Some(pid);
                break;
            } else if other_pid.is_none() {
                other_pid = Some(pid);
            }
        }
    }

    listening_pid.or(other_pid)
}

/// Parses tasklist CSV output to extract the image/executable name.
pub fn parse_tasklist_output(output: &str) -> Option<String> {
    for line in output.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('"') {
            let parts: Vec<&str> = trimmed.split("\",\"").collect();
            if !parts.is_empty() {
                let name = parts[0].trim_matches('"').trim();
                if !name.is_empty() && !name.starts_with("INFO:") {
                    return Some(name.to_string());
                }
            }
        }
    }
    None
}

/// Retrieves the process executable name for a given PID.
pub fn get_process_name_by_pid(pid: u32) -> Option<String> {
    if pid == 0 {
        return Some("System Idle Process".to_string());
    }
    if pid == 4 {
        return Some("System".to_string());
    }

    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x08000000;

        let output = std::process::Command::new("tasklist")
            .args(["/FI", &format!("PID eq {}", pid), "/FO", "CSV", "/NH"])
            .creation_flags(CREATE_NO_WINDOW)
            .output()
            .ok()?;

        if output.status.success() {
            let text = String::from_utf8_lossy(&output.stdout);
            return parse_tasklist_output(&text);
        }
    }

    None
}

/// Inspects the system to find the PID and process name holding `port`.
pub fn find_process_for_port(port: u16) -> (Option<u32>, Option<String>) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x08000000;

        if let Ok(output) = std::process::Command::new("netstat")
            .args(["-ano", "-p", "tcp"])
            .creation_flags(CREATE_NO_WINDOW)
            .output()
        {
            if output.status.success() {
                let text = String::from_utf8_lossy(&output.stdout);
                if let Some(pid) = parse_netstat_output(&text, port) {
                    let name = get_process_name_by_pid(pid);
                    return (Some(pid), name);
                }
            }
        }
    }

    (None, None)
}

/// Checks whether a TCP port is in use and identifies conflicting process details.
pub fn check_port_status(port: u16) -> PortStatus {
    let in_use = !is_port_bindable(port);
    if in_use {
        let (pid, process_name) = find_process_for_port(port);
        PortStatus {
            port,
            in_use: true,
            process_name,
            pid,
        }
    } else {
        PortStatus {
            port,
            in_use: false,
            process_name: None,
            pid: None,
        }
    }
}

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#x27;")
}

fn sanitize_path(raw_path: &str) -> PathBuf {
    let mut safe = PathBuf::new();
    let trimmed = raw_path.trim_start_matches('/');
    for comp in Path::new(trimmed).components() {
        if let std::path::Component::Normal(c) = comp {
            safe.push(c);
        }
    }
    safe
}

fn render_not_found_page(domain: &str) -> Response {
    let html = format!(
        r#"<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <title>Domain Not Registered - DockPanel</title>
    <style>
        body {{ font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, sans-serif; background: #0f172a; color: #f8fafc; display: flex; align-items: center; justify-content: center; height: 100vh; margin: 0; }}
        .card {{ background: #1e293b; padding: 2.5rem; border-radius: 1rem; border: 1px solid #334155; text-align: center; max-width: 500px; box-shadow: 0 10px 25px rgba(0,0,0,0.5); }}
        h1 {{ color: #f87171; margin-top: 0; font-size: 1.5rem; }}
        p {{ color: #94a3b8; font-size: 1rem; line-height: 1.6; }}
        .domain {{ color: #e2e8f0; font-weight: 600; font-family: monospace; background: #0f172a; padding: 0.2rem 0.5rem; border-radius: 0.375rem; }}
        .badge {{ background: #dc2626; color: white; padding: 0.35rem 0.75rem; border-radius: 9999px; font-size: 0.875rem; font-weight: 600; display: inline-block; margin-top: 1rem; }}
    </style>
</head>
<body>
    <div class="card">
        <h1>Domain not registered in DockPanel</h1>
        <p>The domain <span class="domain">{domain}</span> is not mapped to any site in DockPanel Desktop.</p>
        <span class="badge">404 Not Found</span>
    </div>
</body>
</html>"#,
        domain = html_escape(domain)
    );
    Response::builder()
        .status(StatusCode::NOT_FOUND)
        .header(axum::http::header::CONTENT_TYPE, "text/html; charset=utf-8")
        .body(axum::body::Body::from(html))
        .unwrap()
}

fn render_stopped_page(domain: &str) -> Response {
    let html = format!(
        r#"<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <title>{domain} - Site is Stopped</title>
    <style>
        body {{ font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, sans-serif; background: #0f172a; color: #f8fafc; display: flex; align-items: center; justify-content: center; height: 100vh; margin: 0; }}
        .card {{ background: #1e293b; padding: 2.5rem; border-radius: 1rem; border: 1px solid #334155; text-align: center; max-width: 500px; box-shadow: 0 10px 25px rgba(0,0,0,0.5); }}
        h1 {{ color: #f59e0b; margin-top: 0; font-size: 1.8rem; word-break: break-all; }}
        h2 {{ color: #fbbf24; font-size: 1.25rem; margin-top: 0.5rem; }}
        p {{ color: #94a3b8; font-size: 1rem; line-height: 1.6; margin-top: 1rem; }}
        .badge {{ background: #d97706; color: white; padding: 0.35rem 0.75rem; border-radius: 9999px; font-size: 0.875rem; font-weight: 600; display: inline-block; margin-top: 1rem; }}
    </style>
</head>
<body>
    <div class="card">
        <h1>{domain}</h1>
        <h2>Site is Stopped</h2>
        <p>This site is currently stopped in DockPanel Desktop. Click <strong>Start Site</strong> in DockPanel to activate.</p>
        <span class="badge">Stopped</span>
    </div>
</body>
</html>"#,
        domain = html_escape(domain)
    );
    Response::builder()
        .status(StatusCode::OK)
        .header(axum::http::header::CONTENT_TYPE, "text/html; charset=utf-8")
        .body(axum::body::Body::from(html))
        .unwrap()
}

fn render_error_page(status: StatusCode, title: &str, message: &str) -> Response {
    let html = format!(
        r#"<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <title>{title} - DockPanel</title>
    <style>
        body {{ font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, sans-serif; background: #0f172a; color: #f8fafc; display: flex; align-items: center; justify-content: center; height: 100vh; margin: 0; }}
        .card {{ background: #1e293b; padding: 2.5rem; border-radius: 1rem; border: 1px solid #334155; text-align: center; max-width: 500px; box-shadow: 0 10px 25px rgba(0,0,0,0.5); }}
        h1 {{ color: #f87171; margin-top: 0; font-size: 1.5rem; }}
        p {{ color: #94a3b8; font-size: 1rem; line-height: 1.6; }}
    </style>
</head>
<body>
    <div class="card">
        <h1>{title}</h1>
        <p>{message}</p>
    </div>
</body>
</html>"#,
        title = html_escape(title),
        message = html_escape(message)
    );
    Response::builder()
        .status(status)
        .header(axum::http::header::CONTENT_TYPE, "text/html; charset=utf-8")
        .body(axum::body::Body::from(html))
        .unwrap()
}

fn is_hop_by_hop_or_framing_header(name: &str) -> bool {
    matches!(
        name.to_ascii_lowercase().as_str(),
        "connection"
            | "content-length"
            | "transfer-encoding"
            | "keep-alive"
            | "te"
            | "trailer"
            | "upgrade"
    )
}

async fn proxy_to_worker(
    worker_port: u16,
    target_path_and_query: &str,
    req: axum::extract::Request,
) -> Response {
    static CLIENT: std::sync::LazyLock<reqwest::Client> = std::sync::LazyLock::new(|| {
        reqwest::Client::builder()
            .connect_timeout(std::time::Duration::from_secs(5))
            .timeout(std::time::Duration::from_secs(30))
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .unwrap_or_default()
    });

    let proto = req
        .extensions()
        .get::<IsHttps>()
        .map(|h| h.0)
        .unwrap_or(false);
    let (parts, body) = req.into_parts();
    let url = format!("http://127.0.0.1:{}{}", worker_port, target_path_and_query);

    let method = match reqwest::Method::from_bytes(parts.method.as_str().as_bytes()) {
        Ok(m) => m,
        Err(_) => {
            return Response::builder()
                .status(StatusCode::BAD_REQUEST)
                .body(axum::body::Body::from("Invalid HTTP method"))
                .unwrap();
        }
    };

    let mut builder = CLIENT.request(method, &url);

    let original_host = parts
        .headers
        .get(axum::http::header::HOST)
        .and_then(|h| h.to_str().ok())
        .map(|s| s.to_string());

    for (name, val) in parts.headers.iter() {
        if name != axum::http::header::HOST && !is_hop_by_hop_or_framing_header(name.as_str()) {
            if let (Ok(hname), Ok(hval)) = (
                reqwest::header::HeaderName::from_bytes(name.as_str().as_bytes()),
                reqwest::header::HeaderValue::from_bytes(val.as_bytes()),
            ) {
                builder = builder.header(hname, hval);
            }
        }
    }

    if let Some(host_str) = &original_host {
        if let Ok(hval) = reqwest::header::HeaderValue::from_str(host_str) {
            builder = builder.header(reqwest::header::HOST, hval.clone());
            builder = builder.header("x-forwarded-host", hval);
        }
    }
    builder = builder.header("x-forwarded-proto", if proto { "https" } else { "http" });
    builder = builder.header("x-forwarded-for", "127.0.0.1");

    let body_bytes = match axum::body::to_bytes(body, 64 * 1024 * 1024).await {
        Ok(b) => b,
        Err(e) => {
            return Response::builder()
                .status(StatusCode::BAD_REQUEST)
                .body(axum::body::Body::from(format!("Failed to read request body: {}", e)))
                .unwrap();
        }
    };

    if !body_bytes.is_empty() {
        builder = builder.body(body_bytes);
    }

    match builder.send().await {
        Ok(resp) => {
            let status = StatusCode::from_u16(resp.status().as_u16())
                .unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);

            let mut res_builder = Response::builder().status(status);

            for (k, v) in resp.headers().iter() {
                if k != reqwest::header::TRANSFER_ENCODING && k != reqwest::header::CONNECTION {
                    if let (Ok(hk), Ok(hv)) = (
                        axum::http::HeaderName::from_bytes(k.as_str().as_bytes()),
                        axum::http::HeaderValue::from_bytes(v.as_bytes()),
                    ) {
                        res_builder = res_builder.header(hk, hv);
                    }
                }
            }

            match resp.bytes().await {
                Ok(bytes) => res_builder.body(axum::body::Body::from(bytes)).unwrap(),
                Err(e) => Response::builder()
                    .status(StatusCode::BAD_GATEWAY)
                    .body(axum::body::Body::from(format!("Error reading worker response: {}", e)))
                    .unwrap(),
            }
        }
        Err(e) => Response::builder()
            .status(StatusCode::BAD_GATEWAY)
            .header(axum::http::header::CONTENT_TYPE, "text/html; charset=utf-8")
            .body(axum::body::Body::from(format!(
                "<!DOCTYPE html><html><body><h1>502 Bad Gateway</h1><p>Error connecting to PHP worker: {}</p></body></html>",
                html_escape(&e.to_string())
            )))
            .unwrap(),
    }
}

pub async fn handle_router_request(req: axum::extract::Request, state: Arc<AppState>) -> Response {
    let req_started = std::time::Instant::now();
    let method_str = req.method().as_str().to_string();
    let is_https = req.extensions().get::<IsHttps>().map(|h| h.0).unwrap_or(false);
    let path_and_query_for_log = req
        .uri()
        .path_and_query()
        .map(|p| p.as_str().to_string())
        .unwrap_or_else(|| req.uri().path().to_string());

    // 1. Extract Host header. Strip port if present.
    let host_header = match req.headers().get(axum::http::header::HOST).and_then(|h| h.to_str().ok()) {
        Some(h) if !h.trim().is_empty() => h.trim(),
        _ => {
            return Response::builder()
                .status(StatusCode::BAD_REQUEST)
                .header(axum::http::header::CONTENT_TYPE, "text/plain; charset=utf-8")
                .body(axum::body::Body::from("Bad Request: Missing Host header"))
                .unwrap();
        }
    };

    let domain = host_header.split(':').next().unwrap_or(host_header).trim();

    // 2. Lookup site in AppState::get_site
    let site = match state.get_site(domain) {
        Some(s) => s,
        None => {
            // 3. Domain not registered in DockPanel
            return render_not_found_page(domain);
        }
    };

    let resp = route_matched_site_request(req, &state, &site).await;
    let elapsed_ms = req_started.elapsed().as_millis();
    let status_code = resp.status().as_u16();
    let scheme = if is_https { "HTTPS" } else { "HTTP" };
    let location_note = resp
        .headers()
        .get(axum::http::header::LOCATION)
        .and_then(|v| v.to_str().ok())
        .map(|loc| format!(" (Location: {})", loc))
        .unwrap_or_default();
    state.append_site_access_log(
        &site.domain,
        &format!(
            "{} {} {} -> {}{} [{}ms]",
            scheme, method_str, path_and_query_for_log, status_code, location_note, elapsed_ms
        ),
    );
    resp
}

async fn route_matched_site_request(
    req: axum::extract::Request,
    state: &Arc<AppState>,
    site: &crate::state::LocalSite,
) -> Response {
    // 4. If site status is "stopped"
    if site.status.eq_ignore_ascii_case("stopped") {
        return render_stopped_page(&site.domain);
    }

    // 4b. Enforce HTTP Basic Password Protection when enabled for this site
    if let Some((expected_user, expected_hash)) = state.get_site_pass_protect(&site.domain) {
        let authorized = req
            .headers()
            .get(axum::http::header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .and_then(|h| h.strip_prefix("Basic ").or_else(|| h.strip_prefix("basic ")))
            .and_then(|b64| base64::engine::general_purpose::STANDARD.decode(b64.trim()).ok())
            .and_then(|raw| String::from_utf8(raw).ok())
            .and_then(|pair| {
                let (u, p) = pair.split_once(':')?;
                Some(u == expected_user && crate::downloads::sha256_hex(p) == expected_hash)
            })
            .unwrap_or(false);

        if !authorized {
            return Response::builder()
                .status(StatusCode::UNAUTHORIZED)
                .header(
                    axum::http::header::WWW_AUTHENTICATE,
                    "Basic realm=\"DockPanel Protected Site\"",
                )
                .header(axum::http::header::CONTENT_TYPE, "text/html; charset=utf-8")
                .body(axum::body::Body::from(
                    "<!DOCTYPE html><html><body><h1>401 Unauthorized</h1><p>Authentication required.</p></body></html>",
                ))
                .unwrap();
        }
    }

    // 5. Site status is active (or other non-stopped)
    let uri_path = req.uri().path().to_string();
    let path_and_query_str = req
        .uri()
        .path_and_query()
        .map(|p| p.as_str().to_string())
        .unwrap_or_else(|| uri_path.clone());
    let safe_rel = sanitize_path(&uri_path);
    let doc_root = state.site_dir(&site);
    let target_path = doc_root.join(&safe_rel);

    let worker_port = state.get_site_worker_port(&site.domain);

    // Proxying runtimes (Node.js / Reverse Proxy) forward all requests to their local process/target
    if site.runtime == "node" || site.runtime == "proxy" {
        if let Some(port) = worker_port {
            return proxy_to_worker(port, &path_and_query_str, req).await;
        } else {
            return render_error_page(
                StatusCode::BAD_GATEWAY,
                "Application Worker Not Running",
                &format!(
                    "Application process is not running for site '{}'. Start the site in DockPanel.",
                    site.domain
                ),
            );
        }
    }

    // If path points to an existing file in site.document_root
    if target_path.is_file() {
        let is_php = target_path
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| e.eq_ignore_ascii_case("php"))
            .unwrap_or(false);

        if is_php {
            if let Some(port) = worker_port {
                return proxy_to_worker(port, &path_and_query_str, req).await;
            } else {
                return render_error_page(
                    StatusCode::BAD_GATEWAY,
                    "PHP Worker Not Running",
                    &format!("PHP worker is not running for site '{}'. Start the site in DockPanel.", site.domain),
                );
            }
        } else {
            // Serve static file with appropriate Content-Type header
            let mime = mime_guess::from_path(&target_path).first_or_octet_stream();
            let content_type = if mime.as_ref() == "text/html" {
                "text/html; charset=utf-8"
            } else {
                mime.as_ref()
            };

            return match std::fs::read(&target_path) {
                Ok(content) => Response::builder()
                    .status(StatusCode::OK)
                    .header(axum::http::header::CONTENT_TYPE, content_type)
                    .body(axum::body::Body::from(content))
                    .unwrap(),
                Err(_) => Response::builder()
                    .status(StatusCode::NOT_FOUND)
                    .body(axum::body::Body::from("File not found"))
                    .unwrap(),
            };
        }
    }

    // If path is a directory (or "/")
    if target_path.is_dir() || uri_path == "/" {
        let index_php = target_path.join("index.php");
        let index_html = target_path.join("index.html");

        if index_php.is_file() {
            if let Some(port) = worker_port {
                // Preserve the original URI ("/" or "/wp-admin/") instead of rewriting to
                // "/index.php", otherwise WordPress canonical redirects emit an infinite 301 loop.
                return proxy_to_worker(port, &path_and_query_str, req).await;
            } else {
                return render_error_page(
                    StatusCode::BAD_GATEWAY,
                    "PHP Worker Not Running",
                    &format!("PHP worker is not running for site '{}'. Start the site in DockPanel.", site.domain),
                );
            }
        } else if index_html.is_file() {
            return match std::fs::read(&index_html) {
                Ok(content) => Response::builder()
                    .status(StatusCode::OK)
                    .header(axum::http::header::CONTENT_TYPE, "text/html; charset=utf-8")
                    .body(axum::body::Body::from(content))
                    .unwrap(),
                Err(_) => Response::builder()
                    .status(StatusCode::NOT_FOUND)
                    .body(axum::body::Body::from("File not found"))
                    .unwrap(),
            };
        } else if let Some(port) = worker_port {
            return proxy_to_worker(port, &path_and_query_str, req).await;
        } else {
            return Response::builder()
                .status(StatusCode::NOT_FOUND)
                .header(axum::http::header::CONTENT_TYPE, "text/html; charset=utf-8")
                .body(axum::body::Body::from("<!DOCTYPE html><html><body><h1>404 Not Found</h1><p>No index.php or index.html found in directory.</p></body></html>"))
                .unwrap();
        }
    }

    // Path does not exist as file or directory on disk
    if let Some(port) = worker_port {
        proxy_to_worker(port, &path_and_query_str, req).await
    } else {
        Response::builder()
            .status(StatusCode::NOT_FOUND)
            .header(axum::http::header::CONTENT_TYPE, "text/html; charset=utf-8")
            .body(axum::body::Body::from("<!DOCTYPE html><html><body><h1>404 Not Found</h1><p>The requested file does not exist.</p></body></html>"))
            .unwrap()
    }
}

pub fn start_site_router_checked(
    state: Arc<AppState>,
    port: u16,
) -> (tokio::task::JoinHandle<()>, tokio::sync::oneshot::Receiver<bool>) {
    let (tx, rx) = tokio::sync::oneshot::channel();
    let handle = tokio::spawn(async move {
        let state_clone = state.clone();
        let app = axum::Router::new().fallback(move |req: axum::extract::Request| {
            let s = state_clone.clone();
            async move { handle_router_request(req, s).await }
        });

        let bind_result = match tokio::net::TcpListener::bind(("127.0.0.1", port)).await {
            Ok(probe) => {
                drop(probe);
                match tokio::net::TcpListener::bind(("0.0.0.0", port)).await {
                    Ok(l) => Ok(l),
                    Err(_) => tokio::net::TcpListener::bind(("127.0.0.1", port)).await,
                }
            }
            Err(e) => Err(e),
        };

        match bind_result {
            Ok(listener) => {
                let _ = tx.send(true);
                let _ = axum::serve(listener, app).await;
            }
            Err(e) => {
                eprintln!("Failed to bind site router on port {}: {}", port, e);
                let _ = tx.send(false);
            }
        }
    });
    (handle, rx)
}

pub fn start_site_router(state: Arc<AppState>, port: u16) -> tokio::task::JoinHandle<()> {
    let (handle, _rx) = start_site_router_checked(state, port);
    handle
}

pub fn start_ssl_site_router_checked(
    state: Arc<AppState>,
    port: u16,
) -> (tokio::task::JoinHandle<()>, tokio::sync::oneshot::Receiver<bool>) {
    let (tx, rx) = tokio::sync::oneshot::channel();
    let handle = tokio::spawn(async move {
        let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
        let mut tls_cfg = rustls::ServerConfig::builder()
            .with_no_client_auth()
            .with_cert_resolver(state.cert_resolver.clone());
        tls_cfg.alpn_protocols = vec![b"http/1.1".to_vec()];
        let acceptor = tokio_rustls::TlsAcceptor::from(Arc::new(tls_cfg));

        let bind_result = match tokio::net::TcpListener::bind(("127.0.0.1", port)).await {
            Ok(probe) => {
                drop(probe);
                match tokio::net::TcpListener::bind(("0.0.0.0", port)).await {
                    Ok(l) => Ok(l),
                    Err(_) => tokio::net::TcpListener::bind(("127.0.0.1", port)).await,
                }
            }
            Err(e) => Err(e),
        };

        let listener = match bind_result {
            Ok(l) => {
                let _ = tx.send(true);
                l
            }
            Err(e) => {
                eprintln!("Failed to bind HTTPS site router on port {}: {}", port, e);
                let _ = tx.send(false);
                return;
            }
        };

        let state_clone = state.clone();
        let app = axum::Router::new()
            .fallback(move |req: axum::extract::Request| {
                let s = state_clone.clone();
                async move { handle_router_request(req, s).await }
            })
            .layer(axum::Extension(IsHttps(true)));

        loop {
            let Ok((stream, _peer)) = listener.accept().await else {
                continue;
            };
            let acc = acceptor.clone();
            let svc = app.clone();
            tokio::spawn(async move {
                let Ok(tls_stream) = acc.accept(stream).await else {
                    return;
                };
                let io = hyper_util::rt::TokioIo::new(tls_stream);
                let hyper_svc = hyper_util::service::TowerToHyperService::new(svc);
                let _ = hyper_util::server::conn::auto::Builder::new(
                    hyper_util::rt::TokioExecutor::new(),
                )
                .serve_connection(io, hyper_svc)
                .await;
            });
        }
    });
    (handle, rx)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_port_checker_available_port() {
        // Binding to port 0 checks random free port
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);
        let status = check_port_status(port);
        assert!(!status.in_use);
        assert_eq!(status.port, port);
        assert!(status.pid.is_none());
        assert!(status.process_name.is_none());
    }

    #[test]
    fn test_port_checker_occupied_port() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let status = check_port_status(port);
        assert!(status.in_use);
        assert_eq!(status.port, port);
        drop(listener);
    }

    #[test]
    fn test_check_port_status_occupied_apache_80() {
        let status = check_port_status(80);
        if status.in_use {
            assert_eq!(status.port, 80);
            assert!(status.pid.is_some());
            if let Some(ref name) = status.process_name {
                assert!(!name.trim().is_empty(), "Expected resolved process name for port 80 listener");
            }
        }
    }

    #[test]
    fn test_parse_netstat_output() {
        let mock = r#"
Active Connections

  Proto  Local Address          Foreign Address        State           PID
  TCP    0.0.0.0:80             0.0.0.0:0              LISTENING       20408
  TCP    0.0.0.0:135            0.0.0.0:0              LISTENING       1184
  TCP    127.0.0.1:8080         0.0.0.0:0              LISTENING       5678
  TCP    192.168.1.10:60000     23.15.142.75:80        ESTABLISHED     3716
  TCP    [::]:80                [::]:0                 LISTENING       20408
"#;
        assert_eq!(parse_netstat_output(mock, 80), Some(20408));
        assert_eq!(parse_netstat_output(mock, 8080), Some(5678));
        assert_eq!(parse_netstat_output(mock, 135), Some(1184));
        assert_eq!(parse_netstat_output(mock, 9999), None);
        assert_eq!(parse_netstat_output(mock, 60000), Some(3716));
    }

    #[test]
    fn test_parse_tasklist_output() {
        let mock = "\"httpd.exe\",\"20408\",\"Console\",\"1\",\"21,312 K\"\r\n";
        assert_eq!(parse_tasklist_output(mock), Some("httpd.exe".to_string()));

        let empty = "INFO: No tasks are running which match the specified criteria.\r\n";
        assert_eq!(parse_tasklist_output(empty), None);
    }

    #[test]
    fn test_router_config_serialization() {
        let config = RouterConfig {
            preferred_port: 80,
            active_port: 8080,
        };
        let json = serde_json::to_string(&config).unwrap();
        let deserialized: RouterConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(config, deserialized);

        // Test defaults on empty JSON
        let default_deser: RouterConfig = serde_json::from_str("{}").unwrap();
        assert_eq!(default_deser.preferred_port, 80);
        assert_eq!(default_deser.active_port, 80);
    }

    #[tokio::test]
    async fn test_stopped_site_returns_holding_page() {
        use std::sync::Arc;
        let temp_dir = std::env::temp_dir().join(format!("dockpanel_router_test_{}", uuid::Uuid::new_v4()));
        let config_dir = temp_dir.join("config");
        let sites_dir = temp_dir.join("sites");
        let state = Arc::new(crate::state::AppState::new_with_dirs(config_dir, sites_dir));
        let _site = state.create_site("test-stopped.test", Some("8.3")).unwrap();
        // Mark site as stopped
        {
            let mut data = state.data.lock().unwrap();
            if let Some(s) = data.sites.iter_mut().find(|s| s.domain == "test-stopped.test") {
                s.status = "stopped".to_string();
            }
        }

        let req = axum::http::Request::builder()
            .header("host", "test-stopped.test:8080")
            .uri("/")
            .body(axum::body::Body::empty())
            .unwrap();

        let resp = handle_router_request(req, state).await;
        assert_eq!(resp.status(), axum::http::StatusCode::OK);
        let body_bytes = axum::body::to_bytes(resp.into_body(), 1024 * 64).await.unwrap();
        let body_str = String::from_utf8_lossy(&body_bytes);
        assert!(body_str.contains("Site is Stopped"));
        assert!(body_str.contains("DockPanel Desktop"));

        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[tokio::test]
    async fn test_domain_not_registered_returns_404() {
        use std::sync::Arc;
        let temp_dir = std::env::temp_dir().join(format!("dockpanel_router_404_{}", uuid::Uuid::new_v4()));
        let config_dir = temp_dir.join("config");
        let sites_dir = temp_dir.join("sites");
        let state = Arc::new(crate::state::AppState::new_with_dirs(config_dir, sites_dir));

        let req = axum::http::Request::builder()
            .header("host", "unregistered-domain.test")
            .uri("/")
            .body(axum::body::Body::empty())
            .unwrap();

        let resp = handle_router_request(req, state).await;
        assert_eq!(resp.status(), axum::http::StatusCode::NOT_FOUND);
        let body_bytes = axum::body::to_bytes(resp.into_body(), 1024 * 64).await.unwrap();
        let body_str = String::from_utf8_lossy(&body_bytes);
        assert!(body_str.contains("Domain not registered in DockPanel"));

        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[tokio::test]
    async fn test_missing_host_header_returns_400() {
        use std::sync::Arc;
        let temp_dir = std::env::temp_dir().join(format!("dockpanel_router_400_{}", uuid::Uuid::new_v4()));
        let config_dir = temp_dir.join("config");
        let sites_dir = temp_dir.join("sites");
        let state = Arc::new(crate::state::AppState::new_with_dirs(config_dir, sites_dir));

        let req = axum::http::Request::builder()
            .uri("/")
            .body(axum::body::Body::empty())
            .unwrap();

        let resp = handle_router_request(req, state).await;
        assert_eq!(resp.status(), axum::http::StatusCode::BAD_REQUEST);

        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[tokio::test]
    async fn test_active_site_serves_static_file() {
        use std::sync::Arc;
        let temp_dir = std::env::temp_dir().join(format!("dockpanel_router_static_{}", uuid::Uuid::new_v4()));
        let config_dir = temp_dir.join("config");
        let sites_dir = temp_dir.join("sites");
        let state = Arc::new(crate::state::AppState::new_with_dirs(config_dir, sites_dir));
        let site = state.create_site("my-static-site.test", Some("8.3")).unwrap();

        // Add a custom css file
        let css_path = Path::new(&site.document_root).join("style.css");
        std::fs::write(&css_path, "body { background: purple; }").unwrap();

        // 1. Test index.html on "/"
        let req1 = axum::http::Request::builder()
            .header("host", "my-static-site.test:80")
            .uri("/")
            .body(axum::body::Body::empty())
            .unwrap();
        let resp1 = handle_router_request(req1, state.clone()).await;
        assert_eq!(resp1.status(), axum::http::StatusCode::OK);
        assert_eq!(resp1.headers().get("content-type").unwrap(), "text/html; charset=utf-8");
        let b1 = axum::body::to_bytes(resp1.into_body(), 1024 * 64).await.unwrap();
        assert!(String::from_utf8_lossy(&b1).contains("my-static-site.test"));

        // 2. Test style.css on "/style.css"
        let req2 = axum::http::Request::builder()
            .header("host", "my-static-site.test")
            .uri("/style.css")
            .body(axum::body::Body::empty())
            .unwrap();
        let resp2 = handle_router_request(req2, state).await;
        assert_eq!(resp2.status(), axum::http::StatusCode::OK);
        assert_eq!(resp2.headers().get("content-type").unwrap(), "text/css");
        let b2 = axum::body::to_bytes(resp2.into_body(), 1024 * 64).await.unwrap();
        assert_eq!(String::from_utf8_lossy(&b2), "body { background: purple; }");

        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[tokio::test]
    async fn test_active_site_proxies_to_php_worker() {
        use std::sync::Arc;
        let temp_dir = std::env::temp_dir().join(format!("dockpanel_router_php_{}", uuid::Uuid::new_v4()));
        let config_dir = temp_dir.join("config");
        let sites_dir = temp_dir.join("sites");
        let state = Arc::new(crate::state::AppState::new_with_dirs(config_dir, sites_dir));
        let site = state.create_site("php-test.test", Some("8.3")).unwrap();

        // Create index.php in document root
        let php_file = Path::new(&site.document_root).join("index.php");
        std::fs::write(&php_file, "<?php echo 'Mock PHP'; ?>").unwrap();

        // Spawn mock PHP worker
        let mock_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let mock_port = mock_listener.local_addr().unwrap().port();

        let mock_server = tokio::spawn(async move {
            let mock_app = axum::Router::new().fallback(|req: axum::extract::Request| async move {
                let path = req.uri().to_string();
                let has_keep_alive = req.headers().contains_key("keep-alive");
                let has_te = req.headers().contains_key("te");
                let has_upgrade = req.headers().contains_key("upgrade");
                let has_trailer = req.headers().contains_key("trailer");
                let has_custom = req.headers().get("x-custom-test").and_then(|v| v.to_str().ok()) == Some("ok");
                axum::response::Response::builder()
                    .status(axum::http::StatusCode::OK)
                    .header("x-mock-php", "true")
                    .header("x-stripped-hop", format!("{},{},{},{}", has_keep_alive, has_te, has_upgrade, has_trailer))
                    .header("x-custom-forwarded", has_custom.to_string())
                    .body(axum::body::Body::from(format!("Hello from PHP Worker at {}", path)))
                    .unwrap()
            });
            let _ = axum::serve(mock_listener, mock_app).await;
        });

        // Set worker port in state
        state.set_site_worker_port("php-test.test", mock_port);

        // Send request to "/" with hop-by-hop headers that should be stripped
        let req = axum::http::Request::builder()
            .header("host", "php-test.test")
            .header("keep-alive", "timeout=5")
            .header("te", "trailers")
            .header("upgrade", "websocket")
            .header("trailer", "X-Trailer")
            .header("x-custom-test", "ok")
            .uri("/?page=about")
            .body(axum::body::Body::empty())
            .unwrap();

        let resp = handle_router_request(req, state.clone()).await;
        assert_eq!(resp.status(), axum::http::StatusCode::OK);
        assert_eq!(resp.headers().get("x-mock-php").unwrap(), "true");
        assert_eq!(resp.headers().get("x-stripped-hop").unwrap(), "false,false,false,false");
        assert_eq!(resp.headers().get("x-custom-forwarded").unwrap(), "true");
        let b = axum::body::to_bytes(resp.into_body(), 1024 * 64).await.unwrap();
        let body_str = String::from_utf8_lossy(&b);
        assert!(body_str.contains("Hello from PHP Worker at /?page=about"));

        mock_server.abort();
        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[tokio::test]
    async fn test_active_site_worker_not_running_returns_502() {
        use std::sync::Arc;
        let temp_dir = std::env::temp_dir().join(format!("dockpanel_router_502_{}", uuid::Uuid::new_v4()));
        let config_dir = temp_dir.join("config");
        let sites_dir = temp_dir.join("sites");
        let state = Arc::new(crate::state::AppState::new_with_dirs(config_dir, sites_dir));
        let site = state.create_site("no-worker.test", Some("8.3")).unwrap();

        // Create index.php
        let php_file = Path::new(&site.document_root).join("index.php");
        std::fs::write(&php_file, "<?php echo 'No worker'; ?>").unwrap();

        // Do NOT set worker port

        let req = axum::http::Request::builder()
            .header("host", "no-worker.test")
            .uri("/")
            .body(axum::body::Body::empty())
            .unwrap();

        let resp = handle_router_request(req, state).await;
        assert_eq!(resp.status(), axum::http::StatusCode::BAD_GATEWAY);
        let b = axum::body::to_bytes(resp.into_body(), 1024 * 64).await.unwrap();
        assert!(String::from_utf8_lossy(&b).contains("PHP Worker Not Running"));

        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[tokio::test]
    async fn test_start_site_router_e2e() {
        use std::sync::Arc;
        let temp_dir = std::env::temp_dir().join(format!("dockpanel_router_e2e_{}", uuid::Uuid::new_v4()));
        let config_dir = temp_dir.join("config");
        let sites_dir = temp_dir.join("sites");
        let state = Arc::new(crate::state::AppState::new_with_dirs(config_dir, sites_dir));
        let _site = state.create_site("e2e-site.test", Some("8.3")).unwrap();

        // Find available port
        let probe = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = probe.local_addr().unwrap().port();
        drop(probe);

        let handle = start_site_router(state.clone(), port);
        // Wait briefly for server to bind
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;

        let client = reqwest::Client::new();
        let resp = client
            .get(format!("http://127.0.0.1:{}/", port))
            .header("host", "e2e-site.test")
            .send()
            .await
            .expect("Failed to send request to site router");

        assert_eq!(resp.status(), 200);
        let body = resp.text().await.unwrap();
        assert!(body.contains("e2e-site.test"));
        assert!(body.contains("DockPanel Desktop"));

        handle.abort();
        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[tokio::test]
    async fn test_root_with_index_php_preserves_root_path_without_redirect_loop() {
        use axum::{http::Uri, routing::any, Router};
        use std::sync::Arc;

        let temp_dir = std::env::temp_dir().join(format!(
            "dockpanel_wp_root_path_{}",
            uuid::Uuid::new_v4()
        ));
        let config_dir = temp_dir.join("config");
        let sites_dir = temp_dir.join("sites");
        let state = Arc::new(crate::state::AppState::new_with_dirs(config_dir, sites_dir));
        let site = state.create_site("wp-root.test", Some("8.3")).unwrap();

        // WordPress sites always have index.php at document root; proxying "/" as
        // "/index.php" triggers WordPress redirect_canonical() ERR_TOO_MANY_REDIRECTS.
        let index_php = Path::new(&site.document_root).join("index.php");
        std::fs::write(&index_php, "<?php echo 'wp'; ?>").unwrap();

        let mock_app = Router::new().fallback(any(|uri: Uri| async move {
            format!("WORKER_URI={}", uri)
        }));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let worker_port = listener.local_addr().unwrap().port();
        let mock_server = tokio::spawn(async move {
            axum::serve(listener, mock_app).await.unwrap();
        });

        state.set_site_worker_port("wp-root.test", worker_port);

        for req_uri in ["/", "/?page_id=42", "/wp-admin/"] {
            let req = axum::http::Request::builder()
                .header("host", "wp-root.test")
                .uri(req_uri)
                .body(axum::body::Body::empty())
                .unwrap();

            let resp = handle_router_request(req, state.clone()).await;
            assert_eq!(resp.status(), axum::http::StatusCode::OK);
            let bytes = axum::body::to_bytes(resp.into_body(), 64 * 1024).await.unwrap();
            let body = String::from_utf8_lossy(&bytes);
            assert_eq!(
                body,
                format!("WORKER_URI={}", req_uri),
                "Router must preserve original URI {} instead of rewriting to /index.php",
                req_uri
            );
        }

        mock_server.abort();
        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[tokio::test]
    async fn test_pass_protect_enforces_http_basic_auth() {
        use base64::Engine;
        use std::sync::Arc;

        let temp_dir = std::env::temp_dir().join(format!(
            "dockpanel_basic_auth_{}",
            uuid::Uuid::new_v4()
        ));
        let config_dir = temp_dir.join("config");
        let sites_dir = temp_dir.join("sites");
        let state = Arc::new(crate::state::AppState::new_with_dirs(config_dir, sites_dir));
        let site = state.create_site("protected.test", Some("8.3")).unwrap();
        std::fs::write(Path::new(&site.document_root).join("hello.html"), "Secret OK").unwrap();

        state
            .set_site_pass_protect("protected.test", true, Some("tester"), Some("s3cr3t"))
            .unwrap();

        // Missing credentials -> 401
        let unauth_req = axum::http::Request::builder()
            .header("host", "protected.test")
            .uri("/hello.html")
            .body(axum::body::Body::empty())
            .unwrap();
        let unauth_resp = handle_router_request(unauth_req, state.clone()).await;
        assert_eq!(unauth_resp.status(), axum::http::StatusCode::UNAUTHORIZED);
        assert!(unauth_resp.headers().contains_key(axum::http::header::WWW_AUTHENTICATE));

        // Wrong credentials -> 401
        let wrong_hdr = format!(
            "Basic {}",
            base64::engine::general_purpose::STANDARD.encode("tester:wrong")
        );
        let wrong_req = axum::http::Request::builder()
            .header("host", "protected.test")
            .header(axum::http::header::AUTHORIZATION, wrong_hdr)
            .uri("/hello.html")
            .body(axum::body::Body::empty())
            .unwrap();
        let wrong_resp = handle_router_request(wrong_req, state.clone()).await;
        assert_eq!(wrong_resp.status(), axum::http::StatusCode::UNAUTHORIZED);

        // Valid credentials -> 200
        let ok_hdr = format!(
            "Basic {}",
            base64::engine::general_purpose::STANDARD.encode("tester:s3cr3t")
        );
        let ok_req = axum::http::Request::builder()
            .header("host", "protected.test")
            .header(axum::http::header::AUTHORIZATION, ok_hdr)
            .uri("/hello.html")
            .body(axum::body::Body::empty())
            .unwrap();
        let ok_resp = handle_router_request(ok_req, state.clone()).await;
        assert_eq!(ok_resp.status(), axum::http::StatusCode::OK);

        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[tokio::test]
    async fn test_node_site_proxies_all_paths_including_existing_files() {
        let temp_dir = std::env::temp_dir().join(format!("dockpanel_node_router_{}", uuid::Uuid::new_v4()));
        let config_dir = temp_dir.join("config");
        let sites_dir = temp_dir.join("sites");
        let state = Arc::new(AppState::new_with_dirs(config_dir, sites_dir.clone()));

        let site = state
            .create_site_with_options("node-proxy.test", Some("node"), None, None, Some("node server.js"), None)
            .unwrap();

        // Spawn a tiny mock HTTP server acting as the Node.js worker
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let worker_port = listener.local_addr().unwrap().port();
        let mock_app = axum::Router::new().fallback(|| async { "hello-from-node-worker" });
        tokio::spawn(async move {
            let _ = axum::serve(listener, mock_app).await;
        });
        state.set_site_worker_port(&site.domain, worker_port);

        // Even requesting /server.js (an existing file on disk) must proxy to the Node worker
        let req = axum::http::Request::builder()
            .header("host", "node-proxy.test")
            .uri("/server.js")
            .body(axum::body::Body::empty())
            .unwrap();
        let resp = handle_router_request(req, state.clone()).await;
        assert_eq!(resp.status(), axum::http::StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        assert_eq!(std::str::from_utf8(&body).unwrap(), "hello-from-node-worker");

        let _ = std::fs::remove_dir_all(&temp_dir);
    }
}
