//! Credentials of the AI accounts listed in `ai_accounts`. Each account owns one directory,
//! `$OCR_DATA_DIR/accounts/<id>/`, holding its provider's credential file:
//! `claude/oauth-token`, `codex/auth.json` or `opencode/go-api-key`.
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

pub(crate) fn opencode_key_path(id: i64) -> PathBuf {
    root().join(id.to_string()).join("opencode/go-api-key")
}

pub(crate) fn validate_opencode_key(key: &str) -> Result<(), String> {
    if key.is_empty() || key.len() > 4096 || !key.bytes().all(|byte| byte.is_ascii_graphic()) {
        return Err("Invalid OpenCode Go API key".into());
    }
    Ok(())
}

/// Only explicit keys enter storage. A partial replacement revokes both copies.
pub(crate) fn store_opencode_key(id: i64, key: &str) -> Result<(), String> {
    validate_opencode_key(key)?;
    if std::env::var_os("OCR_REVIEW_CLAUDE_AUTH_DIR").is_none() {
        return Err("OpenCode Go authentication projection is unavailable".into());
    }
    let result = store(&opencode_key_path(id), id, key.as_bytes()).and_then(|()| {
        if has_credential("opencode", id) {
            Ok(())
        } else {
            Err("OpenCode Go authentication projection did not complete".into())
        }
    });
    if result.is_err() {
        let _ = fs::remove_file(opencode_key_path(id));
        let _ = sandbox_sync::revoke_opencode(id);
    }
    result
}

fn read_opencode_key(path: &std::path::Path) -> Option<String> {
    let metadata = fs::symlink_metadata(path).ok()?;
    if !metadata.is_file() || metadata.len() > 4096 {
        return None;
    }
    let key = fs::read_to_string(path).ok()?;
    validate_opencode_key(&key).ok()?;
    Some(key)
}

pub(crate) fn store_claude_token(id: i64, token: &str) -> Result<(), String> {
    store(&claude_token_path(id), id, token.as_bytes())
}

/// Saves a Codex `auth.json`, from a sign-in or refreshed by a review stage.
pub(crate) fn store_codex_auth(id: i64, raw: &str) -> Result<(), String> {
    crate::providers::codex::validate_auth(raw)?;
    store(&codex_auth_path(id), id, raw.as_bytes())
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
        "opencode" => {
            let Some(key) = read_opencode_key(&opencode_key_path(id)) else {
                return false;
            };
            std::env::var_os("OCR_REVIEW_CLAUDE_AUTH_DIR")
                .map(PathBuf::from)
                .and_then(|root| read_opencode_key(&root.join(format!("accounts/{id}/opencode/go-api-key"))))
                .is_some_and(|projected| projected == key)
        }
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

/// Deletes retired sign-ins, preserving only the explicit Go key under OpenCode.
/// Legacy OpenCode auth.json is never reused. Returns whether it deleted any.
fn remove_unsupported_sign_ins(root: &std::path::Path) -> Result<bool, String> {
    let mut removed = false;
    let Ok(entries) = fs::read_dir(root) else {
        return Ok(false);
    };
    for account in entries.flatten().map(|entry| entry.path()) {
        match fs::remove_dir_all(account.join("gemini")) {
            Ok(()) => {
                removed = true;
                let _ = fs::remove_dir(&account);
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => return Err("Could not remove the sign-in of a removed account".into()),
        }
        let opencode = account.join("opencode");
        if let Ok(entries) = fs::read_dir(&opencode) {
            for entry in entries {
                let entry = entry.map_err(|_| "Could not inspect a legacy OpenCode sign-in")?;
                let kind = entry
                    .file_type()
                    .map_err(|_| "Could not inspect a legacy OpenCode sign-in")?;
                if entry.file_name() == "go-api-key" && kind.is_file() {
                    continue;
                }
                let result = if kind.is_dir() {
                    fs::remove_dir_all(entry.path())
                } else {
                    fs::remove_file(entry.path())
                };
                result.map_err(|_| "Could not remove a legacy OpenCode sign-in")?;
                removed = true;
            }
            let _ = fs::remove_dir(opencode);
            let _ = fs::remove_dir(&account);
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
    fn go_key_validation_rejects_invalid_or_absent_credentials() {
        assert!(super::validate_opencode_key("synthetic-go-key").is_ok());
        for key in ["", " ", "two words", "line\nbreak", "\t", "é"] {
            assert!(super::validate_opencode_key(key).is_err());
        }
        assert!(super::validate_opencode_key(&"x".repeat(4097)).is_err());
        assert!(super::store_opencode_key(i64::MAX, "invalid\nkey").is_err());
        assert!(!super::has_credential("opencode", i64::MAX));
    }

    #[test]
    fn only_removed_providers_lose_their_sign_ins() {
        let root = std::env::temp_dir().join(format!("ocr-unsupported-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        for path in ["1/claude", "2/codex", "3/gemini", "4/opencode"] {
            std::fs::create_dir_all(root.join(path)).unwrap();
            std::fs::write(root.join(path).join("secret"), "x").unwrap();
        }
        std::fs::create_dir_all(root.join("5/opencode")).unwrap();
        std::fs::write(root.join("5/opencode/go-api-key"), "synthetic-go-key").unwrap();
        std::fs::write(root.join("5/opencode/auth.json"), "legacy-never-reused").unwrap();
        assert!(super::remove_unsupported_sign_ins(&root).unwrap());
        assert!(root.join("1/claude/secret").exists());
        assert!(root.join("2/codex/secret").exists());
        assert!(!root.join("3").exists());
        assert!(!root.join("4").exists());
        assert_eq!(
            std::fs::read_to_string(root.join("5/opencode/go-api-key")).unwrap(),
            "synthetic-go-key"
        );
        assert!(!root.join("5/opencode/auth.json").exists());
        // Nothing left to remove, and a missing root is not an error.
        assert!(!super::remove_unsupported_sign_ins(&root).unwrap());
        std::fs::remove_dir_all(&root).unwrap();
        assert!(!super::remove_unsupported_sign_ins(&root).unwrap());
    }
}
