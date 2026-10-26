//! Connection pools. The server holds one `DbPool`: an application pool for reads and writes and a
//! read-only pool whose sessions default to read-only transactions, so a read path cannot write
//! even by mistake.

use std::str::FromStr;
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use postgres::config::{Host, SslMode};
use r2d2_postgres::PostgresConnectionManager;
use scheduled_thread_pool::ScheduledThreadPool;
use tokio_postgres_rustls::MakeRustlsConnect;

use super::conn::{Conn, Error};

pub type Manager = PostgresConnectionManager<MakeRustlsConnect>;
pub type Pooled = r2d2::PooledConnection<Manager>;

const DEFAULT_POOL_MAX: u32 = 32;
const DEFAULT_READ_POOL_MAX: u32 = 16;
/// Reviews building their head-commit index at the same time.
const DEFAULT_SCRATCH_POOL_MAX: u32 = 8;
const CHECKOUT_TIMEOUT: Duration = Duration::from_secs(5);

/// Where the database is and how to reach it. Built once at startup from the environment.
#[derive(Clone)]
pub struct DbSettings {
    pub app: postgres::Config,
    pub read: postgres::Config,
    /// Sees only the `scratch` schema: the per-review index of a pull request's head commit.
    pub scratch: postgres::Config,
    /// Runs migrations; owns the schema. Falls back to `app` when not configured separately.
    pub owner: postgres::Config,
    pub pool_max: u32,
    pub read_pool_max: u32,
    pub scratch_pool_max: u32,
    /// Connections each pool keeps open while idle.
    pub min_idle: u32,
    tls: MakeRustlsConnect,
}

impl DbSettings {
    /// Reads `OCR_DATABASE_URL_FILE` (a mounted secret) or, for development, `OCR_DATABASE_URL`;
    /// the same for the optional `_RO_` and `_OWNER_` URLs. The variables are removed from the
    /// process environment once read, so the git, gh and AI CLIs the server spawns never inherit
    /// a database credential. Call it before any thread starts.
    pub fn from_env() -> Result<Self, String> {
        let app = take_url("OCR_DATABASE_URL")?
            .ok_or("OCR_DATABASE_URL_FILE (or OCR_DATABASE_URL) must name the Postgres database to use")?;
        let read = take_url("OCR_DATABASE_RO_URL")?;
        let owner = take_url("OCR_DATABASE_OWNER_URL")?;
        let insecure = std::env::var("OCR_DATABASE_INSECURE").is_ok_and(|value| value == "1");
        let ca_file = std::env::var("OCR_DATABASE_CA_FILE")
            .ok()
            .filter(|value| !value.is_empty());
        let pool_max = env_u32("OCR_DB_POOL_MAX", DEFAULT_POOL_MAX)?;
        let read_pool_max = env_u32("OCR_DB_READ_POOL_MAX", DEFAULT_READ_POOL_MAX)?;
        let parse = |url: &str| postgres::Config::from_str(url).map_err(|_| "Invalid Postgres URL".to_string());
        let app = parse(&app)?;
        let read = read.as_deref().map(parse).transpose()?.unwrap_or_else(|| app.clone());
        let owner = owner.as_deref().map(parse).transpose()?.unwrap_or_else(|| app.clone());
        for config in [&app, &read, &owner] {
            require_tls_off_host(config, insecure)?;
        }
        Self::new(app, read, owner, pool_max, read_pool_max, ca_file.as_deref())
    }

    pub fn new(
        app: postgres::Config,
        read: postgres::Config,
        owner: postgres::Config,
        pool_max: u32,
        read_pool_max: u32,
        ca_file: Option<&str>,
    ) -> Result<Self, String> {
        Ok(Self {
            scratch: scratch_only(with_application_name(app.clone())),
            app: with_application_name(app),
            read: read_only(with_application_name(read)),
            owner: with_application_name(owner),
            pool_max,
            read_pool_max,
            scratch_pool_max: DEFAULT_SCRATCH_POOL_MAX,
            min_idle: 1,
            tls: tls_connector(ca_file)?,
        })
    }

    pub fn tls(&self) -> MakeRustlsConnect {
        self.tls.clone()
    }

    /// A single connection outside the pools, for migrations and session-long advisory locks.
    pub fn connect_owner(&self) -> Result<postgres::Client, Error> {
        self.owner.connect(self.tls()).map_err(Error::from)
    }
}

/// Both pools; cheap to clone, shared by every thread.
#[derive(Clone)]
pub struct DbPool {
    app: r2d2::Pool<Manager>,
    read: r2d2::Pool<Manager>,
    scratch: r2d2::Pool<Manager>,
    settings: Arc<DbSettings>,
}

impl DbPool {
    pub fn connect(settings: DbSettings) -> Result<Self, String> {
        let pool = |config, max| build_pool(config, max, settings.min_idle, settings.tls());
        let app = pool(&settings.app, settings.pool_max)?;
        let read = pool(&settings.read, settings.read_pool_max)?;
        let scratch = pool(&settings.scratch, settings.scratch_pool_max)?;
        Ok(Self {
            app,
            read,
            scratch,
            settings: Arc::new(settings),
        })
    }

    /// A connection for reads and writes. Hold it for one unit of work: never across git, HTTP
    /// or model calls, which would starve the pool.
    pub fn get(&self) -> Result<Conn, Error> {
        self.app
            .get()
            .map(Conn::new)
            .map_err(|error| Error::Pool(error.to_string()))
    }

