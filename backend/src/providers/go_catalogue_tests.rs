use super::*;
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc, Barrier,
};

fn envelope(model: &Model) -> Vec<u8> {
    serde_json::to_vec(&json!({"object":"list","data":[model]})).unwrap()
}

#[test]
fn dynamic_ids_efforts_and_removal_require_no_source_edit_or_implicit_default() {
    let first = fixture(
        "newly-announced-model",
        &["novel-effort", "other-effort"],
        "chat-completions",
    );
    assert_eq!(first.models[0].default_effort(), "novel-effort");
    let selected = first.select("newly-announced-model", "novel-effort").unwrap();
    selected.validate("newly-announced-model", "novel-effort").unwrap();
    assert!(first.select("newly-announced-model", "").is_err());
    let removed = fixture("replacement-model", &[], "responses");
    assert!(removed.select("newly-announced-model", "novel-effort").is_err());
    assert!(removed.select("replacement-model", "").is_ok());
    assert_ne!(removed.id, first.id);
    let mut model = first.models[0].clone();
    model.capabilities.reasoning.default = Some(json!({"type":"effort","value":"other-effort"}));
    let catalogue = Catalogue::parse(&envelope(&model)).unwrap();
    assert_eq!(catalogue.models[0].default_effort(), "other-effort");
    model.capabilities.reasoning.default = Some(json!({"type":"effort","value":"not-advertised"}));
    assert!(Catalogue::parse(&envelope(&model)).is_err());
    model.capabilities.reasoning.default = None;
    model.capabilities.reasoning.effort = None;
    model.capabilities.reasoning.toggle = Some(true);
    model.capabilities.reasoning.budget = Some(Budget {
        min: Some(1024),
        max: Some(4096),
    });
    let catalogue = Catalogue::parse(&envelope(&model)).unwrap();
    assert!(catalogue.models[0].efforts().is_empty());
    assert_eq!(catalogue.models[0].default_effort(), "");
    assert!(catalogue.select(&model.id, "high").is_err());
    assert!(catalogue.select(&model.id, "").is_ok());
}

#[test]
fn snapshot_tampering_and_untrusted_transports_fail_independently() {
    for protocol in ["chat-completions", "responses", "messages", "generate-content"] {
        let catalogue = fixture("model-new", &["new-effort"], protocol);
        let snapshot = catalogue.select("model-new", "new-effort").unwrap();
        let cli = snapshot.cli_catalogue().unwrap();
        assert_eq!(
            cli["opencode-go"]["models"]["model-new"],
            serde_json::to_value(&snapshot.model).unwrap()
        );
        let mut bad = snapshot.clone();
        bad.model.name = "changed".into();
        assert!(bad.validate("model-new", "new-effort").is_err());
        let mut bad = snapshot.clone();
        bad.effort = "not-advertised".into();
        assert!(bad.validate("model-new", "not-advertised").is_err());
        for url in [
            "http://opencode.ai/inference/go/openai/v1",
            "https://opencode.ai.evil.test/v1",
            "https://user:pass@opencode.ai/inference/go/openai/v1",
            "https://opencode.ai:443/inference/go/openai/v1",
            "https://opencode.ai/inference/go/openai/v1?key=x",
            "https://opencode.ai/inference/go/openai/v1#fragment",
            "http://127.0.0.1:18081/v1",
        ] {
            let mut model = snapshot.model.clone();
            model.api.values_mut().next().unwrap().base_url = url.into();
            assert!(Catalogue::parse(&envelope(&model)).is_err());
        }
        let mut model = snapshot.model.clone();
        model.api.values_mut().next().unwrap().sdk.opencodeai = "evil-module".into();
        assert!(Catalogue::parse(&envelope(&model)).is_err());
    }
}

