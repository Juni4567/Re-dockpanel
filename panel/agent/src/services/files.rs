use std::path::{Path, PathBuf};
use tokio::fs;

const WEBROOT: &str = "/var/www";

#[derive(serde::Serialize, serde::Deserialize, Clone, Debug)]
pub struct FileEntry {
    pub name: String,
    pub path: String,
    pub is_dir: bool,
    pub size: u64,
    pub modified: String,
    pub permissions: String,
    pub owner: String,
}

#[derive(serde::Serialize, serde::Deserialize, Clone, Debug)]
pub struct DirItem {
    pub name: String,
    pub path: String,
    pub has_subdirs: bool,
}

#[derive(serde::Serialize)]
pub struct FileContent {
    pub content: String,
    pub size: u64,
    pub modified: String,
}

/// Resolve a user-provided path to a safe absolute path within /var/www/{domain}/.
/// Prevents path traversal attacks.
pub fn resolve_safe_path(domain: &str, relative_path: &str) -> Result<PathBuf, String> {
    let base = PathBuf::from(format!("{WEBROOT}/{domain}"));
    resolve_within(&base, relative_path)
}

/// Resolve a path that must name something INSIDE the site root — never the root itself.
///
/// `resolve_within` deliberately permits the site root, because listing it is the file
/// manager's default view (`?path=/`). Containment and identity are different questions,
/// though, and only the containment one was being asked: `.`, `""` and `/` all canonicalise
/// to the webroot, and a path always starts with itself, so the traversal check passes them.
/// A destructive verb handed that path is a request to erase the entire site in one call —
/// `remove_dir_all` on the webroot takes the site with it, and the caller sees `success`.
///
/// So the read verbs keep using the permissive resolver and the destructive ones use this,
/// which refuses the root and lets everything below it through unchanged.
pub fn resolve_safe_child(domain: &str, relative_path: &str) -> Result<PathBuf, String> {
    let base = PathBuf::from(format!("{WEBROOT}/{domain}"));
    let resolved = resolve_within(&base, relative_path)?;
    let canon_base = base
        .canonicalize()
        .map_err(|_| format!("Site root does not exist: {}", base.display()))?;
    if resolved == canon_base {
        return Err("Refusing to operate on the site root itself".into());
    }
    Ok(resolved)
}

/// Core of [`resolve_safe_path`], parameterized on the base directory so it is
/// unit-testable. Guarantees the returned path is inside `base` AND that no symlink
/// sits in the to-be-created portion of the path — otherwise a create/upload sink
/// (which opens O_CREAT and follows the final symlink) could write OUTSIDE the site
/// root as root. A LIVE symlink escaping the root is caught by canonicalize + the
/// `starts_with` check; a DANGLING symlink is invisible to `exists()`/canonicalize
/// (stat follows it, sees the absent target) but visible to lstat, so it is rejected
/// explicitly here.
pub(crate) fn resolve_within(base: &Path, relative_path: &str) -> Result<PathBuf, String> {
    // Normalize: strip leading slashes (traversal via ".." is rejected below).
    let cleaned = relative_path.trim_start_matches('/');
    let candidate = base.join(cleaned);

    // Canonicalize base (must exist).
    let canon_base = base
        .canonicalize()
        .map_err(|_| format!("Site root does not exist: {}", base.display()))?;

    let canon = if candidate.exists() {
        // Existing target: canonicalize fully resolves symlinks, and the
        // starts_with check below then guarantees the resolved target is in-root.
        candidate
            .canonicalize()
            .map_err(|e| format!("Path error: {e}"))?
    } else {
        // Target does not exist yet (create/write/upload). Walk up to the first
        // existing ancestor.
        let mut existing = candidate.clone();
        let mut trail = Vec::new();
        while !existing.exists() {
            if let Some(name) = existing.file_name() {
                trail.push(name.to_owned());
            } else {
                return Err("Invalid path".into());
            }
            existing = existing.parent().ok_or("Invalid path")?.to_path_buf();
        }
        // `exists()` follows symlinks, so a DANGLING symlink read as "absent" and
        // became a trailing component. Re-check each trailing component on the REAL
        // filesystem path with lstat and refuse ANY symlink — a create/upload sink
        // would otherwise follow it out of the site root (sandbox escape).
        let mut real = existing.clone();
        for component in trail.iter().rev() {
            real = real.join(component);
            if let Ok(meta) = std::fs::symlink_metadata(&real) {
                if meta.file_type().is_symlink() {
                    return Err("Path traversal denied (symlink in target path)".into());
                }
            }
        }
        // Build the returned path on the CANONICAL ancestor so starts_with is
        // symlink-free.
        let mut resolved = existing
            .canonicalize()
            .map_err(|e| format!("Path resolution error: {e}"))?;
        for component in trail.into_iter().rev() {
            resolved = resolved.join(component);
        }
        resolved
    };

    if !canon.starts_with(&canon_base) {
        return Err("Path traversal denied".into());
    }

    Ok(canon)
}

