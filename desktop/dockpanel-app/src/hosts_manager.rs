use std::fs;
use std::net::IpAddr;
use std::path::Path;

pub const DOCKPANEL_TAG: &str = "# dockpanel-desktop";
pub const DEFAULT_HOSTS_PATH: &str = r"C:\Windows\System32\drivers\etc\hosts";
pub const DEFAULT_IP: &str = "127.0.0.1";
pub const HOST_HELPER_PIPE: &str = r"\\.\pipe\dockpanel-host-helper";

pub fn is_host_helper_pipe_present() -> bool {
    #[cfg(windows)]
    {
        Path::new(HOST_HELPER_PIPE).exists()
    }
    #[cfg(not(windows))]
    {
        false
    }
}

pub fn validate_ip(ip: &str) -> Result<IpAddr, String> {
    let trimmed = ip.trim();
    if trimmed.is_empty() {
        return Err("IP address cannot be empty".to_string());
    }
    trimmed
        .parse::<IpAddr>()
        .map_err(|e| format!("Invalid IP address format: {e}"))
}

pub fn validate_domain(domain: &str) -> Result<&str, String> {
    let d = domain.trim();
    if d.is_empty() {
        return Err("Domain cannot be empty".to_string());
    }
    if d.len() > 253 {
        return Err("Domain cannot exceed 253 characters".to_string());
    }
    // Reject newlines, comments, slashes, whitespace, null bytes
    if d.contains(|c: char| {
        c == '\r' || c == '\n' || c == '#' || c == '/' || c == '\\' || c == '\0' || c.is_whitespace()
    }) {
        return Err(
            "Domain contains prohibited characters (newlines, comments, slashes, null bytes, or whitespace)"
                .to_string(),
        );
    }
    // Strictly only valid hostname characters: a-z, A-Z, 0-9, ., -
    if !d.chars().all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-') {
        return Err(
            "Domain contains invalid characters; only letters, digits, '.', and '-' are allowed"
                .to_string(),
        );
    }
    // Labels cannot be empty and cannot start or end with a hyphen
    for label in d.split('.') {
        if label.is_empty() {
            return Err("Domain cannot contain empty labels or consecutive/leading/trailing dots".to_string());
        }
        if label.starts_with('-') || label.ends_with('-') {
            return Err("Domain label cannot start or end with a hyphen".to_string());
        }
    }
    Ok(d)
}

pub fn is_dockpanel_line(line: &str) -> bool {
    line.to_ascii_lowercase().contains(DOCKPANEL_TAG)
}

pub fn line_matches_domain(line: &str, domain: &str) -> bool {
    let before_comment = match line.find('#') {
        Some(idx) => &line[..idx],
        None => line,
    };
    let tokens: Vec<&str> = before_comment.split_whitespace().collect();
    if tokens.len() >= 2 {
        tokens[1..].iter().any(|d| d.eq_ignore_ascii_case(domain))
    } else {
        false
    }
}

pub fn add_entry(hosts_content: &str, domain: &str, ip: &str) -> String {
    if validate_domain(domain).is_err() || validate_ip(ip).is_err() {
        return hosts_content.to_string();
    }

    let domain = domain.trim();
    let ip = ip.trim();
    let is_crlf = hosts_content.contains("\r\n");
    let newline = if is_crlf { "\r\n" } else { "\n" };

    let lines: Vec<String> = hosts_content
        .lines()
        .map(|l| l.to_string())
        .collect();

    let new_entry = format!("{ip} {domain} {DOCKPANEL_TAG}");

    let mut updated_existing = false;
    let mut new_lines = Vec::with_capacity(lines.len() + 1);
    for line in lines {
        if is_dockpanel_line(&line) && line_matches_domain(&line, domain) {
            if !updated_existing {
                new_lines.push(new_entry.clone());
                updated_existing = true;
            }
        } else {
            new_lines.push(line);
        }
    }

    if !updated_existing {
        new_lines.push(new_entry);
    }

    let mut result = new_lines.join(newline);
    if !result.is_empty() {
        result.push_str(newline);
    }
    result
}

