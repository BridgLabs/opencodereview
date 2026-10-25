use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChunkProgress {
    pub id: usize,
    pub status: String,
    pub queued_at: i64,
    pub started_at: Option<i64>,
    pub execution_finished_at: Option<i64>,
    pub finished_at: Option<i64>,
    pub file_count: usize,
    pub cached: bool,
    pub attempts: usize,
    pub log: String,
    pub error: Option<String>,
}

impl ChunkProgress {
    pub fn queued(id: usize, file_count: usize, now: i64) -> Self {
        Self {
            id,
            status: "queued".into(),
            queued_at: now,
            started_at: None,
            execution_finished_at: None,
            finished_at: None,
            file_count,
            cached: false,
            attempts: 0,
            log: String::new(),
            error: None,
        }
    }

    pub fn observe(&mut self, phase: &str, tail: &str, now: i64) {
        if self.finished_at.is_some() {
            return;
        }
        let phases: Vec<serde_json::Value> = serde_json::from_str(phase).unwrap_or_default();
        if let Some((state, at)) = phases.iter().find_map(|phase| {
            phase
                .get("executionState")
                .and_then(|value| value.as_str())
                .map(|state| (state, phase["executionAt"].as_i64().unwrap_or(now)))
        }) {
            match state {
                "queued" => self.status = "queued".into(),
                "running" => {
                    self.status = "running".into();
                    self.started_at.get_or_insert(at);
                    self.execution_finished_at = None;
                    self.attempts += 1;
                }
                "validating" => {
                    self.status = "validating".into();
                    self.execution_finished_at = Some(at);
                }
                _ => {}
            }
        }
        let output = if self.status == "validating" && !self.log.is_empty() && !self.log.ends_with(tail) {
            format!("{}\n\n{tail}", self.log)
        } else if self.status == "validating" && self.log.ends_with(tail) {
            self.log.clone()
        } else {
            tail.to_string()
        };
        self.log = output
            .chars()
            .rev()
            .take(2048)
            .collect::<String>()
            .chars()
            .rev()
            .collect();
    }

    pub fn finish(&mut self, cached: bool, error: Option<&str>, now: i64) {
        self.cached = cached;
        self.status = match error {
            Some("cancelled") => "cancelled",
            Some(_) => "failed",
            None if cached => "cached",
            None => "done",
        }
        .into();
        self.error = error.map(|error| error.chars().take(1000).collect());
        self.finished_at = Some(now);
    }
}

pub fn execution_phase(raw: &str, id: &str, state: &str, at: i64) -> String {
    let mut phases: Vec<serde_json::Value> = serde_json::from_str(raw).unwrap_or_default();
    if let Some(phase) = phases.iter_mut().find(|phase| phase["id"] == id) {
        phase["executionState"] = state.into();
        phase["executionAt"] = at.into();
        if id != "chunks" {
            phase["status"] = if state == "queued" { "queued" } else { "running" }.into();
        }
    }
    serde_json::to_string(&phases).unwrap_or_else(|_| raw.into())
}

pub fn with_chunks(raw: &str, chunks: &BTreeMap<usize, ChunkProgress>) -> String {
    let mut phases: Vec<serde_json::Value> = serde_json::from_str(raw).unwrap_or_default();
    if let Some(phase) = phases.iter_mut().find(|phase| phase["id"] == "chunks") {
        phase["chunks"] = serde_json::to_value(chunks.values().collect::<Vec<_>>()).unwrap_or_default();
    }
    serde_json::to_string(&phases).unwrap_or_else(|_| raw.into())
}

#[cfg(test)]
#[path = "progress_tests.rs"]
mod tests;
