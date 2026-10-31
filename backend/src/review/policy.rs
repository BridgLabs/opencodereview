use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Condvar, Mutex, OnceLock};
use std::time::Duration;

pub const TARGET_CONTEXT_BYTES: usize = 160_000;
pub const MAX_EVIDENCE_BYTES: usize = 8 * 1024 * 1024;
pub const MAX_PROMPT_BYTES: usize = 200_000;
/// Upper bound on runner replicas; `docker-compose.yml` starts `OCR_REVIEW_RUNNERS` of them.
pub const MAX_RUNNERS: usize = 64;
/// Model processes running at once across reviews; each also needs a free runner replica.
pub const MAX_PROCESSES: usize = MAX_RUNNERS;
/// Chunks one review runs at once, so a large pull request leaves runners for other reviews.
pub const MAX_PROCESSES_PER_REVIEW: usize = 6;

pub fn validate_prepared_inputs(evidence_bytes: usize, prompt_bytes: usize) -> Result<(), String> {
    if evidence_bytes > MAX_EVIDENCE_BYTES {
        return Err(format!("Incomplete review: prepared evidence needs {evidence_bytes} bytes; disk artifact limit is {MAX_EVIDENCE_BYTES}. This is not the model context window. No model execution started for this attempt."));
    }
    if prompt_bytes > MAX_PROMPT_BYTES {
        return Err(format!("Incomplete review: rendered prompt needs {prompt_bytes} bytes; prompt safety limit is {MAX_PROMPT_BYTES}. No model execution started for this attempt."));
    }
    Ok(())
}

#[derive(Default)]
struct Slots {
    active: usize,
    next: u64,
    waiting: VecDeque<(u64, usize, i64, usize)>,
    active_by_group: HashMap<i64, usize>,
    last_group: Option<i64>,
    limits: Vec<usize>,
}

fn slots() -> &'static (Mutex<Slots>, Condvar) {
    static SLOTS: OnceLock<(Mutex<Slots>, Condvar)> = OnceLock::new();
    SLOTS.get_or_init(|| (Mutex::new(Slots::default()), Condvar::new()))
}

pub struct ProcessPermit(usize, i64);

impl Drop for ProcessPermit {
    fn drop(&mut self) {
        let (lock, changed) = slots();
        if let Ok(mut state) = lock.lock() {
            state.active = state.active.saturating_sub(1);
            if let Some(active) = state.active_by_group.get_mut(&self.1) {
                *active = active.saturating_sub(1);
                if *active == 0 {
                    state.active_by_group.remove(&self.1);
                }
            }
            if let Some(index) = state.limits.iter().position(|limit| *limit == self.0) {
                state.limits.remove(index);
            }
            changed.notify_all();
        }
    }
}

pub fn acquire(limit: usize, group_limit: usize, cancel: &AtomicBool, group: i64) -> Result<ProcessPermit, String> {
    let (lock, changed) = slots();
    let mut state = lock.lock().map_err(|_| "Review process queue unavailable")?;
    let id = state.next;
    state.next += 1;
    let limit = limit.clamp(1, 8);
    state.waiting.push_back((id, limit, group, group_limit.clamp(1, 20)));
    loop {
        if cancel.load(Ordering::Relaxed) {
            state.waiting.retain(|(ticket, _, _, _)| *ticket != id);
            changed.notify_all();
            return Err("cancelled".into());
        }
        let effective_limit = state
            .limits
            .iter()
            .copied()
            .chain(state.waiting.iter().map(|(_, limit, _, _)| *limit))
            .min()
            .unwrap_or(limit);
        let next = state
            .waiting
            .iter()
            .enumerate()
            .filter(|(_, (_, _, group, cap))| state.active_by_group.get(group).copied().unwrap_or(0) < *cap)
            .min_by_key(|(_, (ticket, _, group, _))| (Some(*group) == state.last_group, *ticket))
            .map(|(index, _)| index);
        if let Some(next) = next.filter(|index| state.waiting[*index].0 == id && state.active < effective_limit) {
            state.waiting.remove(next);
            state.last_group = Some(group);
            state.active += 1;
            *state.active_by_group.entry(group).or_default() += 1;
            state.limits.push(limit);
            changed.notify_all();
            return Ok(ProcessPermit(limit, group));
        }
        state = changed
            .wait_timeout(state, Duration::from_millis(100))
            .map_err(|_| "Review process queue unavailable")?
            .0;
    }
}

#[cfg(test)]
#[path = "policy_tests.rs"]
mod tests;
