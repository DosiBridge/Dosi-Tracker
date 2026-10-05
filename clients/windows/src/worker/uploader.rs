//! Uploading the queue — on a thread of its own, with its own connection to
//! the queue and its own API client, so a long backlog on a slow uplink never
//! delays pausing, quitting, or saving the block before the machine sleeps.
//!
//! The answers are handled as CONTRACT §1 ("How agents treat upload
//! responses") says, oldest row first:
//!
//! | answer | what happens |
//! |---|---|
//! | 2xx | the row leaves the queue |
//! | network, 5xx, 404/405/408/429, a 4xx without a code | kept; retried after 30 s, doubling up to 30 min (`Retry-After` honoured) |
//! | sign-in refused | kept; uploads stop, the worker asks for the password |
//! | 402 subscription inactive | kept; retried hourly |
//! | 403 not a member / 409 archived | the projects are fetched again: parked if the project is gone, else kept |
//! | 425 in the future | kept; retried in 5 minutes (the clock is ahead) |
//! | 413, capture too large/invalid | the images are stripped and it is sent again at once |
//! | overlap, id conflict, too long/old, invalid range, validation | parked with the server's reason |
//!
//! `sentAt` is stamped by the API client on every attempt.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};
use tokio::time::Instant;

use crate::api::{ApiClient, ServerClock, SubmitError};
use crate::model::Project;
use crate::state::{StateHandle, UploadIssue, UploadState};
use crate::storage::Storage;

use super::{publish_queue, set_upload_state, Account};

/// Retried after a 402 (the workspace's subscription is inactive).
pub const SUBSCRIPTION_RETRY: Duration = Duration::from_secs(60 * 60);

/// Retried after a 425 (the block ends in the server's future).
pub const CLOCK_AHEAD_RETRY: Duration = Duration::from_secs(5 * 60);

/// How often opening (or reading) the queue is retried after it failed.
const STORE_RETRY: Duration = Duration::from_secs(60);

/// Exponential back-off for transient failures: 30 s, doubling, at most 30
/// min — and never sooner than the server's `Retry-After`.
#[derive(Debug, Default, Clone)]
pub struct Backoff {
    failures: u32,
}

impl Backoff {
    pub const FIRST: Duration = Duration::from_secs(30);
    pub const MAX: Duration = Duration::from_secs(30 * 60);

    /// The wait after one more failure.
    pub fn next(&mut self, retry_after: Option<Duration>) -> Duration {
        let step = Self::FIRST.saturating_mul(1u32 << self.failures.min(16)).min(Self::MAX);
        self.failures = self.failures.saturating_add(1);
        step.max(retry_after.unwrap_or_default())
    }

    pub fn reset(&mut self) {
        self.failures = 0;
    }
}

/// What the worker tells the uploader.
#[derive(Debug)]
pub enum UploadCommand {
    /// Upload this account's rows from now on (`None`: stop uploading).
    Account(Option<Account>),
    /// New rows were queued: upload them, unless waiting out a back-off.
    RowsQueued,
    /// Upload now, whatever the back-off ("Sync now", the server is back).
    Now,
    /// Signing out: upload what fits before `deadline`, then stop.
    SignOut { deadline: std::time::Instant },
    /// Quitting: upload what fits before `deadline`, then end the thread.
    Shutdown { deadline: std::time::Instant },
}

impl UploadCommand {
    /// A command that must cut a running pass short: by when.
    fn deadline(&self) -> Option<std::time::Instant> {
        match self {
            UploadCommand::SignOut { deadline } | UploadCommand::Shutdown { deadline } => Some(*deadline),
            // Another account: the current pass (of the previous one) stops now.
            UploadCommand::Account(_) => Some(std::time::Instant::now()),
            UploadCommand::RowsQueued | UploadCommand::Now => None,
        }
    }
}

/// What the uploader tells the worker.
#[derive(Debug)]
///
/// The events about an account carry its owner key: a pass of the previous
/// account (a sign-out's final drain) can still report after another account
/// signed in, and must not be applied to it.
pub enum UploadEvent {
    /// The server no longer accepts the saved password of `owner`.
    SignInExpired { owner: String, reason: String },
    /// The project list of `owner`, fetched after a 403/409.
    Projects { owner: String, projects: Vec<Project> },
    /// Rows reached the server: the totals are worth refreshing.
    Uploaded(u32),
    /// Rows of `owner` were parked (rejected by the server): the project list
    /// is worth fetching again (CONTRACT §2: after any 403/409).
    Parked { owner: String, count: u32 },
}