// `ensure_site_root` used to live here, and `list` called it before resolving. It was the
// only silent failure in the file manager: asked for a domain this host does not serve, it
// created `/var/www/<domain>` as root and answered 200 with an empty listing, which then
// unblocked write/create/upload into a directory no vhost serves. Every other verb already
// fails loudly ("Site root does not exist"), and site provisioning creates the webroot, so
// nothing legitimate depended on creating it here. Removing it turns that silence into an
// error. Note it also means a site whose document root was pointed somewhere other than
// `/var/www/<domain>` now reports that plainly instead of showing an empty invented folder.

#[cfg(unix)]
fn get_user_group(uid: u32, gid: u32) -> String {
    let user = unsafe {
        let pwd = libc::getpwuid(uid);
        if !pwd.is_null() && !(*pwd).pw_name.is_null() {
            std::ffi::CStr::from_ptr((*pwd).pw_name)
                .to_string_lossy()
                .to_string()
        } else {
            uid.to_string()
        }
    };
    let group = unsafe {
        let grp = libc::getgrgid(gid);
        if !grp.is_null() && !(*grp).gr_name.is_null() {
            std::ffi::CStr::from_ptr((*grp).gr_name)
                .to_string_lossy()
                .to_string()
        } else {
            gid.to_string()
        }
    };
    format!("{user}:{group}")
}

#[cfg(not(unix))]
fn get_user_group(_uid: u32, _gid: u32) -> String {
    "www-data:www-data".to_string()
}

#[cfg(unix)]
fn get_permissions(mode: u32) -> String {
    format!("{:04o}", mode & 0o7777)
}

#[cfg(not(unix))]
fn get_permissions(_mode: u32) -> String {
    "0644".to_string()
}

/// List directory contents.
/// The `site_root` parameter is used to strip absolute paths to relative paths in the response.
/// If `None`, paths are returned relative to the listed directory.
pub async fn list_directory(path: &Path, site_root: Option<&Path>) -> Result<Vec<FileEntry>, String> {
    let mut entries = Vec::new();
    let mut reader = fs::read_dir(path)
        .await
        .map_err(|e| format!("Cannot read directory: {e}"))?;

    while let Some(entry) = reader
        .next_entry()
        .await
        .map_err(|e| format!("Read entry error: {e}"))?
    {
        let meta = entry.metadata().await.ok();
        let is_dir = meta.as_ref().map(|m| m.is_dir()).unwrap_or(false);
        let size = meta.as_ref().map(|m| m.len()).unwrap_or(0);
        let modified = meta
            .as_ref()
            .and_then(|m| m.modified().ok())
            .map(|t| {
                let dt: chrono::DateTime<chrono::Utc> = t.into();
                dt.format("%Y-%m-%d %H:%M:%S").to_string()
            })
            .unwrap_or_default();

        let (permissions, owner) = if let Some(m) = meta.as_ref() {
            #[cfg(unix)]
            {
                use std::os::unix::fs::MetadataExt;
                (get_permissions(m.mode()), get_user_group(m.uid(), m.gid()))
            }
            #[cfg(not(unix))]
            {
                let _ = m;
                (get_permissions(0o644), get_user_group(0, 0))
            }
        } else {
            (get_permissions(0o644), get_user_group(0, 0))
        };

        let name = entry.file_name().to_string_lossy().to_string();
        let abs_path = format!("{}/{}", path.display(), &name);

        // Return paths relative to the site root to avoid leaking server paths.
        // `path` is already canonical, so canonicalize the root too (in case
        // /var/www is itself a symlink) and strip that. Fail CLOSED to the bare
        // name — never the absolute server path — if the strip misses.
        let relative_path = if let Some(root) = site_root {
            let canon_root = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
            match std::path::Path::new(&abs_path).strip_prefix(&canon_root) {
                Ok(rel) => rel.to_string_lossy().to_string(),
                Err(_) => name.clone(),
            }
        } else {
            name.clone()
        };

        entries.push(FileEntry {
            path: relative_path,
            name,
            is_dir,
            size,
            modified,
            permissions,
            owner,
        });
    }

    // Sort: directories first, then alphabetical
    entries.sort_by(|a, b| b.is_dir.cmp(&a.is_dir).then(a.name.cmp(&b.name)));
    Ok(entries)
}

