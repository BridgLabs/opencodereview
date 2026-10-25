use super::*;

#[test]
fn selection_target_is_not_an_artifact_or_model_window_limit() {
    assert!(validate_prepared_inputs(TARGET_CONTEXT_BYTES + 1, 4_000).is_ok());
    assert!(validate_prepared_inputs(MAX_EVIDENCE_BYTES, MAX_PROMPT_BYTES).is_ok());
    assert!(validate_prepared_inputs(MAX_EVIDENCE_BYTES + 1, 0)
        .unwrap_err()
        .contains("disk artifact limit"));
    assert!(validate_prepared_inputs(0, MAX_PROMPT_BYTES + 1)
        .unwrap_err()
        .contains("rendered prompt"));
}

#[test]
fn concurrent_workers_respect_the_global_limit() {
    let active = std::sync::atomic::AtomicUsize::new(0);
    let peak = std::sync::atomic::AtomicUsize::new(0);
    let cancel = AtomicBool::new(false);
    std::thread::scope(|scope| {
        for group in 10..18 {
            let (active, peak, cancel) = (&active, &peak, &cancel);
            scope.spawn(move || {
                let _permit = acquire(2, 2, cancel, group).unwrap();
                let current = active.fetch_add(1, Ordering::SeqCst) + 1;
                peak.fetch_max(current, Ordering::SeqCst);
                std::thread::sleep(Duration::from_millis(5));
                active.fetch_sub(1, Ordering::SeqCst);
            });
        }
    });
    assert!((1..=2).contains(&peak.load(Ordering::SeqCst)));
    assert_eq!(active.load(Ordering::SeqCst), 0);
}

#[test]
fn workers_respect_the_parallel_limit_of_one_pr() {
    let active = std::sync::atomic::AtomicUsize::new(0);
    let peak = std::sync::atomic::AtomicUsize::new(0);
    let cancel = AtomicBool::new(false);
    std::thread::scope(|scope| {
        for _ in 0..4 {
            let (active, peak, cancel) = (&active, &peak, &cancel);
            scope.spawn(move || {
                let _permit = acquire(3, 1, cancel, 90).unwrap();
                peak.fetch_max(active.fetch_add(1, Ordering::SeqCst) + 1, Ordering::SeqCst);
                std::thread::sleep(Duration::from_millis(5));
                active.fetch_sub(1, Ordering::SeqCst);
            });
        }
    });
    assert_eq!(peak.load(Ordering::SeqCst), 1);
    assert_eq!(active.load(Ordering::SeqCst), 0);
}

#[test]
fn queue_releases_cancelled_waiters_and_permits() {
    let cancel = AtomicBool::new(false);
    let permit = acquire(1, 1, &cancel, 1).unwrap();
    let cancelled = AtomicBool::new(true);
    assert!(acquire(1, 1, &cancelled, 2).is_err());
    drop(permit);
    assert!(acquire(1, 1, &cancel, 1).is_ok());
}
