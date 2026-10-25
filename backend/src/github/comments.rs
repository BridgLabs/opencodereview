use super::*;

/// Post a review to a PR via the GitHub REST API (write). `payload_json` is the
/// `pulls/{n}/reviews` body: `{ commit_id, event, body, comments: [...] }`.
/// Requires `api.github.com` (there is no git-only way to post a review).
/// Returns the created review's id (so OpenCodeReview can later dismiss its OWN review).
#[derive(Debug, Clone)]
pub struct IssueCommentUpsert {
    pub id: i64,
    pub created: bool,
}

/// Owns the payload file so every exit path removes it, including write errors.
pub struct PreparedReview {
    path: std::path::PathBuf,
}

impl Drop for PreparedReview {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

impl PreparedReview {
    pub(crate) fn in_directory(payload_json: &str, directory: &std::path::Path) -> Result<Self, String> {
        static NEXT_PAYLOAD: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        Self::with_paths(payload_json, || {
            let stamp = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or_default();
            let sequence = NEXT_PAYLOAD.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            directory.join(format!("ocr-review-{}-{stamp}-{sequence}.json", std::process::id()))
        })
    }

    fn with_paths(payload_json: &str, mut next_path: impl FnMut() -> std::path::PathBuf) -> Result<Self, String> {
        use std::io::Write;
        for _ in 0..16 {
            let path = next_path();
            let mut options = std::fs::OpenOptions::new();
            options.write(true).create_new(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o600);
            }
            let mut file = match options.open(&path) {
                Ok(file) => file,
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(error.to_string()),
            };
            // Only exclusive creation grants ownership of the path for cleanup.
            let prepared = Self { path };
            let written = file.write_all(payload_json.as_bytes());
            drop(file);
            written.map_err(|error| error.to_string())?;
            return Ok(prepared);
        }
        Err("Could not create a unique review payload after 16 attempts".into())
    }
}

pub fn prepare_review(payload_json: &str) -> Result<PreparedReview, String> {
    PreparedReview::in_directory(payload_json, &std::env::temp_dir())
}

/// Execute the publication command only after local payload preparation succeeds.
pub fn post_review(full_name: &str, number: i64, prepared: PreparedReview) -> Result<Option<i64>, String> {
    let route = format!("repos/{full_name}/pulls/{number}/reviews");
    let path_str = prepared.path.to_string_lossy().to_string();
    let result = run_raw("gh", &["api", "--method", "POST", &route, "--input", &path_str]);
    let bytes = result.map_err(|error| format!("could not post review (GitHub API): {error}"))?;
    let review_id = serde_json::from_slice::<serde_json::Value>(&bytes)
        .ok()
        .and_then(|value| value.get("id").and_then(|id| id.as_i64()));
    Ok(review_id)
}

pub fn find_review_id_by_marker(full_name: &str, number: i64, marker: &str) -> Result<Option<i64>, String> {
    let route = format!("repos/{full_name}/pulls/{number}/reviews?per_page=100");
    let bytes = transport::gh_capture(&["api", "--paginate", "--slurp", &route])?;
    let pages = serde_json::from_slice::<serde_json::Value>(&bytes)
        .map_err(|error| error.to_string())?
        .as_array()
        .cloned()
        .unwrap_or_default();
    Ok(pages
        .into_iter()
        .flat_map(|page| page.as_array().cloned().unwrap_or_default())
        .find_map(|review| {
            review
                .get("body")
                .and_then(|body| body.as_str())
                .is_some_and(|body| body.contains(marker))
                .then(|| review.get("id").and_then(|id| id.as_i64()))
                .flatten()
        }))
}

pub fn upsert_issue_comment_by_marker(
    full_name: &str,
    number: i64,
    marker: &str,
    body: &str,
) -> Result<IssueCommentUpsert, String> {
    let existing = find_issue_comment_id_by_marker(full_name, number, marker)?;
    write_issue_comment(full_name, number, existing, body, "PR summary")
}

pub(crate) fn write_issue_comment(
    full_name: &str,
    number: i64,
    existing: Option<i64>,
    body: &str,
    kind: &str,
) -> Result<IssueCommentUpsert, String> {
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or_default();
    let path = std::env::temp_dir().join(format!("ocr-issue-comment-{}-{stamp}.json", std::process::id()));
    std::fs::write(&path, serde_json::json!({ "body": body }).to_string()).map_err(|error| error.to_string())?;
    let path_str = path.to_string_lossy().to_string();
    let (method, route, created) = if let Some(id) = existing {
        ("PATCH", format!("repos/{full_name}/issues/comments/{id}"), false)
    } else {
        ("POST", format!("repos/{full_name}/issues/{number}/comments"), true)
    };
    let result = run_raw("gh", &["api", "--method", method, &route, "--input", &path_str]);
    let _ = std::fs::remove_file(&path);
    let bytes = result.map_err(|error| format!("could not upsert {kind} comment (GitHub API): {error}"))?;
    let id = serde_json::from_slice::<serde_json::Value>(&bytes)
        .ok()
        .and_then(|value| value.get("id").and_then(|id| id.as_i64()))
        .ok_or_else(|| "GitHub did not return issue comment id".to_string())?;
    Ok(IssueCommentUpsert { id, created })
}

