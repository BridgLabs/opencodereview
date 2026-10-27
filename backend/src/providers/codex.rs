//! Codex CLI execution for review stages that run on a Codex (ChatGPT) account.
use serde_json::Value;
use std::ffi::OsString;
use std::path::Path;

use crate::providers::claude::Failure;

/// `auth.json` written by `codex login`; anything larger is not a Codex credential.
pub(crate) const MAX_AUTH_BYTES: usize = 64 * 1024;

/// Features that reach services beyond the model, run other agents, or load local extensions.
const DISABLED_FEATURES: &[&str] = &[
    "apps",
    "browser_use",
    "browser_use_external",
    "browser_use_full_cdp_access",
    "computer_use",
    "image_generation",
    "multi_agent",
    "plugins",
    "remote_plugin",
    "plugin_sharing",
    "skill_mcp_dependency_install",
    "skill_search",
    "tool_suggest",
    "goals",
    "hooks",
    "in_app_browser",
    "daemon_auto_start",
    "workspace_dependencies",
    "worktrees",
    "shell_snapshot",
];

/// Arguments for one non-interactive execution whose prompt arrives on stdin, whose JSONL events
/// go to stdout and whose final message is written to `last_message`.
///
/// Codex's own sandbox needs user namespaces, which the hardened runner container denies, so the
/// container is the sandbox: read-only root, no capabilities, an unprivileged UID and egress
/// limited to the model provider.
pub(crate) fn exec_args(workdir: &Path, last_message: &Path, model: &str, effort: &str) -> Vec<OsString> {
    let mut args: Vec<OsString> = vec!["exec".into()];
    for feature in DISABLED_FEATURES {
        args.extend(["--disable".into(), OsString::from(feature)]);
    }
    args.extend(
        [
            "--json",
            "--color",
            "never",
            "--ephemeral",
            "--ignore-user-config",
            "--ignore-rules",
            "--skip-git-repo-check",
            "--dangerously-bypass-approvals-and-sandbox",
            "-c",
            "web_search=\"disabled\"",
        ]
        .map(OsString::from),
    );
    // `default` leaves the model to Codex.
    if model != "default" {
        args.extend(["-m".into(), OsString::from(model)]);
    }
    args.extend([
        "-c".into(),
        OsString::from(format!("model_reasoning_effort=\"{effort}\"")),
    ]);
    args.extend([
        OsString::from("-C"),
        workdir.as_os_str().to_owned(),
        OsString::from("-o"),
        last_message.as_os_str().to_owned(),
        OsString::from("-"),
    ]);
    args
}

/// Checks that `raw` is a Codex `auth.json` holding a ChatGPT sign-in.
pub(crate) fn validate_auth(raw: &str) -> Result<(), String> {
    if raw.len() > MAX_AUTH_BYTES {
        return Err("Codex authentication is too large".into());
    }
    let value: Value = serde_json::from_str(raw).map_err(|_| "Codex authentication is not valid JSON")?;
    if value["tokens"]["refresh_token"].as_str().is_none_or(str::is_empty) {
        return Err("Codex authentication has no ChatGPT sign-in; sign in again".into());
    }
    Ok(())
}

