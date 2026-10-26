//! GUI link thread: blocking IPC client, `hello` + `state.subscribe`, local
//! snapshot, reconnect with back-off; never shows stale data as live.
//!
//! Two background threads talk to the daemon over its socket:
//! - `dosi-gui-link` says `hello` (client `gui`), subscribes to `state` and
//!   keeps a local [`StateHandle`] of the snapshot; `show` events (a second
//!   launch, the tray) are queued for the window. When the daemon goes away
//!   the status turns [`LinkStatus::NotRunning`] at once and the link
//!   reconnects with a back-off from 0.5 s doubling to 5 s.
//! - `dosi-gui-calls` runs the window's requests one by one on its own
//!   connection, so a sign-in that takes 45 s never stalls the snapshot. Each
//!   answer comes back as a [`Replied`] with the daemon's error code, the
//!   same codes (and so the same exit-code classes) the CLI reports.
//!
//! Every event and answer asks the window to repaint.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::Value;

use crate::agent::state::{SharedState, StateHandle};
use crate::ipc::client::{CallResult, Client, ClientError};
use crate::ipc::{
    method, ErrorBody, ErrorCode, HelloResult, EVENT_SHOW, EVENT_STATE, SIGN_IN_TIMEOUT,
};

/// The first reconnect wait.
pub const BACKOFF_START: Duration = Duration::from_millis(500);
/// The longest reconnect wait.
pub const BACKOFF_MAX: Duration = Duration::from_secs(5);

/// How long the link waits for an event before looking at its stop flag.
const EVENT_POLL: Duration = Duration::from_millis(250);

/// Asks the window to repaint (from any thread).
pub type Repaint = Arc<dyn Fn() + Send + Sync>;

/// The wait after `current`: doubled, at most [`BACKOFF_MAX`].
pub fn next_backoff(current: Duration) -> Duration {
    (current * 2).min(BACKOFF_MAX)
}

/// How the window is connected to the daemon.
#[derive(Debug, Clone, PartialEq)]
pub enum LinkStatus {
    /// Not connected yet (start-up).
    Connecting,
    /// Subscribed: the snapshot is live.
    Connected { hello: HelloResult },
    /// No daemon answers (or the connection was lost).
    NotRunning { why: String },
    /// The daemon speaks another protocol major: read-only, restart it.
    Mismatch { message: String },
    /// A synthetic scene (`--preview`, test builds only): nothing is sent.
    Preview,
}

impl LinkStatus {
    /// The snapshot may be shown as live.
    pub fn is_live(&self) -> bool {
        matches!(self, LinkStatus::Connected { .. } | LinkStatus::Preview)
    }
}

/// One request of the window.
#[derive(Debug)]
pub struct Call {
    pub method: &'static str,
    pub params: Value,
}

/// The answer to a [`Call`].
#[derive(Debug, Clone)]
pub struct Replied {
    pub method: &'static str,
    pub outcome: Result<CallResult, ErrorBody>,
}

/// The window's side of the link.
pub struct Link {
    state: StateHandle,
    status: Arc<Mutex<LinkStatus>>,
    calls: Option<Sender<Call>>,
    replies: Receiver<Replied>,
    shows: Receiver<Option<String>>,
    stop: Arc<AtomicBool>,
    threads: Vec<std::thread::JoinHandle<()>>,
    /// Preview only: the requests the window made.
    recorded: Arc<Mutex<Vec<&'static str>>>,
}

/// An error answer for a failure that did not come from the daemon.
pub fn error_of(error: ClientError) -> ErrorBody {
    match error {
        ClientError::Daemon(body) | ClientError::Version(body) => body,
        ClientError::Timeout => ErrorBody::new(ErrorCode::Timeout, error.to_string()),
        other => ErrorBody::new(ErrorCode::Internal, other.to_string()),
    }
}

/// `data` over the defaults of every field, so a daemon of the same major
/// that lacks a newer field still yields a snapshot.
pub fn decode_snapshot(data: Value) -> Option<SharedState> {
    let Value::Object(fields) = data else {
        return None;
    };
    let mut base = serde_json::to_value(SharedState::default()).ok()?;
    if let Value::Object(defaults) = &mut base {
        defaults.extend(fields);
    }
    serde_json::from_value(base).ok()
}

