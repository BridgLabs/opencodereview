use super::*;

#[test]
fn reprocess_scope_repeats_only_the_previous_paths_still_in_the_pr() {
    let current = vec!["docs/programs.md".into(), "src/other.ts".into()];
    assert_eq!(
        reprocess_files(&["docs/programs.md".into()], &current).unwrap(),
        vec!["docs/programs.md"]
    );
    assert_eq!(
        reprocess_files(&["removed.ts".into(), "docs/programs.md".into()], &current).unwrap(),
        vec!["docs/programs.md"]
    );
    for previous in [
        vec![],
        vec!["removed.ts".into()],
        vec!["../unsafe".into()],
        vec!["docs/programs.md".into(), "docs/programs.md".into()],
    ] {
        assert!(reprocess_files(&previous, &current).is_err());
    }
}

#[test]
fn reprocess_selection_applies_current_policy_before_planning() {
    let current = vec![
        "src/code.ts".into(),
        ".windsurf/rules.md".into(),
        "pnpm-lock.yaml".into(),
        "src/reused.ts".into(),
    ];
    let (assigned, selection) = reprocess_selection(&current[..3], &current, 1522).unwrap();
    assert_eq!(assigned, ["src/code.ts"]);
    assert_eq!(selection.reused_files, ["src/reused.ts"]);
    assert_eq!(selection.excluded_files.len(), 2);
    assert_eq!(selection.full_pr_files, current);
    assert_eq!(selection.source_run_id, Some(1522));
    assert_eq!(selection.mode, "reprocess");
    assert_eq!(
        reprocess_selection(&current[1..3], &current, 1522).unwrap_err(),
        "previous analysis has no reviewable paths under the current policy"
    );
}

