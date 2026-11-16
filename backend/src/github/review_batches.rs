//! Paces a review's inline comments under GitHub's secondary rate limit on content creation
//! (about 80 a minute), where each inline comment counts on its own.

use std::time::Duration;

/// Inline comments in one posted review.
pub(crate) const BATCH_COMMENTS: usize = 40;
/// Pause between two reviews of one publication, so two batches never share a minute.
pub(crate) const BATCH_PAUSE: Duration = Duration::from_secs(65);
/// Waits before each new try of a review GitHub refused with its secondary rate limit.
pub(crate) const RATE_LIMIT_WAITS: [Duration; 3] = [
    Duration::from_secs(60),
    Duration::from_secs(120),
    Duration::from_secs(240),
];

const POST_MARKER: &str = "<!-- ocr-review-post:v1 ";
const PART_MARKER: &str = "<!-- ocr-review-post-part:v1 ";

/// The marker of the earlier batches of the review that carries `post_marker`. It never contains
/// the post marker itself, so finding the post marker still means the last review landed.
pub(crate) fn part_marker(post_marker: &str) -> Option<String> {
    post_marker
        .strip_prefix(POST_MARKER)
        .map(|rest| format!("{PART_MARKER}{rest}"))
}

fn post_marker_in(body: &str) -> Option<&str> {
    let start = body.find(POST_MARKER)?;
    let end = start + body[start..].find("-->")? + 3;
    Some(&body[start..end])
}

pub(crate) trait ReviewPoster {
    fn post(&mut self, payload_json: &str) -> Result<Option<i64>, String>;
    fn wait(&mut self, duration: Duration);
}

pub(crate) fn is_secondary_rate_limit(error: &str) -> bool {
    let error = error.to_ascii_lowercase();
    error.contains("secondary rate limit") || error.contains("abuse detection")
}

/// Posts the review as is up to `BATCH_COMMENTS` inline comments. Past that, the earlier batches go
/// first as plain comment reviews and the review with the body, the event and the publication
/// marker goes last, so finding the marker still means every batch landed. A retry after a failed
/// batch is safe: publication drops the comments GitHub already holds before it builds the payload.
pub(crate) fn post_review_batches(poster: &mut impl ReviewPoster, payload_json: &str) -> Result<Option<i64>, String> {
    let payload = serde_json::from_str::<serde_json::Value>(payload_json).map_err(|error| error.to_string())?;
    let comments = payload
        .get("comments")
        .and_then(|value| value.as_array())
        .cloned()
        .unwrap_or_default();
    if comments.len() <= BATCH_COMMENTS {
        return post_with_rate_limit(poster, payload_json);
    }
    let commit_id = payload.get("commit_id").cloned().unwrap_or_default();
    let part_marker = payload
        .get("body")
        .and_then(|body| body.as_str())
        .and_then(post_marker_in)
        .and_then(part_marker)
        .map(|marker| format!("\n\n{marker}"))
        .unwrap_or_default();
    let batches = comments.chunks(BATCH_COMMENTS).collect::<Vec<_>>();
    let total = batches.len();
    for (index, batch) in batches[..total - 1].iter().enumerate() {
        if index > 0 {
            poster.wait(BATCH_PAUSE);
        }
        let body = format!(
            "OpenCodeReview inline comments, part {} of {total}. The review summary is in part {total}.{part_marker}",
            index + 1
        );
        let part = serde_json::json!({ "commit_id": commit_id, "event": "COMMENT", "body": body, "comments": batch });
        post_with_rate_limit(poster, &part.to_string())?;
    }
    poster.wait(BATCH_PAUSE);
    let mut last = payload;
    last["comments"] = serde_json::Value::Array(batches[total - 1].to_vec());
    post_with_rate_limit(poster, &last.to_string())
}

