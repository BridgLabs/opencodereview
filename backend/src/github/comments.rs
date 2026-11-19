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
pub fn post_review(full_name: &str, number: i64, prepared: PreparedReview, login: &str) -> Result<Option<i64>, String> {
    struct Gh<'a> {
        route: String,
        login: &'a str,
        path: &'a std::path::Path,
    }
    impl super::review_batches::ReviewPoster for Gh<'_> {
        fn post(&mut self, payload_json: &str) -> Result<Option<i64>, String> {
            std::fs::write(self.path, payload_json).map_err(|error| error.to_string())?;
            let path = self.path.to_string_lossy();
            let bytes = run_raw("gh", &["api", "--method", "POST", &self.route, "--input", &path])
                .map_err(|error| format!("could not post review (GitHub API): {error}"))?;
            Ok(serde_json::from_slice::<serde_json::Value>(&bytes)
                .ok()
                .and_then(|value| value.get("id").and_then(|id| id.as_i64())))
        }
        fn find_posted(&mut self, needles: &[&str]) -> Result<Option<i64>, String> {
            let bytes = capture_review_pages(&self.route)?;
            find_review_id_in_pages(&bytes, self.login, needles)
        }
        fn wait(&mut self, duration: std::time::Duration) {
            std::thread::sleep(duration);
        }
    }
    let payload = std::fs::read_to_string(&prepared.path).map_err(|error| error.to_string())?;
    let mut gh = Gh {
        route: format!("repos/{full_name}/pulls/{number}/reviews"),
        login,
        path: &prepared.path,
    };
    super::review_batches::post_review_batches(&mut gh, &payload)
}

pub fn find_review_id_by_marker(full_name: &str, number: i64, marker: &str) -> Result<Option<i64>, String> {
    let bytes = capture_review_pages(&format!("repos/{full_name}/pulls/{number}/reviews"))?;
    let pages = serde_json::from_slice::<serde_json::Value>(&bytes).map_err(|error| error.to_string())?;
    Ok(pages
        .as_array()
        .into_iter()
        .flatten()
        .flat_map(|page| page.as_array().into_iter().flatten())
        .find_map(|review| {
            review["body"]
                .as_str()
                .filter(|body| body.contains(marker))
                .and_then(|_| review["id"].as_i64())
        }))
}

fn capture_review_pages(route: &str) -> Result<Vec<u8>, String> {
    transport::gh_capture(&["api", "--paginate", "--slurp", &format!("{route}?per_page=100")])
}

/// The first review submitted by `login` whose body holds every needle.
fn find_review_id_in_pages(bytes: &[u8], login: &str, needles: &[&str]) -> Result<Option<i64>, String> {
    #[derive(Deserialize)]
    struct User {
        login: String,
    }
    #[derive(Deserialize)]
    struct Review {
        id: i64,
        #[serde(deserialize_with = "Deserialize::deserialize")]
        body: Option<String>,
        #[serde(deserialize_with = "Deserialize::deserialize")]
        user: Option<User>,
        state: String,
    }

    // An unexpected response is not proof that GitHub did not create the review.
    let pages: Vec<Vec<Review>> = serde_json::from_slice(bytes).map_err(|error| error.to_string())?;
    Ok(pages.into_iter().flatten().find_map(|review| {
        let by_publisher = review.user.is_some_and(|user| user.login.eq_ignore_ascii_case(login));
        let body = review.body.filter(|_| by_publisher && review.state != "PENDING")?;
        needles.iter().all(|needle| body.contains(needle)).then_some(review.id)
    }))
}

#[cfg(test)]
mod review_lookup_tests {
    use super::find_review_id_in_pages;

    const MARKER: &str = "<!-- ocr-review-post-part:v1 run=7 -->";

    fn review(id: i64, login: &str, body: String) -> serde_json::Value {
        serde_json::json!({"id": id, "body": body, "state": "COMMENTED", "user": {"login": login}})
    }

