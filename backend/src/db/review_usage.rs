use super::*;

impl Database {
    pub fn cached_review_chunk(&self, cache_key: &str) -> Result<Option<(String, String)>, String> {
        let mut stmt = self.conn.prepare("SELECT summary, comments_json FROM review_chunk_cache WHERE cache_key = $1 AND comments_json IS NOT NULL AND last_seen_at >= $2").map_err(|error| error.to_string())?;
        let mut rows = stmt
            .query(params![cache_key, now_millis() - 7 * 24 * 60 * 60 * 1000])
            .map_err(|error| error.to_string())?;
        rows.next()
            .map_err(|error| error.to_string())?
            .map(|row| {
                Ok((
                    row.get(0).map_err(|error| error.to_string())?,
                    row.get(1).map_err(|error| error.to_string())?,
                ))
            })
            .transpose()
    }

    pub fn review_chunk_cache_exists(&self, cache_key: &str) -> Result<bool, String> {
        self.conn
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM review_chunk_cache WHERE cache_key = $1)",
                params![cache_key],
                |row| row.get::<_, i64>(0),
            )
            .map(|value| value != 0)
            .map_err(|error| error.to_string())
    }

    pub fn upsert_review_chunk_cache(&self, cache: &ReviewChunkCacheInput) -> Result<(), String> {
        self.conn.execute(
            "INSERT INTO review_chunk_cache (cache_key, repo_id, pr_id, run_id, chunk_id, head_sha, prior_sha, model, prompt_version, skill_hash, index_spec_version, chunk_json, summary, comments_json, usage_json, prompt_bytes, diff_bytes, context_bytes, created_at, last_seen_at, seen_count)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17, $18, $19, $19, 1)
             ON CONFLICT(cache_key) DO UPDATE SET run_id = excluded.run_id, pr_id = excluded.pr_id, chunk_id = excluded.chunk_id, summary = excluded.summary, comments_json = excluded.comments_json, usage_json = excluded.usage_json, prompt_bytes = excluded.prompt_bytes, diff_bytes = excluded.diff_bytes, context_bytes = excluded.context_bytes, last_seen_at = excluded.last_seen_at, seen_count = review_chunk_cache.seen_count + 1",
            params![
                cache.cache_key,
                cache.repo_id,
                cache.pr_id,
                cache.run_id,
                cache.chunk_id,
                cache.head_sha,
                cache.prior_sha,
                cache.model,
                cache.prompt_version,
                cache.skill_hash,
                cache.index_spec_version,
                cache.chunk_json,
                cache.summary,
                cache.comments_json,
                cache.usage_json,
                cache.prompt_bytes,
                cache.diff_bytes,
                cache.context_bytes,
                cache.now,
            ],
        ).map_err(|error| error.to_string())?;
        self.conn.execute("DELETE FROM review_chunk_cache WHERE repo_id = $1 AND (last_seen_at < $2 OR cache_key NOT IN (SELECT cache_key FROM review_chunk_cache WHERE repo_id = $1 ORDER BY last_seen_at DESC, cache_key DESC LIMIT 256))", params![cache.repo_id, cache.now - 7 * 24 * 60 * 60 * 1000]).map_err(|error| error.to_string())?;
        Ok(())
    }

    /// Replace the per-model usage rows for a run. `rows` = (model, in, out, cache_read, cache_creation).
    pub fn set_run_model_usage(
        &self,
        run_id: i64,
        pr_id: i64,
        rows: &[(String, i64, i64, i64, i64)],
    ) -> Result<(), String> {
        self.conn
            .execute("DELETE FROM review_run_model_usage WHERE run_id = $1", params![run_id])
            .map_err(|error| error.to_string())?;
        for (model, input, output, cache_read, cache_creation) in rows {
            if *input + *output == 0 {
                continue;
            }
            self.conn.execute(
        "INSERT INTO review_run_model_usage (run_id, pr_id, model, input_tokens, output_tokens, cache_read_tokens, cache_creation_tokens)
         VALUES ($1, $2, $3, $4, $5, $6, $7)
         ON CONFLICT (run_id, model) DO UPDATE SET pr_id = excluded.pr_id, input_tokens = excluded.input_tokens,
           output_tokens = excluded.output_tokens, cache_read_tokens = excluded.cache_read_tokens,
           cache_creation_tokens = excluded.cache_creation_tokens",
        params![run_id, pr_id, model, input, output, cache_read, cache_creation],
      ).map_err(|error| error.to_string())?;
        }
        Ok(())
    }

    /// Aggregate token usage across all runs of a PR (cumulative, not just latest).
    pub fn pr_usage(&self, pr_id: i64) -> Result<UsageStats, String> {
        self.conn.query_row(
      "SELECT COALESCE(SUM(input_tokens), 0)::BIGINT, COALESCE(SUM(output_tokens), 0)::BIGINT, COALESCE(SUM(cache_read_tokens), 0)::BIGINT, COALESCE(SUM(cache_creation_tokens), 0)::BIGINT, COUNT(*)
       FROM review_runs WHERE pr_id = $1 AND (input_tokens > 0 OR output_tokens > 0)",
      params![pr_id],
      |row| Ok(UsageStats {
        input_tokens: row.get(0)?,
        output_tokens: row.get(1)?,
        cache_read_tokens: row.get(2)?,
        cache_creation_tokens: row.get(3)?,
        run_count: row.get(4)?,
      }),
    ).map_err(|error| error.to_string())
    }
}
