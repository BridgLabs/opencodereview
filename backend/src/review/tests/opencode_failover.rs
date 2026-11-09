use super::super::*;
use std::io::{BufRead, BufReader};

struct Runner(std::process::Child);

#[test]
fn chunk_cache_identity_separates_confirmed_go_models_efforts_and_providers() {
    let first = crate::providers::go_catalogue::fixture("fixture-model-a", &["medium", "high"], "chat-completions");
    let second = crate::providers::go_catalogue::fixture("fixture-model-b", &["medium"], "chat-completions");
    let high = first.select("fixture-model-a", "high").unwrap();
    let first = first.select("fixture-model-a", "medium").unwrap();
    let second = second.select("fixture-model-b", "medium").unwrap();
    let key = |provider: &str, model: &str, effort: &str, snapshot| {
        let engine = super::super::frozen::cache_identity(provider, model, effort, snapshot);
        super::super::sha256_hex(
            serde_json::to_string(&serde_json::json!({"version":3,"engine":engine}))
                .unwrap()
                .as_bytes(),
        )
    };
    let original = key("opencode", &first.model.id, &first.effort, Some(&first));
    assert_ne!(
        original,
        key("opencode", &second.model.id, &second.effort, Some(&second))
    );
    assert_ne!(original, key("opencode", &high.model.id, &high.effort, Some(&high)));
    assert_ne!(original, key("claude", &first.model.id, &first.effort, None));
}

