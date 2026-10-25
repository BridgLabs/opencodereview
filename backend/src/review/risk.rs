use super::*;

#[cfg(test)]
pub(crate) fn empty_review_risk() -> ReviewRisk {
    ReviewRisk {
        score: 0,
        level: "low".to_string(),
        summary: "Risk pending.".to_string(),
        auto_merge_eligible: false,
        drivers: Vec::new(),
    }
}

pub(crate) fn apply_evidence_warning_risk(
    risk: &mut ReviewRisk,
    warnings: &[crate::review::warnings::EvidenceWarning],
) {
    if warnings.is_empty() {
        return;
    }
    risk.auto_merge_eligible = false;
    let level = match risk.level.as_str() {
        "high" => "High",
        "medium" => "Medium",
        _ => "Low",
    };
    risk.summary =
        format!("{level} landing risk. Supporting evidence is incomplete; human review is required before merge.");
}

pub(crate) fn score_review_risk(plan: &ReviewContextPlan, diff: &str, comments_json: Option<&str>) -> ReviewRisk {
    let (added, removed) = diff_churn(diff);
    let changed = plan.changed_files.len() as i64;
    let related = plan.related_files.len() as i64;
    let tests_changed = plan.changed_files.iter().filter(|path| is_test_path(path)).count() as i64;
    let comments = comments_json
        .and_then(|raw| serde_json::from_str::<serde_json::Value>(raw).ok())
        .and_then(|value| value["comments"].as_array().cloned())
        .unwrap_or_default()
        .iter()
        .filter(|comment| {
            crate::review::findings::point_kind(comment["body"].as_str().unwrap_or_default()) == "finding"
        })
        .count() as i64;
    let critical_paths: Vec<&String> = plan.changed_files.iter().filter(|path| is_risky_path(path)).collect();
    let config_paths: Vec<&String> = plan.changed_files.iter().filter(|path| is_config_path(path)).collect();
    let generated_paths = plan
        .changed_files
        .iter()
        .filter(|path| is_generated_or_lockfile(path))
        .count() as i64;
    let source_changed = (changed - tests_changed - generated_paths).max(0);
    let mut drivers = Vec::new();
    let mut score = 5;
    let churn_points = match added + removed {
        n if n >= 2000 => 30,
        n if n >= 800 => 22,
        n if n >= 300 => 14,
        n if n >= 80 => 7,
        n if n > 0 => 2,
        _ => 0,
    };
    push_driver(
        &mut drivers,
        churn_points,
        "Diff size",
        format!("{} added / {} removed lines", added, removed),
    );
    score += churn_points;
    let file_points = match changed {
        n if n >= 30 => 18,
        n if n >= 15 => 12,
        n if n >= 7 => 7,
        n if n >= 3 => 3,
        _ => 0,
    };
    push_driver(
        &mut drivers,
        file_points,
        "File spread",
        format!("{} changed files", changed),
    );
    score += file_points;
    let blast_points = (plan.dependent_packages.len() as i64 * 3 + (related / 6)).min(18);
    push_driver(
        &mut drivers,
        blast_points,
        "Blast radius",
        format!(
            "{} dependent packages, {} related files",
            plan.dependent_packages.len(),
            related
        ),
    );
    score += blast_points;
    let package_points = (plan
        .packages
        .iter()
        .filter(|pkg| pkg.kind == "app" || pkg.kind == "lib" || pkg.kind == "root")
        .count() as i64
        * 3)
    .min(12);
    push_driver(
        &mut drivers,
        package_points,
        "Touched packages",
        format!("{} indexed packages affected", plan.packages.len()),
    );
    score += package_points;
    let critical_points = (critical_paths.len() as i64 * 10).min(25);
    push_driver(
        &mut drivers,
        critical_points,
        "Critical surface",
        sample_paths(&critical_paths),
    );
    score += critical_points;
    let config_points = (config_paths.len() as i64 * 5).min(15);
    push_driver(
        &mut drivers,
        config_points,
        "Config/build surface",
        sample_paths(&config_paths),
    );
    score += config_points;
    if source_changed > 0 && tests_changed == 0 {
        push_driver(
            &mut drivers,
            12,
            "No changed tests",
            "Source changed without nearby test updates".to_string(),
        );
        score += 12;
    } else if tests_changed > 0 {
        push_driver(
            &mut drivers,
            -8,
            "Tests changed",
            format!("{} test files changed", tests_changed),
        );
        score -= 8;
    }
    if source_changed == 0 && generated_paths == changed && changed > 0 {
        push_driver(
            &mut drivers,
            -18,
            "Generated/lockfile-only",
            "Only lockfiles or generated files changed".to_string(),
        );
        score -= 18;
    }
    drivers.sort_by(|a, b| {
        b.points
            .abs()
            .cmp(&a.points.abs())
            .then_with(|| b.points.cmp(&a.points))
    });
    let score = score.clamp(0, 100);
    let level = if score >= 70 {
        "high"
    } else if score >= 35 {
        "medium"
    } else {
        "low"
    }
    .to_string();
    let auto_merge_eligible = score < 25 && comments == 0 && source_changed <= 3 && critical_paths.is_empty();
    let summary = match level.as_str() {
        "high" => "High landing risk — human review recommended before merge.",
        "medium" => "Medium landing risk — check the drivers before merging.",
        _ if auto_merge_eligible => "Low landing risk — eligible for auto-merge if CI and policy checks pass.",
        _ => "Low landing risk — still requires normal CI and approval policy.",
    }
    .to_string();
    ReviewRisk {
        score,
        level,
        summary,
        auto_merge_eligible,
        drivers: drivers.into_iter().take(6).collect(),
    }
}

