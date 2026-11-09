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

#[test]
fn stage_activity_counts_round_trips_tool_calls_and_reasoning_of_each_cli() {
    let activity =
        |lines: &[&str]| super::usage::collect_activity(&super::transcript_events(&lines.join("\n")).unwrap());
    // Claude: two blocks of one message are one round trip; the result carries the API time.
    let claude = activity(&[
        r#"{"type":"assistant","message":{"id":"m1","content":[{"type":"thinking","thinking":"..."}]}}"#,
        r#"{"type":"assistant","message":{"id":"m1","content":[{"type":"tool_use","name":"Read"},{"type":"tool_use","name":"Grep"}]}}"#,
        r#"{"type":"user","message":{"content":[{"type":"tool_result"}]}}"#,
        r#"{"type":"assistant","message":{"id":"m2","content":[{"type":"text","text":"{}"}]}}"#,
        r#"{"type":"result","subtype":"success","num_turns":3,"duration_ms":9000,"duration_api_ms":7000}"#,
    ]);
    assert_eq!(
        claude,
        super::StageActivity {
            turns: 2,
            tool_calls: 2,
            reasoning_tokens: 0,
            model_ms: 7000
        }
    );
    // Codex reports commands and, in newer versions, reasoning tokens, but no round trips.
    let codex = activity(&[
        r#"{"type":"item.completed","item":{"type":"reasoning"}}"#,
        r#"{"type":"item.completed","item":{"type":"command_execution","aggregated_output":""}}"#,
        r#"{"type":"item.completed","item":{"type":"agent_message"}}"#,
        r#"{"type":"turn.completed","usage":{"input_tokens":10,"cached_input_tokens":0,"output_tokens":5,"reasoning_output_tokens":3}}"#,
    ]);
    assert_eq!(
        codex,
        super::StageActivity {
            turns: 0,
            tool_calls: 1,
            reasoning_tokens: 3,
            model_ms: 0
        }
    );
    let opencode = activity(&[
        r#"{"type":"step_start","timestamp":1000}"#,
        r#"{"type":"tool_use","timestamp":1500,"part":{"state":{"input":{"command":"ls"}}}}"#,
        r#"{"type":"step_finish","timestamp":3000,"part":{"tokens":{"input":1,"output":1,"reasoning":50,"cache":{"read":0,"write":0}}}}"#,
        r#"{"type":"step_start","timestamp":3400}"#,
        r#"{"type":"step_finish","timestamp":4000,"part":{"tokens":{"input":1,"output":1,"reasoning":20,"cache":{"read":0,"write":0}}}}"#,
    ]);
    assert_eq!(
        opencode,
        super::StageActivity {
            turns: 2,
            tool_calls: 1,
            reasoning_tokens: 70,
            model_ms: 2600
        }
    );
    // Retries and failovers of a stage add up.
    let total = super::paths::sum_usage(vec![
        super::RunUsage {
            activity: claude,
            ..Default::default()
        },
        super::RunUsage {
            activity: opencode,
            ..Default::default()
        },
    ]);
    assert_eq!(total.activity.turns, 4);
    assert_eq!(total.activity.model_ms, 9600);
}
