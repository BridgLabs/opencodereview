use super::usage_tests::{graph_chunk_fixture, review_inputs};
use super::*;
use serde_json::json;

struct Fixture {
    root: PathBuf,
    context: PathBuf,
    directory: PathBuf,
    inputs: ReviewInputs,
    map: ReviewMap,
    chunk: ReviewChunk,
}

impl Fixture {
    fn new() -> Self {
        static NEXT_ID: std::sync::atomic::AtomicI64 = std::sync::atomic::AtomicI64::new(-10_000);
        let id = NEXT_ID.fetch_sub(2, Ordering::Relaxed);
        let (source, map) = graph_chunk_fixture(id, 20_000, 20_000);
        let root = make_run_dir(id - 1).unwrap();
        let context = root.join("context");
        fs::rename(source, &context).unwrap();
        let directory = root.join("chunks/001");
        let target = directory.join("context");
        fs::create_dir_all(target.join("changed")).unwrap();
        fs::copy(context.join("changed/a.rs"), target.join("changed/a.rs")).unwrap();
        fs::write(context.join("PR.md"), "# PR\n\nAuthor intent and deferrals\n").unwrap();
        fs::copy(context.join("PR.md"), target.join("PR.md")).unwrap();
        copy_tree(&context.join("skill"), &target.join("skill"));
        fs::write(target.join("PR_DIFF.patch"), "").unwrap();
        fs::write(target.join("CONTEXT_RETRIEVAL.md"), recovery::NOTE).unwrap();
        let chunk = ReviewChunk {
            id: 1,
            files: vec!["a.rs".into()],
            related_files: vec![],
            global_context_files: vec![],
            omitted_convention_files: vec![],
            risk_triggers: vec![],
            cross_chunk_group_ids: vec![],
        };
        write_review_map(&target, &chunk_review_map(&map, &chunk)).unwrap();
        let inputs = review_inputs("deep", 10, 5);
        fs::write(
            directory.join("prompt.md"),
            render_chunk_prompt(&inputs, &target, &directory.join("summary.md"), "head", None, &chunk),
        )
        .unwrap();
        Self {
            root,
            context,
            directory,
            inputs,
            map,
            chunk,
        }
    }

