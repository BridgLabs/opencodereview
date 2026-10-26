//! Several server instances sharing one database.

use std::sync::{Arc, Barrier};
use std::thread;

use super::*;

fn pr_with_run(db: &Database, pr_id: i64) -> i64 {
    db.conn
        .execute(
            "INSERT INTO pull_requests (id, repo_id, number, title, author, head_sha, last_synced_at, created_at, updated_at)
             VALUES ($1, 1, $1, 'PR', 'author', 'head', 1, 1, 1)",
            [pr_id],
        )
        .unwrap();
    db.start_review_skeleton_with(pr_id, None).unwrap()
}

fn run_status(db: &Database, run_id: i64) -> String {
    db.conn
        .query_row("SELECT status FROM review_runs WHERE id = $1", [run_id], |row| {
            row.get(0)
        })
        .unwrap()
}

#[test]
fn only_work_of_stopped_instances_is_reclaimed() {
    let shared = Db::for_test();
    shared
        .open()
        .unwrap()
        .ensure_temp_repo_for_index(1, "acme/api")
        .unwrap();
    let live = shared.clone().with_instance("live");
    let stopped = shared.clone().with_instance("stopped");
    live.open().unwrap().register_instance("live", "host-a").unwrap();
    stopped.open().unwrap().register_instance("stopped", "host-b").unwrap();
    let live_run = pr_with_run(&live.open().unwrap(), 1);
    let stopped_run = pr_with_run(&stopped.open().unwrap(), 2);
    let untracked_run = pr_with_run(&shared.open().unwrap(), 3);
    let db = shared.open().unwrap();
    db.conn
        .execute(
            "UPDATE server_instances SET heartbeat_at = heartbeat_at - $1 WHERE id = 'stopped'",
            [INSTANCE_TIMEOUT_MS + 1_000],
        )
        .unwrap();
    assert_eq!(db.reclaim_orphaned_work().unwrap(), 2);
    assert_eq!(run_status(&db, live_run), "pending");
    assert_eq!(run_status(&db, stopped_run), "cancelled");
    assert_eq!(run_status(&db, untracked_run), "cancelled");
    let instances: i64 = db
        .conn
        .query_row("SELECT COUNT(*) FROM server_instances", [], |row| row.get(0))
        .unwrap();
    assert_eq!(instances, 1, "the stopped instance is forgotten");
}

#[test]
fn a_restarted_instance_reclaims_its_previous_run_at_once() {
    let shared = Db::for_test();
    shared
        .open()
        .unwrap()
        .ensure_temp_repo_for_index(1, "acme/api")
        .unwrap();
    let before = shared.clone().with_instance("before-restart");
    before
        .open()
        .unwrap()
        .register_instance("before-restart", "host-a")
        .unwrap();
    let run = pr_with_run(&before.open().unwrap(), 1);
    // Same host, fresh heartbeat: still its previous incarnation, whose reviews died with it.
    let db = shared.open().unwrap();
    db.register_instance("after-restart", "host-a").unwrap();
    db.reclaim_orphaned_work().unwrap();
    assert_eq!(run_status(&db, run), "cancelled");
}

#[test]
fn two_instances_cannot_both_start_a_review_of_one_pr() {
    let shared = Db::for_test();
    let db = shared.open().unwrap();
    db.ensure_temp_repo_for_index(1, "acme/api").unwrap();
    db.conn
        .execute(
            "INSERT INTO pull_requests (id, repo_id, number, title, author, head_sha, last_synced_at, created_at, updated_at)
             VALUES (1, 1, 1, 'PR', 'author', 'head', 1, 1, 1)",
            [],
        )
        .unwrap();
    let barrier = Arc::new(Barrier::new(2));
    let starts: Vec<_> = ["a", "b"]
        .into_iter()
        .map(|instance| {
            let (handle, barrier) = (shared.clone().with_instance(instance), barrier.clone());
            thread::spawn(move || {
                let db = handle.open().unwrap();
                barrier.wait();
                db.start_review_skeleton_with(1, None)
            })
        })
        .collect();
    let started = starts
        .into_iter()
        .map(|start| start.join().unwrap())
        .filter(Result::is_ok)
        .count();
    assert_eq!(started, 1);
}

