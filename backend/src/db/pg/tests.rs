use super::test_support::TestDb;
use super::*;

fn setting(conn: &Conn, key: &str) -> Option<String> {
    conn.query_row("SELECT value FROM settings WHERE key = $1", [key], |row| row.get(0))
        .optional()
        .unwrap()
}

#[test]
fn baseline_seeds_settings_and_runs_once() {
    let db = TestDb::new();
    let conn = db.pool().get().unwrap();
    assert_eq!(setting(&conn, "theme").as_deref(), Some("system"));
    migrate(db.settings()).unwrap();
    let versions: i64 = conn
        .query_row("SELECT COUNT(*) FROM schema_migrations", [], |row| row.get(0))
        .unwrap();
    assert_eq!(versions, super::migrate::MIGRATIONS.len() as i64);
}

#[test]
fn refuses_a_schema_newer_than_the_application() {
    let db = TestDb::new();
    db.client()
        .execute("INSERT INTO schema_migrations (version) VALUES (9999)", &[])
        .unwrap();
    let error = migrate(db.settings()).unwrap_err();
    assert!(error.contains("newer than this application"), "{error}");
}

#[test]
fn query_row_without_a_row_is_query_returned_no_rows() {
    let db = TestDb::new();
    let conn = db.pool().get().unwrap();
    let missing = conn.query_row("SELECT value FROM settings WHERE key = 'nope'", [], |row| {
        row.get::<_, String>(0)
    });
    assert!(matches!(missing, Err(Error::QueryReturnedNoRows)));
    assert_eq!(setting(&conn, "nope"), None);
}

