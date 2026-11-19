//! Paces a review's inline comments under GitHub's secondary rate limit on content creation
//! (about 80 a minute), where each inline comment counts on its own.

use sha2::{Digest, Sha256};
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

const SERVER_ERROR_WAITS: [Duration; 3] = [BATCH_PAUSE, Duration::from_secs(120), Duration::from_secs(240)];

const POST_MARKER: &str = "<!-- ocr-review-post:v1 ";
const PART_MARKER: &str = "<!-- ocr-review-post-part:v1 ";
const BATCH_MARKER: &str = "<!-- ocr-review-batch:v1 ";

/// The marker of the earlier batches of the review that carries `post_marker`. It never contains
/// the post marker itself, so finding the post marker still means the last review landed.
pub(crate) fn part_marker(post_marker: &str) -> Option<String> {
    post_marker
        .strip_prefix(POST_MARKER)
        .map(|rest| format!("{PART_MARKER}{rest}"))
}

/// Identifies the exact comments of one earlier batch, so a retry never adopts a part with the same
/// run marker and label from an earlier publication of another selection.
fn batch_marker(comments: &[serde_json::Value]) -> Result<String, String> {
    let comments = serde_json::to_vec(comments).map_err(|error| error.to_string())?;
    Ok(format!("{BATCH_MARKER}sha256={:x} -->", Sha256::digest(comments)))
}

fn post_marker_in(body: &str) -> Option<&str> {
    let start = body.rfind(POST_MARKER)?;
    let end = start + body[start..].find("-->")? + 3;
    Some(&body[start..end])
}

pub(crate) trait ReviewPoster {
    fn post(&mut self, payload_json: &str) -> Result<Option<i64>, String>;
    /// The ID of a submitted review whose body holds every needle, if GitHub has one.
    fn find_posted(&mut self, needles: &[&str]) -> Result<Option<i64>, String>;
    fn wait(&mut self, duration: Duration);
}

pub(crate) fn is_secondary_rate_limit(error: &str) -> bool {
    let error = error.to_ascii_lowercase();
    error.contains("secondary rate limit") || error.contains("abuse detection")
}

fn is_retryable_server_error(error: &str) -> bool {
    // `gh api` prints `gh: <message> (HTTP <status>)` for a JSON error body and only
    // `gh: HTTP <status>` when the body is not JSON, such as a gateway's HTML 502 or 504 page.
    let status = error.lines().rev().find_map(|line| {
        let (_, message) = line.trim_end().rsplit_once("gh: ")?;
        match message.rsplit_once(" (HTTP ") {
            Some((_, status)) => status.strip_suffix(')'),
            None => message.strip_prefix("HTTP "),
        }
    });
    matches!(status, Some("500" | "502" | "503" | "504"))
}

/// Posts the review as is up to `BATCH_COMMENTS` inline comments. Past that, the earlier batches go
/// first as plain comment reviews and the review with the body, the event and the publication
/// marker goes last, so finding the marker still means every batch landed. Republishing after a
/// failed batch is safe: publication drops the comments GitHub already holds before building the payload.
pub(crate) fn post_review_batches(poster: &mut impl ReviewPoster, payload_json: &str) -> Result<Option<i64>, String> {
    let mut payload = serde_json::from_str::<serde_json::Value>(payload_json).map_err(|error| error.to_string())?;
    let comments = payload
        .get("comments")
        .and_then(|value| value.as_array())
        .cloned()
        .unwrap_or_default();
    let post_marker = payload["body"].as_str().and_then(post_marker_in).map(str::to_owned);
    let final_needles = post_marker.as_deref().into_iter().collect::<Vec<_>>();
    if comments.len() <= BATCH_COMMENTS {
        return post_with_retry(poster, payload_json, &final_needles);
    }
    let commit_id = payload.get("commit_id").cloned().unwrap_or_default();
    let part_marker = post_marker.as_deref().and_then(part_marker);
    let marker_suffix = part_marker
        .as_ref()
        .map(|marker| format!("\n\n{marker}"))
        .unwrap_or_default();
    let batches = comments.chunks(BATCH_COMMENTS).collect::<Vec<_>>();
    let total = batches.len();
    for (index, batch) in batches[..total - 1].iter().enumerate() {
        if index > 0 {
            poster.wait(BATCH_PAUSE);
        }
        let part_label = format!("part {} of {total}.", index + 1);
        let batch_marker = batch_marker(batch)?;
        let body = format!(
            "OpenCodeReview inline comments, {part_label} The review summary is in part {total}.{marker_suffix}\n{batch_marker}"
        );
        let part = serde_json::json!({ "commit_id": commit_id, "event": "COMMENT", "body": body, "comments": batch });
        let needles = part_marker
            .as_deref()
            .map(|marker| vec![marker, part_label.as_str(), batch_marker.as_str()])
            .unwrap_or_default();
        post_with_retry(poster, &part.to_string(), &needles)?;
    }
    poster.wait(BATCH_PAUSE);
    payload["comments"] = serde_json::Value::Array(batches[total - 1].to_vec());
    post_with_retry(poster, &payload.to_string(), &final_needles)
}

