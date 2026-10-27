//! Races SQLite ruled out by letting one writer in at a time, run here on two connections at once.

use std::sync::{Arc, Barrier};
use std::thread;

use super::*;

/// A PR with two finished, unpublished runs of the same head.
fn two_preview_runs(handle: &Db) -> (i64, i64, i64) {
    let db = handle.open().unwrap();
    db.ensure_temp_repo_for_index(1, "acme/api").unwrap();
    db.conn
        .execute(
            "INSERT INTO pull_requests (id, repo_id, number, title, author, head_sha, last_synced_at, created_at, updated_at)
             VALUES (1, 1, 7, 'PR', 'author', 'head', 1, 1, 1)",
            [],
        )
        .unwrap();
    let first = db.start_review_skeleton_with(1, None).unwrap();
    // One active review per PR: the first finishes before the second starts.
    db.conn
        .execute("UPDATE review_runs SET status = 'preview' WHERE id = $1", [first])
        .unwrap();
    let second = db.start_review_skeleton_with(1, None).unwrap();
    db.conn
        .execute(
            "UPDATE review_runs SET status = 'preview', human_selection_required = 0 WHERE pr_id = 1",
            [],
        )
        .unwrap();
    (1, first, second)
}

/// Runs `work` on two threads, each with its own connection, released together.
fn race<T: Send + 'static>(handle: &Db, work: impl Fn(Database, usize) -> T + Send + Sync + 'static) -> Vec<T> {
    let barrier = Arc::new(Barrier::new(2));
    let work = Arc::new(work);
    let threads: Vec<_> = (0..2)
        .map(|index| {
            let (barrier, work, handle) = (barrier.clone(), work.clone(), handle.clone());
            thread::spawn(move || {
                let db = handle.open().unwrap();
                barrier.wait();
                work(db, index)
            })
        })
        .collect();
    threads.into_iter().map(|thread| thread.join().unwrap()).collect()
}

#[test]
fn concurrent_posts_of_one_pr_head_let_exactly_one_through() {
    for _ in 0..10 {
        let handle = Db::for_test();
        let (pr_id, first, second) = two_preview_runs(&handle);
        let outcomes = race(&handle, move |db, index| {
            let run_id = if index == 0 { first } else { second };
            db.try_begin_review_post(run_id, pr_id, "head", "COMMENT", "comment", "manual", None)
        });
        let started: Vec<bool> = outcomes.into_iter().map(Result::unwrap).collect();
        assert_eq!(started.iter().filter(|started| **started).count(), 1, "{started:?}");
        let posting: i64 = handle
            .open()
            .unwrap()
            .conn
            .query_row("SELECT COUNT(*) FROM review_runs WHERE status = 'posting'", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(posting, 1);
    }
}

#[test]
fn concurrent_restoration_claims_hand_each_job_to_one_worker() {
    for _ in 0..10 {
        let handle = Db::for_test();
        let (_, run_id, _) = two_preview_runs(&handle);
        handle
            .open()
            .unwrap()
            .conn
            .execute(
                "INSERT INTO review_request_restorations (run_id, snapshot_json, marker, next_attempt_at)
                 VALUES ($1, $2, 'marker', 0)",
                params![
                    run_id,
                    r#"{"login":"reviewer","requested":{"reviewers":["reviewer"],"team_reviewers":[]},"events":["request"]}"#
                ],
            )
            .unwrap();
        let claims = race(&handle, |db, _| db.claim_review_request_restoration().unwrap());
        assert_eq!(claims.iter().filter(|claim| claim.is_some()).count(), 1);
    }
}

#[test]
fn concurrent_acknowledgements_of_one_cycle_start_one_generation() {
    for _ in 0..10 {
        let handle = Db::for_test();
        two_preview_runs(&handle);
        race(&handle, |db, _| db.acknowledge_review_cycle(1, false, None).unwrap());
        let generation: i64 = handle
            .open()
            .unwrap()
            .conn
            .query_row(
                "SELECT generation FROM queue_acknowledgements WHERE pr_id = 1",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(generation, 1, "a second generation would post the comment twice");
    }
}

#[test]
fn concurrent_attention_syncs_leave_one_notice_per_pr() {
    for _ in 0..10 {
        let handle = Db::for_test();
        two_preview_runs(&handle);
        race(&handle, |db, _| {
            db.upsert_pending_action("review_ready", 1, 1, "Review ready", "ready")
                .unwrap()
        });
        let notices: i64 = handle
            .open()
            .unwrap()
            .conn
            .query_row(
                "SELECT COUNT(*) FROM pending_actions WHERE pr_id = 1 AND status != 'resolved'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(notices, 1);
    }
}

#[test]
fn a_rerun_request_waiting_on_a_post_claim_is_refused() {
    let handle = Db::for_test();
    let (_, first, _) = two_preview_runs(&handle);
    let barrier = Arc::new(Barrier::new(2));
    let claim = {
        let (handle, barrier) = (handle.clone(), barrier.clone());
        thread::spawn(move || {
            // A post claim: holds the PR's lock and marks a run posting, without touching the PR.
            let db = handle.open().unwrap();
            let tx = db.conn.unchecked_transaction().unwrap();
            db.lock_pull_request(1).unwrap();
            db.conn
                .execute("UPDATE review_runs SET status = 'posting' WHERE id = $1", [first])
                .unwrap();
            barrier.wait();
            thread::sleep(std::time::Duration::from_millis(300));
            tx.commit().unwrap();
        })
    };
    barrier.wait();
    let intent = handle.open().unwrap().register_review_intent(1, true, false);
    claim.join().unwrap();
    assert!(intent.is_err(), "a rerun accepted during publication: {intent:?}");
}
