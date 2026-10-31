use super::*;

pub(crate) fn merge_chunk_findings(results: &[ChunkResult], head_sha: &str) -> serde_json::Value {
    let mut comments = Vec::new();
    for result in results {
        if let Some(raw) = &result.comments_json {
            if let Ok(value) = serde_json::from_str::<serde_json::Value>(raw) {
                if value.get("commit_id").and_then(|v| v.as_str()) != Some(head_sha) {
                    continue;
                }
                if let Some(items) = value.get("comments").and_then(|v| v.as_array()) {
                    for item in items {
                        if item.get("path").and_then(|v| v.as_str()).is_some()
                            && item.get("line").and_then(|v| v.as_i64()).is_some()
                            && item.get("side").and_then(|v| v.as_str()).is_some()
                            && item.get("body").and_then(|v| v.as_str()).is_some()
                        {
                            comments.push(serde_json::json!({ "chunkId": result.chunk.id, "comment": item }));
                        }
                    }
                }
            }
        }
    }
    let mut findings = serde_json::json!({ "commit_id": head_sha, "comments": comments });
    crate::review::findings::tag_candidates(&mut findings);
    findings
}

pub(crate) fn render_chunk_summaries(results: &[ChunkResult]) -> String {
    results
        .iter()
        .map(|r| {
            format!(
                "# Chunk {}\n\nFiles:\n{}\n{}\n\n",
                r.chunk.id,
                r.chunk.files.iter().map(|f| format!("- {f}\n")).collect::<String>(),
                r.summary
            )
        })
        .collect()
}

pub(crate) fn dedupe_candidates(findings: &serde_json::Value) -> serde_json::Value {
    let mut seen = std::collections::HashSet::new();
    let mut dupes = Vec::new();
    if let Some(items) = findings.get("comments").and_then(|v| v.as_array()) {
        for item in items {
            let c = item.get("comment").unwrap_or(item);
            let key = format!(
                "{}:{}:{}",
                c.get("path").and_then(|v| v.as_str()).unwrap_or_default(),
                c.get("line").and_then(|v| v.as_i64()).unwrap_or_default(),
                normalize_finding_key(c.get("body").and_then(|v| v.as_str()).unwrap_or_default())
            );
            if !seen.insert(key.clone()) {
                dupes.push(serde_json::json!({ "key": key, "candidate": item }));
            }
        }
    }
    serde_json::json!({ "duplicates": dupes })
}

pub(crate) fn comments_json_contains_items(comments_json: Option<&str>) -> bool {
    comments_json
        .and_then(|raw| serde_json::from_str::<serde_json::Value>(raw).ok())
        .and_then(|value| value.get("comments").cloned())
        .and_then(|value| value.as_array().map(|items| !items.is_empty()))
        .unwrap_or(false)
}

pub(crate) fn sanitize_review_comments(comments_json: Option<&str>) -> Result<Option<String>, String> {
    let Some(raw) = comments_json else {
        return Ok(None);
    };
    let mut value = serde_json::from_str::<serde_json::Value>(raw).map_err(|error| error.to_string())?;
    let Some(items) = value.get_mut("comments").and_then(|value| value.as_array_mut()) else {
        return Ok(None);
    };
    let mut kept = Vec::new();
    for item in items.iter() {
        let body = item.get("body").and_then(|value| value.as_str()).unwrap_or_default();
        if is_structured_review_point(body) {
            kept.push(item.clone());
        } else if looks_like_review_finding(body) {
            return Err("Review inline comment does not use the required [Finding]/[Question] template.".to_string());
        }
    }
    *items = kept;
    serde_json::to_string(&value)
        .map(Some)
        .map_err(|error| error.to_string())
}

