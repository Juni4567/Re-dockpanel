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

pub fn sanitize_db_name(name: &str) -> Result<String, String> {
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
    $affected = 0;

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
        $res = $pdo->exec($sql);
        $affected = is_int($res) ? $res : 0;
    }

    echo "\n__DOCKPANEL_JSON__" . json_encode(['ok' => true, 'columns' => $cols, 'rows' => $rows, 'row_count' => count($rows), 'affected_rows' => $affected], JSON_INVALID_UTF8_SUBSTITUTE);
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
            "affected_rows": parsed.get("affected_rows").and_then(|v| v.as_u64()).unwrap_or(0),
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

// ── Database Health, Optimization & Pruning Tools ──────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DbTableHealth {
    pub name: String,
    pub rows: u64,
    pub data_bytes: u64,
    pub index_bytes: u64,
    pub overhead_bytes: u64,
    pub engine: String,
    pub dead_tuples: u64,
    pub needs_optimize: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct WpBloatStats {
    pub detected: bool,
    pub table_prefix: String,
    pub revisions: u64,
    pub auto_drafts: u64,
    pub trashed_posts: u64,
    pub spam_comments: u64,
    pub trashed_comments: u64,
    pub orphaned_meta: u64,
    pub expired_transients: u64,
    pub total_transients: u64,
    pub autoload_bytes: u64,
    pub autoload_count: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DbHealthReport {
    pub database_id: String,
    pub database_name: String,
    pub engine: String,
    pub total_bytes: u64,
    pub data_bytes: u64,
    pub index_bytes: u64,
    pub overhead_bytes: u64,
    pub table_count: usize,
    pub fragmented_table_count: usize,
    pub dump_count: usize,
    pub dump_bytes: u64,
    pub integrity: String,
    pub tables: Vec<DbTableHealth>,
    pub wp_bloat: WpBloatStats,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DbOptimizeRequest {
    #[serde(default)]
    pub action: Option<String>,
    #[serde(default)]
    pub tables: Option<Vec<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DbOptimizeTableResult {
    pub table: String,
    pub status: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DbOptimizeReport {
    pub ok: bool,
    pub action: String,
    pub engine: String,
    pub tables_processed: usize,
    pub size_before_bytes: u64,
    pub size_after_bytes: u64,
    pub reclaimed_bytes: u64,
    pub integrity_status: String,
    pub results: Vec<DbOptimizeTableResult>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DbPruneRequest {
    #[serde(default)]
    pub dry_run: bool,
    #[serde(default)]
    pub categories: Vec<String>,
    #[serde(default)]
    pub keep_dumps: Option<usize>,
    #[serde(default)]
    pub older_than_days: Option<u64>,
    #[serde(default)]
    pub optimize_after: bool,
    #[serde(default)]
    pub truncate_tables: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DbPruneReport {
    pub ok: bool,
    pub dry_run: bool,
    pub rows_deleted: u64,
    pub dumps_deleted: usize,
    pub reclaimed_bytes: u64,
    pub optimized: bool,
    pub breakdown: serde_json::Value,
}

fn is_safe_sql_ident(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 128
        && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

fn parse_u64_cell(v: Option<&serde_json::Value>) -> u64 {
    v.and_then(|x| {
        x.as_u64()
            .or_else(|| x.as_str().and_then(|s| s.trim().parse::<u64>().ok()))
    })
    .unwrap_or(0)
}

fn detect_wp_prefix_from_tables(table_names: &[String]) -> Option<String> {
    let lower_set: std::collections::HashSet<String> =
        table_names.iter().map(|t| t.to_ascii_lowercase()).collect();
    for tbl in table_names {
        let lower = tbl.to_ascii_lowercase();
        if let Some(prefix) = lower.strip_suffix("options") {
            let has_posts = lower_set.contains(&format!("{}posts", prefix));
            let has_postmeta = lower_set.contains(&format!("{}postmeta", prefix));
            if (prefix.is_empty() || is_safe_sql_ident(prefix)) && has_posts && has_postmeta {
                return Some(tbl[..prefix.len()].to_string());
            }
        }
    }
    None
}

fn run_db_scalar_u64(db: &LocalDatabase, sql: &str) -> u64 {
    let res = if db.engine == "mariadb" {
        let Some(port) = db.port.and_then(|p| u16::try_from(p).ok()) else {
            return 0;
        };
        crate::mariadb::run_mysql_query(port, Some(&db.name), sql)
    } else {
        run_sqlite_query(&db.file_path, sql)
    };
    res.ok()
        .and_then(|v| v["rows"].as_array()?.first()?.as_array()?.first().cloned())
        .map(|cell| parse_u64_cell(Some(&cell)))
        .unwrap_or(0)
}

fn run_db_dml_count(db: &LocalDatabase, count_sql: &str, delete_sql: &str, dry_run: bool) -> Result<u64, String> {
    let count = run_db_scalar_u64(db, count_sql);
    if dry_run || count == 0 {
        return Ok(count);
    }
    if db.engine == "mariadb" {
        let port = db
            .port
            .and_then(|p| u16::try_from(p).ok())
            .ok_or_else(|| "MariaDB port unavailable".to_string())?;
        crate::mariadb::run_mysql_query(port, Some(&db.name), delete_sql)?;
    } else {
        let res = run_sqlite_query(&db.file_path, delete_sql)?;
        let affected = res["affected_rows"].as_u64().unwrap_or(count);
        return Ok(affected.max(count));
    }
    Ok(count)
}

fn collect_wp_bloat(db: &LocalDatabase, table_names: &[String]) -> WpBloatStats {
    let Some(prefix) = detect_wp_prefix_from_tables(table_names) else {
        return WpBloatStats::default();
    };
    let q = if db.engine == "mariadb" { '`' } else { '"' };
    let posts = format!("{q}{}posts{q}", prefix);
    let postmeta = format!("{q}{}postmeta{q}", prefix);
    let comments = format!("{q}{}comments{q}", prefix);
    let commentmeta = format!("{q}{}commentmeta{q}", prefix);
    let options = format!("{q}{}options{q}", prefix);

    let has_comments = table_names
        .iter()
        .any(|t| t.eq_ignore_ascii_case(&format!("{}comments", prefix)));
    let has_commentmeta = table_names
        .iter()
        .any(|t| t.eq_ignore_ascii_case(&format!("{}commentmeta", prefix)));

    let revisions = run_db_scalar_u64(
        db,
        &format!("SELECT COUNT(*) FROM {posts} WHERE post_type = 'revision'"),
    );
    let auto_drafts = run_db_scalar_u64(
        db,
        &format!("SELECT COUNT(*) FROM {posts} WHERE post_status = 'auto-draft'"),
    );
    let trashed_posts = run_db_scalar_u64(
        db,
        &format!("SELECT COUNT(*) FROM {posts} WHERE post_status = 'trash'"),
    );
    let spam_comments = if has_comments {
        run_db_scalar_u64(
            db,
            &format!("SELECT COUNT(*) FROM {comments} WHERE comment_approved = 'spam'"),
        )
    } else {
        0
    };
    let trashed_comments = if has_comments {
        run_db_scalar_u64(
            db,
            &format!("SELECT COUNT(*) FROM {comments} WHERE comment_approved = 'trash'"),
        )
    } else {
        0
    };
    let orphaned_postmeta = run_db_scalar_u64(
        db,
        &format!(
            "SELECT COUNT(*) FROM {postmeta} pm LEFT JOIN {posts} p ON p.ID = pm.post_id WHERE p.ID IS NULL"
        ),
    );
    let orphaned_commentmeta = if has_comments && has_commentmeta {
        run_db_scalar_u64(
            db,
            &format!(
                "SELECT COUNT(*) FROM {commentmeta} cm LEFT JOIN {comments} c ON c.comment_ID = cm.comment_id WHERE c.comment_ID IS NULL"
            ),
        )
    } else {
        0
    };
    let total_transients = run_db_scalar_u64(
        db,
        &format!(
            "SELECT COUNT(*) FROM {options} WHERE option_name LIKE '\\_transient\\_%' ESCAPE '\\' OR option_name LIKE '\\_site\\_transient\\_%' ESCAPE '\\'"
        ),
    );
    let now_epoch = chrono::Utc::now().timestamp();
    let int_cast = if db.engine == "mariadb" { "UNSIGNED" } else { "INTEGER" };
    let expired_transients = run_db_scalar_u64(
        db,
        &format!(
            "SELECT COUNT(*) FROM {options} WHERE (option_name LIKE '\\_transient\\_timeout\\_%' ESCAPE '\\' OR option_name LIKE '\\_site\\_transient\\_timeout\\_%' ESCAPE '\\') AND CAST(option_value AS {int_cast}) < {now_epoch}"
        ),
    );
    let autoload_count = run_db_scalar_u64(
        db,
        &format!("SELECT COUNT(*) FROM {options} WHERE autoload IN ('yes', 'on', 'auto', 'auto-on')"),
    );
    let autoload_bytes = if db.engine == "mariadb" {
        run_db_scalar_u64(
            db,
            &format!("SELECT COALESCE(SUM(OCTET_LENGTH(option_value)), 0) FROM {options} WHERE autoload IN ('yes', 'on', 'auto', 'auto-on')"),
        )
    } else {
        run_db_scalar_u64(
            db,
            &format!("SELECT COALESCE(SUM(LENGTH(CAST(option_value AS BLOB))), 0) FROM {options} WHERE autoload IN ('yes', 'on', 'auto', 'auto-on')"),
        )
    };

    WpBloatStats {
        detected: true,
        table_prefix: prefix,
        revisions,
        auto_drafts,
        trashed_posts,
        spam_comments,
        trashed_comments,
        orphaned_meta: orphaned_postmeta + orphaned_commentmeta,
        expired_transients,
        total_transients,
        autoload_bytes,
        autoload_count,
    }
}

pub fn database_dump_stats(db: &LocalDatabase) -> (usize, u64) {
    let dumps = list_dumps(db);
    let count = dumps.len();
    let bytes: u64 = dumps
        .iter()
        .filter_map(|d| d.get("size_bytes").and_then(|v| v.as_u64()))
        .sum();
    (count, bytes)
}

pub fn database_disk_size_bytes(db: &LocalDatabase) -> u64 {
    if db.engine == "mariadb" {
        if let Some(port) = db.port.and_then(|p| u16::try_from(p).ok()) {
            let sql = format!(
                "SELECT COALESCE(SUM(data_length + index_length + data_free), 0) FROM information_schema.TABLES WHERE table_schema='{}'",
                db.name.replace('\'', "''")
            );
            if let Ok(v) = crate::mariadb::run_mysql_query(port, None, &sql) {
                return parse_u64_cell(v["rows"][0].get(0));
            }
        }
        return 0;
    }
    let main = fs::metadata(&db.file_path).map(|m| m.len()).unwrap_or(0);
    let wal = fs::metadata(format!("{}-wal", db.file_path.display()))
        .map(|m| m.len())
        .unwrap_or(0);
    let journal = fs::metadata(format!("{}-journal", db.file_path.display()))
        .map(|m| m.len())
        .unwrap_or(0);
    main + wal + journal
}

pub fn inspect_database_health(db: &LocalDatabase) -> Result<DbHealthReport, String> {
    let (dump_count, dump_bytes) = database_dump_stats(db);

    if db.engine == "mariadb" {
        let port = db
            .port
            .and_then(|p| u16::try_from(p).ok())
            .ok_or_else(|| "MariaDB daemon is not running".to_string())?;
        let sql = format!(
            "SELECT table_name, COALESCE(engine, 'InnoDB'), COALESCE(table_rows, 0), COALESCE(data_length, 0), COALESCE(index_length, 0), COALESCE(data_free, 0) FROM information_schema.TABLES WHERE table_schema='{}' ORDER BY (data_length + index_length + data_free) DESC",
            db.name.replace('\'', "''")
        );
        let qr = crate::mariadb::run_mysql_query(port, None, &sql)?;
        let mut tables = Vec::new();
        let mut table_names = Vec::new();
        let mut data_bytes = 0u64;
        let mut index_bytes = 0u64;
        let mut overhead_bytes = 0u64;
        let mut fragmented = 0usize;

        if let Some(rows) = qr["rows"].as_array() {
            for r in rows {
                let Some(arr) = r.as_array() else { continue };
                let name = arr.first().and_then(|v| v.as_str()).unwrap_or("").to_string();
                if name.is_empty() {
                    continue;
                }
                let engine = arr.get(1).and_then(|v| v.as_str()).unwrap_or("InnoDB").to_string();
                let rows_cnt = parse_u64_cell(arr.get(2));
                let data_len = parse_u64_cell(arr.get(3));
                let idx_len = parse_u64_cell(arr.get(4));
                let free_len = parse_u64_cell(arr.get(5));
                let needs_opt = free_len >= 64 * 1024 || (data_len > 0 && free_len * 10 > data_len);
                if needs_opt {
                    fragmented += 1;
                }
                data_bytes += data_len;
                index_bytes += idx_len;
                overhead_bytes += free_len;
                table_names.push(name.clone());
                tables.push(DbTableHealth {
                    name,
                    rows: rows_cnt,
                    data_bytes: data_len,
                    index_bytes: idx_len,
                    overhead_bytes: free_len,
                    engine,
                    dead_tuples: 0,
                    needs_optimize: needs_opt,
                });
            }
        }

        let wp_bloat = collect_wp_bloat(db, &table_names);
        return Ok(DbHealthReport {
            database_id: db.id.clone(),
            database_name: db.name.clone(),
            engine: db.engine.clone(),
            total_bytes: data_bytes + index_bytes + overhead_bytes,
            data_bytes,
            index_bytes,
            overhead_bytes,
            table_count: tables.len(),
            fragmented_table_count: fragmented,
            dump_count,
            dump_bytes,
            integrity: "ok".to_string(),
            tables,
            wp_bloat,
        });
    }

    // SQLite inspection
    let page_count = run_db_scalar_u64(db, "PRAGMA page_count");
    let page_size = run_db_scalar_u64(db, "PRAGMA page_size").max(4096);
    let freelist_count = run_db_scalar_u64(db, "PRAGMA freelist_count");
    let overhead_bytes = freelist_count.saturating_mul(page_size);
    let wal_bytes = fs::metadata(format!("{}-wal", db.file_path.display()))
        .map(|m| m.len())
        .unwrap_or(0);
    let total_bytes = database_disk_size_bytes(db).max(page_count.saturating_mul(page_size));
    let data_bytes = total_bytes.saturating_sub(overhead_bytes + wal_bytes);

    let integrity_q = run_sqlite_query(&db.file_path, "PRAGMA integrity_check(1)").ok();
    let integrity = integrity_q
        .as_ref()
        .and_then(|v| v["rows"][0][0].as_str())
        .map(|s| if s.eq_ignore_ascii_case("ok") { "ok".to_string() } else { s.to_string() })
        .unwrap_or_else(|| "unknown".to_string());

    let tbl_res = run_sqlite_query(
        &db.file_path,
        "SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%' ORDER BY name",
    )?;
    let mut table_names = Vec::new();
    let mut tables = Vec::new();
    if let Some(rows) = tbl_res["rows"].as_array() {
        for r in rows {
            if let Some(name) = r.as_array().and_then(|a| a.first()).and_then(|v| v.as_str()) {
                if !is_safe_sql_ident(name) {
                    continue;
                }
                let row_cnt = run_db_scalar_u64(db, &format!("SELECT COUNT(*) FROM \"{}\"", name));
                table_names.push(name.to_string());
                tables.push(DbTableHealth {
                    name: name.to_string(),
                    rows: row_cnt,
                    data_bytes: 0,
                    index_bytes: 0,
                    overhead_bytes: 0,
                    engine: "SQLite".to_string(),
                    dead_tuples: 0,
                    needs_optimize: overhead_bytes > 4096,
                });
            }
        }
    }

    let wp_bloat = collect_wp_bloat(db, &table_names);
    Ok(DbHealthReport {
        database_id: db.id.clone(),
        database_name: db.name.clone(),
        engine: db.engine.clone(),
        total_bytes,
        data_bytes,
        index_bytes: 0,
        overhead_bytes: overhead_bytes + wal_bytes,
        table_count: tables.len(),
        fragmented_table_count: if overhead_bytes > 4096 { tables.len() } else { 0 },
        dump_count,
        dump_bytes,
        integrity,
        tables,
        wp_bloat,
    })
}

pub fn optimize_database(db: &LocalDatabase, req: &DbOptimizeRequest) -> Result<DbOptimizeReport, String> {
    let action = req
        .action
        .as_deref()
        .unwrap_or("optimize")
        .trim()
        .to_ascii_lowercase();
    if !matches!(action.as_str(), "optimize" | "analyze" | "repair" | "check") {
        return Err("Unsupported optimization action (expected optimize, analyze, repair, or check)".to_string());
    }

    let before = database_disk_size_bytes(db);

    if db.engine == "mariadb" {
        let port = db
            .port
            .and_then(|p| u16::try_from(p).ok())
            .ok_or_else(|| "MariaDB daemon is not running".to_string())?;
        let target_tables: Vec<String> = if let Some(ref requested) = req.tables {
            let mut valid = Vec::new();
            for t in requested {
                if !is_safe_sql_ident(t) {
                    return Err(format!("Invalid table name '{}'", t));
                }
                valid.push(t.clone());
            }
            valid
        } else {
            let qr = crate::mariadb::run_mysql_query(
                port,
                Some(&db.name),
                "SELECT table_name FROM information_schema.TABLES WHERE table_schema = DATABASE() ORDER BY table_name",
            )?;
            qr["rows"]
                .as_array()
                .map(|rows| {
                    rows.iter()
                        .filter_map(|r| r.as_array()?.first()?.as_str().map(String::from))
                        .filter(|t| is_safe_sql_ident(t))
                        .collect()
                })
                .unwrap_or_default()
        };

        let verb = match action.as_str() {
            "analyze" => "ANALYZE TABLE",
            "repair" => "REPAIR TABLE",
            "check" => "CHECK TABLE",
            _ => "OPTIMIZE TABLE",
        };

        let mut results = Vec::new();
        for tbl in &target_tables {
            let res = crate::mariadb::run_mysql_query(
                port,
                Some(&db.name),
                &format!("{} `{}`", verb, tbl),
            );
            match res {
                Ok(v) => {
                    let msg = v["rows"]
                        .as_array()
                        .and_then(|rows| rows.last())
                        .and_then(|r| r.get(3))
                        .and_then(|m| m.as_str())
                        .unwrap_or("OK")
                        .to_string();
                    results.push(DbOptimizeTableResult {
                        table: tbl.clone(),
                        status: "ok".to_string(),
                        message: msg,
                    });
                }
                Err(e) => {
                    results.push(DbOptimizeTableResult {
                        table: tbl.clone(),
                        status: "error".to_string(),
                        message: e,
                    });
                }
            }
        }

        let after = database_disk_size_bytes(db);
        return Ok(DbOptimizeReport {
            ok: true,
            action,
            engine: db.engine.clone(),
            tables_processed: results.len(),
            size_before_bytes: before,
            size_after_bytes: after,
            reclaimed_bytes: before.saturating_sub(after),
            integrity_status: "ok".to_string(),
            results,
        });
    }

    // SQLite optimization
    let _ = run_sqlite_query(&db.file_path, "PRAGMA wal_checkpoint(TRUNCATE)");
    let mut results = Vec::new();
    match action.as_str() {
        "check" => {
            let chk = run_sqlite_query(&db.file_path, "PRAGMA integrity_check")?;
            let msg = chk["rows"][0][0].as_str().unwrap_or("ok").to_string();
            results.push(DbOptimizeTableResult {
                table: db.name.clone(),
                status: if msg.eq_ignore_ascii_case("ok") { "ok" } else { "warn" }.to_string(),
                message: format!("integrity_check: {}", msg),
            });
        }
        "analyze" => {
            run_sqlite_query(&db.file_path, "ANALYZE")?;
            results.push(DbOptimizeTableResult {
                table: db.name.clone(),
                status: "ok".to_string(),
                message: "Query planner statistics updated (ANALYZE)".to_string(),
            });
        }
        _ => {
            run_sqlite_query(&db.file_path, "VACUUM")?;
            let _ = run_sqlite_query(&db.file_path, "PRAGMA optimize");
            let _ = run_sqlite_query(&db.file_path, "ANALYZE");
            results.push(DbOptimizeTableResult {
                table: db.name.clone(),
                status: "ok".to_string(),
                message: "Compacted free pages (VACUUM) & refreshed statistics".to_string(),
            });
        }
    }

    let integrity_q = run_sqlite_query(&db.file_path, "PRAGMA integrity_check(1)").ok();
    let integrity_status = integrity_q
        .as_ref()
        .and_then(|v| v["rows"][0][0].as_str())
        .unwrap_or("ok")
        .to_string();
    let after = database_disk_size_bytes(db);

    Ok(DbOptimizeReport {
        ok: true,
        action,
        engine: db.engine.clone(),
        tables_processed: results.len(),
        size_before_bytes: before,
        size_after_bytes: after,
        reclaimed_bytes: before.saturating_sub(after),
        integrity_status,
        results,
    })
}

pub fn prune_dumps(
    db: &LocalDatabase,
    keep_latest: Option<usize>,
    older_than_days: Option<u64>,
) -> Result<(usize, u64), String> {
    let dir = database_dumps_dir(db);
    if !dir.is_dir() {
        return Ok((0, 0));
    }

    let mut entries: Vec<(PathBuf, u64, std::time::SystemTime)> = Vec::new();
    if let Ok(rd) = fs::read_dir(&dir) {
        for entry in rd.flatten() {
            let p = entry.path();
            if !p.is_file() {
                continue;
            }
            let Ok(meta) = entry.metadata() else { continue };
            let mtime = meta.modified().unwrap_or(std::time::UNIX_EPOCH);
            entries.push((p, meta.len(), mtime));
        }
    }
    // Sort newest first
    entries.sort_by(|a, b| b.2.cmp(&a.2));

    let cutoff = older_than_days.and_then(|days| {
        std::time::SystemTime::now().checked_sub(std::time::Duration::from_secs(days * 86400))
    });

    let mut deleted = 0usize;
    let mut reclaimed = 0u64;
    for (idx, (path, len, mtime)) in entries.iter().enumerate() {
        let exceeds_keep = keep_latest.map(|k| idx >= k).unwrap_or(false);
        let exceeds_age = cutoff.map(|c| *mtime < c).unwrap_or(false);
        if exceeds_keep || exceeds_age {
            if fs::remove_file(path).is_ok() {
                deleted += 1;
                reclaimed += *len;
            }
        }
    }

    Ok((deleted, reclaimed))
}

pub fn prune_database(db: &LocalDatabase, req: &DbPruneRequest) -> Result<DbPruneReport, String> {
    let health_before = inspect_database_health(db)?;
    let size_before = database_disk_size_bytes(db);
    let mut breakdown = serde_json::Map::new();
    let mut rows_deleted = 0u64;
    let q = if db.engine == "mariadb" { '`' } else { '"' };

    if health_before.wp_bloat.detected {
        let prefix = &health_before.wp_bloat.table_prefix;
        let posts = format!("{q}{}posts{q}", prefix);
        let postmeta = format!("{q}{}postmeta{q}", prefix);
        let comments = format!("{q}{}comments{q}", prefix);
        let commentmeta = format!("{q}{}commentmeta{q}", prefix);
        let options = format!("{q}{}options{q}", prefix);
        let has_comments = health_before
            .tables
            .iter()
            .any(|t| t.name.eq_ignore_ascii_case(&format!("{}comments", prefix)));
        let has_commentmeta = health_before
            .tables
            .iter()
            .any(|t| t.name.eq_ignore_ascii_case(&format!("{}commentmeta", prefix)));

        for cat in &req.categories {
            let deleted = match cat.as_str() {
                "revisions" => run_db_dml_count(
                    db,
                    &format!("SELECT COUNT(*) FROM {posts} WHERE post_type = 'revision'"),
                    &format!("DELETE FROM {posts} WHERE post_type = 'revision'"),
                    req.dry_run,
                )?,
                "auto_drafts" => run_db_dml_count(
                    db,
                    &format!("SELECT COUNT(*) FROM {posts} WHERE post_status = 'auto-draft'"),
                    &format!("DELETE FROM {posts} WHERE post_status = 'auto-draft'"),
                    req.dry_run,
                )?,
                "trashed_posts" => run_db_dml_count(
                    db,
                    &format!("SELECT COUNT(*) FROM {posts} WHERE post_status = 'trash'"),
                    &format!("DELETE FROM {posts} WHERE post_status = 'trash'"),
                    req.dry_run,
                )?,
                "spam_comments" if has_comments => run_db_dml_count(
                    db,
                    &format!("SELECT COUNT(*) FROM {comments} WHERE comment_approved IN ('spam', 'trash')"),
                    &format!("DELETE FROM {comments} WHERE comment_approved IN ('spam', 'trash')"),
                    req.dry_run,
                )?,
                "orphaned_meta" => {
                    let pm = run_db_dml_count(
                        db,
                        &format!("SELECT COUNT(*) FROM {postmeta} pm LEFT JOIN {posts} p ON p.ID = pm.post_id WHERE p.ID IS NULL"),
                        &format!("DELETE FROM {postmeta} WHERE post_id NOT IN (SELECT ID FROM {posts})"),
                        req.dry_run,
                    )?;
                    let cm = if has_comments && has_commentmeta {
                        run_db_dml_count(
                            db,
                            &format!("SELECT COUNT(*) FROM {commentmeta} cm LEFT JOIN {comments} c ON c.comment_ID = cm.comment_id WHERE c.comment_ID IS NULL"),
                            &format!("DELETE FROM {commentmeta} WHERE comment_id NOT IN (SELECT comment_ID FROM {comments})"),
                            req.dry_run,
                        )?
                    } else {
                        0
                    };
                    pm + cm
                }
                "expired_transients" => {
                    let now_epoch = chrono::Utc::now().timestamp();
                    let int_cast = if db.engine == "mariadb" { "UNSIGNED" } else { "INTEGER" };
                    let expired_cnt = run_db_scalar_u64(
                        db,
                        &format!("SELECT COUNT(*) FROM {options} WHERE (option_name LIKE '\\_transient\\_timeout\\_%' ESCAPE '\\' OR option_name LIKE '\\_site\\_transient\\_timeout\\_%' ESCAPE '\\') AND CAST(option_value AS {int_cast}) < {now_epoch}"),
                    );
                    if !req.dry_run && expired_cnt > 0 {
                        if db.engine == "mariadb" {
                            let port = db.port.and_then(|p| u16::try_from(p).ok()).unwrap_or(3306);
                            let _ = crate::mariadb::run_mysql_query(
                                port,
                                Some(&db.name),
                                &format!("DELETE a, b FROM {options} a JOIN {options} b ON a.option_name = REPLACE(b.option_name, '_timeout_', '_') WHERE (b.option_name LIKE '\\_transient\\_timeout\\_%' ESCAPE '\\' OR b.option_name LIKE '\\_site\\_transient\\_timeout\\_%' ESCAPE '\\') AND CAST(b.option_value AS UNSIGNED) < {now_epoch}"),
                            );
                        } else {
                            let _ = run_sqlite_query(
                                &db.file_path,
                                &format!("DELETE FROM {options} WHERE option_name IN (SELECT REPLACE(option_name, '_timeout_', '_') FROM {options} WHERE (option_name LIKE '\\_transient\\_timeout\\_%' ESCAPE '\\' OR option_name LIKE '\\_site\\_transient\\_timeout\\_%' ESCAPE '\\') AND CAST(option_value AS INTEGER) < {now_epoch})"),
                            );
                        }
                        let _ = run_db_dml_count(
                            db,
                            &format!("SELECT COUNT(*) FROM {options} WHERE (option_name LIKE '\\_transient\\_timeout\\_%' ESCAPE '\\' OR option_name LIKE '\\_site\\_transient\\_timeout\\_%' ESCAPE '\\') AND CAST(option_value AS {int_cast}) < {now_epoch}"),
                            &format!("DELETE FROM {options} WHERE (option_name LIKE '\\_transient\\_timeout\\_%' ESCAPE '\\' OR option_name LIKE '\\_site\\_transient\\_timeout\\_%' ESCAPE '\\') AND CAST(option_value AS {int_cast}) < {now_epoch}"),
                            false,
                        );
                    }
                    expired_cnt
                }
                "all_transients" => run_db_dml_count(
                    db,
                    &format!("SELECT COUNT(*) FROM {options} WHERE option_name LIKE '\\_transient\\_%' ESCAPE '\\' OR option_name LIKE '\\_site\\_transient\\_%' ESCAPE '\\'"),
                    &format!("DELETE FROM {options} WHERE option_name LIKE '\\_transient\\_%' ESCAPE '\\' OR option_name LIKE '\\_site\\_transient\\_%' ESCAPE '\\'"),
                    req.dry_run,
                )?,
                _ => 0,
            };
            rows_deleted += deleted;
            breakdown.insert(cat.clone(), serde_json::json!(deleted));
        }
    }

    // Explicit table truncation (if requested)
    if !req.truncate_tables.is_empty() {
        let mut trunc_rows = 0u64;
        for tbl in &req.truncate_tables {
            if !is_safe_sql_ident(tbl) {
                return Err(format!("Invalid table name '{}'", tbl));
            }
            let del = if db.engine == "mariadb" {
                run_db_dml_count(
                    db,
                    &format!("SELECT COUNT(*) FROM `{}`", tbl),
                    &format!("TRUNCATE TABLE `{}`", tbl),
                    req.dry_run,
                )?
            } else {
                run_db_dml_count(
                    db,
                    &format!("SELECT COUNT(*) FROM \"{}\"", tbl),
                    &format!("DELETE FROM \"{}\"", tbl),
                    req.dry_run,
                )?
            };
            trunc_rows += del;
            rows_deleted += del;
        }
        breakdown.insert("truncated_rows".to_string(), serde_json::json!(trunc_rows));
    }

    let mut dumps_deleted = 0usize;
    let mut dumps_reclaimed = 0u64;
    if !req.dry_run && (req.keep_dumps.is_some() || req.older_than_days.is_some()) {
        let (cnt, bytes) = prune_dumps(db, req.keep_dumps, req.older_than_days)?;
        dumps_deleted = cnt;
        dumps_reclaimed = bytes;
        breakdown.insert("dumps_deleted".to_string(), serde_json::json!(cnt));
    }

    let mut optimized = false;
    let mut reclaimed_bytes = dumps_reclaimed;
    if !req.dry_run && req.optimize_after {
        let opt = optimize_database(
            db,
            &DbOptimizeRequest {
                action: Some("optimize".to_string()),
                tables: None,
            },
        )?;
        optimized = opt.ok;
        reclaimed_bytes += size_before.saturating_sub(opt.size_after_bytes);
    } else if !req.dry_run {
        let after = database_disk_size_bytes(db);
        reclaimed_bytes += size_before.saturating_sub(after);
    }

    Ok(DbPruneReport {
        ok: true,
        dry_run: req.dry_run,
        rows_deleted,
        dumps_deleted,
        reclaimed_bytes,
        optimized,
        breakdown: serde_json::Value::Object(breakdown),
    })
}

pub fn linked_database_names_and_ids(
    sites: &[LocalSite],
    sites_dir: &std::path::Path,
) -> (std::collections::HashSet<String>, std::collections::HashSet<String>) {
    let mut linked_names = std::collections::HashSet::new();
    let mut linked_ids = std::collections::HashSet::new();

    for site in sites {
        let doc_root = if !site.document_root.trim().is_empty() {
            PathBuf::from(&site.document_root)
        } else {
            sites_dir.join(&site.domain)
        };
        if doc_root.join("wp-content").join("database").join(".ht.sqlite").is_file() {
            linked_ids.insert(format!("wp:{}", site.id));
        }
        if let Some(db_name) = crate::wordpress::read_wp_mysql_db_name(&doc_root) {
            linked_names.insert(db_name.to_ascii_lowercase());
        }
        let env_path = doc_root.join(".env");
        if let Ok(raw) = fs::read_to_string(&env_path) {
            for line in raw.lines() {
                let trimmed = line.trim();
                if trimmed.starts_with("DB_DATABASE=") {
                    let val = trimmed["DB_DATABASE=".len()..]
                        .trim()
                        .trim_matches('"')
                        .trim_matches('\'');
                    if !val.is_empty() {
                        linked_names.insert(val.to_ascii_lowercase());
                    }
                }
            }
        }
        linked_names.insert(site.domain.replace(['.', '-'], "_").to_ascii_lowercase());
    }

    (linked_names, linked_ids)
}

pub fn delete_local_database(db: &LocalDatabase) -> Result<(), String> {
    if db.id.starts_with("wp:") {
        return Err("Cannot delete an active WordPress site's primary SQLite database directly; delete or switch the site instead".to_string());
    }
    if db.engine == "mariadb" {
        let port = db
            .port
            .and_then(|p| u16::try_from(p).ok())
            .ok_or_else(|| "MariaDB daemon is not running".to_string())?;
        return crate::mariadb::drop_database(port, &db.name);
    }
    if db.file_path.is_file() {
        fs::remove_file(&db.file_path).map_err(|e| format!("Failed deleting database file: {e}"))?;
        let _ = fs::remove_file(format!("{}-wal", db.file_path.display()));
        let _ = fs::remove_file(format!("{}-shm", db.file_path.display()));
    }
    Ok(())
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