/// The ChatGPT email in the sign-in's ID token, to name the account.
pub(crate) fn account_email(raw: &str) -> Option<String> {
    use base64::Engine;
    let value: Value = serde_json::from_str(raw).ok()?;
    let payload = value["tokens"]["id_token"].as_str()?.split('.').nth(1)?;
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

/// Credential values in an `auth.json` that must never appear in review output.
pub(crate) fn secrets(raw: &str) -> Vec<String> {
    let Ok(value) = serde_json::from_str::<Value>(raw) else {
        return Vec::new();
    };
    ["access_token", "refresh_token", "id_token"]
        .iter()
        .filter_map(|key| value["tokens"][key].as_str())
        .chain(value["OPENAI_API_KEY"].as_str())
        .filter(|secret| secret.len() >= 16)
        .map(str::to_string)
        .collect()
}

/// Why an execution failed, read only from Codex's own error events and stderr, never from
/// command output, so reviewed code that mentions "usage limit" can't trip it.
pub(crate) fn classify(stdout: &str, stderr: &str) -> Failure {
    let mut text = stdout
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line.trim()).ok())
        .filter_map(|event| match event["type"].as_str() {
            Some("error") => event["message"].as_str().map(str::to_string),
            Some("turn.failed") => event["error"]["message"].as_str().map(str::to_string),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n");
    text.push('\n');
    text.push_str(stderr);
    let lower = text.to_ascii_lowercase();
    if ["usage limit", "usage_limit_reached", "hit your limit"]
        .iter()
        .any(|word| lower.contains(word))
    {
        return Failure::UsageLimit {
            resets_at: retry_after(&lower).map(|millis| crate::db::now_millis() + millis),
        };
    }
    if [
        "401 unauthorized",
        "refresh token",
        "token_expired",
        "not logged in",
        "please log in",
        "sign in again",
        "login again",
    ]
    .iter()
    .any(|word| lower.contains(word))
    {
        return Failure::Auth;
    }
    if ["429", "rate limit", "overloaded", "503", "server_is_overloaded"]
        .iter()
        .any(|word| lower.contains(word))
    {
        return Failure::Overloaded;
    }
    if lower.contains("model")
        && (lower.contains("not found") || lower.contains("not supported") || lower.contains("does not exist"))
    {
        return Failure::Model;
    }
    Failure::Other
}

/// Codex says when a window resets as "try again in 2 hours 13 minutes" (or days/seconds).
fn retry_after(lower: &str) -> Option<i64> {
    let tail = &lower[lower.find("try again in ")? + "try again in ".len()..];
    let words = tail
        .split(|c: char| c.is_whitespace() || c == ',')
        .filter(|word| !word.is_empty())
        .take(8)
        .collect::<Vec<_>>();
    let mut total = 0i64;
    for pair in words.windows(2) {
        let Ok(amount) = pair[0].parse::<i64>() else { continue };
        let unit = pair[1].trim_end_matches(|c: char| !c.is_ascii_alphabetic());
        total += amount
            * match unit.trim_end_matches('s') {
                "day" => 86_400_000,
                "hour" | "hr" => 3_600_000,
                "minute" | "min" => 60_000,
                "second" | "sec" => 1_000,
                _ => 0,
            };
    }
    (total > 0).then_some(total)
}

/// Short, safe diagnostic for a failed process; raw CLI output stays private.
pub(crate) fn failure(stdout: &str, stderr: &str) -> String {
    match classify(stdout, stderr) {
        Failure::Other if stdout.trim().is_empty() && stderr.trim().is_empty() => {
            "Codex process exited without diagnostics; verify runner health and authentication".into()
        }
        Failure::Other => "Codex execution failed; verify authentication".into(),
        Failure::Overloaded => "Codex is temporarily overloaded; retry later".into(),
        Failure::Model => "Codex model is unavailable for this account".into(),
        failure => failure.encode(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exec_args_disable_extensions_and_read_the_prompt_from_stdin() {
        let args = exec_args(Path::new("/work"), Path::new("/scratch/last.txt"), "default", "medium");
        let args = args.iter().map(|arg| arg.to_str().unwrap()).collect::<Vec<_>>();
        assert_eq!(args[0], "exec");
        assert_eq!(args.last(), Some(&"-"));
        for feature in ["apps", "plugins", "multi_agent", "browser_use", "hooks"] {
            assert!(args.windows(2).any(|pair| pair == ["--disable", feature]), "{feature}");
        }
        assert!(args.windows(2).any(|pair| pair == ["-c", "web_search=\"disabled\""]));
        assert!(args.contains(&"--ignore-user-config") && args.contains(&"--ephemeral"));
        assert!(args
            .windows(2)
            .any(|pair| pair == ["-c", "model_reasoning_effort=\"medium\""]));
        assert!(!args.contains(&"-m"), "the default model is Codex's own");
        let args = exec_args(Path::new("/work"), Path::new("/scratch/last.txt"), "gpt-5.5", "high");
        let args = args.iter().map(|arg| arg.to_str().unwrap()).collect::<Vec<_>>();
        assert!(args.windows(2).any(|pair| pair == ["-m", "gpt-5.5"]));
        assert!(args
            .windows(2)
            .any(|pair| pair == ["-c", "model_reasoning_effort=\"high\""]));
        assert_eq!(args.last(), Some(&"-"));
    }

    #[test]
    fn auth_needs_a_chatgpt_sign_in_and_exposes_its_secrets_for_redaction() {
        let auth = r#"{"OPENAI_API_KEY":null,"tokens":{"id_token":"eyJ-fixture-id-token-value","access_token":"eyJ-fixture-access-token","refresh_token":"rt_fixture_refresh_token","account_id":"a"},"last_refresh":"2026-10-24T00:00:00Z"}"#;
        assert!(validate_auth(auth).is_ok());
        assert!(validate_auth(r#"{"tokens":{}}"#).is_err());
        assert!(validate_auth("not json").is_err());
        assert_eq!(secrets(auth).len(), 3);
        assert_eq!(account_email(auth), None);
        let claims = base64::Engine::encode(
            &base64::engine::general_purpose::URL_SAFE_NO_PAD,
            br#"{"email":"dev@example.com"}"#,
        );
        let signed =
            format!(r#"{{"tokens":{{"id_token":"h.{claims}.s","refresh_token":"rt_fixture_refresh_token"}}}}"#);
        assert_eq!(account_email(&signed).as_deref(), Some("dev@example.com"));
    }

    #[test]
    fn failures_come_from_codex_error_events_not_command_output() {
        let command = r#"{"type":"item.completed","item":{"type":"command_execution","aggregated_output":"usage limit 401 Unauthorized"}}"#;
        assert_eq!(classify(command, ""), Failure::Other);
        let limited = r#"{"type":"turn.failed","error":{"message":"You've hit your usage limit. Upgrade to Pro, or try again in 2 hours 13 minutes."}}"#;
        let Failure::UsageLimit { resets_at: Some(at) } = classify(limited, "") else {
            panic!("expected a usage limit with a reset")
        };
        let expected = crate::db::now_millis() + 2 * 3_600_000 + 13 * 60_000;
        assert!((at - expected).abs() < 5_000);
        assert_eq!(
            classify(
                r#"{"type":"error","message":"unexpected status 401 Unauthorized: token expired"}"#,
                ""
            ),
            Failure::Auth
        );
        assert_eq!(classify("", "stream error: 429 Too Many Requests"), Failure::Overloaded);
        assert_eq!(
            classify(
                r#"{"type":"turn.failed","error":{"message":"You've hit your usage limit."}}"#,
                ""
            ),
            Failure::UsageLimit { resets_at: None }
        );
    }
}
