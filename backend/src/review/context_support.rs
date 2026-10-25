use super::*;

pub(crate) fn render_context_md(
    inputs: &ReviewInputs,
    plan: &ReviewContextPlan,
    head_sha: &str,
    changed: &[String],
) -> String {
    let mut out = String::new();
    out.push_str(&format!(
        "# Review context for {} PR #{}\n\n",
        inputs.full_name, inputs.number
    ));
    out.push_str("This pack is CONTEXT to help you, not rules. The repo's review skill is the source of truth.\n\n");
    out.push_str(&format!("- Head SHA: {head_sha}\n- Base: {}\n\n", inputs.base_ref));
    out.push_str(&format!(
        "- Review range: {}\n- Scope: {}\n- Base SHA: {}\n- Merge-base SHA: {}\n\n",
        plan.range,
        plan.scope_reason.as_deref().unwrap_or("Full review"),
        plan.reviewed_base_sha.as_deref().unwrap_or("unavailable"),
        plan.merge_base_sha.as_deref().unwrap_or("unavailable")
    ));
    if let Some(selection) = &plan.scope_selection {
        out.push_str(&format!(
            "- Selection mode: {}\n- Files assigned: {} of {} PR files\n- Files with reused coverage: {}\n\n",
            selection.mode,
            changed.len(),
            selection.full_pr_files.len(),
            selection.reused_files.len()
        ));
    }
    out.push_str("## Changed files (review these; head content under context/changed/)\n");
    for path in changed {
        out.push_str(&format!("- {path}\n"));
    }
    out.push_str("\n## Owning packages\n");
    for package in &plan.packages {
        out.push_str(&format!(
            "- {} ({}, {})\n",
            package.package_path, package.name, package.kind
        ));
    }
    if !plan.dependent_packages.is_empty() {
        out.push_str("\n## Dependent packages (impacted by these changes)\n");
        for path in &plan.dependent_packages {
            out.push_str(&format!("- {path}\n"));
        }
    }
    out.push_str("\n## Related files (pinned head content under context/related/)\n");
    for file in &plan.related_files {
        out.push_str(&format!("- {} — {}\n", file.path, file.reason));
    }
    if plan.omitted_count > 0 {
        out.push_str(&format!(
            "- … {} more related files omitted (bounded)\n",
            plan.omitted_count
        ));
    }
    let unlisted_tests = plan
        .test_files
        .iter()
        .filter(|path| !plan.related_files.iter().any(|file| &file.path == *path))
        .collect::<Vec<_>>();
    if !unlisted_tests.is_empty() {
        out.push_str("\n## Other tests of the change\n");
        for path in unlisted_tests {
            out.push_str(&format!("- {path}\n"));
        }
    }
    out.push_str("\n## Conventions (base-branch content under context/conventions/)\n");
    for path in &plan.convention_files {
        out.push_str(&format!("- {path}\n"));
    }
    out
}

pub(crate) fn render_prompt(
    inputs: &ReviewInputs,
    context_dir: &Path,
    summary_path: &Path,
    files: &[String],
) -> String {
    frozen::prompt(inputs, context_dir, files, &summary_path.with_file_name("report.json"))
}
