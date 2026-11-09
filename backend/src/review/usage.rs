use super::*;

pub(crate) fn parse_transcript_usage(transcript_path: &Path) -> RunUsage {
    let mut usages = vec![parse_transcript_usage_single(transcript_path)];
    if let Some(parent) = transcript_path.parent() {
        for attempt in 1..=3 {
            let retry = parent.join(format!("transcript.retry-{attempt}.json"));
            if retry.exists() {
                usages.push(parse_transcript_usage_single(&retry));
            }
        }
        // Attempts abandoned when an account hit its limit still consumed tokens. They are
        // numbered without gaps, across the stage's executions.
        for attempt in 1.. {
            let failover = parent.join(format!("transcript.failover-{attempt}.json"));
            if !failover.exists() {
                break;
            }
            usages.push(parse_transcript_usage_single(&failover));
        }
    }
    sum_usage(usages)
}

pub(crate) fn parse_transcript_usage_single(transcript_path: &Path) -> RunUsage {
    let Some(value) = fs::read_to_string(transcript_path)
        .ok()
        .and_then(|raw| transcript_events(&raw))
    else {
        return RunUsage::default();
    };
    let mut per_model = std::collections::HashMap::new();
    collect_usage(&value, &mut per_model);
    let mut usage = finalize_usage(per_model);
    usage.activity = collect_activity(&value);
    usage
}

/// Counts what one execution did from its events: Claude `assistant` and `result` events, Codex
/// `item.completed` and `turn.completed` events and OpenCode `step_start`, `tool_use` and
/// `step_finish` events.
pub fn collect_activity(value: &serde_json::Value) -> StageActivity {
    let mut activity = StageActivity::default();
    let mut messages = std::collections::HashSet::new();
    // OpenCode runs a step's tools before the step finishes; their time is not the provider's.
    let (mut step_started, mut step_tools_ms) = (None, 0);
    for event in value.as_array().into_iter().flatten() {
        match event["type"].as_str() {
            // Claude writes one event per content block; blocks of one round trip share its id.
            Some("assistant") => {
                if let Some(id) = event["message"]["id"].as_str() {
                    messages.insert(id);
                }
                activity.tool_calls += event["message"]["content"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter(|block| block["type"] == "tool_use")
                    .count() as i64;
            }
            Some("result") if event["stats"].is_null() => {
                activity.model_ms += event["duration_api_ms"].as_i64().unwrap_or(0);
            }
            Some("item.completed") if event["item"]["type"] == "command_execution" => activity.tool_calls += 1,
            Some("turn.completed") => {
                activity.reasoning_tokens += event["usage"]["reasoning_output_tokens"].as_i64().unwrap_or(0);
            }
            Some("step_start") => (step_started, step_tools_ms) = (event["timestamp"].as_i64(), 0),
            Some("tool_use") => {
                activity.tool_calls += 1;
                let time = &event["part"]["state"]["time"];
                if let (Some(start), Some(end)) = (time["start"].as_i64(), time["end"].as_i64()) {
                    step_tools_ms += (end - start).max(0);
                }
            }
            Some("step_finish") => {
                activity.turns += 1;
                activity.reasoning_tokens += event["part"]["tokens"]["reasoning"].as_i64().unwrap_or(0);
                if let (Some(start), Some(end)) = (step_started.take(), event["timestamp"].as_i64()) {
                    activity.model_ms += (end - start - step_tools_ms).max(0);
                }
            }
            _ => {}
        }
    }
    activity.turns += messages.len() as i64;
    activity
}

/// Parses a stream-json transcript (one event per line) into an array of events.
pub(crate) fn transcript_events(raw: &str) -> Option<serde_json::Value> {
    if raw.trim().is_empty() {
        return None;
    }
    raw.lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| serde_json::from_str(line).ok())
        .collect::<Option<Vec<serde_json::Value>>>()
        .map(serde_json::Value::Array)
}

/// Build a RunUsage (grand totals + dominant model + per-model rows) from a map.
pub fn finalize_usage(per_model: std::collections::HashMap<String, ModelUsageEntry>) -> RunUsage {
    let mut usage = RunUsage::default();
    for (_, entry) in per_model.into_iter() {
        usage.input_tokens += entry.input_tokens;
        usage.output_tokens += entry.output_tokens;
        usage.cache_read_tokens += entry.cache_read_tokens;
        usage.cache_creation_tokens += entry.cache_creation_tokens;
        usage.by_model.push(entry);
    }
    // Dominant model = highest token volume (for the single-label "This run" chip).
    usage
        .by_model
        .sort_by_key(|entry| std::cmp::Reverse(entry.input_tokens + entry.output_tokens));
    usage.model = usage.by_model.first().map(|entry| entry.model.clone());
    usage
}

