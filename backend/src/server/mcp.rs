//! Read-only MCP server (Streamable HTTP, JSON responses) that lets a coding agent
//! ask how an OpenCodeReview review of a pull request is going instead of polling GitHub.

use super::*;
use crate::db::{ReviewRun, ReviewStatusPullRequest};
use serde_json::{json, Value};

pub(crate) const MCP_PATH: &str = "/mcp";
const SUPPORTED_PROTOCOL_VERSIONS: [&str; 4] = ["2025-11-25", "2025-06-18", "2025-03-26", "2024-11-05"];
const MAX_WAIT_SECONDS: u64 = 120;
const WAIT_POLL_INTERVAL: Duration = Duration::from_secs(2);
const ACTIVE_REVIEW_LIMIT: i64 = 50;
/// How long an agent should wait before asking again about a review still in progress.
const RETRY_AFTER_SECONDS: u64 = 30;

const PARSE_ERROR: i64 = -32700;
const INVALID_REQUEST: i64 = -32600;
const METHOD_NOT_FOUND: i64 = -32601;
const INVALID_PARAMS: i64 = -32602;

pub(crate) fn handle_mcp(request: &Request, state: &SharedState) -> Response {
    if request.method != "POST" {
        return json_error(405, "the MCP endpoint accepts only POST");
    }
    let message: Value = match serde_json::from_slice(&request.body) {
        Ok(message) => message,
        Err(error) => return json_ok(&rpc_error(Value::Null, PARSE_ERROR, &error.to_string())),
    };
    match dispatch(&message, &|name, arguments| call_tool(state, name, arguments)) {
        Some(reply) => json_ok(&reply),
        None => Response {
            status: 202,
            content_type: "application/json",
            body: Vec::new(),
        },
    }
}

type ToolCall<'a> = dyn Fn(&str, &Value) -> Result<Value, String> + 'a;

/// Answers one JSON-RPC message; `None` for notifications and responses, which get no reply.
pub(crate) fn dispatch(message: &Value, call: &ToolCall<'_>) -> Option<Value> {
    let Some(object) = message.as_object() else {
        return Some(rpc_error(Value::Null, INVALID_REQUEST, "expected one JSON-RPC message"));
    };
    let id = object.get("id").cloned()?;
    let method = object.get("method").and_then(Value::as_str)?;
    let params = object.get("params").cloned().unwrap_or(Value::Null);
    let result = match method {
        "initialize" => Ok(initialize_result(&params)),
        "ping" => Ok(json!({})),
        "tools/list" => Ok(json!({ "tools": tool_definitions() })),
        "tools/call" => {
            let Some(name) = params.get("name").and_then(Value::as_str) else {
                return Some(rpc_error(id, INVALID_PARAMS, "tools/call needs a tool name"));
            };
            if !tool_definitions().iter().any(|tool| tool["name"] == name) {
                return Some(rpc_error(id, INVALID_PARAMS, &format!("unknown tool: {name}")));
            }
            let arguments = params.get("arguments").cloned().unwrap_or_else(|| json!({}));
            Ok(match call(name, &arguments) {
                Ok(value) => tool_result(value, false),
                Err(error) => tool_result(json!({ "error": error }), true),
            })
        }
        _ => Err((METHOD_NOT_FOUND, format!("method not found: {method}"))),
    };
    Some(match result {
        Ok(result) => json!({ "jsonrpc": "2.0", "id": id, "result": result }),
        Err((code, message)) => rpc_error(id, code, &message),
    })
}

fn rpc_error(id: Value, code: i64, message: &str) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } })
}

fn initialize_result(params: &Value) -> Value {
    let requested = params.get("protocolVersion").and_then(Value::as_str);
    let version = requested
        .filter(|version| SUPPORTED_PROTOCOL_VERSIONS.contains(version))
        .unwrap_or(SUPPORTED_PROTOCOL_VERSIONS[0]);
    json!({
        "protocolVersion": version,
        "capabilities": { "tools": { "listChanged": false } },
        "serverInfo": { "name": "opencodereview", "version": env!("CARGO_PKG_VERSION") },
        "instructions": "Ask OpenCodeReview how its review of a pull request is going. Call get_review_status with the pull request instead of polling GitHub for the review; pass waitSeconds to wait until the review finishes. `done: true` means nothing more will happen until the pull request changes.",
    })
}

