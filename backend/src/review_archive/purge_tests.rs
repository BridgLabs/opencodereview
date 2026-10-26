use super::*;
use crate::db::{Database, Db};
use crate::review_archive::{Archive, ArchiveStore, Scope, SourceEntry};
use rusqlite::{params, Connection};
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;

const NOW: i64 = 10 * RETENTION_MS;

struct Fixture {
    root: PathBuf,
    handle: Db,
    db: Database,
}

impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!("ocr-retention-{}", crate::sandbox::secret().unwrap()));
        std::fs::create_dir(&root).unwrap();
        let handle = Db::for_test();
        let db = handle.open().unwrap();
        db.execute(
            "INSERT INTO repos (id, owner, name, full_name, created_at, updated_at) VALUES (1, 'owner', 'repo', 'owner/repo', 0, 0)",
            [],
        )
        .unwrap();
        Self { root, handle, db }
    }

    fn pr(&self, id: i64, state: &str) {
        self.db
            .execute(
                "INSERT INTO pull_requests (id, repo_id, number, title, author, head_sha, state, review_lane, last_synced_at, created_at, updated_at)
                 VALUES ($1, 1, $1, 'PR', 'author', 'head', $2, 'merged', 0, 0, 0)",
                crate::pg_params![id, state],
            )
            .unwrap();
    }

    fn review_run(&self, id: i64, pr_id: i64, status: &str, finished_at: Option<i64>) {
        self.db
            .execute(
                "INSERT INTO review_runs (id, pr_id, head_sha, status, finished_at, review_body, created_at, updated_at)
                 VALUES ($1, $2, 'head', $3, $4, 'retained result', 0, 0)",
                crate::pg_params![id, pr_id, status, finished_at],
            )
            .unwrap();
    }

    fn archive(&self, id: i64) -> Archive {
        let scope = Scope {
            host: "github.com".into(),
            repository: "owner/repo".into(),
            head_sha: "a".repeat(40),
            base_sha: "b".repeat(40),
            merge_base_sha: "c".repeat(40),
            prior_sha: None,
        };
        let content = format!("source {id}\n");
        let source = SourceEntry {
            path: "file.rs".into(),
            revision: scope.head_sha.clone(),
            mode: "100644".into(),
            git_blob_id: "d".repeat(40),
            bytes: content.len() as u64,
        };
        Archive::build(scope, vec![source], &AtomicBool::new(false), |_| {
            Ok(content.as_bytes().to_vec())
        })
        .unwrap()
    }

    fn add(&self, id: i64) -> Archive {
        self.pr(id, "merged");
        self.review_run(id, id, "posted", Some(NOW - 2 * RETENTION_MS));
        self.db
            .execute(
                "INSERT INTO review_coverage (repo_id, pr_id, run_id, status, base_ref, base_sha, merge_base_sha, head_sha,
                   changed_files_json, covered_patch_ids_json, diff_hash, diff_text, created_at)
                 VALUES (1, $1, $1, 'valid', 'main', 'b', 'c', 'head', '[]', '[]', 'h', '', 0)",
                [id],
            )
            .unwrap();
        self.db
            .execute(
                "INSERT INTO review_finding_audits (run_id, audit_json, updated_at) VALUES ($1, 'retained audit', 0)",
                [id],
            )
            .unwrap();
        let archive = self.archive(id);
        let mut store = ArchiveStore::open(&self.root.join("review-archives")).unwrap();
        store.deposit(&archive).unwrap();
        store.link(&format!("review:{id}"), &archive.descriptor()).unwrap();
        archive
    }

    fn path(&self, archive: &Archive) -> PathBuf {
        self.root
            .join("review-archives/sealed")
            .join(format!("{}.archive", archive.descriptor().id))
    }

    fn index(&self) -> Connection {
        Connection::open(self.root.join("review-archives/index.sqlite")).unwrap()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

#[test]
fn archive_retention_preview_is_read_only_and_apply_preserves_review_history() {
    let fixture = Fixture::new();
    let archive = fixture.add(1);
    let before = std::fs::read(fixture.path(&archive)).unwrap();
    let preview = run(&fixture.handle, &fixture.root.join("review-archives"), false, NOW).unwrap();
    assert_eq!(preview.candidates.len(), 1);
    assert!(preview.candidate_bytes > 0);
    assert!(!preview.applied);
    assert_eq!(std::fs::read(fixture.path(&archive)).unwrap(), before);
    assert_eq!(
        fixture
            .index()
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE name LIKE 'archive_retention_%'",
                [],
                |row| row.get::<_, i64>(0)
            )
            .unwrap(),
        0
    );
    let applied = run(&fixture.handle, &fixture.root.join("review-archives"), true, NOW).unwrap();
    assert_eq!(applied.deleted_archives, 1);
    assert_eq!(applied.deleted_bytes, preview.candidate_bytes);
    assert_eq!(applied.pending_deletions, 0);
    assert!(!fixture.path(&archive).exists());
    for table in ["review_runs", "review_coverage", "review_finding_audits"] {
        assert_eq!(
            fixture
                .db
                .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| row.get::<_, i64>(0))
                .unwrap(),
            1
        );
    }
    assert_eq!(
        fixture
            .db
            .query_row("SELECT review_body FROM review_runs", [], |row| row.get::<_, String>(0))
            .unwrap(),
        "retained result"
    );
    assert_eq!(
        fixture
            .index()
            .query_row("SELECT COUNT(*) FROM receipts", [], |row| row.get::<_, i64>(0))
            .unwrap(),
        0
    );
    assert_eq!(
        fixture
            .index()
            .query_row("SELECT COUNT(*) FROM archive_retention_events", [], |row| row
                .get::<_, i64>(0))
            .unwrap(),
        1
    );
    assert_eq!(
        run(&fixture.handle, &fixture.root.join("review-archives"), true, NOW)
            .unwrap()
            .deleted_archives,
        0
    );
}

