use serde::{Deserialize, Serialize};
use serde_json::json;
use std::fs;
use std::path::PathBuf;

use crate::state::LocalSite;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LocalDatabase {
    pub id: String,
    pub site_id: Option<String>,
    pub stack_id: Option<String>,
    pub name: String,
    pub engine: String,
    pub db_user: String,
    pub container_id: Option<String>,
    pub port: Option<i32>,
    pub created_at: String,
    #[serde(skip)]
    pub file_path: PathBuf,
}

pub fn databases_dir() -> PathBuf {
    let user_profile = std::env::var("USERPROFILE").unwrap_or_else(|_| ".".into());
    PathBuf::from(user_profile).join("DockPanel").join("databases")
}

fn sanitize_db_name(name: &str) -> Result<String, String> {
    let clean: String = name
        .trim()
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '-')
        .collect();
    if clean.is_empty() {
        return Err("Database name must contain letters, digits, '-' or '_'".to_string());
    }
    Ok(clean)
}

pub fn discover_databases_with_mariadb(
    sites: &[LocalSite],
    sites_dir: &std::path::Path,
    mariadb_port: Option<u16>,
) -> Vec<LocalDatabase> {
    let mut list = Vec::new();
    let mut wp_mariadb_map: std::collections::HashMap<String, (String, String)> =
        std::collections::HashMap::new();

    // 1. Auto-discover WordPress SQLite and MariaDB databases inside local sites
    for site in sites {
        let doc_root = if !site.document_root.trim().is_empty() {
            PathBuf::from(&site.document_root)
        } else {
            sites_dir.join(&site.domain)
        };
        let wp_sqlite = doc_root.join("wp-content").join("database").join(".ht.sqlite");
        if wp_sqlite.is_file() {
            list.push(LocalDatabase {
                id: format!("wp:{}", site.id),
                site_id: Some(site.id.clone()),
                stack_id: None,
                name: format!("{} (WordPress SQLite)", site.domain),
                engine: "sqlite".to_string(),
                db_user: "local".to_string(),
                container_id: None,
                port: None,
                created_at: site.created_at.clone(),
                file_path: wp_sqlite,
            });
        } else if let Some(db_name) = crate::wordpress::read_wp_mysql_db_name(&doc_root) {
            wp_mariadb_map.insert(db_name, (site.id.clone(), site.created_at.clone()));
        }
    }

    // 2. Discover local MariaDB databases when MariaDB is active
    if let Some(port) = mariadb_port {
        if let Ok(names) = crate::mariadb::list_user_databases(port) {
            for db_name in names {
                let (site_id, created_at) = wp_mariadb_map
                    .get(&db_name)
                    .cloned()
                    .map(|(sid, ts)| (Some(sid), ts))
                    .unwrap_or_else(|| (None, chrono::Utc::now().to_rfc3339()));
                list.push(LocalDatabase {
                    id: format!("mariadb:{}", db_name),
                    site_id,
                    stack_id: None,
                    name: db_name,
                    engine: "mariadb".to_string(),
                    db_user: "root".to_string(),
                    container_id: None,
                    port: Some(port as i32),
                    created_at,
                    file_path: crate::mariadb::mariadb_data_dir(),
                });
            }
        }
    }

    // 3. Discover standalone local SQLite databases in %USERPROFILE%\DockPanel\databases
    let db_root = databases_dir();
    if let Ok(rd) = fs::read_dir(&db_root) {
        for entry in rd.flatten() {
            let p = entry.path();
            if p.is_file()
                && p.extension()
                    .and_then(|e| e.to_str())
                    .map(|e| e.eq_ignore_ascii_case("sqlite") || e.eq_ignore_ascii_case("db"))
                    .unwrap_or(false)
            {
                let stem = p
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("local")
                    .to_string();
                let fname = entry.file_name().to_string_lossy().to_string();
                list.push(LocalDatabase {
                    id: format!("local:{}", fname),
                    site_id: None,
                    stack_id: None,
                    name: stem,
                    engine: "sqlite".to_string(),
                    db_user: "local".to_string(),
                    container_id: None,
                    port: None,
                    created_at: chrono::Utc::now().to_rfc3339(),
                    file_path: p,
                });
            }
        }
    }

    list
}

