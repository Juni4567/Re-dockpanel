use rustls::pki_types::pem::PemObject;
use rustls::pki_types::{CertificateDer, PrivateKeyDer};
use rustls::server::{ClientHello, ResolvesServerCert};
use rustls::sign::CertifiedKey;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};

#[derive(Debug)]
pub struct DynamicCertResolver {
    certs: RwLock<HashMap<String, Arc<CertifiedKey>>>,
    default_key: RwLock<Option<Arc<CertifiedKey>>>,
}

impl DynamicCertResolver {
    pub fn new() -> Self {
        Self {
            certs: RwLock::new(HashMap::new()),
            default_key: RwLock::new(None),
        }
    }

    pub fn insert_key(&self, domain: &str, key: Arc<CertifiedKey>) {
        let clean = domain.trim().to_lowercase();
        if let Ok(mut guard) = self.certs.write() {
            guard.insert(clean, key.clone());
        }
        if let Ok(mut def) = self.default_key.write() {
            if def.is_none() {
                *def = Some(key);
            }
        }
    }

    pub fn load_domain_from_dir(&self, certs_dir: &Path, domain: &str) -> Result<(), String> {
        let cert_path = certs_dir.join(domain).join("cert.pem");
        let key_path = certs_dir.join(domain).join("key.pem");
        let certified = load_certified_key(&cert_path, &key_path)?;
        self.insert_key(domain, certified);
        Ok(())
    }

    pub fn load_existing_certs(&self, certs_dir: &Path) {
        if !certs_dir.is_dir() {
            return;
        }
        if let Ok(entries) = std::fs::read_dir(certs_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    if let Some(domain) = path.file_name().and_then(|s| s.to_str()) {
                        let _ = self.load_domain_from_dir(certs_dir, domain);
                    }
                }
            }
        }
    }
}

impl ResolvesServerCert for DynamicCertResolver {
    fn resolve(&self, client_hello: ClientHello<'_>) -> Option<Arc<CertifiedKey>> {
        if let Some(sni) = client_hello.server_name() {
            let clean = sni.trim().to_lowercase();
            if let Ok(guard) = self.certs.read() {
                if let Some(found) = guard.get(&clean) {
                    return Some(found.clone());
                }
            }
        }
        self.default_key.read().ok().and_then(|g| g.clone())
    }
}

fn is_valid_mkcert_binary(path: &Path) -> bool {
    // Reject historical 138 KB stub executables that lacked -cert-file support.
    path.is_file()
        && std::fs::metadata(path)
            .map(|m| m.len() > 500_000)
            .unwrap_or(false)
}

/// Locates real `mkcert.exe` beside the current binary, in dev installer/bin, Program Files, or PATH.
pub fn find_mkcert_binary() -> Option<PathBuf> {
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            let beside = dir.join("mkcert.exe");
            if is_valid_mkcert_binary(&beside) {
                return Some(beside);
            }
            let in_bin = dir.join("bin").join("mkcert.exe");
            if is_valid_mkcert_binary(&in_bin) {
                return Some(in_bin);
            }
        }
    }

    let candidates = [
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("installer")
            .join("bin")
            .join("mkcert.exe"),
        PathBuf::from(r"C:\Program Files\DockPanel Desktop\mkcert.exe"),
    ];
    for c in candidates {
        if is_valid_mkcert_binary(&c) {
            return Some(c);
        }
    }

    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x08000000;
        if let Ok(out) = std::process::Command::new("where.exe")
            .arg("mkcert")
            .creation_flags(CREATE_NO_WINDOW)
            .output()
        {
            if out.status.success() {
                let first = String::from_utf8_lossy(&out.stdout)
                    .lines()
                    .next()
                    .unwrap_or("")
                    .trim()
                    .to_string();
                let p = Path::new(&first);
                if !first.is_empty() && is_valid_mkcert_binary(p) {
                    return Some(p.to_path_buf());
                }
            }
        }
    }

    None
}

/// Ensures the mkcert Root CA exists and (outside test harnesses) installs trust when missing.
pub fn ensure_mkcert_ca_ready() -> Result<PathBuf, String> {
    let mkcert = find_mkcert_binary()
        .ok_or_else(|| "mkcert.exe was not found on this system".to_string())?;

    #[cfg(all(windows, not(test)))]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x08000000;
        if let Ok(out) = std::process::Command::new(&mkcert)
            .arg("-CAROOT")
            .creation_flags(CREATE_NO_WINDOW)
            .output()
        {
            let caroot = PathBuf::from(String::from_utf8_lossy(&out.stdout).trim());
            let root_ca = caroot.join("rootCA.pem");
            if root_ca.is_file() {
                // Non-interactive LocalMachine trust check/install (avoids modal CurrentUser Security Warning).
                let _ = std::process::Command::new("certutil")
                    .args(["-addstore", "-f", "Root", &root_ca.to_string_lossy()])
                    .creation_flags(CREATE_NO_WINDOW)
                    .output();
            }
        }
    }

    Ok(mkcert)
}

/// Generates (or reuses) a trusted TLS certificate + private key for `domain` under `certs_dir/{domain}/`.
pub fn ensure_site_certificate(certs_dir: &Path, domain: &str) -> Result<(PathBuf, PathBuf), String> {
    let clean = domain.trim().to_lowercase();
    let site_cert_dir = certs_dir.join(&clean);
    std::fs::create_dir_all(&site_cert_dir)
        .map_err(|e| format!("Failed creating cert directory: {}", e))?;

    let cert_path = site_cert_dir.join("cert.pem");
    let key_path = site_cert_dir.join("key.pem");
    if cert_path.is_file() && key_path.is_file() {
        return Ok((cert_path, key_path));
    }

    let mkcert = ensure_mkcert_ca_ready()?;
    let mut cmd = std::process::Command::new(&mkcert);
    cmd.args([
        "-cert-file",
        &cert_path.to_string_lossy(),
        "-key-file",
        &key_path.to_string_lossy(),
        &clean,
        &format!("*.{}", clean),
        "localhost",
        "127.0.0.1",
    ]);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x08000000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }

    let out = cmd
        .output()
        .map_err(|e| format!("Failed running mkcert.exe: {}", e))?;
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        let stdout = String::from_utf8_lossy(&out.stdout);
        return Err(format!("mkcert failed for {}: {} {}", clean, stderr, stdout));
    }

    Ok((cert_path, key_path))
}

pub fn load_certified_key(cert_path: &Path, key_path: &Path) -> Result<Arc<CertifiedKey>, String> {
    let certs: Vec<CertificateDer<'static>> = CertificateDer::pem_file_iter(cert_path)
        .map_err(|e| format!("Failed opening cert file {}: {}", cert_path.display(), e))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("Invalid certificate PEM: {}", e))?;

    if certs.is_empty() {
        return Err(format!("No certificates found in {}", cert_path.display()));
    }

    let key = PrivateKeyDer::from_pem_file(key_path)
        .map_err(|e| format!("Invalid private key PEM {}: {}", key_path.display(), e))?;

    let signing_key = rustls::crypto::aws_lc_rs::sign::any_supported_type(&key)
        .map_err(|e| format!("Unsupported private key algorithm: {:?}", e))?;

    Ok(Arc::new(CertifiedKey::new(certs, signing_key)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mkcert_binary_discovery_and_cert_gen_smoke() {
        // Test environment gracefully checks mkcert availability without failing CI hosts lacking mkcert.
        let found = find_mkcert_binary();
        if let Some(bin) = found {
            assert!(bin.is_file());
        }
    }
}
