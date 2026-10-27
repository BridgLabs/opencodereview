use super::*;
use std::fs;
use std::io::Read;
use std::net::{TcpListener, TcpStream};
use std::sync::OnceLock;

/// Device codes expire after 15 minutes.
const FLOW_TIMEOUT: Duration = Duration::from_secs(900);
const DEVICE_URL: &str = "https://auth.openai.com/codex/device";
/// `codex login` waits for the browser on 127.0.0.1:1455 inside this container. Docker publishes
/// the host's 127.0.0.1:1455 to this address, which is forwarded there while a sign-in runs.
const CALLBACK_TARGET: &str = "127.0.0.1:1455";

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

/// The one-time code `codex login --device-auth` prints, as `ABCD-EFGH`.
pub(crate) fn device_code(text: &str) -> Option<String> {
    text.split_whitespace()
        .find(|word| {
            let Some((left, right)) = word.split_once('-') else {
                return false;
            };
            let part = |value: &str| {
                (4..=6).contains(&value.len())
                    && value
                        .bytes()
                        .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit())
            };
            part(left) && part(right) && word.bytes().any(|byte| byte.is_ascii_digit())
        })
        .map(str::to_string)
}

/// The authorize link `codex login` prints for the browser sign-in.
pub(crate) fn browser_url(text: &str) -> Option<String> {
    text.split_whitespace()
        .find(|word| word.starts_with("https://auth.openai.com/oauth/authorize?"))
        .map(str::to_string)
}

/// Relays the browser's callback from the published port to `codex login` until `stop` is set.
fn forward_callback(listener: TcpListener, stop: Arc<AtomicBool>) {
    let _ = listener.set_nonblocking(true);
    while !stop.load(Ordering::Relaxed) {
        let Ok((client, _)) = listener.accept() else {
            thread::sleep(Duration::from_millis(200));
            continue;
        };
        let Ok(server) = TcpStream::connect(CALLBACK_TARGET) else {
            continue;
        };
        for stream in [&client, &server] {
            let _ = stream.set_nonblocking(false);
            let _ = stream.set_read_timeout(Some(Duration::from_secs(30)));
        }
        let (Ok(mut client_read), Ok(mut server_write)) = (client.try_clone(), server.try_clone()) else {
            continue;
        };
        let (mut server_read, mut client_write) = (server, client);
        thread::spawn(move || {
            let _ = std::io::copy(&mut client_read, &mut server_write);
            let _ = server_write.shutdown(std::net::Shutdown::Write);
        });
        thread::spawn(move || {
            let _ = std::io::copy(&mut server_read, &mut client_write);
            let _ = client_write.shutdown(std::net::Shutdown::Write);
        });
    }
}

