use super::*;

#[test]
fn cancelled_evidence_capture_never_starts_a_github_request() {
    let cancel = std::sync::atomic::AtomicBool::new(true);
    assert_eq!(
        gh_capture_cancellable(&["api", "user"], &cancel).unwrap_err(),
        "cancelled"
    );
}

#[test]
fn parses_live_pr_state_without_unsupported_fields() {
    assert_eq!(LIVE_PR_JSON_FIELDS, "state,headRefOid,baseRefName,baseRefOid,mergedAt");
    for state in ["OPEN", "CLOSED", "MERGED"] {
        let raw = format!(
            r#"{{"state":"{state}","headRefOid":"abc123","baseRefName":"parent","baseRefOid":"base123","mergedAt":"2026-09-27T00:47:42Z"}}"#
        );
        let parsed = parse_live_pr_state(raw.as_bytes()).expect("parse live PR state");
        assert_eq!(parsed.state, state);
        assert_eq!(parsed.head_sha, "abc123");
        assert_eq!(parsed.base_ref, "parent");
        assert_eq!(parsed.base_sha, "base123");
        assert_eq!(parsed.merged_at, Some(1790470062000));
    }
    let raw_unmerged =
        r#"{"state":"OPEN","headRefOid":"abc123","baseRefName":"parent","baseRefOid":"base123","mergedAt":null}"#;
    let parsed = parse_live_pr_state(raw_unmerged.as_bytes()).expect("parse unmerged");
    assert_eq!(parsed.merged_at, None);
}

#[test]
fn accepts_plain_hosts_and_ports() {
    for host in [
        "github.com",
        "ghe.example.com",
        "ghe.internal",
        "localhost",
        "git.example.co.uk",
        "github.example.com:8443",
    ] {
        assert!(is_valid_gh_host(host), "should accept {host}");
    }
}

#[test]
fn rejects_schemes_paths_creds_and_whitespace() {
    for host in [
        "",
        " ",
        "https://github.com",
        "github.com/owner/repo",
        "github.com/",
        "user@github.com",
        "github.com ",
        "evil.com\ngithub.com",
        "github.com:",
        "github.com:port",
        "-github.com",
        "github-.com",
        "git_hub.com",
    ] {
        assert!(!is_valid_gh_host(host), "should reject {host:?}");
    }
}

#[test]
fn sign_out_treats_an_absent_session_as_done_and_reports_real_failures() {
    assert!(auth::sign_out_result(true, "").is_ok());
    assert!(auth::sign_out_result(false, "not logged in to any hosts\n").is_ok());
    assert_eq!(
        auth::sign_out_result(false, "some noise\nfailed to write config: read-only file system\n\n").unwrap_err(),
        "Could not sign out of GitHub: failed to write config: read-only file system"
    );
}

#[test]
fn forgetting_the_identity_clears_cached_login_and_teams() {
    *GH_LOGIN.lock().unwrap() = Some("octocat".into());
    *GH_TEAM_SLUGS.lock().unwrap() = Some(HashSet::from(["core".to_string()]));
    auth::forget_identity();
    assert!(GH_LOGIN.lock().unwrap().is_none());
    assert!(GH_TEAM_SLUGS.lock().unwrap().is_none());
}
