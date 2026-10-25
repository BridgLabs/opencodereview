use serde_json::{json, Value};
use std::ffi::OsString;
use std::path::Path;
use std::process::Command;

/// The default model and reasoning effort; Settings can pick others from `engines`.
pub const MODEL: &str = "claude-opus-5-5";
pub const EFFORT: &str = "medium";

/// Built-in tools every isolated stage may call; the PreToolUse guard narrows each call further.
pub(crate) const RUNNER_TOOLS: &str = "Read,Grep,Glob,Bash,Write";
pub(crate) const MAX_TOKEN_BYTES: usize = 4096;

/// Arguments for one non-interactive execution whose prompt arrives on stdin and whose
/// stream-json events are written to stdout.
pub(crate) fn print_args(settings: &Path, tools: &str, model: &str, effort: &str) -> Vec<OsString> {
    let mut args: Vec<OsString> = [
        "--print",
        "--model",
        model,
        "--effort",
        effort,
        "--output-format",
        "stream-json",
        "--verbose",
        "--setting-sources",
        "user",
        "--strict-mcp-config",
        "--no-session-persistence",
        "--disable-slash-commands",
        "--permission-mode",
        "dontAsk",
        "--tools",
        tools,
    ]
    .into_iter()
    .map(Into::into)
    .collect();
    args.extend([OsString::from("--settings"), settings.as_os_str().to_owned()]);
    args
}

/// Removes ambient configuration and non-essential traffic from a CLI process.
pub(crate) fn isolate(command: &mut Command, home: &Path, token: &str) {
    let state = home.join(".claude");
    for (name, directory) in [
        ("XDG_CONFIG_HOME", "config"),
        ("XDG_CACHE_HOME", "cache"),
        ("XDG_DATA_HOME", "data"),
        ("XDG_STATE_HOME", "state"),
    ] {
        command.env(name, state.join(directory));
    }
    command
        .env("HOME", home)
        .env("CLAUDE_CONFIG_DIR", &state)
        .env("CLAUDE_CODE_OAUTH_TOKEN", token)
        .env("DISABLE_AUTOUPDATER", "1")
        .env("DISABLE_TELEMETRY", "1")
        .env("DISABLE_ERROR_REPORTING", "1")
        .env("DISABLE_BUG_COMMAND", "1")
        .env("CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC", "1")
        .env("CLAUDE_CODE_DISABLE_FEEDBACK_SURVEY", "1")
        .env("CLAUDE_CODE_DISABLE_TERMINAL_TITLE", "1");
}

/// Location of the subscription token owned by the app; runners receive projected copies.
pub(crate) fn app_token_path() -> std::path::PathBuf {
    std::env::var_os("OCR_CLAUDE_TOKEN_PATH")
        .map(Into::into)
        .unwrap_or_else(|| {
            std::env::var_os("OCR_DATA_DIR")
                .map(std::path::PathBuf::from)
                .unwrap_or_else(|| "data".into())
                .join("claude/oauth-token")
        })
}

/// Command for a local (non-runner) execution, authenticated with the app token.
pub(crate) fn local_command(home: &Path) -> Result<Command, String> {
    let token = read_token(&app_token_path())?;
    let mut command = Command::new("claude");
    std::fs::create_dir_all(home.join(".claude")).map_err(|error| error.to_string())?;
    isolate(&mut command, home, &token);
    Ok(command)
}

pub(crate) fn read_token(path: &Path) -> Result<String, String> {
    let raw = std::fs::read_to_string(path)
        .map_err(|_| "Claude authentication is unavailable; sign in from Settings → Integrations")?;
    let token = raw.trim();
    if token.is_empty() || token.len() > MAX_TOKEN_BYTES || !token.bytes().all(|byte| byte.is_ascii_graphic()) {
        return Err("Claude authentication token is invalid; sign in again".into());
    }
    Ok(token.into())
}

/// Settings for frozen review stages: subagents, web access and MCP stay unavailable,
/// only snapshot reads and the report files are permitted, and every tool call passes the guard.
pub(crate) fn runner_settings(guard: &str, extra_hooks: Vec<Value>) -> Value {
    let mut pre_tool_use = vec![json!({"matcher":"*","hooks":[{"type":"command","command":guard,"timeout":5}]})];
    pre_tool_use.extend(extra_hooks);
    json!({
        "permissions": {
            "defaultMode": "dontAsk",
            "deny": ["Agent", "Task", "WebFetch", "WebSearch", "mcp__*", "Read(//scratch/**)", "Read(//claude-auth/**)", "Read(//ipc/**)", "Read(//proc/**)", "Edit(//work/**)", "Edit(//evidence/**)", "Write(//work/**)", "Write(//evidence/**)"],
            "allow": ["Bash", "Grep", "Glob", "Read(//work/**)", "Read(//evidence/**)", "Read(//output/**)", "Write(//output/report.json)", "Write(//output/finding_decisions.json)"]
        },
        "hooks": {"PreToolUse": pre_tool_use},
        "enableAllProjectMcpServers": false,
        "includeCoAuthoredBy": false,
        "cleanupPeriodDays": 1
    })
}

