use super::*;
use serde_json::json;

#[cfg(unix)]
#[test]
fn isolated_cli_requires_supported_configuration_and_fail_closed_tool_guard() {
    let command = cli_command();
    assert_eq!(command.get_program(), CLI);
    assert!(command
        .get_envs()
        .any(|(key, value)| key == "HOME" && value == Some(std::ffi::OsStr::new(SESSION))));
    assert!(!command.get_envs().any(|(key, _)| key == "CLAUDE_CODE_OAUTH_TOKEN"));
    let config = session_config();
    assert_eq!(config["permissions"]["defaultMode"], "dontAsk");
    assert_eq!(config["hooks"]["PreToolUse"][0]["matcher"], "*");
    assert_eq!(
        config["hooks"]["PreToolUse"][0]["hooks"][0]["command"],
        "/usr/local/bin/ocr-sandbox tool-guard || exit 2"
    );
    for tool in ["Agent", "Task", "WebFetch", "WebSearch", "mcp__*"] {
        assert!(config["permissions"]["deny"].as_array().unwrap().contains(&json!(tool)));
    }
    assert!(config["permissions"]["deny"]
        .as_array()
        .unwrap()
        .contains(&json!("Read(//scratch/**)")));
    assert!(config["permissions"]["allow"]
        .as_array()
        .unwrap()
        .contains(&json!("Write(//output/report.json)")));
    assert!(!crate::providers::claude::RUNNER_TOOLS.split(',').any(|tool| [
        "Agent",
        "Task",
        "WebFetch",
        "WebSearch",
        "Edit"
    ]
    .contains(&tool)));
}

#[test]
fn isolated_tool_guard_rejects_unknown_tools_paths_and_malformed_input() {
    let call = |tool: &str, input: serde_json::Value| {
        guard_tool(
            json!({"hook_event_name":"PreToolUse","tool_name":tool,"tool_input":input})
                .to_string()
                .as_bytes(),
        )
    };
    assert!(call("Read", json!({"file_path":"/work/context/PR.md","limit":200})).is_ok());
    assert!(call("Read", json!({"file_path":"/evidence/snapshot/trees/a/b.rs"})).is_ok());
    assert!(call("Grep", json!({"pattern":"guard","path":"/evidence/snapshot/trees"})).is_ok());
    assert!(call("Grep", json!({"pattern":"guard"})).is_ok());
    assert!(call("Glob", json!({"pattern":"**/*.rs","path":"/work/context"})).is_ok());
    assert!(call("Write", json!({"file_path":"/output/report.json","content":"{}"})).is_ok());
    for (tool, input) in [
        ("Read", json!({"file_path":"/scratch/session/settings.json"})),
        ("Read", json!({"file_path":"/claude-auth/claude/oauth-token"})),
        ("Read", json!({"file_path":"/work/../proc/self/environ"})),
        ("Read", json!({"file_path":"context/PR.md"})),
        ("Grep", json!({"pattern":"token","path":"/proc"})),
        ("Glob", json!({"pattern":"*","path":"/"})),
        ("Write", json!({"file_path":"/output/other.json","content":"{}"})),
        ("Write", json!({"file_path":"/work/context/PR.md","content":""})),
        ("Bash", json!({"command":"cat /claude-auth/claude/oauth-token"})),
        ("Agent", json!({"prompt":"delegate"})),
        ("WebFetch", json!({"url":"https://example.com"})),
        ("mcp__example__read", json!({})),
        ("NotebookEdit", json!({})),
    ] {
        assert!(call(tool, input.clone()).is_err(), "{tool} {input}");
    }
    for raw in [
        "{}",
        "not-json",
        "{\"tool_name\":\"Read\"}",
        "{\"hook_event_name\":\"PostToolUse\",\"tool_name\":\"Read\"}",
    ] {
        assert!(guard_tool(raw.as_bytes()).is_err());
    }
    assert!(guard_tool(std::io::repeat(b' ').take(8_388_609)).is_err());
}

