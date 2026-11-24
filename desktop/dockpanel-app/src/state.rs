use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LocalSite {
    pub id: String,
    pub domain: String,
    pub php_version: String,
    pub document_root: String,
    pub status: String,
    pub created_at: String,
    pub ssl_enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AppData {
    pub sites: Vec<LocalSite>,
}

#[derive(Clone)]
pub struct AppState {
    data: Arc<Mutex<AppData>>,
    config_dir: PathBuf,
    sites_dir: PathBuf,
}

impl AppState {
    pub fn new() -> Self {
        let app_data = std::env::var("APPDATA")
            .unwrap_or_else(|_| std::env::var("USERPROFILE").unwrap_or_else(|_| ".".into()));
        let config_dir = PathBuf::from(app_data).join("DockPanel");
        let user_profile = std::env::var("USERPROFILE").unwrap_or_else(|_| ".".into());
        let sites_dir = PathBuf::from(user_profile).join("DockPanel").join("sites");

        let _ = fs::create_dir_all(&config_dir);
        let _ = fs::create_dir_all(&sites_dir);

        let data = Self::load_data(&config_dir);
        Self {
            data: Arc::new(Mutex::new(data)),
            config_dir,
            sites_dir,
        }
    }

    fn load_data(config_dir: &PathBuf) -> AppData {
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

    pub fn list_sites(&self) -> Vec<LocalSite> {
        self.data.lock().unwrap().sites.clone()
    }

    pub fn get_site(&self, id: &str) -> Option<LocalSite> {
        self.data
            .lock()
            .unwrap()
            .sites
            .iter()
            .find(|s| s.id == id || s.domain == id)
            .cloned()
    }

    pub fn create_site(&self, domain: &str, php_version: Option<&str>) -> LocalSite {
        let clean_domain = domain.trim().to_lowercase();
        let id = uuid::Uuid::new_v4().to_string();
        let site_dir = self.sites_dir.join(&clean_domain);
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
            php_version: php_version.unwrap_or("8.3").to_string(),
            document_root: site_dir.to_string_lossy().to_string(),
            status: "active".to_string(),
            created_at: chrono::Utc::now().to_rfc3339(),
            ssl_enabled: true,
        };

        {
            let mut data = self.data.lock().unwrap();
            data.sites.retain(|s| s.domain != clean_domain);
            data.sites.push(site.clone());
        }
        self.save_data();

        // Register domain in Windows hosts file via Zero-UAC Host Helper named pipe
        Self::notify_host_helper("add", &clean_domain, "127.0.0.1");

        site
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
            self.save_data();
            Self::notify_host_helper("remove", &dom, "127.0.0.1");
            true
        } else {
            false
        }
    }

    fn notify_host_helper(action: &str, domain: &str, ip: &str) {
        #[cfg(windows)]
        {
            use std::io::Write;
            let mut attempts = 0;
            while attempts < 2 {
                if let Ok(mut pipe) = std::fs::OpenOptions::new()
                    .write(true)
                    .open(r"\\.\pipe\dockpanel-host-helper")
                {
                    let req = serde_json::json!({
                        "action": action,
                        "domain": domain,
                        "ip": ip
                    });
                    let _ = writeln!(pipe, "{}", req);
                    break;
                }
                attempts += 1;
            }
        }
    }
}
