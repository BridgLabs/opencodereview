use rusqlite::{params, Connection, OpenFlags, TransactionBehavior};
use serde::Serialize;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

pub const RETENTION_MS: i64 = 24 * 60 * 60 * 1000;
pub const BATCH_LIMIT: usize = 256;
type RetentionResult<T> = Result<T, Box<dyn std::error::Error>>;

const CANDIDATES: &str = "
    SELECT a.id,a.bytes FROM artifacts a
    JOIN receipts x ON x.id=a.id
    LEFT JOIN operational.review_runs r ON x.owner=printf('review:%d',r.id)
    LEFT JOIN operational.pull_requests p ON p.id=r.pr_id
    WHERE NOT EXISTS(SELECT 1 FROM uploads u WHERE u.id=a.id)
    GROUP BY a.id,a.bytes
    HAVING MIN(CASE WHEN r.id IS NOT NULL AND p.state='merged' AND p.review_lane='merged'
        AND p.reprocess_next_review=0 AND p.force_full_next_review=0
        AND r.status IN ('approved','posted','failed','cancelled','skipped','superseded','succeeded')
        AND r.finished_at>0 AND r.finished_at<?1
        AND NOT EXISTS(SELECT 1 FROM operational.review_runs j
            WHERE j.pr_id=p.id AND j.status IN ('pending','running','posting','queued','cancelling','preview'))
        AND NOT EXISTS(SELECT 1 FROM operational.review_results z
            WHERE z.pr_id=p.id AND (z.effective_state='awaiting_action' OR z.publication_status IN ('pending','posting')))
        THEN 1 ELSE 0 END)=1
    ORDER BY MAX(r.finished_at),a.id LIMIT ?2";

#[derive(Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Report {
    pub applied: bool,
    pub used_bytes: u64,
    pub candidate_bytes: u64,
    pub deleted_bytes: u64,
    pub deleted_archives: usize,
    pub pending_deletions: usize,
    pub active_jobs: u64,
    pub candidates: Vec<Candidate>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Candidate {
    pub id: String,
    pub bytes: u64,
}

fn archive_path(root: &Path, id: &str) -> RetentionResult<PathBuf> {
    if id.len() != 64
        || !id
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err("Invalid archive identity; retention stopped".into());
    }
    Ok(root.join("sealed").join(format!("{id}.archive")))
}

fn used_bytes(conn: &Connection) -> rusqlite::Result<u64> {
    conn.query_row(
        "SELECT COALESCE((SELECT SUM(bytes) FROM artifacts),0)+COALESCE((SELECT SUM(reservation) FROM uploads),0)",
        [],
        |row| row.get(0),
    )
}

fn pending_deletions(conn: &Connection) -> rusqlite::Result<usize> {
    let exists: bool = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE name='archive_retention_queue')",
        [],
        |row| row.get(0),
    )?;
    if exists {
        conn.query_row("SELECT COUNT(*) FROM archive_retention_queue", [], |row| row.get(0))
    } else {
        Ok(0)
    }
}