pub fn remove_entry(hosts_content: &str, domain: &str) -> String {
    if validate_domain(domain).is_err() {
        return hosts_content.to_string();
    }

    let domain = domain.trim();
    let is_crlf = hosts_content.contains("\r\n");
    let newline = if is_crlf { "\r\n" } else { "\n" };

    let lines: Vec<String> = hosts_content
        .lines()
        .filter(|line| !(is_dockpanel_line(line) && line_matches_domain(line, domain)))
        .map(|l| l.to_string())
        .collect();

    let mut result = lines.join(newline);
    if !result.is_empty() && (hosts_content.ends_with('\n') || hosts_content.ends_with("\r\n")) {
        result.push_str(newline);
    }
    result
}

#[allow(dead_code)]
pub fn list_dockpanel_entries(hosts_content: &str) -> Vec<(String, String)> {
    let mut entries = Vec::new();

    for line in hosts_content.lines() {
        if is_dockpanel_line(line) {
            let before_comment = match line.find('#') {
                Some(idx) => &line[..idx],
                None => line,
            };
            let tokens: Vec<&str> = before_comment.split_whitespace().collect();
            if tokens.len() >= 2 {
                let ip = tokens[0];
                for d in &tokens[1..] {
                    entries.push((d.to_string(), ip.to_string()));
                }
            }
        }
    }

    entries
}

pub fn read_hosts_file(path: Option<&Path>) -> std::io::Result<String> {
    let p = path.unwrap_or_else(|| Path::new(DEFAULT_HOSTS_PATH));
    fs::read_to_string(p)
}

pub fn write_hosts_file_atomic(path: Option<&Path>, content: &str) -> std::io::Result<()> {
    let target = path.unwrap_or_else(|| Path::new(DEFAULT_HOSTS_PATH));
    if let Some(parent) = target.parent() {
        if !parent.as_os_str().is_empty() && !parent.exists() {
            fs::create_dir_all(parent)?;
        }
        let temp_path = parent.join(format!(".hosts.tmp.{}", uuid::Uuid::new_v4()));
        fs::write(&temp_path, content)?;
        if let Err(_) = fs::rename(&temp_path, target) {
            fs::write(target, content)?;
            let _ = fs::remove_file(&temp_path);
        }
    } else {
        fs::write(target, content)?;
    }
    Ok(())
}

#[allow(dead_code)]
pub fn add_host_entry(path: Option<&Path>, domain: &str, ip: &str) -> std::io::Result<()> {
    validate_domain(domain).map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidInput, e))?;
    let validated_ip = validate_ip(ip).map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidInput, e))?;
    let ip_str = validated_ip.to_string();
    let current = read_hosts_file(path).unwrap_or_default();
    let updated = add_entry(&current, domain, &ip_str);
    write_hosts_file_atomic(path, &updated)
}

#[allow(dead_code)]
pub fn remove_host_entry(path: Option<&Path>, domain: &str) -> std::io::Result<()> {
    validate_domain(domain).map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidInput, e))?;
    let current = read_hosts_file(path)?;
    let updated = remove_entry(&current, domain);
    write_hosts_file_atomic(path, &updated)
}

#[allow(dead_code)]
pub fn list_hosts_file_entries(path: Option<&Path>) -> std::io::Result<Vec<(String, String)>> {
    let current = read_hosts_file(path)?;
    Ok(list_dockpanel_entries(&current))
}

fn sync_domain_to_hosts_io(domain: &str, hosts_path: &Path) -> std::io::Result<()> {
    let clean_domain = validate_domain(domain)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidInput, e))?;
    let current = match read_hosts_file(Some(hosts_path)) {
        Ok(content) => content,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(e),
    };
    let updated = add_entry(&current, clean_domain, DEFAULT_IP);
    if updated == current && hosts_path.exists() {
        return Ok(());
    }
    write_hosts_file_atomic(Some(hosts_path), &updated)
}

