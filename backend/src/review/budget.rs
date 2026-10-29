//! A review's time budget. A benchmark gives every review a fixed time, so a review with a budget
//! starts optional work (the lens pass, an output repair, a final integration batch) only while the
//! work it still requires fits before the deadline. Without a budget, as in the product, every
//! step runs. The estimates come from the stages this review already finished.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Room kept for estimates that run short and for validation after the last execution.
const MARGIN: Duration = Duration::from_secs(120);
/// What a review stage is expected to take before any has finished.
const FIRST_STAGE: Duration = Duration::from_secs(300);
/// Final integration takes about this much of a chunk review's time, per chunk.
const FINAL_PER_CHUNK: f64 = 0.8;
const LENS_PER_STAGE: f64 = 0.9;
const REPAIR_PER_STAGE: f64 = 0.4;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Step {
    Lens,
    Repair,
    FinalBatch,
}

pub struct Budget {
    deadline: Instant,
    state: Mutex<State>,
    /// The validated comments of each chunk that finished, for a review the deadline cancels.
    validated: Mutex<Vec<String>>,
}

#[derive(Default)]
struct State {
    chunks: usize,
    parallel: usize,
    reviewed: Vec<Duration>,
    batches: Vec<Duration>,
    in_final: bool,
}

impl State {
    fn stage(&self) -> Duration {
        mean(&self.reviewed).unwrap_or(FIRST_STAGE)
    }

    fn batch(&self) -> Duration {
        mean(&self.batches).unwrap_or_else(|| self.stage().mul_f64(FINAL_PER_CHUNK))
    }

    /// The work the review still requires after the step being decided. A repair is decided while
    /// its stage is still being validated, before `reviewed` records it, so that stage is not pending.
    fn required(&self, step: Step) -> Duration {
        if self.in_final {
            return Duration::ZERO;
        }
        let current = usize::from(step == Step::Repair);
        let pending = self.chunks.saturating_sub(self.reviewed.len() + current) as f64;
        let chunks = self.stage().mul_f64(pending / self.parallel.max(1) as f64);
        let integration = if self.chunks > 1 {
            self.stage().mul_f64(FINAL_PER_CHUNK * self.chunks as f64)
        } else {
            Duration::ZERO
        };
        chunks + integration
    }

    fn cost(&self, step: Step) -> Duration {
        match step {
            Step::Lens => self.stage().mul_f64(LENS_PER_STAGE),
            Step::Repair if self.in_final => self.batch().mul_f64(REPAIR_PER_STAGE),
            Step::Repair => self.stage().mul_f64(REPAIR_PER_STAGE),
            Step::FinalBatch => self.batch(),
        }
    }
}

fn mean(durations: &[Duration]) -> Option<Duration> {
    (!durations.is_empty()).then(|| durations.iter().sum::<Duration>() / durations.len() as u32)
}

impl Budget {
    pub fn new(time: Duration) -> Arc<Self> {
        Self::until(Instant::now() + time)
    }

    pub(crate) fn until(deadline: Instant) -> Arc<Self> {
        Arc::new(Self {
            deadline,
            state: Mutex::new(State::default()),
            validated: Mutex::new(Vec::new()),
        })
    }

    fn with<T>(&self, update: impl FnOnce(&mut State) -> T) -> T {
        update(&mut self.state.lock().unwrap_or_else(|poisoned| poisoned.into_inner()))
    }

    /// The review runs `chunks` stages, `parallel` at a time; one stage counts as one chunk.
    pub(crate) fn plan(&self, chunks: usize, parallel: usize) {
        self.with(|state| {
            state.chunks = chunks;
            state.parallel = parallel;
        });
    }

    /// A stage's review (its first execution and any repair) took `took`.
    pub(crate) fn reviewed(&self, took: Duration) {
        self.with(|state| state.reviewed.push(took));
    }

    /// Final integration started: the chunk reviews are all done.
    pub(crate) fn integrating(&self) {
        self.with(|state| state.in_final = true);
    }