pub fn find_issue_comment_id_by_marker(full_name: &str, number: i64, marker: &str) -> Result<Option<i64>, String> {
    let route = format!("repos/{full_name}/issues/{number}/comments?per_page=100");
    let bytes = run_raw("gh", &["api", "--paginate", "--slurp", &route])?;
    let pages = serde_json::from_slice::<serde_json::Value>(&bytes)
        .map_err(|error| error.to_string())?
        .as_array()
        .cloned()
        .unwrap_or_default();
    let comments = pages
        .into_iter()
        .flat_map(|page| page.as_array().cloned().unwrap_or_default());
    Ok(comments.into_iter().find_map(|comment| {
        let body = comment.get("body").and_then(|v| v.as_str()).unwrap_or_default();
        body.contains(marker)
            .then(|| comment.get("id").and_then(|v| v.as_i64()))
            .flatten()
    }))
}

const OCR_FINGERPRINT_PREFIX: &str = "<!-- ocr:fingerprint=";

#[derive(Debug, Clone)]
pub struct DedupeResult {
    pub post_comments: serde_json::Value,
    pub annotated_comments_json: String,
    pub suppressed_count: usize,
}

pub(crate) fn normalize_finding_text(value: &str) -> String {
    value
        .lines()
        .filter(|line| !line.trim_start().starts_with("<!-- ocr:fingerprint="))
        .collect::<Vec<_>>()
        .join("\n")
        .to_lowercase()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

pub(crate) fn ocr_fingerprint(path: &str, line: i64, body: &str) -> String {
    let input = format!("{}:{}:{}", path, line, normalize_finding_text(body));
    let mut hash = 0xcbf29ce484222325u64;
    for byte in input.as_bytes() {
        hash ^= *byte as u64;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("fnv64:{hash:016x}")
}

pub(crate) fn extract_ocr_fingerprint(body: &str) -> Option<String> {
    let start = body.find(OCR_FINGERPRINT_PREFIX)? + OCR_FINGERPRINT_PREFIX.len();
    let rest = &body[start..];
    let end = rest.find(" -->")?;
    Some(rest[..end].trim().to_string())
}

pub(crate) fn is_ocr_comment(body: &str) -> bool {
    extract_ocr_fingerprint(body).is_some() || crate::review::contains_review_attribution(body)
}

pub(crate) fn with_fingerprint_marker(path: &str, line: i64, body: &str) -> (String, String) {
    if let Some(fingerprint) = extract_ocr_fingerprint(body) {
        return (fingerprint, body.to_string());
    }
    let fingerprint = ocr_fingerprint(path, line, body);
    (
        fingerprint.clone(),
        format!("<!-- ocr:fingerprint={fingerprint} -->\n{body}"),
    )
}

pub(crate) fn github_review_comment(comment: &serde_json::Value, body: String) -> serde_json::Value {
    let mut obj = serde_json::Map::new();
    for key in ["path", "line", "side", "start_line", "start_side"] {
        if let Some(value) = comment.get(key) {
            obj.insert(key.to_string(), value.clone());
        }
    }
    obj.insert("body".to_string(), serde_json::json!(body));
    serde_json::Value::Object(obj)
}

pub fn dedupe_review_comments(full_name: &str, number: i64, comments: serde_json::Value) -> DedupeResult {
    let existing = existing_ocr_comment_keys(full_name, number);
    let mut post_comments = Vec::new();
    let mut annotated = Vec::new();
    let mut suppressed_count = 0usize;
    for value in comments.as_array().cloned().unwrap_or_default() {
        let mut comment = value;
        let path = comment
            .get("path")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string();
        let line = comment.get("line").and_then(|v| v.as_i64()).unwrap_or_default();
        let body = comment
            .get("body")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string();
        let (fingerprint, body_with_marker) = with_fingerprint_marker(&path, line, &body);
        let loc_key = format!("{}:{}", path, line);
        if let Some(duplicate) = existing.get(&fingerprint).or_else(|| existing.get(&loc_key)) {
            if let Some(obj) = comment.as_object_mut() {
                obj.insert("status".to_string(), serde_json::json!("duplicate_suppressed"));
                obj.insert("fingerprint".to_string(), serde_json::json!(fingerprint));
                obj.insert("duplicateUrl".to_string(), serde_json::json!(duplicate.url));
                obj.insert("duplicateAuthor".to_string(), serde_json::json!(duplicate.author));
            }
            suppressed_count += 1;
            annotated.push(comment);
        } else {
            let post_comment = github_review_comment(&comment, body_with_marker.clone());
            if let Some(obj) = comment.as_object_mut() {
                obj.insert("body".to_string(), serde_json::json!(body_with_marker));
                obj.insert("status".to_string(), serde_json::json!("new"));
                obj.insert("fingerprint".to_string(), serde_json::json!(fingerprint));
            }
            post_comments.push(post_comment);
            annotated.push(comment);
        }
    }
    DedupeResult {
        post_comments: serde_json::Value::Array(post_comments),
        annotated_comments_json: serde_json::json!({ "comments": annotated }).to_string(),
        suppressed_count,
    }
}

#[derive(Debug, Clone)]
pub(crate) struct ExistingOcrComment {
    author: String,
    url: String,
}

pub(crate) fn existing_ocr_comment_keys(full_name: &str, number: i64) -> HashMap<String, ExistingOcrComment> {
    let route = format!("repos/{full_name}/pulls/{number}/comments?per_page=100");
    let bytes = match run_raw("gh", &["api", "--paginate", &route]) {
        Ok(bytes) => bytes,
        Err(_) => return HashMap::new(),
    };
    let comments = match serde_json::from_slice::<serde_json::Value>(&bytes) {
        Ok(value) => value.as_array().cloned().unwrap_or_default(),
        Err(_) => return HashMap::new(),
    };
    let mut out = HashMap::new();
    for comment in comments {
        let body = comment.get("body").and_then(|v| v.as_str()).unwrap_or_default();
        if !is_ocr_comment(body) {
            continue;
        }
        let path = comment.get("path").and_then(|v| v.as_str()).unwrap_or_default();
        let line = comment.get("line").and_then(|v| v.as_i64()).unwrap_or_default();
        let item = ExistingOcrComment {
            author: comment
                .pointer("/user/login")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown")
                .to_string(),
            url: comment
                .get("html_url")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string(),
        };
        if let Some(fingerprint) = extract_ocr_fingerprint(body) {
            out.insert(fingerprint, item.clone());
        }
        out.insert(format!("{}:{}", path, line), item);
    }
    out
}

#[cfg(test)]
mod prepared_review_tests {
    use super::PreparedReview;

    #[test]
    fn prepared_review_retries_collision_without_overwriting_or_removing_existing_file() {
        let existing = super::prepare_review("original").unwrap();
        let unused = super::prepare_review("placeholder").unwrap();
        let available = unused.path.clone();
        drop(unused);
        let mut paths = [existing.path.clone(), available.clone()].into_iter();
        let prepared = PreparedReview::with_paths("new payload", || paths.next().unwrap()).unwrap();
        assert_eq!(prepared.path, available);
        assert_eq!(std::fs::read_to_string(&existing.path).unwrap(), "original");
        assert_eq!(std::fs::read_to_string(&prepared.path).unwrap(), "new payload");
        drop(prepared);
        assert!(!available.exists());
        assert_eq!(std::fs::read_to_string(&existing.path).unwrap(), "original");
    }

    #[test]
    fn prepared_review_exhausted_collisions_leave_existing_file_untouched() {
        let existing = super::prepare_review("original").unwrap();
        let mut attempts = 0;
        let result = PreparedReview::with_paths("replacement", || {
            attempts += 1;
            existing.path.clone()
        });
        assert!(result.is_err());
        assert_eq!(attempts, 16);
        assert_eq!(std::fs::read_to_string(&existing.path).unwrap(), "original");
    }

    #[test]
    fn prepared_review_concurrent_payloads_have_independent_contents_and_cleanup() {
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(16));
        let handles: Vec<_> = (0..16)
            .map(|id| {
                let barrier = barrier.clone();
                std::thread::spawn(move || {
                    barrier.wait();
                    let body = format!("payload-{id}");
                    (super::prepare_review(&body).unwrap(), body)
                })
            })
            .collect();
        let mut prepared: Vec<_> = handles.into_iter().map(|handle| handle.join().unwrap()).collect();
        let paths: std::collections::HashSet<_> = prepared.iter().map(|(file, _)| file.path.clone()).collect();
        assert_eq!(paths.len(), 16);
        while let Some((file, body)) = prepared.pop() {
            assert_eq!(std::fs::read_to_string(&file.path).unwrap(), body);
            let path = file.path.clone();
            drop(file);
            assert!(!path.exists());
            for (remaining, expected) in &prepared {
                assert_eq!(&std::fs::read_to_string(&remaining.path).unwrap(), expected);
            }
        }
    }
}
