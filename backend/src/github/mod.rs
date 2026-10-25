use std::collections::{HashMap, HashSet};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

static GH_LOGIN: Mutex<Option<String>> = Mutex::new(None);
static GH_TEAM_SLUGS: Mutex<Option<HashSet<String>>> = Mutex::new(None);
static GH_HOST: OnceLock<String> = OnceLock::new();
static GH_CONFIG_DIR: OnceLock<Option<PathBuf>> = OnceLock::new();
static GIT_TOKEN: OnceLock<String> = OnceLock::new();

mod activation;
pub(crate) use activation::{activation_pr, conversation_comments_page, unedited_author_command, ConversationComment};

mod auth;
mod comments;
mod pr;
mod repo;
mod review_requests;
mod status_comments;
mod transport;

pub(crate) use auth::{apply_gh_host, apply_git_https_auth, is_valid_gh_host};
pub use auth::{auth_status, current_login, gh_host, run_auth_flow, sign_out};
pub use comments::{
    dedupe_review_comments, find_review_id_by_marker, post_review, prepare_review, upsert_issue_comment_by_marker,
    DedupeResult, PreparedReview,
};
pub use pr::{diff_anchors, pr_description};
pub use repo::{
    list_open_prs, list_open_prs_for_login, list_repos, list_workspaces, live_pr_state, live_pr_state_cancellable,
    LivePrState,
};
#[cfg(test)]
pub(crate) use status_comments::StatusComment;
pub(crate) use status_comments::{GitHubStatusComments, StatusCommentApi};
pub(crate) use transport::{gh_capture_cancellable, gh_json, iso8601_to_epoch_ms, run_raw};

#[cfg(test)]
pub(crate) use repo::{parse_live_pr_state, LIVE_PR_JSON_FIELDS};

use crate::db::{RepoInput, WorkspaceInput};

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GhUser {
    login: String,
}

#[derive(Debug, Deserialize)]
struct GhOrg {
    login: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GhRepo {
    name: String,
    name_with_owner: String,
    description: Option<String>,
    is_private: bool,
    default_branch_ref: Option<GhBranch>,
}

#[derive(Debug, Deserialize)]
struct GhBranch {
    name: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GhPullRequest {
    pub number: i64,
    pub title: String,
    pub author: GhPrAuthor,
    pub is_draft: bool,
    pub head_ref_oid: String,
    pub base_ref_oid: Option<String>,
    pub base_ref_name: Option<String>,
    pub head_ref_name: Option<String>,
    pub url: Option<String>,
    pub updated_at: Option<String>,
    pub changed_files: Option<i64>,
    pub assignees: Option<Vec<GhPrPrincipal>>,
    pub review_requests: Option<Vec<GhPrPrincipal>>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GhPrPrincipal {
    pub login: Option<String>,
    pub name: Option<String>,
    pub slug: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct GhPrAuthor {
    pub login: String,
    pub avatar_url: Option<String>,
}

pub fn avatar_url(login: &str) -> String {
    format!("https://{}/{login}.png?size=64", gh_host())
}

#[cfg(test)]
mod tests;

pub(crate) use review_requests::{restore_review_requests, snapshot_review_requests, ReviewRequestSnapshot};
