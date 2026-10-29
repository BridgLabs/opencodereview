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

#[test]
fn a_saved_selection_keeps_the_summary_suggestions() {
    let section = crate::review::evidence::suggestions_section(&[
        json!({"path":"src/a.rs","line":2,"title":"Stale test name","detail":"Detail","action":"Rename it"}),
    ]);
    let comment = json!({"path":"src/a.rs","line":3,"side":"RIGHT","body":"Finding body"});
    let audit = FindingAudit {
        evidence_warnings: None,
        head_sha: "head".into(),
        complete: true,
        original_body: format!("Automated review produced 1 inline comment(s).\n\n{section}"),
        diff: None,
        candidates: vec![FindingCandidate {
            id: "candidate-1".into(),
            kind: "finding".into(),
            origin: "chunk".into(),
            chunk_id: Some(1),
            ai_decision: "keep".into(),
            ai_reason: None,
            default_selected: true,
            selected: true,
            blocked_reason: None,
            comment: Some(comment),
            body: "Finding body".into(),
            original_body: None,
        }],
    };
    let (body, comments) = selection_payload(&audit);
    assert!(
        body.starts_with("1 review comments selected for publication."),
        "{body}"
    );
    assert!(body.ends_with(section.trim()), "{body}");
    assert!(comments.contains("Finding body"));
    // A general finding in the body carries the section too; it is listed once.
    let mut general = audit.clone();
    general.candidates[0].comment = None;
    general.candidates[0].body = format!("**[Finding] General**\n\nDetail\n\n{section}");
    let (body, _) = selection_payload(&general);
    assert_eq!(body.matches(crate::review::SUGGESTIONS_HEADING).count(), 1, "{body}");
    assert!(body.starts_with("**[Finding] General**"), "{body}");
}
