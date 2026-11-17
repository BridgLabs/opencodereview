//! Cursor CLI (`cursor-agent`) execution for review stages that run on a Cursor subscription,
//! signed in through the browser or with a user API key.
use serde_json::{json, Value};
use std::ffi::OsString;

use crate::providers::claude::Failure;

/// `auth.json` this app writes for a Cursor account; anything larger is not one.
pub(crate) const MAX_AUTH_BYTES: usize = 16 * 1024;

/// The variable the CLI reads a user API key from.
pub(crate) const KEY_ENV: &str = "CURSOR_API_KEY";

/// The CLI treats a sign-in as expired when its access token has less than this left, and the
/// browser sign-in is never renewed (only an API key is exchanged again).
const EXPIRY_MARGIN_MS: i64 = 300_000;

/// Where `cursor-agent login` sends the user; nothing else is shown as a sign-in page.
pub(crate) const LOGIN_PAGE: &str = "https://cursor.com/loginDeepControl?";

/// The system-wide hooks file. Cursor gives it priority over team, project and user hooks, and
/// the runner image installs it read-only.
pub(crate) const HOOKS_PATH: &str = "/etc/cursor/hooks.json";

/// Environment that keeps the CLI to its private homes, file credentials and the egress proxy.
pub(crate) const QUIET_ENV: &[(&str, &str)] = &[
    ("AGENT_CLI_CREDENTIAL_STORE", "file"),
    ("NODE_USE_ENV_PROXY", "1"),
    ("NO_OPEN_BROWSER", "1"),
];

/// Arguments for one non-interactive execution: the prompt arrives on stdin and the events go to
/// stdout as JSON lines. Approvals are skipped (`--force`) because the hooks in [`hooks`] are the
/// gate: every tool call runs through `ocr-sandbox tool-guard cursor`, which fails closed. Project
/// configuration, workspace rules, indexing, the CLI's own sandbox (it needs user namespaces,
/// which the runner denies) and updates are off.
pub(crate) fn exec_args(model: &str) -> Vec<OsString> {
    [
        "--print",
        "--output-format",
        "stream-json",
        "--trust",
        "--force",
        "--sandbox",
        "disabled",
        "--disable-auto-update",
        "--disable-project-configs",
        "--disable-indexing",
        "--workspace",
        "/work",
        "--model",
        model,
    ]
    .map(OsString::from)
    .to_vec()
}

/// `cli-config.json` for review stages: no updates, no web search, the CLI's sandbox off, and
/// writes, deletes, web fetches and MCP denied before the hooks even run.
pub(crate) fn cli_config() -> Value {
    json!({
        "version": 1,
        "channel": "static",
        "approvalMode": "allowlist",
        "autoAcceptWebSearch": false,
        "sandbox": { "mode": "disabled" },
        "permissions": {
            "allow": [],
            "deny": ["Write(**)", "Delete(**)", "WebFetch(*)", "Mcp(*:*)"],
        },
    })
}

/// The hooks every Cursor stage runs: each tool call, shell command, file read and MCP call goes
/// through the tool guard, and a guard that fails or times out denies the call.
pub(crate) fn hooks() -> Value {
    let guard =
        || json!([{ "command": "/usr/local/bin/ocr-sandbox tool-guard cursor", "timeout": 10, "failClosed": true }]);
    json!({
        "version": 1,
        "hooks": {
            "preToolUse": guard(),
            "beforeShellExecution": guard(),
            "beforeReadFile": guard(),
            "beforeMCPExecution": guard(),
        },
    })
}

fn token(value: &Value) -> Option<&str> {
    value
        .as_str()
        .filter(|token| (16..=8192).contains(&token.len()) && token.bytes().all(|byte| byte.is_ascii_graphic()))
}

/// A Cursor account holding a user API key.
pub(crate) fn key_auth(key: &str) -> Result<String, String> {
    let raw = json!({ "key": key }).to_string();
    validate_auth(&raw)?;
    Ok(raw)
}

