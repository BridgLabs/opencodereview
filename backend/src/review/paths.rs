use super::*;

pub(crate) fn incremental_review_changes(
    repo: &Path,
    old: &str,
    head: &str,
    full_files: &[String],
    cancel: &AtomicBool,
) -> Result<(Vec<String>, String), String> {
    let range = format!("{old}..{head}");
    let mut changed = changed_files_local(repo, &range, cancel)?;
    changed.retain(|path| full_files.contains(path));
    let diff = diff_local_paths(repo, &range, &changed, cancel);
    if !changed.is_empty() && diff.is_empty() {
        return Err("Incremental diff is unavailable".into());
    }
    Ok((changed, diff))
}

#[cfg(test)]
pub(crate) fn incremental_diff_is_publishable(diff: &str, full_diff: &str) -> bool {
    let published = diff_anchor_lines(full_diff);
    diff_anchor_lines(diff)
        .iter()
        .all(|(anchor, text)| published.get(anchor) == Some(text))
}

pub(crate) fn diff_anchors(diff: &str) -> std::collections::HashSet<(String, i64, String)> {
    diff_anchor_lines(diff).into_keys().collect()
}

pub(crate) fn diff_anchor_lines(diff: &str) -> std::collections::HashMap<(String, i64, String), String> {
    let mut anchors = std::collections::HashMap::new();
    let mut path = String::new();
    let mut old_line = 0i64;
    let mut new_line = 0i64;
    for line in diff.lines() {
        if line.starts_with("diff --git ") {
            path.clear();
        } else if let Some(raw) = line.strip_prefix("+++ ").or_else(|| line.strip_prefix("--- ")) {
            if let Some(decoded) = crate::review_archive::decoded_git_path(raw) {
                if let Some(relative) = decoded.strip_prefix("a/").or_else(|| decoded.strip_prefix("b/")) {
                    path = relative.into();
                }
            }
        } else if line.starts_with("@@") {
            if let Some((old_start, new_start)) = parse_hunk_header(line) {
                old_line = old_start;
                new_line = new_start;
            }
        } else if line.starts_with('+') && !line.starts_with("+++") {
            anchors.insert((path.clone(), new_line, "RIGHT".to_string()), line[1..].to_string());
            new_line += 1;
        } else if line.starts_with('-') && !line.starts_with("---") {
            anchors.insert((path.clone(), old_line, "LEFT".to_string()), line[1..].to_string());
            old_line += 1;
        } else if !line.starts_with("diff --git") && !line.starts_with("index ") {
            old_line += 1;
            new_line += 1;
        }
    }
    anchors
}

/// The anchor that line `number` (1-based) of `diff` itself adds or removes, if any. Models that
/// read PR_DIFF.patch with line numbers sometimes report that line number instead of the file's.
pub(crate) fn diff_line_anchor(diff: &str, number: i64) -> Option<(String, i64, String)> {
    let mut path = String::new();
    let mut old_line = 0i64;
    let mut new_line = 0i64;
    for (index, line) in diff.lines().enumerate() {
        let here = index as i64 + 1 == number;
        if line.starts_with("diff --git ") {
            path.clear();
        } else if let Some(raw) = line.strip_prefix("+++ ").or_else(|| line.strip_prefix("--- ")) {
            if let Some(decoded) = crate::review_archive::decoded_git_path(raw) {
                if let Some(relative) = decoded.strip_prefix("a/").or_else(|| decoded.strip_prefix("b/")) {
                    path = relative.into();
                }
            }
        } else if line.starts_with("@@") {
            if let Some((old_start, new_start)) = parse_hunk_header(line) {
                old_line = old_start;
                new_line = new_start;
            }
        } else if line.starts_with('+') {
            if here {
                return Some((path, new_line, "RIGHT".into()));
            }
            new_line += 1;
        } else if line.starts_with('-') {
            if here {
                return Some((path, old_line, "LEFT".into()));
            }
            old_line += 1;
        } else if !line.starts_with("index ") {
            old_line += 1;
            new_line += 1;
        }
        if here {
            return None;
        }
    }
    None
}

pub(crate) fn parse_hunk_header(line: &str) -> Option<(i64, i64)> {
    let mut parts = line.split_whitespace();
    parts.next()?;
    let old = parts.next()?.trim_start_matches('-').split(',').next()?.parse().ok()?;
    let new = parts.next()?.trim_start_matches('+').split(',').next()?.parse().ok()?;
    Some((old, new))
}