#[test]
fn rejects_oversized_truncated_and_unknown_frames() {
    assert!(receive::<Request>(&mut (MAX_FRAME as u32 + 1).to_be_bytes().as_slice()).is_err());
    assert!(receive::<Request>(&mut [0, 0, 0, 20, b'{'].as_slice()).is_err());
    let mut bytes = Vec::new();
    send(&mut bytes, &json!({"operation":"Catalog", "token":"not-accepted"})).unwrap();
    assert!(receive::<Request>(&mut bytes.as_slice()).is_err());
}

#[cfg(unix)]
#[test]
fn catalogue_preserves_actionable_runner_errors() {
    let (client, mut server) = UnixStream::pair().unwrap();
    let error = "Runtime model catalogue remained empty; no inference started";
    std::thread::scope(|scope| {
        scope.spawn(|| {
            assert!(matches!(receive::<Request>(&mut server).unwrap(), Request::Catalog {}));
            send(&mut server, &Response::Error { error: error.into() }).unwrap();
        });
        assert_eq!(catalog_stream(client).unwrap_err(), error);
    });
}

#[test]
fn round_trip_and_stage_scope_validation() {
    let mut request = Request::Run {
        archive_id: None,
        preflight: false,
        account: 1,
        provider: "claude".into(),
        effort: String::new(),
        model: "model-a".into(),
        version: "v1".into(),
        prompt: "read snapshot".into(),
        files: BTreeMap::from([("context/changed/file.ts".into(), "source".into())]),
    };
    assert!(validate(&request).is_ok());
    let mut bytes = Vec::new();
    send(&mut bytes, &request).unwrap();
    assert!(matches!(
        receive::<Request>(&mut bytes.as_slice()).unwrap(),
        Request::Run { .. }
    ));
    // OpenCode models carry their provider; no other provider's model may hold a path.
    let with = |provider: &str, name: &str| {
        let mut request = request.clone();
        if let Request::Run { provider: p, model, .. } = &mut request {
            *p = provider.into();
            *model = name.into();
        }
        validate(&request)
    };
    assert!(with("opencode", "opencode-go/deepseek-v4.1-flash").is_ok());
    assert!(with("opencode", "opencode/claude-opus-5-5").is_ok());
    assert!(with("opencode", "anthropic/claude-opus-5-5").is_err());
    assert!(with("opencode", "opencode/../x").is_err());
    assert!(with("claude", "opencode/claude-opus-5-5").is_err());
    if let Request::Run { files, .. } = &mut request {
        files.insert("../auth/worker-token".into(), "invalid".into());
    }
    assert!(validate(&request).is_err());
}

