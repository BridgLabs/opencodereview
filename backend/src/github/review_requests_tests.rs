use super::*;

fn set(items: &[&str]) -> BTreeSet<String> {
    items.iter().map(|s| s.to_string()).collect()
}

fn fixture() -> (ReviewRequestSnapshot, RequestTimeline) {
    (
        ReviewRequestSnapshot {
            login: "reviewer".into(),
            requested: RequestedReviewers {
                reviewers: set(&["reviewer", "other"]),
                team_reviewers: set(&["backend", "frontend"]),
            },
            events: set(&["original-request"]),
        },
        RequestTimeline {
            open: true,
            events: set(&["original-request", "ocr-review"]),
            reviews: vec![AttributedReview {
                id: 42,
                node_id: "ocr-review".into(),
                login: "reviewer".into(),
                teams: set(&["backend", "not-requested"]),
            }],
        },
    )
}

#[test]
fn restores_only_consumed_authenticated_user_and_attributed_requested_teams() {
    let (snapshot, timeline) = fixture();
    let RestoreRequests::Request(plan) =
        plan_request_restoration(&snapshot, 42, &timeline, &RequestedReviewers::default()).unwrap()
    else {
        panic!("expected restoration");
    };
    assert_eq!(plan.reviewers, set(&["reviewer"]));
    assert_eq!(plan.team_reviewers, set(&["backend"]));
}

#[test]
fn does_not_request_reviewers_already_pending() {
    let (snapshot, timeline) = fixture();
    assert!(matches!(
        plan_request_restoration(&snapshot, 42, &timeline, &snapshot.requested).unwrap(),
        RestoreRequests::Done
    ));
}

#[test]
fn concurrent_review_request_removal_or_addition_wins_even_by_same_user() {
    for event in ["human-review", "request-removed", "new-request", "review-dismissed"] {
        let (snapshot, mut timeline) = fixture();
        timeline.events.insert(event.into());
        assert!(matches!(
            plan_request_restoration(&snapshot, 42, &timeline, &RequestedReviewers::default()).unwrap(),
            RestoreRequests::Changed
        ));
    }
}

#[test]
fn missing_history_identity_or_closed_pr_never_restores() {
    let (snapshot, mut timeline) = fixture();
    timeline.events.remove("original-request");
    assert!(matches!(
        plan_request_restoration(&snapshot, 42, &timeline, &RequestedReviewers::default()).unwrap(),
        RestoreRequests::Changed
    ));
    let (snapshot, mut timeline) = fixture();
    timeline.reviews[0].login = "different-user".into();
    assert!(matches!(
        plan_request_restoration(&snapshot, 42, &timeline, &RequestedReviewers::default()).unwrap(),
        RestoreRequests::Changed
    ));
    timeline.open = false;
    assert!(matches!(
        plan_request_restoration(&snapshot, 42, &timeline, &RequestedReviewers::default()).unwrap(),
        RestoreRequests::Done
    ));
}

#[test]
fn missing_published_review_is_retryable_without_another_publication() {
    let (snapshot, timeline) = fixture();
    assert!(plan_request_restoration(&snapshot, 99, &timeline, &RequestedReviewers::default()).is_err());
}

#[test]
fn no_attribution_and_no_user_request_never_adds_a_request() {
    let (mut snapshot, mut timeline) = fixture();
    snapshot.requested.reviewers.clear();
    timeline.reviews[0].teams.clear();
    assert!(matches!(
        plan_request_restoration(&snapshot, 42, &timeline, &RequestedReviewers::default()).unwrap(),
        RestoreRequests::Done
    ));
}

fn requests(users: &[&str], teams: &[&str]) -> serde_json::Value {
    serde_json::json!([{
        "users": users.iter().map(|login| serde_json::json!({"login":login})).collect::<Vec<_>>(),
        "teams": teams.iter().map(|slug| serde_json::json!({"slug":slug})).collect::<Vec<_>>()
    }])
}

fn history(published: bool) -> serde_json::Value {
    let mut nodes = vec![serde_json::json!({"id":"original-request","__typename":"ReviewRequestedEvent"})];
    if published {
        nodes.push(serde_json::json!({
            "id":"ocr-review", "__typename":"PullRequestReview", "databaseId":42,
            "author":{"login":"reviewer"}, "onBehalfOf":{"pageInfo":{"hasNextPage":false},"nodes":[{"slug":"backend"}]}
        }));
    }
    serde_json::json!({"data":{"repository":{"pullRequest":{
        "state":"OPEN","timelineItems":{"nodes":nodes,"pageInfo":{"hasNextPage":false,"endCursor":"cursor"}}
    }}}})
}

