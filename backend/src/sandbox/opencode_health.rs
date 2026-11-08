use std::io::{Read, Write};
use std::os::unix::{
    fs::PermissionsExt,
    net::{UnixListener, UnixStream},
};
use std::path::{Path, PathBuf};

#[derive(PartialEq)]
enum Terminal {
    Complete(String),
    Auth(String),
    UsageLimit(String),
}

pub(super) struct Health {
    listener: UnixListener,
    stream: Option<UnixStream>,
    buffer: Vec<u8>,
    pub(super) path: PathBuf,
    pub(super) nonce: String,
    uid: u32,
    ready: bool,
    terminal: Option<Terminal>,
    failed: bool,
}
impl Health {
    pub(super) fn ready(&self) -> bool {
        self.ready && !self.failed
    }
    pub(super) fn new(root: &Path, uid: u32) -> Result<Self, String> {
        let nonce = super::secret()?;
        let path = root.join(format!("guard-{nonce}.sock"));
        let listener = UnixListener::bind(&path).map_err(|_| "OpenCode guard channel unavailable")?;
        listener
            .set_nonblocking(true)
            .map_err(|_| "OpenCode guard channel unavailable")?;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o622))
            .map_err(|_| "OpenCode guard channel unavailable")?;
        Ok(Self {
            listener,
            stream: None,
            buffer: Vec::new(),
            path,
            nonce,
            uid,
            ready: false,
            terminal: None,
            failed: false,
        })
    }
    pub(super) fn poll(&mut self) -> Result<(), String> {
        if self.failed {
            return Err("OpenCode guard infrastructure failed".into());
        }
        let result = self.drain();
        if result.is_err() {
            self.failed = true;
            // A rejected peer must not remain blocked awaiting an acknowledgement while
            // the supervisor is stopping the CLI's private server.
            self.stream.take();
        }
        result
    }
    fn drain(&mut self) -> Result<(), String> {
        use std::os::fd::AsRawFd;
        let error = "OpenCode guard infrastructure failed";
        match self.listener.accept() {
            Ok((stream, _)) => {
                if self.stream.is_some() {
                    return Err(error.into());
                }
                let mut peer = std::mem::MaybeUninit::<libc::ucred>::uninit();
                let mut size = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
                let status = unsafe {
                    libc::getsockopt(
                        stream.as_raw_fd(),
                        libc::SOL_SOCKET,
                        libc::SO_PEERCRED,
                        peer.as_mut_ptr().cast(),
                        &mut size,
                    )
                };
                if status != 0
                    || size as usize != std::mem::size_of::<libc::ucred>()
                    || unsafe { peer.assume_init() }.uid != self.uid
                {
                    return Err(error.into());
                }
                stream.set_nonblocking(true).map_err(|_| error)?;
                self.stream = Some(stream);
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {}
            Err(_) => return Err(error.into()),
        }
        let Some(stream) = self.stream.as_mut() else {
            return Ok(());
        };
        loop {
            let mut bytes = [0; 1024];
            match stream.read(&mut bytes) {
                Ok(0) => {
                    return if self.terminal.is_some() && self.buffer.is_empty() {
                        Ok(())
                    } else {
                        Err(error.into())
                    }
                }
                Ok(count) => {
                    for byte in &bytes[..count] {
                        if *byte != b'\n' {
                            self.buffer.push(*byte);
                            if self.buffer.len() > 1024 {
                                return Err(error.into());
                            }
                            continue;
                        }
                        #[derive(serde::Deserialize)]
                        #[serde(tag = "type", deny_unknown_fields)]
                        enum Frame {
                            #[serde(rename = "ready")]
                            Ready { v: u8, nonce: String },
                            #[serde(rename = "complete")]
                            Complete {
                                v: u8,
                                #[serde(rename = "sessionID")]
                                session: String,
                            },
                            #[serde(rename = "provider_failed")]
                            ProviderFailed {
                                v: u8,
                                #[serde(rename = "sessionID")]
                                session: String,
                                code: String,
                                status: u16,
                            },
                            #[serde(rename = "fault")]
                            Fault { v: u8, code: String },
                        }
                        let frame: Frame = serde_json::from_slice(&self.buffer).map_err(|_| error)?;
                        self.buffer.clear();
                        let ack = match frame {
                            Frame::Ready { v: 1, nonce } if !self.ready && nonce == self.nonce => {
                                self.ready = true;
                                "ready"
                            }
                            Frame::Complete { v: 1, session }
                                if self.ready
                                    && self.terminal.is_none()
                                    && !session.is_empty()
                                    && session.len() <= 128 =>
                            {
                                self.terminal = Some(Terminal::Complete(session));
                                "complete"
                            }
                            Frame::ProviderFailed {
                                v: 1,
                                session,
                                code,
                                status: 401,
                            } if self.ready
                                && self.terminal.is_none()
                                && code == "auth"
                                && !session.is_empty()
                                && session.len() <= 128 =>
                            {
                                self.terminal = Some(Terminal::Auth(session));
                                "provider_failed"
                            }
                            Frame::ProviderFailed {
                                v: 1,
                                session,
                                code,
                                status: 429,
                            } if self.ready
                                && self.terminal.is_none()
                                && code == "usage_limit"
                                && !session.is_empty()
                                && session.len() <= 128 =>
                            {
                                self.terminal = Some(Terminal::UsageLimit(session));
                                "provider_failed"
                            }
                            Frame::Fault { v, code } => {
                                let _ = (v, code);
                                return Err(error.into());
                            }
                            _ => return Err(error.into()),
                        };
                        writeln!(stream, "{}", serde_json::json!({"v":1,"type":"ack","for":ack})).map_err(|_| error)?;
                    }
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => return Ok(()),
                Err(_) => return Err(error.into()),
            }
        }
    }
    pub(super) fn finish(&mut self, session: &str) -> Result<(), String> {
        self.finish_terminal(Terminal::Complete(session.into()))
    }
    pub(super) fn finish_auth(&mut self, session: &str) -> Result<(), String> {
        self.finish_terminal(Terminal::Auth(session.into()))
    }
    pub(super) fn finish_usage_limit(&mut self, session: &str) -> Result<(), String> {
        self.finish_terminal(Terminal::UsageLimit(session.into()))
    }
    fn finish_terminal(&mut self, terminal: Terminal) -> Result<(), String> {
        self.poll()?;
        if !self.failed && self.ready && self.terminal.as_ref() == Some(&terminal) {
            Ok(())
        } else {
            self.failed = true;
            Err("OpenCode guard completion missing".into())
        }
    }
}
impl Drop for Health {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn opencode_limit_receipt_requires_matching_session_and_final_fault_drain() {
        for scenario in ["valid", "missing", "wrong_session", "late_fault", "duplicate"] {
            let mut health = Health::new(&std::env::temp_dir(), unsafe { libc::geteuid() }).unwrap();
            let mut client = UnixStream::connect(&health.path).unwrap();
            writeln!(
                client,
                "{}",
                serde_json::json!({"v":1,"type":"ready","nonce":health.nonce})
            )
            .unwrap();
            health.poll().unwrap();
            let receipt = serde_json::json!({"v":1,"type":"provider_failed","sessionID":"session","code":"usage_limit","status":429});
            if scenario != "missing" {
                writeln!(client, "{receipt}").unwrap();
                health.poll().unwrap();
            }
            if scenario == "duplicate" {
                writeln!(client, "{receipt}").unwrap();
            }
            if scenario == "late_fault" {
                writeln!(
                    client,
                    "{}",
                    serde_json::json!({"v":1,"type":"fault","code":"helper_timeout"})
                )
                .unwrap();
            }
            let session = if scenario == "wrong_session" {
                "other"
            } else {
                "session"
            };
            assert_eq!(
                health.finish_usage_limit(session).is_ok(),
                scenario == "valid",
                "{scenario}"
            );
        }
    }
    #[test]
    fn opencode_auth_receipt_is_exclusive_session_bound_and_fault_latched() {
        let auth = serde_json::json!({"v":1,"type":"provider_failed","sessionID":"session","code":"auth","status":401});
        for scenario in [
            "valid",
            "missing",
            "wrong_status",
            "wrong_session",
            "duplicate",
            "complete",
            "fault",
            "earlier_fault",
        ] {
            let mut health = Health::new(&std::env::temp_dir(), unsafe { libc::geteuid() }).unwrap();
            let mut client = UnixStream::connect(&health.path).unwrap();
            writeln!(
                client,
                "{}",
                serde_json::json!({"v":1,"type":"ready","nonce":health.nonce})
            )
            .unwrap();
            health.poll().unwrap();
            let mut frame = auth.clone();
            if scenario == "wrong_status" {
                frame["status"] = 403.into();
            }
            if scenario == "wrong_session" {
                frame["sessionID"] = "other".into();
            }
            if scenario == "earlier_fault" {
                writeln!(
                    client,
                    "{}",
                    serde_json::json!({"v":1,"type":"fault","code":"helper_timeout"})
                )
                .unwrap();
            }
            if scenario != "missing" {
                writeln!(client, "{frame}").unwrap();
            }
            match scenario {
                "duplicate" => writeln!(client, "{auth}").unwrap(),
                "complete" => writeln!(
                    client,
                    "{}",
                    serde_json::json!({"v":1,"type":"complete","sessionID":"session"})
                )
                .unwrap(),
                "fault" => writeln!(
                    client,
                    "{}",
                    serde_json::json!({"v":1,"type":"fault","code":"work_after_seal"})
                )
                .unwrap(),
                _ => {}
            }
            if scenario == "valid" {
                health.finish_auth("session").unwrap();
                client.shutdown(std::net::Shutdown::Both).unwrap();
                health.finish_auth("session").unwrap();
            } else {
                assert!(health.finish_auth("session").is_err(), "{scenario}");
                assert!(health.finish_auth("session").is_err(), "fault recovered: {scenario}");
            }
        }
    }
    #[test]
    fn opencode_health_accepts_authenticated_ready_and_matching_completion() {
        let root = std::env::temp_dir();
        let mut health = Health::new(&root, unsafe { libc::geteuid() }).unwrap();
        let mut client = UnixStream::connect(&health.path).unwrap();
        writeln!(
            client,
            "{}",
            serde_json::json!({"v":1,"type":"ready","nonce":health.nonce})
        )
        .unwrap();
        health.poll().unwrap();
        writeln!(
            client,
            "{}",
            serde_json::json!({"v":1,"type":"complete","sessionID":"session"})
        )
        .unwrap();
        health.finish("session").unwrap();
    }
    #[test]
    fn opencode_health_wrong_session_cannot_be_recovered() {
        let mut health = Health::new(&std::env::temp_dir(), unsafe { libc::geteuid() }).unwrap();
        let mut client = UnixStream::connect(&health.path).unwrap();
        writeln!(
            client,
            "{}",
            serde_json::json!({"v":1,"type":"ready","nonce":health.nonce})
        )
        .unwrap();
        health.poll().unwrap();
        writeln!(
            client,
            "{}",
            serde_json::json!({"v":1,"type":"complete","sessionID":"other"})
        )
        .unwrap();
        assert!(health.finish("expected").is_err());
        assert!(health.finish("other").is_err());
    }
    #[test]
    fn opencode_health_rejects_wrong_nonce_oversize_eof_and_reconnection() {
        for scenario in ["nonce", "oversize", "eof", "duplicate", "fault", "reconnect", "session"] {
            let mut health = Health::new(&std::env::temp_dir(), unsafe { libc::geteuid() }).unwrap();
            let mut client = UnixStream::connect(&health.path).unwrap();
            if scenario == "nonce" {
                writeln!(
                    client,
                    "{}",
                    serde_json::json!({"v":1,"type":"ready","nonce":"incorrect"})
                )
                .unwrap();
                assert!(health.poll().is_err());
                continue;
            }
            writeln!(
                client,
                "{}",
                serde_json::json!({"v":1,"type":"ready","nonce":health.nonce})
            )
            .unwrap();
            health.poll().unwrap();
            match scenario {
                "oversize" => client.write_all(&[b'x'; 1025]).unwrap(),
                "eof" => client.shutdown(std::net::Shutdown::Both).unwrap(),
                "duplicate" => writeln!(
                    client,
                    "{}",
                    serde_json::json!({"v":1,"type":"ready","nonce":health.nonce})
                )
                .unwrap(),
                "fault" => writeln!(
                    client,
                    "{}\n{}",
                    serde_json::json!({"v":1,"type":"fault","code":"helper_timeout"}),
                    serde_json::json!({"v":1,"type":"complete","sessionID":"session"})
                )
                .unwrap(),
                "reconnect" => {
                    let _extra = UnixStream::connect(&health.path).unwrap();
                }
                "session" => writeln!(
                    client,
                    "{}",
                    serde_json::json!({"v":1,"type":"complete","sessionID":"different"})
                )
                .unwrap(),
                _ => unreachable!(),
            }
            assert!(health.finish("session").is_err(), "{scenario}");
            assert!(health.finish("session").is_err(), "fault recovered: {scenario}");
        }
    }
}
