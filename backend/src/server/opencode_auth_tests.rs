use super::*;

// Credential paths and CLI probes use process environment. Isolate those tests from all other tests.
fn isolated(name: &str) -> bool {
    if std::env::var("OCR_GO_API_TEST").as_deref() == Ok(name) {
        return true;
    }
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            &format!("server::opencode_auth_tests::{name}"),
            "--nocapture",
            "--test-threads=1",
        ])
        .env("OCR_GO_API_TEST", name)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    false
}

fn state() -> SharedState {
    let root = std::env::temp_dir().join(format!("ocr-go-api-{}", crate::sandbox::secret().unwrap()));
    let mut server = ServerState::new(ServerConfig {
        bind: "127.0.0.1:0".into(),
        db: crate::db::Db::for_test(),
        data_dir: root.clone(),
        repos_dir: root.join("repos"),
        auto_reindex: false,
        poll_interval_seconds: 300,
        api_token: Some("fixture-api-auth".into()),
    });
    server.go_catalogue = crate::providers::go_catalogue::Discovery::test_source(|| Ok(catalogue()));
    server.go_catalogue.install(catalogue());
    Arc::new(server)
}

fn catalogue() -> crate::providers::go_catalogue::Catalogue {
    crate::providers::go_catalogue::fixture("new-dynamic-model", &["new-advertised-effort"], "chat-completions")
}

fn request(method: &str, path: &str, body: serde_json::Value) -> Request {
    Request {
        method: method.into(),
        path: path.into(),
        query: String::new(),
        origin: Some("http://localhost:1421".into()),
        authorization: Some("Bearer fixture-api-auth".into()),
        content_type: Some("application/json".into()),
        flow_secret: None,
        body: serde_json::to_vec(&body).unwrap(),
    }
}

fn json(response: Response) -> serde_json::Value {
    assert_eq!(response.status, 200, "{}", String::from_utf8_lossy(&response.body));
    serde_json::from_slice(&response.body).unwrap()
}

#[test]
fn opencode_api_dynamic_catalogue_requires_current_confirmation_and_preserves_invalid_preferences() {
    let state = state();
    let engines = json(route(
        request("GET", "/review-engines", serde_json::Value::Null),
        state.clone(),
    ));
    let go = engines
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["provider"] == "opencode")
        .unwrap();
    assert_eq!(go["catalogue"]["status"], "ready");
    assert_eq!(go["selectionValid"], false);
    assert_eq!(go["model"], "");
    assert_eq!(go["models"][0]["id"], "new-dynamic-model");
    let db = state.config.db.open().unwrap();
    db.save_setting_value("review_engine_opencode", "malformed-imported-preference")
        .unwrap();
    assert!(
        db.go_preference().is_err(),
        "preparation must not replace a malformed preference with a default"
    );
    let engines = json(route(
        request("GET", "/review-engines", serde_json::Value::Null),
        state.clone(),
    ));
    let go = engines
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["provider"] == "opencode")
        .unwrap();
    assert_eq!(
        go["catalogue"]["status"], "ready",
        "the user must still be able to repair the preference explicitly"
    );
    assert_eq!(go["selectionValid"], false);
    drop(db);
    for id in [None, Some("outdated")] {
        let mut body =
            serde_json::json!({"provider":"opencode","model":"new-dynamic-model","effort":"new-advertised-effort"});
        if let Some(id) = id {
            body["catalogueId"] = id.into();
        }
        assert_ne!(
            route(request("POST", "/review-engines", body), state.clone()).status,
            200
        );
    }
    let saved = json(route(
        request(
            "POST",
            "/review-engines",
            serde_json::json!({
                "provider":"opencode","model":"new-dynamic-model","effort":"new-advertised-effort","catalogueId":catalogue().id
            }),
        ),
        state.clone(),
    ));
    let go = saved
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["provider"] == "opencode")
        .unwrap();
    assert_eq!(go["selectionValid"], true);
    state.go_catalogue.install(crate::providers::go_catalogue::fixture(
        "model-added-later",
        &[],
        "responses",
    ));
    let engines = json(route(
        request("GET", "/review-engines", serde_json::Value::Null),
        state.clone(),
    ));
    let go = engines
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["provider"] == "opencode")
        .unwrap();
    assert_eq!(go["model"], "new-dynamic-model");
    assert_eq!(go["selectionValid"], false);
    let current = go["catalogue"]["id"].clone();
    let saved = json(route(
        request(
            "POST",
            "/review-engines",
            serde_json::json!({
                "provider":"opencode","model":"model-added-later","effort":"","catalogueId":current
            }),
        ),
        state.clone(),
    ));
    let go = saved
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["provider"] == "opencode")
        .unwrap();
    assert_eq!(go["selectionValid"], true);
    assert_eq!(go["effort"], "");
    let mut refresh = request("POST", "/review-engines/opencode/refresh", serde_json::json!({}));
    refresh.origin = None;
    assert_eq!(route(refresh, state.clone()).status, 403);
    assert_ne!(
        route(
            request(
                "POST",
                "/review-engines/opencode/refresh",
                serde_json::json!({"url":"https://evil.test"})
            ),
            state.clone()
        )
        .status,
        200
    );
    assert_eq!(
        json(route(
            request("POST", "/review-engines/opencode/refresh", serde_json::json!({})),
            state
        )),
        serde_json::json!({"ok":true})
    );
}