#[test]
fn binds_every_parameter_form() {
    let db = TestDb::new();
    let conn = db.pool().get().unwrap();
    let now = 7_i64;
    conn.execute(
        "INSERT INTO settings (key, value, updated_at) VALUES ($1, $2, $3)",
        params!["a", "1", now],
    )
    .unwrap();
    conn.execute("INSERT INTO settings (key, value, updated_at) VALUES ('b', '2', 0)", [])
        .unwrap();
    assert_eq!(conn.changes(), 1);
    let keys = ["a".to_string(), "b".to_string()];
    let mut stmt = conn
        .prepare("SELECT key FROM settings WHERE key = ANY($1) ORDER BY key")
        .unwrap();
    let found: Vec<String> = stmt
        .query_map(params![&keys[..]], |row| row.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(found, keys);
    let mut stmt = conn
        .prepare("SELECT key FROM settings WHERE key IN ($1, $2) ORDER BY key")
        .unwrap();
    let mut rows = stmt.query(&keys).unwrap();
    assert_eq!(rows.next().unwrap().unwrap().get::<_, String>(0).unwrap(), "a");
}

#[test]
fn nested_transactions_are_savepoints_and_unfinished_ones_roll_back() {
    let db = TestDb::new();
    let conn = db.pool().get().unwrap();
    let outer = conn.unchecked_transaction().unwrap();
    outer
        .execute("UPDATE settings SET value = 'dark' WHERE key = 'theme'", [])
        .unwrap();
    {
        let inner = outer.unchecked_transaction().unwrap();
        inner
            .execute("UPDATE settings SET value = '99' WHERE key = 'font_size'", [])
            .unwrap();
        // Dropped without commit: only the savepoint's work is undone.
    }
    {
        let inner = outer.unchecked_transaction().unwrap();
        // A failed statement aborts the transaction in Postgres; the savepoint rollback recovers it.
        assert!(inner
            .execute(
                "INSERT INTO settings (key, value, updated_at) VALUES ('theme', 'x', 0)",
                []
            )
            .is_err());
    }
    assert!(!conn.is_autocommit());
    outer.commit().unwrap();
    assert!(conn.is_autocommit());
    assert_eq!(setting(&conn, "theme").as_deref(), Some("dark"));
    assert_eq!(setting(&conn, "font_size").as_deref(), Some("14"));

    let abandoned = conn.unchecked_transaction().unwrap();
    abandoned
        .execute("UPDATE settings SET value = 'light' WHERE key = 'theme'", [])
        .unwrap();
    drop(abandoned);
    assert_eq!(setting(&conn, "theme").as_deref(), Some("dark"));
}

#[test]
fn commit_fails_after_an_ignored_statement_failure_outside_a_savepoint() {
    let db = TestDb::new();
    let conn = db.pool().get().unwrap();
    let tx = conn.unchecked_transaction().unwrap();
    tx.execute("UPDATE settings SET value = 'dark' WHERE key = 'theme'", [])
        .unwrap();
    // SQLite code could ignore a failed statement and carry on; Postgres has aborted the transaction.
    let _ = tx.execute(
        "INSERT INTO settings (key, value, updated_at) VALUES ('theme', 'x', 0)",
        [],
    );
    assert!(matches!(tx.commit(), Err(Error::TransactionAborted)));
    assert!(conn.is_autocommit());
    assert_eq!(setting(&conn, "theme").as_deref(), Some("system"));

    let tx = conn.unchecked_transaction().unwrap();
    tx.execute("UPDATE settings SET value = 'dark' WHERE key = 'theme'", [])
        .unwrap();
    tx.commit().unwrap();
    assert_eq!(setting(&conn, "theme").as_deref(), Some("dark"));
}

#[test]
fn a_savepoint_failure_released_instead_of_rolled_back_fails_the_outer_commit() {
    let db = TestDb::new();
    let conn = db.pool().get().unwrap();
    let outer = conn.unchecked_transaction().unwrap();
    let inner = outer.unchecked_transaction().unwrap();
    let _ = inner.execute(
        "INSERT INTO settings (key, value, updated_at) VALUES ('theme', 'x', 0)",
        [],
    );
    assert!(inner.commit().is_err());
    assert!(matches!(outer.commit(), Err(Error::TransactionAborted)));
}

#[test]
fn a_connection_dropped_mid_transaction_returns_clean_to_the_pool() {
    let db = TestDb::new();
    {
        let conn = db.pool().get().unwrap();
        let tx = conn.unchecked_transaction().unwrap();
        tx.execute("UPDATE settings SET value = 'leak' WHERE key = 'theme'", [])
            .unwrap();
        std::mem::forget(tx);
    }
    // Whichever pooled session comes back next, none may still sit in that transaction.
    let stuck: i64 = db
        .client()
        .query_one(
            "SELECT COUNT(*) FROM pg_stat_activity WHERE datname = current_database() AND state LIKE 'idle in transaction%'",
            &[],
        )
        .unwrap()
        .get(0);
    assert_eq!(stuck, 0);
    let conn = db.pool().get().unwrap();
    assert_eq!(setting(&conn, "theme").as_deref(), Some("system"));
}

#[test]
fn the_read_pool_rejects_writes() {
    let db = TestDb::new();
    let conn = db.pool().get_read().unwrap();
    assert_eq!(setting(&conn, "theme").as_deref(), Some("system"));
    let error = conn
        .execute("UPDATE settings SET value = 'dark' WHERE key = 'theme'", [])
        .unwrap_err();
    assert_eq!(
        error.code(),
        Some(&postgres::error::SqlState::READ_ONLY_SQL_TRANSACTION)
    );
}

#[test]
fn errors_shown_to_clients_leave_out_row_values() {
    let db = TestDb::new();
    let conn = db.pool().get().unwrap();
    let error = conn
        .execute(
            "INSERT INTO settings (key, value, updated_at) VALUES ('secret-key', 'x', 0), ('secret-key', 'y', 0)",
            [],
        )
        .unwrap_err();
    assert!(error.is_unique_violation());
    let shown = error.to_string();
    assert!(shown.starts_with("database error 23505"), "{shown}");
    assert!(!shown.contains("secret-key"), "{shown}");
}

#[test]
fn columns_read_as_leniently_as_sqlite_did() {
    let db = TestDb::new();
    let conn = db.pool().get().unwrap();
    let (literal, exists, flag, small): (i64, i64, bool, usize) = conn
        .query_row(
            "SELECT CASE WHEN true THEN 1 ELSE 0 END, EXISTS(SELECT 1), 2::BIGINT, 3::INT",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .unwrap();
    assert_eq!((literal, exists, flag, small), (1, 1, true, 3));
    let missing: Option<i64> = conn.query_row("SELECT NULL::INT", [], |row| row.get(0)).unwrap();
    assert_eq!(missing, None);
    assert!(conn
        .query_row("SELECT NULL::INT", [], |row| row.get::<_, i64>(0))
        .is_err());
    let sum = conn
        .query_row("SELECT SUM(1::BIGINT)", [], |row| row.get::<_, i64>(0))
        .unwrap_err();
    assert!(sum.to_string().contains("cast it to bigint"), "{sum}");
    let by_name: String = conn
        .query_row("SELECT 'x' AS label", [], |row| row.get("label"))
        .unwrap();
    assert_eq!(by_name, "x");
}

#[test]
fn the_scratch_pool_sees_only_the_per_review_index() {
    let db = TestDb::new();
    let scratch = db.pool().get_scratch().unwrap();
    for run_id in [7_i64, 8] {
        scratch
            .execute(
                "INSERT INTO repos (id, owner, name, full_name, created_at, updated_at) VALUES ($1, 'o', 'n', 'o/n', 1, 1)",
                [run_id],
            )
            .unwrap();
    }
    let error = scratch
        .query_row("SELECT COUNT(*) FROM pull_requests", [], |row| row.get::<_, i64>(0))
        .unwrap_err();
    assert_eq!(error.code(), Some(&postgres::error::SqlState::UNDEFINED_TABLE));
    let app = db.pool().get().unwrap();
    let public_repos: i64 = app
        .query_row("SELECT COUNT(*) FROM repos", [], |row| row.get(0))
        .unwrap();
    let scratch_repos: i64 = app
        .query_row("SELECT COUNT(*) FROM scratch.repos", [], |row| row.get(0))
        .unwrap();
    assert_eq!((public_repos, scratch_repos), (0, 2));
    let persistence: String = app
        .query_row(
            "SELECT relpersistence::TEXT FROM pg_class WHERE oid = 'scratch.repo_index_files'::regclass",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(persistence, "u", "scratch tables skip the WAL");
}

#[test]
fn json_functions_keep_sqlite_semantics() {
    let db = TestDb::new();
    let conn = db.pool().get().unwrap();
    let length = |json: &str| -> Option<i64> {
        conn.query_row("SELECT json_member_array_length($1, 'items')", [json], |row| row.get(0))
            .unwrap()
    };
    assert_eq!(length(r#"{"items":[1,2,3]}"#), Some(3));
    assert_eq!(
        length(r#"{"items":{"a":1}}"#),
        Some(0),
        "non-arrays count as 0, as in SQLite"
    );
    assert_eq!(length(r#"{"items":null}"#), Some(0));
    assert_eq!(length(r#"{"other":[]}"#), None);
    assert_eq!(length("not json"), None);
    let is_true = |json: &str| -> bool {
        conn.query_row("SELECT json_member_is_true($1, 'complete')", [json], |row| row.get(0))
            .unwrap()
    };
    assert!(is_true(r#"{"complete":true}"#) && is_true(r#"{"complete":1}"#));
    assert!(!is_true(r#"{"complete":false}"#) && !is_true(r#"{"complete":"true"}"#) && !is_true("{"));
    let without = |json: &str| -> String {
        conn.query_row("SELECT json_without_member($1, 'diff')", [json], |row| row.get(0))
            .unwrap()
    };
    assert_eq!(without(r#"{"diff":"x","keep":1}"#), r#"{"keep": 1}"#);
    assert_eq!(without("not json"), "not json");
    let kind: Option<String> = conn
        .query_row("SELECT json_member_type('{\"a\":[]}', 'a')", [], |row| row.get(0))
        .unwrap();
    assert_eq!(kind.as_deref(), Some("array"));
}

#[test]
fn a_test_database_written_outside_the_pools_is_emptied_before_reuse() {
    for _ in 0..5 {
        let db = TestDb::new();
        db.client()
            .execute(
                "INSERT INTO settings (key, value, updated_at) VALUES ('leftover', 'x', 0)",
                &[],
            )
            .unwrap();
        drop(db);
        let next = TestDb::new();
        let leftover = next
            .client()
            .query_opt("SELECT 1 FROM settings WHERE key = 'leftover'", &[])
            .unwrap();
        assert!(leftover.is_none(), "a reused test database kept another test's rows");
    }
}