fn tool_result(value: Value, is_error: bool) -> Value {
    let text = serde_json::to_string_pretty(&value).unwrap_or_default();
    let mut result = json!({ "content": [{ "type": "text", "text": text }], "isError": is_error });
    if !is_error {
        result["structuredContent"] = value;
    }
    result
}

fn tool_definitions() -> Vec<Value> {
    vec![
        json!({
            "name": "get_review_status",
            "title": "Get review status",
            "description": "Current OpenCodeReview review of one pull request: queued, reviewing (with the running stage and chunk progress), publishing, ready, posted, reviewed (marked reviewed in OpenCodeReview, nothing posted), failed (with the error), cancelled, skipped, finished or not reviewed. `done` is false while OpenCodeReview still has work to do; `retryAfterSeconds` says when to ask again. Set waitSeconds to block until the review is done or the wait runs out.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "pullRequest": {
                        "type": "string",
                        "description": "The pull request: its URL (https://github.com/owner/repo/pull/123) or owner/repo#123."
                    },
                    "waitSeconds": {
                        "type": "integer",
                        "minimum": 0,
                        "maximum": MAX_WAIT_SECONDS,
                        "description": "Wait up to this many seconds for the review to finish before answering. 0 (the default) answers at once."
                    }
                },
                "required": ["pullRequest"],
                "additionalProperties": false
            },
            "annotations": { "readOnlyHint": true, "openWorldHint": false }
        }),
        json!({
            "name": "list_active_reviews",
            "title": "List active reviews",
            "description": "Open pull requests whose OpenCodeReview review is queued, running or being posted, most recently updated first.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "repository": {
                        "type": "string",
                        "description": "Only this repository (owner/repo). All repositories when omitted."
                    }
                },
                "additionalProperties": false
            },
            "annotations": { "readOnlyHint": true, "openWorldHint": false }
        }),
    ]
}

fn call_tool(state: &SharedState, name: &str, arguments: &Value) -> Result<Value, String> {
    match name {
        "get_review_status" => {
            let reference = arguments
                .get("pullRequest")
                .and_then(Value::as_str)
                .ok_or("pullRequest is required: a pull request URL or owner/repo#123")?;
            let (repository, number) = parse_pull_request_reference(reference)?;
            let wait = arguments
                .get("waitSeconds")
                .and_then(Value::as_u64)
                .unwrap_or(0)
                .min(MAX_WAIT_SECONDS);
            wait_for_review_status(state, &repository, number, Duration::from_secs(wait))
        }
        "list_active_reviews" => {
            let repository = arguments.get("repository").and_then(Value::as_str);
            // A connection per call: never hold one while building the reply.
            let reviews = {
                let db = state.config.db.open_read()?;
                db.active_review_pull_requests(repository, ACTIVE_REVIEW_LIMIT)?
                    .iter()
                    .map(|pr| pull_request_status(&db, pr))
                    .collect::<Result<Vec<_>, _>>()?
            };
            Ok(json!({ "reviews": reviews }))
        }
        _ => Err(format!("unknown tool: {name}")),
    }
}

fn wait_for_review_status(state: &SharedState, repository: &str, number: i64, wait: Duration) -> Result<Value, String> {
    let started = Instant::now();
    loop {
        let status = {
            let db = state.config.db.open_read()?;
            let pr = db.review_status_pull_request(repository, number)?.ok_or_else(|| {
                format!(
                    "{repository}#{number} is not tracked by OpenCodeReview: check that the repository is selected in Settings and that the pull request is open"
                )
            })?;
            pull_request_status(&db, &pr)?
        };
        if status["done"] == true || started.elapsed() + WAIT_POLL_INTERVAL > wait {
            return Ok(status);
        }
        thread::sleep(WAIT_POLL_INTERVAL);
    }
}

fn pull_request_status(db: &Database, pr: &ReviewStatusPullRequest) -> Result<Value, String> {
    let run = db.latest_review_run(pr.id)?;
    let result = db.review_result(pr.id)?;
    Ok(review_status(
        pr,
        run.as_ref(),
        result.as_ref(),
        crate::db::now_millis(),
    ))
}

