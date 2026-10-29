use super::*;
use crate::review_archive::{Archive, Scope, SourceEntry};
use serde_json::json;

struct Fixture {
    directory: PathBuf,
    context: PathBuf,
    archive: Archive,
    report: Value,
    diff: String,
}

impl Fixture {
    fn new() -> Self {
        let directory = std::env::temp_dir().join(format!("ocr-output-repair-{}", crate::sandbox::secret().unwrap()));
        let context = directory.join("context");
        fs::create_dir_all(context.join("skill")).unwrap();
        fs::write(context.join("PR.md"), "Fixture intent").unwrap();
        fs::write(context.join("skill/SKILL.md"), "Verify evidence").unwrap();
        let head = "a".repeat(40);
        let content = "call();\n";
        let scope = Scope {
            host: "github.com".into(),
            repository: "fixture/repo".into(),
            head_sha: head.clone(),
            base_sha: "b".repeat(40),
            merge_base_sha: "c".repeat(40),
            prior_sha: None,
        };
        let entry = SourceEntry {
            path: "code.rs".into(),
            revision: head.clone(),
            mode: "100644".into(),
            git_blob_id: "d".repeat(40),
            bytes: content.len() as u64,
        };
        let archive = Archive::build(scope, vec![entry], &AtomicBool::new(false), |_| {
            Ok(content.as_bytes().to_vec())
        })
        .unwrap();
        let diff =
            "diff --git a/code.rs b/code.rs\n--- /dev/null\n+++ b/code.rs\n@@ -0,0 +1 @@\n+call();\n".to_string();
        archive
            .prepare_stage(&context, &["code.rs".into()], &diff, &archive.scope().merge_base_sha)
            .unwrap();
        let reference = json!({"path":"code.rs","revision":head,"line":1,"quote":"call();"});
        let report = json!({"contractVersion":2,"snapshotId":archive.descriptor().id,"reviewedFiles":["code.rs"],"missingEvidence":[],"findings":[{"kind":"finding","title":"Fixture boundary","detail":"Fixture defect detail","action":"Restore the guard","path":"code.rs","line":1,"side":"RIGHT","evidence":[reference],"counterEvidence":[reference]}]});
        fs::write(
            directory.join("prompt.md"),
            format!(
                "Read {} and write {}",
                context.display(),
                directory.join("report.json").display()
            ),
        )
        .unwrap();
        fs::write(directory.join("transcript.json"), usage(40)).unwrap();
        Self {
            directory,
            context,
            archive,
            report,
            diff,
        }
    }

    fn write(&self, report: &Value) {
        fs::write(self.directory.join("report.json"), report.to_string()).unwrap();
    }

    fn validate(&self) -> Result<(), String> {
        crate::review::evidence::EvidenceResolver::frozen(&self.context, &self.archive).materialize(
            &self.directory.join("report.json"),
            &["code.rs".into()],
            &self.archive.scope().head_sha,
            &self.diff,
            &self.directory.join("summary.md"),
            &self.directory.join("comments.json"),
        )
    }

    fn run(&self, cancel: &AtomicBool, launch: impl FnMut(&str) -> Result<(), String>) -> Result<(), String> {
        run(
            &self.directory,
            &self.context,
            &self.directory.join("prompt.md"),
            &self.directory.join("transcript.json"),
            cancel,
            false,
            || self.validate(),
            launch,
        )
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.directory);
    }
}

fn usage(input: i64) -> String {
    json!({"type":"result","subtype":"success","modelUsage":{"fixture-model":{"inputTokens":input,"outputTokens":2}}})
        .to_string()
}

