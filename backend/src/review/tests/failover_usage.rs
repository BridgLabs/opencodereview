use super::frozen::next_failover_transcript;
use super::parse_transcript_usage;
use std::fs;

fn codex_turn(input: i64, output: i64) -> String {
    format!("{{\"type\":\"turn.completed\",\"usage\":{{\"input_tokens\":{input},\"cached_input_tokens\":0,\"output_tokens\":{output}}}}}\n")
}

/// A stage whose output is repaired calls `execute` again with a fresh failover count. Its
/// failovers continue the numbering instead of overwriting the first execution's, and the stage
/// total reads every one of them, however many there were.
#[test]
fn failover_transcripts_never_overwrite_and_all_count_toward_the_stage() {
    let directory = std::env::temp_dir().join(format!("ocr-failover-usage-{}", crate::sandbox::secret().unwrap()));
    fs::create_dir_all(&directory).unwrap();
    let transcript = directory.join("transcript.json");
    for attempt in 1..=10 {
        fs::write(&transcript, codex_turn(attempt, 1)).unwrap();
        let failover = next_failover_transcript(&transcript);
        assert_eq!(failover, directory.join(format!("transcript.failover-{attempt}.json")));
        fs::rename(&transcript, failover).unwrap();
    }
    fs::write(&transcript, codex_turn(100, 1)).unwrap();
    let usage = parse_transcript_usage(&transcript);
    assert_eq!(
        (usage.input_tokens, usage.output_tokens),
        ((1..=10).sum::<i64>() + 100, 11)
    );
    fs::remove_dir_all(&directory).unwrap();
}
