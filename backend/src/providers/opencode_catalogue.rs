//! OpenCode's public model lists, read from opencode.ai instead of a list in the source: the Go
//! plan's and Zen's. Kept for an hour; when opencode.ai can't be reached the last list stands.
use serde_json::Value;
use std::io::Read;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// The plan (`go` or `zen`) an OpenCode model id (`<provider>/<model>`) bills.
pub(crate) fn plan(model: &str) -> Option<&'static str> {
    match model.split_once('/')?.0 {
        "opencode-go" => Some("go"),
        "opencode" => Some("zen"),
        _ => None,
    }
}

/// Each OpenCode provider, the plan it bills and its public model list.
const SOURCES: &[(&str, &str, &str)] = &[
    ("opencode-go", "Go", "https://opencode.ai/inference/go/v1/models"),
    ("opencode", "Zen", "https://opencode.ai/zen/v1/models"),
];
const MAX_BYTES: u64 = 4 * 1024 * 1024;
const FRESH: Duration = Duration::from_secs(3600);
/// After a failed refresh, the last list is kept and opencode.ai is asked again this much later.
const RETRY: Duration = Duration::from_secs(300);

/// One model reviews can run on, as `<provider>/<model>`.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Model {
    pub id: String,
    pub label: String,
    pub efforts: Vec<String>,
    pub default_effort: String,
}

fn name_ok(value: &str, max: usize) -> bool {
    (1..=max).contains(&value.len())
        && value
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'.' | b'-' | b'_'))
        && !value.starts_with(['.', '-'])
}

/// Whether `model` (`<provider>/<id>`) and `effort` are shaped like a catalogue choice. Runners,
/// which can't reach the lists, check only this; the server checks the lists themselves.
pub(crate) fn selector_ok(model: &str, effort: &str) -> bool {
    model
        .split_once('/')
        .is_some_and(|(provider, id)| SOURCES.iter().any(|(known, _, _)| *known == provider) && name_ok(id, 100))
        && (effort.is_empty() || name_ok(effort, 32))
}

/// Whether every price of a model is zero: OpenCode's free models, which may train on requests.
fn free(model: &Value) -> bool {
    let prices = ["input", "output"]
        .iter()
        .filter_map(|side| model["cost"]["text"][side]["price"].as_f64())
        .collect::<Vec<_>>();
    !prices.is_empty() && prices.iter().all(|price| *price == 0.0)
}

/// The models of one list that can review: chat models with tools, not deprecated and not free.
pub(crate) fn parse(provider: &str, plan: &str, list: &Value) -> Vec<Model> {
    list["data"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|model| {
            model["type"].as_str().is_none_or(|kind| kind == "chat")
                && model["capabilities"]["tools"]["supported"] == true
                && model["status"] != "deprecated"
                && !free(model)
        })
        .filter_map(|model| {
            let id = model["id"]
                .as_str()
                .filter(|id| name_ok(id, 100) && !id.ends_with("-free"))?;
            let name = model["name"]
                .as_str()
                .filter(|name| !name.trim().is_empty() && name.len() <= 80 && !name.chars().any(char::is_control))
                .unwrap_or(id);
            let reasoning = &model["capabilities"]["reasoning"];
            let efforts = reasoning["effort"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .filter(|effort| name_ok(effort, 32))
                .map(str::to_string)
                .collect::<Vec<_>>();
            let pick = |effort: &str| efforts.iter().any(|known| known == effort).then(|| effort.to_string());
            let default_effort = reasoning["default"]
                .as_str()
                .and_then(pick)
                .or_else(|| pick("medium"))
                .or_else(|| pick("high"))
                .or_else(|| efforts.first().cloned())
                .unwrap_or_default();
            Some(Model {
                id: format!("{provider}/{id}"),
                label: format!("{plan} · {}", name.trim()),
                efforts,
                default_effort,
            })
        })
        .collect()
}

fn fetch() -> Result<Vec<Model>, String> {
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(15))
        .build()
        .map_err(|error| error.to_string())?;
    let mut models = Vec::new();
    for (provider, plan, url) in SOURCES {
        let response = client
            .get(*url)
            .send()
            .and_then(reqwest::blocking::Response::error_for_status)
            .map_err(|_| "Could not read OpenCode's model list; check the connection to opencode.ai".to_string())?;
        let mut body = Vec::new();
        response
            .take(MAX_BYTES)
            .read_to_end(&mut body)
            .map_err(|error| error.to_string())?;
        let list = serde_json::from_slice::<Value>(&body).map_err(|_| "OpenCode's model list is not valid JSON")?;
        models.extend(parse(provider, plan, &list));
    }
    if models.is_empty() {
        return Err("OpenCode's model list is empty".into());
    }
    Ok(models)
}

