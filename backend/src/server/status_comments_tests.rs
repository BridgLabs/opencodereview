use super::test_support::FakeStatusComments;
use super::*;

const MARKER: &str = "<!-- ocr-result-key:v1 repo=owner/repo pr=7 installation=inst-a -->";
const OTHER_MARKER: &str = "<!-- ocr-result-key:v1 repo=owner/repo pr=7 installation=inst-b -->";
const CLEAN_DATA: &str = "<!-- ocr-result-data:v1 head=abc reviewed_head=abc observation=clean state=clean inherited_from=none reviewer=ocr-1-review-v1 skill=v1 -->";
const FINDINGS_DATA: &str = "<!-- ocr-result-data:v1 head=abc reviewed_head=abc observation=findings state=findings_posted inherited_from=none reviewer=ocr-1-review-v1 skill=v1 -->";

fn clean_body(run: Option<i64>) -> String {
    match run {
        Some(run) => format!("{MARKER}\n{CLEAN_DATA}\n{}\n\nvisible", status_publication_marker(run)),
        None => format!("{MARKER}\n{CLEAN_DATA}\n\nvisible"),
    }
}

#[test]
fn replace_posts_fresh_comment_when_none_exists() {
    let mut api = FakeStatusComments::new();
    let id = replace_clean_comment(&mut api, "owner/repo", 7, MARKER, 3, &clean_body(Some(3))).unwrap();
    assert_eq!(api.posts, 1);
    assert_eq!(id, 1);
    assert_eq!(api.comments.len(), 1);
}

#[test]
fn replace_posts_then_deletes_legacy_and_older_run_comments() {
    let mut api = FakeStatusComments::new();
    api.own(&clean_body(None));
    api.own(&clean_body(Some(2)));
    let id = replace_clean_comment(&mut api, "owner/repo", 7, MARKER, 5, &clean_body(Some(5))).unwrap();
    assert_eq!(api.posts, 1);
    assert_eq!(api.deletes, 2);
    assert_eq!(api.ids(), vec![id]);
}

#[test]
fn replace_adopts_this_run_comment_after_lost_response() {
    let mut api = FakeStatusComments::new();
    api.own(&clean_body(Some(2)));
    let existing = api.own(&clean_body(Some(5)));
    let id = replace_clean_comment(&mut api, "owner/repo", 7, MARKER, 5, &clean_body(Some(5))).unwrap();
    assert_eq!(api.posts, 0);
    assert_eq!(id, existing);
    assert_eq!(api.ids(), vec![existing]);
}

#[test]
fn replace_keeps_lowest_of_duplicate_run_comments() {
    let mut api = FakeStatusComments::new();
    let first = api.own(&clean_body(Some(5)));
    api.own(&clean_body(Some(5)));
    let id = replace_clean_comment(&mut api, "owner/repo", 7, MARKER, 5, &clean_body(Some(5))).unwrap();
    assert_eq!(api.posts, 0);
    assert_eq!(id, first);
    assert_eq!(api.ids(), vec![first]);
}

#[test]
fn replace_never_deletes_foreign_or_newer_comments() {
    let mut api = FakeStatusComments::new();
    let mut kept = vec![
        api.own(&format!(
            "{OTHER_MARKER}\n{CLEAN_DATA}\n{}\n\nvisible",
            status_publication_marker(2)
        )),
        api.add(999, &clean_body(Some(2))),
        api.own(&format!("> {MARKER}\n\nhuman note")),
        api.own(&format!("{MARKER}\n{FINDINGS_DATA}\n\nvisible")),
        api.own("<!-- ocr-summary:v1 repo=owner/repo pr=7 -->\nsummary"),
        api.own(&clean_body(Some(9))),
    ];
    let id = replace_clean_comment(&mut api, "owner/repo", 7, MARKER, 5, &clean_body(Some(5))).unwrap();
    kept.push(id);
    kept.sort();
    let mut actual = api.ids();
    actual.sort();
    assert_eq!(actual, kept);
}

#[test]
fn replace_retry_after_lost_post_adopts_without_posting_again() {
    let mut api = FakeStatusComments::new();
    api.post_writes_then_fails = true;
    assert!(replace_clean_comment(&mut api, "owner/repo", 7, MARKER, 5, &clean_body(Some(5))).is_err());
    assert_eq!(api.comments.len(), 1);
    api.post_writes_then_fails = false;
    let id = replace_clean_comment(&mut api, "owner/repo", 7, MARKER, 5, &clean_body(Some(5))).unwrap();
    assert_eq!(api.posts, 1);
    assert_eq!(id, 1);
    assert_eq!(api.comments.len(), 1);
}

