use super::mcp::*;
use super::*;
use serde_json::{json, Value};

fn no_tools(_: &str, _: &Value) -> Result<Value, String> {
    panic!("no tool call expected")
}

#[test]
fn parses_pull_request_urls_and_short_references() {
    for (reference, repository, number) in [
        ("https://github.com/acme/api/pull/12", "acme/api", 12),
        ("https://github.com/acme/api/pull/12/files?w=1", "acme/api", 12),
        ("https://ghe.example.com/acme/api/pull/7#discussion", "acme/api", 7),
        ("github.com/acme/api/pull/3", "acme/api", 3),
        ("acme/api#45", "acme/api", 45),
        (" Acme/API#1 ", "Acme/API", 1),
    ] {
        assert_eq!(
            parse_pull_request_reference(reference).unwrap(),
            (repository.to_string(), number),
            "{reference}"
        );
    }
    for reference in [
        "",
        "acme/api",
        "acme#1",
        "acme/api#0",
        "acme/api#x",
        "https://github.com/acme/api/issues/1",
    ] {
        assert!(parse_pull_request_reference(reference).is_err(), "{reference}");
    }
}

#[test]
fn initialize_negotiates_the_protocol_version() {
    let reply = |version: &str| {
        dispatch(
            &json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":version}}),
            &no_tools,
        )
        .unwrap()
    };
    assert_eq!(reply("2025-03-26")["result"]["protocolVersion"], "2025-03-26");
    assert_eq!(reply("1999-01-01")["result"]["protocolVersion"], "2025-11-25");
    let result = &reply("2025-06-18")["result"];
    assert_eq!(result["serverInfo"]["name"], "opencodereview");
    assert!(result["capabilities"]["tools"].is_object());
}

#[test]
fn notifications_get_no_reply_and_unknown_methods_fail() {
    assert!(dispatch(
        &json!({"jsonrpc":"2.0","method":"notifications/initialized"}),
        &no_tools
    )
    .is_none());
    let unknown = dispatch(&json!({"jsonrpc":"2.0","id":"a","method":"resources/list"}), &no_tools).unwrap();
    assert_eq!(unknown["id"], "a");
    assert_eq!(unknown["error"]["code"], -32601);
    let batch = dispatch(&json!([{"jsonrpc":"2.0","id":1,"method":"ping"}]), &no_tools).unwrap();
    assert_eq!(batch["error"]["code"], -32600);
}

