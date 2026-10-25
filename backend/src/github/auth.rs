use super::*;

/// A writable gh config directory (cached for the process), or None to use gh's
/// default. Docker mounts the host's `~/.config/gh` read-only, but a newer gh
/// migrates the on-disk config format on startup and aborts when it can't write
/// (`failed to write config after migration: ... read-only file system`). We
/// copy the config into a writable dir once and point every `gh` call at it via
/// `GH_CONFIG_DIR`, leaving the user's real config untouched.
pub(crate) fn gh_config_dir() -> Option<PathBuf> {
    GH_CONFIG_DIR.get_or_init(prepare_gh_config_dir).clone()
}

pub(crate) fn prepare_gh_config_dir() -> Option<PathBuf> {
    // Respect an explicit override.
    if let Some(dir) = std::env::var_os("GH_CONFIG_DIR") {
        return Some(PathBuf::from(dir));
    }
    let source = std::env::var_os("XDG_CONFIG_HOME")
        .map(|base| PathBuf::from(base).join("gh"))
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config").join("gh")))?;
    if !source.join("hosts.yml").is_file() {
        return None;
    }
    // If the real gh config dir is writable, use it directly (return None) so that
    // in-app sign-ins, token refreshes, and scope grants persist to the mounted
    // volume. Only fall back to a copy when it is read-only — the case a newer gh
    // aborts on because it can't rewrite the config during its startup migration.
    if dir_is_writable(&source) {
        return None;
    }
    // Copy into a private, owner-only directory. The copied `hosts.yml` holds the
    // gh credential, so it must not live at a fixed, world-predictable path on a
    // shared /tmp where another local user could read or substitute it.
    let target = gh_config_cache_dir();
    if create_private_dir(&target).is_err() {
        return None;
    }
    let mut copied = false;
    for name in ["hosts.yml", "config.yml"] {
        let src = source.join(name);
        if src.is_file() && std::fs::copy(&src, target.join(name)).is_ok() {
            copied = true;
        }
    }
    copied.then_some(target)
}

/// A private, writable location for the copied gh config. Prefers the app's
/// own data/cache dir (in Docker this is the private `/data` volume via
/// `XDG_DATA_HOME`); otherwise a per-process temp dir so the path is not a
/// fixed, world-predictable location on a shared `/tmp`.
pub(crate) fn gh_config_cache_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("XDG_DATA_HOME").or_else(|| std::env::var_os("XDG_CACHE_HOME")) {
        return PathBuf::from(dir).join("opencodereview").join("gh-config");
    }
    std::env::temp_dir().join(format!("ocr-gh-config-{}", std::process::id()))
}

/// Create `path` (recursively) restricted to the owner. On Unix the directory is
/// created 0700 and re-tightened afterwards; if it already exists but is not
/// owner-writable (e.g. pre-created by another user), `set_permissions` fails and
/// the error propagates so the caller falls back to gh's default config rather
/// than trusting an attacker-controlled directory.
pub(crate) fn create_private_dir(path: &Path) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
        std::fs::DirBuilder::new().recursive(true).mode(0o700).create(path)?;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))?;
    }
    #[cfg(not(unix))]
    {
        std::fs::create_dir_all(path)?;
    }
    Ok(())
}

/// Best-effort check that files can be created in `dir` (i.e. gh can persist its
/// config there). Writes and removes a uniquely-named probe file.
pub(crate) fn dir_is_writable(dir: &Path) -> bool {
    let probe = dir.join(format!(".ocr-write-probe-{}", std::process::id()));
    match std::fs::File::create(&probe) {
        Ok(_) => {
            let _ = std::fs::remove_file(&probe);
            true
        }
        Err(_) => false,
    }
}

/// Point a `gh` invocation at the writable config dir (no-op if none is needed).
pub(crate) fn apply_gh_config_dir(command: &mut Command) {
    if let Some(dir) = gh_config_dir() {
        command.env("GH_CONFIG_DIR", dir);
    }
}

