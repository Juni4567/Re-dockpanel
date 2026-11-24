use axum::http::{HeaderMap, StatusCode};
use jsonwebtoken::{decode, DecodingKey, Validation};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};
use uuid::Uuid;

use dockpanel_api::auth::Claims;
use dockpanel_api::config::Config;
use dockpanel_api::models::User;
use dockpanel_api::routes::auth::{
    extract_bootstrap_client_ip, is_loopback_ip, issue_session_pub, verify_desktop_bootstrap_guards,
};
use dockpanel_api::AppState;

/// Process-global lock to serialize tests that mutate environment variables.
static ENV_LOCK: OnceLock<Mutex<()>> = OnceLock::new();
fn env_lock() -> MutexGuard<'static, ()> {
    ENV_LOCK
        .get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|e| e.into_inner())
}

/// Helper to create a dummy AppState for token signing verification without live Postgres.
fn create_test_app_state() -> AppState {
    let config = Arc::new(Config {
        database_url: "postgres://mock:mock@127.0.0.1:5432/mock".into(),
        jwt_secret: "mock-jwt-secret-at-least-32-chars-long-security".into(),
        agent_socket: "/var/run/dockpanel/agent.sock".into(),
        agent_token: "mock-agent-token".into(),
        listen_addr: "127.0.0.1:3080".into(),
        db_max_connections: 5,
        base_url: "".into(),
        cors_origins: vec![],
    });

    let (notif_tx, _) = tokio::sync::broadcast::channel(16);
    let (shutdown_tx, _) = tokio::sync::broadcast::channel(1);

    // Dummy PgPool placeholder
    let db_opts = sqlx::postgres::PgPoolOptions::new()
        .max_connections(1);
    // Unconnected pool for token-minting tests
    let db = db_opts.connect_lazy("postgres://mock:mock@127.0.0.1:5432/mock").unwrap();

    let agent = dockpanel_api::services::agent::AgentClient::new(
        config.agent_socket.clone(),
        config.agent_token.clone(),
    );
    let agents = dockpanel_api::services::agent::AgentRegistry::new(
        agent.clone(),
        db.clone(),
        config.jwt_secret.clone(),
    );

    AppState {
        db,
        config: config.clone(),
        agent,
        agents,
        login_attempts: Arc::new(Mutex::new(std::collections::HashMap::new())),
        token_blacklist: Arc::new(tokio::sync::RwLock::new(std::collections::HashSet::new())),
        twofa_attempts: Arc::new(Mutex::new(std::collections::HashMap::new())),
        webhook_attempts: Arc::new(Mutex::new(std::collections::HashMap::new())),
        agent_rate_limits: Arc::new(Mutex::new(std::collections::HashMap::new())),
        provision_logs: Arc::new(Mutex::new(std::collections::HashMap::new())),
        oauth_states: Arc::new(Mutex::new(std::collections::HashMap::new())),
        notif_tx,
        sessions_revoked_at: Arc::new(tokio::sync::RwLock::new(None)),
        deploy_owners: Arc::new(Mutex::new(std::collections::HashMap::new())),
        shutdown_tx,
        passkey_challenges: dockpanel_api::routes::passkeys::new_challenge_store(),
        panel_update_state: dockpanel_api::services::panel_update::new_state_handle(),
    }
}

