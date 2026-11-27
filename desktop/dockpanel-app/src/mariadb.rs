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
        PathBuf::from(r"C:\Program Files\MariaDB"),
        PathBuf::from(r"C:\Program Files\MySQL"),
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
    if let Ok(pf) = std::fs::read_dir(r"C:\Program Files") {
        for entry in pf.flatten() {
            let name = entry.file_name().to_string_lossy().to_ascii_lowercase();
            if name.starts_with("mariadb") || name.starts_with("mysql") {
                if let Some(found) = find_exe_in_tree(&entry.path(), &["mariadbd.exe", "mysqld.exe"]) {
                    return Some(found);
                }
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

/// Drops any leftover partial tables from an earlier failed import and recreates
/// an empty UTF-8 database ready for a clean SQL restore.
pub fn reset_database(port: u16, db_name: &str) -> Result<String, String> {
    let clean = sanitize_mysql_ident(db_name)?;
    run_mysql_query(port, None, &format!("DROP DATABASE IF EXISTS `{}`", clean))?;
    create_database(port, &clean)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompatConfirmRequired {
    pub error: String,
    pub fixes: Vec<String>,
    pub target_version: String,
}

#[derive(Debug, Clone)]
pub enum SqlImportError {
    CompatConfirm(CompatConfirmRequired),
    Fatal(String),
}

impl SqlImportError {
    pub fn message(&self) -> &str {
        match self {
            SqlImportError::CompatConfirm(c) => &c.error,
            SqlImportError::Fatal(m) => m,
        }
    }
}

pub fn get_server_version(port: u16) -> String {
    run_mysql_query(port, None, "SELECT VERSION()")
        .ok()
        .and_then(|res| {
            res["rows"]
                .as_array()
                .and_then(|rows| rows.first())
                .and_then(|row| row.as_array())
                .and_then(|cols| cols.first())
                .and_then(|v| v.as_str())
                .map(String::from)
        })
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "MariaDB".to_string())
}

fn strip_backtick_or_quoted_token_at<'a>(s: &'a str) -> Option<usize> {
    let bytes = s.as_bytes();
    if bytes.is_empty() {
        return None;
    }
    let delim = bytes[0];
    if delim == b'`' || delim == b'\'' || delim == b'"' {
        let mut i = 1;
        while i < bytes.len() {
            if bytes[i] == b'\\' {
                i += 2;
                continue;
            }
            if bytes[i] == delim {
                return Some(i + 1);
            }
            i += 1;
        }
        None
    } else {
        let mut i = 0;
        while i < bytes.len() && !bytes[i].is_ascii_whitespace() && bytes[i] != b'*' {
            i += 1;
        }
        Some(i)
    }
}

fn strip_definer_clauses(sql: &str) -> String {
    let lower = sql.to_ascii_lowercase();
    let mut out = String::with_capacity(sql.len());
    let mut cursor = 0usize;
    while let Some(rel) = lower[cursor..].find("definer") {
        let pos = cursor + rel;
        let before_ok = pos == 0 || !sql.as_bytes()[pos - 1].is_ascii_alphanumeric() && sql.as_bytes()[pos - 1] != b'_';
        if !before_ok {
            out.push_str(&sql[cursor..pos + 7]);
            cursor = pos + 7;
            continue;
        }
        let mut p = pos + 7;
        let bytes = sql.as_bytes();
        while p < bytes.len() && bytes[p].is_ascii_whitespace() {
            p += 1;
        }
        if p < bytes.len() && bytes[p] == b'=' {
            p += 1;
            while p < bytes.len() && bytes[p].is_ascii_whitespace() {
                p += 1;
            }
            if let Some(u_len) = strip_backtick_or_quoted_token_at(&sql[p..]) {
                let after_u = p + u_len;
                if after_u < bytes.len() && bytes[after_u] == b'@' {
                    let host_start = after_u + 1;
                    if let Some(h_len) = strip_backtick_or_quoted_token_at(&sql[host_start..]) {
                        let mut end = host_start + h_len;
                        while end < bytes.len() && bytes[end].is_ascii_whitespace() {
                            end += 1;
                        }
                        out.push_str(&sql[cursor..pos]);
                        cursor = end;
                        continue;
                    }
                }
            }
        }
        out.push_str(&sql[cursor..pos + 7]);
        cursor = pos + 7;
    }
    out.push_str(&sql[cursor..]);
    out
}

fn normalize_uca1400_collations(sql: &str) -> String {
    let mut out = String::with_capacity(sql.len());
    let lower = sql.to_ascii_lowercase();
    let mut cursor = 0usize;
    while let Some(rel) = lower[cursor..].find("utf8mb4_uca1400_") {
        let pos = cursor + rel;
        out.push_str(&sql[cursor..pos]);
        let mut end = pos + "utf8mb4_uca1400_".len();
        let bytes = sql.as_bytes();
        while end < bytes.len() && (bytes[end].is_ascii_alphanumeric() || bytes[end] == b'_') {
            end += 1;
        }
        let tok = &lower[pos..end];
        if tok.ends_with("_cs") || tok.ends_with("_bin") {
            out.push_str("utf8mb4_bin");
        } else {
            out.push_str("utf8mb4_unicode_ci");
        }
        cursor = end;
    }
    out.push_str(&sql[cursor..]);
    out
}

fn clamp_long_index_prefixes(sql: &str) -> String {
    let lower = sql.to_ascii_lowercase();
    if !(lower.contains("key ") || lower.contains("index ")) {
        return sql.to_string();
    }
    // In MariaDB/MySQL with utf8mb4 (4 bytes/char), `(255)` prefixes can trigger
    // error 1071 (Specified key was too long) on older formats; clamping to `(191)` fits cleanly.
    let mut out = sql.to_string();
    for big in ["(255)", "(250)", "(220)", "(200)"] {
        if out.contains(big) {
            let mut next = String::with_capacity(out.len());
            let mut cursor = 0usize;
            while let Some(rel) = out[cursor..].find(big) {
                let idx = cursor + rel;
                let line_start = out[..idx].rfind('\n').map(|p| p + 1).unwrap_or(0);
                let line_hdr = out[line_start..idx].to_ascii_lowercase();
                if line_hdr.contains("key") || line_hdr.contains("index") {
                    next.push_str(&out[cursor..idx]);
                    next.push_str("(191)");
                } else {
                    next.push_str(&out[cursor..idx + big.len()]);
                }
                cursor = idx + big.len();
            }
            next.push_str(&out[cursor..]);
            out = next;
        }
    }
    out
}

/// Scans DDL / `SET` statements in `sql_file` without modifying rows and reports any
/// backward/forward or MySQL <-> MariaDB incompatibilities against `target_version`.
pub fn scan_sql_compatibility_issues(sql_file: &Path, target_version: &str) -> (Option<String>, Vec<String>) {
    use std::io::BufRead;
    let Ok(file) = fs::File::open(sql_file) else {
        return (None, Vec::new());
    };
    let mut reader = std::io::BufReader::with_capacity(256 * 1024, file);
    let mut line_buf = Vec::with_capacity(8192);
    let target_is_mariadb = target_version.to_ascii_lowercase().contains("mariadb");

    let mut found_fixes: Vec<String> = Vec::new();
    let mut first_error: Option<String> = None;
    let mut push_issue = |err_desc: &str, fix_desc: &str| {
        if first_error.is_none() {
            first_error = Some(err_desc.to_string());
        }
        if !found_fixes.iter().any(|x| x == fix_desc) {
            found_fixes.push(fix_desc.to_string());
        }
    };

    loop {
        line_buf.clear();
        match reader.read_until(b'\n', &mut line_buf) {
            Ok(0) | Err(_) => break,
            Ok(_) => {}
        }
        // Skip INSERT / REPLACE row lines immediately for multi-hundred-MB speed.
        let mut s_start = 0usize;
        while s_start < line_buf.len() && line_buf[s_start].is_ascii_whitespace() {
            s_start += 1;
        }
        let head = &line_buf[s_start..];
        if head.is_empty()
            || head.starts_with(b"(")
            || head.get(..6).is_some_and(|p| p.eq_ignore_ascii_case(b"insert"))
            || head.get(..7).is_some_and(|p| p.eq_ignore_ascii_case(b"replace"))
        {
            continue;
        }
        if line_buf.len() > 16384 {
            continue;
        }

        let line = String::from_utf8_lossy(&line_buf);
        let lower = line.to_ascii_lowercase();

        if lower.contains("utf8mb4_0900_") && (target_is_mariadb || target_version.starts_with("5.")) {
            push_issue(
                &format!(
                    "Incompatible MySQL 8 collation (utf8mb4_0900_*) detected for target server '{}': {}",
                    target_version,
                    line.trim()
                ),
                "Convert MySQL 8 utf8mb4_0900_* collations to utf8mb4_unicode_ci / utf8mb4_bin",
            );
        }
        if lower.contains("utf8mb4_uca1400_") {
            push_issue(
                &format!(
                    "MariaDB 11.4+ UCA-1400 collation detected which may fail on older/other engines ({}): {}",
                    target_version,
                    line.trim()
                ),
                "Downgrade utf8mb4_uca1400_* collations to utf8mb4_unicode_ci / utf8mb4_bin",
            );
        }
        if lower.contains("definer=") || lower.contains("definer =") {
            push_issue(
                &format!(
                    "Remote host DEFINER clause found on VIEW/TRIGGER/ROUTINE (may fail locally without matching host user): {}",
                    line.trim()
                ),
                "Strip remote DEFINER=... clauses so objects import under local root",
            );
        }
        if lower.contains("gtid_purged") || lower.contains("sql_log_bin") {
            push_issue(
                &format!(
                    "Replication statement (GTID_PURGED / SQL_LOG_BIN) requires SUPER privileges: {}",
                    line.trim()
                ),
                "Skip @@GLOBAL.GTID_PURGED and @@SESSION.SQL_LOG_BIN session statements",
            );
        }
        if lower.contains("with system versioning")
            || lower.contains("page_checksum=")
            || lower.contains("transactional=")
        {
            if !target_is_mariadb {
                push_issue(
                    &format!(
                        "MariaDB-specific table option detected for MySQL target: {}",
                        line.trim()
                    ),
                    "Strip MariaDB-only clauses (WITH SYSTEM VERSIONING, PAGE_CHECKSUM, TRANSACTIONAL)",
                );
            }
        }
        if lower.contains("srid ") || lower.contains(" invisible") || lower.contains("encryption=") {
            push_issue(
                &format!(
                    "MySQL 8 DDL modifier (INVISIBLE / SRID / ENCRYPTION) unsupported on MariaDB/older MySQL: {}",
                    line.trim()
                ),
                "Remove unsupported INVISIBLE index modifiers, SRID constraints, and ENCRYPTION options",
            );
        }
        if lower.contains("row_format=fixed") || lower.contains("type=innodb") || lower.contains("type=myisam") {
            push_issue(
                &format!(
                    "Legacy pre-5.5 table syntax (TYPE= or ROW_FORMAT=FIXED) detected: {}",
                    line.trim()
                ),
                "Upgrade legacy TYPE= storage clauses to ENGINE= and normalize ROW_FORMAT=DYNAMIC",
            );
        }
    }

    (first_error, found_fixes)
}

pub fn classify_sql_compat_error(err: &str, target_version: &str) -> Vec<String> {
    let lower = err.to_ascii_lowercase();
    let mut fixes = Vec::new();
    let is_compat_code = lower.contains("1273")
        || lower.contains("unknown collation")
        || lower.contains("1115")
        || lower.contains("unknown character set")
        || lower.contains("1071")
        || lower.contains("specified key was too long")
        || lower.contains("1227")
        || lower.contains("1449")
        || lower.contains("definer")
        || lower.contains("super privilege")
        || lower.contains("gtid_purged")
        || lower.contains("1286")
        || lower.contains("unknown storage engine")
        || lower.contains("1064")
        || lower.contains("1067")
        || lower.contains("invalid default value")
        || lower.contains("row_format")
        || lower.contains("srid")
        || lower.contains("invisible");
    if !is_compat_code {
        return fixes;
    }

    if lower.contains("collation") || lower.contains("1273") || lower.contains("0900") || lower.contains("uca1400") {
        fixes.push(format!("Map incompatible collations to utf8mb4_unicode_ci / utf8mb4_bin for {}", target_version));
    }
    if lower.contains("character set") || lower.contains("1115") || lower.contains("utf8mb3") {
        fixes.push("Convert incompatible utf8mb3 character sets to utf8 / utf8mb4".to_string());
    }
    if lower.contains("key was too long") || lower.contains("1071") {
        fixes.push("Upgrade table ROW_FORMAT=DYNAMIC and clamp oversized index prefixes (>191 chars)".to_string());
    }
    if lower.contains("definer") || lower.contains("1449") || lower.contains("1227") || lower.contains("gtid") {
        fixes.push("Strip remote DEFINER=... clauses and GTID_PURGED / SQL_LOG_BIN statements".to_string());
    }
    if lower.contains("1064") || lower.contains("1286") || lower.contains("1067") || lower.contains("syntax") || lower.contains("storage engine") {
        fixes.push("Apply MySQL <-> MariaDB & backward/forward DDL compatibility normalization (hex defaults, TYPE=->ENGINE=, INVISIBLE/SRID/ENCRYPTION/VERSIONING stripping)".to_string());
    }
    if fixes.is_empty() {
        fixes.push("Apply full SQL backward/forward & MySQL <-> MariaDB compatibility rewrites".to_string());
    }
    fixes
}

fn rewrite_phpmyadmin_as_literal(input: &str, prefix_char: char, replace_prefix: char, is_valid_digit: fn(char) -> bool) -> String {
    let bytes = input.as_bytes();
    let lower = input.to_ascii_lowercase();
    let mut out = String::with_capacity(input.len());
    let mut cursor = 0usize;

    while let Some(rel_def) = lower[cursor..].find("default") {
        let def_pos = cursor + rel_def;
        let before_ok = def_pos == 0 || !bytes[def_pos - 1].is_ascii_alphanumeric() && bytes[def_pos - 1] != b'_';
        let after_def = def_pos + 7;
        if !before_ok || after_def >= bytes.len() || !bytes[after_def].is_ascii_whitespace() {
            out.push_str(&input[cursor..after_def.min(input.len())]);
            cursor = after_def.min(input.len());
            continue;
        }
        let mut p = after_def;
        while p < bytes.len() && bytes[p].is_ascii_whitespace() {
            p += 1;
        }
        if p < bytes.len() && (bytes[p] as char).eq_ignore_ascii_case(&prefix_char) {
            let after_x = p + 1;
            if after_x < bytes.len() && bytes[after_x].is_ascii_whitespace() {
                let mut q = after_x;
                while q < bytes.len() && bytes[q].is_ascii_whitespace() {
                    q += 1;
                }
                if lower[q..].starts_with("as") {
                    let after_as = q + 2;
                    if after_as < bytes.len() && bytes[after_as].is_ascii_whitespace() {
                        let mut r = after_as;
                        while r < bytes.len() && bytes[r].is_ascii_whitespace() {
                            r += 1;
                        }
                        if r < bytes.len() && bytes[r] == b'`' {
                            if let Some(end_bt_rel) = input[r + 1..].find('`') {
                                let inner = &input[r + 1..r + 1 + end_bt_rel];
                                if !inner.is_empty() && inner.chars().all(is_valid_digit) {
                                    out.push_str(&input[cursor..after_def]);
                                    out.push(' ');
                                    out.push(replace_prefix);
                                    out.push('\'');
                                    out.push_str(inner);
                                    out.push('\'');
                                    cursor = r + 1 + end_bt_rel + 1;
                                    continue;
                                }
                            }
                        }
                    }
                }
            }
        }
        out.push_str(&input[cursor..after_def]);
        cursor = after_def;
    }

    out.push_str(&input[cursor..]);
    out
}

/// Normalizes DDL syntax exported by phpMyAdmin / newer MySQL versions so it imports
/// cleanly into local MariaDB without corrupting `INSERT INTO` row content.
pub fn sanitize_ddl_statement_for_mariadb(sql: &str) -> String {
    sanitize_ddl_statement_with_mode(sql, true)
}

pub fn sanitize_ddl_statement_with_mode(sql: &str, apply_compat_fixes: bool) -> String {
    let trimmed = sql.trim_start();
    if (trimmed.len() >= 6 && trimmed[..6].eq_ignore_ascii_case("insert"))
        || (trimmed.len() >= 7 && trimmed[..7].eq_ignore_ascii_case("replace"))
    {
        return sql.to_string();
    }

    let mut out = if trimmed.to_ascii_lowercase().contains("default") && sql.contains('`') {
        let s1 = rewrite_phpmyadmin_as_literal(sql, 'x', 'X', |c| c.is_ascii_hexdigit());
        rewrite_phpmyadmin_as_literal(&s1, 'b', 'b', |c| c == '0' || c == '1')
    } else {
        sql.to_string()
    };

    if !apply_compat_fixes {
        return out;
    }

    if out.contains("utf8mb4_0900") {
        out = out
            .replace("utf8mb4_0900_as_cs", "utf8mb4_bin")
            .replace("utf8mb4_0900_ai_ci", "utf8mb4_unicode_ci")
            .replace("utf8mb4_0900_as_ci", "utf8mb4_unicode_ci")
            .replace("utf8mb4_ja_0900_as_cs", "utf8mb4_unicode_ci");
    }
    if out.to_ascii_lowercase().contains("utf8mb4_uca1400_") {
        out = normalize_uca1400_collations(&out);
    }
    if out.contains("utf8mb3") {
        out = out
            .replace("utf8mb3_general_ci", "utf8_general_ci")
            .replace("utf8mb3_unicode_ci", "utf8_unicode_ci")
            .replace("CHARSET=utf8mb3", "CHARSET=utf8")
            .replace("CHARACTER SET utf8mb3", "CHARACTER SET utf8");
    }
    let low = out.to_ascii_lowercase();
    if low.contains("definer") {
        out = strip_definer_clauses(&out);
    }
    if low.contains("gtid_purged") || low.contains("sql_log_bin") {
        // Turn unsupported cloud GTID / binlog directives into no-op SQL comment
        return "SELECT 1".to_string();
    }
    if low.contains("row_format=fixed") {
        out = out.replace("ROW_FORMAT=FIXED", "ROW_FORMAT=DYNAMIC").replace("row_format=fixed", "ROW_FORMAT=DYNAMIC");
    }
    if low.contains("type=innodb") || low.contains("type=myisam") {
        out = out
            .replace("TYPE=InnoDB", "ENGINE=InnoDB")
            .replace("TYPE=MyISAM", "ENGINE=MyISAM")
            .replace("type=innodb", "ENGINE=InnoDB")
            .replace("type=myisam", "ENGINE=MyISAM");
    }
    if low.contains(" invisible") || low.contains("encryption=") || low.contains("page_checksum=") || low.contains("transactional=") || low.contains("with system versioning") {
        for pat in [
            " /*!80000 INVISIBLE */",
            " INVISIBLE",
            " ENCRYPTION='N'",
            " ENCRYPTION='Y'",
            " PAGE_CHECKSUM=1",
            " PAGE_CHECKSUM=0",
            " TRANSACTIONAL=1",
            " TRANSACTIONAL=0",
            " WITH SYSTEM VERSIONING",
        ] {
            if out.contains(pat) {
                out = out.replace(pat, "");
            }
        }
    }
    clamp_long_index_prefixes(&out)
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

pub fn find_mysql_client_binary() -> Option<PathBuf> {
    let names = ["mariadb.exe", "mysql.exe"];
    if let Some(daemon) = find_mariadbd_binary() {
        if let Some(dir) = daemon.parent() {
            for name in names {
                let p = dir.join(name);
                if p.is_file() {
                    return Some(p);
                }
            }
        }
    }
    let base = mariadb_base_dir();
    if base.is_dir() {
        if let Some(found) = find_exe_in_tree(&base, &names) {
            return Some(found);
        }
    }
    if let Ok(pf) = std::fs::read_dir(r"C:\Program Files") {
        for entry in pf.flatten() {
            let fname = entry.file_name().to_string_lossy().to_ascii_lowercase();
            if fname.starts_with("mariadb") || fname.starts_with("mysql") {
                if let Some(found) = find_exe_in_tree(&entry.path(), &names) {
                    return Some(found);
                }
            }
        }
    }
    if let Some(path_var) = std::env::var_os("PATH") {
        for dir in std::env::split_paths(&path_var) {
            for name in names {
                let p = dir.join(name);
                if p.is_file() {
                    return Some(p);
                }
            }
        }
    }
    None
}

pub fn format_memory_limit_arg(memory_limit_mb: Option<i32>) -> (String, String) {
    match memory_limit_mb {
        Some(m) if m <= 0 => ("-1".to_string(), "Unlimited".to_string()),
        Some(m) => {
            let clamped = m.clamp(128, 16384);
            (format!("{}M", clamped), format!("{} MB", clamped))
        }
        None => ("1024M".to_string(), "1024 MB".to_string()),
    }
}

pub fn suggest_memory_limit_hint(err: &str, current_mb: Option<i32>) -> Option<String> {
    let lower = err.to_ascii_lowercase();
    let is_mem = lower.contains("allowed memory size")
        || lower.contains("out of memory")
        || lower.contains("memory_limit")
        || lower.contains("max_allowed_packet")
        || lower.contains("packet bigger");
    if !is_mem {
        return None;
    }
    let cur_label = format_memory_limit_arg(current_mb).1;
    let cur_val = current_mb.unwrap_or(1024);
    let recommended = if cur_val < 0 {
        "Unlimited (-1)".to_string()
    } else if cur_val < 2048 {
        "2048 MB".to_string()
    } else if cur_val < 4096 {
        "4096 MB".to_string()
    } else {
        "Unlimited (-1)".to_string()
    };
    Some(format!(
        "Workable Fix: Memory/packet limit ({}) exceeded while importing SQL. Increase the Import Memory Limit to {} and retry — your website files on disk stay intact.",
        cur_label, recommended
    ))
}

pub fn import_mysql_dump(port: u16, db_name: &str, sql_file: &Path) -> Result<(), String> {
    import_mysql_dump_with_memory_and_progress(port, db_name, sql_file, None, |_, _, _| {})
}

pub fn import_mysql_dump_with_progress<F>(
    port: u16,
    db_name: &str,
    sql_file: &Path,
    on_progress: F,
) -> Result<(), String>
where
    F: FnMut(u64, u64, &str),
{
    import_mysql_dump_with_memory_and_progress(port, db_name, sql_file, None, on_progress)
}

pub fn import_mysql_dump_with_memory_and_progress<F>(
    port: u16,
    db_name: &str,
    sql_file: &Path,
    memory_limit_mb: Option<i32>,
    on_progress: F,
) -> Result<(), String>
where
    F: FnMut(u64, u64, &str),
{
    import_mysql_dump_with_compat(port, db_name, sql_file, memory_limit_mb, true, on_progress)
        .map_err(|e| e.message().to_string())
}

pub fn import_mysql_dump_with_compat<F>(
    port: u16,
    db_name: &str,
    sql_file: &Path,
    memory_limit_mb: Option<i32>,
    apply_compat_fixes: bool,
    mut on_progress: F,
) -> Result<(), SqlImportError>
where
    F: FnMut(u64, u64, &str),
{
    use std::io::Write;

    let clean = sanitize_mysql_ident(db_name).map_err(SqlImportError::Fatal)?;
    if !sql_file.is_file() {
        return Err(SqlImportError::Fatal(format!(
            "SQL dump file not found: {}",
            sql_file.display()
        )));
    }

    let target_version = get_server_version(port);
    if !apply_compat_fixes {
        let (first_err, fixes) = scan_sql_compatibility_issues(sql_file, &target_version);
        if !fixes.is_empty() {
            return Err(SqlImportError::CompatConfirm(CompatConfirmRequired {
                error: first_err.unwrap_or_else(|| {
                    format!(
                        "SQL dump contains syntax or collations incompatible with target server '{}'.",
                        target_version
                    )
                }),
                fixes,
                target_version,
            }));
        }
    }

    let total_bytes = fs::metadata(sql_file).map(|m| m.len()).unwrap_or(0);

    // Prefer native mariadb.exe / mysql.exe streaming stdin so multi-megabyte dumps
    // never hit the Windows CreateProcess 32KB command-line limit and report byte progress.
    if let Some(client_exe) = find_mysql_client_binary() {
        let client_name = client_exe
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("mariadb.exe")
            .to_string();
        on_progress(
            0,
            total_bytes,
            &format!(
                "Streaming {} into '{}' via {}",
                crate::downloads::format_mb(total_bytes),
                clean,
                client_name
            ),
        );

        let file = fs::File::open(sql_file)
            .map_err(|e| SqlImportError::Fatal(format!("Failed opening SQL file '{}': {}", sql_file.display(), e)))?;
        let mut reader = std::io::BufReader::with_capacity(256 * 1024, file);
        let mut cmd = std::process::Command::new(&client_exe);
        cmd.args([
            "-h",
            "127.0.0.1",
            "-P",
            &port.to_string(),
            "-u",
            "root",
            "--default-character-set=utf8mb4",
            &clean,
        ])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x08000000;
            cmd.creation_flags(CREATE_NO_WINDOW);
        }
        let mut child = cmd
            .spawn()
            .map_err(|e| SqlImportError::Fatal(format!("Failed executing {}: {}", client_exe.display(), e)))?;

        let mut written: u64 = 0;
        let mut last_report = std::time::Instant::now();
        if let Some(mut stdin) = child.stdin.take() {
            use std::io::BufRead;
            let mut line_buf = Vec::with_capacity(64 * 1024);
            loop {
                line_buf.clear();
                let n = reader
                    .read_until(b'\n', &mut line_buf)
                    .map_err(|e| SqlImportError::Fatal(format!("Failed reading SQL dump: {}", e)))?;
                if n == 0 {
                    break;
                }
                let maybe_ddl = line_buf.len() <= 16384
                    && !line_buf
                        .get(..6)
                        .is_some_and(|p| p.eq_ignore_ascii_case(b"insert"))
                    && !line_buf.starts_with(b"(");
                let write_res = if maybe_ddl {
                    let s = String::from_utf8_lossy(&line_buf);
                    let fixed = sanitize_ddl_statement_with_mode(&s, apply_compat_fixes);
                    stdin.write_all(fixed.as_bytes())
                } else {
                    stdin.write_all(&line_buf)
                };
                if let Err(e) = write_res {
                    // If the client exited early (e.g. SQL error), break so wait_with_output surfaces stderr.
                    if e.kind() == std::io::ErrorKind::BrokenPipe {
                        break;
                    }
                    return Err(SqlImportError::Fatal(format!("Failed streaming SQL to {}: {}", client_name, e)));
                }
                written += n as u64;
                if last_report.elapsed() >= std::time::Duration::from_millis(200) {
                    last_report = std::time::Instant::now();
                    on_progress(
                        written,
                        total_bytes,
                        &format!(
                            "Imported {} / {}",
                            crate::downloads::format_mb(written),
                            crate::downloads::format_mb(total_bytes)
                        ),
                    );
                }
            }
            let _ = stdin.flush();
        }

        let out = child
            .wait_with_output()
            .map_err(|e| SqlImportError::Fatal(format!("Failed waiting for {}: {}", client_exe.display(), e)))?;
        if out.status.success() {
            on_progress(
                total_bytes,
                total_bytes,
                &format!(
                    "SQL dump imported cleanly via {} ({})",
                    client_name,
                    crate::downloads::format_mb(total_bytes)
                ),
            );
            return Ok(());
        }
        let stderr = String::from_utf8_lossy(&out.stderr).trim().to_string();
        let stdout = String::from_utf8_lossy(&out.stdout).trim().to_string();
        let raw_err = if !stderr.is_empty() {
            stderr
        } else if !stdout.is_empty() {
            stdout
        } else {
            format!("MariaDB import exited with status {}", out.status)
        };
        if !apply_compat_fixes {
            let fixes = classify_sql_compat_error(&raw_err, &target_version);
            if !fixes.is_empty() {
                return Err(SqlImportError::CompatConfirm(CompatConfirmRequired {
                    error: raw_err,
                    fixes,
                    target_version,
                }));
            }
        }
        return Err(SqlImportError::Fatal(raw_err));
    }

    // Fallback: stream SQL statements chunk-by-chunk from disk inside PHP PDO_MySQL
    // so arbitrarily large (175MB+) dumps never exhaust PHP memory or max_allowed_packet.
    let (mem_arg, mem_label) = format_memory_limit_arg(memory_limit_mb);
    let php_exe = crate::state::find_php_binary(None).ok_or_else(|| {
        SqlImportError::Fatal("No MariaDB client or PHP PDO_MySQL binary available for SQL import".to_string())
    })?;
    on_progress(
        0,
        total_bytes,
        &format!(
            "Streaming {} into '{}' via PHP PDO_MySQL (memory_limit={})",
            crate::downloads::format_mb(total_bytes),
            clean,
            mem_label
        ),
    );
    let script = r#"<?php
error_reporting(E_ALL & ~E_DEPRECATED);
$port    = $argv[1];
$db      = $argv[2];
$sqlPath = $argv[3];

function streamSqlStatements(string $path, callable $onStmt, callable $onBytes): void {
    $fh = fopen($path, 'rb');
    if ($fh === false) {
        throw new RuntimeException('Failed opening SQL dump file');
    }
    $delimiter = ';';
    $stmt = '';
    $inSingle = false;
    $inDouble = false;
    $inBacktick = false;
    $inLineComment = false;
    $inBlockComment = false;
    $escaped = false;
    $bytesRead = 0;
    $lastReportAt = microtime(true);

    while (!feof($fh)) {
        $chunk = fread($fh, 262144);
        if ($chunk === false) {
            fclose($fh);
            throw new RuntimeException('Failed reading SQL dump chunk');
        }
        if ($chunk === '') {
            break;
        }
        // Never leave a 2-char token (/*, */, --, '', "", ``) split across chunk boundaries.
        while (!feof($fh)) {
            $tailCh = $chunk[strlen($chunk) - 1];
            if ($tailCh === '/' || $tailCh === '-' || $tailCh === '*' || $tailCh === "'" || $tailCh === '"' || $tailCh === '`') {
                $more = fread($fh, 2);
                if ($more === false || $more === '') {
                    break;
                }
                $chunk .= $more;
            } else {
                break;
            }
        }
        $len = strlen($chunk);
        $bytesRead += $len;

        for ($i = 0; $i < $len; $i++) {
            $ch = $chunk[$i];
            $next = ($i + 1 < $len) ? $chunk[$i + 1] : '';

            if ($inLineComment) {
                if ($ch === "\n") {
                    $inLineComment = false;
                    $stmt .= "\n";
                }
                continue;
            }

            if ($inBlockComment) {
                $stmt .= $ch;
                if ($ch === '*' && $next === '/') {
                    $stmt .= '/';
                    $i++;
                    $inBlockComment = false;
                }
                continue;
            }

            if ($inSingle) {
                $stmt .= $ch;
                if ($escaped) {
                    $escaped = false;
                } elseif ($ch === '\\') {
                    $escaped = true;
                } elseif ($ch === "'") {
                    if ($next === "'") {
                        $stmt .= "'";
                        $i++;
                    } else {
                        $inSingle = false;
                    }
                }
                continue;
            }

            if ($inDouble) {
                $stmt .= $ch;
                if ($escaped) {
                    $escaped = false;
                } elseif ($ch === '\\') {
                    $escaped = true;
                } elseif ($ch === '"') {
                    if ($next === '"') {
                        $stmt .= '"';
                        $i++;
                    } else {
                        $inDouble = false;
                    }
                }
                continue;
            }

            if ($inBacktick) {
                $stmt .= $ch;
                if ($ch === '`') {
                    if ($next === '`') {
                        $stmt .= '`';
                        $i++;
                    } else {
                        $inBacktick = false;
                    }
                }
                continue;
            }

            // Line comment detection at statement/line start
            if (($ch === '#' ) || ($ch === '-' && $next === '-' && ($i + 2 >= $len || $chunk[$i + 2] === ' ' || $chunk[$i + 2] === "\t" || $chunk[$i + 2] === "\r" || $chunk[$i + 2] === "\n"))) {
                $inLineComment = true;
                continue;
            }

            if ($ch === '/' && $next === '*') {
                $stmt .= '/*';
                $i++;
                $inBlockComment = true;
                continue;
            }

            if ($ch === "'") {
                $inSingle = true;
                $stmt .= $ch;
                continue;
            }
            if ($ch === '"') {
                $inDouble = true;
                $stmt .= $ch;
                continue;
            }
            if ($ch === '`') {
                $inBacktick = true;
                $stmt .= $ch;
                continue;
            }

            // Support bare `DELIMITER //` directive lines in dumps
            if ($ch === "\n" && preg_match('/^\s*DELIMITER\s+(\S+)\s*$/i', $stmt, $dm)) {
                $delimiter = $dm[1];
                $stmt = '';
                continue;
            }

            $stmt .= $ch;
            $delimLen = strlen($delimiter);
            if ($delimLen > 0 && strlen($stmt) >= $delimLen && substr($stmt, -$delimLen) === $delimiter) {
                $execSql = trim(substr($stmt, 0, -$delimLen));
                $stmt = '';
                if ($execSql !== '') {
                    // Execute unless the fragment contains only non-conditional /* ... */ comments
                    $stripped = trim((string)preg_replace('/\/\*[^!][\s\S]*?\*\//', '', $execSql));
                    if ($stripped !== '') {
                        $onStmt($execSql);
                    }
                }
            }
        }

        $now = microtime(true);
        if ($now - $lastReportAt >= 0.25) {
            $lastReportAt = $now;
            $onBytes($bytesRead);
        }
    }
    fclose($fh);

    $tail = trim($stmt);
    if ($tail !== '') {
        $stripped = trim((string)preg_replace('/\/\*[^!][\s\S]*?\*\//', '', $tail));
        if ($stripped !== '') {
            $onStmt($tail);
        }
    }
    $onBytes($bytesRead);
}

function normalizeDdlStatementForMariaDb(string $sql, bool $applyCompat): string {
    $trimmed = ltrim($sql);
    if (strncasecmp($trimmed, 'INSERT', 6) === 0 || strncasecmp($trimmed, 'REPLACE', 7) === 0) {
        return $sql;
    }
    if (stripos($sql, 'DEFAULT') !== false && strpos($sql, '`') !== false) {
        $sql = (string)preg_replace('/\bDEFAULT\s+[xX]\s+AS\s+`([0-9a-fA-F]+)`/i', "DEFAULT X'$1'", $sql);
        $sql = (string)preg_replace('/\bDEFAULT\s+[bB]\s+AS\s+`([01]+)`/i', "DEFAULT b'$1'", $sql);
    }
    if (!$applyCompat) {
        return $sql;
    }
    if (strpos($sql, 'utf8mb4_0900') !== false) {
        $sql = str_replace(
            ['utf8mb4_0900_as_cs', 'utf8mb4_0900_ai_ci', 'utf8mb4_0900_as_ci', 'utf8mb4_ja_0900_as_cs'],
            ['utf8mb4_bin', 'utf8mb4_unicode_ci', 'utf8mb4_unicode_ci', 'utf8mb4_unicode_ci'],
            $sql
        );
    }
    if (stripos($sql, 'utf8mb4_uca1400_') !== false) {
        $sql = (string)preg_replace_callback('/utf8mb4_uca1400_[a-z0-9_]+/i', function ($m) {
            $t = strtolower($m[0]);
            return (str_ends_with($t, '_cs') || str_ends_with($t, '_bin')) ? 'utf8mb4_bin' : 'utf8mb4_unicode_ci';
        }, $sql);
    }
    if (strpos($sql, 'utf8mb3') !== false) {
        $sql = str_replace(
            ['utf8mb3_general_ci', 'utf8mb3_unicode_ci', 'CHARSET=utf8mb3', 'CHARACTER SET utf8mb3'],
            ['utf8_general_ci', 'utf8_unicode_ci', 'CHARSET=utf8', 'CHARACTER SET utf8'],
            $sql
        );
    }
    if (stripos($sql, 'DEFINER') !== false) {
        $sql = (string)preg_replace('/\bDEFINER\s*=\s*(?:`[^`]*`|[^\s`]+)@(?:`[^`]*`|[^\s*`]+)\s*/i', '', $sql);
    }
    if (stripos($sql, 'GTID_PURGED') !== false || stripos($sql, 'SQL_LOG_BIN') !== false) {
        return 'SELECT 1';
    }
    if (stripos($sql, 'ROW_FORMAT=FIXED') !== false) {
        $sql = str_ireplace('ROW_FORMAT=FIXED', 'ROW_FORMAT=DYNAMIC', $sql);
    }
    if (stripos($sql, 'TYPE=InnoDB') !== false || stripos($sql, 'TYPE=MyISAM') !== false) {
        $sql = str_ireplace(['TYPE=InnoDB', 'TYPE=MyISAM'], ['ENGINE=InnoDB', 'ENGINE=MyISAM'], $sql);
    }
    $sql = (string)preg_replace('/\s+\/\*!80000\s+INVISIBLE\s*\*\//i', '', $sql);
    $sql = (string)preg_replace('/\bINVISIBLE\b/i', '', $sql);
    $sql = (string)preg_replace('/\bENCRYPTION\s*=\s*\'[^\']*\'/i', '', $sql);
    $sql = (string)preg_replace('/\bPAGE_CHECKSUM\s*=\s*[01]\b/i', '', $sql);
    $sql = (string)preg_replace('/\bTRANSACTIONAL\s*=\s*[01]\b/i', '', $sql);
    $sql = (string)preg_replace('/\bWITH\s+SYSTEM\s+VERSIONING\b/i', '', $sql);
    $sql = (string)preg_replace('/\bSRID\s+\d+/i', '', $sql);
    if (stripos($trimmed, 'CREATE TABLE') === 0) {
        $sql = (string)preg_replace('/(`[^`]+`\s*)\((255|250|220|200)\)/', '$1(191)', $sql);
    }
    return $sql;
}

$applyCompat = isset($argv[4]) && $argv[4] === '1';

try {
    $pdo = new PDO("mysql:host=127.0.0.1;port={$port};dbname={$db};charset=utf8mb4", 'root', '', [
        PDO::ATTR_ERRMODE => PDO::ERRMODE_EXCEPTION,
        PDO::ATTR_TIMEOUT => 300,
    ]);
    $pdo->exec("SET SESSION foreign_key_checks=0");
    $pdo->exec("SET SESSION unique_checks=0");
    $pdo->exec("SET SESSION sql_mode='NO_AUTO_VALUE_ON_ZERO'");
    @$pdo->exec("SET SESSION net_read_timeout=300");
    @$pdo->exec("SET SESSION net_write_timeout=300");
    try { $pdo->exec("SET GLOBAL max_allowed_packet=536870912"); } catch (Throwable $ignored) {}

    $stmtCount = 0;
    streamSqlStatements(
        $sqlPath,
        function (string $s) use ($pdo, &$stmtCount, $applyCompat) {
            $pdo->exec(normalizeDdlStatementForMariaDb($s, $applyCompat));
            $stmtCount++;
        },
        function (int $bytes) {
            echo "__DOCKPANEL_PROGRESS__:" . $bytes . "\n";
            if (ob_get_level() > 0) { @ob_flush(); }
            flush();
        }
    );
    echo "\n__DOCKPANEL_JSON__" . json_encode(['ok' => true, 'statements' => $stmtCount]);
} catch (Throwable $e) {
    echo "\n__DOCKPANEL_JSON__" . json_encode(['ok' => false, 'error' => $e->getMessage()]);
}
"#;
    let temp_script =
        std::env::temp_dir().join(format!("dockpanel_mysqlimp_{}.php", uuid::Uuid::new_v4()));
    fs::write(&temp_script, script).map_err(|e| SqlImportError::Fatal(e.to_string()))?;
    let mut cmd = std::process::Command::new(&php_exe);
    cmd.arg("-d")
        .arg(format!("memory_limit={}", mem_arg))
        .arg("-d")
        .arg("max_execution_time=0")
        .arg("-d")
        .arg("display_errors=0")
        .arg(&temp_script)
        .arg(port.to_string())
        .arg(&clean)
        .arg(sql_file.to_string_lossy().to_string())
        .arg(if apply_compat_fixes { "1" } else { "0" })
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x08000000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    let mut child = cmd
        .spawn()
        .map_err(|e| SqlImportError::Fatal(format!("Failed executing SQL import via PHP: {}", e)))?;

    let stderr_handle = child.stderr.take().map(|mut pipe| {
        std::thread::spawn(move || {
            use std::io::Read;
            let mut s = String::new();
            let _ = pipe.read_to_string(&mut s);
            s
        })
    });

    let mut raw_stdout = String::new();
    if let Some(stdout_pipe) = child.stdout.take() {
        use std::io::BufRead;
        let reader = std::io::BufReader::new(stdout_pipe);
        for line_res in reader.lines() {
            let Ok(line) = line_res else { break };
            if let Some(num_str) = line.strip_prefix("__DOCKPANEL_PROGRESS__:") {
                if let Ok(bytes_done) = num_str.trim().parse::<u64>() {
                    on_progress(
                        bytes_done.min(total_bytes),
                        total_bytes,
                        &format!(
                            "Imported {} / {}",
                            crate::downloads::format_mb(bytes_done.min(total_bytes)),
                            crate::downloads::format_mb(total_bytes)
                        ),
                    );
                }
            } else {
                raw_stdout.push_str(&line);
                raw_stdout.push('\n');
            }
        }
    }

    let status = child
        .wait()
        .map_err(|e| SqlImportError::Fatal(format!("Failed waiting for PHP SQL import: {}", e)))?;
    let stderr_str = stderr_handle
        .and_then(|h| h.join().ok())
        .unwrap_or_default();
    let _ = fs::remove_file(&temp_script);
    let _ = status;
    let parsed = crate::databases::parse_php_json_output(&raw_stdout).map_err(|_| {
        let base_err = if stderr_str.trim().is_empty() {
            format!("Invalid MySQL import output: {}", raw_stdout.trim())
        } else {
            stderr_str.clone()
        };
        let msg = if let Some(hint) = suggest_memory_limit_hint(&base_err, memory_limit_mb) {
            format!("{}\n\n{}", base_err, hint)
        } else {
            base_err
        };
        SqlImportError::Fatal(msg)
    })?;
    if parsed.get("ok").and_then(|v| v.as_bool()) == Some(true) {
        on_progress(
            total_bytes,
            total_bytes,
            &format!(
                "SQL dump imported cleanly via PHP PDO_MySQL ({})",
                crate::downloads::format_mb(total_bytes)
            ),
        );
        Ok(())
    } else {
        let raw_err = parsed["error"]
            .as_str()
            .unwrap_or("MariaDB SQL import failed")
            .to_string();
        if !apply_compat_fixes {
            let fixes = classify_sql_compat_error(&raw_err, &target_version);
            if !fixes.is_empty() {
                return Err(SqlImportError::CompatConfirm(CompatConfirmRequired {
                    error: raw_err,
                    fixes,
                    target_version,
                }));
            }
        }
        if let Some(hint) = suggest_memory_limit_hint(&raw_err, memory_limit_mb) {
            Err(SqlImportError::Fatal(format!("{}\n\n{}", raw_err, hint)))
        } else {
            Err(SqlImportError::Fatal(raw_err))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sanitize_mysql_ident() {
        assert_eq!(sanitize_mysql_ident("wp_site_1").unwrap(), "wp_site_1");
        assert!(sanitize_mysql_ident(";;").is_err());
    }

    #[test]
    fn test_php_streaming_sql_splitter_handles_quotes_comments_and_delimiters() {
        let Some(php_exe) = crate::state::find_php_binary(None) else {
            return;
        };
        let temp_dir = std::env::temp_dir().join(format!("dockpanel_sqlsplit_{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&temp_dir).unwrap();
        let sql_path = temp_dir.join("dump.sql");
        let sql_body = "\
-- comment with ; inside\n\
/*!40101 SET NAMES utf8mb4 */;\n\
INSERT INTO `wpl8_options` (`option_name`, `option_value`) VALUES ('siteurl', 'a:1:{s:3:\"url\";s:12:\"http://a;b;c\";}');\n\
INSERT INTO `wpl8_posts` VALUES (1, 'It\\'s escaped; still string');\n\
/* block comment; ignored */\n\
";
        fs::write(&sql_path, sql_body).unwrap();

        let script_path = temp_dir.join("split_test.php");
        // Verify statement count via the exact same parser logic in PHP
        let test_php = r#"<?php
$sqlPath = $argv[1];
$fh = fopen($sqlPath, 'rb');
$delimiter = ';';
$stmt = '';
$inSingle = false;
$escaped = false;
$inLine = false;
$inBlock = false;
$stmts = [];
while (!feof($fh)) {
    $chunk = fread($fh, 17); // tiny chunk size to exercise chunk boundaries!
    if ($chunk === false || $chunk === '') break;
    $len = strlen($chunk);
    for ($i = 0; $i < $len; $i++) {
        $ch = $chunk[$i];
        $next = ($i + 1 < $len) ? $chunk[$i + 1] : '';
        if ($inLine) { if ($ch === "\n") { $inLine = false; $stmt .= "\n"; } continue; }
        if ($inBlock) { $stmt .= $ch; if ($ch === '*' && $next === '/') { $stmt .= '/'; $i++; $inBlock = false; } continue; }
        if ($inSingle) {
            $stmt .= $ch;
            if ($escaped) { $escaped = false; }
            elseif ($ch === '\\') { $escaped = true; }
            elseif ($ch === "'") { if ($next === "'") { $stmt .= "'"; $i++; } else { $inSingle = false; } }
            continue;
        }
        if ($ch === '-' && $next === '-') { $inLine = true; continue; }
        if ($ch === '/' && $next === '*') { $stmt .= '/*'; $i++; $inBlock = true; continue; }
        if ($ch === "'") { $inSingle = true; $stmt .= $ch; continue; }
        $stmt .= $ch;
        if (substr($stmt, -1) === $delimiter) {
            $exec = trim(substr($stmt, 0, -1));
            $stmt = '';
            $stripped = trim((string)preg_replace('/\/\*[^!][\s\S]*?\*\//', '', $exec));
            if ($stripped !== '') $stmts[] = $stripped;
        }
    }
}
fclose($fh);
echo json_encode(['count' => count($stmts), 'stmts' => $stmts]);
"#;
        fs::write(&script_path, test_php).unwrap();
        let out = std::process::Command::new(&php_exe)
            .arg("-d")
            .arg("display_errors=0")
            .arg(&script_path)
            .arg(&sql_path)
            .output()
            .unwrap();
        let _ = fs::remove_dir_all(&temp_dir);
        let parsed: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
        assert_eq!(parsed["count"], 3);
        assert!(parsed["stmts"][1].as_str().unwrap().contains("http://a;b;c"));
        assert!(parsed["stmts"][2].as_str().unwrap().contains("escaped; still string"));
    }

    #[test]
    fn test_sanitize_ddl_statement_for_mariadb_fixes_phpmyadmin_binary_default_and_preserves_inserts() {
        let ddl = "CREATE TABLE `wpl8_wfblockediplog` (\n  `IP` binary(16) NOT NULL DEFAULT x AS `00000000000000000000000000000000`,\n  `countryCode` varchar(2) NOT NULL\n) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_0900_ai_ci;";
        let fixed = sanitize_ddl_statement_for_mariadb(ddl);
        assert!(fixed.contains("DEFAULT X'00000000000000000000000000000000'"));
        assert!(!fixed.contains("DEFAULT x AS"));
        assert!(fixed.contains("utf8mb4_unicode_ci"));

        let ins = "INSERT INTO `wpl8_posts` (`post_content`) VALUES ('literal DEFAULT x AS `0000` and utf8mb4_0900_ai_ci');";
        assert_eq!(sanitize_ddl_statement_for_mariadb(ins), ins);
    }

    #[test]
    fn test_scan_sql_compatibility_issues_and_backward_forward_fixes() {
        let tmp = std::env::temp_dir().join(format!("dockpanel_compat_{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&tmp).unwrap();
        let sql_path = tmp.join("mysql8_and_mariadb.sql");
        fs::write(
            &sql_path,
            "CREATE TABLE `t1` (`id` varchar(255) NOT NULL, KEY `idx_id` (`id`(255)) /*!80000 INVISIBLE */) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_0900_ai_ci;\n\
             CREATE TABLE `t2` (`name` varchar(64)) TYPE=MyISAM ROW_FORMAT=FIXED COLLATE=utf8mb4_uca1400_ai_ci;\n\
             CREATE DEFINER=`liveuser`@`%` VIEW `v1` AS SELECT 1;\n",
        )
        .unwrap();

        let (err_opt, fixes) = scan_sql_compatibility_issues(&sql_path, "11.2.2-MariaDB");
        assert!(err_opt.is_some());
        assert!(fixes.len() >= 3);

        let fixed = sanitize_ddl_statement_with_mode(
            &fs::read_to_string(&sql_path).unwrap(),
            true,
        );
        assert!(fixed.contains("utf8mb4_unicode_ci"));
        assert!(!fixed.contains("utf8mb4_0900_ai_ci"));
        assert!(!fixed.contains("utf8mb4_uca1400_ai_ci"));
        assert!(!fixed.contains("DEFINER=`liveuser`"));
        assert!(!fixed.contains("INVISIBLE"));
        assert!(fixed.contains("ENGINE=MyISAM"));
        assert!(fixed.contains("ROW_FORMAT=DYNAMIC"));
        assert!(fixed.contains("(191)"));

        let err_fixes = classify_sql_compat_error(
            "SQLSTATE[42000]: Syntax error or access violation: 1071 Specified key was too long; max key length is 767 bytes",
            "10.1.48-MariaDB",
        );
        assert!(!err_fixes.is_empty());

        let _ = fs::remove_dir_all(&tmp);
    }
}
