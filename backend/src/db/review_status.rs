use super::*;

/// A tracked pull request as the review status tools report it.
#[derive(Debug, Clone)]
pub struct ReviewStatusPullRequest {
    pub id: i64,
    pub repository: String,
    pub number: i64,
    pub title: String,
    pub url: Option<String>,
    pub state: String,
    pub review_lane: String,
    pub status_reason: Option<String>,
    pub head_sha: String,
    pub last_synced_at: i64,
}

const REVIEW_STATUS_COLUMNS: &str = "pr.id, r.full_name, pr.number, pr.title, pr.url, pr.state, pr.review_lane,
     pr.status_reason, pr.head_sha, pr.last_synced_at";

fn review_status_pull_request_from_row(row: &pg::Row) -> Result<ReviewStatusPullRequest, pg::Error> {
    Ok(ReviewStatusPullRequest {
        id: row.get(0)?,
        repository: row.get(1)?,
        number: row.get(2)?,
        title: row.get(3)?,
        url: row.get(4)?,
        state: row.get(5)?,
        review_lane: row.get(6)?,
        status_reason: row.get(7)?,
        head_sha: row.get(8)?,
        last_synced_at: row.get(9)?,
    })
}

impl Database {
    /// The pull request `repository#number`, matching the repository name case-insensitively
    /// as GitHub does.
    pub fn review_status_pull_request(
        &self,
        repository: &str,
        number: i64,
    ) -> Result<Option<ReviewStatusPullRequest>, String> {
        self.conn
            .query_row(
                &format!(
                    "SELECT {REVIEW_STATUS_COLUMNS}
                     FROM pull_requests pr JOIN repos r ON r.id = pr.repo_id AND r.removed_at IS NULL
                     WHERE LOWER(r.full_name) = LOWER($1) AND pr.number = $2"
                ),
                params![repository, number],
                review_status_pull_request_from_row,
            )
            .optional()
            .map_err(|error| error.to_string())
    }

    /// Pull requests with a review queued, running or being posted, most recently updated first.
    pub fn active_review_pull_requests(
        &self,
        repository: Option<&str>,
        limit: i64,
    ) -> Result<Vec<ReviewStatusPullRequest>, String> {
        let sql = format!(
            "SELECT {REVIEW_STATUS_COLUMNS}
             FROM pull_requests pr JOIN repos r ON r.id = pr.repo_id AND r.removed_at IS NULL
             WHERE (LOWER(r.full_name) = LOWER($1) OR $1 = '')
               AND LOWER(pr.state) = 'open'
               AND (pr.review_lane IN ('queued', 'reviewing')
                 OR EXISTS(SELECT 1 FROM review_runs rr
                           WHERE rr.pr_id = pr.id AND rr.status IN ('pending', 'running', 'posting'))
                 OR EXISTS(SELECT 1 FROM review_results res
                           WHERE res.pr_id = pr.id AND res.publication_status IN ('pending', 'posting')))
             ORDER BY pr.updated_at DESC LIMIT $2"
        );
        let mut stmt = self.conn.prepare(&sql).map_err(|error| error.to_string())?;
        let rows = stmt
            .query_map(
                params![repository.unwrap_or_default(), limit],
                review_status_pull_request_from_row,
            )
            .map_err(|error| error.to_string())?;
        rows.collect::<Result<Vec<_>, _>>().map_err(|error| error.to_string())
    }
}
