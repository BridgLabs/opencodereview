use super::*;
use std::fs;
use std::io::{Read, Write};
use std::process::ChildStdin;
use std::sync::OnceLock;

const FLOW_TIMEOUT: Duration = Duration::from_secs(600);
const SETUP: &str = "stty cols 1000 rows 50 2>/dev/null; exec claude setup-token";
const TOKEN_PREFIX: &str = "sk-ant-oat";

#[derive(Default)]
struct Flow {
    cancel: Option<Arc<AtomicBool>>,
    events: Vec<serde_json::Value>,
    secret: Option<String>,
    input: Option<ChildStdin>,
}

fn flow() -> std::sync::MutexGuard<'static, Flow> {
    static FLOW: OnceLock<Mutex<Flow>> = OnceLock::new();
    FLOW.get_or_init(Default::default)
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn authorized(request: &Request, flow: &Flow) -> Result<(), String> {
    require_trusted_origin(request)?;
    match &flow.secret {
        Some(expected) if request.flow_secret.as_deref() != Some(expected.as_str()) => {
            Err("forbidden: invalid flow secret".into())
        }
        _ => Ok(()),
    }
}

fn emit(event: serde_json::Value) {
    flow().events.push(event);
}

pub(crate) fn strip_terminal(raw: &str) -> String {
    let mut text = String::new();
    let mut chars = raw.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\u{1b}' => match chars.next() {
                Some('[') => {
                    // The CLI draws its screen with cursor moves: forward means a gap, the rest a new line.
                    match chars.by_ref().find(|next| ('@'..='~').contains(next)) {
                        Some('m') | None => {}
                        Some('C') => text.push(' '),
                        Some(_) => text.push('\n'),
                    }
                }
                Some(']') => {
                    while let Some(next) = chars.next() {
                        if next == '\u{7}' || next == '\u{1b}' && chars.next_if_eq(&'\\').is_some() {
                            break;
                        }
                    }
                }
                _ => {}
            },
            '\r' => {}
            c if c.is_control() && c != '\n' => {}
            c => text.push(c),
        }
    }
    text
}

/// Rejoins a value the terminal wrapped across lines: single line breaks continue it,
/// a blank line or any character outside `allowed` ends it.
pub(crate) fn wrapped(text: &str, start: usize, allowed: impl Fn(char) -> bool) -> String {
    let mut value = String::new();
    let mut newlines = 0;
    for c in text[start..].chars() {
        if c == '\n' {
            newlines += 1;
            if newlines > 1 {
                break;
            }
        } else if allowed(c) {
            newlines = 0;
            value.push(c);
        } else {
            break;
        }
    }
    value
}

pub(crate) fn sign_in_url(text: &str) -> Option<String> {
    let start = text.find("https://")?;
    let url = wrapped(text, start, |c| c.is_ascii_graphic());
    url.contains("/oauth/authorize").then_some(url)
}

