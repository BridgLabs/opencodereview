//! Pinned OpenCode Go protocol. Application identity remains `opencode`.
pub(crate) const CLI: &str = "/opt/opencode/bin/opencode";
pub(crate) const VERSION: &str = "2.2.6";
// npm archive digests, mirrored by Dockerfile and included in the executor fingerprint.
pub(crate) const CLI_SHA256: [&str; 2] = [
    "dfcd69f1d511ae10ed626768b41d84b8100cc4aa64cecb69da38ba7a29d33551",
    "22352e200630ec61a48a65b9f5c2b627328ae5002658d81c067f67c5fb689898",
];
pub(crate) fn stream_failure(pending: &mut Vec<u8>, data: &[u8]) -> Result<Option<String>, String> {
    let old = pending.len();
    if old + data.len() > 32_000_000 {
        return Err("OpenCode protocol limit exceeded".into());
    }
    pending.extend_from_slice(data);
    let mut consumed = 0;
    for (index, byte) in data.iter().enumerate() {
        if *byte != b'\n' {
            continue;
        }
        let end = old + index;
        let line = &pending[consumed..end];
        consumed = end + 1;
        if line.iter().all(u8::is_ascii_whitespace) {
            continue;
        }
        let line = std::str::from_utf8(line).map_err(|_| "OpenCode protocol is not UTF-8")?;
        serde_json::from_str::<serde_json::Value>(line).map_err(|_| "OpenCode protocol is not JSON")?;
        if let Some(error) = failure(line) {
            pending.clear();
            return Ok(Some(error));
        }
    }
    pending.drain(..consumed);
    Ok(None)
}
pub(crate) fn failure(raw: &str) -> Option<String> {
    raw.lines()
        .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
        .find_map(|event| {
            if !matches!(event["type"].as_str(), Some("retry" | "error")) {
                return None;
            }
            match (event["error"]["type"].as_str(), event["error"]["status"].as_u64()) {
                (Some("provider.rate-limit"), Some(429)) => {
                    Some(crate::providers::claude::Failure::UsageLimit { resets_at: None }.encode())
                }
                _ => None,
            }
        })
}

/// A candidate only: root must additionally verify the authenticated failure seal and exit.
pub(crate) fn auth_session(raw: &str, exit_code: Option<i32>) -> Option<String> {
    if exit_code != Some(1) {
        return None;
    }
    let mut session = None;
    let mut terminal = false;
    for line in raw.lines().filter(|line| !line.trim().is_empty()) {
        if terminal {
            return None;
        }
        let event: serde_json::Value = serde_json::from_str(line).ok()?;
        let id = event["sessionID"]
            .as_str()
            .filter(|id| !id.is_empty() && id.len() <= 128)?;
        if session.as_deref().is_some_and(|previous| previous != id) {
            return None;
        }
        session = Some(id.to_string());
        match event["type"].as_str()? {
            "error" if event["error"]["type"] == "provider.auth" && event["error"]["status"] == 401 => terminal = true,
            "step_start" | "step_finish" | "text" | "tool_use" | "reasoning" | "retry" => {}
            _ => return None,
        }
    }
    if terminal {
        session
    } else {
        None
    }
}

pub(crate) fn exec_args(model: &str, effort: &str) -> Result<Vec<String>, String> {
    if !crate::providers::go_catalogue::selector(model)
        || !effort.is_empty() && !crate::providers::go_catalogue::selector(effort)
    {
        return Err("Invalid OpenCode Go selector".into());
    }
    Ok(vec![
        "run".into(),
        "--standalone".into(),
        "--format".into(),
        "json".into(),
        "--title".into(),
        "Frozen review".into(),
        "--agent".into(),
        "ocr-review".into(),
        "--model".into(),
        if effort.is_empty() {
            format!("opencode-go/{model}")
        } else {
            format!("opencode-go/{model}#{effort}")
        },
    ])
}

