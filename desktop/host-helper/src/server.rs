use std::path::Path;
use std::sync::Arc;
use crate::hosts_manager;
use crate::protocol::{HelperRequest, HelperResponse, HostEntryInfo};

pub const DEFAULT_PIPE_NAME: &str = r"\\.\pipe\dockpanel-host-helper";

pub fn process_request(req: HelperRequest, hosts_path: Option<&Path>) -> HelperResponse {
    match req {
        HelperRequest::Ping => HelperResponse::success("pong"),
        HelperRequest::Add { domain, ip } => {
            if let Err(e) = hosts_manager::validate_domain(&domain) {
                return HelperResponse::error(e);
            }
            let ip_str = ip.unwrap_or_else(|| hosts_manager::DEFAULT_IP.to_string());
            if let Err(e) = hosts_manager::validate_ip(&ip_str) {
                return HelperResponse::error(e);
            }
            match hosts_manager::add_host_entry(hosts_path, &domain, &ip_str) {
                Ok(()) => HelperResponse::success(format!("Added host mapping {domain} -> {ip_str}")),
                Err(e) => HelperResponse::error(format!("Failed to write hosts entry: {e}")),
            }
        }
        HelperRequest::Remove { domain } => {
            if let Err(e) = hosts_manager::validate_domain(&domain) {
                return HelperResponse::error(e);
            }
            match hosts_manager::remove_host_entry(hosts_path, &domain) {
                Ok(()) => HelperResponse::success(format!("Removed host mapping for {domain}")),
                Err(e) => HelperResponse::error(format!("Failed to remove hosts entry: {e}")),
            }
        }
        HelperRequest::List => {
            match hosts_manager::list_hosts_file_entries(hosts_path) {
                Ok(entries) => {
                    let items = entries
                        .into_iter()
                        .map(|(domain, ip)| HostEntryInfo { domain, ip })
                        .collect();
                    HelperResponse::entries(items)
                }
                Err(e) => HelperResponse::error(format!("Failed to read hosts entries: {e}")),
            }
        }
    }
}

pub fn process_request_str(input: &str, hosts_path: Option<&Path>) -> HelperResponse {
    match serde_json::from_str::<HelperRequest>(input) {
        Ok(req) => process_request(req, hosts_path),
        Err(e) => HelperResponse::error(format!("Invalid request JSON: {e}")),
    }
}

pub async fn process_request_async(
    req: HelperRequest,
    hosts_path: Option<&Path>,
    lock: &tokio::sync::Mutex<()>,
) -> HelperResponse {
    match req {
        HelperRequest::Ping => HelperResponse::success("pong"),
        HelperRequest::Add { domain, ip } => {
            if let Err(e) = hosts_manager::validate_domain(&domain) {
                return HelperResponse::error(e);
            }
            let ip_str = ip.unwrap_or_else(|| hosts_manager::DEFAULT_IP.to_string());
            if let Err(e) = hosts_manager::validate_ip(&ip_str) {
                return HelperResponse::error(e);
            }
            let _guard = lock.lock().await;
            match hosts_manager::add_host_entry(hosts_path, &domain, &ip_str) {
                Ok(()) => HelperResponse::success(format!("Added host mapping {domain} -> {ip_str}")),
                Err(e) => HelperResponse::error(format!("Failed to write hosts entry: {e}")),
            }
        }
        HelperRequest::Remove { domain } => {
            if let Err(e) = hosts_manager::validate_domain(&domain) {
                return HelperResponse::error(e);
            }
            let _guard = lock.lock().await;
            match hosts_manager::remove_host_entry(hosts_path, &domain) {
                Ok(()) => HelperResponse::success(format!("Removed host mapping for {domain}")),
                Err(e) => HelperResponse::error(format!("Failed to remove hosts entry: {e}")),
            }
        }
        HelperRequest::List => {
            let _guard = lock.lock().await;
            match hosts_manager::list_hosts_file_entries(hosts_path) {
                Ok(entries) => {
                    let items = entries
                        .into_iter()
                        .map(|(domain, ip)| HostEntryInfo { domain, ip })
                        .collect();
                    HelperResponse::entries(items)
                }
                Err(e) => HelperResponse::error(format!("Failed to read hosts entries: {e}")),
            }
        }
    }
}

pub async fn process_request_str_async(
    input: &str,
    hosts_path: Option<&Path>,
    lock: &tokio::sync::Mutex<()>,
) -> HelperResponse {
    match serde_json::from_str::<HelperRequest>(input) {
        Ok(req) => process_request_async(req, hosts_path, lock).await,
        Err(e) => HelperResponse::error(format!("Invalid request JSON: {e}")),
    }
}

#[cfg(windows)]
pub async fn run_named_pipe_server(hosts_path: Option<&Path>) -> std::io::Result<()> {
    run_named_pipe_server_with_pipe(DEFAULT_PIPE_NAME, hosts_path).await
}