#[cfg(unix)]
#[test]
fn cancellation_returns_partial_usage_over_private_ipc() {
    let root = std::env::temp_dir().join(format!("sandbox-ipc-{}", secret().unwrap()));
    fs::create_dir(&root).unwrap();
    let (client, mut server) = UnixStream::pair().unwrap();
    for stream in [&client, &server] {
        stream.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
    }
    let cancel = AtomicBool::new(false);
    std::thread::scope(|scope| {
        scope.spawn(move || {
            assert!(matches!(receive::<Request>(&mut server).unwrap(), Request::Run { .. }));
            send(&mut server, &Response::Started).unwrap();
            let error = check_control(&mut server).unwrap_err();
            send(
                &mut server,
                &Response::Finished {
                    report: None,
                    decisions: None,
                    transcript: Some("{\"partial\":true}".into()),
                    error: Some(error),
                    credential: None,
                },
            )
            .unwrap();
        });
        let request = Request::Run {
            archive_id: None,
            preflight: false,
            account: 1,
            provider: "claude".into(),
            effort: String::new(),
            model: "fixture".into(),
            version: "v1".into(),
            prompt: String::new(),
            files: BTreeMap::new(),
        };
        let result = exchange(
            client,
            &request,
            &root,
            &root.join("transcript.json"),
            &cancel,
            &mut || cancel.store(true, Ordering::Relaxed),
        );
        assert_eq!(result.unwrap_err(), "cancelled");
    });
    assert_eq!(
        fs::read_to_string(root.join("transcript.json")).unwrap(),
        "{\"partial\":true}"
    );
    fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
#[test]
fn runner_control_distinguishes_cancellation_disconnect_and_invalid_bytes() {
    for (signal, expected) in [
        (Some(1), "cancelled"),
        (Some(2), "Invalid runner control message"),
        (None, "Broker disconnected; inference cancelled"),
    ] {
        let (mut client, mut server) = UnixStream::pair().unwrap();
        server.set_read_timeout(Some(Duration::from_millis(5))).unwrap();
        assert!(check_control(&mut server).is_ok());
        if let Some(signal) = signal {
            client.write_all(&[signal]).unwrap();
        } else {
            drop(client);
        }
        assert_eq!(check_control(&mut server).unwrap_err(), expected);
    }
}

#[test]
fn dispatch_contains_only_assigned_evidence_and_fixed_output_paths() {
    let root = std::env::temp_dir().join(format!("sandbox-dispatch-{}", secret().unwrap()));
    let stage = root.join("merge");
    let context = root.join("context");
    fs::create_dir_all(&stage).unwrap();
    fs::create_dir_all(&context).unwrap();
    fs::write(context.join("PR_DIFF.patch"), "historical diff").unwrap();
    fs::write(root.join("pending-result.json"), "not sent").unwrap();
    fs::write(stage.join("chunk_findings.json"), "[]").unwrap();
    fs::write(stage.join("prompt.md"), "Read /work/context/PR_DIFF.patch and chunk_summaries.md; write /work/merge/report.json and /work/merge/finding_decisions.json").unwrap();
    let Request::Run { prompt, files, .. } = stage_request(
        "fixture",
        "v1",
        1,
        "claude",
        &root,
        &stage,
        &context,
        &stage.join("prompt.md"),
    )
    .unwrap() else {
        panic!("Expected a stage");
    };
    assert_eq!(files.len(), 2);
    assert!(!files.contains_key("pending-result.json"));
    assert!(prompt.contains("/output/report.json"));
    assert!(prompt.contains("/output/finding_decisions.json"));
    assert!(prompt.contains("/work/integration/chunk_summaries.md"));
    fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
#[test]
fn frozen_ipc_preflight_transfers_archive_without_starting_inference() {
    let root = PathBuf::from("/tmp").join(format!("frozen-ipc-{}", &secret().unwrap()[..16]));
    let context = root.join("context");
    let stage = root.join("single");
    fs::create_dir_all(context.join("skill")).unwrap();
    fs::create_dir_all(&stage).unwrap();
    fs::write(context.join("PR.md"), "Intent").unwrap();
    fs::write(context.join("skill/SKILL.md"), "Review").unwrap();
    let archive = crate::sandbox::tests::archive();
    let diff = "diff --git a/src/a.ts b/src/a.ts\n--- a/src/a.ts\n+++ b/src/a.ts\n@@ -0,0 +1 @@\n+test\n";
    archive
        .prepare_stage(&context, &["src/a.ts".into()], diff, &archive.scope().merge_base_sha)
        .unwrap();
    fs::write(stage.join("prompt.md"), "Write /work/single/report.json").unwrap();
    let request = stage_request(
        "fixture",
        "v1",
        1,
        "claude",
        &root,
        &stage,
        &context,
        &stage.join("prompt.md"),
    )
    .unwrap();
    let socket = root.join("runner.sock");
    let listener = UnixListener::bind(&socket).unwrap();
    let starts = std::sync::atomic::AtomicUsize::new(0);
    std::thread::scope(|scope| {
        scope.spawn(|| {
            for (preflight, altered) in [(true,false),(false,false),(false,true)] {
                let (mut stream,_)=listener.accept().unwrap();
                let (request,received)=receive_job(&mut stream).unwrap();
                let received=received.unwrap();assert_eq!(received.descriptor(),archive.descriptor());
                let Request::Run {files,preflight:actual,..}=request else {panic!("Missing stage")};
                assert_eq!(preflight,actual);
                if altered {
                    let error = received.validate_stage_files(&files).unwrap_err();
                    send(&mut stream, &Response::Error { error: error.to_string() }).unwrap();
                    continue;
                }
                received.validate_stage_files(&files).unwrap();
                if preflight {send(&mut stream,&Response::Ready).unwrap();}
                else {
                    send(&mut stream,&Response::Started).unwrap();
                    send(&mut stream,&Response::Finished {report:Some(json!({"contractVersion":2,"snapshotId":archive.descriptor().id,"reviewedFiles":["src/a.ts"],"missingEvidence":[],"findings":[]}).to_string()),decisions:None,transcript:Some("{}".into()),error:None,credential:None}).unwrap();
                }
            }
        });
        preflight_at(&socket, &archive, &request, &AtomicBool::new(false)).unwrap();
        assert!(!stage.join("report.json").exists());
        assert_eq!(starts.load(Ordering::SeqCst), 0);
        execute_archive_at(
            &socket,
            &archive,
            &request,
            &stage,
            &stage.join("transcript.json"),
            &AtomicBool::new(false),
            &mut || {
                starts.fetch_add(1, Ordering::SeqCst);
            },
        )
        .unwrap();
        let mut tampered = request.clone();
        if let Request::Run { files, .. } = &mut tampered {
            files.insert("context/PR.md".into(), "Altered after preflight".into());
        }
        assert!(execute_archive_at(
            &socket,
            &archive,
            &tampered,
            &stage,
            &stage.join("transcript.json"),
            &AtomicBool::new(false),
            &mut || {
                starts.fetch_add(1, Ordering::SeqCst);
            }
        )
        .is_err());
    });
    assert_eq!(starts.load(Ordering::SeqCst), 1);
    crate::review::evidence::EvidenceResolver::frozen(&context, &archive)
        .materialize(
            &stage.join("report.json"),
            &["src/a.ts".into()],
            &archive.scope().head_sha,
            diff,
            &stage.join("summary.md"),
            &stage.join("comments.json"),
        )
        .unwrap();
    fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
#[test]
fn rejects_symlink_outputs() {
    let root = std::env::temp_dir().join(format!("sandbox-output-{}", secret().unwrap()));
    fs::create_dir(&root).unwrap();
    fs::write(root.join("real"), "{}").unwrap();
    std::os::unix::fs::symlink(root.join("real"), root.join("report.json")).unwrap();
    assert!(read_output(&root.join("report.json"), 100).is_err());
    fs::remove_dir_all(root).unwrap();
}

fn final_transcript(message: serde_json::Value) -> String {
    [
        json!({"type":"system","subtype":"init","model":"claude-opus-5-5","tools":["Read"]}),
        json!({"type":"assistant","message":{"content":[{"type":"tool_use","name":"Read","input":{"file_path":"/work/context/PR.md"}}]}}),
        json!({"type":"user","message":{"content":[{"type":"tool_result","content":"untrusted output"}]}}),
        json!({"type":"result","subtype":"success","is_error":false,"result":message,"modelUsage":{"claude-opus-5-5":{"inputTokens":10,"outputTokens":5}}}),
    ].iter().map(|event| event.to_string()).collect::<Vec<_>>().join("\n")
}

#[test]
fn final_response_reads_only_the_terminal_answer_and_preserves_objects() {
    let report = json!({"contractVersion":2,"snapshotId":"snapshot","reviewedFiles":["a.ts"],"missingEvidence":[],"findings":[]});
    let decisions = json!({"contractVersion":2,"snapshotId":"snapshot","decisions":[{"candidateId":"candidate-1","decision":"discard","reason":"Guard refutes it","counterEvidence":[{"path":"a.ts","revision":"head","line":1,"quote":"guard"}]}]});
    let output = json!({"report":report,"findingDecisions":decisions}).to_string();
    for message in [
        json!(output),
        json!(format!("```json\n{output}\n```")),
        json!(format!("```JSON\n{output}\n```")),
        json!(format!(
            "Review complete. All assigned files were covered.\n\n```json\n{output}\n```"
        )),
        json!(format!("Here is the review:\n{output}")),
        json!(format!("{output}\n\nThe report above covers every assigned file.")),
    ] {
        let (actual, actual_decisions) = final_response(Some(&final_transcript(message))).unwrap();
        assert_eq!(serde_json::from_str::<serde_json::Value>(&actual).unwrap(), report);
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&actual_decisions.unwrap()).unwrap(),
            decisions
        );
    }
}

#[test]
fn final_response_of_other_clis_accepts_stringified_objects_and_bare_or_empty_decisions() {
    let report = json!({"reviewedFiles":["a.ts"],"missingEvidence":[],"findings":[]});
    let decision = json!({"candidateId":"candidate-1","decision":"keep","commentIndex":0,"reason":"Verified"});
    let decode = |raw: Option<String>| raw.map(|raw| serde_json::from_str::<serde_json::Value>(&raw).unwrap());
    for (output, decisions) in [
        (json!({"report":report.to_string()}), None),
        (json!({"report":report,"findingDecisions":[]}), None),
        (json!({"report":report,"findingDecisions":null}), None),
        (
            json!({"report":report,"findingDecisions":[decision]}),
            Some(json!({"decisions":[decision]})),
        ),
        (
            json!({"report":report,"findingDecisions":json!({"decisions":[decision]}).to_string()}),
            Some(json!({"decisions":[decision]})),
        ),
    ] {
        let (actual, actual_decisions) = parse_final_response(&output.to_string(), true).unwrap();
        assert_eq!(serde_json::from_str::<serde_json::Value>(&actual).unwrap(), report);
        assert_eq!(decode(actual_decisions), decisions);
        // Claude's output stays strict.
        if output["findingDecisions"] != json!(null) || output["report"].is_string() {
            assert!(parse_final_response(&output.to_string(), false).is_err(), "{output}");
        }
    }
}

#[test]
fn final_response_of_other_clis_gives_a_bare_decisions_list_the_report_identity() {
    let report =
        json!({"contractVersion":2,"snapshotId":"snapshot","reviewedFiles":[],"missingEvidence":[],"findings":[]});
    let decision = json!({"candidateId":"candidate-1","decision":"discard","reason":"Guard"});
    let output = json!({"report":report,"findingDecisions":[decision]}).to_string();
    let (_, decisions) = parse_final_response(&output, true).unwrap();
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&decisions.unwrap()).unwrap(),
        json!({"contractVersion":2,"snapshotId":"snapshot","decisions":[decision]})
    );
}

