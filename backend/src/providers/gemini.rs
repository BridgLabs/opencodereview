//! Gemini CLI execution for review stages that run on a Gemini (Google account) sign-in.
use serde_json::{json, Value};
use std::ffi::OsString;
use std::path::Path;

use crate::providers::claude::Failure;

/// `oauth_creds.json` written by the Gemini CLI's Google sign-in; anything larger is not one.
pub(crate) const MAX_AUTH_BYTES: usize = 64 * 1024;

/// Settings for a private Gemini home: the Google sign-in, and nothing that phones home,
/// updates itself or checkpoints the workspace.
pub(crate) fn settings() -> Value {
    json!({
        "general": { "enableAutoUpdate": false, "enableAutoUpdateNotification": false, "checkpointing": { "enabled": false } },
        "privacy": { "usageStatisticsEnabled": false },
        "telemetry": { "enabled": false },
        "security": { "auth": { "selectedType": "oauth-personal" }, "folderTrust": { "enabled": false } },
    })
}

/// Policy for review stages: no web search or fetch (search runs on Google's side and would get
/// past the egress proxy), no subagents and no skills.
pub(crate) const POLICY: &str = r#"[[rule]]
toolName = ["google_web_search", "web_fetch", "invoke_agent", "activate_skill", "get_internal_docs"]
decision = "deny"
priority = 999
"#;

/// Arguments for one non-interactive execution: the prompt arrives on stdin, which makes the CLI
/// headless, and its events go to stdout as JSON lines.
///
/// Like Codex, the hardened runner container is the sandbox: read-only root, no capabilities,
/// an unprivileged UID and egress limited to the model provider.
pub(crate) fn exec_args(policy: &Path, model: &str) -> Vec<OsString> {
    let mut args: Vec<OsString> = [
        "--output-format",
        "stream-json",
        "--approval-mode",
        "yolo",
        "--skip-trust",
        "--include-directories",
        "/evidence/snapshot",
        "--policy",
    ]
    .map(OsString::from)
    .to_vec();
    args.push(policy.as_os_str().to_owned());
    args.extend(["--model".into(), OsString::from(model)]);
    args
}

/// Checks that `raw` is an `oauth_creds.json` holding a Google sign-in.
pub(crate) fn validate_auth(raw: &str) -> Result<(), String> {
    if raw.len() > MAX_AUTH_BYTES {
        return Err("Gemini authentication is too large".into());
    }
    let value: Value = serde_json::from_str(raw).map_err(|_| "Gemini authentication is not valid JSON")?;
    if value["refresh_token"].as_str().is_none_or(str::is_empty) {
        return Err("Gemini authentication has no Google sign-in; sign in again".into());
    }
    Ok(())
}

/// The Google email in the sign-in's ID token, to name the account.
pub(crate) fn account_email(raw: &str) -> Option<String> {
    use base64::Engine;
    let value: Value = serde_json::from_str(raw).ok()?;
    let payload = value["id_token"].as_str()?.split('.').nth(1)?;
    let claims: Value = serde_json::from_slice(
        &base64::engine::general_purpose::URL_SAFE_NO_PAD
            .decode(payload.trim_end_matches('='))
            .ok()?,
    )
    .ok()?;
    claims["email"]
        .as_str()
        .filter(|email| email.contains('@') && email.len() <= 80)
        .map(str::to_string)
}

/// Credential values in an `oauth_creds.json` that must never appear in review output.
pub(crate) fn secrets(raw: &str) -> Vec<String> {
    let Ok(value) = serde_json::from_str::<Value>(raw) else {
        return Vec::new();
    };
    ["access_token", "refresh_token", "id_token"]
        .iter()
        .filter_map(|key| value[key].as_str())
        .filter(|secret| secret.len() >= 16)
        .map(str::to_string)
        .collect()
}

