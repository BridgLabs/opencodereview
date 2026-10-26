use super::*;

/// The database the server works against: the Postgres pools. Cheap to clone and safe to share
/// between threads; open a `Database` from it for each unit of work and drop it before slow work
/// (git, GitHub, models), which would otherwise hold a pooled connection.
#[derive(Clone)]
pub struct Db {
    pool: pg::DbPool,
    /// Keeps a test's throwaway database alive while any handle to it exists.
    #[cfg(test)]
    _test: Option<std::sync::Arc<pg::test_support::TestDb>>,
    /// A test database gets its setting defaults on first use, so a handle a test builds but never
    /// opens costs no database work.
    #[cfg(test)]
    defaults: std::sync::Arc<std::sync::Once>,
}

impl std::fmt::Debug for Db {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("Db").field(&self.id()).finish()
    }
}

impl Db {
    /// Migrates the schema, connects the pools and fills setting defaults. Runs once at startup,
    /// before any thread starts (it removes the database URL from the environment).
    pub fn from_env() -> Result<Self, String> {
        Self::connect(pg::DbSettings::from_env()?)
    }

    pub fn connect(settings: pg::DbSettings) -> Result<Self, String> {
        pg::migrate(&settings)?;
        let db = Self {
            pool: pg::DbPool::connect(settings)?,
            #[cfg(test)]
            _test: None,
            #[cfg(test)]
            defaults: done(),
        };
        db.open()?.ensure_defaults()?;
        Ok(db)
    }

    pub fn open(&self) -> Result<Database, String> {
        let conn = self.pool.get().map_err(|error| error.to_string())?;
        let db = Database { conn, db: self.clone() };
        #[cfg(test)]
        {
            let mut defaults = Ok(());
            self.defaults.call_once(|| defaults = db.ensure_defaults());
            defaults?;
        }
        Ok(db)
    }

    /// A connection that cannot write.
    pub fn open_read(&self) -> Result<Database, String> {
        let conn = self.pool.get_read().map_err(|error| error.to_string())?;
        Ok(Database { conn, db: self.clone() })
    }

    /// A connection to the per-review index of a pull request's head commit (`scratch`). Use the
    /// review's run id as the repository id: it keeps concurrent reviews apart.
    pub fn open_scratch(&self) -> Result<Database, String> {
        let conn = self.pool.get_scratch().map_err(|error| error.to_string())?;
        Ok(Database { conn, db: self.clone() })
    }

    /// Holds the archive gate exclusively until dropped: archive retention takes it for its whole
    /// run, reviews take it shared as they start, so retention never deletes an archive while a
    /// review that could read it is starting.
    pub fn lock_archive_gate(&self) -> Result<ArchiveGate, String> {
        let db = self.open()?;
        db.conn
            .execute("SELECT pg_advisory_lock($1)", [ARCHIVE_GATE_KEY])
            .map_err(|error| error.to_string())?;
        Ok(ArchiveGate(db))
    }

    /// Names this database (host and database, never credentials), for coalescing background work
    /// per database.
    pub fn id(&self) -> String {
        let config = &self.pool.settings().app;
        let hosts = config
            .get_hosts()
            .iter()
            .map(|host| match host {
                postgres::config::Host::Tcp(name) => name.clone(),
                #[cfg(unix)]
                postgres::config::Host::Unix(path) => path.to_string_lossy().into_owned(),
            })
            .collect::<Vec<_>>()
            .join(",");
        format!("{hosts}/{}", config.get_dbname().unwrap_or_default())
    }

    /// A migrated database of its own, dropped with the last handle to it.
    #[cfg(test)]
    pub fn for_test() -> Self {
        let test = pg::test_support::TestDb::new();
        Self {
            pool: test.pool().clone(),
            _test: Some(std::sync::Arc::new(test)),
            defaults: std::sync::Arc::new(std::sync::Once::new()),
        }
    }

