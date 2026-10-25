use super::*;

#[test]
fn mixed_review_points_are_actionable_in_either_order() {
    for body in [
        "**[Question] Confirm**\n\n**[Finding] Fix**",
        "**[Finding] Fix**\n\n**[Question] Confirm**",
    ] {
        assert_eq!(point_kind(body), "mixed");
        assert!(actionable(None, Some(body)));
        assert!(actionable(Some(&json!({"comments":[{"body":body}]}).to_string()), None));
    }
    for (body, kind) in [
        ("[Question] Confirm\n```text\n[Finding] Example\n```", "question"),
        ("[Finding] Fix\n~~~text\n[Question] Example\n~~~", "finding"),
        ("No findings.", "unknown"),
    ] {
        assert_eq!(point_kind(body), kind);
    }
}

#[test]
fn auto_post_event_uses_selected_findings_not_questions_or_fenced_examples() {
    for (body, actionable) in [
        ("[Question] Confirm", false),
        ("[Question] Confirm\n```text\n[Finding] Example\n```", false),
        ("[Finding] Fix", true),
        ("[Question] Confirm\n\n[Finding] Fix", true),
    ] {
        let comments = json!({"comments":[{"body":body}]}).to_string();
        for (comments, body) in [(Some(comments.as_str()), None), (None, Some(body))] {
            assert_eq!(auto_post_event("off", comments, body), None);
            assert_eq!(auto_post_event("invalid", comments, body), None);
            assert_eq!(auto_post_event("comment", comments, body), Some("COMMENT"));
            assert_eq!(
                auto_post_event("request_changes", comments, body),
                Some(if actionable { "REQUEST_CHANGES" } else { "COMMENT" })
            );
        }
    }
}

#[test]
fn missing_or_partial_decisions_are_incomplete() {
    let sources = json!({"comments": [{"candidateId": "a", "comment": {"body": "claim"}}]});
    assert!(validate_decisions(&sources, None, r#"{"comments":[]}"#).is_err());
    assert!(validate_decisions(&sources, Some(r#"{"decisions":[]}"#), r#"{"comments":[]}"#).is_err());
}

#[test]
fn explicit_refutation_stays_empty() {
    let sources = json!({"comments": [{"candidateId": "a", "comment": {"body": "claim"}}]});
    let decisions = r#"{"decisions":[{"candidateId":"a","decision":"discard","reason":"src/auth.rs:10 already enforces the boundary"}]}"#;
    assert!(validate_decisions(&sources, Some(decisions), r#"{"comments":[]}"#).is_ok());
    assert!(all_candidates_discarded(&sources, Some(decisions)));
    assert!(validate_decisions(&sources, Some(decisions), r#"{"comments":[{"body":"claim"}]}"#).is_err());
}

#[test]
fn unmapped_keep_and_missing_context_are_incomplete() {
    let sources = json!({"comments": [{"candidateId": "a", "comment": {"body": "claim"}}]});
    for decision in ["keep", "needs_context"] {
        let decisions =
            json!({"decisions":[{"candidateId":"a","decision":decision,"reason":"missing caller","commentIndex":0}]})
                .to_string();
        assert!(validate_decisions(&sources, Some(&decisions), r#"{"comments":[]}"#).is_err());
    }
}
