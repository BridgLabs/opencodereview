//! `ocr-server import-sqlite`: moves an install's SQLite database (and its review archive index)
//! into an empty Postgres database, once, with the server stopped.
//!
//! The server's SQLite enforced foreign keys, but a file edited by other tools, or with
//! `PRAGMA foreign_keys` off, can hold rows whose parent is gone. Such rows keep what history
//! they can: a missing repository comes back marked removed, a nullable reference whose target is
//! gone becomes NULL (as `ON DELETE SET NULL` would have made it), and any other orphan is left
//! out and counted in the report.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

use postgres::types::ToSql;
use rusqlite::types::ValueRef;
use rusqlite::OpenFlags;
use serde::Serialize;

use super::{migrations, now_millis, pg, INSTANCE_TIMEOUT_MS};

/// Operational tables in an order where every foreign key points at a table already copied.
const TABLES: &[&str] = &[
    "workspaces",
    "repos",
    "subscriptions",
    "pull_requests",
    "review_runs",
    "pending_actions",
    "repo_index_state",
    "repo_index_packages",
    "repo_index_files",
    "repo_index_pkg_deps",
    "repo_index_edges",
    "review_context_snapshots",
    "review_run_model_usage",
    "review_threads",
    "skill_versions",
    "review_run_stages",
    "review_chunk_cache",
    "pr_summaries",
    "repo_index_symbols",
    "repo_index_symbol_edges",
    "review_results",
    "review_finding_audits",
    "pr_review_file_counts",
    "review_coverage",
    "review_request_restorations",
    "comment_scan_cursors",
    "comment_activation_receipts",
    "pr_comment_activations",
    "queue_acknowledgements",
    "ai_accounts",
];

/// The review archive index: (table in index.sqlite, table in Postgres).
const ARCHIVE_TABLES: &[(&str, &str)] = &[
    ("artifacts", "archive.artifacts"),
    ("uploads", "archive.uploads"),
    ("receipts", "archive.receipts"),
    ("deletions", "archive.deletions"),
    ("archive_retention_queue", "archive.retention_queue"),
    ("archive_retention_events", "archive.retention_events"),
];

/// Rows per INSERT statement; Postgres takes at most 65535 parameters per statement.
const BATCH_ROWS: usize = 500;

#[derive(Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportReport {
    /// Rows copied per table.
    pub tables: BTreeMap<String, u64>,
    /// Repositories that were missing and came back marked removed, so their history stays.
    pub restored_repositories: Vec<i64>,
    /// References set to NULL because their target was gone, per `table.column`.
    pub cleared_references: BTreeMap<String, u64>,
    /// Rows left out because a required parent was gone, per table.
    pub skipped_orphans: BTreeMap<String, u64>,
    /// Text values with bytes Postgres cannot store (NUL, invalid UTF-8), replaced.
    pub repaired_text: u64,
    pub archive_index_imported: bool,
}