#[test]
fn repairs_invalid_final_anchors_and_chunk_quotes_with_unchanged_frozen_evidence() {
    for kind in [
        "anchor",
        "quote",
        "line",
        "counter-evidence",
        "unknown-field",
        "field-type",
    ] {
        let fixture = Fixture::new();
        let mut invalid = fixture.report.clone();
        if kind == "anchor" {
            invalid["findings"][0]["line"] = json!(99);
        } else if kind == "unknown-field" {
            invalid["findings"][0]["severity"] = json!("high");
        } else if kind == "field-type" {
            invalid["findings"][0]["evidence"][0]["line"] = json!("1");
        } else if kind == "quote" {
            invalid["findings"][0]["evidence"][0]["quote"] = json!("invented();");
        } else if kind == "line" {
            invalid["findings"][0]["evidence"][0]["line"] = json!(50);
        } else {
            invalid["findings"][0]["counterEvidence"] = json!([]);
        }
        fixture.write(&invalid);
        let original_error = fixture.validate().unwrap_err();
        let fingerprint = context_fingerprint(&fixture.context).unwrap();
        let prompt = fs::read_to_string(fixture.directory.join("prompt.md")).unwrap();
        let mut attempts = 0;
        fixture
            .run(&AtomicBool::new(false), |diagnostic| {
                attempts += 1;
                assert!(diagnostic.contains(&original_error));
                assert!(diagnostic.contains("findings[0]"));
                assert!(!diagnostic.contains("invented();") && !diagnostic.contains("Fixture defect detail"));
                let request = crate::sandbox::runner::stage_request(
                    "fixture-model",
                    "v1",
                    1,
                    "claude",
                    &fixture.directory,
                    &fixture.directory,
                    &fixture.context,
                    &fixture.directory.join("prompt.md"),
                )?;
                let crate::sandbox::runner::Request::Run { files, prompt, .. } = request else {
                    panic!("Expected frozen stage");
                };
                fixture.archive.validate_stage_files(&files).unwrap();
                assert_eq!(
                    serde_json::from_str::<Value>(&files["integration/rejected-report.json"]).unwrap(),
                    invalid
                );
                assert!(files["integration/rejected-report.json"].lines().count() > 1);
                assert!(prompt.contains("/output/report.json"));
                fixture.write(&fixture.report);
                fs::write(fixture.directory.join("transcript.json"), usage(20)).unwrap();
                Ok(())
            })
            .unwrap();
        assert_eq!(attempts, 1);
        assert_eq!(context_fingerprint(&fixture.context).unwrap(), fingerprint);
        assert_eq!(fs::read_to_string(fixture.directory.join("prompt.md")).unwrap(), prompt);
        assert_eq!(
            parse_transcript_usage(&fixture.directory.join("transcript.json")).input_tokens,
            60
        );
        fixture
            .run(&AtomicBool::new(false), |_| {
                panic!("Validated output must not run again")
            })
            .unwrap();
    }
}

#[test]
fn repairs_external_quotes_that_do_not_match_the_prepared_context() {
    let fixture = Fixture::new();
    let mut valid = fixture.report.clone();
    valid["findings"][0]["evidence"]
        .as_array_mut()
        .unwrap()
        .push(json!({"path":"PR.md","line":1,"quote":"Fixture intent"}));
    let mut invalid = valid.clone();
    invalid["findings"][0]["evidence"][1]["quote"] = json!("Invented intent");
    fixture.write(&invalid);
    let original_error = fixture.validate().unwrap_err();
    assert_eq!(
        original_error,
        "Incomplete review: quote does not match external evidence at PR.md:1 [findings[0].evidence[1]]"
    );
    let mut attempts = 0;
    fixture
        .run(&AtomicBool::new(false), |diagnostic| {
            attempts += 1;
            assert!(diagnostic.contains(&original_error) && !diagnostic.contains("Invented intent"));
            fixture.write(&valid);
            Ok(())
        })
        .unwrap();
    assert_eq!(attempts, 1);
}

#[test]
fn one_repair_receives_every_mismatched_quote() {
    let fixture = Fixture::new();
    let mut valid = fixture.report.clone();
    valid["findings"][0]["evidence"]
        .as_array_mut()
        .unwrap()
        .push(json!({"path":"PR.md","line":1,"quote":"Fixture intent"}));
    let mut invalid = valid.clone();
    invalid["findings"][0]["evidence"][0]["quote"] = json!("invented();");
    invalid["findings"][0]["evidence"][1]["quote"] = json!("Invented intent");
    fixture.write(&invalid);
    let error = fixture.validate().unwrap_err();
    assert!(
        error.contains("[findings[0].evidence[0]]") && error.contains("PR.md:1 [findings[0].evidence[1]]"),
        "{error}"
    );
    let mut attempts = 0;
    fixture
        .run(&AtomicBool::new(false), |_| {
            attempts += 1;
            fixture.write(&valid);
            Ok(())
        })
        .unwrap();
    assert_eq!(attempts, 1);
}

