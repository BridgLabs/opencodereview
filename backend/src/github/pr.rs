use super::*;

/// Valid inline-comment anchors for a PR, parsed from its unified diff: per file
/// path, the line numbers commentable on each side — `.0` = RIGHT (new file),
/// `.1` = LEFT (old file). GitHub returns 422 for the WHOLE review if ANY inline
/// comment is outside these (e.g. a file/line not in the diff), so callers
/// validate against this first. Empty map = couldn't fetch the diff (skip checks).
pub fn diff_anchors(full_name: &str, number: i64) -> HashMap<String, (HashSet<i64>, HashSet<i64>)> {
    let mut map: HashMap<String, (HashSet<i64>, HashSet<i64>)> = HashMap::new();
    let diff = match run_raw("gh", &["pr", "diff", &number.to_string(), "-R", full_name]) {
        Ok(bytes) => String::from_utf8_lossy(&bytes).into_owned(),
        Err(_) => return map,
    };
    let mut path: Option<String> = None;
    let mut old_path: Option<String> = None;
    let (mut old_ln, mut new_ln) = (0i64, 0i64);
    for line in diff.lines() {
        if let Some(rest) = line.strip_prefix("--- a/") {
            old_path = Some(rest.trim().to_string());
            path = None;
        } else if line.starts_with("--- ") {
            old_path = None;
            path = None;
        } else if let Some(rest) = line.strip_prefix("+++ b/") {
            let name = rest.trim().to_string();
            map.entry(name.clone()).or_default();
            path = Some(name);
        } else if line.starts_with("+++ /dev/null") {
            if let Some(name) = old_path.take() {
                map.entry(name.clone()).or_default();
                path = Some(name);
            }
        } else if line.starts_with("+++") {
            path = None;
        } else if line.starts_with("@@") {
            if let Some((old_start, new_start)) = parse_hunk_starts(line) {
                old_ln = old_start;
                new_ln = new_start;
            }
        } else if let Some(entry) = path.as_ref().and_then(|p| map.get_mut(p)) {
            match line.as_bytes().first() {
                Some(b'+') => {
                    entry.0.insert(new_ln);
                    new_ln += 1;
                }
                Some(b'-') => {
                    entry.1.insert(old_ln);
                    old_ln += 1;
                }
                Some(b' ') => {
                    entry.0.insert(new_ln);
                    entry.1.insert(old_ln);
                    new_ln += 1;
                    old_ln += 1;
                }
                _ => {} // "\ No newline at end of file", blank EOF, etc.
            }
        }
    }
    map
}

/// Parse a hunk header `@@ -oldStart,oldLen +newStart,newLen @@` → (oldStart, newStart).
pub(crate) fn parse_hunk_starts(line: &str) -> Option<(i64, i64)> {
    let (mut old_start, mut new_start) = (None, None);
    for token in line.split("@@").nth(1)?.split_whitespace() {
        if let Some(rest) = token.strip_prefix('-') {
            old_start = rest.split(',').next().and_then(|n| n.parse::<i64>().ok());
        } else if let Some(rest) = token.strip_prefix('+') {
            new_start = rest.split(',').next().and_then(|n| n.parse::<i64>().ok());
        }
    }
    Some((old_start?, new_start?))
}

// This message is persisted in review history. Never include raw command output,
// which may contain credentials from authentication helpers or transport errors.
fn classify_gh_error(err: &str) -> &'static str {
    let bounded: String = err.chars().take(200).collect();
    let lower = bounded.to_ascii_lowercase();
    if lower.contains("auth")
        || lower.contains("login")
        || lower.contains("token")
        || lower.contains("401")
        || lower.contains("403")
        || lower.contains("credentials")
    {
        "authentication failure: verify GitHub CLI authentication and repository access"
    } else if lower.contains("could not resolve host")
        || lower.contains("timeout")
        || lower.contains("connection")
        || lower.contains("network")
        || lower.contains("timed out")
    {
        "transport failure: check network connectivity and retry"
    } else {
        "command error: GitHub CLI could not load the PR description"
    }
}

pub(crate) fn parse_pr_description_payload(
    full_name: &str,
    number: i64,
    bytes: &[u8],
) -> Result<(String, String), String> {
    let value: serde_json::Value = serde_json::from_slice(bytes)
        .map_err(|error| format!("Malformed PR description JSON for {full_name}#{number}: {error}"))?;
    let title = value
        .get("title")
        .and_then(|v| v.as_str())
        .ok_or_else(|| {
            format!("Malformed PR description JSON for {full_name}#{number}: missing or invalid 'title' string")
        })?
        .to_string();
    let body = value
        .get("body")
        .and_then(|v| v.as_str())
        .ok_or_else(|| {
            format!("Malformed PR description JSON for {full_name}#{number}: missing or invalid 'body' string")
        })?
        .to_string();
    Ok((title, body))
}