/// Canonicalize a model label so effort variants and dated snapshots collapse into one row.
pub fn normalize_model(raw: &str) -> String {
    let mut s = raw.trim().to_lowercase().replace(['.', ' ', '_'], "-");
    while s.contains("--") {
        s = s.replace("--", "-");
    }
    if let Some((model, date)) = s.rsplit_once('-') {
        if date.len() == 8 && date.bytes().all(|byte| byte.is_ascii_digit()) {
            s = model.to_string();
        }
    }
    for suffix in ["low", "medium", "high", "xhigh", "max"] {
        if let Some(stripped) = s.strip_suffix(&format!("-{suffix}")) {
            s = stripped.to_string();
            break;
        }
    }
    let s = s.trim_matches('-').to_string();
    if s.is_empty() {
        "unknown".to_string()
    } else {
        s
    }
}

/// Accumulates per-model token usage from Claude `result` events, Codex `turn.completed`
/// events and OpenCode `step_finish` events. Each Claude execution emits exactly one result whose `modelUsage` already totals
/// every turn, so assistant messages are not summed again.
pub fn collect_usage(value: &serde_json::Value, per_model: &mut std::collections::HashMap<String, ModelUsageEntry>) {
    let events = match value {
        serde_json::Value::Array(items) => items.iter().collect::<Vec<_>>(),
        value => vec![value],
    };
    // Codex reports each turn's totals; its input count includes the cached part.
    for usage in events
        .iter()
        .filter(|event| event["type"] == "turn.completed")
        .map(|event| &event["usage"])
    {
        let entry = per_model.entry("codex".into()).or_insert_with(|| ModelUsageEntry {
            model: "codex".into(),
            ..Default::default()
        });
        let cached = usage["cached_input_tokens"].as_i64().unwrap_or(0);
        entry.input_tokens += (usage["input_tokens"].as_i64().unwrap_or(0) - cached).max(0);
        entry.cache_read_tokens += cached;
        entry.output_tokens += usage["output_tokens"].as_i64().unwrap_or(0);
    }
    // OpenCode reports each step's tokens when the step finishes; `input` excludes the cache.
    for tokens in events
        .iter()
        .filter(|event| event["type"] == "step_finish")
        .map(|event| &event["part"]["tokens"])
        .filter(|tokens| tokens.is_object())
    {
        let entry = per_model.entry("opencode".into()).or_insert_with(|| ModelUsageEntry {
            model: "opencode".into(),
            ..Default::default()
        });
        entry.input_tokens += tokens["input"].as_i64().unwrap_or(0);
        entry.output_tokens += tokens["output"].as_i64().unwrap_or(0) + tokens["reasoning"].as_i64().unwrap_or(0);
        entry.cache_read_tokens += tokens["cache"]["read"].as_i64().unwrap_or(0);
        entry.cache_creation_tokens += tokens["cache"]["write"].as_i64().unwrap_or(0);
    }
    for event in events
        .into_iter()
        .filter(|event| event["type"] == "result" && event["stats"].is_null())
    {
        let mut add = |raw: &str, usage: &serde_json::Value, camel: bool| {
            let model = normalize_model(raw);
            let entry = per_model.entry(model.clone()).or_insert_with(|| ModelUsageEntry {
                model,
                ..Default::default()
            });
            let get = |camel_key: &str, snake_key: &str| {
                usage[if camel { camel_key } else { snake_key }].as_i64().unwrap_or(0)
            };
            entry.input_tokens += get("inputTokens", "input_tokens");
            entry.output_tokens += get("outputTokens", "output_tokens");
            entry.cache_read_tokens += get("cacheReadInputTokens", "cache_read_input_tokens");
            entry.cache_creation_tokens += get("cacheCreationInputTokens", "cache_creation_input_tokens");
        };
        match event["modelUsage"].as_object().filter(|models| !models.is_empty()) {
            Some(models) => {
                for (model, usage) in models {
                    add(model, usage, true);
                }
            }
            None if event["usage"].is_object() => add("unknown", &event["usage"], false),
            None => {}
        }
    }
}
