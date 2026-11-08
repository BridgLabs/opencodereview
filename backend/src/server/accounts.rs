use super::*;

/// Serializes credential replacement and deletion across server instances.
pub(crate) const ACCOUNT_CREDENTIAL_WRITE: i64 = 0x4f4352474f4b4559;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct AccountUpdate {
    label: Option<String>,
    enabled: Option<bool>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct AccountOrder {
    ids: Vec<i64>,
}

/// Accounts with whether their credential is present, so the UI can ask for a new sign-in.
pub(crate) fn list_accounts(state: &SharedState) -> Result<serde_json::Value, String> {
    let accounts = with_db_read_raw(state, |db| db.list_ai_accounts())?;
    Ok(serde_json::Value::Array(
        accounts
            .into_iter()
            .map(|account| {
                let signed_in = crate::accounts::has_credential(&account.provider, account.id)
                    && (account.provider != "opencode" || account.status != "auth_failed");
                let mut value = serde_json::to_value(&account).unwrap_or_default();
                value["signedIn"] = signed_in.into();
                value
            })
            .collect(),
    ))
}

pub(crate) fn update_account(request: &Request, state: &SharedState, id: i64) -> Result<serde_json::Value, String> {
    require_trusted_origin(request)?;
    let update: AccountUpdate = serde_json::from_slice(&request.body).map_err(|_| "Invalid account update")?;
    with_db_raw(state, |db| {
        db.ai_account(id)?;
        if let Some(label) = &update.label {
            db.rename_ai_account(id, label)?;
        }
        if let Some(enabled) = update.enabled {
            db.set_ai_account_enabled(id, enabled)?;
        }
        Ok(())
    })?;
    list_accounts(state)
}

pub(crate) fn reorder_accounts(request: &Request, state: &SharedState) -> Result<serde_json::Value, String> {
    require_trusted_origin(request)?;
    let order: AccountOrder = serde_json::from_slice(&request.body).map_err(|_| "Invalid account order")?;
    with_db_raw(state, |db| db.reorder_ai_accounts(&order.ids))?;
    list_accounts(state)
}

pub(crate) fn delete_account(request: &Request, state: &SharedState, id: i64) -> Result<serde_json::Value, String> {
    require_trusted_origin(request)?;
    state.config.db.run_exclusive_after(ACCOUNT_CREDENTIAL_WRITE, || {
        with_db_raw(state, |db| {
            let account = db.ai_account(id)?;
            db.mark_ai_account_auth_failed(id, "Account disconnection did not complete")?;
            if account.provider == "opencode" {
                crate::accounts::sandbox_sync::revoke_opencode(id)?;
            }
            crate::accounts::remove(id)?;
            db.delete_ai_account(id)
        })?;
        crate::accounts::usage::forget(id);
        Ok::<_, String>(())
    })??;
    list_accounts(state)
}

/// What is left of each signed-in account's plan, read from every provider at once.
pub(crate) fn account_usage(state: &SharedState) -> Result<serde_json::Value, String> {
    let accounts = with_db_read_raw(state, |db| db.list_ai_accounts())?;
    let reads = accounts
        .into_iter()
        .filter(|account| crate::accounts::has_credential(&account.provider, account.id))
        .map(|account| std::thread::spawn(move || crate::accounts::usage::usage(account.id, &account.provider)))
        .collect::<Vec<_>>();
    Ok(serde_json::Value::Array(
        reads.into_iter().filter_map(|read| read.join().ok()).collect(),
    ))
}

/// The release in `--version` output: `2.1.292 (Claude Code)`, `codex-cli 0.163.0`, `0.63.0` or `1.18.35`.
pub(crate) fn cli_version(output: &str) -> Option<String> {
    output
        .split_whitespace()
        .map(|word| word.strip_prefix('v').unwrap_or(word))
        .find(|word| word.contains('.') && word.bytes().all(|byte| byte.is_ascii_digit() || byte == b'.'))
        .map(str::to_string)
}

/// Each provider's model and effort choices and the ones reviews use now.
pub(crate) fn review_engines(state: &SharedState) -> Result<serde_json::Value, String> {
    use crate::providers::engines;
    state.go_catalogue.queue(true)?;
    let stored = with_db_read_raw(state, |db| db.setting("review_engine_opencode"))?;
    // Keep discovery usable to repair a malformed imported preference. This never saves a
    // replacement or selects a default; preparation still uses the strict database reader.
    let preference = stored
        .and_then(|raw| serde_json::from_str::<serde_json::Value>(&raw).ok())
        .and_then(|value| {
            Some((
                value["model"].as_str()?.to_string(),
                value["effort"].as_str()?.to_string(),
            ))
        });
    let go = state.go_catalogue.view(preference.as_ref())?;
    with_db_read_raw(state, |db| {
        engines::PROVIDERS
            .iter()
            .map(|(provider, name)| {
                if *provider == "opencode" {
                    return Ok(go.clone());
                }
                let (model, effort) = db.review_engine(provider)?;
                let models = engines::models(provider)
                    .iter()
                    .map(|entry| {
                        serde_json::json!({
                            "id": entry.id,
                            "label": entry.label,
                            "efforts": entry.efforts,
                            "defaultEffort": entry.default_effort,
                        })
                    })
                    .collect::<Vec<_>>();
                Ok(serde_json::json!({
                    "provider": provider,
                    "name": name,
                    "model": model,
                    "effort": effort,
                    "label": engines::label(provider, &model, &effort),
                    "models": models,
                }))
            })
            .collect::<Result<Vec<_>, String>>()
            .map(serde_json::Value::Array)
    })
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct ReviewEngineInput {
    provider: String,
    model: String,
    #[serde(default)]
    effort: String,
    #[serde(rename = "catalogueId")]
    catalogue_id: Option<String>,
}

/// Saves the model and effort `provider` reviews with; running reviews pick it up at their next stage.
pub(crate) fn save_review_engine(request: &Request, state: &SharedState) -> Result<serde_json::Value, String> {
    let input: ReviewEngineInput = serde_json::from_slice(&request.body).map_err(|error| error.to_string())?;
    if input.provider == "opencode" {
        require_auth(request, state)?;
        require_trusted_origin(request)?;
        let id = input
            .catalogue_id
            .as_deref()
            .ok_or("Confirm the current OpenCode Go catalogue before saving")?;
        state.go_catalogue.with_current(id, |catalogue| {
            let selected = catalogue.select(&input.model, &input.effort)?;
            with_db_raw(state, |db| db.save_go_preference(&selected))
        })?;
        return review_engines(state);
    }
    with_db_raw(state, |db| {
        db.save_review_engine(&input.provider, &input.model, &input.effort)
    })?;
    review_engines(state)
}

pub(crate) fn refresh_go_catalogue(request: &Request, state: &SharedState) -> Result<serde_json::Value, String> {
    require_auth(request, state)?;
    require_trusted_origin(request)?;
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Refresh {}
    let _: Refresh = serde_json::from_slice(&request.body).map_err(|_| "Invalid OpenCode Go refresh request")?;
    state.go_catalogue.queue(false)?;
    Ok(serde_json::json!({"ok":true}))
}

/// Each review provider: whether its CLI runs here, its version, and how many accounts are ready.
pub(crate) fn ai_providers(state: &SharedState) -> Result<serde_json::Value, String> {
    let accounts = with_db_read_raw(state, |db| db.list_ai_accounts())?;
    Ok(serde_json::Value::Array(
        crate::providers::engines::PROVIDERS
            .iter()
            .copied()
            .map(|(provider, name)| {
                // A scratch home keeps CLIs that log or create state on every start out of the data volume.
                let home = std::env::temp_dir().join("ocr-cli-version");
                let home = home.to_string_lossy();
                let homes = [
                    "HOME",
                    "XDG_DATA_HOME",
                    "XDG_CONFIG_HOME",
                    "XDG_CACHE_HOME",
                    "XDG_STATE_HOME",
                ]
                .map(|name| format!("{name}={home}"));
                let args = homes
                    .iter()
                    .map(String::as_str)
                    .chain([provider, "--version"])
                    .collect::<Vec<_>>();
                let version = command_output_timeout("env", &args, Duration::from_secs(10))
                    .ok()
                    .filter(|output| output.status.success())
                    .and_then(|output| cli_version(&String::from_utf8_lossy(&output.stdout)))
                    .filter(|version| provider != "opencode" || version == crate::providers::opencode::VERSION);
                let mine = accounts
                    .iter()
                    .filter(|account| account.provider == provider)
                    .collect::<Vec<_>>();
                let ready = mine
                    .iter()
                    .filter(|account| {
                        account.enabled
                            && account.status != "auth_failed"
                            && crate::accounts::has_credential(provider, account.id)
                    })
                    .collect::<Vec<_>>();
                serde_json::json!({
                    "provider": provider,
                    "name": name,
                    "installed": version.is_some(),
                    "version": version,
                    "accounts": mine.len(),
                    "ready": ready.len(),
                    "label": ready.first().map(|account| account.label.clone()),
                })
            })
            .collect(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cli_version_reads_both_providers() {
        assert_eq!(cli_version("2.1.292 (Claude Code)\n").as_deref(), Some("2.1.292"));
        assert_eq!(cli_version("codex-cli 0.163.0\n").as_deref(), Some("0.163.0"));
        assert_eq!(cli_version("0.63.0\n").as_deref(), Some("0.63.0"));
        assert_eq!(cli_version("command not found"), None);
        assert_eq!(cli_version("opencode v2.2.6\n").as_deref(), Some("2.2.6"));
    }
}