#[test]
fn repair_cannot_drop_findings_change_coverage_or_replace_evidence_identity() {
    for mutation in [
        "drop",
        "coverage",
        "revision",
        "path",
        "detail",
        "evidence",
        "snapshot",
        "malformed-report",
        "malformed-point",
        "existing-counter",
    ] {
        let fixture = Fixture::new();
        let mut invalid = fixture.report.clone();
        invalid["findings"][0]["line"] = json!(99);
        fixture.write(&invalid);
        let result = fixture.run(&AtomicBool::new(false), |_| {
            let mut repaired = fixture.report.clone();
            match mutation {
                "drop" => repaired["findings"] = json!([]),
                "coverage" => repaired["reviewedFiles"] = json!([]),
                "revision" => repaired["findings"][0]["evidence"][0]["revision"] = json!("b".repeat(40)),
                "path" => repaired["findings"][0]["path"] = json!("other.rs"),
                "detail" => repaired["findings"][0]["detail"] = json!("Changed conclusion"),
                "evidence" => repaired["findings"][0]["evidence"] = json!([]),
                "malformed-report" => repaired = json!([]),
                "malformed-point" => repaired["findings"] = json!([42]),
                "existing-counter" => {
                    repaired["findings"][0]["counterEvidence"] = json!([
                        repaired["findings"][0]["counterEvidence"][0],
                        repaired["findings"][0]["counterEvidence"][0]
                    ])
                }
                _ => repaired["snapshotId"] = json!("changed"),
            }
            fixture.write(&repaired);
            Ok(())
        });
        if matches!(mutation, "coverage" | "revision" | "path" | "detail" | "snapshot") {
            // Anything beyond the mechanical corrections is put back as the model first reported it.
            result.unwrap();
            let kept = read_json(&fixture.directory.join("report.json")).unwrap();
            assert_eq!(
                crate::review::evidence::schema_report_fields(kept),
                crate::review::evidence::schema_report_fields(fixture.report.clone()),
                "{mutation}"
            );
        } else {
            let error = result.unwrap_err();
            assert!(
                error.contains("changed findings, coverage or evidence identities"),
                "{mutation}: {error}"
            );
        }
    }
}

#[test]
fn incomplete_or_stale_reports_never_trigger_output_repair() {
    for field in ["requestedFiles", "snapshotId", "contractVersion", "reviewedFiles"] {
        let fixture = Fixture::new();
        let mut invalid = fixture.report.clone();
        invalid["findings"][0]["line"] = json!(99);
        invalid[field] = match field {
            "snapshotId" => json!("stale"),
            "contractVersion" => json!(1),
            _ => json!(["unavailable"]),
        };
        fixture.write(&invalid);
        assert!(fixture
            .run(&AtomicBool::new(false), |_| panic!(
                "Incomplete reports must not trigger repair"
            ))
            .is_err());
    }
}

