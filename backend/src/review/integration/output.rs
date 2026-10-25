use super::prepare::BatchOutput;
use super::*;

pub(crate) fn validate_output(
    inputs: &ReviewInputs,
    stage: &Prepared,
    payload: &Payload,
    head: &str,
) -> Result<BatchOutput, String> {
    let summary = stage.directory.join("summary.md");
    let comments = stage.directory.join("comments.json");
    let evidence_warnings = frozen::materialize(
        inputs,
        &stage.directory.join("report.json"),
        &stage.context,
        &stage.files,
        head,
        &stage.diff,
        &summary,
        &comments,
    )?;
    let publication = crate::review::evidence::publication_diff(&stage.context, &stage.diff)?;
    let raw = require_valid_comments(
        &fs::read_to_string(&comments).map_err(|error| error.to_string())?,
        head,
        &publication,
    )?;
    let decisions =
        fs::read_to_string(stage.directory.join("finding_decisions.json")).map_err(|error| error.to_string())?;
    let decisions = crate::review::evidence::remap_decisions(&stage.directory.join("report.json"), &decisions, &raw)?;
    crate::review::findings::validate_decisions(&payload.sources_for(&stage.candidate_ids), Some(&decisions), &raw)?;
    frozen::decisions(inputs, &decisions, &stage.context)?;
    let comments: Value = serde_json::from_str(&raw).map_err(|error| error.to_string())?;
    let decisions: Value = serde_json::from_str(&decisions).map_err(|error| error.to_string())?;
    let entries = decisions["decisions"]
        .as_array()
        .ok_or("Missing integration decisions")?
        .clone();
    if entries.len() != stage.candidate_ids.len() {
        return Err("Unexpected integration decisions".into());
    }
    Ok(BatchOutput {
        evidence_warnings,
        files: stage.files.clone(),
        body: fs::read_to_string(summary).map_err(|error| error.to_string())?,
        comments: comments["comments"]
            .as_array()
            .ok_or("Missing integrated comments")?
            .clone(),
        decisions: entries,
        report: serde_json::from_str(
            &fs::read_to_string(stage.directory.join("report.json")).map_err(|error| error.to_string())?,
        )
        .map_err(|error| error.to_string())?,
    })
}

pub(crate) fn comment_key(comment: &Value) -> String {
    json!([comment["path"], comment["line"], comment["side"], comment["body"]]).to_string()
}

pub(crate) fn combine(
    inputs: &ReviewInputs,
    payload: &Payload,
    outputs: Vec<BatchOutput>,
    expected_files: &[String],
    head: &str,
    diff: &str,
    merge: &Path,
) -> Result<(String, String, Vec<crate::review::warnings::EvidenceWarning>), String> {
    let covered = outputs
        .iter()
        .flat_map(|output| output.files.iter())
        .collect::<BTreeSet<_>>();
    if outputs.is_empty() || covered != expected_files.iter().collect() {
        return Err("Incomplete final integration coverage".into());
    }
    let mut comments = Vec::new();
    let mut indices = BTreeMap::new();
    let mut decisions = Vec::new();
    let mut bodies = Vec::new();
    let mut reports = Vec::new();
    let mut evidence_warnings = Vec::new();
    for output in outputs {
        evidence_warnings.extend(output.evidence_warnings);
        let mut remap = Vec::new();
        for comment in output.comments {
            let key = comment_key(&comment);
            let index = *indices.entry(key).or_insert_with(|| {
                let index = comments.len();
                comments.push(comment);
                index
            });
            remap.push(index);
        }
        for mut decision in output.decisions {
            if decision["decision"] == "keep" {
                let local = decision["commentIndex"]
                    .as_u64()
                    .and_then(|index| usize::try_from(index).ok())
                    .ok_or("Invalid integrated comment index")?;
                decision["commentIndex"] =
                    json!(*remap.get(local).ok_or("Integrated comment index is out of range")?);
            }
            decisions.push(decision);
        }
        if is_structured_review_point(&output.body) && !bodies.contains(&output.body) {
            bodies.push(output.body);
        }
        reports.push(output.report);
    }
    let raw = require_valid_comments(&json!({"commit_id":head,"comments":comments}).to_string(), head, diff)?;
    let archive = inputs.frozen.as_ref().ok_or("Frozen integration archive missing")?;
    let decisions =
        json!({"contractVersion":2,"snapshotId":archive.archive.descriptor().id,"decisions":decisions}).to_string();
    crate::review::findings::validate_decisions(&payload.sources, Some(&decisions), &raw)?;
    let body = if bodies.is_empty() {
        if comments.is_empty() {
            "No findings.".into()
        } else {
            format!("Automated review produced {} inline comment(s).", comments.len())
        }
    } else {
        bodies.join("\n\n")
    };
    validate_review_templates(&body, Some(&raw))?;
    fs::create_dir_all(merge).map_err(|error| error.to_string())?;
    fs::write(merge.join("finding_decisions.json"), decisions).map_err(|error| error.to_string())?;
    fs::write(merge.join("report.json"), json!({"batches":reports}).to_string()).map_err(|error| error.to_string())?;
    Ok((body, raw, crate::review::warnings::merge(evidence_warnings)))
}