    /// The test database registered under `key`, created on first use. Tests that open "the same
    /// database" from several places (a fixture, a server config, a raw connection) pass the same
    /// key, as they passed the same SQLite path before.
    #[cfg(test)]
    pub fn for_test_at(key: impl AsRef<std::path::Path>) -> Self {
        use std::collections::HashMap;
        use std::path::PathBuf;
        use std::sync::{Arc, Mutex, OnceLock};
        // The registry keeps each database until the process ends (a test may drop every handle
        // and reopen by key); the next test run drops it. Pools are per handle, so a test's
        // connections close when it is done with them.
        static REGISTRY: OnceLock<Mutex<HashMap<PathBuf, Arc<pg::test_support::TestDb>>>> = OnceLock::new();
        let key = key.as_ref().to_path_buf();
        // Opening a SQLite path created its folder, and tests write fixtures next to it.
        if let Some(parent) = key.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let (test, created) = {
            let mut registry = REGISTRY
                .get_or_init(Default::default)
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            match registry.get(&key) {
                Some(test) => (test.clone(), false),
                None => {
                    let test = Arc::new(pg::test_support::TestDb::new());
                    registry.insert(key, test.clone());
                    (test, true)
                }
            }
        };
        Self {
            pool: pg::DbPool::connect(test.settings().clone()).expect("test pool"),
            _test: Some(test),
            defaults: if created {
                std::sync::Arc::new(std::sync::Once::new())
            } else {
                done()
            },
        }
    }

    /// The server's own database, from its environment, for maintainer diagnostics run inside the
    /// server container (`docker/check-review-context.py`). It neither migrates nor writes
    /// defaults; open it with `open_read`.
    #[cfg(test)]
    pub fn diagnostic_from_env() -> Self {
        Self {
            pool: pg::DbPool::connect(pg::DbSettings::from_env().expect("database settings")).expect("connect"),
            _test: None,
            defaults: done(),
        }
    }

    /// A plain client on this database, for test fixtures and assertions written in SQL.
    #[cfg(test)]
    pub fn test_client(&self) -> postgres::Client {
        // Through the test database, so it knows it was written to and is emptied before reuse.
        match &self._test {
            Some(test) => test.client(),
            None => self.pool.settings().connect_owner().expect("connect to test database"),
        }
    }
}

/// Advisory lock key shared by every instance for the archive gate.
pub(crate) const ARCHIVE_GATE_KEY: i64 = 0x6f63_725f_6172_6368; // "ocr_arch"

pub struct ArchiveGate(Database);

impl Drop for ArchiveGate {
    fn drop(&mut self) {
        let _ = self.0.conn.execute("SELECT pg_advisory_unlock($1)", [ARCHIVE_GATE_KEY]);
    }
}

/// What archive retention reads from the operational tables, taken in one consistent snapshot.
pub struct OperationalSnapshot {
    pub review_runs: Vec<(i64, i64, String, Option<i64>)>,
    pub pull_requests: Vec<(i64, String, String, i64, i64)>,
    pub review_results: Vec<(i64, String, String)>,
}

impl Database {
    pub fn operational_snapshot(&self) -> Result<OperationalSnapshot, String> {
        let tx = self.conn.unchecked_transaction().map_err(|error| error.to_string())?;
        tx.execute("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ", [])
            .map_err(|error| error.to_string())?;
        let review_runs = tx
            .prepare("SELECT id, pr_id, status, finished_at FROM review_runs")
            .and_then(|mut stmt| {
                stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)))?
                    .collect()
            })
            .map_err(|error| error.to_string())?;
        let pull_requests = tx
            .prepare("SELECT id, state, review_lane, reprocess_next_review, force_full_next_review FROM pull_requests")
            .and_then(|mut stmt| {
                stmt.query_map([], |row| {
                    Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?))
                })?
                .collect()
            })
            .map_err(|error| error.to_string())?;
        let review_results = tx
            .prepare("SELECT pr_id, effective_state, publication_status FROM review_results")
            .and_then(|mut stmt| {
                stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))?
                    .collect()
            })
            .map_err(|error| error.to_string())?;
        tx.commit().map_err(|error| error.to_string())?;
        Ok(OperationalSnapshot {
            review_runs,
            pull_requests,
            review_results,
        })
    }

    /// The database this connection came from, for handing work to another thread.
    pub fn handle(&self) -> Db {
        self.db.clone()
    }
}

/// Tests set up and inspect rows in SQL through the connection itself (`db.execute(..)`), as they
/// did through a second rusqlite connection.
#[cfg(test)]
impl std::ops::Deref for Database {
    type Target = pg::Conn;

    fn deref(&self) -> &pg::Conn {
        &self.conn
    }
}

/// A `Once` already run: for handles whose database has its defaults.
#[cfg(test)]
fn done() -> std::sync::Arc<std::sync::Once> {
    let once = std::sync::Once::new();
    once.call_once(|| {});
    std::sync::Arc::new(once)
}
