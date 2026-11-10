//! The model and reasoning effort each provider reviews with. Settings keep one choice per
//! provider, so a review that moves to another provider's account uses that provider's choice.

pub(crate) struct EngineModel {
    pub id: &'static str,
    pub label: &'static str,
    /// Reasoning efforts the CLI accepts for this model; empty when the provider has none.
    pub efforts: &'static [&'static str],
    pub default_effort: &'static str,
}

const CLAUDE_EFFORTS: &[&str] = &["low", "medium", "high", "xhigh", "max"];
const CODEX_EFFORTS: &[&str] = &["low", "medium", "high", "xhigh", "max", "ultra"];
const CODEX_EFFORTS_MAX: &[&str] = &["low", "medium", "high", "xhigh", "max"];
const CODEX_EFFORTS_XHIGH: &[&str] = &["low", "medium", "high", "xhigh"];

/// The first entry of each list is the provider's default.
const CLAUDE: &[EngineModel] = &[
    model("claude-opus-5-5", "Opus 5.5", CLAUDE_EFFORTS, "medium"),
    model("claude-sonnet-5-5", "Sonnet 5.5", CLAUDE_EFFORTS, "medium"),
    model("claude-haiku-5-5", "Haiku 5.5", CLAUDE_EFFORTS, "medium"),
];
/// Codex's `debug models` catalogue for ChatGPT plans; `default` leaves the choice to Codex.
const CODEX: &[EngineModel] = &[
    model("default", "Codex default", CODEX_EFFORTS_XHIGH, "medium"),
    model("gpt-6-astra", "GPT-6 Astra", CODEX_EFFORTS, "low"),
    model("gpt-6.1-sol", "GPT-6.1 Sol", CODEX_EFFORTS, "low"),
    model("gpt-6-sol", "GPT-6 Sol", CODEX_EFFORTS, "medium"),
    model("gpt-6-luna", "GPT-6 Luna", CODEX_EFFORTS_MAX, "medium"),
    model("gpt-5.6-sol", "GPT-5.6 Sol", CODEX_EFFORTS, "low"),
    model("gpt-5.6-terra", "GPT-5.6 Terra", CODEX_EFFORTS, "medium"),
    model("gpt-5.6-luna", "GPT-5.6 Luna", CODEX_EFFORTS_MAX, "medium"),
    model("gpt-5.5", "GPT-5.5", CODEX_EFFORTS_XHIGH, "medium"),
];
const fn model(
    id: &'static str,
    label: &'static str,
    efforts: &'static [&'static str],
    default_effort: &'static str,
) -> EngineModel {
    EngineModel {
        id,
        label,
        efforts,
        default_effort,
    }
}

pub(crate) const PROVIDERS: &[(&str, &str)] = &[("claude", "Claude"), ("codex", "ChatGPT"), ("opencode", "OpenCode")];

/// A model `provider` reviews with, from the lists above or, for OpenCode, from opencode.ai.
pub(crate) type Model = crate::providers::opencode_catalogue::Model;

fn listed(models: &[EngineModel]) -> Vec<Model> {
    models
        .iter()
        .map(|model| Model {
            id: model.id.to_string(),
            label: model.label.to_string(),
            efforts: model.efforts.iter().map(|effort| effort.to_string()).collect(),
            default_effort: model.default_effort.to_string(),
        })
        .collect()
}

/// The OpenCode plans of the accounts reviews can run on (`Database::opencode_plans`), or None
/// without one. Only OpenCode's list depends on it.
pub(crate) type Plans = Option<Vec<String>>;

/// `provider`'s models, the default first. OpenCode's come from opencode.ai and are empty while
/// its list can't be read.
pub(crate) fn models(provider: &str, plans: &Plans) -> Vec<Model> {
    match provider {
        "claude" => listed(CLAUDE),
        "codex" => listed(CODEX),
        "opencode" => opencode_models(plans, true),
        _ => vec![],
    }
}

/// OpenCode's models that `plans` pay for: Go models need a workspace with Go, Zen models one with
/// Zen. Without an OpenCode account nothing runs on its models, so opencode.ai is not asked and
/// the list is the one already read, whole; `refresh` false never asks either.
fn opencode_models(plans: &Plans, refresh: bool) -> Vec<Model> {
    use crate::providers::opencode_catalogue as catalogue;
    let Some(plans) = plans else {
        return catalogue::cached();
    };
    let models = if refresh {
        catalogue::models().unwrap_or_default()
    } else {
        catalogue::cached()
    };
    models
        .into_iter()
        .filter(|model| catalogue::plan(&model.id).is_some_and(|plan| plans.iter().any(|known| known == plan)))
        .collect()
}

fn first(models: Vec<Model>) -> (String, String) {
    models
        .into_iter()
        .next()
        .map(|model| (model.id, model.default_effort))
        .unwrap_or_default()
}

/// The provider's default model and effort.
pub(crate) fn default_engine(provider: &str, plans: &Plans) -> (String, String) {
    first(models(provider, plans))
}