impl Link {
    /// Connect to the daemon at `socket` in the background.
    pub fn start(socket: PathBuf, repaint: Repaint) -> Self {
        let state = StateHandle::new();
        let status = Arc::new(Mutex::new(LinkStatus::Connecting));
        let stop = Arc::new(AtomicBool::new(false));
        let (calls_tx, calls_rx) = mpsc::channel::<Call>();
        let (replies_tx, replies) = mpsc::channel();
        let (shows_tx, shows) = mpsc::channel();
        let mut threads = Vec::new();
        {
            let (socket, state, status, stop, repaint) = (
                socket.clone(),
                state.clone(),
                status.clone(),
                stop.clone(),
                repaint.clone(),
            );
            match std::thread::Builder::new()
                .name("dosi-gui-link".into())
                .spawn(move || subscribe_loop(&socket, &state, &status, &stop, &shows_tx, &repaint))
            {
                Ok(thread) => threads.push(thread),
                Err(e) => tracing::error!(error = %e, "the link to the agent cannot start"),
            }
        }
        {
            let stop = stop.clone();
            match std::thread::Builder::new()
                .name("dosi-gui-calls".into())
                .spawn(move || calls_loop(&socket, &calls_rx, &replies_tx, &stop, &repaint))
            {
                Ok(thread) => threads.push(thread),
                Err(e) => tracing::error!(error = %e, "requests to the agent cannot be sent"),
            }
        }
        Self {
            state,
            status,
            calls: Some(calls_tx),
            replies,
            shows,
            stop,
            threads,
            recorded: Arc::new(Mutex::new(Vec::new())),
        }
    }

    /// A link that shows `snapshot` and sends nothing (preview scenes).
    pub fn preview(snapshot: SharedState) -> Self {
        let state = StateHandle::new();
        state.update(|s| *s = snapshot);
        let (_, replies) = mpsc::channel();
        let (_, shows) = mpsc::channel();
        Self {
            state,
            status: Arc::new(Mutex::new(LinkStatus::Preview)),
            calls: None,
            replies,
            shows,
            stop: Arc::new(AtomicBool::new(true)),
            threads: Vec::new(),
            recorded: Arc::new(Mutex::new(Vec::new())),
        }
    }

    pub fn snapshot(&self) -> SharedState {
        self.state.snapshot()
    }

    pub fn status(&self) -> LinkStatus {
        self.status
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    /// Change the local snapshot at once (a pause shows as "Pausing…" before
    /// the daemon confirms); the next `state` event replaces it.
    pub fn update_local(&self, f: impl FnOnce(&mut SharedState)) {
        self.state.update(f);
    }

    /// Send `method` with `params` (answered later through [`Link::try_reply`]).
    pub fn call(&self, method: &'static str, params: Value) {
        match &self.calls {
            Some(calls) => {
                if calls.send(Call { method, params }).is_err() {
                    tracing::warn!(method, "the request thread is gone");
                }
            }
            None => self
                .recorded
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .push(method),
        }
    }

    /// The requests a preview link was asked to send.
    pub fn recorded(&self) -> Vec<&'static str> {
        self.recorded
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    pub fn try_reply(&self) -> Option<Replied> {
        self.replies.try_recv().ok()
    }

    /// The next `show` request (its view), if one came.
    pub fn try_show(&self) -> Option<Option<String>> {
        self.shows.try_recv().ok()
    }

    /// Stop both threads (they notice within a quarter second).
    pub fn stop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        self.calls = None;
        for thread in self.threads.drain(..) {
            let _ = thread.join();
        }
    }
}

impl Drop for Link {
    fn drop(&mut self) {
        self.stop();
    }
}

fn set(status: &Mutex<LinkStatus>, value: LinkStatus) {
    *status.lock().unwrap_or_else(|e| e.into_inner()) = value;
}

/// Wait `total` in small steps; false when asked to stop meanwhile.
fn pause(total: Duration, stop: &AtomicBool) -> bool {
    let deadline = Instant::now() + total;
    while Instant::now() < deadline {
        if stop.load(Ordering::SeqCst) {
            return false;
        }
        std::thread::sleep(Duration::from_millis(50).min(deadline - Instant::now()));
    }
    !stop.load(Ordering::SeqCst)
}

