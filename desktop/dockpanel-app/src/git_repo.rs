use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GitCommitInfo {
    pub hash: String,
    pub author: String,
    pub date: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GitRepoStatus {
    pub git_available: bool,
    pub git_version: Option<String>,
    pub initialized: bool,
    pub doc_root: String,
    pub branch: Option<String>,
    pub branches: Vec<String>,
    pub remote_name: Option<String>,
    pub remote_url: Option<String>,
    pub clean: bool,
    pub changed_files: usize,
    pub recent_commits: Vec<GitCommitInfo>,
    pub has_gitignore: bool,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct GitInitRequest {
    pub branch: Option<String>,
    pub gitignore_preset: Option<String>,
    pub initial_commit: Option<bool>,
    pub commit_message: Option<String>,
    pub user_name: Option<String>,
    pub user_email: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct GitConnectRequest {
    pub repo_url: String,
    pub branch: Option<String>,
    pub token: Option<String>,
    pub mode: Option<String>, // "link" | "clone" | "link_and_pull"
    pub gitignore_preset: Option<String>,
    pub user_name: Option<String>,
    pub user_email: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct GitPullRequest {
    pub branch: Option<String>,
    pub token: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct GitCommitPushRequest {
    pub message: String,
    pub branch: Option<String>,
    pub token: Option<String>,
    pub push: Option<bool>,
}

pub fn find_git_binary() -> Option<PathBuf> {
    let candidates = [
        "git",
        r"C:\Program Files\Git\cmd\git.exe",
        r"C:\Program Files\Git\bin\git.exe",
        r"C:\Program Files (x86)\Git\cmd\git.exe",
    ];
    for c in candidates {
        let mut cmd = Command::new(c);
        cmd.arg("--version");
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x08000000;
            cmd.creation_flags(CREATE_NO_WINDOW);
        }
        if let Ok(out) = cmd.output() {
            if out.status.success() {
                return Some(PathBuf::from(c));
            }
        }
    }
    None
}

fn run_git(dir: &Path, args: &[&str]) -> Result<(bool, String), String> {
    let git_exe = find_git_binary().ok_or_else(|| {
        "Git is not installed or not found on PATH. Install Git for Windows to manage repositories.".to_string()
    })?;
    let mut cmd = Command::new(git_exe);
    cmd.args(args).current_dir(dir);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x08000000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    let out = cmd
        .output()
        .map_err(|e| format!("Failed executing git {}: {}", args.first().copied().unwrap_or(""), e))?;
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    let combined = match (stdout.trim().is_empty(), stderr.trim().is_empty()) {
        (true, true) => String::new(),
        (false, true) => stdout.trim().to_string(),
        (true, false) => stderr.trim().to_string(),
        (false, false) => format!("{}\n{}", stdout.trim(), stderr.trim()),
    };
    Ok((out.status.success(), combined))
}

pub fn validate_branch_name(branch: &str) -> Result<String, String> {
    let clean = branch.trim();
    if clean.is_empty()
        || clean.len() > 100
        || clean.starts_with('-')
        || clean.contains("..")
        || clean.chars().any(|c| c.is_whitespace() || c == '\0' || c == '\\')
    {
        return Err("Invalid branch name".to_string());
    }
    Ok(clean.to_string())
}

pub fn validate_repo_url(url: &str) -> Result<String, String> {
    let clean = url.trim();
    if clean.is_empty() || clean.starts_with('-') || clean.chars().any(|c| c.is_whitespace() || c == '\0') {
        return Err("Invalid repository URL".to_string());
    }
    if !clean.starts_with("https://")
        && !clean.starts_with("http://")
        && !clean.starts_with("git@")
        && !clean.starts_with("ssh://")
    {
        return Err("Repository URL must start with https://, http://, ssh://, or git@".to_string());
    }
    Ok(clean.to_string())
}

pub fn mask_repo_url(url: &str) -> String {
    if let Some(scheme_end) = url.find("://") {
        let after_scheme = &url[scheme_end + 3..];
        let auth_end = after_scheme.find('/').unwrap_or(after_scheme.len());
        if let Some(at_pos) = after_scheme[..auth_end].rfind('@') {
            return format!(
                "{}***@{}",
                &url[..scheme_end + 3],
                &after_scheme[at_pos + 1..]
            );
        }
    }
    url.to_string()
}

fn inject_https_token(url: &str, token: Option<&str>) -> String {
    let Some(tok) = token.map(str::trim).filter(|t| !t.is_empty()) else {
        return url.to_string();
    };
    if let Some(rest) = url.strip_prefix("https://") {
        let stripped = rest
            .split_once('@')
            .and_then(|(_, right)| if !rest[..rest.find('@').unwrap()].contains('/') { Some(right) } else { None })
            .unwrap_or(rest);
        format!("https://x-access-token:{}@{}", tok, stripped)
    } else if let Some(rest) = url.strip_prefix("http://") {
        format!("http://x-access-token:{}@{}", tok, rest)
    } else {
        url.to_string()
    }
}

fn scrub_token_text(text: &str, token: Option<&str>) -> String {
    let Some(tok) = token.map(str::trim).filter(|t| !t.is_empty()) else {
        return text.to_string();
    };
    text.replace(tok, "***")
}

fn gitignore_template(preset: &str) -> Option<&'static str> {
    match preset.trim().to_ascii_lowercase().as_str() {
        "wordpress" => Some(
            "# WordPress\nwp-content/cache/\nwp-content/upgrade/\nwp-content/uploads/\nwp-content/ai1wm-backups/\nwp-content/updraft/\nwp-config.php\n*.sql\n*.sql.gz\n*.log\n.DS_Store\nThumbs.db\n",
        ),
        "node" => Some(
            "# Node.js\nnode_modules/\n.next/\ndist/\nbuild/\n.env\n.env.local\nnpm-debug.log*\n*.log\n.DS_Store\n",
        ),
        "php" | "laravel" => Some(
            "# PHP / Laravel\nvendor/\nnode_modules/\n.env\nstorage/logs/*.log\n*.sql\n*.sqlite\n*.log\n.DS_Store\n",
        ),
        "static" => Some("# Static site\nnode_modules/\n.env\n*.log\n.DS_Store\nThumbs.db\n"),
        _ => None,
    }
}

fn maybe_write_gitignore(site_dir: &Path, preset: Option<&str>) {
    let Some(p) = preset else {
        return;
    };
    let Some(body) = gitignore_template(p) else {
        return;
    };
    let gi_path = site_dir.join(".gitignore");
    if !gi_path.exists() {
        let _ = std::fs::write(gi_path, body);
    }
}

fn ensure_local_git_identity(site_dir: &Path, name: Option<&str>, email: Option<&str>) {
    let has_name = run_git(site_dir, &["config", "--get", "user.name"])
        .map(|(ok, s)| ok && !s.is_empty())
        .unwrap_or(false);
    if !has_name {
        let n = name.map(str::trim).filter(|s| !s.is_empty()).unwrap_or("DockPanel Admin");
        let _ = run_git(site_dir, &["config", "user.name", n]);
    }
    let has_email = run_git(site_dir, &["config", "--get", "user.email"])
        .map(|(ok, s)| ok && !s.is_empty())
        .unwrap_or(false);
    if !has_email {
        let e = email.map(str::trim).filter(|s| !s.is_empty()).unwrap_or("admin@dockpanel.local");
        let _ = run_git(site_dir, &["config", "user.email", e]);
    }
}

pub fn get_status(site_dir: &Path) -> GitRepoStatus {
    let doc_root = site_dir.to_string_lossy().to_string();
    let Some(git_exe) = find_git_binary() else {
        return GitRepoStatus {
            git_available: false,
            git_version: None,
            initialized: site_dir.join(".git").exists(),
            doc_root,
            branch: None,
            branches: Vec::new(),
            remote_name: None,
            remote_url: None,
            clean: true,
            changed_files: 0,
            recent_commits: Vec::new(),
            has_gitignore: site_dir.join(".gitignore").is_file(),
            error: Some("Git is not installed on this machine.".to_string()),
        };
    };

    let mut ver_cmd = Command::new(git_exe);
    ver_cmd.arg("--version");
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x08000000;
        ver_cmd.creation_flags(CREATE_NO_WINDOW);
    }
    let git_version = ver_cmd
        .output()
        .ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .filter(|s| !s.is_empty());

    let initialized = site_dir.join(".git").exists();
    if !initialized || !site_dir.is_dir() {
        return GitRepoStatus {
            git_available: true,
            git_version,
            initialized: false,
            doc_root,
            branch: None,
            branches: Vec::new(),
            remote_name: None,
            remote_url: None,
            clean: true,
            changed_files: 0,
            recent_commits: Vec::new(),
            has_gitignore: site_dir.join(".gitignore").is_file(),
            error: None,
        };
    }

    let branch = run_git(site_dir, &["symbolic-ref", "--short", "HEAD"])
        .ok()
        .filter(|(ok, s)| *ok && !s.is_empty())
        .map(|(_, s)| s);

    let branches = run_git(site_dir, &["for-each-ref", "--format=%(refname:short)", "refs/heads"])
        .ok()
        .filter(|(ok, _)| *ok)
        .map(|(_, out)| out.lines().map(|l| l.trim().to_string()).filter(|s| !s.is_empty()).collect())
        .unwrap_or_default();

    let remote_url = run_git(site_dir, &["remote", "get-url", "origin"])
        .ok()
        .filter(|(ok, s)| *ok && !s.is_empty())
        .map(|(_, u)| mask_repo_url(&u));

    let changed_files = run_git(site_dir, &["status", "--porcelain"])
        .ok()
        .filter(|(ok, _)| *ok)
        .map(|(_, out)| out.lines().filter(|l| !l.trim().is_empty()).count())
        .unwrap_or(0);

    let recent_commits = run_git(
        site_dir,
        &["log", "-n", "5", "--pretty=format:%h%x09%an%x09%cr%x09%s"],
    )
    .ok()
    .filter(|(ok, _)| *ok)
    .map(|(_, out)| {
        out.lines()
            .filter_map(|line| {
                let mut parts = line.splitn(4, '\t');
                Some(GitCommitInfo {
                    hash: parts.next()?.to_string(),
                    author: parts.next()?.to_string(),
                    date: parts.next()?.to_string(),
                    message: parts.next().unwrap_or("").to_string(),
                })
            })
            .collect()
    })
    .unwrap_or_default();

    GitRepoStatus {
        git_available: true,
        git_version,
        initialized: true,
        doc_root,
        branch,
        branches,
        remote_name: remote_url.as_ref().map(|_| "origin".to_string()),
        remote_url,
        clean: changed_files == 0,
        changed_files,
        recent_commits,
        has_gitignore: site_dir.join(".gitignore").is_file(),
        error: None,
    }
}

pub fn init_repo(site_dir: &Path, req: &GitInitRequest) -> Result<(GitRepoStatus, String), String> {
    if !site_dir.is_dir() {
        return Err("Site directory does not exist".to_string());
    }
    let branch = validate_branch_name(req.branch.as_deref().unwrap_or("main"))?;
    let mut log_lines = Vec::new();

    if !site_dir.join(".git").exists() {
        let (ok, out) = run_git(site_dir, &["init", "-b", &branch])?;
        if !ok {
            let (ok2, out2) = run_git(site_dir, &["init"])?;
            if !ok2 {
                return Err(if out2.is_empty() { out } else { out2 });
            }
            let _ = run_git(site_dir, &["symbolic-ref", "HEAD", &format!("refs/heads/{}", branch)]);
        }
        log_lines.push(format!("Initialized empty Git repository on branch '{}'", branch));
    } else {
        log_lines.push("Git repository already initialized".to_string());
    }

    maybe_write_gitignore(site_dir, req.gitignore_preset.as_deref());
    ensure_local_git_identity(
        site_dir,
        req.user_name.as_deref(),
        req.user_email.as_deref(),
    );

    if req.initial_commit.unwrap_or(false) {
        let _ = run_git(site_dir, &["add", "-A"])?;
        let msg = req
            .commit_message
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .unwrap_or("Initial commit");
        let (ok_commit, commit_out) = run_git(site_dir, &["commit", "-m", msg])?;
        if ok_commit {
            log_lines.push(format!("Created initial commit: {}", commit_out));
        } else if commit_out.contains("nothing to commit") {
            log_lines.push("Working tree empty/clean — skipped empty initial commit".to_string());
        } else {
            return Err(format!("Initial commit failed: {}", commit_out));
        }
    }

    Ok((get_status(site_dir), log_lines.join("\n")))
}

pub fn connect_repo(site_dir: &Path, req: &GitConnectRequest) -> Result<(GitRepoStatus, String), String> {
    if !site_dir.is_dir() {
        return Err("Site directory does not exist".to_string());
    }
    let clean_url = validate_repo_url(&req.repo_url)?;
    let branch = validate_branch_name(req.branch.as_deref().unwrap_or("main"))?;
    let mode = req.mode.as_deref().unwrap_or("link").trim().to_ascii_lowercase();
    let token = req.token.as_deref();
    let authed_url = inject_https_token(&clean_url, token);
    let mut logs = Vec::new();

    if mode == "clone" && !site_dir.join(".git").exists() {
        let entries: Vec<_> = std::fs::read_dir(site_dir)
            .map(|r| r.flatten().collect())
            .unwrap_or_default();
        let non_placeholder: Vec<_> = entries
            .iter()
            .filter(|e| {
                let name = e.file_name().to_string_lossy().to_ascii_lowercase();
                name != "index.html" && name != ".htaccess"
            })
            .collect();
        if !non_placeholder.is_empty() {
            return Err(
                "Folder already contains website files — use 'Link Existing Site & Fetch' mode instead of overwriting via Clone."
                    .to_string(),
            );
        }
        let staging = std::env::temp_dir().join(format!("dockpanel_clone_{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&staging).map_err(|e| e.to_string())?;
        let (ok, clone_out) = run_git(
            &staging,
            &["clone", "--branch", &branch, &authed_url, "."],
        )?;
        let clone_out = scrub_token_text(&clone_out, token);
        if !ok {
            let _ = std::fs::remove_dir_all(&staging);
            return Err(format!("Git clone failed: {}", clone_out));
        }
        for entry in entries {
            let p = entry.path();
            if p.is_file() {
                let _ = std::fs::remove_file(p);
            }
        }
        for item in std::fs::read_dir(&staging).map_err(|e| e.to_string())?.flatten() {
            let dst = site_dir.join(item.file_name());
            if let Err(e) = std::fs::rename(item.path(), &dst) {
                let _ = std::fs::remove_dir_all(&staging);
                return Err(format!("Failed moving cloned repository into site folder: {}", e));
            }
        }
        let _ = std::fs::remove_dir_all(&staging);
        // Strip inline PAT from persisted origin URL after clone if desired? Keep clean URL if token wasn't stored, or keep clean URL in config unless HTTPS push needed
        let _ = run_git(site_dir, &["remote", "set-url", "origin", &clean_url]);
        logs.push(format!("Cloned '{}' ({}) into site directory.", mask_repo_url(&clean_url), branch));
        return Ok((get_status(site_dir), logs.join("\n")));
    }

    if !site_dir.join(".git").exists() {
        let init_res = init_repo(
            site_dir,
            &GitInitRequest {
                branch: Some(branch.clone()),
                gitignore_preset: req.gitignore_preset.clone(),
                initial_commit: Some(false),
                commit_message: None,
                user_name: req.user_name.clone(),
                user_email: req.user_email.clone(),
            },
        )?;
        logs.push(init_res.1);
    }

    let has_origin = run_git(site_dir, &["remote", "get-url", "origin"])
        .map(|(ok, _)| ok)
        .unwrap_or(false);
    if has_origin {
        let (ok, out) = run_git(site_dir, &["remote", "set-url", "origin", &clean_url])?;
        if !ok {
            return Err(format!("Failed updating remote origin: {}", scrub_token_text(&out, token)));
        }
        logs.push(format!("Updated remote 'origin' -> {}", mask_repo_url(&clean_url)));
    } else {
        let (ok, out) = run_git(site_dir, &["remote", "add", "origin", &clean_url])?;
        if !ok {
            return Err(format!("Failed adding remote origin: {}", scrub_token_text(&out, token)));
        }
        logs.push(format!("Connected remote 'origin' -> {}", mask_repo_url(&clean_url)));
    }

    let (ok_fetch, fetch_out) = run_git(site_dir, &["fetch", &authed_url, &branch])?;
    let fetch_clean = scrub_token_text(&fetch_out, token);
    if ok_fetch {
        logs.push(format!("Fetched branch '{}' from remote.", branch));
    } else {
        logs.push(format!("Remote linked (fetch warning: {}).", fetch_clean));
    }

    if mode == "link_and_pull" && ok_fetch {
        ensure_local_git_identity(
            site_dir,
            req.user_name.as_deref(),
            req.user_email.as_deref(),
        );
        let (ok_pull, pull_out) = run_git(
            site_dir,
            &["pull", &authed_url, &branch, "--allow-unrelated-histories", "--no-edit"],
        )?;
        let pull_clean = scrub_token_text(&pull_out, token);
        if !ok_pull {
            return Err(format!("Connected remote, but pull failed: {}", pull_clean));
        }
        logs.push(format!("Merged remote branch '{}': {}", branch, pull_clean));
    }

    Ok((get_status(site_dir), logs.join("\n")))
}

pub fn pull_repo(site_dir: &Path, req: &GitPullRequest) -> Result<(GitRepoStatus, String), String> {
    if !site_dir.join(".git").exists() {
        return Err("Site directory does not have an initialized Git repository".to_string());
    }
    let branch = validate_branch_name(req.branch.as_deref().unwrap_or("main"))?;
    let remote = run_git(site_dir, &["remote", "get-url", "origin"])
        .ok()
        .filter(|(ok, s)| *ok && !s.is_empty())
        .map(|(_, s)| s)
        .ok_or_else(|| "No 'origin' remote configured for this repository".to_string())?;
    let authed = inject_https_token(&remote, req.token.as_deref());
    let (ok, out) = run_git(site_dir, &["pull", &authed, &branch, "--no-edit"])?;
    let clean_out = scrub_token_text(&out, req.token.as_deref());
    if !ok {
        return Err(format!("Git pull failed: {}", clean_out));
    }
    Ok((get_status(site_dir), clean_out))
}

pub fn commit_and_push(site_dir: &Path, req: &GitCommitPushRequest) -> Result<(GitRepoStatus, String), String> {
    if !site_dir.join(".git").exists() {
        return Err("Site directory does not have an initialized Git repository".to_string());
    }
    let msg = req.message.trim();
    if msg.is_empty() {
        return Err("Commit message is required".to_string());
    }
    ensure_local_git_identity(site_dir, None, None);
    let _ = run_git(site_dir, &["add", "-A"])?;
    let mut logs = Vec::new();
    let (ok_c, out_c) = run_git(site_dir, &["commit", "-m", msg])?;
    if ok_c {
        logs.push(out_c);
    } else if out_c.contains("nothing to commit") {
        logs.push("Working tree clean — nothing new to commit".to_string());
    } else {
        return Err(format!("Git commit failed: {}", out_c));
    }

    if req.push.unwrap_or(false) {
        let cur_branch = req
            .branch
            .as_deref()
            .map(validate_branch_name)
            .transpose()?
            .or_else(|| get_status(site_dir).branch)
            .unwrap_or_else(|| "main".to_string());
        let remote = run_git(site_dir, &["remote", "get-url", "origin"])
            .ok()
            .filter(|(ok, s)| *ok && !s.is_empty())
            .map(|(_, s)| s)
            .ok_or_else(|| "Committed locally, but no 'origin' remote is connected for push".to_string())?;
        let authed = inject_https_token(&remote, req.token.as_deref());
        let (ok_p, out_p) = run_git(site_dir, &["push", "-u", &authed, &cur_branch])?;
        let clean_p = scrub_token_text(&out_p, req.token.as_deref());
        if !ok_p {
            return Err(format!("Git push failed: {}", clean_p));
        }
        logs.push(clean_p);
    }

    Ok((get_status(site_dir), logs.join("\n")))
}

pub fn disconnect_remote(site_dir: &Path) -> Result<GitRepoStatus, String> {
    if site_dir.join(".git").exists() {
        let _ = run_git(site_dir, &["remote", "remove", "origin"]);
    }
    Ok(get_status(site_dir))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_git_repo_init_connect_and_status_lifecycle() {
        if find_git_binary().is_none() {
            return;
        }
        let tmp = std::env::temp_dir().join(format!("dockpanel_git_{}", uuid::Uuid::new_v4()));
        let site_dir = tmp.join("mysite");
        let bare_remote = tmp.join("remote.git");
        std::fs::create_dir_all(&site_dir).unwrap();
        std::fs::create_dir_all(&bare_remote).unwrap();
        std::fs::write(site_dir.join("index.php"), "<?php echo 'hello';").unwrap();

        let (ok_bare, _) = run_git(&bare_remote, &["init", "--bare"]).unwrap();
        assert!(ok_bare);

        let st0 = get_status(&site_dir);
        assert!(st0.git_available);
        assert!(!st0.initialized);

        let (st1, init_log) = init_repo(
            &site_dir,
            &GitInitRequest {
                branch: Some("main".to_string()),
                gitignore_preset: Some("wordpress".to_string()),
                initial_commit: Some(true),
                commit_message: Some("Initial WordPress commit".to_string()),
                user_name: Some("Test Admin".to_string()),
                user_email: Some("admin@test.local".to_string()),
            },
        )
        .unwrap();
        assert!(init_log.contains("Initialized"));
        assert!(st1.initialized);
        assert_eq!(st1.branch.as_deref(), Some("main"));
        assert!(st1.has_gitignore);
        assert_eq!(st1.recent_commits.len(), 1);
        assert_eq!(st1.recent_commits[0].message, "Initial WordPress commit");

        // Connect origin using local file:// URI? Note validate_repo_url restricts to http/https/ssh/git@,
        // so verify URL validation and mask_repo_url directly:
        assert_eq!(
            mask_repo_url("https://ghp_secret123@github.com/org/repo.git"),
            "https://***@github.com/org/repo.git"
        );
        assert!(validate_repo_url("https://github.com/org/repo.git").is_ok());
        assert!(validate_repo_url("--upload-pack=evil").is_err());

        let (ok_add, _) = run_git(
            &site_dir,
            &["remote", "add", "origin", &bare_remote.to_string_lossy()],
        )
        .unwrap();
        assert!(ok_add);
        let st2 = get_status(&site_dir);
        assert!(st2.remote_url.is_some());

        let st3 = disconnect_remote(&site_dir).unwrap();
        assert!(st3.remote_url.is_none());

        let _ = std::fs::remove_dir_all(&tmp);
    }
}