#[allow(dead_code)]
pub fn discover_databases(sites: &[LocalSite], sites_dir: &std::path::Path) -> Vec<LocalDatabase> {
    discover_databases_with_mariadb(sites, sites_dir, None)
}

pub fn create_local_sqlite_db(name: &str) -> Result<LocalDatabase, String> {
    let clean = sanitize_db_name(name)?;
    let root = databases_dir();
    fs::create_dir_all(&root).map_err(|e| format!("Failed creating databases folder: {e}"))?;
    let file_name = format!("{clean}.sqlite");
    let path = root.join(&file_name);
    if path.exists() {
        return Err(format!("Database '{clean}' already exists"));
    }
    fs::write(&path, b"").map_err(|e| format!("Failed creating database file: {e}"))?;
    Ok(LocalDatabase {
        id: format!("local:{file_name}"),
        site_id: None,
        stack_id: None,
        name: clean,
        engine: "sqlite".to_string(),
        db_user: "local".to_string(),
        container_id: None,
        port: None,
        created_at: chrono::Utc::now().to_rfc3339(),
        file_path: path,
    })
}

/// Parses JSON emitted by PHP helper scripts even when row values contain `{...}`,
/// `\u0000`, or PHP warning prefixes before the JSON object.
pub fn parse_php_json_output(raw_stdout: &str) -> Result<serde_json::Value, String> {
    let trimmed = raw_stdout.trim();
    if let Some((_, after)) = trimmed.rsplit_once("__DOCKPANEL_JSON__") {
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(after.trim()) {
            return Ok(v);
        }
    }
    if let Ok(v) = serde_json::from_str::<serde_json::Value>(trimmed) {
        return Ok(v);
    }
    for (idx, _) in trimmed.match_indices('{') {
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(&trimmed[idx..]) {
            return Ok(v);
        }
    }
    for (idx, _) in trimmed.match_indices('[') {
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(&trimmed[idx..]) {
            return Ok(v);
        }
    }
    Err("Failed to parse PHP JSON output".to_string())
}

pub fn run_sqlite_query(db_path: &std::path::Path, sql: &str) -> Result<serde_json::Value, String> {
    let php_exe = crate::state::find_php_binary(None)
        .ok_or_else(|| "No PHP runtime with PDO_SQLite installed".to_string())?;

    let start = std::time::Instant::now();
    // Detect query type: SELECT-like statements use query() (returns a result set),
    // DDL/DML (CREATE, INSERT, UPDATE, DELETE, DROP, ALTER) use exec() which returns
    // affected row count or false. This is necessary because query() on DDL returns
    // false (no result set) on some PHP/SQLite configurations, breaking JSON output.
    let script = r#"<?php
$dbPath = $argv[1];
$sql    = $argv[2];
try {
    $pdo = new PDO('sqlite:' . $dbPath);
    $pdo->setAttribute(PDO::ATTR_ERRMODE, PDO::ERRMODE_EXCEPTION);

    // Determine whether this is a SELECT-type statement (stripping leading SQL comments)
    $trimmed  = ltrim(preg_replace('/^(?:--[^\n]*\n|\/\*[\s\S]*?\*\/\s*)*/', '', $sql));
    $head     = strtoupper(preg_replace('/\s+/', ' ', substr($trimmed, 0, 12)));
    $isSelect = str_starts_with($head, 'SELECT')
        || str_starts_with($head, 'PRAGMA')
        || str_starts_with($head, 'EXPLAIN')
        || str_starts_with($head, 'WITH');

    $cols = [];
    $rows = [];

    if ($isSelect) {
        $stmt = $pdo->query($sql);
        if ($stmt !== false) {
            $colCount = $stmt->columnCount();
            for ($i = 0; $i < $colCount; $i++) {
                $meta   = $stmt->getColumnMeta($i);
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
        // DDL / DML — use exec() which does not return a result set
        $affected = $pdo->exec($sql);
    }

    echo "\n__DOCKPANEL_JSON__" . json_encode(['ok' => true, 'columns' => $cols, 'rows' => $rows, 'row_count' => count($rows)], JSON_INVALID_UTF8_SUBSTITUTE);
} catch (Throwable $e) {
    echo "\n__DOCKPANEL_JSON__" . json_encode(['ok' => false, 'error' => $e->getMessage()], JSON_INVALID_UTF8_SUBSTITUTE);
}
"#;

    let temp_script = std::env::temp_dir().join(format!("dockpanel_sql_{}.php", uuid::Uuid::new_v4()));
    fs::write(&temp_script, script).map_err(|e| e.to_string())?;

    let mut cmd = std::process::Command::new(&php_exe);
    cmd.arg(&temp_script)
        .arg(db_path.to_string_lossy().to_string())
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

    let output = out.map_err(|e| format!("Failed executing query via PHP: {e}"))?;
    let raw_stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let parsed: serde_json::Value = parse_php_json_output(&raw_stdout)
        .map_err(|_| format!("Invalid query response: {}", String::from_utf8_lossy(&output.stderr)))?;

    if parsed.get("ok").and_then(|v| v.as_bool()) == Some(true) {
        Ok(json!({
            "columns": parsed["columns"],
            "rows": parsed["rows"],
            "row_count": parsed["row_count"],
            "execution_time_ms": elapsed_ms,
            "truncated": false
        }))
    } else {
        Err(parsed["error"]
            .as_str()
            .unwrap_or("SQL query failed")
            .to_string())
    }
}

fn validate_dump_filename(name: &str) -> Result<String, String> {
    let trimmed = name.trim();
    if trimmed.is_empty()
        || trimmed.contains("..")
        || trimmed.contains('/')
        || trimmed.contains('\\')
        || trimmed.contains(':')
    {
        return Err("Invalid dump filename".to_string());
    }
    Ok(trimmed.to_string())
}

pub fn database_dumps_dir(db: &LocalDatabase) -> PathBuf {
    let safe_id: String = db
        .id
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '_' })
        .collect();
    databases_dir().join("dumps").join(safe_id)
}

