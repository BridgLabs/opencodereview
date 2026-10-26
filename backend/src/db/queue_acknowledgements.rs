use super::*;

/// A failed acknowledgement POST can mean GitHub accepted the comment
/// anyway; a retirement scans this long before trusting an empty list.
pub(crate) const QUEUE_POST_SETTLE_MS: i64 = 10 * 60 * 1000;

pub(crate) struct QueueAcknowledgement {
    pub pr_id: i64,
    pub number: i64,
    pub full_name: String,
    pub generation: i64,
    pub head_sha: String,
    pub status: String,
}

impl Database {
    /// The PR's review scope revision while a full or reprocess intent is still
    /// stored: the revision an acknowledged cycle was recorded under before the
    /// queue replay bumps it again.
    pub(crate) fn stored_review_intent_revision(&self, pr_id: i64) -> Result<Option<i64>, String> {
        self.conn
            .query_row(
                "SELECT review_scope_revision FROM pull_requests WHERE id = $1 AND (force_full_next_review <> 0 OR reprocess_next_review <> 0)",
                params![pr_id],
                |row| row.get(0),
            )
            .optional()
            .map_err(|e| e.to_string())
    }

    /// Records that the PR's current review cycle should carry a "queued for
    /// review" acknowledgement. Without `refresh` an existing row for the same
    /// head and scope revision is left alone whatever its status; otherwise the
    /// row is (re)created as a new generation pending publication. When a
    /// replayed stored intent bumps the scope revision once more, `carried_from`
    /// carries the existing row's revision forward so a drain replay does not
    /// start a new generation.
    pub(crate) fn acknowledge_review_cycle(
        &self,
        pr_id: i64,
        refresh: bool,
        carried_from: Option<i64>,
    ) -> Result<(), String> {
        // Under the PR's lock: two concurrent calls must not both see no row and both bump the
        // generation, which would post the comment twice.
        let tx = self.conn.unchecked_transaction().map_err(|e| e.to_string())?;
        self.lock_pull_request(pr_id)?;
        self.acknowledge_review_cycle_locked(pr_id, refresh, carried_from)?;
        tx.commit().map_err(|e| e.to_string())
    }

    fn acknowledge_review_cycle_locked(
        &self,
        pr_id: i64,
        refresh: bool,
        carried_from: Option<i64>,
    ) -> Result<(), String> {
        let (head_sha, scope_revision): (String, i64) = self
            .conn
            .query_row(
                "SELECT head_sha, review_scope_revision FROM pull_requests WHERE id = $1",
                params![pr_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .map_err(|e| e.to_string())?;
        if !refresh {
            if let Some(previous) = carried_from {
                self.conn
                    .execute(
                        "UPDATE queue_acknowledgements SET scope_revision = $3
                         WHERE pr_id = $1 AND head_sha = $2 AND scope_revision = $4",
                        params![pr_id, head_sha, scope_revision, previous],
                    )
                    .map_err(|e| e.to_string())?;
            }
            let current: bool = self
                .conn
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM queue_acknowledgements WHERE pr_id = $1 AND head_sha = $2 AND scope_revision = $3)",
                    params![pr_id, head_sha, scope_revision],
                    |row| row.get(0),
                )
                .map_err(|e| e.to_string())?;
            if current {
                return Ok(());
            }
        }
        self.conn
            .execute(
                "INSERT INTO queue_acknowledgements (pr_id, generation, head_sha, scope_revision, status)
                 VALUES ($1, 1, $2, $3, 'pending')
                 ON CONFLICT(pr_id) DO UPDATE SET
                   generation = queue_acknowledgements.generation + 1,
                   head_sha = excluded.head_sha,
                   scope_revision = excluded.scope_revision,
                   status = 'pending',
                   comment_id = NULL,
                   attempts = 0,
                   next_attempt_at = 0,
                   last_error = NULL",
                params![pr_id, head_sha, scope_revision],
            )
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    /// Marks this head's acknowledgement for deletion. Only `pending` and
    /// `posted` rows retire: a comment whose review result never reached
    /// GitHub stays up. A passed `run_id` retires only when it is still the
    /// PR's latest run with no stored rerun intent, so a newer cycle's
    /// acknowledgement is never retired.
    pub(crate) fn retire_queue_acknowledgement(
        &self,
        pr_id: i64,
        head_sha: &str,
        run_id: Option<i64>,
    ) -> Result<(), String> {
        self.conn
            .execute(
                "UPDATE queue_acknowledgements SET status = 'retiring', attempts = 0, next_attempt_at = 0
                 WHERE pr_id = $1 AND head_sha = $2 AND status IN ('pending', 'posted')
                 AND ($3::BIGINT IS NULL OR (
                   NOT EXISTS (SELECT 1 FROM review_runs newer WHERE newer.pr_id = $1 AND newer.id > $3)
                   AND NOT EXISTS (SELECT 1 FROM pull_requests p WHERE p.id = $1 AND (p.force_full_next_review <> 0 OR p.reprocess_next_review <> 0))))",
                params![pr_id, head_sha, run_id],
            )
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    /// Marks for deletion every acknowledgement whose head's review already finished: the PR's
    /// latest run reviewed that head and was published, and no rerun is stored. Returns how many.
    pub(crate) fn retire_finished_queue_acknowledgements(&self) -> Result<usize, String> {
        self.conn
            .execute(
                "UPDATE queue_acknowledgements SET status = 'retiring', attempts = 0, next_attempt_at = 0
                 WHERE status IN ('pending', 'posted')
                 AND EXISTS (
                   SELECT 1 FROM review_runs r
                   WHERE r.pr_id = queue_acknowledgements.pr_id
                     AND r.head_sha = queue_acknowledgements.head_sha
                     AND r.status IN ('posted', 'approved')
                     AND r.id = (SELECT MAX(id) FROM review_runs latest WHERE latest.pr_id = r.pr_id))
                 AND NOT EXISTS (
                   SELECT 1 FROM pull_requests p WHERE p.id = queue_acknowledgements.pr_id
                     AND (p.force_full_next_review <> 0 OR p.reprocess_next_review <> 0))",
                [],
            )
            .map_err(|e| e.to_string())
    }

    /// `pending` rows are due only while the PR is open; `retiring` rows are
    /// due regardless of PR state so the comment still gets deleted.
    pub(crate) fn due_queue_acknowledgements(&self, now: i64) -> Result<Vec<QueueAcknowledgement>, String> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT a.pr_id, p.number, r.full_name, a.generation, a.head_sha, a.status
                 FROM queue_acknowledgements a
                 JOIN pull_requests p ON p.id = a.pr_id
                 JOIN repos r ON r.id = p.repo_id
                 WHERE a.next_attempt_at <= $1
                   AND ((a.status = 'pending' AND LOWER(p.state) = 'open') OR a.status = 'retiring')
                 ORDER BY a.pr_id",
            )
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map(params![now], |row| {
                Ok(QueueAcknowledgement {
                    pr_id: row.get(0)?,
                    number: row.get(1)?,
                    full_name: row.get(2)?,
                    generation: row.get(3)?,
                    head_sha: row.get(4)?,
                    status: row.get(5)?,
                })
            })
            .map_err(|e| e.to_string())?;
        rows.collect::<Result<Vec<_>, _>>().map_err(|e| e.to_string())
    }

