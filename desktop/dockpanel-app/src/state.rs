use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use crate::router::RouterConfig;

fn default_runtime() -> String {
    "php".to_string()
}

fn default_true() -> bool {
    true
}

fn default_upload_mb() -> i32 {
    64
}

fn default_php_memory_mb() -> i32 {
    256
}

fn default_php_workers() -> i32 {
    4
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LocalSite {
    pub id: String,
    pub domain: String,
    #[serde(default = "default_runtime")]
    pub runtime: String,
    pub php_version: String,
    pub document_root: String,
    pub status: String,
    pub created_at: String,
    pub ssl_enabled: bool,
    #[serde(default)]
    pub php_preset: Option<String>,
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default = "default_upload_mb")]
    pub max_upload_mb: i32,
    #[serde(default = "default_php_memory_mb")]
    pub php_memory_mb: i32,
    #[serde(default = "default_php_workers")]
    pub php_max_workers: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AppData {
    pub sites: Vec<LocalSite>,
}

#[derive(Debug, Clone, Default)]
pub struct PhpInstallJob {
    pub steps: Vec<serde_json::Value>,
    pub done: bool,
}

#[derive(Clone)]
pub struct AppState {
    pub data: Arc<Mutex<AppData>>,
    pub router_config: Arc<Mutex<RouterConfig>>,
    pub worker_ports: Arc<Mutex<HashMap<String, u16>>>,
    pub worker_processes: Arc<Mutex<HashMap<String, std::process::Child>>>,
    pub router_handle: Arc<tokio::sync::Mutex<Option<tokio::task::JoinHandle<()>>>>,
    pub php_installs: Arc<Mutex<HashMap<String, PhpInstallJob>>>,
    pub config_dir: PathBuf,
    pub sites_dir: PathBuf,
}

/// Finds an available loopback TCP port.
pub fn find_available_loopback_port() -> Option<u16> {
    std::net::TcpListener::bind("127.0.0.1:0")
        .ok()
        .and_then(|l| l.local_addr().ok().map(|addr| addr.port()))
}

/// Returns the `%USERPROFILE%\DockPanel\bin\php` base directory.
pub fn dockpanel_php_dir() -> Option<PathBuf> {
    std::env::var("USERPROFILE").ok().map(|u| {
        PathBuf::from(u).join("DockPanel").join("bin").join("php")
    })
}

/// Collects `(dir_name, php_exe_path)` across versioned PHP install directories
/// (`%USERPROFILE%\DockPanel\bin\php` and `C:\wamp64\bin\php`), sorted descending by version name.
fn collect_versioned_php_dirs() -> Vec<(String, PathBuf)> {
    let mut dirs = Vec::new();
    let mut search_roots = Vec::new();
    if let Some(dp_dir) = dockpanel_php_dir() {
        search_roots.push(dp_dir);
    }
    search_roots.push(PathBuf::from(r"C:\wamp64\bin\php"));

    for root in search_roots {
        if root.is_dir() {
            if let Ok(entries) = fs::read_dir(&root) {
                for entry in entries.flatten() {
                    let path = entry.path();
                    if path.is_dir() {
                        let exe = path.join("php.exe");
                        if exe.is_file() {
                            let name = path
                                .file_name()
                                .and_then(|n| n.to_str())
                                .unwrap_or("")
                                .to_string();
                            dirs.push((name, exe));
                        }
                    }
                }
            }
        }
    }
    dirs.sort_by(|a, b| b.0.cmp(&a.0));
    dirs
}

/// Returns candidate standalone `php.exe` paths from PATH and common Windows locations.
fn collect_standalone_php_paths() -> Vec<PathBuf> {
    let mut paths = Vec::new();
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x08000000;
        if let Ok(output) = std::process::Command::new("where.exe")
            .arg("php")
            .creation_flags(CREATE_NO_WINDOW)
            .output()
        {
            if output.status.success() {
                for line in String::from_utf8_lossy(&output.stdout).lines() {
                    let p = PathBuf::from(line.trim());
                    if p.is_file() {
                        paths.push(p);
                    }
                }
            }
        }
    }
    if let Ok(path_var) = std::env::var("PATH") {
        for dir in std::env::split_paths(&path_var) {
            let candidate = dir.join("php.exe");
            if candidate.is_file() && !paths.contains(&candidate) {
                paths.push(candidate);
            }
        }
    }
    for common in [r"C:\php\php.exe", r"C:\tools\php\php.exe"] {
        let p = PathBuf::from(common);
        if p.is_file() && !paths.contains(&p) {
            paths.push(p);
        }
    }
    if let Some(dp_dir) = dockpanel_php_dir() {
        let p = dp_dir.join("php.exe");
        if p.is_file() && !paths.contains(&p) {
            paths.push(p);
        }
    }
    paths
}

