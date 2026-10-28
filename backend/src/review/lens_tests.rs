use super::*;
use serde_json::json;

fn finding(path: &str, line: i64, title: &str) -> Value {
    json!({"kind": "finding", "path": path, "line": line, "side": "RIGHT", "title": title})
}

#[test]
fn merge_appends_only_new_findings_and_questions() {
    let first = json!({
        "reviewedFiles": ["a.rs"],
        "findings": [finding("a.rs", 10, "Off by one in the loop bound")],
        "questions": [],
        "missingEvidence": [],
    });
    let second = json!({
        "reviewedFiles": ["a.rs"],
        "findings": [
            finding("a.rs", 10, "Another way to say the bound is wrong"),
            finding("a.rs", 40, "off by one in the loop bound"),
            finding("a.rs", 22, "Test no longer asserts the empty case"),
            finding("b.rs", 10, "Off by one in the loop bound"),
        ],
        "questions": [{"title": "Is the retry meant to be unbounded?"}],
    });
    let merged = merge(first.clone(), &second);
    let titles = merged["findings"]
        .as_array()
        .unwrap()
        .iter()
        .map(|item| format!("{}:{}", item["path"].as_str().unwrap(), item["line"]))
        .collect::<Vec<_>>();
    assert_eq!(titles, ["a.rs:10", "a.rs:22", "b.rs:10"]);
    assert_eq!(merged["questions"].as_array().unwrap().len(), 1);
    assert_eq!(merged["reviewedFiles"], first["reviewedFiles"]);
    assert_eq!(merge(first.clone(), &first), first);
}

#[test]
fn merge_appends_only_new_suggestions() {
    let suggestion = |path: &str, line: i64, title: &str| json!({"path": path, "line": line, "title": title});
    let first = json!({"findings": [], "suggestions": [suggestion("a.rs", 3, "Stale test name")]});
    let second = json!({
        "findings": [],
        "suggestions": [suggestion("a.rs", 9, "stale test name"), suggestion("a.rs", 9, "No test covers the retry")],
    });
    let merged = merge(first, &second);
    let titles = merged["suggestions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|item| item["title"].as_str().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(titles, ["Stale test name", "No test covers the retry"]);
}

#[test]
fn lens_prompt_maps_to_the_runner_like_its_stage() {
    let root = std::env::temp_dir().join(format!("ocr-lens-{}", crate::sandbox::secret().unwrap()));
    let stage = root.join("chunks").join("001");
    let lens = stage.join("lens");
    fs::create_dir_all(lens.join("context")).unwrap();
    fs::write(lens.join("context").join("PR.md"), "pr").unwrap();
    let prompt = format!(
        "Read {stage}/context/PR.md. Write {stage}/report.json.",
        stage = stage.display()
    );
    fs::write(lens.join("prompt.md"), prompt_for(&prompt, &stage, &lens).unwrap()).unwrap();
    fs::write(lens.join("chunk_findings.json"), "{}").unwrap();
    let request = crate::sandbox::runner::stage_request(
        crate::providers::claude::MODEL,
        "v1",
        1,
        "claude",
        &root,
        &lens,
        &lens.join("context"),
        &lens.join("prompt.md"),
    )
    .unwrap();
    let crate::sandbox::runner::Request::Run { prompt, files, .. } = request else {
        panic!("not a run request");
    };
    assert!(
        prompt.starts_with("Read /work/context/PR.md. Write /output/report.json."),
        "{prompt}"
    );
    assert!(prompt.contains("/work/integration/chunk_findings.json"));
    assert!(files.contains_key("integration/chunk_findings.json"));
    fs::remove_dir_all(root).unwrap();
}