#[test]
fn repair_runs_beside_missing_evidence_but_never_changes_it() {
    let missing = json!([{"reason":"evidence_excluded","path":"code.rs"}]);
    for changed in [false, true] {
        let fixture = Fixture::new();
        let mut invalid = fixture.report.clone();
        invalid["missingEvidence"] = missing.clone();
        invalid["findings"][0]["line"] = json!(99);
        fixture.write(&invalid);
        let mut launched = false;
        // Reviews keep missing evidence as warnings, as the product validates them.
        let validate = || {
            crate::review::evidence::EvidenceResolver::frozen(&fixture.context, &fixture.archive)
                .materialize_with_warnings(
                    &fixture.directory.join("report.json"),
                    &["code.rs".into()],
                    &fixture.archive.scope().head_sha,
                    &fixture.diff,
                    &fixture.directory.join("summary.md"),
                    &fixture.directory.join("comments.json"),
                )
                .map(|_| ())
        };
        let error = run(
            &fixture.directory,
            &fixture.context,
            &fixture.directory.join("prompt.md"),
            &fixture.directory.join("transcript.json"),
            &AtomicBool::new(false),
            false,
            validate,
            |_| {
                launched = true;
                let mut repaired = invalid.clone();
                repaired["findings"][0]["line"] = json!(1);
                if changed {
                    repaired["missingEvidence"] = json!([]);
                }
                fixture.write(&repaired);
                Ok(())
            },
        )
        .err()
        .unwrap_or_default();
        assert!(launched, "{error}");
        assert!(error.is_empty(), "{error}");
        assert_eq!(
            read_json(&fixture.directory.join("report.json")).unwrap()["missingEvidence"],
            missing,
            "changed: {changed}"
        );
    }
}

#[test]
fn missing_counter_evidence_stays_incomplete_until_real_quotes_validate() {
    for counter in [
        json!([]),
        json!([{"path":"code.rs","revision":"a".repeat(40),"line":1,"quote":"invented_guard();"}]),
        json!([{"path":"code.rs","revision":"b".repeat(40),"line":1,"quote":"call();"}]),
    ] {
        let fixture = Fixture::new();
        let mut original = fixture.report.clone();
        original["findings"][0]["counterEvidence"] = json!([]);
        fixture.write(&original);
        let mut attempts = 0;
        let error = fixture
            .run(&AtomicBool::new(false), |_| {
                attempts += 1;
                let mut repaired = original.clone();
                repaired["findings"][0]["counterEvidence"] = counter.clone();
                fixture.write(&repaired);
                Ok(())
            })
            .unwrap_err();
        assert_eq!(attempts, 1);
        assert!(error.contains("Repair unsuccessful"));
        assert!(!fixture.directory.join("comments.json").exists());
    }
}

#[test]
fn missing_finding_fields_do_not_trigger_counter_evidence_repair() {
    for field in ["title", "detail", "action", "path", "kind", "evidence"] {
        let fixture = Fixture::new();
        let mut original = fixture.report.clone();
        original["findings"][0]["counterEvidence"] = json!([]);
        original["findings"][0][field] = if field == "evidence" { json!([]) } else { json!("") };
        fixture.write(&original);
        let error = fixture
            .run(&AtomicBool::new(false), |_| {
                panic!("Other incomplete fields must not trigger this repair")
            })
            .unwrap_err();
        assert!(
            error.contains("findings[0]") && error.contains("invalid required fields"),
            "{error}"
        );
    }
}

#[test]
fn repair_preflight_rejects_oversized_inputs_without_starting_another_model() {
    for oversized in ["prompt", "evidence"] {
        let fixture = Fixture::new();
        let mut invalid = fixture.report.clone();
        invalid["findings"][0]["line"] = json!(99);
        fixture.write(&invalid);
        if oversized == "prompt" {
            fs::write(fixture.directory.join("prompt.md"), "x".repeat(200_000)).unwrap();
        } else {
            fs::write(
                fixture.context.join("extra.txt"),
                "x".repeat(crate::review::policy::MAX_EVIDENCE_BYTES),
            )
            .unwrap();
        }
        let mut executions = 0;
        let error = fixture
            .run(&AtomicBool::new(false), |_| {
                crate::sandbox::runner::stage_request(
                    "fixture-model",
                    "v1",
                    1,
                    "claude",
                    &fixture.directory,
                    &fixture.directory,
                    &fixture.context,
                    &fixture.directory.join("prompt.md"),
                )?;
                executions += 1;
                Ok(())
            })
            .unwrap_err();
        assert_eq!(executions, 0);
        assert!(error.contains("budget"));
        assert_eq!(
            parse_transcript_usage(&fixture.directory.join("transcript.json")).input_tokens,
            40
        );
    }
}

