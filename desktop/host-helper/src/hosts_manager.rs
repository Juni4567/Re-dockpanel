use std::fs;
use std::net::IpAddr;
use std::path::Path;

pub const DOCKPANEL_TAG: &str = "# dockpanel-desktop";
pub const DEFAULT_HOSTS_PATH: &str = r"C:\Windows\System32\drivers\etc\hosts";
pub const DEFAULT_IP: &str = "127.0.0.1";

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

    let mut lines: Vec<String> = hosts_content
        .lines()
        .map(|l| l.to_string())
        .collect();

    let new_entry = format!("{ip} {domain} {DOCKPANEL_TAG}");

    let mut updated_existing = false;
    for line in lines.iter_mut() {
        if is_dockpanel_line(line) && line_matches_domain(line, domain) {
            *line = new_entry.clone();
            updated_existing = true;
            break;
        }
    }

    if !updated_existing {
        lines.push(new_entry);
    }

    let mut result = lines.join(newline);
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
        let unique_suffix = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let temp_path = parent.join(format!(".hosts.tmp.{}", unique_suffix));
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

pub fn add_host_entry(path: Option<&Path>, domain: &str, ip: &str) -> std::io::Result<()> {
    validate_domain(domain).map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidInput, e))?;
    let validated_ip = validate_ip(ip).map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidInput, e))?;
    let ip_str = validated_ip.to_string();
    let current = read_hosts_file(path).unwrap_or_default();
    let updated = add_entry(&current, domain, &ip_str);
    write_hosts_file_atomic(path, &updated)
}

pub fn remove_host_entry(path: Option<&Path>, domain: &str) -> std::io::Result<()> {
    validate_domain(domain).map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidInput, e))?;
    let current = read_hosts_file(path)?;
    let updated = remove_entry(&current, domain);
    write_hosts_file_atomic(path, &updated)
}

pub fn list_hosts_file_entries(path: Option<&Path>) -> std::io::Result<Vec<(String, String)>> {
    let current = read_hosts_file(path)?;
    Ok(list_dockpanel_entries(&current))
}