/// How a pass ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PassEnd {
    /// Nothing left to upload.
    Drained,
    /// A row must wait; the next attempt is scheduled.
    Waiting,
    /// The sign-in was refused: uploads stopped.
    SignInExpired,
    /// The queue could not be read; retried later.
    StoreFailed,
    /// No account or queue to upload with.
    Idle,
}

enum Flow {
    Continue,
    Exit,
}

/// The uploading account and its client (which keeps its own token).
struct Uploading {
    account: Account,
    owner: String,
    client: ApiClient,
}

/// The uploader's state, owned by its thread.
pub struct Uploader {
    path: PathBuf,
    store: Option<Storage>,
    store_retry_at: Option<Instant>,
    state: StateHandle,
    events: UnboundedSender<UploadEvent>,
    clock: ServerClock,
    repaint: Arc<dyn Fn() + Send + Sync>,
    current: Option<Uploading>,
    backoff: Backoff,
    /// No upload before this (a back-off, an inactive subscription, a clock
    /// ahead of the server's).
    not_before: Option<Instant>,
}

/// The worker's side of the uploader.
pub struct UploaderHandle {
    commands: UnboundedSender<UploadCommand>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl UploaderHandle {
    pub fn send(&self, command: UploadCommand) {
        if self.commands.send(command).is_err() {
            tracing::warn!("the uploader has stopped; queued blocks upload on the next launch");
        }
    }

    /// Wait for the thread to end (after a [`UploadCommand::Shutdown`]), at
    /// most until `deadline`.
    pub async fn finish(&mut self, deadline: std::time::Instant) {
        let Some(thread) = self.thread.take() else {
            return;
        };
        while !thread.is_finished() && std::time::Instant::now() < deadline {
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        if thread.is_finished() {
            let _ = thread.join();
        } else {
            tracing::warn!("the uploader did not finish in time; the rest uploads on the next launch");
        }
    }

    /// A handle with no thread behind it: the test reads what would be sent.
    #[cfg(test)]
    pub fn detached() -> (Self, UnboundedReceiver<UploadCommand>) {
        let (commands, receiver) = tokio::sync::mpsc::unbounded_channel();
        (Self { commands, thread: None }, receiver)
    }
}

/// Start the uploader thread on the queue at `path`.
pub fn spawn(
    path: PathBuf,
    state: StateHandle,
    clock: ServerClock,
    repaint: Arc<dyn Fn() + Send + Sync>,
    events: UnboundedSender<UploadEvent>,
) -> UploaderHandle {
    let (commands, receiver) = tokio::sync::mpsc::unbounded_channel();
    let uploader = Uploader::new(path, None, state.clone(), clock, repaint.clone(), events);
    let spawned = std::thread::Builder::new().name("dosi-uploader".into()).spawn(move || {
        let runtime = match tokio::runtime::Builder::new_current_thread().enable_all().build() {
            Ok(runtime) => runtime,
            Err(e) => {
                tracing::error!(?e, "could not start the uploader runtime");
                state.update(|s| s.last_error = Some(format!("Uploads could not start: {e}")));
                repaint();
                return;
            }
        };
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            runtime.block_on(uploader.run(receiver));
        }));
        if result.is_err() {
            tracing::error!("the uploader panicked; uploads stopped");
            state.update(|s| {
                s.last_error = Some("Uploads stopped unexpectedly; your work stays queued. Please restart Dosi Tracker.".into())
            });
            repaint();
        }
    });
    let thread = match spawned {
        Ok(thread) => Some(thread),
        Err(e) => {
            tracing::error!(?e, "could not start the uploader thread");
            None
        }
    };
    UploaderHandle { commands, thread }
}

impl Uploader {
    fn new(
        path: PathBuf,
        store: Option<Storage>,
        state: StateHandle,
        clock: ServerClock,
        repaint: Arc<dyn Fn() + Send + Sync>,
        events: UnboundedSender<UploadEvent>,
    ) -> Self {
        Self {
            path,
            store,
            store_retry_at: None,
            state,
            events,
            clock,
            repaint,
            current: None,
            backoff: Backoff::default(),
            not_before: None,
        }
    }

    async fn run(mut self, mut commands: UnboundedReceiver<UploadCommand>) {
        loop {
            self.ensure_store();
            if self.ready() {
                if let Flow::Exit = self.pass_listening(&mut commands).await {
                    return;
                }
                continue;
            }
            let wake = self.next_wake();
            tokio::select! {
                command = commands.recv() => {
                    let Some(command) = command else { return };
                    if let Flow::Exit = self.apply(command).await {
                        return;
                    }
                }
                _ = tokio::time::sleep_until(wake.unwrap_or_else(Instant::now)), if wake.is_some() => {
                    if self.not_before.is_some_and(|at| at <= Instant::now()) {
                        self.not_before = None;
                    }
                }
            }
        }
    }

