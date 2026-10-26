use super::*;

/// An instance whose heartbeat is older than this has stopped; its reviews and indexing are
/// reclaimed. Heartbeats come every `INSTANCE_HEARTBEAT`.
pub const INSTANCE_TIMEOUT_MS: i64 = 60_000;
pub const INSTANCE_HEARTBEAT: std::time::Duration = std::time::Duration::from_secs(10);

/// Work no live instance owns: its instance stopped, or it predates instance tracking.
const ORPHANED: &str =
    "(owner_instance IS NULL OR owner_instance NOT IN (SELECT id FROM server_instances WHERE heartbeat_at > $1))";

impl Database {
    /// Records this server instance. Earlier rows for the same host are its previous runs (one
    /// server per host, and Compose gives each container its own host name); their work stopped
    /// with them, so they are removed and that work becomes reclaimable at once.
    pub fn register_instance(&self, id: &str, host: &str) -> Result<(), String> {
        let now = now_millis();
        let tx = self.conn.unchecked_transaction().map_err(|error| error.to_string())?;
        tx.execute("DELETE FROM server_instances WHERE host = $1", [host])
            .map_err(|error| error.to_string())?;
        tx.execute(
            "INSERT INTO server_instances (id, host, started_at, heartbeat_at) VALUES ($1, $2, $3, $3)",
            params![id, host, now],
        )
        .map_err(|error| error.to_string())?;
        tx.commit().map_err(|error| error.to_string())
    }

    /// Refreshes this instance's heartbeat; registers it again if another instance reclaimed it
    /// after a long pause (its reviews then find their runs cancelled and stop).
    pub fn heartbeat_instance(&self, id: &str, host: &str) -> Result<(), String> {
        let now = now_millis();
        self.conn
            .execute(
                "INSERT INTO server_instances (id, host, started_at, heartbeat_at) VALUES ($1, $2, $3, $3)
                 ON CONFLICT (id) DO UPDATE SET heartbeat_at = excluded.heartbeat_at",
                params![id, host, now],
            )
            .map(|_| ())
            .map_err(|error| error.to_string())
    }

    /// Reviews and index runs whose instance is gone are interrupted: reviews are cancelled and
    /// their PRs need a manual review again; index runs are cancelled. Stopped instances are
    /// forgotten. Returns how many reviews were interrupted.
    pub fn reclaim_orphaned_work(&self) -> Result<usize, String> {
        let now = now_millis();
        let live_after = now - INSTANCE_TIMEOUT_MS;
        let tx = self.conn.unchecked_transaction().map_err(|error| error.to_string())?;
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
}
