use dockpanel_desktop_lib::wsl_manager::{
    self, parse_wsl_status_output,
};

#[test]
fn test_wsl_status_parsing_combinations() {
    let active = parse_wsl_status_output("active", Some(0));
    assert!(active.installed && active.running && active.service_active);

    let inactive = parse_wsl_status_output("inactive", Some(3));
    assert!(inactive.installed && inactive.running && !inactive.service_active);

    let failed = parse_wsl_status_output("failed", Some(3));
    assert!(failed.installed && failed.running && !failed.service_active);

    let distro_not_found = parse_wsl_status_output(
        "There is no distribution with the supplied name. Error code: Wsl/Service/WSL_E_DISTRO_NOT_FOUND",
        Some(1),
    );
    assert!(!distro_not_found.installed && !distro_not_found.running);
}

#[test]
fn test_open_folder_validation() {
    assert!(wsl_manager::open_folder("".to_string()).is_err());
    assert!(wsl_manager::open_site_folder("   ".to_string()).is_err());
}

#[test]
fn test_open_vscode_validation() {
    assert!(wsl_manager::open_vscode("".to_string()).is_err());
    assert!(wsl_manager::open_vscode("   ".to_string()).is_err());
}

#[tokio::test]
async fn test_bootstrap_session_integration_mock() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();

    tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut buf = vec![0u8; 1024];
        let n = socket.read(&mut buf).await.unwrap();
        let req = String::from_utf8_lossy(&buf[..n]);

        assert!(req.contains("POST /api/auth/desktop-bootstrap"));
        assert!(req.contains("x-real-ip: 127.0.0.1") || req.contains("X-Real-IP: 127.0.0.1"));

        let body = r#"{"token":"jwt-mock-integration-token","user":{"id":42,"email":"admin@example.test","role":"admin"}}"#;
        let resp = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nSet-Cookie: session=xyz; Path=/; HttpOnly\r\nContent-Length: {}\r\n\r\n{}",
            body.len(),
            body
        );
        socket.write_all(resp.as_bytes()).await.unwrap();
    });

    let endpoint = format!("http://127.0.0.1:{}/api/auth/desktop-bootstrap", port);
    let res = wsl_manager::bootstrap_session(Some(endpoint)).await.unwrap();

    assert_eq!(res.token, "jwt-mock-integration-token");
    assert_eq!(res.user["id"], 42);
    assert_eq!(res.user["email"], "admin@example.test");
    assert!(res.cookie.unwrap().contains("session=xyz"));
}