/// The final answer in a stream-json transcript: the assistant text after the last tool call.
pub(crate) fn final_message(transcript: &str) -> String {
    let mut message = String::new();
    for event in transcript
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line.trim()).ok())
    {
        match event["type"].as_str() {
            Some("tool_use" | "tool_result") => message.clear(),
            Some("message") if event["role"] == "assistant" => {
                if event["delta"] != true {
                    message.clear();
                }
                message.push_str(event["content"].as_str().unwrap_or_default());
            }
            _ => {}
        }
    }
    message
}

/// Why an execution failed, read only from Gemini's own error events and stderr, never from
/// tool output, so reviewed code that mentions "quota" can't trip it.
pub(crate) fn classify(stdout: &str, stderr: &str) -> Failure {
    let mut text = stdout
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line.trim()).ok())
        .filter(|event| event["type"] == "error")
        .filter_map(|event| event["message"].as_str().map(str::to_string))
        .collect::<Vec<_>>()
        .join("\n");
    text.push('\n');
    text.push_str(stderr);
    let lower = text.to_ascii_lowercase();
    // "Capacity" is Google's shared capacity, not this account's quota.
    if [
        "exhausted your capacity",
        "capacity exceeded",
        "model_capacity_exhausted",
        "unavailable",
        "overloaded",
    ]
    .iter()
    .any(|word| lower.contains(word))
        || status(&lower, "503")
    {
        return Failure::Overloaded;
    }
    if [
        "exhausted your daily quota",
        "quota exceeded",
        "resource_exhausted",
        "terminalquotaerror",
        "usage limit",
    ]
    .iter()
    .any(|word| lower.contains(word))
    {
        return Failure::UsageLimit {
            resets_at: retry_after(&lower).map(|millis| crate::db::now_millis() + millis),
        };
    }
    if [
        "manual authorization is required",
        "error authenticating",
        "invalid_grant",
        "unauthenticated",
        "sign in again",
    ]
    .iter()
    .any(|word| lower.contains(word))
        || status(&lower, "401")
    {
        return Failure::Auth;
    }
    if lower.contains("rate limit") || status(&lower, "429") {
        return Failure::Overloaded;
    }
    if lower.contains("model") && (lower.contains("not found") || lower.contains("not supported")) {
        return Failure::Model;
    }
    Failure::Other
}

/// Whether `code` appears as a whole number, so "retry in 1503s" is not a 503.
fn status(lower: &str, code: &str) -> bool {
    lower.match_indices(code).any(|(at, _)| {
        let before = lower[..at].chars().next_back();
        let after = lower[at + code.len()..].chars().next();
        !before.is_some_and(|c| c.is_ascii_alphanumeric()) && !after.is_some_and(|c| c.is_ascii_alphanumeric())
    })
}

/// Google says when to come back as "Please retry in 3600s" (or `ms`).
fn retry_after(lower: &str) -> Option<i64> {
    let tail = &lower[lower.find("retry in ")? + "retry in ".len()..];
    let number = tail
        .chars()
        .take_while(|c| c.is_ascii_digit() || *c == '.')
        .collect::<String>();
    let amount = number.parse::<f64>().ok()?;
    let millis = if tail[number.len()..].starts_with("ms") {
        amount
    } else if tail[number.len()..].starts_with('s') {
        amount * 1000.0
    } else {
        return None;
    };
    (millis >= 1.0).then_some(millis as i64)
}

