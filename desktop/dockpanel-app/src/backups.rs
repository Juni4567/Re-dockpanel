use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackupEntry {
    pub id: String,
    pub site_id: String,
    pub filename: String,
    pub size_bytes: u64,
    pub created_at: String,
    pub databases_included: i32,
    pub databases_expected: i32,
}

pub fn backups_root_dir() -> PathBuf {
    let user_profile = std::env::var("USERPROFILE").unwrap_or_else(|_| ".".into());
    PathBuf::from(user_profile).join("DockPanel").join("backups")
}

pub fn site_backups_dir(domain: &str) -> PathBuf {
    backups_root_dir().join(domain.trim().to_lowercase())
}

fn validate_backup_filename(filename: &str) -> Result<&str, String> {
    let clean = filename.trim();
    if clean.is_empty()
        || clean.contains("..")
        || clean.contains('/')
        || clean.contains('\\')
        || !clean.to_ascii_lowercase().ends_with(".zip")
    {
        return Err("Invalid backup filename".to_string());
    }
    Ok(clean)
}

pub fn list_site_backups(site_id: &str, domain: &str) -> Vec<BackupEntry> {
    let dir = site_backups_dir(domain);
    if !dir.is_dir() {
        return Vec::new();
    }
    let mut entries = Vec::new();
    if let Ok(rd) = fs::read_dir(&dir) {
        for entry in rd.flatten() {
            let path = entry.path();
            if path.is_file()
                && path
                    .extension()
                    .and_then(|e| e.to_str())
                    .map(|e| e.eq_ignore_ascii_case("zip"))
                    .unwrap_or(false)
            {
                let filename = entry.file_name().to_string_lossy().to_string();
                let meta = match entry.metadata() {
                    Ok(m) => m,
                    Err(_) => continue,
                };
                let created_at = meta
                    .modified()
                    .ok()
                    .map(|t| {
                        let dt: chrono::DateTime<chrono::Utc> = t.into();
                        dt.to_rfc3339()
                    })
                    .unwrap_or_else(|| chrono::Utc::now().to_rfc3339());
                entries.push(BackupEntry {
                    id: filename.clone(),
                    site_id: site_id.to_string(),
                    filename,
                    size_bytes: meta.len(),
                    created_at,
                    databases_included: 0,
                    databases_expected: 0,
                });
            }
        }
    }
    entries.sort_by(|a, b| b.created_at.cmp(&a.created_at));
    entries
}

pub fn resolve_backup_file(domain: &str, id_or_name: &str) -> Result<PathBuf, String> {
    let clean = validate_backup_filename(id_or_name)?;
    let path = site_backups_dir(domain).join(clean);
    if !path.is_file() {
        return Err("Backup archive not found".to_string());
    }
    Ok(path)
}

pub fn create_archive_sync(site_dir: &Path, domain: &str) -> Result<BackupEntry, String> {
    if !site_dir.is_dir() {
        return Err("Site directory does not exist".to_string());
    }
    let bdir = site_backups_dir(domain);
    fs::create_dir_all(&bdir).map_err(|e| format!("Failed creating backup folder: {e}"))?;

    let stamp = chrono::Utc::now().format("%Y%m%d-%H%M%S");
    let filename = format!("{}-{}.zip", domain.replace(['/', '\\'], "_"), stamp);
    let archive_path = bdir.join(&filename);

    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x08000000;
        let src_glob = format!("{}\\*", site_dir.to_string_lossy().replace('\'', "''"));
        let dst = archive_path.to_string_lossy().replace('\'', "''");
        let ps = format!(
            "Compress-Archive -Path '{src_glob}' -DestinationPath '{dst}' -Force"
        );
        let out = std::process::Command::new("powershell.exe")
            .args(["-NoProfile", "-ExecutionPolicy", "Bypass", "-Command", &ps])
            .creation_flags(CREATE_NO_WINDOW)
            .output()
            .map_err(|e| format!("Failed starting Compress-Archive: {e}"))?;
        if !out.status.success() {
            let err = String::from_utf8_lossy(&out.stderr).trim().to_string();
            return Err(format!("Backup compression failed: {err}"));
        }
    }

    #[cfg(not(windows))]
    {
        fs::write(&archive_path, b"PK\x05\x06\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0")
            .map_err(|e| e.to_string())?;
    }

    let size_bytes = fs::metadata(&archive_path).map(|m| m.len()).unwrap_or(0);
    Ok(BackupEntry {
        id: filename.clone(),
        site_id: domain.to_string(),
        filename,
        size_bytes,
        created_at: chrono::Utc::now().to_rfc3339(),
        databases_included: 0,
        databases_expected: 0,
    })
}

pub fn restore_archive_sync(site_dir: &Path, archive_path: &Path) -> Result<(), String> {
    if !archive_path.is_file() {
        return Err("Backup archive missing".to_string());
    }
    fs::create_dir_all(site_dir).map_err(|e| e.to_string())?;

    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x08000000;
        let src = archive_path.to_string_lossy().replace('\'', "''");
        let dst = site_dir.to_string_lossy().replace('\'', "''");
        let ps = format!("Expand-Archive -Path '{src}' -DestinationPath '{dst}' -Force");
        let out = std::process::Command::new("powershell.exe")
            .args(["-NoProfile", "-ExecutionPolicy", "Bypass", "-Command", &ps])
            .creation_flags(CREATE_NO_WINDOW)
            .output()
            .map_err(|e| format!("Failed starting Expand-Archive: {e}"))?;
        if !out.status.success() {
            let err = String::from_utf8_lossy(&out.stderr).trim().to_string();
            return Err(format!("Backup restore failed: {err}"));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_backup_filename_validation() {
        assert!(validate_backup_filename("../evil.zip").is_err());
        assert!(validate_backup_filename("sub/file.zip").is_err());
        assert!(validate_backup_filename("not_a_zip.txt").is_err());
        assert_eq!(validate_backup_filename("site-2026.zip").unwrap(), "site-2026.zip");
    }
}