/// Checks if a given directory name or path matches a major.minor version like `"8.3"`.
fn dir_matches_version(dir_name: &str, version: &str) -> bool {
    let lower = dir_name.to_lowercase();
    let v = version.trim().to_lowercase();
    lower.contains(&format!("php{v}")) || lower.contains(&format!("php-{v}")) || lower == v
}

/// Finds a PHP binary matching the exact requested `major.minor` version (without falling back to other versions).
pub fn find_php_binary_exact(version: &str) -> Option<PathBuf> {
    let dirs = collect_versioned_php_dirs();
    if let Some((_, exe)) = dirs.iter().find(|(name, _)| dir_matches_version(name, version)) {
        return Some(exe.clone());
    }

    // Check standalone paths by inspecting parent dir name or invoking `php -v`
    for exe in collect_standalone_php_paths() {
        if let Some(parent) = exe.parent().and_then(|p| p.file_name()).and_then(|n| n.to_str()) {
            if dir_matches_version(parent, version) {
                return Some(exe);
            }
        }
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x08000000;
            if let Ok(out) = std::process::Command::new(&exe)
                .args(["-r", "echo PHP_MAJOR_VERSION.'.'.PHP_MINOR_VERSION;"])
                .creation_flags(CREATE_NO_WINDOW)
                .output()
            {
                if out.status.success() {
                    let ver = String::from_utf8_lossy(&out.stdout).trim().to_string();
                    if ver == version.trim() {
                        return Some(exe);
                    }
                }
            }
        }
    }
    None
}

/// Finds a PHP CLI binary on the host machine, preferring `version_hint` if installed,
/// and falling back to any installed PHP version on Windows.
pub fn find_php_binary(version_hint: Option<&str>) -> Option<PathBuf> {
    if let Some(hint) = version_hint {
        if let Some(exact) = find_php_binary_exact(hint) {
            return Some(exact);
        }
    }

    let dirs = collect_versioned_php_dirs();
    if let Some((_, exe)) = dirs.first() {
        return Some(exe.clone());
    }

    collect_standalone_php_paths().into_iter().next()
}

impl AppState {
    pub fn new() -> Self {
        let app_data = std::env::var("APPDATA")
            .unwrap_or_else(|_| std::env::var("USERPROFILE").unwrap_or_else(|_| ".".into()));
        let config_dir = PathBuf::from(app_data).join("DockPanel");
        let user_profile = std::env::var("USERPROFILE").unwrap_or_else(|_| ".".into());
        let sites_dir = PathBuf::from(user_profile).join("DockPanel").join("sites");

        Self::new_with_dirs(config_dir, sites_dir)
    }

    pub fn new_with_dirs(config_dir: PathBuf, sites_dir: PathBuf) -> Self {
        let _ = fs::create_dir_all(&config_dir);
        let _ = fs::create_dir_all(&sites_dir);

        let data = Self::load_data(&config_dir);
        let router_config = Self::load_router_config(&config_dir);
        Self {
            data: Arc::new(Mutex::new(data)),
            router_config: Arc::new(Mutex::new(router_config)),
            worker_ports: Arc::new(Mutex::new(HashMap::new())),
            worker_processes: Arc::new(Mutex::new(HashMap::new())),
            router_handle: Arc::new(tokio::sync::Mutex::new(None)),
            php_installs: Arc::new(Mutex::new(HashMap::new())),
            config_dir,
            sites_dir,
        }
    }

    fn load_data(config_dir: &Path) -> AppData {
        let file = config_dir.join("sites.json");
        if let Ok(content) = fs::read_to_string(file) {
            serde_json::from_str(&content).unwrap_or_default()
        } else {
            AppData::default()
        }
    }

    fn save_data(&self) {
        let file = self.config_dir.join("sites.json");
        if let Ok(data) = self.data.lock() {
            if let Ok(json) = serde_json::to_string_pretty(&*data) {
                let _ = fs::write(file, json);
            }
        }
    }

