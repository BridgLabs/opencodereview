//! Bounded public Go discovery. No credentials, database access, or fallback inventory.
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::io::Read;
use std::time::Duration;

pub(crate) const SOURCE: &str = "https://opencode.ai/inference/go/v1/models";
pub(crate) const MAX_BYTES: usize = 2 * 1024 * 1024;
const INVALID: &str = "OpenCode Go catalogue is invalid or unsupported";

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub(crate) struct Model {
    pub id: String,
    pub name: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub release_date: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub family: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    pub modalities: Modalities,
    pub capabilities: Capabilities,
    pub limit: Limit,
    pub api: BTreeMap<String, Api>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub(crate) struct Modalities {
    pub input: Vec<String>,
    pub output: Vec<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub(crate) struct Capabilities {
    pub tools: Tools,
    pub reasoning: Reasoning,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<bool>,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub(crate) struct Tools {
    pub supported: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub(crate) struct Reasoning {
    pub supported: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effort: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub toggle: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub budget: Option<Budget>,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub(crate) struct Budget {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max: Option<u64>,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub(crate) struct Limit {
    pub context: u64,
    pub output: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input: Option<u64>,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub(crate) struct Api {
    pub base_url: String,
    pub sdk: Sdk,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub compatibility: Option<Compatibility>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capabilities: Option<Value>,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub(crate) struct Sdk {
    pub aisdk: String,
    pub opencodeai: String,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub(crate) struct Compatibility {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning_field: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_tokens_field: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub media_input: Option<String>,
}

#[derive(Clone, Debug)]
pub(crate) struct Catalogue {
    pub id: String,
    pub models: Vec<Model>,
}

/// Only privileged runner IPC carries this type. Browser inputs never contain model metadata.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub(crate) struct Snapshot {
    pub catalogue_id: String,
    pub id: String,
    pub model: Model,
    pub effort: String,
}

pub(crate) fn selector(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 120
        && value.bytes().all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b))
        && !matches!(value, "." | ".." | "__proto__" | "constructor" | "prototype")
}

fn bounded(value: &Value, depth: usize, nodes: &mut usize) -> Result<(), String> {
    *nodes += 1;
    if depth > 24 || *nodes > 100_000 {
        return Err(INVALID.into());
    }
    match value {
        Value::String(s) if s.len() > 8192 => return Err(INVALID.into()),
        Value::Array(items) => {
            if items.len() > 1024 {
                return Err(INVALID.into());
            }
            for item in items {
                bounded(item, depth + 1, nodes)?;
            }
        }
        Value::Object(items) => {
            if items.len() > 128 {
                return Err(INVALID.into());
            }
            for (key, item) in items {
                if key.len() > 160 || key.chars().any(|c| c.is_control()) {
                    return Err(INVALID.into());
                }
                bounded(item, depth + 1, nodes)?;
            }
        }
        _ => {}
    }
    Ok(())
}

pub(crate) fn digest<T: Serialize>(value: &T) -> Result<String, String> {
    let bytes = serde_json::to_vec(value).map_err(|_| INVALID)?;
    if bytes.len() > MAX_BYTES {
        return Err(INVALID.into());
    }
    Ok(format!("{:x}", Sha256::digest(bytes)))
}

impl Model {
    pub fn efforts(&self) -> &[String] {
        self.capabilities.reasoning.effort.as_deref().unwrap_or_default()
    }
    pub fn default_effort(&self) -> &str {
        self.capabilities
            .reasoning
            .default
            .as_ref()
            .filter(|d| d["type"] == "effort")
            .and_then(|d| d["value"].as_str())
            .or_else(|| self.efforts().first().map(String::as_str))
            .unwrap_or("")
    }
    pub fn choice(&self, effort: &str) -> Result<(), String> {
        if self.efforts().is_empty() && effort.is_empty() || self.efforts().iter().any(|e| e == effort) {
            Ok(())
        } else {
            Err("OpenCode Go effort is no longer available; select it again in Settings".into())
        }
    }
    pub fn validate(&self) -> Result<(), String> {
        bounded(&serde_json::to_value(self).map_err(|_| INVALID)?, 0, &mut 0)?;
        if !selector(&self.id)
            || self.name.is_empty()
            || self.name.len() > 256
            || self.kind != "chat"
            || self.release_date.len() > 32
            || self.family.as_ref().is_some_and(|s| s.len() > 160)
            || self
                .status
                .as_deref()
                .is_some_and(|s| !matches!(s, "active" | "alpha" | "beta" | "deprecated"))
            || !self.capabilities.tools.supported
            || self.limit.context == 0
            || self.limit.output == 0
            || self.limit.context > 100_000_000
            || self.limit.output > 100_000_000
            || self.limit.input.is_some_and(|n| n == 0 || n > 100_000_000)
            || self.modalities.input.is_empty()
            || self.modalities.output.is_empty()
            || self.modalities.input.len() > 32
            || self.modalities.output.len() > 32
            || self
                .modalities
                .input
                .iter()
                .chain(&self.modalities.output)
                .any(|s| s.len() > 128)
            || self.api.len() != 1
        {
            return Err(INVALID.into());
        }
        let efforts = self.efforts();
        if efforts.len() > 32
            || efforts.iter().any(|e| !selector(e) || e == "default")
            || efforts.iter().collect::<BTreeSet<_>>().len() != efforts.len()
            || (!self.capabilities.reasoning.supported && !efforts.is_empty())
        {
            return Err(INVALID.into());
        }
        if let Some(default) = &self.capabilities.reasoning.default {
            let fields = default.as_object().ok_or(INVALID)?;
            if fields.len() != 2 || !fields.contains_key("value") {
                return Err(INVALID.into());
            }
            match default["type"].as_str() {
                Some("effort")
                    if default["value"]
                        .as_str()
                        .is_some_and(|e| efforts.iter().any(|v| v == e)) => {}
                Some("toggle") if self.capabilities.reasoning.toggle == Some(true) && default["value"].is_boolean() => {
                }
                Some("budget") | Some("budget_tokens")
                    if default["value"].as_u64().is_some_and(|n| {
                        self.capabilities.reasoning.budget.as_ref().is_some_and(|budget| {
                            n <= 100_000_000
                                && budget.min.is_none_or(|min| n >= min)
                                && budget.max.is_none_or(|max| n <= max)
                        })
                    }) => {}
                _ => return Err(INVALID.into()),
            }
        }
        if let Some(budget) = &self.capabilities.reasoning.budget {
            // Reasoning budgets and visible output limits are independent provider controls.
            if budget.min.is_some_and(|n| n > 100_000_000)
                || budget.max.is_some_and(|n| n > 100_000_000)
                || matches!((budget.min, budget.max), (Some(a), Some(b)) if a > b)
            {
                return Err(INVALID.into());
            }
        }
        for (protocol, api) in &self.api {
            let (path, aisdk, sdk) = match protocol.as_str() {
                "chat-completions" => (
                    "/inference/go/openai/v1",
                    "@ai-sdk/openai-compatible",
                    "@opencode/ai/providers/openai-compatible",
                ),
                "responses" => (
                    "/inference/go/openai/v1",
                    "@ai-sdk/openai",
                    "@opencode/ai/providers/openai/responses",
                ),
                "messages" => (
                    "/inference/go/anthropic/v1",
                    "@ai-sdk/anthropic",
                    "@opencode/ai/providers/anthropic",
                ),
                "generate-content" => (
                    "/inference/go/google/v1beta",
                    "@ai-sdk/google",
                    "@opencode/ai/providers/google",
                ),
                _ => return Err(INVALID.into()),
            };
            // Literal comparison also excludes ports, credentials, query, fragments and redirects.
            if api.base_url != format!("https://opencode.ai{path}")
                || api.sdk.aisdk != aisdk
                || api.sdk.opencodeai != sdk
            {
                return Err(INVALID.into());
            }
            if let Some(c) = &api.compatibility {
                // These are data tokens, never settings/body/header objects. The guard also
                // requires the CLI normalizer to preserve each compatibility field exactly.
                if [
                    c.reasoning_field.as_deref(),
                    c.max_tokens_field.as_deref(),
                    c.media_input.as_deref(),
                ]
                .into_iter()
                .flatten()
                .any(|s| !selector(s))
                {
                    return Err(INVALID.into());
                }
            }
        }
        Ok(())
    }
    pub fn view(&self) -> Value {
        json!({"id":self.id,"label":self.name,"efforts":self.efforts(),"defaultEffort":self.default_effort()})
    }
}

impl Catalogue {
    pub fn parse(bytes: &[u8]) -> Result<Self, String> {
        if bytes.len() > MAX_BYTES {
            return Err(INVALID.into());
        }
        let value: Value = serde_json::from_slice(bytes).map_err(|_| INVALID)?;
        bounded(&value, 0, &mut 0)?;
        if value["object"] != "list" {
            return Err(INVALID.into());
        }
        let data = value["data"].as_array().ok_or(INVALID)?;
        if data.is_empty() || data.len() > 512 {
            return Err(INVALID.into());
        }
        let mut seen = BTreeSet::new();
        let mut models = Vec::new();
        for source in data {
            let Some(id) = source["id"].as_str().filter(|id| selector(id)) else {
                continue;
            };
            if !seen.insert(id) {
                return Err(INVALID.into());
            }
            if source["type"] != "chat" || source["capabilities"]["tools"]["supported"] != true {
                continue;
            }
            let Some(api) = source["api"].as_object() else {
                continue;
            };
            // Do not guess a transport, and never pass arbitrary metadata/configuration through.
            let supported = api
                .iter()
                .filter(|(key, _)| {
                    matches!(
                        key.as_str(),
                        "chat-completions" | "responses" | "messages" | "generate-content"
                    )
                })
                .collect::<Vec<_>>();
            if supported.is_empty() {
                continue;
            }
            if supported.len() != 1 {
                // No transport priority is implied by a JSON object's sorted keys. Exclude
                // this ambiguous model without hiding unrelated supported models.
                continue;
            }
            let (protocol, transport) = supported[0];
            let Ok(model) = serde_json::from_value::<Model>(json!({
                "id":id,"name":source["name"],"type":"chat","release_date":source["release_date"],"family":source["family"],"status":source["status"],
                "modalities":source["modalities"],"limit":source["limit"],
                "capabilities":{"tools":{"supported":true},"reasoning":source["capabilities"]["reasoning"],"temperature":source["capabilities"]["temperature"]},
                "api":{(protocol):transport}
            })) else {
                continue;
            };
            if model.validate().is_err() {
                continue;
            }
            models.push(model);
        }
        if models.is_empty() {
            return Err("OpenCode Go catalogue has no supported tool models".into());
        }
        models.sort_by(|a, b| a.id.cmp(&b.id));
        Ok(Self {
            id: digest(&models)?,
            models,
        })
    }
    pub fn select(&self, model: &str, effort: &str) -> Result<Snapshot, String> {
        let model = self
            .models
            .iter()
            .find(|entry| entry.id == model)
            .ok_or("OpenCode Go model is no longer available; select it again in Settings")?
            .clone();
        model.choice(effort)?;
        let id = digest(&(self.id.as_str(), &model, effort))?;
        Ok(Snapshot {
            catalogue_id: self.id.clone(),
            id,
            model,
            effort: effort.into(),
        })
    }
}

impl Snapshot {
    pub fn validate(&self, model: &str, effort: &str) -> Result<(), String> {
        self.model.validate()?;
        self.model.choice(&self.effort)?;
        if self.model.id != model
            || self.effort != effort
            || self.catalogue_id.len() != 64
            || !self.catalogue_id.bytes().all(|b| b.is_ascii_hexdigit())
            || digest(&(self.catalogue_id.as_str(), &self.model, self.effort.as_str()))? != self.id
        {
            return Err("OpenCode Go snapshot identity mismatch".into());
        }
        Ok(())
    }
    pub fn cli_catalogue(&self) -> Result<Value, String> {
        self.validate(&self.model.id, &self.effort)?;
        Ok(
            json!({"opencode-go":{"id":"opencode-go","name":"OpenCode Go","env":[],"models":{(self.model.id.clone()):self.model}}}),
        )
    }
}

pub(crate) fn download() -> Result<Catalogue, String> {
    #[cfg(test)]
    if let Some(catalogue) = TEST_CATALOGUE.lock().map_err(|_| INVALID)?.clone() {
        return catalogue;
    }
    let client = reqwest::blocking::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_secs(3))
        .timeout(Duration::from_secs(10))
        .user_agent("OpenCodeReview/Go-catalogue")
        .build()
        .map_err(|_| "OpenCode Go discovery is unavailable")?;
    download_from(&client, SOURCE)
}

fn download_from(client: &reqwest::blocking::Client, source: &str) -> Result<Catalogue, String> {
    let response = client
        .get(source)
        .header("Cache-Control", "no-store")
        .header("Accept", "application/json")
        .send()
        .map_err(|_| "OpenCode Go discovery failed or timed out")?;
    if response.status() != reqwest::StatusCode::OK || response.content_length().is_some_and(|n| n > MAX_BYTES as u64) {
        return Err("OpenCode Go discovery rejected its response".into());
    }
    let mut bytes = Vec::new();
    response
        .take(MAX_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "OpenCode Go discovery body failed or timed out")?;
    Catalogue::parse(&bytes)
}

enum Status {
    Empty,
    Loading,
    Ready(Catalogue),
    Error(String),
}

/// One process-local discovery transaction per server. Refresh never serves old inventory.
pub(crate) struct Discovery {
    state: std::sync::Mutex<Status>,
    fetch: Box<dyn Fn() -> Result<Catalogue, String> + Send + Sync>,
}

impl Discovery {
    pub fn new() -> std::sync::Arc<Self> {
        #[cfg(test)]
        return Self::with_fetch(|| Err("No synthetic Go catalogue installed for this test".into()));
        #[cfg(not(test))]
        Self::with_fetch(download)
    }
    fn with_fetch(fetch: impl Fn() -> Result<Catalogue, String> + Send + Sync + 'static) -> std::sync::Arc<Self> {
        std::sync::Arc::new(Self {
            state: std::sync::Mutex::new(Status::Empty),
            fetch: Box::new(fetch),
        })
    }
    pub fn queue(self: &std::sync::Arc<Self>, initial: bool) -> Result<(), String> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| "OpenCode Go discovery state is unavailable")?;
        if matches!(*state, Status::Loading) || initial && !matches!(*state, Status::Empty) {
            return Ok(());
        }
        *state = Status::Loading;
        let discovery = self.clone();
        if std::thread::Builder::new()
            .name("go-catalogue".into())
            .spawn(move || {
                let result = (discovery.fetch)();
                if let Ok(mut state) = discovery.state.lock() {
                    *state = match result {
                        Ok(catalogue) => Status::Ready(catalogue),
                        Err(error) => Status::Error(error),
                    };
                }
            })
            .is_err()
        {
            *state = Status::Error("OpenCode Go discovery could not start".into());
            return Err("OpenCode Go discovery could not start".into());
        }
        Ok(())
    }
    pub fn view(&self, preference: Option<&(String, String)>) -> Result<Value, String> {
        let state = self
            .state
            .lock()
            .map_err(|_| "OpenCode Go discovery state is unavailable")?;
        let (status, id, error, models, valid) = match &*state {
            Status::Empty => ("empty", None, None, Vec::new(), false),
            Status::Loading => ("loading", None, None, Vec::new(), false),
            Status::Error(error) => ("error", None, Some(error.as_str()), Vec::new(), false),
            Status::Ready(catalogue) => (
                "ready",
                Some(catalogue.id.as_str()),
                None,
                catalogue.models.iter().map(Model::view).collect(),
                preference.is_some_and(|(model, effort)| catalogue.select(model, effort).is_ok()),
            ),
        };
        let (model, effort) = preference.map(|(m, e)| (m.as_str(), e.as_str())).unwrap_or(("", ""));
        Ok(
            json!({"provider":"opencode","name":"OpenCode Go","model":model,"effort":effort,"label":model,
            "models":models,"catalogue":{"status":status,"id":id,"error":error},"selectionValid":valid}),
        )
    }
    pub fn with_current<T>(&self, id: &str, work: impl FnOnce(&Catalogue) -> Result<T, String>) -> Result<T, String> {
        let state = self
            .state
            .lock()
            .map_err(|_| "OpenCode Go discovery state is unavailable")?;
        match &*state {
            Status::Ready(catalogue) if catalogue.id == id => work(catalogue),
            _ => Err("OpenCode Go catalogue changed or is unavailable; refresh and confirm the selection".into()),
        }
    }
    #[cfg(test)]
    pub(crate) fn install(&self, catalogue: Catalogue) {
        *self.state.lock().unwrap() = Status::Ready(catalogue);
    }
    #[cfg(test)]
    pub(crate) fn test_source(
        fetch: impl Fn() -> Result<Catalogue, String> + Send + Sync + 'static,
    ) -> std::sync::Arc<Self> {
        Self::with_fetch(fetch)
    }
}

#[cfg(test)]
static TEST_CATALOGUE: std::sync::Mutex<Option<Result<Catalogue, String>>> = std::sync::Mutex::new(None);
#[cfg(test)]
pub(crate) fn install_test_source(catalogue: Catalogue) {
    *TEST_CATALOGUE.lock().unwrap() = Some(Ok(catalogue));
}

#[cfg(test)]
pub(crate) fn install_test_error(error: &str) {
    *TEST_CATALOGUE.lock().unwrap() = Some(Err(error.into()));
}

#[cfg(test)]
pub(crate) fn fixture(id: &str, efforts: &[&str], protocol: &str) -> Catalogue {
    let (url, aisdk, sdk) = match protocol {
        "responses" => (
            "https://opencode.ai/inference/go/openai/v1",
            "@ai-sdk/openai",
            "@opencode/ai/providers/openai/responses",
        ),
        "messages" => (
            "https://opencode.ai/inference/go/anthropic/v1",
            "@ai-sdk/anthropic",
            "@opencode/ai/providers/anthropic",
        ),
        "generate-content" => (
            "https://opencode.ai/inference/go/google/v1beta",
            "@ai-sdk/google",
            "@opencode/ai/providers/google",
        ),
        _ => (
            "https://opencode.ai/inference/go/openai/v1",
            "@ai-sdk/openai-compatible",
            "@opencode/ai/providers/openai-compatible",
        ),
    };
    Catalogue::parse(
        &serde_json::to_vec(&json!({"object":"list","data":[{
            "id":id,"name":format!("Discovered {id}"),"type":"chat","release_date":"2026-11-08",
            "modalities":{"input":["text"],"output":["text"]},"limit":{"context":65536,"output":8192},
            "capabilities":{"tools":{"supported":true},"reasoning":{"supported":true,"effort":efforts}},
            "api":{(protocol):{"base_url":url,"sdk":{"aisdk":aisdk,"opencodeai":sdk}}}
        }]}))
        .unwrap(),
    )
    .unwrap()
}

#[cfg(test)]
#[path = "go_catalogue_tests.rs"]
mod tests;