pub(crate) fn sum_usage(usages: Vec<RunUsage>) -> RunUsage {
    let mut per_model = std::collections::HashMap::new();
    for usage in usages {
        for entry in usage.by_model {
            let item = per_model.entry(entry.model.clone()).or_insert_with(|| ModelUsageEntry {
                model: entry.model.clone(),
                ..Default::default()
            });
            item.input_tokens += entry.input_tokens;
            item.output_tokens += entry.output_tokens;
            item.cache_read_tokens += entry.cache_read_tokens;
            item.cache_creation_tokens += entry.cache_creation_tokens;
        }
    }
    finalize_usage(per_model)
}

pub(crate) fn review_range(incremental_base: Option<&str>, base_rev: &str, head_sha: &str) -> String {
    incremental_base
        .map(|old| format!("{old}..{head_sha}"))
        .unwrap_or_else(|| format!("{base_rev}...{head_sha}"))
}

pub(crate) fn copy_if_exists(from: &Path, to: &Path) {
    if from.exists() {
        if let Some(parent) = to.parent() {
            let _ = fs::create_dir_all(parent);
        }
        let _ = fs::copy(from, to);
    }
}

pub(crate) fn copy_tree(from: &Path, to: &Path) {
    for path in collect_files(from) {
        if let Ok(rel) = path.strip_prefix(from) {
            copy_if_exists(&path, &to.join(rel));
        }
    }
}

pub(crate) fn copy_dir_subset(from: &Path, to: &Path, rels: &[String]) {
    for rel in rels {
        copy_if_exists(&from.join(rel), &to.join(rel));
    }
}

pub(crate) fn is_noise_path(path: &str) -> bool {
    is_generated_or_lockfile(path)
        || path.contains("/target/")
        || path.ends_with(".png")
        || path.ends_with(".jpg")
        || path.ends_with(".jpeg")
        || path.ends_with(".gif")
        || path.ends_with(".pdf")
}

/// Whether a changed file counts toward the review file limit: the review reads it. Paths the
/// evidence policy excludes and noise without risk (build output, vendored code, bundles, maps,
/// generated code) are left out of the review, so they do not count.
pub(crate) fn counts_toward_file_limit(path: &str) -> bool {
    !matches!(crate::review_archive::path_exclusion(path), Ok(Some(_)))
        && (!is_noise_path(path) || is_risk_bearing_path(path))
}

pub(crate) fn is_risk_bearing_path(path: &str) -> bool {
    is_risky_path(path) || is_config_path(path) || looks_public_contract(path)
}

pub(crate) fn categorize_path(path: &str) -> &'static str {
    let lower = path.to_ascii_lowercase();
    if is_test_path(path) {
        "tests"
    } else if is_noise_path(path) {
        "generated/vendor/noise"
    } else if lower.contains("migration") || lower.contains("schema") || lower.contains("model") {
        "DB/schema/migration"
    } else if lower.contains("docker") || is_config_path(path) {
        "config/runtime/Docker"
    } else if lower.ends_with(".md") || lower.starts_with("docs/") {
        "docs"
    } else if lower.ends_with(".rs") || lower.contains("server") || lower.contains("api") {
        "backend/API"
    } else if lower.ends_with(".tsx") || lower.ends_with(".ts") || lower.ends_with(".jsx") || lower.ends_with(".js") {
        "frontend"
    } else {
        "other"
    }
}

pub(crate) fn looks_public_contract(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    lower.contains("api")
        || lower.contains("schema")
        || lower.contains("types")
        || lower.contains("sdk")
        || lower.contains("cli")
        || lower.contains("config")
}

pub(crate) fn extract_http_refs(diff: &str) -> Vec<HttpRef> {
    let mut refs = Vec::new();
    let mut seen = std::collections::BTreeSet::new();
    let mut path = String::new();
    for line in diff.lines() {
        if line.starts_with("diff --git ") {
            path.clear();
            continue;
        }
        if let Some(rest) = line.strip_prefix("+++ b/").or_else(|| line.strip_prefix("--- a/")) {
            path = rest.to_string();
            continue;
        }
        if path.is_empty() {
            continue;
        }
        let Some(route) = extract_route(line) else {
            continue;
        };
        for method in ["GET", "POST", "PUT", "PATCH", "DELETE"] {
            if line
                .split(|c: char| !c.is_ascii_alphanumeric() && c != '_')
                .any(|token| token.eq_ignore_ascii_case(method))
                && seen.insert((path.clone(), method.to_string(), route.clone()))
            {
                refs.push(HttpRef {
                    path: path.clone(),
                    method: method.to_string(),
                    route: route.clone(),
                    side: if categorize_path(&path) == "backend/API" {
                        "backend"
                    } else {
                        "frontend"
                    }
                    .to_string(),
                });
            }
        }
    }
    refs
}

