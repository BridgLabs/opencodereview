use super::*;

use crate::db::{UsageFilter, USAGE_RANGES};

/// Furthest offset from UTC a time zone has, in minutes (UTC+14).
const MAX_TZ_MINUTES: i64 = 14 * 60;
const MAX_PR_PAGE: i64 = 100;

/// `days`, `tz` and `repo` from the query string. Values out of range fall back to the defaults
/// (30 days, UTC, every repository) rather than failing the page.
fn usage_filter(request: &Request) -> UsageFilter {
    let number = |name| query_param(request, name).and_then(|value| value.parse::<i64>().ok());
    UsageFilter {
        days: number("days").filter(|days| USAGE_RANGES.contains(days)).unwrap_or(30),
        tz_minutes: number("tz").unwrap_or(0).clamp(-MAX_TZ_MINUTES, MAX_TZ_MINUTES),
        repo_id: number("repo"),
        now: crate::db::now_millis(),
    }
}

pub(crate) fn usage_report_http(request: &Request, state: &SharedState) -> Result<serde_json::Value, String> {
    let filter = usage_filter(request);
    with_db_read(state, |db| db.usage_report(&filter))
}

pub(crate) fn usage_prs_http(request: &Request, state: &SharedState) -> Result<serde_json::Value, String> {
    let filter = usage_filter(request);
    let number = |name| query_param(request, name).and_then(|value| value.parse::<i64>().ok());
    let offset = number("offset").unwrap_or(0).max(0);
    let limit = number("limit").unwrap_or(50).clamp(1, MAX_PR_PAGE);
    with_db_read(state, |db| db.usage_prs(&filter, offset, limit))
}

pub(crate) fn usage_pr_runs_http(
    request: &Request,
    state: &SharedState,
    pr_id: i64,
) -> Result<serde_json::Value, String> {
    let filter = usage_filter(request);
    with_db_read(state, |db| db.pr_usage_runs(pr_id, &filter))
}
