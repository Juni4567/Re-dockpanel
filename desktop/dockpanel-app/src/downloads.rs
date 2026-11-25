use sha2::{Digest, Sha256};
use std::path::Path;
use std::time::{Duration, Instant};

#[derive(Debug, Clone)]
pub struct DownloadProgress {
    pub downloaded_bytes: u64,
    pub total_bytes: Option<u64>,
    pub percent: Option<u8>,
}

/// Formats bytes into a concise MB label such as "12.4 MB".
pub fn format_mb(bytes: u64) -> String {
    format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
}

/// Streams an HTTP GET download to `dest_path` using an atomic `.part` temp file
/// and invokes `on_progress` periodically with byte and percentage counts.
pub async fn download_with_progress<F>(
    url: &str,
    dest_path: &Path,
    timeout_secs: u64,
    mut on_progress: F,
) -> Result<u64, String>
where
    F: FnMut(DownloadProgress),
{
    if let Some(parent) = dest_path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("Failed to create cache directory: {}", e))?;
    }

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(timeout_secs))
        .user_agent("DockPanel-Desktop/2.249.0")
        .build()
        .map_err(|e| format!("HTTP client init failed: {}", e))?;

    let mut resp = client
        .get(url)
        .send()
        .await
        .map_err(|e| format!("Download request failed for {}: {}", url, e))?;

    if !resp.status().is_success() {
        return Err(format!("HTTP {} while downloading {}", resp.status(), url));
    }

    let total_bytes = resp.content_length();
    on_progress(DownloadProgress {
        downloaded_bytes: 0,
        total_bytes,
        percent: Some(0),
    });

    let part_path = dest_path.with_extension("part");
    let mut file = tokio::fs::File::create(&part_path)
        .await
        .map_err(|e| format!("Failed creating temp file {}: {}", part_path.display(), e))?;

    let mut downloaded: u64 = 0;
    let mut last_emit = Instant::now() - Duration::from_secs(1);

    use tokio::io::AsyncWriteExt;
    while let Some(chunk) = resp
        .chunk()
        .await
        .map_err(|e| format!("Download stream error: {}", e))?
    {
        file.write_all(&chunk)
            .await
            .map_err(|e| format!("Write error: {}", e))?;
        downloaded += chunk.len() as u64;

        if last_emit.elapsed() >= Duration::from_millis(250) {
            let percent = total_bytes
                .filter(|t| *t > 0)
                .map(|t| ((downloaded.saturating_mul(100)) / t).min(100) as u8);
            on_progress(DownloadProgress {
                downloaded_bytes: downloaded,
                total_bytes,
                percent,
            });
            last_emit = Instant::now();
        }
    }

    file.flush().await.map_err(|e| e.to_string())?;
    drop(file);

    tokio::fs::rename(&part_path, dest_path)
        .await
        .map_err(|e| format!("Failed moving downloaded archive into place: {}", e))?;

    on_progress(DownloadProgress {
        downloaded_bytes: downloaded,
        total_bytes: total_bytes.or(Some(downloaded)),
        percent: Some(100),
    });

    Ok(downloaded)
}

#[allow(dead_code)]
/// Computes lowercase hex SHA-256 for a local file.
pub fn sha256_file_hex(path: &Path) -> Result<String, String> {
    let mut file = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 64 * 1024];
    loop {
        use std::io::Read;
        let n = file.read(&mut buf).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hex::encode(hasher.finalize()))
}

/// Computes lowercase hex SHA-256 of an input string.
pub fn sha256_hex(input: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(input.as_bytes());
    hex::encode(hasher.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_mb_and_sha256() {
        assert_eq!(format_mb(5 * 1024 * 1024), "5.0 MB");
        let h = sha256_hex("dockpanel");
        assert_eq!(h.len(), 64);
    }
}
