//! A throwaway database per test. The first test in a process migrates a template database named
//! after the migrations' hash; every test then clones it with `CREATE DATABASE … TEMPLATE`, which
//! is a file copy, and drops its clone when done. Point `OCR_TEST_DATABASE_URL` at a server where
//! that role may create databases, e.g. `postgres://ocr:dev@127.0.0.1:55432/postgres`.

use std::str::FromStr;
use std::sync::OnceLock;

use sha2::{Digest, Sha256};

use super::migrate::{migrate_client, MIGRATIONS};
use super::pool::{DbPool, DbSettings};

const TEMPLATE_LOCK_KEY: i64 = 0x6f63_725f_746d_706c; // "ocr_tmpl"

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
        let name = format!("ocr_test_{}", random_suffix());
        let mut client = connect(&admin);
        client
            .batch_execute(&format!("CREATE DATABASE {name} TEMPLATE {template}"))
            .expect("create test database");
        let mut config = admin.clone();
        config.dbname(&name);
        let settings = DbSettings::new(config.clone(), config.clone(), config, 4, 2, None).expect("test settings");
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
        if let Ok(mut client) = self.admin.connect(postgres::NoTls) {
            let _ = client.batch_execute(&format!("DROP DATABASE IF EXISTS {} WITH (FORCE)", self.name));
        }
    }
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

/// The migrated template for this build's migrations, created once per server.
fn template(admin: &postgres::Config) -> &'static str {
    static TEMPLATE: OnceLock<String> = OnceLock::new();
    TEMPLATE.get_or_init(|| {
        let mut hasher = Sha256::new();
        for (version, sql) in MIGRATIONS {
            hasher.update(version.to_le_bytes());
            hasher.update(sql.as_bytes());
        }
        let digest = hasher.finalize();
        let name = format!(
            "ocr_test_tmpl_{}",
            digest[..6].iter().map(|b| format!("{b:02x}")).collect::<String>()
        );
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
        client
            .execute("SELECT pg_advisory_unlock($1)", &[&TEMPLATE_LOCK_KEY])
            .expect("template unlock");
        name
    })
}

fn random_suffix() -> String {
    crate::db::lifecycle::generate_installation_id()
        .expect("random id")
        .replace('-', "")
}