    fn load_router_config(config_dir: &Path) -> RouterConfig {
        let file = config_dir.join("router_config.json");
        if let Ok(content) = fs::read_to_string(file) {
            serde_json::from_str(&content).unwrap_or_default()
        } else {
            RouterConfig::default()
        }
    }

    #[allow(dead_code)]
    fn save_router_config(&self) {
        let file = self.config_dir.join("router_config.json");
        if let Ok(cfg) = self.router_config.lock() {
            if let Ok(json) = serde_json::to_string_pretty(&*cfg) {
                let _ = fs::write(file, json);
            }
        }
    }

    #[allow(dead_code)]
    pub fn get_router_config(&self) -> RouterConfig {
        self.router_config.lock().unwrap().clone()
    }

    #[allow(dead_code)]
    pub fn set_router_port(&self, port: u16) {
        {
            let mut cfg = self.router_config.lock().unwrap();
            cfg.active_port = port;
        }
        self.save_router_config();
    }

    pub fn list_sites(&self) -> Vec<LocalSite> {
        self.data.lock().unwrap().sites.clone()
    }

    pub fn get_site(&self, id: &str) -> Option<LocalSite> {
        let clean = id.trim().to_lowercase();
        self.data
            .lock()
            .unwrap()
            .sites
            .iter()
            .find(|s| s.id == id || s.domain.to_lowercase() == clean)
            .cloned()
    }

    pub fn update_site_status(&self, id_or_domain: &str, status: &str) -> bool {
        let clean = id_or_domain.trim().to_lowercase();
        let mut data = self.data.lock().unwrap();
        if let Some(s) = data.sites.iter_mut().find(|s| s.id == id_or_domain || s.domain.to_lowercase() == clean) {
            s.status = status.to_string();
            drop(data);
            self.save_data();
            true
        } else {
            false
        }
    }

    pub fn update_site_preset(&self, id_or_domain: &str, preset: &str) -> bool {
        let clean = id_or_domain.trim().to_lowercase();
        let mut data = match self.data.lock() {
            Ok(d) => d,
            Err(_) => return false,
        };
        if let Some(s) = data.sites.iter_mut().find(|s| s.id == id_or_domain || s.domain.to_lowercase() == clean) {
            s.php_preset = Some(preset.to_string());
            s.runtime = "php".to_string();
            drop(data);
            self.save_data();
            true
        } else {
            false
        }
    }

    pub fn update_site_php_version(&self, id_or_domain: &str, php_version: &str) -> Result<LocalSite, String> {
        let clean = id_or_domain.trim().to_lowercase();
        let site = {
            let mut data = self.data.lock().map_err(|_| "State lock failed".to_string())?;
            if let Some(s) = data.sites.iter_mut().find(|s| s.id == id_or_domain || s.domain.to_lowercase() == clean) {
                s.php_version = php_version.trim().to_string();
                s.runtime = "php".to_string();
                s.clone()
            } else {
                return Err(format!("Site '{}' not found", id_or_domain));
            }
        };
        self.save_data();
        if site.status == "active" {
            self.spawn_php_worker(&site.domain, &site.document_root, Some(&site.php_version));
        }
        Ok(site)
    }

    pub fn update_site_runtime(
        &self,
        id_or_domain: &str,
        runtime: &str,
        php_version: Option<&str>,
    ) -> Result<LocalSite, String> {
        let clean = id_or_domain.trim().to_lowercase();
        let site = {
            let mut data = self.data.lock().map_err(|_| "State lock failed".to_string())?;
            if let Some(s) = data.sites.iter_mut().find(|s| s.id == id_or_domain || s.domain.to_lowercase() == clean) {
                s.runtime = runtime.trim().to_string();
                if let Some(ver) = php_version {
                    s.php_version = ver.trim().to_string();
                }
                s.clone()
            } else {
                return Err(format!("Site '{}' not found", id_or_domain));
            }
        };
        self.save_data();
        if site.status == "active" && site.runtime == "php" {
            self.spawn_php_worker(&site.domain, &site.document_root, Some(&site.php_version));
        }
        Ok(site)
    }