pub(crate) fn repair_body_only_review_point(body: String, comments_json: Option<&str>) -> String {
    let (main, suggestions) = split_suggestions(&body);
    if !suggestions.is_empty() {
        let main = repair_body_only_review_point(main.trim_end().to_string(), comments_json);
        return if main.trim().is_empty() {
            suggestions.to_string()
        } else {
            format!("{main}\n\n{suggestions}")
        };
    }
    if comments_json_contains_items(comments_json)
        || is_no_issue_summary(&body)
        || is_structured_review_point(&body)
        || !looks_like_review_finding(&body)
    {
        return body;
    }
    let is_question = looks_like_body_only_question(&body);
    let title = body_only_review_title(&body, if is_question { "Question" } else { "Finding" });
    if is_question {
        format!(
            "{REVIEW_ATTRIBUTION}\n\n**[Question] {title}**\n\n{}\n\n<details>\n<summary>🔎 Why I'm asking</summary>\n\n{}\n\nWhy: The original generated review described this as a question that needs author confirmation before a code change.\n\n</details>\n\n<details>\n<summary>💬 What to confirm</summary>\n\nConfirm the intended behavior and whether this needs a code change.\n\n</details>\n\n<details>\n<summary>🤖 Prompt for AI Agents</summary>\n\n```text\n{}\n\nReview question to verify:\n{}\n\nInspect the affected changed files and ask only if still ambiguous.\n```\n\n</details>",
            body.trim(),
            body.trim(),
            QUESTION_PROMPT_PREAMBLE,
            body.trim()
        )
    } else {
        format!(
            "{REVIEW_ATTRIBUTION}\n\n**[Finding] {title}**\n\n{}\n\n<details>\n<summary>🔎 Details</summary>\n\n{}\n\nWhy: The original generated review described this as a finding that must stay visible until verified or fixed.\n\n</details>\n\n<details>\n<summary>🛠️ Suggested fix</summary>\n\nInvestigate and fix the issue described above, then validate with the relevant tests/typecheck.\n\n</details>\n\n<details>\n<summary>🤖 Prompt for AI Agents</summary>\n\n```text\n{}\n\nReview finding to verify:\n{}\n\nInspect the affected changed files and fix only if still valid.\n```\n\n</details>",
            body.trim(),
            body.trim(),
            FINDING_PROMPT_PREAMBLE,
            body.trim()
        )
    }
}

pub(crate) fn looks_like_body_only_question(body: &str) -> bool {
    let trimmed = body.trim();
    let lower = trimmed.to_lowercase();
    trimmed.ends_with('?')
        || lower.starts_with("question")
        || lower.starts_with("[question]")
        || lower.starts_with("is ")
        || lower.starts_with("are ")
        || lower.starts_with("should ")
        || lower.starts_with("can ")
}

pub(crate) fn body_only_review_title(body: &str, fallback: &str) -> String {
    let mut title = body
        .lines()
        .find(|line| !line.trim().is_empty())
        .unwrap_or(fallback)
        .trim()
        .trim_start_matches(['*', '_', '>', '-', '#'])
        .trim()
        .trim_start_matches("[Finding]")
        .trim_start_matches("[Question]")
        .trim()
        .to_string();
    let lower = title.to_lowercase();
    for prefix in ["found one ", "found a ", "finding:", "bug:", "issue:", "question:"] {
        if lower.starts_with(prefix) {
            title = title[prefix.len()..].trim().to_string();
            break;
        }
    }
    if let Some((head, _)) = title.split_once(". ") {
        title = head.trim().to_string();
    }
    let mut chars = title.chars();
    let shortened = chars.by_ref().take(120).collect::<String>();
    if chars.next().is_some() {
        title = format!("{}…", shortened.trim_end());
    }
    if title.is_empty() {
        fallback.to_string()
    } else {
        title
    }
}

pub(crate) fn validate_review_templates(body: &str, comments_json: Option<&str>) -> Result<(), String> {
    let (body, _) = split_suggestions(body);
    let comments = comments_json
        .and_then(|raw| serde_json::from_str::<serde_json::Value>(raw).ok())
        .and_then(|value| value.get("comments").cloned())
        .and_then(|value| value.as_array().cloned())
        .unwrap_or_default();
    if comments.is_empty() {
        if is_no_issue_summary(body) || is_structured_review_point(body) || !looks_like_review_finding(body) {
            return Ok(());
        }
        return Err("Review summary has a body-only finding/question but does not use the required [Finding]/[Question] template.".to_string());
    }
    for comment in comments {
        let text = comment.get("body").and_then(|value| value.as_str()).unwrap_or_default();
        if !is_structured_review_point(text) {
            return Err("Review inline comment does not use the required [Finding]/[Question] template.".to_string());
        }
    }
    Ok(())
}

pub(crate) fn looks_like_review_finding(body: &str) -> bool {
    let lower = body.trim().to_lowercase();
    lower.contains("[finding]")
        || lower.contains("[question]")
        || lower.contains("finding")
        || lower.contains("question")
        || lower.contains("bug")
        || lower.contains("issue")
        || lower.contains("suggested fix")
        || lower.contains("details")
}

pub(crate) fn is_no_issue_summary(body: &str) -> bool {
    let lower = body.trim().to_lowercase();
    let words = lower
        .split(|ch: char| !(ch.is_ascii_alphanumeric() || ch == '-'))
        .filter(|word| !word.is_empty())
        .collect::<Vec<_>>();
    lower.contains("nothing to flag")
        || lower.contains("nothing evidence-backed to flag")
        || words.iter().enumerate().any(|(index, word)| {
            *word == "no"
                && words
                    .iter()
                    .skip(index + 1)
                    .take(3)
                    .any(|word| matches!(*word, "issue" | "issues" | "bug" | "bugs" | "finding" | "findings"))
        })
        || (words.iter().enumerate().any(|(index, word)| {
            *word == "no"
                && words
                    .iter()
                    .skip(index + 1)
                    .take(3)
                    .any(|word| matches!(*word, "question" | "questions"))
        }) && !contains_positive_finding_signal(&lower))
}

