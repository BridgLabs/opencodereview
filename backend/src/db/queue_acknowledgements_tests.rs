use super::*;
use std::sync::atomic::{AtomicU64, Ordering};

static COUNTER: AtomicU64 = AtomicU64::new(0);

fn temp_db() -> Database {
    let root = std::env::temp_dir().join(format!(
        "ocr-ack-db-{}-{}",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    let db = crate::db::Db::for_test_at(root.join("test.sqlite"))
        .open()
        .expect("open db");
    db.ensure_temp_repo_for_index(1, "acme/api").expect("repo");
    db.conn
        .execute(
            "INSERT INTO pull_requests (id, repo_id, number, title, author, head_sha, last_synced_at, created_at, updated_at)
             VALUES (1, 1, 7, 'PR', 'author', 'head-a', 1, 1, 1)",
            [],
        )
        .expect("pr");
    db
}

fn ack_state(db: &Database) -> (i64, String, Option<i64>, i64) {
    db.conn
        .query_row(
            "SELECT generation, status, comment_id, attempts FROM queue_acknowledgements WHERE pr_id = 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .expect("ack row")
}

#[test]
fn acknowledge_starts_generation_one_pending_then_stays_put() {
    let db = temp_db();
    db.acknowledge_review_cycle(1, false, None).unwrap();
    assert_eq!(ack_state(&db), (1, "pending".to_string(), None, 0));
    db.acknowledge_review_cycle(1, false, None).unwrap();
    assert_eq!(ack_state(&db).0, 1);
    db.record_queue_acknowledgement(1, 1, "pending", Ok(Some(42))).unwrap();
    db.retire_queue_acknowledgement(1, "head-a", None).unwrap();
    db.record_queue_acknowledgement(1, 1, "retiring", Ok(None)).unwrap();
    assert_eq!(ack_state(&db), (1, "retired".to_string(), Some(42), 0));
    // A row for the same head and scope is left alone whatever its status.
    db.acknowledge_review_cycle(1, false, None).unwrap();
    assert_eq!(ack_state(&db), (1, "retired".to_string(), Some(42), 0));
}

#[test]
fn acknowledge_bumps_generation_on_refresh_head_or_scope_change() {
    let db = temp_db();
    db.acknowledge_review_cycle(1, false, None).unwrap();
    db.record_queue_acknowledgement(1, 1, "pending", Ok(Some(42))).unwrap();
    db.acknowledge_review_cycle(1, true, None).unwrap();
    assert_eq!(ack_state(&db), (2, "pending".to_string(), None, 0));
    db.conn
        .execute("UPDATE pull_requests SET head_sha = 'head-b' WHERE id = 1", [])
        .unwrap();
    db.acknowledge_review_cycle(1, false, None).unwrap();
    assert_eq!(ack_state(&db).0, 3);
    db.conn
        .execute(
            "UPDATE pull_requests SET review_scope_revision = review_scope_revision + 1 WHERE id = 1",
            [],
        )
        .unwrap();
    db.acknowledge_review_cycle(1, false, None).unwrap();
    assert_eq!(ack_state(&db).0, 4);
}

#[test]
fn retire_only_affects_matching_head_and_live_status() {
    let db = temp_db();
    db.acknowledge_review_cycle(1, false, None).unwrap();
    db.retire_queue_acknowledgement(1, "other-head", None).unwrap();
    assert_eq!(ack_state(&db).1, "pending");
    db.retire_queue_acknowledgement(1, "head-a", None).unwrap();
    assert_eq!(ack_state(&db).1, "retiring");
    db.acknowledge_review_cycle(1, true, None).unwrap();
    db.record_queue_acknowledgement(1, 2, "pending", Ok(Some(9))).unwrap();
    assert_eq!(ack_state(&db).1, "posted");
    db.retire_queue_acknowledgement(1, "head-a", None).unwrap();
    db.record_queue_acknowledgement(1, 2, "retiring", Ok(None)).unwrap();
    assert_eq!(ack_state(&db).1, "retired");
    db.retire_queue_acknowledgement(1, "head-a", None).unwrap();
    assert_eq!(ack_state(&db).1, "retired");
}

#[test]
fn record_queue_acknowledgement_is_guarded_and_backs_off() {
    let db = temp_db();
    db.acknowledge_review_cycle(1, false, None).unwrap();
    // A newer generation or a mid-call retirement wins over the stale record.
    db.record_queue_acknowledgement(1, 99, "pending", Ok(Some(1))).unwrap();
    assert_eq!(ack_state(&db).1, "pending");
    db.record_queue_acknowledgement(1, 1, "retiring", Ok(None)).unwrap();
    assert_eq!(ack_state(&db).1, "pending");
    db.record_queue_acknowledgement(1, 1, "pending", Err("boom")).unwrap();
    let (attempts, next_at, last_error): (i64, i64, Option<String>) = db
        .conn
        .query_row(
            "SELECT attempts, next_attempt_at, last_error FROM queue_acknowledgements WHERE pr_id = 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap();
    assert_eq!(attempts, 1);
    assert_eq!(last_error.as_deref(), Some("boom"));
    let now = now_millis();
    assert!(next_at > now && next_at <= now + 120_000);
    db.record_queue_acknowledgement(1, 1, "pending", Err("boom again"))
        .unwrap();
    let (attempts, later_at): (i64, i64) = db
        .conn
        .query_row(
            "SELECT attempts, next_attempt_at FROM queue_acknowledgements WHERE pr_id = 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(attempts, 2);
    assert!(later_at > next_at);
}

#[test]
fn due_queue_acknowledgements_selects_open_pending_and_any_retiring() {
    let db = temp_db();
    let now = now_millis();
    db.acknowledge_review_cycle(1, false, None).unwrap();
    let due = db.due_queue_acknowledgements(now).unwrap();
    assert_eq!(due.len(), 1);
    assert_eq!(
        (
            due[0].full_name.as_str(),
            due[0].number,
            due[0].generation,
            due[0].status.as_str()
        ),
        ("acme/api", 7, 1, "pending")
    );
    db.conn
        .execute("UPDATE pull_requests SET state = 'closed' WHERE id = 1", [])
        .unwrap();
    assert!(db.due_queue_acknowledgements(now).unwrap().is_empty());
    db.retire_queue_acknowledgement(1, "head-a", None).unwrap();
    let due = db.due_queue_acknowledgements(now).unwrap();
    assert_eq!(due.len(), 1);
    assert_eq!(due[0].status, "retiring");
    db.conn
        .execute(
            "UPDATE queue_acknowledgements SET next_attempt_at = $1 WHERE pr_id = 1",
            params![now + 60_000],
        )
        .unwrap();
    assert!(db.due_queue_acknowledgements(now).unwrap().is_empty());
}

#[test]
fn fresh_author_command_on_active_pr_bumps_the_acknowledgement() {
    let db = temp_db();
    db.save_repo_subscription(1, true, "manual", true, false).unwrap();
    let (enabled_at, _) = db.comment_scan_cursor(1).unwrap();
    let activation = (1i64, "head-a", "marker", "body");

    // First command admits the PR and records the acknowledgement right away.
    assert!(db
        .consume_activation_command(1, enabled_at, 11, Some(activation))
        .unwrap());
    assert_eq!(ack_state(&db), (1, "pending".to_string(), None, 0));
    // The job start's acknowledge for the same head and scope is a no-op.
    db.acknowledge_review_cycle(1, false, None).unwrap();
    assert_eq!(ack_state(&db).0, 1);

    // A fresh command while queued at the same head reposts the acknowledgement.
    assert!(!db
        .consume_activation_command(1, enabled_at, 12, Some(activation))
        .unwrap());
    assert_eq!(ack_state(&db).0, 2);
    // A replayed receipt does nothing.
    assert!(!db
        .consume_activation_command(1, enabled_at, 12, Some(activation))
        .unwrap());
    assert_eq!(ack_state(&db).0, 2);
    // Same while the run is reviewing.
    db.conn
        .execute("UPDATE pull_requests SET review_lane = 'reviewing' WHERE id = 1", [])
        .unwrap();
    assert!(!db
        .consume_activation_command(1, enabled_at, 13, Some(activation))
        .unwrap());
    assert_eq!(ack_state(&db).0, 3);

    // A command after the cycle was decided at this head does nothing.
    let run_id = db.start_review_skeleton_with(1, None).unwrap();
    db.finalize_review_decision(
        run_id,
        1,
        "approve_post",
        "posted",
        "posted",
        "Done",
        Some("APPROVE"),
        "head-a",
        Some("you"),
    )
    .unwrap();
    assert!(!db
        .consume_activation_command(1, enabled_at, 14, Some(activation))
        .unwrap());
    assert_eq!(ack_state(&db).0, 3);
}

#[test]
fn acknowledge_carries_the_row_across_a_replayed_intent() {
    let db = temp_db();
    // The explicit request stored the intent and acked the bumped revision.
    db.register_review_intent(1, true, false).unwrap();
    db.acknowledge_review_cycle(1, false, None).unwrap();
    db.record_queue_acknowledgement(1, 1, "pending", Ok(Some(42))).unwrap();
    let replayed = db.stored_review_intent_revision(1).unwrap();
    assert_eq!(replayed, Some(1));
    // The queue drain replays the stored intent and bumps the revision once more.
    db.register_review_intent(1, false, false).unwrap();
    db.acknowledge_review_cycle(1, false, replayed).unwrap();
    let (generation, status, comment_id, scope): (i64, String, Option<i64>, i64) = db
        .conn
        .query_row(
            "SELECT generation, status, comment_id, scope_revision FROM queue_acknowledgements WHERE pr_id = 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .unwrap();
    assert_eq!(
        (generation, status.as_str(), comment_id, scope),
        (1, "posted", Some(42), 2)
    );
}

#[test]
fn acknowledge_still_bumps_when_the_scope_moved_since_the_request() {
    let db = temp_db();
    db.register_review_intent(1, true, false).unwrap();
    db.acknowledge_review_cycle(1, false, None).unwrap();
    // A base change while queued moved the scope beyond the replayed revision.
    db.invalidate_review_scope(1, None, "PR base changed").unwrap();
    let replayed = db.stored_review_intent_revision(1).unwrap();
    db.register_review_intent(1, false, false).unwrap();
    db.acknowledge_review_cycle(1, false, replayed).unwrap();
    assert_eq!(ack_state(&db).0, 2);
}

#[test]
fn retire_queue_acknowledgement_skips_a_newer_cycle() {
    let fixture = || {
        let db = temp_db();
        db.acknowledge_review_cycle(1, false, None).unwrap();
        db.conn
            .execute(
                "INSERT INTO review_runs (pr_id, head_sha, status, created_at, updated_at, model) VALUES (1, 'head-a', 'completed', 1, 1, 'gpt')",
                [],
            )
            .expect("run");
        let run_id = db.conn.last_insert_rowid();
        (db, run_id)
    };
    {
        let (db, run_id) = fixture();
        db.conn
            .execute(
                "INSERT INTO review_runs (pr_id, head_sha, status, created_at, updated_at, model) VALUES (1, 'head-a', 'running', 1, 1, 'gpt')",
                [],
            )
            .unwrap();
        db.retire_queue_acknowledgement(1, "head-a", Some(run_id)).unwrap();
        assert_eq!(ack_state(&db).1, "pending");
    }
    {
        let (db, run_id) = fixture();
        db.set_force_full_next_review(1, true).unwrap();
        db.retire_queue_acknowledgement(1, "head-a", Some(run_id)).unwrap();
        assert_eq!(ack_state(&db).1, "pending");
    }
    {
        let (db, run_id) = fixture();
        db.retire_queue_acknowledgement(1, "head-a", Some(run_id)).unwrap();
        assert_eq!(ack_state(&db).1, "retiring");
    }
}

#[test]
fn finished_reviews_retire_their_leftover_acknowledgement() {
    let db = temp_db();
    db.acknowledge_review_cycle(1, false, None).unwrap();
    db.record_queue_acknowledgement(1, 1, "pending", Ok(Some(42))).unwrap();
    db.conn
        .execute(
            "INSERT INTO review_runs (pr_id, head_sha, status, created_at, updated_at) VALUES (1, 'head-a', 'running', 1, 1)",
            [],
        )
        .unwrap();
    // A review still running keeps its comment.
    assert_eq!(db.retire_finished_queue_acknowledgements().unwrap(), 0);
    db.conn.execute("UPDATE review_runs SET status = 'posted'", []).unwrap();
    assert_eq!(db.retire_finished_queue_acknowledgements().unwrap(), 1);
    assert_eq!(ack_state(&db), (1, "retiring".to_string(), Some(42), 0));
}
