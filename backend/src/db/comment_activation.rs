use super::*;

impl Database {
    pub(crate) fn activated_open_pr_numbers(&self, repo_id: i64) -> Result<Vec<i64>, String> {
        let mut stmt = self.conn.prepare("SELECT p.number FROM pull_requests p JOIN pr_comment_activations a ON a.pr_id = p.id WHERE p.repo_id = ?1 AND p.state = 'open'").map_err(|e| e.to_string())?;
        let rows = stmt.query_map([repo_id], |r| r.get(0)).map_err(|e| e.to_string())?;
        rows.collect::<Result<Vec<_>, _>>().map_err(|e| e.to_string())
    }

    pub(crate) fn comment_queue_requires_eligibility(&self, pr_id: i64) -> Result<bool, String> {
        self.conn
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM pr_comment_activations WHERE pr_id = ?1)",
                [pr_id],
                |r| r.get(0),
            )
            .map_err(|e| e.to_string())
    }

    pub(crate) fn comment_scan_cursor(&self, repo_id: i64) -> Result<(i64, String), String> {
        self.conn.query_row(
            "SELECT enabled_at, strftime('%Y-%m-%dT%H:%M:%SZ', MAX(enabled_at, checkpoint - 60000) / 1000, 'unixepoch') FROM comment_scan_cursors WHERE repo_id = ?1",
            [repo_id], |row| Ok((row.get(0)?, row.get(1)?)),
        ).map_err(|e| e.to_string())
    }

    pub(crate) fn finish_comment_scan(&self, repo_id: i64, started_at: i64) -> Result<(), String> {
        self.conn
            .execute(
                "UPDATE comment_scan_cursors SET checkpoint = MAX(checkpoint, ?2) WHERE repo_id = ?1",
                params![repo_id, started_at],
            )
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    pub(crate) fn comment_activation_seen(&self, repo_id: i64, comment_id: i64) -> Result<bool, String> {
        self.conn
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM comment_activation_receipts WHERE repo_id = ?1 AND comment_id = ?2)",
                params![repo_id, comment_id],
                |r| r.get(0),
            )
            .map_err(|e| e.to_string())
    }

    pub(crate) fn pr_comment_activated(&self, repo_id: i64, number: i64) -> Result<bool, String> {
        self.conn.query_row("SELECT EXISTS(SELECT 1 FROM pr_comment_activations a JOIN pull_requests p ON p.id = a.pr_id WHERE p.repo_id = ?1 AND p.number = ?2)", params![repo_id, number], |r| r.get(0)).map_err(|e| e.to_string())
    }

    pub(crate) fn consume_activation_command(
        &self,
        repo_id: i64,
        enabled_at: i64,
        comment_id: i64,
        activation: Option<(i64, &str, &str, &str)>,
    ) -> Result<bool, String> {
        let tx = self.conn.unchecked_transaction().map_err(|e| e.to_string())?;
        let current: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM comment_scan_cursors c JOIN subscriptions s ON s.repo_id = c.repo_id WHERE c.repo_id = ?1 AND c.enabled_at = ?2 AND s.enabled = 1)", params![repo_id, enabled_at], |r| r.get(0)).map_err(|e| e.to_string())?;
        if !current {
            return Err("Repository monitoring changed during command scan".into());
        }
        let fresh = tx
            .execute(
                "INSERT OR IGNORE INTO comment_activation_receipts VALUES (?1, ?2)",
                params![repo_id, comment_id],
            )
            .map_err(|e| e.to_string())?
            == 1;
        let mut admitted = false;
        // Candidates already passed the title/size filters and author check.
        // An unassigned non-draft PR may still be in auto_skipped before admission.
        if let Some((pr_id, head, marker, body)) = activation.filter(|_| fresh) {
            admitted = tx
                .execute(
                    "INSERT OR IGNORE INTO pr_comment_activations(pr_id, command_id, head_sha, marker, body)
                 SELECT p.id, ?2, ?3, ?4, ?5 FROM pull_requests p JOIN subscriptions s ON s.repo_id = p.repo_id
                 WHERE p.id = ?1 AND p.repo_id = ?6 AND s.enabled = 1 AND p.state = 'open' AND p.head_sha = ?3
                 AND p.review_lane IN ('draft', 'open', 'manual_required', 'auto_skipped')
                 AND (p.last_decision_sha IS NULL OR p.last_decision_sha != ?3)
                 AND NOT EXISTS(SELECT 1 FROM review_runs r WHERE r.pr_id = p.id AND r.head_sha = ?3)",
                    params![pr_id, comment_id, head, marker, body, repo_id],
                )
                .map_err(|e| e.to_string())?
                == 1;
            if admitted {
                tx.execute("UPDATE pull_requests SET review_lane = 'queued', status_reason = 'Queued by PR author', updated_at = ?2 WHERE id = ?1", params![pr_id, now_millis()]).map_err(|e| e.to_string())?;
                tx.execute("UPDATE pending_actions SET status = 'resolved', resolved_at = ?2 WHERE pr_id = ?1 AND type = 'manual_review_needed' AND resolved_at IS NULL", params![pr_id, now_millis()]).map_err(|e| e.to_string())?;
                self.acknowledge_review_cycle(pr_id, false, None)?;
            } else {
                // A fresh author command on a PR already queued or reviewing at
                // the same head is a new explicit request: bump the
                // acknowledgement generation so the comment is reposted.
                let active: bool = tx.query_row(
                    "SELECT EXISTS(SELECT 1 FROM pull_requests WHERE id = ?1 AND head_sha = ?2 AND review_lane IN ('queued', 'reviewing'))",
                    params![pr_id, head],
                    |r| r.get(0),
                ).map_err(|e| e.to_string())?;
                if active {
                    self.acknowledge_review_cycle(pr_id, true, None)?;
                }
            }
        }
        tx.commit().map_err(|e| e.to_string())?;
        Ok(admitted)
    }
}
