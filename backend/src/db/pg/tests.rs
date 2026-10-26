use std::sync::{Arc, Barrier};

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
    assert_eq!(versions, MIGRATIONS.len() as i64);
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
        .prepare("SELECT key FROM settings WHERE key IN ($1, $2) ORDER BY key")
        .unwrap();
    let found: Vec<String> = stmt
        .query_map(params_from_iter(keys.iter()), |row| row.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(found, keys);
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
    let conn = db.pool().get().unwrap();
    assert_eq!(setting(&conn, "theme").as_deref(), Some("system"));
    assert!(conn.unchecked_transaction().is_ok());
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
fn transaction_with_retry_reruns_the_loser_of_a_deadlock() {
    let db = TestDb::new();
    let barrier = Arc::new(Barrier::new(2));
    let run = |first: &'static str, second: &'static str| {
        let pool = db.pool().clone();
        let barrier = barrier.clone();
        std::thread::spawn(move || {
            let conn = pool.get().unwrap();
            let mut attempts = 0;
            conn.transaction_with_retry(|tx| {
                attempts += 1;
                tx.execute(
                    "UPDATE settings SET updated_at = updated_at + 1 WHERE key = $1",
                    [first],
                )?;
                if attempts == 1 {
                    barrier.wait();
                }
                tx.execute(
                    "UPDATE settings SET updated_at = updated_at + 1 WHERE key = $1",
                    [second],
                )?;
                Ok(())
            })
            .unwrap();
            attempts
        })
    };
    let a = run("theme", "font_size");
    let b = run("font_size", "theme");
    let attempts = a.join().unwrap() + b.join().unwrap();
    assert_eq!(attempts, 3, "exactly one side retries");
}