fn create_mock_admin_user() -> User {
    let now = chrono::Utc::now();
    User {
        id: Uuid::new_v4(),
        email: "local@dockpanel.desktop".to_string(),
        password_hash: "dummy_hash".to_string(),
        role: "admin".to_string(),
        email_verified: true,
        email_token: None,
        reset_token: None,
        reset_expires: None,
        totp_secret: None,
        totp_enabled: false,
        recovery_codes: None,
        oauth_provider: None,
        oauth_id: None,
        created_at: now,
        updated_at: now,
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Security Constraint 1: DESKTOP_MODE Gate Tests
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn test_desktop_bootstrap_rejected_when_desktop_mode_unset() {
    let _guard = env_lock();
    unsafe { std::env::remove_var("DESKTOP_MODE"); }

    let mut headers = HeaderMap::new();
    headers.insert("x-real-ip", "127.0.0.1".parse().unwrap());

    let result = verify_desktop_bootstrap_guards(&headers);
    assert!(result.is_err(), "Must be rejected when DESKTOP_MODE is unset");

    let err = result.unwrap_err();
    assert_eq!(err.0, StatusCode::FORBIDDEN);
    assert_eq!(
        err.1.0.get("error").and_then(|v| v.as_str()),
        Some("Desktop mode is not enabled")
    );
}

#[test]
fn test_desktop_bootstrap_rejected_when_desktop_mode_false() {
    let _guard = env_lock();
    unsafe { std::env::set_var("DESKTOP_MODE", "false"); }

    let mut headers = HeaderMap::new();
    headers.insert("x-real-ip", "127.0.0.1".parse().unwrap());

    let result = verify_desktop_bootstrap_guards(&headers);
    unsafe { std::env::remove_var("DESKTOP_MODE"); }

    assert!(result.is_err(), "Must be rejected when DESKTOP_MODE=false");
    let err = result.unwrap_err();
    assert_eq!(err.0, StatusCode::FORBIDDEN);
    assert_eq!(
        err.1.0.get("error").and_then(|v| v.as_str()),
        Some("Desktop mode is not enabled")
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// Security Constraint 2: Loopback IP Gate Tests
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn test_desktop_bootstrap_rejected_when_ip_is_external() {
    let _guard = env_lock();
    unsafe { std::env::set_var("DESKTOP_MODE", "true"); }

    for external_ip in &["192.168.1.100", "10.0.0.1", "172.16.0.5", "8.8.8.8", "203.0.113.1"] {
        let mut headers = HeaderMap::new();
        headers.insert("x-real-ip", external_ip.parse().unwrap());

        let result = verify_desktop_bootstrap_guards(&headers);
        assert!(
            result.is_err(),
            "Must reject external IP {external_ip} even when DESKTOP_MODE=true"
        );
        let err = result.unwrap_err();
        assert_eq!(err.0, StatusCode::FORBIDDEN);
        assert_eq!(
            err.1.0.get("error").and_then(|v| v.as_str()),
            Some("Desktop bootstrap only allowed from local loopback")
        );
    }

    unsafe { std::env::remove_var("DESKTOP_MODE"); }
}

#[test]
fn test_desktop_bootstrap_rejected_when_no_ip_header_provided() {
    let _guard = env_lock();
    unsafe { std::env::set_var("DESKTOP_MODE", "true"); }

    let headers = HeaderMap::new();
    let result = verify_desktop_bootstrap_guards(&headers);
    unsafe { std::env::remove_var("DESKTOP_MODE"); }

    assert!(result.is_err(), "Must reject request with missing IP header");
    let err = result.unwrap_err();
    assert_eq!(err.0, StatusCode::FORBIDDEN);
    assert_eq!(
        err.1.0.get("error").and_then(|v| v.as_str()),
        Some("Desktop bootstrap only allowed from local loopback")
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// Security Constraint 3: Loopback Extraction & Parsing Helpers
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn test_is_loopback_ip_helper() {
    assert!(is_loopback_ip("127.0.0.1"));
    assert!(is_loopback_ip("127.0.0.2"));
    assert!(is_loopback_ip("127.255.255.254"));
    assert!(is_loopback_ip("::1"));
    assert!(is_loopback_ip("localhost"));

    assert!(!is_loopback_ip("192.168.0.1"));
    assert!(!is_loopback_ip("10.10.10.10"));
    assert!(!is_loopback_ip("172.20.0.1"));
    assert!(!is_loopback_ip("unknown"));
    assert!(!is_loopback_ip(""));
}

#[test]
fn test_extract_bootstrap_client_ip_headers() {
    let mut headers = HeaderMap::new();
    headers.insert("x-real-ip", "127.0.0.1".parse().unwrap());
    assert_eq!(extract_bootstrap_client_ip(&headers), Some("127.0.0.1".to_string()));

    let mut headers = HeaderMap::new();
    headers.insert("x-forwarded-for", "::1, 10.0.0.1".parse().unwrap());
    assert_eq!(extract_bootstrap_client_ip(&headers), Some("::1".to_string()));
}

// ─────────────────────────────────────────────────────────────────────────────
// Functional Test: Loopback + DESKTOP_MODE=true mints valid admin session
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn test_desktop_bootstrap_guards_succeed_for_ipv4_and_ipv6_loopback() {
    let _guard = env_lock();
    unsafe { std::env::set_var("DESKTOP_MODE", "true"); }

    // Test IPv4 loopback
    let mut headers_v4 = HeaderMap::new();
    headers_v4.insert("x-real-ip", "127.0.0.1".parse().unwrap());
    let res_v4 = verify_desktop_bootstrap_guards(&headers_v4);
    assert!(res_v4.is_ok());
    assert_eq!(res_v4.unwrap(), "127.0.0.1");

    // Test IPv6 loopback
    let mut headers_v6 = HeaderMap::new();
    headers_v6.insert("x-real-ip", "::1".parse().unwrap());
    let res_v6 = verify_desktop_bootstrap_guards(&headers_v6);
    assert!(res_v6.is_ok());
    assert_eq!(res_v6.unwrap(), "::1");

    unsafe { std::env::remove_var("DESKTOP_MODE"); }
}

#[tokio::test]
async fn test_desktop_bootstrap_issues_valid_admin_session_jwt_and_cookie() {
    let state = create_test_app_state();
    let admin_user = create_mock_admin_user();
    let mut headers = HeaderMap::new();
    headers.insert("x-real-ip", "127.0.0.1".parse().unwrap());

    let (token, cookie, jti) = issue_session_pub(&state, &admin_user, &headers)
        .expect("Minting admin session must succeed");

    assert!(!token.is_empty(), "Token must not be empty");
    assert!(!jti.is_empty(), "JTI must not be empty");
    assert!(cookie.contains(&format!("token={token}")), "Cookie must contain minted token");
    assert!(cookie.contains("HttpOnly"), "Cookie must be HttpOnly");
    assert!(cookie.contains("SameSite=Lax"), "Cookie must have SameSite=Lax");
    assert!(cookie.contains("Max-Age=7200"), "Cookie must have 2h lifetime");

    // Decode and verify JWT claims
    let decoding_key = DecodingKey::from_secret(state.config.jwt_secret.as_bytes());
    let mut validation = Validation::default();
    validation.validate_exp = true;

    let token_data = decode::<Claims>(&token, &decoding_key, &validation)
        .expect("Minted token must decode and validate successfully");

    assert_eq!(token_data.claims.sub, admin_user.id);
    assert_eq!(token_data.claims.email, "local@dockpanel.desktop");
    assert_eq!(token_data.claims.role, "admin");
    assert_eq!(token_data.claims.jti, Some(jti));
    assert!(token_data.claims.exp > token_data.claims.iat);
}
