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

/// `provider`'s models, the default first. OpenCode's come from opencode.ai and are empty while
/// its list can't be read.
pub(crate) fn models(provider: &str) -> Vec<Model> {
    match provider {
        "claude" => listed(CLAUDE),
        "codex" => listed(CODEX),
        "opencode" => opencode_models(),
        _ => vec![],
    }
}

/// OpenCode's models that the plans of the connected OpenCode accounts pay for: Go models need a
/// workspace with Go, Zen models one with Zen. Without an OpenCode account, every model.
fn opencode_models() -> Vec<Model> {
    let models = crate::providers::opencode_catalogue::models().unwrap_or_default();
    match crate::accounts::opencode_plans() {
        Some(plans) => models
            .into_iter()
            .filter(|model| {
                crate::providers::opencode_catalogue::plan(&model.id)
                    .is_some_and(|plan| plans.iter().any(|known| known == plan))
            })
            .collect(),
        None => models,
    }
}

/// The provider's default model and effort.
pub(crate) fn default_engine(provider: &str) -> (String, String) {
    models(provider)
        .into_iter()
        .next()
        .map(|model| (model.id, model.default_effort))
        .unwrap_or_default()
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
/// model's default. While OpenCode's list can't be read, an OpenCode choice is checked only for
/// its shape, as runners do.
pub(crate) fn resolve(provider: &str, model: &str, effort: &str) -> Result<(String, String), String> {
    if provider == "opencode" {
        if let Err(error) = crate::providers::opencode_catalogue::models() {
            return checked_shape(model, effort).map_err(|_| error);
        }
    }
    let entry = models(provider)
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
        resolve(provider, model, effort)
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
        models(provider)
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
            let (model, effort) = default_engine(provider);
            assert_eq!(resolve(provider, &model, &effort).unwrap(), (model, effort));
        }
    }

    #[test]
    fn resolve_rejects_unknown_models_and_efforts() {
        assert_eq!(
            resolve("claude", "claude-sonnet-5-5", "").unwrap(),
            ("claude-sonnet-5-5".to_string(), "medium".to_string())
        );
        assert!(resolve("claude", "gpt-5.5", "medium").is_err());
        assert!(resolve("codex", "gpt-5.5", "ultra").is_err());
        // Tests never read opencode.ai, so OpenCode choices are checked for their shape.
        assert_eq!(
            resolve("opencode", "opencode-go/glm-5.3", "high").unwrap(),
            ("opencode-go/glm-5.3".to_string(), "high".to_string())
        );
        assert!(resolve("opencode", "anthropic/claude-opus-5-5", "").is_err());
        assert!(resolve_offline("opencode", "opencode/x#y", "").is_err());
        assert_eq!(default_engine("opencode"), (String::new(), String::new()));
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