#[test]
fn final_response_escapes_stray_backslashes_in_quoted_code() {
    let raw = r#"{"report":{"findings":[{"quote":"value.match(/\d+\s/) and \"done\" \\ \n"}]}}"#;
    let (report, _) = final_response(Some(&final_transcript(json!(raw)))).unwrap();
    let quote = serde_json::from_str::<serde_json::Value>(&report).unwrap()["findings"][0]["quote"]
        .as_str()
        .unwrap()
        .to_string();
    assert_eq!(quote, "value.match(/\\d+\\s/) and \"done\" \\ \n");
    let error = final_response(Some(&final_transcript(json!(
        r#"{"report":{"title":"unclosed "quote" here"}}"#
    ))))
    .unwrap_err();
    assert!(
        error.starts_with("Review final response is not valid structured JSON (") && error.contains("column"),
        "{error}"
    );
}

#[test]
fn final_response_drops_closers_that_close_nothing() {
    let raw = r#"{"report":{"findings":[{"title":"a","counterEvidence":[{"quote":"if (x) {"}]}},{"title":"b ]}"}]}}"#;
    let (report, _) = final_response(Some(&final_transcript(json!(raw)))).unwrap();
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&report).unwrap(),
        json!({"findings":[{"title":"a","counterEvidence":[{"quote":"if (x) {"}]},{"title":"b ]}"}]})
    );
}