#[test]
fn opencode_api_failed_refresh_never_serves_the_old_inventory() {
    let mut state = state();
    let discovery =
        crate::providers::go_catalogue::Discovery::test_source(|| Err("Synthetic catalogue download failed".into()));
    discovery.install(catalogue());
    Arc::get_mut(&mut state).unwrap().go_catalogue = discovery;
    json(route(
        request("POST", "/review-engines/opencode/refresh", serde_json::json!({})),
        state.clone(),
    ));
    let mut final_status = String::new();
    for _ in 0..1000 {
        let engines = json(route(
            request("GET", "/review-engines", serde_json::Value::Null),
            state.clone(),
        ));
        let go = engines
            .as_array()
            .unwrap()
            .iter()
            .find(|e| e["provider"] == "opencode")
            .unwrap();
        assert_eq!(go["models"], serde_json::json!([]));
        assert_eq!(go["selectionValid"], false);
        assert!(go["catalogue"]["id"].is_null());
        final_status = go["catalogue"]["status"].as_str().unwrap().into();
        if final_status == "error" {
            break;
        }
        std::thread::sleep(Duration::from_millis(1));
    }
    assert_eq!(final_status, "error");
    assert_ne!(route(request("POST", "/review-engines", serde_json::json!({
        "provider":"opencode","model":"new-dynamic-model","effort":"new-advertised-effort","catalogueId":catalogue().id
    })), state).status, 200);
}