pub fn run_from_env(source: &Path, archive_index: Option<&Path>) -> Result<(), String> {
    let settings = pg::DbSettings::from_env()?;
    pg::migrate(&settings)?;
    let archive_index = archive_index.map(Path::to_path_buf).or_else(|| {
        let data_dir = std::env::var_os("OCR_DATA_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("data"));
        Some(crate::review_archive::archive_dir(&data_dir).join("index.sqlite")).filter(|path| path.is_file())
    });
    let mut client = settings.connect_owner().map_err(|error| error.to_string())?;
    let report = import(&mut client, source, archive_index.as_deref())?;
    println!("{}", serde_json::to_string(&report).map_err(|error| error.to_string())?);
    Ok(())
}

/// Imports `source` (and the archive index, when given) into the database behind `client`, which
/// must be the schema owner. Refuses while a server instance is running or once data exists.
pub fn import(
    client: &mut postgres::Client,
    source: &Path,
    archive_index: Option<&Path>,
) -> Result<ImportReport, String> {
    refuse_running_or_populated(client)?;
    let scratch = tempfile_dir()?;
    let result = (|| -> Result<ImportReport, String> {
        let operational = snapshot(source, &scratch.join("opencodereview.sqlite"))?;
        bring_up_to_date(&operational)?;
        let archive = archive_index
            .map(|path| snapshot(path, &scratch.join("index.sqlite")))
            .transpose()?;
        let mut report = ImportReport::default();
        let mut tx = client.transaction().map_err(db_error)?;
        copy_operational(&mut tx, &operational, &mut report)?;
        if let Some(archive) = &archive {
            copy_archive(&mut tx, archive, &mut report)?;
        }
        tx.commit().map_err(db_error)?;
        Ok(report)
    })();
    let _ = fs::remove_dir_all(&scratch);
    let mut report = result?;
    // Only now that the rows are in Postgres: the archive store refuses to open while this
    // file exists (its recovery would otherwise discard archives the empty index lacked).
    if let Some(index) = archive_index {
        retire_archive_index(index)?;
        report.archive_index_imported = true;
    }
    Ok(report)
}

fn db_error(error: postgres::Error) -> String {
    pg::Error::from(error).to_string()
}

fn sqlite_error(error: rusqlite::Error) -> String {
    format!("SQLite source: {error}")
}

fn refuse_running_or_populated(client: &mut postgres::Client) -> Result<(), String> {
    let live: i64 = client
        .query_one(
            "SELECT COUNT(*) FROM server_instances WHERE heartbeat_at > $1",
            &[&(now_millis() - INSTANCE_TIMEOUT_MS)],
        )
        .map_err(db_error)?
        .get(0);
    if live > 0 {
        return Err("Stop every OpenCodeReview server before importing (a server instance is running)".into());
    }
    let populated: bool = client
        .query_one(
            "SELECT EXISTS (SELECT 1 FROM workspaces) OR EXISTS (SELECT 1 FROM repos)
               OR EXISTS (SELECT 1 FROM review_runs) OR EXISTS (SELECT 1 FROM ai_accounts)
               OR EXISTS (SELECT 1 FROM archive.artifacts)",
            &[],
        )
        .map_err(db_error)?
        .get(0);
    if populated {
        return Err("The Postgres database already has data; import only into a new, empty database".into());
    }
    Ok(())
}

fn tempfile_dir() -> Result<PathBuf, String> {
    let directory = std::env::temp_dir().join(format!(
        "ocr-import-{}",
        crate::sandbox::secret().map_err(|error| error.to_string())?
    ));
    fs::create_dir(&directory).map_err(|error| error.to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&directory, fs::Permissions::from_mode(0o700)).map_err(|error| error.to_string())?;
    }
    Ok(directory)
}

/// A consistent copy of `source`, WAL included, opened from the copy: the source is only read.
fn snapshot(source: &Path, copy: &Path) -> Result<rusqlite::Connection, String> {
    if !source.is_file() {
        return Err(format!("{} is not a file", source.display()));
    }
    let original =
        rusqlite::Connection::open_with_flags(source, OpenFlags::SQLITE_OPEN_READ_ONLY).map_err(sqlite_error)?;
    original
        .execute("VACUUM INTO ?1", [copy.to_string_lossy()])
        .map_err(sqlite_error)?;
    drop(original);
    let snapshot = rusqlite::Connection::open(copy).map_err(sqlite_error)?;
    // The file is untrusted input: no schema-defined functions run while it is read.
    snapshot
        .execute_batch("PRAGMA trusted_schema = OFF")
        .map_err(sqlite_error)?;
    Ok(snapshot)
}