#[test]
fn final_response_rejects_missing_malformed_and_unsuccessful_results() {
    for raw in [
        None,
        Some(""),
        Some("not JSON"),
        Some("{}"),
        Some(r#"{"type":"assistant"}"#),
    ] {
        assert!(final_response(raw).is_err());
    }
    for message in [
        json!(""),
        json!("Done"),
        json!("{}"),
        json!(r#"{"report":[]}"#),
        json!(r#"{"report":{},"findingDecisions":[]}"#),
        json!(r#"{"report":{},"unexpected":true}"#),
        json!("```json\n{\"report\":{}}"),
        json!("Review complete with no findings."),
        serde_json::Value::Null,
    ] {
        assert!(
            final_response(Some(&final_transcript(message.clone()))).is_err(),
            "{message}"
        );
    }
    let valid = json!({"report":{"findings":[]}}).to_string();
    assert!(final_response(Some(&final_transcript(json!(valid.clone())))).is_ok());
    let failed = json!({"type":"result","subtype":"error_max_turns","is_error":true,"result":valid}).to_string();
    assert!(final_response(Some(&failed)).is_err());
    assert!(final_response(Some(&final_transcript(json!(
        json!({"report":{"text":"x".repeat(2_000_000)}}).to_string()
    ))))
    .unwrap_err()
    .contains("byte budget"));
}

#[cfg(unix)]
#[test]
fn final_response_materializes_report_and_decisions_without_model_written_files() {
    let root = std::env::temp_dir().join(format!("review-final-response-{}", secret().unwrap()));
    fs::create_dir(&root).unwrap();
    fs::create_dir(root.join("changed")).unwrap();
    fs::write(root.join("changed/code.rs"), "call();\n").unwrap();
    let reference = json!({"path":"code.rs","line":1,"quote":"call();"});
    let point = json!({"kind":"finding","title":"Boundary","detail":"Guard is missing","action":"Restore guard","path":"code.rs","line":1,"side":"RIGHT","evidence":[reference],"counterEvidence":[reference]});
    let report = json!({"reviewedFiles":["code.rs"],"missingEvidence":[],"findings":[point]});
    let decisions = json!({"decisions":[{"candidateId":"candidate-1","decision":"keep","commentIndex":0}]});
    let transcript = final_transcript(json!(json!({"report":report,"findingDecisions":decisions}).to_string()));
    let (client, mut server) = UnixStream::pair().unwrap();
    std::thread::scope(|scope| {
        scope.spawn(|| {
            receive::<Request>(&mut server).unwrap();
            send(&mut server, &Response::Started).unwrap();
            let (report, decisions) = final_response(Some(&transcript)).unwrap();
            assert!(!root.join("report.json").exists());
            assert!(!root.join("finding_decisions.json").exists());
            send(
                &mut server,
                &Response::Finished {
                    report: Some(report),
                    decisions,
                    transcript: Some(transcript.clone()),
                    error: None,
                    credential: None,
                },
            )
            .unwrap();
        });
        let request = Request::Run {
            archive_id: None,
            preflight: false,
            account: 1,
            provider: "claude".into(),
            effort: String::new(),
            model: "fixture".into(),
            version: "v1".into(),
            prompt: String::new(),
            files: BTreeMap::new(),
        };
        exchange(
            client,
            &request,
            &root,
            &root.join("transcript.json"),
            &AtomicBool::new(false),
            &mut || {},
        )
        .unwrap();
    });
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&fs::read_to_string(root.join("report.json")).unwrap()).unwrap(),
        report
    );
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&fs::read_to_string(root.join("finding_decisions.json")).unwrap())
            .unwrap(),
        decisions
    );
    let diff = "diff --git a/code.rs b/code.rs\n--- /dev/null\n+++ b/code.rs\n@@ -0,0 +1 @@\n+call();\n";
    crate::review::evidence::materialize(
        &root.join("report.json"),
        &root,
        &["code.rs".into()],
        "head",
        diff,
        &root.join("summary.md"),
        &root.join("comments.json"),
    )
    .unwrap();
    let comments = fs::read_to_string(root.join("comments.json")).unwrap();
    let mapped =
        crate::review::evidence::remap_decisions(&root.join("report.json"), &decisions.to_string(), &comments).unwrap();
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&mapped).unwrap()["decisions"][0]["commentIndex"],
        0
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn kept_work_entries_accept_plain_names_and_never_stage_paths() {
    assert!(kept_work_entries(None).unwrap().is_empty());
    assert_eq!(kept_work_entries(Some("repo,pr,out")).unwrap(), ["repo", "pr", "out"]);
    for invalid in ["../etc", "repo/x", "context", "integration", "prompt.md", "a b"] {
        assert!(kept_work_entries(Some(invalid)).is_err(), "{invalid}");
    }
}

