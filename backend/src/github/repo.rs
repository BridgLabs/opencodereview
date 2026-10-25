use super::*;

pub fn list_workspaces() -> Result<Vec<WorkspaceInput>, String> {
    let user: GhUser = gh_json(&["api", "user"])?;
    let mut workspaces = vec![WorkspaceInput {
        login: user.login,
        kind: "user".to_string(),
    }];
    let orgs: Vec<GhOrg> = gh_json(&["api", "user/orgs", "--paginate"])?;
    workspaces.extend(orgs.into_iter().map(|org| WorkspaceInput {
        login: org.login,
        kind: "org".to_string(),
    }));
    workspaces.sort_by(|a, b| a.login.cmp(&b.login));
    workspaces.dedup_by(|a, b| a.login == b.login);
    Ok(workspaces)
}

pub fn list_repos(workspace_login: &str) -> Result<Vec<RepoInput>, String> {
    let repos: Vec<GhRepo> = gh_json(&[
        "repo",
        "list",
        workspace_login,
        "--limit",
        "1000",
        "--json",
        "name,nameWithOwner,description,isPrivate,defaultBranchRef",
    ])?;
    Ok(repos
        .into_iter()
        .filter_map(|repo| {
            let (owner, name) = repo.name_with_owner.split_once('/')?;
            Some(RepoInput {
                workspace_login: workspace_login.to_string(),
                owner: owner.to_string(),
                name: if repo.name.is_empty() {
                    name.to_string()
                } else {
                    repo.name
                },
                full_name: repo.name_with_owner,
                description: repo.description.filter(|value| !value.trim().is_empty()),
                is_private: repo.is_private,
                default_branch: repo.default_branch_ref.map(|branch| branch.name),
            })
        })
        .collect())
}

pub fn list_open_prs(full_name: &str) -> Result<Vec<GhPullRequest>, String> {
    let limit = std::env::var("OCR_GH_PR_LIMIT")
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(1000)
        .to_string();
    list_open_prs_with_args(full_name, &["--limit", &limit])
}

pub fn list_open_prs_for_login(full_name: &str, login: &str) -> Result<Vec<GhPullRequest>, String> {
    let mut prs = list_open_prs_with_args(full_name, &["--assignee", login, "--limit", "100"])?;
    let requested = list_open_prs_with_args(
        full_name,
        &["--search", &format!("review-requested:{login}"), "--limit", "100"],
    )?;
    let mut seen: std::collections::HashSet<i64> = prs.iter().map(|pr| pr.number).collect();
    for pr in requested {
        if seen.insert(pr.number) {
            prs.push(pr);
        }
    }
    Ok(prs)
}

pub(crate) fn list_open_prs_with_args(full_name: &str, extra: &[&str]) -> Result<Vec<GhPullRequest>, String> {
    let mut args = vec!["pr", "list", "--repo", full_name, "--state", "open"];
    args.extend_from_slice(extra);
    args.extend_from_slice(&["--json", "number,title,author,isDraft,headRefOid,baseRefOid,baseRefName,headRefName,url,updatedAt,changedFiles,assignees,reviewRequests"]);
    gh_json(&args)
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct GhPrState {
    state: String,
    head_ref_oid: String,
    base_ref_name: String,
    base_ref_oid: String,
    merged_at: Option<String>,
}

pub(crate) const LIVE_PR_JSON_FIELDS: &str = "state,headRefOid,baseRefName,baseRefOid,mergedAt";

#[derive(Debug, Clone)]
pub struct LivePrState {
    pub state: String,
    pub head_sha: String,
    pub base_ref: String,
    pub base_sha: String,
    pub merged_at: Option<i64>,
}

impl LivePrState {
    pub fn review_scope(&self) -> crate::review::scope::ReviewScope {
        crate::review::scope::ReviewScope {
            head_sha: self.head_sha.clone(),
            base_ref: self.base_ref.clone(),
            base_sha: self.base_sha.clone(),
        }
    }
}

pub fn live_pr_state(full_name: &str, number: i64) -> Result<LivePrState, String> {
    live_pr_state_cancellable(full_name, number, &std::sync::atomic::AtomicBool::new(false))
}

pub fn live_pr_state_cancellable(
    full_name: &str,
    number: i64,
    cancel: &std::sync::atomic::AtomicBool,
) -> Result<LivePrState, String> {
    let output = gh_capture_cancellable(
        &[
            "pr",
            "view",
            &number.to_string(),
            "--repo",
            full_name,
            "--json",
            LIVE_PR_JSON_FIELDS,
        ],
        cancel,
    )?;
    parse_live_pr_state(&output)
}

pub(crate) fn parse_live_pr_state(output: &[u8]) -> Result<LivePrState, String> {
    let parsed: GhPrState = serde_json::from_slice(output).map_err(|error| error.to_string())?;
    Ok(LivePrState {
        state: parsed.state,
        head_sha: parsed.head_ref_oid,
        base_ref: parsed.base_ref_name,
        base_sha: parsed.base_ref_oid,
        merged_at: parsed.merged_at.as_deref().and_then(crate::github::iso8601_to_epoch_ms),
    })
}
