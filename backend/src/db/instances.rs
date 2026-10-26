use super::*;

/// An instance whose heartbeat is older than this has stopped; its reviews and indexing are
/// reclaimed. Heartbeats come every `INSTANCE_HEARTBEAT`.
pub const INSTANCE_TIMEOUT_MS: i64 = 60_000;
pub const INSTANCE_HEARTBEAT: std::time::Duration = std::time::Duration::from_secs(10);

/// Heartbeats are stamped and judged by the database's clock: instances on hosts whose clocks
/// disagree would otherwise take each other for stopped.
const DATABASE_NOW_MS: &str = "(EXTRACT(EPOCH FROM clock_timestamp()) * 1000)::bigint";

/// Work no live instance owns: its instance stopped, or it predates instance tracking.
const ORPHANED: &str =
    "(owner_instance IS NULL OR owner_instance NOT IN (SELECT id FROM server_instances WHERE heartbeat_at > $1))";

impl Database {
    /// The heartbeat time after which an instance still counts as live.
    pub(crate) fn instance_live_after(&self) -> Result<i64, String> {
        self.conn
            .query_row(&format!("SELECT {DATABASE_NOW_MS}"), [], |row| row.get::<_, i64>(0))
            .map(|now| now - INSTANCE_TIMEOUT_MS)
            .map_err(|error| error.to_string())
    }

    /// Records this server instance. Earlier rows for the same host are its previous runs (one
    /// server per host, and Compose gives each container its own host name); their work stopped
    /// with them, so they are removed and that work becomes reclaimable at once.
    pub fn register_instance(&self, id: &str, host: &str) -> Result<(), String> {
        let tx = self.conn.unchecked_transaction().map_err(|error| error.to_string())?;
        tx.execute("DELETE FROM server_instances WHERE host = $1", [host])
            .map_err(|error| error.to_string())?;
        tx.execute(
            &format!(
                "INSERT INTO server_instances (id, host, started_at, heartbeat_at) VALUES ($1, $2, {DATABASE_NOW_MS}, {DATABASE_NOW_MS})"
            ),
            params![id, host],
        )
        .map_err(|error| error.to_string())?;
        tx.commit().map_err(|error| error.to_string())
    }

    /// Refreshes this instance's heartbeat; registers it again if another instance reclaimed it
    /// after a long pause (its reviews then find their runs cancelled and stop).
    pub fn heartbeat_instance(&self, id: &str, host: &str) -> Result<(), String> {
        self.conn
            .execute(
                &format!(
                    "INSERT INTO server_instances (id, host, started_at, heartbeat_at) VALUES ($1, $2, {DATABASE_NOW_MS}, {DATABASE_NOW_MS})
                     ON CONFLICT (id) DO UPDATE SET heartbeat_at = excluded.heartbeat_at"
                ),
                params![id, host],
            )
            .map(|_| ())
            .map_err(|error| error.to_string())
    }

    /// Reviews and index runs whose instance is gone are interrupted: reviews are cancelled and
    /// their PRs need a manual review again; index runs are cancelled. Stopped instances are
    /// forgotten. Returns how many reviews were interrupted.
    pub fn reclaim_orphaned_work(&self) -> Result<usize, String> {
        let now = now_millis();
        let tx = self.conn.unchecked_transaction().map_err(|error| error.to_string())?;
        let live_after = self.instance_live_after()?;
        tx.execute(
            &format!(
                "UPDATE pull_requests SET review_lane = 'manual_required', status_reason = 'Review interrupted by a server restart', updated_at = $2
                 WHERE id IN (SELECT pr_id FROM review_runs WHERE status IN ('pending', 'running') AND {ORPHANED})"
            ),
            params![live_after, now],
        )
        .map_err(|error| error.to_string())?;
        let interrupted = tx
            .execute(
                &format!(
                    "UPDATE review_runs SET status = 'cancelled', error_message = 'Interrupted by a server restart.', finished_at = $2, updated_at = $2
                     WHERE status IN ('pending', 'running') AND {ORPHANED}"
                ),
                params![live_after, now],
            )
            .map_err(|error| error.to_string())?;
        tx.execute(
            "UPDATE review_finding_audits SET audit_json = json_without_member(audit_json, 'diff')
             WHERE run_id IN (SELECT id FROM review_runs WHERE status = 'cancelled') AND json_member_type(audit_json, 'diff') IS NOT NULL",
            [],
        )
        .map_err(|error| error.to_string())?;
        tx.execute(
            &format!(
                "UPDATE repo_index_state SET status = 'cancelled', stage = 'cancelled', error_message = 'Indexing interrupted by a server restart.', updated_at = $2
                 WHERE status = 'indexing' AND {ORPHANED}"
            ),
            params![live_after, now],
        )
        .map_err(|error| error.to_string())?;
        tx.execute("DELETE FROM server_instances WHERE heartbeat_at <= $1", [live_after])
            .map_err(|error| error.to_string())?;
        tx.commit().map_err(|error| error.to_string())?;
        Ok(interrupted)
    }

    /// Cancels the reviews this instance owns but no longer runs (`running_prs` are the PRs it
    /// does run): a start or a finish that failed to record its outcome left them pending or
    /// running, and they would refuse every later review of their PR while this instance lives.
    /// Returns how many were cancelled.
    pub fn cancel_untracked_review_runs(&self, running_prs: &[i64]) -> Result<usize, String> {
        let Some(instance) = self.db.instance_id() else {
            return Ok(0);
        };
        const UNTRACKED: &str = "status IN ('pending', 'running') AND owner_instance = $1 AND NOT (pr_id = ANY($2))";
        let now = now_millis();
        let tx = self.conn.unchecked_transaction().map_err(|error| error.to_string())?;
        tx.execute(
            &format!(
                "UPDATE pull_requests SET review_lane = 'manual_required', status_reason = 'Review stopped without finishing', updated_at = $3
                 WHERE id IN (SELECT pr_id FROM review_runs WHERE {UNTRACKED})"
            ),
            params![instance, running_prs, now],
        )
        .map_err(|error| error.to_string())?;
        let cancelled = tx
            .execute(
                &format!(
                    "UPDATE review_runs SET status = 'cancelled', error_message = 'Stopped without finishing.', finished_at = $3, updated_at = $3
                     WHERE {UNTRACKED}"
                ),
                params![instance, running_prs, now],
            )
            .map_err(|error| error.to_string())?;
        tx.commit().map_err(|error| error.to_string())?;
        Ok(cancelled as usize)
    }
}
