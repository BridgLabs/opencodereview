use super::*;
use std::fs;

#[test]
fn snapshot_commands_allow_bounded_reads_searches_and_listings() {
    let root = std::env::temp_dir().join(format!("ocr-read-{}", crate::sandbox::secret().unwrap()));
    fs::create_dir_all(&root).unwrap();
    let file = root.join("source file.rs");
    fs::write(&file, "fn example() {}\n").unwrap();
    for command in [
        format!("/usr/bin/sed -n '1,120p' -- '{}'", file.display()),
        format!(
            "/usr/bin/grep -r -n -F -- 'fn example' '{}' | /usr/bin/head -n 80",
            root.display()
        ),
        format!("/usr/bin/ls -1A -- '{}'", root.display()),
    ] {
        validate(&command, &[root.as_path()]).unwrap();
    }
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn snapshot_commands_reject_shell_execution_and_unbounded_reads() {
    let root = std::env::temp_dir();
    for command in [
        "cat /etc/passwd",
        "/usr/bin/sed -n '1,201p' -- '/tmp/a'",
        "/usr/bin/sed -n '0,20p' -- '/tmp/a'",
        "/usr/bin/sed -n '20,1p' -- '/tmp/a'",
        "/usr/bin/sed -n '1,$p' -- '/tmp/a'",
        "/usr/bin/sed -n '1,20p; e id' -- '/tmp/a'",
        "/usr/bin/sed -n '1,20p' -- '/tmp/a'; id",
        "/usr/bin/ls -1A -- '/tmp' > /tmp/output",
        "/usr/bin/ls -1A -- '/tmp' && id",
        "/usr/bin/ls -1A -- '/tmp'$(id)'",
        "/usr/bin/grep -r -n -F -- '' '/tmp' | /usr/bin/head -n 80",
        "/usr/bin/grep -r -n -F -- 'x' '/tmp' | sh",
        "env PATH=/tmp /usr/bin/ls -1A -- '/tmp'",
    ] {
        assert!(validate(command, &[root.as_path()]).is_err(), "{command}");
    }
}

#[test]
fn snapshot_commands_reject_relative_line_ranges() {
    let root = std::env::temp_dir();
    let file = root.join(format!("ocr-read-{}", crate::sandbox::secret().unwrap()));
    fs::write(&file, "value\n").unwrap();
    for range in ["200,+201p", "+1,2p"] {
        let command = format!("/usr/bin/sed -n '{range}' -- '{}'", file.display());
        assert!(validate(&command, &[root.as_path()]).is_err(), "{command}");
    }
    fs::remove_file(file).unwrap();
}

#[test]
fn snapshot_commands_reject_outside_paths_and_symlink_escapes() {
    let root = std::env::temp_dir().join(format!("ocr-read-{}", crate::sandbox::secret().unwrap()));
    let allowed = root.join("allowed");
    fs::create_dir_all(&allowed).unwrap();
    fs::write(root.join("private"), "not evidence").unwrap();
    for path in [root.join("private"), allowed.join("../private")] {
        assert!(validate(
            &format!("/usr/bin/sed -n '1,20p' -- '{}'", path.display()),
            &[allowed.as_path()]
        )
        .is_err());
    }
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(root.join("private"), allowed.join("link")).unwrap();
        assert!(validate(
            &format!("/usr/bin/sed -n '1,20p' -- '{}'", allowed.join("link").display()),
            &[allowed.as_path()]
        )
        .is_err());
    }
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn snapshot_commands_reject_environment_and_interactive_overrides() {
    let root = std::env::temp_dir();
    let command = format!("/usr/bin/ls -1A -- '{}'", root.display());
    let base =
        serde_json::json!({"command":command,"description":"List snapshot","timeout":10000,"run_in_background":false});
    validate_input(&base, &[root.as_path()]).unwrap();
    for (key, value) in [
        ("run_in_background", serde_json::json!(true)),
        ("dangerouslyDisableSandbox", serde_json::json!(true)),
        ("timeout", serde_json::json!(0)),
        ("timeout", serde_json::json!(30001)),
        ("unexpected", serde_json::json!(true)),
    ] {
        let mut input = base.clone();
        input[key] = value;
        assert!(validate_input(&input, &[root.as_path()]).is_err(), "{key}");
    }
}

#[test]
fn snapshot_commands_handle_single_quotes_without_allowing_shell_fragments() {
    let root = std::env::temp_dir().join(format!("ocr-read-{}", crate::sandbox::secret().unwrap()));
    fs::create_dir_all(&root).unwrap();
    let file = root.join("a'b.rs");
    fs::write(&file, "value\n").unwrap();
    let quoted = file.to_str().unwrap().replace('\'', "'\\''");
    let command = format!("/usr/bin/sed -n '1,10p' -- '{quoted}'");
    validate(&command, &[root.as_path()]).unwrap();
    assert!(validate(&format!("{command}; touch /tmp/unexpected"), &[root.as_path()]).is_err());
    fs::remove_dir_all(root).unwrap();
}

#[cfg(target_os = "linux")]
#[test]
fn snapshot_commands_read_real_files_without_evaluating_the_query() {
    let root = std::env::temp_dir().join(format!("ocr-read-{}", crate::sandbox::secret().unwrap()));
    fs::create_dir_all(&root).unwrap();
    fs::write(root.join("source.rs"), "first\n$(touch forbidden)\nlast\n").unwrap();
    let commands = [
        (
            format!("/usr/bin/sed -n '2,2p' -- '{}/source.rs'", root.display()),
            "$(touch forbidden)",
        ),
        (
            format!(
                "/usr/bin/grep -r -n -F -- '$(touch forbidden)' '{}' | /usr/bin/head -n 80",
                root.display()
            ),
            ":2:$(touch forbidden)",
        ),
        (format!("/usr/bin/ls -1A -- '{}'", root.display()), "source.rs"),
    ];
    for (command, expected) in commands {
        validate(&command, &[root.as_path()]).unwrap();
        let output = std::process::Command::new("/bin/bash")
            .args(["-c", &command])
            .current_dir(&root)
            .output()
            .unwrap();
        assert!(output.status.success());
        assert!(String::from_utf8(output.stdout).unwrap().contains(expected));
        assert!(!root.join("forbidden").exists());
    }
    fs::remove_dir_all(root).unwrap();
}

#[cfg(target_os = "linux")]
#[test]
#[ignore = "Requires explicit authorization for one short model execution in a disposable runner"]
fn real_cli_reads_snapshot_with_the_production_tool_guard() {
    use super::super::{final_response, inference, private_dir, read_output, session, terminate};
    use std::os::unix::fs::PermissionsExt;
    use std::time::{Duration, Instant};
    assert_eq!(std::env::var("OCR_SNAPSHOT_READ_PROBE_ALLOWED").as_deref(), Ok("1"));
    let root = Path::new("/diagnostic");
    assert!(
        !root.join("execution-started").exists(),
        "This probe cannot be replayed"
    );
    private_dir(Path::new("/work/context"), 0o755).unwrap();
    let expected = crate::sandbox::secret().unwrap();
    fs::write("/work/context/read-probe.txt", &expected).unwrap();
    fs::set_permissions("/work/context/read-probe.txt", fs::Permissions::from_mode(0o444)).unwrap();
    let command = "/usr/bin/sed -n '1,1p' -- '/work/context/read-probe.txt'";
    let prompt = format!("{INSTRUCTIONS}\nUse Bash once with command {command}. The file contains an unknown value. Return ONLY {{\"report\":{{\"value\":\"the exact file contents\"}}}} after reading it. Do not call other tools, write files, or guess the value.");
    fs::write("/work/prompt.md", prompt).unwrap();
    fs::set_permissions("/work/prompt.md", fs::Permissions::from_mode(0o444)).unwrap();
    let model = std::env::var("OCR_DIAGNOSTIC_MODEL").unwrap();
    let account = std::env::var("OCR_DIAGNOSTIC_ACCOUNT").unwrap().parse().unwrap();
    let mut run = session(account).unwrap();
    inference(&mut run, &model, crate::providers::claude::EFFORT).unwrap();
    fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(root.join("execution-started"))
        .unwrap();
    run.stdin(fs::File::open("/work/prompt.md").unwrap())
        .stdout(fs::File::create(root.join("transcript.jsonl")).unwrap())
        .stderr(fs::File::create(root.join("stderr.txt")).unwrap());
    let mut child = run.spawn().unwrap();
    let start = Instant::now();
    let successful = loop {
        if start.elapsed() > Duration::from_secs(90) {
            break false;
        }
        if let Some(status) = child.try_wait().unwrap() {
            break status.success();
        }
        std::thread::sleep(Duration::from_millis(100));
    };
    terminate(&mut child).unwrap();
    let raw = read_output(&root.join("transcript.jsonl"), 4_000_000).unwrap().unwrap();
    let events = crate::review::transcript_events(&raw).unwrap();
    let calls = events
        .as_array()
        .unwrap()
        .iter()
        .filter(|event| event["type"] == "assistant")
        .flat_map(|event| event["message"]["content"].as_array().into_iter().flatten())
        .filter(|part| part["type"] == "tool_use")
        .collect::<Vec<_>>();
    let (report, _) = final_response(Some(&raw)).unwrap();
    let report: serde_json::Value = serde_json::from_str(&report).unwrap();
    assert!(successful && calls.iter().any(|call| call["name"] == "Bash"));
    assert_eq!(report["value"], expected);
    let usage = crate::review::parse_transcript_usage(&root.join("transcript.jsonl"));
    println!(
        "SNAPSHOT_READ_VALIDATED {}",
        serde_json::json!({"model":model,"toolCalls":calls.len(),"inputTokens":usage.input_tokens,"outputTokens":usage.output_tokens,"productionGuard":true})
    );
}
