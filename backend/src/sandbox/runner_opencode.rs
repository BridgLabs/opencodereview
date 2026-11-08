use super::*;

fn complete_prefix(raw: &mut String) {
    raw.truncate(raw.rfind('\n').map_or(0, |end| end + 1));
}

pub(super) fn read_failed_transcript() -> Result<Option<String>, String> {
    let Some(mut bytes) = super::read_output_bytes(Path::new(TRANSCRIPT), 32_000_000)? else {
        return Ok(None);
    };
    // Recover only a failed OpenCode execution, before decoding its incomplete tail.
    bytes.truncate(bytes.iter().rposition(|byte| *byte == b'\n').map_or(0, |end| end + 1));
    let text = String::from_utf8(bytes).map_err(|_| "Runner output is not UTF8")?;
    fs::write(TRANSCRIPT, &text).map_err(|_| "OpenCode output recovery failed")?;
    Ok(Some(text))
}

fn inspect_json(text: &str) -> Result<Option<serde_json::Value>, ()> {
    match serde_json::from_str(text) {
        Ok(value) => Ok(Some(value)),
        // serde_json's pinned recursion bound is a safety failure, not plaintext.
        Err(error) if error.to_string().starts_with("recursion limit exceeded") => Err(()),
        Err(_) => Ok(None),
    }
}

fn escaped_ascii_contains(text: &str, secrets: &[String]) -> Result<bool, ()> {
    let mut inspected = text.as_bytes().to_vec();
    for _ in 0..64 {
        let mut normalized = Vec::with_capacity(inspected.len());
        let mut index = 0;
        let mut changed = false;
        while index < inspected.len() {
            if inspected[index..].starts_with(b"\\u") {
                let escape = inspected.get(index + 2..index + 6).ok_or(())?;
                let hex = std::str::from_utf8(escape).map_err(|_| ())?;
                if !escape.iter().all(u8::is_ascii_hexdigit) {
                    return Err(());
                }
                let code = u16::from_str_radix(hex, 16).map_err(|_| ())?;
                if code <= 0x7f {
                    normalized.push(code as u8);
                    index += 6;
                    changed = true;
                    continue;
                }
            }
            normalized.push(inspected[index]);
            index += 1;
        }
        let decoded = std::str::from_utf8(&normalized).map_err(|_| ())?;
        if secrets.iter().any(|secret| decoded.contains(secret)) {
            return Ok(true);
        }
        if !changed {
            return Ok(false);
        }
        inspected = normalized;
    }
    Err(())
}

fn redact_text(text: &str, secrets: &[String], depth: usize) -> Result<(String, bool), ()> {
    if depth > 64 {
        return Err(());
    }
    fn value(node: &mut serde_json::Value, secrets: &[String], depth: usize) -> Result<bool, ()> {
        if depth > 64 {
            return Err(());
        }
        let mut found = false;
        match node {
            serde_json::Value::String(text) => {
                let (clean, changed) = redact_text(text, secrets, depth + 1)?;
                *text = clean;
                found = changed;
            }
            serde_json::Value::Array(items) => {
                for item in items {
                    found |= value(item, secrets, depth + 1)?;
                }
            }
            serde_json::Value::Object(items) => {
                let mut clean = serde_json::Map::new();
                for (key, mut item) in std::mem::take(items) {
                    let (key, changed) = redact_text(&key, secrets, depth + 1)?;
                    found |= changed | value(&mut item, secrets, depth + 1)?;
                    clean.insert(key, item);
                }
                *items = clean;
            }
            _ => {}
        }
        Ok(found)
    }
    let mut clean = String::new();
    let mut found = false;
    let mut source = text.to_string();
    for secret in secrets {
        if source.contains(secret) {
            found = true;
            source = source.replace(secret, "[redacted]");
        }
    }
    if let Some(mut parsed) = inspect_json(&source)? {
        found |= value(&mut parsed, secrets, depth)?;
        let mut clean = serde_json::to_string(&parsed).map_err(|_| ())?;
        if source.ends_with('\n') {
            clean.push('\n');
        }
        return Ok((if found { clean } else { text.to_string() }, found));
    }
    for line in source.split_inclusive('\n') {
        if let Some(mut parsed) = inspect_json(line)? {
            found |= value(&mut parsed, secrets, depth)?;
            clean.push_str(&serde_json::to_string(&parsed).map_err(|_| ())?);
            if line.ends_with('\n') {
                clean.push('\n');
            }
        } else {
            let mut line = line.to_string();
            // Detection only: never normalize evidence bytes. Each pass shrinks ASCII
            // escapes; malformed escapes or exhausted inspection fail closed.
            found |= escaped_ascii_contains(&line, secrets)?;
            for secret in secrets {
                let mut escaped = secret.clone();
                while !escaped.is_empty() && escaped.len() <= line.len() {
                    if line.contains(&escaped) {
                        found = true;
                        line = line.replace(&escaped, "[redacted]");
                    }
                    let next = serde_json::to_string(&escaped).map_err(|_| ())?;
                    let next = next[1..next.len() - 1].to_string();
                    if next == escaped {
                        break;
                    }
                    escaped = next;
                }
            }
            clean.push_str(&line);
        }
    }
    Ok((if found { clean } else { text.to_string() }, found))
}

