use serde_json::json;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use crate::downloads;
use crate::state::AppState;

const RELEASES_API_URL: &str = "https://api.github.com/repos/Juni4567/Re-dockpanel/releases?per_page=12";
const CACHE_TTL_SECS: u64 = 900;

pub fn parse_semver(raw: &str) -> Option<(u64, u64, u64)> {
    let trimmed = raw
        .trim()
        .trim_start_matches("desktop-v")
        .trim_start_matches('v');
    let core = trimmed.split(['-', '+']).next().unwrap_or("");
    let mut parts = core.split('.');
    let major = parts.next()?.parse::<u64>().ok()?;
    let minor = parts.next()?.parse::<u64>().ok()?;
    let patch = parts.next().unwrap_or("0").parse::<u64>().ok()?;
    Some((major, minor, patch))
}

pub fn is_newer_version(current: &str, candidate: &str) -> bool {
    match (parse_semver(current), parse_semver(candidate)) {
        (Some(cur), Some(cand)) => cand > cur,
        _ => false,
    }
}

pub fn extract_sha256_from_notes(body: &str) -> Option<String> {
    for token in body.split(|c: char| !c.is_ascii_hexdigit()) {
        if token.len() == 64 {
            return Some(token.to_ascii_uppercase());
        }
    }
    None
}

pub fn pick_latest_desktop_release(releases: &[serde_json::Value], current_version: &str) -> serde_json::Value {
    let mut best: Option<((u64, u64, u64), serde_json::Value)> = None;

    for rel in releases {
        if rel["draft"].as_bool() == Some(true) || rel["prerelease"].as_bool() == Some(true) {
            continue;
        }
        let tag = rel["tag_name"].as_str().unwrap_or("");
        if !tag.starts_with("desktop-v") && !tag.starts_with('v') {
            continue;
        }
        let assets = rel["assets"].as_array();
        let installer_asset = assets.and_then(|arr| {
            arr.iter().find(|a| {
                a["name"]
                    .as_str()
                    .map(|n| n.eq_ignore_ascii_case("DockPanel-Desktop-Setup.exe"))
                    .unwrap_or(false)
            })
        });
        // Prefer releases that ship the Desktop installer
        if !tag.starts_with("desktop-v") && installer_asset.is_none() {
            continue;
        }
        let Some(semver) = parse_semver(tag) else {
            continue;
        };
        let replace = match &best {
            None => true,
            Some((cur_best, _)) => semver > *cur_best,
        };
        if replace {
            best = Some((semver, rel.clone()));
        }
    }

    let Some((_, rel)) = best else {
        return json!({
            "current_version": current_version,
            "update_available": false,
        });
    };

    let tag = rel["tag_name"].as_str().unwrap_or("");
    let latest_ver = tag
        .trim_start_matches("desktop-v")
        .trim_start_matches('v')
        .to_string();
    let update_available = is_newer_version(current_version, &latest_ver);
    let html_url = rel["html_url"].as_str().unwrap_or("").to_string();
    let body = rel["body"].as_str().unwrap_or("").to_string();
    let expected_sha256 = extract_sha256_from_notes(&body);
    let download_url = rel["assets"]
        .as_array()
        .and_then(|arr| {
            arr.iter().find(|a| {
                a["name"]
                    .as_str()
                    .map(|n| n.eq_ignore_ascii_case("DockPanel-Desktop-Setup.exe"))
                    .unwrap_or(false)
            })
        })
        .and_then(|a| a["browser_download_url"].as_str())
        .map(String::from);

    json!({
        "current_version": current_version,
        "update_available": update_available,
        "update_available_version": if update_available { Some(latest_ver.clone()) } else { None },
        "latest_version": latest_ver,
        "update_release_url": html_url,
        "download_url": download_url,
        "expected_sha256": expected_sha256,
        "release_notes": body,
        "checked_at": chrono::Utc::now().to_rfc3339(),
    })
}