/// Resolved GitHub host for every `gh`/git operation (cached for the process).
/// Precedence: `OCR_GH_HOST`, `GH_HOST`, the first host reported by
/// `gh auth status`, then `github.com`. This lets OpenCodeReview target GitHub
/// Enterprise (e.g. `ghe.example.com`) instead of assuming public github.com.
pub fn gh_host() -> String {
    GH_HOST.get_or_init(detect_gh_host).clone()
}

pub(crate) fn detect_gh_host() -> String {
    for key in ["OCR_GH_HOST", "GH_HOST"] {
        if let Ok(value) = std::env::var(key) {
            let value = value.trim();
            // Reject anything that isn't a bare `host[:port]`. The value is
            // interpolated into the `http.https://<host>/.extraheader` auth key
            // and clone URLs, so a scheme, path, credentials, or whitespace/newline
            // could send the gh token to an unintended host. Skip invalid input
            // and fall back to gh's own resolved host.
            if is_valid_gh_host(value) {
                return value.to_string();
            }
        }
    }
    // Ask gh which host it's logged into (do NOT route through `run_raw`, which
    // would call back into `gh_host` and recurse).
    let mut status = Command::new("gh");
    status.args(["auth", "status"]);
    apply_gh_config_dir(&mut status);
    if let Ok(output) = status.output() {
        let text = format!(
            "{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        if let Some(host) = parse_gh_host(&text) {
            return host;
        }
    }
    "github.com".to_string()
}

pub(crate) fn parse_gh_host(text: &str) -> Option<String> {
    text.lines().find_map(|line| {
        let idx = line.find("Logged in to ")?;
        let rest = &line[idx + "Logged in to ".len()..];
        let host = rest.split_whitespace().next()?.trim();
        (!host.is_empty()).then(|| host.to_string())
    })
}

/// A bare `host` or `host:port` suitable for interpolation into a git config key
/// and clone URL. Rejects schemes, paths, credentials, and any whitespace so an
/// operator-supplied (or misconfigured) `OCR_GH_HOST`/`GH_HOST` cannot redirect
/// the authenticated request to an unintended host.
pub(crate) fn is_valid_gh_host(host: &str) -> bool {
    if host.is_empty() || host.len() > 253 {
        return false;
    }
    let (name, port) = match host.rsplit_once(':') {
        Some((name, port)) => (name, Some(port)),
        None => (host, None),
    };
    if let Some(port) = port {
        if port.is_empty() || !port.bytes().all(|b| b.is_ascii_digit()) {
            return false;
        }
    }
    if name.is_empty() {
        return false;
    }
    name.split('.').all(|label| {
        !label.is_empty()
            && !label.starts_with('-')
            && !label.ends_with('-')
            && label.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
    })
}

/// Point a `gh` invocation at the resolved host (see [`apply_gh_host_as`]).
pub(crate) fn apply_gh_host(command: &mut Command) {
    apply_gh_host_as(command, &gh_host());
}

/// Point a `gh` invocation at a specific `host`. For an enterprise host, also
/// expose the token via `GH_ENTERPRISE_TOKEN`: a bare `GH_TOKEN`/`GITHUB_TOKEN`
/// is github.com-scoped and gh ignores it for other hosts. Used to fetch a
/// linked PR from the host named in its URL rather than the configured host.
fn apply_gh_host_as(command: &mut Command, host: &str) {
    apply_gh_config_dir(command);
    if host != "github.com"
        && std::env::var_os("GH_ENTERPRISE_TOKEN").is_none()
        && std::env::var_os("GITHUB_ENTERPRISE_TOKEN").is_none()
    {
        if let Ok(token) = std::env::var("GH_TOKEN").or_else(|_| std::env::var("GITHUB_TOKEN")) {
            let token = token.trim().to_string();
            if !token.is_empty() {
                command.env("GH_ENTERPRISE_TOKEN", token);
            }
        }
    }
    command.env("GH_HOST", host);
}