pub(crate) fn oauth_token(text: &str) -> Option<String> {
    let start = text.rfind(TOKEN_PREFIX)?;
    let token: String = text[start..]
        .chars()
        .take_while(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
        .collect();
    (token.len() > TOKEN_PREFIX.len() + 16 && token.len() <= crate::providers::claude::MAX_TOKEN_BYTES).then_some(token)
}

pub(crate) fn rejected(text: &str) -> bool {
    let compact: String = text
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect::<String>()
        .to_ascii_lowercase();
    compact.contains("oautherror") || compact.contains("pressentertoretry")
}

pub(crate) fn failure_message(text: &str) -> String {
    let lower = text.to_ascii_lowercase();
    if lower.contains("invalid code") || lower.contains("oauth error") || lower.contains("invalid_grant") {
        "Claude did not accept that code. Start again and paste the newest code shown after signing in.".into()
    } else if lower.contains("subscription") && lower.contains("required") {
        "This Claude account has no eligible subscription.".into()
    } else {
        "Claude sign-in did not finish. Start again.".into()
    }
}

/// Saves a signed-in token to `account` (a re-sign-in) or to a new Claude account.
fn store_token(db_path: &Path, account: Option<i64>, token: &str) -> Result<(), String> {
    let db = Database::open_at(db_path.to_path_buf())?;
    let id = match account {
        Some(id) => {
            if db.ai_account(id)?.provider != "claude" {
                return Err("That account is not a Claude account".into());
            }
            id
        }
        None => {
            let count = db
                .list_ai_accounts()?
                .iter()
                .filter(|account| account.provider == "claude")
                .count();
            db.create_ai_account(
                "claude",
                &if count == 0 {
                    "Claude".to_string()
                } else {
                    format!("Claude {}", count + 1)
                },
            )?
            .id
        }
    };
    let stored = crate::accounts::store_claude_token(id, token);
    if stored.is_err() && account.is_none() {
        let _ = db.delete_ai_account(id);
    }
    stored?;
    db.reset_ai_account_status(id)
}

pub(crate) fn claude_cli_version(output: Result<Output, String>) -> Option<String> {
    let output = output.ok().filter(|output| output.status.success())?;
    let text = String::from_utf8(output.stdout).ok()?;
    text.split_whitespace()
        .next()
        .filter(|version| version.bytes().all(|byte| byte.is_ascii_digit() || byte == b'.'))
        .map(str::to_string)
}

/// Connected when at least one enabled account, of any provider, is signed in and not rejected.
pub(crate) fn claude_auth_status(state: &SharedState) -> Result<serde_json::Value, String> {
    let version = claude_cli_version(command_output_timeout("claude", &["--version"], Duration::from_secs(5)));
    let _ = crate::accounts::sandbox_sync::sync_app_auth();
    let accounts = with_db_read_raw(state, |db| db.list_ai_accounts())?;
    let ready = accounts
        .iter()
        .filter(|account| {
            account.enabled
                && account.status != "auth_failed"
                && crate::accounts::has_credential(&account.provider, account.id)
        })
        .count();
    Ok(serde_json::json!({
      "loggedIn": ready > 0,
      "message": match (ready, accounts.len()) {
          (0, 0) => "Connect an AI account to run reviews.".to_string(),
          (0, _) => "No account is ready; sign in again from Settings → Accounts.".to_string(),
          (1, _) => "1 account ready.".to_string(),
          (count, _) => format!("{count} accounts ready."),
      },
      "version": version,
    }))
}

/// Removes every Claude account and its credentials.
pub(crate) fn claude_auth_logout(request: &Request, state: &SharedState) -> Result<serde_json::Value, String> {
    require_trusted_origin(request)?;
    let ids = with_db_raw(state, |db| {
        let ids = db
            .list_ai_accounts()?
            .into_iter()
            .filter(|account| account.provider == "claude")
            .map(|account| account.id)
            .collect::<Vec<_>>();
        for id in &ids {
            db.delete_ai_account(*id)?;
        }
        Ok(ids)
    })?;
    for id in ids {
        crate::accounts::remove(id)?;
    }
    crate::accounts::sandbox_sync::sync_app_auth()?;
    Ok(serde_json::json!({ "ok": true, "message": "Signed out of Claude." }))
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ClaudeAuthStart {
    account_id: Option<i64>,
}

/// Starts `claude setup-token`; the token is saved to `accountId` when given, otherwise to a new account.
pub(crate) fn start_claude_auth(request: &Request, state: &SharedState) -> Result<serde_json::Value, String> {
    require_trusted_origin(request)?;
    let target: ClaudeAuthStart = if request.body.iter().all(u8::is_ascii_whitespace) {
        ClaudeAuthStart::default()
    } else {
        serde_json::from_slice(&request.body).map_err(|_| "Invalid sign-in request")?
    };
    let account = target.account_id;
    if let Some(id) = account {
        with_db_read_raw(state, |db| db.ai_account(id))?;
    }
    let db_path = state.config.db_path.clone();
    let mut current = flow();
    if current.cancel.is_some() {
        return Ok(serde_json::json!({ "running": true, "flowSecret": current.secret }));
    }
    let home = std::env::temp_dir().join(format!("ocr-claude-auth-{}", crate::sandbox::secret()?));
    fs::create_dir_all(home.join(".claude")).map_err(|error| error.to_string())?;
    let mut command = Command::new("script");
    command
        .args(["-qfec", SETUP, "/dev/null"])
        .env("HOME", &home)
        .env("CLAUDE_CONFIG_DIR", home.join(".claude"))
        .env("BROWSER", "true")
        .env("COLUMNS", "1000")
        .env("NO_COLOR", "1")
        .env("DISABLE_AUTOUPDATER", "1")
        .env("DISABLE_TELEMETRY", "1")
        .env("DISABLE_ERROR_REPORTING", "1")
        .env_remove("CLAUDE_CODE_OAUTH_TOKEN")
        .env_remove("ANTHROPIC_API_KEY")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = command
        .spawn()
        .map_err(|error| format!("Could not start the Claude CLI: {error}"))?;
    let cancel = Arc::new(AtomicBool::new(false));
    let secret = generate_flow_secret();
    *current = Flow {
        cancel: Some(cancel.clone()),
        events: vec![],
        secret: Some(secret.clone()),
        input: child.stdin.take(),
    };
    drop(current);
    let output = Arc::new(Mutex::new(String::new()));
    for stream in [
        child.stdout.take().map(|s| Box::new(s) as Box<dyn Read + Send>),
        child.stderr.take().map(|s| Box::new(s) as Box<dyn Read + Send>),
    ]
    .into_iter()
    .flatten()
    {
        let output = output.clone();
        thread::spawn(move || {
            let mut stream = stream;
            let mut buffer = [0u8; 4096];
            while let Ok(read) = stream.read(&mut buffer) {
                if read == 0 {
                    break;
                }
                if let Ok(mut text) = output.lock() {
                    text.push_str(&String::from_utf8_lossy(&buffer[..read]));
                    if text.len() > 65536 {
                        let mut cut = text.len() - 32768;
                        while !text.is_char_boundary(cut) {
                            cut += 1;
                        }
                        text.replace_range(..cut, "");
                    }
                }
            }
        });
    }
    thread::spawn(move || {
        let pid = child.id();
        let started = Instant::now();
        let mut url_sent = false;
        let finish = |event: serde_json::Value| {
            if let Ok(mut text) = output.lock() {
                text.clear();
            }
            let _ = fs::remove_dir_all(&home);
            let mut current = flow();
            current.events.push(event);
            current.cancel = None;
            current.input = None;
        };
        loop {
            let text = strip_terminal(&output.lock().map(|text| text.clone()).unwrap_or_default());
            if !url_sent {
                if let Some(url) = sign_in_url(&text) {
                    url_sent = true;
                    emit(serde_json::json!({ "kind": "prompt", "url": url }));
                }
            }
            if let Some(token) = oauth_token(&text) {
                crate::review::kill_tree(pid, &mut child);
                let message = match store_token(&db_path, account, &token) {
                    Ok(()) => serde_json::json!({ "kind": "done", "success": true, "message": "Signed in to Claude." }),
                    Err(error) => serde_json::json!({ "kind": "done", "success": false, "message": error }),
                };
                return finish(message);
            }
            if rejected(&text) {
                crate::review::kill_tree(pid, &mut child);
                return finish(
                    serde_json::json!({ "kind": "done", "success": false, "message": failure_message(&text) }),
                );
            }
            if cancel.load(Ordering::Relaxed) || started.elapsed() > FLOW_TIMEOUT {
                crate::review::kill_tree(pid, &mut child);
                let message = if cancel.load(Ordering::Relaxed) {
                    "Sign-in was cancelled."
                } else {
                    "Sign-in timed out. Start again."
                };
                return finish(serde_json::json!({ "kind": "done", "success": false, "message": message }));
            }
            match child.try_wait() {
                Ok(Some(_)) => {
                    thread::sleep(Duration::from_millis(200));
                    let text = strip_terminal(&output.lock().map(|text| text.clone()).unwrap_or_default());
                    if oauth_token(&text).is_some() {
                        continue;
                    }
                    return finish(
                        serde_json::json!({ "kind": "done", "success": false, "message": failure_message(&text) }),
                    );
                }
                Ok(None) => thread::sleep(Duration::from_millis(300)),
                Err(error) => {
                    return finish(
                        serde_json::json!({ "kind": "done", "success": false, "message": error.to_string() }),
                    )
                }
            }
        }
    });
    Ok(serde_json::json!({ "running": true, "flowSecret": secret }))
}

pub(crate) fn claude_auth_events(request: &Request) -> Result<serde_json::Value, String> {
    let current = flow();
    authorized(request, &current)?;
    Ok(serde_json::json!({ "running": current.cancel.is_some(), "events": current.events }))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ClaudeTokenPaste {
    token: String,
    account_id: Option<i64>,
}

/// Saves a token the user made with `claude setup-token` on their own machine.
pub(crate) fn save_claude_token(request: &Request, state: &SharedState) -> Result<serde_json::Value, String> {
    require_trusted_origin(request)?;
    let paste: ClaudeTokenPaste = serde_json::from_slice(&request.body).map_err(|_| "Invalid token")?;
    let token = paste.token.trim();
    if oauth_token(token).as_deref() != Some(token) {
        return Err(
            "That is not a Claude token. Run `claude setup-token` and paste the token that starts with sk-ant-oat"
                .into(),
        );
    }
    if let Some(id) = paste.account_id {
        with_db_read_raw(state, |db| db.ai_account(id))?;
    }
    store_token(&state.config.db_path, paste.account_id, token)?;
    Ok(serde_json::json!({ "ok": true }))
}

pub(crate) fn claude_auth_input(request: &Request) -> Result<serde_json::Value, String> {
    let mut current = flow();
    authorized(request, &current)?;
    let payload: serde_json::Value =
        serde_json::from_slice(&request.body).map_err(|_| "Invalid sign-in code".to_string())?;
    let code = payload["input"].as_str().map(str::trim).unwrap_or_default();
    if code.is_empty() || code.len() > 4096 || code.chars().any(char::is_control) {
        return Err("Paste the code shown after signing in to Claude".into());
    }
    if !code.contains('#') {
        return Err("That code is incomplete. Copy the whole code Claude shows, including the part after #".into());
    }
    let input = current
        .input
        .as_mut()
        .ok_or_else(|| "Claude sign-in is not running; start again".to_string())?;
    input
        .write_all(code.as_bytes())
        .and_then(|_| input.flush())
        .map_err(|_| "Claude sign-in is not running; start again".to_string())?;
    thread::sleep(Duration::from_millis(500));
    input
        .write_all(b"\r")
        .and_then(|_| input.flush())
        .map_err(|_| "Claude sign-in is not running; start again".to_string())?;
    current.events.push(serde_json::json!({ "kind": "submitted" }));
    Ok(serde_json::json!({ "ok": true }))
}

pub(crate) fn cancel_claude_auth(request: &Request) -> Result<serde_json::Value, String> {
    let current = flow();
    authorized(request, &current)?;
    if let Some(cancel) = &current.cancel {
        cancel.store(true, Ordering::Relaxed);
    }
    Ok(serde_json::json!({ "running": false }))
}

#[cfg(test)]
#[path = "claude_auth_tests.rs"]
mod tests;