    pub fn list_php_versions(&self) -> Vec<serde_json::Value> {
        const STANDARD_VERSIONS: &[&str] = &["8.6", "8.5", "8.4", "8.3", "8.2", "8.1", "8.0", "7.4"];
        STANDARD_VERSIONS
            .iter()
            .map(|&ver| {
                let exe = find_php_binary_exact(ver);
                let installed = exe.is_some();
                serde_json::json!({
                    "version": ver,
                    "installed": installed,
                    // Desktop runs PHP-CGI workers on demand per site, so an installed binary is immediately ready.
                    "fpm_running": installed,
                    "socket": exe.as_ref().map(|p| p.to_string_lossy().to_string()).unwrap_or_default(),
                    "default": ver == "8.3",
                })
            })
            .collect()
    }

    pub fn push_php_install_step(&self, install_id: &str, step: serde_json::Value, done: bool) {
        if let Ok(mut jobs) = self.php_installs.lock() {
            let job = jobs.entry(install_id.to_string()).or_default();
            job.steps.push(step);
            if done {
                job.done = true;
            }
        }
    }

    pub fn get_site_worker_port(&self, domain: &str) -> Option<u16> {
        let clean = domain.trim().to_lowercase();
        self.worker_ports.lock().unwrap().get(&clean).copied()
    }

    pub fn set_site_worker_port(&self, domain: &str, port: u16) {
        let clean = domain.trim().to_lowercase();
        self.worker_ports.lock().unwrap().insert(clean, port);
    }

    pub fn remove_site_worker_port(&self, domain: &str) {
        let clean = domain.trim().to_lowercase();
        self.worker_ports.lock().unwrap().remove(&clean);
    }

