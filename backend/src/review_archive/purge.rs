use crate::db::pg::{params, Conn, Transaction};
use crate::db::Db;
use serde::Serialize;
use std::fs;
use std::path::{Path, PathBuf};

pub const RETENTION_MS: i64 = 24 * 60 * 60 * 1000;
pub const BATCH_LIMIT: usize = 256;
type RetentionResult<T> = Result<T, Box<dyn std::error::Error>>;

const CANDIDATES: &str = "
    SELECT a.id, a.bytes FROM archive.artifacts a
    JOIN archive.receipts x ON x.id = a.id
    LEFT JOIN review_runs r ON x.owner = 'review:' || r.id
    LEFT JOIN pull_requests p ON p.id = r.pr_id
    WHERE NOT EXISTS (SELECT 1 FROM archive.uploads u WHERE u.id = a.id)
    GROUP BY a.id, a.bytes
    HAVING MIN(CASE WHEN r.id IS NOT NULL AND p.state = 'merged' AND p.review_lane = 'merged'
        AND p.reprocess_next_review = 0 AND p.force_full_next_review = 0
        AND r.status IN ('approved', 'posted', 'failed', 'cancelled', 'skipped', 'superseded', 'succeeded')
        AND r.finished_at > 0 AND r.finished_at < $1
        AND NOT EXISTS (SELECT 1 FROM review_runs j
            WHERE j.pr_id = p.id AND j.status IN ('pending', 'running', 'posting', 'queued', 'cancelling', 'preview'))
        AND NOT EXISTS (SELECT 1 FROM review_results z
            WHERE z.pr_id = p.id AND (z.effective_state = 'awaiting_action' OR z.publication_status IN ('pending', 'posting')))
        THEN 1 ELSE 0 END) = 1
    ORDER BY MAX(r.finished_at), a.id LIMIT $2";

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

fn used_bytes(conn: &Conn) -> RetentionResult<u64> {
    Ok(conn.query_row(
        "SELECT (COALESCE((SELECT SUM(bytes) FROM archive.artifacts), 0)
           + COALESCE((SELECT SUM(reservation) FROM archive.uploads), 0))::BIGINT",
        [],
        |row| row.get(0),
    )?)
}

fn pending_deletions(conn: &Conn) -> RetentionResult<usize> {
    Ok(conn.query_row("SELECT COUNT(*) FROM archive.retention_queue", [], |row| row.get(0))?)
}

