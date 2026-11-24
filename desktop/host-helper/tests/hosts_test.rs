use dockpanel_host_helper::hosts_manager;
use dockpanel_host_helper::protocol::{HelperRequest, HelperResponse, HostEntryInfo};
use dockpanel_host_helper::server;
use std::io::Write;
use tempfile::NamedTempFile;

#[test]
fn test_add_and_remove_host_entry() {
    let original = "127.0.0.1 localhost\n::1 localhost\n";
    let updated = hosts_manager::add_entry(original, "mysite.test", "127.0.0.1");
    assert!(updated.contains("127.0.0.1 mysite.test # dockpanel-desktop"));
    let removed = hosts_manager::remove_entry(&updated, "mysite.test");
    assert_eq!(removed.trim(), original.trim());
}

#[test]
fn test_add_entry_updates_existing_dockpanel_entry() {
    let original = "127.0.0.1 localhost\n192.168.1.100 blog.test # dockpanel-desktop\n::1 localhost\n";
    let updated = hosts_manager::add_entry(original, "blog.test", "127.0.0.1");
    assert!(updated.contains("127.0.0.1 blog.test # dockpanel-desktop"));
    assert!(!updated.contains("192.168.1.100"));
    let occurrences = updated.matches("blog.test").count();
    assert_eq!(occurrences, 1);
}

#[test]
fn test_add_entry_leaves_manual_entry_intact() {
    let original = "127.0.0.1 localhost\n10.0.0.5 manual.test\n";
    let updated = hosts_manager::add_entry(original, "manual.test", "127.0.0.1");
    assert!(updated.contains("10.0.0.5 manual.test"));
    assert!(updated.contains("127.0.0.1 manual.test # dockpanel-desktop"));
}

#[test]
fn test_remove_entry_only_removes_dockpanel_tagged_lines() {
    let original = "127.0.0.1 localhost\n127.0.0.1 manual.test\n127.0.0.1 managed.test # dockpanel-desktop\n";
    let after_manual_remove = hosts_manager::remove_entry(original, "manual.test");
    assert!(after_manual_remove.contains("127.0.0.1 manual.test"));

    let after_managed_remove = hosts_manager::remove_entry(original, "managed.test");
    assert!(!after_managed_remove.contains("managed.test"));
    assert!(after_managed_remove.contains("127.0.0.1 manual.test"));
}

#[test]
fn test_list_dockpanel_entries() {
    let content = "127.0.0.1 localhost\n\
                   127.0.0.1 app1.test # dockpanel-desktop\n\
                   ::1 localhost\n\
                   127.0.0.1 app2.local # dockpanel-desktop\n\
                   10.0.0.1 other.com\n";
    let entries = hosts_manager::list_dockpanel_entries(content);
    assert_eq!(entries.len(), 2);
    assert_eq!(entries[0], ("app1.test".to_string(), "127.0.0.1".to_string()));
    assert_eq!(entries[1], ("app2.local".to_string(), "127.0.0.1".to_string()));
}

#[test]
fn test_atomic_file_operations() {
    let mut temp = NamedTempFile::new().expect("create temp file");
    let initial_content = "127.0.0.1 localhost\n";
    temp.write_all(initial_content.as_bytes()).expect("write initial");

    let temp_path = temp.path();

    hosts_manager::add_host_entry(Some(temp_path), "store.test", "127.0.0.1").expect("add host entry");
    let content = hosts_manager::read_hosts_file(Some(temp_path)).expect("read hosts");
    assert!(content.contains("127.0.0.1 store.test # dockpanel-desktop"));

    hosts_manager::remove_host_entry(Some(temp_path), "store.test").expect("remove host entry");
    let final_content = hosts_manager::read_hosts_file(Some(temp_path)).expect("read hosts");
    assert!(!final_content.contains("store.test"));
    assert!(final_content.contains("127.0.0.1 localhost"));
}

