use super::usage_tests::review_inputs;

fn convention_fixture(root: &std::path::Path) -> (std::path::PathBuf, String, String) {
    let repo = root.join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    let git = |args: &[&str]| {
        let output = std::process::Command::new("git")
            .arg("-C")
            .arg(&repo)
            .args(["-c", "user.name=Fixture", "-c", "user.email=fixture@example.test"])
            .args(args)
            .output()
            .unwrap();
        assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
        String::from_utf8_lossy(&output.stdout).trim().to_string()
    };
    git(&["init", "-q", "-b", "main"]);
    std::fs::write(repo.join("REVIEW.md"), "base rules\n").unwrap();
    std::fs::write(repo.join("README.md"), "readme\n").unwrap();
    std::fs::write(repo.join("entry.ts"), "export const value = 1;\n").unwrap();
    git(&["add", "."]);
    git(&["commit", "-qm", "base"]);
    let base = git(&["rev-parse", "HEAD"]);
    std::fs::remove_file(repo.join("README.md")).unwrap();
    std::fs::write(repo.join("AGENTS.md"), "agents\n").unwrap();
    std::fs::write(repo.join("REVIEW.md"), "head rules\n").unwrap();
    git(&["add", "."]);
    git(&["commit", "-qm", "head"]);
    let head = git(&["rev-parse", "HEAD"]);
    (repo, base, head)
}

#[test]
fn convention_files_are_selected_at_the_pinned_base() {
    let root = super::make_run_dir(-783).unwrap();
    let (repo, base, head) = convention_fixture(&root);
    let cancel = std::sync::atomic::AtomicBool::new(false);
    assert_eq!(
        super::convention_files_at(&repo, &base, &cancel).unwrap(),
        ["REVIEW.md", "README.md"]
    );
    assert_eq!(
        super::convention_files_at(&repo, &head, &cancel).unwrap(),
        ["REVIEW.md", "AGENTS.md"]
    );
    let mut inputs = review_inputs("auto", 20, 5);
    inputs.repo_path = repo.clone();
    inputs.base_ref = base.clone();
    let (plan, _) =
        super::build_snapshot_context_plan(&inputs, &["entry.ts".into()], &head, &base, &cancel, &mut |_, _| {})
            .unwrap();
    assert_eq!(plan.convention_files, ["REVIEW.md", "README.md"]);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn review_guidelines_reach_the_context_with_base_bytes() {
    let root = super::make_run_dir(-784).unwrap();
    let (repo, base, head) = convention_fixture(&root);
    let cancel = std::sync::atomic::AtomicBool::new(false);
    let mut inputs = review_inputs("auto", 20, 5);
    inputs.repo_path = repo.clone();
    inputs.base_ref = base.clone();
    let (plan, _) =
        super::build_snapshot_context_plan(&inputs, &["entry.ts".into()], &head, &base, &cancel, &mut |_, _| {})
            .unwrap();
    let context = root.join("context");
    super::materialize_files(
        &repo,
        &base,
        &head,
        &plan.changed_files,
        &plan,
        &context,
        "skill",
        false,
        &cancel,
    )
    .unwrap();
    assert_eq!(
        std::fs::read_to_string(context.join("conventions/REVIEW.md")).unwrap(),
        "base rules\n"
    );
    assert!(!context.join("conventions/AGENTS.md").exists());
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn missing_convention_documents_are_optional() {
    let root = super::make_run_dir(-785).unwrap();
    let repo = root.join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    let git = |args: &[&str]| {
        let output = std::process::Command::new("git")
            .arg("-C")
            .arg(&repo)
            .args(["-c", "user.name=Fixture", "-c", "user.email=fixture@example.test"])
            .args(args)
            .output()
            .unwrap();
        assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
        String::from_utf8_lossy(&output.stdout).trim().to_string()
    };
    git(&["init", "-q", "-b", "main"]);
    std::fs::write(repo.join("entry.ts"), "export const value = 1;\n").unwrap();
    git(&["add", "."]);
    git(&["commit", "-qm", "base"]);
    let base = git(&["rev-parse", "HEAD"]);
    let cancel = std::sync::atomic::AtomicBool::new(false);
    assert!(super::convention_files_at(&repo, &base, &cancel).unwrap().is_empty());
    let mut inputs = review_inputs("auto", 20, 5);
    inputs.repo_path = repo.clone();
    inputs.base_ref = base.clone();
    let (plan, _) =
        super::build_snapshot_context_plan(&inputs, &["entry.ts".into()], &base, &base, &cancel, &mut |_, _| {})
            .unwrap();
    let context = root.join("context");
    super::materialize_files(
        &repo,
        &base,
        &base,
        &plan.changed_files,
        &plan,
        &context,
        "skill",
        false,
        &cancel,
    )
    .unwrap();
    assert!(!context.join("conventions").exists());
    std::fs::remove_dir_all(root).unwrap();
}