fn drain(db: &Db, root: &Path, report: &mut Report) -> RetentionResult<()> {
    let conn = db.connection()?;
    let tx = super::store::write_transaction(&conn)?;
    let pending = tx
        .prepare("SELECT id, bytes FROM archive.retention_queue ORDER BY id LIMIT $1")?
        .query_map([BATCH_LIMIT as i64], |row| {
            Ok(Candidate {
                id: row.get(0)?,
                bytes: row.get(1)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    for candidate in pending {
        let referenced: bool = tx.query_row(
            "SELECT EXISTS (SELECT 1 FROM archive.artifacts WHERE id = $1 UNION ALL SELECT 1 FROM archive.receipts WHERE id = $1
               UNION ALL SELECT 1 FROM archive.uploads WHERE id = $1)",
            [&candidate.id],
            |row| row.get(0),
        )?;
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
        tx.execute("DELETE FROM archive.retention_queue WHERE id = $1", [&candidate.id])?;
    }
    fs::File::open(root.join("sealed"))?.sync_all()?;
    tx.commit()?;
    Ok(())
}

/// A preview reads through the read-only pool, so it cannot write even by mistake. Applying holds
/// the archive gate (reviews take it shared as they start) and the index writers' lock until the
/// deletions commit, as `BEGIN IMMEDIATE` across the two SQLite files did.
fn begin<'c>(conn: &'c Conn, apply: bool) -> RetentionResult<Transaction<'c>> {
    let tx = if apply {
        let tx = conn.unchecked_transaction()?;
        tx.execute("SELECT pg_advisory_xact_lock($1)", [crate::db::ARCHIVE_GATE_KEY])?;
        tx.execute("SELECT pg_advisory_xact_lock($1)", [super::store::ARCHIVE_INDEX_KEY])?;
        tx
    } else {
        conn.unchecked_transaction()?
    };
    Ok(tx)
}

fn run_inner(db: &Db, root: &Path, apply: bool, now: i64) -> RetentionResult<Report> {
    if now <= 0 {
        return Err("Archive retention requires a valid current timestamp".into());
    }
    let mut report = Report {
        applied: apply,
        ..Report::default()
    };
    match fs::symlink_metadata(root) {
        Ok(metadata) if metadata.is_dir() => {}
        Ok(_) => return Err("Archive retention refuses a nonregular store directory".into()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(report),
        Err(error) => return Err(error.into()),
    }
    if !fs::symlink_metadata(root.join("sealed"))?.is_dir() {
        return Err("Archive retention refuses symlinked or nonregular storage".into());
    }
    if fs::symlink_metadata(root.join("index.sqlite")).is_ok() {
        return Err(crate::review_archive::Error::UnimportedIndex.to_string().into());
    }
    let conn = if apply { db.connection()? } else { db.read_connection()? };
    let tx = begin(&conn, apply)?;
    report.used_bytes = used_bytes(&tx)?;
    report.pending_deletions = pending_deletions(&tx)?;
    report.active_jobs = tx.query_row(
        "SELECT COUNT(*) FROM (
           SELECT pr_id FROM review_runs WHERE status IN ('pending', 'running', 'posting', 'queued', 'cancelling')
           UNION SELECT id FROM pull_requests WHERE review_lane IN ('queued', 'reviewing')) active",
        [],
        |row| row.get(0),
    )?;
    if report.active_jobs > 0 {
        return Ok(report);
    }
    report.candidates = tx
        .prepare(CANDIDATES)?
        .query_map(params![now.saturating_sub(RETENTION_MS), BATCH_LIMIT as i64], |row| {
            Ok(Candidate {
                id: row.get(0)?,
                bytes: row.get(1)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    for candidate in &report.candidates {
        if !fs::symlink_metadata(archive_path(root, &candidate.id)?)?.is_file() {
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
    for candidate in &report.candidates {
        tx.execute(
            "INSERT INTO archive.retention_events (owner, id, expired_at, bytes)
             SELECT owner, id, $2, $3 FROM archive.receipts WHERE id = $1
             ON CONFLICT (owner, id) DO UPDATE SET expired_at = excluded.expired_at, bytes = excluded.bytes",
            params![candidate.id, now, candidate.bytes as i64],
        )?;
        tx.execute(
            "INSERT INTO archive.retention_queue (id, bytes) VALUES ($1, $2) ON CONFLICT DO NOTHING",
            params![candidate.id, candidate.bytes as i64],
        )?;
        tx.execute("DELETE FROM archive.receipts WHERE id = $1", [&candidate.id])?;
        tx.execute("DELETE FROM archive.artifacts WHERE id = $1", [&candidate.id])?;
    }
    tx.commit()?;
    drain(db, root, &mut report)?;
    report.pending_deletions = pending_deletions(&conn)?;
    report.used_bytes = used_bytes(&conn)?;
    Ok(report)
}

pub fn run(db: &Db, root: &Path, apply: bool, now: i64) -> Result<Report, String> {
    run_inner(db, root, apply, now).map_err(|error| format!("Archive retention failed: {error}"))
}

pub fn run_from_env(apply: bool) -> Result<(), String> {
    let data_dir = std::env::var_os("OCR_DATA_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("data"));
    let db = Db::from_env()?;
    // The same store the server writes to.
    let root = crate::review_archive::archive_dir(&data_dir);
    let report = run(&db, &root, apply, crate::db::now_millis())?;
    println!("{}", serde_json::to_string(&report).map_err(|error| error.to_string())?);
    Ok(())
}

#[cfg(test)]
#[path = "purge_tests.rs"]
mod tests;