/// PR title + author body (the description) via gh. Returns an error if retrieval
/// fails or returns malformed data, so unavailable author context is not silently
/// treated as empty text. A successfully loaded empty body remains Ok((title, "")).
pub fn pr_description(full_name: &str, number: i64) -> Result<(String, String), String> {
    let bytes = run_raw(
        "gh",
        &[
            "pr",
            "view",
            &number.to_string(),
            "-R",
            full_name,
            "--json",
            "title,body",
        ],
    )
    .map_err(|error| {
        format!(
            "Failed to load PR description for {full_name}#{number}: {}",
            classify_gh_error(&error)
        )
    })?;
    parse_pr_description_payload(full_name, number, &bytes)
}

#[cfg(test)]
mod hunk_regression_tests {
    #[test]
    fn hunk_positions_ignore_function_context() {
        for context in [
            "fn normalize(team: &str) -> Vec<String> {",
            "fn sum() + extra - context",
            "",
        ] {
            assert_eq!(
                super::parse_hunk_starts(&format!("@@ -440,3 +306,18 @@ {context}")),
                Some((440, 306))
            );
        }
        assert_eq!(super::parse_hunk_starts("@@ -1 +2 @@"), Some((1, 2)));
        assert_eq!(super::parse_hunk_starts("not a hunk"), None);
    }

    #[test]
    fn gh_error_classification_identifies_auth_transport_and_command() {
        let auth_err = super::classify_gh_error("GraphQL: Resource not accessible by integration (403)");
        assert!(auth_err.starts_with("authentication failure:"), "{auth_err}");

        let network_err = super::classify_gh_error(
            "fatal: unable to access 'https://github.com/': Could not resolve host: github.com",
        );
        assert!(network_err.starts_with("transport failure:"), "{network_err}");

        let cmd_err = super::classify_gh_error("pull request not found for branch");
        assert!(cmd_err.starts_with("command error:"), "{cmd_err}");

        // Persisted messages have fixed size regardless of command output.
        let long_msg = "x".repeat(500);
        let classified = super::classify_gh_error(&long_msg);
        assert!(classified.len() <= "command error: ".len() + 200);
    }

    #[test]
    fn gh_error_messages_never_include_command_output() {
        let secret = "fixture-private-value";
        for error in [
            format!("authentication failed: Authorization: Bearer {secret}"),
            format!("connection refused for https://user:{secret}@github.com/acme/api"),
            format!("unexpected response: ?access_key={secret}"),
            format!("{} password={secret}", "é".repeat(250)),
        ] {
            let classified = super::classify_gh_error(&error);
            assert!(
                !classified.contains(secret),
                "Command output must not reach review history"
            );
            assert!(!classified.contains("user:"));
            assert!(!classified.contains("access_key"));
            assert!(!classified.contains("password="));
        }
    }

    #[test]
    fn pr_description_payload_requires_valid_strings_and_permits_empty_body() {
        // Valid title and body
        let valid = br#"{"title":"Add feature","body":"Description text"}"#;
        assert_eq!(
            super::parse_pr_description_payload("owner/repo", 1, valid).unwrap(),
            ("Add feature".into(), "Description text".into())
        );

        // Valid empty body succeeds
        let empty_body = br#"{"title":"Quick fix","body":""}"#;
        assert_eq!(
            super::parse_pr_description_payload("owner/repo", 2, empty_body).unwrap(),
            ("Quick fix".into(), "".into())
        );

        // Missing body fails
        let missing_body = br#"{"title":"No body"}"#;
        let err = super::parse_pr_description_payload("owner/repo", 3, missing_body).unwrap_err();
        assert!(err.contains("missing or invalid 'body' string"), "{err}");

        // Missing title fails
        let missing_title = br#"{"body":"Some body"}"#;
        let err = super::parse_pr_description_payload("owner/repo", 4, missing_title).unwrap_err();
        assert!(err.contains("missing or invalid 'title' string"), "{err}");

        // Array body fails
        let array_body = br#"{"title":"Example","body":[]}"#;
        let err = super::parse_pr_description_payload("owner/repo", 5, array_body).unwrap_err();
        assert!(err.contains("missing or invalid 'body' string"), "{err}");

        // Null body fails
        let null_body = br#"{"title":"Example","body":null}"#;
        let err = super::parse_pr_description_payload("owner/repo", 6, null_body).unwrap_err();
        assert!(err.contains("missing or invalid 'body' string"), "{err}");

        // Empty object fails
        let empty_obj = b"{}";
        assert!(super::parse_pr_description_payload("owner/repo", 7, empty_obj).is_err());

        // Malformed JSON syntax fails
        let syntax_err = b"not-json";
        assert!(super::parse_pr_description_payload("owner/repo", 8, syntax_err).is_err());
    }
}
