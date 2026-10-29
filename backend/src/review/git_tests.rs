use super::*;
use std::process::Command;

fn run(dir: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .env("GIT_NO_LAZY_FETCH", "1")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).trim().to_string()
}

#[test]
fn diff_blob_ids_skip_null_ids_and_paths() {
    let (old, new) = ("a".repeat(40), "b".repeat(40));
    let raw = format!(
        ":100644 100644 {old} {new} M\0src/a.rs\0:000000 100644 {} {new} A\0src/b.rs\0",
        "0".repeat(40)
    );
    assert_eq!(diff_blob_ids(raw.as_bytes()), vec![old, new]);
}

#[test]
fn blobless_clones_fetch_every_diff_blob_before_diffing() {
    let root = std::env::temp_dir().join(format!(
        "ocr-prefetch-{}-{}",
        std::process::id(),
        crate::db::now_millis()
    ));
    let (origin, clone) = (root.join("origin"), root.join("clone"));
    fs::create_dir_all(&origin).unwrap();
    run(&origin, &["init", "-q", "-b", "main"]);
    run(&origin, &["config", "uploadpack.allowFilter", "true"]);
    run(&origin, &["config", "uploadpack.allowAnySHA1InWant", "true"]);
    let commit = |message: &str, suffix: &str| {
        for index in 0..5 {
            fs::write(
                origin.join(format!("file-{index}.txt")),
                format!("line {index}{suffix}\n"),
            )
            .unwrap();
        }
        run(&origin, &["add", "."]);
        run(
            &origin,
            &[
                "-c",
                "user.name=OpenCodeReview",
                "-c",
                "user.email=ocr@example.com",
                "commit",
                "-q",
                "-m",
                message,
            ],
        );
        run(&origin, &["rev-parse", "HEAD"])
    };
    let base = commit("base", "");
    let head = commit("head", " changed");
    let url = format!("file://{}", origin.display());
    let output = Command::new("git")
        .args(["clone", "-q", "--filter=blob:none", "--no-checkout", &url])
        .arg(&clone)
        .output()
        .unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let range = format!("{base}...{head}");
    let ids = diff_blob_ids(
        &git_capture(
            &clone,
            &["diff", "--raw", "--no-abbrev", "--no-renames", "-z", &range],
            &AtomicBool::new(false),
            60,
        )
        .unwrap(),
    );
    assert_eq!(ids.len(), 10);
    let missing = |ids: &[String]| {
        ids.iter()
            .filter(|id| {
                !Command::new("git")
                    .arg("-C")
                    .arg(&clone)
                    .args(["cat-file", "-e", id])
                    .env("GIT_NO_LAZY_FETCH", "1")
                    .status()
                    .unwrap()
                    .success()
            })
            .count()
    };
    assert!(missing(&ids) > 0);
    prefetch_diff_blobs(&clone, &range, &AtomicBool::new(false)).unwrap();
    assert_eq!(missing(&ids), 0);
    assert!(diff_local(&clone, &range, &AtomicBool::new(false))
        .unwrap()
        .contains("+line 4 changed"));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn path_diffs_read_bracketed_names_literally() {
    let repo = std::env::temp_dir().join(format!(
        "ocr-literal-paths-{}-{}",
        std::process::id(),
        crate::db::now_millis()
    ));
    fs::create_dir_all(&repo).unwrap();
    run(&repo, &["init", "-q", "-b", "main"]);
    run(&repo, &["config", "user.email", "test@example.invalid"]);
    run(&repo, &["config", "user.name", "Test"]);
    fs::write(repo.join("[key].ts"), "export const route = 1;\n").unwrap();
    fs::write(repo.join("k.ts"), "glob decoy\n").unwrap();
    run(&repo, &["add", "."]);
    run(&repo, &["commit", "-q", "-m", "base"]);
    let base = run(&repo, &["rev-parse", "HEAD"]);
    fs::write(repo.join("[key].ts"), "export const route = 2;\n").unwrap();
    fs::write(repo.join("k.ts"), "glob decoy changed\n").unwrap();
    run(&repo, &["commit", "-q", "-am", "head"]);
    let diff = diff_local_paths(
        &repo,
        &format!("{base}..HEAD"),
        &["[key].ts".to_string()],
        &AtomicBool::new(false),
    );
    let _ = fs::remove_dir_all(&repo);
    assert!(diff.contains("diff --git a/[key].ts b/[key].ts"), "{diff}");
    assert!(!diff.contains("k.ts b/k.ts"), "{diff}");
}

#[test]
fn blobs_of_deleted_bracketed_paths_are_absent() {
    let repo = std::env::temp_dir().join(format!(
        "ocr-deleted-blob-{}-{}",
        std::process::id(),
        crate::db::now_millis()
    ));
    fs::create_dir_all(&repo).unwrap();
    run(&repo, &["init", "-q", "-b", "main"]);
    run(&repo, &["config", "user.email", "test@example.invalid"]);
    run(&repo, &["config", "user.name", "Test"]);
    fs::write(repo.join("[key].ts"), "export const route = 1;\n").unwrap();
    run(&repo, &["add", "."]);
    run(&repo, &["commit", "-q", "-m", "base"]);
    run(&repo, &["mv", "[key].ts", "[...key].ts"]);
    run(&repo, &["commit", "-q", "-m", "head"]);
    let cancel = AtomicBool::new(false);
    let (moved, kept) = (
        show_blob(&repo, "HEAD", "[key].ts", &cancel),
        show_blob(&repo, "HEAD", "[...key].ts", &cancel),
    );
    let _ = fs::remove_dir_all(&repo);
    assert_eq!(moved, None);
    assert_eq!(kept.as_deref(), Some("export const route = 1;\n"));
}