/// Saves a signed-in `auth.json` to `account` (a re-sign-in) or to a new Codex account.
fn store_auth(db_handle: &Db, account: Option<i64>, raw: &str) -> Result<(), String> {
    let db = db_handle.open()?;
    let id = match account {
        Some(id) => {
            if db.ai_account(id)?.provider != "codex" {
                return Err("That account is not a Codex account".into());
            }
            id
        }
        None => {
            let count = db
                .list_ai_accounts()?
                .iter()
                .filter(|account| account.provider == "codex")
                .count();
            let label = crate::providers::codex::account_email(raw).unwrap_or_else(|| {
                if count == 0 {
                    "ChatGPT".to_string()
                } else {
                    format!("ChatGPT {}", count + 1)
                }
            });
            db.create_ai_account("codex", &label)?.id
        }
    };
    let stored = crate::accounts::store_codex_auth(id, raw);
    if stored.is_err() && account.is_none() {
        let _ = db.delete_ai_account(id);
    }
    stored?;
    db.reset_ai_account_status(id)
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct CodexAuthStart {
    account_id: Option<i64>,
    /// `device` signs in with a one-time code instead of the browser redirect.
    mode: Option<String>,
}

/// Starts `codex login` in a fresh Codex home, with the browser redirect or, in `device` mode, a
/// one-time code; the resulting sign-in is saved to `accountId` when given, otherwise to a new account.
pub(crate) fn start_codex_auth(request: &Request, state: &SharedState) -> Result<serde_json::Value, String> {
    require_trusted_origin(request)?;
    let target: CodexAuthStart = if request.body.iter().all(u8::is_ascii_whitespace) {
        CodexAuthStart::default()
    } else {
        serde_json::from_slice(&request.body).map_err(|_| "Invalid sign-in request")?
    };
    let account = target.account_id;
    let device = target.mode.as_deref() == Some("device");
    if let Some(id) = account {
        with_db_read_raw(state, |db| db.ai_account(id))?;
    }
    let db_handle = state.config.db.clone();
    let mut current = flow();
    if current.cancel.is_some() {
        return Ok(serde_json::json!({ "running": true, "flowSecret": current.secret }));
    }
    let callback = if device {
        None
    } else {
        let bind = std::env::var("OCR_CODEX_CALLBACK_BIND").unwrap_or_else(|_| "0.0.0.0:1456".into());
        Some(TcpListener::bind(&bind).map_err(|_| "The browser sign-in port is busy. Sign in with a code instead.")?)
    };
    let home = std::env::temp_dir().join(format!("ocr-codex-auth-{}", crate::sandbox::secret()?));
    fs::create_dir_all(&home).map_err(|error| error.to_string())?;
    let mut child = Command::new("codex")
        .args(if device {
            &["login", "--device-auth"][..]
        } else {
            &["login"][..]
        })
        .env("CODEX_HOME", &home)
        .env("HOME", &home)
        .env("NO_COLOR", "1")
        .env_remove("OPENAI_API_KEY")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| format!("Could not start the Codex CLI: {error}"))?;
    let cancel = Arc::new(AtomicBool::new(false));
    let secret = generate_flow_secret();
    *current = Flow {
        cancel: Some(cancel.clone()),
        events: vec![],
        secret: Some(secret.clone()),
    };
    drop(current);
    let relay = Arc::new(AtomicBool::new(false));
    if let Some(listener) = callback {
        let relay = relay.clone();
        thread::spawn(move || forward_callback(listener, relay));
    }
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
            relay.store(true, Ordering::Relaxed);
            let _ = fs::remove_dir_all(&home);
            let mut current = flow();
            current.events.push(event);
            current.cancel = None;
        };
        loop {
            let text = strip_terminal(&output.lock().map(|text| text.clone()).unwrap_or_default());
            if !prompted {
                let prompt = if device {
                    device_code(&text)
                        .map(|code| serde_json::json!({ "kind": "prompt", "url": DEVICE_URL, "code": code }))
                } else {
                    browser_url(&text).map(|url| serde_json::json!({ "kind": "prompt", "url": url }))
                };
                if let Some(prompt) = prompt {
                    prompted = true;
                    flow().events.push(prompt);
                }
            }
            if cancel.load(Ordering::Relaxed) || started.elapsed() > FLOW_TIMEOUT {
                crate::review::kill_tree(pid, &mut child);
                let message = if cancel.load(Ordering::Relaxed) {
                    "Sign-in was cancelled."
                } else {
                    "The code expired. Start again."
                };
                return finish(serde_json::json!({ "kind": "done", "success": false, "message": message }));
            }
            match child.try_wait() {
                Ok(Some(status)) => {
                    let saved = fs::read_to_string(home.join("auth.json"))
                        .ok()
                        .filter(|_| status.success());
                    let event = match saved.map(|raw| store_auth(&db_handle, account, &raw)) {
                        Some(Ok(())) => {
                            serde_json::json!({ "kind": "done", "success": true, "message": "Signed in to Codex." })
                        }
                        Some(Err(error)) => serde_json::json!({ "kind": "done", "success": false, "message": error }),
                        None => {
                            serde_json::json!({ "kind": "done", "success": false, "message": "Codex sign-in did not finish. Start again." })
                        }
                    };
                    return finish(event);
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

pub(crate) fn codex_auth_events(request: &Request) -> Result<serde_json::Value, String> {
    let current = flow();
    authorized(request, &current)?;
    Ok(serde_json::json!({ "running": current.cancel.is_some(), "events": current.events }))
}

pub(crate) fn cancel_codex_auth(request: &Request) -> Result<serde_json::Value, String> {
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
    fn device_code_is_read_from_the_cli_instructions() {
        let text = "1. Open this link in your browser and sign in to your account\n   https://auth.openai.com/codex/device\n\n2. Enter this one-time code (expires in 15 minutes)\n   7KQ2-9XWPL\n";
        assert_eq!(device_code(text).as_deref(), Some("7KQ2-9XWPL"));
        assert_eq!(device_code("Welcome to Codex [v0.163.0] command-line"), None);
        assert_eq!(device_code("https://auth.openai.com/codex/device"), None);
    }

    #[test]
    fn browser_url_is_read_from_the_login_instructions() {
        let text = "Starting local login server on http://localhost:1455.\nIf your browser did not open, navigate to this URL to authenticate:\n\nhttps://auth.openai.com/oauth/authorize?response_type=code&client_id=app_x&state=s\n\nOn a remote or headless machine? Use `codex login --device-auth` instead.";
        assert_eq!(
            browser_url(text).as_deref(),
            Some("https://auth.openai.com/oauth/authorize?response_type=code&client_id=app_x&state=s")
        );
        assert_eq!(
            browser_url("Starting local login server on http://localhost:1455."),
            None
        );
    }
}
