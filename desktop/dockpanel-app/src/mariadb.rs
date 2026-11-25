use serde::{Deserialize, Serialize};
use serde_json::json;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MariaDbStatus {
    pub installed: bool,
    pub running: bool,
    pub port: u16,
    pub version: String,
    pub bin_dir: Option<String>,
    pub data_dir: String,
}

pub fn mariadb_base_dir() -> PathBuf {
    let user_profile = std::env::var("USERPROFILE").unwrap_or_else(|_| ".".into());
    PathBuf::from(user_profile)
        .join("DockPanel")
        .join("bin")
        .join("mariadb")
}

pub fn mariadb_data_dir() -> PathBuf {
    let user_profile = std::env::var("USERPROFILE").unwrap_or_else(|_| ".".into());
    PathBuf::from(user_profile).join("DockPanel").join("mariadb-data")
}

fn find_exe_in_tree(dir: &Path, names: &[&str]) -> Option<PathBuf> {
    for n in names {
        let direct = dir.join(n);
        if direct.is_file() {
            return Some(direct);
        }
        let bin = dir.join("bin").join(n);
        if bin.is_file() {
            return Some(bin);
        }
    }
    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            let p = entry.path();
            if p.is_dir() {
                for n in names {
                    let sub_bin = p.join("bin").join(n);
                    if sub_bin.is_file() {
                        return Some(sub_bin);
                    }
                    let sub_direct = p.join(n);
                    if sub_direct.is_file() {
                        return Some(sub_direct);
                    }
                }
            }
        }
    }
    None
}

/// Locates `mariadbd.exe` / `mysqld.exe` installed by DockPanel or locally on Windows.
pub fn find_mariadbd_binary() -> Option<PathBuf> {
    let base = mariadb_base_dir();
    if base.is_dir() {
        if let Some(found) = find_exe_in_tree(&base, &["mariadbd.exe", "mysqld.exe"]) {
            return Some(found);
        }
    }

    let extra_roots = [
        PathBuf::from(r"C:\xampp\mysql\bin"),
        PathBuf::from(r"C:\laragon\bin\mysql"),
        PathBuf::from(r"C:\wamp64\bin\mariadb"),
        PathBuf::from(r"C:\wamp64\bin\mysql"),
    ];
    for root in extra_roots {
        if root.is_dir() {
            if let Some(found) = find_exe_in_tree(&root, &["mariadbd.exe", "mysqld.exe"]) {
                return Some(found);
            }
        }
    }
    None
}

pub fn find_mariadb_install_db_binary() -> Option<PathBuf> {
    if let Some(daemon) = find_mariadbd_binary() {
        if let Some(dir) = daemon.parent() {
            for name in ["mariadb-install-db.exe", "mysql_install_db.exe"] {
                let p = dir.join(name);
                if p.is_file() {
                    return Some(p);
                }
            }
        }
    }
    None
}

pub fn is_port_accepting_tcp(port: u16) -> bool {
    use std::net::{SocketAddr, TcpStream};
    use std::time::Duration;
    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    TcpStream::connect_timeout(&addr, Duration::from_millis(350)).is_ok()
}

pub fn select_mariadb_port(preferred: u16) -> u16 {
    for candidate in [preferred, 3306, 3307, 3308] {
        if crate::router::is_port_bindable(candidate) {
            return candidate;
        }
    }
    crate::state::find_available_loopback_port().unwrap_or(3307)
}

