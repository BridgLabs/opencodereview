use std::path::{Path, PathBuf};

use super::super::Db;
use super::*;

struct Fixture {
    root: PathBuf,
    db: Db,
}

impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!("ocr-import-test-{}", crate::sandbox::secret().unwrap()));
        std::fs::create_dir_all(root.join("review-archives")).unwrap();
        Self {
            root,
            db: Db::for_test(),
        }
    }

    fn source(&self) -> PathBuf {
        self.root.join("opencodereview.sqlite")
    }

    fn index(&self) -> PathBuf {
        self.root.join("review-archives/index.sqlite")
    }

    fn import(&self, archive: bool) -> Result<ImportReport, String> {
        let index = self.index();
        import(
            &mut self.db.test_client(),
            &self.source(),
            archive.then_some(index.as_path()),
        )
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

/// An install at SQLite schema 1 with what a file can hold when foreign keys were not enforced
/// (rows of a removed repository, references to deleted runs), plus invalid JSON, a NUL byte and
/// ids handed out and then deleted.
fn write_source(path: &Path) {
    let conn = rusqlite::Connection::open(path).unwrap();
    conn.execute_batch(migrations::MIGRATIONS[0]).unwrap();
    conn.execute_batch(
        "PRAGMA foreign_keys = OFF;
         PRAGMA user_version = 1;
         UPDATE settings SET value = 'dark' WHERE key = 'theme';
         INSERT INTO settings (key, value, updated_at) VALUES ('installation_id', 'installation-from-sqlite', 1);
         INSERT INTO workspaces (id, login, created_at, updated_at) VALUES (1, 'acme', 1, 1);
         INSERT INTO repos (id, workspace_id, owner, name, full_name, created_at, updated_at) VALUES (1, 1, 'acme', 'api', 'acme/api', 1, 1);
         INSERT INTO subscriptions (repo_id, enabled, created_at, updated_at) VALUES (1, 1, 5, 5);
         UPDATE comment_scan_cursors SET enabled_at = 10, checkpoint = 20 WHERE repo_id = 1;
         INSERT INTO pull_requests (id, repo_id, number, title, author, head_sha, last_synced_at, created_at, updated_at)
           VALUES (1, 1, 1, 'Live', 'a', 'head', 1, 1, 1), (2, 99, 7, 'From a removed repository', 'a', 'head', 1, 1, 1);
         INSERT INTO review_runs (id, pr_id, head_sha, status, tail, created_at, updated_at)
           VALUES (1, 1, 'head', 'preview', 'before' || char(0) || 'after', 1, 1), (2, 2, 'head', 'posted', NULL, 1, 1);
         UPDATE sqlite_sequence SET seq = 50 WHERE name = 'review_runs';
         INSERT INTO review_finding_audits (run_id, audit_json, updated_at) VALUES (1, 'not json', 1);
         INSERT INTO review_results (pr_id, installation_id, marker_key, run_id, observed_head_sha, reviewed_head_sha,
             observation, effective_state, publication_status, reviewer_version, skill_version, created_at, updated_at)
           VALUES (1, 'installation-from-sqlite', 'marker', 777, 'head', 'head', 'o', 'clean', 'posted', 'v', 's', 1, 1);
         INSERT INTO pending_actions (type, pr_id, review_run_id, title, message, created_at)
           VALUES ('review_ready', 1, 888, 'Ready', 'For a deleted run', 1);
         INSERT INTO ai_accounts (provider, label, priority, created_at, updated_at) VALUES ('codex', 'ChatGPT', 1, 1, 1);",
    )
    .unwrap();
}

fn write_archive_index(path: &Path) {
    let conn = rusqlite::Connection::open(path).unwrap();
    conn.execute_batch(
        "CREATE TABLE artifacts(id TEXT PRIMARY KEY,descriptor TEXT NOT NULL,bytes INTEGER NOT NULL);
         CREATE TABLE uploads(owner TEXT PRIMARY KEY,id TEXT NOT NULL,descriptor TEXT NOT NULL,manifest BLOB NOT NULL,deadline INTEGER NOT NULL,reservation INTEGER NOT NULL);
         CREATE TABLE receipts(owner TEXT PRIMARY KEY,id TEXT NOT NULL);
         CREATE TABLE deletions(id TEXT PRIMARY KEY,bytes INTEGER NOT NULL,phase TEXT NOT NULL,created_at INTEGER NOT NULL);
         CREATE TABLE archive_retention_events(owner TEXT NOT NULL,id TEXT NOT NULL,expired_at INTEGER NOT NULL,bytes INTEGER NOT NULL,PRIMARY KEY(owner,id));
         INSERT INTO artifacts VALUES ('a', '{}', 4096);
         INSERT INTO receipts VALUES ('review:1', 'a');
         INSERT INTO uploads VALUES ('upload', 'a', '{}', X'00ff', 9, 100);
         INSERT INTO archive_retention_events VALUES ('review:2', 'b', 3, 10);",
    )
    .unwrap();
}

fn scalar<T: for<'a> postgres::types::FromSql<'a>>(db: &Db, sql: &str) -> T {
    db.test_client().query_one(sql, &[]).unwrap().get(0)
}

#[test]
fn an_install_moves_with_its_history_and_identity() {
    let fixture = Fixture::new();
    write_source(&fixture.source());
    write_archive_index(&fixture.index());
    let report = fixture.import(true).unwrap();

    assert_eq!(report.restored_repositories, [99], "{report:?}");
    assert_eq!(report.cleared_references.get("review_results.run_id"), Some(&1));
    assert_eq!(report.skipped_orphans.get("pending_actions"), Some(&1));
    assert_eq!(report.repaired_text, 1);
    assert_eq!(report.tables.get("review_runs"), Some(&2));
    assert!(report.archive_index_imported);

    let db = &fixture.db;
    let setting = |key: &str| -> String {
        db.test_client()
            .query_one("SELECT value FROM settings WHERE key = $1", &[&key])
            .unwrap()
            .get(0)
    };
    assert_eq!(setting("installation_id"), "installation-from-sqlite");
    assert_eq!(setting("theme"), "dark");
    let removed: Option<i64> = scalar(db, "SELECT removed_at FROM repos WHERE id = 99");
    assert!(removed.is_some(), "the removed repository is marked removed");
    assert_eq!(
        scalar::<String>(db, "SELECT audit_json FROM review_finding_audits WHERE run_id = 1"),
        "not json"
    );
    assert_eq!(
        scalar::<String>(db, "SELECT tail FROM review_runs WHERE id = 1"),
        "before\u{FFFD}after"
    );
    // The source's comment cursor, not the one the subscription trigger would have seeded.
    let cursor = db
        .test_client()
        .query_one(
            "SELECT enabled_at, checkpoint FROM comment_scan_cursors WHERE repo_id = 1",
            &[],
        )
        .unwrap();
    assert_eq!((cursor.get::<_, i64>(0), cursor.get::<_, i64>(1)), (10, 20));
    // Ids continue after the last one SQLite handed out, deleted rows included.
    let next_id: i64 = scalar(db, "SELECT nextval(pg_get_serial_sequence('review_runs', 'id'))");
    assert_eq!(next_id, 51);
    let workspace_id: i64 = scalar(db, "SELECT nextval(pg_get_serial_sequence('workspaces', 'id'))");
    assert_eq!(workspace_id, 2);

    assert_eq!(
        scalar::<i64>(db, "SELECT bytes FROM archive.artifacts WHERE id = 'a'"),
        4096
    );
    assert_eq!(
        scalar::<Vec<u8>>(db, "SELECT manifest FROM archive.uploads"),
        [0x00, 0xff]
    );
    assert_eq!(scalar::<i64>(db, "SELECT COUNT(*) FROM archive.retention_events"), 1);
    assert!(!fixture.index().exists() && fixture.root.join("review-archives/index.sqlite.imported").exists());
    assert!(fixture.source().exists(), "the source file is only read");
}

#[test]
fn a_second_import_or_one_into_a_used_database_is_refused() {
    let fixture = Fixture::new();
    write_source(&fixture.source());
    fixture.import(false).unwrap();
    let error = fixture.import(false).unwrap_err();
    assert!(error.contains("already has data"), "{error}");
}

#[test]
fn importing_while_a_server_runs_is_refused() {
    let fixture = Fixture::new();
    write_source(&fixture.source());
    fixture.db.open().unwrap().register_instance("running", "host").unwrap();
    let error = fixture.import(false).unwrap_err();
    assert!(error.contains("Stop every OpenCodeReview server"), "{error}");
}

#[test]
fn a_failed_import_leaves_the_database_and_the_archive_index_as_they_were() {
    let fixture = Fixture::new();
    write_source(&fixture.source());
    write_archive_index(&fixture.index());
    rusqlite::Connection::open(fixture.source())
        .unwrap()
        .execute("UPDATE review_runs SET created_at = 'not a number' WHERE id = 1", [])
        .unwrap();
    let error = fixture.import(true).unwrap_err();
    assert!(error.contains("review_runs"), "{error}");
    assert_eq!(scalar::<i64>(&fixture.db, "SELECT COUNT(*) FROM repos"), 0);
    assert!(
        fixture.index().exists(),
        "the archive index stays until an import succeeds"
    );
}