/// Let the pinned CLI close its private-server scope before the existing forced cleanup.
/// Killing the client first bypasses its finalizers and can strand a detached server/helper.
pub(super) fn terminate(child: &mut std::process::Child) -> Result<(), String> {
    unsafe {
        libc::kill(-(child.id() as i32), libc::SIGTERM);
    }
    let deadline = Instant::now() + Duration::from_millis(3500);
    while Instant::now() < deadline {
        if child
            .try_wait()
            .map_err(|_| "OpenCode child supervision failed")?
            .is_some()
        {
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    super::terminate(child)
}

pub(super) struct Session {
    catalogue: PathBuf,
    contract: PathBuf,
    snapshot: crate::providers::go_catalogue::Snapshot,
    key: String,
    home: PathBuf,
    state: PathBuf,
    offset: u64,
    pending: Vec<u8>,
    pub(super) exit_code: Option<i32>,
    pub(super) health: crate::sandbox::opencode_health::Health,
}

impl Session {
    pub(super) fn new(account: i64, snapshot: &crate::providers::go_catalogue::Snapshot) -> Result<Self, String> {
        snapshot.validate(&snapshot.model.id, &snapshot.effort)?;
        let root = Path::new(SESSION);
        private_dir(root, 0o755)?;
        let catalogue = root.join("go-models.json");
        let contract = root.join("go-contract.json");
        for (path, value) in [
            (&catalogue, snapshot.cli_catalogue()?),
            (
                &contract,
                serde_json::to_value(snapshot).map_err(|_| "Go catalogue serialization failed")?,
            ),
        ] {
            let bytes = serde_json::to_vec(&value).map_err(|_| "Go catalogue serialization failed")?;
            if bytes.len() > crate::providers::go_catalogue::MAX_BYTES {
                return Err("Go catalogue exceeds its budget".into());
            }
            let mut file = fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o444)
                .open(path)
                .map_err(|_| "Go catalogue could not be installed")?;
            file.write_all(&bytes)
                .and_then(|_| file.sync_all())
                .map_err(|_| "Go catalogue could not be persisted")?;
            file.set_permissions(fs::Permissions::from_mode(0o444))
                .map_err(|_| "Go catalogue permissions failed")?;
            let mut probe = Command::new("/bin/sh");
            probe
                .args([
                    "-c",
                    "test -f \"$1\" && test -r \"$1\" && ! test -w \"$1\" && ! test -w \"$2\"",
                    "go-catalogue-probe",
                ])
                .arg(path)
                .arg(root)
                .uid(UID)
                .gid(UID);
            bounded(&mut probe, Path::new("/scratch"), &AtomicBool::new(false), 10)
                .map_err(|_| "Go catalogue is not immutable for model UID")?;
        }
        let auth = std::env::var("OCR_RUNNER_AUTH_ROOT").unwrap_or_else(|_| "/claude-auth/app-managed".into());
        let path = Path::new(&auth).join(format!("accounts/{account}/opencode/go-api-key"));
        let file = crate::review_archive::open_archive(&path, false)
            .map_err(|_| "OpenCode projected credential unavailable")?;
        if !file
            .metadata()
            .map_err(|_| "OpenCode credential unavailable")?
            .is_file()
        {
            return Err("OpenCode credential unavailable".into());
        }
        let mut key = String::new();
        file.take(4097)
            .read_to_string(&mut key)
            .map_err(|_| "OpenCode credential unavailable")?;
        crate::accounts::validate_opencode_key(&key).map_err(|_| "OpenCode projected credential invalid")?;
        let home = root.join("opencode-home");
        let state = root.join("opencode-state");
        for path in [&home, &home.join(".config"), &home.join(".config/opencode")] {
            private_dir(path, 0o555)?;
        }
        private_dir(&state, 0o700)?;
        std::os::unix::fs::chown(&state, Some(UID), Some(UID)).map_err(|_| "OpenCode state unavailable")?;
        private_dir(&state.join("tmp"), 0o700)?;
        std::os::unix::fs::chown(state.join("tmp"), Some(UID), Some(UID)).map_err(|_| "OpenCode state unavailable")?;
        let health = crate::sandbox::opencode_health::Health::new(root, UID)?;
        let session = Self {
            catalogue,
            contract,
            snapshot: snapshot.clone(),
            key,
            home,
            state,
            health,
            offset: 0,
            pending: Vec::new(),
            exit_code: None,
        };
        let mut version = session.base_command();
        version.arg("--version");
        let output = bounded(&mut version, Path::new("/scratch"), &AtomicBool::new(false), 15)?;
        if String::from_utf8_lossy(&output).trim() != format!("opencode v{}", crate::providers::opencode::VERSION) {
            return Err("OpenCode CLI version mismatch".into());
        }
        Ok(session)
    }

    fn base_command(&self) -> Command {
        let mut command = Command::new(crate::providers::opencode::CLI);
        let proxy = std::env::var("OCR_RUNNER_PROXY").unwrap_or_else(|_| "http://review-egress:8790".into());
        command
            .env_clear()
            .env("PATH", "/usr/local/bin:/usr/bin:/bin")
            .env("HOME", &self.home)
            .env("XDG_CONFIG_HOME", self.home.join(".config"))
            .env("XDG_DATA_HOME", self.state.join("data"))
            .env("XDG_STATE_HOME", self.state.join("state"))
            .env("XDG_CACHE_HOME", self.state.join("cache"))
            .env("TMPDIR", self.state.join("tmp"))
            .env("HTTPS_PROXY", &proxy)
            .env("HTTP_PROXY", &proxy)
            .env("OCR_OPENCODE_GO_KEY", &self.key)
            .env("OCR_OPENCODE_GUARD_SOCKET", &self.health.path)
            .env("OCR_OPENCODE_GUARD_NONCE", &self.health.nonce)
            .env("OPENCODE_CONFIG_PROJECT_DISABLE", "1")
            .env("OPENCODE_DISABLE_CLAUDE_CODE", "1")
            .env("OPENCODE_DISABLE_MODELS_FETCH", "1")
            .env("OPENCODE_MODELS_PATH", &self.catalogue)
            .env("OCR_OPENCODE_CATALOGUE_CONTRACT", &self.contract)
            .env("OPENCODE_FILEWATCHER_DISABLE", "1")
            .env(
                "OPENCODE_CONFIG_CONTENT",
                r#"{"providers":{"opencode-go":{"settings":{"apiKey":"{env:OCR_OPENCODE_GO_KEY}"}}}}"#,
            )
            .current_dir("/output")
            .uid(UID)
            .gid(UID)
            .process_group(0);
        command
    }

    pub(super) fn command(&self, model: &str, effort: &str) -> Result<Command, String> {
        self.snapshot.validate(model, effort)?;
        let mut command = self.base_command();
        command.args(crate::providers::opencode::exec_args(model, effort)?);
        Ok(command)
    }

    pub(super) fn failure(&mut self) -> Result<Option<String>, String> {
        use std::io::{Seek, SeekFrom};
        let mut file = fs::File::open(TRANSCRIPT).map_err(|_| "OpenCode transcript unavailable")?;
        let length = file.metadata().map_err(|_| "OpenCode transcript unavailable")?.len();
        if length > 32_000_000 || length < self.offset {
            return Err("OpenCode transcript limit or identity changed".into());
        }
        file.seek(SeekFrom::Start(self.offset))
            .map_err(|_| "OpenCode transcript unavailable")?;
        let mut remaining = length - self.offset;
        while remaining > 0 {
            let mut data = [0; 8192];
            let count = file
                .read(&mut data[..remaining.min(8192) as usize])
                .map_err(|_| "OpenCode transcript unavailable")?;
            if count == 0 {
                return Err("OpenCode transcript truncated".into());
            }
            self.offset += count as u64;
            remaining -= count as u64;
            if let Some(error) = crate::providers::opencode::stream_failure(&mut self.pending, &data[..count])? {
                return Ok(Some(error));
            }
        }
        Ok(None)
    }

    pub(super) fn finish(
        &mut self,
        outcome: Result<(), String>,
        transcript: &mut Option<String>,
        model: &str,
    ) -> Result<(String, Option<String>), String> {
        let mut secrets = vec![self.key.clone(), self.health.nonce.clone()];
        let encoded = serde_json::to_string(&self.key).map_err(|_| "OpenCode redaction unavailable")?;
        secrets.push(encoded[1..encoded.len() - 1].to_string());
        let mut leaked = false;
        // Redact the IPC-owned copy before any fallible output/state operation can return.
        if let Some(raw) = transcript.as_mut() {
            match redact_text(raw, &secrets, 0) {
                Ok((clean, found)) => {
                    *raw = clean;
                    leaked |= found;
                }
                Err(()) => {
                    *raw = String::new();
                    leaked = true;
                }
            }
        }
        if leaked {
            *transcript = None;
            fs::write(TRANSCRIPT, b"").map_err(|_| "OpenCode output redaction failed")?;
        }
        let stderr = read_output(Path::new("/scratch/stderr"), 8_000_000)?.unwrap_or_default();
        for path in [
            Path::new(TRANSCRIPT),
            Path::new("/scratch/stderr"),
            Path::new("/output/report.json"),
            Path::new("/output/finding_decisions.json"),
        ] {
            if let Some(mut text) = read_output(path, 32_000_000)? {
                match redact_text(&text, &secrets, 0) {
                    Ok((clean, found)) => {
                        text = clean;
                        leaked |= found;
                    }
                    Err(()) => {
                        text.clear();
                        leaked = true;
                    }
                }
                fs::write(path, text).map_err(|_| "OpenCode output redaction failed")?;
            }
        }
        fs::remove_dir_all(&self.state).map_err(|_| "OpenCode private state cleanup failed")?;
        if leaked || secrets.iter().any(|secret| stderr.contains(secret)) {
            *transcript = None;
            fs::write(TRANSCRIPT, b"").map_err(|_| "OpenCode output redaction failed")?;
            return Err("OpenCode output contained selected credential; discarded".into());
        }
        let limited = outcome.as_ref().err().is_some_and(|error| {
            matches!(
                crate::providers::claude::decode(error).0,
                crate::providers::claude::Failure::UsageLimit { .. }
            )
        });
        let limit_session = transcript
            .as_deref()
            .and_then(crate::providers::opencode::usage_limit_session);
        let auth = transcript
            .as_deref()
            .and_then(|raw| crate::providers::opencode::auth_session(raw, self.exit_code));
        let terminal = outcome.and_then(|()| {
            let raw = transcript.as_deref().ok_or("OpenCode transcript missing")?;
            crate::providers::opencode::final_text(raw)
        });
        if let Some(raw) = transcript.as_mut() {
            // Failed executions may stop mid-record. Keep only complete records for
            // accounting, after strict terminal parsing has already failed.
            if terminal.is_err() {
                complete_prefix(raw);
            }
            let mut marker = serde_json::json!({"type":"ocr.opencode.model","model":model,
                "catalogueId":self.snapshot.catalogue_id,"catalogueSnapshotId":self.snapshot.id})
            .to_string();
            for secret in &secrets {
                marker = marker.replace(secret, "[redacted]");
            }
            raw.push('\n');
            raw.push_str(&marker);
            raw.push('\n');
        }
        if limited {
            self.health
                .finish_usage_limit(&limit_session.ok_or("OpenCode rate-limit session missing")?)?;
            return Err(crate::providers::claude::Failure::UsageLimit { resets_at: None }.encode());
        }
        if let Some(session) = auth {
            self.health.finish_auth(&session)?;
            return Err(crate::providers::claude::Failure::Auth.encode());
        }
        let (session, text) = terminal?;
        self.health.finish(&session)?;
        let result = super::parse_final_response(&text, true)?;
        for decoded in [Some(&result.0), result.1.as_ref()].into_iter().flatten() {
            if redact_text(decoded, &secrets, 0).map_or(true, |(_, found)| found) {
                *transcript = None;
                fs::write(TRANSCRIPT, b"").map_err(|_| "OpenCode output redaction failed")?;
                return Err("OpenCode decoded report contained selected credential; discarded".into());
            }
        }
        Ok(result)
    }
}
impl Drop for Session {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.state);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opencode_redaction_preserves_safe_bytes_and_exact_evidence() {
        for text in [
            "{\"enabled\": true}",
            " 123 \n",
            "true\n\n",
            "plain snippet\n",
            "{unfinished",
            "[invalid,]",
            "{\n  \"enabled\": true\n}\n",
            "\"quoted\\nstring\"\n",
        ] {
            assert_eq!(
                redact_text(text, &["secret".into()], 0).unwrap(),
                (text.to_string(), false)
            );
        }
        let fixture = if let Ok(path) = std::env::var("OCR_OPENCODE_IDENTITY_FIXTURE") {
            serde_json::from_str::<serde_json::Value>(&fs::read_to_string(path).unwrap()).unwrap()
        } else {
            serde_json::json!({"snapshot":"{\"enabled\": true}\n", "report":{"findings":[{"evidence":[{"quote":"{\"enabled\": true}"}]}]}})
        };
        let snapshot = fixture["snapshot"].as_str().unwrap();
        let quote = fixture["report"]["findings"][0]["evidence"][0]["quote"]
            .as_str()
            .unwrap();
        assert_eq!(quote, "{\"enabled\": true}");
        assert_eq!(crate::review::evidence::quoted_line(snapshot, 1, quote), Some(1));
        let nested = "{\"x\":".repeat(140) + "0" + &"}".repeat(140);
        assert!(redact_text(&nested, &["secret".into()], 0).is_err());
        assert!(redact_text(
            &format!("{{\"body\":{}}}", serde_json::to_string(&nested).unwrap()),
            &["secret".into()],
            0
        )
        .is_err());
    }

    #[test]
    fn opencode_plaintext_unicode_inspection_is_bounded_and_identity_preserving() {
        let key = "proof-\"quoted\\key".to_string();
        let encoded = key.bytes().map(|byte| format!("\\u{byte:04x}")).collect::<String>();
        let mut text = format!("prefix {encoded} suffix");
        for _ in 0..4 {
            assert!(redact_text(&text, std::slice::from_ref(&key), 0).unwrap().1);
            text = text.replace('\\', "\\u005c");
        }
        let safe = r#"const label = "\u0061"; // \u263A"#;
        assert_eq!(
            redact_text(safe, std::slice::from_ref(&key), 0).unwrap(),
            (safe.into(), false)
        );
        assert!(redact_text(r"prefix \u00xz suffix", std::slice::from_ref(&key), 0).is_err());
        let exhausted = "prefix \\u005c".to_string() + &"u005c".repeat(65) + "u0070 suffix";
        assert!(redact_text(&exhausted, &[key], 0).is_err());
    }

    #[test]
    fn opencode_redacts_nested_json_keys_without_narrowing_credentials() {
        for key in ["123", "proof-\"quoted\\key", "ordinary-sentinel"] {
            let secrets = vec![key.to_string()];
            let mut raw = serde_json::json!({"summary":key}).to_string();
            for _ in 0..8 {
                raw = serde_json::json!({"body":raw}).to_string();
                let (clean, found) = redact_text(&raw, &secrets, 0).unwrap();
                assert!(found);
                let (_, found_again) = redact_text(&clean, &secrets, 0).unwrap();
                assert!(!found_again);
            }
        }
        let escaped = "{\n  \"summary\": \"proof-\\u0022quoted\\u005ckey\"\n}";
        let wrapped = serde_json::json!({"body":escaped}).to_string();
        let (clean, found) = redact_text(&wrapped, &["proof-\"quoted\\key".into()], 0).unwrap();
        assert!(found);
        assert!(!clean.contains("u0022"));
        assert!(redact_text("safe", &["secret".into()], 65).is_err());
    }

    #[test]
    fn opencode_failed_prefix_keeps_usage_once_and_terminal_parsing_strict() {
        let step = serde_json::json!({"type":"step_finish","sessionID":"session","part":{"id":"step","tokens":{"input":14,"output":5,"reasoning":2,"cache":{"read":3,"write":1}}}}).to_string();
        let mut raw = format!("{step}\n{step}\n{{\"type\":\"text\",\"part\":");
        assert!(crate::providers::opencode::final_text(&raw).is_err());
        complete_prefix(&mut raw);
        raw.push_str("{\"type\":\"ocr.opencode.model\",\"model\":\"kimi-k3\"}\n");
        let events = crate::review::transcript_events(&raw).unwrap();
        let mut models = std::collections::HashMap::new();
        crate::review::collect_usage(&events, &mut models);
        let usage = crate::review::finalize_usage(models);
        assert_eq!(
            (
                usage.input_tokens,
                usage.output_tokens,
                usage.cache_read_tokens,
                usage.cache_creation_tokens
            ),
            (14, 7, 3, 1)
        );
        assert_eq!(usage.model.as_deref(), Some("kimi-k3"));
        assert!(crate::providers::opencode::final_text(&raw).is_err());
    }
}