#[cfg(windows)]
unsafe fn create_pipe_with_user_access(
    pipe_name: &str,
    first_instance: bool,
) -> std::io::Result<tokio::net::windows::named_pipe::NamedPipeServer> {
    use std::ffi::c_void;
    use tokio::net::windows::named_pipe::ServerOptions;

    #[repr(C)]
    struct SecurityAttributes {
        n_length: u32,
        lp_security_descriptor: *mut c_void,
        b_inherit_handle: i32,
    }

    #[link(name = "advapi32")]
    extern "system" {
        fn ConvertStringSecurityDescriptorToSecurityDescriptorW(
            string_security_descriptor: *const u16,
            string_sd_revision: u32,
            security_descriptor: *mut *mut c_void,
            security_descriptor_size: *mut u32,
        ) -> i32;
    }

    #[link(name = "kernel32")]
    extern "system" {
        fn LocalFree(hmem: *mut c_void) -> *mut c_void;
    }

    // Grant Full Control to LocalSystem (SY) and Administrators (BA),
    // and Generic Read/Write to Authenticated Users (AU) so non-elevated Desktop UI can manage hosts.
    let sddl: Vec<u16> = "D:(A;;GA;;;SY)(A;;GA;;;BA)(A;;GRGW;;;AU)\0"
        .encode_utf16()
        .collect();
    let mut sd_ptr: *mut c_void = std::ptr::null_mut();
    let ok = ConvertStringSecurityDescriptorToSecurityDescriptorW(
        sddl.as_ptr(),
        1, // SDDL_REVISION_1
        &mut sd_ptr,
        std::ptr::null_mut(),
    );

    let mut opts = ServerOptions::new();
    if first_instance {
        opts.first_pipe_instance(true);
    }

    if ok != 0 && !sd_ptr.is_null() {
        let mut sa = SecurityAttributes {
            n_length: std::mem::size_of::<SecurityAttributes>() as u32,
            lp_security_descriptor: sd_ptr,
            b_inherit_handle: 0,
        };
        let res = opts.create_with_security_attributes_raw(
            pipe_name,
            &mut sa as *mut SecurityAttributes as *mut c_void,
        );
        LocalFree(sd_ptr);
        res
    } else {
        opts.create(pipe_name)
    }
}

#[cfg(windows)]
pub async fn run_named_pipe_server_with_pipe(
    pipe_name: &str,
    hosts_path: Option<&Path>,
) -> std::io::Result<()> {
    println!("Starting DockPanel Host Helper named pipe server on {pipe_name}...");

    // Shared mutex guarding hosts file mutations across concurrent IPC connections
    let lock = Arc::new(tokio::sync::Mutex::new(()));

    let mut server = unsafe { create_pipe_with_user_access(pipe_name, true)? };

    println!("Named pipe server listening for connections.");

    loop {
        server.connect().await?;
        let connected_client = server;

        server = unsafe { create_pipe_with_user_access(pipe_name, false)? };

        let hosts_path_buf = hosts_path.map(|p| p.to_path_buf());
        let lock_clone = lock.clone();
        tokio::spawn(async move {
            if let Err(e) = handle_pipe_client(connected_client, hosts_path_buf.as_deref(), lock_clone).await {
                eprintln!("Client handling error: {e}");
            }
        });
    }
}

#[cfg(windows)]
pub async fn handle_pipe_client(
    stream: tokio::net::windows::named_pipe::NamedPipeServer,
    hosts_path: Option<&Path>,
    lock: Arc<tokio::sync::Mutex<()>>,
) -> std::io::Result<()> {
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

    let (reader, mut writer) = tokio::io::split(stream);
    let mut buf_reader = BufReader::new(reader);
    let mut line = String::new();

    while buf_reader.read_line(&mut line).await? > 0 {
        let trimmed = line.trim();
        if !trimmed.is_empty() {
            let resp = process_request_str_async(trimmed, hosts_path, &lock).await;
            let mut serialized = serde_json::to_vec(&resp).unwrap_or_default();
            serialized.push(b'\n');
            writer.write_all(&serialized).await?;
            writer.flush().await?;
        }
        line.clear();
    }

    Ok(())
}

#[cfg(not(windows))]
pub async fn run_named_pipe_server(_hosts_path: Option<&Path>) -> std::io::Result<()> {
    println!("Named pipe server is only supported on Windows.");
    Ok(())
}

#[cfg(not(windows))]
pub async fn run_named_pipe_server_with_pipe(
    _pipe_name: &str,
    _hosts_path: Option<&Path>,
) -> std::io::Result<()> {
    println!("Named pipe server is only supported on Windows.");
    Ok(())
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader, Write};

    #[tokio::test]
    async fn test_pipe_security_attributes_allows_client_roundtrip() {
        let pipe_name = format!(r"\\.\pipe\dockpanel-host-helper-test-{}", std::process::id());
        let pipe_clone = pipe_name.clone();
        let server = unsafe { create_pipe_with_user_access(&pipe_clone, true).unwrap() };
        let lock = Arc::new(tokio::sync::Mutex::new(()));

        let srv_task = tokio::spawn(async move {
            server.connect().await.unwrap();
            let _ = handle_pipe_client(server, None, lock).await;
        });

        let client_res = tokio::task::spawn_blocking(move || {
            let mut pipe = std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .open(&pipe_name)
                .unwrap();
            writeln!(pipe, r#"{{"action":"ping"}}"#).unwrap();
            pipe.flush().unwrap();
            let mut reader = BufReader::new(pipe);
            let mut line = String::new();
            reader.read_line(&mut line).unwrap();
            line
        })
        .await
        .unwrap();

        assert!(client_res.contains("\"success\":true"));
        assert!(client_res.contains("pong"));
        srv_task.abort();
    }
}
