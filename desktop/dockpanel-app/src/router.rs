use serde::{Deserialize, Serialize};

fn default_port_80() -> u16 {
    80
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PortStatus {
    pub port: u16,
    pub in_use: bool,
    pub process_name: Option<String>,
    pub pid: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RouterConfig {
    #[serde(default = "default_port_80")]
    pub preferred_port: u16,
    #[serde(default = "default_port_80")]
    pub active_port: u16,
}

impl Default for RouterConfig {
    fn default() -> Self {
        Self {
            preferred_port: default_port_80(),
            active_port: default_active_port(),
        }
    }
}

fn default_active_port() -> u16 {
    80
}

/// Checks if a port is bindable on 127.0.0.1 and 0.0.0.0.
pub fn is_port_bindable(port: u16) -> bool {
    // Probe 127.0.0.1 first
    let l1 = match std::net::TcpListener::bind(("127.0.0.1", port)) {
        Ok(l) => l,
        Err(_) => return false,
    };
    drop(l1);

    // Probe 0.0.0.0 as well (e.g. Apache binding wildcard)
    match std::net::TcpListener::bind(("0.0.0.0", port)) {
        Ok(l2) => {
            drop(l2);
            true
        }
        Err(_) => false,
    }
}

/// Parses netstat -ano output to find the PID listening or bound to the target port.
pub fn parse_netstat_output(output: &str, target_port: u16) -> Option<u32> {
    let mut listening_pid = None;
    let mut other_pid = None;

    let port_str = target_port.to_string();

    for line in output.lines() {
        let line = line.trim();
        if !line.starts_with("TCP") && !line.starts_with("tcp") {
            continue;
        }

        let parts: Vec<&str> = line.split_whitespace().collect();
        // Standard TCP row: Proto, Local Address, Foreign Address, State, PID
        if parts.len() < 5 {
            continue;
        }

        let local_addr = parts[1];
        // Match address ending in :<target_port>
        let has_matching_port = local_addr
            .rsplit(':')
            .next()
            .map(|p| p == port_str)
            .unwrap_or(false);

        if !has_matching_port {
            continue;
        }

        let state = parts[3];
        if let Ok(pid) = parts[parts.len() - 1].parse::<u32>() {
            if state.eq_ignore_ascii_case("LISTENING") {
                listening_pid = Some(pid);
                break;
            } else if other_pid.is_none() {
                other_pid = Some(pid);
            }
        }
    }

    listening_pid.or(other_pid)
}

/// Parses tasklist CSV output to extract the image/executable name.
pub fn parse_tasklist_output(output: &str) -> Option<String> {
    for line in output.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('"') {
            let parts: Vec<&str> = trimmed.split("\",\"").collect();
            if !parts.is_empty() {
                let name = parts[0].trim_matches('"').trim();
                if !name.is_empty() && !name.starts_with("INFO:") {
                    return Some(name.to_string());
                }
            }
        }
    }
    None
}

/// Retrieves the process executable name for a given PID.
pub fn get_process_name_by_pid(pid: u32) -> Option<String> {
    if pid == 0 {
        return Some("System Idle Process".to_string());
    }
    if pid == 4 {
        return Some("System".to_string());
    }

    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x08000000;

        let output = std::process::Command::new("tasklist")
            .args(["/FI", &format!("PID eq {}", pid), "/FO", "CSV", "/NH"])
            .creation_flags(CREATE_NO_WINDOW)
            .output()
            .ok()?;

        if output.status.success() {
            let text = String::from_utf8_lossy(&output.stdout);
            return parse_tasklist_output(&text);
        }
    }

    None
}

/// Inspects the system to find the PID and process name holding `port`.
pub fn find_process_for_port(port: u16) -> (Option<u32>, Option<String>) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x08000000;

        if let Ok(output) = std::process::Command::new("netstat")
            .args(["-ano", "-p", "tcp"])
            .creation_flags(CREATE_NO_WINDOW)
            .output()
        {
            if output.status.success() {
                let text = String::from_utf8_lossy(&output.stdout);
                if let Some(pid) = parse_netstat_output(&text, port) {
                    let name = get_process_name_by_pid(pid);
                    return (Some(pid), name);
                }
            }
        }
    }

    (None, None)
}

/// Checks whether a TCP port is in use and identifies conflicting process details.
pub fn check_port_status(port: u16) -> PortStatus {
    let in_use = !is_port_bindable(port);
    if in_use {
        let (pid, process_name) = find_process_for_port(port);
        PortStatus {
            port,
            in_use: true,
            process_name,
            pid,
        }
    } else {
        PortStatus {
            port,
            in_use: false,
            process_name: None,
            pid: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_port_checker_available_port() {
        // Binding to port 0 checks random free port
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);
        let status = check_port_status(port);
        assert!(!status.in_use);
        assert_eq!(status.port, port);
        assert!(status.pid.is_none());
        assert!(status.process_name.is_none());
    }

    #[test]
    fn test_port_checker_occupied_port() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let status = check_port_status(port);
        assert!(status.in_use);
        assert_eq!(status.port, port);
        drop(listener);
    }

    #[test]
    fn test_check_port_status_occupied_apache_80() {
        let status = check_port_status(80);
        if status.in_use {
            assert_eq!(status.port, 80);
            assert!(status.pid.is_some());
            if let Some(ref name) = status.process_name {
                assert!(name.to_lowercase().contains("httpd") || name.to_lowercase().contains("system"));
            }
        }
    }

    #[test]
    fn test_parse_netstat_output() {
        let mock = r#"
Active Connections

  Proto  Local Address          Foreign Address        State           PID
  TCP    0.0.0.0:80             0.0.0.0:0              LISTENING       20408
  TCP    0.0.0.0:135            0.0.0.0:0              LISTENING       1184
  TCP    127.0.0.1:8080         0.0.0.0:0              LISTENING       5678
  TCP    192.168.1.10:60000     23.15.142.75:80        ESTABLISHED     3716
  TCP    [::]:80                [::]:0                 LISTENING       20408
"#;
        assert_eq!(parse_netstat_output(mock, 80), Some(20408));
        assert_eq!(parse_netstat_output(mock, 8080), Some(5678));
        assert_eq!(parse_netstat_output(mock, 135), Some(1184));
        assert_eq!(parse_netstat_output(mock, 9999), None);
        assert_eq!(parse_netstat_output(mock, 60000), Some(3716));
    }

    #[test]
    fn test_parse_tasklist_output() {
        let mock = "\"httpd.exe\",\"20408\",\"Console\",\"1\",\"21,312 K\"\r\n";
        assert_eq!(parse_tasklist_output(mock), Some("httpd.exe".to_string()));

        let empty = "INFO: No tasks are running which match the specified criteria.\r\n";
        assert_eq!(parse_tasklist_output(empty), None);
    }

    #[test]
    fn test_router_config_serialization() {
        let config = RouterConfig {
            preferred_port: 80,
            active_port: 8080,
        };
        let json = serde_json::to_string(&config).unwrap();
        let deserialized: RouterConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(config, deserialized);

        // Test defaults on empty JSON
        let default_deser: RouterConfig = serde_json::from_str("{}").unwrap();
        assert_eq!(default_deser.preferred_port, 80);
        assert_eq!(default_deser.active_port, 80);
    }
}