#[test]
fn lists_read_only_tools_and_reports_tool_errors_in_the_result() {
    let list = dispatch(&json!({"jsonrpc":"2.0","id":1,"method":"tools/list"}), &no_tools).unwrap();
    let names: Vec<&str> = list["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|tool| tool["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["get_review_status", "list_active_reviews"]);
    assert!(list["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .all(|tool| tool["annotations"]["readOnlyHint"] == true));

    let call = |name: &str| json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":name,"arguments":{}}});
    let failing = dispatch(&call("get_review_status"), &|_, _| Err("not tracked".into())).unwrap();
    assert_eq!(failing["result"]["isError"], true);
    assert!(failing["result"]["content"][0]["text"]
        .as_str()
        .unwrap()
        .contains("not tracked"));
    let ok = dispatch(&call("list_active_reviews"), &|_, _| Ok(json!({"reviews":[]}))).unwrap();
    assert_eq!(ok["result"]["isError"], false);
    assert_eq!(ok["result"]["structuredContent"], json!({"reviews":[]}));
    let unknown = dispatch(&call("start_review"), &no_tools).unwrap();
    assert_eq!(unknown["error"]["code"], -32602);
}

struct Fixture {
    state: SharedState,
    db: Database,
}

impl Fixture {
    fn new() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let root = std::env::temp_dir().join(format!(
            "ocr-mcp-{}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
            NEXT.fetch_add(1, Ordering::Relaxed),
        ));
        let db = crate::db::Db::for_test_at(root.join("test.sqlite")).open().unwrap();
        db.ensure_temp_repo_for_index(1, "acme/api").unwrap();
        let state = Arc::new(ServerState::new(ServerConfig {
            bind: "127.0.0.1:0".into(),
            db: crate::db::Db::for_test_at(root.join("test.sqlite")),
            data_dir: root.to_path_buf(),
            repos_dir: root.join("repos"),
            auto_reindex: false,
            poll_interval_seconds: 300,
            api_token: Some("token".into()),
        }));
        Self { state, db }
    }

    fn pull_request(&self, number: i64, lane: &str) -> i64 {
        self.db
            .upsert_pull_request(&crate::db::PullRequestInput {
                repo_id: 1,
                number,
                title: format!("PR {number}"),
                author: "alice".into(),
                author_avatar_url: None,
                is_draft: false,
                head_sha: "head".into(),
                base_sha: None,
                base_ref: Some("main".into()),
                head_ref: None,
                default_branch: Some("main".into()),
                url: Some(format!("https://github.com/acme/api/pull/{number}")),
                lane: lane.into(),
                status_reason: None,
            })
            .unwrap()
    }

    fn status(&self, reference: &str) -> Result<Value, String> {
        let message = json!({
            "jsonrpc": "2.0", "id": 1, "method": "tools/call",
            "params": { "name": "get_review_status", "arguments": { "pullRequest": reference } }
        });
        let response = handle_mcp(&self.request(&message), &self.state);
        assert_eq!(response.status, 200);
        let reply: Value = serde_json::from_slice(&response.body).unwrap();
        let result = &reply["result"];
        if result["isError"] == true {
            return Err(result["content"][0]["text"].as_str().unwrap().to_string());
        }
        Ok(result["structuredContent"].clone())
    }

    fn request(&self, message: &Value) -> Request {
        Request {
            method: "POST".into(),
            path: "/mcp".into(),
            query: String::new(),
            origin: None,
            authorization: Some("Bearer token".into()),
            content_type: Some("application/json".into()),
            flow_secret: None,
            body: serde_json::to_vec(message).unwrap(),
        }
    }
}

#[test]
fn reports_each_stage_of_a_review() {
    let fixture = Fixture::new();
    let error = fixture.status("acme/api#1").unwrap_err();
    assert!(error.contains("not tracked"), "{error}");

    let pr = fixture.pull_request(1, "queued");
    let queued = fixture.status("https://github.com/ACME/api/pull/1").unwrap();
    assert_eq!(queued["state"], "queued");
    assert_eq!(queued["done"], false);
    assert_eq!(queued["retryAfterSeconds"], 30);

    let run = fixture.db.start_review_skeleton_with(pr, Some("model")).unwrap();
    fixture
        .db
        .set_review_run_running(run, &review::index_phase_json("running"))
        .unwrap();
    let running = fixture.status("acme/api#1").unwrap();
    assert_eq!(running["state"], "reviewing");
    assert_eq!(running["done"], false);
    assert_eq!(running["run"]["id"], run);
    assert_eq!(running["run"]["currentHead"], true);
    assert!(
        running["summary"].as_str().unwrap().starts_with("Reviewing:"),
        "{running}"
    );

    fixture.db.fail_review_run(run, "runner exited").unwrap();
    let failed = fixture.status("acme/api#1").unwrap();
    assert_eq!(failed["state"], "failed");
    assert_eq!(failed["done"], true);
    assert_eq!(failed["run"]["error"], "runner exited");
    assert!(failed["summary"].as_str().unwrap().contains("runner exited"));
    assert!(failed.get("retryAfterSeconds").is_none());
}

#[test]
fn lists_only_active_reviews() {
    let fixture = Fixture::new();
    fixture.pull_request(1, "queued");
    fixture.pull_request(2, "open");
    let running = fixture.pull_request(3, "open");
    let run = fixture.db.start_review_skeleton_with(running, Some("model")).unwrap();
    fixture
        .db
        .set_review_run_running(run, &review::index_phase_json("running"))
        .unwrap();
    let message = json!({
        "jsonrpc": "2.0", "id": 1, "method": "tools/call",
        "params": { "name": "list_active_reviews", "arguments": { "repository": "acme/api" } }
    });
    let response = handle_mcp(&fixture.request(&message), &fixture.state);
    let reply: Value = serde_json::from_slice(&response.body).unwrap();
    let mut numbers: Vec<i64> = reply["result"]["structuredContent"]["reviews"]
        .as_array()
        .unwrap()
        .iter()
        .map(|review| review["pullRequest"]["number"].as_i64().unwrap())
        .collect();
    numbers.sort();
    assert_eq!(numbers, [1, 3]);
}

#[test]
fn notifications_are_accepted_without_a_body_and_get_is_rejected() {
    let fixture = Fixture::new();
    let notification = fixture.request(&json!({"jsonrpc":"2.0","method":"notifications/initialized"}));
    let response = handle_mcp(&notification, &fixture.state);
    assert_eq!(response.status, 202);
    assert!(response.body.is_empty());
    let mut get = fixture.request(&json!({}));
    get.method = "GET".into();
    assert_eq!(handle_mcp(&get, &fixture.state).status, 405);
}

fn finished_run(status: &str, phase_json: Option<&str>) -> crate::db::ReviewRun {
    serde_json::from_value(json!({
        "id": 9, "prId": 1, "status": status, "headSha": "head", "reviewBody": null,
        "commentsJson": null, "riskScore": null, "riskLevel": null, "riskJson": null,
        "errorMessage": "comment post failed", "phaseJson": phase_json, "tail": null,
        "startedAt": 1, "finishedAt": 2, "createdAt": 1, "inputTokens": 0, "outputTokens": 0,
        "cacheReadTokens": 0, "cacheCreationTokens": 0, "usageModel": null, "model": null,
        "contextIndexSha": null, "reviewedHeadSha": null
    }))
    .unwrap()
}

fn open_pull_request() -> crate::db::ReviewStatusPullRequest {
    crate::db::ReviewStatusPullRequest {
        id: 1,
        repository: "acme/api".into(),
        number: 1,
        title: "PR".into(),
        url: None,
        state: "open".into(),
        review_lane: "open".into(),
        status_reason: None,
        head_sha: "head".into(),
        last_synced_at: 1,
    }
}

#[test]
fn maps_every_finished_run_status() {
    let pr = open_pull_request();
    for (status, state) in [
        ("approved", "reviewed"),
        ("result_failed", "failed"),
        ("failed", "failed"),
        ("posted", "posted"),
        ("superseded", "skipped"),
        ("succeeded", "finished"),
    ] {
        let value = review_status(&pr, Some(&finished_run(status, None)), None, 3);
        assert_eq!(value["state"], state, "{status}");
        assert_eq!(value["done"], true, "{status}");
    }
    let approved = finished_run("approved", None);
    let posted_result: crate::db::ReviewResult = serde_json::from_value(json!({
        "id": 1, "prId": 1, "installationId": "i", "markerKey": "m", "githubCommentId": 5,
        "runId": 9, "actionableRunId": null, "observedHeadSha": "head", "reviewedHeadSha": "head",
        "priorResultState": null, "priorResultHeadSha": null, "observation": "clean",
        "effectiveState": "clean", "publicationStatus": "posted", "body": null, "bodyHash": null,
        "githubReviewId": null, "githubReviewUrl": null, "reviewerVersion": "v", "skillVersion": "v",
        "errorMessage": null, "createdAt": 1, "updatedAt": 2, "postedAt": 2
    }))
    .unwrap();
    assert_eq!(
        review_status(&pr, Some(&approved), Some(&posted_result), 3)["state"],
        "posted"
    );
    let failed = review_status(&pr, Some(&finished_run("result_failed", None)), None, 3);
    assert!(failed["summary"].as_str().unwrap().contains("comment post failed"));
}

#[test]
fn counts_cancelled_chunks_as_finished() {
    let phases = r#"[{"id":"chunks","label":"Review chunks","status":"failed","chunks":[
        {"id":1,"status":"failed"},{"id":2,"status":"cancelled"},{"id":3,"status":"done"}]}]"#;
    let value = review_status(
        &open_pull_request(),
        Some(&finished_run("failed", Some(phases))),
        None,
        3,
    );
    let chunks = &value["run"]["stages"][0]["chunks"];
    assert_eq!(chunks["total"], 3);
    assert_eq!(chunks["done"], 3);
    assert_eq!(chunks["failed"], 1);
    assert_eq!(chunks["cancelled"], 1);
}