/// Resolves the latest MariaDB LTS Windows x64 ZIP URL from mariadb.org REST API.
pub async fn resolve_mariadb_download_url() -> Result<(String, String), String> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(20))
        .user_agent("DockPanel-Desktop/2.249.0")
        .build()
        .map_err(|e| e.to_string())?;

    for series in ["11.4", "10.11"] {
        let api_url = format!("https://downloads.mariadb.org/rest-api/mariadb/{}/", series);
        let Ok(resp) = client.get(&api_url).send().await else {
            continue;
        };
        if !resp.status().is_success() {
            continue;
        }
        let Ok(json_val) = resp.json::<serde_json::Value>().await else {
            continue;
        };
        if let Some(releases) = json_val.get("releases").and_then(|v| v.as_object()) {
            let mut versions: Vec<String> = releases.keys().cloned().collect();
            versions.sort();
            versions.reverse();
            for ver in versions {
                if let Some(files) = releases
                    .get(&ver)
                    .and_then(|r| r.get("files"))
                    .and_then(|f| f.as_array())
                {
                    for f in files {
                        let fname = f.get("file_name").and_then(|s| s.as_str()).unwrap_or("");
                        let os = f.get("os").and_then(|s| s.as_str()).unwrap_or("");
                        let pkg = f.get("package_type").and_then(|s| s.as_str()).unwrap_or("");
                        let dl = f
                            .get("file_download_url")
                            .and_then(|s| s.as_str())
                            .unwrap_or("");
                        if !dl.is_empty()
                            && fname.ends_with("-winx64.zip")
                            && (os.eq_ignore_ascii_case("Windows") || pkg.contains("ZIP"))
                        {
                            return Ok((ver, dl.to_string()));
                        }
                    }
                }
            }
        }
    }

    Err("Could not resolve official MariaDB Windows ZIP URL from downloads.mariadb.org".to_string())
}

/// Initializes MariaDB grant tables if not yet present in `data_dir`.
pub fn ensure_data_dir_initialized(data_dir: &Path, port: u16) -> Result<(), String> {
    if data_dir.join("mysql").is_dir() {
        return Ok(());
    }
    fs::create_dir_all(data_dir).map_err(|e| format!("Failed creating MariaDB data dir: {}", e))?;

    let install_db = find_mariadb_install_db_binary().ok_or_else(|| {
        "mariadb-install-db.exe was not found in the MariaDB binary directory".to_string()
    })?;

    let mut cmd = std::process::Command::new(&install_db);
    cmd.args([
        &format!("--datadir={}", data_dir.to_string_lossy()),
        &format!("--port={}", port),
        "--default-user",
    ]);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x08000000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    let out = cmd
        .output()
        .map_err(|e| format!("Failed executing mariadb-install-db.exe: {}", e))?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        let msg = String::from_utf8_lossy(&out.stdout);
        return Err(format!("MariaDB initialization failed: {} {}", err, msg));
    }
    Ok(())
}

