use super::*;
use std::fs;
use std::io::Read;
use std::sync::OnceLock;

/// `cursor-agent login` polls for the browser sign-in for about 24 minutes; the flow ends sooner.
const FLOW_TIMEOUT: Duration = Duration::from_secs(900);
/// How long the CLI calls after the sign-in may take; they only read the account and its models.
const CLI_TIMEOUT: Duration = Duration::from_secs(60);

/// A Cursor user API key, for a new account or to replace `accountId`'s.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct CursorKey {
    key: String,
    account_id: Option<i64>,
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct CursorAuthStart {
    account_id: Option<i64>,
}

#[derive(Default)]
struct Flow {
    cancel: Option<Arc<AtomicBool>>,
    events: Vec<serde_json::Value>,
    secret: Option<String>,
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

/// The page `cursor-agent login` asks the user to open, from its output.
pub(crate) fn login_page(text: &str) -> Option<String> {
    text.split_whitespace()
        .find(|word| word.starts_with(crate::providers::cursor::LOGIN_PAGE) && word.len() <= 2048)
        .map(str::to_string)
}

/// The Cursor CLI in a private home under `home`, talking to Cursor directly.
fn cursor(home: &std::path::Path) -> Command {
    let mut command = Command::new("cursor-agent");
    command
        .env_clear()
        .env("PATH", "/usr/local/bin:/usr/bin:/bin")
        .env("HOME", home)
        .env("XDG_CONFIG_HOME", home.join("config"))
        .env("XDG_DATA_HOME", home.join("data"))
        .env("XDG_CACHE_HOME", home.join("cache"))
        .env("CURSOR_CONFIG_DIR", home.join("config/cursor"))
        .env("CURSOR_DATA_DIR", home.join("data"))
        .env("NO_COLOR", "1")
        .envs(crate::providers::cursor::QUIET_ENV.iter().copied())
        .current_dir(home)
        .stdin(Stdio::null());
    // Its own process group, so stopping it (`kill_tree`) also stops what it started.
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    command
}

fn private_home(prefix: &str) -> Result<std::path::PathBuf, String> {
    let home = std::env::temp_dir().join(format!("{prefix}-{}", crate::sandbox::secret()?));
    fs::create_dir_all(&home).map_err(|error| error.to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(&home, fs::Permissions::from_mode(0o700));
    }
    Ok(home)
}

/// The models and email of the account signed in at `home` (or with `key`): an account the CLI
/// can't list models for is not signed in.
fn account_details(
    home: &std::path::Path,
    key: Option<&str>,
    cancel: &AtomicBool,
) -> Result<(Vec<crate::providers::cursor::ListedModel>, Option<String>), String> {
    let with_key = |mut command: Command| {
        if let Some(key) = key {
            command.env(crate::providers::cursor::KEY_ENV, key);
        }
        command
    };
    let mut models = with_key(cursor(home));
    models.arg("models");
    let (status, listed) = bounded_cli_output(models, cancel, CLI_TIMEOUT, "Cursor")?;
    let listed = crate::providers::cursor::listed_models(&strip_terminal(&listed));
    if !status.success() || listed.is_empty() {
        return Err(if key.is_some() {
            "Cursor rejected that key".into()
        } else {
            "Signed in to Cursor, but could not read the models of its plan. Start again.".into()
        });
    }
    let mut about = with_key(cursor(home));
    about.args(["about", "--format", "json"]);
    let email = bounded_cli_output(about, cancel, CLI_TIMEOUT, "Cursor")
        .ok()
        .filter(|(status, _)| status.success())
        .and_then(|(_, output)| crate::providers::cursor::about_email(&output));
    Ok((listed, email))
}

/// Saves `raw` and its models to `account` (a re-sign-in) or to a new Cursor account named `label`.
fn store_account(
    db_handle: &Db,
    account: Option<i64>,
    raw: &str,
    label: Option<String>,
    models: &[crate::providers::cursor::ListedModel],
) -> Result<(), String> {
    let db = db_handle.open()?;
    let id = match account {
        Some(id) => {
            if db.ai_account(id)?.provider != "cursor" {
                return Err("That account is not a Cursor account".into());
            }
            id
        }
        None => {
            let count = db
                .list_ai_accounts()?
                .iter()
                .filter(|account| account.provider == "cursor")
                .count();
            let label = label.unwrap_or_else(|| {
                if count == 0 {
                    "Cursor".to_string()
                } else {
                    format!("Cursor {}", count + 1)
                }
            });
            db.create_ai_account("cursor", &label)?.id
        }
    };
    let stored =
        crate::accounts::store_cursor_auth(id, raw).and_then(|()| crate::accounts::store_cursor_models(id, models));
    if stored.is_err() && account.is_none() {
        // The folder goes with the row: a sign-in without its account would still reach runners.
        let _ = db.delete_ai_account(id);
        let _ = crate::accounts::remove(id);
    }
    stored?;
    db.reset_ai_account_status(id)
}

/// Saves a Cursor user API key to `accountId` or to a new account, once Cursor lists models for it.
pub(crate) fn save_cursor_key(request: &Request, state: &SharedState) -> Result<serde_json::Value, String> {
    require_trusted_origin(request)?;
    let input: CursorKey = serde_json::from_slice(&request.body).map_err(|_| "Invalid Cursor request")?;
    let key = input.key.trim();
    let raw = crate::providers::cursor::key_auth(key)?;
    if let Some(id) = input.account_id {
        if with_db_read_raw(state, |db| db.ai_account(id))?.provider != "cursor" {
            return Err("That account is not a Cursor account".into());
        }
    }
    let home = private_home("ocr-cursor-key")?;
    let details = account_details(&home, Some(key), &AtomicBool::new(false));
    let _ = fs::remove_dir_all(&home);
    let (models, email) = details?;
    store_account(&state.config.db, input.account_id, &raw, email, &models)?;
    Ok(serde_json::json!({ "ok": true }))
}

/// The browser sign-in the CLI stored at `home`, with its account's models and email.
fn signed_in(
    home: &std::path::Path,
    cancel: &AtomicBool,
) -> Result<(String, Vec<crate::providers::cursor::ListedModel>, Option<String>), String> {
    let stored = crate::sandbox::runner::read_output(
        &home.join("config/cursor/auth.json"),
        crate::providers::cursor::MAX_AUTH_BYTES as u64,
    )
    .ok()
    .flatten()
    .ok_or("Cursor sign-in did not finish. Start again.")?;
    let raw = crate::providers::cursor::login_auth(&stored)?;
    let (models, email) = account_details(home, None, cancel)?;
    Ok((raw, models, email))
}

/// Starts `cursor-agent login` in a fresh home and shows the page it asks for; once the user signs
/// in there, the sign-in is saved to `accountId` when given, otherwise to a new account.
pub(crate) fn start_cursor_auth(request: &Request, state: &SharedState) -> Result<serde_json::Value, String> {
    require_trusted_origin(request)?;
    let target: CursorAuthStart = if request.body.iter().all(u8::is_ascii_whitespace) {
        CursorAuthStart::default()
    } else {
        serde_json::from_slice(&request.body).map_err(|_| "Invalid sign-in request")?
    };
    let account = target.account_id;
    if let Some(id) = account {
        if with_db_read_raw(state, |db| db.ai_account(id))?.provider != "cursor" {
            return Err("That account is not a Cursor account".into());
        }
    }
    let db_handle = state.config.db.clone();
    let mut current = flow();
    if current.cancel.is_some() {
        return Ok(serde_json::json!({ "running": true, "flowSecret": current.secret }));
    }
    let home = private_home("ocr-cursor-auth")?;
    let mut child = cursor(&home)
        .arg("login")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| format!("Could not start the Cursor CLI: {error}"))?;
    let cancel = Arc::new(AtomicBool::new(false));
    let secret = generate_flow_secret();
    *current = Flow {
        cancel: Some(cancel.clone()),
        events: vec![],
        secret: Some(secret.clone()),
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
                    if text.len() < 65536 {
                        text.push_str(&String::from_utf8_lossy(&buffer[..read]));
                    }
                }
            }
        });
    }
    thread::spawn(move || {
        let pid = child.id();
        let started = Instant::now();
        let mut prompted = false;
        let finish = |event: serde_json::Value| {
            let _ = fs::remove_dir_all(&home);
            let mut current = flow();
            current.events.push(event);
            current.cancel = None;
        };
        loop {
            if !prompted {
                let text = strip_terminal(&output.lock().map(|text| text.clone()).unwrap_or_default());
                if let Some(url) = login_page(&text) {
                    prompted = true;
                    flow().events.push(serde_json::json!({ "kind": "prompt", "url": url }));
                }
            }
            if cancel.load(Ordering::Relaxed) || started.elapsed() > FLOW_TIMEOUT {
                crate::review::kill_tree(pid, &mut child);
                let message = if cancel.load(Ordering::Relaxed) {
                    "Sign-in was cancelled."
                } else {
                    "The sign-in page expired. Start again."
                };
                return finish(serde_json::json!({ "kind": "done", "success": false, "message": message }));
            }
            match child.try_wait() {
                Ok(Some(status)) => {
                    let saved = if status.success() {
                        signed_in(&home, &cancel)
                            .and_then(|(raw, models, label)| store_account(&db_handle, account, &raw, label, &models))
                    } else {
                        Err("Cursor sign-in did not finish. Start again.".into())
                    };
                    return finish(match saved {
                        Ok(()) => {
                            serde_json::json!({ "kind": "done", "success": true, "message": "Signed in to Cursor." })
                        }
                        Err(error) => serde_json::json!({ "kind": "done", "success": false, "message": error }),
                    });
                }
                Ok(None) => thread::sleep(Duration::from_millis(500)),
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

pub(crate) fn cursor_auth_events(request: &Request) -> Result<serde_json::Value, String> {
    let current = flow();
    authorized(request, &current)?;
    Ok(serde_json::json!({ "running": current.cancel.is_some(), "events": current.events }))
}

pub(crate) fn cancel_cursor_auth(request: &Request) -> Result<serde_json::Value, String> {
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
    fn the_sign_in_page_is_read_from_the_login_output_and_only_from_cursor() {
        let text = "Waiting for browser authentication...\nOpen a browser and navigate to this link: https://cursor.com/loginDeepControl?challenge=abc&uuid=1234&mode=login&redirectTarget=cli\n";
        assert_eq!(
            login_page(text).as_deref(),
            Some("https://cursor.com/loginDeepControl?challenge=abc&uuid=1234&mode=login&redirectTarget=cli")
        );
        assert_eq!(
            login_page("navigate to https://evil.example/loginDeepControl?x=1"),
            None
        );
        assert_eq!(login_page("https://cursor.com.evil.example/loginDeepControl?x=1"), None);
    }

    #[test]
    fn requests_take_only_a_key_and_an_account() {
        assert!(serde_json::from_str::<CursorKey>("{}").is_err());
        assert!(serde_json::from_str::<CursorKey>(r#"{"key":"x","model":"auto"}"#).is_err());
        let request: CursorKey = serde_json::from_str(r#"{"key":"x","accountId":3}"#).unwrap();
        assert_eq!(request.account_id, Some(3));
        assert!(serde_json::from_str::<CursorAuthStart>(r#"{"mode":"device"}"#).is_err());
    }
}
