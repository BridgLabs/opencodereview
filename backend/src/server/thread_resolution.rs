use super::*;

/// Resolves the review threads of findings this review proved fixed. It never fails the review or
/// its publication: a GitHub failure is logged and noted on the run, and the next review lists
/// the still open findings again and retries.
pub(crate) fn resolve_fixed_threads(db: &Database, inputs: &review::ReviewInputs, output: &review::ReviewOutput) {
    // A review with limited evidence, or one that reviewed nothing new, proves no fix.
    if output.no_new_changes || !output.evidence_warnings.is_empty() || !inputs.resolutions.any_fixed() {
        return;
    }
    let warn = |reason: &str| {
        eprintln!(
            "thread-resolution {}#{} run {}: {reason}",
            inputs.full_name, inputs.number, inputs.run_id
        );
        let _ = db.note_review_run_warning(inputs.run_id, &format!("Review thread resolution: {reason}"));
    };
    let Ok(Some(run)) = db.review_run_by_id(inputs.run_id) else {
        return;
    };
    // The proof is about this head only; a newer head or a closed PR leaves every thread alone.
    match github::live_pr_state(&inputs.full_name, inputs.number) {
        Ok(live) if live.state == "OPEN" && live.head_sha == run.head_sha => {}
        Ok(_) => return,
        Err(error) => return warn(&format!("pull request state unavailable: {error}")),
    }
    let threads = match github::published_finding_threads(&inputs.full_name, inputs.number) {
        Ok(threads) => threads,
        Err(error) => return warn(&format!("review threads unavailable: {error}")),
    };
    let mut failed = Vec::new();
    for thread in inputs
        .resolutions
        .threads_to_resolve(&threads, output.comments_json.as_deref())
    {
        if let Err(error) = github::resolve_review_thread(&thread.thread_id) {
            failed.push(error);
        }
    }
    if let Some(error) = failed.first() {
        warn(&format!(
            "could not resolve {} fixed thread(s): {error}. The next review retries.",
            failed.len()
        ));
    }
}