pub(crate) fn contains_positive_finding_signal(lower: &str) -> bool {
    lower.contains("finding")
        || lower.contains("bug")
        || lower.contains("issue")
        || lower.contains("regression")
        || lower.contains("can fail")
        || lower.contains("hide an untemplated")
}

/// Start of the summary section that lists lower-priority suggestions instead of posting them
/// inline: a collapsed `<details>` block whose summary is `💡 Suggestions (n)`, so the review shows
/// one line until the reader opens it. It never contains a [Finding]/[Question] marker, so
/// suggestions never count as findings.
pub(crate) const SUGGESTIONS_HEADING: &str = "<details>\n<summary>💡 Suggestions";
/// The heading reviews used before the section was collapsed; stored reviews still carry it.
const LEGACY_SUGGESTIONS_HEADING: &str = "### 💡 Suggestions";
pub(crate) const SUGGESTIONS_INTRO: &str =
    "Lower-priority points, such as test gaps, stale names or docs, and duplication. They are not posted as inline comments.";

/// `body` split before its suggestions section: the review points, then the section (empty when
/// there is none).
pub(crate) fn split_suggestions(body: &str) -> (&str, &str) {
    let start = [SUGGESTIONS_HEADING, LEGACY_SUGGESTIONS_HEADING]
        .into_iter()
        .filter_map(|heading| {
            if body.starts_with(heading) {
                Some(0)
            } else {
                body.find(&format!("\n{heading}")).map(|at| at + 1)
            }
        })
        .min();
    match start {
        Some(at) => (&body[..at], &body[at..]),
        None => (body, ""),
    }
}

/// `body` without its suggestions section, for a review whose suggestions are not posted. What
/// follows the collapsed section (the note on files a focused review left out) stays.
pub(crate) fn without_suggestions(body: &str) -> String {
    let (points, section) = split_suggestions(body);
    if section.is_empty() {
        return body.to_string();
    }
    let rest = section
        .strip_prefix(SUGGESTIONS_HEADING)
        .and_then(|section| section.split_once("</details>"))
        .map_or("", |(_, rest)| rest);
    format!("{}{rest}", points.trim_end())
}

/// First line of every generated finding/question. Reviews post from the user's
/// own GitHub account, so this line is what marks the comment as automated.
pub(crate) const REVIEW_ATTRIBUTION: &str = "<sub>🤖 Automated review by OpenCodeReview</sub>";

/// Whether `line` is the attribution line, current or the legacy italic one that
/// comments posted by earlier versions still carry.
pub(crate) fn is_review_attribution_line(line: &str) -> bool {
    let line = line.trim();
    line.starts_with("<sub>🤖 Automated review") || line.starts_with("_Generated by automated code review_")
}

pub(crate) fn contains_review_attribution(body: &str) -> bool {
    body.lines().any(is_review_attribution_line)
}

pub(crate) const FINDING_PROMPT_PREAMBLE: &str = "Verify this finding against the current code.\nFix only if still valid; skip the rest with a brief reason. Keep\nchanges minimal, and validate with the relevant tests/typecheck.";
pub(crate) const QUESTION_PROMPT_PREAMBLE: &str = "Validate this review question against the current code.\nDo not apply a fix unless the author already answered. If answered,\nimplement only the confirmed path and validate with the relevant\ntests/typecheck.";

pub(crate) fn is_structured_review_point(body: &str) -> bool {
    let lower = body.to_lowercase();
    let is_finding = lower.contains("[finding]")
        && body.contains("🔎 Details")
        && body.contains("🛠️ Suggested fix")
        && body.contains(FINDING_PROMPT_PREAMBLE);
    let is_question = lower.contains("[question]")
        && body.contains("🔎 Why I'm asking")
        && body.contains("💬 What to confirm")
        && body.contains(QUESTION_PROMPT_PREAMBLE);
    (is_finding || is_question) && body.contains("🤖 Prompt for AI Agents")
}