#[test]
fn repair_is_bounded_preserves_usage_and_keeps_cancellation_distinct() {
    for failure in ["invalid", "cancelled", "preflight", "mutation"] {
        let fixture = Fixture::new();
        let mut invalid = fixture.report.clone();
        invalid["findings"][0]["line"] = json!(99);
        fixture.write(&invalid);
        let original_prompt = fs::read_to_string(fixture.directory.join("prompt.md")).unwrap();
        let mut attempts = 0;
        let error = fixture
            .run(&AtomicBool::new(false), |_| {
                attempts += 1;
                if failure == "cancelled" {
                    return Err("cancelled".into());
                }
                if failure == "preflight" {
                    return Err("Stage budget exceeded".into());
                }
                fixture.write(if failure == "invalid" {
                    &invalid
                } else {
                    &fixture.report
                });
                if failure == "mutation" {
                    fs::write(fixture.context.join("PR.md"), "Changed").unwrap();
                }
                fs::write(fixture.directory.join("transcript.json"), usage(20)).unwrap();
                Ok(())
            })
            .unwrap_err();
        assert_eq!(attempts, 1);
        assert_eq!(
            fs::read_to_string(fixture.directory.join("prompt.md")).unwrap(),
            original_prompt
        );
        if failure == "cancelled" {
            assert_eq!(error, "cancelled");
        } else {
            assert!(error.contains("Repair unsuccessful"));
        }
        let expected = if matches!(failure, "cancelled" | "preflight") {
            40
        } else {
            60
        };
        assert_eq!(
            parse_transcript_usage(&fixture.directory.join("transcript.json")).input_tokens,
            expected
        );
    }
}

#[test]
fn schema_repair_cannot_change_reported_content() {
    let fixture = Fixture::new();
    let mut invalid = fixture.report.clone();
    invalid["findings"][0]["severity"] = json!("high");
    fixture.write(&invalid);
    let mut changed = fixture.report.clone();
    changed["findings"][0]["title"] = json!("Another conclusion");
    fixture
        .run(&AtomicBool::new(false), |_| {
            fixture.write(&changed);
            Ok(())
        })
        .unwrap();
    assert_eq!(
        read_json(&fixture.directory.join("report.json")).unwrap()["findings"][0]["title"],
        fixture.report["findings"][0]["title"]
    );
    let fixture = Fixture::new();
    let mut missing = fixture.report.clone();
    missing["findings"][0].as_object_mut().unwrap().remove("title");
    fixture.write(&missing);
    let error = fixture
        .run(&AtomicBool::new(false), |_| {
            panic!("A missing conclusion cannot be repaired")
        })
        .unwrap_err();
    assert!(error.contains("report.findings[0].title: missing"), "{error}");
}

#[test]
fn context_questions_written_as_objects_become_their_text() {
    let fixture = Fixture::new();
    let mut report = fixture.report.clone();
    report["contextQuestions"] = json!([{"title":"Confirm the boundary.","detail":"Check the caller."}]);
    fixture.write(&report);
    let error = fixture.validate().err().unwrap_or_default();
    assert!(!error.contains("invalid structured report"), "{error}");
    assert_eq!(
        read_json(&fixture.directory.join("report.json")).unwrap()["contextQuestions"],
        json!(["Confirm the boundary. Check the caller."])
    );
}

