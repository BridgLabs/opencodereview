//! Credentials of the AI accounts listed in `ai_accounts`. Each account owns one directory,
//! `$OCR_DATA_DIR/accounts/<id>/`, holding its provider's credential file:
//! `claude/oauth-token`, `codex/auth.json`, `opencode/auth.json` or `cursor/auth.json`.
pub(crate) mod sandbox_sync;
pub(crate) mod usage;

use std::fs;
use std::io::Write;
use std::path::PathBuf;

use crate::db::Database;

pub(crate) fn root() -> PathBuf {
    std::env::var_os("OCR_DATA_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| "data".into())
        .join("accounts")
}

pub(crate) fn claude_token_path(id: i64) -> PathBuf {
    root().join(id.to_string()).join("claude/oauth-token")
}

/// Ids of accounts that have a credential directory, in ascending order.
pub(crate) fn stored_ids() -> Vec<i64> {
    let mut ids = fs::read_dir(root())
        .map(|entries| {
            entries
                .flatten()
                .filter_map(|entry| entry.file_name().to_str()?.parse::<i64>().ok())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    ids.sort_unstable();
    ids
}

pub(crate) fn codex_auth_path(id: i64) -> PathBuf {
    root().join(id.to_string()).join("codex/auth.json")
}

pub(crate) fn opencode_auth_path(id: i64) -> PathBuf {
    root().join(id.to_string()).join("opencode/auth.json")
}

pub(crate) fn cursor_auth_path(id: i64) -> PathBuf {
    root().join(id.to_string()).join("cursor/auth.json")
}

/// The models a Cursor account's subscription offers (`cursor-agent models` at sign-in), kept
/// beside its sign-in for the server only: runners never receive it.
pub(crate) fn cursor_models_path(id: i64) -> PathBuf {
    root().join(id.to_string()).join("cursor/models.json")
}

/// Saves a Cursor account: its browser sign-in or user API key.
pub(crate) fn store_cursor_auth(id: i64, raw: &str) -> Result<(), String> {
    crate::providers::cursor::validate_auth(raw)?;
    store(&cursor_auth_path(id), id, raw.as_bytes())
}

pub(crate) fn store_cursor_models(id: i64, models: &[crate::providers::cursor::ListedModel]) -> Result<(), String> {
    store(
        &cursor_models_path(id),
        id,
        serde_json::json!({ "models": models }).to_string().as_bytes(),
    )
}

/// The models the Cursor accounts `ids` offer together, each account's default first, without
/// repeats; empty when none of them has a usable sign-in and a recorded list. The caller passes
/// the accounts reviews can run on (`Database::cursor_review_accounts`).
pub(crate) fn cursor_models(ids: &[i64]) -> Vec<crate::providers::cursor::ListedModel> {
    cursor_models_under(&root(), ids)
}

fn cursor_models_under(root: &std::path::Path, ids: &[i64]) -> Vec<crate::providers::cursor::ListedModel> {
    let mut models: Vec<crate::providers::cursor::ListedModel> = Vec::new();
    for id in ids {
        let directory = root.join(id.to_string()).join("cursor");
        let signed_in = fs::read_to_string(directory.join("auth.json"))
            .is_ok_and(|raw| crate::providers::cursor::validate_auth(&raw).is_ok());
        if !signed_in {
            continue;
        }
        let listed = fs::read_to_string(directory.join("models.json"))
            .ok()
            .and_then(|raw| serde_json::from_str::<serde_json::Value>(&raw).ok())
            .and_then(|value| {
                serde_json::from_value::<Vec<crate::providers::cursor::ListedModel>>(value["models"].clone()).ok()
            })
            .unwrap_or_default();
        for mut model in listed {
            if crate::providers::cursor::model_ok(&model.id) && !models.iter().any(|known| known.id == model.id) {
                model.label = crate::providers::cursor::clean_label(&model.label);
                models.push(model);
            }
        }
    }
    models
}

pub(crate) fn store_claude_token(id: i64, token: &str) -> Result<(), String> {
    store(&claude_token_path(id), id, token.as_bytes())
}

/// Saves a Codex `auth.json`, from a sign-in or refreshed by a review stage.
pub(crate) fn store_codex_auth(id: i64, raw: &str) -> Result<(), String> {
    crate::providers::codex::validate_auth(raw)?;
    store(&codex_auth_path(id), id, raw.as_bytes())
}

/// The OpenCode plans an account's workspace has (`go`, `zen`), kept beside its sign-in for the
/// server only: runners never receive it.
pub(crate) fn opencode_plans_path(id: i64) -> PathBuf {
    root().join(id.to_string()).join("opencode/plans.json")
}

pub(crate) fn store_opencode_plans(id: i64, plans: &[&str]) -> Result<(), String> {
    store(
        &opencode_plans_path(id),
        id,
        serde_json::json!({ "plans": plans }).to_string().as_bytes(),
    )
}

/// The OpenCode plans of the accounts `ids` together, or None when none of them has a usable
/// sign-in. The caller passes the accounts reviews can run on (`Database::opencode_plans`): a
/// folder left without its account, or a disabled or signed-out account, offers no models. An
/// account saved before its plans were recorded counts as having both.
pub(crate) fn opencode_plans(ids: &[i64]) -> Option<Vec<String>> {
    plans_under(&root(), ids)
}

fn plans_under(root: &std::path::Path, ids: &[i64]) -> Option<Vec<String>> {
    let mut plans = Vec::new();
    let mut any = false;
    for id in ids {
        let directory = root.join(id.to_string()).join("opencode");
        let signed_in = fs::read_to_string(directory.join("auth.json"))
            .is_ok_and(|raw| crate::providers::opencode::validate_auth(&raw).is_ok());
        if !signed_in {
            continue;
        }
        any = true;
        let known = fs::read_to_string(directory.join("plans.json"))
            .ok()
            .and_then(|raw| serde_json::from_str::<serde_json::Value>(&raw).ok())
            .and_then(|value| {
                value["plans"].as_array().map(|list| {
                    list.iter()
                        .filter_map(|plan| plan.as_str().map(str::to_string))
                        .collect::<Vec<_>>()
                })
            })
            .unwrap_or_else(|| vec!["go".into(), "zen".into()]);
        for plan in known {
            if !plans.contains(&plan) {
                plans.push(plan);
            }
        }
    }
    any.then_some(plans)
}

/// Saves an OpenCode account: its API key or Console sign-in.
pub(crate) fn store_opencode_auth(id: i64, raw: &str) -> Result<(), String> {
    crate::providers::opencode::validate_auth(raw)?;
    store(&opencode_auth_path(id), id, raw.as_bytes())
}

fn store(path: &std::path::Path, id: i64, bytes: &[u8]) -> Result<(), String> {
    let parent = path.parent().ok_or("Invalid credential location")?;
    fs::create_dir_all(parent).map_err(|_| "Credential location is not writable")?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        for directory in [parent.to_path_buf(), root().join(id.to_string())] {
            let _ = fs::set_permissions(directory, fs::Permissions::from_mode(0o700));
        }
    }
    let temporary = parent.join(format!(".credential-{}", crate::sandbox::secret()?));
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let result = options
        .open(&temporary)
        .and_then(|mut file| file.write_all(bytes).and_then(|_| file.sync_all()))
        .and_then(|_| fs::rename(&temporary, path))
        .map_err(|_| "Could not persist the account sign-in".to_string());
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result?;
    sandbox_sync::sync_app_auth()
}

pub(crate) fn remove(id: i64) -> Result<(), String> {
    match fs::remove_dir_all(root().join(id.to_string())) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(_) => return Err("Could not remove the account credentials".into()),
    }
    sandbox_sync::sync_app_auth()
}

pub(crate) fn has_credential(provider: &str, id: i64) -> bool {
    match provider {
        "claude" => crate::providers::claude::read_token(&claude_token_path(id)).is_ok(),
        "codex" => fs::read_to_string(codex_auth_path(id))
            .is_ok_and(|raw| crate::providers::codex::validate_auth(&raw).is_ok()),
        "opencode" => fs::read_to_string(opencode_auth_path(id))
            .is_ok_and(|raw| crate::providers::opencode::validate_auth(&raw).is_ok()),
        // A browser sign-in is never renewed, so an expired one is no credential.
        "cursor" => fs::read_to_string(cursor_auth_path(id))
            .is_ok_and(|raw| crate::providers::cursor::usable(&raw, crate::db::now_millis())),
        _ => false,
    }
}

/// Removes accounts of providers the app no longer runs, and their sign-ins. The sign-ins are found
/// by their provider folder, not by the removed rows: a database that is still empty, such as one
/// waiting for a SQLite import, must not cost other accounts their sign-ins, and a folder that
/// could not be removed is tried again on the next start.
pub(crate) fn remove_unsupported(db: &Database) -> Result<(), String> {
    db.remove_unsupported_ai_accounts()?;
    if remove_unsupported_sign_ins(&root())? {
        sandbox_sync::sync_app_auth()?;
    }
    Ok(())
}

/// Deletes the Gemini folders under `root`, and each account folder they leave
/// empty. Returns whether it deleted any.
fn remove_unsupported_sign_ins(root: &std::path::Path) -> Result<bool, String> {
    let mut removed = false;
    let Ok(entries) = fs::read_dir(root) else {
        return Ok(false);
    };
    for account in entries.flatten().map(|entry| entry.path()) {
        for provider in ["gemini"] {
            match fs::remove_dir_all(account.join(provider)) {
                Ok(()) => {
                    removed = true;
                    let _ = fs::remove_dir(&account);
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(_) => return Err("Could not remove the sign-in of a removed account".into()),
            }
        }
    }
    Ok(removed)
}

/// Installs from before multiple accounts kept one token at the app path; it becomes the first
/// Claude account so reviews keep working after the upgrade.
pub(crate) fn adopt_legacy_token(db: &Database) -> Result<(), String> {
    let legacy = crate::providers::claude::app_token_path();
    let Ok(token) = crate::providers::claude::read_token(&legacy) else {
        return Ok(());
    };
    if db
        .list_ai_accounts()?
        .iter()
        .any(|account| account.provider == "claude")
    {
        return Ok(());
    }
    let account = db.create_ai_account("claude", "Claude")?;
    store_claude_token(account.id, &token)?;
    fs::remove_file(&legacy).map_err(|_| "Could not retire the previous Claude token location".to_string())?;
    sandbox_sync::sync_app_auth()
}

#[cfg(test)]
mod tests {
    #[test]
    fn only_removed_providers_lose_their_sign_ins() {
        let root = std::env::temp_dir().join(format!("ocr-unsupported-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        for path in ["1/claude", "2/codex", "3/gemini", "4/opencode", "5/cursor"] {
            std::fs::create_dir_all(root.join(path)).unwrap();
            std::fs::write(root.join(path).join("secret"), "x").unwrap();
        }
        assert!(super::remove_unsupported_sign_ins(&root).unwrap());
        assert!(root.join("1/claude/secret").exists());
        assert!(root.join("2/codex/secret").exists());
        assert!(!root.join("3").exists());
        assert!(root.join("4/opencode/secret").exists());
        assert!(root.join("5/cursor/secret").exists());
        // Nothing left to remove, and a missing root is not an error.
        assert!(!super::remove_unsupported_sign_ins(&root).unwrap());
        std::fs::remove_dir_all(&root).unwrap();
        assert!(!super::remove_unsupported_sign_ins(&root).unwrap());
    }

    #[test]
    fn opencode_plans_join_the_given_accounts_with_a_usable_sign_in() {
        let root = std::env::temp_dir().join(format!("ocr-opencode-plans-{}", std::process::id()));
        let key = r#"{"key":"sk-0123456789abcdef"}"#;
        let account = |id: i64, auth: &str, plans: Option<&str>| {
            let directory = root.join(id.to_string()).join("opencode");
            std::fs::create_dir_all(&directory).unwrap();
            std::fs::write(directory.join("auth.json"), auth).unwrap();
            if let Some(plans) = plans {
                std::fs::write(directory.join("plans.json"), plans).unwrap();
            }
        };
        std::fs::create_dir_all(root.join("1/claude")).unwrap();
        assert_eq!(super::plans_under(&root, &[1, 2]), None);
        account(2, key, Some(r#"{"plans":["zen"]}"#));
        assert_eq!(super::plans_under(&root, &[1, 2]), Some(vec!["zen".to_string()]));
        account(3, key, Some(r#"{"plans":["go","zen"]}"#));
        let mut both = super::plans_under(&root, &[2, 3]).unwrap();
        both.sort();
        assert_eq!(both, vec!["go".to_string(), "zen".to_string()]);
        // A folder whose account is not asked for (removed, disabled or signed out) offers nothing.
        assert_eq!(super::plans_under(&root, &[2]), Some(vec!["zen".to_string()]));
        assert_eq!(super::plans_under(&root, &[]), None);
        // Nor does a sign-in that does not validate.
        account(5, "{}", Some(r#"{"plans":["go"]}"#));
        assert_eq!(super::plans_under(&root, &[5]), None);
        // An account saved before plans were recorded counts as both.
        account(4, key, None);
        let mut legacy = super::plans_under(&root, &[4]).unwrap();
        legacy.sort();
        assert_eq!(legacy, vec!["go".to_string(), "zen".to_string()]);
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn cursor_models_join_the_given_accounts_with_a_usable_sign_in() {
        let root = std::env::temp_dir().join(format!("ocr-cursor-models-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let key = r#"{"key":"key_0123456789abcdef0123"}"#;
        let account = |id: i64, auth: &str, models: &str| {
            let directory = root.join(id.to_string()).join("cursor");
            std::fs::create_dir_all(&directory).unwrap();
            std::fs::write(directory.join("auth.json"), auth).unwrap();
            std::fs::write(directory.join("models.json"), models).unwrap();
        };
        account(
            1,
            key,
            r#"{"models":[{"id":"auto","label":"Auto"},{"id":"gpt-5.5","label":"GPT-5.5"}]}"#,
        );
        account(
            2,
            key,
            r#"{"models":[{"id":"gpt-5.5","label":"GPT-5.5"},{"id":"sonnet-5","label":"Sonnet 5"},{"id":"bad id","label":"x"}]}"#,
        );
        account(3, "{}", r#"{"models":[{"id":"secret-model","label":"x"}]}"#);
        let ids = |ids: &[i64]| {
            super::cursor_models_under(&root, ids)
                .into_iter()
                .map(|model| model.id)
                .collect::<Vec<_>>()
        };
        assert_eq!(ids(&[1, 2, 3]), vec!["auto", "gpt-5.5", "sonnet-5"]);
        assert_eq!(ids(&[2]), vec!["gpt-5.5", "sonnet-5"]);
        assert!(ids(&[3]).is_empty() && ids(&[]).is_empty());
        std::fs::remove_dir_all(&root).unwrap();
    }
}