/// Accepts `https://<host>/owner/repo/pull/123` (any GitHub host) and `owner/repo#123`.
pub(crate) fn parse_pull_request_reference(reference: &str) -> Result<(String, i64), String> {
    let invalid = || format!("not a pull request reference: {reference:?}; use a pull request URL or owner/repo#123");
    let reference = reference.trim();
    let (repository, number) = if reference.contains("/pull/") {
        let path = reference.split_once("://").map_or(reference, |(_, rest)| rest);
        // Drop the host, then any query or fragment.
        let path = path.split_once('/').map_or("", |(_, path)| path);
        let parts: Vec<&str> = path.split(['?', '#']).next().unwrap_or("").split('/').collect();
        match parts.as_slice() {
            [owner, repo, "pull", number, ..] => (format!("{owner}/{repo}"), *number),
            _ => return Err(invalid()),
        }
    } else {
        let (repository, number) = reference.split_once('#').ok_or_else(invalid)?;
        (repository.to_string(), number)
    };
    let number = number
        .trim()
        .parse::<i64>()
        .ok()
        .filter(|n| *n > 0)
        .ok_or_else(invalid)?;
    let valid_repository = repository
        .split_once('/')
        .is_some_and(|(owner, name)| !owner.is_empty() && !name.is_empty() && !name.contains('/'));
    if !valid_repository {
        return Err(invalid());
    }
    Ok((repository, number))
}

/// What an agent needs to know about a pull request's review, derived from stored state only.
pub(crate) fn review_status(
    pr: &ReviewStatusPullRequest,
    run: Option<&ReviewRun>,
    result: Option<&ReviewResult>,
    now: i64,
) -> Value {
    let run_status = run.map(|run| run.status.as_str());
    let result_for_run = result.filter(|result| run.is_some_and(|run| result.run_id == Some(run.id)));
    let publishing = run_status == Some("posting")
        || result_for_run.is_some_and(|result| matches!(result.publication_status.as_str(), "pending" | "posting"));
    let (state, summary): (&str, String) = if matches!(run_status, Some("pending" | "running")) {
        ("reviewing", running_summary(run))
    } else if publishing {
        (
            "publishing",
            "The review finished and is being posted to GitHub.".into(),
        )
    } else if !pr.state.eq_ignore_ascii_case("open") {
        (
            "closed",
            format!(
                "The pull request is {}; no review will run.",
                pr.state.to_ascii_lowercase()
            ),
        )
    } else if pr.review_lane == "queued" {
        let reason = pr.status_reason.as_deref().unwrap_or("Waiting for a free review slot");
        ("queued", format!("Queued: {reason}."))
    } else if pr.review_lane == "reviewing" {
        ("reviewing", "The review is starting.".into())
    } else {
        match run_status {
            None => (
                "not_reviewed",
                pr.status_reason
                    .clone()
                    .unwrap_or_else(|| "No review has run for this pull request.".into()),
            ),
            Some("failed" | "result_failed") => (
                "failed",
                format!(
                    "The review failed: {}",
                    run.and_then(|run| run.error_message.as_deref())
                        .or(pr.status_reason.as_deref())
                        .unwrap_or("no error was recorded")
                ),
            ),
            Some("cancelled") => ("cancelled", "The review was cancelled.".into()),
            Some("posted" | "resolved") => ("posted", "The review is posted on GitHub.".into()),
            Some("preview" | "approved")
                if result_for_run.is_some_and(|result| result.publication_status == "posted") =>
            {
                ("posted", "The review is posted on GitHub.".into())
            }
            Some("preview") => (
                "ready",
                "The review finished and waits in OpenCodeReview for someone to post or dismiss it.".into(),
            ),
            Some("approved") => (
                "reviewed",
                "The review finished and was marked as reviewed in OpenCodeReview.".into(),
            ),
            Some("superseded" | "skipped") => (
                "skipped",
                pr.status_reason
                    .clone()
                    .unwrap_or_else(|| "The last review was skipped or replaced.".into()),
            ),
            Some(status) => ("finished", format!("The last review ended with status {status}.")),
        }
    };
    let done = !matches!(state, "reviewing" | "publishing" | "queued");
    let mut status = json!({
        "pullRequest": {
            "repository": pr.repository,
            "number": pr.number,
            "title": pr.title,
            "url": pr.url,
            "state": pr.state,
            "headSha": pr.head_sha,
            "lastSyncedAt": iso_time(Some(pr.last_synced_at)),
        },
        "state": state,
        "done": done,
        "summary": summary,
        "run": run.map(|run| run_status_json(run, pr, now)),
        "publication": result_for_run.map(|result| json!({
            "status": result.publication_status,
            "result": result.effective_state,
            "reviewUrl": result.github_review_url,
            "postedAt": iso_time(result.posted_at),
            "error": result.error_message,
        })),
    });
    if !done {
        status["retryAfterSeconds"] = json!(RETRY_AFTER_SECONDS);
    }
    status
}