/// Applies the SQLite migrations an older install had not run yet, on the copy.
fn bring_up_to_date(snapshot: &rusqlite::Connection) -> Result<(), String> {
    let version: i64 = snapshot
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .map_err(sqlite_error)?;
    let latest = migrations::MIGRATIONS.len() as i64;
    if version > latest {
        return Err(format!(
            "The SQLite database is schema {version}, newer than this importer knows ({latest}); import with the matching release"
        ));
    }
    if version == 0 {
        return Err("The SQLite file has no OpenCodeReview schema".into());
    }
    for (index, migration) in migrations::MIGRATIONS.iter().enumerate().skip(version as usize) {
        snapshot
            .execute_batch(&format!(
                "BEGIN;\n{migration}\nPRAGMA user_version = {};\nCOMMIT;",
                index + 1
            ))
            .map_err(sqlite_error)?;
    }
    Ok(())
}

#[derive(Clone, Copy, PartialEq)]
enum Kind {
    Integer,
    Text,
    Bytes,
}

/// One value on its way to Postgres, typed by the target column.
enum Value {
    Integer(Option<i64>),
    Text(Option<String>),
    Bytes(Option<Vec<u8>>),
}

impl Value {
    fn as_sql(&self) -> &(dyn ToSql + Sync) {
        match self {
            Value::Integer(value) => value,
            Value::Text(value) => value,
            Value::Bytes(value) => value,
        }
    }

    fn integer(&self) -> Option<i64> {
        match self {
            Value::Integer(value) => *value,
            _ => None,
        }
    }
}

fn convert(value: ValueRef<'_>, kind: Kind, repaired: &mut u64) -> Result<Value, String> {
    let text = |bytes: &[u8], repaired: &mut u64| match std::str::from_utf8(bytes) {
        Ok(text) if !text.contains('\0') => text.to_string(),
        _ => {
            *repaired += 1;
            String::from_utf8_lossy(bytes).replace('\0', "\u{FFFD}")
        }
    };
    Ok(match (kind, value) {
        (Kind::Integer, ValueRef::Null) => Value::Integer(None),
        (Kind::Integer, ValueRef::Integer(number)) => Value::Integer(Some(number)),
        (Kind::Integer, ValueRef::Real(number)) => Value::Integer(Some(number.round() as i64)),
        (Kind::Integer, ValueRef::Text(bytes)) => Value::Integer(Some(
            String::from_utf8_lossy(bytes)
                .trim()
                .parse()
                .map_err(|_| "a number column holds text that is not a number".to_string())?,
        )),
        (Kind::Integer, ValueRef::Blob(_)) => return Err("a number column holds binary data".into()),
        (Kind::Text, ValueRef::Null) => Value::Text(None),
        (Kind::Text, ValueRef::Text(bytes) | ValueRef::Blob(bytes)) => Value::Text(Some(text(bytes, repaired))),
        (Kind::Text, ValueRef::Integer(number)) => Value::Text(Some(number.to_string())),
        (Kind::Text, ValueRef::Real(number)) => Value::Text(Some(number.to_string())),
        (Kind::Bytes, ValueRef::Null) => Value::Bytes(None),
        (Kind::Bytes, ValueRef::Blob(bytes) | ValueRef::Text(bytes)) => Value::Bytes(Some(bytes.to_vec())),
        (Kind::Bytes, ValueRef::Integer(_) | ValueRef::Real(_)) => {
            return Err("a binary column holds a number".into());
        }
    })
}

/// Target columns present in both databases, with their Postgres kind.
fn shared_columns(
    tx: &mut postgres::Transaction<'_>,
    source: &rusqlite::Connection,
    source_table: &str,
    target_table: &str,
) -> Result<Vec<(String, Kind)>, String> {
    let (schema, table) = target_table.split_once('.').unwrap_or(("public", target_table));
    let target: Vec<(String, Kind)> = tx
        .query(
            "SELECT column_name, data_type FROM information_schema.columns
             WHERE table_schema = $1 AND table_name = $2 ORDER BY ordinal_position",
            &[&schema, &table],
        )
        .map_err(db_error)?
        .iter()
        .map(|row| {
            let kind = match row.get::<_, String>(1).as_str() {
                "bigint" | "integer" | "smallint" => Kind::Integer,
                "bytea" => Kind::Bytes,
                _ => Kind::Text,
            };
            (row.get(0), kind)
        })
        .collect();
    let mut stmt = source
        .prepare(&format!("SELECT name FROM pragma_table_info('{source_table}')"))
        .map_err(sqlite_error)?;
    let present: HashSet<String> = stmt
        .query_map([], |row| row.get(0))
        .map_err(sqlite_error)?
        .collect::<Result<_, _>>()
        .map_err(sqlite_error)?;
    Ok(target.into_iter().filter(|(name, _)| present.contains(name)).collect())
}