    fn run(
        &self,
        cancel: &AtomicBool,
        launch: &mut dyn FnMut(&Path, &ReviewChunk) -> Result<(), String>,
    ) -> Result<(PathBuf, ReviewChunk), String> {
        recovery::run(
            &self.inputs,
            &self.context,
            &self.map,
            &self.chunk,
            &self.directory,
            "head",
            None,
            cancel,
            launch,
        )
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn request(directory: &Path, paths: serde_json::Value) {
    fs::write(directory.join("report.json"), json!({"reviewedFiles":["a.rs"],"missingEvidence":["Implementation required"],"requestedFiles":paths,"findings":[]}).to_string()).unwrap();
}

fn clean(directory: &Path) {
    fs::write(
        directory.join("report.json"),
        json!({"reviewedFiles":["a.rs"],"missingEvidence":[],"requestedFiles":[],"findings":[]}).to_string(),
    )
    .unwrap();
}

#[test]
fn retrieves_frozen_evidence_once_without_mutating_original_snapshots() {
    let fixture = Fixture::new();
    let full_hash = context_fingerprint(&fixture.context).unwrap();
    let initial_hash = context_fingerprint(&fixture.directory.join("context")).unwrap();
    let mut calls = 0;
    let (directory, chunk) = fixture
        .run(&AtomicBool::new(false), &mut |directory, chunk| {
            calls += 1;
            assert_eq!(chunk.files, ["a.rs"]);
            if calls == 1 {
                request(directory, json!(["c.rs"]));
            } else {
                assert_eq!(
                    fs::read(directory.join("context/global/c.rs")).unwrap(),
                    fs::read(fixture.context.join("changed/c.rs")).unwrap()
                );
                assert_eq!(
                    fs::read(directory.join("context/changed/a.rs")).unwrap(),
                    fs::read(fixture.context.join("changed/a.rs")).unwrap()
                );
                assert!(!directory.join("context/CONTEXT_RETRIEVAL.md").exists());
                assert!(
                    dir_bytes(&directory.join("context")) + file_bytes(&directory.join("prompt.md"))
                        <= crate::review::policy::TARGET_CONTEXT_BYTES as i64
                );
                clean(directory);
            }
            Ok(())
        })
        .unwrap();
    assert_eq!(calls, 2);
    assert_eq!(chunk.global_context_files, ["c.rs"]);
    assert_eq!(context_fingerprint(&fixture.context).unwrap(), full_hash);
    assert_eq!(
        context_fingerprint(&fixture.directory.join("context")).unwrap(),
        initial_hash
    );
    crate::review::evidence::materialize(
        &directory.join("report.json"),
        &directory.join("context"),
        &chunk.files,
        "head",
        "",
        &directory.join("summary.md"),
        &directory.join("comments.json"),
    )
    .unwrap();
    assert!(crate::review::evidence::materialize(
        &fixture.directory.join("report.json"),
        &fixture.directory.join("context"),
        &chunk.files,
        "head",
        "",
        &fixture.directory.join("summary.md"),
        &fixture.directory.join("comments.json")
    )
    .is_err());
}

#[test]
fn required_recovery_evidence_can_exceed_the_selection_target() {
    let fixture = Fixture::new();
    fs::write(fixture.context.join("changed/c.rs"), "x".repeat(240_000)).unwrap();
    let mut calls = 0;
    let (directory, chunk) = fixture
        .run(&AtomicBool::new(false), &mut |directory, _| {
            calls += 1;
            if calls == 1 {
                request(directory, json!(["c.rs"]));
            } else {
                assert_eq!(file_bytes(&directory.join("context/global/c.rs")), 240_000);
                clean(directory);
            }
            Ok(())
        })
        .unwrap();
    assert_eq!(calls, 2);
    assert_eq!(chunk.files, fixture.chunk.files);
    assert!(dir_bytes(&directory.join("context")) > crate::review::policy::TARGET_CONTEXT_BYTES as i64);
    crate::review::evidence::materialize(
        &directory.join("report.json"),
        &directory.join("context"),
        &chunk.files,
        "head",
        "",
        &directory.join("summary.md"),
        &directory.join("comments.json"),
    )
    .unwrap();
}

#[test]
fn retrieval_retains_existing_evidence_above_the_selection_target() {
    let mut fixture = Fixture::new();
    for path in ["aa.rs", "ab.rs"] {
        fs::create_dir_all(fixture.context.join("related")).unwrap();
        fs::write(fixture.context.join("related").join(path), "x".repeat(50_000)).unwrap();
        fixture.map.related_files.push(ContextFile {
            path: path.into(),
            package_path: None,
            reason: "same package".into(),
            source: "head".into(),
        });
        fixture.chunk.related_files.push(path.into());
        copy_dir_subset(
            &fixture.context.join("related"),
            &fixture.directory.join("context/related"),
            &[path.into()],
        );
    }
    let mut calls = 0;
    let (_, chunk) = fixture
        .run(&AtomicBool::new(false), &mut |directory, _| {
            calls += 1;
            if calls == 1 {
                request(directory, json!(["c.rs"]));
            } else {
                clean(directory);
            }
            Ok(())
        })
        .unwrap();
    assert_eq!(chunk.global_context_files, ["c.rs"]);
    assert_eq!(chunk.related_files.len(), 2);
}

#[test]
fn rejects_unsafe_unknown_duplicate_or_already_available_requests_without_retry() {
    for paths in [
        json!(["../outside"]),
        json!(["/outside"]),
        json!(["a//b"]),
        json!([".git/config"]),
        json!(["x\\y"]),
        json!(["unknown.rs"]),
        json!(["a.rs"]),
        json!(["c.rs", "c.rs"]),
        json!(["b.rs", "c.rs", "d.rs", "e.rs", "f.rs"]),
    ] {
        let fixture = Fixture::new();
        let mut calls = 0;
        assert!(fixture
            .run(&AtomicBool::new(false), &mut |directory, _| {
                calls += 1;
                request(directory, paths.clone());
                Ok(())
            })
            .is_err());
        assert_eq!(calls, 1);
    }
}

#[test]
fn refuses_missing_oversized_or_symlinked_evidence_before_retry() {
    for mode in ["missing", "oversized", "symlink", "parent-symlink"] {
        let mut fixture = Fixture::new();
        let mut path = "c.rs";
        match mode {
            "missing" => fs::remove_file(fixture.context.join("changed/c.rs")).unwrap(),
            "oversized" => fs::write(
                fixture.context.join("changed/c.rs"),
                "x".repeat(crate::review::policy::MAX_EVIDENCE_BYTES + 1),
            )
            .unwrap(),
            "symlink" => {
                fs::remove_file(fixture.context.join("changed/c.rs")).unwrap();
                std::os::unix::fs::symlink(
                    fixture.context.join("changed/b.rs"),
                    fixture.context.join("changed/c.rs"),
                )
                .unwrap();
            }
            _ => {
                path = "redirect/c.rs";
                std::os::unix::fs::symlink(
                    fixture.context.join("changed"),
                    fixture.context.join("changed/redirect"),
                )
                .unwrap();
                fixture.map.reviewable_files.push(path.into());
            }
        }
        let mut calls = 0;
        assert!(fixture
            .run(&AtomicBool::new(false), &mut |directory, _| {
                calls += 1;
                request(directory, json!([path]));
                Ok(())
            })
            .is_err());
        assert_eq!(calls, usize::from(mode == "missing"));
    }
}

#[test]
fn stops_on_cancellation_process_failure_or_evidence_tampering() {
    for mode in [
        "cancel-before",
        "cancel-after",
        "process",
        "chunk-tamper",
        "source-tamper",
    ] {
        let fixture = Fixture::new();
        let cancel = AtomicBool::new(mode == "cancel-before");
        let mut calls = 0;
        let error = fixture
            .run(&cancel, &mut |directory, _| {
                calls += 1;
                request(directory, json!(["c.rs"]));
                match mode {
                    "cancel-after" => cancel.store(true, Ordering::Relaxed),
                    "process" => return Err("process failed".into()),
                    "chunk-tamper" => fs::write(directory.join("context/changed/a.rs"), "tampered").unwrap(),
                    "source-tamper" => fs::write(fixture.context.join("changed/c.rs"), "tampered").unwrap(),
                    _ => {}
                }
                Ok(())
            })
            .unwrap_err();
        assert_eq!(calls, usize::from(mode != "cancel-before"));
        if mode.starts_with("cancel") {
            assert_eq!(error, "cancelled");
        }
    }
}

#[test]
fn no_third_attempt_and_missing_evidence_still_blocks_success() {
    for repeat in [false, true] {
        let fixture = Fixture::new();
        let mut calls = 0;
        let result = fixture.run(&AtomicBool::new(false), &mut |directory, _| {
            calls += 1;
            if calls == 1 || repeat {
                request(directory, json!(["c.rs"]));
            } else {
                fs::write(
                    directory.join("report.json"),
                    json!({"reviewedFiles":["a.rs"],"missingEvidence":["Still incomplete"],"findings":[]}).to_string(),
                )
                .unwrap();
            }
            Ok(())
        });
        assert_eq!(calls, 2);
        if repeat {
            assert!(result.unwrap_err().contains("limit"));
        } else {
            let (directory, chunk) = result.unwrap();
            assert!(crate::review::evidence::materialize(
                &directory.join("report.json"),
                &directory.join("context"),
                &chunk.files,
                "head",
                "",
                &directory.join("summary.md"),
                &directory.join("comments.json")
            )
            .is_err());
        }
    }
}

#[test]
fn recovery_never_overwrites_an_existing_output_directory() {
    let fixture = Fixture::new();
    let destination = fixture.directory.join("retrieval-1");
    fs::create_dir(&destination).unwrap();
    fs::write(destination.join("sentinel"), "preserve").unwrap();
    let mut calls = 0;
    let error = fixture
        .run(&AtomicBool::new(false), &mut |directory, _| {
            calls += 1;
            request(directory, json!(["c.rs"]));
            Ok(())
        })
        .unwrap_err();
    assert!(error.contains("already exists"));
    assert_eq!(calls, 1);
    assert_eq!(fs::read_to_string(destination.join("sentinel")).unwrap(), "preserve");
}

#[test]
fn failed_recovery_process_never_starts_another_attempt() {
    let fixture = Fixture::new();
    let mut calls = 0;
    let error = fixture
        .run(&AtomicBool::new(false), &mut |directory, _| {
            calls += 1;
            if calls == 2 {
                return Err("cancelled".into());
            }
            request(directory, json!(["c.rs"]));
            Ok(())
        })
        .unwrap_err();
    assert_eq!(error, "cancelled");
    assert_eq!(calls, 2);
}

#[test]
fn malformed_output_or_existing_candidates_do_not_trigger_repair() {
    let fixture = Fixture::new();
    for raw in ["invalid".to_string(), json!({"reviewedFiles":["a.rs"],"missingEvidence":["Missing"],"requestedFiles":["c.rs"],"findings":[],"contextQuestions":["Unresolved candidate"]}).to_string()] {
        let mut calls = 0;
        assert!(fixture.run(&AtomicBool::new(false), &mut |directory, _| { calls += 1; fs::write(directory.join("report.json"), &raw).unwrap(); Ok(()) }).is_err());
        assert_eq!(calls, 1);
    }
}

#[test]
fn records_each_attempt_and_retains_cost_when_recovery_fails_or_is_cancelled() {
    for status in ["done", "failed", "cancelled"] {
        let mut fixture = Fixture::new();
        let db_path = fixture.root.join("test.sqlite");
        let db = Database::open_at(db_path.clone()).unwrap();
        db.ensure_temp_repo_for_index(1, "owner/repo").unwrap();
        rusqlite::Connection::open(&db_path).unwrap().execute("INSERT INTO pull_requests (id, repo_id, number, title, author, head_sha, last_synced_at, created_at, updated_at) VALUES (1, 1, 1, 'PR', 'a', 'head', 1, 1, 1)", []).unwrap();
        fixture.inputs.run_id = db.start_review_skeleton_with(1, None).unwrap();
        fixture.inputs.db_path = db_path.to_string_lossy().into();
        let transcript = fixture.root.join("transcript.json");
        fs::write(&transcript, json!({"type":"result","subtype":"success","modelUsage":{"fixture-model":{"inputTokens":10,"outputTokens":5}}}).to_string()).unwrap();
        let usage = parse_transcript_usage(&transcript);
        let metrics = StageMetrics {
            prompt_bytes: 100,
            diff_bytes: 10,
            context_bytes: 200,
            file_count: 1,
            related_file_count: 1,
        };
        let attempts = vec![
            recovery::Attempt {
                usage: usage.clone(),
                metrics,
                started_at: 10,
                finished_at: 20,
            },
            recovery::Attempt {
                usage,
                metrics,
                started_at: 30,
                finished_at: 40,
            },
        ];
        let total = recovery::record_attempts(
            &fixture.inputs,
            &fixture.chunk,
            &attempts,
            status,
            "initial-cache-key",
            false,
        );
        assert_eq!(total.input_tokens, 20);
        db.sync_review_stage_usage(fixture.inputs.run_id).unwrap();
        let stages = db.review_stage_breakdown(fixture.inputs.run_id).unwrap().stages;
        assert_eq!(stages.len(), 2);
        assert_eq!(stages[0].status, "retried");
        assert_eq!(stages[1].status, status);
        assert_eq!(stages[0].input_tokens + stages[1].input_tokens, 20);
        assert!(stages.iter().all(|stage| !stage.cache_hit));
        assert!(stages[1].cache_key.is_none());
        assert_eq!(db.latest_review_run(1).unwrap().unwrap().input_tokens, 20);
    }
}

#[test]
fn recovery_progress_counts_only_started_processes() {
    let mut chunk = crate::review::progress::ChunkProgress::queued(1, 1, 1);
    let raw = sharded_phase_json("chunks", "running");
    for (state, at) in [("running", 2), ("validating", 3), ("queued", 4)] {
        chunk.observe(
            &crate::review::progress::execution_phase(&raw, "chunks", state, at),
            state,
            at,
        );
    }
    assert_eq!(chunk.attempts, 1);
    assert_eq!(chunk.status, "queued");
    chunk.observe(
        &crate::review::progress::execution_phase(&raw, "chunks", "running", 5),
        "Recovery started",
        5,
    );
    assert_eq!(chunk.attempts, 2);
    assert_eq!(chunk.started_at, Some(2));
    assert_eq!(chunk.execution_finished_at, None);
    chunk.finish(false, Some("cancelled"), 6);
    assert_eq!(chunk.status, "cancelled");
}

#[test]
#[ignore]
fn saved_snapshot_recovers_omitted_dependency_without_inference() {
    let context = PathBuf::from(std::env::var("OCR_DIAGNOSTIC_CONTEXT").unwrap());
    let original = PathBuf::from(std::env::var("OCR_DIAGNOSTIC_CHUNK").unwrap());
    let dependency = std::env::var("OCR_DIAGNOSTIC_DEPENDENCY").unwrap();
    let map: ReviewMap = serde_json::from_str(&fs::read_to_string(context.join("review_map.json")).unwrap()).unwrap();
    let chunk: ReviewChunk = serde_json::from_str(&fs::read_to_string(original.join("chunk.json")).unwrap()).unwrap();
    let root = make_run_dir(-997).unwrap();
    let directory = root.join("chunk");
    copy_tree(&original.join("context"), &directory.join("context"));
    fs::write(directory.join("context/CONTEXT_RETRIEVAL.md"), recovery::NOTE).unwrap();
    let mut calls = 0;
    let inputs = review_inputs("deep", 10, 5);
    let (recovered, effective) = recovery::run(&inputs, &context, &map, &chunk, &directory, &map.head_sha, None, &AtomicBool::new(false), &mut |directory, _| {
        calls += 1;
        let report = if calls == 1 { json!({"reviewedFiles":chunk.files,"missingEvidence":["Implementation required"],"requestedFiles":[dependency],"findings":[]}) } else { json!({"reviewedFiles":chunk.files,"missingEvidence":[],"findings":[]}) };
        fs::write(directory.join("report.json"), report.to_string()).unwrap();
        Ok(())
    }).unwrap();
    assert_eq!(calls, 2);
    assert_eq!(effective.files, chunk.files);
    assert_eq!(
        fs::read(recovered.join("context/global").join(&dependency)).unwrap(),
        fs::read(context.join("changed").join(&dependency)).unwrap()
    );
    assert_eq!(
        fs::read(recovered.join("context/PR_DIFF.patch")).unwrap(),
        fs::read(original.join("context/PR_DIFF.patch")).unwrap()
    );
    let bytes = dir_bytes(&recovered.join("context")) + file_bytes(&recovered.join("prompt.md"));
    validate_review_stage_inputs(&recovered.join("context"), None, &recovered.join("prompt.md")).unwrap();
    println!(
        "SAVED_RECOVERY_CHECK {}",
        json!({"chunk":chunk.id,"preparedBytes":bytes,"dependencyPresent":true,"modelCalls":0})
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
#[ignore]
fn saved_snapshot_recovers_multiple_large_dependencies_without_inference() {
    let context = PathBuf::from(std::env::var("OCR_DIAGNOSTIC_CONTEXT").unwrap());
    let original = PathBuf::from(std::env::var("OCR_DIAGNOSTIC_CHUNK").unwrap());
    let map: ReviewMap = serde_json::from_str(&fs::read_to_string(context.join("review_map.json")).unwrap()).unwrap();
    let chunk: ReviewChunk = serde_json::from_str(&fs::read_to_string(original.join("chunk.json")).unwrap()).unwrap();
    let candidates = map
        .reviewable_files
        .iter()
        .map(|path| (path.clone(), "changed"))
        .chain(map.related_files.iter().map(|file| (file.path.clone(), "related")))
        .filter(|(path, _)| {
            !chunk.files.contains(path)
                && !chunk.global_context_files.contains(path)
                && !chunk.related_files.contains(path)
        })
        .filter(|(path, area)| context.join(area).join(path).is_file())
        .collect::<BTreeMap<_, _>>();
    let mut candidates = candidates.into_iter().collect::<Vec<_>>();
    candidates.sort_by_key(|(path, area)| std::cmp::Reverse(file_bytes(&context.join(area).join(path))));
    candidates.truncate(4);
    assert_eq!(candidates.len(), 4);
    let requested = candidates.iter().map(|(path, _)| path.clone()).collect::<Vec<_>>();
    let root = make_run_dir(-1003).unwrap();
    let directory = root.join("chunk");
    copy_tree(&original.join("context"), &directory.join("context"));
    fs::write(directory.join("context/CONTEXT_RETRIEVAL.md"), recovery::NOTE).unwrap();
    let mut calls = 0;
    let (recovered, effective) = recovery::run(&review_inputs("deep", 10, 5), &context, &map, &chunk, &directory, &map.head_sha, None, &AtomicBool::new(false), &mut |directory, _| {
        calls += 1;
        let report = if calls == 1 { json!({"reviewedFiles":chunk.files,"missingEvidence":["Required supporting files"],"requestedFiles":requested,"findings":[]}) } else { json!({"reviewedFiles":chunk.files,"missingEvidence":[],"findings":[]}) };
        fs::write(directory.join("report.json"), report.to_string()).unwrap();
        Ok(())
    }).unwrap();
    assert_eq!(calls, 2);
    assert_eq!(effective.files, chunk.files);
    for (path, area) in &candidates {
        let target = if *area == "changed" { "global" } else { "related" };
        assert_eq!(
            fs::read(recovered.join("context").join(target).join(path)).unwrap(),
            fs::read(context.join(area).join(path)).unwrap()
        );
    }
    assert_eq!(
        fs::read(recovered.join("context/PR_DIFF.patch")).unwrap(),
        fs::read(original.join("context/PR_DIFF.patch")).unwrap()
    );
    let bytes = dir_bytes(&recovered.join("context")) + file_bytes(&recovered.join("prompt.md"));
    assert!(bytes > crate::review::policy::TARGET_CONTEXT_BYTES as i64);
    validate_review_stage_inputs(&recovered.join("context"), None, &recovered.join("prompt.md")).unwrap();
    println!(
        "SAVED_LARGE_RECOVERY_CHECK {}",
        json!({"chunk":chunk.id,"preparedBytes":bytes,"requestedFiles":requested.len(),"modelCalls":0})
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn successful_initial_report_uses_one_call_and_unmarked_context_never_retries() {
    let fixture = Fixture::new();
    let mut calls = 0;
    let (directory, _) = fixture
        .run(&AtomicBool::new(false), &mut |directory, _| {
            calls += 1;
            clean(directory);
            Ok(())
        })
        .unwrap();
    assert_eq!(directory, fixture.directory);
    assert_eq!(calls, 1);
    fs::remove_file(fixture.directory.join("context/CONTEXT_RETRIEVAL.md")).unwrap();
    assert!(fixture
        .run(&AtomicBool::new(false), &mut |directory, _| {
            calls += 1;
            request(directory, json!(["c.rs"]));
            Ok(())
        })
        .unwrap_err()
        .contains("unavailable"));
    assert_eq!(calls, 2);
}
