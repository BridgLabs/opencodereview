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
    finalize_usage(per_model)
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

/// Accumulates per-model token usage from Claude `result` events and Codex `turn.completed`
/// events. Each Claude execution emits exactly one result whose `modelUsage` already totals
/// every turn, so assistant messages are not summed again.
pub fn collect_usage(value: &serde_json::Value, per_model: &mut std::collections::HashMap<String, ModelUsageEntry>) {
    let events = match value {
        serde_json::Value::Array(items) => items.iter().collect::<Vec<_>>(),
        value => vec![value],
    };
    let model = events
        .iter()
        .find(|event| event["type"] == "ocr.opencode.model")
        .and_then(|event| event["model"].as_str())
        .map(normalize_model)
        .unwrap_or_else(|| "opencode".into());
    let mut seen = std::collections::HashSet::new();
    for event in events.iter().filter(|event| event["type"] == "step_finish") {
        let Some(id) = event["part"]["id"].as_str() else {
            continue;
        };
        let Some(session) = event["sessionID"].as_str() else {
            continue;
        };
        if !seen.insert((session, id)) {
            continue;
        }
        let usage = &event["part"]["tokens"];
        // Per-step counts above u32 are impossible for the curated Go context/output limits.
        let get =
            |value: &serde_json::Value| value.as_u64().filter(|value| *value <= u32::MAX as u64).unwrap_or(0) as i64;
        let input = get(&usage["input"]);
        let output = get(&usage["output"]).saturating_add(get(&usage["reasoning"]));
        let read = get(&usage["cache"]["read"]);
        let write = get(&usage["cache"]["write"]);
        if input == 0 && output == 0 && read == 0 && write == 0 {
            continue;
        }
        let entry = per_model.entry(model.clone()).or_insert_with(|| ModelUsageEntry {
            model: model.clone(),
            ..Default::default()
        });
        entry.input_tokens = entry.input_tokens.saturating_add(input);
        entry.output_tokens = entry.output_tokens.saturating_add(output);
        entry.cache_read_tokens = entry.cache_read_tokens.saturating_add(read);
        entry.cache_creation_tokens = entry.cache_creation_tokens.saturating_add(write);
    }
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

#[cfg(test)]
mod opencode_tests {
    use super::*;
    #[test]
    fn opencode_deepseek_usage_uses_actual_selected_model() {
        let events = if let Ok(path) = std::env::var("OCR_OPENCODE_MODEL_STAGE") {
            let stage: serde_json::Value = serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap();
            assert!(stage["error"].is_null());
            transcript_events(stage["transcript"].as_str().unwrap()).unwrap()
        } else {
            serde_json::json!([
                {"type":"step_finish","sessionID":"session","part":{"id":"one","tokens":{"input":14,"output":5,"reasoning":2,"cache":{"read":3,"write":0}}}},
                {"type":"step_finish","sessionID":"session","part":{"id":"two","tokens":{"input":14,"output":5,"reasoning":2,"cache":{"read":3,"write":0}}}},
                {"type":"ocr.opencode.model","model":"deepseek-v4.1-flash"}
            ])
        };
        let mut models = std::collections::HashMap::new();
        collect_usage(&events, &mut models);
        let usage = finalize_usage(models);
        assert_eq!(usage.model.as_deref(), Some("deepseek-v4-1-flash"));
        assert_eq!(usage.by_model.len(), 1);
        assert_eq!(
            (
                usage.input_tokens,
                usage.output_tokens,
                usage.cache_read_tokens,
                usage.cache_creation_tokens
            ),
            (28, 14, 6, 0)
        );
    }
    #[test]
    fn opencode_missing_or_zero_usage_remains_unreported() {
        let mut models = std::collections::HashMap::new();
        collect_usage(
            &serde_json::json!([
                {"type":"step_finish","sessionID":"session","part":{"id":"missing"}},
                    {"type":"step_finish","sessionID":"session","part":{"id":"zero","tokens":{"input":0,"output":0,"reasoning":0,"cache":{"read":0,"write":0}}}},
                    {"type":"step_finish","sessionID":"session","part":{"id":"overflow","tokens":{"input":9223372036854775807i64}}}
            ]),
            &mut models,
        );
        assert!(finalize_usage(models).by_model.is_empty());
    }
    #[test]
    fn opencode_usage_deduplicates_step_parts_and_includes_reasoning() {
        let step = serde_json::json!({"type":"step_finish","sessionID":"session","part":{"id":"step","tokens":{"input":14,"output":5,"reasoning":2,"cache":{"read":3,"write":1}}}});
        let mut models = std::collections::HashMap::new();
        collect_usage(&serde_json::json!([step.clone(), step]), &mut models);
        let usage = finalize_usage(models);
        assert_eq!(
            (
                usage.input_tokens,
                usage.output_tokens,
                usage.cache_read_tokens,
                usage.cache_creation_tokens
            ),
            (14, 7, 3, 1)
        );
    }
}