pub async fn check_for_desktop_update(state: &AppState, force: bool) -> serde_json::Value {
    let current = env!("CARGO_PKG_VERSION");

    if !force {
        if let Ok(guard) = state.update_cache.lock() {
            if let Some((ts, cached)) = guard.as_ref() {
                if ts.elapsed() < Duration::from_secs(CACHE_TTL_SECS) {
                    return cached.clone();
                }
            }
        }
    }

    let client = match reqwest::Client::builder()
        .timeout(Duration::from_secs(8))
        .user_agent(format!("DockPanel-Desktop/{}", current))
        .build()
    {
        Ok(c) => c,
        Err(e) => {
            return json!({
                "current_version": current,
                "update_available": false,
                "check_error": e.to_string(),
            });
        }
    };

    let result = match client.get(RELEASES_API_URL).send().await {
        Ok(resp) if resp.status().is_success() => match resp.json::<Vec<serde_json::Value>>().await {
            Ok(releases) => pick_latest_desktop_release(&releases, current),
            Err(e) => json!({
                "current_version": current,
                "update_available": false,
                "check_error": format!("Failed parsing release feed: {}", e),
            }),
        },
        Ok(resp) => json!({
            "current_version": current,
            "update_available": false,
            "check_error": format!("GitHub Releases returned HTTP {}", resp.status()),
        }),
        Err(e) => json!({
            "current_version": current,
            "update_available": false,
            "check_error": format!("Offline or network unreachable: {}", e),
        }),
    };

    if result.get("check_error").is_none() {
        if let Ok(mut guard) = state.update_cache.lock() {
            *guard = Some((Instant::now(), result.clone()));
        }
    }

    result
}

fn updates_cache_dir() -> PathBuf {
    let base = std::env::var("LOCALAPPDATA")
        .or_else(|_| std::env::var("TEMP"))
        .unwrap_or_else(|_| ".".into());
    PathBuf::from(base).join("DockPanel").join("updates")
}

pub async fn download_and_launch_update(state: &AppState) -> Result<serde_json::Value, String> {
    let info = check_for_desktop_update(state, true).await;
    if info["update_available"].as_bool() != Some(true) {
        return Err("DockPanel Desktop is already on the latest version".to_string());
    }
    let version = info["latest_version"]
        .as_str()
        .ok_or_else(|| "Missing target release version".to_string())?
        .to_string();
    let download_url = info["download_url"]
        .as_str()
        .ok_or_else(|| "No DockPanel-Desktop-Setup.exe asset attached to release".to_string())?
        .to_string();
    let expected_sha = info["expected_sha256"].as_str().map(String::from);

    let dest_dir = updates_cache_dir();
    std::fs::create_dir_all(&dest_dir).map_err(|e| e.to_string())?;
    let installer_path = dest_dir.join(format!("DockPanel-Desktop-Setup-{}.exe", version));

    downloads::download_with_progress(&download_url, &installer_path, 300, |_| {}).await?;

    let actual_sha = downloads::sha256_file_hex(&installer_path)?.to_ascii_uppercase();
    if let Some(ref exp) = expected_sha {
        if !actual_sha.eq_ignore_ascii_case(exp) {
            let _ = std::fs::remove_file(&installer_path);
            return Err(format!(
                "Installer SHA-256 verification failed (expected {}, got {})",
                exp, actual_sha
            ));
        }
    }

    #[cfg(windows)]
    {
        std::process::Command::new(&installer_path)
            .spawn()
            .map_err(|e| format!("Failed launching installer {}: {}", installer_path.display(), e))?;
    }

    Ok(json!({
        "ok": true,
        "version": version,
        "installer_path": installer_path.to_string_lossy().to_string(),
        "sha256": actual_sha,
        "sha256_verified": expected_sha.is_some(),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_semver_parse_and_compare() {
        assert_eq!(parse_semver("desktop-v2.253.0"), Some((2, 253, 0)));
        assert_eq!(parse_semver("v2.254.1"), Some((2, 254, 1)));
        assert!(is_newer_version("2.253.0", "2.254.0"));
        assert!(is_newer_version("2.253.0", "desktop-v2.253.1"));
        assert!(!is_newer_version("2.253.0", "2.253.0"));
        assert!(!is_newer_version("2.254.0", "2.253.9"));
    }

    #[test]
    fn test_pick_latest_desktop_release_and_sha256() {
        let sha = "CA7F94258D5E10E2A5913A8A92D870CE70F53B4B2002B8B8D3D638DF16475CCA";
        let releases = vec![
            json!({
                "tag_name": "desktop-v2.254.0",
                "draft": false,
                "prerelease": false,
                "html_url": "https://github.com/Juni4567/Re-dockpanel/releases/tag/desktop-v2.254.0",
                "body": format!("Release notes\nSHA-256: `{}`", sha),
                "assets": [{
                    "name": "DockPanel-Desktop-Setup.exe",
                    "browser_download_url": "https://example.com/setup.exe"
                }]
            }),
            json!({
                "tag_name": "desktop-v2.255.0",
                "draft": true,
                "prerelease": false,
                "assets": []
            }),
        ];

        let res = pick_latest_desktop_release(&releases, "2.253.0");
        assert_eq!(res["update_available"], true);
        assert_eq!(res["update_available_version"], "2.254.0");
        assert_eq!(res["expected_sha256"], sha);
        assert_eq!(res["download_url"], "https://example.com/setup.exe");

        let same = pick_latest_desktop_release(&releases, "2.254.0");
        assert_eq!(same["update_available"], false);
    }
}
