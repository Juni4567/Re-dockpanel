use serde::{Deserialize, Serialize};

pub const WSL_DISTRO_NAME: &str = "dockpanel-desktop";
pub const DOCKPANEL_API_SERVICE: &str = "dockpanel-api";
pub const DEFAULT_BOOTSTRAP_URL: &str = "http://127.0.0.1:8080/api/auth/desktop-bootstrap";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WslStatus {
    pub installed: bool,
    pub running: bool,
    pub service_active: bool,
    pub output: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BootstrapResponse {
    pub token: String,
    pub user: serde_json::Value,
    pub cookie: Option<String>,
}

/// Parse command output from `systemctl is-active` or WSL errors into a structured `WslStatus`.
pub fn parse_wsl_status_output(output: &str, exit_code: Option<i32>) -> WslStatus {
    let trimmed = output.trim();
    if exit_code == Some(0) && trimmed == "active" {
        WslStatus {
            installed: true,
            running: true,
            service_active: true,
            output: trimmed.to_string(),
        }
    } else if trimmed == "inactive"
        || trimmed == "failed"
        || trimmed == "activating"
        || trimmed == "deactivating"
    {
        WslStatus {
            installed: true,
            running: true,
            service_active: false,
            output: trimmed.to_string(),
        }
    } else if trimmed.contains("There is no distribution")
        || trimmed.contains("not found")
        || trimmed.contains("Wsl/Service")
    {
        WslStatus {
            installed: false,
            running: false,
            service_active: false,
            output: trimmed.to_string(),
        }
    } else {
        WslStatus {
            installed: true,
            running: exit_code.map(|c| c != 0).unwrap_or(false) && !trimmed.is_empty(),
            service_active: false,
            output: trimmed.to_string(),
        }
    }
}

/// Opens a path in Windows File Explorer (or platform default file manager).
#[tauri::command]
pub fn open_folder(path: String) -> Result<String, String> {
    let trimmed = path.trim();
    if trimmed.is_empty() {
        return Err("Path cannot be empty".to_string());
    }

    #[cfg(target_os = "windows")]
    {
        let mut cmd = std::process::Command::new("explorer.exe");
        cmd.arg(trimmed);
        match cmd.spawn() {
            Ok(_) => Ok(format!("Opened explorer for {}", trimmed)),
            Err(e) => Err(format!("Failed to spawn explorer.exe: {}", e)),
        }
    }

    #[cfg(not(target_os = "windows"))]
    {
        let mut cmd = std::process::Command::new("xdg-open");
        cmd.arg(trimmed);
        match cmd.spawn() {
            Ok(_) => Ok(format!("Opened file manager for {}", trimmed)),
            Err(_) => Ok(format!("Simulated folder open for {}", trimmed)),
        }
    }
}

/// Alias for open_folder to match frontend invocation options.
#[tauri::command]
pub fn open_site_folder(path: String) -> Result<String, String> {
    open_folder(path)
}

/// Opens a given path in VS Code via Windows `cmd.exe /c code <path>`.
#[tauri::command]
pub fn open_vscode(path: String) -> Result<String, String> {
    let trimmed = path.trim();
    if trimmed.is_empty() {
        return Err("Path cannot be empty".to_string());
    }

    #[cfg(target_os = "windows")]
    {
        let mut cmd = std::process::Command::new("cmd.exe");
        cmd.args(["/c", "code", trimmed]);
        match cmd.spawn() {
            Ok(_) => Ok(format!("Opened VS Code for {}", trimmed)),
            Err(e) => Err(format!("Failed to execute 'cmd.exe /c code': {}", e)),
        }
    }

    #[cfg(not(target_os = "windows"))]
    {
        let mut cmd = std::process::Command::new("code");
        cmd.arg(trimmed);
        match cmd.spawn() {
            Ok(_) => Ok(format!("Opened VS Code for {}", trimmed)),
            Err(_) => Ok(format!("Simulated VS Code launch for {}", trimmed)),
        }
    }
}

/// Check WSL2 micro-instance and dockpanel-api service health.
pub fn check_wsl_status_custom(distro: &str, service: &str) -> Result<WslStatus, String> {
    #[cfg(target_os = "windows")]
    {
        let mut cmd = std::process::Command::new("wsl.exe");
        cmd.args(["-d", distro, "-u", "root", "--", "systemctl", "is-active", service]);
        match cmd.output() {
            Ok(output) => {
                let stdout = String::from_utf8_lossy(&output.stdout);
                let stderr = String::from_utf8_lossy(&output.stderr);
                let combined = if stderr.trim().is_empty() {
                    stdout.to_string()
                } else if stdout.trim().is_empty() {
                    stderr.to_string()
                } else {
                    format!("{}\n{}", stdout.trim(), stderr.trim())
                };
                Ok(parse_wsl_status_output(&combined, output.status.code()))
            }
            Err(e) => {
                if e.kind() == std::io::ErrorKind::NotFound {
                    Ok(WslStatus {
                        installed: false,
                        running: false,
                        service_active: false,
                        output: "wsl.exe not found on system".to_string(),
                    })
                } else {
                    Err(format!("Failed to execute wsl.exe: {}", e))
                }
            }
        }
    }

    #[cfg(not(target_os = "windows"))]
    {
        let _ = (distro, service);
        Ok(WslStatus {
            installed: true,
            running: true,
            service_active: true,
            output: "active (simulated)".to_string(),
        })
    }
}

#[tauri::command]
pub fn check_wsl_status() -> Result<WslStatus, String> {
    check_wsl_status_custom(WSL_DISTRO_NAME, DOCKPANEL_API_SERVICE)
}

/// Restart the dockpanel-api engine inside the WSL2 micro-instance.
pub fn restart_engine_custom(distro: &str, service: &str) -> Result<String, String> {
    #[cfg(target_os = "windows")]
    {
        let mut cmd = std::process::Command::new("wsl.exe");
        cmd.args(["-d", distro, "-u", "root", "--", "systemctl", "restart", service]);
        match cmd.output() {
            Ok(output) => {
                if output.status.success() {
                    Ok(format!("Successfully restarted {} in {}", service, distro))
                } else {
                    let stderr = String::from_utf8_lossy(&output.stderr);
                    let stdout = String::from_utf8_lossy(&output.stdout);
                    let msg = if !stderr.trim().is_empty() {
                        stderr.trim().to_string()
                    } else {
                        stdout.trim().to_string()
                    };
                    Err(format!("Failed to restart {}: {}", service, msg))
                }
            }
            Err(e) => Err(format!("Failed to execute wsl.exe: {}", e)),
        }
    }

    #[cfg(not(target_os = "windows"))]
    {
        let _ = (distro, service);
        Ok("Successfully restarted dockpanel-api engine (simulated)".to_string())
    }
}

#[tauri::command]
pub fn restart_engine() -> Result<String, String> {
    restart_engine_custom(WSL_DISTRO_NAME, DOCKPANEL_API_SERVICE)
}

/// Issues loopback request to bootstrap authentication with dockpanel-api.
#[tauri::command]
pub async fn bootstrap_session(endpoint: Option<String>) -> Result<BootstrapResponse, String> {
    let url = endpoint.as_deref().unwrap_or(DEFAULT_BOOTSTRAP_URL);
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .map_err(|e| format!("Failed to create HTTP client: {}", e))?;

    let res = client
        .post(url)
        .header("X-Real-IP", "127.0.0.1")
        .header("X-Forwarded-For", "127.0.0.1")
        .header("Content-Type", "application/json")
        .body("{}")
        .send()
        .await
        .map_err(|e| format!("Bootstrap request failed: {}", e))?;

    let status = res.status();
    let cookie = res
        .headers()
        .get(reqwest::header::SET_COOKIE)
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());

    if !status.is_success() {
        let err_text = res.text().await.unwrap_or_default();
        return Err(format!(
            "Bootstrap endpoint returned HTTP {}: {}",
            status.as_u16(),
            err_text
        ));
    }

    let body: serde_json::Value = res
        .json()
        .await
        .map_err(|e| format!("Failed to parse bootstrap JSON response: {}", e))?;

    let token = body
        .get("token")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "Bootstrap response missing 'token' string field".to_string())?
        .to_string();

    let user = body.get("user").cloned().unwrap_or(serde_json::Value::Null);

    Ok(BootstrapResponse {
        token,
        user,
        cookie,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_wsl_status_active() {
        let status = parse_wsl_status_output("active\n", Some(0));
        assert_eq!(
            status,
            WslStatus {
                installed: true,
                running: true,
                service_active: true,
                output: "active".to_string(),
            }
        );
    }

    #[test]
    fn test_parse_wsl_status_inactive() {
        let status = parse_wsl_status_output("inactive\n", Some(3));
        assert_eq!(
            status,
            WslStatus {
                installed: true,
                running: true,
                service_active: false,
                output: "inactive".to_string(),
            }
        );
    }

    #[test]
    fn test_parse_wsl_status_failed() {
        let status = parse_wsl_status_output("failed\n", Some(3));
        assert_eq!(
            status,
            WslStatus {
                installed: true,
                running: true,
                service_active: false,
                output: "failed".to_string(),
            }
        );
    }

    #[test]
    fn test_parse_wsl_status_distro_not_found() {
        let status = parse_wsl_status_output(
            "There is no distribution with the supplied name.\nError code: Wsl/Service/WSL_E_DISTRO_NOT_FOUND",
            Some(1),
        );
        assert!(!status.installed);
        assert!(!status.running);
        assert!(!status.service_active);
    }

    #[test]
    fn test_open_folder_empty_path_error() {
        let err = open_folder("  ".to_string()).unwrap_err();
        assert!(err.contains("cannot be empty"));
    }

    #[test]
    fn test_open_vscode_empty_path_error() {
        let err = open_vscode("".to_string()).unwrap_err();
        assert!(err.contains("cannot be empty"));
    }

    #[tokio::test]
    async fn test_bootstrap_session_against_mock_http_server() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        use tokio::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();

        // Spawn mock server task
        tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut buf = vec![0u8; 2048];
            let n = socket.read(&mut buf).await.unwrap();
            let req = String::from_utf8_lossy(&buf[..n]);

            // Verify loopback headers
            assert!(req.contains("POST /api/auth/desktop-bootstrap"));
            assert!(req.contains("x-real-ip: 127.0.0.1") || req.contains("X-Real-IP: 127.0.0.1"));

            let body = r#"{"token":"test-admin-jwt-token-xyz","user":{"id":1,"email":"local@dockpanel.desktop","role":"admin"}}"#;
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nSet-Cookie: dockpanel_session=cookie123; HttpOnly\r\nContent-Length: {}\r\n\r\n{}",
                body.len(),
                body
            );
            socket.write_all(response.as_bytes()).await.unwrap();
        });

        let target_url = format!("http://127.0.0.1:{}/api/auth/desktop-bootstrap", port);
        let resp = bootstrap_session(Some(target_url)).await.unwrap();

        assert_eq!(resp.token, "test-admin-jwt-token-xyz");
        assert_eq!(resp.cookie.as_deref(), Some("dockpanel_session=cookie123; HttpOnly"));
        assert_eq!(resp.user["email"], "local@dockpanel.desktop");
        assert_eq!(resp.user["role"], "admin");
    }

    #[tokio::test]
    async fn test_bootstrap_session_handles_server_error() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        use tokio::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();

        tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut buf = vec![0u8; 1024];
            let _ = socket.read(&mut buf).await.unwrap();

            let body = r#"{"error":"Desktop mode is not enabled"}"#;
            let response = format!(
                "HTTP/1.1 403 Forbidden\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
                body.len(),
                body
            );
            socket.write_all(response.as_bytes()).await.unwrap();
        });

        let target_url = format!("http://127.0.0.1:{}/api/auth/desktop-bootstrap", port);
        let err = bootstrap_session(Some(target_url)).await.unwrap_err();
        assert!(err.contains("HTTP 403"));
        assert!(err.contains("Desktop mode is not enabled"));
    }
}