    /// Open the uploader's own connection to the queue (the worker opened —
    /// and if need be recovered — it first), retrying every minute.
    fn ensure_store(&mut self) {
        if self.store.is_some() || self.store_retry_at.is_some_and(|at| at > Instant::now()) {
            return;
        }
        match Storage::open(&self.path.to_string_lossy(), None) {
            Ok(store) => {
                self.store = Some(store);
                self.store_retry_at = None;
            }
            Err(e) => {
                tracing::warn!(?e, "the uploader could not open the local queue; retrying");
                self.store_retry_at = Some(Instant::now() + STORE_RETRY);
            }
        }
    }

    fn pending(&self) -> u32 {
        match (&self.store, &self.current) {
            (Some(store), Some(current)) => store.pending_count(&current.owner).unwrap_or(0),
            _ => 0,
        }
    }

    /// Is there something to upload, and may it be uploaded now?
    fn ready(&self) -> bool {
        self.not_before.is_none_or(|at| at <= Instant::now()) && self.pending() > 0
    }

    /// When the loop must wake up by itself: to retry after a wait, or to
    /// open the queue again.
    fn next_wake(&self) -> Option<Instant> {
        let retry = self.not_before.filter(|_| self.pending() > 0);
        match (retry, self.store_retry_at.filter(|_| self.store.is_none())) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        }
    }

    /// Run a pass while still taking commands: one that must cut it short
    /// (sign-out, quit, another account) bounds it; the rest are applied
    /// after it.
    async fn pass_listening(&mut self, commands: &mut UnboundedReceiver<UploadCommand>) -> Flow {
        let mut deferred = Vec::new();
        let mut deadline: Option<std::time::Instant> = None;
        {
            let pass = self.drain(None);
            tokio::pin!(pass);
            loop {
                let cut_off = deadline.map(Instant::from_std);
                tokio::select! {
                    _ = &mut pass => break,
                    command = commands.recv() => match command {
                        Some(command) => {
                            if let Some(by) = command.deadline() {
                                deadline = Some(deadline.map_or(by, |current| current.min(by)));
                            }
                            deferred.push(command);
                        }
                        None => {
                            // The worker is gone: stop at once.
                            deferred.push(UploadCommand::Shutdown { deadline: std::time::Instant::now() });
                            break;
                        }
                    },
                    _ = tokio::time::sleep_until(cut_off.unwrap_or_else(Instant::now)), if cut_off.is_some() => break,
                }
            }
        }
        for command in deferred {
            if let Flow::Exit = self.apply(command).await {
                return Flow::Exit;
            }
        }
        Flow::Continue
    }

    async fn apply(&mut self, command: UploadCommand) -> Flow {
        match command {
            UploadCommand::Account(Some(account)) => self.set_account(account),
            UploadCommand::Account(None) => self.drop_account(),
            UploadCommand::RowsQueued => {}
            UploadCommand::Now => self.not_before = None,
            UploadCommand::SignOut { deadline } => {
                self.final_drain(deadline).await;
                self.drop_account();
            }
            UploadCommand::Shutdown { deadline } => {
                self.final_drain(deadline).await;
                return Flow::Exit;
            }
        }
        Flow::Continue
    }

    fn set_account(&mut self, account: Account) {
        let unchanged = self.current.as_ref().is_some_and(|current| current.account == account);
        if !unchanged {
            let client = account.client(&self.clock);
            // Rows are only ever uploaded under the token of their owner.
            let owner = client.owner_key();
            self.current = Some(Uploading { account, owner, client });
            self.backoff.reset();
        }
        self.not_before = None;
    }

    fn drop_account(&mut self) {
        self.current = None;
        self.not_before = None;
        self.backoff.reset();
        self.state.update(|s| s.upload_issue = None);
    }

    /// One last bounded pass (quitting, signing out), whatever the back-off.
    async fn final_drain(&mut self, deadline: std::time::Instant) {
        if self.current.is_none() {
            return;
        }
        let deadline = Instant::from_std(deadline);
        if tokio::time::timeout_at(deadline, self.drain(Some(deadline))).await.is_err() {
            tracing::info!("the last uploads were cut short; they go up on the next launch");
        }
    }

    /// Upload the account's queued rows, oldest first, one at a time (each
    /// can carry a screenshot), until the queue is empty or a row must wait.
    async fn drain(&mut self, deadline: Option<Instant>) -> PassEnd {
        let (Some(store), Some(current)) = (self.store.as_ref(), self.current.as_mut()) else {
            return PassEnd::Idle;
        };
        let owner = current.owner.clone();
        let mut cursor = 0;
        let mut uploaded = 0u32;
        let mut parked = 0u32;
        // The project list, once fetched in this pass (after a 403/409).
        let mut listed: Option<Vec<Project>> = None;

        let end = loop {
            if uploaded + parked > 0 {
                // A long backlog shows its progress.
                (self.repaint)();
            }
            if deadline.is_some_and(|by| Instant::now() >= by) {
                break PassEnd::Waiting;
            }
            let row = match store.pending_batch(&owner, cursor, 1) {
                Ok(mut rows) => rows.pop(),
                Err(e) => {
                    tracing::error!(?e, "could not read the local queue");
                    self.not_before = Some(Instant::now() + STORE_RETRY);
                    break PassEnd::StoreFailed;
                }
            };
            let Some((id, activity)) = row else {
                break PassEnd::Drained;
            };
            cursor = id;

            let error = match current.client.submit_activity(&activity).await {
                Ok(()) => {
                    if let Err(e) = store.remove_synced(id) {
                        // It would be sent again at once, and again: wait a
                        // while (the server takes it back idempotently).
                        tracing::warn!(?e, id, "uploaded, but could not remove it from the queue");
                        self.not_before = Some(Instant::now() + STORE_RETRY);
                        break PassEnd::StoreFailed;
                    }
                    set_upload_state(&self.state, id, UploadState::Uploaded);
                    self.state.update(|s| {
                        s.last_sync = Some(Utc::now());
                        s.upload_issue = None;
                    });
                    self.backoff.reset();
                    uploaded += 1;
                    publish_queue(store, &self.state, Some(&owner));
                    tracing::debug!(id, "activity uploaded");
                    continue;
                }
                Err(error) => error,
            };

            let wait = match error {
                SubmitError::Rejected(reason) => {
                    if !park(store, &self.state, id, &reason) {
                        self.not_before = Some(Instant::now() + STORE_RETRY);
                        break PassEnd::StoreFailed;
                    }
                    parked += 1;
                    publish_queue(store, &self.state, Some(&owner));
                    continue;
                }
                SubmitError::StripCaptures(reason) => match store.strip_captures(id) {
                    Ok(true) => {
                        tracing::warn!(id, %reason, "the server refused the images; sending the block without them");
                        // Read the same row again, now without its images.
                        cursor = id - 1;
                        continue;
                    }
                    Ok(false) => {
                        // Nothing left to strip: the block itself is too
                        // big for the server (or a proxy in front of it).
                        Wait::Backoff { reason, retry_after: None }
                    }
                    Err(e) => Wait::Backoff { reason: format!("could not strip the images: {e}"), retry_after: None },
                },
                SubmitError::ProjectUnavailable { code, message } => {
                    if listed.is_none() {
                        match current.client.my_projects().await {
                            Ok(projects) => {
                                let _ = self.events.send(UploadEvent::Projects { owner: owner.clone(), projects: projects.clone() });
                                listed = Some(projects);
                            }
                            Err(e) if e.is_auth_rejected() => {
                                let _ = store.note_attempt(id, Some(&e.to_string()));
                                let _ = self.events.send(UploadEvent::SignInExpired { owner: owner.clone(), reason: e.to_string() });
                                break PassEnd::SignInExpired;
                            }
                            Err(e) => {
                                tracing::warn!(error = %e, "could not check the projects after {code}");
                            }
                        }
                    }
                    match &listed {
                        Some(projects) if !projects.iter().any(|p| p.id == activity.project_id) => {
                            // The project is really gone: this block can
                            // never be accepted.
                            if !park(store, &self.state, id, &message) {
                                self.not_before = Some(Instant::now() + STORE_RETRY);
                                break PassEnd::StoreFailed;
                            }
                            parked += 1;
                            publish_queue(store, &self.state, Some(&owner));
                            continue;
                        }
                        _ => Wait::Backoff { reason: message, retry_after: None },
                    }
                }
                SubmitError::Transient { reason, retry_after } => Wait::Backoff { reason, retry_after },
                SubmitError::SubscriptionInactive(message) => Wait::Subscription(message),
                SubmitError::ClockAhead(message) => Wait::ClockAhead(message),
                SubmitError::AuthRejected(reason) => {
                    let _ = store.note_attempt(id, Some(&format!("sign-in refused: {reason}")));
                    let _ = self.events.send(UploadEvent::SignInExpired { owner: owner.clone(), reason });
                    break PassEnd::SignInExpired;
                }
            };

            // Keep the row, say why, and come back later.
            let (delay, issue, note) = match wait {
                Wait::Backoff { reason, retry_after } => {
                    let delay = self.backoff.next(retry_after);
                    let next = Utc::now() + chrono::Duration::from_std(delay).unwrap_or_default();
                    (delay, UploadIssue::Retrying { next_attempt_at: next, reason: reason.clone() }, reason)
                }
                Wait::Subscription(message) => {
                    let next = Utc::now() + chrono::Duration::from_std(SUBSCRIPTION_RETRY).unwrap_or_default();
                    (SUBSCRIPTION_RETRY, UploadIssue::SubscriptionInactive { next_attempt_at: next, message: message.clone() }, message)
                }
                Wait::ClockAhead(message) => {
                    let next = Utc::now() + chrono::Duration::from_std(CLOCK_AHEAD_RETRY).unwrap_or_default();
                    (CLOCK_AHEAD_RETRY, UploadIssue::ClockAhead { next_attempt_at: next }, message)
                }
            };
            tracing::warn!(id, reason = %note, wait = ?delay, "upload must wait");
            if let Err(e) = store.note_attempt(id, Some(&note)) {
                tracing::debug!(?e, "could not note the failed attempt");
            }
            self.not_before = Some(Instant::now() + delay);
            self.state.update(|s| s.upload_issue = Some(issue));
            break PassEnd::Waiting;
        };

        if end == PassEnd::SignInExpired {
            // Nothing more until the password is entered again.
            self.current = None;
            self.state.update(|s| s.upload_issue = None);
        }
        if let Some(store) = self.store.as_ref() {
            // Uploaded rows (and their screenshots) leave free pages behind.
            if let Err(e) = store.reclaim_space() {
                tracing::warn!(?e, "could not reclaim the local queue's free space");
            }
            publish_queue(store, &self.state, Some(&owner));
        }
        if uploaded > 0 {
            let _ = self.events.send(UploadEvent::Uploaded(uploaded));
        }
        if parked > 0 {
            let _ = self.events.send(UploadEvent::Parked { owner: owner.clone(), count: parked });
        }
        (self.repaint)();
        end
    }
}

