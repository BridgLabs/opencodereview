use super::*;

pub(crate) fn handle_stream(mut stream: TcpStream, state: SharedState) -> Result<(), String> {
    let request = read_request(&mut stream)?;
    let cors_origin = trusted_cors_origin(request.origin.as_deref());
    if request.method == "GET" {
        if let Some(pr_id) = parse_review_progress_path(&request.path) {
            if let Some(response) = check_auth(&request, &state) {
                return write_response(&mut stream, response, cors_origin.as_deref());
            }
            return stream_review_progress(&mut stream, &state, pr_id, cors_origin.as_deref());
        }
    }
    let response = route(request, state);
    write_response(&mut stream, response, cors_origin.as_deref())
}

pub(crate) const MAX_REVIEW_SSE_CONNECTIONS: usize = 8;
pub(crate) const REVIEW_SSE_HEARTBEAT: Duration = Duration::from_secs(15);
pub(crate) const REVIEW_SSE_MAX_AGE: Duration = Duration::from_secs(2 * 60 * 60);

pub(crate) struct ReviewSseGuard<'a> {
    pub(crate) active: &'a AtomicUsize,
}

impl Drop for ReviewSseGuard<'_> {
    fn drop(&mut self) {
        self.active.fetch_sub(1, Ordering::Relaxed);
    }
}

pub(crate) fn parse_review_progress_path(path: &str) -> Option<i64> {
    let parts: Vec<&str> = path
        .trim_matches('/')
        .split('/')
        .filter(|part| !part.is_empty())
        .collect();
    if parts.len() != 4 || parts[0] != "reviews" || parts[2] != "run" || parts[3] != "events" {
        return None;
    }
    parts[1].parse().ok()
}

pub(crate) fn stream_review_progress(
    stream: &mut TcpStream,
    state: &SharedState,
    pr_id: i64,
    cors_origin: Option<&str>,
) -> Result<(), String> {
    let active = state.review_sse_connections.fetch_add(1, Ordering::Relaxed);
    if active >= MAX_REVIEW_SSE_CONNECTIONS {
        state.review_sse_connections.fetch_sub(1, Ordering::Relaxed);
        return write_response(stream, json_error(429, "too many review progress streams"), cors_origin);
    }
    let _guard = ReviewSseGuard {
        active: &state.review_sse_connections,
    };
    stream
        .set_write_timeout(Some(Duration::from_secs(20)))
        .map_err(|error| error.to_string())?;
    let cors = cors_origin
        .map(|origin| format!(
            "access-control-allow-origin: {origin}\r\naccess-control-allow-methods: GET,POST,PUT,DELETE,OPTIONS\r\naccess-control-allow-headers: content-type,authorization,x-flow-secret\r\n"
        ))
        .unwrap_or_default();
    let head = format!(
        "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream; charset=utf-8\r\ncache-control: no-cache, no-store\r\n{}x-accel-buffering: no\r\nconnection: keep-alive\r\n\r\n",
        cors,
    );
    stream.write_all(head.as_bytes()).map_err(|error| error.to_string())?;
    stream.flush().map_err(|error| error.to_string())?;

    let db = Database::open_read_at(state.config.db_path.clone())?;
    let started = Instant::now();
    let mut last_key: Option<(i64, i64)> = None;
    let mut sent_initial = false;
    let mut last_heartbeat = Instant::now();
    while started.elapsed() < REVIEW_SSE_MAX_AGE {
        let snapshot = db.review_progress_snapshot(pr_id)?;
        let key = snapshot.as_ref().map(|value| (value.run_id, value.updated_at));
        if !sent_initial || key != last_key {
            write_review_progress_event(stream, snapshot.as_ref())?;
            stream.flush().map_err(|error| error.to_string())?;
            last_key = key;
            sent_initial = true;
            if snapshot.is_none()
                || snapshot
                    .as_ref()
                    .is_some_and(|value| review_status_is_terminal(&value.status))
            {
                break;
            }
        }
        if last_heartbeat.elapsed() >= REVIEW_SSE_HEARTBEAT {
            stream
                .write_all(b": keep-alive\n\n")
                .map_err(|error| error.to_string())?;
            stream.flush().map_err(|error| error.to_string())?;
            last_heartbeat = Instant::now();
        }
        if wait_for_progress_disconnect(stream, Duration::from_millis(500))? {
            break;
        }
    }
    Ok(())
}

pub(crate) fn wait_for_progress_disconnect(stream: &TcpStream, interval: Duration) -> Result<bool, String> {
    stream
        .set_read_timeout(Some(interval))
        .map_err(|error| error.to_string())?;
    match stream.peek(&mut [0]) {
        Ok(_) => Ok(true),
        Err(error)
            if matches!(
                error.kind(),
                std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut | std::io::ErrorKind::Interrupted
            ) =>
        {
            Ok(false)
        }
        Err(error) => Err(error.to_string()),
    }
}

pub(crate) fn write_review_progress_event(
    stream: &mut TcpStream,
    snapshot: Option<&crate::db::ReviewProgressSnapshot>,
) -> Result<(), String> {
    let data = serde_json::to_string(&snapshot).map_err(|error| error.to_string())?;
    let event_id = snapshot
        .map(|value| format!("{}-{}", value.run_id, value.updated_at))
        .unwrap_or_else(|| "empty".to_string());
    let frame = format!("event: review-progress\nid: {event_id}\ndata: {data}\n\n");
    stream.write_all(frame.as_bytes()).map_err(|error| error.to_string())
}

pub(crate) fn review_status_is_terminal(status: &str) -> bool {
    !matches!(status, "pending" | "running")
}