static CACHE: Mutex<Option<(Instant, Vec<Model>)>> = Mutex::new(None);

/// The models reviews can pick, Go's first: refreshed from opencode.ai at most hourly.
pub(crate) fn models() -> Result<Vec<Model>, String> {
    if cfg!(test) {
        return Err("OpenCode's model list is not read in tests".into());
    }
    let mut cache = CACHE.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    if let Some((at, models)) = cache.as_ref() {
        if at.elapsed() < FRESH {
            return Ok(models.clone());
        }
    }
    match fetch() {
        Ok(models) => {
            *cache = Some((Instant::now(), models.clone()));
            Ok(models)
        }
        Err(error) => match cache.as_mut() {
            Some((at, models)) => {
                *at = Instant::now() - FRESH + RETRY;
                Ok(models.clone())
            }
            None => Err(error),
        },
    }
}

/// The list already read, without asking opencode.ai.
pub(crate) fn cached() -> Vec<Model> {
    CACHE
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .as_ref()
        .map(|(_, models)| models.clone())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn entry(id: &str, extra: Value) -> Value {
        let mut model = json!({
            "id": id,
            "name": "GLM-5.3",
            "type": "chat",
            "capabilities": { "tools": { "supported": true }, "reasoning": { "supported": true, "effort": ["low", "high", "max"] } },
            "cost": { "text": { "input": { "price": 1.4 }, "output": { "price": 4.4 } } }
        });
        for (key, value) in extra.as_object().unwrap() {
            model[key] = value.clone();
        }
        model
    }

    #[test]
    fn the_list_keeps_paid_chat_models_with_tools() {
        let list = json!({ "data": [
            entry("glm-5.3", json!({})),
            entry("big-pickle", json!({ "cost": { "text": { "input": { "price": 0 }, "output": { "price": 0 } } } })),
            entry("glm-5.3-flash-free", json!({})),
            entry("old", json!({ "status": "deprecated" })),
            entry("embed", json!({ "type": "embedding" })),
            entry("no-tools", json!({ "capabilities": { "tools": { "supported": false } } })),
            entry("../escape", json!({})),
            entry("plain", json!({ "name": "Plain", "capabilities": { "tools": { "supported": true }, "reasoning": { "supported": false } } })),
        ]});
        let models = parse("opencode", "Zen", &list);
        assert_eq!(
            models,
            vec![
                Model {
                    id: "opencode/glm-5.3".into(),
                    label: "Zen · GLM-5.3".into(),
                    efforts: vec!["low".into(), "high".into(), "max".into()],
                    default_effort: "high".into(),
                },
                Model {
                    id: "opencode/plain".into(),
                    label: "Zen · Plain".into(),
                    efforts: vec![],
                    default_effort: String::new(),
                },
            ]
        );
        assert!(parse("opencode", "Zen", &json!({ "data": "nope" })).is_empty());
    }

    #[test]
    fn each_model_bills_its_providers_plan() {
        assert_eq!(plan("opencode-go/glm-5.3"), Some("go"));
        assert_eq!(plan("opencode/claude-opus-5-5"), Some("zen"));
        assert_eq!(plan("anthropic/claude-opus-5-5"), None);
        assert_eq!(plan("glm-5.3"), None);
    }

    #[test]
    fn selectors_name_a_known_provider_and_a_plain_model() {
        assert!(selector_ok("opencode-go/glm-5.3", "high"));
        assert!(selector_ok("opencode/claude-opus-5-5", ""));
        assert!(!selector_ok("anthropic/claude-opus-5-5", ""));
        assert!(!selector_ok("opencode/../x", ""));
        assert!(!selector_ok("glm-5.3", ""));
        assert!(!selector_ok("opencode/glm-5.3#high", ""));
        assert!(!selector_ok("opencode/glm-5.3", "high max"));
    }
}