#[test]
fn replace_delete_failure_errs_and_retry_finishes_without_reposting() {
    let mut api = FakeStatusComments::new();
    api.own(&clean_body(Some(2)));
    api.fail_delete = true;
    assert!(replace_clean_comment(&mut api, "owner/repo", 7, MARKER, 5, &clean_body(Some(5))).is_err());
    assert_eq!(api.posts, 1);
    api.fail_delete = false;
    let id = replace_clean_comment(&mut api, "owner/repo", 7, MARKER, 5, &clean_body(Some(5))).unwrap();
    assert_eq!(api.posts, 1);
    assert_eq!(api.ids(), vec![id]);
}

#[test]
fn retire_before_deletes_older_and_legacy_but_keeps_current_and_newer() {
    let mut api = FakeStatusComments::new();
    api.own(&clean_body(Some(3)));
    api.own(&clean_body(None));
    let mut kept = vec![
        api.own(&clean_body(Some(8))),
        api.own(&clean_body(Some(9))),
        api.add(999, &clean_body(Some(2))),
        api.own(&format!("{OTHER_MARKER}\n{CLEAN_DATA}\n\nvisible")),
    ];
    retire_clean_comments_before(&mut api, "owner/repo", 7, MARKER, 8).unwrap();
    kept.sort();
    let mut actual = api.ids();
    actual.sort();
    assert_eq!(actual, kept);
}

#[test]
fn replace_stamps_legacy_body_and_retry_adopts_it() {
    let mut api = FakeStatusComments::new();
    let stamped = with_status_publication_marker(&clean_body(None), MARKER, 5);
    assert_eq!(owned_clean_comment_run(&stamped, MARKER), Some(Some(5)));
    let id = replace_clean_comment(&mut api, "owner/repo", 7, MARKER, 5, &stamped).unwrap();
    assert_eq!(api.posts, 1);
    assert_eq!(api.comments[0].body, stamped);
    let again = replace_clean_comment(&mut api, "owner/repo", 7, MARKER, 5, &stamped).unwrap();
    assert_eq!(api.posts, 1);
    assert_eq!(again, id);
}

#[test]
fn with_status_publication_marker_leaves_stamped_and_foreign_bodies_alone() {
    assert_eq!(
        with_status_publication_marker(&clean_body(Some(4)), MARKER, 5),
        clean_body(Some(4))
    );
    let foreign = format!("{OTHER_MARKER}\n{CLEAN_DATA}\n\nvisible");
    assert_eq!(with_status_publication_marker(&foreign, MARKER, 5), foreign);
    assert_eq!(
        with_status_publication_marker("just a comment", MARKER, 5),
        "just a comment"
    );
}

#[test]
fn find_clean_comment_returns_lowest_matching_run_only() {
    let mut api = FakeStatusComments::new();
    let older = api.own(&clean_body(Some(4)));
    api.own(&clean_body(None));
    api.add(999, &clean_body(Some(6)));
    let first = api.own(&clean_body(Some(6)));
    api.own(&clean_body(Some(6)));
    assert_eq!(
        find_clean_comment(&mut api, "owner/repo", 7, MARKER, 6).unwrap(),
        Some(first)
    );
    assert_eq!(
        find_clean_comment(&mut api, "owner/repo", 7, MARKER, 4).unwrap(),
        Some(older)
    );
    assert_eq!(find_clean_comment(&mut api, "owner/repo", 7, MARKER, 8).unwrap(), None);
}

#[test]
fn owned_clean_comment_run_parses_generations() {
    assert_eq!(owned_clean_comment_run(&clean_body(Some(5)), MARKER), Some(Some(5)));
    assert_eq!(owned_clean_comment_run(&clean_body(None), MARKER), Some(None));
}

#[test]
fn owned_clean_comment_run_rejects_non_family_bodies() {
    let findings = format!("{MARKER}\n{FINDINGS_DATA}\n\nvisible");
    let missing_data = format!("{MARKER}\n\nvisible");
    let malformed_run = format!("{MARKER}\n{CLEAN_DATA}\n<!-- ocr-status-publication:v1 run=abc -->\n\nvisible");
    let quoted = format!("> {MARKER}\n{CLEAN_DATA}\n\nvisible");
    let other_installation = format!("{OTHER_MARKER}\n{CLEAN_DATA}\n\nvisible");
    let stray_third_line = format!("{MARKER}\n{CLEAN_DATA}\n<!-- other -->\n\nvisible");
    assert_eq!(owned_clean_comment_run(&findings, MARKER), None);
    assert_eq!(owned_clean_comment_run(&missing_data, MARKER), None);
    assert_eq!(owned_clean_comment_run(&malformed_run, MARKER), None);
    assert_eq!(owned_clean_comment_run(&quoted, MARKER), None);
    assert_eq!(owned_clean_comment_run(&other_installation, MARKER), None);
    assert_eq!(owned_clean_comment_run(&stray_third_line, MARKER), None);
    assert_eq!(owned_clean_comment_run("just a comment", MARKER), None);
}