#[test]
fn duplicates_unsafe_selectors_depth_count_and_body_budgets_are_rejected() {
    let catalogue = fixture("new-id", &["new-effort"], "chat-completions");
    let mut model = catalogue.models[0].clone();
    for id in ["../model", "vendor/model", "id#effort", "id with spaces", ""] {
        model.id = id.into();
        assert!(Catalogue::parse(&envelope(&model)).is_err());
    }
    model = catalogue.models[0].clone();
    model.capabilities.reasoning.effort = Some(vec!["repeat".into(), "repeat".into()]);
    assert!(Catalogue::parse(&envelope(&model)).is_err());
    let data = vec![catalogue.models[0].clone(); 2];
    assert!(Catalogue::parse(&serde_json::to_vec(&json!({"object":"list","data":data})).unwrap()).is_err());
    assert!(Catalogue::parse(&vec![b' '; MAX_BYTES + 1]).is_err());
    assert!(Catalogue::parse(b"{not-json").is_err());
    let mut nested = Value::Null;
    for _ in 0..30 {
        nested = json!([nested]);
    }
    assert!(Catalogue::parse(&serde_json::to_vec(&nested).unwrap()).is_err());
    let mut data = Vec::new();
    for i in 0..513 {
        let mut m = catalogue.models[0].clone();
        m.id = format!("m-{i}");
        data.push(m);
    }
    assert!(Catalogue::parse(&serde_json::to_vec(&json!({"object":"list","data":data})).unwrap()).is_err());
}

#[test]
fn asynchronous_refresh_is_single_inflight_and_never_exposes_stale_models() {
    let calls = Arc::new(AtomicUsize::new(0));
    let entered = Arc::new(Barrier::new(2));
    let release = Arc::new(Barrier::new(2));
    let (c, e, r) = (calls.clone(), entered.clone(), release.clone());
    let discovery = Discovery::with_fetch(move || {
        c.fetch_add(1, Ordering::SeqCst);
        e.wait();
        r.wait();
        Err("synthetic discovery failure".into())
    });
    assert_eq!(discovery.view(None).unwrap()["catalogue"]["status"], "empty");
    discovery.install(fixture("old-model", &[], "responses"));
    discovery.queue(false).unwrap();
    entered.wait();
    for _ in 0..8 {
        discovery.queue(false).unwrap();
    }
    let loading = discovery.view(None).unwrap();
    assert_eq!(loading["catalogue"]["status"], "loading");
    assert_eq!(loading["models"], json!([]));
    release.wait();
    for _ in 0..1000 {
        if discovery.view(None).unwrap()["catalogue"]["status"] == "error" {
            break;
        }
        std::thread::sleep(Duration::from_millis(1));
    }
    let error = discovery.view(None).unwrap();
    assert_eq!(error["catalogue"]["status"], "error");
    assert_eq!(error["models"], json!([]));
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert!(discovery.with_current("stale-id", |_| Ok(())).is_err());
}

#[test]
fn http_client_rejects_redirects_oversize_and_timeouts_without_authentication() {
    use std::io::Write;
    use std::net::TcpListener;
    for response in [
        "HTTP/1.1 302 Found\r\nLocation: http://127.0.0.1:1/never\r\nContent-Length: 0\r\n\r\n",
        "HTTP/1.1 200 OK\r\nContent-Length: 2097153\r\n\r\n",
        "",
    ] {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/models", listener.local_addr().unwrap());
        let worker = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut bytes = [0; 4096];
            let count = stream.read(&mut bytes).unwrap();
            let request = String::from_utf8_lossy(&bytes[..count]).to_lowercase();
            assert!(!request.contains("authorization:") && !request.contains("x-api-key:"));
            if response.is_empty() {
                std::thread::sleep(Duration::from_millis(80));
            } else {
                stream.write_all(response.as_bytes()).unwrap();
            }
        });
        let client = reqwest::blocking::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_millis(30))
            .build()
            .unwrap();
        assert!(download_from(&client, &url).is_err());
        worker.join().unwrap();
    }
}
#[test]
fn advertised_reasoning_budget_is_independent_of_visible_output_limit() {
    let mut model = fixture("budget-model", &[], "messages").models.remove(0);
    model.limit.output = 4096;
    model.capabilities.reasoning.effort = None;
    model.capabilities.reasoning.budget = Some(Budget {
        min: Some(8192),
        max: Some(262144),
    });
    model.capabilities.reasoning.default = Some(json!({"type":"budget_tokens","value":65536}));
    let catalogue = Catalogue::parse(&envelope(&model)).unwrap();
    assert_eq!(
        catalogue.models[0].capabilities.reasoning.budget.as_ref().unwrap().max,
        Some(262144)
    );
    assert!(catalogue.models[0].efforts().is_empty());
    assert!(catalogue.select("budget-model", "").is_ok());
    assert!(catalogue.select("budget-model", "high").is_err());
    model.capabilities.reasoning.default = Some(json!({"type":"budget_tokens","value":262145}));
    assert!(Catalogue::parse(&envelope(&model)).is_err());
    model.capabilities.reasoning.default = None;
    model.capabilities.reasoning.budget = Some(Budget {
        min: Some(262145),
        max: Some(262144),
    });
    assert!(Catalogue::parse(&envelope(&model)).is_err());
}