#[test]
fn opencode_api_key_connection_is_private_and_requires_fresh_explicit_key() {
    if !isolated("opencode_api_key_connection_is_private_and_requires_fresh_explicit_key") {
        return;
    }
    let state = state();
    let root = state.config.data_dir.clone();
    std::fs::create_dir_all(&root).unwrap();
    // This test runs alone in a filtered process; no other credential-path test runs beside it.
    std::env::set_var("OCR_DATA_DIR", &root);
    std::env::set_var("OCR_REVIEW_CLAUDE_AUTH_DIR", root.join("projection"));
    let key = "synthetic-go-key-no-provider-prefix";
    let connected = json(route(
        request("POST", "/auth/opencode/key", serde_json::json!({"apiKey": key})),
        state.clone(),
    ));
    assert_eq!(connected["ok"], true);
    let id = connected["accountId"].as_i64().unwrap();
    assert_eq!(connected["signedIn"], true);
    assert_eq!(
        connected,
        serde_json::json!({"ok": true, "accountId": id, "signedIn": true})
    );
    assert!(!connected.to_string().contains(key));
    let accounts = json(route(
        request("GET", "/accounts", serde_json::Value::Null),
        state.clone(),
    ));
    assert_eq!(accounts[0]["provider"], "opencode");
    assert_eq!(accounts[0]["signedIn"], true);
    assert!(!accounts.to_string().contains(key));
    let source = root.join(format!("accounts/{id}/opencode/go-api-key"));
    let projection = root.join(format!("projection/accounts/{id}/opencode/go-api-key"));
    assert_eq!(std::fs::read_to_string(&source).unwrap(), key);
    assert_eq!(std::fs::read_to_string(&projection).unwrap(), key);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        for file in [&source, &projection] {
            assert_eq!(std::fs::metadata(file).unwrap().permissions().mode() & 0o777, 0o600);
            assert_eq!(
                std::fs::metadata(file.parent().unwrap()).unwrap().permissions().mode() & 0o777,
                0o700
            );
        }
    }
    let usage = json(route(
        request("GET", "/accounts/usage", serde_json::Value::Null),
        state.clone(),
    ));
    assert_eq!(usage[0]["windows"], serde_json::json!([]));
    assert_eq!(usage[0]["error"], "OpenCode Go plan usage is unavailable");
    assert!(!usage.to_string().contains(key));
    let replaced = json(route(
        request(
            "POST",
            "/auth/opencode/key",
            serde_json::json!({"apiKey": "replacement-synthetic", "accountId": id}),
        ),
        state.clone(),
    ));
    assert_eq!(replaced["accountId"], id);
    assert_eq!(
        json(route(
            request("GET", "/accounts", serde_json::Value::Null),
            state.clone()
        ))
        .as_array()
        .unwrap()
        .len(),
        1
    );
    json(route(
        request("DELETE", &format!("/accounts/{id}"), serde_json::Value::Null),
        state,
    ));
    assert!(!source.exists());
    assert!(!projection.exists());
    std::env::remove_var("OCR_DATA_DIR");
    std::env::remove_var("OCR_REVIEW_CLAUDE_AUTH_DIR");
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn opencode_api_preserves_accounts_and_fresh_keys_but_never_uses_legacy_auth() {
    if !isolated("opencode_api_preserves_accounts_and_fresh_keys_but_never_uses_legacy_auth") {
        return;
    }
    let state = state();
    let root = state.config.data_dir.clone();
    std::env::set_var("OCR_DATA_DIR", &root);
    std::env::set_var("OCR_REVIEW_CLAUDE_AUTH_DIR", root.join("projection"));
    let db = state.config.db.open().unwrap();
    let legacy = db.create_ai_account("opencode", "Legacy OpenCode").unwrap();
    let folder = root.join(format!("accounts/{}/opencode", legacy.id));
    std::fs::create_dir_all(&folder).unwrap();
    std::fs::write(
        folder.join("auth.json"),
        r#"{"opencode":{"type":"api","key":"legacy-synthetic"}}"#,
    )
    .unwrap();
    let old_projection = root.join(format!("projection/accounts/{}/opencode/auth.json", legacy.id));
    std::fs::create_dir_all(old_projection.parent().unwrap()).unwrap();
    std::fs::write(&old_projection, "legacy-projection-synthetic").unwrap();
    let accounts = json(route(
        request("GET", "/accounts", serde_json::Value::Null),
        state.clone(),
    ));
    assert_eq!(accounts[0]["signedIn"], false);
    assert!(
        db.pick_ai_account(&[]).unwrap().is_none(),
        "Legacy OpenCode credentials must never become selectable"
    );
    db.save_setting_value("review_engine_opencode", r#"{"model":"legacy-model","effort":"high"}"#)
        .unwrap();
    crate::accounts::remove_unsupported(&db).unwrap();
    let accounts = json(route(
        request("GET", "/accounts", serde_json::Value::Null),
        state.clone(),
    ));
    assert_eq!(accounts[0]["id"], legacy.id);
    assert_eq!(accounts[0]["signedIn"], false);
    assert!(!folder.join("auth.json").exists());
    assert!(!old_projection.exists());
    let engines = json(route(
        request("GET", "/review-engines", serde_json::Value::Null),
        state.clone(),
    ));
    let go = engines
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["provider"] == "opencode")
        .unwrap();
    assert_eq!(go["model"], "legacy-model");
    assert_eq!(go["effort"], "high");
    assert_eq!(go["selectionValid"], false);
    json(route(
        request(
            "POST",
            "/auth/opencode/key",
            serde_json::json!({"apiKey": "fresh-synthetic", "accountId": legacy.id}),
        ),
        state.clone(),
    ));
    json(route(
        request(
            "POST",
            "/review-engines",
            serde_json::json!({"provider": "opencode", "model": "new-dynamic-model", "effort": "new-advertised-effort", "catalogueId": catalogue().id}),
        ),
        state.clone(),
    ));
    crate::accounts::remove_unsupported(&db).unwrap();
    assert_eq!(db.pick_ai_account(&[]).unwrap().unwrap().id, legacy.id);
    assert!(crate::accounts::has_credential("opencode", legacy.id));
    assert!(db.setting("review_engine_opencode").unwrap().is_some());
    assert_eq!(
        json(route(request("GET", "/accounts", serde_json::Value::Null), state))[0]["signedIn"],
        true
    );
    std::env::remove_var("OCR_DATA_DIR");
    std::env::remove_var("OCR_REVIEW_CLAUDE_AUTH_DIR");
    std::fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
#[test]
fn opencode_api_reports_pinned_cli_version_without_claiming_legacy_readiness() {
    if !isolated("opencode_api_reports_pinned_cli_version_without_claiming_legacy_readiness") {
        return;
    }
    use std::os::unix::fs::PermissionsExt;
    let state = state();
    let root = state.config.data_dir.clone();
    let bin = root.join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    for (command, version) in [
        ("opencode", "opencode v2.2.6"),
        ("claude", "2.1.292 (Claude Code)"),
        ("codex", "codex-cli 0.163.0"),
    ] {
        let path = bin.join(command);
        std::fs::write(&path, format!("#!/bin/sh\nprintf '%s\\n' '{version}'\n")).unwrap();
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700)).unwrap();
    }
    std::env::set_var("PATH", format!("{}:/usr/bin:/bin", bin.display()));
    let providers = json(route(
        request("GET", "/ai/providers", serde_json::Value::Null),
        state.clone(),
    ));
    let go = providers
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["provider"] == "opencode")
        .expect("Provider status must expose OpenCode Go");
    assert_eq!(go["name"], "OpenCode Go");
    assert_eq!(go["installed"], true);
    assert_eq!(go["version"], "2.2.6");
    assert_eq!(go["ready"], 0);
    assert_eq!(providers[0]["version"], "2.1.292");
    assert_eq!(providers[1]["version"], "0.163.0");
    std::fs::write(bin.join("opencode"), "#!/bin/sh\nprintf 'opencode v0.0.1\\n'\n").unwrap();
    let providers = json(route(request("GET", "/ai/providers", serde_json::Value::Null), state));
    let go = providers
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["provider"] == "opencode")
        .unwrap();
    assert_eq!(
        go["installed"], false,
        "a different CLI version is not available for reviews"
    );
    assert!(go["version"].is_null());
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn opencode_api_rejects_untrusted_invalid_or_wrong_account_writes_without_mutation() {
    if !isolated("opencode_api_rejects_untrusted_invalid_or_wrong_account_writes_without_mutation") {
        return;
    }
    let state = state();
    let root = state.config.data_dir.clone();
    std::env::set_var("OCR_DATA_DIR", &root);
    std::env::set_var("OCR_REVIEW_CLAUDE_AUTH_DIR", root.join("projection"));
    let key = "synthetic-private-key";
    let connected = json(route(
        request("POST", "/auth/opencode/key", serde_json::json!({"apiKey": key})),
        state.clone(),
    ));
    let id = connected["accountId"].as_i64().unwrap();
    let db = state.config.db.open().unwrap();
    let wrong = db.create_ai_account("claude", "Unchanged").unwrap();
    let before = json(route(
        request("GET", "/accounts", serde_json::Value::Null),
        state.clone(),
    ));
    for value in [
        "".to_string(),
        " ".into(),
        "x y".into(),
        "x\ty".into(),
        "x\ny".into(),
        "x\0y".into(),
        "é".into(),
        "x".repeat(4097),
    ] {
        let response = route(
            request(
                "POST",
                "/auth/opencode/key",
                serde_json::json!({"apiKey": value, "accountId": id}),
            ),
            state.clone(),
        );
        assert_ne!(response.status, 200);
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&response.body).unwrap()["error"],
            "Invalid OpenCode Go API key"
        );
    }
    for body in [
        serde_json::json!({"apiKey": key, "extra": true}),
        serde_json::json!({"apiKey": null}),
        serde_json::json!({}),
        serde_json::json!({"apiKey": 42}),
        serde_json::json!({"apiKey": key, "accountId": "1"}),
        serde_json::json!({"apiKey": key, "accountId": true}),
    ] {
        assert_ne!(
            route(request("POST", "/auth/opencode/key", body), state.clone()).status,
            200
        );
    }
    for account_id in [wrong.id, -1, i64::MAX] {
        let response = route(
            request(
                "POST",
                "/auth/opencode/key",
                serde_json::json!({"apiKey": key, "accountId": account_id}),
            ),
            state.clone(),
        );
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&response.body).unwrap()["error"],
            "Invalid OpenCode Go account"
        );
    }
    for authorization in [None, Some("Bearer wrong-token".into())] {
        let mut denied = request("POST", "/auth/opencode/key", serde_json::json!({"apiKey": key}));
        denied.authorization = authorization;
        assert_eq!(route(denied, state.clone()).status, 401);
    }
    for origin in [None, Some("https://untrusted.example".into())] {
        let mut denied = request("POST", "/auth/opencode/key", serde_json::json!({"apiKey": key}));
        denied.origin = origin;
        assert_eq!(route(denied, state.clone()).status, 403);
    }
    assert_eq!(
        json(route(
            request("GET", "/accounts", serde_json::Value::Null),
            state.clone()
        )),
        before
    );
    assert_eq!(
        std::fs::read_to_string(crate::accounts::opencode_key_path(id)).unwrap(),
        key
    );
    assert!(!crate::accounts::opencode_key_path(wrong.id).exists());
    assert_eq!(
        std::fs::read_to_string(root.join(format!("projection/accounts/{id}/opencode/go-api-key"))).unwrap(),
        key
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn opencode_api_projection_failure_never_leaves_account_ready_for_execution() {
    if !isolated("opencode_api_projection_failure_never_leaves_account_ready_for_execution") {
        return;
    }
    let state = state();
    let root = state.config.data_dir.clone();
    std::env::set_var("OCR_DATA_DIR", &root);
    std::env::set_var("OCR_REVIEW_CLAUDE_AUTH_DIR", root.join("projection"));
    let connected = json(route(
        request(
            "POST",
            "/auth/opencode/key",
            serde_json::json!({"apiKey": "synthetic-first"}),
        ),
        state.clone(),
    ));
    let id = connected["accountId"].as_i64().unwrap();
    std::env::set_var("OCR_REVIEW_CLAUDE_AUTH_DIR", root.join("blocked"));
    std::fs::write(root.join("blocked"), "not a directory").unwrap();
    let failed = route(
        request(
            "POST",
            "/auth/opencode/key",
            serde_json::json!({"apiKey": "synthetic-replacement", "accountId": id}),
        ),
        state.clone(),
    );
    assert_ne!(failed.status, 200);
    assert!(!String::from_utf8_lossy(&failed.body).contains("synthetic-replacement"));
    assert!(!crate::accounts::opencode_key_path(id).exists());
    let accounts = json(route(
        request("GET", "/accounts", serde_json::Value::Null),
        state.clone(),
    ));
    assert_eq!(
        accounts[0]["status"], "auth_failed",
        "Failed projection must prevent execution until explicit reconnect"
    );
    assert!(state.config.db.open().unwrap().pick_ai_account(&[]).unwrap().is_none());
    let failed = route(
        request(
            "POST",
            "/auth/opencode/key",
            serde_json::json!({"apiKey": "synthetic-new-account"}),
        ),
        state.clone(),
    );
    assert_ne!(failed.status, 200);
    assert_eq!(
        json(route(
            request("GET", "/accounts", serde_json::Value::Null),
            state.clone()
        ))
        .as_array()
        .unwrap()
        .len(),
        1
    );
    std::env::set_var("OCR_REVIEW_CLAUDE_AUTH_DIR", root.join("projection"));
    json(route(
        request(
            "POST",
            "/auth/opencode/key",
            serde_json::json!({"apiKey": "synthetic-recovered", "accountId": id}),
        ),
        state.clone(),
    ));
    let selected = state.config.db.open().unwrap().pick_ai_account(&[]).unwrap().unwrap();
    assert_eq!(selected.id, id);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn opencode_api_missing_projection_and_source_failure_are_not_runnable() {
    if !isolated("opencode_api_missing_projection_and_source_failure_are_not_runnable") {
        return;
    }
    let state = state();
    let root = state.config.data_dir.clone();
    std::env::set_var("OCR_DATA_DIR", &root);
    std::env::set_var("OCR_REVIEW_CLAUDE_AUTH_DIR", root.join("projection"));
    let id = json(route(
        request(
            "POST",
            "/auth/opencode/key",
            serde_json::json!({"apiKey": "synthetic-first"}),
        ),
        state.clone(),
    ))["accountId"]
        .as_i64()
        .unwrap();
    let source = crate::accounts::opencode_key_path(id);
    let projection = root.join(format!("projection/accounts/{id}/opencode/go-api-key"));
    std::fs::remove_file(&projection).unwrap();
    let db = state.config.db.open().unwrap();
    assert!(!crate::accounts::has_credential("opencode", id));
    assert!(db.pick_ai_account(&[]).unwrap().is_none());
    assert_eq!(
        json(route(
            request("GET", "/accounts", serde_json::Value::Null),
            state.clone()
        ))[0]["signedIn"],
        false
    );
    json(route(
        request(
            "POST",
            "/auth/opencode/key",
            serde_json::json!({"apiKey": "synthetic-reconnect", "accountId": id}),
        ),
        state.clone(),
    ));
    assert_eq!(db.pick_ai_account(&[]).unwrap().unwrap().id, id);
    std::fs::remove_file(&source).unwrap();
    std::fs::create_dir(&source).unwrap();
    let failed = route(
        request(
            "POST",
            "/auth/opencode/key",
            serde_json::json!({"apiKey": "synthetic-source-failure", "accountId": id}),
        ),
        state.clone(),
    );
    assert_ne!(failed.status, 200);
    assert!(!String::from_utf8_lossy(&failed.body).contains("synthetic-source-failure"));
    assert!(!projection.exists(), "source failure must revoke the old projection");
    assert_eq!(db.ai_account(id).unwrap().status, "auth_failed");
    assert!(db.pick_ai_account(&[]).unwrap().is_none());
    std::fs::remove_dir(&source).unwrap();
    json(route(
        request(
            "POST",
            "/auth/opencode/key",
            serde_json::json!({"apiKey": "synthetic-final", "accountId": id}),
        ),
        state.clone(),
    ));
    assert_eq!(db.pick_ai_account(&[]).unwrap().unwrap().id, id);
    let mut malformed = request("POST", "/auth/opencode/key", serde_json::Value::Null);
    malformed.body = b"{\"apiKey\":".to_vec();
    let denied = route(malformed, state.clone());
    assert_ne!(denied.status, 200);
    assert_eq!(std::fs::read_to_string(&source).unwrap(), "synthetic-final");
    json(route(
        request("DELETE", &format!("/accounts/{id}"), serde_json::Value::Null),
        state,
    ));
    assert!(db.list_ai_accounts().unwrap().is_empty());
    assert!(!source.exists() && !projection.exists());
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn opencode_api_accounts_keep_priority_failover_and_explicit_reconnect_behavior() {
    if !isolated("opencode_api_accounts_keep_priority_failover_and_explicit_reconnect_behavior") {
        return;
    }
    let state = state();
    let root = state.config.data_dir.clone();
    std::env::set_var("OCR_DATA_DIR", &root);
    std::env::set_var("OCR_REVIEW_CLAUDE_AUTH_DIR", root.join("projection"));
    let first = json(route(
        request(
            "POST",
            "/auth/opencode/key",
            serde_json::json!({"apiKey": "first-synthetic"}),
        ),
        state.clone(),
    ))["accountId"]
        .as_i64()
        .unwrap();
    let second = json(route(
        request(
            "POST",
            "/auth/opencode/key",
            serde_json::json!({"apiKey": "second-synthetic"}),
        ),
        state.clone(),
    ))["accountId"]
        .as_i64()
        .unwrap();
    let db = state.config.db.open().unwrap();
    let other = db.create_ai_account("claude", "Other provider").unwrap().id;
    json(route(
        request(
            "POST",
            "/accounts/order",
            serde_json::json!({"ids": [first, other, second]}),
        ),
        state.clone(),
    ));
    assert_eq!(db.pick_ai_account(&[]).unwrap().unwrap().id, first);
    assert_eq!(db.pick_ai_account(&[first]).unwrap().unwrap().id, second);
    assert_eq!(db.pick_ai_account(&[first, second]).unwrap().unwrap().id, other);
    db.mark_ai_account_limited(first, None, "Synthetic usage limit")
        .unwrap();
    assert_eq!(db.pick_ai_account(&[]).unwrap().unwrap().id, second);
    db.mark_ai_account_auth_failed(second, "Synthetic authentication failure")
        .unwrap();
    assert_eq!(db.pick_ai_account(&[]).unwrap().unwrap().id, other);
    json(route(
        request(
            "POST",
            "/accounts/order",
            serde_json::json!({"ids": [first, second, other]}),
        ),
        state.clone(),
    ));
    json(route(
        request(
            "POST",
            &format!("/accounts/{other}"),
            serde_json::json!({"enabled": false}),
        ),
        state.clone(),
    ));
    assert!(db.pick_ai_account(&[]).unwrap().is_none());
    json(route(
        request(
            "POST",
            "/auth/opencode/key",
            serde_json::json!({"apiKey": "reconnected-synthetic", "accountId": second}),
        ),
        state.clone(),
    ));
    assert_eq!(db.pick_ai_account(&[]).unwrap().unwrap().id, second);
    json(route(
        request(
            "POST",
            &format!("/accounts/{second}"),
            serde_json::json!({"enabled": false}),
        ),
        state.clone(),
    ));
    json(route(
        request(
            "POST",
            "/auth/opencode/key",
            serde_json::json!({"apiKey": "still-disabled-synthetic", "accountId": second}),
        ),
        state,
    ));
    assert!(
        db.pick_ai_account(&[]).unwrap().is_none(),
        "Explicit reconnect must not enable a disabled account"
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn opencode_api_replacement_and_deletion_serialize_without_resurrecting_credentials() {
    if !isolated("opencode_api_replacement_and_deletion_serialize_without_resurrecting_credentials") {
        return;
    }
    let state = state();
    let root = state.config.data_dir.clone();
    std::env::set_var("OCR_DATA_DIR", &root);
    std::env::set_var("OCR_REVIEW_CLAUDE_AUTH_DIR", root.join("projection"));
    let connected = json(route(
        request(
            "POST",
            "/auth/opencode/key",
            serde_json::json!({"apiKey": "synthetic-race-first"}),
        ),
        state.clone(),
    ));
    let id = connected["accountId"].as_i64().unwrap();
    let barrier = Arc::new(std::sync::Barrier::new(2));
    let replacing = state.clone();
    let ready = barrier.clone();
    let replacement = std::thread::spawn(move || {
        ready.wait();
        route(
            request(
                "POST",
                "/auth/opencode/key",
                serde_json::json!({"apiKey": "synthetic-race-replacement", "accountId": id}),
            ),
            replacing,
        )
    });
    barrier.wait();
    json(route(
        request("DELETE", &format!("/accounts/{id}"), serde_json::Value::Null),
        state.clone(),
    ));
    let replaced = replacement.join().unwrap();
    assert!(!String::from_utf8_lossy(&replaced.body).contains("synthetic-race-replacement"));
    assert!(state.config.db.open().unwrap().ai_account(id).is_err());
    assert!(!crate::accounts::opencode_key_path(id).exists());
    assert!(!root
        .join(format!("projection/accounts/{id}/opencode/go-api-key"))
        .exists());
    std::fs::remove_dir_all(root).unwrap();
}