#[test]
fn exclusive_work_runs_on_one_instance_at_a_time() {
    let shared = Db::for_test();
    let barrier = Arc::new(Barrier::new(2));
    let ran: Vec<_> = (0..2)
        .map(|_| {
            let (handle, barrier) = (shared.clone(), barrier.clone());
            thread::spawn(move || {
                barrier.wait();
                handle
                    .run_exclusive(42, || thread::sleep(std::time::Duration::from_millis(300)))
                    .unwrap()
                    .is_some()
            })
        })
        .collect();
    let ran = ran
        .into_iter()
        .map(|thread| thread.join().unwrap())
        .filter(|ran| *ran)
        .count();
    assert_eq!(ran, 1);
    assert!(
        shared.run_exclusive(42, || ()).unwrap().is_some(),
        "the lock ends with the work"
    );
}

#[test]
fn a_repository_indexing_on_a_live_instance_is_not_indexed_again() {
    let shared = Db::for_test();
    shared
        .open()
        .unwrap()
        .ensure_temp_repo_for_index(1, "acme/api")
        .unwrap();
    let a = shared.clone().with_instance("a");
    let b = shared.clone().with_instance("b");
    a.open().unwrap().register_instance("a", "host-a").unwrap();
    b.open().unwrap().register_instance("b", "host-b").unwrap();
    assert!(a.open().unwrap().claim_repo_index(1, Some("main")).unwrap().is_some());
    assert!(b.open().unwrap().claim_repo_index(1, Some("main")).unwrap().is_none());
    // Once instance a stops, the index can be taken over.
    let db = shared.open().unwrap();
    db.conn
        .execute(
            "UPDATE server_instances SET heartbeat_at = heartbeat_at - $1 WHERE id = 'a'",
            [INSTANCE_TIMEOUT_MS + 1_000],
        )
        .unwrap();
    assert!(b.open().unwrap().claim_repo_index(1, Some("main")).unwrap().is_some());
}

#[test]
fn only_posts_of_stopped_instances_are_reconciled() {
    let shared = Db::for_test();
    shared
        .open()
        .unwrap()
        .ensure_temp_repo_for_index(1, "acme/api")
        .unwrap();
    let live = shared.clone().with_instance("live");
    live.open().unwrap().register_instance("live", "host-a").unwrap();
    let db = shared.open().unwrap();
    let live_post = pr_with_run(&live.open().unwrap(), 1);
    let stopped_post = pr_with_run(&db, 2);
    db.conn
        .execute(
            "UPDATE review_runs SET status = 'posting', owner_instance = CASE id WHEN $1 THEN 'live' ELSE 'gone' END",
            [live_post],
        )
        .unwrap();
    let posts: Vec<i64> = db
        .interrupted_review_posts()
        .unwrap()
        .iter()
        .map(|post| post.run_id)
        .collect();
    assert_eq!(posts, [stopped_post], "a live instance's post in flight is left alone");
}

#[test]
fn work_requested_during_another_run_waits_for_it() {
    let shared = Db::for_test();
    let (first, second) = (shared.clone(), shared.clone());
    let started = Arc::new(Barrier::new(2));
    let holder = {
        let started = started.clone();
        thread::spawn(move || {
            first
                .run_exclusive(7, || {
                    started.wait();
                    thread::sleep(std::time::Duration::from_millis(300));
                    std::time::Instant::now()
                })
                .unwrap()
                .unwrap()
        })
    };
    started.wait();
    let ran_at = second.run_exclusive_after(7, std::time::Instant::now).unwrap();
    let finished_at = holder.join().unwrap();
    assert!(
        ran_at >= finished_at,
        "the request ran during the other run instead of after it"
    );
}
