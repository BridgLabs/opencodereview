use super::*;
use std::fs;
use std::io::{Read, Write};
use std::process::ChildStdin;
use std::sync::OnceLock;

/// The CLI gives up on the pasted code after five minutes.
const FLOW_TIMEOUT: Duration = Duration::from_secs(300);
/// Without a browser the CLI prints the Google sign-in link and waits for the code Google shows.
const SIGN_IN: &str = "stty cols 1000 rows 50 2>/dev/null; exec gemini";
const AUTHORIZE: &str = "https://accounts.google.com/o/oauth2/v2/auth";

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

/// The Google sign-in link the CLI prints, rejoined if the terminal wrapped it.
pub(crate) fn google_sign_in_url(text: &str) -> Option<String> {
    let start = text.find(AUTHORIZE)?;
    Some(wrapped(text, start, |c| c.is_ascii_graphic()))
}

pub(crate) fn google_rejected(text: &str) -> bool {
    text.contains("Failed to authenticate with authorization code") || text.contains("Authorization code is required")
}

/// Saves a signed-in `oauth_creds.json` to `account` (a re-sign-in) or to a new Gemini account,
/// named after the Google email from the ID token or the CLI's account cache.
fn store_auth(db_handle: &Db, account: Option<i64>, raw: &str, cached_email: Option<String>) -> Result<(), String> {
    let db = db_handle.open()?;
    let id = match account {
        Some(id) => {
            if db.ai_account(id)?.provider != "gemini" {
                return Err("That account is not a Gemini account".into());
            }
            id
        }
        None => {
            let count = db
                .list_ai_accounts()?
                .iter()
                .filter(|account| account.provider == "gemini")
                .count();
            let label = crate::providers::gemini::account_email(raw)
                .or(cached_email)
                .unwrap_or_else(|| {
                    if count == 0 {
                        "Gemini".to_string()
                    } else {
                        format!("Gemini {}", count + 1)
                    }
                });
            db.create_ai_account("gemini", &label)?.id
        }
    };
    let stored = crate::accounts::store_gemini_auth(id, raw);
    if stored.is_err() && account.is_none() {
        let _ = db.delete_ai_account(id);
    }
    stored?;
    db.reset_ai_account_status(id)
}