/// Token for git-over-HTTPS to the resolved host. Reads `GH_TOKEN`/`GITHUB_TOKEN`
/// first, then falls back to `gh auth token` for the host — the latter works
/// inside Docker where only `~/.config/gh` is mounted and no token env is set.
///
/// A successful token is cached for the process; failures are NOT cached, so a
/// transient early failure (e.g. gh not ready during a boot-time clone storm)
/// can't wedge every later fetch into "could not read Username" until restart.
pub(crate) fn git_token() -> Option<String> {
    if let Some(token) = GIT_TOKEN.get() {
        return Some(token.clone());
    }
    let token = resolve_git_token();
    if let Some(token) = &token {
        let _ = GIT_TOKEN.set(token.clone());
    }
    token
}

pub(crate) fn resolve_git_token() -> Option<String> {
    if let Ok(token) = std::env::var("GH_TOKEN").or_else(|_| std::env::var("GITHUB_TOKEN")) {
        let token = token.trim().to_string();
        if !token.is_empty() {
            return Some(token);
        }
    }
    let mut token_cmd = Command::new("gh");
    token_cmd
        .args(["auth", "token", "--hostname", &gh_host()])
        .stdin(Stdio::null());
    apply_gh_config_dir(&mut token_cmd);
    let output = token_cmd.output().ok()?;
    if !output.status.success() {
        return None;
    }
    let token = String::from_utf8(output.stdout).ok()?.trim().to_string();
    (!token.is_empty()).then_some(token)
}

/// Configure a `git` command to authenticate HTTPS requests to the resolved
/// host with the gh token via `http.https://<host>/.extraheader`. This lets
/// OpenCodeReview clone/fetch private repos over HTTPS without SSH keys, regardless of
/// the host's configured `git_protocol`.
pub(crate) fn apply_git_https_auth(command: &mut Command) {
    if let Some(token) = git_token() {
        command
            .env("GIT_CONFIG_COUNT", "1")
            .env("GIT_CONFIG_KEY_0", format!("http.https://{}/.extraheader", gh_host()))
            .env(
                "GIT_CONFIG_VALUE_0",
                format!("AUTHORIZATION: basic {}", basic_auth_token(&token)),
            );
    }
}

/// base64 of `x-access-token:<token>` for an HTTP Basic `Authorization` header.
pub(crate) fn basic_auth_token(token: &str) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let bytes = format!("x-access-token:{token}").into_bytes();
    let mut out = String::new();
    let mut i = 0;
    while i < bytes.len() {
        let b0 = bytes[i];
        let b1 = bytes.get(i + 1).copied().unwrap_or(0);
        let b2 = bytes.get(i + 2).copied().unwrap_or(0);
        out.push(TABLE[(b0 >> 2) as usize] as char);
        out.push(TABLE[(((b0 & 0b11) << 4) | (b1 >> 4)) as usize] as char);
        out.push(if i + 1 < bytes.len() {
            TABLE[(((b1 & 0b1111) << 2) | (b2 >> 6)) as usize] as char
        } else {
            '='
        });
        out.push(if i + 2 < bytes.len() {
            TABLE[(b2 & 0b111111) as usize] as char
        } else {
            '='
        });
        i += 3;
    }
    out
}

/// Authenticated `gh` user login (cached). Used to detect the user's own PRs
/// (GitHub forbids approving / requesting changes on your own PR).
pub fn current_login() -> Option<String> {
    if let Some(login) = GH_LOGIN.lock().ok().and_then(|cached| cached.clone()) {
        return Some(login);
    }
    let login = run_raw("gh", &["api", "user", "--jq", ".login"])
        .ok()
        .and_then(|bytes| String::from_utf8(bytes).ok())
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());
    if let (Some(value), Ok(mut cached)) = (&login, GH_LOGIN.lock()) {
        *cached = Some(value.clone());
    }
    login
}

pub(crate) fn forget_identity() {
    if let Ok(mut cached) = GH_LOGIN.lock() {
        *cached = None;
    }
    if let Ok(mut cached) = GH_TEAM_SLUGS.lock() {
        *cached = None;
    }
}