    /// A connection whose transactions are read-only; any write fails.
    pub fn get_read(&self) -> Result<Conn, Error> {
        self.read
            .get()
            .map(Conn::new)
            .map_err(|error| Error::Pool(error.to_string()))
    }

    /// A connection to the per-review index tables and nothing else.
    pub fn get_scratch(&self) -> Result<Conn, Error> {
        self.scratch
            .get()
            .map(Conn::new)
            .map_err(|error| Error::Pool(error.to_string()))
    }

    pub fn settings(&self) -> &DbSettings {
        &self.settings
    }
}

/// r2d2 gives every pool three threads of its own for opening and reaping connections; the pools
/// share four instead, so the thread count stays flat however many pools a process has.
fn maintenance_threads() -> Arc<ScheduledThreadPool> {
    static THREADS: OnceLock<Arc<ScheduledThreadPool>> = OnceLock::new();
    THREADS
        .get_or_init(|| {
            Arc::new(
                ScheduledThreadPool::builder()
                    .num_threads(4)
                    .thread_name_pattern("ocr-db-pool-{}")
                    .build(),
            )
        })
        .clone()
}

fn build_pool(
    config: &postgres::Config,
    max: u32,
    min_idle: u32,
    tls: MakeRustlsConnect,
) -> Result<r2d2::Pool<Manager>, String> {
    r2d2::Pool::builder()
        .thread_pool(maintenance_threads())
        .max_size(max.max(1))
        .min_idle(Some(min_idle.min(max)))
        .connection_timeout(CHECKOUT_TIMEOUT)
        .build(PostgresConnectionManager::new(config.clone(), tls))
        .map_err(|error| format!("Could not connect to Postgres: {error}"))
}

fn take_url(name: &str) -> Result<Option<String>, String> {
    let file_var = format!("{name}_FILE");
    let from_file = match std::env::var(&file_var) {
        Ok(path) if !path.is_empty() => Some(
            std::fs::read_to_string(&path)
                .map_err(|error| format!("Could not read {file_var}: {error}"))?
                .trim()
                .to_string(),
        ),
        _ => None,
    };
    let inline = std::env::var(name).ok().filter(|value| !value.is_empty());
    std::env::remove_var(&file_var);
    std::env::remove_var(name);
    Ok(from_file.or(inline))
}

fn env_u32(name: &str, default: u32) -> Result<u32, String> {
    match std::env::var(name) {
        Ok(value) if !value.is_empty() => value.parse().map_err(|_| format!("{name} must be a whole number")),
        _ => Ok(default),
    }
}

fn with_application_name(mut config: postgres::Config) -> postgres::Config {
    if config.get_application_name().is_none() {
        config.application_name("ocr-server");
    }
    config
}

fn read_only(config: postgres::Config) -> postgres::Config {
    with_option(config, "-c default_transaction_read_only=on")
}

/// Without `public` on the path, a query naming an operational table fails instead of reaching it.
fn scratch_only(config: postgres::Config) -> postgres::Config {
    with_option(config, "-c search_path=scratch")
}

fn with_option(mut config: postgres::Config, option: &str) -> postgres::Config {
    let options = match config.get_options() {
        Some(existing) => format!("{existing} {option}"),
        None => option.to_string(),
    };
    config.options(&options);
    config
}

/// Plain-text connections are only for the compose network or this machine. Anywhere else the
/// URL must say `sslmode=require` (rustls then verifies the certificate and host name), unless
/// `OCR_DATABASE_INSECURE=1` says otherwise on purpose.
fn require_tls_off_host(config: &postgres::Config, insecure: bool) -> Result<(), String> {
    if insecure || config.get_ssl_mode() == SslMode::Require {
        return Ok(());
    }
    let local = config.get_hosts().iter().all(|host| match host {
        Host::Tcp(name) => matches!(name.as_str(), "localhost" | "127.0.0.1" | "::1" | "postgres"),
        #[cfg(unix)]
        Host::Unix(_) => true,
    });
    if local {
        Ok(())
    } else {
        Err("Postgres outside this host needs sslmode=require in the URL (or OCR_DATABASE_INSECURE=1)".into())
    }
}

fn tls_connector(ca_file: Option<&str>) -> Result<MakeRustlsConnect, String> {
    use rustls::pki_types::pem::PemObject;
    use rustls::pki_types::CertificateDer;

    let mut roots = rustls::RootCertStore::empty();
    match ca_file {
        Some(path) => {
            for cert in CertificateDer::pem_file_iter(path)
                .map_err(|error| format!("Could not read OCR_DATABASE_CA_FILE: {error}"))?
            {
                let cert = cert.map_err(|error| format!("Invalid certificate in OCR_DATABASE_CA_FILE: {error}"))?;
                roots
                    .add(cert)
                    .map_err(|error| format!("Invalid certificate in OCR_DATABASE_CA_FILE: {error}"))?;
            }
        }
        None => {
            for cert in rustls_native_certs::load_native_certs().certs {
                let _ = roots.add(cert);
            }
        }
    }
    let config = rustls::ClientConfig::builder()
        .with_root_certificates(roots)
        .with_no_client_auth();
    Ok(MakeRustlsConnect::new(config))
}