/// A Cursor account holding the browser sign-in the CLI stored in its `auth.json`.
pub(crate) fn login_auth(stored: &str) -> Result<String, String> {
    let stored: Value = serde_json::from_str(stored).map_err(|_| "Cursor sign-in did not finish. Start again.")?;
    let raw = json!({
        "login": {
            "accessToken": stored["accessToken"],
            "refreshToken": stored["refreshToken"],
        }
    })
    .to_string();
    validate_auth(&raw).map_err(|_| "Cursor sign-in did not finish. Start again.".to_string())?;
    Ok(raw)
}

/// Checks that `raw` holds either a plausible Cursor API key or a browser sign-in, and nothing else.
pub(crate) fn validate_auth(raw: &str) -> Result<(), String> {
    if raw.len() > MAX_AUTH_BYTES {
        return Err("Cursor authentication is too large".into());
    }
    let value: Value = serde_json::from_str(raw).map_err(|_| "Cursor authentication is not valid JSON")?;
    let object = value.as_object().ok_or("Cursor authentication is not valid JSON")?;
    if object.len() != 1 {
        return Err("Cursor authentication is not valid JSON".into());
    }
    if let Some(login) = object.get("login") {
        if login.as_object().is_none_or(|fields| fields.len() != 2)
            || token(&login["accessToken"]).is_none()
            || token(&login["refreshToken"]).is_none()
            || claims(login["accessToken"].as_str().unwrap_or_default()).is_none()
        {
            return Err("This is not a Cursor sign-in".into());
        }
        return Ok(());
    }
    let key = object.get("key").and_then(Value::as_str).unwrap_or_default();
    if key.len() < 16 || key.len() > 512 || !key.bytes().all(|byte| byte.is_ascii_graphic()) {
        return Err("Paste the whole Cursor API key".into());
    }
    Ok(())
}

fn parsed(raw: &str) -> Option<Value> {
    validate_auth(raw).ok()?;
    serde_json::from_str(raw).ok()
}

fn claims(jwt: &str) -> Option<Value> {
    use base64::Engine;
    let payload = jwt.split('.').nth(1)?;
    serde_json::from_slice(
        &base64::engine::general_purpose::URL_SAFE_NO_PAD
            .decode(payload.trim_end_matches('='))
            .ok()?,
    )
    .ok()
    .filter(Value::is_object)
}

/// The key of a validated key account.
pub(crate) fn key(raw: &str) -> Option<String> {
    parsed(raw)?["key"].as_str().map(str::to_string)
}

/// The `auth.json` the CLI reads for a validated sign-in account.
pub(crate) fn session_auth(raw: &str) -> Option<String> {
    let login = parsed(raw)?.get("login").cloned()?;
    Some(login.to_string())
}

/// When a browser sign-in stops working, in milliseconds; None for a key, which never expires
/// on its own, or a token without an expiry.
pub(crate) fn expires_at(raw: &str) -> Option<i64> {
    let value = parsed(raw)?;
    let exp = claims(value["login"]["accessToken"].as_str()?)?["exp"].as_i64()?;
    Some(exp.saturating_mul(1000))
}

/// Whether the CLI accepts `raw` at `now`: a key always, a sign-in until it expires.
pub(crate) fn usable(raw: &str, now: i64) -> bool {
    validate_auth(raw).is_ok() && expires_at(raw).is_none_or(|at| at - EXPIRY_MARGIN_MS > now)
}

/// The account's key or tokens, which must never appear in review output.
pub(crate) fn secrets(raw: &str) -> Vec<String> {
    let Some(value) = parsed(raw) else {
        return Vec::new();
    };
    match value.get("login") {
        Some(login) => ["accessToken", "refreshToken"]
            .iter()
            .filter_map(|name| token(&login[*name]).map(str::to_string))
            .collect(),
        None => key(raw).into_iter().collect(),
    }
}

/// The email `cursor-agent about --format json` reports, to name a new account.
pub(crate) fn about_email(output: &str) -> Option<String> {
    let value: Value = serde_json::from_str(output.trim()).ok()?;
    value["userEmail"]
        .as_str()
        .filter(|email| email.contains('@') && email.len() <= 200)
        .map(str::to_string)
}

