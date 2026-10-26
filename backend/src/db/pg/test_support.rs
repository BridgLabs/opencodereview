//! A database of its own for every test. The first test in a process migrates a template database
//! named after the migrations' hash, and new test databases are cloned from it. A finished test's
//! database is emptied (`TRUNCATE … RESTART IDENTITY`) and handed to the next test rather than
//! dropped: `DROP DATABASE` forces a checkpoint and costs seconds, a truncate milliseconds.
//! Databases left by exited test processes are adopted the same way, or dropped when they were
//! built from other migrations. Point `OCR_TEST_DATABASE_URL` at a server where that role may
//! create databases, e.g. `postgres://ocr:dev@127.0.0.1:55432/postgres`.

use std::str::FromStr;
use std::sync::{Mutex, OnceLock};

use sha2::{Digest, Sha256};

use super::migrate::{migrate_client, MIGRATIONS};
use super::pool::{DbPool, DbSettings};

const TEMPLATE_LOCK_KEY: i64 = 0x6f63_725f_746d_706c; // "ocr_tmpl"

/// Databases ready for the next test in this process, and whether they still need emptying
/// (adopted ones are emptied by the test that takes them, so adoption itself stays quick).
static FREE: Mutex<Vec<(String, bool)>> = Mutex::new(Vec::new());

pub(crate) struct TestDb {
    name: String,
    admin: postgres::Config,
    settings: DbSettings,
    pool: DbPool,
}

impl TestDb {
    pub(crate) fn new() -> Self {
        let admin = admin_config();
        let template = template(&admin);
        let reused = FREE.lock().unwrap_or_else(|error| error.into_inner()).pop();
        let reused = reused.filter(|(name, dirty)| {
            !dirty || reset(&client_config(name), &template.seed).is_ok() || {
                let _ = connect(&admin).batch_execute(&format!("DROP DATABASE IF EXISTS {name} WITH (FORCE)"));
                false
            }
        });
        let name = reused.map(|(name, _)| name).unwrap_or_else(|| {
            let name = format!("{}_{}_{}", template.prefix, std::process::id(), random_suffix());
            connect(&admin)
                .batch_execute(&format!("CREATE DATABASE {name} TEMPLATE {}", template.name))
                .expect("create test database");
            name
        });
        let mut config = admin.clone();
        config.dbname(&name);
        // Durability is irrelevant to a test database; skipping the commit flush is much faster.
        config.options("-c synchronous_commit=off");
        let mut settings = DbSettings::new(config.clone(), config.clone(), config, 8, 4, None).expect("test settings");
        // Many test databases exist at once; open connections only when a test uses them.
        settings.min_idle = 0;
        settings.scratch_pool_max = 2;
        let pool = DbPool::connect(settings.clone()).expect("test pool");
        Self {
            name,
            admin,
            settings,
            pool,
        }
    }

    pub(crate) fn pool(&self) -> &DbPool {
        &self.pool
    }

    pub(crate) fn settings(&self) -> &DbSettings {
        &self.settings
    }

    /// A plain client on the test database, for fixtures and assertions written in raw SQL.
    pub(crate) fn client(&self) -> postgres::Client {
        self.settings.connect_owner().expect("connect to test database")
    }
}

impl Drop for TestDb {
    fn drop(&mut self) {
        // A test that built a handle but never queried left the database as it was cloned.
        if self.pool.never_used() {
            FREE.lock()
                .unwrap_or_else(|error| error.into_inner())
                .push((self.name.clone(), false));
            return;
        }
        let template = template(&self.admin);
        let mut config = self.admin.clone();
        config.dbname(&self.name);
        if reset(&config, &template.seed).is_ok() {
            FREE.lock()
                .unwrap_or_else(|error| error.into_inner())
                .push((self.name.clone(), false));
        } else if let Ok(mut client) = self.admin.connect(postgres::NoTls) {
            // Something still holds its tables (a thread the test left running); start afresh.
            let _ = client.batch_execute(&format!("DROP DATABASE IF EXISTS {} WITH (FORCE)", self.name));
        }
    }
}

/// Empties every table and restores the rows the migrations seeded.
fn reset(config: &postgres::Config, seed: &[(String, String, i64)]) -> Result<(), postgres::Error> {
    let mut client = config.connect(postgres::NoTls)?;
    client.batch_execute("SET lock_timeout = '2s'")?;
    let tables: Vec<String> = client
        .query(
            "SELECT format('%I.%I', schemaname, tablename) FROM pg_tables
             WHERE schemaname IN ('public', 'scratch') AND tablename <> 'schema_migrations'",
            &[],
        )?
        .iter()
        .map(|row| row.get(0))
        .collect();
    let latest = MIGRATIONS.last().map_or(0, |(version, _)| *version);
    let mut tx = client.transaction()?;
    tx.batch_execute(&format!("TRUNCATE {} RESTART IDENTITY CASCADE", tables.join(", ")))?;
    // A test may record a migration from the future to check it is refused.
    tx.execute("DELETE FROM schema_migrations WHERE version > $1", &[&latest])?;
    for (key, value, updated_at) in seed {
        tx.execute(
            "INSERT INTO settings (key, value, updated_at) VALUES ($1, $2, $3)",
            &[key, value, updated_at],
        )?;
    }
    tx.commit()
}