/// A foreign key of a target table: the column, the table and column it references, and whether
/// a dangling value may become NULL.
struct ForeignKey {
    column: String,
    parent: String,
    parent_column: String,
    set_null: bool,
}

fn foreign_keys(tx: &mut postgres::Transaction<'_>, table: &str) -> Result<Vec<ForeignKey>, String> {
    Ok(tx
        .query(
            "SELECT a.attname, c.confrelid::regclass::text, af.attname, c.confdeltype = 'n'
             FROM pg_constraint c
             JOIN pg_attribute a ON a.attrelid = c.conrelid AND a.attnum = c.conkey[1]
             JOIN pg_attribute af ON af.attrelid = c.confrelid AND af.attnum = c.confkey[1]
             WHERE c.contype = 'f' AND c.conrelid = to_regclass($1)",
            &[&table],
        )
        .map_err(db_error)?
        .iter()
        .map(|row| ForeignKey {
            column: row.get(0),
            parent: row.get(1),
            parent_column: row.get(2),
            set_null: row.get(3),
        })
        .collect())
}

fn copy_operational(
    tx: &mut postgres::Transaction<'_>,
    source: &rusqlite::Connection,
    report: &mut ImportReport,
) -> Result<(), String> {
    // Settings: the source's replace the fresh database's defaults, installation id included,
    // so result markers already posted on GitHub stay this installation's.
    let mut stmt = source
        .prepare("SELECT key, value, updated_at FROM settings")
        .map_err(sqlite_error)?;
    let settings: Vec<(String, String, i64)> = stmt
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
        .map_err(sqlite_error)?
        .collect::<Result<_, _>>()
        .map_err(sqlite_error)?;
    for (key, value, updated_at) in &settings {
        // Postgres migration 0010 ran before this copy; apply it to the imported value too.
        let value = match (key.as_str(), value.as_str()) {
            ("review_archive_retention_days", "1") => "0",
            _ => value.as_str(),
        };
        tx.execute(
            "INSERT INTO settings (key, value, updated_at) VALUES ($1, $2, $3)
             ON CONFLICT (key) DO UPDATE SET value = excluded.value, updated_at = excluded.updated_at",
            &[key, &value, updated_at],
        )
        .map_err(db_error)?;
    }
    report.tables.insert("settings".into(), settings.len() as u64);
    // Enabling a subscription seeds its comment cursor through a trigger; the source's cursors
    // are copied as they are instead.
    tx.batch_execute("ALTER TABLE subscriptions DISABLE TRIGGER USER")
        .map_err(db_error)?;
    // Keys copied so far, to tell a dangling reference from a good one.
    let mut copied: HashMap<(String, String), HashSet<i64>> = HashMap::new();
    for table in TABLES {
        copy_table(tx, source, table, table, &mut copied, report)?;
    }
    tx.batch_execute("ALTER TABLE subscriptions ENABLE TRIGGER USER")
        .map_err(db_error)?;
    restart_identities(tx, source)
}