    pub(crate) fn batch_done(&self, took: Duration) {
        self.with(|state| state.batches.push(took));
    }

    /// Keeps a finished chunk's validated comments (`{"comments": [...]}`).
    pub(crate) fn validated(&self, comments: &str) {
        self.validated
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .push(comments.to_string());
    }

    /// The validated comments of every chunk that finished, in the order they finished.
    pub fn finished_chunks(&self) -> Vec<String> {
        self.validated
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }

    pub(crate) fn allows(&self, step: Step) -> bool {
        self.allows_at(step, Instant::now())
    }

    fn allows_at(&self, step: Step, now: Instant) -> bool {
        let needed = self.with(|state| state.cost(step) + state.required(step));
        now + needed + MARGIN <= self.deadline
    }
}

/// Whether `inputs` may start `step`: always, unless its budget says the time is needed elsewhere.
pub(crate) fn allows(inputs: &super::ReviewInputs, step: Step) -> bool {
    inputs.budget.as_ref().is_none_or(|budget| budget.allows(step))
}

pub(crate) fn with(inputs: &super::ReviewInputs, update: impl FnOnce(&Budget)) {
    if let Some(budget) = &inputs.budget {
        update(budget);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MINUTE: Duration = Duration::from_secs(60);

    #[test]
    fn optional_work_runs_while_the_required_work_still_fits() {
        let now = Instant::now();
        let budget = Budget::until(now + 60 * MINUTE);
        budget.plan(6, 1);
        budget.reviewed(5 * MINUTE);
        // 5 chunks left (25 min) and final integration (24 min) leave no room for a 4.5 min lens.
        assert!(!budget.allows_at(Step::Lens, now + 5 * MINUTE));
        assert!(budget.allows_at(Step::Repair, now));
        let roomy = Budget::until(now + 120 * MINUTE);
        roomy.plan(6, 1);
        roomy.reviewed(5 * MINUTE);
        assert!(roomy.allows_at(Step::Lens, now + 5 * MINUTE));
        // Runners side by side review the remaining chunks in less time.
        let parallel = Budget::until(now + 60 * MINUTE);
        parallel.plan(6, 5);
        parallel.reviewed(5 * MINUTE);
        assert!(parallel.allows_at(Step::Lens, now + 5 * MINUTE));
    }

    #[test]
    fn final_batches_run_while_one_more_fits() {
        let now = Instant::now();
        let budget = Budget::until(now + 30 * MINUTE);
        budget.plan(4, 1);
        for _ in 0..4 {
            budget.reviewed(5 * MINUTE);
        }
        budget.integrating();
        budget.batch_done(3 * MINUTE);
        assert!(budget.allows_at(Step::FinalBatch, now + 24 * MINUTE));
        assert!(!budget.allows_at(Step::FinalBatch, now + 26 * MINUTE));
    }

    #[test]
    fn a_repair_does_not_reserve_time_for_its_own_stage_again() {
        let now = Instant::now();
        // A single-stage review validated 9 minutes in, with 7 minutes left: a
        // repair (~2 min with the first-stage estimate) fits, and no other stage is pending.
        let budget = Budget::until(now + 16 * MINUTE);
        budget.plan(1, 1);
        assert!(budget.allows_at(Step::Repair, now + 9 * MINUTE));
        // Two stages: the other one still is.
        let two = Budget::until(now + 16 * MINUTE);
        two.plan(2, 1);
        assert!(!two.allows_at(Step::Repair, now + 9 * MINUTE));
    }

    #[test]
    fn a_single_stage_review_reserves_no_integration() {
        let now = Instant::now();
        let budget = Budget::until(now + 10 * MINUTE);
        budget.plan(1, 1);
        budget.reviewed(3 * MINUTE);
        assert!(budget.allows_at(Step::Lens, now + 3 * MINUTE));
        assert!(!budget.allows_at(Step::Lens, now + 6 * MINUTE));
    }
}