pub(crate) fn extract_route(line: &str) -> Option<String> {
    line.split(['\'', '"', '`'])
        .find(|part| part.starts_with('/') && part.len() > 1)
        .map(|part| part.split_whitespace().next().unwrap_or(part).to_string())
}

pub(crate) fn test_relationship_hints(changed: &[String]) -> Vec<String> {
    let impls: Vec<&String> = changed.iter().filter(|p| !is_test_path(p)).collect();
    changed
        .iter()
        .filter(|p| is_test_path(p))
        .filter_map(|test| {
            let stem = path_stem_key(test);
            impls
                .iter()
                .find(|imp| related_by_name(&stem, imp))
                .map(|imp| format!("{} may test {}", test, imp))
        })
        .collect()
}

pub(crate) fn path_stem_key(path: &str) -> String {
    Path::new(path)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or(path)
        .replace(".test", "")
        .replace(".spec", "")
        .replace("_test", "")
        .to_ascii_lowercase()
}

pub(crate) fn package_key(path: &str) -> String {
    let parts = path.split('/').collect::<Vec<_>>();
    if parts.first() == Some(&"packages") && parts.len() >= 3 {
        return parts[..3].join("/");
    }
    if parts.first() == Some(&"apps") && parts.len() >= 2 {
        return parts[..2].join("/");
    }
    parts.first().copied().unwrap_or_default().to_string()
}

pub(crate) fn directory_key(path: &str) -> String {
    Path::new(path)
        .parent()
        .and_then(|parent| parent.to_str())
        .unwrap_or_default()
        .to_string()
}

pub(crate) fn path_topic_tokens(path: &str) -> std::collections::HashSet<String> {
    path.split(|c: char| !c.is_ascii_alphanumeric())
        .map(|token| token.to_ascii_lowercase())
        .filter(|token| token.len() >= 4)
        .filter(|token| {
            !matches!(
                token.as_str(),
                "src"
                    | "test"
                    | "tests"
                    | "component"
                    | "components"
                    | "index"
                    | "handler"
                    | "handlers"
                    | "utils"
                    | "features"
                    | "packages"
                    | "apps"
            )
        })
        .collect()
}

pub(crate) fn shared_topic_tokens(a: &str, b: &str) -> usize {
    let a_tokens = path_topic_tokens(a);
    let b_tokens = path_topic_tokens(b);
    a_tokens.intersection(&b_tokens).count()
}

pub(crate) fn test_source_pair(a: &str, b: &str) -> bool {
    is_test_path(a) != is_test_path(b) && shared_topic_tokens(a, b) > 0
}

#[cfg(test)]
pub(crate) fn is_bridge_context_path(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    is_config_path(path)
        || is_risky_path(path)
        || looks_public_contract(path)
        || lower.contains("util")
        || lower.contains("config")
        || lower.contains("type")
        || lower.contains("handler")
        || lower.contains("timezone")
        || lower.contains("filter")
}

pub(crate) fn related_by_name(stem: &str, path: &str) -> bool {
    !stem.is_empty() && path.to_ascii_lowercase().contains(stem)
}

pub(crate) fn risk_group_match(a: &str, b: &str) -> bool {
    (is_config_path(a) && is_config_path(b))
        || (is_risky_path(a) && is_risky_path(b))
        || (categorize_path(a) == "DB/schema/migration" && categorize_path(b) == "DB/schema/migration")
}

pub(crate) fn normalize_finding_key(body: &str) -> String {
    body.lines()
        .map(str::trim)
        .find(|line| !line.is_empty() && !is_review_attribution_line(line))
        .unwrap_or(body)
        .chars()
        .filter(|c| c.is_alphanumeric() || c.is_whitespace())
        .collect::<String>()
        .to_ascii_lowercase()
        .split_whitespace()
        .take(16)
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod file_limit_tests {
    #[test]
    fn file_limit_counts_only_files_a_review_reads() {
        for counted in [
            "src/main.rs",
            "package-lock.json",
            "go.sum",
            "src/old_module.rs",
            "deploy/generated/config.yaml",
        ] {
            assert!(super::counts_toward_file_limit(counted), "{counted}");
        }
        for skipped in [
            "dist/app.js",
            "vendor/github.com/pkg/errors/errors.go",
            "static/app.min.js",
            "static/app.js.map",
            "internal/pb/service.pb.go",
            "assets/logo.png",
            "src/__generated__/types.ts",
            // The evidence policy never freezes these, so a review cannot read them.
            "Cargo.lock",
            "pnpm-lock.yaml",
        ] {
            assert!(!super::counts_toward_file_limit(skipped), "{skipped}");
        }
    }
}
