//! Credentials of the AI accounts listed in `ai_accounts`. Each account owns one directory,
//! `$OCR_DATA_DIR/accounts/<id>/`, holding its provider's credential file:
//! `claude/oauth-token`, `codex/auth.json` or `opencode/auth.json`.
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

/// The OpenCode plans of every OpenCode account together, or None without one. An account saved
/// before its plans were recorded counts as having both.
pub(crate) fn opencode_plans() -> Option<Vec<String>> {
    plans_under(&root())
}

fn plans_under(root: &std::path::Path) -> Option<Vec<String>> {
    let mut plans = Vec::new();
    let mut any = false;
    for entry in fs::read_dir(root).ok()?.flatten() {
        let directory = entry.path().join("opencode");
        if !directory.join("auth.json").exists() {
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
        for path in ["1/claude", "2/codex", "3/gemini", "4/opencode"] {
            std::fs::create_dir_all(root.join(path)).unwrap();
            std::fs::write(root.join(path).join("secret"), "x").unwrap();
        }
        assert!(super::remove_unsupported_sign_ins(&root).unwrap());
        assert!(root.join("1/claude/secret").exists());
        assert!(root.join("2/codex/secret").exists());
        assert!(!root.join("3").exists());
        assert!(root.join("4/opencode/secret").exists());
        // Nothing left to remove, and a missing root is not an error.
        assert!(!super::remove_unsupported_sign_ins(&root).unwrap());
        std::fs::remove_dir_all(&root).unwrap();
        assert!(!super::remove_unsupported_sign_ins(&root).unwrap());
    }

    #[test]
    fn opencode_plans_join_every_opencode_account() {
        let root = std::env::temp_dir().join(format!("ocr-opencode-plans-{}", std::process::id()));
        let account = |id: i64, plans: Option<&str>| {
            let directory = root.join(id.to_string()).join("opencode");
            std::fs::create_dir_all(&directory).unwrap();
            std::fs::write(directory.join("auth.json"), "{}").unwrap();
            if let Some(plans) = plans {
                std::fs::write(directory.join("plans.json"), plans).unwrap();
            }
        };
        std::fs::create_dir_all(root.join("1/claude")).unwrap();
        assert_eq!(super::plans_under(&root), None);
        account(2, Some(r#"{"plans":["zen"]}"#));
        assert_eq!(super::plans_under(&root), Some(vec!["zen".to_string()]));
        account(3, Some(r#"{"plans":["go","zen"]}"#));
        let mut both = super::plans_under(&root).unwrap();
        both.sort();
        assert_eq!(both, vec!["go".to_string(), "zen".to_string()]);
        std::fs::remove_dir_all(root.join("3")).unwrap();
        // An account saved before plans were recorded counts as both.
        account(4, None);
        let mut legacy = super::plans_under(&root).unwrap();
        legacy.sort();
        assert_eq!(legacy, vec!["go".to_string(), "zen".to_string()]);
        std::fs::remove_dir_all(&root).unwrap();
    }
}