#[test]
fn evidence_cited_without_its_directory_is_completed_only_when_unique() {
    for ambiguous in [false, true] {
        let directory = std::env::temp_dir().join(format!("ocr-path-rebind-{}", crate::sandbox::secret().unwrap()));
        let context = directory.join("context");
        fs::create_dir_all(context.join("skill")).unwrap();
        fs::write(context.join("PR.md"), "Fixture intent").unwrap();
        fs::write(context.join("skill/SKILL.md"), "Verify evidence").unwrap();
        let head = "a".repeat(40);
        let content = "call();\n";
        let scope = Scope {
            host: "github.com".into(),
            repository: "fixture/repo".into(),
            head_sha: head.clone(),
            base_sha: "b".repeat(40),
            merge_base_sha: "c".repeat(40),
            prior_sha: None,
        };
        let mut paths = vec!["app/code.rs"];
        if ambiguous {
            paths.push("lib/code.rs");
        }
        let entries = paths
            .iter()
            .map(|path| SourceEntry {
                path: (*path).into(),
                revision: head.clone(),
                mode: "100644".into(),
                git_blob_id: "d".repeat(40),
                bytes: content.len() as u64,
            })
            .collect();
        let archive = Archive::build(scope, entries, &AtomicBool::new(false), |_| {
            Ok(content.as_bytes().to_vec())
        })
        .unwrap();
        let diff =
            "diff --git a/app/code.rs b/app/code.rs\n--- /dev/null\n+++ b/app/code.rs\n@@ -0,0 +1 @@\n+call();\n";
        archive
            .prepare_stage(&context, &["app/code.rs".into()], diff, &archive.scope().merge_base_sha)
            .unwrap();
        let short = json!({"path":"code.rs","revision":head,"line":1,"quote":"call();"});
        let report = json!({"contractVersion":2,"snapshotId":archive.descriptor().id,"reviewedFiles":["app/code.rs"],"missingEvidence":[],"findings":[{"kind":"finding","title":"Fixture boundary","detail":"Fixture defect detail","action":"Restore the guard","path":"app/code.rs","line":1,"side":"RIGHT","evidence":[short],"counterEvidence":[short]}]});
        let report_path = directory.join("report.json");
        fs::write(&report_path, report.to_string()).unwrap();
        let result = crate::review::evidence::EvidenceResolver::frozen(&context, &archive).materialize(
            &report_path,
            &["app/code.rs".into()],
            &head,
            diff,
            &directory.join("summary.md"),
            &directory.join("comments.json"),
        );
        if ambiguous {
            assert!(result.unwrap_err().contains("absent"));
        } else {
            result.unwrap();
            let rebound = read_json(&report_path).unwrap();
            assert_eq!(rebound["findings"][0]["evidence"][0]["path"], json!("app/code.rs"));
        }
        fs::remove_dir_all(directory).unwrap();
    }
}

#[test]
fn findings_that_stay_invalid_are_dropped_but_never_all_of_them() {
    let fixture = Fixture::new();
    let mut invalid = fixture.report["findings"][0].clone();
    invalid["title"] = json!("Cites a file the snapshot lacks");
    invalid["evidence"][0]["path"] = json!("absent/elsewhere.rs");
    let mut report = fixture.report.clone();
    report["findings"] = json!([fixture.report["findings"][0].clone(), invalid.clone()]);
    fixture.write(&report);
    fixture
        .run(&AtomicBool::new(false), |_| panic!("an absent path is not repairable"))
        .unwrap();
    let kept = read_json(&fixture.directory.join("report.json")).unwrap();
    assert_eq!(kept["findings"].as_array().unwrap().len(), 1);
    assert_eq!(kept["findings"][0]["title"], fixture.report["findings"][0]["title"]);
    assert_eq!(kept["missingEvidence"], json!([]));
    assert_eq!(
        read_json(&fixture.directory.join(DROPPED)).unwrap(),
        json!([{"path":"code.rs","title":"Cites a file the snapshot lacks"}])
    );

    let mut only = fixture.report.clone();
    only["findings"] = json!([invalid]);
    fixture.write(&only);
    let error = fixture
        .run(&AtomicBool::new(false), |_| panic!("an absent path is not repairable"))
        .unwrap_err();
    assert!(error.contains("absent"), "{error}");
    assert_eq!(read_json(&fixture.directory.join("report.json")).unwrap(), only);
    // A chunk drops that last point too, so the other chunks' findings stand; it stays a warning.
    fs::remove_file(fixture.directory.join(DROPPED)).unwrap();
    drop_every_invalid(&fixture.directory, || fixture.validate()).unwrap();
    assert_eq!(
        read_json(&fixture.directory.join("report.json")).unwrap()["findings"],
        json!([])
    );
    assert_eq!(
        read_json(&fixture.directory.join(DROPPED)).unwrap(),
        json!([{"path":"code.rs","title":"Cites a file the snapshot lacks"}])
    );

    // A general question that stays invalid is dropped the same way, beside a valid finding.
    let question = json!({
        "title": "Is the retry meant to be unbounded?", "detail": "Fixture detail",
        "action": "Confirm the bound", "evidence": invalid["evidence"].clone(), "counterEvidence": invalid["counterEvidence"].clone(),
    });
    let mut asked = fixture.report.clone();
    asked["questions"] = json!([question]);
    fixture.write(&asked);
    fixture
        .run(&AtomicBool::new(false), |_| panic!("an absent path is not repairable"))
        .unwrap();
    let kept = read_json(&fixture.directory.join("report.json")).unwrap();
    assert_eq!(kept["findings"].as_array().unwrap().len(), 1);
    assert_eq!(kept["questions"], json!([]));

    // So is a suggestion.
    let suggestion = json!({
        "title": "No test covers the retry", "detail": "Fixture detail", "action": "Add a test",
        "path": "code.rs", "line": 1, "evidence": invalid["evidence"].clone(),
        "counterEvidence": invalid["counterEvidence"].clone(),
    });
    let mut suggested = fixture.report.clone();
    suggested["suggestions"] = json!([suggestion]);
    fixture.write(&suggested);
    fixture
        .run(&AtomicBool::new(false), |_| panic!("an absent path is not repairable"))
        .unwrap();
    let kept = read_json(&fixture.directory.join("report.json")).unwrap();
    assert_eq!(kept["findings"].as_array().unwrap().len(), 1);
    assert_eq!(kept["suggestions"], json!([]));
}