fn admin_config() -> postgres::Config {
    let url = std::env::var("OCR_TEST_DATABASE_URL").expect(
        "OCR_TEST_DATABASE_URL must point at a Postgres server for tests, e.g. postgres://ocr:dev@127.0.0.1:55432/postgres",
    );
    postgres::Config::from_str(&url).expect("valid OCR_TEST_DATABASE_URL")
}

fn connect(config: &postgres::Config) -> postgres::Client {
    config
        .connect(postgres::NoTls)
        .expect("connect to OCR_TEST_DATABASE_URL")
}

struct Template {
    name: String,
    /// Test databases cloned from this template start with this prefix.
    prefix: String,
    /// The settings rows the migrations insert, restored when a database is emptied.
    seed: Vec<(String, String, i64)>,
}

/// The migrated template for this build's migrations, created once per server.
fn template(admin: &postgres::Config) -> &'static Template {
    static TEMPLATE: OnceLock<Template> = OnceLock::new();
    TEMPLATE.get_or_init(|| {
        let mut hasher = Sha256::new();
        for (version, sql) in MIGRATIONS {
            hasher.update(version.to_le_bytes());
            hasher.update(sql.as_bytes());
        }
        let hash = hasher.finalize()[..6]
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>();
        let name = format!("ocr_test_tmpl_{hash}");
        let prefix = format!("ocr_test_{hash}");
        // Test binaries run in parallel; one creates the template while the others wait.
        let mut client = connect(admin);
        client
            .execute("SELECT pg_advisory_lock($1)", &[&TEMPLATE_LOCK_KEY])
            .expect("template lock");
        let exists = client
            .query_opt("SELECT 1 FROM pg_database WHERE datname = $1", &[&name])
            .expect("look up template")
            .is_some();
        if !exists {
            let building = format!("{name}_building");
            // Separate statements: DROP and CREATE DATABASE refuse to run inside a transaction.
            client
                .batch_execute(&format!("DROP DATABASE IF EXISTS {building} WITH (FORCE)"))
                .expect("clear unfinished template");
            client
                .batch_execute(&format!("CREATE DATABASE {building}"))
                .expect("create template");
            let mut config = admin.clone();
            config.dbname(&building);
            let mut template_client = connect(&config);
            migrate_client(&mut template_client).expect("migrate template");
            drop(template_client);
            // Renamed only once complete, so a crash midway never leaves a half-built template.
            client
                .batch_execute(&format!("ALTER DATABASE {building} RENAME TO {name}"))
                .expect("publish template");
            // Any role with CREATEDB may clone a database marked as a template, not just its owner.
            client
                .batch_execute(&format!("ALTER DATABASE {name} IS_TEMPLATE true"))
                .expect("mark template");
        }
        let mut config = admin.clone();
        config.dbname(&name);
        let seed = connect(&config)
            .query("SELECT key, value, updated_at FROM settings", &[])
            .expect("read template settings")
            .iter()
            .map(|row| (row.get(0), row.get(1), row.get(2)))
            .collect();
        let template = Template { name, prefix, seed };
        adopt_abandoned(&mut client, &template);
        client
            .execute("SELECT pg_advisory_unlock($1)", &[&TEMPLATE_LOCK_KEY])
            .expect("template unlock");
        template
    })
}

/// Takes over the databases of test processes that have exited: test databases a process still
/// holds when it ends (its free list, `Db::for_test_at` registrations) are never dropped. Those
/// built from this template are renamed and reused (emptied by the test that takes them); any
/// other is dropped.
fn adopt_abandoned(client: &mut postgres::Client, template: &Template) {
    let Ok(rows) = client.query(
        "SELECT datname FROM pg_database WHERE datname ~ '^ocr_test_[0-9a-f]{12}_[0-9]+_' AND NOT datistemplate",
        &[],
    ) else {
        return;
    };
    for row in rows {
        let name: String = row.get(0);
        let mut parts = name.splitn(5, '_');
        let (hash, pid) = (parts.nth(2), parts.next());
        if pid.and_then(|pid| pid.parse::<i32>().ok()).is_some_and(process_exists) {
            continue;
        }
        let adopted = format!("{}_{}_{}", template.prefix, std::process::id(), random_suffix());
        let same_template = hash.is_some_and(|hash| template.prefix.ends_with(hash));
        let renamed = same_template
            && client
                .batch_execute(&format!("ALTER DATABASE {name} RENAME TO {adopted}"))
                .is_ok();
        if renamed {
            FREE.lock()
                .unwrap_or_else(|error| error.into_inner())
                .push((adopted, true));
        } else {
            let _ = client.batch_execute(&format!("DROP DATABASE IF EXISTS {name} WITH (FORCE)"));
        }
    }
}

fn client_config(name: &str) -> postgres::Config {
    let mut config = admin_config();
    config.dbname(name);
    config
}

fn random_suffix() -> String {
    crate::db::lifecycle::generate_installation_id()
        .expect("random id")
        .replace('-', "")
}

fn process_exists(pid: i32) -> bool {
    // Signal 0 checks for the process without signalling it; EPERM means it exists as another user.
    let signalled = unsafe { libc::kill(pid, 0) } == 0;
    signalled || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
}