#[test]
fn test_protocol_request_deserialization() {
    let add_json = r#"{"action": "add", "domain": "site.test", "ip": "127.0.0.1"}"#;
    let req: HelperRequest = serde_json::from_str(add_json).expect("deserialize add");
    match req {
        HelperRequest::Add { domain, ip } => {
            assert_eq!(domain, "site.test");
            assert_eq!(ip, Some("127.0.0.1".to_string()));
        }
        _ => panic!("Expected Add variant"),
    }

    let remove_json = r#"{"action": "remove", "domain": "site.test"}"#;
    let req: HelperRequest = serde_json::from_str(remove_json).expect("deserialize remove");
    match req {
        HelperRequest::Remove { domain } => {
            assert_eq!(domain, "site.test");
        }
        _ => panic!("Expected Remove variant"),
    }

    let ping_json = r#"{"action": "ping"}"#;
    let req: HelperRequest = serde_json::from_str(ping_json).expect("deserialize ping");
    assert!(matches!(req, HelperRequest::Ping));

    let list_json = r#"{"action": "list"}"#;
    let req: HelperRequest = serde_json::from_str(list_json).expect("deserialize list");
    assert!(matches!(req, HelperRequest::List));
}

#[test]
fn test_protocol_response_serialization() {
    let success_resp = HelperResponse::success("Operation completed");
    let json = serde_json::to_string(&success_resp).expect("serialize success");
    assert!(json.contains(r#""success":true"#));
    assert!(json.contains("Operation completed"));

    let err_resp = HelperResponse::error("Permission denied");
    let json = serde_json::to_string(&err_resp).expect("serialize error");
    assert!(json.contains(r#""success":false"#));
    assert!(json.contains("Permission denied"));

    let list_resp = HelperResponse::entries(vec![HostEntryInfo {
        domain: "app.test".to_string(),
        ip: "127.0.0.1".to_string(),
    }]);
    let json = serde_json::to_string(&list_resp).expect("serialize entries");
    assert!(json.contains(r#""success":true"#));
    assert!(json.contains("app.test"));
}

#[test]
fn test_server_process_request_str() {
    let mut temp = NamedTempFile::new().expect("create temp file");
    temp.write_all(b"127.0.0.1 localhost\n").expect("write initial");
    let temp_path = temp.path();

    // Ping
    let ping_resp = server::process_request_str(r#"{"action": "ping"}"#, Some(temp_path));
    assert!(ping_resp.success);
    assert_eq!(ping_resp.message.as_deref(), Some("pong"));

    // Add
    let add_resp = server::process_request_str(
        r#"{"action": "add", "domain": "mysite.local", "ip": "127.0.0.1"}"#,
        Some(temp_path),
    );
    assert!(add_resp.success);

    // List
    let list_resp = server::process_request_str(r#"{"action": "list"}"#, Some(temp_path));
    assert!(list_resp.success);
    let entries = list_resp.entries.expect("entries present");
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].domain, "mysite.local");
    assert_eq!(entries[0].ip, "127.0.0.1");

    // Remove
    let remove_resp = server::process_request_str(
        r#"{"action": "remove", "domain": "mysite.local"}"#,
        Some(temp_path),
    );
    assert!(remove_resp.success);

    // Verify removed
    let list_after = server::process_request_str(r#"{"action": "list"}"#, Some(temp_path));
    assert_eq!(list_after.entries.expect("entries present").len(), 0);
}

#[cfg(windows)]
#[tokio::test]
async fn test_named_pipe_e2e_communication() {
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
    use tokio::net::windows::named_pipe::ClientOptions;

    let pipe_name = r"\\.\pipe\dockpanel-test-pipe-e2e";
    let temp = NamedTempFile::new().expect("create temp file");
    let temp_path = temp.path().to_path_buf();

    let pipe_name_clone = pipe_name.to_string();
    tokio::spawn(async move {
        let _ = server::run_named_pipe_server_with_pipe(&pipe_name_clone, Some(&temp_path)).await;
    });

    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    let client = ClientOptions::new().open(pipe_name).expect("connect client to pipe");
    let (reader, mut writer) = tokio::io::split(client);
    let mut buf_reader = BufReader::new(reader);

    writer.write_all(b"{\"action\": \"ping\"}\n").await.expect("write ping");
    writer.flush().await.expect("flush");

    let mut response_line = String::new();
    buf_reader.read_line(&mut response_line).await.expect("read response");
    let resp: HelperResponse = serde_json::from_str(&response_line).expect("parse response");
    assert!(resp.success);
    assert_eq!(resp.message.as_deref(), Some("pong"));
}
