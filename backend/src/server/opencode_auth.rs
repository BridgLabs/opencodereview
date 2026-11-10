use super::*;
use std::fs;
use std::io::Read;
use std::sync::OnceLock;

/// OpenCode Console device codes expire after 10 minutes.
const FLOW_TIMEOUT: Duration = Duration::from_secs(600);
/// How long the CLI calls after the approval may take; they only read the sign-in and list models.
const CLI_TIMEOUT: Duration = Duration::from_secs(60);
/// Where the Console asks the user to approve a device code.
const DEVICE_PAGE: &str = "https://opencode.ai/console/device?";

/// An OpenCode API key, for a new account or to replace `accountId`'s.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct OpencodeKey {
    key: String,
    account_id: Option<i64>,
}

/// The Go and Zen model lists, with the plan each one bills; they answer 401 to a key they don't
/// accept, and cost no tokens.
const KEY_CHECKS: &[(&str, &str)] = &[
    ("go", "https://opencode.ai/inference/go/v1/models"),
    ("zen", "https://opencode.ai/zen/v1/models"),
];

/// The plans whose check accepted the key, from each check's HTTP status (None when OpenCode
/// could not be reached). A plan is left out only when OpenCode refuses the key for it: any other
/// failure fails the whole check, so a key is never saved without a plan it pays for.
fn accepted_plans(checks: &[(&'static str, Option<u16>)]) -> Result<Vec<&'static str>, String> {
    let mut plans = Vec::new();
    for (plan, status) in checks {
        match status {
            Some(200..=299) => plans.push(*plan),
            Some(401 | 403) => {}
            Some(status) => return Err(format!("OpenCode could not check the key (HTTP {status}); try again")),
            None => return Err("Could not reach OpenCode to check the key; try again".into()),
        }
    }
    if plans.is_empty() {
        return Err("OpenCode rejected that key".into());
    }
    Ok(plans)
}

/// The plans OpenCode accepts `key` for, or why it accepts it for none.
fn verify_key(key: &str) -> Result<Vec<&'static str>, String> {
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(15))
        .build()
        .map_err(|error| error.to_string())?;
    let checks = KEY_CHECKS
        .iter()
        .map(|(plan, url)| {
            let status = client.get(*url).bearer_auth(key).send().ok();
            (*plan, status.map(|response| response.status().as_u16()))
        })
        .collect::<Vec<_>>();
    accepted_plans(&checks)
}

/// Saves `raw` to `account` (a re-sign-in) or to a new OpenCode account named `label`.
fn store_account(
    db_handle: &Db,
    account: Option<i64>,
    raw: &str,
    label: Option<String>,
    plans: &[&str],
) -> Result<(), String> {
    let db = db_handle.open()?;
    let id = match account {
        Some(id) => {
            if db.ai_account(id)?.provider != "opencode" {
                return Err("That account is not an OpenCode account".into());
            }
            id
        }
        None => {
            let count = db
                .list_ai_accounts()?
                .iter()
                .filter(|account| account.provider == "opencode")
                .count();
            let label = label.unwrap_or_else(|| {
                if count == 0 {
                    "OpenCode".to_string()
                } else {
                    format!("OpenCode {}", count + 1)
                }
            });
            db.create_ai_account("opencode", &label)?.id
        }
    };
    let stored =
        crate::accounts::store_opencode_auth(id, raw).and_then(|()| crate::accounts::store_opencode_plans(id, plans));
    if stored.is_err() && account.is_none() {
        // The folder goes with the row: a sign-in without its account would still reach runners.
        let _ = db.delete_ai_account(id);
        let _ = crate::accounts::remove(id);
    }
    stored?;
    db.reset_ai_account_status(id)
}

/// Saves an OpenCode API key to `accountId` or to a new account, once OpenCode accepts it.
pub(crate) fn save_opencode_key(request: &Request, state: &SharedState) -> Result<serde_json::Value, String> {
    require_trusted_origin(request)?;
    let input: OpencodeKey = serde_json::from_slice(&request.body).map_err(|_| "Invalid OpenCode request")?;
    let key = input.key.trim();
    let raw = crate::providers::opencode::auth(key)?;
    if let Some(id) = input.account_id {
        if with_db_read_raw(state, |db| db.ai_account(id))?.provider != "opencode" {
            return Err("That account is not an OpenCode account".into());
        }
    }
    let plans = verify_key(key)?;
    store_account(&state.config.db, input.account_id, &raw, None, &plans)?;
    Ok(serde_json::json!({ "ok": true }))
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

/// The one-time code and approval page `opencode auth login opencode --method device` prints.
pub(crate) fn device_prompt(text: &str) -> Option<(String, String)> {
    let code = text
        .lines()
        .find_map(|line| line.split_once("Enter code:"))
        .map(|(_, code)| code.trim().to_string())
        .filter(|code| {
            !code.is_empty() && code.len() <= 16 && code.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
        })?;
    let url = text
        .split_whitespace()
        .find(|word| word.starts_with(DEVICE_PAGE))?
        .to_string();
    Some((code, url))
}

/// OpenCode in a private home under `home`, talking to opencode.ai directly.
fn opencode(home: &std::path::Path) -> Command {
    let mut command = Command::new("opencode");
    command
        .env_clear()
        .env("PATH", "/usr/local/bin:/usr/bin:/bin")
        .env("HOME", home)
        .env("XDG_DATA_HOME", home.join("data"))
        .env("XDG_CONFIG_HOME", home.join("config"))
        .env("XDG_CACHE_HOME", home.join("cache"))
        .env("XDG_STATE_HOME", home.join("state"))
        .env("NO_COLOR", "1")
        .envs(crate::providers::opencode::QUIET_ENV.iter().copied())
        .current_dir(home)
        .stdin(Stdio::null());
    command
}

/// The plans a workspace has, from the models `opencode models` lists for it: Go models only appear
/// with a Go subscription.
pub(crate) fn plans_listed(models: &str) -> Vec<&'static str> {
    ["go", "zen"]
        .into_iter()
        .filter(|plan| {
            models
                .lines()
                .any(|line| crate::providers::opencode_catalogue::plan(line.trim()) == Some(*plan))
        })
        .collect()
}

/// Runs `command` to its end and returns its exit status and output. It is killed when the flow
/// is cancelled or it takes longer than `limit`, so the thread watching a sign-in always finishes.
fn bounded_output(
    mut command: Command,
    cancel: &AtomicBool,
    limit: Duration,
) -> Result<(std::process::ExitStatus, String), String> {
    // Its own process group, so stopping it also stops what it started.
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    let mut child = command
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|error| format!("Could not start the OpenCode CLI: {error}"))?;
    let pid = child.id();
    let stdout = child.stdout.take();
    let reader = thread::spawn(move || {
        let mut bytes = Vec::new();
        if let Some(stdout) = stdout {
            let _ = stdout.take(1 << 20).read_to_end(&mut bytes);
        }
        String::from_utf8_lossy(&bytes).into_owned()
    });
    let started = Instant::now();
    loop {
        let stopped = match child.try_wait() {
            Ok(Some(status)) => return Ok((status, reader.join().unwrap_or_default())),
            Ok(None) if cancel.load(Ordering::Relaxed) => "Sign-in was cancelled.".to_string(),
            Ok(None) if started.elapsed() > limit => "OpenCode did not answer in time. Start again.".to_string(),
            Ok(None) => {
                thread::sleep(Duration::from_millis(100));
                continue;
            }
            Err(error) => error.to_string(),
        };
        crate::review::kill_tree(pid, &mut child);
        return Err(stopped);
    }
}

