use super::*;

#[test]
fn a_starting_review_pins_no_opencode_engine_before_its_list_was_read() {
    let db = Db::for_test().open().unwrap();
    db.create_ai_account("claude", "Work").unwrap();
    // No OpenCode choice and no list read yet: nothing to pin, so OpenCode stages read it later.
    assert_eq!(
        db.review_engine_offline("opencode").unwrap(),
        (String::new(), String::new())
    );
    let pinned = crate::review::PinnedEngines::current(&db).unwrap();
    assert_eq!(pinned.engine("opencode"), None);
    assert_eq!(
        pinned.engine("claude"),
        Some(("claude-opus-5-5".to_string(), "medium".to_string()))
    );
    // A stored choice is pinned by its shape, without the list.
    db.save_setting_value(
        "review_engine_opencode",
        r#"{"model":"opencode/glm-5.3","effort":"high"}"#,
    )
    .unwrap();
    assert_eq!(
        crate::review::PinnedEngines::current(&db).unwrap().engine("opencode"),
        Some(("opencode/glm-5.3".to_string(), "high".to_string()))
    );
}

#[test]
fn opencode_plans_count_only_accounts_reviews_can_run_on() {
    let db = Db::for_test().open().unwrap();
    assert_eq!(db.engine_plans("claude").unwrap(), None);
    let go = db.create_ai_account("opencode", "Go").unwrap();
    let zen = db.create_ai_account("opencode", "Zen").unwrap();
    let signed_out = db.create_ai_account("opencode", "Old").unwrap();
    db.create_ai_account("claude", "Work").unwrap();
    assert_eq!(
        db.opencode_review_accounts().unwrap(),
        vec![go.id, zen.id, signed_out.id]
    );
    // A disabled account and a signed-out one no longer decide which models are offered.
    db.set_ai_account_enabled(go.id, false).unwrap();
    db.conn
        .execute(
            "UPDATE ai_accounts SET status = 'auth_failed' WHERE id = $1",
            params![signed_out.id],
        )
        .unwrap();
    assert_eq!(db.opencode_review_accounts().unwrap(), vec![zen.id]);
}