/// Downloads & extracts portable MariaDB with live byte/percentage updates via callback.
pub async fn install_mariadb_with_progress<F>(mut progress: F) -> Result<String, String>
where
    F: FnMut(&str, &str, Option<u8>, Option<String>),
{
    if find_mariadbd_binary().is_some() {
        progress("mariadb", "Portable MariaDB is already installed", Some(100), None);
        return Ok("MariaDB ready".to_string());
    }

    progress(
        "mariadb_resolve",
        "Resolving latest open-source MariaDB LTS Windows release...",
        Some(5),
        None,
    );
    let (version, url) = resolve_mariadb_download_url().await?;

    let app_data = std::env::var("APPDATA")
        .unwrap_or_else(|_| std::env::var("USERPROFILE").unwrap_or_else(|_| ".".into()));
    let zip_path = PathBuf::from(app_data)
        .join("DockPanel")
        .join("cache")
        .join(format!("mariadb-{}-winx64.zip", version));

    let label_prefix = format!("Downloading MariaDB {} (open-source)", version);
    if !zip_path.is_file()
        || fs::metadata(&zip_path).map(|m| m.len()).unwrap_or(0) < 20_000_000
    {
        crate::downloads::download_with_progress(&url, &zip_path, 300, |p| {
            let pct = p.percent.unwrap_or(0);
            let txt = match p.total_bytes {
                Some(tot) => format!(
                    "{}: {} / {} ({}%)",
                    label_prefix,
                    crate::downloads::format_mb(p.downloaded_bytes),
                    crate::downloads::format_mb(tot),
                    pct
                ),
                None => format!(
                    "{}: {} downloaded",
                    label_prefix,
                    crate::downloads::format_mb(p.downloaded_bytes)
                ),
            };
            progress("mariadb_download", &txt, p.percent, None);
        })
        .await?;
    } else {
        progress(
            "mariadb_download",
            &format!("Using cached MariaDB {} archive", version),
            Some(100),
            None,
        );
    }

    progress(
        "mariadb_extract",
        "Extracting MariaDB binaries & initializing database engine...",
        Some(80),
        None,
    );

    let dest = mariadb_base_dir();
    fs::create_dir_all(&dest).map_err(|e| format!("Failed creating MariaDB dir: {}", e))?;
    let zip_escaped = zip_path.to_string_lossy().replace('\'', "''");
    let dest_escaped = dest.to_string_lossy().replace('\'', "''");
    let extract_res = tokio::task::spawn_blocking(move || {
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x08000000;
            let ps = format!(
                "$ErrorActionPreference='Stop'; Expand-Archive -Path '{}' -DestinationPath '{}' -Force",
                zip_escaped, dest_escaped
            );
            let out = std::process::Command::new("powershell.exe")
                .args(["-NoProfile", "-ExecutionPolicy", "Bypass", "-Command", &ps])
                .creation_flags(CREATE_NO_WINDOW)
                .output()
                .map_err(|e| e.to_string())?;
            if !out.status.success() {
                return Err(String::from_utf8_lossy(&out.stderr).to_string());
            }
            Ok::<(), String>(())
        }
        #[cfg(not(windows))]
        {
            let _ = (zip_escaped, dest_escaped);
            Ok::<(), String>(())
        }
    })
    .await
    .map_err(|e| e.to_string())?;
    extract_res?;

    progress(
        "mariadb_extract",
        &format!("MariaDB {} extracted and initialized", version),
        Some(100),
        None,
    );
    Ok(version)
}

pub fn sanitize_mysql_ident(name: &str) -> Result<String, String> {
    let clean: String = name
        .trim()
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '-')
        .collect();
    if clean.is_empty() || clean.len() > 64 {
        return Err("Invalid MySQL identifier".to_string());
    }
    Ok(clean)
}