/// The email the CLI cached after signing in, from `google_accounts.json`.
pub(crate) fn cached_google_email(raw: &str) -> Option<String> {
    let value: serde_json::Value = serde_json::from_str(raw).ok()?;
    value["active"]
        .as_str()
        .filter(|email| email.contains('@') && email.len() <= 80)
        .map(str::to_string)
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct GeminiAuthStart {
    account_id: Option<i64>,
}

/// Starts the Gemini CLI's Google sign-in in a fresh home; the sign-in is saved to `accountId`
/// when given, otherwise to a new account.
pub(crate) fn start_gemini_auth(request: &Request, state: &SharedState) -> Result<serde_json::Value, String> {
    require_trusted_origin(request)?;
    let target: GeminiAuthStart = if request.body.iter().all(u8::is_ascii_whitespace) {
        GeminiAuthStart::default()
    } else {
        serde_json::from_slice(&request.body).map_err(|_| "Invalid sign-in request")?
    };
    let account = target.account_id;
    if let Some(id) = account {
        with_db_read_raw(state, |db| db.ai_account(id))?;
    }
    let db_handle = state.config.db.clone();
    let mut current = flow();
    if current.cancel.is_some() {
        return Ok(serde_json::json!({ "running": true, "flowSecret": current.secret }));
    }
    let home = std::env::temp_dir().join(format!("ocr-gemini-auth-{}", crate::sandbox::secret()?));
    let config = home.join(".gemini");
    fs::create_dir_all(&config).map_err(|error| error.to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(&home, fs::Permissions::from_mode(0o700));
    }
    fs::write(
        config.join("settings.json"),
        crate::providers::gemini::settings().to_string(),
    )
    .map_err(|error| error.to_string())?;
    let mut child = Command::new("script")
        .args(["-qfec", SIGN_IN, "/dev/null"])
        .env("HOME", &home)
        .env("NO_BROWSER", "true")
        .env("NO_COLOR", "1")
        .env("COLUMNS", "1000")
        .env("GEMINI_CLI_NO_RELAUNCH", "true")
        .env_remove("GEMINI_API_KEY")
        .env_remove("GOOGLE_API_KEY")
        .env_remove("GOOGLE_APPLICATION_CREDENTIALS")
        .env_remove("GOOGLE_CLOUD_PROJECT")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| format!("Could not start the Gemini CLI: {error}"))?;
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
                if let Some(url) = google_sign_in_url(&text) {
                    url_sent = true;
                    flow().events.push(serde_json::json!({ "kind": "prompt", "url": url }));
                }
            }
            // The CLI saves the sign-in as soon as Google accepts the code, then caches the email.
            if let Some(raw) = fs::read_to_string(config.join("oauth_creds.json"))
                .ok()
                .filter(|raw| crate::providers::gemini::validate_auth(raw).is_ok())
            {
                let deadline = Instant::now() + Duration::from_secs(5);
                let mut email = None;
                while email.is_none() && Instant::now() < deadline {
                    email = fs::read_to_string(config.join("google_accounts.json"))
                        .ok()
                        .and_then(|raw| cached_google_email(&raw));
                    if email.is_none() {
                        thread::sleep(Duration::from_millis(250));
                    }
                }
                crate::review::kill_tree(pid, &mut child);
                let message = match store_auth(&db_handle, account, &raw, email) {
                    Ok(()) => serde_json::json!({ "kind": "done", "success": true, "message": "Signed in to Gemini." }),
                    Err(error) => serde_json::json!({ "kind": "done", "success": false, "message": error }),
                };
                return finish(message);
            }
            if google_rejected(&text) {
                crate::review::kill_tree(pid, &mut child);
                return finish(
                    serde_json::json!({ "kind": "done", "success": false, "message": "Google did not accept that code. Start again and paste the newest code shown after signing in." }),
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
                    if config.join("oauth_creds.json").exists() {
                        continue;
                    }
                    return finish(
                        serde_json::json!({ "kind": "done", "success": false, "message": "Gemini sign-in did not finish. Start again." }),
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

pub(crate) fn gemini_auth_events(request: &Request) -> Result<serde_json::Value, String> {
    let current = flow();
    authorized(request, &current)?;
    Ok(serde_json::json!({ "running": current.cancel.is_some(), "events": current.events }))
}

pub(crate) fn gemini_auth_input(request: &Request) -> Result<serde_json::Value, String> {
    let mut current = flow();
    authorized(request, &current)?;
    let payload: serde_json::Value =
        serde_json::from_slice(&request.body).map_err(|_| "Invalid sign-in code".to_string())?;
    let code = payload["input"].as_str().map(str::trim).unwrap_or_default();
    if code.is_empty() || code.len() > 2048 || code.chars().any(|c| c.is_control() || c.is_whitespace()) {
        return Err("Paste the code Google shows after signing in".into());
    }
    let input = current
        .input
        .as_mut()
        .ok_or_else(|| "Gemini sign-in is not running; start again".to_string())?;
    input
        .write_all(code.as_bytes())
        .and_then(|_| input.write_all(b"\r"))
        .and_then(|_| input.flush())
        .map_err(|_| "Gemini sign-in is not running; start again".to_string())?;
    current.events.push(serde_json::json!({ "kind": "submitted" }));
    Ok(serde_json::json!({ "ok": true }))
}

pub(crate) fn cancel_gemini_auth(request: &Request) -> Result<serde_json::Value, String> {
    let current = flow();
    authorized(request, &current)?;
    if let Some(cancel) = &current.cancel {
        cancel.store(true, Ordering::Relaxed);
    }
    Ok(serde_json::json!({ "running": false }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sign_in_output_yields_the_google_link_and_rejections() {
        let url = "https://accounts.google.com/o/oauth2/v2/auth?redirect_uri=https%3A%2F%2Fcodeassist.google.com%2Fauthcode&access_type=offline&response_type=code&client_id=fixture";
        let raw = format!("\u{1b}[?25lPlease visit the following URL to authorize the application:\r\n\r\n{url}\r\n\r\nEnter the authorization code: ");
        assert_eq!(google_sign_in_url(&strip_terminal(&raw)).as_deref(), Some(url));
        assert_eq!(google_sign_in_url("Visit https://example.com/help"), None);
        assert!(google_rejected(
            "Failed to authenticate with authorization code:invalid_grant"
        ));
        assert!(!google_rejected("Enter the authorization code:"));
    }

    #[test]
    fn the_cached_google_account_names_the_sign_in() {
        assert_eq!(
            cached_google_email(r#"{"active":"dev@example.com","old":[]}"#).as_deref(),
            Some("dev@example.com")
        );
        assert_eq!(cached_google_email(r#"{"active":null,"old":[]}"#), None);
        assert_eq!(cached_google_email("not json"), None);
    }
}