    pub fn create_site(&self, domain: &str, php_version: Option<&str>) -> Result<LocalSite, String> {
        let validated = crate::hosts_manager::validate_domain(domain)?;
        let clean_domain = validated.to_lowercase();
        let id = uuid::Uuid::new_v4().to_string();
        let site_dir = self.sites_dir.join(&clean_domain);
        if site_dir.parent() != Some(self.sites_dir.as_path()) {
            return Err("Invalid domain path".to_string());
        }
        let _ = fs::create_dir_all(&site_dir);

        // Write starter index.html
        let index_file = site_dir.join("index.html");
        if !index_file.exists() {
            let html = format!(
                r#"<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="UTF-8">
    <title>{domain} - DockPanel Desktop</title>
    <style>
        body {{ font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, sans-serif; background: #0f172a; color: #f8fafc; display: flex; align-items: center; justify-content: center; height: 100vh; margin: 0; }}
        .card {{ background: #1e293b; padding: 2.5rem; border-radius: 1rem; border: 1px solid #334155; text-align: center; max-width: 500px; box-shadow: 0 10px 25px rgba(0,0,0,0.5); }}
        h1 {{ color: #38bdf8; margin-top: 0; }}
        p {{ color: #94a3b8; font-size: 1.1rem; line-height: 1.6; }}
        .badge {{ background: #0284c7; color: white; padding: 0.35rem 0.75rem; border-radius: 9999px; font-size: 0.875rem; font-weight: 600; display: inline-block; margin-top: 1rem; }}
    </style>
</head>
<body>
    <div class="card">
        <h1>{domain}</h1>
        <p>Your local site is running natively on Windows via <strong>DockPanel Desktop</strong>.</p>
        <span class="badge">Local Development Active</span>
    </div>
</body>
</html>"#,
                domain = clean_domain
            );
            let _ = fs::write(index_file, html);
        }

        let site = LocalSite {
            id,
            domain: clean_domain.clone(),
            runtime: "php".to_string(),
            php_version: php_version.unwrap_or("8.3").to_string(),
            document_root: site_dir.to_string_lossy().to_string(),
            status: "active".to_string(),
            created_at: chrono::Utc::now().to_rfc3339(),
            ssl_enabled: true,
            php_preset: None,
            enabled: true,
            max_upload_mb: 64,
            php_memory_mb: 256,
            php_max_workers: 4,
        };

        {
            let mut data = self.data.lock().unwrap();
            data.sites.retain(|s| s.domain != clean_domain);
            data.sites.push(site.clone());
        }
        self.save_data();

        // Register domain in Windows hosts file (best-effort; never fail site creation on permission errors)
        let _ = crate::hosts_manager::sync_domain_to_hosts(&clean_domain);

        Ok(site)
    }

    pub fn delete_site(&self, id: &str) -> bool {
        let domain = {
            let mut data = self.data.lock().unwrap();
            let pos = data.sites.iter().position(|s| s.id == id || s.domain == id);
            if let Some(idx) = pos {
                let site = data.sites.remove(idx);
                Some(site.domain)
            } else {
                None
            }
        };

        if let Some(dom) = domain {
            let clean = dom.trim().to_lowercase();
            if let Ok(mut procs) = self.worker_processes.lock() {
                if let Some(mut child) = procs.remove(&clean) {
                    let _ = child.kill();
                    let _ = child.wait();
                }
            }
            self.remove_site_worker_port(&clean);
            self.save_data();
            let _ = crate::hosts_manager::remove_domain_from_hosts(&dom);
            true
        } else {
            false
        }
    }

    pub fn spawn_php_worker(
        &self,
        domain: &str,
        doc_root: &str,
        php_version: Option<&str>,
    ) -> Option<u16> {
        let clean = domain.trim().to_lowercase();

        // Check if existing worker is still alive
        {
            let mut procs = self.worker_processes.lock().unwrap();
            if let Some(child) = procs.get_mut(&clean) {
                match child.try_wait() {
                    Ok(None) => {
                        // Still running
                        if let Some(port) = self.get_site_worker_port(&clean) {
                            return Some(port);
                        } else {
                            // Inconsistent state: child running without worker port, kill it
                            let _ = child.kill();
                            let _ = child.wait();
                            procs.remove(&clean);
                        }
                    }
                    _ => {
                        procs.remove(&clean);
                    }
                }
            }
        }

        // If an entry still exists in worker_processes for the domain, kill and wait on it before spawning
        {
            let mut procs = self.worker_processes.lock().unwrap();
            if let Some(mut old_child) = procs.remove(&clean) {
                let _ = old_child.kill();
                let _ = old_child.wait();
            }
        }

        let php_exe = find_php_binary(php_version)?;
        let worker_port = find_available_loopback_port()?;

        #[cfg(windows)]
        use std::os::windows::process::CommandExt;
        #[cfg(windows)]
        const CREATE_NO_WINDOW: u32 = 0x08000000;

        let mut cmd = std::process::Command::new(&php_exe);
        cmd.args(["-S", &format!("127.0.0.1:{}", worker_port), "-t", doc_root])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null());

        #[cfg(windows)]
        cmd.creation_flags(CREATE_NO_WINDOW);

        match cmd.spawn() {
            Ok(child) => {
                let mut procs = self.worker_processes.lock().unwrap();
                if let Some(mut old_child) = procs.insert(clean.clone(), child) {
                    let _ = old_child.kill();
                    let _ = old_child.wait();
                }
                self.set_site_worker_port(&clean, worker_port);
                Some(worker_port)
            }
            Err(e) => {
                eprintln!("Failed to spawn PHP worker for {}: {}", clean, e);
                None
            }
        }
    }

    pub fn start_site(&self, id_or_domain: &str) -> Result<LocalSite, String> {
        let clean = id_or_domain.trim().to_lowercase();
        let site = {
            let data = self.data.lock().unwrap();
            data.sites
                .iter()
                .find(|s| s.id == id_or_domain || s.domain.to_lowercase() == clean)
                .cloned()
        };

        let site = match site {
            Some(s) => s,
            None => return Err(format!("Site '{}' not found", id_or_domain)),
        };

        // If site uses PHP, spawn or ensure worker is running
        self.spawn_php_worker(&site.domain, &site.document_root, Some(&site.php_version));

        // Update status to active
        {
            let mut data = self.data.lock().unwrap();
            if let Some(s) = data.sites.iter_mut().find(|s| s.id == site.id) {
                s.status = "active".to_string();
            }
        }
        self.save_data();

        // Sync domain in Windows hosts file (best-effort; never fail site startup on permission errors)
        let _ = crate::hosts_manager::sync_domain_to_hosts(&site.domain);

        self.get_site(&site.id)
            .ok_or_else(|| "Failed to retrieve updated site".to_string())
    }

    pub fn stop_site(&self, id_or_domain: &str) -> Result<LocalSite, String> {
        let clean = id_or_domain.trim().to_lowercase();
        let site = {
            let data = self.data.lock().unwrap();
            data.sites
                .iter()
                .find(|s| s.id == id_or_domain || s.domain.to_lowercase() == clean)
                .cloned()
        };

        let site = match site {
            Some(s) => s,
            None => return Err(format!("Site '{}' not found", id_or_domain)),
        };

        // Terminate child worker process if exists
        {
            let mut procs = self.worker_processes.lock().unwrap();
            if let Some(mut child) = procs.remove(&site.domain.to_lowercase()) {
                let _ = child.kill();
                let _ = child.wait();
            }
        }
        self.remove_site_worker_port(&site.domain);

        // Update status to stopped
        {
            let mut data = self.data.lock().unwrap();
            if let Some(s) = data.sites.iter_mut().find(|s| s.id == site.id) {
                s.status = "stopped".to_string();
            }
        }
        self.save_data();

        self.get_site(&site.id)
            .ok_or_else(|| "Failed to retrieve updated site".to_string())
    }

    pub fn reconcile_active_sites(&self) {
        let active_sites: Vec<LocalSite> = {
            let data = self.data.lock().unwrap();
            data.sites
                .iter()
                .filter(|s| s.status.eq_ignore_ascii_case("active"))
                .cloned()
                .collect()
        };

        for site in active_sites {
            self.spawn_php_worker(&site.domain, &site.document_root, Some(&site.php_version));
            let _ = crate::hosts_manager::sync_domain_to_hosts(&site.domain);
        }
    }

    pub fn stop_all_workers(&self) {
        if let Ok(mut procs) = self.worker_processes.lock() {
            for (_dom, mut child) in procs.drain() {
                let _ = child.kill();
                let _ = child.wait();
            }
        }
        if let Ok(mut ports) = self.worker_ports.lock() {
            ports.clear();
        }
    }

    pub async fn restart_site_router(&self, port: u16) -> bool {
        let mut handle_guard = self.router_handle.lock().await;
        if let Some(old_handle) = handle_guard.take() {
            old_handle.abort();
            let _ = old_handle.await;
        }
        let (handle, rx) = crate::router::start_site_router_checked(Arc::new(self.clone()), port);
        let bound = rx.await.unwrap_or(false);
        if bound {
            *handle_guard = Some(handle);
        } else {
            handle.abort();
        }
        bound
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_router_config_state_lifecycle() {
        let temp_dir = std::env::temp_dir().join(format!("dockpanel_test_{}", uuid::Uuid::new_v4()));
        let config_dir = temp_dir.join("config");
        let sites_dir = temp_dir.join("sites");
        let state = AppState::new_with_dirs(config_dir.clone(), sites_dir);

        // Default config
        let cfg = state.get_router_config();
        assert_eq!(cfg.preferred_port, 80);
        assert_eq!(cfg.active_port, 80);

        // Mutate port
        state.set_router_port(8080);
        let updated = state.get_router_config();
        assert_eq!(updated.preferred_port, 80);
        assert_eq!(updated.active_port, 8080);

        // Check persisted file
        let file = config_dir.join("router_config.json");
        assert!(file.exists());
        let content = fs::read_to_string(file).unwrap();
        let loaded: RouterConfig = serde_json::from_str(&content).unwrap();
        assert_eq!(loaded.active_port, 8080);

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_worker_port_state_lifecycle() {
        let temp_dir = std::env::temp_dir().join(format!("dockpanel_worker_test_{}", uuid::Uuid::new_v4()));
        let config_dir = temp_dir.join("config");
        let sites_dir = temp_dir.join("sites");
        let state = AppState::new_with_dirs(config_dir, sites_dir);

        assert_eq!(state.get_site_worker_port("mysite.test"), None);

        state.set_site_worker_port("mysite.test", 9001);
        assert_eq!(state.get_site_worker_port("mysite.test"), Some(9001));
        // Case-insensitive check
        assert_eq!(state.get_site_worker_port("MYSITE.TEST"), Some(9001));

        state.remove_site_worker_port("mysite.test");
        assert_eq!(state.get_site_worker_port("mysite.test"), None);

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_site_start_stop_lifecycle() {
        let temp_dir = std::env::temp_dir().join(format!("dockpanel_lifecycle_test_{}", uuid::Uuid::new_v4()));
        let config_dir = temp_dir.join("config");
        let sites_dir = temp_dir.join("sites");
        let state = AppState::new_with_dirs(config_dir, sites_dir);

        let site = state.create_site("lifecycle-test.local", Some("8.3")).unwrap();
        assert_eq!(site.status, "active");

        let stopped = state.stop_site(&site.id).expect("Should stop site");
        assert_eq!(stopped.status, "stopped");
        assert_eq!(state.get_site(&site.id).unwrap().status, "stopped");

        let started = state.start_site(&site.id).expect("Should start site");
        assert_eq!(started.status, "active");
        assert_eq!(state.get_site(&site.id).unwrap().status, "active");

        let _ = state.stop_site(&site.id);
        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_find_php_binary() {
        let php = find_php_binary(Some("8.3"));
        if let Some(path) = php {
            assert!(path.exists());
            assert!(path.to_string_lossy().to_lowercase().contains("php"));
        }
    }

    #[test]
    fn test_reconcile_active_sites() {
        let temp_dir = std::env::temp_dir().join(format!("dockpanel_reconcile_test_{}", uuid::Uuid::new_v4()));
        let config_dir = temp_dir.join("config");
        let sites_dir = temp_dir.join("sites");
        let state = AppState::new_with_dirs(config_dir, sites_dir);

        let site = state.create_site("reconcile-test.local", Some("8.3")).unwrap();
        assert_eq!(site.status, "active");

        // Stop worker manually without changing status
        if let Ok(mut procs) = state.worker_processes.lock() {
            if let Some(mut child) = procs.remove("reconcile-test.local") {
                let _ = child.kill();
                let _ = child.wait();
            }
        }
        state.remove_site_worker_port("reconcile-test.local");
        assert_eq!(state.get_site_worker_port("reconcile-test.local"), None);

        // Reconcile should restart the worker if PHP is present
        state.reconcile_active_sites();
        if find_php_binary(Some("8.3")).is_some() {
            assert!(state.get_site_worker_port("reconcile-test.local").is_some());
        }

        let _ = state.stop_site(&site.id);
        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_site_delete_lifecycle() {
        let temp_dir = std::env::temp_dir().join(format!("dockpanel_delete_test_{}", uuid::Uuid::new_v4()));
        let config_dir = temp_dir.join("config");
        let sites_dir = temp_dir.join("sites");
        let state = AppState::new_with_dirs(config_dir, sites_dir);

        let site = state.create_site("delete-test.local", Some("8.3")).unwrap();
        assert!(state.get_site(&site.id).is_some());

        assert!(state.delete_site(&site.id));
        assert!(state.get_site(&site.id).is_none());
        assert!(!state.delete_site(&site.id));

        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_create_site_rejects_invalid_and_traversal_domains() {
        let temp_dir = std::env::temp_dir().join(format!("dockpanel_domain_val_{}", uuid::Uuid::new_v4()));
        let config_dir = temp_dir.join("config");
        let sites_dir = temp_dir.join("sites");
        let state = AppState::new_with_dirs(config_dir, sites_dir.clone());

        for bad in ["../escape", "..\\escape", "foo/bar", "foo..bar", "", "bad domain.test", "-bad.test"] {
            assert!(state.create_site(bad, Some("8.3")).is_err(), "Expected error for {}", bad);
        }
        assert!(state.list_sites().is_empty());
        assert!(!temp_dir.join("escape").exists());
        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[tokio::test]
    async fn test_restart_site_router_returns_bind_status_and_releases_socket() {
        let temp_dir = std::env::temp_dir().join(format!("dockpanel_router_restart_{}", uuid::Uuid::new_v4()));
        let config_dir = temp_dir.join("config");
        let sites_dir = temp_dir.join("sites");
        let state = AppState::new_with_dirs(config_dir, sites_dir);

        let p1 = find_available_loopback_port().unwrap();
        assert!(state.restart_site_router(p1).await);

        // Restarting onto the same port should cleanly release and rebind
        assert!(state.restart_site_router(p1).await);

        // Attempting to restart onto an occupied port should return false
        let occ = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let occ_port = occ.local_addr().unwrap().port();
        assert!(!state.restart_site_router(occ_port).await);
        drop(occ);

        if let Some(h) = state.router_handle.lock().await.take() {
            h.abort();
        }
        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_local_site_defaults_runtime_and_enabled() {
        let legacy_json = r#"{"id":"1","domain":"domain.local","php_version":"8.3","document_root":"C:\\sites\\domain.local","status":"active","created_at":"2026-01-01T00:00:00Z","ssl_enabled":true}"#;
        let site: LocalSite = serde_json::from_str(legacy_json).unwrap();
        assert_eq!(site.runtime, "php");
        assert!(site.enabled);
        assert_eq!(site.max_upload_mb, 64);
        assert_eq!(site.php_memory_mb, 256);
        assert_eq!(site.php_max_workers, 4);
    }
}