/// Read file content (text only, max 2MB).
pub async fn read_file(path: &Path) -> Result<FileContent, String> {
    let meta = fs::metadata(path)
        .await
        .map_err(|e| format!("File not found: {e}"))?;

    if meta.len() > 2 * 1024 * 1024 {
        return Err("File too large (max 2MB)".into());
    }

    let content = fs::read_to_string(path)
        .await
        .map_err(|_| "File is binary or not readable as text".to_string())?;

    let modified = meta
        .modified()
        .ok()
        .map(|t| {
            let dt: chrono::DateTime<chrono::Utc> = t.into();
            dt.format("%Y-%m-%d %H:%M:%S").to_string()
        })
        .unwrap_or_default();

    Ok(FileContent {
        content,
        size: meta.len(),
        modified,
    })
}

/// Write file content. Creates parent directories if needed.
pub async fn write_file(path: &Path, content: &str) -> Result<(), String> {
    // Ensure parent directory exists
    if let Some(parent) = path.parent() {
        if !parent.exists() {
            fs::create_dir_all(parent)
                .await
                .map_err(|e| format!("Failed to create directory: {e}"))?;
        }
    }
    // Write atomically via a collision-free temp sibling in the same (validated)
    // directory, then rename over the target. `with_extension("tmp")` would REPLACE
    // the extension and could clobber a distinct real sibling (saving report.php
    // would destroy an existing report.tmp). The UUID alone guarantees uniqueness, so
    // keep the temp name a FIXED short length — embedding the real filename could push
    // the component past NAME_MAX (255) for a long-but-valid basename -> ENAMETOOLONG
    // on save. Clean up the temp on any failure so a failed write can't leak orphans.
    let tmp = match path.parent() {
        Some(parent) => parent.join(format!(".dptmp.{}", uuid::Uuid::new_v4())),
        None => return Err("Invalid path".into()),
    };
    if let Err(e) = fs::write(&tmp, content).await {
        let _ = fs::remove_file(&tmp).await;
        return Err(format!("Failed to write: {e}"));
    }
    if let Err(e) = fs::rename(&tmp, path).await {
        let _ = fs::remove_file(&tmp).await;
        return Err(format!("Failed to finalize write: {e}"));
    }
    // Ensure written files are owned by www-data:www-data
    let _ = crate::safe_cmd::safe_command("chown")
        .args(["www-data:www-data", &path.to_string_lossy()])
        .output()
        .await;
    Ok(())
}

/// Create a file or directory.
pub async fn create_entry(path: &Path, is_dir: bool) -> Result<(), String> {
    if path.exists() {
        return Err("Path already exists".into());
    }
    if is_dir {
        fs::create_dir_all(path)
            .await
            .map_err(|e| format!("Failed to create directory: {e}"))?;
    } else {
        // Ensure parent exists
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).await.ok();
        }
        fs::write(path, "")
            .await
            .map_err(|e| format!("Failed to create file: {e}"))?;
    }
    let _ = crate::safe_cmd::safe_command("chown")
        .args(["-R", "www-data:www-data", &path.to_string_lossy()])
        .output()
        .await;
    Ok(())
}

/// Rename/move an entry.
pub async fn rename_entry(from: &Path, to: &Path) -> Result<(), String> {
    if !from.exists() {
        return Err("Source does not exist".into());
    }
    if to.exists() {
        return Err("Destination already exists".into());
    }
    fs::rename(from, to)
        .await
        .map_err(|e| format!("Rename failed: {e}"))?;
    Ok(())
}

