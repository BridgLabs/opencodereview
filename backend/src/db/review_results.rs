use super::*;

/// The stable first-line ownership marker shared by every clean result comment
/// this installation posts on a pull request.
pub(crate) fn result_marker_key(full_name: &str, number: i64, installation_id: &str) -> String {
    format!(
        "<!-- ocr-result-key:v1 repo={} pr={} installation={} -->",
        full_name, number, installation_id
    )
}

/// Current-run eligibility for automatic republication of a failed clean
/// result, shared by `failed_clean_results_due` and
/// `claim_due_failed_clean_result` so the SELECT's due-set and the claim's
/// recheck can never drift. References the result row as `result`.
const FAILED_CLEAN_RESULT_ELIGIBLE: &str = "result.publication_status = 'failed'
               AND result.effective_state = 'clean'
               AND result.updated_at <= $2
               AND EXISTS (SELECT 1 FROM pull_requests pr
                           JOIN review_runs run ON run.id = result.run_id AND run.pr_id = result.pr_id
                           WHERE pr.id = result.pr_id AND LOWER(pr.state) = 'open'
                             AND pr.head_sha = result.observed_head_sha
                             AND pr.force_full_next_review = 0 AND pr.reprocess_next_review = 0
                             AND pr.review_scope_revision = run.review_scope_revision
                             AND pr.review_lane != 'queued')
               AND NOT EXISTS (SELECT 1 FROM review_runs newer WHERE newer.pr_id = result.pr_id AND newer.id > result.run_id)";

impl Database {
    /// Remember the review OpenCodeReview itself posted (APPROVE/REQUEST_CHANGES), so we can
    /// later dismiss OUR review on new commits — never touching the user's own web reviews.
    pub fn set_ocr_review(&self, pr_id: i64, review_id: i64, event: &str) -> Result<(), String> {
        self.conn
            .execute(
                "UPDATE pull_requests SET ocr_review_id = $1, ocr_review_event = $2, updated_at = $3 WHERE id = $4",
                params![review_id, event, now_millis(), pr_id],
            )
            .map_err(|error| error.to_string())?;
        Ok(())
    }

    pub fn installation_id(&self) -> Result<String, String> {
        self.setting("installation_id")?
            .filter(|value| !value.is_empty())
            .ok_or_else(|| "installation identity is missing".to_string())
    }

    pub fn review_result(&self, pr_id: i64) -> Result<Option<ReviewResult>, String> {
        let installation_id = self.installation_id()?;
        match self.conn.query_row(
            "SELECT id, pr_id, installation_id, marker_key, github_comment_id, run_id, actionable_run_id,
                    observed_head_sha, reviewed_head_sha, prior_result_state,
                    prior_result_head_sha, observation, effective_state, publication_status,
                    body, body_hash, github_review_id, github_review_url, reviewer_version,
                    skill_version, error_message, created_at, updated_at, posted_at
             FROM review_results WHERE pr_id = $1 AND installation_id = $2",
            params![pr_id, installation_id],
            review_result_from_row,
        ) {
            Ok(result) => Ok(Some(result)),
            Err(pg::Error::QueryReturnedNoRows) => Ok(None),
            Err(error) => Err(error.to_string()),
        }
    }

    pub fn latest_legacy_review_outcome(
        &self,
        pr_id: i64,
        excluded_run_id: i64,
    ) -> Result<Option<LegacyReviewOutcome>, String> {
        match self.conn.query_row(
            "SELECT id, status, decision, head_sha, reviewed_head_sha, comments_json, review_body
             FROM review_runs
             WHERE pr_id = $1 AND id != $2 AND review_body IS NOT NULL
             ORDER BY id DESC LIMIT 1",
            params![pr_id, excluded_run_id],
            |row| {
                Ok(LegacyReviewOutcome {
                    run_id: row.get(0)?,
                    status: row.get(1)?,
                    decision: row.get(2)?,
                    head_sha: row.get(3)?,
                    reviewed_head_sha: row.get(4)?,
                    comments_json: row.get(5)?,
                    review_body: row.get(6)?,
                })
            },
        ) {
            Ok(outcome) => Ok(Some(outcome)),
            Err(pg::Error::QueryReturnedNoRows) => Ok(None),
            Err(error) => Err(error.to_string()),
        }
    }

