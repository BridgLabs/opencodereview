use crate::db::Database;
use crate::review_archive::{ArchiveStore, STORE_QUOTA};
use crate::server::SharedState;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{atomic::AtomicBool, Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

pub const MS_PER_DAY: i64 = 86_400_000;
pub const MAX_RETENTION_DAYS: i64 = 365;
pub const MIN_RETENTION_DAYS: i64 = 1;
pub const DEFAULT_RETENTION_DAYS: i64 = 1;
pub const STORE_QUOTA_BYTES: u64 = STORE_QUOTA;

#[derive(Clone, Default)]
pub struct ArchiveLeaseTracker {
    active_runs: Arc<Mutex<HashSet<i64>>>,
}

pub struct ArchiveRunLease {
    tracker: Arc<Mutex<HashSet<i64>>>,
    run_id: i64,
}

impl Drop for ArchiveRunLease {
    fn drop(&mut self) {
        if let Ok(mut set) = self.tracker.lock() {
            set.remove(&self.run_id);
        }
    }
}

impl ArchiveLeaseTracker {
    pub fn new() -> Self {
        Self {
            active_runs: Arc::new(Mutex::new(HashSet::new())),
        }
    }

    pub fn acquire(&self, run_id: i64) -> ArchiveRunLease {
        if let Ok(mut set) = self.active_runs.lock() {
            set.insert(run_id);
        }
        ArchiveRunLease {
            tracker: self.active_runs.clone(),
            run_id,
        }
    }

    pub fn is_active(&self, run_id: i64) -> bool {
        self.active_runs
            .lock()
            .map(|set| set.contains(&run_id))
            .unwrap_or(false)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ArchiveStorageStatus {
    pub quota_bytes: u64,
    pub used_bytes: u64,
    pub available_bytes: u64,
    pub last_cleanup_at: Option<i64>,
    pub last_packages_removed: usize,
    pub last_bytes_removed: u64,
    pub pending_deferrals: usize,
    pub last_error: Option<String>,
}

impl Default for ArchiveStorageStatus {
    fn default() -> Self {
        Self {
            quota_bytes: STORE_QUOTA_BYTES,
            used_bytes: 0,
            available_bytes: STORE_QUOTA_BYTES,
            last_cleanup_at: None,
            last_packages_removed: 0,
            last_bytes_removed: 0,
            pending_deferrals: 0,
            last_error: None,
        }
    }
}

pub struct RetentionCoordinator {
    pub db_path: PathBuf,
    pub archive_root: PathBuf,
    pub leases: ArchiveLeaseTracker,
    pub status: Arc<Mutex<ArchiveStorageStatus>>,
    pub notify: Arc<(Mutex<bool>, Condvar)>,
    backoff: Mutex<HashMap<i64, Instant>>,
}

impl RetentionCoordinator {
    pub fn new(
        db_path: PathBuf,
        archive_root: PathBuf,
        leases: ArchiveLeaseTracker,
        status: Arc<Mutex<ArchiveStorageStatus>>,
        notify: Arc<(Mutex<bool>, Condvar)>,
    ) -> Self {
        Self {
            db_path,
            archive_root,
            leases,
            status,
            notify,
            backoff: Mutex::new(HashMap::new()),
        }
    }

    pub fn perform_maintenance(&self, cancel: &AtomicBool) -> Result<MaintenanceResult, String> {
        self.perform_maintenance_with_backfill(|store, db| self.backfill_merges(store, db, cancel))
    }

    fn perform_maintenance_with_backfill(
        &self,
        backfill: impl FnOnce(&ArchiveStore, &Database) -> Result<(), String>,
    ) -> Result<MaintenanceResult, String> {
        let result = self.maintenance_pass(backfill);
        if let Err(error) = &result {
            eprintln!("Archive retention maintenance error: {error}");
            if let Ok(mut status) = self.status.lock() {
                status.last_error = Some(
                    "Archive maintenance failed. Check server logs; previous successful metrics are retained.".into(),
                );
            }
        }
        result
    }

    fn maintenance_pass(
        &self,
        backfill: impl FnOnce(&ArchiveStore, &Database) -> Result<(), String>,
    ) -> Result<MaintenanceResult, String> {
        let mut warnings = Vec::new();
        let now_ms = crate::sandbox::now();
        let db = Database::open_at(self.db_path.clone()).map_err(|e| format!("Could not open database: {e}"))?;
        let mut store =
            ArchiveStore::open(&self.archive_root).map_err(|e| format!("Could not open archive store: {e:?}"))?;

        // 1. Validate retention setting.
        let raw_setting = db
            .setting("review_archive_retention_days")
            .map_err(|e| format!("Could not read retention setting: {e}"))?;

        let retention_days = match raw_setting {
            Some(value) => match value.parse::<i64>() {
                Ok(days) if (MIN_RETENTION_DAYS..=MAX_RETENTION_DAYS).contains(&days) => days,
                _ => {
                    let err = "Malformed retention days setting; deletion suspended.".to_string();
                    if let Ok(mut st) = self.status.lock() {
                        st.last_error = Some(err.clone());
                    }
                    return Err(err);
                }
            },
            None => DEFAULT_RETENTION_DAYS,
        };

        // 2. Local cleanup pass for already-confirmed merges
        let (removed_packages, removed_bytes, _deferrals) =
            self.cleanup_pass(&mut store, &db, retention_days, now_ms, &mut warnings)?;

        // 3. Backfill merge timestamps from GitHub for PRs with operational archives
        if let Err(backfill_err) = backfill(&store, &db) {
            eprintln!("Merge backfill warning: {backfill_err}");
            warnings.push("Merge timestamps could not all be verified; affected archives were deferred.");
        }

        // 4. Second quick cleanup pass if backfill confirmed new merges
        let (additional_packages, additional_bytes, final_deferrals) =
            self.cleanup_pass(&mut store, &db, retention_days, now_ms, &mut warnings)?;

        let total_packages = removed_packages + additional_packages;
        let total_bytes = removed_bytes + additional_bytes;

        // 5. Compute usage and update storage status
        let (recorded, actual) = store
            .storage_usage()
            .map_err(|e| format!("Could not measure storage usage: {e:?}"))?;
        let used_bytes = recorded.max(actual);
        let available_bytes = STORE_QUOTA_BYTES.saturating_sub(used_bytes);

        let mut st = self.status.lock().map_err(|_| "Status lock poisoned")?;
        st.quota_bytes = STORE_QUOTA_BYTES;
        st.used_bytes = used_bytes;
        st.available_bytes = available_bytes;
        st.last_cleanup_at = Some(now_ms);
        st.last_packages_removed = total_packages;
        st.last_bytes_removed = total_bytes;
        st.pending_deferrals = final_deferrals;
        warnings.sort_unstable();
        warnings.dedup();
        st.last_error = (!warnings.is_empty()).then(|| warnings.join(" "));

        Ok(MaintenanceResult {
            packages_removed: total_packages,
            bytes_removed: total_bytes,
            deferrals: final_deferrals,
            used_bytes,
            available_bytes,
        })
    }

    fn cleanup_pass(
        &self,
        store: &mut ArchiveStore,
        db: &Database,
        retention_days: i64,
        now_ms: i64,
        warnings: &mut Vec<&'static str>,
    ) -> Result<(usize, u64, usize), String> {
        let retention_ms = retention_days
            .checked_mul(MS_PER_DAY)
            .ok_or_else(|| "Retention duration overflow".to_string())?;
        let cutoff_ms = now_ms.saturating_sub(retention_ms);

        let artifacts = store
            .list_artifacts()
            .map_err(|e| format!("Could not list artifacts: {e:?}"))?;

        let mut removed_packages = 0;
        let mut removed_bytes = 0;
        let mut deferrals = 0;

        for (id, _bytes) in artifacts {
            // Check active upload
            if store
                .has_active_upload_for_artifact(&id)
                .map_err(|e| format!("Upload check failed: {e:?}"))?
            {
                deferrals += 1;
                continue;
            }

            let receipts = store
                .receipts_for_artifact(&id)
                .map_err(|e| format!("Receipts check failed: {e:?}"))?;

            // Unresolved ownership
            if receipts.is_empty() {
                deferrals += 1;
                continue;
            }

            let active_runs = self.leases.active_runs.lock().map_err(|_| "Lease lock poisoned")?;
            let mut all_owners_eligible = true;
            for owner in &receipts {
                let Some(run_id_str) = owner.strip_prefix("review:") else {
                    all_owners_eligible = false;
                    break;
                };
                let Ok(run_id) = run_id_str.parse::<i64>() else {
                    all_owners_eligible = false;
                    break;
                };

                // Check in-flight lease
                if active_runs.contains(&run_id) {
                    all_owners_eligible = false;
                    break;
                }

                // Check DB for run and PR
                match self.check_run_and_pr_eligibility(db, run_id, cutoff_ms) {
                    Ok(true) => {}
                    Err(error) => {
                        eprintln!("Archive eligibility check failed: {error}");
                        warnings.push("Some archive owners could not be checked; affected archives were deferred.");
                        all_owners_eligible = false;
                        break;
                    }
                    Ok(false) => {
                        all_owners_eligible = false;
                        break;
                    }
                }
            }

            if all_owners_eligible {
                match store.delete_artifact(&id, &receipts) {
                    Ok(None) => {
                        deferrals += 1;
                    }
                    Ok(Some(freed)) => {
                        removed_packages += 1;
                        removed_bytes += freed;
                    }
                    Err(e) => {
                        eprintln!("Failed to delete artifact {id}: {e:?}");
                        warnings.push("Some archives could not be deleted. Check server logs.");
                        deferrals += 1;
                    }
                }
            } else {
                deferrals += 1;
            }
        }

        Ok((removed_packages, removed_bytes, deferrals))
    }

    fn check_run_and_pr_eligibility(&self, db: &Database, run_id: i64, cutoff_ms: i64) -> Result<bool, String> {
        let Some((status, pr_id)) = db.review_run_retention_info(run_id)? else {
            return Ok(false); // Absent run
        };

        // If run is still in-flight / non-terminal, preserve
        if matches!(
            status.to_ascii_lowercase().as_str(),
            "pending" | "running" | "posting" | "preview"
        ) {
            return Ok(false);
        }

        let Some((state, merged_at)) = db.pr_retention_info(pr_id)? else {
            return Ok(false);
        };

        let Some(merged_at_ms) = merged_at else {
            return Ok(false); // Merge not confirmed
        };

        // Must be merged and merged_at <= cutoff_ms (inclusive comparison)
        if state.eq_ignore_ascii_case("merged") && merged_at_ms <= cutoff_ms {
            Ok(true)
        } else {
            Ok(false)
        }
    }

    fn backfill_merges(&self, store: &ArchiveStore, db: &Database, cancel: &AtomicBool) -> Result<(), String> {
        let run_ids = store
            .distinct_review_receipt_run_ids()
            .map_err(|e| format!("Could not get distinct receipt run IDs: {e:?}"))?;

        if run_ids.is_empty() {
            return Ok(());
        }

        let active_candidates = self.next_backfill_candidates(db, &run_ids)?;

        let mut failed = !self.backoff.lock().map_err(|_| "Backoff lock poisoned")?.is_empty();
        // Process in batches of up to 25
        for batch in active_candidates.chunks(25) {
            if cancel.load(std::sync::atomic::Ordering::Relaxed) {
                return Err("Merge backfill cancelled".into());
            }
            if let Err(e) = self.query_github_batch(batch, db, cancel) {
                failed = true;
                eprintln!("Batch GitHub merge query error: {e}");
                // Apply backoff to this batch
                if let Ok(mut b) = self.backoff.lock() {
                    let backoff_until = Instant::now() + Duration::from_secs(300);
                    for (pr_id, _, _) in batch {
                        b.insert(*pr_id, backoff_until);
                    }
                }
            }
        }

        if failed {
            Err("Some merge checks failed or remain on backoff".into())
        } else {
            Ok(())
        }
    }

    fn next_backfill_candidates(&self, db: &Database, run_ids: &[i64]) -> Result<Vec<(i64, String, i64)>, String> {
        let mut candidates = db.find_candidate_prs_for_merge_backfill(run_ids)?;
        candidates.sort_by_key(|candidate| candidate.0);
        candidates.dedup_by_key(|candidate| candidate.0);
        let cursor = db
            .setting("review_archive_backfill_cursor")?
            .and_then(|value| value.parse::<i64>().ok())
            .unwrap_or(0);
        let split = candidates.partition_point(|candidate| candidate.0 <= cursor);
        candidates.rotate_left(split);
        let now = Instant::now();
        let mut backoff = self.backoff.lock().map_err(|_| "Backoff lock poisoned")?;
        backoff.retain(|_, until| *until > now);
        let selected: Vec<_> = candidates
            .into_iter()
            .filter(|candidate| !backoff.contains_key(&candidate.0))
            .take(200)
            .collect();
        drop(backoff);
        if let Some(last) = selected.last() {
            db.save_setting_value("review_archive_backfill_cursor", &last.0.to_string())?;
        }
        Ok(selected)
    }

    fn query_github_batch(
        &self,
        batch: &[(i64, String, i64)],
        db: &Database,
        cancel: &AtomicBool,
    ) -> Result<(), String> {
        let mut fields = Vec::new();
        for (index, (_pr_id, full_name, number)) in batch.iter().enumerate() {
            let (owner, repo) = full_name
                .split_once('/')
                .ok_or_else(|| "Invalid repository identity".to_string())?;
            fields.push(format!(
                "p{index}:repository(owner:{},name:{}){{pullRequest(number:{}){{state mergedAt}}}}",
                serde_json::json!(owner),
                serde_json::json!(repo),
                number
            ));
        }

        let query = format!("query{{{}}}", fields.join(" "));
        let response_bytes =
            crate::github::gh_capture_cancellable(&["api", "graphql", "-f", &format!("query={query}")], cancel)?;

        let response: serde_json::Value =
            serde_json::from_slice(&response_bytes).map_err(|e| format!("Invalid JSON from GitHub GraphQL: {e}"))?;

        let mut incomplete = response
            .get("errors")
            .and_then(|value| value.as_array())
            .is_some_and(|errors| !errors.is_empty());
        if let Some(data) = response.get("data").and_then(|d| d.as_object()) {
            for (index, (pr_id, _full_name, _number)) in batch.iter().enumerate() {
                let key = format!("p{index}");
                let pr_node = data
                    .get(&key)
                    .and_then(|v| v.get("pullRequest"))
                    .filter(|value| value.is_object());
                let Some(pr) = pr_node else {
                    incomplete = true;
                    // This specific PR or repo had an error or was null; back off on it
                    if let Ok(mut b) = self.backoff.lock() {
                        b.insert(*pr_id, Instant::now() + Duration::from_secs(600));
                    }
                    continue;
                };

                let state = pr["state"].as_str().unwrap_or("");
                if state.eq_ignore_ascii_case("MERGED") {
                    let merged_at = pr["mergedAt"]
                        .as_str()
                        .and_then(crate::github::iso8601_to_epoch_ms)
                        .ok_or("GitHub returned a merged PR without a valid merge timestamp")?;
                    db.set_pr_merged_at(*pr_id, merged_at)?;
                } else if state.eq_ignore_ascii_case("CLOSED") {
                    db.mark_pr_terminal(*pr_id, "closed", "PR closed on GitHub")?;
                } else if !state.eq_ignore_ascii_case("OPEN") {
                    incomplete = true;
                }
            }
        } else {
            return Err("GitHub GraphQL returned no data object".to_string());
        }

        if incomplete {
            Err("GitHub returned incomplete merge information".into())
        } else {
            Ok(())
        }
    }
}

#[derive(Debug, Clone)]
pub struct MaintenanceResult {
    pub packages_removed: usize,
    pub bytes_removed: u64,
    pub deferrals: usize,
    pub used_bytes: u64,
    pub available_bytes: u64,
}

fn wait_for_maintenance(notify: &(Mutex<bool>, Condvar), interval: Duration) -> Result<(), String> {
    let (lock, cvar) = notify;
    let wake = lock.lock().map_err(|_| "Retention notification lock poisoned")?;
    let (mut wake, _) = cvar
        .wait_timeout_while(wake, interval, |wake| !*wake)
        .map_err(|_| "Retention notification lock poisoned")?;
    *wake = false;
    Ok(())
}

pub(crate) fn start_archive_retention_worker(state: SharedState) {
    let coordinator = Arc::new(RetentionCoordinator::new(
        state.config.db_path.clone(),
        state
            .config
            .db_path
            .parent()
            .unwrap_or(Path::new("."))
            .join("review-archives"),
        state.archive_leases.clone(),
        state.archive_storage_status.clone(),
        state.retention_worker_notify.clone(),
    ));

    std::thread::Builder::new()
        .name("archive-retention".into())
        .spawn(move || {
            let cancel = AtomicBool::new(false);
            loop {
                let _ = coordinator.perform_maintenance(&cancel);

                if wait_for_maintenance(&coordinator.notify, Duration::from_secs(3600)).is_err() {
                    break;
                }
            }
        })
        .expect("start archive retention thread");
}

#[cfg(test)]
#[path = "retention_tests.rs"]
mod tests;