/// Delete a file or directory.
pub async fn delete_entry(path: &Path) -> Result<(), String> {
    if !path.exists() {
        return Err("Path does not exist".into());
    }
    if path.is_dir() {
        fs::remove_dir_all(path)
            .await
            .map_err(|e| format!("Failed to delete directory: {e}"))?;
    } else {
        fs::remove_file(path)
            .await
            .map_err(|e| format!("Failed to delete file: {e}"))?;
    }
    Ok(())
}

/// Get direct subdirectories of a path, indicating whether each child has subdirectories of its own.
pub async fn get_subdirectories(path: &Path, site_root: Option<&Path>) -> Result<Vec<DirItem>, String> {
    let mut items = Vec::new();
    let mut reader = fs::read_dir(path)
        .await
        .map_err(|e| format!("Cannot read directory: {e}"))?;

    while let Some(entry) = reader
        .next_entry()
        .await
        .map_err(|e| format!("Read entry error: {e}"))?
    {
        let is_dir = entry.file_type().await.map(|t| t.is_dir()).unwrap_or(false);
        if !is_dir {
            continue;
        }

        let name = entry.file_name().to_string_lossy().to_string();
        if name.starts_with('.') {
            continue; // Skip hidden dirs like .git in directory tree navigation
        }

        let abs_path = format!("{}/{}", path.display(), &name);
        let relative_path = if let Some(root) = site_root {
            let canon_root = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
            match std::path::Path::new(&abs_path).strip_prefix(&canon_root) {
                Ok(rel) => rel.to_string_lossy().to_string(),
                Err(_) => name.clone(),
            }
        } else {
            name.clone()
        };

        // Check if child has any subdirectories
        let mut has_subdirs = false;
        if let Ok(mut sub_reader) = fs::read_dir(entry.path()).await {
            while let Ok(Some(sub_entry)) = sub_reader.next_entry().await {
                if let Ok(ft) = sub_entry.file_type().await {
                    if ft.is_dir() && !sub_entry.file_name().to_string_lossy().starts_with('.') {
                        has_subdirs = true;
                        break;
                    }
                }
            }
        }

        items.push(DirItem {
            name,
            path: relative_path,
            has_subdirs,
        });
    }

    items.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    Ok(items)
}

/// Extract an archive (.zip, .tar.gz, .tgz, .tar, .tar.bz2) into dest_dir.
pub async fn extract_archive(archive_path: &Path, dest_dir: &Path) -> Result<String, String> {
    if !archive_path.exists() {
        return Err("Archive file does not exist".into());
    }
    if !dest_dir.exists() {
        fs::create_dir_all(dest_dir)
            .await
            .map_err(|e| format!("Failed to create destination directory: {e}"))?;
    }

    let archive_str = archive_path.to_string_lossy().to_string();
    let dest_str = dest_dir.to_string_lossy().to_string();
    let lower = archive_str.to_lowercase();

    let output = if lower.ends_with(".zip") {
        crate::safe_cmd::safe_command("unzip")
            .args(["-o", "-q", &archive_str, "-d", &dest_str])
            .output()
            .await
            .map_err(|e| format!("Failed to execute unzip: {e}"))?
    } else if lower.ends_with(".tar.gz") || lower.ends_with(".tgz") {
        crate::safe_cmd::safe_command("tar")
            .args(["-xzf", &archive_str, "-C", &dest_str])
            .output()
            .await
            .map_err(|e| format!("Failed to execute tar: {e}"))?
    } else if lower.ends_with(".tar.bz2") {
        crate::safe_cmd::safe_command("tar")
            .args(["-xjf", &archive_str, "-C", &dest_str])
            .output()
            .await
            .map_err(|e| format!("Failed to execute tar: {e}"))?
    } else if lower.ends_with(".tar") {
        crate::safe_cmd::safe_command("tar")
            .args(["-xf", &archive_str, "-C", &dest_str])
            .output()
            .await
            .map_err(|e| format!("Failed to execute tar: {e}"))?
    } else {
        return Err("Unsupported archive format. Supported: .zip, .tar.gz, .tgz, .tar, .tar.bz2".into());
    };

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("Extraction failed: {stderr}"));
    }

    // Set ownership of extracted files to www-data:www-data
    let _ = crate::safe_cmd::safe_command("chown")
        .args(["-R", "www-data:www-data", &dest_str])
        .output()
        .await;

    // Secure .git if it exists in dest_str
    let git_dir = format!("{dest_str}/.git");
    if std::path::Path::new(&git_dir).exists() {
        let _ = crate::safe_cmd::safe_command("chown").args(["-R", "root:root", &git_dir]).output().await;
        let _ = crate::safe_cmd::safe_command("chmod").args(["-R", "go-rwx", &git_dir]).output().await;
    }

    Ok("Extraction completed successfully".into())
}

