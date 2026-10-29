use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use sha2::{Digest, Sha256};

#[derive(Debug, Clone)]
pub(crate) struct CoverageResolution {
    pub review_base: Option<String>,
    pub fully_covered: bool,
    pub usable: bool,
}

#[derive(Debug, Clone)]
struct CommitPatch {
    parent: Option<String>,
    patch_id: String,
}

fn patch_id(repo: &Path, diff: &str, cancel: &AtomicBool) -> Result<Option<String>, String> {
    if diff.trim().is_empty() {
        return Ok(None);
    }
    if cancel.load(Ordering::Relaxed) {
        return Err("cancelled".into());
    }
    let mut child = Command::new("git")
        .env("GIT_NO_REPLACE_OBJECTS", "1")
        .arg("-C")
        .arg(repo)
        .args(["patch-id", "--stable"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| error.to_string())?;
    child
        .stdin
        .take()
        .ok_or("Could not open git patch-id input")?
        .write_all(diff.as_bytes())
        .map_err(|error| error.to_string())?;
    let started = Instant::now();
    loop {
        if cancel.load(Ordering::Relaxed) {
            let _ = child.kill();
            let _ = child.wait();
            return Err("cancelled".into());
        }
        if started.elapsed() > Duration::from_secs(60) {
            let _ = child.kill();
            let _ = child.wait();
            return Err("git patch-id timed out".into());
        }
        if child.try_wait().map_err(|error| error.to_string())?.is_some() {
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    let output = child.wait_with_output().map_err(|error| error.to_string())?;
    if cancel.load(Ordering::Relaxed) {
        return Err("cancelled".into());
    }
    if !output.status.success() {
        return Err(format!(
            "git patch-id failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout)
        .split_whitespace()
        .next()
        .filter(|value| !value.is_empty())
        .map(ToString::to_string))
}

fn commit_patches(repo: &Path, range: &str, cancel: &AtomicBool) -> Result<Vec<CommitPatch>, String> {
    let raw = crate::review::git_capture(repo, &["rev-list", "--reverse", range], cancel, 60)?;
    let commit_text = String::from_utf8_lossy(&raw);
    let commits = commit_text
        .lines()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .collect::<Vec<_>>();
    let mut patches = Vec::new();
    for commit in commits {
        let parents = crate::review::git_capture(repo, &["rev-list", "--parents", "-n", "1", commit], cancel, 30)?;
        let parents = String::from_utf8_lossy(&parents)
            .split_whitespace()
            .skip(1)
            .map(ToString::to_string)
            .collect::<Vec<_>>();
        let Some(parent) = parents.first() else {
            continue;
        };
        let diff = crate::review::git_capture(repo, &["diff", parent, commit], cancel, 90)?;
        if let Some(patch_id) = patch_id(repo, &String::from_utf8_lossy(&diff), cancel)? {
            patches.push(CommitPatch {
                parent: Some(parent.clone()),
                patch_id,
            });
        }
    }
    Ok(patches)
}

pub(crate) fn current_patch_ids(
    repo: &Path,
    base: &str,
    head: &str,
    full_diff: &str,
    cancel: &AtomicBool,
) -> Result<Vec<String>, String> {
    let mut ids = commit_patches(repo, &format!("{base}..{head}"), cancel)?
        .into_iter()
        .map(|patch| patch.patch_id)
        .collect::<Vec<_>>();
    if let Some(id) = patch_id(repo, full_diff, cancel)? {
        ids.push(id);
    }
    ids.sort();
    ids.dedup();
    Ok(ids)
}

pub(crate) fn snapshot_for_reuse(
    repo: &Path,
    db: &crate::db::Database,
    repo_id: i64,
    pr_id: i64,
    head: &str,
    cancel: &AtomicBool,
) -> Result<Option<crate::review::scope::CoverageSnapshot>, String> {
    if cancel.load(Ordering::Relaxed) {
        return Err("cancelled".into());
    }
    if let Some(coverage) = db.review_coverage_snapshot(pr_id, head)? {
        let Err(error) = crate::review::scope::verify_coverage(repo, &coverage, cancel) else {
            return Ok(Some(coverage));
        };
        if cancel.load(Ordering::Relaxed) {
            return Err("cancelled".into());
        }
        if !db.is_legacy_coverage_backfill(coverage.run_id)? {
            return Err(error);
        }
        crate::review::scope::verify_legacy_backfill(repo, &coverage, cancel)?;
        let candidate = db
            .historical_review_coverage_for_head(repo_id, pr_id, head, true)?
            .filter(|candidate| {
                candidate.run_id == coverage.run_id
                    && candidate.base_ref == coverage.scope.base_ref
                    && candidate.base_sha == coverage.scope.base_sha
                    && candidate.head_sha == coverage.scope.head_sha
            })
            .ok_or_else(|| error.clone())?;
        return recover_full_coverage(repo, db, repo_id, &candidate, cancel)?
            .map(Some)
            .ok_or(error);
    }
    let Some(candidate) = db.historical_review_coverage_for_head(repo_id, pr_id, head, false)? else {
        return Ok(None);
    };
    recover_full_coverage(repo, db, repo_id, &candidate, cancel)
}

fn recover_full_coverage(
    repo: &Path,
    db: &crate::db::Database,
    repo_id: i64,
    candidate: &crate::db::HistoricalReviewCoverageCandidate,
    cancel: &AtomicBool,
) -> Result<Option<crate::review::scope::CoverageSnapshot>, String> {
    let Some(raw) = candidate.manifest_json.as_deref() else {
        return Ok(None);
    };
    let Ok(plan) = serde_json::from_str::<crate::db::ReviewContextPlan>(raw) else {
        return Ok(None);
    };
    let range = format!("{}...{}", candidate.base_sha, candidate.head_sha);
    if plan.incremental
        || plan.changed_files.is_empty()
        || plan.repo_id != repo_id
        || plan.range != range
        || plan.head_sha.as_deref() != Some(candidate.head_sha.as_str())
        || plan.reviewed_base_sha.as_deref() != Some(candidate.base_sha.as_str())
        || plan.base_ref.as_deref() != Some(candidate.base_ref.as_str())
        || plan
            .scope_selection
            .as_ref()
            .is_some_and(|selection| !selection.reused_files.is_empty())
        || [&candidate.head_sha, &candidate.base_sha]
            .iter()
            .any(|sha| !matches!(sha.len(), 40 | 64) || !sha.bytes().all(|byte| byte.is_ascii_hexdigit()))
    {
        return Ok(None);
    }
    let Some(audit) = db.load_finding_audit(candidate.run_id)? else {
        return Ok(None);
    };
    if !audit.complete
        || audit.head_sha != candidate.head_sha
        || audit
            .evidence_warnings
            .as_ref()
            .is_some_and(|warnings| !warnings.is_empty())
    {
        return Ok(None);
    }
    let merge_base_sha = crate::review::scope::merge_base(repo, &candidate.base_sha, &candidate.head_sha, cancel)?;
    if plan.merge_base_sha.as_deref() != Some(merge_base_sha.as_str()) {
        return Ok(None);
    }
    let changed_files = crate::review::changed_files_local(repo, &range, cancel)?;
    if changed_files != plan.changed_files {
        return Ok(None);
    }
    let diff_text = crate::review::diff_local(repo, &range, cancel)?;
    Ok(Some(crate::review::scope::CoverageSnapshot {
        run_id: candidate.run_id,
        scope: crate::review::scope::ReviewScope {
            head_sha: candidate.head_sha.clone(),
            base_ref: candidate.base_ref.clone(),
            base_sha: candidate.base_sha.clone(),
        },
        merge_base_sha,
        changed_files,
        diff_hash: diff_hash(&diff_text),
        diff_text,
    }))
}

fn backfill_historical(repo: &Path, db: &crate::db::Db, repo_id: i64, cancel: &AtomicBool) -> Result<(), String> {
    let db = db.open()?;
    for candidate in db.historical_review_coverage_candidates(repo_id)? {
        if db.review_coverage_exists(candidate.run_id)? {
            continue;
        }
        if cancel.load(Ordering::Relaxed) {
            return Err("cancelled".into());
        }
        let Some(source) =
            db.historical_review_coverage_for_head(repo_id, candidate.pr_id, &candidate.head_sha, false)?
        else {
            continue;
        };
        if source.run_id != candidate.run_id {
            continue;
        }
        let Ok(Some(coverage)) = recover_full_coverage(repo, &db, repo_id, &source, cancel) else {
            continue;
        };
        let diff = coverage.diff_text;
        let changed_files = coverage.changed_files;
        let merge_base_sha = coverage.merge_base_sha;
        let Ok(patch_ids) = current_patch_ids(repo, &candidate.base_sha, &candidate.head_sha, &diff, cancel) else {
            continue;
        };
        let source_json = serde_json::json!({ "backfilled": true, "sourceRunId": candidate.run_id }).to_string();
        db.begin_review_coverage(&crate::db::ReviewCoverageInput {
            repo_id,
            pr_id: candidate.pr_id,
            run_id: candidate.run_id,
            base_ref: &candidate.base_ref,
            base_sha: &candidate.base_sha,
            merge_base_sha: &merge_base_sha,
            head_sha: &candidate.head_sha,
            changed_files: &changed_files,
            covered_patch_ids: &patch_ids,
            diff_hash: &diff_hash(&diff),
            diff_text: &diff,
            source_json: Some(&source_json),
        })?;
        db.finalize_review_coverage(candidate.run_id)?;
    }
    Ok(())
}

pub(crate) fn reuse_stacked_scope(
    repo: &Path,
    db: &crate::db::Database,
    repo_id: i64,
    pr_id: i64,
    previous: Option<&crate::review::scope::CoverageSnapshot>,
    current: &crate::review::scope::ReviewScope,
    changed: &mut Vec<String>,
    selection: &mut crate::review::scope::ScopeSelection,
    cancel: &AtomicBool,
) -> Result<Vec<i64>, String> {
    if selection.requested_full || changed.is_empty() {
        return Ok(Vec::new());
    }
    let mut sources = Vec::new();
    for (run_id, source_pr, head) in db.stacked_review_coverage_sources(repo_id, pr_id)? {
        if cancel.load(Ordering::Relaxed) {
            return Err("cancelled".into());
        }
        let Ok(Some(source)) = snapshot_for_reuse(repo, db, repo_id, source_pr, &head, cancel) else {
            continue;
        };
        if source.run_id == run_id
            && !sources
                .iter()
                .any(|saved: &crate::review::scope::CoverageSnapshot| saved.run_id == run_id)
            && source.changed_files.iter().any(|path| changed.contains(path))
        {
            sources.push(source);
        }
    }
    if cancel.load(Ordering::Relaxed) {
        return Err("cancelled".into());
    }
    crate::review::scope::reuse_stacked_files(repo, previous, current, &sources, changed, selection, cancel)
}

pub(crate) fn resolve(
    repo: &Path,
    db: &crate::db::Db,
    repo_id: i64,
    base: &str,
    head: &str,
    full_diff: &str,
    prior_head: Option<&str>,
    cancel: &AtomicBool,
) -> Result<Option<CoverageResolution>, String> {
    backfill_historical(repo, db, repo_id, cancel)?;
    let known = db.open()?.review_coverage_patch_ids(repo_id)?;
    let Some(known) = known else {
        return Ok(Some(CoverageResolution {
            review_base: None,
            fully_covered: false,
            usable: false,
        }));
    };
    if known.is_empty() {
        return Ok(None);
    }
    let patches = commit_patches(repo, &format!("{base}..{head}"), cancel)?;
    if patches.is_empty() {
        return Ok(None);
    }
    let all_covered = patches.iter().all(|patch| known.contains(&patch.patch_id));
    if all_covered {
        return Ok(Some(CoverageResolution {
            review_base: Some(prior_head.unwrap_or(base).to_string()),
            fully_covered: true,
            usable: true,
        }));
    }
    let first_uncovered = patches.iter().position(|patch| !known.contains(&patch.patch_id));
    let Some(first_uncovered) = first_uncovered else {
        return Ok(None);
    };
    let suffix_uncovered = patches[first_uncovered..]
        .iter()
        .all(|patch| !known.contains(&patch.patch_id));
    if suffix_uncovered {
        if let Some(parent) = patches[first_uncovered].parent.as_deref() {
            return Ok(Some(CoverageResolution {
                review_base: Some(parent.to_string()),
                fully_covered: false,
                usable: true,
            }));
        }
    }
    let _ = full_diff;
    Ok(None)
}

pub(crate) fn diff_hash(diff: &str) -> String {
    format!("{:x}", Sha256::digest(diff.as_bytes()))
}

pub(crate) fn source_json(resolution: Option<&CoverageResolution>) -> Option<String> {
    resolution.filter(|value| value.usable).map(|value| {
        serde_json::json!({
            "reviewBase": value.review_base,
            "fullyCovered": value.fully_covered,
        })
        .to_string()
    })
}

/// What a review that deferred some files covers: the patch id of the diff it reviewed, and no
/// per-commit ids, which would mark every commit, deferred files included, as reviewed. The next
/// review then finds its commits uncovered and reviews from the base again.
pub(crate) fn partial_ids(repo: &Path, reviewed_diff: &str, cancel: &AtomicBool) -> Result<Vec<String>, String> {
    Ok(patch_id(repo, reviewed_diff, cancel)?.into_iter().collect())
}

pub(crate) fn covered_ids(
    repo: &Path,
    base: &str,
    head: &str,
    full_diff: &str,
    cancel: &AtomicBool,
) -> Result<Vec<String>, String> {
    current_patch_ids(repo, base, head, full_diff, cancel)
}

#[cfg(test)]
#[path = "coverage_tests.rs"]
mod tests;