#[test]
fn snapshot_requires_stable_history_and_complete_reads_before_publication() {
    let mut replies = vec![
        serde_json::json!({"login":"Reviewer"}),
        history(false),
        requests(&["Reviewer", "other"], &["backend", "frontend"]),
        history(false),
    ]
    .into_iter();
    let snapshot = snapshot_review_requests_with("o/r", 1, &mut |_| Ok(replies.next().unwrap())).unwrap();
    assert_eq!(snapshot.login, "reviewer");
    assert_eq!(snapshot.requested.reviewers, set(&["reviewer", "other"]));
    let mut replies = vec![
        serde_json::json!({"login":"reviewer"}),
        history(false),
        requests(&[], &["backend"]),
        history(true),
    ]
    .into_iter();
    assert!(snapshot_review_requests_with("o/r", 1, &mut |_| Ok(replies.next().unwrap())).is_err());
    assert!(snapshot_review_requests_with("o/r", 1, &mut |_| Err("offline".into())).is_err());
}

#[test]
fn restore_transport_only_posts_missing_requests_and_timeout_retry_never_posts_review() {
    let (snapshot, _) = fixture();
    let mut replies = vec![
        Ok(requests(&[], &["frontend"])),
        Ok(history(true)),
        Err("timeout after acceptance".into()),
    ]
    .into_iter();
    let mut calls = vec![];
    let result = restore_review_requests_with("o/r", 1, 42, &snapshot, &mut |args| {
        calls.push(args.iter().map(|arg| arg.to_string()).collect::<Vec<_>>());
        replies.next().unwrap()
    });
    assert!(result.is_err());
    assert_eq!(
        calls[2],
        vec![
            "api",
            "--method",
            "POST",
            "repos/o/r/pulls/1/requested_reviewers",
            "-f",
            "reviewers[]=reviewer",
            "-f",
            "team_reviewers[]=backend"
        ]
    );

    // GitHub accepted the write despite the timeout; a retry observes the requests.
    let mut changed = history(true);
    changed["data"]["repository"]["pullRequest"]["timelineItems"]["nodes"]
        .as_array_mut()
        .unwrap()
        .push(serde_json::json!({"id":"restored-request","__typename":"ReviewRequestedEvent"}));
    let mut replies = vec![requests(&["reviewer"], &["backend", "frontend"]), changed].into_iter();
    let mut read_count = 0;
    assert!(restore_review_requests_with("o/r", 1, 42, &snapshot, &mut |args| {
        assert!(!args.contains(&"POST"));
        read_count += 1;
        Ok(replies.next().unwrap())
    })
    .unwrap());
    assert_eq!(read_count, 2);
}

#[test]
fn timeline_paginates_and_rejects_partial_graphql_evidence() {
    let mut first = history(false);
    first["data"]["repository"]["pullRequest"]["timelineItems"]["pageInfo"]["hasNextPage"] = true.into();
    let mut replies = vec![first, history(true)].into_iter();
    let mut calls = 0;
    let timeline = request_timeline("o/r", 1, &mut |args| {
        if calls == 1 {
            assert!(args.contains(&"cursor=cursor"));
        }
        calls += 1;
        Ok(replies.next().unwrap())
    })
    .unwrap();
    assert_eq!(calls, 2);
    assert!(timeline.events.contains("ocr-review"));
    let mut partial = history(false);
    partial["errors"] = serde_json::json!([{"message":"permission denied"}]);
    assert!(request_timeline("o/r", 1, &mut |_| Ok(partial.clone())).is_err());
}

#[test]
fn concurrent_activity_never_calls_the_write_endpoint() {
    let (snapshot, _) = fixture();
    let mut changed = history(true);
    changed["data"]["repository"]["pullRequest"]["timelineItems"]["nodes"]
        .as_array_mut()
        .unwrap()
        .push(serde_json::json!({"id":"human-removal","__typename":"ReviewRequestRemovedEvent"}));
    let mut replies = vec![requests(&[], &[]), changed].into_iter();
    assert!(!restore_review_requests_with("o/r", 1, 42, &snapshot, &mut |args| {
        assert!(!args.contains(&"POST"));
        Ok(replies.next().unwrap())
    })
    .unwrap());
}