pub fn list_dumps(db: &LocalDatabase) -> Vec<serde_json::Value> {
    let dir = database_dumps_dir(db);
    if !dir.is_dir() {
        return Vec::new();
    }
    let mut items = Vec::new();
    if let Ok(rd) = fs::read_dir(&dir) {
        for entry in rd.flatten() {
            let p = entry.path();
            if p.is_file() {
                if let Ok(meta) = entry.metadata() {
                    let filename = entry.file_name().to_string_lossy().to_string();
                    let created_at = meta
                        .modified()
                        .ok()
                        .map(|t| chrono::DateTime::<chrono::Utc>::from(t).to_rfc3339())
                        .unwrap_or_else(|| chrono::Utc::now().to_rfc3339());
                    items.push(json!({
                        "filename": filename,
                        "size_bytes": meta.len(),
                        "created_at": created_at,
                    }));
                }
            }
        }
    }
    items.sort_by(|a, b| {
        b["created_at"]
            .as_str()
            .unwrap_or("")
            .cmp(a["created_at"].as_str().unwrap_or(""))
    });
    items
}

pub fn export_sqlite_dump(db: &LocalDatabase) -> Result<String, String> {
    let dir = database_dumps_dir(db);
    fs::create_dir_all(&dir).map_err(|e| format!("Failed creating dumps folder: {e}"))?;

    let clean_name = sanitize_db_name(&db.name).unwrap_or_else(|_| "database".to_string());
    let stamp = chrono::Utc::now().format("%Y%m%d-%H%M%S");
    let sql_filename = format!("{clean_name}_{stamp}.sql");
    let sql_path = dir.join(&sql_filename);

    // Try generating a clean SQL dump using PHP PDO_SQLite; fallback to binary .sqlite copy if PHP is absent.
    if let Some(php_exe) = crate::state::find_php_binary(None) {
        let script = r#"<?php
$dbPath = $argv[1];
$outPath = $argv[2];
try {
    $pdo = new PDO('sqlite:' . $dbPath);
    $pdo->setAttribute(PDO::ATTR_ERRMODE, PDO::ERRMODE_EXCEPTION);
    $lines = ["-- DockPanel Desktop SQLite Dump", "PRAGMA foreign_keys=OFF;", "BEGIN TRANSACTION;"];
    $tables = $pdo->query("SELECT name, sql FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%' ORDER BY name")->fetchAll(PDO::FETCH_ASSOC);
    foreach ($tables as $t) {
        $name = $t['name'];
        if (!empty($t['sql'])) {
            $lines[] = "DROP TABLE IF EXISTS \"" . str_replace('"', '""', $name) . "\";";
            $lines[] = $t['sql'] . ";";
        }
        $rows = $pdo->query("SELECT * FROM \"" . str_replace('"', '""', $name) . "\"")->fetchAll(PDO::FETCH_ASSOC);
        foreach ($rows as $row) {
            $cols = array_map(fn($c) => '"' . str_replace('"', '""', $c) . '"', array_keys($row));
            $vals = array_map(fn($v) => $v === null ? 'NULL' : $pdo->quote((string)$v), array_values($row));
            $lines[] = "INSERT INTO \"" . str_replace('"', '""', $name) . "\" (" . implode(',', $cols) . ") VALUES (" . implode(',', $vals) . ");";
        }
    }
    $lines[] = "COMMIT;";
    file_put_contents($outPath, implode("\n", $lines) . "\n");
    echo json_encode(['ok' => true]);
} catch (Throwable $e) {
    echo json_encode(['ok' => false, 'error' => $e->getMessage()]);
}
"#;
        let tmp = std::env::temp_dir().join(format!("dockpanel_dump_{}.php", uuid::Uuid::new_v4()));
        fs::write(&tmp, script).map_err(|e| e.to_string())?;
        let mut cmd = std::process::Command::new(&php_exe);
        cmd.arg(&tmp)
            .arg(db.file_path.to_string_lossy().to_string())
            .arg(sql_path.to_string_lossy().to_string());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x08000000;
            cmd.creation_flags(CREATE_NO_WINDOW);
        }
        let out = cmd.output();
        let _ = fs::remove_file(&tmp);
        if let Ok(output) = out {
            let stdout = String::from_utf8_lossy(&output.stdout);
            if stdout.contains("\"ok\":true") && sql_path.is_file() {
                return Ok(sql_filename);
            }
        }
    }

    let bin_filename = format!("{clean_name}_{stamp}.sqlite");
    fs::copy(&db.file_path, dir.join(&bin_filename))
        .map_err(|e| format!("Failed exporting database: {e}"))?;
    Ok(bin_filename)
}

