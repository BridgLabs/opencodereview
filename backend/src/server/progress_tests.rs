use super::*;
use std::net::Shutdown;
use std::sync::mpsc;

struct Fixture {
    state: SharedState,
    db: Database,
    reviews: Vec<(i64, i64)>,
    workers: Vec<thread::JoinHandle<()>>,
}

impl Fixture {
    fn new() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let root = std::env::temp_dir().join(format!(
            "ocr-progress-stream-{}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
            NEXT.fetch_add(1, Ordering::Relaxed),
        ));
        let db = crate::db::Db::for_test_at(root.join("test.sqlite")).open().unwrap();
        db.ensure_temp_repo_for_index(1, "acme/fixture").unwrap();
        let reviews = (1..=5)
            .map(|number| {
                let pr = db
                    .upsert_pull_request(&crate::db::PullRequestInput {
                        repo_id: 1,
                        number,
                        title: "Fixture".into(),
                        author: "fixture".into(),
                        author_avatar_url: None,
                        is_draft: false,
                        head_sha: "head".into(),
                        base_sha: None,
                        base_ref: Some("main".into()),
                        head_ref: None,
                        default_branch: Some("main".into()),
                        url: None,
                        lane: "reviewing".into(),
                        status_reason: None,
                    })
                    .unwrap();
                let run = db.start_review_skeleton_with(pr, Some("fixture")).unwrap();
                db.set_review_run_running(run, &review::index_phase_json("running"))
                    .unwrap();
                (pr, run)
            })
            .collect();
        let state = Arc::new(ServerState::new(ServerConfig {
            bind: "127.0.0.1:0".into(),
            db: crate::db::Db::for_test_at(root.join("test.sqlite")),
            data_dir: root.to_path_buf(),
            repos_dir: root.join("repos"),
            auto_reindex: false,
            poll_interval_seconds: 300,
            api_token: None,
        }));
        Self {
            state,
            db,
            reviews,
            workers: Vec::new(),
        }
    }

    fn connect(&mut self, index: usize) -> (TcpStream, mpsc::Receiver<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let mut client = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        client.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
        let (mut server, _) = listener.accept().unwrap();
        let state = self.state.clone();
        let id = self.reviews[index].0;
        let (finished, receiver) = mpsc::channel();
        self.workers.push(thread::spawn(move || {
            let _ = stream_review_progress(&mut server, &state, id, None);
            let _ = finished.send(());
        }));
        let mut initial = Vec::new();
        while !initial.ends_with(b"\n\n") {
            let mut byte = [0];
            client.read_exact(&mut byte).unwrap();
            initial.push(byte[0]);
        }
        assert!(initial.starts_with(b"HTTP/1.1 200 OK\r\n"));
        (client, receiver)
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        for (_, run) in &self.reviews {
            let _ = self.db.cancel_review_run(*run);
        }
        for worker in self.workers.drain(..) {
            let _ = worker.join();
        }
        // Nothing is written there now that the database is Postgres; remove it if a test did.
        let _ = std::fs::remove_dir_all(&self.state.config.data_dir);
    }
}

#[test]
fn progress_stream_releases_slots_while_switching_between_five_reviews() {
    let mut fixture = Fixture::new();
    for index in 0..20 {
        let (client, finished) = fixture.connect(index % 5);
        assert_eq!(fixture.state.review_sse_connections.load(Ordering::Relaxed), 1);
        client.shutdown(Shutdown::Both).unwrap();
        drop(client);
        assert!(
            finished.recv_timeout(Duration::from_secs(1)).is_ok(),
            "Disconnected review held its SSE slot until the next write"
        );
        assert_eq!(fixture.state.review_sse_connections.load(Ordering::Relaxed), 0);
    }
}

#[test]
fn progress_stream_keeps_idle_clients_connected_and_delivers_terminal_updates() {
    let mut fixture = Fixture::new();
    let (mut client, finished) = fixture.connect(0);
    assert!(matches!(
        finished.recv_timeout(Duration::from_millis(1100)),
        Err(mpsc::RecvTimeoutError::Timeout)
    ));
    assert_eq!(fixture.state.review_sse_connections.load(Ordering::Relaxed), 1);
    fixture.db.cancel_review_run(fixture.reviews[0].1).unwrap();
    let mut update = String::new();
    client.read_to_string(&mut update).unwrap();
    assert!(update.contains("event: review-progress\n"));
    assert!(update.contains("\"status\":\"cancelled\""));
    finished.recv_timeout(Duration::from_secs(1)).unwrap();
    assert_eq!(fixture.state.review_sse_connections.load(Ordering::Relaxed), 0);
}

#[test]
fn progress_stream_keeps_the_limit_for_live_clients_and_reuses_released_slots() {
    let mut fixture = Fixture::new();
    let mut clients = Vec::new();
    for index in 0..MAX_REVIEW_SSE_CONNECTIONS {
        clients.push(fixture.connect(index % 5));
    }
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let mut client = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
    client.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
    let (mut server, _) = listener.accept().unwrap();
    stream_review_progress(&mut server, &fixture.state, fixture.reviews[0].0, None).unwrap();
    drop(server);
    let mut response = String::new();
    client.read_to_string(&mut response).unwrap();
    assert!(response.starts_with("HTTP/1.1 429 "));
    assert_eq!(
        fixture.state.review_sse_connections.load(Ordering::Relaxed),
        MAX_REVIEW_SSE_CONNECTIONS
    );
    let (released, finished) = clients.pop().unwrap();
    released.shutdown(Shutdown::Both).unwrap();
    drop(released);
    assert!(finished.recv_timeout(Duration::from_secs(1)).is_ok());
    clients.push(fixture.connect(0));
    assert_eq!(
        fixture.state.review_sse_connections.load(Ordering::Relaxed),
        MAX_REVIEW_SSE_CONNECTIONS
    );
    drop(clients);
}