pub(crate) fn usage_limit_session(raw: &str) -> Option<String> {
    let mut session = None;
    let mut limited = false;
    for line in raw.lines().filter(|line| !line.trim().is_empty()) {
        let event: serde_json::Value = serde_json::from_str(line).ok()?;
        let id = event["sessionID"]
            .as_str()
            .filter(|id| !id.is_empty() && id.len() <= 128)?;
        if session.as_deref().is_some_and(|previous| previous != id) {
            return None;
        }
        session = Some(id.to_string());
        limited = matches!(event["type"].as_str(), Some("retry" | "error"))
            && event["error"]["type"] == "provider.rate-limit"
            && event["error"]["status"] == 429;
    }
    if limited {
        session
    } else {
        None
    }
}
pub(crate) fn final_text(raw: &str) -> Result<(String, String), String> {
    let error = "OpenCode terminal response unavailable";
    let mut session = None;
    let mut texts = Vec::new();
    let mut parts = std::collections::HashMap::new();
    let mut finish = None;
    for line in raw.lines().filter(|line| !line.trim().is_empty()) {
        let event: serde_json::Value = serde_json::from_str(line).map_err(|_| error)?;
        let id = event["sessionID"]
            .as_str()
            .filter(|id| !id.is_empty() && id.len() <= 128)
            .ok_or(error)?;
        if session.as_deref().is_some_and(|previous| previous != id) {
            return Err(error.into());
        }
        session = Some(id.to_string());
        let part = &event["part"];
        match event["type"].as_str() {
            Some("text") => {
                let id = part["id"].as_str().ok_or(error)?.to_string();
                let message = part["messageID"].as_str().ok_or(error)?.to_string();
                let text = part["text"].as_str().ok_or(error)?.to_string();
                if let Some(previous) = parts.insert(id.clone(), (message.clone(), text.clone())) {
                    if previous != (message, text) {
                        return Err(error.into());
                    }
                } else {
                    texts.push(id);
                }
            }
            Some("step_finish") => {
                finish = Some((
                    part["messageID"].as_str().ok_or(error)?.to_string(),
                    part["reason"].as_str().ok_or(error)?.to_string(),
                ))
            }
            Some("retry") => finish = None,
            Some("step_start" | "tool_use" | "reasoning") => {}
            _ => return Err(error.into()),
        }
    }
    let (message, reason) = finish.ok_or(error)?;
    if reason != "stop" {
        return Err(error.into());
    }
    let text: String = texts
        .iter()
        .filter_map(|id| parts.get(id))
        .filter(|(id, _)| *id == message)
        .map(|(_, text)| text.as_str())
        .collect();
    if text.trim().is_empty() {
        return Err(error.into());
    }
    Ok((session.ok_or(error)?, text))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opencode_dynamic_selectors_do_not_invent_effort_or_accept_paths() {
        assert_eq!(
            exec_args("new-model-2099", "new-effort").unwrap().last().unwrap(),
            "opencode-go/new-model-2099#new-effort"
        );
        assert_eq!(
            exec_args("new-model-2099", "").unwrap().last().unwrap(),
            "opencode-go/new-model-2099"
        );
        for invalid in ["../model", "provider/model", "model#effort", "two words", ""] {
            assert!(exec_args(invalid, "").is_err());
        }
    }

    #[test]
    fn opencode_limit_candidate_binds_all_events_to_the_retry_session() {
        let retry = r#"{"type":"retry","sessionID":"session","error":{"type":"provider.rate-limit","status":429}}"#;
        assert_eq!(usage_limit_session(retry).as_deref(), Some("session"));
        assert!(
            usage_limit_session(&format!("{{\"type\":\"step_start\",\"sessionID\":\"other\"}}\n{retry}")).is_none()
        );
        assert!(usage_limit_session(&retry.replace("429", "401")).is_none());
        assert!(usage_limit_session(&format!("{retry}\n{{\"type\":\"text\",\"sessionID\":\"session\"}}")).is_none());
    }
    #[test]
    fn opencode_auth_requires_one_terminal_structured_event_and_normal_failure_exit() {
        let event = r#"{"type":"error","sessionID":"session","error":{"type":"provider.auth","status":401}}"#;
        assert_eq!(auth_session(event, Some(1)).as_deref(), Some("session"));
        for code in [None, Some(0), Some(2), Some(143)] {
            assert!(auth_session(event, code).is_none());
        }
        for raw in [
            format!("{event}\n{event}"),
            format!("{{\"type\":\"step_start\",\"sessionID\":\"other\"}}\n{event}"),
            event.replace("401", "403"),
            event.replace("\"error\",\"sessionID\"", "\"retry\",\"sessionID\""),
            serde_json::json!({"type":"text","sessionID":"session","part":{"text":event}}).to_string(),
        ] {
            assert!(auth_session(&raw, Some(1)).is_none(), "{raw}");
        }
    }
    #[test]
    fn opencode_stream_waits_for_complete_utf8_json_before_classifying_retry() {
        let mut pending = Vec::new();
        assert!(
            stream_failure(&mut pending, b"{\"type\":\"text\",\"part\":{\"text\":\"\xe4")
                .unwrap()
                .is_none()
        );
        assert!(stream_failure(&mut pending, b"\xbd\xa0\"}}\n").unwrap().is_none());
        let result = stream_failure(
            &mut pending,
            b"{\"type\":\"retry\",\"error\":{\"type\":\"provider.rate-limit\",\"status\":429}}\n",
        )
        .unwrap();
        assert!(result.unwrap().starts_with("usage_limit:"));
    }
    #[test]
    fn opencode_only_trusted_retry_events_trigger_account_failover() {
        assert!(failure(r#"{"type":"text","part":{"text":"429 provider.rate-limit"}}"#).is_none());
        let error = failure(r#"{"type":"retry","error":{"type":"provider.rate-limit","status":429}}"#).unwrap();
        assert!(matches!(
            crate::providers::claude::decode(&error).0,
            crate::providers::claude::Failure::UsageLimit { .. }
        ));
    }
    #[test]
    fn opencode_recovered_retry_requires_a_later_terminal_response() {
        let retry =
            serde_json::json!({"type":"retry","sessionID":"session","error":{"type":"provider.error","status":503}})
                .to_string();
        let text = serde_json::json!({"type":"text","sessionID":"session","part":{"id":"answer","messageID":"message","text":"report"}}).to_string();
        let stop = serde_json::json!({"type":"step_finish","sessionID":"session","part":{"messageID":"message","reason":"stop"}}).to_string();
        assert!(failure(&retry).is_none());
        assert_eq!(
            final_text(&format!("{retry}\n{text}\n{stop}")).unwrap(),
            ("session".into(), "report".into())
        );
        for raw in [
            format!("{retry}\n{text}"),
            format!("{text}\n{stop}\n{retry}"),
            format!("{retry}\n{text}\n{}", stop.replace("stop", "length")),
            format!("{retry}\n{text}\n{}", stop.replace("message\"", "other\"")),
            format!("{retry}\n{text}\n{stop}\n{{"),
            format!("{retry}\n{text}\n{stop}\n{}", retry.replace("retry", "error")),
            format!("{}\n{text}\n{stop}", retry.replace("session\"", "other\"")),
        ] {
            assert!(final_text(&raw).is_err(), "{raw}");
        }
    }

    #[test]
    fn opencode_terminal_text_ignores_intermediate_steps_and_duplicate_text() {
        let raw = [
            serde_json::json!({"type":"text","sessionID":"session","part":{"id":"early","messageID":"one","text":"not final"}}),
            serde_json::json!({"type":"step_finish","sessionID":"session","part":{"id":"first","messageID":"one","reason":"tool-calls"}}),
            serde_json::json!({"type":"text","sessionID":"session","part":{"id":"answer","messageID":"two","text":"{\"findings\":[]}"}}),
            serde_json::json!({"type":"text","sessionID":"session","part":{"id":"answer","messageID":"two","text":"{\"findings\":[]}"}}),
            serde_json::json!({"type":"step_finish","sessionID":"session","part":{"id":"last","messageID":"two","reason":"stop"}}),
        ].map(|value|value.to_string()).join("\n");
        assert_eq!(
            final_text(&raw).unwrap(),
            ("session".into(), "{\"findings\":[]}".into())
        );
    }
}
