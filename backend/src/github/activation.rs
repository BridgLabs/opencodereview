use super::*;

#[derive(Debug, Deserialize)]
pub(crate) struct ConversationComment {
    pub id: i64,
    pub node_id: String,
    pub body: String,
    pub created_at: String,
    pub updated_at: String,
    pub issue_url: String,
    pub user: CommentAuthor,
}

#[derive(Debug, Deserialize)]
pub(crate) struct CommentAuthor {
    pub id: i64,
    #[serde(rename = "type")]
    pub kind: String,
}

impl ConversationComment {
    pub(crate) fn command_number(&self, full_name: &str, enabled_at: i64) -> Option<i64> {
        if self.body.trim() != "/ocr review"
            || self.created_at != self.updated_at
            || self.user.kind != "User"
            || self.user.id <= 0
            || iso8601_to_epoch_ms(&self.created_at)? <= enabled_at
        {
            return None;
        }
        let (_, path) = self.issue_url.split_once("://")?;
        let path = path.split_once('/')?.1;
        let number = path
            .strip_prefix(&format!("repos/{full_name}/issues/"))
            .or_else(|| path.strip_prefix(&format!("api/v3/repos/{full_name}/issues/")))?;
        number.parse::<i64>().ok().filter(|n| *n > 0)
    }
}

pub(crate) fn conversation_comments_page(
    full_name: &str,
    since: &str,
    page: usize,
) -> Result<Vec<ConversationComment>, String> {
    let route =
        format!("repos/{full_name}/issues/comments?since={since}&sort=updated&direction=asc&per_page=100&page={page}");
    let bytes = transport::gh_capture(&["api", &route])?;
    serde_json::from_slice(&bytes).map_err(|e| e.to_string())
}

pub(crate) fn activation_pr(full_name: &str, number: i64) -> Result<Option<GhPullRequest>, String> {
    let bytes = transport::gh_capture(&["pr", "view", &number.to_string(), "--repo", full_name,
        "--json", "state,number,title,author,isDraft,headRefOid,baseRefOid,baseRefName,headRefName,url,updatedAt,changedFiles,assignees,reviewRequests"])?;
    let value: serde_json::Value = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
    match value.get("state").and_then(|v| v.as_str()) {
        Some("OPEN") => serde_json::from_value(value).map(Some).map_err(|e| e.to_string()),
        Some("CLOSED" | "MERGED") => Ok(None),
        _ => Err("Missing pull request state".into()),
    }
}

pub(crate) fn is_pr_author(full_name: &str, number: i64, user_id: i64) -> Result<bool, String> {
    let route = format!("repos/{full_name}/issues/{number}");
    let bytes = transport::gh_capture(&["api", &route])?;
    let issue: serde_json::Value = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
    Ok(pr_author_response(&issue, user_id))
}

fn pr_author_response(issue: &serde_json::Value, user_id: i64) -> bool {
    issue.get("pull_request").is_some()
        && issue.get("state").and_then(|v| v.as_str()) == Some("open")
        && issue.pointer("/user/id").and_then(|v| v.as_i64()) == Some(user_id)
}

pub(crate) fn unedited_author_command(
    full_name: &str,
    number: i64,
    comment: &ConversationComment,
) -> Result<bool, String> {
    if !is_pr_author(full_name, number, comment.user.id)? {
        return Ok(false);
    }
    let query = "query($id:ID!){node(id:$id){... on IssueComment{body lastEditedAt}}}";
    let bytes = transport::gh_capture(&[
        "api",
        "graphql",
        "-f",
        &format!("query={query}"),
        "-f",
        &format!("id={}", comment.node_id),
    ])?;
    let value: serde_json::Value = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
    unedited_command_response(&value)
}

pub(crate) fn unedited_command_response(value: &serde_json::Value) -> Result<bool, String> {
    if value.get("errors").is_some() {
        return Err("Could not verify command edit history".into());
    }
    let Some(node) = value.pointer("/data/node").filter(|v| !v.is_null()) else {
        return Ok(false);
    };
    let edited = node.get("lastEditedAt").ok_or("Missing command edit history")?;
    Ok(edited.is_null()
        && node
            .get("body")
            .and_then(|v| v.as_str())
            .is_some_and(|body| body.trim() == "/ocr review"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn activation_matches_author_identity_not_organization_role_or_regular_issue() {
        let pr =
            serde_json::json!({"pull_request": {}, "state":"open", "user":{"id":123}, "author_association":"MEMBER"});
        assert!(pr_author_response(&pr, 123));
        assert!(!pr_author_response(&pr, 456));
        assert!(!pr_author_response(
            &serde_json::json!({"state":"open","user":{"id":123}}),
            123
        ));
        assert!(!pr_author_response(
            &serde_json::json!({"pull_request":{},"state":"closed","user":{"id":123}}),
            123
        ));
    }

    #[test]
    fn activation_rejects_same_second_edit_and_missing_history() {
        assert!(unedited_command_response(
            &serde_json::json!({"data":{"node":{"body":" /ocr review\n", "lastEditedAt":null}}})
        )
        .unwrap());
        assert!(!unedited_command_response(
            &serde_json::json!({"data":{"node":{"body":"/ocr review", "lastEditedAt":"2026-09-21T01:00:00Z"}}})
        )
        .unwrap());
        assert!(!unedited_command_response(&serde_json::json!({"data":{"node":null}})).unwrap());
        assert!(unedited_command_response(&serde_json::json!({"data":{"node":{"body":"/ocr review"}}})).is_err());
        assert!(unedited_command_response(&serde_json::json!({"errors":[{}]})).is_err());
    }
}
