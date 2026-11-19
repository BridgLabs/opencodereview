use super::*;

const MAX_THREAD_PAGES: usize = 20;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PublishedThread {
    pub thread_id: String,
    pub comment_id: i64,
    pub fingerprint: String,
    pub path: String,
    pub line: i64,
    pub body: String,
    pub resolved: bool,
}

impl PublishedThread {
    fn from_node(node: &serde_json::Value, login: &str) -> Option<Self> {
        let comment = node.pointer("/comments/nodes/0")?;
        let body = comment["body"].as_str()?;
        let author = comment.pointer("/author/login").and_then(serde_json::Value::as_str)?;
        if !author.eq_ignore_ascii_case(login) || crate::review::findings::point_kind(body) != "finding" {
            return None;
        }
        Some(Self {
            thread_id: node["id"].as_str()?.to_string(),
            comment_id: comment["databaseId"].as_i64()?,
            fingerprint: comments::extract_ocr_fingerprint(body)?,
            path: node["path"].as_str().unwrap_or_default().to_string(),
            // An outdated thread has no current line; its original one still places it.
            line: node["line"]
                .as_i64()
                .or_else(|| node["originalLine"].as_i64())
                .unwrap_or_default(),
            body: body.to_string(),
            resolved: node["isResolved"].as_bool()?,
        })
    }
}

const THREADS_QUERY: &str = r#"query($owner:String!,$repo:String!,$number:Int!,$cursor:String) {
  repository(owner:$owner,name:$repo) { pullRequest(number:$number) {
    reviewThreads(first:100,after:$cursor) {
      pageInfo { hasNextPage endCursor }
      nodes { id isResolved path line originalLine
        comments(first:1) { nodes { databaseId body author { login } } } }
    }
  } }
}"#;

/// Every thread of the pull request opened by an OpenCodeReview finding, resolved or not. A
/// partial read is an error: a missing page could hide the thread a decision is about.
pub(crate) fn published_finding_threads(full_name: &str, number: i64) -> Result<Vec<PublishedThread>, String> {
    let (owner, repo) = full_name.split_once('/').ok_or("Invalid repository")?;
    let login = current_login().ok_or("Missing authenticated GitHub login")?;
    let mut threads = Vec::new();
    let mut cursor: Option<String> = None;
    for _ in 0..MAX_THREAD_PAGES {
        let mut args = vec![
            "api".to_string(),
            "graphql".into(),
            "-f".into(),
            format!("query={THREADS_QUERY}"),
            "-f".into(),
            format!("owner={owner}"),
            "-f".into(),
            format!("repo={repo}"),
            "-F".into(),
            format!("number={number}"),
        ];
        if let Some(cursor) = &cursor {
            args.extend(["-f".into(), format!("cursor={cursor}")]);
        }
        let value: serde_json::Value = gh_json(&args.iter().map(String::as_str).collect::<Vec<_>>())?;
        if value.get("errors").is_some() {
            return Err("GitHub could not read the review threads".into());
        }
        let page = &value["data"]["repository"]["pullRequest"]["reviewThreads"];
        for node in page["nodes"].as_array().ok_or("Missing review threads")? {
            threads.extend(PublishedThread::from_node(node, &login));
        }
        match page["pageInfo"]["hasNextPage"].as_bool() {
            Some(false) => return Ok(threads),
            Some(true) => {
                cursor = Some(
                    page["pageInfo"]["endCursor"]
                        .as_str()
                        .ok_or("Missing review thread cursor")?
                        .to_string(),
                )
            }
            None => return Err("Missing review thread pagination".into()),
        }
    }
    Err("Review threads exceed the supported limit".into())
}

/// Resolves one review thread. GitHub answers an already resolved thread with success, so a
/// repeat is a no-op.
pub(crate) fn resolve_review_thread(thread_id: &str) -> Result<(), String> {
    let query = "mutation($thread:ID!) { resolveReviewThread(input:{threadId:$thread}) { thread { isResolved } } }";
    let value: serde_json::Value = gh_json(&[
        "api",
        "graphql",
        "-f",
        &format!("query={query}"),
        "-f",
        &format!("thread={thread_id}"),
    ])?;
    if value.get("errors").is_some()
        || value.pointer("/data/resolveReviewThread/thread/isResolved") != Some(&serde_json::Value::Bool(true))
    {
        return Err("GitHub did not resolve the review thread".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node(body: &str, author: &str) -> serde_json::Value {
        serde_json::json!({
            "id": "PRRT_1", "isResolved": false, "path": "src/a.rs", "line": null, "originalLine": 7,
            "comments": {"nodes": [{"databaseId": 42, "body": body, "author": {"login": author}}]}
        })
    }

    const FINDING: &str =
        "<!-- ocr:fingerprint=fnv64:0123 -->\n<sub>🤖 Automated review by OpenCodeReview</sub>\n\n**[Finding] Title**";

    #[test]
    fn own_finding_thread_is_eligible_and_keeps_its_identity() {
        let thread = PublishedThread::from_node(&node(FINDING, "Octocat"), "octocat").unwrap();
        assert_eq!(
            (
                thread.comment_id,
                thread.fingerprint.as_str(),
                thread.line,
                thread.resolved
            ),
            (42, "fnv64:0123", 7, false)
        );
    }

    #[test]
    fn human_foreign_question_and_unmarked_threads_are_never_eligible() {
        let question = FINDING.replace("[Finding]", "[Question]");
        let unmarked = "**[Finding] Title** written by a person";
        for (body, author) in [
            (FINDING, "someone-else"),
            (question.as_str(), "octocat"),
            (unmarked, "octocat"),
        ] {
            assert!(PublishedThread::from_node(&node(body, author), "octocat").is_none());
        }
    }
}
