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

pub(crate) const PROVIDERS: &[(&str, &str)] =
    &[("claude", "Claude"), ("codex", "ChatGPT"), ("opencode", "OpenCode Go")];

pub(crate) fn models(provider: &str) -> &'static [EngineModel] {
    match provider {
        "claude" => CLAUDE,
        "codex" => CODEX,
        _ => &[],
    }
}

/// Static provider defaults. Go requires an explicit catalogue-confirmed preference.
pub(crate) fn default_engine(provider: &str) -> (String, String) {
    models(provider)
        .first()
        .map(|model| (model.id.to_string(), model.default_effort.to_string()))
        .unwrap_or_default()
}

/// Checks `model` and `effort` for `provider` and returns them, an empty effort becoming the
/// model's default.
pub(crate) fn resolve(provider: &str, model: &str, effort: &str) -> Result<(String, String), String> {
    if !PROVIDERS.iter().any(|(id, _)| *id == provider) {
        return Err(format!("{provider} is not an enabled review provider"));
    }
    resolve_model(provider, model, effort)
}

/// Static model validation. Go uses its independently validated frozen catalogue instead.
pub(crate) fn resolve_model(provider: &str, model: &str, effort: &str) -> Result<(String, String), String> {
    if provider == "opencode" {
        return Err("OpenCode Go requires a fresh catalogue and explicit selection".into());
    }
    let entry = models(provider)
        .iter()
        .find(|entry| entry.id == model)
        .ok_or_else(|| format!("{model} is not a model OpenCodeReview runs on {provider}"))?;
    let effort = if effort.is_empty() {
        entry.default_effort
    } else {
        effort
    };
    if entry.efforts.is_empty() && effort.is_empty() || entry.efforts.contains(&effort) {
        Ok((entry.id.to_string(), effort.to_string()))
    } else {
        Err(format!("{} doesn't take a {effort} effort", entry.label))
    }
}

/// "Claude · Opus 5.5 · medium", as reviews show the engine they ran on.
pub(crate) fn label(provider: &str, model: &str, effort: &str) -> String {
    let name = PROVIDERS
        .iter()
        .find(|(id, _)| *id == provider)
        .map_or(provider, |(_, name)| *name);
    let model = models(provider)
        .iter()
        .find(|entry| entry.id == model)
        .map_or(model, |entry| entry.label);
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
    fn static_providers_default_to_a_valid_engine() {
        for provider in ["claude", "codex"] {
            let (model, effort) = default_engine(provider);
            assert_eq!(resolve(provider, &model, &effort).unwrap(), (model, effort));
        }
    }

    #[test]
    fn go_has_no_static_inventory_or_implicit_default() {
        assert!(models("opencode").is_empty());
        assert_eq!(default_engine("opencode"), (String::new(), String::new()));
        assert!(resolve("opencode", "new-model", "new-effort").is_err());
    }

    #[test]
    fn resolve_rejects_unknown_models_and_efforts() {
        assert_eq!(
            resolve("claude", "claude-sonnet-5-5", "").unwrap(),
            ("claude-sonnet-5-5".to_string(), "medium".to_string())
        );
        assert!(resolve("claude", "gpt-5.5", "medium").is_err());
        assert!(resolve("codex", "gpt-5.5", "ultra").is_err());
    }

    #[test]
    fn labels_name_the_provider_model_and_effort() {
        assert_eq!(label("claude", "claude-opus-5-5", "high"), "Claude · Opus 5.5 · high");
    }
}