#[test]
fn final_response_moves_a_report_identity_written_beside_it_into_the_report() {
    let report = json!({"reviewedFiles":["a.ts"],"missingEvidence":[],"findings":[]});
    let output = json!({"contractVersion":2,"snapshotId":"snapshot","report":report}).to_string();
    let (actual, _) = parse_final_response(&output, false).unwrap();
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&actual).unwrap(),
        json!({"contractVersion":2,"snapshotId":"snapshot","reviewedFiles":["a.ts"],"missingEvidence":[],"findings":[]})
    );
    let same = json!({"snapshotId":"snapshot","report":{"snapshotId":"snapshot","findings":[]}}).to_string();
    assert!(parse_final_response(&same, false).is_ok());
    let conflicting = json!({"snapshotId":"other","report":{"snapshotId":"snapshot","findings":[]}}).to_string();
    let (report, _) = parse_final_response(&conflicting, false).unwrap();
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&report).unwrap()["snapshotId"],
        "snapshot"
    );
    assert!(parse_final_response(&json!({"title":"x","report":{}}).to_string(), false).is_err());
}

#[test]
fn final_response_reads_an_object_written_right_after_a_sentence() {
    let report = json!({"reviewedFiles":["a.go"],"missingEvidence":[],"findings":[]});
    let output = format!("I checked every caller. {}", json!({"report":report}));
    let (actual, _) = parse_final_response(&output, false).unwrap();
    assert_eq!(serde_json::from_str::<serde_json::Value>(&actual).unwrap(), report);
    assert!(parse_final_response("I found {two} issues but wrote no object.", false).is_err());
}