/// A model `cursor-agent models` lists: `<id> - <name>`, the CLI's default marked.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(crate) struct ListedModel {
    pub id: String,
    pub label: String,
}

/// Whether `model` can be passed to `--model`: a plain slug, without the bracket overrides.
pub(crate) fn model_ok(model: &str) -> bool {
    (1..=120).contains(&model.len())
        && model.bytes().all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b))
        && !model.starts_with(['-', '.'])
}

/// A model name as shown: Cursor pads some with double spaces and zero-width characters.
pub(crate) fn clean_label(name: &str) -> String {
    name.split(|c: char| c.is_whitespace() || c.is_control() || ('\u{200b}'..='\u{200f}').contains(&c))
        .filter(|word| !word.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(80)
        .collect()
}

/// The models of `cursor-agent models` output, the CLI's default first.
pub(crate) fn listed_models(output: &str) -> Vec<ListedModel> {
    let mut models = Vec::new();
    let mut default = None;
    for line in output.lines().map(str::trim) {
        let Some((id, rest)) = line.split_once(" - ") else {
            continue;
        };
        let id = id.trim();
        if !model_ok(id) || models.iter().any(|model: &ListedModel| model.id == id) {
            continue;
        }
        let marked = rest.contains("(default") || rest.contains(", default)");
        let label = clean_label(rest.split(" (").next().unwrap_or(rest));
        if marked && default.is_none() {
            default = Some(models.len());
        }
        models.push(ListedModel {
            id: id.to_string(),
            label: if label.is_empty() { id.to_string() } else { label },
        });
    }
    if let Some(index) = default {
        let model = models.remove(index);
        models.insert(0, model);
    }
    models
}

fn events(transcript: &str) -> impl Iterator<Item = Value> + '_ {
    transcript
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line.trim()).ok())
}

/// The final answer: the text of the last successful `result` event.
pub(crate) fn final_message(transcript: &str) -> Result<String, String> {
    let event = events(transcript)
        .filter(|event| event["type"] == "result")
        .last()
        .ok_or("Cursor final response is missing: no result event")?;
    if event["is_error"] == true || event["subtype"] != "success" {
        return Err(format!(
            "Cursor execution ended without a successful result ({})",
            event["subtype"].as_str().unwrap_or("unknown")
        ));
    }
    event["result"]
        .as_str()
        .map(str::to_string)
        .ok_or_else(|| "Cursor final response has no text".into())
}

/// Why an execution failed, read only from the CLI's stderr and its failed `result` events,
/// never from tool output, so reviewed code that mentions "usage limit" can't trip it.
pub(crate) fn classify(stdout: &str, stderr: &str) -> Failure {
    let mut text = events(stdout)
        .filter(|event| event["type"] == "result" && event["is_error"] == true)
        .filter_map(|event| event["result"].as_str().map(str::to_string))
        .collect::<Vec<_>>()
        .join("\n");
    text.push('\n');
    text.push_str(stderr);
    let lower = text.to_ascii_lowercase();
    if [
        "usage limit",
        "usage_limit",
        "hit your limit",
        "usage_pricing_required",
        "spend limit",
    ]
    .iter()
    .any(|word| lower.contains(word))
    {
        return Failure::UsageLimit { resets_at: None };
    }
    if [
        "authentication required",
        "authentication is invalid",
        "log in again",
        "please log in",
        "agent login",
        "401",
        "unauthenticated",
        "invalid api key",
        "api key is invalid",
    ]
    .iter()
    .any(|word| lower.contains(word))
    {
        return Failure::Auth;
    }
    if [
        "rate_limit",
        "rate limit",
        "rate_limited",
        "resource_exhausted",
        "429",
        "overloaded",
        "503",
    ]
    .iter()
    .any(|word| lower.contains(word))
    {
        return Failure::Overloaded;
    }
    if lower.contains("model")
        && (lower.contains("not found")
            || lower.contains("not available")
            || lower.contains("not supported")
            || lower.contains("invalid model"))
    {
        return Failure::Model;
    }
    Failure::Other
}

