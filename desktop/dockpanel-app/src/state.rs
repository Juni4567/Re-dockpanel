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
    #[serde(default)]
    pub app_command: Option<String>,
    #[serde(default)]
    pub proxy_port: Option<u16>,
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LocalNotification {
    pub id: String,
    pub title: String,
    pub message: String,
    pub severity: String,
    pub read: bool,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LocalActivity {
    pub id: String,
    pub action: String,
    pub target_type: String,
    pub target_name: String,
    pub details: String,
    pub user_email: String,
    pub created_at: String,
}

#[cfg(windows)]
#[derive(Debug)]
pub struct WorkerJobGuard(isize);

#[cfg(windows)]
impl Drop for WorkerJobGuard {
    fn drop(&mut self) {
        if self.0 != 0 {
            #[link(name = "kernel32")]
            extern "system" {
                fn CloseHandle(h: isize) -> i32;
            }
            unsafe {
                CloseHandle(self.0);
            }
        }
    }
}

#[cfg(windows)]
fn create_kill_on_close_job() -> Option<Arc<WorkerJobGuard>> {
    use std::ffi::c_void;

    #[repr(C)]
    struct BasicLimitInfo {
        per_process_user_time_limit: i64,
        per_job_user_time_limit: i64,
        limit_flags: u32,
        minimum_working_set_size: usize,
        maximum_working_set_size: usize,
        active_process_limit: u32,
        affinity: usize,
        priority_class: u32,
        scheduling_class: u32,
    }

    #[repr(C)]
    struct IoCounters {
        vals: [u64; 6],
    }

    #[repr(C)]
    struct ExtendedLimitInfo {
        basic: BasicLimitInfo,
        io: IoCounters,
        process_memory_limit: usize,
        job_memory_limit: usize,
        peak_process_memory_used: usize,
        peak_job_memory_used: usize,
    }

    #[link(name = "kernel32")]
    extern "system" {
        fn CreateJobObjectW(attrs: *const c_void, name: *const u16) -> isize;
        fn SetInformationJobObject(job: isize, info_class: u32, info: *const c_void, len: u32) -> i32;
    }

    const JOB_OBJECT_EXTENDED_LIMIT_INFORMATION: u32 = 9;
    const JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE: u32 = 0x2000;

    unsafe {
        let job = CreateJobObjectW(std::ptr::null(), std::ptr::null());
        if job == 0 {
            return None;
        }
        let mut info: ExtendedLimitInfo = std::mem::zeroed();
        info.basic.limit_flags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        let ok = SetInformationJobObject(
            job,
            JOB_OBJECT_EXTENDED_LIMIT_INFORMATION,
            &info as *const _ as *const c_void,
            std::mem::size_of::<ExtendedLimitInfo>() as u32,
        );
        if ok == 0 {
            Some(Arc::new(WorkerJobGuard(job)))
        } else {
            Some(Arc::new(WorkerJobGuard(job)))
        }
    }
}

#[cfg(windows)]
fn assign_child_to_job(job: &Option<Arc<WorkerJobGuard>>, child: &std::process::Child) {
    use std::os::windows::io::AsRawHandle;
    #[link(name = "kernel32")]
    extern "system" {
        fn AssignProcessToJobObject(job: isize, proc_handle: isize) -> i32;
    }
    if let Some(j) = job {
        if j.0 != 0 {
            unsafe {
                AssignProcessToJobObject(j.0, child.as_raw_handle() as isize);
            }
        }
    }
}

#[derive(Clone)]
pub struct AppState {
    pub data: Arc<Mutex<AppData>>,
    pub router_config: Arc<Mutex<RouterConfig>>,
    pub worker_ports: Arc<Mutex<HashMap<String, u16>>>,
    pub worker_processes: Arc<Mutex<HashMap<String, std::process::Child>>>,
    pub router_handle: Arc<tokio::sync::Mutex<Option<tokio::task::JoinHandle<()>>>>,
    pub ssl_router_handle: Arc<tokio::sync::Mutex<Option<tokio::task::JoinHandle<()>>>>,
    pub ssl_port: Arc<Mutex<Option<u16>>>,
    pub cert_resolver: Arc<crate::ssl::DynamicCertResolver>,
    pub mariadb_process: Arc<Mutex<Option<(std::process::Child, u16)>>>,
    pub php_installs: Arc<Mutex<HashMap<String, PhpInstallJob>>>,
    pub notifications: Arc<Mutex<Vec<LocalNotification>>>,
    pub activity_log: Arc<Mutex<Vec<LocalActivity>>>,
    pub notif_tx: tokio::sync::broadcast::Sender<serde_json::Value>,
    #[cfg(windows)]
    pub worker_job: Option<Arc<WorkerJobGuard>>,
    pub started_at: std::time::Instant,
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
/// Locate a locally installed Node.js binary on PATH or common Windows install locations.
pub fn find_node_binary() -> Option<PathBuf> {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x08000000;
        if let Ok(out) = std::process::Command::new("where.exe")
            .arg("node")
            .creation_flags(CREATE_NO_WINDOW)
            .output()
        {
            if out.status.success() {
                for line in String::from_utf8_lossy(&out.stdout).lines() {
                    let p = PathBuf::from(line.trim());
                    if p.is_file() {
                        return Some(p);
                    }
                }
            }
        }
        let mut candidates = vec![
            PathBuf::from(r"C:\Program Files\nodejs\node.exe"),
            PathBuf::from(r"C:\Program Files (x86)\nodejs\node.exe"),
        ];
        if let Ok(local_app) = std::env::var("LOCALAPPDATA") {
            candidates.push(PathBuf::from(&local_app).join("Programs").join("nodejs").join("node.exe"));
            candidates.push(PathBuf::from(&local_app).join("fnm_multishells").join("node.exe"));
        }
        if let Ok(user_prof) = std::env::var("USERPROFILE") {
            candidates.push(PathBuf::from(&user_prof).join(".volta").join("bin").join("node.exe"));
        }
        for c in candidates {
            if c.is_file() {
                return Some(c);
            }
        }
        None
    }
    #[cfg(not(windows))]
    {
        if let Ok(out) = std::process::Command::new("which").arg("node").output() {
            if out.status.success() {
                let p = PathBuf::from(String::from_utf8_lossy(&out.stdout).trim());
                if p.is_file() {
                    return Some(p);
                }
            }
        }
        None
    }
}

/// Returns the installed Node.js version string (e.g. "v22.14.0") if available.
pub fn get_node_version() -> Option<String> {
    let node = find_node_binary()?;
    let mut cmd = std::process::Command::new(node);
    cmd.arg("--version");
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x08000000);
    }
    let out = cmd.output().ok()?;
    if out.status.success() {
        let v = String::from_utf8_lossy(&out.stdout).trim().to_string();
        if !v.is_empty() {
            return Some(v);
        }
    }
    None
}

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
        let (notif_tx, _) = tokio::sync::broadcast::channel(64);
        let cert_resolver = Arc::new(crate::ssl::DynamicCertResolver::new());
        cert_resolver.load_existing_certs(&config_dir.join("certs"));
        Self {
            data: Arc::new(Mutex::new(data)),
            router_config: Arc::new(Mutex::new(router_config)),
            worker_ports: Arc::new(Mutex::new(HashMap::new())),
            worker_processes: Arc::new(Mutex::new(HashMap::new())),
            router_handle: Arc::new(tokio::sync::Mutex::new(None)),
            ssl_router_handle: Arc::new(tokio::sync::Mutex::new(None)),
            ssl_port: Arc::new(Mutex::new(None)),
            cert_resolver,
            mariadb_process: Arc::new(Mutex::new(None)),
            php_installs: Arc::new(Mutex::new(HashMap::new())),
            notifications: Arc::new(Mutex::new(Vec::new())),
            activity_log: Arc::new(Mutex::new(Vec::new())),
            notif_tx,
            #[cfg(windows)]
            worker_job: create_kill_on_close_job(),
            started_at: std::time::Instant::now(),
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
        // Stop any existing worker first so switching to static or between runtimes is clean
        {
            let mut procs = self.worker_processes.lock().unwrap();
            if let Some(mut child) = procs.remove(&site.domain.to_lowercase()) {
                let _ = child.kill();
                let _ = child.wait();
            }
        }
        self.remove_site_worker_port(&site.domain);
        if site.status == "active" {
            self.spawn_site_worker(&site);
        }
        Ok(site)
    }

    pub fn list_php_versions(&self) -> Vec<serde_json::Value> {
        const STANDARD_VERSIONS: &[&str] = &["8.6", "8.5", "8.4", "8.3", "8.2", "8.1", "8.0", "7.4"];
        let highest_installed = STANDARD_VERSIONS
            .iter()
            .copied()
            .find(|v| find_php_binary_exact(v).is_some())
            .unwrap_or("8.3");
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
                    "default": ver == highest_installed,
                })
            })
            .collect()
    }

    pub fn push_php_install_step(&self, install_id: &str, step: serde_json::Value, done: bool) {
        if let Ok(mut jobs) = self.php_installs.lock() {
            let job = jobs.entry(install_id.to_string()).or_default();
            if let Some(step_key) = step.get("step").and_then(|v| v.as_str()) {
                if let Some(existing) = job.steps.iter_mut().find(|s| {
                    s.get("step").and_then(|v| v.as_str()) == Some(step_key)
                }) {
                    *existing = step;
                } else {
                    job.steps.push(step);
                }
            } else {
                job.steps.push(step);
            }
            if done {
                job.done = true;
            }
        }
    }

    pub fn push_install_progress(
        &self,
        install_id: &str,
        step: &str,
        label: &str,
        status: &str,
        progress: Option<u8>,
        message: Option<&str>,
        done: bool,
    ) {
        let mut obj = serde_json::json!({
            "step": step,
            "label": label,
            "status": status,
        });
        if let Some(p) = progress {
            obj["progress"] = serde_json::json!(p);
        }
        if let Some(m) = message {
            obj["message"] = serde_json::json!(m);
        }
        self.push_php_install_step(install_id, obj, done);
    }

    pub fn certs_dir(&self) -> PathBuf {
        self.config_dir.join("certs")
    }

    pub fn get_default_sites_dir(&self) -> PathBuf {
        let path = self.config_dir.join("desktop_settings.json");
        if let Ok(content) = fs::read_to_string(path) {
            if let Ok(saved) = serde_json::from_str::<HashMap<String, String>>(&content) {
                if let Some(custom) = saved.get("default_sites_dir").map(|s| s.trim()).filter(|s| !s.is_empty()) {
                    let candidate = PathBuf::from(custom);
                    if candidate.is_absolute() && candidate.parent().is_some() {
                        return candidate;
                    }
                }
            }
        }
        self.sites_dir.clone()
    }

    pub fn site_dir(&self, site: &LocalSite) -> PathBuf {
        let trimmed = site.document_root.trim();
        if !trimmed.is_empty() {
            PathBuf::from(trimmed)
        } else {
            self.get_default_sites_dir().join(&site.domain)
        }
    }

    pub fn ensure_site_ssl(&self, domain: &str) -> Result<(), String> {
        let clean = domain.trim().to_lowercase();
        let (cert, key) = crate::ssl::ensure_site_certificate(&self.certs_dir(), &clean)?;
        let ck = crate::ssl::load_certified_key(&cert, &key)?;
        self.cert_resolver.insert_key(&clean, ck);
        Ok(())
    }

    pub fn get_ssl_port(&self) -> Option<u16> {
        *self.ssl_port.lock().unwrap()
    }

    pub fn set_ssl_port(&self, port: Option<u16>) {
        *self.ssl_port.lock().unwrap() = port;
    }

    fn site_security_path(&self, domain: &str) -> PathBuf {
        let clean = domain.trim().to_lowercase();
        self.config_dir.join("site_security").join(format!("{}.json", clean))
    }

    /// Returns `Some((username, sha256_password_hex))` when site password protection is active.
    pub fn get_site_pass_protect(&self, domain: &str) -> Option<(String, String)> {
        let path = self.site_security_path(domain);
        let content = fs::read_to_string(path).ok()?;
        let val: serde_json::Value = serde_json::from_str(&content).ok()?;
        if val.get("enabled").and_then(|v| v.as_bool()) != Some(true) {
            return None;
        }
        let user = val.get("user").and_then(|v| v.as_str()).unwrap_or("admin").to_string();
        let hash = val.get("password_hash").and_then(|v| v.as_str())?.to_string();
        if hash.is_empty() {
            return None;
        }
        Some((user, hash))
    }

    pub fn set_site_pass_protect(
        &self,
        domain: &str,
        enabled: bool,
        user: Option<&str>,
        password: Option<&str>,
    ) -> Result<(), String> {
        let path = self.site_security_path(domain);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        let mut current: serde_json::Value = fs::read_to_string(&path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_else(|| serde_json::json!({}));

        current["enabled"] = serde_json::json!(enabled);
        if let Some(u) = user.map(str::trim).filter(|s| !s.is_empty()) {
            current["user"] = serde_json::json!(u);
        } else if current.get("user").is_none() {
            current["user"] = serde_json::json!("admin");
        }
        if let Some(p) = password.filter(|s| !s.is_empty()) {
            current["password_hash"] = serde_json::json!(crate::downloads::sha256_hex(p));
        } else if enabled && current.get("password_hash").and_then(|v| v.as_str()).unwrap_or("").is_empty() {
            return Err("A password is required to enable Password Protection".to_string());
        }

        let serialized = serde_json::to_string_pretty(&current).map_err(|e| e.to_string())?;
        fs::write(&path, serialized).map_err(|e| e.to_string())
    }

    pub fn toggle_site_enabled(&self, id_or_domain: &str, enabled: bool) -> Result<LocalSite, String> {
        let clean = id_or_domain.trim().to_lowercase();
        {
            let mut data = self.data.lock().map_err(|_| "State lock failed".to_string())?;
            if let Some(s) = data.sites.iter_mut().find(|s| s.id == id_or_domain || s.domain.to_lowercase() == clean) {
                s.enabled = enabled;
            } else {
                return Err("Site not found".to_string());
            }
        }
        self.save_data();
        if enabled {
            self.start_site(id_or_domain)
        } else {
            self.stop_site(id_or_domain)
        }
    }

    pub fn rename_site_domain(&self, id_or_domain: &str, new_domain: &str) -> Result<LocalSite, String> {
        let validated = crate::hosts_manager::validate_domain(new_domain)?;
        let clean_new = validated.to_lowercase();
        let clean = id_or_domain.trim().to_lowercase();

        let (old_domain, site_id, was_active) = {
            let data = self.data.lock().map_err(|_| "State lock failed".to_string())?;
            if data.sites.iter().any(|s| s.domain.to_lowercase() == clean_new) {
                return Err(format!("Domain '{}' already exists", clean_new));
            }
            if let Some(s) = data.sites.iter().find(|s| s.id == id_or_domain || s.domain.to_lowercase() == clean) {
                (s.domain.clone(), s.id.clone(), s.status == "active")
            } else {
                return Err("Site not found".to_string());
            }
        };

        if was_active {
            let _ = self.stop_site(&site_id);
        }

        let old_dir = self
            .get_site(&site_id)
            .map(|s| self.site_dir(&s))
            .unwrap_or_else(|| self.sites_dir.join(&old_domain));
        let new_dir = if old_dir.parent().is_some() {
            old_dir.parent().unwrap().join(&clean_new)
        } else {
            self.get_default_sites_dir().join(&clean_new)
        };
        if old_dir.exists() && !new_dir.exists() {
            fs::rename(&old_dir, &new_dir).map_err(|e| format!("Failed to rename site folder: {}", e))?;
        }
        let _ = crate::hosts_manager::remove_domain_from_hosts(&old_domain);
        let _ = crate::hosts_manager::sync_domain_to_hosts(&clean_new);

        {
            let mut data = self.data.lock().map_err(|_| "State lock failed".to_string())?;
            if let Some(s) = data.sites.iter_mut().find(|s| s.id == site_id) {
                s.domain = clean_new.clone();
                s.document_root = new_dir.to_string_lossy().to_string();
            }
        }
        self.save_data();

        if was_active {
            self.start_site(&site_id)
        } else {
            self.get_site(&site_id).ok_or_else(|| "Site not found".to_string())
        }
    }

    pub fn clone_site(&self, id_or_domain: &str, new_domain: &str) -> Result<LocalSite, String> {
        let src = self.get_site(id_or_domain).ok_or_else(|| "Site not found".to_string())?;
        let cloned = self.create_site(new_domain, Some(&src.php_version))?;
        let src_dir = self.site_dir(&src);
        let dst_dir = self.site_dir(&cloned);
        if src_dir.exists() {
            let _ = crate::files::copy_dir_recursive(&src_dir, &dst_dir);
        }
        if let Some(preset) = src.php_preset.as_deref() {
            self.update_site_preset(&cloned.id, preset);
        }
        self.start_site(&cloned.id)
    }

    pub fn update_site_limits(
        &self,
        id_or_domain: &str,
        max_upload_mb: Option<i32>,
        php_memory_mb: Option<i32>,
        php_max_workers: Option<i32>,
    ) -> Result<LocalSite, String> {
        let clean = id_or_domain.trim().to_lowercase();
        let site = {
            let mut data = self.data.lock().map_err(|_| "State lock failed".to_string())?;
            if let Some(s) = data.sites.iter_mut().find(|s| s.id == id_or_domain || s.domain.to_lowercase() == clean) {
                if let Some(v) = max_upload_mb {
                    s.max_upload_mb = v;
                }
                if let Some(v) = php_memory_mb {
                    s.php_memory_mb = v;
                }
                if let Some(v) = php_max_workers {
                    s.php_max_workers = v;
                }
                s.clone()
            } else {
                return Err("Site not found".to_string());
            }
        };
        self.save_data();
        Ok(site)
    }

    pub fn dir_size_bytes(dir: &Path) -> u64 {
        let mut total = 0u64;
        if let Ok(entries) = fs::read_dir(dir) {
            for entry in entries.flatten() {
                if let Ok(meta) = entry.metadata() {
                    if meta.is_dir() {
                        total += Self::dir_size_bytes(&entry.path());
                    } else {
                        total += meta.len();
                    }
                }
            }
        }
        total
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
        self.create_site_with_root(domain, php_version, None)
    }

    pub fn create_site_with_root(
        &self,
        domain: &str,
        php_version: Option<&str>,
        custom_root: Option<&str>,
    ) -> Result<LocalSite, String> {
        self.create_site_with_options(domain, Some("php"), php_version, custom_root, None, None)
    }

    pub fn create_site_with_options(
        &self,
        domain: &str,
        runtime: Option<&str>,
        php_version: Option<&str>,
        custom_root: Option<&str>,
        app_command: Option<&str>,
        proxy_port: Option<u16>,
    ) -> Result<LocalSite, String> {
        let validated = crate::hosts_manager::validate_domain(domain)?;
        let clean_domain = validated.to_lowercase();
        let id = uuid::Uuid::new_v4().to_string();

        let runtime_str = match runtime.map(str::trim).filter(|s| !s.is_empty()) {
            Some("static") => "static".to_string(),
            Some("node") => "node".to_string(),
            Some("proxy") => "proxy".to_string(),
            _ => "php".to_string(),
        };

        let effective_app_cmd = if runtime_str == "node" {
            let cmd = app_command
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .unwrap_or("npm start")
                .to_string();
            Some(cmd)
        } else {
            None
        };

        let base_dir = self.get_default_sites_dir();
        let site_dir = if let Some(raw_root) = custom_root.map(str::trim).filter(|s| !s.is_empty()) {
            let candidate = PathBuf::from(raw_root);
            if !candidate.is_absolute() || candidate.parent().is_none() {
                return Err("Custom site path must be a valid absolute non-root folder path".to_string());
            }
            let lower = raw_root.replace('/', "\\").to_lowercase();
            if lower.starts_with(r"c:\windows") || lower == r"c:\program files" {
                return Err("Cannot place website files inside a system directory".to_string());
            }
            candidate
        } else {
            let d = base_dir.join(&clean_domain);
            if d.parent() != Some(base_dir.as_path()) {
                return Err("Invalid domain path".to_string());
            }
            d
        };
        fs::create_dir_all(&site_dir)
            .map_err(|e| format!("Failed creating site directory '{}': {}", site_dir.display(), e))?;

        if runtime_str == "node" {
            let pkg_json = site_dir.join("package.json");
            let server_js = site_dir.join("server.js");
            let index_js = site_dir.join("index.js");
            let app_js = site_dir.join("app.js");
            if !pkg_json.exists() && !server_js.exists() && !index_js.exists() && !app_js.exists() {
                let pkg_name: String = clean_domain
                    .chars()
                    .map(|c| if c.is_ascii_alphanumeric() || c == '-' { c } else { '-' })
                    .collect();
                let pkg_content = format!(
                    "{{\n  \"name\": \"{pkg_name}\",\n  \"version\": \"1.0.0\",\n  \"private\": true,\n  \"scripts\": {{\n    \"start\": \"node server.js\"\n  }}\n}}\n"
                );
                let _ = fs::write(&pkg_json, pkg_content);
                let server_content = format!(
                    r#"const http = require('http');

const port = Number(process.env.PORT) || 3000;
const server = http.createServer((req, res) => {{
  res.writeHead(200, {{ 'Content-Type': 'text/html; charset=utf-8' }});
  res.end(`<!DOCTYPE html>
<html lang="en">
<head>
  <meta charset="UTF-8">
  <title>{domain} - DockPanel Desktop</title>
  <style>
    body {{ font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, sans-serif; background: #0f172a; color: #f8fafc; display: flex; align-items: center; justify-content: center; height: 100vh; margin: 0; }}
    .card {{ background: #1e293b; padding: 2.5rem; border-radius: 1rem; border: 1px solid #334155; text-align: center; max-width: 520px; box-shadow: 0 10px 25px rgba(0,0,0,0.5); }}
    h1 {{ color: #38bdf8; margin-top: 0; }}
    p {{ color: #94a3b8; font-size: 1.05rem; line-height: 1.6; }}
    .badge {{ background: #059669; color: white; padding: 0.35rem 0.75rem; border-radius: 9999px; font-size: 0.875rem; font-weight: 600; display: inline-block; margin-top: 1rem; }}
  </style>
</head>
<body>
  <div class="card">
    <h1>{domain}</h1>
    <p>Your local Node.js app is running natively via <strong>DockPanel Desktop</strong> (Node ${{process.version}} on port ${{port}}).</p>
    <span class="badge">Node.js Runtime Active</span>
  </div>
</body>
</html>`);
}});

server.listen(port, '127.0.0.1', () => {{
  console.log(`Server listening on http://127.0.0.1:${{port}}`);
}});
"#,
                    domain = clean_domain
                );
                let _ = fs::write(&server_js, server_content);
            }
        } else {
            // Write starter index.html only if folder doesn't already have index.html or index.php
            let index_file = site_dir.join("index.html");
            let index_php = site_dir.join("index.php");
            if !index_file.exists() && !index_php.exists() {
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
        }

        let site = LocalSite {
            id,
            domain: clean_domain.clone(),
            runtime: runtime_str,
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
            app_command: effective_app_cmd,
            proxy_port,
        };

        {
            let mut data = self.data.lock().unwrap();
            data.sites.retain(|s| s.domain != clean_domain);
            data.sites.push(site.clone());
        }
        self.save_data();

        // Provision local mkcert TLS certificate + register in Windows hosts (best-effort)
        #[cfg(not(test))]
        {
            let _ = self.ensure_site_ssl(&clean_domain);
        }
        let _ = crate::hosts_manager::sync_domain_to_hosts(&clean_domain);

        Ok(site)
    }

    pub fn delete_site(&self, id: &str) -> bool {
        let removed = {
            let mut data = self.data.lock().unwrap();
            let pos = data.sites.iter().position(|s| s.id == id || s.domain == id);
            pos.map(|idx| data.sites.remove(idx))
        };

        if let Some(site) = removed {
            let clean = site.domain.trim().to_lowercase();
            let doc_root = self.site_dir(&site);
            if let Ok(mut procs) = self.worker_processes.lock() {
                if let Some(mut child) = procs.remove(&clean) {
                    let _ = child.kill();
                    let _ = child.wait();
                }
            }
            self.remove_site_worker_port(&clean);
            self.save_data();
            let _ = crate::hosts_manager::remove_domain_from_hosts(&site.domain);

            // Clean up any per-site MariaDB database if present in wp-config.php
            if let Some(wp_db) = crate::wordpress::read_wp_mysql_db_name(&doc_root) {
                if let Some(port) = self.get_mariadb_port() {
                    let _ = crate::mariadb::drop_database(port, &wp_db);
                }
            }

            // Delete local site directory and all files to free disk space
            let default_base = self.get_default_sites_dir();
            let safe_to_remove = doc_root.exists()
                && doc_root.parent().is_some()
                && doc_root != self.sites_dir
                && doc_root != default_base
                && doc_root != self.config_dir;
            if safe_to_remove {
                for _ in 0..5 {
                    if fs::remove_dir_all(&doc_root).is_ok() || !doc_root.exists() {
                        break;
                    }
                    std::thread::sleep(std::time::Duration::from_millis(120));
                }
            }

            let cert_dir = self.certs_dir().join(&clean);
            if cert_dir.is_dir() {
                let _ = fs::remove_dir_all(cert_dir);
            }
            let sec_file = self.site_security_path(&clean);
            if sec_file.is_file() {
                let _ = fs::remove_file(sec_file);
            }
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

        let php_error_log = self.config_dir.join("php_errors.log");
        let error_log_arg = format!("error_log={}", php_error_log.to_string_lossy());
        let mut cmd = std::process::Command::new(&php_exe);
        cmd.args([
            "-d",
            &error_log_arg,
            "-S",
            &format!("127.0.0.1:{}", worker_port),
            "-t",
            doc_root,
        ])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());

        #[cfg(windows)]
        cmd.creation_flags(CREATE_NO_WINDOW);

        match cmd.spawn() {
            Ok(child) => {
                #[cfg(windows)]
                assign_child_to_job(&self.worker_job, &child);

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

    pub fn app_log_path(&self, domain: &str) -> PathBuf {
        let clean = domain.trim().to_lowercase();
        self.config_dir.join("logs").join(format!("{}.app.log", clean))
    }

    pub fn spawn_site_worker(&self, site: &LocalSite) {
        match site.runtime.as_str() {
            "node" => {
                self.spawn_node_worker(&site.domain, &site.document_root, site.app_command.as_deref());
            }
            "static" => {}
            "proxy" => {
                if let Some(p) = site.proxy_port {
                    self.set_site_worker_port(&site.domain, p);
                }
            }
            _ => {
                self.spawn_php_worker(&site.domain, &site.document_root, Some(&site.php_version));
            }
        }
    }

    pub fn spawn_node_worker(
        &self,
        domain: &str,
        doc_root: &str,
        app_command: Option<&str>,
    ) -> Option<u16> {
        let clean = domain.trim().to_lowercase();

        // Check if existing worker is still alive
        {
            let mut procs = self.worker_processes.lock().unwrap();
            if let Some(child) = procs.get_mut(&clean) {
                if let Ok(None) = child.try_wait() {
                    if let Some(port) = self.get_site_worker_port(&clean) {
                        return Some(port);
                    }
                } else {
                    procs.remove(&clean);
                }
            }
        }

        let listener = std::net::TcpListener::bind("127.0.0.1:0").ok()?;
        let worker_port = listener.local_addr().ok()?.port();
        drop(listener);

        let command_line = app_command
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .unwrap_or("npm start");

        let log_path = self.app_log_path(&clean);
        if let Some(parent) = log_path.parent() {
            let _ = fs::create_dir_all(parent);
        }
        let stdout_file = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&log_path)
            .ok();
        let stderr_file = stdout_file.as_ref().and_then(|f| f.try_clone().ok());

        #[cfg(windows)]
        let mut cmd = {
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x08000000;
            let mut c = std::process::Command::new("cmd.exe");
            c.args(["/C", command_line]);
            c.creation_flags(CREATE_NO_WINDOW);
            if let Some(node_exe) = find_node_binary() {
                if let Some(node_dir) = node_exe.parent() {
                    let current_path = std::env::var("PATH").unwrap_or_default();
                    c.env("PATH", format!("{};{}", node_dir.to_string_lossy(), current_path));
                }
            }
            c
        };
        #[cfg(not(windows))]
        let mut cmd = {
            let mut c = std::process::Command::new("sh");
            c.args(["-c", command_line]);
            c
        };

        cmd.current_dir(doc_root)
            .env("PORT", worker_port.to_string())
            .env("HOST", "127.0.0.1")
            .env("SITE_DOMAIN", &clean);

        if let Some(out) = stdout_file {
            cmd.stdout(std::process::Stdio::from(out));
        } else {
            cmd.stdout(std::process::Stdio::null());
        }
        if let Some(err) = stderr_file {
            cmd.stderr(std::process::Stdio::from(err));
        } else {
            cmd.stderr(std::process::Stdio::null());
        }

        match cmd.spawn() {
            Ok(child) => {
                #[cfg(windows)]
                assign_child_to_job(&self.worker_job, &child);

                {
                    let mut procs = self.worker_processes.lock().unwrap();
                    if let Some(mut old_child) = procs.insert(clean.clone(), child) {
                        let _ = old_child.kill();
                        let _ = old_child.wait();
                    }
                }
                self.set_site_worker_port(&clean, worker_port);

                // Briefly wait up to 1.5s for Node.js to bind its port so immediate requests succeed
                for _ in 0..30 {
                    if std::net::TcpStream::connect(("127.0.0.1", worker_port)).is_ok() {
                        break;
                    }
                    std::thread::sleep(std::time::Duration::from_millis(50));
                }
                Some(worker_port)
            }
            Err(e) => {
                eprintln!("Failed to spawn Node.js worker for {}: {}", clean, e);
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

        // Spawn runtime worker if applicable (PHP, Node.js, or Proxy)
        self.spawn_site_worker(&site);

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
            self.spawn_site_worker(&site);
            let _ = crate::hosts_manager::sync_domain_to_hosts(&site.domain);
            #[cfg(not(test))]
            {
                let _ = self.ensure_site_ssl(&site.domain);
            }
        }
    }

    pub fn get_mariadb_port(&self) -> Option<u16> {
        if let Ok(mut guard) = self.mariadb_process.lock() {
            if let Some((child, port)) = guard.as_mut() {
                if matches!(child.try_wait(), Ok(None)) {
                    return Some(*port);
                }
                *guard = None;
            }
        }
        #[cfg(not(test))]
        if crate::mariadb::is_port_accepting_tcp(3306) {
            return Some(3306);
        }
        None
    }

    pub fn start_mariadb(&self) -> Result<u16, String> {
        if let Some(existing) = self.get_mariadb_port() {
            return Ok(existing);
        }
        let daemon = crate::mariadb::find_mariadbd_binary().ok_or_else(|| {
            "MariaDB is not installed yet. Install MariaDB from Settings -> PHP & Services or Databases."
                .to_string()
        })?;
        let port = crate::mariadb::select_mariadb_port(3306);
        let data_dir = crate::mariadb::mariadb_data_dir();
        crate::mariadb::ensure_data_dir_initialized(&data_dir, port)?;

        let mut cmd = std::process::Command::new(&daemon);
        cmd.args([
            &format!("--datadir={}", data_dir.to_string_lossy()),
            &format!("--port={}", port),
            "--bind-address=127.0.0.1",
            "--console",
        ]);
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x08000000;
            cmd.creation_flags(CREATE_NO_WINDOW);
        }
        cmd.stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null());

        let child = cmd
            .spawn()
            .map_err(|e| format!("Failed starting MariaDB daemon: {}", e))?;

        #[cfg(windows)]
        assign_child_to_job(&self.worker_job, &child);

        for _ in 0..40 {
            if crate::mariadb::is_port_accepting_tcp(port) {
                if let Ok(mut guard) = self.mariadb_process.lock() {
                    *guard = Some((child, port));
                }
                return Ok(port);
            }
            std::thread::sleep(std::time::Duration::from_millis(250));
        }

        let _ = {
            let mut c = child;
            let _ = c.kill();
            let _ = c.wait();
        };
        Err(format!("MariaDB daemon started but port {} did not become ready in time", port))
    }

    pub fn stop_mariadb(&self) {
        if let Ok(mut guard) = self.mariadb_process.lock() {
            if let Some((mut child, _)) = guard.take() {
                let _ = child.kill();
                let _ = child.wait();
            }
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
        self.stop_mariadb();
    }

    pub fn record_event(
        &self,
        action: &str,
        target_type: &str,
        target_name: &str,
        title: &str,
        message: &str,
        severity: &str,
    ) {
        let now = chrono::Utc::now().to_rfc3339();
        let entry = LocalActivity {
            id: uuid::Uuid::new_v4().to_string(),
            action: action.to_string(),
            target_type: target_type.to_string(),
            target_name: target_name.to_string(),
            details: message.to_string(),
            user_email: "local@dockpanel.desktop".to_string(),
            created_at: now.clone(),
        };
        if let Ok(mut act) = self.activity_log.lock() {
            act.insert(0, entry);
            act.truncate(100);
        }

        let notif = LocalNotification {
            id: uuid::Uuid::new_v4().to_string(),
            title: title.to_string(),
            message: message.to_string(),
            severity: severity.to_string(),
            read: false,
            created_at: now.clone(),
        };
        let _ = self.notif_tx.send(serde_json::json!({
            "id": notif.id,
            "title": notif.title,
            "message": notif.message,
            "severity": notif.severity,
            "category": "desktop",
            "link": serde_json::Value::Null,
            "read": false,
            "read_at": serde_json::Value::Null,
            "created_at": notif.created_at,
        }));
        if let Ok(mut notifs) = self.notifications.lock() {
            notifs.insert(0, notif);
            notifs.truncate(50);
        }

        let log_file = self.config_dir.join("desktop_activity.log");
        if let Ok(mut f) = fs::OpenOptions::new().create(true).append(true).open(log_file) {
            use std::io::Write;
            let _ = writeln!(f, "[{}] [{}] {}: {}", now, action, target_name, message);
        }
    }

    pub fn get_settings(&self) -> HashMap<String, String> {
        let mut defaults = HashMap::new();
        let default_php = ["8.6", "8.5", "8.4", "8.3", "8.2", "8.1", "8.0", "7.4"]
            .into_iter()
            .find(|v| find_php_binary_exact(v).is_some())
            .unwrap_or("8.3");
        defaults.insert("panel_name".to_string(), "DockPanel Desktop".to_string());
        defaults.insert("base_url".to_string(), "http://127.0.0.1".to_string());
        defaults.insert("default_php_version".to_string(), default_php.to_string());
        defaults.insert("preferred_port".to_string(), self.get_router_config().preferred_port.to_string());
        defaults.insert(
            "default_sites_dir".to_string(),
            self.sites_dir.to_string_lossy().to_string(),
        );

        let path = self.config_dir.join("desktop_settings.json");
        if let Ok(content) = fs::read_to_string(path) {
            if let Ok(saved) = serde_json::from_str::<HashMap<String, String>>(&content) {
                for (k, v) in saved {
                    defaults.insert(k, v);
                }
            }
        }
        defaults
    }

    pub fn update_settings(&self, updates: HashMap<String, String>) -> HashMap<String, String> {
        let mut current = self.get_settings();
        for (k, v) in updates {
            current.insert(k, v);
        }
        let path = self.config_dir.join("desktop_settings.json");
        if let Ok(json) = serde_json::to_string_pretty(&current) {
            let _ = fs::write(path, json);
        }
        current
    }

    /// Terminates stale PHP built-in server processes pointing at `sites_dir` left over from previous sessions.
    pub fn cleanup_orphan_php_workers(&self) {
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x08000000;
            let sites_str = self.sites_dir.to_string_lossy().replace('\'', "''");
            let ps = format!(
                "Get-CimInstance Win32_Process -Filter \"Name='php.exe'\" -ErrorAction SilentlyContinue | Where-Object {{ $_.CommandLine -like '*{}*' }} | ForEach-Object {{ Stop-Process -Id $_.ProcessId -Force -ErrorAction SilentlyContinue }}",
                sites_str
            );
            let _ = std::process::Command::new("powershell.exe")
                .args(["-NoProfile", "-ExecutionPolicy", "Bypass", "-Command", &ps])
                .creation_flags(CREATE_NO_WINDOW)
                .output();
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

    pub async fn restart_ssl_router(&self, port: u16) -> bool {
        let mut handle_guard = self.ssl_router_handle.lock().await;
        if let Some(old_handle) = handle_guard.take() {
            old_handle.abort();
            let _ = old_handle.await;
        }
        let (handle, rx) = crate::router::start_ssl_site_router_checked(Arc::new(self.clone()), port);
        let bound = rx.await.unwrap_or(false);
        if bound {
            *handle_guard = Some(handle);
            self.set_ssl_port(Some(port));
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

    #[test]
    fn test_custom_root_site_and_disk_cleanup() {
        let temp_dir = std::env::temp_dir().join(format!("dockpanel_custom_root_{}", uuid::Uuid::new_v4()));
        let config_dir = temp_dir.join("config");
        let sites_dir = temp_dir.join("sites");
        let custom_root = temp_dir.join("custom_websites").join("my-custom.test");
        let state = AppState::new_with_dirs(config_dir, sites_dir);

        assert!(state.create_site_with_root("rel.test", Some("8.3"), Some("relative/folder")).is_err());

        let custom_str = custom_root.to_string_lossy().to_string();
        let site = state
            .create_site_with_root("my-custom.test", Some("8.3"), Some(&custom_str))
            .unwrap();
        assert_eq!(std::path::PathBuf::from(&site.document_root), custom_root);
        assert!(custom_root.join("index.php").exists() || custom_root.join("index.html").exists());

        std::fs::write(custom_root.join("extra.txt"), "cleanup-me").unwrap();
        assert!(state.delete_site(&site.id));
        assert!(!custom_root.exists(), "Deleting a site must remove its document root folder from disk");

        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_create_node_site_scaffolds_starter_app() {
        let temp_dir = std::env::temp_dir().join(format!("dockpanel_node_site_{}", uuid::Uuid::new_v4()));
        let config_dir = temp_dir.join("config");
        let sites_dir = temp_dir.join("sites");
        let state = AppState::new_with_dirs(config_dir, sites_dir.clone());

        let site = state
            .create_site_with_options("mynode.test", Some("node"), None, None, Some("node server.js"), None)
            .unwrap();
        assert_eq!(site.runtime, "node");
        assert_eq!(site.app_command.as_deref(), Some("node server.js"));

        let site_dir = sites_dir.join("mynode.test");
        assert!(site_dir.join("package.json").is_file());
        assert!(site_dir.join("server.js").is_file());
        assert!(!site_dir.join("index.html").exists());

        let _ = std::fs::remove_dir_all(&temp_dir);
    }
}