/// The signed-in Console credential, the email it signed in with and its workspace's plans.
fn signed_in(
    home: &std::path::Path,
    cancel: &AtomicBool,
) -> Result<(String, Option<String>, Vec<&'static str>), String> {
    let mut export = opencode(home);
    export.args([
        "auth",
        "export",
        crate::providers::opencode::CONSOLE_INTEGRATION,
        "--standalone",
    ]);
    let (status, exported) = bounded_output(export, cancel, CLI_TIMEOUT)?;
    let raw = Some(exported)
        .filter(|_| status.success())
        .and_then(|exported| crate::providers::opencode::exported(&exported))
        .ok_or("OpenCode sign-in did not finish. Start again.")?;
    let mut models = opencode(home);
    models.args(["models", "--standalone"]);
    let (status, listed) = bounded_output(models, cancel, CLI_TIMEOUT)?;
    // A list that could not be read says nothing about the workspace's plans.
    if !status.success() || listed.trim().is_empty() {
        return Err("Signed in to OpenCode, but could not read its workspace's models. Start again.".into());
    }
    let plans = plans_listed(&listed);
    if plans.is_empty() {
        return Err("Signed in to OpenCode, but its workspace offers no Go or Zen models. Start again.".into());
    }
    let label = crate::providers::opencode::account_email(&raw);
    Ok((raw, label, plans))
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct OpencodeAuthStart {
    account_id: Option<i64>,
}

/// Starts `opencode auth login opencode --method device` in a fresh home; once the user approves
/// the code in the OpenCode console, the sign-in is saved to `accountId` when given, otherwise to a
/// new account.
pub(crate) fn start_opencode_auth(request: &Request, state: &SharedState) -> Result<serde_json::Value, String> {
    require_trusted_origin(request)?;
    let target: OpencodeAuthStart = if request.body.iter().all(u8::is_ascii_whitespace) {
        OpencodeAuthStart::default()
    } else {
        serde_json::from_slice(&request.body).map_err(|_| "Invalid sign-in request")?
    };
    let account = target.account_id;
    if let Some(id) = account {
        if with_db_read_raw(state, |db| db.ai_account(id))?.provider != "opencode" {
            return Err("That account is not an OpenCode account".into());
        }
    }
    let db_handle = state.config.db.clone();
    let mut current = flow();
    if current.cancel.is_some() {
        return Ok(serde_json::json!({ "running": true, "flowSecret": current.secret }));
    }
    let home = std::env::temp_dir().join(format!("ocr-opencode-auth-{}", crate::sandbox::secret()?));
    fs::create_dir_all(&home).map_err(|error| error.to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(&home, fs::Permissions::from_mode(0o700));
    }
    let mut child = opencode(&home)
        .args([
            "auth",
            "login",
            crate::providers::opencode::CONSOLE_INTEGRATION,
            "--method",
            "device",
            "--standalone",
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| format!("Could not start the OpenCode CLI: {error}"))?;
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
                if let Some((code, url)) = device_prompt(&text) {
                    prompted = true;
                    flow()
                        .events
                        .push(serde_json::json!({ "kind": "prompt", "url": url, "code": code }));
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
                    let saved = if status.success() {
                        signed_in(&home, &cancel)
                            .and_then(|(raw, label, plans)| store_account(&db_handle, account, &raw, label, &plans))
                    } else {
                        Err("OpenCode sign-in did not finish. Start again.".into())
                    };
                    return finish(match saved {
                        Ok(()) => {
                            serde_json::json!({ "kind": "done", "success": true, "message": "Signed in to OpenCode." })
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

pub(crate) fn opencode_auth_events(request: &Request) -> Result<serde_json::Value, String> {
    let current = flow();
    authorized(request, &current)?;
    Ok(serde_json::json!({ "running": current.cancel.is_some(), "events": current.events }))
}

pub(crate) fn cancel_opencode_auth(request: &Request) -> Result<serde_json::Value, String> {
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
    fn the_key_check_goes_to_a_host_the_review_egress_allows() {
        for (_, url) in KEY_CHECKS {
            assert_eq!(reqwest::Url::parse(url).unwrap().host_str(), Some("opencode.ai"));
        }
        assert!(serde_json::from_str::<OpencodeKey>("{}").is_err());
        assert!(serde_json::from_str::<OpencodeKey>(r#"{"key":"x","model":"opencode/big-pickle"}"#).is_err());
        let request: OpencodeKey = serde_json::from_str(r#"{"key":"x","accountId":3}"#).unwrap();
        assert_eq!(request.account_id, Some(3));
    }

    #[test]
    fn a_key_loses_a_plan_only_when_opencode_refuses_it() {
        assert_eq!(
            accepted_plans(&[("go", Some(200)), ("zen", Some(200))]),
            Ok(vec!["go", "zen"])
        );
        assert_eq!(
            accepted_plans(&[("go", Some(401)), ("zen", Some(200))]),
            Ok(vec!["zen"])
        );
        assert_eq!(
            accepted_plans(&[("go", Some(403)), ("zen", Some(401))]),
            Err("OpenCode rejected that key".into())
        );
        // A check that failed for another reason never saves the key without that plan.
        for failed in [None, Some(500), Some(429)] {
            let error = accepted_plans(&[("go", failed), ("zen", Some(200))]).unwrap_err();
            assert!(error.ends_with("try again"), "{error}");
        }
    }

    #[cfg(unix)]
    #[test]
    fn a_cli_call_after_the_approval_ends_when_cancelled_or_late() {
        let shell = |script: &str| {
            let mut command = Command::new("/bin/sh");
            command.args(["-c", script]).stdin(Stdio::null());
            command
        };
        let running = AtomicBool::new(false);
        let (status, output) = bounded_output(shell("echo listed"), &running, Duration::from_secs(30)).unwrap();
        assert!(status.success());
        assert_eq!(output, "listed\n");
        let started = Instant::now();
        assert_eq!(
            bounded_output(shell("sleep 30"), &running, Duration::from_millis(200)).unwrap_err(),
            "OpenCode did not answer in time. Start again."
        );
        assert_eq!(
            bounded_output(shell("sleep 30"), &AtomicBool::new(true), Duration::from_secs(30)).unwrap_err(),
            "Sign-in was cancelled."
        );
        assert!(started.elapsed() < Duration::from_secs(10));
        // What the CLI started is stopped with it: a child that outlives the kill would still
        // write its marker.
        let marker = std::env::temp_dir().join(format!("ocr-opencode-auth-kill-{}", std::process::id()));
        let _ = std::fs::remove_file(&marker);
        let script = format!("(sleep 1; echo alive > '{}') & wait", marker.display());
        assert!(bounded_output(shell(&script), &running, Duration::from_millis(200)).is_err());
        thread::sleep(Duration::from_millis(1500));
        assert!(!marker.exists());
    }

    #[test]
    fn a_workspace_has_go_only_when_go_models_are_listed() {
        assert_eq!(
            plans_listed("opencode/glm-5.3\nopencode/claude-opus-5-5\n"),
            vec!["zen"]
        );
        assert_eq!(
            plans_listed("opencode-go/glm-5.3\nopencode/glm-5.3\n"),
            vec!["go", "zen"]
        );
        assert!(plans_listed("anthropic/claude-opus-5-5\n").is_empty());
    }

    #[test]
    fn the_device_code_and_page_are_read_from_the_login_output() {
        let text = "┌  Connect an integration\n│\n◇  Authorization started\n│\n●  Enter code: MCTD-DMGG\n│\n●  https://opencode.ai/console/device?user_code=MCTD-DMGG&client_id=opencode-cli\n│\n◇  Waiting for authorization...";
        assert_eq!(
            device_prompt(text),
            Some((
                "MCTD-DMGG".to_string(),
                "https://opencode.ai/console/device?user_code=MCTD-DMGG&client_id=opencode-cli".to_string()
            ))
        );
        assert_eq!(device_prompt("●  Enter code: MCTD-DMGG"), None);
        assert_eq!(
            device_prompt("●  https://evil.example/console/device?user_code=X\nEnter code: X"),
            None
        );
    }
}