fn decode_base64(input: &str) -> Result<Vec<u8>, String> {
    let mut out = Vec::with_capacity(input.len() * 3 / 4);
    let mut buf: u32 = 0;
    let mut bits: u8 = 0;
    for b in input.bytes() {
        if b == b'=' || b.is_ascii_whitespace() {
            continue;
        }
        let val = match b {
            b'A'..=b'Z' => b - b'A',
            b'a'..=b'z' => b - b'a' + 26,
            b'0'..=b'9' => b - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            _ => return Err("Invalid base64 character in upload".to_string()),
        };
        buf = (buf << 6) | (val as u32);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push(((buf >> bits) & 0xff) as u8);
        }
    }
    Ok(out)
}

pub fn upload_dump(
    db: &LocalDatabase,
    filename: &str,
    base64_content: &str,
    auto_import: bool,
) -> Result<String, String> {
    let clean = validate_dump_filename(filename)?;
    let bytes = decode_base64(base64_content)?;
    let dir = database_dumps_dir(db);
    fs::create_dir_all(&dir).map_err(|e| format!("Failed creating dumps folder: {e}"))?;
    let target = dir.join(&clean);
    fs::write(&target, bytes).map_err(|e| format!("Failed saving dump file: {e}"))?;
    if auto_import {
        import_dump(db, &clean)?;
    }
    Ok(clean)
}