#[test]
fn a_chunk_drops_every_invalid_point_after_a_drop_that_kept_the_last() {
    let fixture = Fixture::new();
    let mut invalid = fixture.report["findings"][0].clone();
    invalid["evidence"][0]["path"] = json!("absent/elsewhere.rs");
    let question = json!({
        "title": "Is the retry meant to be unbounded?", "detail": "Fixture detail",
        "action": "Confirm the bound", "evidence": invalid["evidence"].clone(), "counterEvidence": invalid["counterEvidence"].clone(),
    });
    let mut report = fixture.report.clone();
    report["findings"] = json!([invalid]);
    report["questions"] = json!([question]);
    fixture.write(&report);
    // The repair drops the finding, keeps the last point and returns the finding's error, whose
    // index no longer exists.
    let error = fixture
        .run(&AtomicBool::new(false), |_| panic!("an absent path is not repairable"))
        .unwrap_err();
    assert!(error.contains("findings[0]"), "{error}");
    assert!(drop_points(&fixture.directory, error, || fixture.validate(), false).is_err());
    drop_every_invalid(&fixture.directory, || fixture.validate()).unwrap();
    let kept = read_json(&fixture.directory.join("report.json")).unwrap();
    assert_eq!(
        (kept["findings"].clone(), kept["questions"].clone()),
        (json!([]), json!([]))
    );
}

#[test]
fn the_only_finding_is_never_dropped_for_a_suggestion_left_alone() {
    let fixture = Fixture::new();
    let valid = &fixture.report["findings"][0];
    let suggestion = json!({
        "title": "No test covers the retry", "detail": "Fixture detail", "action": "Add a test",
        "path": valid["path"].clone(), "line": valid["line"].clone(), "evidence": valid["evidence"].clone(),
        "counterEvidence": valid["counterEvidence"].clone(),
    });
    let mut invalid = valid.clone();
    invalid["evidence"][0]["path"] = json!("absent/elsewhere.rs");
    let mut report = fixture.report.clone();
    report["findings"] = json!([invalid]);
    report["suggestions"] = json!([suggestion]);
    fixture.write(&report);
    let error = fixture
        .run(&AtomicBool::new(false), |_| panic!("an absent path is not repairable"))
        .unwrap_err();
    assert!(error.contains("absent"), "{error}");
    assert_eq!(
        read_json(&fixture.directory.join("report.json")).unwrap()["findings"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
}