#[test]
fn a_fence_mentioned_inside_the_report_is_not_read_as_one() {
    let report = json!({"reviewedFiles":["a.md"],"missingEvidence":[],"findings":[{"detail":"A plain ``` fence has no language."}]});
    let output = format!("I reviewed the file.\n{}", json!({"report":report}));
    let (actual, _) = parse_final_response(&output, false).unwrap();
    assert_eq!(serde_json::from_str::<serde_json::Value>(&actual).unwrap(), report);
    let fenced = format!("Here it is:\n```json\n{}\n```", json!({"report":report}));
    assert!(parse_final_response(&fenced, false).is_ok());
    assert!(parse_final_response(&format!("```json\n{}", json!({"report":report})), false).is_err());
}

#[test]
fn final_response_of_other_clis_takes_report_fields_written_beside_or_without_the_wrapper() {
    let report = json!({"reviewedFiles":["a.ts"],"missingEvidence":[],"findings":[]});
    let decision = json!({"candidateId":"candidate-1","decision":"keep","commentIndex":0,"reason":"Verified"});
    let beside = json!({"report":report,"requestedFiles":[],"findings":[]}).to_string();
    let (actual, _) = parse_final_response(&beside, true).unwrap();
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&actual).unwrap(),
        json!({"reviewedFiles":["a.ts"],"missingEvidence":[],"findings":[],"requestedFiles":[]})
    );
    let bare = json!({"reviewedFiles":["a.ts"],"missingEvidence":[],"findings":[],"findingDecisions":[decision]});
    let (actual, decisions) = parse_final_response(&bare.to_string(), true).unwrap();
    assert_eq!(serde_json::from_str::<serde_json::Value>(&actual).unwrap(), report);
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&decisions.unwrap()).unwrap(),
        json!({"decisions":[decision]})
    );
    let unclosed = format!("Here it is:\n```json\n{}", json!({"report":report}));
    assert!(parse_final_response(&unclosed, true).is_ok());
    // Claude's output stays strict, and an object that is no report is never read as one.
    for strict in [beside, bare.to_string(), unclosed] {
        assert!(parse_final_response(&strict, false).is_err(), "{strict}");
    }
    assert!(parse_final_response(&json!({"title":"x"}).to_string(), true).is_err());
    // Findings written beside a report that holds other findings are never dropped: the response fails.
    let conflict = json!({"report":report,"findings":[{"title":"lost"}]}).to_string();
    assert!(parse_final_response(&conflict, true).is_err());
}