#[test]
fn archive_retention_protects_open_closed_recent_pending_and_unknown_reviews() {
    for sql in [
        "UPDATE pull_requests SET state='open'",
        "UPDATE pull_requests SET state='closed'",
        "UPDATE pull_requests SET reprocess_next_review=1",
        "UPDATE pull_requests SET force_full_next_review=1",
        "UPDATE pull_requests SET review_lane='queued'",
        "UPDATE pull_requests SET review_lane='reviewing'",
        "UPDATE pull_requests SET review_lane='ready'",
        "UPDATE review_runs SET status='preview'",
        "UPDATE review_runs SET status='unknown'",
        "UPDATE review_runs SET finished_at=NULL",
        "UPDATE review_runs SET finished_at=0",
        "UPDATE review_runs SET finished_at=777600000",
        "INSERT INTO review_results (pr_id, installation_id, marker_key, observed_head_sha, reviewed_head_sha, observation, effective_state, publication_status, reviewer_version, skill_version, created_at, updated_at) VALUES (1, 'i', 'm', 'head', 'head', 'o', 'awaiting_action', 'disabled', 'v', 's', 0, 0)",
        "INSERT INTO review_results (pr_id, installation_id, marker_key, observed_head_sha, reviewed_head_sha, observation, effective_state, publication_status, reviewer_version, skill_version, created_at, updated_at) VALUES (1, 'i', 'm', 'head', 'head', 'o', 'clean', 'pending', 'v', 's', 0, 0)",
        "INSERT INTO review_results (pr_id, installation_id, marker_key, observed_head_sha, reviewed_head_sha, observation, effective_state, publication_status, reviewer_version, skill_version, created_at, updated_at) VALUES (1, 'i', 'm', 'head', 'head', 'o', 'clean', 'posting', 'v', 's', 0, 0)",
    ] {
        let fixture = Fixture::new();
        let archive = fixture.add(1);
        fixture.db.execute_batch(sql).unwrap();
        assert!(
            run(&fixture.handle, &fixture.root.join("review-archives"), true, NOW)
                .unwrap()
                .candidates
                .is_empty(),
            "{sql}"
        );
        assert!(fixture.path(&archive).exists(), "{sql}");
    }
}