/// Short, safe diagnostic for a failed process; raw CLI output stays private.
pub(crate) fn failure(stdout: &str, stderr: &str) -> String {
    match classify(stdout, stderr) {
        Failure::Other if stdout.trim().is_empty() && stderr.trim().is_empty() => {
            "Cursor process exited without diagnostics; verify runner health and authentication".into()
        }
        Failure::Other => "Cursor execution failed; verify authentication".into(),
        Failure::Overloaded => "Cursor is temporarily overloaded; retry later".into(),
        Failure::Model => "Cursor model is unavailable for this account".into(),
        failure => failure.encode(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn jwt(claims: &str) -> String {
        use base64::Engine;
        let encode = |bytes: &[u8]| base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes);
        format!(
            "{}.{}.signature-fixture",
            encode(br#"{"alg":"HS256"}"#),
            encode(claims.as_bytes())
        )
    }

    #[test]
    fn exec_args_skip_project_configuration_updates_and_the_cli_sandbox() {
        let args = exec_args("auto");
        let args = args.iter().map(|arg| arg.to_str().unwrap()).collect::<Vec<_>>();
        assert_eq!(args[0], "--print");
        assert!(args.windows(2).any(|pair| pair == ["--output-format", "stream-json"]));
        assert!(args.windows(2).any(|pair| pair == ["--model", "auto"]));
        assert!(args.windows(2).any(|pair| pair == ["--sandbox", "disabled"]));
        assert!(args.windows(2).any(|pair| pair == ["--workspace", "/work"]));
        for flag in [
            "--trust",
            "--disable-auto-update",
            "--disable-project-configs",
            "--disable-indexing",
        ] {
            assert!(args.contains(&flag), "{flag}");
        }
        // Cursor refuses it for some plans and models, and then never answers.
        assert!(!args.contains(&"--exclude-workspace-context"));
        assert!(!args.contains(&"--approve-mcps"));
    }

    #[test]
    fn every_hook_runs_the_guard_and_fails_closed() {
        let hooks = hooks();
        for event in [
            "preToolUse",
            "beforeShellExecution",
            "beforeReadFile",
            "beforeMCPExecution",
        ] {
            let hook = &hooks["hooks"][event][0];
            assert_eq!(
                hook["command"], "/usr/local/bin/ocr-sandbox tool-guard cursor",
                "{event}"
            );
            assert_eq!(hook["failClosed"], true, "{event}");
        }
        // The runner image installs exactly these hooks.
        let installed: Value = serde_json::from_str(include_str!("../../../docker/cursor-hooks.json")).unwrap();
        assert_eq!(installed, hooks);
        let deny = cli_config()["permissions"]["deny"].clone();
        for rule in ["Write(**)", "WebFetch(*)", "Mcp(*:*)"] {
            assert!(deny.as_array().unwrap().iter().any(|value| value == rule), "{rule}");
        }
    }

    #[test]
    fn a_sign_in_keeps_only_its_tokens_and_expires_with_its_access_token() {
        let access = jwt(r#"{"sub":"user_fixture","exp":2000000000}"#);
        let stored = json!({
            "accessToken": access,
            "refreshToken": "refresh-token-fixture-value",
            "apiKey": null,
            "bedrockCredentials": null,
        })
        .to_string();
        let raw = login_auth(&stored).unwrap();
        assert!(validate_auth(&raw).is_ok());
        assert_eq!(expires_at(&raw), Some(2_000_000_000_000));
        assert!(usable(&raw, 1_900_000_000_000));
        assert!(
            !usable(&raw, 2_000_000_000_000 - 60_000),
            "the CLI needs five minutes left"
        );
        assert_eq!(secrets(&raw).len(), 2);
        let session: Value = serde_json::from_str(&session_auth(&raw).unwrap()).unwrap();
        assert_eq!(session["accessToken"], access);
        assert!(session.get("apiKey").is_none());
        assert_eq!(key(&raw), None);
        assert!(
            login_auth(r#"{"accessToken":"not-a-jwt-but-long-enough","refreshToken":"refresh-token-fixture"}"#)
                .is_err()
        );
        assert!(login_auth("{}").is_err());
    }

    #[test]
    fn a_key_never_expires_and_is_its_own_secret() {
        let raw = key_auth("key_0123456789abcdef0123").unwrap();
        assert_eq!(key(&raw).as_deref(), Some("key_0123456789abcdef0123"));
        assert_eq!(expires_at(&raw), None);
        assert!(usable(&raw, i64::MAX / 2));
        assert_eq!(secrets(&raw), vec!["key_0123456789abcdef0123".to_string()]);
        assert_eq!(session_auth(&raw), None);
        assert!(key_auth("short").is_err());
        assert!(validate_auth(r#"{"key":"key_0123456789abcdef0123","login":{}}"#).is_err());
    }

    #[test]
    fn models_come_from_the_cli_list_with_its_default_first() {
        let output = "Available models\n\ngpt-5.5 - GPT-5.5\nauto - Auto  (current, default)\nclaude-opus-4-8-thinking - Claude Opus 4.8 Thinking\nbad model - Nope\n../x - Path\n\nTip: use --model <id> to switch.";
        let models = listed_models(output);
        assert_eq!(
            models.iter().map(|model| model.id.as_str()).collect::<Vec<_>>(),
            vec!["auto", "gpt-5.5", "claude-opus-4-8-thinking"]
        );
        assert_eq!(models[0].label, "Auto");
        assert_eq!(models[2].label, "Claude Opus 4.8 Thinking");
        assert!(listed_models("Error: Authentication required.").is_empty());
        let padded = listed_models("grok-4.7-low-fast - Grok 4.7  Low Fast\u{200b}\u{200b}");
        assert_eq!(padded[0].label, "Grok 4.7 Low Fast");
        assert!(!model_ok("gpt[effort=high]") && !model_ok("-rf") && model_ok("gpt-5.5"));
        assert_eq!(
            about_email(r#"{"userEmail":"dev@example.com","subscriptionTier":"pro"}"#).as_deref(),
            Some("dev@example.com")
        );
    }

    #[test]
    fn the_final_answer_is_the_last_successful_result() {
        let ok = "{\"type\":\"system\",\"subtype\":\"init\"}\n{\"type\":\"result\",\"subtype\":\"success\",\"is_error\":false,\"result\":\"{}\",\"usage\":{\"inputTokens\":5}}\n";
        assert_eq!(final_message(ok).unwrap(), "{}");
        assert!(final_message("{\"type\":\"assistant\"}").is_err());
        assert!(final_message("{\"type\":\"result\",\"subtype\":\"error\",\"is_error\":true}").is_err());
    }

    #[test]
    fn failures_come_from_stderr_and_failed_results_not_tool_output() {
        let tool =
            r#"{"type":"tool_call","subtype":"completed","tool_call":{"shellToolCall":{"result":"usage limit 401"}}}"#;
        assert_eq!(classify(tool, ""), Failure::Other);
        assert_eq!(
            classify("", "Error: You've hit your usage limit for Pro."),
            Failure::UsageLimit { resets_at: None }
        );
        assert_eq!(
            classify(
                "",
                "Error: Authentication required. Please run 'agent login' first, or set CURSOR_API_KEY environment variable."
            ),
            Failure::Auth
        );
        assert_eq!(
            classify(
                "",
                "Warning: The provided API key is invalid.\nThe API key was loaded from the CURSOR_API_KEY environment variable."
            ),
            Failure::Auth
        );
        assert_eq!(
            classify("", "ConnectError: [resource_exhausted] RATE_LIMITED"),
            Failure::Overloaded
        );
        assert_eq!(
            failure("", ""),
            "Cursor process exited without diagnostics; verify runner health and authentication"
        );
    }
}
