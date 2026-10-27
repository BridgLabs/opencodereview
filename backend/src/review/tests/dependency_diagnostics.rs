use std::{collections::BTreeSet, fs, path::Path, sync::atomic::AtomicBool};

fn git(repo: &Path, args: &[&str]) -> String {
    String::from_utf8(crate::indexer::git(repo, args, None).unwrap())
        .unwrap()
        .trim()
        .into()
}

fn commit(repo: &Path) -> String {
    git(repo, &["add", "."]);
    git(
        repo,
        &[
            "-c",
            "user.name=Fixture",
            "-c",
            "user.email=fixture@example.test",
            "commit",
            "-m",
            "fixture",
        ],
    );
    git(repo, &["rev-parse", "HEAD"])
}

#[test]
fn offline_export_restores_submodule_only_and_mixed_archives() {
    for with_file_link in [false, true] {
        let root = super::super::make_run_dir(-63001).unwrap();
        let common = root.join("repos/acme/common");
        let api = root.join("repos/acme/api");
        let repo = root.join("repos/acme/app");
        for (path, name) in [(&common, "acme/common"), (&api, "acme/api"), (&repo, "acme/app")] {
            fs::create_dir_all(path).unwrap();
            git(path, &["init", "--initial-branch=main"]);
            git(
                path,
                &["remote", "add", "origin", &format!("git@github.com:{name}.git")],
            );
        }
        fs::write(common.join("guard.rs"), "pub const GUARD: bool = true;\n").unwrap();
        let common_sha = commit(&common);
        fs::write(api.join("schema.sql"), "SELECT 1;\n").unwrap();
        let api_sha = commit(&api);
        let doc = if with_file_link {
            format!("[schema](https://github.com/acme/api/blob/{api_sha}/schema.sql)\n")
        } else {
            "# Native submodule without file links\n".into()
        };
        fs::write(repo.join("README.md"), doc).unwrap();
        fs::write(
            repo.join(".gitmodules"),
            "[submodule \"lib/common\"]\n path = lib/common\n url = git@github.com:acme/common.git\n",
        )
        .unwrap();
        commit(&repo);
        git(
            &repo,
            &[
                "update-index",
                "--add",
                "--cacheinfo",
                "160000",
                &common_sha,
                "lib/common",
            ],
        );
        git(
            &repo,
            &[
                "-c",
                "user.name=Fixture",
                "-c",
                "user.email=fixture@example.test",
                "commit",
                "-m",
                "submodule",
            ],
        );
        let head = git(&repo, &["rev-parse", "HEAD"]);
        let cancel = AtomicBool::new(false);
        let primary = || {
            crate::review_archive::capture(
                &repo,
                crate::review_archive::Scope {
                    host: "github.com".into(),
                    repository: "acme/app".into(),
                    head_sha: head.clone(),
                    base_sha: head.clone(),
                    merge_base_sha: head.clone(),
                    prior_sha: None,
                },
                &cancel,
            )
            .unwrap()
        };
        let changed = ["README.md".into()];
        let prepared = BTreeSet::from(["acme/common".into(), "acme/api".into()]);
        let archive =
            crate::review::dependencies::capture(primary(), &repo, &changed, "", &prepared, false, &cancel).unwrap();
        assert_eq!(archive.dependencies().len(), if with_file_link { 2 } else { 1 });
        assert!(archive
            .dependencies()
            .iter()
            .find(|dependency| dependency.repository == "acme/common")
            .unwrap()
            .provenance
            .is_empty());
        let mut store = crate::review_archive::ArchiveStore::open(
            &root.join("review-archives"),
            &crate::db::Db::for_test_at(root.join("review-archives")),
        )
        .unwrap();
        store.deposit_and_link(&archive, "review:42").unwrap();
        drop(store);

        let recorded = super::load_recorded_dependencies(
            &root.join("review-archives"),
            &crate::db::Db::for_test_at(root.join("review-archives")),
            42,
        )
        .unwrap()
        .unwrap();
        let expected = if with_file_link {
            prepared
        } else {
            BTreeSet::from(["acme/common".into()])
        };
        assert_eq!(recorded.repositories, expected);
        assert_eq!(recorded.provenance.len(), usize::from(with_file_link));
        if with_file_link {
            assert_eq!(recorded.provenance[0].repository, "acme/api");
            assert_eq!(recorded.provenance[0].commit, api_sha);
        }
        let reconstructed = crate::review::dependencies::capture_with_recorded(
            primary(),
            &repo,
            &changed,
            "",
            &recorded.repositories,
            false,
            Some(&recorded.provenance),
            &cancel,
        )
        .unwrap();
        assert_eq!(reconstructed.descriptor(), archive.descriptor());
        assert_eq!(
            reconstructed
                .resolve_dependency("acme/common", &common_sha, "guard.rs")
                .unwrap(),
            "pub const GUARD: bool = true;\n"
        );
        if with_file_link {
            assert_eq!(
                reconstructed
                    .resolve_dependency("acme/api", &api_sha, "schema.sql")
                    .unwrap(),
                "SELECT 1;\n"
            );
        }
        fs::remove_dir_all(root).unwrap();
    }
}