/// [`default_engine`] without asking opencode.ai: OpenCode's default comes from the list already
/// read, and is empty until one was.
pub(crate) fn default_engine_offline(provider: &str, plans: &Plans) -> (String, String) {
    if provider == "opencode" {
        first(opencode_models(plans, false))
    } else {
        default_engine(provider, plans)
    }
}

fn check(entry: &Model, effort: &str) -> Result<(String, String), String> {
    let effort = if effort.is_empty() {
        entry.default_effort.as_str()
    } else {
        effort
    };
    if entry.efforts.is_empty() && effort.is_empty() || entry.efforts.iter().any(|known| known == effort) {
        Ok((entry.id.clone(), effort.to_string()))
    } else {
        Err(format!("{} doesn't take a {effort} effort", entry.label))
    }
}

/// Checks `model` and `effort` for `provider` and returns them, an empty effort becoming the
/// model's default. Without an OpenCode account, or while OpenCode's list can't be read, an
/// OpenCode choice is checked only for its shape, as runners do.
pub(crate) fn resolve(provider: &str, model: &str, effort: &str, plans: &Plans) -> Result<(String, String), String> {
    if provider == "opencode" {
        if plans.is_none() {
            return checked_shape(model, effort);
        }
        if let Err(error) = crate::providers::opencode_catalogue::models() {
            return checked_shape(model, effort).map_err(|_| error);
        }
    }
    let entry = models(provider, plans)
        .into_iter()
        .find(|entry| entry.id == model)
        .ok_or_else(|| format!("{model} is not a model OpenCodeReview runs on {provider}"))?;
    check(&entry, effort)
}

fn checked_shape(model: &str, effort: &str) -> Result<(String, String), String> {
    if crate::providers::opencode_catalogue::selector_ok(model, effort) {
        Ok((model.to_string(), effort.to_string()))
    } else {
        Err(if model.is_empty() {
            "OpenCode's model list could not be read from opencode.ai; try again when it is reachable".to_string()
        } else {
            format!("{model} is not an OpenCode model")
        })
    }
}

/// [`resolve`] for runners, which can't reach opencode.ai: OpenCode choices, which the server
/// already checked against the list, are checked for their shape only.
pub(crate) fn resolve_offline(provider: &str, model: &str, effort: &str) -> Result<(String, String), String> {
    if provider == "opencode" {
        checked_shape(model, effort)
    } else {
        resolve(provider, model, effort, &None)
    }
}

/// "Claude · Opus 5.5 · medium", as reviews show the engine they ran on.
pub(crate) fn label(provider: &str, model: &str, effort: &str) -> String {
    let name = PROVIDERS
        .iter()
        .find(|(id, _)| *id == provider)
        .map_or(provider, |(_, name)| *name);
    let known = if provider == "opencode" {
        crate::providers::opencode_catalogue::cached()
    } else {
        models(provider, &None)
    };
    let model = known
        .iter()
        .find(|entry| entry.id == model)
        .map_or(model, |entry| entry.label.as_str());
    [name, model, effort]
        .into_iter()
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join(" · ")
}

/// The settings key holding `provider`'s choice.
pub(crate) fn setting_key(provider: &str) -> String {
    format!("review_engine_{provider}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_listed_provider_defaults_to_a_valid_engine() {
        for provider in ["claude", "codex"] {
            let (model, effort) = default_engine(provider, &None);
            assert_eq!(resolve(provider, &model, &effort, &None).unwrap(), (model, effort));
        }
    }

    #[test]
    fn resolve_rejects_unknown_models_and_efforts() {
        assert_eq!(
            resolve("claude", "claude-sonnet-5-5", "", &None).unwrap(),
            ("claude-sonnet-5-5".to_string(), "medium".to_string())
        );
        assert!(resolve("claude", "gpt-5.5", "medium", &None).is_err());
        assert!(resolve("codex", "gpt-5.5", "ultra", &None).is_err());
        // Tests never read opencode.ai, so OpenCode choices are checked for their shape.
        // With or without an OpenCode account.
        for plans in [None, Some(vec!["go".to_string()])] {
            assert_eq!(
                resolve("opencode", "opencode-go/glm-5.3", "high", &plans).unwrap(),
                ("opencode-go/glm-5.3".to_string(), "high".to_string())
            );
            assert!(resolve("opencode", "anthropic/claude-opus-5-5", "", &plans).is_err());
            assert_eq!(default_engine("opencode", &plans), (String::new(), String::new()));
            assert_eq!(
                default_engine_offline("opencode", &plans),
                (String::new(), String::new())
            );
        }
        assert!(resolve_offline("opencode", "opencode/x#y", "").is_err());
        assert_eq!(default_engine_offline("claude", &None), default_engine("claude", &None));
    }

    #[test]
    fn labels_name_the_provider_model_and_effort() {
        assert_eq!(label("claude", "claude-opus-5-5", "high"), "Claude · Opus 5.5 · high");
        assert_eq!(
            label("opencode", "opencode-go/glm-5.3", "high"),
            "OpenCode · opencode-go/glm-5.3 · high"
        );
    }
}