fn post_with_retry(
    poster: &mut impl ReviewPoster,
    payload_json: &str,
    needles: &[&str],
) -> Result<Option<i64>, String> {
    let mut rate_limit_waits = RATE_LIMIT_WAITS.iter();
    let mut server_waits = SERVER_ERROR_WAITS.into_iter();
    let mut failed_posts = 0;
    let mut waited = Duration::ZERO;
    // Set when the last scheduled lookup allowed a resend: that resend keeps one read-only check.
    let mut extra_check = false;
    'posting: loop {
        match poster.post(payload_json) {
            Err(error) if is_retryable_server_error(&error) => {
                if needles.is_empty() {
                    return Err(format!(
                        "{error} Cannot check whether the review was posted without a marker."
                    ));
                }
                failed_posts += 1;
                let mut lookup_error = None;
                loop {
                    let (wait, may_resend) = match server_waits.next() {
                        Some(wait) => (wait, true),
                        None if std::mem::take(&mut extra_check) => (BATCH_PAUSE, false),
                        None => break,
                    };
                    poster.wait(wait);
                    waited += wait;
                    match poster.find_posted(needles) {
                        Ok(Some(id)) => return Ok(Some(id)),
                        Ok(None) if may_resend && failed_posts < SERVER_ERROR_WAITS.len() => {
                            extra_check = server_waits.len() == 0;
                            continue 'posting;
                        }
                        // After three failed posts, the last lookup can recover the ID but cannot resend.
                        Ok(None) => {
                            lookup_error = None;
                            break;
                        }
                        Err(error) => lookup_error = Some(error),
                    }
                }
                let lookup_error = lookup_error
                    .map(|error| format!(" Could not check whether the review was posted: {error}."))
                    .unwrap_or_default();
                let minutes = waited.as_secs() / 60;
                return Err(format!(
                    "{error}{lookup_error} OpenCodeReview tried again for {minutes} minutes; publish again later."
                ));
            }
            // GitHub creates nothing on a secondary rate limit, so it is safe to resend.
            Err(error) if is_secondary_rate_limit(&error) => match rate_limit_waits.next() {
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
    use std::collections::VecDeque;

    #[derive(Default)]
    struct Fake {
        posts: Vec<serde_json::Value>,
        waits: Vec<u64>,
        refusals: Vec<String>,
        attempts: Vec<serde_json::Value>,
        outcomes: VecDeque<Result<Option<i64>, &'static str>>,
        /// Attempts GitHub stores even though it answers them with an error.
        stored_despite_error: Vec<usize>,
        /// Reviews by the publisher a lookup can find once `lookup_outcomes` runs out.
        github: Vec<(i64, String)>,
        found_reviews: Vec<i64>,
        search_github: bool,
        lookups: Vec<(usize, Vec<String>)>,
        lookup_outcomes: VecDeque<Result<Option<i64>, &'static str>>,
    }

    impl ReviewPoster for Fake {
        fn post(&mut self, payload_json: &str) -> Result<Option<i64>, String> {
            let payload = serde_json::from_str(payload_json).unwrap();
            self.attempts.push(payload);
            if !self.refusals.is_empty() {
                return Err(self.refusals.remove(0));
            }
            let attempt = self.attempts.len();
            let result = self
                .outcomes
                .pop_front()
                .unwrap_or(Ok(Some(self.posts.len() as i64 + 1)));
            if result.is_ok() || self.stored_despite_error.contains(&attempt) {
                let posted = self.attempts.last().unwrap().clone();
                let id = result.ok().flatten().unwrap_or(100 + attempt as i64);
                self.github
                    .push((id, posted["body"].as_str().unwrap_or_default().into()));
                self.posts.push(posted);
            }
            result.map_err(str::to_owned)
        }
        fn find_posted(&mut self, needles: &[&str]) -> Result<Option<i64>, String> {
            self.lookups.push((
                self.attempts.len(),
                needles.iter().map(|needle| needle.to_string()).collect(),
            ));
            if let Some(outcome) = self.lookup_outcomes.pop_front() {
                return outcome.map_err(str::to_owned);
            }
            assert!(self.search_github, "unexpected lookup");
            let id = self
                .github
                .iter()
                .find(|(_, body)| needles.iter().all(|needle| body.contains(needle)))
                .map(|(id, _)| *id);
            self.found_reviews.extend(id);
            Ok(id)
        }
        fn wait(&mut self, duration: Duration) {
            self.waits.push(duration.as_secs());
        }
    }

    const LIMIT: &str = "gh: You have exceeded a secondary rate limit and have been temporarily blocked from content creation. (HTTP 403)";

    fn payload(comments: usize) -> String {
        payload_with(comments, "c")
    }

    fn payload_with(comments: usize, prefix: &str) -> String {
        let comments = (0..comments)
            .map(|line| serde_json::json!({ "path": "a.rs", "line": line + 1, "body": format!("{prefix}{line}") }))
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

    const BAD_GATEWAY: &str = "gh: Server Error (HTTP 502)";
    const UNAVAILABLE: &str = "gh: Service Unavailable (HTTP 503)";
    const UNPROCESSABLE: &str = "gh: Validation Failed (HTTP 422)";
    const LOOKUP_ERROR: &str = "could not list reviews";
    const FINAL_MARKER: &str = "<!-- ocr-review-post:v1 -->";
    const EARLIER_MARKER: &str = "<!-- ocr-review-post-part:v1 -->";

    fn assert_part_needles(fake: &Fake, label: &str) {
        let body = fake.attempts[0]["body"].as_str().unwrap();
        let fingerprint = body.lines().find(|line| line.starts_with(BATCH_MARKER)).unwrap();
        for (_, needles) in &fake.lookups {
            assert_eq!(needles, &[EARLIER_MARKER, label, fingerprint]);
            assert!(needles.iter().all(|needle| body.contains(needle)));
        }
    }

    #[test]
    fn early_batch_lookup_rejects_an_old_part_with_other_comments() {
        let mut old = Fake::default();
        post_review_batches(&mut old, &payload_with(42, "old")).unwrap();
        let old_part = old.posts[0]["body"].as_str().unwrap().to_owned();
        let mut fake = Fake {
            outcomes: [Err(BAD_GATEWAY)].into(),
            github: vec![(7, old_part)],
            search_github: true,
            ..Fake::default()
        };
        assert_eq!(post_review_batches(&mut fake, &payload(42)), Ok(Some(2)));
        assert_eq!(fake.attempts.len(), 3);
        assert_eq!(fake.attempts[0], fake.attempts[1]);
        assert_eq!(fake.posts.iter().map(count).collect::<Vec<_>>(), [40, 2]);
        assert_eq!(fake.posts[0]["comments"][0]["body"], "c0");
        assert_eq!(fake.posts[1]["body"], format!("summary\n\n{FINAL_MARKER}"));
        assert_eq!(fake.waits, [65, 65]);
        assert_part_needles(&fake, "part 1 of 2.");
    }

    #[test]
    fn early_part_503_found_by_lookup_continues_to_the_summary() {
        let mut old = Fake::default();
        post_review_batches(&mut old, &payload_with(42, "old")).unwrap();
        let mut fake = Fake {
            outcomes: [Err(UNAVAILABLE)].into(),
            stored_despite_error: vec![1],
            github: vec![(7, old.posts[0]["body"].as_str().unwrap().into())],
            search_github: true,
            ..Fake::default()
        };
        assert_eq!(post_review_batches(&mut fake, &payload(42)), Ok(Some(2)));
        assert_eq!(fake.attempts.iter().map(count).collect::<Vec<_>>(), [40, 2]);
        assert_eq!(fake.posts[1]["body"], format!("summary\n\n{FINAL_MARKER}"));
        assert_eq!(fake.waits, [65, 65]);
        assert_eq!(fake.lookups.len(), 1);
        assert_eq!(fake.lookups[0].0, 1);
        // ID 101 is the newly stored part, not the part from the previous selection.
        assert_eq!(fake.found_reviews, [101]);
    }

    #[test]
    fn resend_after_the_last_scheduled_lookup_gets_one_read_only_check() {
        let mut fake = Fake {
            outcomes: [Err(BAD_GATEWAY), Err(BAD_GATEWAY)].into(),
            stored_despite_error: vec![2],
            search_github: true,
            lookup_outcomes: [Err(LOOKUP_ERROR), Err(LOOKUP_ERROR), Ok(None)].into(),
            ..Fake::default()
        };
        assert_eq!(post_review_batches(&mut fake, &payload(42)), Ok(Some(2)));
        assert_eq!(fake.attempts.iter().map(count).collect::<Vec<_>>(), [40, 40, 2]);
        assert_eq!(fake.attempts[0], fake.attempts[1]);
        assert_eq!(fake.posts.iter().map(count).collect::<Vec<_>>(), [40, 2]);
        assert_eq!(fake.posts[1]["body"], format!("summary\n\n{FINAL_MARKER}"));
        assert_eq!(fake.waits, [65, 120, 240, 65, 65]);
        assert_eq!(
            fake.lookups.iter().map(|(attempt, _)| *attempt).collect::<Vec<_>>(),
            [1, 1, 1, 2]
        );
    }

    #[test]
    fn the_extra_check_never_authorizes_another_post() {
        let mut fake = Fake {
            outcomes: [Err(BAD_GATEWAY), Err(BAD_GATEWAY)].into(),
            search_github: true,
            lookup_outcomes: [Err(LOOKUP_ERROR), Err(LOOKUP_ERROR), Ok(None)].into(),
            ..Fake::default()
        };
        let error = post_review_batches(&mut fake, &payload(42)).unwrap_err();
        assert!(error.contains(BAD_GATEWAY) && error.contains("8 minutes"), "{error}");
        assert_eq!(fake.attempts.len(), 2);
        assert!(fake.posts.is_empty());
        assert_eq!(fake.waits, [65, 120, 240, 65]);
        assert_eq!(fake.lookups.len(), 4);
    }

    #[test]
    fn last_batch_502_then_success_retries_only_that_batch() {
        let mut fake = Fake {
            outcomes: [Ok(Some(1)), Ok(Some(2)), Err(BAD_GATEWAY), Ok(Some(3))].into(),
            lookup_outcomes: [Ok(None)].into(),
            ..Fake::default()
        };
        assert_eq!(post_review_batches(&mut fake, &payload(84)), Ok(Some(3)));
        assert_eq!(fake.attempts.len(), 4);
        assert_eq!(fake.attempts[2], fake.attempts[3]);
        assert_eq!(fake.posts.iter().map(count).collect::<Vec<_>>(), [40, 40, 4]);
        assert_eq!(fake.posts[2]["body"], format!("summary\n\n{FINAL_MARKER}"));
        assert_eq!(fake.waits, [65, 65, 65]);
        assert_eq!(fake.lookups, [(3, vec![FINAL_MARKER.into()])]);
    }

    #[test]
    fn a_gateway_502_without_a_json_body_is_retried_like_any_other() {
        let mut fake = Fake {
            outcomes: [Err("could not post review (GitHub API): gh: HTTP 502"), Ok(Some(7))].into(),
            lookup_outcomes: [Ok(None)].into(),
            ..Fake::default()
        };
        assert_eq!(post_review_batches(&mut fake, &payload(4)), Ok(Some(7)));
        assert_eq!(fake.attempts.len(), 2);
        assert_eq!(fake.attempts[0], fake.attempts[1]);
        assert_eq!(fake.waits, [65]);
        assert_eq!(fake.lookups.len(), 1);
    }

    #[test]
    fn last_batch_502_found_by_lookup_returns_id_without_resending() {
        let mut fake = Fake {
            outcomes: [Ok(Some(1)), Err(BAD_GATEWAY)].into(),
            lookup_outcomes: [Ok(Some(42))].into(),
            ..Fake::default()
        };
        assert_eq!(post_review_batches(&mut fake, &payload(42)), Ok(Some(42)));
        assert_eq!(fake.attempts.iter().map(count).collect::<Vec<_>>(), [40, 2]);
        assert_eq!(fake.waits, [65, 65]);
        assert_eq!(fake.lookups, [(2, vec![FINAL_MARKER.into()])]);
    }

    #[test]
    fn first_part_503_three_times_stops_before_part_two() {
        let mut fake = Fake {
            outcomes: [Err(UNAVAILABLE); 3].into(),
            lookup_outcomes: [Ok(None); 3].into(),
            ..Fake::default()
        };
        let error = post_review_batches(&mut fake, &payload(84)).unwrap_err();
        assert!(error.contains("7 minutes"), "{error}");
        assert!(error.contains("HTTP 503"), "{error}");
        assert_eq!(fake.attempts.len(), 3);
        assert!(fake.attempts.iter().all(|post| post == &fake.attempts[0]));
        assert!(fake.posts.is_empty());
        assert_eq!(fake.waits, [65, 120, 240]);
        assert_eq!(
            fake.lookups.iter().map(|(attempt, _)| *attempt).collect::<Vec<_>>(),
            [1, 2, 3]
        );
        assert_part_needles(&fake, "part 1 of 3.");
    }

    #[test]
    fn failed_lookup_then_found_does_not_resend() {
        let mut fake = Fake {
            outcomes: [Err(BAD_GATEWAY)].into(),
            lookup_outcomes: [Err(LOOKUP_ERROR), Ok(Some(42))].into(),
            ..Fake::default()
        };
        assert_eq!(post_review_batches(&mut fake, &payload(3)), Ok(Some(42)));
        assert_eq!(fake.attempts.len(), 1);
        assert_eq!(fake.waits, [65, 120]);
        assert_eq!(fake.lookups, [1, 1].map(|attempt| (attempt, vec![FINAL_MARKER.into()])));
    }

    #[test]
    fn three_failed_lookups_do_not_resend() {
        let mut fake = Fake {
            outcomes: [Err(BAD_GATEWAY)].into(),
            lookup_outcomes: [Err(LOOKUP_ERROR); 3].into(),
            ..Fake::default()
        };
        let error = post_review_batches(&mut fake, &payload(3)).unwrap_err();
        assert!(error.contains(LOOKUP_ERROR) && error.contains("7 minutes"), "{error}");
        assert_eq!(fake.attempts.len(), 1);
        assert_eq!(fake.waits, [65, 120, 240]);
        assert_eq!(
            fake.lookups,
            [1, 1, 1].map(|attempt| (attempt, vec![FINAL_MARKER.into()]))
        );
    }

    #[test]
    fn a_422_after_a_502_fails_without_another_retry() {
        let mut fake = Fake {
            outcomes: [Err(BAD_GATEWAY), Err(UNPROCESSABLE)].into(),
            lookup_outcomes: [Ok(None)].into(),
            ..Fake::default()
        };
        assert_eq!(post_review_batches(&mut fake, &payload(84)), Err(UNPROCESSABLE.into()));
        assert_eq!(fake.attempts.len(), 2);
        assert_eq!(fake.attempts[0], fake.attempts[1]);
        assert_eq!(fake.waits, [65]);
        assert_eq!(fake.lookups.len(), 1);
    }

    #[test]
    fn retry_uses_appended_marker_not_one_quoted_in_the_summary() {
        let mut fake = Fake {
            outcomes: [Err(BAD_GATEWAY)].into(),
            lookup_outcomes: [Ok(Some(42))].into(),
            ..Fake::default()
        };
        let json =
            serde_json::json!({"body": format!("Old review: <!-- ocr-review-post:v1 run=1 -->\n\n{FINAL_MARKER}")});
        assert_eq!(post_review_batches(&mut fake, &json.to_string()), Ok(Some(42)));
        assert_eq!(fake.attempts.len(), 1);
        assert_eq!(fake.lookups, [(1, vec![FINAL_MARKER.into()])]);
    }

    #[test]
    fn server_status_takes_precedence_over_rate_limit_words() {
        let mut fake = Fake {
            outcomes: [Err("gh: Server Error while checking secondary rate limits (HTTP 502)")].into(),
            lookup_outcomes: [Ok(Some(42))].into(),
            ..Fake::default()
        };
        assert_eq!(post_review_batches(&mut fake, &payload(3)), Ok(Some(42)));
        assert_eq!(fake.attempts.len(), 1);
        assert_eq!(fake.waits, [65]);
        assert_eq!(fake.lookups, [(1, vec![FINAL_MARKER.into()])]);
    }

    #[test]
    fn marked_retry_requires_a_marker_to_check() {
        let mut fake = Fake {
            outcomes: [Err(BAD_GATEWAY)].into(),
            ..Fake::default()
        };
        let error = post_review_batches(&mut fake, r#"{"body":"unmarked"}"#).unwrap_err();
        assert!(error.contains(BAD_GATEWAY) && error.contains("marker"), "{error}");
        assert_eq!(fake.attempts.len(), 1);
        assert!(fake.waits.is_empty() && fake.lookups.is_empty());
    }

    #[test]
    fn parses_retryable_gh_http_statuses_only() {
        for error in [
            "gh: Internal Server Error (HTTP 500)",
            "could not post review (GitHub API): gh: Server Error (HTTP 502)",
            "could not post review (GitHub API): gh: HTTP 502",
            "gh: HTTP 504",
            "warning: connection interrupted\ngh: Gateway Timeout (HTTP 504)\n",
        ] {
            assert!(is_retryable_server_error(error), "missed {error:?}");
        }
        for error in [
            LIMIT,
            UNPROCESSABLE,
            "gh: Not Implemented (HTTP 501)",
            "HTTP 5020",
            "HTTP 503",
            "gh: HTTP 5020",
            "gh: HTTP 422",
            "gh: timeout",
            "gh: bad HTTP 503 response (HTTP 422)",
            "gh: Validation Failed (HTTP 422)\nHTTP 503",
        ] {
            assert!(!is_retryable_server_error(error), "matched {error:?}");
        }
    }

    #[test]
    fn other_errors_fail_at_once_and_stop_later_batches() {
        let mut fake = Fake {
            refusals: vec!["HTTP 422: Unprocessable".into()],
            ..Fake::default()
        };
        assert!(post_review_batches(&mut fake, &payload(84)).is_err());
        assert_eq!(fake.attempts.len(), 1);
        assert!(fake.posts.is_empty() && fake.waits.is_empty() && fake.lookups.is_empty());
    }
}