/// Flatten `merge_chunk_findings` output (`{comments:[{chunkId, comment}]}`) into
/// a plain review comments document (`{commit_id, comments:[...]}`) so it can be
/// re-validated/anchored by `validate_final_comments` as a final-pass fallback.
pub(crate) fn chunk_findings_as_comments(chunk_findings: &serde_json::Value, head_sha: &str) -> String {
    let mut seen = std::collections::HashSet::new();
    let mut comments: Vec<serde_json::Value> = Vec::new();
    if let Some(items) = chunk_findings.get("comments").and_then(|v| v.as_array()) {
        for item in items {
            let comment = item.get("comment").cloned().unwrap_or_else(|| item.clone());
            // Drop chunk bodies that aren't a structured `[Finding]`/`[Question]`
            // point. `validate_final_comments` hard-errors on a finding-like body
            // that isn't templated, so leaving one in would abort the entire
            // fallback and discard every other valid recovered finding.
            let body = comment.get("body").and_then(|v| v.as_str()).unwrap_or_default();
            if !is_structured_review_point(body) {
                continue;
            }
            // Dedupe by the same (path, line, normalized body) identity the
            // integration pass uses (`dedupe_candidates`), so a finding that
            // appears in more than one chunk isn't posted multiple times when the
            // fallback bypasses that pass.
            let key = format!(
                "{}:{}:{}",
                comment.get("path").and_then(|v| v.as_str()).unwrap_or_default(),
                comment.get("line").and_then(|v| v.as_i64()).unwrap_or_default(),
                normalize_finding_key(comment.get("body").and_then(|v| v.as_str()).unwrap_or_default())
            );
            if seen.insert(key) {
                comments.push(comment);
            }
        }
    }
    serde_json::json!({ "commit_id": head_sha, "comments": comments }).to_string()
}

pub(crate) fn require_valid_comments(raw: &str, head_sha: &str, diff: &str) -> Result<String, String> {
    let input: serde_json::Value = serde_json::from_str(raw)
        .map_err(|error| format!("Incomplete review: invalid or missing comments JSON: {error}"))?;
    let count = input
        .get("comments")
        .and_then(|items| items.as_array())
        .ok_or("Incomplete review: comments array missing")?
        .len();
    let validated = validate_final_comments(raw, head_sha, diff)?;
    let output: serde_json::Value = serde_json::from_str(&validated).map_err(|error| error.to_string())?;
    if output["comments"].as_array().map_or(0, Vec::len) != count {
        let anchors = diff_anchors(diff);
        let mut seen = std::collections::HashSet::new();
        let rejected = input["comments"]
            .as_array()
            .unwrap()
            .iter()
            .enumerate()
            .filter_map(|(index, item)| {
                let path = item["path"].as_str().unwrap_or_default();
                let line = item["line"].as_i64().unwrap_or_default();
                let side = item["side"].as_str().unwrap_or_default();
                let body = item["body"].as_str().unwrap_or_default();
                let reason = if !is_structured_review_point(body) {
                    "missing structured template"
                } else if !anchors.contains(&(path.into(), line, side.into())) {
                    "anchor outside validation diff"
                } else if !seen.insert((path, line, side, body)) {
                    "duplicate inline comment"
                } else {
                    return None;
                };
                Some(format!("comments[{index}]: {reason}"))
            })
            .collect::<Vec<_>>();
        return Err(format!("Incomplete review: invalid or duplicate comments cannot be silently discarded. Rejected {}: {}. Report hash: {}", rejected.len(), rejected.iter().take(8).cloned().collect::<Vec<_>>().join("; "), sha256_hex(raw.as_bytes())));
    }
    Ok(validated)
}

pub(crate) fn validate_final_comments(raw: &str, head_sha: &str, diff: &str) -> Result<String, String> {
    if raw.trim().is_empty() {
        return Ok(format!(r#"{{"commit_id":"{head_sha}","comments":[]}}"#));
    }
    let mut value: serde_json::Value =
        serde_json::from_str(raw.trim()).map_err(|error| format!("final comments.json is invalid JSON: {error}"))?;
    if value.get("commit_id").and_then(|v| v.as_str()) != Some(head_sha) {
        return Err("final comments.json commit_id does not match PR head SHA".to_string());
    }
    let anchors = diff_anchors(diff);
    let Some(items) = value.get_mut("comments").and_then(|v| v.as_array_mut()) else {
        return Err("final comments.json missing comments array".to_string());
    };
    for item in items.iter() {
        let body = item.get("body").and_then(|v| v.as_str()).unwrap_or_default();
        if !is_structured_review_point(body) && looks_like_review_finding(body) {
            return Err("Review inline comment does not use the required [Finding]/[Question] template.".to_string());
        }
    }
    let mut seen = std::collections::HashSet::new();
    items.retain(|item| {
        let path = item.get("path").and_then(|v| v.as_str()).unwrap_or_default();
        let line = item.get("line").and_then(|v| v.as_i64()).unwrap_or_default();
        let side = item.get("side").and_then(|v| v.as_str()).unwrap_or_default();
        let body = item.get("body").and_then(|v| v.as_str()).unwrap_or_default();
        let key = (path.to_string(), line, side.to_string(), body.to_string());
        is_structured_review_point(body)
            && anchors.contains(&(path.to_string(), line, side.to_string()))
            && seen.insert(key)
    });
    serde_json::to_string(&value).map_err(|error| error.to_string())
}
