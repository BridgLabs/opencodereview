use super::*;
use crate::db::{Database, PullRequestInput};
use crate::review_archive::{hash, Archive, Scope, SourceEntry};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

static TEST_DIR_COUNTER: AtomicU64 = AtomicU64::new(0);

struct Directory(std::path::PathBuf);
impl Directory {
    fn new() -> Self {
        let counter = TEST_DIR_COUNTER.fetch_add(1, Ordering::SeqCst);
        let path = std::env::temp_dir().join(format!(
            "ocr-retention-test-{}-{}-{}",
            std::process::id(),
            counter,
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&path).unwrap();
        Self(path.canonicalize().unwrap())
    }
}
impl Drop for Directory {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn sample_archive(suffix: &str) -> Archive {
    let head = hash(format!("{suffix}:head").as_bytes())[..40].to_string();
    let base = hash(format!("{suffix}:base").as_bytes())[..40].to_string();
    let merge_base = hash(format!("{suffix}:merge_base").as_bytes())[..40].to_string();
    let scope = Scope {
        host: "github.com".into(),
        repository: "acme/repo".into(),
        head_sha: head.clone(),
        base_sha: base,
        merge_base_sha: merge_base,
        prior_sha: None,
    };
    let content = format!("content {suffix}").into_bytes();
    let blob_id = hash(&content)[..40].to_string();
    let sources = vec![SourceEntry {
        path: "src/main.rs".into(),
        revision: head,
        mode: "100644".into(),
        git_blob_id: blob_id,
        bytes: content.len() as u64,
    }];
    Archive::build(scope, sources, &AtomicBool::new(false), |_| Ok(content.clone())).unwrap()
}

fn setup_env(dir: &Directory) -> (Database, ArchiveStore, RetentionCoordinator) {
    let db_path = dir.0.join("test.sqlite");
    let archive_root = dir.0.join("archives");
    let db = crate::db::Db::for_test_at(db_path.clone()).open().unwrap();
    db.ensure_temp_repo_for_index(1, "acme/repo").unwrap();
    let store = ArchiveStore::open(&archive_root, &db.handle()).unwrap();
    let leases = ArchiveLeaseTracker::new();
    let status = Arc::new(Mutex::new(ArchiveStorageStatus::default()));
    let notify = Arc::new((Mutex::new(false), Condvar::new()));
    let coordinator = RetentionCoordinator::new(
        crate::db::Db::for_test_at(db_path),
        archive_root,
        leases,
        status,
        notify,
    );
    (db, store, coordinator)
}

#[test]
fn retention_worker_cleans_up_expired_merged_prs() {
    let dir = Directory::new();
    let (db, mut store, coordinator) = setup_env(&dir);

    let pr_id = db
        .upsert_pull_request(&PullRequestInput {
            repo_id: 1,
            number: 101,
            title: "Merged PR".into(),
            author: "alice".into(),
            author_avatar_url: None,
            is_draft: false,
            head_sha: "head_1".into(),
            base_sha: None,
            base_ref: Some("main".into()),
            head_ref: Some("feature".into()),
            default_branch: Some("main".into()),
            url: None,
            lane: "merged".into(),
            status_reason: None,
        })
        .unwrap();

    let run_id = db.start_review_skeleton_with(pr_id, Some("alice")).unwrap();
    db.set_review_run_running(run_id, "[]").unwrap();
    db.finish_review_run(run_id, "Approved", None, "[]").unwrap();

    let archive = sample_archive("expired_merged");
    let descriptor = archive.descriptor();
    store.deposit_and_link(&archive, &format!("review:{run_id}")).unwrap();

    let sealed_path = dir
        .0
        .join("archives")
        .join("sealed")
        .join(format!("{}.archive", descriptor.id));
    assert!(sealed_path.exists());

    let now_ms = crate::sandbox::now();
    let two_days_ago = now_ms - 2 * MS_PER_DAY;
    db.mark_pr_terminal_with_merged_at(pr_id, "merged", "Merged on GitHub", Some(two_days_ago))
        .unwrap();

    let cancel = AtomicBool::new(false);
    let result = coordinator.perform_maintenance(&cancel).unwrap();

    assert_eq!(result.packages_removed, 1);
    assert!(result.bytes_removed > 0);
    assert_eq!(result.deferrals, 0);
    assert!(!sealed_path.exists());
    assert!(store.list_artifacts().unwrap().is_empty());

    // Review run and PR records in DB remain intact
    let (state, merged_at) = db.pr_retention_info(pr_id).unwrap().unwrap();
    assert_eq!(state, "merged");
    assert_eq!(merged_at, Some(two_days_ago));
    let run_record = db.review_run_by_id(run_id).unwrap().unwrap();
    assert!(["completed", "superseded"].contains(&run_record.status.as_str()));

    // Status updated
    let status = coordinator.status.lock().unwrap();
    assert_eq!(status.last_packages_removed, 1);
    assert!(status.last_cleanup_at.is_some());
    assert_eq!(status.last_error, None);
}

#[test]
fn retention_worker_preserves_unmerged_and_recently_merged_prs() {
    let dir = Directory::new();
    let (db, mut store, coordinator) = setup_env(&dir);
    let now_ms = crate::sandbox::now();

    // PR 1: open
    let pr_open = db
        .upsert_pull_request(&PullRequestInput {
            repo_id: 1,
            number: 201,
            title: "Open PR".into(),
            author: "bob".into(),
            author_avatar_url: None,
            is_draft: false,
            head_sha: "head_open".into(),
            base_sha: None,
            base_ref: Some("main".into()),
            head_ref: Some("feature".into()),
            default_branch: Some("main".into()),
            url: None,
            lane: "reviewing".into(),
            status_reason: None,
        })
        .unwrap();
    let run_open = db.start_review_skeleton_with(pr_open, Some("bob")).unwrap();
    db.finish_review_run(run_open, "Done", None, "[]").unwrap();
    let archive_open = sample_archive("open");
    store
        .deposit_and_link(&archive_open, &format!("review:{run_open}"))
        .unwrap();

    // PR 2: closed without merge
    let pr_closed = db
        .upsert_pull_request(&PullRequestInput {
            repo_id: 1,
            number: 202,
            title: "Closed PR".into(),
            author: "carol".into(),
            author_avatar_url: None,
            is_draft: false,
            head_sha: "head_closed".into(),
            base_sha: None,
            base_ref: Some("main".into()),
            head_ref: Some("feature".into()),
            default_branch: Some("main".into()),
            url: None,
            lane: "closed".into(),
            status_reason: None,
        })
        .unwrap();
    db.mark_pr_terminal(pr_closed, "closed", "Closed without merge")
        .unwrap();
    let run_closed = db.start_review_skeleton_with(pr_closed, Some("carol")).unwrap();
    db.finish_review_run(run_closed, "Done", None, "[]").unwrap();
    let archive_closed = sample_archive("closed");
    store
        .deposit_and_link(&archive_closed, &format!("review:{run_closed}"))
        .unwrap();

    // PR 3: merged only 3 hours ago (retention is 1 day)
    let pr_recent = db
        .upsert_pull_request(&PullRequestInput {
            repo_id: 1,
            number: 203,
            title: "Recent Merged PR".into(),
            author: "dan".into(),
            author_avatar_url: None,
            is_draft: false,
            head_sha: "head_recent".into(),
            base_sha: None,
            base_ref: Some("main".into()),
            head_ref: Some("feature".into()),
            default_branch: Some("main".into()),
            url: None,
            lane: "merged".into(),
            status_reason: None,
        })
        .unwrap();
    let three_hours_ago = now_ms - 3 * 3600 * 1000;
    db.mark_pr_terminal_with_merged_at(pr_recent, "merged", "Merged", Some(three_hours_ago))
        .unwrap();
    let run_recent = db.start_review_skeleton_with(pr_recent, Some("dan")).unwrap();
    db.finish_review_run(run_recent, "Done", None, "[]").unwrap();
    let archive_recent = sample_archive("recent");
    store
        .deposit_and_link(&archive_recent, &format!("review:{run_recent}"))
        .unwrap();

    let cancel = AtomicBool::new(false);
    let result = coordinator.perform_maintenance(&cancel).unwrap();

    assert_eq!(result.packages_removed, 0);
    assert_eq!(result.bytes_removed, 0);
    assert_eq!(result.deferrals, 3);
    assert_eq!(store.list_artifacts().unwrap().len(), 3);
}

#[test]
fn retention_worker_preserves_active_leases() {
    let dir = Directory::new();
    let (db, mut store, coordinator) = setup_env(&dir);

    let pr_id = db
        .upsert_pull_request(&PullRequestInput {
            repo_id: 1,
            number: 301,
            title: "Leased PR".into(),
            author: "eve".into(),
            author_avatar_url: None,
            is_draft: false,
            head_sha: "head_leased".into(),
            base_sha: None,
            base_ref: Some("main".into()),
            head_ref: Some("feature".into()),
            default_branch: Some("main".into()),
            url: None,
            lane: "merged".into(),
            status_reason: None,
        })
        .unwrap();

    let run_id = db.start_review_skeleton_with(pr_id, Some("eve")).unwrap();
    db.finish_review_run(run_id, "Done", None, "[]").unwrap();
    let archive = sample_archive("leased");
    store.deposit_and_link(&archive, &format!("review:{run_id}")).unwrap();

    let now_ms = crate::sandbox::now();
    let five_days_ago = now_ms - 5 * MS_PER_DAY;
    db.mark_pr_terminal_with_merged_at(pr_id, "merged", "Merged", Some(five_days_ago))
        .unwrap();

    // Acquire run lease
    let lease = coordinator.leases.acquire(run_id);
    assert!(coordinator.leases.is_active(run_id));

    let cancel = AtomicBool::new(false);
    let result = coordinator.perform_maintenance(&cancel).unwrap();
    assert_eq!(result.packages_removed, 0);
    assert_eq!(result.deferrals, 1);
    assert_eq!(store.list_artifacts().unwrap().len(), 1);

    // Release lease
    drop(lease);
    assert!(!coordinator.leases.is_active(run_id));

    let result2 = coordinator.perform_maintenance(&cancel).unwrap();
    assert_eq!(result2.packages_removed, 1);
    assert_eq!(store.list_artifacts().unwrap().len(), 0);
}

#[test]
fn retention_worker_preserves_shared_artifacts_until_all_runs_eligible() {
    let dir = Directory::new();
    let (db, mut store, coordinator) = setup_env(&dir);
    let now_ms = crate::sandbox::now();

    // PR 1: merged 3 days ago
    let pr1 = db
        .upsert_pull_request(&PullRequestInput {
            repo_id: 1,
            number: 401,
            title: "Shared PR 1".into(),
            author: "frank".into(),
            author_avatar_url: None,
            is_draft: false,
            head_sha: "head_shared".into(),
            base_sha: None,
            base_ref: Some("main".into()),
            head_ref: Some("feature".into()),
            default_branch: Some("main".into()),
            url: None,
            lane: "merged".into(),
            status_reason: None,
        })
        .unwrap();
    let run1 = db.start_review_skeleton_with(pr1, Some("frank")).unwrap();
    db.finish_review_run(run1, "Done", None, "[]").unwrap();
    db.mark_pr_terminal_with_merged_at(pr1, "merged", "Merged", Some(now_ms - 3 * MS_PER_DAY))
        .unwrap();

    // PR 2: still open
    let pr2 = db
        .upsert_pull_request(&PullRequestInput {
            repo_id: 1,
            number: 402,
            title: "Shared PR 2".into(),
            author: "grace".into(),
            author_avatar_url: None,
            is_draft: false,
            head_sha: "head_shared".into(),
            base_sha: None,
            base_ref: Some("main".into()),
            head_ref: Some("feature".into()),
            default_branch: Some("main".into()),
            url: None,
            lane: "reviewing".into(),
            status_reason: None,
        })
        .unwrap();
    let run2 = db.start_review_skeleton_with(pr2, Some("grace")).unwrap();
    db.finish_review_run(run2, "Done", None, "[]").unwrap();

    // Deposit one shared archive linked to both runs
    let archive = sample_archive("shared");
    store.deposit_and_link(&archive, &format!("review:{run1}")).unwrap();
    store.link(&format!("review:{run2}"), &archive.descriptor()).unwrap();

    let receipts = store.receipts_for_artifact(&archive.descriptor().id).unwrap();
    assert_eq!(receipts.len(), 2);

    let cancel = AtomicBool::new(false);
    let result1 = coordinator.perform_maintenance(&cancel).unwrap();
    assert_eq!(result1.packages_removed, 0);
    assert_eq!(store.list_artifacts().unwrap().len(), 1);

    // Now PR 2 is also merged 2 days ago
    db.mark_pr_terminal_with_merged_at(pr2, "merged", "Merged", Some(now_ms - 2 * MS_PER_DAY))
        .unwrap();

    let result2 = coordinator.perform_maintenance(&cancel).unwrap();
    assert_eq!(result2.packages_removed, 1);
    assert_eq!(store.list_artifacts().unwrap().len(), 0);
}

#[test]
fn retention_worker_preserves_replay_or_unknown_receipts() {
    let dir = Directory::new();
    let (_db, mut store, coordinator) = setup_env(&dir);

    let archive = sample_archive("replay");
    // Link with a non-review owner prefix
    store.deposit_and_link(&archive, "external:trace_99").unwrap();

    let cancel = AtomicBool::new(false);
    let result = coordinator.perform_maintenance(&cancel).unwrap();
    assert_eq!(result.packages_removed, 0);
    assert_eq!(result.deferrals, 1);
    assert_eq!(store.list_artifacts().unwrap().len(), 1);
}

#[test]
fn retention_coordinator_recovers_crashed_deletions() {
    let dir = Directory::new();
    let (_db, mut store, _coordinator) = setup_env(&dir);

    let archive = sample_archive("crash_test");
    let descriptor = archive.descriptor();
    store.deposit_and_link(&archive, "review:1").unwrap();

    let sealed_path = dir
        .0
        .join("archives")
        .join("sealed")
        .join(format!("{}.archive", descriptor.id));
    assert!(sealed_path.exists());

    // Record an uncompleted deletion in the deletions table
    let now_ms = crate::sandbox::now();
    store.record_deletion_for_test(&descriptor.id, 100, now_ms);

    // Reopening the store should recover and complete unlinking
    let recovered_store = ArchiveStore::open(
        &dir.0.join("archives"),
        &crate::db::Db::for_test_at(dir.0.join("test.sqlite")),
    )
    .unwrap();
    assert!(!sealed_path.exists());
    assert!(recovered_store.list_artifacts().unwrap().is_empty());
}

#[test]
fn retention_coordinator_rejects_malformed_setting() {
    let dir = Directory::new();
    let (db, mut store, coordinator) = setup_env(&dir);

    let archive = sample_archive("malformed");
    store.deposit_and_link(&archive, "review:1").unwrap();

    // Set invalid setting "0" directly in DB
    db.save_setting_value("review_archive_retention_days", "0").unwrap();

    let cancel = AtomicBool::new(false);
    let result = coordinator.perform_maintenance(&cancel);
    assert!(result.is_err());
    assert_eq!(store.list_artifacts().unwrap().len(), 1);

    let status = coordinator.status.lock().unwrap();
    assert!(status.last_error.is_some());
}

fn backfill_test_pr(db: &Database, number: i64) -> (i64, i64) {
    let pr = db
        .upsert_pull_request(&PullRequestInput {
            repo_id: 1,
            number,
            title: "Backfill candidate".into(),
            author: "alice".into(),
            author_avatar_url: None,
            is_draft: false,
            head_sha: "head".into(),
            base_sha: None,
            base_ref: Some("main".into()),
            head_ref: Some("feature".into()),
            default_branch: Some("main".into()),
            url: None,
            lane: "ready".into(),
            status_reason: None,
        })
        .unwrap();
    let run = db.start_review_skeleton_with(pr, Some("model")).unwrap();
    (pr, run)
}

#[test]
fn backfill_cursor_survives_restart_and_revisits_unmerged_candidates() {
    let dir = Directory::new();
    let (db, _store, coordinator) = setup_env(&dir);
    let prs: Vec<_> = (1..=202).map(|number| backfill_test_pr(&db, number)).collect();
    let mut runs: Vec<_> = prs.iter().map(|pr| pr.1).collect();
    // Repeat owners across SQL chunks to exercise global deduplication.
    runs.extend(runs.clone());
    runs.extend(runs.clone());
    let first = coordinator.next_backfill_candidates(&db, &runs).unwrap();
    assert_eq!(first.len(), 200);
    assert_eq!(first.last().unwrap().0, prs[199].0);
    drop(coordinator);
    let (_, _, restarted) = setup_env(&dir);
    let second = restarted.next_backfill_candidates(&db, &runs).unwrap();
    assert_eq!(second.len(), 200);
    assert_eq!(second[0].0, prs[200].0);
    assert_eq!(second[1].0, prs[201].0);
    assert_eq!(second[2].0, prs[0].0);
}

#[test]
fn backfilled_timestamp_leaves_terminal_transition_to_lifecycle_reconciliation() {
    let dir = Directory::new();
    let (db, _store, _coordinator) = setup_env(&dir);
    let (pr, run) = backfill_test_pr(&db, 1);
    db.set_review_run_running(run, "[]").unwrap();
    let merged_at = crate::sandbox::now() - 2 * MS_PER_DAY;
    db.set_pr_merged_at(pr, merged_at).unwrap();
    assert_eq!(
        db.pr_retention_info(pr).unwrap().unwrap(),
        ("open".into(), Some(merged_at))
    );
    assert!(db
        .lifecycle_candidates(1, None, 200)
        .unwrap()
        .iter()
        .any(|row| row.0 == pr));
    assert_eq!(db.review_run_by_id(run).unwrap().unwrap().status, "running");
    db.mark_pr_terminal_with_merged_at(pr, "merged", "Merged on GitHub", Some(merged_at))
        .unwrap();
    assert_eq!(db.review_run_by_id(run).unwrap().unwrap().status, "superseded");
    assert!(db.lifecycle_candidates(1, None, 200).unwrap().is_empty());
}

#[test]
fn cleanup_rechecks_receipts_after_another_connection_links_the_archive() {
    let dir = Directory::new();
    let (_db, mut store, _) = setup_env(&dir);
    let archive = sample_archive("concurrent-link");
    let id = archive.descriptor().id;
    store.deposit_and_link(&archive, "review:1").unwrap();
    let checked = store.receipts_for_artifact(&id).unwrap();
    let root = dir.0.join("archives");
    let db = crate::db::Db::for_test_at(dir.0.join("test.sqlite"));
    std::thread::spawn(move || {
        ArchiveStore::open(&root, &db)
            .unwrap()
            .deposit_and_link(&archive, "review:2")
            .unwrap();
    })
    .join()
    .unwrap();
    assert_eq!(store.delete_artifact(&id, &checked).unwrap(), None);
    assert_eq!(store.receipts_for_artifact(&id).unwrap().len(), 2);
    assert!(dir.0.join("archives/sealed").join(format!("{id}.archive")).exists());
}

#[test]
fn deposit_completes_reserved_deletion_before_republishing() {
    let dir = Directory::new();
    let (_db, mut store, _) = setup_env(&dir);
    let archive = sample_archive("concurrent-deletion");
    let descriptor = archive.descriptor();
    store.deposit_and_link(&archive, "review:1").unwrap();
    let mut writer = ArchiveStore::open(
        &dir.0.join("archives"),
        &crate::db::Db::for_test_at(dir.0.join("test.sqlite")),
    )
    .unwrap();
    store.record_deletion_for_test(&descriptor.id, 100, crate::sandbox::now());
    writer.deposit_and_link(&archive, "review:2").unwrap();
    assert_eq!(store.receipts_for_artifact(&descriptor.id).unwrap(), vec!["review:2"]);
    assert!(store.load(&descriptor).is_ok());
    drop(writer);
    assert!(ArchiveStore::open(
        &dir.0.join("archives"),
        &crate::db::Db::for_test_at(dir.0.join("test.sqlite"))
    )
    .unwrap()
    .load(&descriptor)
    .is_ok());
}

#[test]
fn cleanup_preserves_an_active_upload_even_when_receipts_match() {
    let dir = Directory::new();
    let (_db, mut store, _) = setup_env(&dir);
    let archive = sample_archive("active-upload");
    let descriptor = archive.descriptor();
    assert!(!store
        .begin(
            &hash(b"upload-owner"),
            &descriptor,
            &archive.manifest_bytes().unwrap(),
            crate::sandbox::now() + 90_000
        )
        .unwrap());
    store.deposit_and_link(&archive, "review:1").unwrap();
    let receipts = store.receipts_for_artifact(&descriptor.id).unwrap();
    assert_eq!(store.delete_artifact(&descriptor.id, &receipts).unwrap(), None);
    assert!(store.load(&descriptor).is_ok());
}

#[test]
fn maintenance_failure_preserves_last_successful_metrics_and_reports_error() {
    let dir = Directory::new();
    let (_db, _store, mut coordinator) = setup_env(&dir);
    coordinator.perform_maintenance_with_backfill(|_, _| Ok(())).unwrap();
    let before = coordinator.status.lock().unwrap().clone();
    coordinator.archive_root = dir.0.join("not-a-directory");
    std::fs::write(&coordinator.archive_root, "file").unwrap();
    assert!(coordinator.perform_maintenance_with_backfill(|_, _| Ok(())).is_err());
    let after = coordinator.status.lock().unwrap();
    assert_eq!(after.last_cleanup_at, before.last_cleanup_at);
    assert_eq!(after.used_bytes, before.used_bytes);
    assert!(after.last_error.as_deref().unwrap().contains("maintenance failed"));
}

#[test]
fn a_failure_shows_locally_when_the_database_is_unreachable() {
    let dir = Directory::new();
    let unreachable = RetentionCoordinator::new(
        crate::db::Db::unreachable_for_test(),
        dir.0.join("archives"),
        ArchiveLeaseTracker::new(),
        Arc::new(Mutex::new(ArchiveStorageStatus::default())),
        Arc::new((Mutex::new(false), Condvar::new())),
    );
    unreachable.publish_failure();
    assert!(unreachable
        .status
        .lock()
        .unwrap()
        .last_error
        .as_deref()
        .unwrap()
        .contains("maintenance failed"));
}

#[test]
fn another_instance_failing_keeps_the_recorded_metrics() {
    let dir = Directory::new();
    let (db, _store, coordinator) = setup_env(&dir);
    coordinator.perform_maintenance_with_backfill(|_, _| Ok(())).unwrap();
    let recorded = coordinator.status.lock().unwrap().clone();
    // A freshly started instance, with no metrics of its own, whose pass fails.
    let other_root = dir.0.join("other-instance");
    std::fs::write(&other_root, "file").unwrap();
    let other = RetentionCoordinator::new(
        db.handle(),
        other_root,
        ArchiveLeaseTracker::new(),
        Arc::new(Mutex::new(ArchiveStorageStatus::default())),
        Arc::new((Mutex::new(false), Condvar::new())),
    );
    assert!(other.perform_maintenance_with_backfill(|_, _| Ok(())).is_err());
    let stored: ArchiveStorageStatus =
        serde_json::from_str(&db.setting(STORAGE_STATUS_SETTING).unwrap().unwrap()).unwrap();
    assert_eq!(stored.last_cleanup_at, recorded.last_cleanup_at);
    assert_eq!(stored.used_bytes, recorded.used_bytes);
    assert!(stored.last_error.as_deref().unwrap().contains("maintenance failed"));
}

#[test]
fn successful_local_cleanup_preserves_safe_backfill_warning_until_recovery() {
    let dir = Directory::new();
    let (_db, _store, coordinator) = setup_env(&dir);
    coordinator
        .perform_maintenance_with_backfill(|_, _| Err("sensitive command output".into()))
        .unwrap();
    let status = coordinator.status.lock().unwrap().clone();
    assert!(status.last_cleanup_at.is_some());
    assert!(status.last_error.as_deref().unwrap().contains("Merge timestamps"));
    assert!(!status.last_error.unwrap().contains("sensitive"));
    coordinator.perform_maintenance_with_backfill(|_, _| Ok(())).unwrap();
    assert!(coordinator.status.lock().unwrap().last_error.is_none());
}

#[test]
fn settings_notification_is_consumed_before_the_next_wait() {
    let notify = Arc::new((Mutex::new(false), Condvar::new()));
    let worker_notify = notify.clone();
    let (ready, started) = std::sync::mpsc::channel();
    let worker = std::thread::spawn(move || {
        ready.send(()).unwrap();
        wait_for_maintenance(&worker_notify, Duration::from_secs(5)).unwrap();
        assert!(!*worker_notify.0.lock().unwrap());
        let next = Instant::now();
        wait_for_maintenance(&worker_notify, Duration::from_millis(30)).unwrap();
        assert!(next.elapsed() >= Duration::from_millis(30));
    });
    started.recv().unwrap();
    *notify.0.lock().unwrap() = true;
    notify.1.notify_one();
    worker.join().unwrap();
    // A notification received during maintenance is also consumed once.
    *notify.0.lock().unwrap() = true;
    wait_for_maintenance(&notify, Duration::from_secs(5)).unwrap();
    assert!(!*notify.0.lock().unwrap());
}

#[test]
fn every_instance_reports_the_storage_status_of_the_last_pass() {
    let dir = Directory::new();
    let (db, _store, coordinator) = setup_env(&dir);
    coordinator.perform_maintenance(&AtomicBool::new(false)).unwrap();
    // Another instance reads what this one's pass recorded.
    let stored = db.setting(STORAGE_STATUS_SETTING).unwrap().expect("status recorded");
    let status: ArchiveStorageStatus = serde_json::from_str(&stored).unwrap();
    assert!(status.last_cleanup_at.is_some());
}