fn remove_domain_from_hosts_io(domain: &str, hosts_path: &Path) -> std::io::Result<()> {
    let clean_domain = validate_domain(domain)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidInput, e))?;
    let current = match read_hosts_file(Some(hosts_path)) {
        Ok(content) => content,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(e),
    };
    let updated = remove_entry(&current, clean_domain);
    if updated == current {
        return Ok(());
    }
    write_hosts_file_atomic(Some(hosts_path), &updated)
}

#[allow(dead_code)]
pub fn sync_domain_to_hosts_with_path(domain: &str, hosts_path: &Path) -> Result<(), String> {
    sync_domain_to_hosts_io(domain, hosts_path).map_err(|e| e.to_string())
}

#[allow(dead_code)]
pub fn remove_domain_from_hosts_with_path(domain: &str, hosts_path: &Path) -> Result<(), String> {
    remove_domain_from_hosts_io(domain, hosts_path).map_err(|e| e.to_string())
}

fn notify_host_helper(action: &str, domain: &str, ip: &str) -> Result<(), String> {
    #[cfg(windows)]
    {
        use std::io::{BufRead, BufReader, Write};
        let mut last_err = None;
        for _attempt in 0..2 {
            match std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .open(HOST_HELPER_PIPE)
            {
                Ok(mut pipe) => {
                    let req = serde_json::json!({
                        "action": action,
                        "domain": domain,
                        "ip": ip
                    });
                    if let Err(e) = writeln!(pipe, "{}", req) {
                        last_err = Some(format!("Failed writing to host helper pipe: {e}"));
                        continue;
                    }
                    let _ = pipe.flush();
                    let mut reader = BufReader::new(pipe);
                    let mut resp_line = String::new();
                    if let Ok(n) = reader.read_line(&mut resp_line) {
                        if n > 0 {
                            if let Ok(val) = serde_json::from_str::<serde_json::Value>(&resp_line) {
                                if val.get("success").and_then(|v| v.as_bool()) == Some(true) {
                                    return Ok(());
                                }
                                let err_msg = val
                                    .get("error")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("Host helper returned failure");
                                last_err = Some(err_msg.to_string());
                                continue;
                            }
                        }
                    }
                    return Ok(());
                }
                Err(e) => {
                    last_err = Some(format!("Failed opening host helper pipe: {e}"));
                }
            }
        }
        Err(last_err.unwrap_or_else(|| "Failed to contact host helper pipe".to_string()))
    }
    #[cfg(not(windows))]
    {
        let _ = (action, domain, ip);
        Err("Host helper named pipe is only available on Windows".to_string())
    }
}

fn run_hidden_powershell(ps_command: &str) -> Result<(), String> {
    let mut cmd = std::process::Command::new("powershell");
    cmd.args(["-NoProfile", "-Command", ps_command])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped());

    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x08000000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }

    let output = cmd
        .output()
        .map_err(|e| format!("Failed to spawn PowerShell: {e}"))?;

    if output.status.success() {
        Ok(())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        if stderr.is_empty() {
            Err(format!("PowerShell exited with status {}", output.status))
        } else {
            Err(format!("PowerShell failed: {}", stderr))
        }
    }
}

fn execute_with_fallback<P, D, F>(
    domain: &str,
    action_label: &str,
    pipe_step: P,
    direct_step: D,
    ps_fallback_step: F,
) -> Result<(), String>
where
    P: FnOnce(&str) -> Result<(), String>,
    D: FnOnce(&str) -> std::io::Result<()>,
    F: FnOnce(&str) -> Result<(), String>,
{
    let clean_domain = validate_domain(domain)?;

    if pipe_step(clean_domain).is_ok() {
        return Ok(());
    }

    match direct_step(clean_domain) {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == std::io::ErrorKind::PermissionDenied => {
            match ps_fallback_step(clean_domain) {
                Ok(()) => Ok(()),
                Err(ps_err) => {
                    let msg = format!(
                        "Direct hosts write failed (PermissionDenied) and PowerShell fallback failed: {}",
                        ps_err
                    );
                    eprintln!(
                        "Warning: failed to {} domain '{}' in hosts file: {}",
                        action_label, clean_domain, msg
                    );
                    Err(msg)
                }
            }
        }
        Err(err) => {
            let msg = format!("Direct hosts write failed: {}", err);
            eprintln!(
                "Warning: failed to {} domain '{}' in hosts file: {}",
                action_label, clean_domain, msg
            );
            Err(msg)
        }
    }
}

