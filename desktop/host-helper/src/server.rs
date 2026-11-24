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
pub async fn run_named_pipe_server_with_pipe(
    pipe_name: &str,
    hosts_path: Option<&Path>,
) -> std::io::Result<()> {
    use tokio::net::windows::named_pipe::ServerOptions;

    println!("Starting DockPanel Host Helper named pipe server on {pipe_name}...");

    // Shared mutex guarding hosts file mutations across concurrent IPC connections
    let lock = Arc::new(tokio::sync::Mutex::new(()));

    let mut server = ServerOptions::new()
        .first_pipe_instance(true)
        .create(pipe_name)?;

    println!("Named pipe server listening for connections.");

    loop {
        server.connect().await?;
        let connected_client = server;

        server = ServerOptions::new().create(pipe_name)?;

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
