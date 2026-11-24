use dockpanel_api::services::live_sync::{
    rewrite_domains, PreflightCheckRequest, PreflightCheckResponse, SyncProgress, SyncRequest,
    SyncResponse,
};

#[test]
fn test_database_domain_rewrite_plain_sql() {
    let sql = r#"INSERT INTO wp_options VALUES (1, 'siteurl', 'https://project.test', 'yes');"#;
    let rewritten = rewrite_domains(sql, "https://project.test", "https://example.com");
    assert!(rewritten.contains("https://example.com"));
    assert!(!rewritten.contains("https://project.test"));
}

#[test]
fn test_database_domain_rewrite_serialized_php_length_increase() {
    // 19 bytes -> 29 bytes
    let sql = r#"s:19:"https://mysite.test";"#;
    let rewritten = rewrite_domains(sql, "https://mysite.test", "https://super-long-domain.com");
    assert_eq!(rewritten, r#"s:29:"https://super-long-domain.com";"#);
}

#[test]
fn test_database_domain_rewrite_serialized_php_length_decrease() {
    // 29 bytes -> 19 bytes
    let sql = r#"s:29:"https://super-long-domain.com";"#;
    let rewritten = rewrite_domains(sql, "https://super-long-domain.com", "https://mysite.test");
    assert_eq!(rewritten, r#"s:19:"https://mysite.test";"#);
}

#[test]
fn test_database_domain_rewrite_serialized_php_length_unchanged() {
    // 19 bytes -> 19 bytes
    let sql = r#"s:19:"https://mysite.test";"#;
    let rewritten = rewrite_domains(sql, "https://mysite.test", "https://target.test");
    assert_eq!(rewritten, r#"s:19:"https://target.test";"#);
}

#[test]
fn test_database_domain_rewrite_serialized_without_semicolon() {
    // 19 bytes -> 23 bytes
    let sql = r#"s:19:"https://mysite.test""#;
    let rewritten = rewrite_domains(sql, "https://mysite.test", "https://mysite-prod.com");
    assert_eq!(rewritten, r#"s:23:"https://mysite-prod.com""#);
}

#[test]
fn test_database_domain_rewrite_serialized_escaped_quotes() {
    let sql = r#"INSERT INTO `wp_options` VALUES (10, 'my_setting', 'a:1:{s:4:\"site\";s:19:\"https://mysite.test\";}', 'yes');"#;
    let rewritten = rewrite_domains(sql, "https://mysite.test", "https://target-live.com");
    assert!(rewritten.contains(r#"s:23:\"https://target-live.com\""#));
    assert!(!rewritten.contains("https://mysite.test"));
}

#[test]
fn test_database_domain_rewrite_multiple_serialized_and_plain() {
    let sql = r#"
INSERT INTO wp_options VALUES (1, 'siteurl', 'https://project.test', 'yes');
INSERT INTO wp_options VALUES (2, 'home', 'https://project.test', 'yes');
INSERT INTO wp_options VALUES (3, 'meta', 'a:2:{s:3:"url";s:20:"https://project.test";s:4:"desc";s:26:"Welcome to project.test ok";}', 'yes');
"#;
    let rewritten = rewrite_domains(sql, "https://project.test", "https://newsite.com");
    assert!(rewritten.contains("'https://newsite.com'"));
    assert!(rewritten.contains(r#"s:19:"https://newsite.com""#));
}

#[test]
fn test_database_domain_rewrite_bare_domain() {
    let sql = r#"s:11:"mysite.test"; INSERT INTO wp_posts VALUES (1, 'check mysite.test/page');"#;
    let rewritten = rewrite_domains(sql, "mysite.test", "newsite-production.com");
    assert_eq!(
        rewritten,
        r#"s:22:"newsite-production.com"; INSERT INTO wp_posts VALUES (1, 'check newsite-production.com/page');"#
    );
}

#[test]
fn test_database_domain_rewrite_target_contains_source() {
    // If target domain contains source domain (e.g. example.com -> sub.example.com),
    // replacement must be single-pass safe and NOT double-replace or corrupt length
    let sql = r#"s:11:"example.com"; INSERT INTO wp_options VALUES (1, 'siteurl', 'https://example.com', 'yes');"#;
    let rewritten = rewrite_domains(sql, "example.com", "sub.example.com");
    assert_eq!(
        rewritten,
        r#"s:15:"sub.example.com"; INSERT INTO wp_options VALUES (1, 'siteurl', 'https://sub.example.com', 'yes');"#
    );
}

#[test]
fn test_database_domain_rewrite_serialized_target_contains_source_full_url() {
    // Verified fix for Bug #1: Ensure single-pass replacement inside serialized string
    // does not double-replace when target contains source domain
    let sql = r#"s:19:"https://example.com"; INSERT INTO wp_options VALUES (1, 'siteurl', 'https://example.com', 'yes');"#;
    let rewritten = rewrite_domains(sql, "https://example.com", "https://sub.example.com");
    assert_eq!(
        rewritten,
        r#"s:23:"https://sub.example.com"; INSERT INTO wp_options VALUES (1, 'siteurl', 'https://sub.example.com', 'yes');"#
    );
    assert!(!rewritten.contains("sub.sub.example.com"));
}

#[test]
fn test_database_domain_rewrite_multibyte_utf8() {
    // "münchen" contains 'ü' which is 2 UTF-8 bytes:
    // https:// (8) + münchen (8 bytes: m(1)+ü(2)+n(1)+c(1)+h(1)+e(1)+n(1)) + .de (3) = 19 bytes
    let sql = r#"s:19:"https://münchen.de";"#;
    let rewritten = rewrite_domains(sql, "https://münchen.de", "https://munich.com");
    assert_eq!(rewritten, r#"s:18:"https://munich.com";"#);
}

#[test]
fn test_sync_request_and_response_structures() {
    let req = SyncRequest {
        remote_url: "https://panel.example.com".to_string(),
        api_token: "secret_token_123".to_string(),
        sync_files: true,
        sync_db: true,
        dry_run: false,
        remote_site_id: Some("remote-uuid-1".to_string()),
    };

    let json = serde_json::to_string(&req).expect("serialize SyncRequest");
    assert!(json.contains("panel.example.com"));
    assert!(json.contains("secret_token_123"));

    let deserialized: SyncRequest =
        serde_json::from_str(&json).expect("deserialize SyncRequest");
    assert_eq!(deserialized.remote_url, req.remote_url);
    assert_eq!(deserialized.sync_files, true);
    assert_eq!(deserialized.sync_db, true);
    assert_eq!(deserialized.dry_run, false);

    let preflight_req = PreflightCheckRequest {
        remote_url: "https://panel.example.com".to_string(),
        api_token: "tok".to_string(),
        remote_site_id: None,
    };
    let preflight_json =
        serde_json::to_string(&preflight_req).expect("serialize PreflightCheckRequest");
    assert!(preflight_json.contains("panel.example.com"));

    let preflight_resp = PreflightCheckResponse {
        success: true,
        message: "Connected successfully".to_string(),
        remote_site_found: true,
        remote_version: Some("2.248.5".to_string()),
        remote_domain: Some("example.com".to_string()),
        remote_site_id: Some("site-uuid-99".to_string()),
        can_push: true,
        can_pull: true,
    };
    let preflight_resp_json =
        serde_json::to_string(&preflight_resp).expect("serialize PreflightCheckResponse");
    assert!(preflight_resp_json.contains("Connected successfully"));

    let sync_resp = SyncResponse {
        sync_id: "sync-123".to_string(),
        message: "Sync started".to_string(),
        status: "in_progress".to_string(),
        progress: vec![SyncProgress {
            step: "preflight".to_string(),
            label: "Verifying credentials".to_string(),
            status: "done".to_string(),
            message: None,
        }],
    };
    let sync_resp_json = serde_json::to_string(&sync_resp).expect("serialize SyncResponse");
    assert!(sync_resp_json.contains("sync-123"));
    assert!(sync_resp_json.contains("Verifying credentials"));
}
