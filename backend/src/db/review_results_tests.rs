use super::*;
#[test]
fn failed_clean_results_due_filters_by_status_state_age_and_installation() {
    let db = Db::for_test().open().expect("open db");
    db.conn
        .execute(
            "INSERT INTO repos (owner, name, full_name, is_private, created_at, updated_at)
             VALUES ('acme', 'api', 'acme/api', 1, 1, 1)",
            [],
        )
        .expect("repo");
    let repo_id = db.conn.last_insert_rowid();
    let installation_id = db.installation_id().expect("installation id");
    let add_pr = |number: i64, state: &str| {
        db.conn
            .execute(
                "INSERT INTO pull_requests (repo_id, number, title, author, head_sha, state, last_synced_at, created_at, updated_at)
                 VALUES ($1, $2, 'PR', 'alice', 'head', $3, 1, 1, 1)",
                params![repo_id, number, state],
            )
            .expect("pr");
        db.conn.last_insert_rowid()
    };
    let add_result = |pr_id: i64, installation: &str, effective_state: &str, status: &str, updated_at: i64| {
        db.conn
            .execute(
                "INSERT INTO review_runs (pr_id, head_sha, status, created_at, updated_at, model) VALUES ($1, 'head', 'completed', 1, 1, 'gpt')",
                params![pr_id],
            )
            .expect("run");
        let run_id = db.conn.last_insert_rowid();
        db.conn
            .execute(
                "INSERT INTO review_results (
                   pr_id, installation_id, marker_key, run_id, observed_head_sha, reviewed_head_sha,
                   observation, effective_state, publication_status, reviewer_version, skill_version,
                   created_at, updated_at
                 ) VALUES ($1, $2, 'marker', $3, 'head', 'head', 'clean', $4, $5, 'v1', 'v1', 1, $6)",
                params![pr_id, installation, run_id, effective_state, status, updated_at],
            )
            .expect("result");
        db.conn.last_insert_rowid()
    };
    let cutoff = 1_000_000i64;
    let due_old = add_result(add_pr(1, "open"), &installation_id, "clean", "failed", cutoff - 100);
    let due_new = add_result(add_pr(2, "open"), &installation_id, "clean", "failed", cutoff - 10);
    add_result(add_pr(3, "open"), &installation_id, "clean", "failed", cutoff + 10);
    add_result(add_pr(4, "open"), &installation_id, "awaiting_action", "failed", 1);
    add_result(add_pr(5, "open"), &installation_id, "clean", "posted", 1);
    add_result(add_pr(6, "open"), &installation_id, "clean", "pending", 1);
    add_result(add_pr(7, "open"), &installation_id, "clean", "disabled", 1);
    add_result(add_pr(8, "closed"), &installation_id, "clean", "failed", 1);
    add_result(add_pr(9, "open"), "other-installation", "clean", "failed", 1);
    let due = db.failed_clean_results_due(cutoff).expect("due results");
    assert_eq!(
        due.iter().map(|result| result.id).collect::<Vec<_>>(),
        vec![due_old, due_new]
    );
    assert!(db.failed_clean_results_due(0).expect("none due").is_empty());
}

#[test]
fn failed_clean_results_due_requires_the_current_run_at_current_head() {
    let due_fixture = || {
        let db = Db::for_test().open().expect("open db");
        db.conn
            .execute(
                "INSERT INTO repos (id, owner, name, full_name, is_private, created_at, updated_at)
                 VALUES (1, 'acme', 'api', 'acme/api', 1, 1, 1)",
                [],
            )
            .expect("repo");
        db.conn
            .execute(
                "INSERT INTO pull_requests (id, repo_id, number, title, author, head_sha, last_synced_at, created_at, updated_at)
                 VALUES (1, 1, 7, 'PR', 'alice', 'head-a', 1, 1, 1)",
                [],
            )
            .expect("pr");
        db.conn
            .execute(
                "INSERT INTO review_runs (pr_id, head_sha, status, created_at, updated_at, model) VALUES (1, 'head-a', 'completed', 1, 1, 'gpt')",
                [],
            )
            .expect("run");
        let run_id = db.conn.last_insert_rowid();
        let installation_id = db.installation_id().expect("installation id");
        db.conn
            .execute(
                "INSERT INTO review_results (
                   pr_id, installation_id, marker_key, run_id, observed_head_sha, reviewed_head_sha,
                   observation, effective_state, publication_status, reviewer_version, skill_version,
                   created_at, updated_at
                 ) VALUES (1, $1, 'marker', $2, 'head-a', 'head-a', 'clean', 'clean', 'failed', 'v1', 'v1', 1, 1)",
                params![installation_id, run_id],
            )
            .expect("result");
        db
    };
    assert_eq!(due_fixture().failed_clean_results_due(10).unwrap().len(), 1);
    {
        let db = due_fixture();
        db.register_review_intent(1, true, false).unwrap();
        assert!(db.failed_clean_results_due(10).unwrap().is_empty());
    }
    {
        let db = due_fixture();
        db.set_force_full_next_review(1, true).unwrap();
        assert!(db.failed_clean_results_due(10).unwrap().is_empty());
    }
    {
        let db = due_fixture();
        db.conn
            .execute(
                "INSERT INTO review_runs (pr_id, head_sha, status, created_at, updated_at, model) VALUES (1, 'head-a', 'pending', 1, 1, 'gpt')",
                [],
            )
            .unwrap();
        assert!(db.failed_clean_results_due(10).unwrap().is_empty());
    }
    {
        let db = due_fixture();
        db.conn
            .execute("UPDATE pull_requests SET review_lane = 'queued' WHERE id = 1", [])
            .unwrap();
        assert!(db.failed_clean_results_due(10).unwrap().is_empty());
    }
    {
        let db = due_fixture();
        db.conn
            .execute("UPDATE pull_requests SET head_sha = 'head-b' WHERE id = 1", [])
            .unwrap();
        assert!(db.failed_clean_results_due(10).unwrap().is_empty());
    }
}