/// Short, safe diagnostic for a failed process; raw CLI output stays private.
pub(crate) fn failure(stdout: &str, stderr: &str) -> String {
    match classify(stdout, stderr) {
        Failure::Other if stdout.trim().is_empty() && stderr.trim().is_empty() => {
            "Gemini process exited without diagnostics; verify runner health and authentication".into()
        }
        Failure::Other => "Gemini execution failed; verify authentication".into(),
        Failure::Overloaded => "Gemini is temporarily overloaded; retry later".into(),
        Failure::Model => "Gemini model is unavailable for this account".into(),
        failure => failure.encode(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exec_args_stream_events_and_deny_web_tools() {
        let args = exec_args(Path::new("/scratch/session/gemini/policy.toml"), "flash");
        let args = args.iter().map(|arg| arg.to_str().unwrap()).collect::<Vec<_>>();
        assert!(args.windows(2).any(|pair| pair == ["--output-format", "stream-json"]));
        assert!(args.windows(2).any(|pair| pair == ["--approval-mode", "yolo"]));
        assert!(args
            .windows(2)
            .any(|pair| pair == ["--policy", "/scratch/session/gemini/policy.toml"]));
        assert!(!args.contains(&"-p"), "the prompt comes from stdin");
        assert!(args.windows(2).any(|pair| pair == ["--model", "flash"]));
        assert!(POLICY.contains("google_web_search") && POLICY.contains("web_fetch") && POLICY.contains("\"deny\""));
        assert_eq!(settings()["security"]["auth"]["selectedType"], "oauth-personal");
        assert_eq!(settings()["privacy"]["usageStatisticsEnabled"], false);
    }

    #[test]
    fn auth_needs_a_google_sign_in_and_exposes_its_secrets_for_redaction() {
        let auth = r#"{"access_token":"ya29.fixture-access-token","refresh_token":"1//fixture-refresh-token","scope":"openid","token_type":"Bearer","expiry_date":1}"#;
        assert!(validate_auth(auth).is_ok());
        assert!(validate_auth(r#"{"access_token":"ya29.fixture-access-token"}"#).is_err());
        assert!(validate_auth("not json").is_err());
        assert_eq!(secrets(auth).len(), 2);
        assert_eq!(account_email(auth), None);
        let claims = base64::Engine::encode(
            &base64::engine::general_purpose::URL_SAFE_NO_PAD,
            br#"{"email":"dev@example.com"}"#,
        );
        let signed = format!(r#"{{"id_token":"h.{claims}.s","refresh_token":"1//fixture-refresh-token"}}"#);
        assert_eq!(account_email(&signed).as_deref(), Some("dev@example.com"));
    }

    #[test]
    fn the_final_message_is_the_text_after_the_last_tool_call() {
        let transcript = [
            r#"{"type":"init","model":"auto"}"#,
            r#"{"type":"message","role":"user","content":"review"}"#,
            r#"{"type":"message","role":"assistant","content":"Let me read","delta":true}"#,
            r#"{"type":"tool_use","tool_name":"read_file"}"#,
            r#"{"type":"tool_result","status":"success"}"#,
            r#"{"type":"message","role":"assistant","content":"{\"report\":","delta":true}"#,
            r#"{"type":"message","role":"assistant","content":"{}}","delta":true}"#,
            r#"{"type":"result","status":"success","stats":{}}"#,
        ]
        .join("\n");
        assert_eq!(final_message(&transcript), r#"{"report":{}}"#);
    }

    #[test]
    fn failures_come_from_gemini_error_events_not_tool_output() {
        let tool = r#"{"type":"tool_result","output":"quota exceeded 401 unauthenticated"}"#;
        assert_eq!(classify(tool, ""), Failure::Other);
        let daily =
            r#"{"type":"error","severity":"error","message":"You have exhausted your daily quota on this model."}"#;
        assert_eq!(classify(daily, ""), Failure::UsageLimit { resets_at: None });
        let Failure::UsageLimit { resets_at: Some(at) } =
            classify("", "Quota exceeded for quota metric. Please retry in 3600s.")
        else {
            panic!("expected a usage limit with a reset")
        };
        assert!((at - (crate::db::now_millis() + 3_600_000)).abs() < 5_000);
        assert_eq!(
            classify(
                "",
                "Manual authorization is required but the current session is non-interactive."
            ),
            Failure::Auth
        );
        assert_eq!(
            classify(
                r#"{"type":"error","severity":"error","message":"You have exhausted your capacity on this model."}"#,
                ""
            ),
            Failure::Overloaded
        );
        assert_eq!(classify("", "Model not found: gemini-x"), Failure::Model);
        assert_eq!(
            classify("", "[API Error: got status 503 Service Unavailable]"),
            Failure::Overloaded
        );
        assert!(matches!(
            classify("", "Quota exceeded. Please retry in 1503s."),
            Failure::UsageLimit { resets_at: Some(_) }
        ));
    }
}
