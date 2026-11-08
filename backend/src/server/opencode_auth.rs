use super::*;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct KeyPaste {
    api_key: String,
    account_id: Option<i64>,
}

/// Stores an explicitly supplied Go key. Presence does not establish provider entitlement.
pub(crate) fn save_key(request: &Request, state: &SharedState) -> Result<serde_json::Value, String> {
    require_auth(request, state)?;
    require_trusted_origin(request)?;
    let paste: KeyPaste = serde_json::from_slice(&request.body).map_err(|_| "Invalid OpenCode Go API key")?;
    crate::accounts::validate_opencode_key(&paste.api_key)?;
    state
        .config
        .db
        .run_exclusive_after(accounts::ACCOUNT_CREDENTIAL_WRITE, || {
            with_db_raw(state, |db| {
                let (id, created) = match paste.account_id {
                    Some(id) => {
                        if db.ai_account(id).map_err(|_| "Invalid OpenCode Go account")?.provider != "opencode" {
                            return Err("Invalid OpenCode Go account".into());
                        }
                        (id, false)
                    }
                    None => (db.create_ai_account("opencode", "OpenCode Go")?.id, true),
                };
                // A failed source/projection write must not leave a runnable partial replacement.
                db.mark_ai_account_auth_failed(id, "OpenCode Go key connection did not complete")?;
                if let Err(error) = crate::accounts::store_opencode_key(id, &paste.api_key) {
                    if created {
                        let _ = db.delete_ai_account(id);
                        let _ = crate::accounts::remove(id);
                    }
                    return Err(error);
                }
                db.reset_ai_account_status(id)?;
                crate::accounts::usage::forget(id);
                Ok(serde_json::json!({ "ok": true, "accountId": id, "signedIn": true }))
            })
        })?
}