#[test]
fn stacked_scopes_distinguish_child_commits_parent_updates_rewrites_and_retargeting() {
    let root = std::env::temp_dir().join(format!(
        "ocr-scope-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&root).unwrap();
    let git = |args: &[&str]| {
        let output = std::process::Command::new("git")
            .arg("-C")
            .arg(&root)
            .args(["-c", "user.name=Fixture", "-c", "user.email=fixture@example.test"])
            .args(args)
            .output()
            .unwrap();
        assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
        String::from_utf8_lossy(&output.stdout).trim().to_string()
    };
    git(&["init", "-q", "-b", "main"]);
    std::fs::write(root.join("base.txt"), "base\n").unwrap();
    git(&["add", "."]);
    git(&["commit", "-qm", "base"]);
    let main = git(&["rev-parse", "HEAD"]);
    git(&["checkout", "-qb", "parent"]);
    std::fs::write(root.join("parent.txt"), "parent\n").unwrap();
    git(&["add", "."]);
    git(&["commit", "-qm", "parent"]);
    let parent = git(&["rev-parse", "HEAD"]);
    git(&["checkout", "-qb", "child"]);
    std::fs::write(root.join("child.txt"), "child\n").unwrap();
    git(&["add", "."]);
    git(&["commit", "-qm", "child"]);
    let previous = ReviewScope {
        head_sha: git(&["rev-parse", "HEAD"]),
        base_ref: "parent".into(),
        base_sha: parent.clone(),
    };
    assert_eq!(git(&["diff", "--name-only", &format!("{parent}...HEAD")]), "child.txt");
    std::fs::write(root.join("child.txt"), "child update\n").unwrap();
    git(&["add", "."]);
    git(&["commit", "-qm", "child update"]);
    let child = ReviewScope {
        head_sha: git(&["rev-parse", "HEAD"]),
        ..previous.clone()
    };
    let cancel = AtomicBool::new(false);
    assert!(can_review_incrementally(&root, &previous, &child, &cancel));
    let retargeted = ReviewScope {
        base_ref: "main".into(),
        base_sha: main,
        ..child.clone()
    };
    assert!(!can_review_incrementally(&root, &child, &retargeted, &cancel));
    assert!(!equivalent(&root, &child, &retargeted, &cancel));
    git(&["checkout", "-q", "parent"]);
    std::fs::write(root.join("parent.txt"), "parent update\n").unwrap();
    git(&["add", "."]);
    git(&["commit", "-qm", "parent update"]);
    let advanced = ReviewScope {
        base_sha: git(&["rev-parse", "HEAD"]),
        ..child.clone()
    };
    assert!(equivalent(&root, &child, &advanced, &cancel));
    git(&["checkout", "-q", "child"]);
    git(&["rebase", "parent"]);
    let restacked = ReviewScope {
        head_sha: git(&["rev-parse", "HEAD"]),
        ..advanced
    };
    assert!(!can_review_incrementally(&root, &child, &restacked, &cancel));
    assert_eq!(git(&["diff", "--name-only", "parent...child"]), "child.txt");
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn base_branch_sync_merge_requires_full_scope_for_overlapping_files() {
    let root = std::env::temp_dir().join(format!(
        "ocr-scope-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&root).unwrap();
    let git = |args: &[&str]| {
        let output = std::process::Command::new("git")
            .arg("-C")
            .arg(&root)
            .args(["-c", "user.name=Fixture", "-c", "user.email=fixture@example.test"])
            .args(args)
            .output()
            .unwrap();
        assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
        String::from_utf8_lossy(&output.stdout).trim().to_string()
    };
    git(&["init", "-q", "-b", "main"]);
    let shared = (0..20).map(|index| format!("line {index}\n")).collect::<String>();
    std::fs::write(root.join("base.txt"), &shared).unwrap();
    git(&["add", "."]);
    git(&["commit", "-qm", "base"]);
    let base = git(&["rev-parse", "HEAD"]);
    git(&["checkout", "-qb", "feature"]);
    std::fs::write(root.join("base.txt"), shared.replace("line 0\n", "feature change\n")).unwrap();
    std::fs::write(root.join("feature.txt"), "feature\n").unwrap();
    git(&["add", "."]);
    git(&["commit", "-qm", "feature"]);
    let previous_head = git(&["rev-parse", "HEAD"]);
    git(&["checkout", "-q", "main"]);
    std::fs::write(root.join("base.txt"), shared.replace("line 19\n", "base-only change\n")).unwrap();
    std::fs::write(root.join("base-update.txt"), "updated\n").unwrap();
    git(&["add", "."]);
    git(&["commit", "-qm", "base update"]);
    let current_base = git(&["rev-parse", "HEAD"]);
    git(&["checkout", "-q", "feature"]);
    git(&["merge", "--no-ff", "-q", "-m", "sync main", "main"]);
    let current_head = git(&["rev-parse", "HEAD"]);
    let previous = ReviewScope {
        head_sha: previous_head,
        base_ref: "main".into(),
        base_sha: base,
    };
    let current = ReviewScope {
        head_sha: current_head,
        base_ref: "main".into(),
        base_sha: current_base,
    };
    let incremental = git(&[
        "diff",
        &format!("{}..{}", previous.head_sha, current.head_sha),
        "--",
        "base.txt",
    ]);
    let full = git(&[
        "diff",
        &format!("{}...{}", current.base_sha, current.head_sha),
        "--",
        "base.txt",
    ]);
    assert!(incremental.contains("+base-only change"));
    assert!(full.contains("+feature change") && !full.contains("base-only change"));
    assert!(!can_review_incrementally(
        &root,
        &previous,
        &current,
        &AtomicBool::new(false)
    ));
    std::fs::remove_dir_all(root).unwrap();
}

fn sync_git(root: &Path, args: &[&str]) -> String {
    let output = std::process::Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["-c", "user.name=Fixture", "-c", "user.email=fixture@example.test"])
        .args(args)
        .output()
        .unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    String::from_utf8(output.stdout).unwrap().trim().to_string()
}

struct SyncFixture(std::path::PathBuf, CoverageSnapshot, ReviewScope);

impl Drop for SyncFixture {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).unwrap();
    }
}

fn sync_fixture(overlap: bool, keep_ours: bool) -> SyncFixture {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let root = std::env::temp_dir().join(format!(
        "ocr-sync-{}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos(),
        NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&root).unwrap();
    let git = |args: &[&str]| sync_git(&root, args);
    git(&["init", "-q", "-b", "main"]);
    std::fs::write(root.join("programs.md"), "base\n").unwrap();
    std::fs::write(root.join("deletable.rs"), "retained source line\n".repeat(20)).unwrap();
    git(&["add", "."]);
    git(&["commit", "-qm", "base"]);
    let original_base = git(&["rev-parse", "HEAD"]);
    git(&["checkout", "-qb", "feature"]);
    std::fs::write(root.join("programs.md"), "base\nfeature\n").unwrap();
    for index in 0..19 {
        std::fs::write(root.join(format!("feature-{index}.rs")), "feature\n").unwrap();
    }
    git(&["add", "."]);
    git(&["commit", "-qm", "reviewed feature"]);
    let previous_head = git(&["rev-parse", "HEAD"]);
    git(&["checkout", "-q", "main"]);
    if overlap {
        std::fs::write(root.join("programs.md"), "base\nmain\n").unwrap();
    }
    std::fs::write(root.join("inherited.rs"), "main\n").unwrap();
    git(&["add", "."]);
    git(&["commit", "-qm", "base update"]);
    let base = git(&["rev-parse", "HEAD"]);
    let cancel = AtomicBool::new(false);
    let range = format!("{base}...{previous_head}");
    let diff_text = crate::review::diff_local(&root, &range, &cancel).unwrap();
    let coverage = CoverageSnapshot {
        run_id: 1487,
        scope: ReviewScope {
            head_sha: previous_head,
            base_ref: "main".into(),
            base_sha: base.clone(),
        },
        merge_base_sha: original_base,
        changed_files: crate::review::changed_files_local(&root, &range, &cancel).unwrap(),
        diff_hash: crate::review::coverage::diff_hash(&diff_text),
        diff_text,
    };
    git(&["checkout", "-q", "feature"]);
    let merge = std::process::Command::new("git")
        .arg("-C")
        .arg(&root)
        .args([
            "-c",
            "user.name=Fixture",
            "-c",
            "user.email=fixture@example.test",
            "merge",
            "--no-ff",
            "--no-commit",
            "main",
        ])
        .output()
        .unwrap();
    assert_eq!(merge.status.success(), !overlap);
    if overlap {
        std::fs::write(
            root.join("programs.md"),
            if keep_ours {
                "base\nfeature\n"
            } else {
                "base\nfeature\nmain\n"
            },
        )
        .unwrap();
    }
    git(&["add", "."]);
    git(&["commit", "-qm", "resolve base synchronization"]);
    let current = ReviewScope {
        head_sha: git(&["rev-parse", "HEAD"]),
        base_ref: "main".into(),
        base_sha: base,
    };
    SyncFixture(root, coverage, current)
}

#[test]
fn coverage_diff_uses_full_object_ids_even_with_abbreviation_requested() {
    let SyncFixture(ref root, ref coverage, _) = sync_fixture(false, false);
    let cancel = AtomicBool::new(false);
    let range = format!("{}...{}", coverage.scope.base_sha, coverage.scope.head_sha);
    let expected = format!(
        "{}\n",
        sync_git(
            root,
            &["diff", "--no-ext-diff", "--no-textconv", "--full-index", &range]
        )
    );
    assert_eq!(crate::review::diff_local(root, &range, &cancel).unwrap(), expected);
    assert_eq!(
        crate::review::diff_local_paths(root, &range, &coverage.changed_files, &cancel),
        expected
    );
    for abbreviation in ["--abbrev=8", "--abbrev=9"] {
        let actual = crate::review::git_capture(root, &["diff", abbreviation, &range], &cancel, 30).unwrap();
        assert_eq!(String::from_utf8(actual).unwrap(), expected);
    }
}

#[test]
fn coverage_reuses_legacy_abbreviations_without_forcing_full_review() {
    let SyncFixture(ref root, ref coverage, ref current) = sync_fixture(true, false);
    let cancel = AtomicBool::new(false);
    let range = format!("{}...{}", coverage.scope.base_sha, coverage.scope.head_sha);
    for abbreviation in ["--abbrev=8", "--abbrev=9", "--abbrev=12"] {
        let mut legacy = coverage.clone();
        legacy.diff_text = format!(
            "{}\n",
            sync_git(root, &["diff", "--no-ext-diff", "--no-textconv", abbreviation, &range])
        );
        legacy.diff_hash = crate::review::coverage::diff_hash(&legacy.diff_text);
        let original = legacy.diff_text.clone();
        let previous = Some(&legacy.scope);
        let head = Some(legacy.scope.head_sha.as_str());
        assert!(
            prepare_scope_reuse(root, previous, head, &legacy.scope, Some(&legacy), false, &cancel)
                .unwrap()
                .unwrap()
                .changed_files
                .is_empty()
        );
        let selection = prepare_scope_reuse(root, previous, head, current, Some(&legacy), false, &cancel)
            .unwrap()
            .unwrap();
        assert_eq!(selection.changed_files, ["programs.md"]);
        assert_eq!(selection.selection.reused_files.len(), 19);
        assert!(!selection.selection.requested_full);
        assert_eq!(legacy.diff_text, original);
        assert_eq!(legacy.diff_hash, crate::review::coverage::diff_hash(&original));
    }
}

#[test]
fn coverage_rejects_changed_legacy_content_headers_and_hashes() {
    let SyncFixture(ref root, ref coverage, _) = sync_fixture(true, false);
    let cancel = AtomicBool::new(false);
    let range = format!("{}...{}", coverage.scope.base_sha, coverage.scope.head_sha);
    let original = format!(
        "{}\n",
        sync_git(root, &["diff", "--no-ext-diff", "--no-textconv", "--abbrev=8", &range])
    );
    let first_index = original.lines().find(|line| line.starts_with("index ")).unwrap();
    for text in [
        original.replace("+feature\n", "+different\n"),
        original.replace("100644", "100755"),
        original.replace("feature-0.rs", "another.rs"),
        original.replacen("index 00000000..", "index 11111111..", 1),
        original.replacen("index 00000000..", "index 0..", 1),
        original.replacen("index 00000000..", "index invalid..", 1),
        original.replacen(first_index, "index 00000000..00000000", 1),
        original.replacen(&format!("{first_index}\n"), "", 1),
        format!("{original}\n"),
        original.trim_end_matches('\n').to_string(),
    ] {
        assert_ne!(text, original);
        let invalid = CoverageSnapshot {
            diff_hash: crate::review::coverage::diff_hash(&text),
            diff_text: text,
            ..coverage.clone()
        };
        assert!(verify_coverage(root, &invalid, &cancel).is_err());
    }
    let invalid = CoverageSnapshot {
        diff_text: original,
        diff_hash: "invalid".into(),
        ..coverage.clone()
    };
    assert!(verify_coverage(root, &invalid, &cancel).is_err());
}

#[test]
fn coverage_legacy_index_matching_preserves_modes_hunks_and_line_boundaries() {
    let before = "a".repeat(40);
    let after = "b".repeat(40);
    let actual = format!("index {before}..{after} 100644\n");
    let legacy = "index aaaaaaaa..bbbbbbbb 100644\n";
    assert!(coverage_diff_matches(legacy, &actual));
    assert!(coverage_diff_matches("", ""));
    for saved in [
        "index aaaaaaaa..cccccccc 100644\n",
        "index cccccccc..bbbbbbbb 100644\n",
        "index aaaaaaaa..bbbbbbbb 100755\n",
        "index aaaaaaaa..bbbbbbbb\n",
        "index aaaaaaaa..bbbbbbbb 100644",
        "index aaaaaaaa..bbbbbbbb 100644\r\n",
        "index aaaaaaaa..bbbbbbbb 100644\nextra\n",
        "",
    ] {
        assert!(!coverage_diff_matches(saved, &actual));
    }
    for prefix in ["+", "-", " "] {
        assert!(!coverage_diff_matches(
            &format!("{prefix}{legacy}"),
            &format!("{prefix}{actual}")
        ));
    }
    assert!(!coverage_diff_matches(legacy, "index aaaaaaaaa..bbbbbbbbb 100644\n"));
    assert!(coverage_diff_matches(
        "index 00000000..bbbbbbbb\n",
        &format!("index {}..{after}\n", "0".repeat(40))
    ));
    assert!(coverage_diff_matches(
        "index aaaaaaaa..00000000\n",
        &format!("index {before}..{}\n", "0".repeat(40))
    ));
    assert!(coverage_diff_matches(
        legacy,
        &format!("index {}..{} 100644\n", "a".repeat(64), "b".repeat(64))
    ));
}

#[test]
#[ignore = "Requires exported coverage on stdin and read-only Git objects; no model or database calls"]
fn persisted_coverage_scope_reuse_without_inference() {
    use std::io::Read;
    let mut raw = String::new();
    std::io::stdin().take(8_388_609).read_to_string(&mut raw).unwrap();
    assert!(raw.len() <= 8_388_608);
    let packet: serde_json::Value = serde_json::from_str(&raw).unwrap();
    let coverage: CoverageSnapshot = serde_json::from_value(packet["coverage"].clone()).unwrap();
    let current: ReviewScope = serde_json::from_value(packet["scope"].clone()).unwrap();
    let expected: Vec<String> = serde_json::from_value(packet["expectedChangedFiles"].clone()).unwrap();
    let repo = std::path::PathBuf::from(std::env::var("OCR_DIAGNOSTIC_REPO").unwrap());
    let cancel = AtomicBool::new(false);
    let selected = prepare_scope_reuse(
        &repo,
        Some(&coverage.scope),
        Some(&coverage.scope.head_sha),
        &current,
        Some(&coverage),
        false,
        &cancel,
    )
    .unwrap();
    let full_range = format!("{}...{}", current.base_sha, current.head_sha);
    let full = crate::review::changed_files_local(&repo, &full_range, &cancel).unwrap();
    let (changed, reused, mode) = if let Some(selected) = selected {
        assert!(!selected.selection.requested_full);
        (
            selected.changed_files,
            selected.selection.reused_files,
            selected.selection.mode,
        )
    } else {
        assert!(can_review_incrementally(&repo, &coverage.scope, &current, &cancel));
        let (changed, _) = crate::review::incremental_review_changes(
            &repo,
            &coverage.scope.head_sha,
            &current.head_sha,
            &full,
            &cancel,
        )
        .unwrap();
        let reused = full
            .iter()
            .filter(|path| !changed.contains(path))
            .cloned()
            .collect::<Vec<_>>();
        (changed, reused, "incremental".into())
    };
    assert_eq!(changed, expected);
    assert_eq!(mode, packet["expectedMode"].as_str().unwrap());
    assert_eq!(reused.len(), packet["expectedReusedCount"].as_u64().unwrap() as usize);
    println!(
        "VERIFIED_COVERAGE_SCOPE {}",
        serde_json::json!({"sourceRun":coverage.run_id,"head":current.head_sha,"mode":mode,"changedFiles":changed,"reusedFiles":reused.len(),"fullPrFiles":full.len(),"modelCalls":0,"databaseWrites":false})
    );
}

#[test]
fn rewritten_scope_reuses_content_between_interleaved_new_changes() {
    let SyncFixture(ref root, ref coverage, _) = sync_fixture(false, false);
    sync_git(root, &["checkout", "-qb", "rewritten", &coverage.scope.base_sha]);
    sync_git(root, &["cherry-pick", &coverage.scope.head_sha]);
    std::fs::write(root.join("programs.md"), "base\nfeature\nnew behavior\n").unwrap();
    sync_git(root, &["commit", "-qam", "new change"]);
    let current = ReviewScope {
        head_sha: sync_git(root, &["rev-parse", "HEAD"]),
        ..coverage.scope.clone()
    };
    let cancel = AtomicBool::new(false);
    assert!(!crate::review::is_ancestor(
        root,
        &coverage.scope.head_sha,
        &current.head_sha,
        &cancel
    ));
    let selected = prepare_scope_reuse(
        root,
        Some(&coverage.scope),
        Some(&coverage.scope.head_sha),
        &current,
        Some(coverage),
        false,
        &cancel,
    )
    .unwrap()
    .expect("Rewritten history must use content proof, not full fallback");
    assert_eq!(selected.selection.mode, "content_reuse");
    assert_eq!(selected.changed_files, ["programs.md"]);
    assert_eq!(selected.selection.reused_files.len(), 19);
    assert!(selected.diff.contains("+new behavior"));
    assert!(!selected.diff.contains("inherited.rs"));
}

#[test]
fn unchanged_scope_reuses_content_without_patch_id_resolution() {
    let SyncFixture(ref root, ref coverage, _) = sync_fixture(false, false);
    let selected = prepare_scope_reuse(
        root,
        Some(&coverage.scope),
        Some(&coverage.scope.head_sha),
        &coverage.scope,
        Some(coverage),
        false,
        &AtomicBool::new(false),
    )
    .unwrap()
    .expect("Same-head coverage must not depend on commit suffixes");
    assert!(selected.changed_files.is_empty());
    assert_eq!(selected.selection.reused_files.len(), 20);
}

#[test]
fn excluded_only_delta_keeps_inventory_without_claiming_new_coverage() {
    let mut changed = vec!["pnpm-lock.yaml".into()];
    let mut selection = ScopeSelection {
        mode: "base_sync".into(),
        requested_full: false,
        source_run_id: Some(1278),
        full_pr_files: vec![
            "pnpm-lock.yaml".into(),
            "src/code.ts".into(),
            ".windsurf/rules.md".into(),
        ],
        reused_files: vec!["src/code.ts".into(), ".windsurf/rules.md".into()],
        excluded_files: Vec::new(),
    };
    exclude_policy_paths(&mut changed, &mut selection).unwrap();
    assert!(changed.is_empty());
    assert_eq!(selection.reused_files, ["src/code.ts"]);
    assert_eq!(selection.full_pr_files.len(), 3);
    assert_eq!(
        selection
            .excluded_files
            .iter()
            .map(|file| file.path.as_str())
            .collect::<Vec<_>>(),
        ["pnpm-lock.yaml", ".windsurf/rules.md"]
    );
    assert_eq!(
        selection.excluded_files[0].reason,
        crate::review_archive::Exclusion::Generated
    );
    changed = vec!["pnpm-lock.yaml".into(), "src/code.ts".into()];
    exclude_policy_paths(&mut changed, &mut selection).unwrap();
    assert_eq!(changed, ["src/code.ts"]);
    assert_eq!(selection.excluded_files.len(), 2);
}

#[test]
fn base_sync_reuses_nineteen_files_and_reviews_only_the_conflict() {
    let SyncFixture(ref root, ref coverage, ref current) = sync_fixture(true, false);
    let selection = select_base_sync(root, coverage, current, &AtomicBool::new(false))
        .unwrap()
        .unwrap();
    assert_eq!(selection.changed_files, ["programs.md"]);
    assert_eq!(selection.selection.reused_files.len(), 19);
    assert_eq!(selection.selection.full_pr_files.len(), 20);
    assert_eq!(selection.selection.source_run_id, Some(1487));
    assert!(selection.diff.contains("+feature"));
    assert!(!selection.diff.contains("+main") && !selection.diff.contains("inherited.rs"));
    let full = crate::review::diff_local(
        root,
        &format!("{}...{}", current.base_sha, current.head_sha),
        &AtomicBool::new(false),
    )
    .unwrap();
    assert!(crate::review::incremental_diff_is_publishable(&selection.diff, &full));
}

#[test]
fn base_sync_rechecks_overlap_even_when_resolution_keeps_old_head_bytes() {
    let SyncFixture(ref root, ref coverage, ref current) = sync_fixture(true, true);
    let selection = select_base_sync(root, coverage, current, &AtomicBool::new(false))
        .unwrap()
        .unwrap();
    assert_eq!(selection.changed_files, ["programs.md"]);
    assert!(selection.diff.contains("-main"));
    assert_eq!(selection.selection.reused_files.len(), 19);
}

#[test]
fn base_sync_requires_completed_matching_coverage_unless_full_is_explicit() {
    let SyncFixture(ref root, ref coverage, ref current) = sync_fixture(true, false);
    let cancel = AtomicBool::new(false);
    let previous = Some(&coverage.scope);
    let head = Some(coverage.scope.head_sha.as_str());
    assert!(prepare_scope_reuse(root, previous, head, current, None, false, &cancel).is_err());
    assert!(prepare_scope_reuse(root, previous, head, current, None, true, &cancel)
        .unwrap()
        .is_none());
    assert!(prepare_scope_reuse(root, None, None, current, None, false, &cancel)
        .unwrap()
        .is_none());
    assert!(prepare_scope_reuse(
        root,
        previous,
        Some("different"),
        current,
        Some(coverage),
        false,
        &cancel
    )
    .is_err());
    let selected = prepare_scope_reuse(root, previous, head, current, Some(coverage), false, &cancel)
        .unwrap()
        .unwrap();
    assert_eq!(selected.changed_files, ["programs.md"]);
    assert!(
        prepare_scope_reuse(root, previous, head, current, Some(coverage), true, &cancel)
            .unwrap()
            .is_none()
    );
}

#[test]
fn base_sync_clean_same_file_update_has_no_new_review_work() {
    let SyncFixture(ref root, _, _) = sync_fixture(false, false);
    sync_git(root, &["checkout", "-qb", "same-file-base", "main"]);
    let original = (0..40).map(|index| format!("line {index}\n")).collect::<String>();
    std::fs::write(root.join("README.md"), &original).unwrap();
    sync_git(root, &["add", "README.md"]);
    sync_git(root, &["commit", "-qm", "shared file"]);
    let base = sync_git(root, &["rev-parse", "HEAD"]);
    sync_git(root, &["checkout", "-qb", "same-file-feature"]);
    std::fs::write(
        root.join("README.md"),
        original.replace("line 35\n", "reviewed feature\n"),
    )
    .unwrap();
    sync_git(root, &["commit", "-qam", "reviewed feature"]);
    let head = sync_git(root, &["rev-parse", "HEAD"]);
    let cancel = AtomicBool::new(false);
    let range = format!("{base}...{head}");
    let diff_text = crate::review::diff_local(root, &range, &cancel).unwrap();
    let coverage = CoverageSnapshot {
        run_id: 1,
        scope: ReviewScope {
            head_sha: head.clone(),
            base_ref: "same-file-base".into(),
            base_sha: base.clone(),
        },
        merge_base_sha: base,
        changed_files: vec!["README.md".into()],
        diff_hash: crate::review::coverage::diff_hash(&diff_text),
        diff_text,
    };
    sync_git(root, &["checkout", "-q", "same-file-base"]);
    std::fs::write(
        root.join("README.md"),
        original.replace("line 0\n", "base image update\nextra base line\n"),
    )
    .unwrap();
    sync_git(root, &["commit", "-qam", "base only"]);
    let base_sha = sync_git(root, &["rev-parse", "HEAD"]);
    for rebased in [false, true] {
        sync_git(
            root,
            &["checkout", "-qb", if rebased { "rebased" } else { "merged" }, &head],
        );
        if rebased {
            sync_git(root, &["rebase", "same-file-base"]);
        } else {
            sync_git(root, &["merge", "--no-ff", "-qm", "sync base", "same-file-base"]);
        }
        let current = ReviewScope {
            head_sha: sync_git(root, &["rev-parse", "HEAD"]),
            base_ref: "same-file-base".into(),
            base_sha: base_sha.clone(),
        };
        let selected = prepare_scope_reuse(
            root,
            Some(&coverage.scope),
            Some(&head),
            &current,
            Some(&coverage),
            false,
            &cancel,
        )
        .unwrap()
        .unwrap();
        assert!(
            selected.changed_files.is_empty(),
            "Base-only changes must not reassign README.md: {selected:?}"
        );
        assert!(selected.diff.is_empty());
        assert_eq!(selected.selection.reused_files, ["README.md"]);
        let content = std::fs::read_to_string(root.join("README.md"))
            .unwrap()
            .replace("line 20\n", "new PR behavior\n");
        std::fs::write(root.join("README.md"), content).unwrap();
        sync_git(root, &["commit", "-qam", "new author change"]);
        let changed = ReviewScope {
            head_sha: sync_git(root, &["rev-parse", "HEAD"]),
            ..current
        };
        let selected = prepare_scope_reuse(
            root,
            Some(&coverage.scope),
            Some(&head),
            &changed,
            Some(&coverage),
            false,
            &cancel,
        )
        .unwrap()
        .unwrap();
        assert_eq!(selected.changed_files, ["README.md"]);
        assert!(selected.diff.contains("+new PR behavior"));
    }
}

#[test]
fn base_sync_without_pr_overlap_has_no_new_review_work() {
    let SyncFixture(ref root, ref coverage, ref current) = sync_fixture(false, false);
    let selection = select_base_sync(root, coverage, current, &AtomicBool::new(false))
        .unwrap()
        .unwrap();
    assert!(selection.changed_files.is_empty() && selection.diff.is_empty());
    assert_eq!(selection.selection.reused_files.len(), 20);
}

#[test]
fn base_sync_rejects_incomplete_tampered_missing_and_retargeted_coverage() {
    let SyncFixture(ref root, ref coverage, ref current) = sync_fixture(true, false);
    let cancel = AtomicBool::new(false);
    for mutation in 0..4 {
        let mut invalid = coverage.clone();
        match mutation {
            0 => {
                invalid.changed_files.pop();
            }
            1 => invalid.diff_text.push_str("tampered"),
            2 => invalid.diff_hash = "unverified".into(),
            _ => invalid.scope.base_sha = "0".repeat(40),
        }
        assert!(select_base_sync(root, &invalid, current, &cancel).is_err());
    }
    let retargeted = ReviewScope {
        base_ref: "other".into(),
        ..current.clone()
    };
    assert!(select_base_sync(root, coverage, &retargeted, &cancel).is_err());
    assert!(select_base_sync(root, coverage, current, &AtomicBool::new(true)).is_err());
}

#[test]
fn base_sync_keeps_both_sides_of_a_previously_reviewed_rename() {
    let fixture = sync_fixture(true, false);
    let SyncFixture(ref root, _, ref current) = fixture;
    sync_git(root, &["mv", "deletable.rs", "renamed.rs"]);
    sync_git(root, &["commit", "-qam", "reviewed rename"]);
    let previous_head = sync_git(root, &["rev-parse", "HEAD"]);
    let cancel = AtomicBool::new(false);
    let range = format!("{}...{previous_head}", current.base_sha);
    let diff_text = crate::review::diff_local(root, &range, &cancel).unwrap();
    let coverage = CoverageSnapshot {
        run_id: 1,
        scope: ReviewScope {
            head_sha: previous_head,
            ..current.clone()
        },
        merge_base_sha: current.base_sha.clone(),
        changed_files: crate::review::changed_files_local(root, &range, &cancel).unwrap(),
        diff_hash: crate::review::coverage::diff_hash(&diff_text),
        diff_text,
    };
    sync_git(root, &["checkout", "-q", "main"]);
    std::fs::write(root.join("inherited.rs"), "main update\n").unwrap();
    sync_git(root, &["commit", "-qam", "another base update"]);
    let base_sha = sync_git(root, &["rev-parse", "HEAD"]);
    sync_git(root, &["checkout", "-q", "feature"]);
    sync_git(root, &["merge", "--no-ff", "-qm", "sync again", "main"]);
    std::fs::write(
        root.join("renamed.rs"),
        format!("new line\n{}", "retained source line\n".repeat(19)),
    )
    .unwrap();
    sync_git(root, &["commit", "-qam", "edit renamed file"]);
    let current = ReviewScope {
        head_sha: sync_git(root, &["rev-parse", "HEAD"]),
        base_sha,
        base_ref: "main".into(),
    };
    let selected = select_base_sync(root, &coverage, &current, &cancel).unwrap().unwrap();
    assert_eq!(selected.changed_files, ["deletable.rs", "renamed.rs"]);
    assert!(selected.diff.contains("rename from deletable.rs") && selected.diff.contains("rename to renamed.rs"));
    let full =
        crate::review::diff_local(root, &format!("{}...{}", current.base_sha, current.head_sha), &cancel).unwrap();
    assert!(crate::review::incremental_diff_is_publishable(&selected.diff, &full));
}

#[test]
fn base_sync_keeps_new_renamed_and_deleted_paths_in_review() {
    let fixture = sync_fixture(true, false);
    let SyncFixture(ref root, ref coverage, ref current) = fixture;
    sync_git(root, &["mv", "feature-0.rs", "renamed feature.rs"]);
    std::fs::remove_file(root.join("deletable.rs")).unwrap();
    std::fs::write(root.join("new.rs"), "new\n").unwrap();
    std::fs::write(root.join("ação.rs"), "new\n").unwrap();
    sync_git(root, &["add", "."]);
    sync_git(root, &["commit", "-qm", "new changes"]);
    let current = ReviewScope {
        head_sha: sync_git(root, &["rev-parse", "HEAD"]),
        ..current.clone()
    };
    let selection = select_base_sync(root, coverage, &current, &AtomicBool::new(false))
        .unwrap()
        .unwrap();
    for path in ["programs.md", "renamed feature.rs", "new.rs", "deletable.rs", "ação.rs"] {
        assert!(selection.changed_files.contains(&path.to_string()), "missing {path}");
    }
    assert_eq!(selection.selection.reused_files.len(), 18);
}