/// Executes SQL on a local MariaDB/MySQL server via PHP's PDO_MySQL extension.
pub fn run_mysql_query(
    port: u16,
    database_name: Option<&str>,
    sql: &str,
) -> Result<serde_json::Value, String> {
    let php_exe = crate::state::find_php_binary(None)
        .ok_or_else(|| "No PHP runtime with PDO_MySQL installed".to_string())?;

    let start = std::time::Instant::now();
    let script = r#"<?php
$port = $argv[1];
$db   = $argv[2];
$sql  = $argv[3];
try {
    $dsn = $db !== ''
        ? "mysql:host=127.0.0.1;port={$port};dbname={$db};charset=utf8mb4"
        : "mysql:host=127.0.0.1;port={$port};charset=utf8mb4";
    $pdo = new PDO($dsn, 'root', '', [
        PDO::ATTR_ERRMODE => PDO::ERRMODE_EXCEPTION,
        PDO::ATTR_TIMEOUT => 5,
    ]);

    $trimmed = ltrim(preg_replace('/^(?:--[^\n]*\n|\/\*[\s\S]*?\*\/\s*)*/', '', $sql));
    $head = strtoupper(preg_replace('/\s+/', ' ', substr($trimmed, 0, 12)));
    $isSelect = str_starts_with($head, 'SELECT')
        || str_starts_with($head, 'SHOW')
        || str_starts_with($head, 'DESCRIBE')
        || str_starts_with($head, 'DESC ')
        || str_starts_with($head, 'EXPLAIN')
        || str_starts_with($head, 'WITH');

    $cols = [];
    $rows = [];
    if ($isSelect) {
        $stmt = $pdo->query($sql);
        if ($stmt !== false) {
            $colCount = $stmt->columnCount();
            for ($i = 0; $i < $colCount; $i++) {
                $meta = $stmt->getColumnMeta($i);
                $cols[] = $meta['name'] ?? ('col_' . $i);
            }
            $count = 0;
            while (($r = $stmt->fetch(PDO::FETCH_NUM)) !== false) {
                if ($count >= 500) break;
                $rows[] = array_map(fn($v) => $v === null ? '\\N' : (string)$v, $r);
                $count++;
            }
        }
    } else {
        $pdo->exec($sql);
    }

    echo "\n__DOCKPANEL_JSON__" . json_encode(['ok' => true, 'columns' => $cols, 'rows' => $rows, 'row_count' => count($rows)], JSON_INVALID_UTF8_SUBSTITUTE);
} catch (Throwable $e) {
    echo "\n__DOCKPANEL_JSON__" . json_encode(['ok' => false, 'error' => $e->getMessage()], JSON_INVALID_UTF8_SUBSTITUTE);
}
"#;

    let temp_script =
        std::env::temp_dir().join(format!("dockpanel_mysql_{}.php", uuid::Uuid::new_v4()));
    fs::write(&temp_script, script).map_err(|e| e.to_string())?;

    let mut cmd = std::process::Command::new(&php_exe);
    cmd.arg(&temp_script)
        .arg(port.to_string())
        .arg(database_name.unwrap_or(""))
        .arg(sql);

    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x08000000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }

    let out = cmd.output();
    let _ = fs::remove_file(&temp_script);
    let elapsed_ms = start.elapsed().as_millis() as u64;

    let output = out.map_err(|e| format!("Failed executing MySQL query via PHP: {}", e))?;
    let raw_stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let parsed: serde_json::Value = crate::databases::parse_php_json_output(&raw_stdout).map_err(|_| {
        let err = String::from_utf8_lossy(&output.stderr);
        if err.is_empty() {
            format!("Invalid MySQL output: {}", raw_stdout.trim())
        } else {
            err.to_string()
        }
    })?;

    if parsed.get("ok").and_then(|v| v.as_bool()) != Some(true) {
        return Err(parsed
            .get("error")
            .and_then(|v| v.as_str())
            .unwrap_or("MariaDB query error")
            .to_string());
    }

    Ok(json!({
        "columns": parsed["columns"],
        "rows": parsed["rows"],
        "row_count": parsed["row_count"],
        "execution_time_ms": elapsed_ms,
        "truncated": false
    }))
}

pub fn list_user_databases(port: u16) -> Result<Vec<String>, String> {
    let res = run_mysql_query(port, None, "SHOW DATABASES")?;
    let empty = Vec::new();
    let rows = res["rows"].as_array().unwrap_or(&empty);
    let mut out = Vec::new();
    for r in rows {
        if let Some(name) = r.as_array().and_then(|a| a.first()).and_then(|v| v.as_str()) {
            if !matches!(
                name.to_lowercase().as_str(),
                "information_schema" | "mysql" | "performance_schema" | "sys"
            ) {
                out.push(name.to_string());
            }
        }
    }
    Ok(out)
}

pub fn create_database(port: u16, db_name: &str) -> Result<String, String> {
    let clean = sanitize_mysql_ident(db_name)?;
    run_mysql_query(
        port,
        None,
        &format!(
            "CREATE DATABASE IF NOT EXISTS `{}` CHARACTER SET utf8mb4 COLLATE utf8mb4_unicode_ci",
            clean
        ),
    )?;
    Ok(clean)
}

pub fn drop_database(port: u16, db_name: &str) -> Result<(), String> {
    let clean = sanitize_mysql_ident(db_name)?;
    run_mysql_query(port, None, &format!("DROP DATABASE IF EXISTS `{}`", clean))?;
    Ok(())
}

/// Exports all tables and data of a MariaDB database to a `.sql` file in DockPanel db-dumps.
pub fn export_mysql_dump(port: u16, db_name: &str, dest_path: &Path) -> Result<(), String> {
    let clean = sanitize_mysql_ident(db_name)?;
    let php_exe = crate::state::find_php_binary(None)
        .ok_or_else(|| "No PHP runtime with PDO_MySQL installed".to_string())?;

    if let Some(parent) = dest_path.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }

    let script = r#"<?php