fn copy_table(
    tx: &mut postgres::Transaction<'_>,
    source: &rusqlite::Connection,
    source_table: &str,
    target_table: &str,
    copied: &mut HashMap<(String, String), HashSet<i64>>,
    report: &mut ImportReport,
) -> Result<(), String> {
    let exists: bool = source
        .query_row(
            "SELECT EXISTS (SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1)",
            [source_table],
            |row| row.get(0),
        )
        .map_err(sqlite_error)?;
    if !exists {
        return Ok(());
    }
    let columns = shared_columns(tx, source, source_table, target_table)?;
    let keys = if target_table.contains('.') {
        Vec::new()
    } else {
        foreign_keys(tx, target_table)?
    };
    let names: Vec<&str> = columns.iter().map(|(name, _)| name.as_str()).collect();
    let mut stmt = source
        .prepare(&format!(
            "SELECT {} FROM \"{source_table}\"{}",
            names
                .iter()
                .map(|name| format!("\"{name}\""))
                .collect::<Vec<_>>()
                .join(", "),
            if names.contains(&"id") { " ORDER BY id" } else { "" }
        ))
        .map_err(sqlite_error)?;
    let mut rows = stmt.query([]).map_err(sqlite_error)?;
    let mut batch: Vec<Vec<Value>> = Vec::new();
    let mut count = 0u64;
    let error = |error: String| format!("{source_table}: {error}");
    while let Some(row) = rows.next().map_err(sqlite_error)? {
        let mut values = Vec::with_capacity(columns.len());
        for (index, (_, kind)) in columns.iter().enumerate() {
            let value = row.get_ref(index).map_err(sqlite_error)?;
            values.push(convert(value, *kind, &mut report.repaired_text).map_err(error)?);
        }
        if !resolve_references(tx, target_table, &names, &mut values, &keys, copied, report)? {
            *report.skipped_orphans.entry(target_table.into()).or_default() += 1;
            continue;
        }
        if let Some(position) = names.iter().position(|name| *name == "id") {
            if let Some(id) = values[position].integer() {
                copied.entry((target_table.into(), "id".into())).or_default().insert(id);
            }
        }
        for key in ["repo_id", "pr_id", "run_id"] {
            // Tables keyed by another table's id (repo_index_state, review_finding_audits, …).
            if let Some(position) = names.iter().position(|name| *name == key) {
                if let Some(value) = values[position].integer() {
                    copied
                        .entry((target_table.into(), key.into()))
                        .or_default()
                        .insert(value);
                }
            }
        }
        batch.push(values);
        count += 1;
        if batch.len() == BATCH_ROWS {
            insert(tx, target_table, &names, &batch)?;
            batch.clear();
        }
    }
    insert(tx, target_table, &names, &batch)?;
    report.tables.insert(target_table.into(), count);
    Ok(())
}

/// Checks a row's references against what was copied; repairs or rejects dangling ones.
fn resolve_references(
    tx: &mut postgres::Transaction<'_>,
    table: &str,
    names: &[&str],
    values: &mut [Value],
    keys: &[ForeignKey],
    copied: &mut HashMap<(String, String), HashSet<i64>>,
    report: &mut ImportReport,
) -> Result<bool, String> {
    for key in keys {
        let Some(position) = names.iter().position(|name| *name == key.column) else {
            continue;
        };
        let Some(target) = values[position].integer() else {
            continue;
        };
        let present = copied
            .get(&(key.parent.clone(), key.parent_column.clone()))
            .is_some_and(|keys| keys.contains(&target));
        // A row may reference itself or an earlier row of its own table (skill_versions).
        let own_earlier = key.parent == table
            && copied
                .get(&(table.to_string(), key.parent_column.clone()))
                .is_some_and(|keys| keys.contains(&target));
        if present || own_earlier {
            continue;
        }
        if key.parent == "repos" && key.parent_column == "id" {
            restore_repository(tx, target, copied, report)?;
        } else if key.set_null {
            values[position] = Value::Integer(None);
            *report
                .cleared_references
                .entry(format!("{table}.{}", key.column))
                .or_default() += 1;
        } else {
            return Ok(false);
        }
    }
    Ok(true)
}