/// Compress sources into an archive.
pub async fn compress_entries(
    source_paths: &[PathBuf],
    archive_path: &Path,
    format: &str,
) -> Result<u64, String> {
    if source_paths.is_empty() {
        return Err("No source files or directories provided".into());
    }
    if let Some(parent) = archive_path.parent() {
        if !parent.exists() {
            fs::create_dir_all(parent)
                .await
                .map_err(|e| format!("Failed to create destination directory: {e}"))?;
        }
    }

    let archive_str = archive_path.to_string_lossy().to_string();
    let lower_format = format.to_lowercase();
    let is_tar = lower_format == "tar.gz" || lower_format == "tgz" || archive_str.ends_with(".tar.gz") || archive_str.ends_with(".tgz");

    let output = if is_tar {
        let mut cmd = crate::safe_cmd::safe_command("tar");
        cmd.arg("-czf").arg(&archive_str);
        for p in source_paths {
            cmd.arg(p.to_string_lossy().as_ref());
        }
        cmd.output()
            .await
            .map_err(|e| format!("Failed to execute tar: {e}"))?
    } else {
        // Default to zip
        let mut cmd = crate::safe_cmd::safe_command("zip");
        cmd.args(["-r", "-q", &archive_str]);
        for p in source_paths {
            cmd.arg(p.to_string_lossy().as_ref());
        }
        cmd.output()
            .await
            .map_err(|e| format!("Failed to execute zip: {e}"))?
    };

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("Compression failed: {stderr}"));
    }

    // Set ownership to www-data:www-data
    let _ = crate::safe_cmd::safe_command("chown")
        .args(["www-data:www-data", &archive_str])
        .output()
        .await;

    let len = fs::metadata(archive_path)
        .await
        .map(|m| m.len())
        .unwrap_or(0);

    Ok(len)
}

/// Copy a file or directory.
pub async fn copy_entry(from: &Path, to: &Path) -> Result<(), String> {
    if !from.exists() {
        return Err("Source does not exist".into());
    }
    if to.exists() {
        return Err("Destination already exists".into());
    }
    if let Some(parent) = to.parent() {
        if !parent.exists() {
            fs::create_dir_all(parent)
                .await
                .map_err(|e| format!("Failed to create parent directory: {e}"))?;
        }
    }

    if from.is_dir() {
        let output = crate::safe_cmd::safe_command("cp")
            .args(["-a", &from.to_string_lossy(), &to.to_string_lossy()])
            .output()
            .await
            .map_err(|e| format!("Failed to copy directory: {e}"))?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(format!("Copy failed: {stderr}"));
        }
    } else {
        fs::copy(from, to)
            .await
            .map_err(|e| format!("Failed to copy file: {e}"))?;
    }

    let _ = crate::safe_cmd::safe_command("chown")
        .args(["-R", "www-data:www-data", &to.to_string_lossy()])
        .output()
        .await;

    Ok(())
}

/// Change permissions of a file or directory.
pub async fn chmod_entry(path: &Path, mode_str: &str, recursive: bool) -> Result<(), String> {
    if !path.exists() {
        return Err("Target path does not exist".into());
    }

    let trimmed = mode_str.trim();
    if !trimmed.chars().all(|c| ('0'..='7').contains(&c)) || trimmed.len() < 3 || trimmed.len() > 4 {
        return Err("Invalid permissions mode format (must be 3 or 4 octal digits, e.g. 0755, 0644)".into());
    }

    let mut cmd = crate::safe_cmd::safe_command("chmod");
    if recursive && path.is_dir() {
        cmd.arg("-R");
    }
    cmd.arg(trimmed).arg(path.to_string_lossy().as_ref());

    let output = cmd.output().await.map_err(|e| format!("Failed to run chmod: {e}"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("chmod failed: {stderr}"));
    }

    Ok(())
}