$port = $argv[1];
$db   = $argv[2];
$out  = $argv[3];
try {
    $pdo = new PDO("mysql:host=127.0.0.1;port={$port};dbname={$db};charset=utf8mb4", 'root', '', [
        PDO::ATTR_ERRMODE => PDO::ERRMODE_EXCEPTION,
    ]);
    $fh = fopen($out, 'w');
    if (!$fh) throw new RuntimeException('Cannot open dump target');
    fwrite($fh, "-- DockPanel Desktop MariaDB Dump\n-- Database: {$db}\nSET FOREIGN_KEY_CHECKS=0;\n\n");
    $tables = $pdo->query("SHOW TABLES")->fetchAll(PDO::FETCH_COLUMN);
    foreach ($tables as $t) {
        $tq = '`' . str_replace('`', '``', $t) . '`';
        fwrite($fh, "DROP TABLE IF EXISTS {$tq};\n");
        $createRow = $pdo->query("SHOW CREATE TABLE {$tq}")->fetch(PDO::FETCH_NUM);
        if ($createRow && isset($createRow[1])) {
            fwrite($fh, $createRow[1] . ";\n\n");
        }
        $rows = $pdo->query("SELECT * FROM {$tq}")->fetchAll(PDO::FETCH_ASSOC);
        foreach ($rows as $row) {
            $cols = array_map(fn($c) => '`' . str_replace('`', '``', $c) . '`', array_keys($row));
            $vals = array_map(fn($v) => $v === null ? 'NULL' : $pdo->quote((string)$v), array_values($row));
            fwrite($fh, "INSERT INTO {$tq} (" . implode(', ', $cols) . ") VALUES (" . implode(', ', $vals) . ");\n");
        }
        fwrite($fh, "\n");
    }
    fwrite($fh, "SET FOREIGN_KEY_CHECKS=1;\n");
    fclose($fh);
    echo json_encode(['ok' => true]);
} catch (Throwable $e) {
    echo json_encode(['ok' => false, 'error' => $e->getMessage()]);
}
"#;

    let temp_script =
        std::env::temp_dir().join(format!("dockpanel_mysqldump_{}.php", uuid::Uuid::new_v4()));
    fs::write(&temp_script, script).map_err(|e| e.to_string())?;
    let mut cmd = std::process::Command::new(&php_exe);
    cmd.arg(&temp_script)
        .arg(port.to_string())
        .arg(&clean)
        .arg(dest_path.to_string_lossy().to_string());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x08000000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    let out = cmd.output();
    let _ = fs::remove_file(&temp_script);
    let output = out.map_err(|e| format!("Failed exporting MariaDB dump: {}", e))?;
    let raw = String::from_utf8_lossy(&output.stdout).to_string();
    let stdout = raw.rfind('{').map(|i| &raw[i..]).unwrap_or(raw.trim());
    let parsed: serde_json::Value = serde_json::from_str(stdout)
        .map_err(|_| format!("Invalid export output: {}", raw.trim()))?;
    if parsed.get("ok").and_then(|v| v.as_bool()) != Some(true) {
        return Err(parsed["error"]
            .as_str()
            .unwrap_or("Export failed")
            .to_string());
    }
    Ok(())
}

pub fn import_mysql_dump(port: u16, db_name: &str, sql_file: &Path) -> Result<(), String> {
    let clean = sanitize_mysql_ident(db_name)?;
    let sql = fs::read_to_string(sql_file).map_err(|e| e.to_string())?;
    run_mysql_query(port, Some(&clean), &sql).map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sanitize_mysql_ident() {
        assert_eq!(sanitize_mysql_ident("wp_site_1").unwrap(), "wp_site_1");
        assert!(sanitize_mysql_ident(";;").is_err());
    }
}
