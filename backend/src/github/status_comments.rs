use super::*;

#[derive(Clone, Debug)]
pub(crate) struct StatusComment {
    pub id: i64,
    pub author_id: i64,
    pub body: String,
}

pub(crate) trait StatusCommentApi {
    fn publisher_id(&mut self) -> Result<i64, String>;
    fn list(&mut self, repo: &str, number: i64) -> Result<Vec<StatusComment>, String>;
    fn post(&mut self, repo: &str, number: i64, body: &str) -> Result<i64, String>;
    fn delete(&mut self, repo: &str, id: i64) -> Result<(), String>;
}

pub(crate) struct GitHubStatusComments;

/// Owns the request payload file so every exit path removes it.
struct StatusCommentPayload(PathBuf);

impl StatusCommentPayload {
    fn new(body: &str) -> Result<Self, String> {
        static NEXT_PAYLOAD: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or_default();
        let path = std::env::temp_dir().join(format!(
            "ocr-status-comment-{}-{stamp}-{}.json",
            std::process::id(),
            NEXT_PAYLOAD.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&path).map_err(|e| e.to_string())?;
        let payload = Self(path);
        let bytes = serde_json::to_vec(&serde_json::json!({ "body": body })).map_err(|e| e.to_string())?;
        file.write_all(&bytes).map_err(|e| e.to_string())?;
        Ok(payload)
    }
}

impl Drop for StatusCommentPayload {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

impl StatusCommentApi for GitHubStatusComments {
    fn publisher_id(&mut self) -> Result<i64, String> {
        let value: serde_json::Value =
            serde_json::from_slice(&transport::gh_capture(&["api", "user"])?).map_err(|e| e.to_string())?;
        value
            .get("id")
            .and_then(|v| v.as_i64())
            .ok_or_else(|| "GitHub publisher identity unavailable".to_string())
    }

    fn list(&mut self, repo: &str, number: i64) -> Result<Vec<StatusComment>, String> {
        parse_status_comments(&transport::gh_capture(&[
            "api",
            "--paginate",
            "--slurp",
            &format!("repos/{repo}/issues/{number}/comments?per_page=100"),
        ])?)
    }

    fn post(&mut self, repo: &str, number: i64, body: &str) -> Result<i64, String> {
        let payload = StatusCommentPayload::new(body)?;
        let bytes = transport::gh_capture(&[
            "api",
            "--method",
            "POST",
            &format!("repos/{repo}/issues/{number}/comments"),
            "--input",
            &payload.0.to_string_lossy(),
        ])?;
        serde_json::from_slice::<serde_json::Value>(&bytes)
            .ok()
            .and_then(|value| value.get("id").and_then(|id| id.as_i64()))
            .ok_or_else(|| "GitHub did not return issue comment id".to_string())
    }

    fn delete(&mut self, repo: &str, id: i64) -> Result<(), String> {
        match transport::gh_capture(&[
            "api",
            "--method",
            "DELETE",
            &format!("repos/{repo}/issues/comments/{id}"),
        ]) {
            // Already gone (e.g. deleted by a concurrent publication).
            Err(error) if comment_already_deleted(&error) => Ok(()),
            result => result.map(|_| ()),
        }
    }
}

fn comment_already_deleted(error: &str) -> bool {
    error.contains("(HTTP 404)")
}

pub(crate) fn parse_status_comments(bytes: &[u8]) -> Result<Vec<StatusComment>, String> {
    let value: serde_json::Value = serde_json::from_slice(bytes).map_err(|e| e.to_string())?;
    let pages = value.as_array().ok_or("GitHub comment pages must be an array")?;
    let mut comments = Vec::new();
    for page in pages {
        for c in page.as_array().ok_or("GitHub comment page must be an array")? {
            comments.push(StatusComment {
                id: c.get("id").and_then(|v| v.as_i64()).ok_or("Missing comment identity")?,
                author_id: c
                    .pointer("/user/id")
                    .and_then(|v| v.as_i64())
                    .ok_or("Missing comment author")?,
                body: c
                    .get("body")
                    .and_then(|v| v.as_str())
                    .ok_or("Missing comment body")?
                    .into(),
            });
        }
    }
    Ok(comments)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_comments_paginated_parse_is_complete_and_fails_closed() {
        let page = |start: i64| {
            (start..start + 100)
                .map(|id| serde_json::json!({"id":id,"body":"status","user":{"id":7}}))
                .collect::<Vec<_>>()
        };
        let raw = serde_json::to_vec(&serde_json::json!([page(1), page(101)])).unwrap();
        let comments = parse_status_comments(&raw).unwrap();
        assert_eq!(comments.len(), 200);
        assert_eq!(comments[199].id, 200);
        assert_eq!(comments[199].author_id, 7);
        assert!(parse_status_comments(br#"[[{"id":1,"body":"missing author"}]]"#).is_err());
        assert!(parse_status_comments(br#"{"message":"API unavailable"}"#).is_err());
    }

    #[test]
    fn comment_already_deleted_matches_gh_404_only() {
        assert!(comment_already_deleted("gh: Not Found (HTTP 404)"));
        assert!(!comment_already_deleted(
            "Delete \"https://api.github.com/repos/o/r/issues/comments/14049\": dial tcp: i/o timeout"
        ));
        assert!(!comment_already_deleted("gh: Server Error (HTTP 500)"));
        assert!(!comment_already_deleted("comment not found"));
    }
}
