use super::*;
use serde_json::{json, Value};
use std::collections::{BTreeSet, VecDeque};

mod output;
mod prepare;

pub(super) use output::{combine, validate_output};
pub(super) use prepare::{prepare, scoped_diff};

const BATCH_INSTRUCTIONS: &str = "\nThis is one final-integration batch of the same complete PR review. Other batches cover the remaining assigned paths and candidates; do not interpret their absence here as missing implementation. Review exactly the paths in this batch's STAGE.json, while consulting the same full frozen archive for cross-file contracts and counter-evidence. Each candidate keeps its original candidateId. A candidate may contain its original structured point instead of repeated rendered Markdown; preserve its title, details, action and evidence when reasoning. Chunk notes retain contextQuestions, general questions and coverage metadata; their reviewedFiles are context, not your assigned coverage. Resolve every provided candidate explicitly. Do not invent decisions for candidates assigned to another batch.\n";

pub(super) struct Payload {
    pub sources: Value,
    candidates: BTreeMap<String, Value>,
    notes: Vec<(Vec<String>, Value)>,
}

impl Payload {
    pub fn empty(head: &str) -> Self {
        Self {
            sources: json!({"commit_id":head,"comments":[]}),
            candidates: BTreeMap::new(),
            notes: vec![],
        }
    }

    pub fn from_results(results: &[ChunkResult], sources: Value) -> Result<Self, String> {
        let mut payload = Self {
            sources,
            candidates: BTreeMap::new(),
            notes: vec![],
        };
        let originals = payload.sources["comments"]
            .as_array()
            .ok_or("Missing integration candidates")?;
        for result in results {
            let report = serde_json::from_str::<Value>(&result.summary).ok();
            let candidates = originals
                .iter()
                .filter(|candidate| candidate["chunkId"].as_u64() == Some(result.chunk.id as u64))
                .collect::<Vec<_>>();
            let points = report.as_ref().and_then(|report| report["findings"].as_array());
            let structured = points.is_some_and(|points| {
                points.len() == candidates.len()
                    && points.iter().zip(&candidates).all(|(point, candidate)| {
                        ["path", "line", "side"]
                            .iter()
                            .all(|key| point[*key] == candidate["comment"][*key])
                    })
            });
            for (index, candidate) in candidates.iter().enumerate() {
                let id = candidate["candidateId"].as_str().ok_or("Missing candidate identity")?;
                let input = if structured {
                    json!({"candidateId":id,"chunkId":result.chunk.id,"point":points.unwrap()[index]})
                } else {
                    (*candidate).clone()
                };
                if payload.candidates.insert(id.into(), input).is_some() {
                    return Err("Duplicate integration candidate identity".into());
                }
            }
            let note = if structured {
                let mut note = report.unwrap();
                note.as_object_mut().ok_or("Invalid chunk report")?.remove("findings");
                json!({"chunkId":result.chunk.id,"report":note})
            } else {
                json!({"chunkId":result.chunk.id,"summary":result.summary})
            };
            payload.notes.push((result.chunk.files.clone(), note));
        }
        if payload.candidates.len() != originals.len() {
            return Err("Integration candidates lack their source chunks".into());
        }
        Ok(payload)
    }

    fn originals(&self) -> &[Value] {
        self.sources["comments"].as_array().map(Vec::as_slice).unwrap_or(&[])
    }

    fn sources_for(&self, ids: &[String]) -> Value {
        let ids = ids.iter().map(String::as_str).collect::<BTreeSet<_>>();
        json!({"commit_id":self.sources["commit_id"],"comments":self.originals().iter().filter(|candidate| candidate["candidateId"].as_str().is_some_and(|id| ids.contains(id))).collect::<Vec<_>>()})
    }
}

pub(super) struct Prepared {
    pub directory: PathBuf,
    pub context: PathBuf,
    pub prompt: PathBuf,
    pub files: Vec<String>,
    pub diff: String,
    pub candidate_ids: Vec<String>,
}

#[derive(Clone)]
struct Task {
    files: Vec<String>,
    ids: Vec<String>,
}

enum Input {
    File(PathBuf),
    Text(String),
    Source(String),
}

#[cfg(test)]
mod tests;
