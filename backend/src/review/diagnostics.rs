use super::*;
use crate::review::warnings::EvidenceWarning;

pub(super) fn restore_chunk(
    chunk: &ReviewChunk,
    directory: &Path,
    validation: Result<Vec<EvidenceWarning>, String>,
    cached: bool,
    usage: RunUsage,
) -> Result<ChunkResult, String> {
    let evidence_warnings = validation?;
    Ok(ChunkResult {
        chunk: chunk.clone(),
        summary: fs::read_to_string(directory.join("report.json")).map_err(|error| error.to_string())?,
        comments_json: Some(fs::read_to_string(directory.join("comments.json")).map_err(|error| error.to_string())?),
        evidence_warnings,
        cached,
        usage,
    })
}

pub(super) fn retain_chunk_warnings(output: &mut ReviewOutput, chunks: &[ChunkResult]) {
    output.evidence_warnings = crate::review::warnings::merge(
        chunks
            .iter()
            .flat_map(|chunk| chunk.evidence_warnings.clone())
            .chain(std::mem::take(&mut output.evidence_warnings)),
    );
    apply_evidence_warning_risk(&mut output.risk, &output.evidence_warnings);
}

pub(super) fn finish_audit(
    inputs: &ReviewInputs,
    root: &Path,
    body: &str,
    comments: &str,
    diff: &str,
    evidence_warnings: &[EvidenceWarning],
) -> Result<crate::review::findings::FindingAudit, String> {
    let db = inputs.open_db()?;
    let source = db
        .load_finding_audit(inputs.run_id)?
        .ok_or("Finding audit seed unavailable")?;
    let decisions = fs::read_to_string(root.join("merge/finding_decisions.json")).map_err(|error| error.to_string())?;
    let mut audit =
        crate::review::findings::finish_audit(source, Some(comments), Some(comments), body, Some(&decisions), diff);
    audit.evidence_warnings = Some(crate::review::warnings::merge(
        audit
            .evidence_warnings
            .take()
            .unwrap_or_default()
            .into_iter()
            .chain(evidence_warnings.iter().cloned()),
    ));
    if !audit.complete
        || audit
            .candidates
            .iter()
            .any(|candidate| candidate.origin == "chunk" && candidate.ai_decision == "unmapped")
    {
        return Err("Diagnostic audit did not resolve all source candidates".into());
    }
    db.store_finding_audit(inputs.run_id, &audit)?;
    Ok(audit)
}
