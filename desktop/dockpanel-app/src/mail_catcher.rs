use base64::Engine;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};

const MAX_INBOX_MESSAGES: usize = 500;
const MAX_EMAIL_BYTES: usize = 25 * 1024 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmailAttachment {
    pub filename: String,
    pub content_type: String,
    pub size_bytes: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CapturedEmail {
    pub id: String,
    pub from: String,
    pub to: Vec<String>,
    #[serde(default)]
    pub cc: Vec<String>,
    pub subject: String,
    pub text_body: String,
    pub html_body: String,
    pub raw: String,
    #[serde(default)]
    pub headers: HashMap<String, String>,
    #[serde(default)]
    pub attachments: Vec<EmailAttachment>,
    pub size_bytes: usize,
    pub read: bool,
    #[serde(default)]
    pub site_domain: Option<String>,
    pub received_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CapturedEmailSummary {
    pub id: String,
    pub from: String,
    pub to: Vec<String>,
    pub subject: String,
    pub snippet: String,
    pub has_html: bool,
    pub attachment_count: usize,
    pub size_bytes: usize,
    pub read: bool,
    pub site_domain: Option<String>,
    pub received_at: String,
}

impl From<&CapturedEmail> for CapturedEmailSummary {
    fn from(m: &CapturedEmail) -> Self {
        let preview_src = if !m.text_body.trim().is_empty() {
            m.text_body.trim().to_string()
        } else {
            strip_html_tags(&m.html_body)
        };
        let snippet: String = preview_src
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .chars()
            .take(140)
            .collect();
        Self {
            id: m.id.clone(),
            from: m.from.clone(),
            to: m.to.clone(),
            subject: m.subject.clone(),
            snippet,
            has_html: !m.html_body.trim().is_empty(),
            attachment_count: m.attachments.len(),
            size_bytes: m.size_bytes,
            read: m.read,
            site_domain: m.site_domain.clone(),
            received_at: m.received_at.clone(),
        }
    }
}

pub struct MailCatcherStore {
    inbox_path: PathBuf,
    messages: Mutex<Vec<CapturedEmail>>,
    smtp_port: Mutex<Option<u16>>,
    running: Mutex<bool>,
}

impl MailCatcherStore {
    pub fn new(config_dir: &Path) -> Arc<Self> {
        let _ = fs::create_dir_all(config_dir);
        let inbox_path = config_dir.join("mail_inbox.json");
        let messages = fs::read_to_string(&inbox_path)
            .ok()
            .and_then(|raw| serde_json::from_str::<Vec<CapturedEmail>>(&raw).ok())
            .unwrap_or_default();

        Arc::new(Self {
            inbox_path,
            messages: Mutex::new(messages),
            smtp_port: Mutex::new(None),
            running: Mutex::new(false),
        })
    }

    fn persist_locked(&self, msgs: &[CapturedEmail]) {
        if let Ok(json) = serde_json::to_string_pretty(msgs) {
            let tmp = self.inbox_path.with_extension("json.tmp");
            if fs::write(&tmp, &json).is_ok() {
                let _ = fs::rename(&tmp, &self.inbox_path);
            }
        }
    }

    pub fn set_listening(&self, port: u16) {
        if let Ok(mut p) = self.smtp_port.lock() {
            *p = Some(port);
        }
        if let Ok(mut r) = self.running.lock() {
            *r = true;
        }
    }

    pub fn smtp_port(&self) -> Option<u16> {
        self.smtp_port.lock().ok().and_then(|p| *p)
    }

    pub fn is_running(&self) -> bool {
        self.running.lock().map(|r| *r).unwrap_or(false)
    }

    pub fn status(&self) -> serde_json::Value {
        let msgs = self.messages.lock().unwrap();
        let total = msgs.len();
        let unread = msgs.iter().filter(|m| !m.read).count();
        serde_json::json!({
            "running": self.is_running(),
            "smtp_host": "127.0.0.1",
            "smtp_port": self.smtp_port(),
            "message_count": total,
            "unread_count": unread,
        })
    }

    pub fn insert_raw_email(
        &self,
        envelope_from: &str,
        envelope_to: &[String],
        raw_bytes: &[u8],
        known_domains: &[String],
    ) -> CapturedEmail {
        let mut parsed = parse_raw_email(envelope_from, envelope_to, raw_bytes);
        if parsed.site_domain.is_none() {
            parsed.site_domain = infer_site_domain(&parsed, known_domains);
        }
        let mut msgs = self.messages.lock().unwrap();
        msgs.insert(0, parsed.clone());
        if msgs.len() > MAX_INBOX_MESSAGES {
            msgs.truncate(MAX_INBOX_MESSAGES);
        }
        self.persist_locked(&msgs);
        parsed
    }

    pub fn list_summaries(
        &self,
        query: Option<&str>,
        domain: Option<&str>,
    ) -> Vec<CapturedEmailSummary> {
        let q = query.map(|s| s.trim().to_lowercase()).filter(|s| !s.is_empty());
        let d = domain.map(|s| s.trim().to_lowercase()).filter(|s| !s.is_empty());
        let msgs = self.messages.lock().unwrap();
        msgs.iter()
            .filter(|m| {
                if let Some(ref dom) = d {
                    let dom_match = m
                        .site_domain
                        .as_deref()
                        .map(|sd| sd.eq_ignore_ascii_case(dom))
                        .unwrap_or(false)
                        || m.from.to_lowercase().contains(dom)
                        || m.to.iter().any(|t| t.to_lowercase().contains(dom));
                    if !dom_match {
                        return false;
                    }
                }
                if let Some(ref needle) = q {
                    let hay = format!(
                        "{} {} {} {} {}",
                        m.subject,
                        m.from,
                        m.to.join(" "),
                        m.text_body,
                        m.site_domain.as_deref().unwrap_or("")
                    )
                    .to_lowercase();
                    if !hay.contains(needle) {
                        return false;
                    }
                }
                true
            })
            .map(CapturedEmailSummary::from)
            .collect()
    }

    pub fn get_message(&self, id: &str) -> Option<CapturedEmail> {
        let msgs = self.messages.lock().unwrap();
        msgs.iter().find(|m| m.id == id).cloned()
    }

    pub fn mark_read(&self, id: &str, read: bool) -> bool {
        let mut msgs = self.messages.lock().unwrap();
        if let Some(m) = msgs.iter_mut().find(|m| m.id == id) {
            if m.read != read {
                m.read = read;
                self.persist_locked(&msgs);
            }
            return true;
        }
        false
    }

    pub fn mark_all_read(&self) {
        let mut msgs = self.messages.lock().unwrap();
        let mut changed = false;
        for m in msgs.iter_mut() {
            if !m.read {
                m.read = true;
                changed = true;
            }
        }
        if changed {
            self.persist_locked(&msgs);
        }
    }

    pub fn delete_message(&self, id: &str) -> bool {
        let mut msgs = self.messages.lock().unwrap();
        let before = msgs.len();
        msgs.retain(|m| m.id != id);
        if msgs.len() != before {
            self.persist_locked(&msgs);
            true
        } else {
            false
        }
    }

    pub fn clear_messages(&self) -> usize {
        let mut msgs = self.messages.lock().unwrap();
        let count = msgs.len();
        if count > 0 {
            msgs.clear();
            self.persist_locked(&msgs);
        }
        count
    }

    pub fn count(&self) -> usize {
        self.messages.lock().map(|m| m.len()).unwrap_or(0)
    }
}

/// Starts the embedded local SMTP catcher on `127.0.0.1:preferred_port` (falling back
/// to another available loopback port if `preferred_port` is occupied).
pub async fn start_smtp_server<F, G>(
    store: Arc<MailCatcherStore>,
    preferred_port: u16,
    known_domains_fn: F,
    on_captured: G,
) -> Result<u16, String>
where
    F: Fn() -> Vec<String> + Send + Sync + 'static,
    G: Fn(&CapturedEmail) + Send + Sync + 'static,
{
    let listener = match TcpListener::bind(("127.0.0.1", preferred_port)).await {
        Ok(l) => l,
        Err(_) => TcpListener::bind(("127.0.0.1", 0))
            .await
            .map_err(|e| format!("Failed binding local SMTP catcher: {}", e))?,
    };

    let bound_port = listener
        .local_addr()
        .map_err(|e| format!("Failed reading SMTP listener addr: {}", e))?
        .port();
    store.set_listening(bound_port);

    let known_domains_fn = Arc::new(known_domains_fn);
    let on_captured = Arc::new(on_captured);

    tokio::spawn(async move {
        loop {
            match listener.accept().await {
                Ok((stream, _addr)) => {
                    let st = store.clone();
                    let domains_cb = known_domains_fn.clone();
                    let cap_cb = on_captured.clone();
                    tokio::spawn(async move {
                        let _ = handle_smtp_connection(stream, st, domains_cb, cap_cb).await;
                    });
                }
                Err(_) => break,
            }
        }
    });

    Ok(bound_port)
}

async fn handle_smtp_connection<F, G>(
    stream: TcpStream,
    store: Arc<MailCatcherStore>,
    known_domains_fn: Arc<F>,
    on_captured: Arc<G>,
) -> std::io::Result<()>
where
    F: Fn() -> Vec<String> + Send + Sync + 'static,
    G: Fn(&CapturedEmail) + Send + Sync + 'static,
{
    let (read_half, mut write_half) = stream.into_split();
    let mut reader = BufReader::new(read_half);

    write_half
        .write_all(b"220 dockpanel.desktop ESMTP Local Mail Catcher ready\r\n")
        .await?;

    let mut mail_from = String::new();
    let mut rcpt_to: Vec<String> = Vec::new();
    let mut in_data = false;
    let mut auth_login_step = 0u8;
    let mut data_buf: Vec<u8> = Vec::new();

    let mut line_buf = Vec::new();
    loop {
        line_buf.clear();
        let n = reader.read_until(b'\n', &mut line_buf).await?;
        if n == 0 {
            break;
        }

        if in_data {
            let trimmed_cr = line_buf.strip_suffix(b"\n").unwrap_or(&line_buf);
            let trimmed = trimmed_cr.strip_suffix(b"\r").unwrap_or(trimmed_cr);
            if trimmed == b"." {
                in_data = false;
                let domains = known_domains_fn();
                let captured = store.insert_raw_email(&mail_from, &rcpt_to, &data_buf, &domains);
                on_captured(&captured);
                mail_from.clear();
                rcpt_to.clear();
                data_buf.clear();
                write_half.write_all(b"250 2.0.0 Ok: queued locally\r\n").await?;
                continue;
            }
            if data_buf.len() + line_buf.len() <= MAX_EMAIL_BYTES {
                // RFC 5321 dot-unstuffing
                if trimmed.starts_with(b"..") {
                    data_buf.extend_from_slice(&line_buf[1..]);
                } else {
                    data_buf.extend_from_slice(&line_buf);
                }
            }
            continue;
        }

        let line_str = String::from_utf8_lossy(&line_buf).trim().to_string();
        if auth_login_step == 1 {
            auth_login_step = 2;
            write_half.write_all(b"334 UGFzc3dvcmQ6\r\n").await?;
            continue;
        } else if auth_login_step == 2 {
            auth_login_step = 0;
            write_half.write_all(b"235 2.7.0 Authentication successful\r\n").await?;
            continue;
        }

        let upper = line_str.to_ascii_uppercase();
        if upper.starts_with("EHLO") {
            write_half
                .write_all(
                    b"250-dockpanel.desktop\r\n250-SIZE 26214400\r\n250-8BITMIME\r\n250 AUTH PLAIN LOGIN\r\n",
                )
                .await?;
        } else if upper.starts_with("HELO") {
            write_half.write_all(b"250 dockpanel.desktop\r\n").await?;
        } else if upper.starts_with("AUTH LOGIN") && upper.trim() == "AUTH LOGIN" {
            auth_login_step = 1;
            write_half.write_all(b"334 VXNlcm5hbWU6\r\n").await?;
        } else if upper.starts_with("AUTH ") {
            write_half.write_all(b"235 2.7.0 Authentication successful\r\n").await?;
        } else if upper.starts_with("MAIL FROM:") {
            mail_from = extract_smtp_addr(&line_str[10..]);
            rcpt_to.clear();
            write_half.write_all(b"250 2.1.0 Ok\r\n").await?;
        } else if upper.starts_with("RCPT TO:") {
            let addr = extract_smtp_addr(&line_str[8..]);
            if !addr.is_empty() {
                rcpt_to.push(addr);
            }
            write_half.write_all(b"250 2.1.5 Ok\r\n").await?;
        } else if upper == "DATA" {
            in_data = true;
            data_buf.clear();
            write_half
                .write_all(b"354 End data with <CR><LF>.<CR><LF>\r\n")
                .await?;
        } else if upper == "RSET" {
            mail_from.clear();
            rcpt_to.clear();
            data_buf.clear();
            write_half.write_all(b"250 2.0.0 Ok\r\n").await?;
        } else if upper == "NOOP" {
            write_half.write_all(b"250 2.0.0 Ok\r\n").await?;
        } else if upper == "QUIT" {
            write_half.write_all(b"221 2.0.0 Bye\r\n").await?;
            break;
        } else {
            write_half.write_all(b"250 Ok\r\n").await?;
        }
    }

    Ok(())
}

/// Sends a test email directly through the local SMTP catcher over TCP so users can
/// verify local mail capture with one click.
pub async fn send_test_email_smtp(
    port: u16,
    from: &str,
    to: &str,
    subject: &str,
    text_body: &str,
    html_body: Option<&str>,
) -> Result<(), String> {
    let stream = TcpStream::connect(("127.0.0.1", port))
        .await
        .map_err(|e| format!("Failed connecting to local SMTP port {}: {}", port, e))?;
    let (read_half, mut write_half) = stream.into_split();
    let mut reader = BufReader::new(read_half);

    async fn expect_reply(
        reader: &mut BufReader<tokio::net::tcp::OwnedReadHalf>,
        expected_prefix: &str,
    ) -> Result<(), String> {
        let mut line = String::new();
        loop {
            line.clear();
            let n = reader.read_line(&mut line).await.map_err(|e| e.to_string())?;
            if n == 0 {
                return Err("Unexpected EOF from SMTP server".to_string());
            }
            let bytes = line.as_bytes();
            // Multi-line SMTP responses have '-' at index 3; final line has ' ' or ends.
            if bytes.len() >= 4 && bytes[3] == b'-' {
                continue;
            }
            if !line.starts_with(expected_prefix) {
                return Err(format!("Unexpected SMTP reply: {}", line.trim()));
            }
            return Ok(());
        }
    }

    expect_reply(&mut reader, "220").await?;
    write_half.write_all(b"EHLO localhost\r\n").await.map_err(|e| e.to_string())?;
    expect_reply(&mut reader, "250").await?;

    write_half
        .write_all(format!("MAIL FROM:<{}>\r\n", from).as_bytes())
        .await
        .map_err(|e| e.to_string())?;
    expect_reply(&mut reader, "250").await?;

    write_half
        .write_all(format!("RCPT TO:<{}>\r\n", to).as_bytes())
        .await
        .map_err(|e| e.to_string())?;
    expect_reply(&mut reader, "250").await?;

    write_half.write_all(b"DATA\r\n").await.map_err(|e| e.to_string())?;
    expect_reply(&mut reader, "354").await?;

    let date_hdr = chrono::Utc::now().to_rfc2822();
    let msg = if let Some(html) = html_body {
        let boundary = format!("dockpanel-{}", uuid::Uuid::new_v4().simple());
        format!(
            "From: {}\r\nTo: {}\r\nSubject: {}\r\nDate: {}\r\nMIME-Version: 1.0\r\nContent-Type: multipart/alternative; boundary=\"{}\"\r\n\r\n--{}\r\nContent-Type: text/plain; charset=utf-8\r\n\r\n{}\r\n--{}\r\nContent-Type: text/html; charset=utf-8\r\n\r\n{}\r\n--{}--\r\n.\r\n",
            from, to, subject, date_hdr, boundary, boundary, text_body, boundary, html, boundary
        )
    } else {
        format!(
            "From: {}\r\nTo: {}\r\nSubject: {}\r\nDate: {}\r\nContent-Type: text/plain; charset=utf-8\r\n\r\n{}\r\n.\r\n",
            from, to, subject, date_hdr, text_body
        )
    };

    write_half.write_all(msg.as_bytes()).await.map_err(|e| e.to_string())?;
    expect_reply(&mut reader, "250").await?;

    let _ = write_half.write_all(b"QUIT\r\n").await;
    Ok(())
}

fn extract_smtp_addr(raw: &str) -> String {
    let trimmed = raw.trim();
    if let (Some(start), Some(end)) = (trimmed.find('<'), trimmed.find('>')) {
        if end > start {
            return trimmed[start + 1..end].trim().to_string();
        }
    }
    trimmed
        .split_whitespace()
        .next()
        .unwrap_or("")
        .trim_matches(|c| c == '<' || c == '>')
        .to_string()
}

fn split_headers_and_body(raw: &str) -> (&str, &str) {
    if let Some(pos) = raw.find("\r\n\r\n") {
        (&raw[..pos], &raw[pos + 4..])
    } else if let Some(pos) = raw.find("\n\n") {
        (&raw[..pos], &raw[pos + 2..])
    } else {
        (raw, "")
    }
}

fn parse_headers(raw_headers: &str) -> HashMap<String, String> {
    let mut unfolded = Vec::<String>::new();
    for line in raw_headers.lines() {
        let line = line.trim_end_matches('\r');
        if (line.starts_with(' ') || line.starts_with('\t')) && !unfolded.is_empty() {
            if let Some(last) = unfolded.last_mut() {
                last.push(' ');
                last.push_str(line.trim());
            }
        } else if !line.is_empty() {
            unfolded.push(line.to_string());
        }
    }

    let mut map = HashMap::new();
    for entry in unfolded {
        if let Some((k, v)) = entry.split_once(':') {
            let key = k.trim().to_lowercase();
            let val = decode_rfc2047(v.trim());
            map.insert(key, val);
        }
    }
    map
}

fn decode_rfc2047(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut rest = input;
    while let Some(start) = rest.find("=?") {
        out.push_str(&rest[..start]);
        let after_prefix = &rest[start + 2..];
        let Some(q1) = after_prefix.find('?') else {
            out.push_str(&rest[start..]);
            return out;
        };
        let after_charset = &after_prefix[q1 + 1..];
        let Some(q2) = after_charset.find('?') else {
            out.push_str(&rest[start..]);
            return out;
        };
        let encoding = &after_charset[..q2];
        let after_enc = &after_charset[q2 + 1..];
        let Some(end) = after_enc.find("?=") else {
            out.push_str(&rest[start..]);
            return out;
        };
        let encoded_text = &after_enc[..end];
        let decoded = if encoding.eq_ignore_ascii_case("B") {
            base64::engine::general_purpose::STANDARD
                .decode(encoded_text.trim())
                .ok()
                .map(|b| String::from_utf8_lossy(&b).to_string())
        } else if encoding.eq_ignore_ascii_case("Q") {
            let underscored = encoded_text.replace('_', " ");
            Some(decode_quoted_printable(&underscored))
        } else {
            None
        };
        match decoded {
            Some(s) => out.push_str(&s),
            None => out.push_str(&rest[start..start + 2 + q1 + 1 + q2 + 1 + end + 2]),
        }
        rest = &after_enc[end + 2..];
        // Collapse linear whitespace between adjacent encoded words per RFC 2047
        if rest.trim_start().starts_with("=?") {
            rest = rest.trim_start();
        }
    }
    out.push_str(rest);
    out
}

fn decode_quoted_printable(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'=' {
            if i + 2 < bytes.len() && bytes[i + 1] == b'\r' && bytes[i + 2] == b'\n' {
                i += 3;
                continue;
            }
            if i + 1 < bytes.len() && bytes[i + 1] == b'\n' {
                i += 2;
                continue;
            }
            if i + 2 < bytes.len() {
                if let (Some(hi), Some(lo)) = (hex_val(bytes[i + 1]), hex_val(bytes[i + 2])) {
                    out.push((hi << 4) | lo);
                    i += 3;
                    continue;
                }
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).to_string()
}

fn hex_val(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

fn decode_transfer_body(body: &str, encoding: &str) -> String {
    let enc = encoding.trim().to_ascii_lowercase();
    if enc == "base64" {
        let compact: String = body.chars().filter(|c| !c.is_whitespace()).collect();
        if let Ok(decoded) = base64::engine::general_purpose::STANDARD.decode(&compact) {
            return String::from_utf8_lossy(&decoded).to_string();
        }
    } else if enc == "quoted-printable" {
        return decode_quoted_printable(body);
    }
    body.to_string()
}

fn extract_header_param(header_val: &str, param: &str) -> Option<String> {
    let lower = header_val.to_ascii_lowercase();
    let needle = format!("{}=", param.to_ascii_lowercase());
    let pos = lower.find(&needle)?;
    let rest = header_val[pos + needle.len()..].trim_start();
    if let Some(stripped) = rest.strip_prefix('"') {
        let end = stripped.find('"').unwrap_or(stripped.len());
        Some(stripped[..end].to_string())
    } else {
        let end = rest.find(';').unwrap_or(rest.len());
        Some(rest[..end].trim().to_string())
    }
}

fn parse_mime_parts(
    content_type: &str,
    transfer_encoding: &str,
    content_disposition: &str,
    body: &str,
    text_out: &mut String,
    html_out: &mut String,
    attachments: &mut Vec<EmailAttachment>,
) {
    let ct_lower = content_type.to_ascii_lowercase();
    if ct_lower.starts_with("multipart/") {
        if let Some(boundary) = extract_header_param(content_type, "boundary") {
            let marker = format!("--{}", boundary);
            let parts: Vec<&str> = body.split(&marker).collect();
            for part in parts.into_iter().skip(1) {
                let part = part.trim_start_matches(['\r', '\n']);
                if part.starts_with("--") || part.is_empty() {
                    continue;
                }
                let part = part.trim_end_matches(['\r', '\n']);
                let (p_hdrs_raw, p_body) = split_headers_and_body(part);
                let p_hdrs = parse_headers(p_hdrs_raw);
                let p_ct = p_hdrs
                    .get("content-type")
                    .map(|s| s.as_str())
                    .unwrap_or("text/plain; charset=utf-8");
                let p_enc = p_hdrs
                    .get("content-transfer-encoding")
                    .map(|s| s.as_str())
                    .unwrap_or("7bit");
                let p_disp = p_hdrs
                    .get("content-disposition")
                    .map(|s| s.as_str())
                    .unwrap_or("");
                parse_mime_parts(p_ct, p_enc, p_disp, p_body, text_out, html_out, attachments);
            }
            return;
        }
    }

    let is_attachment = content_disposition.to_ascii_lowercase().contains("attachment")
        || extract_header_param(content_disposition, "filename").is_some();

    if is_attachment {
        let fname = extract_header_param(content_disposition, "filename")
            .or_else(|| extract_header_param(content_type, "name"))
            .unwrap_or_else(|| "attachment.bin".to_string());
        let mime_only = content_type
            .split(';')
            .next()
            .unwrap_or("application/octet-stream")
            .trim()
            .to_string();
        attachments.push(EmailAttachment {
            filename: fname,
            content_type: mime_only,
            size_bytes: body.len(),
        });
        return;
    }

    let decoded = decode_transfer_body(body, transfer_encoding);
    if ct_lower.contains("text/html") {
        if !html_out.is_empty() {
            html_out.push('\n');
        }
        html_out.push_str(&decoded);
    } else if ct_lower.contains("text/plain") || ct_lower.is_empty() {
        if !text_out.is_empty() {
            text_out.push('\n');
        }
        text_out.push_str(&decoded);
    }
}

fn split_address_list(raw: &str) -> Vec<String> {
    raw.split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

pub fn parse_raw_email(
    envelope_from: &str,
    envelope_to: &[String],
    raw_bytes: &[u8],
) -> CapturedEmail {
    let raw = String::from_utf8_lossy(raw_bytes).to_string();
    let (hdrs_raw, body_raw) = split_headers_and_body(&raw);
    let headers = parse_headers(hdrs_raw);

    let from = headers
        .get("from")
        .cloned()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| envelope_from.to_string());
    let to = headers
        .get("to")
        .map(|s| split_address_list(s))
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| envelope_to.to_vec());
    let cc = headers
        .get("cc")
        .map(|s| split_address_list(s))
        .unwrap_or_default();
    let subject = headers
        .get("subject")
        .cloned()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "(No Subject)".to_string());

    let content_type = headers
        .get("content-type")
        .map(|s| s.as_str())
        .unwrap_or("text/plain; charset=utf-8");
    let transfer_encoding = headers
        .get("content-transfer-encoding")
        .map(|s| s.as_str())
        .unwrap_or("7bit");
    let content_disposition = headers
        .get("content-disposition")
        .map(|s| s.as_str())
        .unwrap_or("");

    let mut text_body = String::new();
    let mut html_body = String::new();
    let mut attachments = Vec::new();
    parse_mime_parts(
        content_type,
        transfer_encoding,
        content_disposition,
        body_raw,
        &mut text_body,
        &mut html_body,
        &mut attachments,
    );

    if text_body.is_empty() && html_body.is_empty() && !body_raw.trim().is_empty() {
        text_body = body_raw.trim().to_string();
    }

    let site_domain = headers
        .get("x-dockpanel-site")
        .cloned()
        .filter(|s| !s.trim().is_empty());

    CapturedEmail {
        id: uuid::Uuid::new_v4().to_string(),
        from,
        to,
        cc,
        subject,
        text_body,
        html_body,
        raw,
        headers,
        attachments,
        size_bytes: raw_bytes.len(),
        read: false,
        site_domain,
        received_at: chrono::Utc::now().to_rfc3339(),
    }
}

fn infer_site_domain(email: &CapturedEmail, known_domains: &[String]) -> Option<String> {
    let hay = format!(
        "{} {} {} {}",
        email.from,
        email.to.join(" "),
        email.subject,
        email.text_body
    )
    .to_lowercase();
    for dom in known_domains {
        let lower = dom.to_lowercase();
        if !lower.is_empty() && hay.contains(&lower) {
            return Some(dom.clone());
        }
    }
    None
}

fn strip_html_tags(html: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let mut in_tag = false;
    for ch in html.chars() {
        match ch {
            '<' => in_tag = true,
            '>' => {
                in_tag = false;
                out.push(' ');
            }
            _ if !in_tag => out.push(ch),
            _ => {}
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_multipart_alternative_and_rfc2047() {
        let raw = concat!(
            "From: WordPress <wordpress@mysite.test>\r\n",
            "To: admin@mysite.test\r\n",
            "Subject: =?UTF-8?B?V2VsY29tZSB0byBEb2NrUGFuZWw=?=\r\n",
            "MIME-Version: 1.0\r\n",
            "Content-Type: multipart/alternative; boundary=\"b1\"\r\n",
            "\r\n",
            "--b1\r\n",
            "Content-Type: text/plain; charset=utf-8\r\n",
            "Content-Transfer-Encoding: quoted-printable\r\n",
            "\r\n",
            "Hello=20World!\r\n",
            "--b1\r\n",
            "Content-Type: text/html; charset=utf-8\r\n",
            "\r\n",
            "<p>Hello <strong>HTML</strong>!</p>\r\n",
            "--b1--\r\n"
        );

        let parsed = parse_raw_email(
            "wordpress@mysite.test",
            &["admin@mysite.test".to_string()],
            raw.as_bytes(),
        );

        assert_eq!(parsed.subject, "Welcome to DockPanel");
        assert!(parsed.text_body.contains("Hello World!"));
        assert!(parsed.html_body.contains("<strong>HTML</strong>"));
        assert_eq!(
            infer_site_domain(&parsed, &["mysite.test".to_string()]),
            Some("mysite.test".to_string())
        );
    }

    #[tokio::test]
    async fn test_smtp_server_captures_and_persists_email() {
        let tmp_dir =
            std::env::temp_dir().join(format!("dockpanel_mail_test_{}", uuid::Uuid::new_v4()));
        let store = MailCatcherStore::new(&tmp_dir);

        let port = start_smtp_server(
            store.clone(),
            0,
            || vec!["blog.test".to_string()],
            |_| {},
        )
        .await
        .expect("SMTP server should bind");

        send_test_email_smtp(
            port,
            "wordpress@blog.test",
            "dev@example.com",
            "Password Reset",
            "Reset link: https://blog.test/wp-login.php",
            Some("<p>Reset link</p>"),
        )
        .await
        .expect("SMTP test email send should succeed");

        let list = store.list_summaries(None, Some("blog.test"));
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].subject, "Password Reset");
        assert_eq!(list[0].site_domain.as_deref(), Some("blog.test"));
        assert!(!list[0].read);

        let id = list[0].id.clone();
        assert!(store.mark_read(&id, true));
        assert!(store.get_message(&id).unwrap().read);
        assert!(store.delete_message(&id));
        assert_eq!(store.count(), 0);

        let _ = fs::remove_dir_all(&tmp_dir);
    }
}