/// Why a row has to wait.
enum Wait {
    Backoff { reason: String, retry_after: Option<Duration> },
    Subscription(String),
    ClockAhead(String),
}

/// The server will never take this row: keep it (and its reason) for the
/// window, never retry it. `false` when the queue could not be updated.
fn park(store: &Storage, state: &StateHandle, id: i64, reason: &str) -> bool {
    tracing::warn!(id, %reason, "the server rejected the block; parking it");
    if let Err(e) = store.mark_rejected(id, reason) {
        tracing::error!(?e, id, "could not park the rejected block");
        return false;
    }
    set_upload_state(state, id, UploadState::Rejected);
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Activity;
    use crate::storage::RowStatus;
    use std::io::{Read, Write};

    #[test]
    fn the_backoff_doubles_from_thirty_seconds_to_thirty_minutes() {
        let mut backoff = Backoff::default();
        let waits: Vec<u64> = (0..9).map(|_| backoff.next(None).as_secs()).collect();
        assert_eq!(waits, [30, 60, 120, 240, 480, 960, 1800, 1800, 1800]);
        backoff.reset();
        assert_eq!(backoff.next(None), Duration::from_secs(30));
        // Never sooner than the server asked, even beyond the cap.
        assert_eq!(backoff.next(Some(Duration::from_secs(5))), Duration::from_secs(60));
        assert_eq!(backoff.next(Some(Duration::from_secs(7200))), Duration::from_secs(7200));
    }

    // --- Against a local canned server and a real queue ---------------------

    fn http(status: &str, headers: &[(&str, &str)], body: &str) -> String {
        let extra: String = headers.iter().map(|(k, v)| format!("{k}: {v}\r\n")).collect();
        format!(
            "HTTP/1.1 {status}\r\nContent-Length: {}\r\nContent-Type: application/json\r\nConnection: close\r\n{extra}\r\n{body}",
            body.len()
        )
    }

    fn token_ok() -> String {
        http("200 OK", &[], r#"{"access_token":"t0k3n","token_type":"Bearer","expires_in":3600}"#)
    }

    fn abp(code: &str, message: &str) -> String {
        format!(r#"{{"error":{{"code":"{code}","message":"{message}","details":null}}}}"#)
    }

    /// Read one request (headers and a Content-Length body).
    fn read_request(stream: &mut std::net::TcpStream) -> String {
        let mut data = Vec::new();
        let mut buf = [0u8; 16 * 1024];
        loop {
            let n = stream.read(&mut buf).unwrap_or(0);
            if n == 0 {
                break;
            }
            data.extend_from_slice(&buf[..n]);
            let text = String::from_utf8_lossy(&data);
            if let Some(end) = text.find("\r\n\r\n") {
                let length = text[..end]
                    .lines()
                    .find_map(|line| {
                        let (name, value) = line.split_once(':')?;
                        name.eq_ignore_ascii_case("content-length").then(|| value.trim().parse::<usize>().ok())?
                    })
                    .unwrap_or(0);
                if data.len() >= end + 4 + length {
                    break;
                }
            }
        }
        String::from_utf8_lossy(&data).into_owned()
    }

    /// Serve `responses` in order, one per connection; pass each request on.
    fn canned_server(responses: Vec<String>) -> (String, std::sync::mpsc::Receiver<String>) {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            for response in responses {
                let Ok((mut stream, _)) = listener.accept() else { return };
                let _ = tx.send(read_request(&mut stream));
                let _ = stream.write_all(response.as_bytes());
                let _ = stream.flush();
            }
        });
        (url, rx)
    }

    fn body_of(request: &str) -> serde_json::Value {
        serde_json::from_str(&request[request.find("\r\n\r\n").unwrap() + 4..]).unwrap()
    }

    struct Harness {
        uploader: Uploader,
        events: UnboundedReceiver<UploadEvent>,
        state: StateHandle,
        owner: String,
    }

    fn harness(url: &str) -> Harness {
        let (tx, events) = tokio::sync::mpsc::unbounded_channel();
        let state = StateHandle::new();
        let store = Storage::open(":memory:", None).unwrap();
        let account = Account {
            server: url.to_string(),
            workspace: "acme".into(),
            username: "alice@acme.com".into(),
            password: "pw".into(),
        };
        let owner = account.owner();
        let mut uploader = Uploader::new(PathBuf::from(":memory:"), Some(store), state.clone(), ServerClock::default(), Arc::new(|| {}), tx);
        uploader.set_account(account);
        Harness { uploader, events, state, owner }
    }

    impl Harness {
        fn queue(&self, project: &str, with_images: bool) -> i64 {
            let now = Utc::now();
            let mut activity = Activity::new(project, now - chrono::Duration::minutes(10), now);
            if with_images {
                activity.screenshot_png_base64 = Some("/9j/c2NyZWVu".into());
                activity.screenshot_thumb_jpg_base64 = Some("/9j/dGh1bWI=".into());
                activity.screenshot_captured_at = Some(now - chrono::Duration::minutes(5));
            }
            self.store().enqueue(&activity, &self.owner).unwrap()
        }

        fn store(&self) -> &Storage {
            self.uploader.store.as_ref().unwrap()
        }

        fn rows(&self) -> Vec<crate::storage::QueuedRow> {
            self.store().recent_rows(&self.owner, 10).unwrap()
        }

        /// The wait scheduled after the pass, rounded to seconds.
        fn waits_for(&self) -> Option<u64> {
            self.uploader
                .not_before
                .map(|at| (at.saturating_duration_since(Instant::now()).as_millis() as f64 / 1000.0).round() as u64)
        }
    }

    #[tokio::test]
    async fn an_oversized_upload_is_sent_again_at_once_without_its_images() {
        let (url, requests) = canned_server(vec![token_ok(), http("413 Payload Too Large", &[], ""), http("200 OK", &[], "{}")]);
        let mut h = harness(&url);
        h.queue("p1", true);

        assert_eq!(h.uploader.drain(None).await, PassEnd::Drained);
        let requests: Vec<String> = requests.try_iter().collect();
        assert_eq!(requests.len(), 3);
        assert!(body_of(&requests[1]).get("screenshotPngBase64").is_some(), "first try with the images");
        let resent = body_of(&requests[2]);
        assert!(resent.get("screenshotPngBase64").is_none() && resent.get("screenshotThumbJpgBase64").is_none());
        assert!(resent["sentAt"].is_string(), "sentAt is stamped on every attempt");
        assert!(h.rows().is_empty(), "uploaded and removed");
        assert!(matches!(h.events.try_recv(), Ok(UploadEvent::Uploaded(1))));
        assert!(h.state.snapshot().last_sync.is_some());
    }

    #[tokio::test]
    async fn a_definitive_rejection_is_parked_with_the_server_reason_and_the_queue_moves_on() {
        let overlap = abp("Tracker:ActivityOverlaps", "This block overlaps another one.");
        let (url, _requests) = canned_server(vec![token_ok(), http("409 Conflict", &[], &overlap), http("200 OK", &[], "{}")]);
        let mut h = harness(&url);
        let parked = h.queue("p1", true);
        h.queue("p1", false);

        assert_eq!(h.uploader.drain(None).await, PassEnd::Drained);
        let rows = h.rows();
        assert_eq!(rows.len(), 1, "the second row went up");
        assert_eq!(rows[0].id, parked);
        assert_eq!(rows[0].status, RowStatus::Rejected);
        assert_eq!(rows[0].reason.as_deref(), Some("This block overlaps another one."));
        let state = h.state.snapshot();
        assert_eq!((state.pending_uploads, state.rejected_uploads), (0, 1));
        assert_eq!(state.queue_rows.len(), 1, "the window lists the parked row and its reason");
        let events: Vec<UploadEvent> = std::iter::from_fn(|| h.events.try_recv().ok()).collect();
        assert!(events.iter().any(|e| matches!(e, UploadEvent::Parked { count: 1, .. })), "{events:?}");
        assert!(events.iter().any(|e| matches!(e, UploadEvent::Uploaded(1))), "{events:?}");
    }

    #[tokio::test]
    async fn a_clock_ahead_rejection_keeps_the_row_and_retries_in_five_minutes() {
        let future = abp("Tracker:ActivityInFuture", "The block ends in the future.");
        let (url, _requests) = canned_server(vec![token_ok(), http("425 Too Early", &[], &future)]);
        let mut h = harness(&url);
        h.queue("p1", false);

        assert_eq!(h.uploader.drain(None).await, PassEnd::Waiting);
        assert_eq!(h.rows()[0].status, RowStatus::Queued);
        assert_eq!(h.rows()[0].reason.as_deref(), Some("The block ends in the future."));
        assert_eq!(h.waits_for(), Some(300));
        assert!(matches!(h.state.snapshot().upload_issue, Some(UploadIssue::ClockAhead { .. })));
        assert!(!h.uploader.ready(), "waiting");
    }

    #[tokio::test]
    async fn server_trouble_backs_off_and_honours_retry_after() {
        let (url, _requests) = canned_server(vec![
            token_ok(),
            http("503 Service Unavailable", &[], "<html>down</html>"),
            http("429 Too Many Requests", &[("Retry-After", "120")], ""),
            http("200 OK", &[], "{}"),
        ]);
        let mut h = harness(&url);
        h.queue("p1", false);

        assert_eq!(h.uploader.drain(None).await, PassEnd::Waiting);
        assert_eq!(h.waits_for(), Some(30));
        match h.state.snapshot().upload_issue {
            Some(UploadIssue::Retrying { reason, .. }) => assert_eq!(reason, "HTTP 503"),
            other => panic!("expected a retry, got {other:?}"),
        }
        assert_eq!(h.rows()[0].status, RowStatus::Queued, "kept");

        // "Sync now" retries at once; the server asks for two minutes.
        h.uploader.apply(UploadCommand::Now).await;
        assert!(h.uploader.ready());
        assert_eq!(h.uploader.drain(None).await, PassEnd::Waiting);
        assert_eq!(h.waits_for(), Some(120), "Retry-After beats the 60 s step");

        h.uploader.apply(UploadCommand::Now).await;
        assert_eq!(h.uploader.drain(None).await, PassEnd::Drained);
        assert!(h.state.snapshot().upload_issue.is_none(), "cleared by the success");
        assert_eq!(h.uploader.backoff.next(None), Duration::from_secs(30), "and the back-off starts over");
    }

    #[tokio::test]
    async fn an_inactive_subscription_keeps_the_row_and_retries_hourly() {
        let inactive = abp("Tracker:SubscriptionInactive", "The subscription is inactive.");
        let (url, _requests) = canned_server(vec![token_ok(), http("402 Payment Required", &[], &inactive)]);
        let mut h = harness(&url);
        h.queue("p1", false);
        assert_eq!(h.uploader.drain(None).await, PassEnd::Waiting);
        assert_eq!(h.waits_for(), Some(3600));
        assert!(matches!(h.state.snapshot().upload_issue, Some(UploadIssue::SubscriptionInactive { .. })));
        assert_eq!(h.rows()[0].status, RowStatus::Queued);
    }

    #[tokio::test]
    async fn a_block_of_a_project_that_is_gone_is_parked_after_checking_the_projects() {
        let not_member = abp("Tracker:NotProjectMember", "You are not a member of this project.");
        let others = r#"[{"id":"other","title":"Other","intervalMinutes":10,"allowScreenshot":true,"allowWebcam":false,
            "allowKeyboard":true,"allowMouse":true,"allowActiveWindow":true,"allowRunningPrograms":true}]"#;
        let (url, requests) = canned_server(vec![
            token_ok(),
            http("403 Forbidden", &[], &not_member),
            http("200 OK", &[], others),
            // The second row, of the same vanished project: no second lookup.
            http("403 Forbidden", &[], &not_member),
        ]);
        let mut h = harness(&url);
        h.queue("gone", false);
        h.queue("gone", false);

        assert_eq!(h.uploader.drain(None).await, PassEnd::Drained);
        let rows = h.rows();
        assert!(rows.iter().all(|r| r.status == RowStatus::Rejected), "{rows:?}");
        assert_eq!(rows[0].reason.as_deref(), Some("You are not a member of this project."));
        match h.events.try_recv() {
            Ok(UploadEvent::Projects { projects, .. }) => assert_eq!(projects[0].id, "other"),
            other => panic!("expected the refreshed projects, got {other:?}"),
        }
        let requests: Vec<String> = requests.try_iter().collect();
        assert_eq!(requests.iter().filter(|r| r.starts_with("GET /api/app/project/my-projects")).count(), 1);
    }

    #[tokio::test]
    async fn a_block_of_a_project_still_listed_is_kept() {
        let archived = abp("Tracker:ProjectArchived", "The project is archived.");
        let listed = r#"[{"id":"p1","title":"P1","intervalMinutes":10,"allowScreenshot":true,"allowWebcam":false,
            "allowKeyboard":true,"allowMouse":true,"allowActiveWindow":true,"allowRunningPrograms":true}]"#;
        let (url, _requests) = canned_server(vec![token_ok(), http("409 Conflict", &[], &archived), http("200 OK", &[], listed)]);
        let mut h = harness(&url);
        h.queue("p1", false);
        assert_eq!(h.uploader.drain(None).await, PassEnd::Waiting);
        assert_eq!(h.rows()[0].status, RowStatus::Queued);
        assert_eq!(h.waits_for(), Some(30));
    }

    #[tokio::test]
    async fn a_refused_password_stops_uploads_and_tells_the_worker() {
        let refused = r#"{"error":"invalid_grant","error_description":"Invalid username or password!"}"#;
        let (url, _requests) = canned_server(vec![http("400 Bad Request", &[], refused)]);
        let mut h = harness(&url);
        h.queue("p1", false);

        assert_eq!(h.uploader.drain(None).await, PassEnd::SignInExpired);
        match h.events.try_recv() {
            Ok(UploadEvent::SignInExpired { reason, .. }) => assert_eq!(reason, "Invalid username or password!"),
            other => panic!("expected sign-in expired, got {other:?}"),
        }
        assert!(h.uploader.current.is_none(), "no more attempts until the password is entered again");
        assert_eq!(h.rows()[0].status, RowStatus::Queued, "the queue is kept");
        assert!(!h.uploader.ready());
        assert_eq!(h.uploader.drain(None).await, PassEnd::Idle);
    }

    #[tokio::test]
    async fn rows_of_another_account_are_never_uploaded() {
        let (url, requests) = canned_server(vec![token_ok()]);
        let mut h = harness(&url);
        let bob = crate::storage::owner_key("acme", "bob@acme.com");
        let now = Utc::now();
        h.store().enqueue(&Activity::new("p1", now - chrono::Duration::minutes(10), now), &bob).unwrap();
        assert!(!h.uploader.ready());
        assert_eq!(h.uploader.drain(None).await, PassEnd::Drained);
        assert_eq!(requests.try_iter().count(), 0, "nothing was sent");
    }

    #[tokio::test]
    async fn a_pass_past_its_deadline_stops_before_the_next_row() {
        let (url, requests) = canned_server(vec![token_ok()]);
        let mut h = harness(&url);
        h.queue("p1", false);
        assert_eq!(h.uploader.drain(Some(Instant::now())).await, PassEnd::Waiting);
        assert_eq!(requests.try_iter().count(), 0);
        assert_eq!(h.rows()[0].status, RowStatus::Queued);
    }
}