    #[test]
    fn batch_lookup_requires_part_label_and_marker_across_pages() {
        let pages = serde_json::json!([
            [
                review(1, "ocr", format!("part 1 of 3.\n\n{MARKER}")),
                review(2, "ocr", format!("part 2 of 30.\n\n{MARKER}")),
                review(
                    3,
                    "ocr",
                    "part 2 of 3.\n\n<!-- ocr-review-post-part:v1 run=8 -->".into()
                )
            ],
            [review(4, "ocr", format!("part 2 of 3.\n\n{MARKER}"))]
        ]);
        let bytes = serde_json::to_vec(&pages).unwrap();
        assert_eq!(
            find_review_id_in_pages(&bytes, "ocr", &[MARKER, "part 2 of 3."]),
            Ok(Some(4))
        );
        assert_eq!(
            find_review_id_in_pages(&bytes, "ocr", &[MARKER, "part 3 of 3."]),
            Ok(None)
        );
        assert_eq!(find_review_id_in_pages(&bytes, "ocr", &[MARKER]), Ok(Some(1)));
    }

    #[test]
    fn batch_lookup_rejects_malformed_pages_as_unconfirmed() {
        for bytes in [
            r#"{"message":"Server error"}"#,
            "[null]",
            r#"[[{"body":"unknown review"}]]"#,
            r#"[[{"id":42}]]"#,
            r#"[[{"id":42,"body":"<!-- ocr-review-post-part:v1 run=7 -->","user":{"login":"ocr"}}]]"#,
        ] {
            assert!(
                find_review_id_in_pages(bytes.as_bytes(), "ocr", &[MARKER]).is_err(),
                "accepted {bytes}"
            );
        }
        assert_eq!(find_review_id_in_pages(b"[[]]", "ocr", &[MARKER]), Ok(None));
    }

    #[test]
    fn batch_lookup_accepts_only_submitted_reviews_by_the_publisher() {
        for needles in [&[MARKER, "part 1 of 2."][..], &["<!-- ocr-review-post:v1 run=7 -->"]] {
            let body = || needles.join("\n\n");
            let pending = serde_json::json!({"id": 3, "body": body(), "state": "PENDING", "user": {"login": "OCR"}});
            let ghost = serde_json::json!({"id": 5, "body": body(), "state": "COMMENTED", "user": null});
            let pages = serde_json::json!([
                [review(1, "someone", body()), pending, ghost],
                [review(4, "OCR", body())]
            ]);
            let bytes = serde_json::to_vec(&pages).unwrap();
            assert_eq!(find_review_id_in_pages(&bytes, "ocr", needles), Ok(Some(4)));
            let other = serde_json::to_vec(&serde_json::json!([[review(1, "someone", body())]])).unwrap();
            assert_eq!(find_review_id_in_pages(&other, "ocr", needles), Ok(None));
        }
    }
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
    dedupe_against(&existing_ocr_comments(full_name, number), comments)
}

/// The `[Finding]` or `[Question]` title of a rendered OpenCodeReview comment.
fn point_title(body: &str) -> Option<&str> {
    let start = ["**[Finding] ", "**[Question] "]
        .iter()
        .find_map(|marker| body.find(marker).map(|at| at + marker.len()))?;
    let end = body[start..].find("**\n").or_else(|| body[start..].find("**"))?;
    Some(body[start..start + end].trim())
}

/// Whether an existing comment already carries this point: the same fingerprint, or the same
/// title in the same file (where a later push may have moved it), or a close rewording within
/// three lines. A different point on the same line is not a duplicate.
fn already_posted<'a>(
    existing: &'a [ExistingOcrComment],
    fingerprint: &str,
    path: &str,
    line: i64,
    body: &str,
) -> Option<&'a ExistingOcrComment> {
    let title = point_title(body);
    existing.iter().find(|comment| {
        comment.fingerprint.as_deref() == Some(fingerprint)
            || title.zip(comment.title.as_deref()).is_some_and(|(title, other)| {
                crate::review::evidence::same_point((&comment.path, comment.line, other), (path, line, title))
            })
    })
}

