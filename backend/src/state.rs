use std::collections::HashMap;
use std::sync::{atomic::AtomicBool, Arc};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IndexProgressEvent {
    pub repo_id: i64,
    pub stage: String,
    pub processed: i64,
    pub total: i64,
    pub overall_percent: i64,
    pub file_count: i64,
    pub package_count: i64,
    pub edge_count: i64,
    pub skipped_count: i64,
    pub files_per_sec: f64,
    pub eta_ms: Option<i64>,
    pub current_path: Option<String>,
    pub updated_at: i64,
}

/// A running review job and its cancel flags.
/// `superseded` distinguishes a cancel caused by a new commit (poll already set
/// the right lane) from a user cancel (which sends the PR back to manual).
pub struct ReviewJob {
    pub cancel_by_run: HashMap<i64, Arc<AtomicBool>>,
    pub superseded: Arc<AtomicBool>,
    pub force_full: bool,
    pub reprocess: bool,
}