fn drain(conn: &mut Connection, root: &Path, report: &mut Report) -> RetentionResult<()> {
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let pending = tx
        .prepare("SELECT id,bytes FROM archive_retention_queue ORDER BY id LIMIT ?1")?
        .query_map([BATCH_LIMIT], |row| {
            Ok(Candidate {
                id: row.get(0)?,
                bytes: row.get(1)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    for candidate in pending {
        let referenced: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM artifacts WHERE id=?1 UNION ALL SELECT 1 FROM receipts WHERE id=?1 UNION ALL SELECT 1 FROM uploads WHERE id=?1)", [&candidate.id], |row| row.get(0))?;
        if !referenced {
            let path = archive_path(root, &candidate.id)?;
            match fs::symlink_metadata(&path) {
                Ok(metadata) if metadata.is_file() => fs::remove_file(path)?,
                Ok(_) => return Err("Archive retention refuses nonregular evidence files".into()),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(error.into()),
            }
            report.deleted_archives += 1;
            report.deleted_bytes = report
                .deleted_bytes
                .checked_add(candidate.bytes)
                .ok_or("Archive retention size overflow")?;
        }
        tx.execute("DELETE FROM archive_retention_queue WHERE id=?1", [&candidate.id])?;
    }
    fs::File::open(root.join("sealed"))?.sync_all()?;
    tx.commit()?;
    Ok(())
}

fn run_inner(database: &Path, apply: bool, now: i64) -> RetentionResult<Report> {
    if now <= 0 {
        return Err("Archive retention requires a valid current timestamp".into());
    }
    let mut report = Report {
        applied: apply,
        ..Report::default()
    };
    let root = database
        .parent()
        .ok_or("Operational database directory unavailable")?
        .join("review-archives");
    match fs::symlink_metadata(&root) {
        Ok(metadata) if metadata.is_dir() => {}
        Ok(_) => return Err("Archive retention refuses a nonregular store directory".into()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(report),
        Err(error) => return Err(error.into()),
    }
    if !fs::symlink_metadata(root.join("sealed"))?.is_dir()
        || !fs::symlink_metadata(root.join("index.sqlite"))?.is_file()
        || !fs::symlink_metadata(database)?.is_file()
    {
        return Err("Archive retention refuses symlinked or nonregular storage".into());
    }
    let database = database.canonicalize()?;
    let mut uri = reqwest::Url::from_file_path(database).map_err(|_| "Invalid operational database path")?;
    uri.query_pairs_mut()
        .append_pair("mode", if apply { "rw" } else { "ro" });
    let flags = if apply {
        OpenFlags::SQLITE_OPEN_READ_WRITE
    } else {
        OpenFlags::SQLITE_OPEN_READ_ONLY
    };
    let mut conn = Connection::open_with_flags(root.join("index.sqlite"), flags | OpenFlags::SQLITE_OPEN_URI)?;
    conn.busy_timeout(Duration::from_secs(5))?;
    conn.execute("ATTACH DATABASE ?1 AS operational", [uri.as_str()])?;
    if !apply {
        conn.execute_batch("PRAGMA query_only=ON")?;
    }
    let tx = conn.transaction_with_behavior(if apply {
        TransactionBehavior::Immediate
    } else {
        TransactionBehavior::Deferred
    })?;
    report.used_bytes = used_bytes(&tx)?;
    report.pending_deletions = pending_deletions(&tx)?;
    report.active_jobs = tx.query_row(
        "SELECT COUNT(*) FROM (
        SELECT pr_id FROM operational.review_runs WHERE status IN ('pending','running','posting','queued','cancelling')
        UNION SELECT id FROM operational.pull_requests WHERE review_lane IN ('queued','reviewing'))",
        [],
        |row| row.get(0),
    )?;
    if report.active_jobs > 0 {
        return Ok(report);
    }
    report.candidates = tx
        .prepare(CANDIDATES)?
        .query_map(params![now.saturating_sub(RETENTION_MS), BATCH_LIMIT], |row| {
            Ok(Candidate {
                id: row.get(0)?,
                bytes: row.get(1)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    for candidate in &report.candidates {
        if !fs::symlink_metadata(archive_path(&root, &candidate.id)?)?.is_file() {
            return Err("Archive retention refuses nonregular evidence files".into());
        }
        report.candidate_bytes = report
            .candidate_bytes
            .checked_add(candidate.bytes)
            .ok_or("Archive retention size overflow")?;
    }
    if !apply {
        return Ok(report);
    }
    tx.execute_batch("CREATE TABLE IF NOT EXISTS archive_retention_queue(id TEXT PRIMARY KEY,bytes INTEGER NOT NULL);
        CREATE TABLE IF NOT EXISTS archive_retention_events(owner TEXT NOT NULL,id TEXT NOT NULL,expired_at INTEGER NOT NULL,bytes INTEGER NOT NULL,PRIMARY KEY(owner,id));")?;
    for candidate in &report.candidates {
        tx.execute(
            "INSERT OR REPLACE INTO archive_retention_events SELECT owner,id,?2,?3 FROM receipts WHERE id=?1",
            params![candidate.id, now, candidate.bytes],
        )?;
        tx.execute(
            "INSERT OR IGNORE INTO archive_retention_queue VALUES(?1,?2)",
            params![candidate.id, candidate.bytes],
        )?;
        tx.execute("DELETE FROM receipts WHERE id=?1", [&candidate.id])?;
        tx.execute("DELETE FROM artifacts WHERE id=?1", [&candidate.id])?;
    }
    tx.commit()?;
    drain(&mut conn, &root, &mut report)?;
    report.pending_deletions = pending_deletions(&conn)?;
    report.used_bytes = used_bytes(&conn)?;
    Ok(report)
}

pub fn run(database: &Path, apply: bool, now: i64) -> Result<Report, String> {
    run_inner(database, apply, now).map_err(|error| format!("Archive retention failed: {error}"))
}

pub fn run_from_env(apply: bool) -> Result<(), String> {
    let database = std::env::var_os("OCR_DB_PATH").map(PathBuf::from).unwrap_or_else(|| {
        std::env::var_os("OCR_DATA_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("data"))
            .join("opencodereview.sqlite")
    });
    let report = run(&database, apply, crate::db::now_millis())?;
    println!("{}", serde_json::to_string(&report).map_err(|error| error.to_string())?);
    Ok(())
}

#[cfg(test)]
#[path = "purge_tests.rs"]
mod tests;