pub(crate) fn push_driver(drivers: &mut Vec<ReviewRiskDriver>, points: i64, label: &str, detail: String) {
    if points != 0 {
        drivers.push(ReviewRiskDriver {
            label: label.to_string(),
            detail,
            points,
        });
    }
}

pub(crate) fn diff_churn(diff: &str) -> (i64, i64) {
    diff.lines().fold((0, 0), |(added, removed), line| {
        if line.starts_with("+++") || line.starts_with("---") {
            (added, removed)
        } else if line.starts_with('+') {
            (added + 1, removed)
        } else if line.starts_with('-') {
            (added, removed + 1)
        } else {
            (added, removed)
        }
    })
}

pub(crate) fn review_comment_count(comments_json: Option<&str>) -> i64 {
    let Some(value) = comments_json.and_then(|json| serde_json::from_str::<serde_json::Value>(json).ok()) else {
        return 0;
    };
    value
        .get("comments")
        .and_then(|comments| comments.as_array())
        .map(|comments| comments.len() as i64)
        .unwrap_or(0)
}

pub(crate) fn is_test_path(path: &str) -> bool {
    path.contains(".test.") || path.contains(".spec.") || path.contains("/__tests__/") || path.contains("/tests/")
}

pub(crate) fn is_risky_path(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    [
        "auth",
        "permission",
        "payment",
        "billing",
        "checkout",
        "security",
        "crypto",
        "migration",
        "schema",
        "database",
        "queue",
        "concurrency",
        "thread",
        "token",
        "secret",
    ]
    .iter()
    .any(|needle| lower.contains(needle))
}

pub(crate) fn is_config_path(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    lower.ends_with(".yml")
        || lower.ends_with(".yaml")
        || lower.ends_with(".toml")
        || lower.ends_with(".lock")
        || lower.ends_with(".sql")
        || lower.ends_with("dockerfile")
        || lower.contains("package.json")
        || lower.contains("cargo.toml")
        || lower.contains("migrations/")
        || lower.contains("/config/")
        || lower.contains(".github/workflows/")
}

pub(crate) fn is_generated_or_lockfile(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    lower.ends_with(".lock")
        || lower.ends_with("generated.ts")
        || lower.ends_with("generated.js")
        || lower.contains("/generated/")
}

pub(crate) fn sample_paths(paths: &[&String]) -> String {
    paths
        .iter()
        .take(3)
        .map(|path| path.as_str())
        .collect::<Vec<_>>()
        .join(", ")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn low_risk() -> ReviewRisk {
        ReviewRisk {
            score: 19,
            level: "low".into(),
            summary: "Low landing risk. Eligible for auto-merge if CI and policy checks pass.".into(),
            auto_merge_eligible: true,
            drivers: vec![ReviewRiskDriver {
                points: 12,
                label: "No changed tests".into(),
                detail: "Source changed without nearby test updates".into(),
            }],
        }
    }

    #[test]
    fn evidence_warnings_disable_auto_merge_and_explain_human_review() {
        let warnings = vec![crate::review::warnings::EvidenceWarning {
            reason: "insufficient_evidence".into(),
            path: Some("docs/identities.md".into()),
            reference: None,
        }];
        for (level, score, label) in [("low", 19, "Low"), ("medium", 45, "Medium"), ("high", 80, "High")] {
            let mut risk = low_risk();
            risk.level = level.into();
            risk.score = score;
            risk.auto_merge_eligible = level == "low";
            let drivers = serde_json::to_value(&risk.drivers).unwrap();

            apply_evidence_warning_risk(&mut risk, &warnings);

            assert!(!risk.auto_merge_eligible);
            assert_eq!(risk.level, level);
            assert_eq!(risk.score, score);
            assert_eq!(serde_json::to_value(&risk.drivers).unwrap(), drivers);
            assert_eq!(
                risk.summary,
                format!(
                    "{label} landing risk. Supporting evidence is incomplete; human review is required before merge."
                )
            );
        }
    }

    #[test]
    fn no_evidence_warnings_preserve_auto_merge_eligibility_and_summary() {
        let mut risk = low_risk();
        let original = serde_json::to_value(&risk).unwrap();

        apply_evidence_warning_risk(&mut risk, &[]);

        assert_eq!(serde_json::to_value(risk).unwrap(), original);
    }
}
