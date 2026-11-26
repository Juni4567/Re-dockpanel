use base64::{engine::general_purpose::STANDARD as B64, Engine};
use flate2::read::GzDecoder;
use serde::{Deserialize, Serialize};
use std::fs;
use std::io::{Cursor, Read, Write};
use std::path::Path;
use std::sync::Arc;
use zip::write::SimpleFileOptions;

use crate::state::AppState;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PreflightCheckRequest {
    pub remote_url: String,
    pub api_token: String,
    #[serde(default)]
    pub remote_site_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PreflightCheckResponse {
    pub success: bool,
    pub message: String,
    #[serde(default)]
    pub remote_site_found: bool,
    #[serde(default)]
    pub remote_version: Option<String>,
    #[serde(default)]
    pub remote_domain: Option<String>,
    #[serde(default)]
    pub remote_site_id: Option<String>,
    #[serde(default)]
    pub can_push: bool,
    #[serde(default)]
    pub can_pull: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncRequest {
    pub remote_url: String,
    pub api_token: String,
    #[serde(default)]
    pub sync_files: bool,
    #[serde(default)]
    pub sync_db: bool,
    #[serde(default)]
    pub dry_run: bool,
    #[serde(default)]
    pub remote_site_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncProgress {
    pub step: String,
    pub label: String,
    pub status: String,
    #[serde(default)]
    pub message: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncResponse {
    pub sync_id: String,
    pub message: String,
    pub status: String,
    #[serde(default)]
    pub progress: Vec<SyncProgress>,
}

/// Builds domain replacement pairs ordered longest-first so URLs with scheme match
/// before bare hostnames.
pub fn build_domain_pairs(source: &str, target: &str) -> Vec<(String, String)> {
    let mut pairs = vec![(source.to_string(), target.to_string())];

    let s_bare = source
        .trim_start_matches("https://")
        .trim_start_matches("http://")
        .trim_start_matches("//")
        .trim_end_matches('/');
    let t_bare = target
        .trim_start_matches("https://")
        .trim_start_matches("http://")
        .trim_start_matches("//")
        .trim_end_matches('/');

    if s_bare != source || t_bare != target {
        for scheme in ["https://", "http://"] {
            let s_url = format!("{scheme}{s_bare}");
            let t_url = format!("{scheme}{t_bare}");
            if !pairs.iter().any(|(s, _)| s == &s_url) {
                pairs.push((s_url, t_url));
            }
        }
        if !pairs.iter().any(|(s, _)| s == s_bare) {
            pairs.push((s_bare.to_string(), t_bare.to_string()));
        }
    }

    pairs.sort_by(|a, b| b.0.len().cmp(&a.0.len()));
    pairs.dedup_by(|a, b| a.0 == b.0);
    pairs
}

fn replace_domain_content_single_pass(content: &str, pairs: &[(String, String)]) -> (String, bool) {
    let mut result = String::with_capacity(content.len() + 32);
    let total_len = content.len();
    let mut i = 0;
    let mut changed = false;

    while i < total_len {
        let mut matched = false;
        for (src, dst) in pairs {
            if content[i..].starts_with(src) {
                result.push_str(dst);
                i += src.len();
                matched = true;
                changed = true;
                break;
            }
        }
        if !matched {
            let ch = content[i..].chars().next().unwrap();
            result.push(ch);
            i += ch.len_utf8();
        }
    }
    (result, changed)
}

fn try_parse_serialized_string(s: &str) -> Option<(usize, &str, String, usize)> {
    if !s.starts_with("s:") {
        return None;
    }
    let rest = &s[2..];
    let colon_pos = rest.find(':')?;
    let old_len = rest[..colon_pos].parse::<usize>().ok()?;
    let after_colon = &rest[colon_pos + 1..];
    let q_str = if after_colon.starts_with("\\\"") {
        "\\\""
    } else if after_colon.starts_with('"') {
        "\""
    } else {
        return None;
    };

    let content_start = 2 + colon_pos + 1 + q_str.len();
    let expected_end = content_start + old_len;
    if expected_end <= s.len()
        && s.is_char_boundary(expected_end)
        && s[expected_end..].starts_with(q_str)
    {
        return Some((
            old_len,
            q_str,
            s[content_start..expected_end].to_string(),
            expected_end + q_str.len(),
        ));
    }

    let search_slice = &s[content_start..];
    let mut offset = 0;
    while offset < search_slice.len() {
        if search_slice.is_char_boundary(offset) && search_slice[offset..].starts_with(q_str) {
            if q_str == "\"" && offset > 0 && search_slice.as_bytes()[offset - 1] == b'\\' {
                offset += 1;
                continue;
            }
            return Some((
                old_len,
                q_str,
                search_slice[..offset].to_string(),
                content_start + offset + q_str.len(),
            ));
        }
        offset += 1;
    }
    None
}

/// Serialization-safe domain rewrite engine for SQL dumps (updates PHP `s:<len>:"..."` byte counts).
pub fn rewrite_domains(sql: &str, source_domain: &str, target_domain: &str) -> String {
    if source_domain.is_empty() || source_domain == target_domain {
        return sql.to_string();
    }

    let pairs = build_domain_pairs(source_domain, target_domain);
    let mut result = String::with_capacity(sql.len() + 128);
    let bytes = sql.as_bytes();
    let total_len = bytes.len();
    let mut i = 0;

    while i < total_len {
        if bytes[i] == b's' && i + 2 < total_len && bytes[i + 1] == b':' {
            if let Some((old_len, q_str, content, match_len)) =
                try_parse_serialized_string(&sql[i..])
            {
                let (new_content, changed) = replace_domain_content_single_pass(&content, &pairs);
                if changed {
                    let old_content_len = content.len();
                    let new_content_len = new_content.len();
                    let delta = new_content_len as isize - old_content_len as isize;
                    let new_len = if old_len == old_content_len {
                        new_content_len
                    } else {
                        (old_len as isize + delta).max(0) as usize
                    };
                    result.push_str(&format!("s:{}:{}{}{}", new_len, q_str, new_content, q_str));
                } else {
                    result.push_str(&sql[i..i + match_len]);
                }
                i += match_len;
                continue;
            }
        }

        let mut replaced_plain = false;
        for (src, dst) in &pairs {
            if sql[i..].starts_with(src) {
                result.push_str(dst);
                i += src.len();
                replaced_plain = true;
                break;
            }
        }
        if replaced_plain {
            continue;
        }

        let ch = sql[i..].chars().next().unwrap();
        result.push(ch);
        i += ch.len_utf8();
    }
    result
}

fn preflight_error(msg: impl Into<String>) -> PreflightCheckResponse {
    PreflightCheckResponse {
        success: false,
        message: msg.into(),
        remote_site_found: false,
        remote_version: None,
        remote_domain: None,
        remote_site_id: None,
        can_push: false,
        can_pull: false,
    }
}

fn build_http_client(timeout_secs: u64) -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(timeout_secs))
        .build()
        .unwrap_or_default()
}

pub async fn check_preflight(
    req: &PreflightCheckRequest,
    local_domain: Option<&str>,
) -> PreflightCheckResponse {
    let raw_url = req.remote_url.trim();
    if raw_url.is_empty() {
        return preflight_error("Remote URL cannot be empty");
    }
    if !raw_url.starts_with("http://") && !raw_url.starts_with("https://") {
        return preflight_error("Remote URL must start with http:// or https://");
    }
    let token = req.api_token.trim();
    if token.is_empty() {
        return preflight_error("API token cannot be empty");
    }

    let base_url = raw_url.trim_end_matches('/');
    let client = build_http_client(15);

    let auth_res = match client
        .get(format!("{base_url}/api/auth/me"))
        .header("Authorization", format!("Bearer {token}"))
        .send()
        .await
    {
        Ok(r) => r,
        Err(e) => return preflight_error(format!("Could not connect to remote panel: {e}")),
    };

    if !auth_res.status().is_success() {
        return preflight_error(format!(
            "Authentication failed: remote server responded with status {}",
            auth_res.status()
        ));
    }

    let sites_url = match &req.remote_site_id {
        Some(id) if !id.trim().is_empty() => format!("{base_url}/api/sites/{}", id.trim()),
        _ => format!("{base_url}/api/sites"),
    };

    let mut remote_site_found = false;
    let mut remote_domain = None;
    let mut remote_site_id = req.remote_site_id.clone().filter(|s| !s.trim().is_empty());

    if let Ok(sites_res) = client
        .get(&sites_url)
        .header("Authorization", format!("Bearer {token}"))
        .send()
        .await
    {
        if sites_res.status().is_success() {
            if let Ok(body) = sites_res.json::<serde_json::Value>().await {
                if remote_site_id.is_some() && body.is_object() {
                    if let Some(dom) = body.get("domain").and_then(|v| v.as_str()) {
                        remote_site_found = true;
                        remote_domain = Some(dom.to_string());
                        if let Some(id_str) = body.get("id").and_then(|v| v.as_str()) {
                            remote_site_id = Some(id_str.to_string());
                        }
                    }
                } else if let Some(arr) = body.as_array() {
                    let target_stem = local_domain
                        .map(|d| {
                            d.trim_end_matches(".test")
                                .trim_end_matches(".local")
                                .to_lowercase()
                        })
                        .unwrap_or_default();
                    let found = arr
                        .iter()
                        .find(|s| {
                            !target_stem.is_empty()
                                && s.get("domain")
                                    .and_then(|d| d.as_str())
                                    .map(|d| d.to_lowercase().contains(&target_stem))
                                    .unwrap_or(false)
                        })
                        .or_else(|| arr.first());
                    if let Some(s) = found {
                        remote_site_found = true;
                        remote_domain = s
                            .get("domain")
                            .and_then(|d| d.as_str())
                            .map(|d| d.to_string());
                        remote_site_id = s
                            .get("id")
                            .and_then(|d| d.as_str())
                            .map(|d| d.to_string());
                    }
                }
            }
        }
    }

    let remote_version = client
        .get(format!("{base_url}/api/health"))
        .send()
        .await
        .ok()
        .and_then(|r| if r.status().is_success() { Some(r) } else { None })
        .and_then(|_| None::<String>);

    PreflightCheckResponse {
        success: true,
        message: if remote_site_found {
            "Connected to remote server and matched site".to_string()
        } else {
            "Connected to remote server (no remote site matched yet)".to_string()
        },
        remote_site_found,
        remote_version,
        remote_domain,
        remote_site_id,
        can_push: remote_site_found,
        can_pull: remote_site_found,
    }
}

async fn resolve_remote_database(
    client: &reqwest::Client,
    base_url: &str,
    token: &str,
    remote_site_id: &str,
    remote_domain: &str,
) -> Option<String> {
    let url = format!("{base_url}/api/databases");
    let res = client
        .get(&url)
        .header("Authorization", format!("Bearer {token}"))
        .send()
        .await
        .ok()?;
    if !res.status().is_success() {
        return None;
    }
    let arr = res.json::<serde_json::Value>().await.ok()?;
    let list = arr.as_array()?;
    let matched = list
        .iter()
        .find(|d| d.get("site_id").and_then(|v| v.as_str()) == Some(remote_site_id))
        .or_else(|| {
            let stem = remote_domain.split('.').next().unwrap_or(remote_domain);
            list.iter().find(|d| {
                d.get("name")
                    .and_then(|v| v.as_str())
                    .map(|n| n.contains(stem))
                    .unwrap_or(false)
            })
        });
    matched
        .and_then(|d| d.get("id").and_then(|v| v.as_str()))
        .map(String::from)
}

fn decode_maybe_gzip_sql(bytes: &[u8]) -> Result<String, String> {
    if bytes.starts_with(&[0x1f, 0x8b]) {
        let mut decoder = GzDecoder::new(bytes);
        let mut out = String::new();
        decoder
            .read_to_string(&mut out)
            .map_err(|e| format!("Failed decompressing gzipped SQL dump: {e}"))?;
        Ok(out)
    } else {
        Ok(String::from_utf8_lossy(bytes).to_string())
    }
}

fn find_local_mariadb_db_for_site(site_dir: &Path, mariadb_port: Option<u16>) -> Option<String> {
    if let Some(db_name) = crate::wordpress::read_wp_mysql_db_name(site_dir) {
        return Some(db_name);
    }
    let port = mariadb_port?;
    let user_dbs = crate::mariadb::list_user_databases(port).ok()?;
    let folder = site_dir.file_name()?.to_string_lossy().to_lowercase();
    let stem = folder.replace(['.', '-'], "_");
    user_dbs
        .into_iter()
        .find(|name| name.eq_ignore_ascii_case(&stem) || name.to_lowercase().contains(&stem))
}

fn collect_zip_bytes(site_dir: &Path, skip_wp_config: bool) -> Result<Vec<u8>, String> {
    let buf = Vec::new();
    let cursor = Cursor::new(buf);
    let mut writer = zip::ZipWriter::new(cursor);
    let opts = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);

    fn walk_dir(
        root: &Path,
        current: &Path,
        writer: &mut zip::ZipWriter<Cursor<Vec<u8>>>,
        opts: SimpleFileOptions,
        skip_wp_config: bool,
    ) -> Result<(), String> {
        let entries = fs::read_dir(current).map_err(|e| e.to_string())?;
        for entry in entries.flatten() {
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().to_string();
            if name == ".git"
                || name == "node_modules"
                || name.starts_with(".dockpanel_sync")
            {
                continue;
            }
            if skip_wp_config && path.parent() == Some(root) && name.eq_ignore_ascii_case("wp-config.php") {
                continue;
            }
            let rel = path
                .strip_prefix(root)
                .map_err(|e| e.to_string())?
                .to_string_lossy()
                .replace('\\', "/");
            if path.is_dir() {
                writer
                    .add_directory(format!("{rel}/"), opts)
                    .map_err(|e| e.to_string())?;
                walk_dir(root, &path, writer, opts, skip_wp_config)?;
            } else if path.is_file() {
                writer.start_file(&rel, opts).map_err(|e| e.to_string())?;
                let bytes = fs::read(&path).map_err(|e| e.to_string())?;
                writer.write_all(&bytes).map_err(|e| e.to_string())?;
            }
        }
        Ok(())
    }

    walk_dir(site_dir, site_dir, &mut writer, opts, skip_wp_config)?;
    let cursor = writer.finish().map_err(|e| e.to_string())?;
    Ok(cursor.into_inner())
}

fn extract_zip_preserving_wp_config(zip_bytes: &[u8], site_dir: &Path) -> Result<usize, String> {
    fs::create_dir_all(site_dir).map_err(|e| e.to_string())?;
    let wp_config_path = site_dir.join("wp-config.php");
    let preserved_wp_config = if wp_config_path.is_file() {
        fs::read(&wp_config_path).ok()
    } else {
        None
    };

    let reader = Cursor::new(zip_bytes);
    let mut archive = zip::ZipArchive::new(reader).map_err(|e| format!("Invalid ZIP: {e}"))?;
    let mut extracted = 0usize;

    for i in 0..archive.len() {
        let mut file = archive.by_index(i).map_err(|e| e.to_string())?;
        let Some(rel_path) = file.enclosed_name().map(|p| p.to_path_buf()) else {
            continue;
        };
        if rel_path
            .file_name()
            .and_then(|n| n.to_str())
            .map(|n| n.starts_with(".dockpanel_sync"))
            .unwrap_or(false)
        {
            continue;
        }
        let target = site_dir.join(&rel_path);
        if file.is_dir() {
            fs::create_dir_all(&target).map_err(|e| e.to_string())?;
        } else {
            if let Some(parent) = target.parent() {
                fs::create_dir_all(parent).map_err(|e| e.to_string())?;
            }
            let mut out = fs::File::create(&target).map_err(|e| e.to_string())?;
            std::io::copy(&mut file, &mut out).map_err(|e| e.to_string())?;
            extracted += 1;
        }
    }

    if let Some(local_cfg) = preserved_wp_config {
        let _ = fs::write(&wp_config_path, local_cfg);
    }

    Ok(extracted)
}

pub async fn execute_pull(
    state: Arc<AppState>,
    site_id: &str,
    req: &SyncRequest,
) -> Result<SyncResponse, (u16, String)> {
    let site = state
        .get_site(site_id)
        .ok_or((404, "Site not found".to_string()))?;
    let local_domain = site.domain.clone();
    let site_dir = state.site_dir(&site);

    let preflight = check_preflight(
        &PreflightCheckRequest {
            remote_url: req.remote_url.clone(),
            api_token: req.api_token.clone(),
            remote_site_id: req.remote_site_id.clone(),
        },
        Some(&local_domain),
    )
    .await;

    if !preflight.success {
        return Err((400, format!("Preflight check failed: {}", preflight.message)));
    }

    let remote_site_id = req
        .remote_site_id
        .clone()
        .filter(|s| !s.trim().is_empty())
        .or(preflight.remote_site_id.clone())
        .ok_or((
            400,
            "No matching remote site found on server for pull".to_string(),
        ))?;

    let remote_domain = preflight
        .remote_domain
        .clone()
        .unwrap_or_else(|| local_domain.replace(".test", ".com").replace(".local", ".com"));
    let sync_id = uuid::Uuid::new_v4().to_string();

    if req.dry_run {
        let mut progress = vec![SyncProgress {
            step: "preflight".into(),
            label: "Remote server preflight verified".into(),
            status: "done".into(),
            message: Some(format!("Matched remote site {} ({})", remote_domain, remote_site_id)),
        }];
        if req.sync_db {
            progress.push(SyncProgress {
                step: "db_preview".into(),
                label: "Database domain rewrite preview".into(),
                status: "done".into(),
                message: Some(format!("Would export remote DB and rewrite {} -> {}", remote_domain, local_domain)),
            });
        }
        if req.sync_files {
            progress.push(SyncProgress {
                step: "files_preview".into(),
                label: "File synchronization preview".into(),
                status: "done".into(),
                message: Some(format!("Would download archive from {} into {}", remote_domain, site_dir.display())),
            });
        }
        progress.push(SyncProgress {
            step: "complete".into(),
            label: "Dry run pull simulation completed".into(),
            status: "done".into(),
            message: Some("No local or remote data modified".into()),
        });
        return Ok(SyncResponse {
            sync_id,
            message: "Dry run pull sync simulation completed".to_string(),
            status: "dry_run_completed".to_string(),
            progress,
        });
    }

    let base_url = req.remote_url.trim().trim_end_matches('/');
    let token = req.api_token.trim();
    let client = build_http_client(180);

    let mut progress = vec![SyncProgress {
        step: "preflight".into(),
        label: "Verified remote server connection".into(),
        status: "done".into(),
        message: Some(format!("Connected to {} ({})", remote_domain, remote_site_id)),
    }];

    if req.sync_db {
        let r_db_id = resolve_remote_database(&client, base_url, token, &remote_site_id, &remote_domain)
            .await
            .ok_or((
                400,
                format!("No remote database found for site '{}'", remote_domain),
            ))?;

        let export_res = client
            .post(format!("{base_url}/api/databases/{r_db_id}/export"))
            .header("Authorization", format!("Bearer {token}"))
            .send()
            .await
            .map_err(|e| (502, format!("Failed triggering remote DB export: {e}")))?;
        if !export_res.status().is_success() {
            return Err((
                502,
                format!("Remote DB export failed (HTTP {})", export_res.status()),
            ));
        }

        let dumps_res = client
            .get(format!("{base_url}/api/databases/{r_db_id}/dumps"))
            .header("Authorization", format!("Bearer {token}"))
            .send()
            .await
            .map_err(|e| (502, format!("Failed listing remote DB dumps: {e}")))?;
        let dumps_json: serde_json::Value = dumps_res
            .json()
            .await
            .map_err(|e| (502, format!("Invalid remote dumps response: {e}")))?;
        let filename = dumps_json
            .as_array()
            .and_then(|arr| arr.first())
            .and_then(|item| item.get("filename").or_else(|| item.get("name")))
            .and_then(|v| v.as_str())
            .ok_or((502, "Remote DB export completed but no dump file was returned".to_string()))?
            .to_string();

        let dl_res = client
            .get(format!("{base_url}/api/databases/{r_db_id}/dumps/{filename}/download"))
            .header("Authorization", format!("Bearer {token}"))
            .send()
            .await
            .map_err(|e| (502, format!("Failed downloading remote DB dump: {e}")))?;
        if !dl_res.status().is_success() {
            return Err((
                502,
                format!("Failed downloading remote DB dump (HTTP {})", dl_res.status()),
            ));
        }
        let raw_bytes = dl_res
            .bytes()
            .await
            .map_err(|e| (502, format!("Failed reading remote DB dump body: {e}")))?;
        let raw_sql = decode_maybe_gzip_sql(&raw_bytes).map_err(|e| (500, e))?;

        progress.push(SyncProgress {
            step: "db_pull".into(),
            label: "Downloaded remote database dump".into(),
            status: "done".into(),
            message: Some(format!("Fetched {} ({} bytes)", filename, raw_sql.len())),
        });

        let rewritten_sql = rewrite_domains(&raw_sql, &remote_domain, &local_domain);
        progress.push(SyncProgress {
            step: "domain_rewrite".into(),
            label: "Rewrote URLs & serialized PHP metadata".into(),
            status: "done".into(),
            message: Some(format!("{} -> {}", remote_domain, local_domain)),
        });

        let mdb_port = state.get_mariadb_port();
        let local_db = find_local_mariadb_db_for_site(&site_dir, mdb_port);
        if let (Some(port), Some(db_name)) = (mdb_port, local_db) {
            let tmp_sql = std::env::temp_dir().join(format!("dp_pull_{}.sql", uuid::Uuid::new_v4()));
            fs::write(&tmp_sql, &rewritten_sql).map_err(|e| (500, e.to_string()))?;
            let import_res = tokio::task::spawn_blocking(move || {
                let r = crate::mariadb::import_mysql_dump(port, &db_name, &tmp_sql);
                let _ = fs::remove_file(&tmp_sql);
                r
            })
            .await
            .map_err(|e| (500, e.to_string()))?;
            import_res.map_err(|e| (500, format!("Local MariaDB import failed: {e}")))?;
            progress.push(SyncProgress {
                step: "db_restore".into(),
                label: "Imported rewritten SQL into local MariaDB".into(),
                status: "done".into(),
                message: None,
            });
        } else {
            let staged_path = site_dir.join(".dockpanel_pull_sync.sql");
            let _ = fs::create_dir_all(&site_dir);
            fs::write(&staged_path, &rewritten_sql).map_err(|e| (500, e.to_string()))?;
            progress.push(SyncProgress {
                step: "db_restore".into(),
                label: "Staged rewritten SQL dump locally".into(),
                status: "done".into(),
                message: Some(format!(
                    "No active local MariaDB database linked; saved to {}",
                    staged_path.display()
                )),
            });
        }
    }

    if req.sync_files {
        let temp_archive = ".dockpanel_sync_pull.zip";
        let comp_res = client
            .post(format!("{base_url}/api/sites/{remote_site_id}/files/compress"))
            .header("Authorization", format!("Bearer {token}"))
            .json(&serde_json::json!({
                "sources": ["."],
                "destination": temp_archive,
                "format": "zip"
            }))
            .send()
            .await
            .map_err(|e| (502, format!("Failed compressing remote site files: {e}")))?;
        if !comp_res.status().is_success() {
            return Err((
                502,
                format!("Remote file compression failed (HTTP {})", comp_res.status()),
            ));
        }

        let dl_res = client
            .get(format!(
                "{base_url}/api/sites/{remote_site_id}/files/download?path={temp_archive}"
            ))
            .header("Authorization", format!("Bearer {token}"))
            .send()
            .await
            .map_err(|e| (502, format!("Failed downloading remote files archive: {e}")))?;
        if !dl_res.status().is_success() {
            let _ = client
                .delete(format!(
                    "{base_url}/api/sites/{remote_site_id}/files?path={temp_archive}"
                ))
                .header("Authorization", format!("Bearer {token}"))
                .send()
                .await;
            return Err((
                502,
                format!("Remote file archive download failed (HTTP {})", dl_res.status()),
            ));
        }
        let zip_bytes = dl_res
            .bytes()
            .await
            .map_err(|e| (502, format!("Failed reading remote archive stream: {e}")))?;

        let _ = client
            .delete(format!(
                "{base_url}/api/sites/{remote_site_id}/files?path={temp_archive}"
            ))
            .header("Authorization", format!("Bearer {token}"))
            .send()
            .await;

        let target_dir = site_dir.clone();
        let extracted = tokio::task::spawn_blocking(move || {
            extract_zip_preserving_wp_config(&zip_bytes, &target_dir)
        })
        .await
        .map_err(|e| (500, e.to_string()))?
        .map_err(|e| (500, e))?;

        progress.push(SyncProgress {
            step: "file_sync".into(),
            label: "Synchronized remote site files locally".into(),
            status: "done".into(),
            message: Some(format!("Extracted {} files into {}", extracted, site_dir.display())),
        });
    }

    progress.push(SyncProgress {
        step: "complete".into(),
        label: "Pull sync completed successfully".into(),
        status: "done".into(),
        message: Some(format!("{} synchronized from {}", local_domain, remote_domain)),
    });

    state.record_event(
        "live_sync_pull",
        "sync",
        &local_domain,
        "Pulled from Live Server",
        &format!("Synced {} <- {}", local_domain, remote_domain),
        "success",
    );

    Ok(SyncResponse {
        sync_id,
        message: "Pull sync completed successfully".to_string(),
        status: "completed".to_string(),
        progress,
    })
}

pub async fn execute_push(
    state: Arc<AppState>,
    site_id: &str,
    req: &SyncRequest,
) -> Result<SyncResponse, (u16, String)> {
    let site = state
        .get_site(site_id)
        .ok_or((404, "Site not found".to_string()))?;
    let local_domain = site.domain.clone();
    let site_dir = state.site_dir(&site);

    let preflight = check_preflight(
        &PreflightCheckRequest {
            remote_url: req.remote_url.clone(),
            api_token: req.api_token.clone(),
            remote_site_id: req.remote_site_id.clone(),
        },
        Some(&local_domain),
    )
    .await;

    if !preflight.success {
        return Err((400, format!("Preflight check failed: {}", preflight.message)));
    }

    let remote_site_id = req
        .remote_site_id
        .clone()
        .filter(|s| !s.trim().is_empty())
        .or(preflight.remote_site_id.clone())
        .ok_or((
            400,
            "Remote site ID could not be resolved for safety snapshot. Aborting push.".to_string(),
        ))?;

    let remote_domain = preflight
        .remote_domain
        .clone()
        .unwrap_or_else(|| local_domain.replace(".test", ".com").replace(".local", ".com"));
    let sync_id = uuid::Uuid::new_v4().to_string();

    if req.dry_run {
        let mut progress = vec![
            SyncProgress {
                step: "preflight".into(),
                label: "Remote server preflight verified".into(),
                status: "done".into(),
                message: Some(format!("Matched remote site {} ({})", remote_domain, remote_site_id)),
            },
            SyncProgress {
                step: "safety_snapshot".into(),
                label: "Remote safety snapshot check".into(),
                status: "done".into(),
                message: Some("Would create pre-flight remote backup before any overwrite".into()),
            },
        ];
        if req.sync_db {
            progress.push(SyncProgress {
                step: "db_preview".into(),
                label: "Database domain rewrite preview".into(),
                status: "done".into(),
                message: Some(format!("Would rewrite {} -> {} and import remotely", local_domain, remote_domain)),
            });
        }
        if req.sync_files {
            progress.push(SyncProgress {
                step: "files_preview".into(),
                label: "File push preview".into(),
                status: "done".into(),
                message: Some(format!("Would package {} and extract remotely (preserving remote wp-config.php)", site_dir.display())),
            });
        }
        progress.push(SyncProgress {
            step: "complete".into(),
            label: "Dry run push simulation completed".into(),
            status: "done".into(),
            message: Some("No changes were made to remote server".into()),
        });
        return Ok(SyncResponse {
            sync_id,
            message: "Dry run push sync simulation completed".to_string(),
            status: "dry_run_completed".to_string(),
            progress,
        });
    }

    let base_url = req.remote_url.trim().trim_end_matches('/');
    let token = req.api_token.trim();
    let client = build_http_client(180);

    // 1. Mandatory pre-flight safety snapshot on remote server before overwriting anything
    let backup_res = client
        .post(format!("{base_url}/api/sites/{remote_site_id}/backups"))
        .header("Authorization", format!("Bearer {token}"))
        .send()
        .await
        .map_err(|e| {
            (
                502,
                format!("Failed creating remote pre-flight safety snapshot: {e}"),
            )
        })?;
    if !backup_res.status().is_success() {
        return Err((
            502,
            format!(
                "Remote safety snapshot failed (HTTP {}). Aborting push to prevent data loss.",
                backup_res.status()
            ),
        ));
    }

    let mut progress = vec![
        SyncProgress {
            step: "preflight".into(),
            label: "Verified remote server connection".into(),
            status: "done".into(),
            message: Some(format!("Connected to {}", remote_domain)),
        },
        SyncProgress {
            step: "safety_snapshot".into(),
            label: "Created remote pre-flight safety snapshot".into(),
            status: "done".into(),
            message: Some(format!("Snapshot created for remote site {}", remote_site_id)),
        },
    ];

    if req.sync_db {
        let mdb_port = state.get_mariadb_port();
        let local_db_name = find_local_mariadb_db_for_site(&site_dir, mdb_port);
        let local_sql = if let (Some(port), Some(db_name)) = (mdb_port, local_db_name) {
            let tmp_sql = std::env::temp_dir().join(format!("dp_push_{}.sql", uuid::Uuid::new_v4()));
            let out_path = tmp_sql.clone();
            let export_res = tokio::task::spawn_blocking(move || {
                crate::mariadb::export_mysql_dump(port, &db_name, &out_path)
            })
            .await
            .map_err(|e| (500, e.to_string()))?;
            export_res.map_err(|e| (500, format!("Local MariaDB export failed: {e}")))?;
            let content = fs::read_to_string(&tmp_sql).map_err(|e| (500, e.to_string()))?;
            let _ = fs::remove_file(&tmp_sql);
            content
        } else {
            return Err((
                400,
                "No local MariaDB database found for this site to push. Uncheck 'Sync Database' to push files only."
                    .to_string(),
            ));
        };

        let rewritten_sql = rewrite_domains(&local_sql, &local_domain, &remote_domain);
        progress.push(SyncProgress {
            step: "domain_rewrite".into(),
            label: "Rewrote URLs & serialized PHP metadata".into(),
            status: "done".into(),
            message: Some(format!("{} -> {}", local_domain, remote_domain)),
        });

        let r_db_id = resolve_remote_database(&client, base_url, token, &remote_site_id, &remote_domain)
            .await
            .ok_or((
                400,
                format!("No remote database found for site '{}'", remote_domain),
            ))?;

        let upload_res = client
            .post(format!("{base_url}/api/databases/{r_db_id}/upload-dump"))
            .header("Authorization", format!("Bearer {token}"))
            .json(&serde_json::json!({
                "filename": "dockpanel_live_sync.sql",
                "content": B64.encode(rewritten_sql.as_bytes()),
                "auto_import": true,
            }))
            .send()
            .await
            .map_err(|e| (502, format!("Failed uploading database dump to remote server: {e}")))?;

        if !upload_res.status().is_success() {
            let err_text = upload_res.text().await.unwrap_or_default();
            return Err((502, format!("Remote database import failed: {err_text}")));
        }

        progress.push(SyncProgress {
            step: "db_restore".into(),
            label: "Uploaded & imported database to remote server".into(),
            status: "done".into(),
            message: Some(format!("Imported into remote database {}", r_db_id)),
        });
    }

    if req.sync_files {
        let src_dir = site_dir.clone();
        let zip_bytes = tokio::task::spawn_blocking(move || collect_zip_bytes(&src_dir, true))
            .await
            .map_err(|e| (500, e.to_string()))?
            .map_err(|e| (500, e))?;

        let temp_archive = ".dockpanel_sync_push.zip";
        let upload_res = client
            .post(format!("{base_url}/api/sites/{remote_site_id}/files/upload"))
            .header("Authorization", format!("Bearer {token}"))
            .json(&serde_json::json!({
                "path": ".",
                "filename": temp_archive,
                "content": B64.encode(&zip_bytes),
            }))
            .send()
            .await
            .map_err(|e| (502, format!("Failed uploading site archive: {e}")))?;

        if !upload_res.status().is_success() {
            let err_text = upload_res.text().await.unwrap_or_default();
            return Err((502, format!("Remote file upload failed: {err_text}")));
        }

        let extract_res = client
            .post(format!("{base_url}/api/sites/{remote_site_id}/files/extract"))
            .header("Authorization", format!("Bearer {token}"))
            .json(&serde_json::json!({
                "archive": temp_archive,
                "destination": "."
            }))
            .send()
            .await
            .map_err(|e| (502, format!("Failed extracting remote site archive: {e}")))?;

        let _ = client
            .delete(format!(
                "{base_url}/api/sites/{remote_site_id}/files?path={temp_archive}"
            ))
            .header("Authorization", format!("Bearer {token}"))
            .send()
            .await;

        if !extract_res.status().is_success() {
            return Err((
                502,
                format!("Remote file extraction failed (HTTP {})", extract_res.status()),
            ));
        }

        progress.push(SyncProgress {
            step: "file_sync".into(),
            label: "Pushed & extracted site files remotely".into(),
            status: "done".into(),
            message: Some(format!("Uploaded {} bytes to {}", zip_bytes.len(), remote_domain)),
        });
    }

    progress.push(SyncProgress {
        step: "complete".into(),
        label: "Push sync completed successfully".into(),
        status: "done".into(),
        message: Some(format!("{} pushed to {}", local_domain, remote_domain)),
    });

    state.record_event(
        "live_sync_push",
        "sync",
        &local_domain,
        "Pushed to Live Server",
        &format!("Synced {} -> {}", local_domain, remote_domain),
        "success",
    );

    Ok(SyncResponse {
        sync_id,
        message: "Push sync completed successfully".to_string(),
        status: "completed".to_string(),
        progress,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{routing::{get, post}, Json, Router};
    use std::sync::atomic::{AtomicBool, Ordering};

    #[test]
    fn test_rewrite_domains_serialized_and_plain() {
        let sql = concat!(
            "INSERT INTO wp_options VALUES (1, 'siteurl', 'https://mysite.test');\n",
            "INSERT INTO wp_options VALUES (2, 'theme', 'a:1:{s:3:\"url\";s:19:\"https://mysite.test\";}');\n",
            "INSERT INTO wp_options VALUES (3, 'esc', 'a:1:{s:3:\\\"url\\\";s:19:\\\"https://mysite.test\\\";}');"
        );
        let out = rewrite_domains(sql, "https://mysite.test", "https://production.example.com");
        assert!(out.contains("'https://production.example.com'"));
        assert!(out.contains("s:30:\"https://production.example.com\""));
        assert!(out.contains("s:30:\\\"https://production.example.com\\\""));
    }

    #[test]
    fn test_rewrite_domains_no_double_replace_on_subdomain() {
        let sql = "s:19:\"https://example.com\";";
        let out = rewrite_domains(sql, "https://example.com", "https://sub.example.com");
        assert_eq!(out, "s:23:\"https://sub.example.com\";");
    }

    #[tokio::test]
    async fn test_preflight_and_push_safety_snapshot_enforcement() {
        let snapshot_called = Arc::new(AtomicBool::new(false));
        let fail_snapshot = Arc::new(AtomicBool::new(true));

        let snap_flag = snapshot_called.clone();
        let fail_flag = fail_snapshot.clone();

        let app = Router::new()
            .route(
                "/api/auth/me",
                get(|| async { Json(serde_json::json!({ "id": "u1", "role": "admin" })) }),
            )
            .route(
                "/api/sites",
                get(|| async {
                    Json(serde_json::json!([
                        { "id": "remote-1", "domain": "mysite.com" }
                    ]))
                }),
            )
            .route(
                "/api/sites/{id}/backups",
                post(move || {
                    let sf = snap_flag.clone();
                    let ff = fail_flag.clone();
                    async move {
                        sf.store(true, Ordering::SeqCst);
                        if ff.load(Ordering::SeqCst) {
                            (axum::http::StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({"error": "snap fail"})))
                        } else {
                            (axum::http::StatusCode::OK, Json(serde_json::json!({"ok": true})))
                        }
                    }
                }),
            );

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });

        let base_url = format!("http://127.0.0.1:{}", addr.port());
        let pre = check_preflight(
            &PreflightCheckRequest {
                remote_url: base_url.clone(),
                api_token: "dp_pat_test".into(),
                remote_site_id: None,
            },
            Some("mysite.test"),
        )
        .await;
        assert!(pre.success);
        assert!(pre.remote_site_found);
        assert_eq!(pre.remote_domain.as_deref(), Some("mysite.com"));
        assert_eq!(pre.remote_site_id.as_deref(), Some("remote-1"));

        let tmp = std::env::temp_dir().join(format!("dp_sync_test_{}", uuid::Uuid::new_v4()));
        let state = Arc::new(AppState::new_with_dirs(tmp.join("cfg"), tmp.join("sites")));
        let created = state
            .create_site("mysite.test", Some("8.2"))
            .unwrap();

        // Push MUST abort when remote safety snapshot fails
        let push_err = execute_push(
            state.clone(),
            &created.id,
            &SyncRequest {
                remote_url: base_url.clone(),
                api_token: "dp_pat_test".into(),
                sync_files: false,
                sync_db: false,
                dry_run: false,
                remote_site_id: None,
            },
        )
        .await;
        assert!(snapshot_called.load(Ordering::SeqCst));
        assert!(push_err.is_err());

        // Dry run pull & push succeed without modifying local/remote state
        let dry_pull = execute_pull(
            state.clone(),
            &created.id,
            &SyncRequest {
                remote_url: base_url.clone(),
                api_token: "dp_pat_test".into(),
                sync_files: true,
                sync_db: true,
                dry_run: true,
                remote_site_id: None,
            },
        )
        .await
        .expect("dry run pull should succeed");
        assert_eq!(dry_pull.status, "dry_run_completed");

        fail_snapshot.store(false, Ordering::SeqCst);
        let dry_push = execute_push(
            state,
            &created.id,
            &SyncRequest {
                remote_url: base_url,
                api_token: "dp_pat_test".into(),
                sync_files: true,
                sync_db: true,
                dry_run: true,
                remote_site_id: None,
            },
        )
        .await
        .expect("dry run push should succeed");
        assert_eq!(dry_push.status, "dry_run_completed");

        let _ = fs::remove_dir_all(&tmp);
    }

    #[tokio::test]
    async fn test_pull_and_push_files_roundtrip_preserves_wp_config() {
        let uploaded_zip_b64: Arc<std::sync::Mutex<Option<String>>> =
            Arc::new(std::sync::Mutex::new(None));
        let extracted = Arc::new(AtomicBool::new(false));

        // Build remote zip containing index.php + remote wp-config.php
        let remote_zip = {
            let buf = Vec::new();
            let mut w = zip::ZipWriter::new(Cursor::new(buf));
            let opts = SimpleFileOptions::default();
            w.start_file("index.php", opts).unwrap();
            w.write_all(b"<?php echo 'from remote';").unwrap();
            w.start_file("wp-config.php", opts).unwrap();
            w.write_all(b"<?php // REMOTE_PROD_CREDS").unwrap();
            w.finish().unwrap().into_inner()
        };

        let remote_zip_dl = remote_zip.clone();
        let up_store = uploaded_zip_b64.clone();
        let ext_flag = extracted.clone();

        let app = Router::new()
            .route(
                "/api/auth/me",
                get(|| async { Json(serde_json::json!({ "id": "u1", "role": "admin" })) }),
            )
            .route(
                "/api/sites",
                get(|| async {
                    Json(serde_json::json!([
                        { "id": "remote-1", "domain": "mysite.com" }
                    ]))
                }),
            )
            .route(
                "/api/sites/{id}/backups",
                post(|| async { Json(serde_json::json!({"ok": true})) }),
            )
            .route(
                "/api/sites/{id}/files/compress",
                post(|| async { Json(serde_json::json!({"ok": true})) }),
            )
            .route(
                "/api/sites/{id}/files/download",
                get(move || {
                    let bytes = remote_zip_dl.clone();
                    async move { bytes }
                }),
            )
            .route(
                "/api/sites/{id}/files",
                axum::routing::delete(|| async { Json(serde_json::json!({"ok": true})) }),
            )
            .route(
                "/api/sites/{id}/files/upload",
                post(move |Json(body): Json<serde_json::Value>| {
                    let store = up_store.clone();
                    async move {
                        if let Some(c) = body.get("content").and_then(|v| v.as_str()) {
                            *store.lock().unwrap() = Some(c.to_string());
                        }
                        Json(serde_json::json!({"ok": true}))
                    }
                }),
            )
            .route(
                "/api/sites/{id}/files/extract",
                post(move || {
                    let flag = ext_flag.clone();
                    async move {
                        flag.store(true, Ordering::SeqCst);
                        Json(serde_json::json!({"ok": true}))
                    }
                }),
            );

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });

        let tmp = std::env::temp_dir().join(format!("dp_sync_rt_{}", uuid::Uuid::new_v4()));
        let state = Arc::new(AppState::new_with_dirs(tmp.join("cfg"), tmp.join("sites")));
        let created = state.create_site("mysite.test", Some("8.2")).unwrap();
        let site_dir = state.site_dir(&created);
        fs::write(site_dir.join("wp-config.php"), "<?php // LOCAL_DEV_CREDS").unwrap();

        let base_url = format!("http://127.0.0.1:{}", addr.port());
        let pull_res = execute_pull(
            state.clone(),
            &created.id,
            &SyncRequest {
                remote_url: base_url.clone(),
                api_token: "dp_pat_test".into(),
                sync_files: true,
                sync_db: false,
                dry_run: false,
                remote_site_id: None,
            },
        )
        .await
        .expect("pull should succeed");
        assert_eq!(pull_res.status, "completed");
        assert_eq!(
            fs::read_to_string(site_dir.join("index.php")).unwrap(),
            "<?php echo 'from remote';"
        );
        // Local wp-config.php MUST be preserved on pull
        assert_eq!(
            fs::read_to_string(site_dir.join("wp-config.php")).unwrap(),
            "<?php // LOCAL_DEV_CREDS"
        );

        let push_res = execute_push(
            state,
            &created.id,
            &SyncRequest {
                remote_url: base_url,
                api_token: "dp_pat_test".into(),
                sync_files: true,
                sync_db: false,
                dry_run: false,
                remote_site_id: None,
            },
        )
        .await
        .expect("push should succeed");
        assert_eq!(push_res.status, "completed");
        assert!(extracted.load(Ordering::SeqCst));

        // Verify pushed archive omitted local wp-config.php and included index.php
        let pushed_b64 = uploaded_zip_b64.lock().unwrap().clone().unwrap();
        let pushed_bytes = B64.decode(pushed_b64).unwrap();
        let archive = zip::ZipArchive::new(Cursor::new(pushed_bytes)).unwrap();
        let names: Vec<String> = archive.file_names().map(String::from).collect();
        assert!(names.iter().any(|n| n == "index.php"));
        assert!(!names.iter().any(|n| n == "wp-config.php"));

        let _ = fs::remove_dir_all(&tmp);
    }
}