/// GitHub creates nothing when it refuses a request with the secondary rate limit, so the same
/// review is sent again after each wait.
fn post_with_rate_limit(poster: &mut impl ReviewPoster, payload_json: &str) -> Result<Option<i64>, String> {
    let mut waits = RATE_LIMIT_WAITS.iter();
    loop {
        match poster.post(payload_json) {
            Err(error) if is_secondary_rate_limit(&error) => match waits.next() {
                Some(wait) => poster.wait(*wait),
                None => {
                    return Err(format!(
                        "{error} OpenCodeReview tried again for {} minutes; publish again later.",
                        RATE_LIMIT_WAITS.iter().map(Duration::as_secs).sum::<u64>() / 60
                    ))
                }
            },
            result => return result,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Default)]
    struct Fake {
        posts: Vec<serde_json::Value>,
        waits: Vec<u64>,
        refusals: Vec<String>,
    }

    impl ReviewPoster for Fake {
        fn post(&mut self, payload_json: &str) -> Result<Option<i64>, String> {
            if !self.refusals.is_empty() {
                return Err(self.refusals.remove(0));
            }
            self.posts.push(serde_json::from_str(payload_json).unwrap());
            Ok(Some(self.posts.len() as i64))
        }
        fn wait(&mut self, duration: Duration) {
            self.waits.push(duration.as_secs());
        }
    }

    const LIMIT: &str = "gh: You have exceeded a secondary rate limit and have been temporarily blocked from content creation. (HTTP 403)";

    fn payload(comments: usize) -> String {
        let comments = (0..comments)
            .map(|line| serde_json::json!({ "path": "a.rs", "line": line + 1, "body": format!("c{line}") }))
            .collect::<Vec<_>>();
        serde_json::json!({ "commit_id": "abc", "event": "REQUEST_CHANGES", "body": "summary\n\n<!-- ocr-review-post:v1 -->", "comments": comments })
            .to_string()
    }

    fn count(post: &serde_json::Value) -> usize {
        post["comments"].as_array().unwrap().len()
    }

    #[test]
    fn small_review_posts_once_unchanged() {
        let mut fake = Fake::default();
        let json = payload(BATCH_COMMENTS);
        assert_eq!(post_review_batches(&mut fake, &json).unwrap(), Some(1));
        assert_eq!(
            fake.posts,
            vec![serde_json::from_str::<serde_json::Value>(&json).unwrap()]
        );
        assert!(fake.waits.is_empty());
    }

    #[test]
    fn large_review_posts_batches_first_and_the_marked_review_last() {
        let mut fake = Fake::default();
        assert_eq!(post_review_batches(&mut fake, &payload(84)).unwrap(), Some(3));
        assert_eq!(fake.posts.iter().map(count).collect::<Vec<_>>(), vec![40, 40, 4]);
        for part in &fake.posts[..2] {
            assert_eq!(part["event"], "COMMENT");
            assert_eq!(part["commit_id"], "abc");
            let body = part["body"].as_str().unwrap();
            assert!(!body.contains("<!-- ocr-review-post:v1") && body.contains("<!-- ocr-review-post-part:v1 -->"));
        }
        assert_eq!(fake.posts[0]["comments"][0]["body"], "c0");
        assert_eq!(fake.posts[2]["event"], "REQUEST_CHANGES");
        assert!(fake.posts[2]["body"]
            .as_str()
            .unwrap()
            .contains("<!-- ocr-review-post:v1 -->"));
        assert_eq!(fake.posts[2]["comments"][3]["body"], "c83");
        assert_eq!(fake.waits, vec![65, 65]);
    }

    #[test]
    fn secondary_rate_limit_waits_and_sends_the_same_review_again() {
        let mut fake = Fake {
            refusals: vec![LIMIT.into(), LIMIT.into()],
            ..Fake::default()
        };
        assert_eq!(post_review_batches(&mut fake, &payload(3)).unwrap(), Some(1));
        assert_eq!(fake.waits, vec![60, 120]);
        assert_eq!(count(&fake.posts[0]), 3);
    }

    #[test]
    fn rate_limit_past_every_wait_fails_with_guidance() {
        let mut fake = Fake {
            refusals: vec![LIMIT.into(); 4],
            ..Fake::default()
        };
        let error = post_review_batches(&mut fake, &payload(3)).unwrap_err();
        assert!(
            error.contains("secondary rate limit") && error.contains("7 minutes"),
            "{error}"
        );
        assert_eq!(fake.waits, vec![60, 120, 240]);
        assert!(fake.posts.is_empty());
    }

    #[test]
    fn other_errors_fail_at_once_and_stop_later_batches() {
        let mut fake = Fake {
            refusals: vec!["HTTP 422: Unprocessable".into()],
            ..Fake::default()
        };
        assert!(post_review_batches(&mut fake, &payload(84)).is_err());
        assert!(fake.posts.is_empty() && fake.waits.is_empty());
    }
}