#[test]
fn malformed_models_do_not_hide_supported_models_or_weaken_snapshots() {
    let stable = fixture("stable-model", &[], "responses");
    let mut original = serde_json::to_value(&stable.models[0]).unwrap();
    original["status"] = Value::Null;
    for (path, value) in [
        ("/id", json!("../unsafe")),
        ("/id", Value::Null),
        ("/api", Value::Null),
        ("/limit", json!({"context":65536,"output":8192,"new_field":true})),
        ("/status", json!("preview")),
        ("/capabilities/reasoning", Value::Null),
        ("/release_date", Value::Null),
        ("/api/responses/base_url", json!("https://opencode.ai/new-path")),
    ] {
        let mut unsupported = original.clone();
        unsupported["id"] = json!("unsupported-model");
        *unsupported
            .pointer_mut(path)
            .unwrap_or_else(|| panic!("missing {path}")) = value;
        let bytes = serde_json::to_vec(&json!({"object":"list","data":[unsupported,original]})).unwrap();
        let catalogue = Catalogue::parse(&bytes).unwrap();
        assert_eq!(catalogue.models, stable.models, "{path}");
        assert_eq!(catalogue.id, stable.id);
        assert!(catalogue.select("unsupported-model", "").is_err());
        let bytes = serde_json::to_vec(&json!({"object":"list","data":[unsupported]})).unwrap();
        assert!(Catalogue::parse(&bytes).is_err());
    }
    let snapshot = stable.select("stable-model", "").unwrap();
    let mut encoded = serde_json::to_value(&snapshot).unwrap();
    encoded["model"]["limit"]["new_field"] = json!(true);
    assert!(serde_json::from_value::<Snapshot>(encoded).is_err());
    let mut bad = snapshot;
    bad.model.status = Some("preview".into());
    bad.id = digest(&(bad.catalogue_id.as_str(), &bad.model, bad.effort.as_str())).unwrap();
    assert!(bad.validate("stable-model", "").is_err());
}

#[test]
fn ambiguous_multi_transport_model_does_not_hide_other_models() {
    let stable = fixture("single-route-model", &["new-effort"], "responses")
        .models
        .remove(0);
    let mut ambiguous = fixture("ambiguous-route-model", &[], "messages").models.remove(0);
    ambiguous.api.extend(stable.api.clone());
    let source = serde_json::to_vec(&json!({"object":"list","data":[ambiguous,stable]})).unwrap();
    let catalogue = Catalogue::parse(&source).unwrap();
    assert_eq!(catalogue.models.len(), 1);
    assert_eq!(catalogue.models[0].id, "single-route-model");
    assert!(catalogue.select("ambiguous-route-model", "").is_err());
}

#[test]
#[ignore = "queries the public Go catalogue; explicit network check, no inference"]
fn live_public_catalogue_matches_supported_schema() {
    let catalogue = super::download().expect("official Go discovery must parse without a fixed inventory");
    assert!(!catalogue.models.is_empty());
    for model in &catalogue.models {
        model.validate().unwrap();
        catalogue.select(&model.id, model.default_effort()).unwrap();
    }
}