pub(crate) fn sign_out_result(success: bool, stderr: &str) -> Result<(), String> {
    if success || stderr.to_ascii_lowercase().contains("not logged in") {
        return Ok(());
    }
    let detail = stderr
        .lines()
        .map(str::trim)
        .rfind(|line| !line.is_empty())
        .unwrap_or("gh auth logout failed");
    Err(format!("Could not sign out of GitHub: {detail}"))
}

pub fn sign_out() -> Result<(), String> {
    if std::env::var_os("GH_TOKEN").is_some() || std::env::var_os("GITHUB_TOKEN").is_some() {
        return Err("GitHub is connected through GH_TOKEN in the environment. Remove it from the OpenCodeReview configuration to sign out.".into());
    }
    let host = gh_host();
    let login = current_login();
    let mut command = Command::new("gh");
    command.args(["auth", "logout", "--hostname", &host]);
    if let Some(login) = &login {
        command.args(["--user", login]);
    }
    apply_gh_config_dir(&mut command);
    let output = command
        .stdin(Stdio::null())
        .output()
        .map_err(|error| format!("Could not run gh: {error}. Is the GitHub CLI installed?"))?;
    forget_identity();
    sign_out_result(output.status.success(), &String::from_utf8_lossy(&output.stderr))
}

// ----- GitHub auth (Settings "GitHub access" panel) -----

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GitHubAuthStatus {
    pub logged_in: bool,
    pub login: Option<String>,
    pub host: String,
    pub detail: String,
}

/// Read-only `gh auth status` (gh writes it to stderr), parsed for the panel.
pub fn auth_status() -> GitHubAuthStatus {
    let host = gh_host();
    if let Some(login) = current_login() {
        let detail = if std::env::var_os("GH_TOKEN").is_some() || std::env::var_os("GITHUB_TOKEN").is_some() {
            "gh authenticated via GH_TOKEN"
        } else {
            "gh authenticated"
        };
        return GitHubAuthStatus {
            logged_in: true,
            login: Some(login),
            host,
            detail: detail.to_string(),
        };
    }
    let mut status_cmd = Command::new("gh");
    status_cmd.args(["auth", "status", "--hostname", &host]);
    apply_gh_config_dir(&mut status_cmd);
    match status_cmd.output() {
        Ok(output) => {
            let text = format!(
                "{}\n{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
            let logged_in = output.status.success() && text.contains("Logged in");
            let detail = text
                .lines()
                .map(str::trim)
                .filter(|line| !line.is_empty())
                .take(6)
                .collect::<Vec<_>>()
                .join("\n");
            GitHubAuthStatus {
                logged_in,
                login: parse_gh_login(&text),
                host,
                detail,
            }
        }
        Err(error) => GitHubAuthStatus {
            logged_in: false,
            login: None,
            host,
            detail: format!("Could not run gh: {error}. Is the GitHub CLI installed?"),
        },
    }
}

pub(crate) fn parse_gh_login(text: &str) -> Option<String> {
    for marker in ["github.com account ", "github.com as "] {
        if let Some(idx) = text.find(marker) {
            let login: String = text[idx + marker.len()..]
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '[' | ']'))
                .collect();
            if !login.is_empty() {
                return Some(login);
            }
        }
    }
    None
}

pub(crate) fn find_device_code(line: &str) -> Option<String> {
    line.split_whitespace().find_map(|token| {
        let tok = token.trim_matches(|c: char| !c.is_ascii_alphanumeric() && c != '-');
        let parts: Vec<&str> = tok.split('-').collect();
        let looks_like_code = parts.len() == 2
            && parts
                .iter()
                .all(|p| p.len() == 4 && p.chars().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit()));
        looks_like_code.then(|| tok.to_string())
    })
}

pub(crate) fn find_url(line: &str) -> Option<String> {
    line.split_whitespace().find(|w| w.starts_with("https://")).map(|w| {
        w.trim_matches(|c: char| {
            !(c.is_ascii_alphanumeric() || matches!(c, '/' | ':' | '.' | '-' | '_' | '?' | '=' | '&'))
        })
        .to_string()
    })
}