impl Drop for Runner {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[test]
fn opencode_review_failover_attributes_tokens_and_never_replays_infrastructure_failures() {
    const NAME: &str = "review::usage_tests::opencode_failover_tests::opencode_review_failover_attributes_tokens_and_never_replays_infrastructure_failures";
    if std::env::var("OCR_GO_FLOW_TEST").as_deref() != Ok("child") {
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", NAME, "--nocapture", "--test-threads=1"])
            .env("OCR_GO_FLOW_TEST", "child")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        for line in String::from_utf8_lossy(&output.stdout)
            .lines()
            .filter_map(|line| line.split_once("GO_FLOW ").map(|(_, message)| message))
        {
            println!("GO_FLOW {line}");
        }
        return;
    }
    for outcome in [
        "auth",
        "limit",
        "startup",
        "health",
        "removed",
        "missing",
        "discovery",
        "missing-third",
        "discovery-third",
        "removed-third",
        "claude-missing",
        "codex-missing",
        "claude-discovery",
        "codex-discovery",
        "claude-failover-missing",
        "codex-failover-discovery",
        "claude-failover-go",
        "claude-failover-missing-third",
        "codex-failover-discovery-third",
        "claude-new-ready",
        "claude-new-ready-alone",
        "claude-frozen-preferences",
    ] {
        run_case(outcome);
    }
}

fn prepared_account_case(
    inputs: &mut ReviewInputs,
    db: &Database,
    root: &Path,
    log: &Path,
    first: i64,
    second: i64,
    third: Option<i64>,
) {
    let cancel = AtomicBool::new(false);
    let head = inputs.fixed_head_sha.clone().unwrap();
    let base = std::process::Command::new("git")
        .arg("-C")
        .arg(&inputs.repo_path)
        .args(["rev-parse", "HEAD~"])
        .output()
        .unwrap();
    assert!(base.status.success());
    let base = String::from_utf8(base.stdout).unwrap().trim().to_string();
    let archive = crate::review_archive::capture(
        &inputs.repo_path,
        crate::review_archive::Scope {
            host: github::gh_host(),
            repository: inputs.full_name.clone(),
            head_sha: head,
            base_sha: base.clone(),
            merge_base_sha: base.clone(),
            prior_sha: None,
        },
        &cancel,
    )
    .unwrap();
    let work = root.join("prepared");
    let context = work.join("context");
    fs::create_dir_all(context.join("skill")).unwrap();
    fs::write(context.join("PR.md"), "Synthetic review").unwrap();
    fs::write(context.join("skill/SKILL.md"), &inputs.skill_content).unwrap();
    let diff = std::process::Command::new("git")
        .arg("-C")
        .arg(&inputs.repo_path)
        .args(["diff", &base, "HEAD"])
        .output()
        .unwrap();
    assert!(diff.status.success());
    archive
        .prepare_stage(
            &context,
            &["code.rs".into()],
            std::str::from_utf8(&diff.stdout).unwrap(),
            &base,
        )
        .unwrap();
    inputs.frozen = Some(std::sync::Arc::new(frozen::FrozenContext::new(archive)));
    let prompt = work.join("prompt.md");
    fs::write(&prompt, "Review frozen evidence.").unwrap();
    let stages = [(work.clone(), context, prompt.clone())];
    frozen::preflight(inputs, &work, &stages, &cancel).unwrap();
    let had_snapshot = crate::accounts::has_credential("opencode", second);
    let expected = if had_snapshot {
        db.mark_ai_account_auth_failed(second, "Synthetic reconnect").unwrap();
        let new = db.create_ai_account("opencode", "New ready Go account").unwrap();
        crate::accounts::store_opencode_key(new.id, "synthetic-new-ready-go").unwrap();
        new.id
    } else {
        crate::accounts::store_opencode_key(second, "synthetic-new-ready-go").unwrap();
        third.unwrap_or(second)
    };
    let changed = crate::providers::go_catalogue::fixture("changed-preference", &["changed-effort"], "responses");
    db.save_go_preference(&changed.select("changed-preference", "changed-effort").unwrap())
        .unwrap();
    crate::providers::go_catalogue::install_test_error("Must not refresh a prepared catalogue");
    db.mark_ai_account_limited(first, None, "Synthetic prior limit")
        .unwrap();
    let transcript = work.join("transcript.json");
    let config = work.join("config.json");
    let stdout = work.join("stdout.log");
    let stderr = work.join("stderr.log");
    let paths = SpawnPaths {
        config: &config,
        prompt: &prompt,
        transcript: &transcript,
        stdout: &stdout,
        stderr: &stderr,
        progress_phase: "reviewing",
        sharded_progress: false,
    };
    let engine = frozen::cache_engine(inputs);
    let additional = frozen::preflight_additional(inputs, &stages, &cancel);
    let result = frozen::execute(inputs, &cancel, &mut |_, _| {}, &paths);
    if !had_snapshot && third.is_none() {
        let error = "OpenCode Go has no frozen confirmed selection; prepare a new review";
        assert_eq!(engine.unwrap_err(), error);
        assert_eq!(additional.unwrap_err(), error);
        assert_eq!(result.unwrap_err(), error);
        assert!(!log.exists());
        assert_eq!(db.ai_account(second).unwrap().status, "ok");
        assert!(db.ai_account(second).unwrap().last_used_at.is_none());
        return;
    }
    let engine = engine.unwrap();
    additional.unwrap();
    result.unwrap();
    let attempts: Vec<serde_json::Value> = fs::read_to_string(log)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(attempts.len(), 1);
    assert_eq!(attempts[0]["account"], expected);
    if had_snapshot {
        assert_eq!(engine["provider"], "opencode");
        assert_eq!(engine["model"], "new-review-model");
        assert_eq!(engine["effort"], "new-review-effort");
        assert_eq!(attempts[0]["model"], "new-review-model");
        assert_eq!(attempts[0]["effort"], "new-review-effort");
        let context = std::sync::Arc::try_unwrap(inputs.frozen.take().unwrap()).ok().unwrap();
        inputs.frozen = Some(std::sync::Arc::new(frozen::FrozenContext::diagnostic(
            context.archive,
            work,
            vec![(
                PathBuf::from(std::env::var("OCR_REVIEW_RUNNER_SOCKETS").unwrap()),
                "2.2.6".into(),
            )],
        )));
        let error = "OpenCode Go review catalogue was not prepared";
        assert_eq!(frozen::cache_engine(inputs).unwrap_err(), error);
        assert_eq!(
            frozen::preflight_additional(inputs, &stages, &cancel).unwrap_err(),
            error
        );
        assert_eq!(
            frozen::execute(inputs, &cancel, &mut |_, _| {}, &paths).unwrap_err(),
            error
        );
    } else {
        assert_eq!(engine["provider"], "codex");
        assert_eq!(attempts[0]["provider"], "codex");
        assert_eq!(db.ai_account(second).unwrap().status, "ok");
        assert!(db.ai_account(second).unwrap().last_used_at.is_none());
    }
}

fn run_case(outcome: &str) {
    let primary_provider = if outcome.starts_with("claude-") {
        "claude"
    } else if outcome.starts_with("codex-") {
        "codex"
    } else {
        "opencode"
    };
    let root = PathBuf::from("/tmp/opencode").join(format!("go-flow-{}", crate::sandbox::secret().unwrap()));
    let repo = root.join("repo");
    fs::create_dir_all(&repo).unwrap();
    std::env::set_var("OCR_DATA_DIR", root.join("data"));
    std::env::set_var("OCR_REVIEW_CLAUDE_AUTH_DIR", root.join("auth"));
    let git = |args: &[&str]| {
        let output = std::process::Command::new("git")
            .arg("-C")
            .arg(&repo)
            .args(["-c", "user.name=Fixture", "-c", "user.email=fixture@example.test"])
            .args(args)
            .output()
            .unwrap();
        assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
        String::from_utf8(output.stdout).unwrap().trim().to_string()
    };
    git(&["init", "--initial-branch=main"]);
    fs::write(repo.join("code.rs"), "pub fn value() -> u8 { 1 }\n").unwrap();
    git(&["add", "."]);
    git(&["commit", "-qm", "base"]);
    let base = git(&["rev-parse", "HEAD"]);
    git(&["checkout", "-qb", "feature"]);
    fs::write(repo.join("code.rs"), "pub fn value() -> u8 { 2 }\n").unwrap();
    git(&["commit", "-qam", "change"]);
    let head = git(&["rev-parse", "HEAD"]);
    git(&["remote", "add", "origin", repo.to_str().unwrap()]);
    let handle = crate::db::Db::for_test();
    let db = handle.open().unwrap();
    db.ensure_temp_repo_for_index(1, "acme/fixture").unwrap();
    let pr = db
        .upsert_pull_request(&crate::db::PullRequestInput {
            repo_id: 1,
            number: 1,
            title: "Synthetic review".into(),
            author: "fixture".into(),
            author_avatar_url: None,
            is_draft: false,
            head_sha: head.clone(),
            base_sha: Some(base),
            base_ref: Some("main".into()),
            head_ref: Some("feature".into()),
            default_branch: Some("main".into()),
            url: None,
            lane: "manual_required".into(),
            status_reason: None,
        })
        .unwrap();
    let run = db.start_review_skeleton_with(pr, Some("fixture")).unwrap();
    db.set_review_run_running(run, "[]").unwrap();
    let catalogue =
        crate::providers::go_catalogue::fixture("new-review-model", &["new-review-effort"], "chat-completions");
    if !outcome.contains("missing") {
        db.save_go_preference(&catalogue.select("new-review-model", "new-review-effort").unwrap())
            .unwrap();
    }
    if outcome.contains("discovery") {
        crate::providers::go_catalogue::install_test_error("Synthetic Go catalogue unavailable");
    } else {
        crate::providers::go_catalogue::install_test_source(if outcome.starts_with("removed") {
            crate::providers::go_catalogue::fixture("replacement-review-model", &[], "responses")
        } else {
            catalogue
        });
    }
    let missing = db.create_ai_account("opencode", "No fresh key").unwrap();
    let first = db.create_ai_account(primary_provider, "Primary account").unwrap();
    let second = db.create_ai_account("opencode", "Secondary Go").unwrap();
    let legacy = crate::accounts::root().join(format!("{}/opencode", missing.id));
    fs::create_dir_all(&legacy).unwrap();
    fs::write(legacy.join("auth.json"), r#"{"key":"legacy-synthetic-not-a-go-key"}"#).unwrap();
    match primary_provider {
        "claude" => crate::accounts::store_claude_token(first.id, "synthetic-primary-claude").unwrap(),
        "codex" => {
            crate::accounts::store_codex_auth(first.id, r#"{"tokens":{"refresh_token":"synthetic-primary-codex"}}"#)
                .unwrap()
        }
        _ => crate::accounts::store_opencode_key(first.id, "synthetic-primary-go").unwrap(),
    }
    if !outcome.starts_with("claude-new-ready") {
        crate::accounts::store_opencode_key(second.id, "synthetic-secondary-go").unwrap();
    }
    let third = if outcome.ends_with("third") || outcome == "claude-new-ready" {
        let provider = if primary_provider == "claude" {
            "codex"
        } else {
            "claude"
        };
        let account = db.create_ai_account(provider, "Prepared third provider").unwrap();
        if provider == "claude" {
            crate::accounts::store_claude_token(account.id, "synthetic-third-claude").unwrap();
        } else {
            crate::accounts::store_codex_auth(account.id, r#"{"tokens":{"refresh_token":"synthetic-third-codex"}}"#)
                .unwrap();
        }
        Some(account)
    } else {
        None
    };
    let socket = root.join("runner.sock");
    let log = root.join("attempts.jsonl");
    let child = std::process::Command::new("python3")
        .args(["-u", "-c", include_str!("opencode_fake_runner.py")])
        .arg(&socket)
        .arg(&log)
        .arg(outcome)
        .arg(first.id.to_string())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    let mut runner = Runner(child);
    let mut ready = String::new();
    BufReader::new(runner.0.stdout.take().unwrap())
        .read_line(&mut ready)
        .unwrap();
    assert_eq!(ready.trim(), "ready");
    std::env::set_var("OCR_REVIEW_RUNNER_SOCKETS", &socket);
    let mut inputs = super::review_inputs("economic", 20, 5);
    inputs.run_id = run;
    inputs.pr_id = pr;
    inputs.repo_path = repo;
    inputs.full_name = "acme/fixture".into();
    inputs.offline = true;
    inputs.fixed_head_sha = Some(head);
    inputs.model = "claude-opus-5-5".into();
    inputs.skill_content = "Review correctness using frozen evidence.".into();
    inputs.db_handle = Some(handle);
    inputs.archive_dir = root.join("archives");
    if outcome.starts_with("claude-new-ready") || outcome == "claude-frozen-preferences" {
        prepared_account_case(
            &mut inputs,
            &db,
            &root,
            &log,
            first.id,
            second.id,
            third.as_ref().map(|a| a.id),
        );
        drop(runner);
        fs::remove_dir_all(root).unwrap();
        return;
    }
    let result = crate::review::execute(&inputs, &AtomicBool::new(false), &mut |_, _| {});
    let attempts: Vec<serde_json::Value> = fs::read_to_string(&log)
        .unwrap_or_default()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    if matches!(outcome, "removed" | "missing" | "discovery") {
        assert!(result.is_err(), "an unusable selected Go engine must stop preparation");
        let error = result.err().unwrap();
        assert!(
            error.contains(match outcome {
                "missing" => "Select an OpenCode Go model explicitly",
                "discovery" => "Synthetic Go catalogue unavailable",
                _ => "OpenCode Go model is no longer available",
            }),
            "{error}"
        );
        assert!(
            attempts.is_empty(),
            "invalid saved Go selection must not invoke any inference or failover"
        );
        assert_eq!(db.ai_account(first.id).unwrap().status, "ok");
        assert_eq!(db.ai_account(second.id).unwrap().status, "ok");
        drop(runner);
        fs::remove_dir_all(root).unwrap();
        println!("GO_FLOW {outcome}: selected Go preparation rejected before inference");
        return;
    }
    if outcome == "claude-failover-go" {
        result.expect("valid Go backup must finish the stage");
        assert_eq!(attempts.len(), 2);
        assert_eq!(attempts[0]["provider"], "claude");
        assert_eq!(attempts[1]["provider"], "opencode");
        assert_ne!(attempts[0]["model"], attempts[1]["model"]);
        let stages = db.review_stage_breakdown(run).unwrap().stages;
        assert!(!stages.is_empty());
        for stage in stages {
            assert_eq!(stage.model.as_deref(), Some("new-review-model"));
            assert_eq!((stage.input_tokens, stage.output_tokens), (160, 19));
        }
        let report = db
            .usage_report(&crate::db::UsageFilter {
                days: 7,
                tz_minutes: 0,
                repo_id: None,
                now: crate::db::now_millis(),
            })
            .unwrap();
        for (id, model) in [
            (first.id, attempts[0]["model"].as_str().unwrap()),
            (second.id, "new-review-model"),
        ] {
            assert!(report.by_account.iter().any(|row| row.key == id.to_string()));
            assert!(report
                .by_model
                .iter()
                .any(|row| row.key == super::super::usage::normalize_model(model)));
        }
        drop(runner);
        fs::remove_dir_all(root).unwrap();
        return;
    }
    if let Some(third) = third {
        result.expect("unavailable Go must not hide the prepared third provider");
        if primary_provider == "opencode" {
            assert_eq!(attempts.len(), 1);
            assert_eq!(attempts[0]["account"], third.id);
            assert_eq!(db.ai_account(first.id).unwrap().status, "ok");
            assert!(db.ai_account(first.id).unwrap().last_used_at.is_none());
        } else {
            assert_eq!(attempts.len(), 2);
            assert_eq!(attempts[0]["account"], first.id);
            assert_eq!(attempts[1]["account"], third.id);
            assert_eq!(db.ai_account(first.id).unwrap().status, "limited");
        }
        assert_eq!(db.ai_account(second.id).unwrap().status, "ok");
        assert!(db.ai_account(second.id).unwrap().last_used_at.is_none());
        assert!(db.ai_account(third.id).unwrap().last_used_at.is_some());
        drop(runner);
        fs::remove_dir_all(root).unwrap();
        return;
    }
    if primary_provider != "opencode" {
        assert_eq!(
            attempts.len(),
            1,
            "optional Go failure must not cause extra execution: {outcome}"
        );
        assert_eq!(attempts[0]["account"], first.id);
        assert_eq!(attempts[0]["provider"], primary_provider);
        assert_eq!(db.ai_account(second.id).unwrap().status, "ok");
        assert!(db.ai_account(second.id).unwrap().last_used_at.is_none());
        if outcome.contains("failover") {
            assert!(result.is_err(), "without an alternative, preserve the frozen Go error");
            assert_eq!(db.ai_account(first.id).unwrap().status, "limited");
            let message = result.err().unwrap();
            assert!(
                message.contains(if outcome.contains("discovery") {
                    "Synthetic Go catalogue unavailable"
                } else {
                    "Select an OpenCode Go model explicitly"
                }),
                "{message}"
            );
        } else {
            result.expect("Claude/Codex must work despite an unusable optional Go backup");
            assert_eq!(db.ai_account(first.id).unwrap().status, "ok");
            assert!(db.ai_account(first.id).unwrap().last_used_at.is_some());
        }
        drop(runner);
        fs::remove_dir_all(root).unwrap();
        println!("GO_FLOW {outcome}: only selected {primary_provider} executed; Go error stayed provider-local");
        return;
    }
    let expected = if matches!(outcome, "auth" | "limit") {
        vec![first.id, second.id]
    } else {
        vec![first.id]
    };
    assert_eq!(
        attempts
            .iter()
            .map(|entry| entry["account"].as_i64().unwrap())
            .collect::<Vec<_>>(),
        expected,
        "{outcome}: {:?}",
        result.as_ref().err()
    );
    assert!(!attempts.iter().any(|entry| entry["account"] == missing.id));
    let stages = db.review_stage_breakdown(run).unwrap().stages;
    assert!(!stages.is_empty(), "pipeline must persist stage attribution");
    for stage in &stages {
        assert_eq!(
            stage.model.as_deref(),
            Some("new-review-model"),
            "{outcome}: stage must not inherit legacy Claude model"
        );
    }
    for attempt in &attempts {
        assert_eq!(attempt["provider"], "opencode");
        assert_eq!(attempt["model"], "new-review-model");
        assert_eq!(attempt["effort"], "new-review-effort");
    }
    if matches!(outcome, "auth" | "limit") {
        let output = result.unwrap();
        assert_eq!(
            (
                output.usage.input_tokens,
                output.usage.output_tokens,
                output.usage.cache_read_tokens,
                output.usage.cache_creation_tokens
            ),
            (34, 19, 9, 3)
        );
        let primary = db.ai_account(first.id).unwrap();
        assert_eq!(
            primary.status,
            if outcome == "auth" { "auth_failed" } else { "limited" }
        );
        assert_eq!(primary.limited_until.is_some(), outcome == "limit");
        if let Some(until) = primary.limited_until {
            assert!(until > crate::db::now_millis());
            assert_eq!(db.next_ai_account_reset().unwrap(), Some(until));
        }
        assert!(primary.last_used_at.is_none());
        assert_eq!(db.ai_account(second.id).unwrap().status, "ok");
        assert!(db.ai_account(second.id).unwrap().last_used_at.is_some());
        assert_eq!(db.pick_ai_account(&[]).unwrap().unwrap().id, second.id);
        let report = db
            .usage_report(&crate::db::UsageFilter {
                days: 7,
                tz_minutes: 0,
                repo_id: None,
                now: crate::db::now_millis(),
            })
            .unwrap();
        for (id, input, output, read, write) in [(first.id, 14, 7, 3, 1), (second.id, 20, 12, 6, 2)] {
            let row = report
                .by_account
                .iter()
                .find(|row| row.key == id.to_string())
                .expect("Attempt must retain account attribution");
            assert_eq!(
                (
                    row.usage.input_tokens,
                    row.usage.output_tokens,
                    row.usage.cache_read_tokens,
                    row.usage.cache_creation_tokens
                ),
                (input, output, read, write)
            );
        }
    } else {
        assert!(result.is_err());
        assert_eq!(db.ai_account(first.id).unwrap().status, "ok");
        assert_eq!(db.ai_account(second.id).unwrap().status, "ok");
        assert!(db.ai_account(first.id).unwrap().limited_until.is_none());
        assert!(db.ai_account(first.id).unwrap().last_used_at.is_none());
        assert!(db.ai_account(second.id).unwrap().last_used_at.is_none());
        assert_eq!(db.pick_ai_account(&[]).unwrap().unwrap().id, first.id);
        let report = db
            .usage_report(&crate::db::UsageFilter {
                days: 7,
                tz_minutes: 0,
                repo_id: None,
                now: crate::db::now_millis(),
            })
            .unwrap();
        assert!(!report.by_account.iter().any(|row| row.key == second.id.to_string()));
        if outcome == "health" {
            let primary = report
                .by_account
                .iter()
                .find(|row| row.key == first.id.to_string())
                .expect("Usage before infrastructure failure must remain attributed");
            assert_eq!(
                (
                    primary.usage.input_tokens,
                    primary.usage.output_tokens,
                    primary.usage.cache_read_tokens,
                    primary.usage.cache_creation_tokens
                ),
                (14, 7, 3, 1)
            );
        } else {
            assert!(
                report.by_account.is_empty(),
                "Startup before inference reports no fabricated usage"
            );
        }
    }
    drop(runner);
    println!(
        "GO_FLOW {outcome}: {} runner execution(s); account status and token attribution passed",
        attempts.len()
    );
    fs::remove_dir_all(root).unwrap();
}
