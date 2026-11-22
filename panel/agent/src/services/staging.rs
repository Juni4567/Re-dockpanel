use std::path::Path;
use crate::safe_cmd::safe_command;

const WEB_ROOT: &str = "/var/www";

fn validate_domain(domain: &str) -> Result<(), String> {
    if domain.is_empty() || domain.contains("..") || domain.contains('/')
        || domain.contains('\\') || domain.contains('\0')
        || !domain.chars().all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-') {
        return Err("Invalid domain format".to_string());
    }
    Ok(())
}

/// Clone site files from source to target domain using rsync.
/// Creates the target directory if it doesn't exist.
pub async fn clone_files(source_domain: &str, target_domain: &str) -> Result<String, String> {
    validate_domain(source_domain)?;
    validate_domain(target_domain)?;

    // Held through the rsync+chown below, both sites at once, in a fixed
    // order — see site_lock's module doc.
    let _guard = crate::site_lock::lock_sites(source_domain, target_domain).await;

    let source = format!("{WEB_ROOT}/{source_domain}/");
    let target = format!("{WEB_ROOT}/{target_domain}/");

    if !Path::new(&source).exists() {
        return Err(format!("Source directory not found: {source}"));
    }

    // Create target directory
    tokio::fs::create_dir_all(&target)
        .await
        .map_err(|e| format!("Failed to create target directory: {e}"))?;

    // rsync -a preserves permissions, ownership, timestamps
    // --delete ensures target is an exact copy
    let output = safe_command("rsync")
        .args(["-a", "--delete", &source, &target])
        .output()
        .await
        .map_err(|e| format!("Failed to run rsync: {e}"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("rsync failed: {stderr}"));
    }

    // Fix ownership to www-data — everything except `.git`, which stays root's.
    // rsync just copied the source tree verbatim, `.git` included if the source
    // is git-deployed; handing it to www-data gives the app `config`/`hooks/`
    // that `deploy.rs::clone_or_pull` runs as root on the next deploy. Mirrors
    // `deploy.rs::hand_tree_to_web_user`.
    if let Ok(mut entries) = tokio::fs::read_dir(&target).await {
        while let Ok(Some(entry)) = entries.next_entry().await {
            if entry.file_name() == std::ffi::OsStr::new(".git") {
                continue;
            }
            let _ = safe_command("chown")
                .args(["-R", "www-data:www-data", &entry.path().to_string_lossy()])
                .output()
                .await;
        }
    }
    let git_dir = format!("{target}.git");
    if Path::new(&git_dir).exists() {
        let _ = safe_command("chown").args(["-R", "root:root", &git_dir]).output().await;
        let _ = safe_command("chmod").args(["-R", "go-rwx", &git_dir]).output().await;
    }

    Ok(format!("Cloned {source} → {target}"))
}

/// Sync files between two site directories.
/// direction: "prod_to_staging" or "staging_to_prod"
pub async fn sync_files(source_domain: &str, target_domain: &str) -> Result<String, String> {
    validate_domain(source_domain)?;
    validate_domain(target_domain)?;

    // Held through the rsync+chown below — same reasoning as `clone_files`.
    let _guard = crate::site_lock::lock_sites(source_domain, target_domain).await;

    let source = format!("{WEB_ROOT}/{source_domain}/");
    let target = format!("{WEB_ROOT}/{target_domain}/");

    if !Path::new(&source).exists() {
        return Err(format!("Source directory not found: {source}"));
    }
    if !Path::new(&target).exists() {
        return Err(format!("Target directory not found: {target}"));
    }

    let output = safe_command("rsync")
        .args(["-a", "--delete", &source, &target])
        .output()
        .await
        .map_err(|e| format!("Failed to run rsync: {e}"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("rsync failed: {stderr}"));
    }

    // Fix ownership — everything except `.git`. See `clone_files` above for why.
    if let Ok(mut entries) = tokio::fs::read_dir(&target).await {
        while let Ok(Some(entry)) = entries.next_entry().await {
            if entry.file_name() == std::ffi::OsStr::new(".git") {
                continue;
            }
            let _ = safe_command("chown")
                .args(["-R", "www-data:www-data", &entry.path().to_string_lossy()])
                .output()
                .await;
        }
    }
    let git_dir = format!("{target}.git");
    if Path::new(&git_dir).exists() {
        let _ = safe_command("chown").args(["-R", "root:root", &git_dir]).output().await;
        let _ = safe_command("chmod").args(["-R", "go-rwx", &git_dir]).output().await;
    }

    Ok(format!("Synced {source} → {target}"))
}

/// Get disk usage of a site directory in bytes.
pub async fn site_disk_usage(domain: &str) -> Result<u64, String> {
    validate_domain(domain)?;
    let path = format!("{WEB_ROOT}/{domain}");
    if !Path::new(&path).exists() {
        return Ok(0);
    }

    let output = safe_command("du")
        .args(["-sb", &path])
        .output()
        .await
        .map_err(|e| format!("du failed: {e}"))?;

    if !output.status.success() {
        return Ok(0);
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    stdout
        .split_whitespace()
        .next()
        .and_then(|s| s.parse().ok())
        .ok_or_else(|| "Failed to parse du output".to_string())
}

/// Delete a site's web directory.
pub async fn delete_site_files(domain: &str) -> Result<(), String> {
    validate_domain(domain)?;

    // Held through the delete below — see site_lock's module doc.
    let _guard = crate::site_lock::lock_site(domain).await;

    let path = format!("{WEB_ROOT}/{domain}");
    if Path::new(&path).exists() {
        tokio::fs::remove_dir_all(&path)
            .await
            .map_err(|e| format!("Failed to delete {path}: {e}"))?;
    }
    Ok(())
}

/// Get disk usage for all site directories under /var/www in bytes in one fast command.
pub async fn all_sites_disk_usage() -> Result<std::collections::HashMap<String, u64>, String> {
    let mut map = std::collections::HashMap::new();

    let mut dir = match tokio::fs::read_dir(WEB_ROOT).await {
        Ok(d) => d,
        Err(_) => return Ok(map),
    };

    let mut paths = Vec::new();
    while let Ok(Some(entry)) = dir.next_entry().await {
        if let Ok(file_type) = entry.file_type().await {
            if file_type.is_dir() {
                if let Ok(name) = entry.file_name().into_string() {
                    if !name.starts_with('.') && name != "acme" && name != "html" {
                        paths.push(name);
                    }
                }
            }
        }
    }

    if paths.is_empty() {
        return Ok(map);
    }

    let mut cmd = safe_command("du");
    cmd.arg("-sb");
    for p in &paths {
        cmd.arg(format!("{WEB_ROOT}/{p}"));
    }
    if let Ok(output) = cmd.output().await {
        if output.status.success() {
            let stdout = String::from_utf8_lossy(&output.stdout);
            for line in stdout.lines() {
                let parts: Vec<&str> = line.split_whitespace().collect();
                if parts.len() >= 2 {
                    if let Ok(bytes) = parts[0].parse::<u64>() {
                        let dir_path = parts[1];
                        let name = dir_path.trim_start_matches(WEB_ROOT).trim_start_matches('/');
                        map.insert(name.to_string(), bytes);
                    }
                }
            }
        }
    }

    Ok(map)
}