#[test]
fn archive_retention_rechecks_live_state_after_preview() {
    let fixture = Fixture::new();
    let archive = fixture.add(1);
    assert_eq!(
        run(&fixture.handle, &fixture.root.join("review-archives"), false, NOW)
            .unwrap()
            .candidates
            .len(),
        1
    );
    fixture.db.execute("UPDATE pull_requests SET state='open'", []).unwrap();
    assert_eq!(
        run(&fixture.handle, &fixture.root.join("review-archives"), true, NOW)
            .unwrap()
            .deleted_archives,
        0
    );
    assert!(fixture.path(&archive).exists());
}

#[test]
fn archive_retention_waits_for_any_active_execution() {
    for status in ["pending", "running", "posting", "queued", "cancelling"] {
        let fixture = Fixture::new();
        let archive = fixture.add(1);
        fixture.pr(2, "open");
        fixture.review_run(2, 2, status, None);
        let report = run(&fixture.handle, &fixture.root.join("review-archives"), true, NOW).unwrap();
        assert_eq!(report.active_jobs, 1);
        assert!(report.candidates.is_empty());
        assert!(fixture.path(&archive).exists());
    }
}

#[test]
fn archive_retention_requires_every_shared_reference_to_be_eligible() {
    let fixture = Fixture::new();
    let archive = fixture.add(1);
    fixture.pr(2, "open");
    fixture.review_run(2, 2, "posted", Some(NOW - 2 * RETENTION_MS));
    let store = ArchiveStore::open(&fixture.root.join("review-archives")).unwrap();
    store.link("review:2", &archive.descriptor()).unwrap();
    assert!(run(&fixture.handle, &fixture.root.join("review-archives"), true, NOW)
        .unwrap()
        .candidates
        .is_empty());
    fixture
        .db
        .execute("UPDATE pull_requests SET state='merged' WHERE id=2", [])
        .unwrap();
    assert_eq!(
        run(&fixture.handle, &fixture.root.join("review-archives"), true, NOW)
            .unwrap()
            .deleted_archives,
        1
    );
    assert_eq!(
        fixture
            .index()
            .query_row("SELECT COUNT(*) FROM archive_retention_events", [], |row| row
                .get::<_, i64>(0))
            .unwrap(),
        2
    );
}

#[test]
fn archive_retention_protects_unknown_owners_and_uploads() {
    for owner in ["external:case", "review:999", "review:01"] {
        let fixture = Fixture::new();
        let archive = fixture.add(1);
        let store = ArchiveStore::open(&fixture.root.join("review-archives")).unwrap();
        store.link(owner, &archive.descriptor()).unwrap();
        assert!(run(&fixture.handle, &fixture.root.join("review-archives"), true, NOW)
            .unwrap()
            .candidates
            .is_empty());
    }
    let fixture = Fixture::new();
    let archive = fixture.add(1);
    fixture
        .index()
        .execute(
            "INSERT INTO uploads VALUES('upload',?1,'{}',X'00',?2,100)",
            params![archive.descriptor().id, NOW + RETENTION_MS],
        )
        .unwrap();
    assert!(run(&fixture.handle, &fixture.root.join("review-archives"), true, NOW)
        .unwrap()
        .candidates
        .is_empty());
}

#[test]
fn archive_retention_does_not_create_a_missing_store() {
    let fixture = Fixture::new();
    assert!(run(&fixture.handle, &fixture.root.join("review-archives"), false, NOW)
        .unwrap()
        .candidates
        .is_empty());
    assert!(run(&fixture.handle, &fixture.root.join("review-archives"), true, NOW)
        .unwrap()
        .candidates
        .is_empty());
    assert!(!fixture.root.join("review-archives").exists());
}

