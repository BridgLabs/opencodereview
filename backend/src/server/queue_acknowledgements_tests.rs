use super::*;
use crate::server::status_comments::test_support::FakeStatusComments;
use std::sync::atomic::{AtomicU64, Ordering};

static COUNTER: AtomicU64 = AtomicU64::new(0);

fn fixture(numbers: &[i64]) -> (PathBuf, Database, Vec<i64>) {
    let root = std::env::temp_dir().join(format!(
        "ocr-queue-ack-{}-{}",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    let db = crate::db::Db::for_test_at(root.join("test.sqlite")).open().unwrap();
    db.ensure_temp_repo_for_index(1, "acme/fixture").unwrap();
    let pr_ids = numbers
        .iter()
        .map(|number| {
            db.upsert_pull_request(&crate::db::PullRequestInput {
                repo_id: 1,
                number: *number,
                title: "Fixture".into(),
                author: "fixture".into(),
                author_avatar_url: None,
                is_draft: false,
                head_sha: "head-a".into(),
                base_sha: Some("base".into()),
                base_ref: Some("main".into()),
                head_ref: Some("feature".into()),
                default_branch: Some("main".into()),
                url: None,
                lane: "reviewing".into(),
                status_reason: None,
            })
            .unwrap()
        })
        .collect();
    (root, db, pr_ids)
}

fn ack_row(root: &Path, pr_id: i64) -> (i64, String, Option<i64>, i64, i64) {
    crate::db::Db::for_test_at(root.join("test.sqlite")).open()
        .unwrap()
        .query_row(
            "SELECT generation, status, comment_id, attempts, next_attempt_at FROM queue_acknowledgements WHERE pr_id = $1",
            [pr_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?)),
        )
        .expect("ack row")
}

fn legacy_confirmation(full_name: &str, number: i64, installation: &str) -> String {
    format!(
        "{}\n\n{}",
        activation_marker(full_name, number, installation),
        queued_for_review_text(full_name, "head-a")
    )
}

#[test]
fn pending_acknowledgement_posts_fresh_and_cleans_only_its_family() {
    let (root, db, pr_ids) = fixture(&[7]);
    let installation = db.installation_id().unwrap();
    db.acknowledge_review_cycle(pr_ids[0], false, None).unwrap();
    db.acknowledge_review_cycle(pr_ids[0], true, None).unwrap();
    let mut api = FakeStatusComments::new();
    api.own(&legacy_confirmation("acme/fixture", 7, &installation));
    api.own(&queue_acknowledgement_body(
        "acme/fixture",
        7,
        &installation,
        "head-a",
        1,
    ));
    let clean = api.own(&format!(
        "{}\n{}\n\nresult",
        crate::db::result_marker_key("acme/fixture", 7, &installation),
        "<!-- ocr-result-data:v1 head=head-a state=clean -->"
    ));
    let foreign = api.own(&legacy_confirmation("acme/fixture", 7, "other-installation"));
    let other_author = api.add(
        999,
        &queue_acknowledgement_body("acme/fixture", 7, &installation, "head-a", 2),
    );
    let human = api.own("Looks good, ship it");

    reconcile_queue_acknowledgements_with(&db, &mut api).unwrap();

    assert_eq!(api.posts, 1);
    assert_eq!(api.deletes, 2);
    assert_eq!(api.ids(), vec![clean, foreign, other_author, human, 7]);
    let posted = api.comments.last().unwrap();
    assert!(posted
        .body
        .starts_with(&activation_marker("acme/fixture", 7, &installation)));
    assert!(posted.body.contains("<!-- ocr-queue:v1 generation=2 -->"));
    assert!(posted
        .body
        .contains("### ⏳ OpenCodeReview is reviewing this pull request"));
    assert_eq!(ack_row(&root, pr_ids[0]), (2, "posted".to_string(), Some(7), 0, 0));
}

#[test]
fn lost_post_response_is_adopted_on_retry() {
    let (root, db, pr_ids) = fixture(&[7]);
    db.acknowledge_review_cycle(pr_ids[0], false, None).unwrap();
    let mut api = FakeStatusComments::new();
    api.post_writes_then_fails = true;
    assert!(reconcile_queue_acknowledgements_with(&db, &mut api).is_err());
    assert_eq!(ack_row(&root, pr_ids[0]).1, "pending");
    crate::db::Db::for_test_at(root.join("test.sqlite"))
        .open()
        .unwrap()
        .execute("UPDATE queue_acknowledgements SET next_attempt_at = 0", [])
        .unwrap();
    api.post_writes_then_fails = false;

    reconcile_queue_acknowledgements_with(&db, &mut api).unwrap();

    assert_eq!(api.posts, 1);
    assert_eq!(api.comments.len(), 1);
    assert_eq!(ack_row(&root, pr_ids[0]), (1, "posted".to_string(), Some(1), 1, 0));
}

#[test]
fn retiring_deletes_owned_comments_without_posting() {
    let (root, db, pr_ids) = fixture(&[7]);
    let installation = db.installation_id().unwrap();
    db.acknowledge_review_cycle(pr_ids[0], false, None).unwrap();
    db.retire_queue_acknowledgement(pr_ids[0], "head-a", None).unwrap();
    let mut api = FakeStatusComments::new();
    api.own(&legacy_confirmation("acme/fixture", 7, &installation));
    api.own(&queue_acknowledgement_body(
        "acme/fixture",
        7,
        &installation,
        "head-a",
        1,
    ));
    let newer = api.own(&queue_acknowledgement_body(
        "acme/fixture",
        7,
        &installation,
        "head-a",
        5,
    ));

    reconcile_queue_acknowledgements_with(&db, &mut api).unwrap();

    assert_eq!(api.posts, 0);
    assert_eq!(api.deletes, 2);
    assert_eq!(api.ids(), vec![newer]);
    assert_eq!(ack_row(&root, pr_ids[0]), (1, "retired".to_string(), None, 0, 0));
}

#[test]
fn uncertain_post_keeps_retirement_open_until_it_settles() {
    let (root, db, pr_ids) = fixture(&[7]);
    let installation = db.installation_id().unwrap();
    db.acknowledge_review_cycle(pr_ids[0], false, None).unwrap();
    let mut api = FakeStatusComments::new();
    api.fail_post = true;

    // The POST fails without adding the comment; the row keeps retrying and
    // records the attempt as uncertain.
    assert!(reconcile_queue_acknowledgements_with(&db, &mut api).is_err());
    assert_eq!(api.posts, 1);
    assert_eq!(ack_row(&root, pr_ids[0]).1, "pending");
    let uncertain: Option<i64> = crate::db::Db::for_test_at(root.join("test.sqlite"))
        .open()
        .unwrap()
        .query_row(
            "SELECT post_uncertain_at FROM queue_acknowledgements WHERE pr_id = $1",
            [pr_ids[0]],
            |row| row.get(0),
        )
        .unwrap();
    assert!(uncertain.is_some());

    // Retiring before the post settles must not trust the first empty scan.
    db.retire_queue_acknowledgement(pr_ids[0], "head-a", None).unwrap();
    api.fail_post = false;
    reconcile_queue_acknowledgements_with(&db, &mut api).unwrap();
    let row = ack_row(&root, pr_ids[0]);
    assert_eq!(row.1, "retiring");
    assert!(row.4 > crate::db::now_millis());
    assert_eq!(api.deletes, 0);

    // The accepted comment becomes visible later; once the settle window has
    // passed the scan deletes it and the row retires without reposting.
    api.own(&queue_acknowledgement_body(
        "acme/fixture",
        7,
        &installation,
        "head-a",
        1,
    ));
    crate::db::Db::for_test_at(root.join("test.sqlite"))
        .open()
        .unwrap()
        .execute(
            "UPDATE queue_acknowledgements SET next_attempt_at = 0, post_uncertain_at = $1 WHERE pr_id = $2",
            [crate::db::now_millis() - crate::db::QUEUE_POST_SETTLE_MS - 1, pr_ids[0]],
        )
        .unwrap();
    reconcile_queue_acknowledgements_with(&db, &mut api).unwrap();
    assert_eq!(api.deletes, 1);
    assert_eq!(api.posts, 1);
    assert_eq!(ack_row(&root, pr_ids[0]).1, "retired");
}

#[test]
fn one_prs_failure_is_recorded_and_does_not_stop_the_others() {
    let (root, db, pr_ids) = fixture(&[7, 8]);
    for pr_id in &pr_ids {
        db.acknowledge_review_cycle(*pr_id, false, None).unwrap();
    }
    let mut api = FakeStatusComments::new();
    api.fail_list_for = Some(8);

    let error = reconcile_queue_acknowledgements_with(&db, &mut api).unwrap_err();

    assert!(error.contains("acme/fixture #8"));
    assert_eq!(api.posts, 1);
    assert_eq!(ack_row(&root, pr_ids[0]).1, "posted");
    let failing = ack_row(&root, pr_ids[1]);
    assert_eq!(failing.1, "pending");
    assert_eq!(failing.3, 1);
    assert!(failing.4 > crate::db::now_millis());
}

#[test]
fn queue_comment_parser_covers_generations_legacy_and_foreign() {
    let ownership = activation_marker("acme/fixture", 7, "inst-a");
    assert_eq!(
        owned_queue_comment_generation(
            &queue_acknowledgement_body("acme/fixture", 7, "inst-a", "head-a", 3),
            &ownership
        ),
        Some(Some(3))
    );
    assert_eq!(
        owned_queue_comment_generation(&format!("{ownership}\n\nvisible"), &ownership),
        Some(None)
    );
    assert_eq!(
        owned_queue_comment_generation(
            &format!("{ownership}\n<!-- ocr-queue:v1 generation=oops -->\n\nvisible"),
            &ownership
        ),
        None
    );
    assert_eq!(
        owned_queue_comment_generation(&format!("{ownership} extra\n\nvisible"), &ownership),
        None
    );
    assert_eq!(owned_queue_comment_generation("unrelated", &ownership), None);
}

static COALESCED: CoalescedTask = CoalescedTask::new();

#[test]
fn coalesced_task_reruns_once_and_never_overlaps_same_key() {
    let key = format!(
        "ocr-coalesced-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    );
    let (block_tx, block_rx) = std::sync::mpsc::channel::<()>();
    let (started_tx, started_rx) = std::sync::mpsc::channel::<()>();
    let passes = Arc::new(AtomicUsize::new(0));
    let in_flight = Arc::new(AtomicUsize::new(0));
    let max_flight = Arc::new(AtomicUsize::new(0));
    {
        let (passes, in_flight, max_flight) = (passes.clone(), in_flight.clone(), max_flight.clone());
        COALESCED.spawn(key.clone(), move || {
            passes.fetch_add(1, Ordering::Relaxed);
            let now = in_flight.fetch_add(1, Ordering::Relaxed) + 1;
            max_flight.fetch_max(now, Ordering::Relaxed);
            started_tx.send(()).ok();
            block_rx.recv_timeout(Duration::from_secs(5)).ok();
            in_flight.fetch_sub(1, Ordering::Relaxed);
        });
    }
    started_rx.recv_timeout(Duration::from_secs(5)).unwrap();
    for _ in 0..3 {
        COALESCED.spawn(key.clone(), || panic!("extra pass must not run"));
    }
    // A different key runs concurrently.
    let (other_tx, other_rx) = std::sync::mpsc::channel::<()>();
    COALESCED.spawn(format!("{key}-other"), move || {
        other_tx.send(()).ok();
    });
    other_rx.recv_timeout(Duration::from_secs(5)).unwrap();
    drop(block_tx);
    for _ in 0..200 {
        if passes.load(Ordering::Relaxed) == 2 {
            break;
        }
        thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(passes.load(Ordering::Relaxed), 2);
    assert_eq!(max_flight.load(Ordering::Relaxed), 1);
    // Once the pass ends the key is free and a later request runs.
    let (done_tx, done_rx) = std::sync::mpsc::channel::<()>();
    let done_tx = Arc::new(Mutex::new(done_tx));
    let mut freed = false;
    for _ in 0..200 {
        let done_tx = done_tx.clone();
        COALESCED.spawn(key.clone(), move || {
            done_tx.lock().unwrap().send(()).ok();
        });
        if done_rx.recv_timeout(Duration::from_millis(25)).is_ok() {
            freed = true;
            break;
        }
    }
    assert!(freed);
}

#[test]
fn coalesced_task_frees_the_key_after_a_panic() {
    let key = format!(
        "ocr-coalesced-panic-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    );
    COALESCED.spawn(key.clone(), || panic!("boom"));
    let (done_tx, done_rx) = std::sync::mpsc::channel::<()>();
    let done_tx = Arc::new(Mutex::new(done_tx));
    let mut freed = false;
    for _ in 0..200 {
        let done_tx = done_tx.clone();
        COALESCED.spawn(key.clone(), move || {
            done_tx.lock().unwrap().send(()).ok();
        });
        if done_rx.recv_timeout(Duration::from_millis(25)).is_ok() {
            freed = true;
            break;
        }
    }
    assert!(freed);
}