fn subscribe_loop(
    socket: &std::path::Path,
    state: &StateHandle,
    status: &Mutex<LinkStatus>,
    stop: &AtomicBool,
    shows: &Sender<Option<String>>,
    repaint: &Repaint,
) {
    let mut backoff = BACKOFF_START;
    while !stop.load(Ordering::SeqCst) {
        let lost = match Client::connect(socket, "gui").and_then(|client| {
            let hello = client.hello().clone();
            client.subscribe().map(|sub| (hello, sub))
        }) {
            Ok((hello, mut subscription)) => {
                set(status, LinkStatus::Connected { hello });
                backoff = BACKOFF_START;
                repaint();
                loop {
                    if stop.load(Ordering::SeqCst) {
                        return;
                    }
                    match subscription.next_event(Some(EVENT_POLL)) {
                        Ok(event) if event.event == EVENT_STATE => {
                            match decode_snapshot(event.data) {
                                Some(snapshot) => state.update(|s| *s = snapshot),
                                None => tracing::warn!("a snapshot was not understood"),
                            }
                            repaint();
                        }
                        Ok(event) if event.event == EVENT_SHOW => {
                            let view = event.data.get("view").and_then(Value::as_str);
                            let _ = shows.send(view.map(str::to_owned));
                            repaint();
                        }
                        Ok(_) | Err(ClientError::Timeout) => {}
                        Err(e) => break e.to_string(),
                    }
                }
            }
            Err(ClientError::Version(body)) => {
                set(
                    status,
                    LinkStatus::Mismatch {
                        message: body.message,
                    },
                );
                backoff = BACKOFF_MAX;
                repaint();
                if !pause(backoff, stop) {
                    return;
                }
                continue;
            }
            Err(e) => e.to_string(),
        };
        set(status, LinkStatus::NotRunning { why: lost });
        repaint();
        if !pause(backoff, stop) {
            return;
        }
        backoff = next_backoff(backoff);
    }
}

fn calls_loop(
    socket: &std::path::Path,
    calls: &Receiver<Call>,
    replies: &Sender<Replied>,
    stop: &AtomicBool,
    repaint: &Repaint,
) {
    let mut client: Option<Client> = None;
    loop {
        let call = match calls.recv_timeout(EVENT_POLL) {
            Ok(call) => call,
            Err(RecvTimeoutError::Timeout) => {
                if stop.load(Ordering::SeqCst) {
                    return;
                }
                continue;
            }
            Err(RecvTimeoutError::Disconnected) => return,
        };
        let outcome = run(&mut client, socket, call.method, call.params);
        let _ = replies.send(Replied {
            method: call.method,
            outcome,
        });
        repaint();
    }
}

/// Send one request, connecting first if needed. A connection that broke
/// (the daemon restarted, or it closed an idle connection) is replaced once.
fn run(
    client: &mut Option<Client>,
    socket: &std::path::Path,
    method: &'static str,
    mut params: Value,
) -> Result<CallResult, ErrorBody> {
    let timeout = if method == method::AUTH_SIGN_IN {
        SIGN_IN_TIMEOUT + Duration::from_secs(5)
    } else {
        crate::ipc::client::CALL_TIMEOUT
    };
    let mut attempt = 0;
    let outcome = loop {
        attempt += 1;
        if client.is_none() {
            match Client::connect(socket, "gui") {
                Ok(connected) => *client = Some(connected),
                Err(e) => break Err(error_of(e)),
            }
        }
        let Some(connected) = client.as_mut() else {
            break Err(ErrorBody::new(ErrorCode::Internal, "not connected"));
        };
        // A copy per attempt; each is wiped by the client once sent.
        match connected.call_with_timeout(method, params.clone(), timeout) {
            Ok(result) => break Ok(result),
            Err(ClientError::Daemon(body)) => break Err(body),
            Err(ClientError::Protocol(why)) if attempt < 2 => {
                tracing::debug!(%why, "the connection to the agent broke; reconnecting");
                *client = None;
            }
            Err(e) => {
                *client = None;
                break Err(error_of(e));
            }
        }
    };
    crate::ipc::client::wipe_strings(&mut params);
    outcome
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_backoff_doubles_up_to_five_seconds() {
        let mut wait = BACKOFF_START;
        let mut seen = vec![wait];
        for _ in 0..6 {
            wait = next_backoff(wait);
            seen.push(wait);
        }
        let ms: Vec<u128> = seen.iter().map(Duration::as_millis).collect();
        assert_eq!(ms, [500, 1000, 2000, 4000, 5000, 5000, 5000]);
    }

    #[test]
    fn snapshots_missing_newer_fields_still_decode() {
        let snapshot = decode_snapshot(serde_json::json!({
            "status": "tracking",
            "display_name": "ayesha@acme.test",
            "a_field_from_the_future": 1
        }))
        .expect("decodes");
        assert_eq!(
            snapshot.status,
            crate::agent::state::TrackerStatus::Tracking
        );
        assert_eq!(snapshot.display_name, "ayesha@acme.test");
        assert!(snapshot.server_reachable, "defaults fill the rest");
        assert!(decode_snapshot(serde_json::json!("nope")).is_none());
        assert!(decode_snapshot(serde_json::json!({ "status": "dancing" })).is_none());
    }

    #[test]
    fn a_preview_link_records_requests_and_sends_nothing() {
        let link = Link::preview(SharedState::default());
        assert_eq!(link.status(), LinkStatus::Preview);
        assert!(link.status().is_live());
        link.call(method::TRACKING_PAUSE, Value::Null);
        assert_eq!(link.recorded(), vec![method::TRACKING_PAUSE]);
        assert!(link.try_reply().is_none());
    }
}