/// Drive `gh auth login --web` / `gh auth refresh` (device flow) from the GUI:
/// capture the one-time code + verification URL from gh's output and report them
/// via `emit`, press Enter for the user so gh opens the browser, then wait until
/// gh exits. Best-effort — if the code can't be captured (no TTY quirks), the
/// caller's UI shows the manual `gh auth refresh` fallback.
pub fn run_auth_flow(cancel: &AtomicBool, action: &str, emit: &mut dyn FnMut(serde_json::Value)) {
    let host = gh_host();
    let args: Vec<&str> = if action == "refresh" {
        vec!["auth", "refresh", "--hostname", host.as_str()]
    } else {
        vec![
            "auth",
            "login",
            "--hostname",
            host.as_str(),
            "--git-protocol",
            "https",
            "--web",
        ]
    };
    let mut auth_cmd = Command::new("gh");
    auth_cmd
        .args(&args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    apply_gh_config_dir(&mut auth_cmd);
    let mut child = match auth_cmd.spawn() {
        Ok(child) => child,
        Err(error) => {
            emit(
                serde_json::json!({ "kind": "done", "success": false, "message": format!("Could not start gh: {error}. Is the GitHub CLI installed?") }),
            );
            return;
        }
    };
    let pid = child.id();
    let mut stdin = child.stdin.take();
    let (tx, rx) = mpsc::channel::<String>();
    for stream in [
        child
            .stdout
            .take()
            .map(|s| Box::new(s) as Box<dyn std::io::Read + Send>),
        child
            .stderr
            .take()
            .map(|s| Box::new(s) as Box<dyn std::io::Read + Send>),
    ]
    .into_iter()
    .flatten()
    {
        let tx = tx.clone();
        std::thread::spawn(move || {
            for line in BufReader::new(stream).lines().map_while(Result::ok) {
                if tx.send(line).is_err() {
                    break;
                }
            }
        });
    }
    drop(tx);
    let started = Instant::now();
    let (mut code_sent, mut url_sent) = (false, false);
    loop {
        if cancel.load(Ordering::Relaxed) {
            crate::review::kill_tree(pid, &mut child);
            emit(serde_json::json!({ "kind": "done", "success": false, "message": "Cancelled." }));
            return;
        }
        if started.elapsed() > Duration::from_secs(300) {
            crate::review::kill_tree(pid, &mut child);
            emit(
                serde_json::json!({ "kind": "done", "success": false, "message": "Timed out. Run `gh auth refresh` in a terminal to finish." }),
            );
            return;
        }
        while let Ok(line) = rx.try_recv() {
            if !code_sent {
                if let Some(code) = find_device_code(&line) {
                    code_sent = true;
                    emit(serde_json::json!({ "kind": "prompt", "code": code }));
                    // gh now waits for Enter to open the browser — press it for the user.
                    if let Some(mut handle) = stdin.take() {
                        let _ = handle.write_all(b"\n");
                        let _ = handle.flush();
                    }
                }
            }
            if !url_sent {
                if let Some(url) = find_url(&line) {
                    url_sent = true;
                    emit(serde_json::json!({ "kind": "prompt", "url": url }));
                }
            }
        }
        match child.try_wait() {
            Ok(Some(status)) => {
                while let Ok(line) = rx.try_recv() {
                    if !url_sent {
                        if let Some(url) = find_url(&line) {
                            url_sent = true;
                            emit(serde_json::json!({ "kind": "prompt", "url": url }));
                        }
                    }
                }
                emit(serde_json::json!({
                  "kind": "done",
                  "success": status.success(),
                  "message": if status.success() { "Signed in to GitHub.".to_string() } else { "gh didn't finish. If no code appeared, run `gh auth refresh` in a terminal.".to_string() },
                }));
                return;
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(300)),
            Err(error) => {
                emit(serde_json::json!({ "kind": "done", "success": false, "message": error.to_string() }));
                return;
            }
        }
    }
}
