use super::*;

impl Database {
    pub(crate) fn review_coverage_snapshot(
        &self,
        pr_id: i64,
        head_sha: &str,
    ) -> Result<Option<crate::review::scope::CoverageSnapshot>, String> {
        let row = self.conn.query_row(
            "SELECT c.run_id,c.base_ref,c.base_sha,c.head_sha,c.merge_base_sha,c.changed_files_json,c.diff_hash,c.diff_text
             FROM review_coverage c JOIN review_runs rr ON rr.id=c.run_id AND rr.pr_id=c.pr_id
             WHERE c.pr_id=?1 AND c.head_sha=?2 AND c.status='valid'
               AND COALESCE((SELECT json_array_length(a.audit_json, '$.evidenceWarnings') FROM review_finding_audits a WHERE a.run_id=rr.id),0)=0
               AND rr.review_body IS NOT NULL AND rr.status NOT IN ('failed','cancelled','pending','running')
             ORDER BY c.run_id DESC,c.id DESC LIMIT 1",
            params![pr_id, head_sha],
            |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?, row.get::<_, String>(2)?, row.get::<_, String>(3)?, row.get::<_, String>(4)?, row.get::<_, String>(5)?, row.get::<_, String>(6)?, row.get::<_, String>(7)?)),
        );
        match row {
            Ok((run_id, base_ref, base_sha, head_sha, merge_base_sha, files, diff_hash, diff_text)) => {
                Ok(Some(crate::review::scope::CoverageSnapshot {
                    run_id,
                    scope: crate::review::scope::ReviewScope {
                        head_sha,
                        base_ref,
                        base_sha,
                    },
                    merge_base_sha,
                    changed_files: serde_json::from_str(&files)
                        .map_err(|_| "Persisted review coverage file inventory is invalid")?,
                    diff_hash,
                    diff_text,
                }))
            }
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(error) => Err(error.to_string()),
        }
    }

    pub(crate) fn stacked_review_coverage_sources(
        &self,
        repo_id: i64,
        pr_id: i64,
    ) -> Result<Vec<(i64, i64, String)>, String> {
        let mut statement = self.conn.prepare(
            "SELECT c.run_id,c.pr_id,c.head_sha FROM review_coverage c
             JOIN review_runs rr ON rr.id=c.run_id AND rr.pr_id=c.pr_id AND rr.head_sha=c.head_sha
             JOIN pull_requests source ON source.id=c.pr_id AND source.repo_id=c.repo_id
             JOIN pull_requests target ON target.id=?2 AND target.repo_id=c.repo_id
             WHERE c.repo_id=?1 AND c.pr_id<>?2 AND c.base_ref=target.head_ref AND c.status='valid'
               AND COALESCE((SELECT json_array_length(a.audit_json, '$.evidenceWarnings') FROM review_finding_audits a WHERE a.run_id=rr.id),0)=0
               AND rr.review_body IS NOT NULL AND rr.finished_at IS NOT NULL
               AND rr.status NOT IN ('failed','cancelled','pending','running')
             ORDER BY c.run_id DESC LIMIT 100",
        ).map_err(|error| error.to_string())?;
        let rows = statement
            .query_map(params![repo_id, pr_id], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?))
            })
            .map_err(|error| error.to_string())?;
        rows.collect::<Result<Vec<_>, _>>().map_err(|error| error.to_string())
    }

    pub(crate) fn historical_review_coverage_for_head(
        &self,
        repo_id: i64,
        pr_id: i64,
        head: &str,
        backfilled: bool,
    ) -> Result<Option<HistoricalReviewCoverageCandidate>, String> {
        self.conn.query_row(
            "SELECT rr.id,rr.pr_id,rr.stacked_base_ref,rr.reviewed_base_sha,rr.reviewed_head_sha,
                    (SELECT manifest_json FROM review_context_snapshots s WHERE s.review_run_id=rr.id AND s.pr_id=rr.pr_id AND s.repo_id=pr.repo_id ORDER BY s.id DESC LIMIT 1)
             FROM review_runs rr JOIN pull_requests pr ON pr.id=rr.pr_id
             WHERE pr.repo_id=?1 AND rr.pr_id=?2 AND rr.head_sha=?3 AND rr.reviewed_head_sha=?3
               AND COALESCE((SELECT json_array_length(a.audit_json, '$.evidenceWarnings') FROM review_finding_audits a WHERE a.run_id=rr.id),0)=0
               AND rr.review_body IS NOT NULL AND rr.finished_at IS NOT NULL
               AND rr.status NOT IN ('failed','cancelled','pending','running')
               AND rr.stacked_base_ref IS NOT NULL AND rr.reviewed_base_sha IS NOT NULL
               AND (?4 OR NOT EXISTS (SELECT 1 FROM review_coverage c WHERE c.run_id=rr.id))
             ORDER BY rr.id DESC LIMIT 1",
            params![repo_id, pr_id, head, backfilled],
            |row| Ok(HistoricalReviewCoverageCandidate { run_id: row.get(0)?, pr_id: row.get(1)?, base_ref: row.get(2)?, base_sha: row.get(3)?, head_sha: row.get(4)?, manifest_json: row.get(5)? }),
        ).optional().map_err(|error| error.to_string())
    }

    pub(crate) fn is_legacy_coverage_backfill(&self, run_id: i64) -> Result<bool, String> {
        self.conn.query_row(
            "SELECT COUNT(*)=1 AND COALESCE(MIN(CASE WHEN status='valid' AND json_valid(source_json)
                 THEN json_extract(source_json,'$.backfilled')=1 AND json_extract(source_json,'$.sourceRunId')=?1 ELSE 0 END),0)
             FROM review_coverage WHERE run_id=?1", [run_id], |row| row.get(0),
        ).map_err(|error| error.to_string())
    }

    pub(crate) fn begin_review_coverage(&self, input: &ReviewCoverageInput<'_>) -> Result<(), String> {
        let changed_files = serde_json::to_string(input.changed_files).map_err(|error| error.to_string())?;
        let covered_patch_ids = serde_json::to_string(input.covered_patch_ids).map_err(|error| error.to_string())?;
        self.conn
            .execute(
                "DELETE FROM review_coverage WHERE run_id = ?1 AND status = 'pending'",
                params![input.run_id],
            )
            .map_err(|error| error.to_string())?;
        self.conn.execute(
            "INSERT INTO review_coverage (repo_id, pr_id, run_id, status, base_ref, base_sha, merge_base_sha, head_sha, changed_files_json, covered_patch_ids_json, diff_hash, diff_text, source_json, created_at)
             VALUES (?1, ?2, ?3, 'pending', ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
            params![
                input.repo_id,
                input.pr_id,
                input.run_id,
                input.base_ref,
                input.base_sha,
                input.merge_base_sha,
                input.head_sha,
                changed_files,
                covered_patch_ids,
                input.diff_hash,
                input.diff_text,
                input.source_json,
                now_millis(),
            ],
        ).map_err(|error| error.to_string())?;
        Ok(())
    }

    pub(crate) fn finalize_review_coverage(&self, run_id: i64) -> Result<(), String> {
        if !self.review_evidence_warnings(run_id)?.is_empty() {
            return self.discard_review_coverage(run_id);
        }
        self.conn.execute(
            "UPDATE review_coverage SET status = 'valid', validated_at = ?1 WHERE run_id = ?2 AND status = 'pending'",
            params![now_millis(), run_id],
        ).map_err(|error| error.to_string())?;
        Ok(())
    }

    pub(crate) fn discard_review_coverage(&self, run_id: i64) -> Result<(), String> {
        self.conn
            .execute(
                "DELETE FROM review_coverage WHERE run_id = ?1 AND status = 'pending'",
                params![run_id],
            )
            .map_err(|error| error.to_string())?;
        Ok(())
    }

    pub(crate) fn review_coverage_exists(&self, run_id: i64) -> Result<bool, String> {
        self.conn
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM review_coverage WHERE run_id = ?1 AND status = 'valid')",
                params![run_id],
                |row| row.get::<_, i64>(0),
            )
            .map(|value| value == 1)
            .map_err(|error| error.to_string())
    }

    pub(crate) fn historical_review_coverage_candidates(
        &self,
        repo_id: i64,
    ) -> Result<Vec<HistoricalReviewCoverageCandidate>, String> {
        let mut stmt = self.conn.prepare(
            "SELECT rr.id, rr.pr_id, COALESCE(rr.stacked_base_ref, pr.base_ref), rr.reviewed_base_sha, COALESCE(rr.reviewed_head_sha, rr.head_sha),
                    (SELECT manifest_json FROM review_context_snapshots snapshot WHERE snapshot.review_run_id = rr.id ORDER BY snapshot.id DESC LIMIT 1)
             FROM review_runs rr JOIN pull_requests pr ON pr.id = rr.pr_id
             WHERE pr.repo_id = ?1
               AND COALESCE((SELECT json_array_length(a.audit_json, '$.evidenceWarnings') FROM review_finding_audits a WHERE a.run_id=rr.id),0)=0
               AND rr.review_body IS NOT NULL
               AND rr.status NOT IN ('failed', 'cancelled')
               AND COALESCE(rr.stacked_base_ref, pr.base_ref) IS NOT NULL
               AND rr.reviewed_base_sha IS NOT NULL
             ORDER BY rr.id DESC LIMIT 100",
        ).map_err(|error| error.to_string())?;
        let rows = stmt
            .query_map(params![repo_id], |row| {
                Ok(HistoricalReviewCoverageCandidate {
                    run_id: row.get(0)?,
                    pr_id: row.get(1)?,
                    base_ref: row.get(2)?,
                    base_sha: row.get(3)?,
                    head_sha: row.get(4)?,
                    manifest_json: row.get(5)?,
                })
            })
            .map_err(|error| error.to_string())?;
        rows.collect::<Result<Vec<_>, _>>().map_err(|error| error.to_string())
    }

    fn review_coverage_patch_ids_by_query(
        &self,
        query: &str,
        scope_id: i64,
    ) -> Result<Option<HashSet<String>>, String> {
        let mut stmt = self.conn.prepare(query).map_err(|error| error.to_string())?;
        let rows = stmt
            .query_map(params![scope_id], |row| {
                Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
            })
            .map_err(|error| error.to_string())?;
        let rows = rows.collect::<Result<Vec<_>, _>>().map_err(|error| error.to_string())?;
        let mut ids = HashSet::new();
        let mut malformed = false;
        for (coverage_id, raw) in rows {
            let values: Vec<String> = match serde_json::from_str(&raw) {
                Ok(values) => values,
                Err(_) => {
                    self.conn.execute(
                        "UPDATE review_coverage SET status = 'invalid', validated_at = ?1 WHERE id = ?2 AND status = 'valid'",
                        params![now_millis(), coverage_id],
                    ).map_err(|error| error.to_string())?;
                    malformed = true;
                    continue;
                }
            };
            ids.extend(values);
        }
        if malformed {
            Ok(None)
        } else {
            Ok(Some(ids))
        }
    }

    pub(crate) fn review_coverage_patch_ids_for_pr(&self, pr_id: i64) -> Result<Option<HashSet<String>>, String> {
        self.review_coverage_patch_ids_by_query(
            "SELECT id, covered_patch_ids_json FROM review_coverage WHERE pr_id = ?1 AND status = 'valid' AND COALESCE((SELECT json_array_length(a.audit_json, '$.evidenceWarnings') FROM review_finding_audits a WHERE a.run_id=review_coverage.run_id),0)=0",
            pr_id,
        )
    }

    pub(crate) fn review_coverage_patch_ids(&self, repo_id: i64) -> Result<Option<HashSet<String>>, String> {
        self.review_coverage_patch_ids_by_query(
            "SELECT id, covered_patch_ids_json FROM review_coverage WHERE repo_id = ?1 AND status = 'valid' AND COALESCE((SELECT json_array_length(a.audit_json, '$.evidenceWarnings') FROM review_finding_audits a WHERE a.run_id=review_coverage.run_id),0)=0",
            repo_id,
        )
    }
}