#[test]
fn archive_retention_recovers_interrupted_deletion_and_protects_reused_archives() {
    for (detached, missing) in [(true, false), (true, true), (false, false)] {
        let fixture = Fixture::new();
        let archive = fixture.add(1);
        let index = fixture.index();
        index
            .execute_batch("CREATE TABLE archive_retention_queue(id TEXT PRIMARY KEY,bytes INTEGER NOT NULL)")
            .unwrap();
        index
            .execute("INSERT INTO archive_retention_queue SELECT id,bytes FROM artifacts", [])
            .unwrap();
        if detached {
            index
                .execute_batch("DELETE FROM receipts; DELETE FROM artifacts;")
                .unwrap();
        } else {
            fixture.db.execute("UPDATE pull_requests SET state='open'", []).unwrap();
        }
        if missing {
            std::fs::remove_file(fixture.path(&archive)).unwrap();
        }
        let report = run(&fixture.handle, &fixture.root.join("review-archives"), true, NOW).unwrap();
        assert_eq!(report.deleted_archives, usize::from(detached));
        assert_eq!(report.pending_deletions, 0);
        assert_eq!(fixture.path(&archive).exists(), !detached);
    }
}

#[cfg(unix)]
#[test]
fn archive_retention_refuses_symlinked_artifacts_without_detaching_references() {
    let fixture = Fixture::new();
    let archive = fixture.add(1);
    let target = fixture.root.join("protected");
    std::fs::write(&target, "untouched").unwrap();
    std::fs::remove_file(fixture.path(&archive)).unwrap();
    std::os::unix::fs::symlink(&target, fixture.path(&archive)).unwrap();
    assert!(run(&fixture.handle, &fixture.root.join("review-archives"), true, NOW).is_err());
    assert_eq!(std::fs::read_to_string(target).unwrap(), "untouched");
    assert_eq!(
        fixture
            .index()
            .query_row("SELECT COUNT(*) FROM receipts", [], |row| row.get::<_, i64>(0))
            .unwrap(),
        1
    );
}

#[test]
fn archive_retention_limits_each_pass_and_continues_on_the_next() {
    let fixture = Fixture::new();
    for id in 1..=BATCH_LIMIT + 1 {
        fixture.add(id as i64);
    }
    assert_eq!(
        run(&fixture.handle, &fixture.root.join("review-archives"), false, NOW)
            .unwrap()
            .candidates
            .len(),
        BATCH_LIMIT
    );
    assert_eq!(
        run(&fixture.handle, &fixture.root.join("review-archives"), true, NOW)
            .unwrap()
            .deleted_archives,
        BATCH_LIMIT
    );
    assert_eq!(
        run(&fixture.handle, &fixture.root.join("review-archives"), true, NOW)
            .unwrap()
            .deleted_archives,
        1
    );
    assert_eq!(
        fixture
            .db
            .query_row("SELECT COUNT(*) FROM review_runs", [], |row| row.get::<_, usize>(0))
            .unwrap(),
        BATCH_LIMIT + 1
    );
}

#[test]
fn archive_retention_waits_for_a_review_that_is_starting() {
    let fixture = Fixture::new();
    let archive = fixture.add(1);
    // A review start holds the gate shared; retention must wait for it rather than decide early.
    let starting = fixture.handle.open().unwrap();
    let tx = starting.unchecked_transaction().unwrap();
    tx.execute("SELECT pg_advisory_xact_lock_shared($1)", [crate::db::ARCHIVE_GATE_KEY])
        .unwrap();
    let handle = fixture.handle.clone();
    let root = fixture.root.join("review-archives");
    let retention = std::thread::spawn(move || run(&handle, &root, true, NOW).unwrap());
    std::thread::sleep(std::time::Duration::from_millis(300));
    assert!(!retention.is_finished(), "retention ran while a review was starting");
    // The review started: its PR is reviewing again, so the archive must stay.
    tx.execute("UPDATE pull_requests SET review_lane = 'reviewing' WHERE id = 1", [])
        .unwrap();
    tx.commit().unwrap();
    let report = retention.join().unwrap();
    assert!(report.candidates.is_empty());
    assert!(fixture.path(&archive).exists());
}