#[test]
fn final_response_of_other_clis_drops_the_identity_repeated_in_every_point() {
    let identity = json!({"contractVersion":2,"snapshotId":"snapshot"});
    let point =
        json!({"title":"t","detail":"d","action":"a","path":"a.rs","line":1,"evidence":[],"counterEvidence":[]});
    let with = |extra: serde_json::Value| {
        let mut point = point.clone();
        point
            .as_object_mut()
            .unwrap()
            .extend(identity.as_object().unwrap().clone());
        point
            .as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        point
    };
    let output = json!({"report":{
        "contractVersion":2,"snapshotId":"snapshot","reviewedFiles":["a.rs"],"missingEvidence":[],
        "findings":[with(json!({"kind":"finding"})), with(json!({"snapshotId":"other"}))],
        "suggestions":[with(json!({"kind":"suggestion"}))],
        "questions":[with(json!({"kind":"finding"}))]
    },"findingDecisions":identity});
    let (report, decisions) = parse_final_response(&output.to_string(), true).unwrap();
    let report = serde_json::from_str::<serde_json::Value>(&report).unwrap();
    let mut finding = point.clone();
    finding["kind"] = json!("finding");
    assert_eq!(report["findings"][0], finding);
    // A value that differs from the report's is left for validation to reject.
    assert_eq!(report["findings"][1]["snapshotId"], "other");
    assert_eq!(report["suggestions"][0], point);
    assert_eq!(report["questions"][0]["kind"], "finding");
    assert_eq!(decisions, None);
    // Claude's output stays as written.
    let (strict, decisions) = parse_final_response(&output.to_string(), false).unwrap();
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&strict).unwrap()["suggestions"][0]["kind"],
        "suggestion"
    );
    assert!(decisions.is_some());
}

#[test]
fn final_response_of_other_clis_reads_a_contract_version_written_as_a_string() {
    let report = json!({"contractVersion":"2","snapshotId":"s","reviewedFiles":["a.rs"],"missingEvidence":[],"findings":[
        {"contractVersion":"2","snapshotId":"s","kind":"finding"}
    ]});
    let decision = json!({"candidateId":"candidate-1","decision":"discard","reason":"Guard"});
    let output =
        json!({"report":report,"findingDecisions":{"contractVersion":"2","snapshotId":"s","decisions":[decision]}});
    let (actual, decisions) = parse_final_response(&output.to_string(), true).unwrap();
    let actual = serde_json::from_str::<serde_json::Value>(&actual).unwrap();
    assert_eq!(actual["contractVersion"], 2);
    assert_eq!(actual["findings"][0], json!({"kind":"finding"}));
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&decisions.unwrap()).unwrap()["contractVersion"],
        2
    );
    // Only a whole number is read; Claude's output stays as written.
    let odd = json!({"report":{"contractVersion":"v2","reviewedFiles":[],"missingEvidence":[],"findings":[]}});
    let (odd, _) = parse_final_response(&odd.to_string(), true).unwrap();
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&odd).unwrap()["contractVersion"],
        "v2"
    );
    let (strict, _) = parse_final_response(&json!({"report":report}).to_string(), false).unwrap();
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&strict).unwrap()["contractVersion"],
        "2"
    );
}