fn running_summary(run: Option<&ReviewRun>) -> String {
    let phases = run.map(|run| run_phases(run.phase_json.as_deref())).unwrap_or_default();
    let Some(current) = phases
        .iter()
        .find(|phase| matches!(phase["status"].as_str(), Some("running" | "queued")))
    else {
        return "The review is running.".into();
    };
    let label = current["label"].as_str().or(current["id"].as_str()).unwrap_or("review");
    let position = phases.iter().position(|phase| phase == current).unwrap_or(0) + 1;
    let mut summary = format!("Reviewing: {label} (stage {position} of {}).", phases.len());
    if let Some(chunks) = current
        .get("chunks")
        .filter(|chunks| chunks["total"].as_u64() > Some(0))
    {
        summary.push_str(&format!(
            " {} of {} chunks done, {} running.",
            chunks["done"], chunks["total"], chunks["running"]
        ));
    }
    summary
}

/// Stages without their logs, chunks reduced to counts.
fn run_phases(phase_json: Option<&str>) -> Vec<Value> {
    let phases: Vec<Value> = phase_json
        .and_then(|raw| serde_json::from_str(raw).ok())
        .unwrap_or_default();
    phases
        .iter()
        .map(|phase| {
            let mut stage = json!({
                "id": phase["id"],
                "label": phase["label"],
                "status": phase["status"],
                "startedAt": iso_time(phase["startedAt"].as_i64()),
                "finishedAt": iso_time(phase["finishedAt"].as_i64()),
            });
            if let Some(chunks) = phase["chunks"].as_array() {
                let count = |states: &[&str]| {
                    chunks
                        .iter()
                        .filter(|chunk| chunk["status"].as_str().is_some_and(|status| states.contains(&status)))
                        .count()
                };
                stage["chunks"] = json!({
                    "total": chunks.len(),
                    "done": count(&["ok", "done", "cached", "failed", "error", "cancelled"]),
                    "running": count(&["running", "validating"]),
                    "failed": count(&["failed", "error"]),
                    "cancelled": count(&["cancelled"]),
                });
            }
            stage
        })
        .collect()
}

fn run_status_json(run: &ReviewRun, pr: &ReviewStatusPullRequest, now: i64) -> Value {
    let finding_count = run
        .comments_json
        .as_deref()
        .and_then(|raw| serde_json::from_str::<Value>(raw).ok())
        .and_then(|value| value.get("comments").cloned().or(Some(value)))
        .and_then(|comments| comments.as_array().map(Vec::len));
    let elapsed = run
        .started_at
        .map(|started| (run.finished_at.unwrap_or(now) - started).max(0) / 1000);
    json!({
        "id": run.id,
        "status": run.status,
        "headSha": run.head_sha,
        "currentHead": run.head_sha == pr.head_sha,
        "model": run.model,
        "createdAt": iso_time(Some(run.created_at)),
        "startedAt": iso_time(run.started_at),
        "finishedAt": iso_time(run.finished_at),
        "elapsedSeconds": elapsed,
        "stages": run_phases(run.phase_json.as_deref()),
        "findings": finding_count,
        "error": run.error_message,
    })
}

fn iso_time(millis: Option<i64>) -> Option<String> {
    let millis = millis?;
    let seconds = millis.div_euclid(1000);
    let (days, rest) = (seconds.div_euclid(86_400), seconds.rem_euclid(86_400));
    // Civil date from days since 1970-01-01 (Howard Hinnant's algorithm).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    Some(format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        rest / 3600,
        rest % 3600 / 60,
        rest % 60
    ))
}