/// Final `result` event of a stream-json transcript.
pub(crate) fn final_result(transcript: &str) -> Result<String, String> {
    let event = last_result(transcript).ok_or("Claude final response is missing: no result event")?;
    if event["is_error"] == true || event["subtype"] != "success" {
        return Err(match classify(transcript, "") {
            Failure::Other => format!(
                "Claude execution ended without a successful result ({})",
                event["subtype"].as_str().unwrap_or("unknown")
            ),
            failure => failure.encode(),
        });
    }
    event["result"]
        .as_str()
        .map(str::to_string)
        .ok_or_else(|| "Claude final response has no text".into())
}

fn last_result(transcript: &str) -> Option<Value> {
    transcript
        .lines()
        .rev()
        .filter_map(|line| serde_json::from_str::<Value>(line.trim()).ok())
        .find(|event| event["type"] == "result")
}

/// Why an execution failed, read only from the CLI's own result/rate-limit events and stderr,
/// never from tool output, so reviewed code that mentions "429" or "quota" can't trip it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Failure {
    /// The account's usage window is exhausted; `resets_at` is epoch milliseconds when known.
    UsageLimit {
        resets_at: Option<i64>,
    },
    /// Temporary provider overload; the same account can retry later.
    Overloaded,
    Auth,
    Model,
    Other,
}

const USAGE_LIMIT: &str = "usage_limit";
const AUTH_FAILED: &str = "auth_failed";

impl Failure {
    /// Stable `code:detail|message` form carried in runner errors; [`decode`] reads it back.
    pub(crate) fn encode(&self) -> String {
        match self {
            Failure::UsageLimit { resets_at } => format!(
                "{USAGE_LIMIT}:{}|Usage limit reached for this account",
                resets_at.unwrap_or(0)
            ),
            Failure::Auth => {
                format!("{AUTH_FAILED}:0|Sign-in failed for this account; sign in again from Settings → Accounts")
            }
            Failure::Overloaded => "Claude is temporarily overloaded; retry later".into(),
            Failure::Model => "Claude model is unavailable for this account".into(),
            Failure::Other => "Claude execution failed; verify model and authentication".into(),
        }
    }
}

/// Splits an encoded runner error into its failure kind and the human message.
pub(crate) fn decode(error: &str) -> (Failure, &str) {
    let Some((head, message)) = error.split_once('|') else {
        return (Failure::Other, error);
    };
    let Some((code, detail)) = head.split_once(':') else {
        return (Failure::Other, error);
    };
    let at = detail.parse::<i64>().ok().filter(|value| *value > 0);
    match code {
        USAGE_LIMIT => (Failure::UsageLimit { resets_at: at }, message),
        AUTH_FAILED => (Failure::Auth, message),
        _ => (Failure::Other, error),
    }
}

pub(crate) fn classify(stdout: &str, stderr: &str) -> Failure {
    let events = stdout
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line.trim()).ok())
        .filter(|event| matches!(event["type"].as_str(), Some("result" | "rate_limit_event")))
        .collect::<Vec<_>>();
    if let Some(info) = events
        .iter()
        .rev()
        .find(|event| event["type"] == "rate_limit_event")
        .map(|event| &event["rate_limit_info"])
    {
        if info["status"] == "rejected" {
            return Failure::UsageLimit {
                resets_at: epoch_millis(&info["resetsAt"]),
            };
        }
    }
    let result = events.iter().rev().find(|event| event["type"] == "result");
    let text = match result.filter(|event| event["is_error"] == true || event["subtype"] != "success") {
        Some(event) => format!("{}\n{stderr}", event["result"].as_str().unwrap_or_default()),
        None => stderr.to_string(),
    };
    let lower = text.to_ascii_lowercase();
    if ["usage limit", "hit your limit", "limit reached", "out of extra usage"]
        .iter()
        .any(|word| lower.contains(word))
    {
        return Failure::UsageLimit {
            resets_at: reset_marker(&text),
        };
    }
    if [
        "invalid api key",
        "oauth token",
        "401",
        "authentication",
        "please run /login",
        "not logged in",
    ]
    .iter()
    .any(|word| lower.contains(word))
    {
        return Failure::Auth;
    }
    if ["rate limit", "429", "overloaded", "529"]
        .iter()
        .any(|word| lower.contains(word))
    {
        return Failure::Overloaded;
    }
    if lower.contains("model") && (lower.contains("not found") || lower.contains("invalid")) {
        return Failure::Model;
    }
    Failure::Other
}

fn epoch_millis(value: &Value) -> Option<i64> {
    let raw = value
        .as_i64()
        .or_else(|| value.as_str().and_then(|text| text.parse().ok()))?;
    Some(if raw < 100_000_000_000 { raw * 1000 } else { raw })
}

