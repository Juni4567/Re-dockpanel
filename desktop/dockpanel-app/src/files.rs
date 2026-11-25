use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Component, Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileEntry {
    pub name: string_type::StringAlias,
    pub path: String,
    pub is_dir: bool,
    pub size: u64,
    pub modified: String,
    pub permissions: String,
    pub owner: String,
}

mod string_type {
    pub type StringAlias = String;
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DirItem {
    pub name: String,
    pub path: String,
    pub has_subdirs: bool,
}

/// Resolves a relative path safely inside `base_dir`, blocking directory traversal (`..`) and root escapes.
pub fn resolve_safe_path(base_dir: &Path, rel_path: &str) -> Result<PathBuf, String> {
    let trimmed = rel_path.trim().trim_start_matches(['/', '\\']);
    let mut resolved = base_dir.to_path_buf();

    if !trimmed.is_empty() && trimmed != "." {
        for comp in Path::new(trimmed).components() {
            match comp {
                Component::Normal(part) => resolved.push(part),
                Component::CurDir => {}
                Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                    return Err("Invalid path: traversal outside site root is not allowed".to_string());
                }
            }
        }
    }

    if let (Ok(canon_base), Ok(canon_target)) = (base_dir.canonicalize(), resolved.canonicalize()) {
        if !canon_target.starts_with(&canon_base) {
            return Err("Path escapes site directory".to_string());
        }
    }
    Ok(resolved)
}

fn format_modified(meta: &fs::Metadata) -> String {
    meta.modified()
        .ok()
        .map(|t| {
            let dt: chrono::DateTime<chrono::Utc> = t.into();
            dt.to_rfc3339()
        })
        .unwrap_or_default()
}

pub fn list_dir(base_dir: &Path, rel_path: &str) -> Result<Vec<FileEntry>, String> {
    let dir = resolve_safe_path(base_dir, rel_path)?;
    if !dir.is_dir() {
        return Err("Directory not found".to_string());
    }

    let prefix = if rel_path.trim().is_empty() || rel_path.trim() == "." {
        String::new()
    } else {
        rel_path.trim().trim_matches('/').to_string()
    };

    let mut entries = Vec::new();
    let read = fs::read_dir(&dir).map_err(|e| format!("Failed to read directory: {}", e))?;
    for entry in read.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        let meta = match entry.metadata() {
            Ok(m) => m,
            Err(_) => continue,
        };
        let is_dir = meta.is_dir();
        let rel = if prefix.is_empty() {
            name.clone()
        } else {
            format!("{}/{}", prefix, name)
        };
        entries.push(FileEntry {
            name,
            path: rel,
            is_dir,
            size: if is_dir { 0 } else { meta.len() },
            modified: format_modified(&meta),
            permissions: if is_dir { "0755".into() } else { "0644".into() },
            owner: "local".into(),
        });
    }

    entries.sort_by(|a, b| b.is_dir.cmp(&a.is_dir).then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase())));
    Ok(entries)
}

pub fn list_tree(base_dir: &Path, rel_path: &str) -> Result<Vec<DirItem>, String> {
    let dir = resolve_safe_path(base_dir, rel_path)?;
    if !dir.is_dir() {
        return Ok(Vec::new());
    }
    let prefix = if rel_path.trim().is_empty() || rel_path.trim() == "." {
        String::new()
    } else {
        rel_path.trim().trim_matches('/').to_string()
    };

    let mut items = Vec::new();
    if let Ok(read) = fs::read_dir(&dir) {
        for entry in read.flatten() {
            let path = entry.path();
            if path.is_dir() {
                let name = entry.file_name().to_string_lossy().to_string();
                let rel = if prefix.is_empty() {
                    name.clone()
                } else {
                    format!("{}/{}", prefix, name)
                };
                let has_subdirs = fs::read_dir(&path)
                    .map(|rd| rd.flatten().any(|e| e.path().is_dir()))
                    .unwrap_or(false);
                items.push(DirItem {
                    name,
                    path: rel,
                    has_subdirs,
                });
            }
        }
    }
    items.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    Ok(items)
}

pub fn read_file_content(base_dir: &Path, rel_path: &str) -> Result<String, String> {
    let path = resolve_safe_path(base_dir, rel_path)?;
    if !path.is_file() {
        return Err("File not found".to_string());
    }
    let bytes = fs::read(&path).map_err(|e| format!("Failed to read file: {}", e))?;
    Ok(String::from_utf8_lossy(&bytes).to_string())
}

