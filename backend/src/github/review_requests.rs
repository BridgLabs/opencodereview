use super::*;
use std::collections::BTreeSet;

/// A pre-publication snapshot, persisted before GitHub can consume a human request.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct ReviewRequestSnapshot {
    pub login: String,
    pub requested: RequestedReviewers,
    pub events: BTreeSet<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct RequestedReviewers {
    pub reviewers: BTreeSet<String>,
    pub team_reviewers: BTreeSet<String>,
}

#[derive(Debug)]
pub(crate) struct RequestTimeline {
    pub open: bool,
    pub events: BTreeSet<String>,
    pub reviews: Vec<AttributedReview>,
}

#[derive(Debug)]
pub(crate) struct AttributedReview {
    pub id: i64,
    pub node_id: String,
    pub login: String,
    pub teams: BTreeSet<String>,
}

pub(crate) enum RestoreRequests {
    Request(RequestedReviewers),
    Done,
    Changed,
}

fn capture_json(args: &[&str]) -> Result<serde_json::Value, String> {
    let bytes = transport::gh_capture(args)?;
    serde_json::from_slice(&bytes).map_err(|e| e.to_string())
}

fn requested_reviewers(
    full_name: &str,
    number: i64,
    api: &mut impl FnMut(&[&str]) -> Result<serde_json::Value, String>,
) -> Result<RequestedReviewers, String> {
    let route = format!("repos/{full_name}/pulls/{number}/requested_reviewers?per_page=100");
    let pages = api(&["api", "--paginate", "--slurp", &route])?;
    let mut requested = RequestedReviewers::default();
    for page in pages.as_array().ok_or("Invalid requested reviewers response")? {
        for (field, key, target) in [
            ("users", "login", &mut requested.reviewers),
            ("teams", "slug", &mut requested.team_reviewers),
        ] {
            for principal in page[field].as_array().ok_or("Missing requested reviewers")? {
                target.insert(
                    principal[key]
                        .as_str()
                        .ok_or("Missing reviewer identity")?
                        .to_lowercase(),
                );
            }
        }
    }
    Ok(requested)
}

fn request_timeline(
    full_name: &str,
    number: i64,
    api: &mut impl FnMut(&[&str]) -> Result<serde_json::Value, String>,
) -> Result<RequestTimeline, String> {
    let (owner, repo) = full_name.split_once('/').ok_or("Invalid repository")?;
    let query = r#"query($owner:String!,$repo:String!,$number:Int!,$cursor:String) {
      repository(owner:$owner,name:$repo) { pullRequest(number:$number) {
        state
        timelineItems(first:100,after:$cursor,itemTypes:[PULL_REQUEST_REVIEW,REVIEW_REQUESTED_EVENT,REVIEW_REQUEST_REMOVED_EVENT,REVIEW_DISMISSED_EVENT,CLOSED_EVENT,MERGED_EVENT,REOPENED_EVENT]) {
          pageInfo { hasNextPage endCursor }
          nodes { __typename ... on Node { id }
            ... on PullRequestReview { databaseId author { login }
              onBehalfOf(first:100) { pageInfo { hasNextPage } nodes { slug } }
            }
          }
        }
      } }
    }"#;
    let mut result = RequestTimeline {
        open: false,
        events: BTreeSet::new(),
        reviews: vec![],
    };
    let mut cursor = None;
    for _ in 0..20 {
        let mut args = vec![
            "api".to_string(),
            "graphql".into(),
            "-f".into(),
            format!("query={query}"),
            "-f".into(),
            format!("owner={owner}"),
            "-f".into(),
            format!("repo={repo}"),
            "-F".into(),
            format!("number={number}"),
        ];
        if let Some(cursor) = cursor.as_ref() {
            args.extend(["-f".into(), format!("cursor={cursor}")]);
        }
        let value = api(&args.iter().map(String::as_str).collect::<Vec<_>>())?;
        if value.get("errors").is_some() {
            return Err("GitHub could not read review request history".into());
        }
        let pr = &value["data"]["repository"]["pullRequest"];
        result.open = pr["state"].as_str().ok_or("Missing PR state")? == "OPEN";
        let items = &pr["timelineItems"];
        for item in items["nodes"].as_array().ok_or("Missing review request history")? {
            let id = item["id"].as_str().ok_or("Missing timeline identity")?.to_string();
            result.events.insert(id.clone());
            if item["__typename"] == "PullRequestReview" {
                if item["onBehalfOf"]["pageInfo"]["hasNextPage"].as_bool() != Some(false) {
                    return Err("Review team attribution exceeds the supported limit".into());
                }
                let teams = item["onBehalfOf"]["nodes"]
                    .as_array()
                    .ok_or("Missing review teams")?
                    .iter()
                    .map(|team| team["slug"].as_str().map(str::to_lowercase).ok_or("Missing team slug"))
                    .collect::<Result<_, _>>()?;
                result.reviews.push(AttributedReview {
                    id: item["databaseId"].as_i64().ok_or("Missing review ID")?,
                    node_id: id,
                    login: item["author"]["login"].as_str().unwrap_or_default().to_lowercase(),
                    teams,
                });
            }
        }
        match items["pageInfo"]["hasNextPage"].as_bool() {
            Some(false) => return Ok(result),
            Some(true) => {
                cursor = Some(
                    items["pageInfo"]["endCursor"]
                        .as_str()
                        .ok_or("Missing timeline cursor")?
                        .to_string(),
                )
            }
            None => return Err("Missing timeline pagination".into()),
        }
    }
    Err("Review request history exceeds the supported limit".into())
}