/// Brings back a repository the source no longer had, marked removed, so its history imports.
fn restore_repository(
    tx: &mut postgres::Transaction<'_>,
    id: i64,
    copied: &mut HashMap<(String, String), HashSet<i64>>,
    report: &mut ImportReport,
) -> Result<(), String> {
    let now = now_millis();
    tx.execute(
        "INSERT INTO repos (id, owner, name, full_name, created_at, updated_at, removed_at)
         VALUES ($1, 'removed', $2, $3, $4, $4, $4)",
        &[
            &id,
            &format!("repository-{id}"),
            &format!("removed/repository-{id}"),
            &now,
        ],
    )
    .map_err(db_error)?;
    copied.entry(("repos".into(), "id".into())).or_default().insert(id);
    report.restored_repositories.push(id);
    Ok(())
}

fn insert(tx: &mut postgres::Transaction<'_>, table: &str, names: &[&str], rows: &[Vec<Value>]) -> Result<(), String> {
    if rows.is_empty() {
        return Ok(());
    }
    let width = names.len();
    let placeholders = (0..rows.len())
        .map(|row| {
            let cells = (1..=width)
                .map(|column| format!("${}", row * width + column))
                .collect::<Vec<_>>()
                .join(", ");
            format!("({cells})")
        })
        .collect::<Vec<_>>()
        .join(", ");
    let sql = format!(
        "INSERT INTO {table} ({}) VALUES {placeholders}",
        names
            .iter()
            .map(|name| format!("\"{name}\""))
            .collect::<Vec<_>>()
            .join(", ")
    );
    let params: Vec<&(dyn ToSql + Sync)> = rows.iter().flatten().map(Value::as_sql).collect();
    tx.execute(sql.as_str(), &params)
        .map_err(|error| format!("{table}: {}", db_error(error)))?;
    Ok(())
}

/// Identity sequences continue after the highest id SQLite ever handed out, deleted rows
/// included (archives still name deleted runs), or start at 1 for an empty table.
fn restart_identities(tx: &mut postgres::Transaction<'_>, source: &rusqlite::Connection) -> Result<(), String> {
    let mut stmt = source
        .prepare("SELECT name, seq FROM sqlite_sequence")
        .map_err(sqlite_error)?;
    let issued: HashMap<String, i64> = stmt
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
        .map_err(sqlite_error)?
        .collect::<Result<_, _>>()
        .map_err(sqlite_error)?;
    for table in TABLES {
        // Tables keyed by another table's id (repo_index_state, …) have no sequence of their own.
        let sequence: Option<String> = tx
            .query_one(
                "SELECT (SELECT pg_get_serial_sequence(table_name, 'id') FROM information_schema.columns
                         WHERE table_schema = 'public' AND table_name = $1 AND column_name = 'id')",
                &[table],
            )
            .map_err(db_error)?
            .get(0);
        let Some(sequence) = sequence else {
            continue;
        };
        let highest: Option<i64> = tx
            .query_one(&format!("SELECT MAX(id) FROM {table}"), &[])
            .map_err(db_error)?
            .get(0);
        let last = highest.unwrap_or(0).max(issued.get(*table).copied().unwrap_or(0));
        tx.execute(
            "SELECT setval(to_regclass($1), GREATEST($2::BIGINT, 1), $3)",
            &[&sequence, &last, &(last > 0)],
        )
        .map_err(db_error)?;
    }
    Ok(())
}

fn copy_archive(
    tx: &mut postgres::Transaction<'_>,
    source: &rusqlite::Connection,
    report: &mut ImportReport,
) -> Result<(), String> {
    let mut copied = HashMap::new();
    for (source_table, target_table) in ARCHIVE_TABLES {
        copy_table(tx, source, source_table, target_table, &mut copied, report)?;
    }
    Ok(())
}

fn retire_archive_index(index: &Path) -> Result<(), String> {
    for suffix in ["", "-wal", "-shm"] {
        let path = PathBuf::from(format!("{}{suffix}", index.display()));
        if path.exists() {
            fs::rename(&path, format!("{}{suffix}.imported", index.display()))
                .map_err(|error| format!("Imported, but could not retire {}: {error}", path.display()))?;
        }
    }
    Ok(())
}

#[cfg(test)]
#[path = "import_sqlite_tests.rs"]
mod tests;