    pub fn reserve_review_result(&self, input: &ReviewResultInput<'_>) -> Result<ReviewResult, String> {
        let installation_id = self.installation_id()?;
        let (_, full_name, number, _) = self.pr_review_coords(input.pr_id)?;
        let marker_key = result_marker_key(&full_name, number, &installation_id);
        let body_hash = input.body.map(hash_text);
        let now = now_millis();
        let changed = self
            .conn
            .execute(
                "INSERT INTO review_results (
               pr_id, installation_id, marker_key, run_id, actionable_run_id, observed_head_sha,
               reviewed_head_sha, prior_result_state, prior_result_head_sha,
               observation, effective_state, publication_status, body, body_hash,
               github_review_id, github_review_url, reviewer_version, skill_version,
               created_at, updated_at
             ) VALUES (
               $1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14,
               $15, $16, $17, $18, $19, $19
             )
             ON CONFLICT(pr_id, installation_id) DO UPDATE SET
               run_id = excluded.run_id,
               actionable_run_id = excluded.actionable_run_id,
               observed_head_sha = excluded.observed_head_sha,
               reviewed_head_sha = excluded.reviewed_head_sha,
               prior_result_state = excluded.prior_result_state,
               prior_result_head_sha = excluded.prior_result_head_sha,
               observation = excluded.observation,
               effective_state = excluded.effective_state,
               publication_status = excluded.publication_status,
               body = excluded.body,
               body_hash = excluded.body_hash,
               github_review_id = COALESCE(excluded.github_review_id, review_results.github_review_id),
               github_review_url = COALESCE(excluded.github_review_url, review_results.github_review_url),
               reviewer_version = excluded.reviewer_version,
               skill_version = excluded.skill_version,
               error_message = NULL,
               updated_at = excluded.updated_at
             WHERE review_results.publication_status != 'posting'",
                params![
                    input.pr_id,
                    installation_id,
                    marker_key,
                    input.run_id,
                    input.actionable_run_id,
                    input.observed_head_sha,
                    input.reviewed_head_sha,
                    input.prior_result_state,
                    input.prior_result_head_sha,
                    input.observation,
                    input.effective_state,
                    input.publication_status,
                    input.body,
                    body_hash,
                    input.github_review_id,
                    input.github_review_url,
                    input.reviewer_version,
                    input.skill_version,
                    now,
                ],
            )
            .map_err(|error| error.to_string())?;
        if changed != 1 {
            return Err("a result comment write is already in progress for this PR".to_string());
        }
        self.resolve_pending_for_pr("review_result_failed", input.pr_id)?;
        self.review_result(input.pr_id)?
            .ok_or_else(|| "could not reload reserved review result".to_string())
    }

    pub fn begin_review_result_publication(
        &self,
        result_id: i64,
        observed_head_sha: &str,
        body_hash: Option<&str>,
    ) -> Result<bool, String> {
        let changed = self
            .conn
            .execute(
                "UPDATE review_results SET publication_status = 'posting', error_message = NULL,
                    updated_at = $1, owner_instance = $5
             WHERE id = $2 AND observed_head_sha = $3
               AND body_hash IS NOT DISTINCT FROM $4
               AND publication_status IN ('pending', 'failed')",
                params![
                    now_millis(),
                    result_id,
                    observed_head_sha,
                    body_hash,
                    self.db.instance_id()
                ],
            )
            .map_err(|error| error.to_string())?;
        Ok(changed == 1)
    }

    pub fn finish_review_result_publication(&self, result_id: i64, github_comment_id: i64) -> Result<(), String> {
        let now = now_millis();
        self.conn
            .execute(
                "UPDATE review_results SET github_comment_id = $1, publication_status = 'posted',
                    error_message = NULL, posted_at = $2, updated_at = $2
             WHERE id = $3 AND publication_status = 'posting'",
                params![github_comment_id, now, result_id],
            )
            .map_err(|error| error.to_string())?;
        Ok(())
    }

    pub fn fail_review_result_publication(&self, result_id: i64, error: &str) -> Result<bool, String> {
        let pr_id: i64 = self
            .conn
            .query_row(
                "SELECT pr_id FROM review_results WHERE id = $1",
                params![result_id],
                |row| row.get(0),
            )
            .map_err(|db_error| db_error.to_string())?;
        let run_is_undecided = self
            .conn
            .query_row(
                "SELECT EXISTS(
                   SELECT 1 FROM review_results result
                   JOIN review_runs run ON run.id = result.run_id
                   WHERE result.id = $1 AND run.decided_at IS NULL
                 )",
                params![result_id],
                |row| row.get::<_, i64>(0),
            )
            .map_err(|db_error| db_error.to_string())?
            != 0;
        self.conn
            .execute(
                "UPDATE review_results SET publication_status = 'failed', error_message = $1,
                    updated_at = $2 WHERE id = $3",
                params![error, now_millis(), result_id],
            )
            .map_err(|db_error| db_error.to_string())?;
        self.conn
            .execute(
                "UPDATE review_runs SET status = 'result_failed', error_message = $1, updated_at = $2
             WHERE id = (SELECT run_id FROM review_results WHERE id = $3)
               AND (SELECT effective_state FROM review_results WHERE id = $3) = 'clean'
               AND decided_at IS NULL",
                params![error, now_millis(), result_id],
            )
            .map_err(|db_error| db_error.to_string())?;
        self.resolve_pending_for_pr("review_ready", pr_id)?;
        Ok(run_is_undecided)
    }

    pub fn review_run_is_decided(&self, run_id: i64) -> Result<bool, String> {
        self.conn
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM review_runs WHERE id = $1 AND decided_at IS NOT NULL)",
                params![run_id],
                |row| row.get::<_, i64>(0),
            )
            .map(|value| value != 0)
            .map_err(|error| error.to_string())
    }

    pub fn supersede_review_result(&self, result_id: i64, reason: &str) -> Result<(), String> {
        self.conn
            .execute(
                "UPDATE review_results SET publication_status = 'superseded', error_message = $1,
                    updated_at = $2 WHERE id = $3",
                params![reason, now_millis(), result_id],
            )
            .map_err(|error| error.to_string())?;
        Ok(())
    }

    pub fn supersede_review_result_after_stale_head(
        &self,
        result_id: i64,
        pr_id: i64,
        reason: &str,
    ) -> Result<(), String> {
        self.supersede_review_result(result_id, reason)?;
        self.resolve_pending_for_pr("review_result_failed", pr_id)
    }

    pub fn disable_review_result_publication(&self, result_id: i64) -> Result<(), String> {
        self.conn
            .execute(
                "UPDATE review_results SET publication_status = 'disabled', error_message = NULL,
                    updated_at = $1 WHERE id = $2 AND publication_status != 'posted'",
                params![now_millis(), result_id],
            )
            .map_err(|error| error.to_string())?;
        Ok(())
    }

    /// Takes over the publications left `posting` by an instance that has stopped (or predates
    /// instance tracking), so the caller posts them again and no other instance does meanwhile.
    pub fn claim_abandoned_review_results(&self) -> Result<Vec<ReviewResult>, String> {
        let mut stmt = self
            .conn
            .prepare(
                "UPDATE review_results SET owner_instance = $2
             WHERE publication_status = 'posting'
               AND (owner_instance IS NULL
                    OR owner_instance NOT IN (SELECT id FROM server_instances WHERE heartbeat_at > $1))
             RETURNING id, pr_id, installation_id, marker_key, github_comment_id, run_id, actionable_run_id,
                    observed_head_sha, reviewed_head_sha, prior_result_state,
                    prior_result_head_sha, observation, effective_state, publication_status,
                    body, body_hash, github_review_id, github_review_url, reviewer_version,
                    skill_version, error_message, created_at, updated_at, posted_at",
            )
            .map_err(|error| error.to_string())?;
        let rows = stmt
            .query_map(
                params![self.instance_live_after()?, self.db.instance_id()],
                review_result_from_row,
            )
            .map_err(|error| error.to_string())?;
        let mut results = rows.collect::<Result<Vec<_>, _>>().map_err(|error| error.to_string())?;
        results.sort_by_key(|result| result.updated_at);
        Ok(results)
    }

    /// Clean-result publications that failed at least `failed_before` millis
    /// ago on still-open PRs, oldest first. Only results for the PR's
    /// still-current run at the current head qualify for automatic retry.
    pub(crate) fn failed_clean_results_due(&self, failed_before: i64) -> Result<Vec<ReviewResult>, String> {
        let installation_id = self.installation_id()?;
        let mut stmt = self
            .conn
            .prepare(&format!(
                "SELECT result.id, result.pr_id, result.installation_id, result.marker_key,
                    result.github_comment_id, result.run_id, result.actionable_run_id,
                    result.observed_head_sha, result.reviewed_head_sha, result.prior_result_state,
                    result.prior_result_head_sha, result.observation, result.effective_state,
                    result.publication_status, result.body, result.body_hash,
                    result.github_review_id, result.github_review_url, result.reviewer_version,
                    result.skill_version, result.error_message, result.created_at,
                    result.updated_at, result.posted_at
             FROM review_results result
             WHERE result.installation_id = $1
               AND {FAILED_CLEAN_RESULT_ELIGIBLE}
             ORDER BY result.updated_at",
            ))
            .map_err(|error| error.to_string())?;
        let rows = stmt
            .query_map(params![installation_id, failed_before], review_result_from_row)
            .map_err(|error| error.to_string())?;
        rows.collect::<Result<Vec<_>, _>>().map_err(|error| error.to_string())
    }

    pub fn retry_review_result(&self, pr_id: i64) -> Result<ReviewResult, String> {
        if self.settings()?.review_result_mode != "result_comment" {
            return Err("enable result comments before retrying this publication".to_string());
        }
        let installation_id = self.installation_id()?;
        let changed = self
            .conn
            .execute(
                "UPDATE review_results SET publication_status = 'pending', error_message = NULL,
                    updated_at = $1
             WHERE pr_id = $2 AND installation_id = $3 AND publication_status = 'failed'",
                params![now_millis(), pr_id, installation_id],
            )
            .map_err(|error| error.to_string())?;
        if changed != 1 {
            return Err("no failed result publication is available to retry".to_string());
        }
        self.review_result(pr_id)?
            .ok_or_else(|| "could not reload review result".to_string())
    }

    /// Claim for the automatic retry pass only: atomically flips the row back
    /// to `pending` while every current-run eligibility predicate still holds,
    /// so a rerun or re-review requested mid-pass can no longer publish a
    /// stale clean comment. The result-comment mode is rechecked in the same
    /// UPDATE, so switching to Silent mid-pass leaves the row `failed` instead
    /// of claiming it into an unrecoverable `disabled` state. Manual "Retry"
    /// keeps `retry_review_result`. Returns the reloaded result when the
    /// claim won the row.
    pub(crate) fn claim_due_failed_clean_result(
        &self,
        result: &ReviewResult,
        failed_before: i64,
    ) -> Result<Option<ReviewResult>, String> {
        let installation_id = self.installation_id()?;
        let changed = self
            .conn
            .execute(
                &format!(
                    "UPDATE review_results AS result SET publication_status = 'pending',
                        error_message = NULL, updated_at = $3
                     WHERE result.installation_id = $1
                       AND {FAILED_CLEAN_RESULT_ELIGIBLE}
                       AND EXISTS (SELECT 1 FROM settings WHERE key = 'review_result_mode' AND value = 'result_comment')
                       AND result.id = $4
                       AND result.run_id IS NOT DISTINCT FROM $5",
                ),
                params![installation_id, failed_before, now_millis(), result.id, result.run_id],
            )
            .map_err(|error| error.to_string())?;
        if changed != 1 {
            return Ok(None);
        }
        self.review_result(result.pr_id)
    }

    pub fn interrupted_review_posts(&self) -> Result<Vec<InterruptedReviewPost>, String> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT rr.id, rr.pr_id, r.full_name, pr.number, rr.head_sha,
                    COALESCE(rr.event, 'COMMENT'), COALESCE(rr.decision, 'comment'),
                    COALESCE(rr.outcome, '')
             FROM review_runs rr
             JOIN pull_requests pr ON pr.id = rr.pr_id
             JOIN repos r ON r.id = pr.repo_id
             WHERE rr.status = 'posting'
               AND (rr.owner_instance IS NULL
                    OR rr.owner_instance NOT IN (SELECT id FROM server_instances WHERE heartbeat_at > $1))
             ORDER BY rr.updated_at",
            )
            .map_err(|error| error.to_string())?;
        let live_after = self.instance_live_after()?;
        let rows = stmt
            .query_map([live_after], |row| {
                let outcome: String = row.get(7)?;
                Ok(InterruptedReviewPost {
                    run_id: row.get(0)?,
                    pr_id: row.get(1)?,
                    full_name: row.get(2)?,
                    number: row.get(3)?,
                    head_sha: row.get(4)?,
                    event: row.get(5)?,
                    decision: row.get(6)?,
                    actor: if outcome.contains("(auto)") {
                        "auto".to_string()
                    } else {
                        "you".to_string()
                    },
                })
            })
            .map_err(|error| error.to_string())?;
        rows.collect::<Result<Vec<_>, _>>().map_err(|error| error.to_string())
    }
}

#[cfg(test)]
#[path = "review_results_tests.rs"]
mod tests;