pub fn write_file_content(base_dir: &Path, rel_path: &str, content: &str) -> Result<(), String> {
    let path = resolve_safe_path(base_dir, rel_path)?;
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    fs::write(&path, content).map_err(|e| format!("Failed to write file: {}", e))
}

pub fn create_entry(base_dir: &Path, rel_path: &str, entry_type: &str) -> Result<(), String> {
    let path = resolve_safe_path(base_dir, rel_path)?;
    if entry_type == "dir" {
        fs::create_dir_all(&path).map_err(|e| format!("Failed to create folder: {}", e))
    } else {
        if let Some(parent) = path.parent() {
            let _ = fs::create_dir_all(parent);
        }
        if path.exists() {
            return Err("File already exists".to_string());
        }
        fs::write(&path, "").map_err(|e| format!("Failed to create file: {}", e))
    }
}

pub fn rename_entry(base_dir: &Path, from: &str, to: &str) -> Result<(), String> {
    let src = resolve_safe_path(base_dir, from)?;
    let dst = resolve_safe_path(base_dir, to)?;
    if !src.exists() {
        return Err("Source path not found".to_string());
    }
    if let Some(parent) = dst.parent() {
        let _ = fs::create_dir_all(parent);
    }
    fs::rename(&src, &dst).map_err(|e| format!("Failed to rename: {}", e))
}

pub fn delete_entry(base_dir: &Path, rel_path: &str) -> Result<(), String> {
    let path = resolve_safe_path(base_dir, rel_path)?;
    if path == base_dir {
        return Err("Cannot delete site root directory".to_string());
    }
    if path.is_dir() {
        fs::remove_dir_all(&path).map_err(|e| format!("Failed to delete folder: {}", e))
    } else if path.exists() {
        fs::remove_file(&path).map_err(|e| format!("Failed to delete file: {}", e))
    } else {
        Err("Path not found".to_string())
    }
}

pub fn copy_dir_recursive(src: &Path, dst: &Path) -> Result<(), String> {
    fs::create_dir_all(dst).map_err(|e| e.to_string())?;
    for entry in fs::read_dir(src).map_err(|e| e.to_string())?.flatten() {
        let target = dst.join(entry.file_name());
        if entry.path().is_dir() {
            copy_dir_recursive(&entry.path(), &target)?;
        } else {
            fs::copy(entry.path(), target).map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

pub fn copy_or_move(base_dir: &Path, sources: &[String], dest_dir: &str, move_items: bool) -> Result<(), String> {
    let dst_parent = resolve_safe_path(base_dir, dest_dir)?;
    fs::create_dir_all(&dst_parent).map_err(|e| e.to_string())?;
    for s in sources {
        let src = resolve_safe_path(base_dir, s)?;
        let Some(name) = src.file_name() else { continue };
        let dst = dst_parent.join(name);
        if move_items {
            fs::rename(&src, &dst).map_err(|e| e.to_string())?;
        } else if src.is_dir() {
            copy_dir_recursive(&src, &dst)?;
        } else if src.is_file() {
            fs::copy(&src, &dst).map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_file_manager_crud_and_traversal_block() {
        let temp_dir = std::env::temp_dir().join(format!("dockpanel_files_test_{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&temp_dir).unwrap();

        assert!(resolve_safe_path(&temp_dir, "../outside.txt").is_err());
        assert!(resolve_safe_path(&temp_dir, "sub/../../outside.txt").is_err());

        create_entry(&temp_dir, "hello.php", "file").unwrap();
        write_file_content(&temp_dir, "hello.php", "<?php echo 'ok';").unwrap();
        assert_eq!(read_file_content(&temp_dir, "hello.php").unwrap(), "<?php echo 'ok';");

        create_entry(&temp_dir, "assets", "dir").unwrap();
        let list = list_dir(&temp_dir, ".").unwrap();
        assert_eq!(list.len(), 2);
        assert!(list[0].is_dir);

        rename_entry(&temp_dir, "hello.php", "assets/index.php").unwrap();
        assert_eq!(read_file_content(&temp_dir, "assets/index.php").unwrap(), "<?php echo 'ok';");

        delete_entry(&temp_dir, "assets").unwrap();
        assert!(list_dir(&temp_dir, ".").unwrap().is_empty());

        let _ = fs::remove_dir_all(&temp_dir);
    }
}