/// Change ownership of a file or directory.
pub async fn chown_entry(path: &Path, owner: &str, recursive: bool) -> Result<(), String> {
    if !path.exists() {
        return Err("Target path does not exist".into());
    }

    let trimmed = owner.trim();
    if !trimmed.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == ':') {
        return Err("Invalid owner format".into());
    }

    let mut cmd = crate::safe_cmd::safe_command("chown");
    if recursive && path.is_dir() {
        cmd.arg("-R");
    }
    cmd.arg(trimmed).arg(path.to_string_lossy().as_ref());

    let output = cmd.output().await.map_err(|e| format!("Failed to run chown: {e}"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("chown failed: {stderr}"));
    }

    Ok(())
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::fs::symlink;

    fn tmpbase() -> PathBuf {
        // std::env::temp_dir() is /tmp on Linux (canonical). Canonicalize anyway so
        // the assertions hold regardless of the host layout.
        let d = std::env::temp_dir().join(format!("dp-fmtest-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&d).unwrap();
        d.canonicalize().unwrap()
    }

    // The sandbox-escape primitive: a DANGLING symlink leaf (target absent) must be
    // rejected — otherwise create/upload would fs::write THROUGH it as root.
    #[test]
    fn rejects_dangling_symlink_leaf() {
        let base = tmpbase();
        symlink("/nonexistent-dp-escape-target-xyz", base.join("evil")).unwrap();
        let r = resolve_within(&base, "evil");
        assert!(r.is_err(), "dangling symlink leaf must be rejected, got {r:?}");
        // Also when the escape target is a real-but-privileged path that does not yet
        // exist as a file (the /etc/cron.d root-RCE vector).
        symlink("/etc/cron.d/dp-pwn-xyz", base.join("evil2")).unwrap();
        assert!(resolve_within(&base, "evil2").is_err());
        // And when reached through a real subdir.
        std::fs::create_dir_all(base.join("pub")).unwrap();
        symlink("/etc/dp-passwd-xyz", base.join("pub/link")).unwrap();
        assert!(resolve_within(&base, "pub/link").is_err());
        std::fs::remove_dir_all(&base).ok();
    }

    // A LIVE symlink escaping the root (target exists) stays rejected via canonicalize.
    #[test]
    fn rejects_live_symlink_escaping_root() {
        let base = tmpbase();
        symlink("/etc", base.join("etclink")).unwrap();
        assert!(resolve_within(&base, "etclink").is_err());
        assert!(resolve_within(&base, "etclink/passwd").is_err());
        std::fs::remove_dir_all(&base).ok();
    }

    #[test]
    fn rejects_dotdot_traversal() {
        let base = tmpbase();
        assert!(resolve_within(&base, "../../etc/passwd").is_err());
        assert!(resolve_within(&base, "a/../../../etc/passwd").is_err());
        std::fs::remove_dir_all(&base).ok();
    }

    // Normal creates must still work.
    #[test]
    fn allows_new_files() {
        let base = tmpbase();
        assert!(resolve_within(&base, "newfile.txt").unwrap().starts_with(&base));
        std::fs::create_dir_all(base.join("wp-content")).unwrap();
        let r = resolve_within(&base, "wp-content/x.php").unwrap();
        assert!(r.starts_with(&base) && r.ends_with("wp-content/x.php"));
        std::fs::remove_dir_all(&base).ok();
    }

    // A legit IN-ROOT symlink directory (e.g. `current -> releases/v1`) must still
    // allow creating files beneath it — the escape fix must not break this.
    #[test]
    fn allows_create_through_in_root_symlink_dir() {
        let base = tmpbase();
        std::fs::create_dir_all(base.join("releases/v1")).unwrap();
        symlink(base.join("releases/v1"), base.join("current")).unwrap();
        let r = resolve_within(&base, "current/new.txt").unwrap();
        assert!(r.starts_with(&base), "in-root symlink-dir create must be allowed, got {r:?}");
        std::fs::remove_dir_all(&base).ok();
    }

    // Regression: the atomic-write temp name must not overflow NAME_MAX for a
    // long-but-legal basename (the collision-free-temp fix must not embed the name).
    #[tokio::test]
    async fn write_file_handles_long_basename() {
        let base = tmpbase();
        let target = base.join("a".repeat(230)); // valid (<=255), long enough to overflow if embedded
        write_file(&target, "hello").await.expect("save of a long-named file must succeed");
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "hello");
        std::fs::remove_dir_all(&base).ok();
    }
}