pub(crate) fn dedupe_against(existing: &[ExistingOcrComment], comments: serde_json::Value) -> DedupeResult {
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
        if let Some(duplicate) = already_posted(existing, &fingerprint, &path, line, &body) {
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
    path: String,
    line: i64,
    fingerprint: Option<String>,
    title: Option<String>,
}

impl ExistingOcrComment {
    fn from_github(comment: &serde_json::Value) -> Option<Self> {
        let body = comment.get("body").and_then(|v| v.as_str()).unwrap_or_default();
        if !is_ocr_comment(body) {
            return None;
        }
        let field = |pointer: &str| comment.pointer(pointer).and_then(|v| v.as_str());
        Some(Self {
            author: field("/user/login").unwrap_or("unknown").to_string(),
            url: field("/html_url").unwrap_or_default().to_string(),
            path: field("/path").unwrap_or_default().to_string(),
            // An outdated comment has no current line; its original one still places it.
            line: ["/line", "/original_line"]
                .iter()
                .find_map(|pointer| comment.pointer(pointer).and_then(|v| v.as_i64()))
                .unwrap_or_default(),
            fingerprint: extract_ocr_fingerprint(body),
            title: point_title(body).map(str::to_string),
        })
    }
}

pub(crate) fn existing_ocr_comments(full_name: &str, number: i64) -> Vec<ExistingOcrComment> {
    let route = format!("repos/{full_name}/pulls/{number}/comments?per_page=100");
    let Ok(bytes) = run_raw("gh", &["api", "--paginate", &route]) else {
        return Vec::new();
    };
    let Ok(comments) = serde_json::from_slice::<serde_json::Value>(&bytes) else {
        return Vec::new();
    };
    comments
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(ExistingOcrComment::from_github)
        .collect()
}

#[cfg(test)]
mod dedupe_tests {
    use super::*;

    fn posted(path: &str, line: i64, title: &str) -> serde_json::Value {
        let body =
            format!("<!-- ocr:fingerprint=fnv64:{line:016x} -->\nOpenCodeReview\n\n**[Finding] {title}**\n\nDetail");
        serde_json::json!({"path": path, "line": line, "body": body, "html_url": format!("https://x/{path}/{line}"), "user": {"login": "me"}})
    }

    fn new_comment(path: &str, line: i64, title: &str) -> serde_json::Value {
        serde_json::json!({"path": path, "line": line, "side": "RIGHT", "body": format!("{}\n\n**[Finding] {title}**\n\nOther detail", crate::review::REVIEW_ATTRIBUTION)})
    }

    fn dedupe(existing: &[serde_json::Value], comments: Vec<serde_json::Value>) -> DedupeResult {
        let existing: Vec<_> = existing.iter().filter_map(ExistingOcrComment::from_github).collect();
        dedupe_against(&existing, serde_json::Value::Array(comments))
    }

    #[test]
    fn a_different_point_on_an_already_commented_line_is_posted() {
        let result = dedupe(
            &[posted("src/a.rs", 10, "Cache is never invalidated")],
            vec![new_comment("src/a.rs", 10, "Lock is held across the network call")],
        );
        assert_eq!(
            (result.suppressed_count, result.post_comments.as_array().unwrap().len()),
            (0, 1)
        );
    }

    #[test]
    fn the_same_point_moved_by_a_later_push_is_not_posted_again() {
        let result = dedupe(
            &[posted("src/a.rs", 10, "Cache is never invalidated")],
            vec![new_comment("src/a.rs", 42, "Cache is never invalidated")],
        );
        assert_eq!(result.suppressed_count, 1);
        assert!(result.annotated_comments_json.contains("https://x/src/a.rs/10"));
    }

    #[test]
    fn a_close_rewording_nearby_is_a_repeat_but_not_in_another_file() {
        let existing = [posted("src/a.rs", 10, "Cache entry is never invalidated after update")];
        let near = dedupe(
            &existing,
            vec![new_comment("src/a.rs", 12, "Cache entry never invalidated on update")],
        );
        assert_eq!(near.suppressed_count, 1);
        let other_file = dedupe(
            &existing,
            vec![new_comment(
                "src/b.rs",
                10,
                "Cache entry is never invalidated after update",
            )],
        );
        assert_eq!(other_file.suppressed_count, 0);
    }

    #[test]
    fn outdated_comments_keep_their_original_line_and_foreign_comments_are_ignored() {
        let mut outdated = posted("src/a.rs", 7, "Wrong default port");
        outdated["original_line"] = outdated["line"].take();
        let comment = ExistingOcrComment::from_github(&outdated).unwrap();
        assert_eq!(
            (comment.line, comment.title.as_deref()),
            (7, Some("Wrong default port"))
        );
        assert!(
            ExistingOcrComment::from_github(&serde_json::json!({"body": "LGTM", "path": "src/a.rs", "line": 7}))
                .is_none()
        );
    }
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