    /// Records the outcome of one reconcile attempt. The generation and status
    /// guard keeps a newer generation or a retirement that landed during the
    /// GitHub call from being overwritten. A failed `pending` attempt marks the
    /// post as uncertain, and a retirement then keeps `retiring` until the
    /// settle window has passed in case GitHub accepted the comment late.
    pub(crate) fn record_queue_acknowledgement(
        &self,
        pr_id: i64,
        generation: i64,
        expected_status: &str,
        outcome: Result<Option<i64>, &str>,
    ) -> Result<(), String> {
        let now = now_millis();
        match outcome {
            Ok(comment_id) => {
                self.conn
                    .execute(
                        "UPDATE queue_acknowledgements SET
                       status = CASE
                         WHEN $3 = 'pending' THEN 'posted'
                         WHEN post_uncertain_at IS NULL OR post_uncertain_at <= $5::BIGINT - $6::BIGINT THEN 'retired'
                         ELSE 'retiring' END,
                       comment_id = CASE
                         WHEN $3 = 'pending' OR post_uncertain_at IS NULL OR post_uncertain_at <= $5 - $6
                         THEN COALESCE($4, comment_id) ELSE comment_id END,
                       last_error = NULL,
                       next_attempt_at = CASE
                         WHEN $3 = 'pending' OR post_uncertain_at IS NULL OR post_uncertain_at <= $5 - $6 THEN 0
                         ELSE post_uncertain_at + $6 END
                     WHERE pr_id = $1 AND generation = $2 AND status = $3",
                        params![
                            pr_id,
                            generation,
                            expected_status,
                            comment_id,
                            now,
                            QUEUE_POST_SETTLE_MS
                        ],
                    )
                    .map_err(|e| e.to_string())?;
            }
            Err(error) => {
                self.conn
                    .execute(
                        "UPDATE queue_acknowledgements SET attempts = attempts + 1, last_error = $4,
                     post_uncertain_at = CASE WHEN $3 = 'pending' THEN $5 ELSE post_uncertain_at END,
                     next_attempt_at = $5 + LEAST(3600000, 60000 * (1::BIGINT << LEAST(attempts, 6)::INT))
                     WHERE pr_id = $1 AND generation = $2 AND status = $3",
                        params![pr_id, generation, expected_status, error, now],
                    )
                    .map_err(|e| e.to_string())?;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "queue_acknowledgements_tests.rs"]
mod tests;