pub(crate) fn snapshot_review_requests(full_name: &str, number: i64) -> Result<ReviewRequestSnapshot, String> {
    snapshot_review_requests_with(full_name, number, &mut capture_json)
}

fn snapshot_review_requests_with(
    full_name: &str,
    number: i64,
    api: &mut impl FnMut(&[&str]) -> Result<serde_json::Value, String>,
) -> Result<ReviewRequestSnapshot, String> {
    let user = api(&["api", "user"])?;
    let login = user["login"]
        .as_str()
        .ok_or("Missing authenticated GitHub login")?
        .to_lowercase();
    let before = request_timeline(full_name, number, api)?;
    let requested = requested_reviewers(full_name, number, api)?;
    let after = request_timeline(full_name, number, api)?;
    if !after.open || before.events != after.events {
        return Err("Review requests changed before publication. Retry the review decision.".into());
    }
    Ok(ReviewRequestSnapshot {
        login,
        requested,
        events: after.events,
    })
}

/// Only compensate an unchanged history plus this exact review. Concurrent human
/// activity wins, even when unrelated, because GitHub offers no conditional write.
pub(crate) fn plan_request_restoration(
    snapshot: &ReviewRequestSnapshot,
    review_id: i64,
    timeline: &RequestTimeline,
    current: &RequestedReviewers,
) -> Result<RestoreRequests, String> {
    if !timeline.open {
        return Ok(RestoreRequests::Done);
    }
    let review = timeline
        .reviews
        .iter()
        .find(|review| review.id == review_id)
        .ok_or("Published review is not visible in GitHub history yet")?;
    if review.login != snapshot.login {
        return Ok(RestoreRequests::Changed);
    }
    let reviewers: BTreeSet<String> = snapshot
        .requested
        .reviewers
        .iter()
        .filter(|login| **login == snapshot.login && !current.reviewers.contains(*login))
        .cloned()
        .collect();
    let team_reviewers: BTreeSet<String> = snapshot
        .requested
        .team_reviewers
        .intersection(&review.teams)
        .filter(|team| !current.team_reviewers.contains(*team))
        .cloned()
        .collect();
    if reviewers.is_empty() && team_reviewers.is_empty() {
        return Ok(RestoreRequests::Done);
    }
    let mut other_events = timeline.events.clone();
    other_events.remove(&review.node_id);
    if other_events != snapshot.events {
        return Ok(RestoreRequests::Changed);
    }
    Ok(RestoreRequests::Request(RequestedReviewers {
        reviewers,
        team_reviewers,
    }))
}

pub(crate) fn restore_review_requests(
    full_name: &str,
    number: i64,
    review_id: i64,
    snapshot: &ReviewRequestSnapshot,
) -> Result<bool, String> {
    restore_review_requests_with(full_name, number, review_id, snapshot, &mut capture_json)
}

fn restore_review_requests_with(
    full_name: &str,
    number: i64,
    review_id: i64,
    snapshot: &ReviewRequestSnapshot,
    api: &mut impl FnMut(&[&str]) -> Result<serde_json::Value, String>,
) -> Result<bool, String> {
    let current = requested_reviewers(full_name, number, api)?;
    // Read history last to catch changes during the requested-reviewers read.
    let timeline = request_timeline(full_name, number, api)?;
    match plan_request_restoration(snapshot, review_id, &timeline, &current)? {
        RestoreRequests::Done => Ok(true),
        RestoreRequests::Changed => Ok(false),
        RestoreRequests::Request(requested) => {
            if requested.reviewers.is_empty() && requested.team_reviewers.is_empty() {
                return Ok(true);
            }
            let route = format!("repos/{full_name}/pulls/{number}/requested_reviewers");
            let mut args = vec!["api".to_string(), "--method".into(), "POST".into(), route];
            for login in requested.reviewers {
                args.extend(["-f".into(), format!("reviewers[]={login}")]);
            }
            for team in requested.team_reviewers {
                args.extend(["-f".into(), format!("team_reviewers[]={team}")]);
            }
            api(&args.iter().map(String::as_str).collect::<Vec<_>>())?;
            Ok(true)
        }
    }
}

#[cfg(test)]
#[path = "review_requests_tests.rs"]
mod tests;