/// Claude reports exhausted windows as `…limit reached|<epoch seconds>`.
fn reset_marker(text: &str) -> Option<i64> {
    text.split('|')
        .nth(1)
        .and_then(|tail| tail.trim().split(|c: char| !c.is_ascii_digit()).next())
        .and_then(|digits| epoch_millis(&Value::String(digits.into())))
}

/// Short, safe diagnostic for a failed process; raw CLI output stays private.
pub(crate) fn failure(stdout: &str, stderr: &str) -> String {
    match classify(stdout, stderr) {
        Failure::Other if stdout.trim().is_empty() && stderr.trim().is_empty() => {
            "Claude process exited without diagnostics; verify runner health and authentication".into()
        }
        failure => failure.encode(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn print_args_pass_the_model_and_never_enable_session_state() {
        let args = print_args(Path::new("/scratch/settings.json"), RUNNER_TOOLS, MODEL, EFFORT);
        let args = args.iter().map(|arg| arg.to_str().unwrap()).collect::<Vec<_>>();
        assert!(args.windows(2).any(|pair| pair == ["--model", "claude-opus-5-5"]));
        assert!(args.windows(2).any(|pair| pair == ["--effort", "medium"]));
        assert!(args.windows(2).any(|pair| pair == ["--permission-mode", "dontAsk"]));
        assert!(args.contains(&"--no-session-persistence") && args.contains(&"--strict-mcp-config"));
    }

    #[test]
    fn final_result_reads_only_the_last_successful_result_event() {
        let ok = "{\"type\":\"system\",\"subtype\":\"init\"}\n{\"type\":\"assistant\",\"message\":{}}\n{\"type\":\"result\",\"subtype\":\"success\",\"is_error\":false,\"result\":\"{}\"}\n";
        assert_eq!(final_result(ok).unwrap(), "{}");
        assert!(final_result("{\"type\":\"result\",\"subtype\":\"error_max_turns\",\"is_error\":true}").is_err());
        assert!(final_result("{\"type\":\"assistant\"}").is_err());
    }

    #[test]
    fn runner_settings_guard_every_tool_and_deny_delegation() {
        let settings = runner_settings("guard || exit 2", vec![]);
        assert_eq!(settings["hooks"]["PreToolUse"][0]["matcher"], "*");
        let deny = settings["permissions"]["deny"].as_array().unwrap();
        assert!(["Agent", "Task", "WebFetch"]
            .iter()
            .all(|tool| deny.contains(&json!(tool))));
    }

    #[test]
    fn tokens_must_be_bounded_printable_text() {
        let root = std::env::temp_dir().join(format!("ocr-claude-token-{}", std::process::id()));
        std::fs::write(&root, "  sk-ant-oat01-fixture\n").unwrap();
        assert_eq!(read_token(&root).unwrap(), "sk-ant-oat01-fixture");
        std::fs::write(&root, "two words").unwrap();
        assert!(read_token(&root).is_err());
        std::fs::remove_file(root).unwrap();
    }

    #[test]
    fn failures_come_from_cli_events_not_tool_output() {
        let tool = "{\"type\":\"user\",\"message\":{\"content\":\"quota 429 oauth token\"}}\n";
        assert_eq!(
            classify(
                &format!(
                    "{tool}{{\"type\":\"result\",\"subtype\":\"success\",\"is_error\":false,\"result\":\"{{}}\"}}"
                ),
                ""
            ),
            Failure::Other
        );
        let limited = "{\"type\":\"result\",\"subtype\":\"success\",\"is_error\":true,\"result\":\"Claude AI usage limit reached|1761300000\"}";
        assert_eq!(
            classify(limited, ""),
            Failure::UsageLimit {
                resets_at: Some(1_761_300_000_000)
            }
        );
        let event =
            "{\"type\":\"rate_limit_event\",\"rate_limit_info\":{\"status\":\"rejected\",\"resetsAt\":1761300000}}";
        assert_eq!(
            classify(event, ""),
            Failure::UsageLimit {
                resets_at: Some(1_761_300_000_000)
            }
        );
        assert_eq!(classify("", "Invalid API key · Please run /login"), Failure::Auth);
        assert_eq!(classify("", "API Error: 529 overloaded"), Failure::Overloaded);
    }

    #[test]
    fn encoded_failures_round_trip() {
        let encoded = Failure::UsageLimit { resets_at: Some(42) }.encode();
        let (kind, message) = decode(&encoded);
        assert_eq!(kind, Failure::UsageLimit { resets_at: Some(42) });
        assert!(message.starts_with("Usage limit reached"));
        assert_eq!(decode(&Failure::Auth.encode()).0, Failure::Auth);
        assert_eq!(decode("Runner unavailable").0, Failure::Other);
        assert_eq!(
            decode(&Failure::UsageLimit { resets_at: None }.encode()).0,
            Failure::UsageLimit { resets_at: None }
        );
    }
}