pub fn import_dump(db: &LocalDatabase, filename: &str) -> Result<(), String> {
    let clean = validate_dump_filename(filename)?;
    let target = database_dumps_dir(db).join(&clean);
    if !target.is_file() {
        return Err("Dump file not found".to_string());
    }
    if clean.to_ascii_lowercase().ends_with(".sqlite") || clean.to_ascii_lowercase().ends_with(".db") {
        fs::copy(&target, &db.file_path).map_err(|e| format!("Failed restoring SQLite file: {e}"))?;
        return Ok(());
    }

    let php_exe = crate::state::find_php_binary(None)
        .ok_or_else(|| "No PHP runtime with PDO_SQLite installed".to_string())?;
    let script = r#"<?php
$dbPath = $argv[1];
$sqlPath = $argv[2];
try {
    $pdo = new PDO('sqlite:' . $dbPath);
    $pdo->setAttribute(PDO::ATTR_ERRMODE, PDO::ERRMODE_EXCEPTION);
    $sql = file_get_contents($sqlPath);
    $pdo->exec($sql);
    echo json_encode(['ok' => true]);
} catch (Throwable $e) {
    echo json_encode(['ok' => false, 'error' => $e->getMessage()]);
}
"#;
    let tmp = std::env::temp_dir().join(format!("dockpanel_imp_{}.php", uuid::Uuid::new_v4()));
    fs::write(&tmp, script).map_err(|e| e.to_string())?;
    let mut cmd = std::process::Command::new(&php_exe);
    cmd.arg(&tmp)
        .arg(db.file_path.to_string_lossy().to_string())
        .arg(target.to_string_lossy().to_string());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x08000000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    let out = cmd.output();
    let _ = fs::remove_file(&tmp);
    let output = out.map_err(|e| format!("Import failed: {e}"))?;
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let parsed: serde_json::Value = serde_json::from_str(&stdout)
        .map_err(|_| format!("Invalid import response: {}", String::from_utf8_lossy(&output.stderr)))?;
    if parsed.get("ok").and_then(|v| v.as_bool()) == Some(true) {
        Ok(())
    } else {
        Err(parsed["error"].as_str().unwrap_or("Import failed").to_string())
    }
}

pub fn delete_dump(db: &LocalDatabase, filename: &str) -> Result<(), String> {
    let clean = validate_dump_filename(filename)?;
    let target = database_dumps_dir(db).join(&clean);
    if !target.is_file() {
        return Err("Dump file not found".to_string());
    }
    fs::remove_file(&target).map_err(|e| format!("Failed deleting dump: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sanitize_db_name() {
        assert!(sanitize_db_name("../bad").is_ok()); // strips invalid chars -> "bad"
        assert!(sanitize_db_name("...").is_err());
        assert_eq!(sanitize_db_name("app_db-1").unwrap(), "app_db-1");
    }

    #[test]
    fn test_sqlite_query_null_update_delete_lifecycle() {
        if crate::state::find_php_binary(None).is_none() {
            return;
        }
        let temp_dir = std::env::temp_dir().join(format!("dockpanel_sqlite_crud_{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&temp_dir).unwrap();
        let db_path = temp_dir.join("test.sqlite");
        fs::write(&db_path, b"").unwrap();

        run_sqlite_query(&db_path, "CREATE TABLE opts (id INTEGER PRIMARY KEY, k TEXT, v TEXT)").unwrap();
        run_sqlite_query(&db_path, "INSERT INTO opts (k, v) VALUES ('siteurl', NULL)").unwrap();

        let res = run_sqlite_query(&db_path, "-- test comment\nSELECT id, k, v FROM opts").unwrap();
        assert_eq!(res["rows"][0][2], "\\N");

        run_sqlite_query(&db_path, "UPDATE \"opts\" SET \"v\" = 'https://example.test' WHERE \"id\" = '1'").unwrap();
        let after = run_sqlite_query(&db_path, "SELECT v FROM opts WHERE id = 1").unwrap();
        assert_eq!(after["rows"][0][0], "https://example.test");

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_parse_php_json_with_nested_braces_and_null_bytes() {
        let noisy = "PHP Deprecated: something\n__DOCKPANEL_JSON__{\"ok\":true,\"rows\":[[\"{\\\"tasks_meta_id\\\":2}\",\"O:32:\\\"ActionScheduler_IntervalSchedule\\\":1:{s:22:\\\"\\u0000*\\u0000ts\\\";i:1;}\"]]}";
        let parsed = parse_php_json_output(noisy).unwrap();
        assert_eq!(parsed["ok"], true);
        assert_eq!(parsed["rows"][0][0], "{\"tasks_meta_id\":2}");

        let fallback = "Warning: foo\n{\"ok\":true,\"rows\":[[\"O:3:{s:1:{}}\"]]}";
        let parsed2 = parse_php_json_output(fallback).unwrap();
        assert_eq!(parsed2["ok"], true);
    }
}