pub fn sync_domain_to_hosts(domain: &str) -> Result<(), String> {
    let hosts_path = Path::new(DEFAULT_HOSTS_PATH);
    execute_with_fallback(
        domain,
        "sync",
        |d| notify_host_helper("add", d, DEFAULT_IP),
        |d| sync_domain_to_hosts_io(d, hosts_path),
        |d| {
            let ps_cmd = format!(
                "Add-Content -Path {} -Value '{} {} {}'",
                DEFAULT_HOSTS_PATH, DEFAULT_IP, d, DOCKPANEL_TAG
            );
            run_hidden_powershell(&ps_cmd)
        },
    )
}

pub fn remove_domain_from_hosts(domain: &str) -> Result<(), String> {
    let hosts_path = Path::new(DEFAULT_HOSTS_PATH);
    execute_with_fallback(
        domain,
        "remove",
        |d| notify_host_helper("remove", d, DEFAULT_IP),
        |d| remove_domain_from_hosts_io(d, hosts_path),
        |d| {
            let ps_cmd = format!(
                "$p = '{}'; $d = '{}'; $lines = Get-Content -Path $p | Where-Object {{ -not ($_ -match [regex]::Escape('{}') -and $_ -match ('(?i)\\s' + [regex]::Escape($d) + '(\\s|#|$)')) }}; Set-Content -Path $p -Value $lines",
                DEFAULT_HOSTS_PATH, d, DOCKPANEL_TAG
            );
            run_hidden_powershell(&ps_cmd)
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_hosts_path(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!("dockpanel_hosts_test_{}_{}", name, uuid::Uuid::new_v4()))
    }

    #[test]
    fn test_hosts_sync_adds_entry_and_preserves_existing() {
        let path = temp_hosts_path("add");
        let initial = "127.0.0.1 localhost\n::1 localhost\n192.168.1.50 intranet.corp # custom entry\n";
        fs::write(&path, initial).unwrap();

        sync_domain_to_hosts_with_path("mytestdomain.local", &path).unwrap();

        let content = fs::read_to_string(&path).unwrap();
        assert!(content.contains("127.0.0.1 localhost\n"));
        assert!(content.contains("::1 localhost\n"));
        assert!(content.contains("192.168.1.50 intranet.corp # custom entry\n"));
        assert!(content.contains("127.0.0.1 mytestdomain.local # dockpanel-desktop\n"));

        let _ = fs::remove_file(&path);
    }

    #[test]
    fn test_hosts_sync_duplicate_does_not_duplicate_lines() {
        let path = temp_hosts_path("dedup");
        let initial = "127.0.0.1 localhost\n127.0.0.2 wordpress.test # dockpanel-desktop\n";
        fs::write(&path, initial).unwrap();

        sync_domain_to_hosts_with_path("wordpress.test", &path).unwrap();
        sync_domain_to_hosts_with_path("wordpress.test", &path).unwrap();

        let content = fs::read_to_string(&path).unwrap();
        let matching: Vec<&str> = content
            .lines()
            .filter(|l| l.contains("wordpress.test"))
            .collect();
        assert_eq!(matching, vec!["127.0.0.1 wordpress.test # dockpanel-desktop"]);
        assert!(content.contains("127.0.0.1 localhost"));

        let _ = fs::remove_file(&path);
    }

    #[test]
    fn test_hosts_remove_removes_only_dockpanel_entry() {
        let path = temp_hosts_path("remove");
        let initial = concat!(
            "127.0.0.1 localhost\n",
            "127.0.0.1 shared.local\n",
            "127.0.0.1 shared.local # dockpanel-desktop\n",
            "127.0.0.1 other.local # dockpanel-desktop\n",
        );
        fs::write(&path, initial).unwrap();

        remove_domain_from_hosts_with_path("shared.local", &path).unwrap();

        let content = fs::read_to_string(&path).unwrap();
        assert!(content.contains("127.0.0.1 localhost\n"));
        assert!(content.contains("127.0.0.1 shared.local\n"));
        assert!(!content.contains("127.0.0.1 shared.local # dockpanel-desktop"));
        assert!(content.contains("127.0.0.1 other.local # dockpanel-desktop\n"));

        let _ = fs::remove_file(&path);
    }

    #[test]
    fn test_hosts_preserves_crlf_and_non_dockpanel_lines() {
        let path = temp_hosts_path("crlf");
        let initial = "# Header comment\r\n127.0.0.1 localhost\r\n";
        fs::write(&path, initial).unwrap();

        sync_domain_to_hosts_with_path("site-one.test", &path).unwrap();
        let added = fs::read_to_string(&path).unwrap();
        assert_eq!(
            added,
            "# Header comment\r\n127.0.0.1 localhost\r\n127.0.0.1 site-one.test # dockpanel-desktop\r\n"
        );

        remove_domain_from_hosts_with_path("site-one.test", &path).unwrap();
        let removed = fs::read_to_string(&path).unwrap();
        assert_eq!(removed, initial);

        let _ = fs::remove_file(&path);
    }

    #[test]
    fn test_hosts_validation_rejects_invalid_domains() {
        let path = temp_hosts_path("invalid");
        assert!(sync_domain_to_hosts_with_path("", &path).is_err());
        assert!(sync_domain_to_hosts_with_path("bad domain.local", &path).is_err());
        assert!(sync_domain_to_hosts_with_path("bad;domain.local", &path).is_err());
        assert!(remove_domain_from_hosts_with_path("bad#domain.local", &path).is_err());
        assert!(sync_domain_to_hosts("bad;domain.local").is_err());
        assert!(remove_domain_from_hosts("bad#domain.local").is_err());
        assert!(!path.exists());
    }

    #[test]
    fn test_hosts_fallback_tiers_and_permission_denied_resilience() {
        // 1. Pipe succeeds -> direct and PowerShell are not invoked
        let mut direct_called = false;
        let mut ps_called = false;
        let res = execute_with_fallback(
            "tier1.local",
            "sync",
            |_| Ok(()),
            |_| {
                direct_called = true;
                Ok(())
            },
            |_| {
                ps_called = true;
                Ok(())
            },
        );
        assert!(res.is_ok());
        assert!(!direct_called);
        assert!(!ps_called);

        // 2. Pipe fails, direct succeeds -> PowerShell is not invoked
        let mut ps_called = false;
        let res = execute_with_fallback(
            "tier2.local",
            "sync",
            |_| Err("no pipe".to_string()),
            |_| Ok(()),
            |_| {
                ps_called = true;
                Ok(())
            },
        );
        assert!(res.is_ok());
        assert!(!ps_called);

        // 3. Pipe fails, direct returns PermissionDenied -> PowerShell fallback invoked
        let mut ps_called = false;
        let res = execute_with_fallback(
            "tier3.local",
            "sync",
            |_| Err("no pipe".to_string()),
            |_| Err(std::io::Error::new(std::io::ErrorKind::PermissionDenied, "denied")),
            |d| {
                ps_called = true;
                assert_eq!(d, "tier3.local");
                Ok(())
            },
        );
        assert!(res.is_ok());
        assert!(ps_called);

        // 4. All tiers fail -> returns Err cleanly without panicking
        let res = execute_with_fallback(
            "tier4.local",
            "sync",
            |_| Err("no pipe".to_string()),
            |_| Err(std::io::Error::new(std::io::ErrorKind::PermissionDenied, "denied")),
            |_| Err("Access is denied".to_string()),
        );
        assert!(res.is_err());
        assert!(res.unwrap_err().contains("PowerShell fallback failed"));
    }
}
