use super::*;

#[test]
fn queued_is_not_running_and_cache_does_not_invent_execution_time() {
    let mut chunk = ChunkProgress::queued(1, 3, 10);
    assert!(chunk.started_at.is_none());
    chunk.finish(true, None, 20);
    assert_eq!(chunk.status, "cached");
    assert!(chunk.started_at.is_none());
    assert_eq!(chunk.finished_at, Some(20));
}

#[test]
fn validation_preserves_the_last_execution_output() {
    let mut chunk = ChunkProgress::queued(10, 5, 10);
    let phase = r#"[{"id":"chunks","status":"running"}]"#;
    chunk.observe(
        &execution_phase(phase, "chunks", "running", 20),
        &format!("{}Last reviewer output", "é".repeat(2200)),
        20,
    );
    let validating = execution_phase(phase, "chunks", "validating", 30);
    chunk.observe(&validating, "Validating chunk coverage, evidence and anchors", 30);
    assert!(chunk.log.contains("Last reviewer output"));
    assert!(chunk.log.contains("Validating chunk coverage, evidence and anchors"));
    assert_eq!(chunk.log.chars().count(), 2048);
    let log = chunk.log.clone();
    chunk.observe(&validating, "Validating chunk coverage, evidence and anchors", 31);
    assert_eq!(chunk.log, log);
    chunk.finish(false, Some("Incomplete review"), 32);
    assert_eq!(chunk.log, log);
    assert_eq!(chunk.status, "failed");
}

#[test]
fn records_start_only_after_the_process_event_and_keeps_cancellation_distinct() {
    let mut chunk = ChunkProgress::queued(1, 3, 10);
    let phase = r#"[{"id":"chunks","status":"running"}]"#;
    chunk.observe(phase, "Agent says running", 20);
    assert_eq!(chunk.status, "queued");
    chunk.observe(&execution_phase(phase, "chunks", "running", 30), "Started", 130);
    assert_eq!(chunk.started_at, Some(30));
    chunk.observe(&execution_phase(phase, "chunks", "validating", 40), "Validating", 40);
    assert_eq!(chunk.execution_finished_at, Some(40));
    chunk.finish(false, Some("cancelled"), 45);
    assert_eq!(chunk.status, "cancelled");
    chunk.observe(&execution_phase(phase, "chunks", "running", 30), "Late event", 50);
    assert_eq!(chunk.status, "cancelled");
}
