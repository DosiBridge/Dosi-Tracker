//! The `dosi-uploader` thread: per-scope sync passes over the outbox with the
//! Windows back-off policy and park-and-continue.
//!
//! Ported from the Windows `worker/uploader.rs` policy, driving the
//! [`SyncPassCoordinator`] one row at a time with one [`HttpDeliveryTransport`]
//! per pending scope (all sharing the account's [`crate::api::TokenCell`]).
//! The thread has its own connection to the outbox, so a long backlog on a
//! slow uplink never delays pausing, quitting, or saving the block before the
//! machine sleeps. It never writes the shared state: everything it learns
//! goes to the worker as an [`UploadEvent`].
//!
//! The answers are handled as design 8.4 says, oldest row first per scope:
//!
//! | answer | what happens |
//! |---|---|
//! | 2xx | the row leaves the queue |
//! | network, 5xx, 404/405/408/429, a 4xx without a code | kept; retried after 30 s, doubling up to 30 min (`Retry-After` honoured, capped at 6 h) |
//! | sign-in refused | kept; uploads stop, the worker asks for the password |
//! | 402 subscription inactive | kept; retried hourly |
//! | 425 in the future | kept; retried in 5 minutes (the clock is ahead) |
//! | 403 not a project member / 409 archived | the transport re-checks the projects: parked if the project is gone, else kept |
//! | 413, capture too large/invalid | the images are stripped and it is sent again at once |
//! | overlap, id conflict, too long/old, invalid range, validation, local integrity | parked with the reason; **the queue moves on** |
//! | 403 not a workspace member / archived, 400 workspace required | that scope's queued rows are parked (cross-lane contract: terminal); other scopes go on |
//! | 403 device revoked | that scope's queued rows are parked; other scopes go on |
//!
//! Each row is sent to the workspace it was recorded in (`X-Workspace-Id`),
//! whichever workspace is tracked now; other accounts' rows are never sent.
//! `sentAt` is stamped by the API client on every attempt.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender};
use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};
use tokio::sync::mpsc::UnboundedSender;

use super::state::{UploadIssue, UploadState};
use super::worker::Account;
use crate::api::{HttpDeliveryTransport, ServerClock, TransportConfig, MAX_RETRY_AFTER};
use crate::outbox::{Outbox, OutboxLimits, OutboxScope};
use crate::sync::{RetryKind, ScopeBlock, SyncPassConfig, SyncPassCoordinator, SyncStopReason};

/// Retried after a 402 (the workspace's subscription is inactive).
pub const SUBSCRIPTION_RETRY: Duration = Duration::from_secs(60 * 60);

/// Retried after a 425 (the block ends in the server's future).
pub const CLOCK_AHEAD_RETRY: Duration = Duration::from_secs(5 * 60);

/// How often opening (or reading) the queue is retried after it failed.
pub const STORE_RETRY: Duration = Duration::from_secs(60);

/// Exponential back-off for transient failures: 30 s, doubling, at most 30
/// min — and never sooner than the server's `Retry-After` (itself capped at
/// [`MAX_RETRY_AFTER`]).
#[derive(Debug, Default, Clone)]
pub struct Backoff {
    failures: u32,
}

impl Backoff {
    pub const FIRST: Duration = Duration::from_secs(30);
    pub const MAX: Duration = Duration::from_secs(30 * 60);

    /// The wait after one more failure.
    pub fn next(&mut self, retry_after: Option<Duration>) -> Duration {
        let step = Self::FIRST
            .saturating_mul(1u32 << self.failures.min(16))
            .min(Self::MAX);
        self.failures = self.failures.saturating_add(1);
        step.max(retry_after.unwrap_or_default().min(MAX_RETRY_AFTER))
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
    SignOut { deadline: Instant },
    /// Quitting: upload what fits before `deadline`, then end the thread.
    Shutdown { deadline: Instant },
    /// The workspace revoked this device: park its queued rows.
    ParkScope {
        workspace_id: String,
        reason: String,
    },
}

impl UploadCommand {
    /// A command that must cut a running pass short: by when.
    fn deadline(&self) -> Option<Instant> {
        match self {
            UploadCommand::SignOut { deadline } | UploadCommand::Shutdown { deadline } => {
                Some(*deadline)
            }
            // Another account: the current pass (of the previous one) stops now.
            UploadCommand::Account(_) | UploadCommand::ParkScope { .. } => Some(Instant::now()),
            UploadCommand::RowsQueued | UploadCommand::Now => None,
        }
    }
}

/// What the uploader tells the worker.
///
/// The events about an account carry its owner key: a pass of the previous
/// account (a sign-out's final drain) can still report after another account
/// signed in, and must not be applied to it.
#[derive(Debug, Clone, PartialEq)]
pub enum UploadEvent {
    /// The server no longer accepts the saved sign-in of `owner`.
    SignInExpired { owner: String, reason: String },
    /// Rows reached the server: the totals are worth refreshing.
    Uploaded(u32),
    /// Rows of `owner` were parked: the project list is worth fetching again
    /// (CONTRACT §2: after any 403/409), and the user is told.
    Parked { owner: String, count: u32 },
    /// Rows of `owner` were sent again without their screenshots.
    Stripped { owner: String, count: u32 },
    /// `owner` lost access to `workspace_id` (its rows were parked).
    WorkspaceLost { owner: String, workspace_id: String },
    /// `workspace_id` revoked this device (its rows were parked).
    DeviceRevoked { owner: String, workspace_id: String },
    /// Why uploads are held back now (`None`: they are not).
    Issue(Option<UploadIssue>),
    /// A row reached the server at this time.
    Synced(DateTime<Utc>),
    /// Where the queued row `row_id` is now.
    RowState { row_id: i64, state: UploadState },
    /// The queue's counts changed.
    QueueChanged,
    /// The uploader stopped for good; the queue stays.
    Failed(String),
}

/// How a pass ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PassEnd {
    /// Nothing left to upload.
    Drained,
    /// A row must wait (or the pass was cut short); the next attempt is scheduled.
    Waiting,
    /// The sign-in was refused: uploads stopped.
    SignInExpired,
    /// The queue could not be read or updated; retried later.
    StoreFailed,
    /// No account or queue to upload with.
    Idle,
}

enum Flow {
    Continue,
    Exit,
}

/// Why a row has to wait.
enum Wait {
    Backoff {
        reason: String,
        retry_after: Option<Duration>,
    },
    Subscription(String),
    ClockAhead(String),
}

/// The uploading account.
struct Uploading {
    account: Account,
    owner: String,
}

/// The uploader's state, owned by its thread.
pub struct Uploader {
    path: PathBuf,
    store: Option<Outbox>,
    store_retry_at: Option<Instant>,
    events: UnboundedSender<UploadEvent>,
    clock: ServerClock,
    current: Option<Uploading>,
    /// One transport per scope of the current account, made on first use.
    transports: HashMap<OutboxScope, HttpDeliveryTransport>,
    backoff: Backoff,
    /// No upload before this (a back-off, an inactive subscription, a clock
    /// ahead of the server's).
    not_before: Option<Instant>,
}

/// The worker's side of the uploader.
pub struct UploaderHandle {
    commands: Sender<UploadCommand>,
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
    pub async fn finish(&mut self, deadline: Instant) {
        let Some(thread) = self.thread.take() else {
            return;
        };
        while !thread.is_finished() && Instant::now() < deadline {
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        if thread.is_finished() {
            let _ = thread.join();
        } else {
            tracing::warn!(
                "the uploader did not finish in time; the rest uploads on the next launch"
            );
        }
    }

    /// A handle with no thread behind it: the test reads what would be sent.
    #[cfg(test)]
    pub fn detached() -> (Self, Receiver<UploadCommand>) {
        let (commands, receiver) = std::sync::mpsc::channel();
        (
            Self {
                commands,
                thread: None,
            },
            receiver,
        )
    }
}

/// Start the uploader thread on the queue at `path`.
pub fn spawn(
    path: PathBuf,
    clock: ServerClock,
    events: UnboundedSender<UploadEvent>,
) -> UploaderHandle {
    let (commands, receiver) = std::sync::mpsc::channel();
    let uploader = Uploader::new(path, None, clock, events.clone());
    let spawned = std::thread::Builder::new()
        .name("dosi-uploader".into())
        .spawn(move || {
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                uploader.run(receiver);
            }));
            if result.is_err() {
                tracing::error!("the uploader panicked; uploads stopped");
                let _ = events.send(UploadEvent::Failed(
                    "Uploads stopped unexpectedly; your work stays queued. Please restart Dosi Tracker."
                        .into(),
                ));
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
        store: Option<Outbox>,
        clock: ServerClock,
        events: UnboundedSender<UploadEvent>,
    ) -> Self {
        Self {
            path,
            store,
            store_retry_at: None,
            events,
            clock,
            current: None,
            transports: HashMap::new(),
            backoff: Backoff::default(),
            not_before: None,
        }
    }

    fn emit(&self, event: UploadEvent) {
        let _ = self.events.send(event);
    }

    fn run(mut self, commands: Receiver<UploadCommand>) {
        loop {
            self.ensure_store();
            if self.ready() {
                if let Flow::Exit = self.pass_listening(&commands) {
                    return;
                }
                continue;
            }
            let command = match self.next_wake() {
                Some(wake) => commands.recv_timeout(wake.saturating_duration_since(Instant::now())),
                None => commands.recv().map_err(|_| RecvTimeoutError::Disconnected),
            };
            match command {
                Ok(command) => {
                    if let Flow::Exit = self.apply(command) {
                        return;
                    }
                }
                Err(RecvTimeoutError::Timeout) => {
                    if self.not_before.is_some_and(|at| at <= Instant::now()) {
                        self.not_before = None;
                    }
                }
                Err(RecvTimeoutError::Disconnected) => return,
            }
        }
    }

    /// Open the uploader's own connection to the queue (the worker opened —
    /// and if need be recovered — it first), retrying every minute.
    fn ensure_store(&mut self) {
        if self.store.is_some() || self.store_retry_at.is_some_and(|at| at > Instant::now()) {
            return;
        }
        match Outbox::open(&self.path, OutboxLimits::default()) {
            Ok(store) => {
                self.store = Some(store);
                self.store_retry_at = None;
            }
            Err(e) => {
                tracing::warn!(error = %e, "the uploader could not open the local queue; retrying");
                self.store_retry_at = Some(Instant::now() + STORE_RETRY);
            }
        }
    }

    fn pending(&self) -> u64 {
        match (&self.store, &self.current) {
            (Some(store), Some(current)) => store
                .queue_summary(&current.owner)
                .map(|s| s.pending)
                .unwrap_or(0),
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

    /// Run a pass while still taking commands between rows: one that must
    /// cut it short (sign-out, quit, another account) bounds it; all are
    /// applied after it.
    fn pass_listening(&mut self, commands: &Receiver<UploadCommand>) -> Flow {
        let mut deferred = Vec::new();
        let mut cut_off: Option<Instant> = None;
        let mut gone = false;
        self.drain(None, &mut || {
            loop {
                match commands.try_recv() {
                    Ok(command) => {
                        if let Some(by) = command.deadline() {
                            cut_off = Some(cut_off.map_or(by, |current| current.min(by)));
                        }
                        deferred.push(command);
                    }
                    Err(std::sync::mpsc::TryRecvError::Empty) => break,
                    Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                        gone = true;
                        break;
                    }
                }
            }
            gone || cut_off.is_some_and(|by| Instant::now() >= by)
        });
        if gone {
            // The worker is gone: stop at once.
            deferred.push(UploadCommand::Shutdown {
                deadline: Instant::now(),
            });
        }
        for command in deferred {
            if let Flow::Exit = self.apply(command) {
                return Flow::Exit;
            }
        }
        Flow::Continue
    }

    fn apply(&mut self, command: UploadCommand) -> Flow {
        match command {
            UploadCommand::Account(Some(account)) => self.set_account(account),
            UploadCommand::Account(None) => self.drop_account(),
            UploadCommand::RowsQueued => {}
            UploadCommand::Now => self.not_before = None,
            UploadCommand::SignOut { deadline } => {
                self.final_drain(deadline);
                self.drop_account();
            }
            UploadCommand::Shutdown { deadline } => {
                self.final_drain(deadline);
                return Flow::Exit;
            }
            UploadCommand::ParkScope {
                workspace_id,
                reason,
            } => self.park_workspace(&workspace_id, &reason),
        }
        Flow::Continue
    }

    fn set_account(&mut self, account: Account) {
        let unchanged = self
            .current
            .as_ref()
            .is_some_and(|current| current.account == account);
        if !unchanged {
            // Rows are only ever uploaded under the token of their owner.
            let owner = account.owner();
            self.current = Some(Uploading { account, owner });
            self.transports.clear();
            self.backoff.reset();
        }
        self.not_before = None;
    }

    fn drop_account(&mut self) {
        self.current = None;
        self.transports.clear();
        self.not_before = None;
        self.backoff.reset();
        self.emit(UploadEvent::Issue(None));
    }

    /// Park every queued row of the current account in `workspace_id`.
    fn park_workspace(&mut self, workspace_id: &str, reason: &str) {
        let (Some(store), Some(current)) = (self.store.as_ref(), self.current.as_ref()) else {
            return;
        };
        let Ok(scope) = OutboxScope::new(current.owner.clone(), workspace_id) else {
            return;
        };
        match store.park_scope(&scope, reason) {
            Ok(parked) => {
                tracing::warn!(parked, %workspace_id, %reason, "queued rows of the workspace parked");
                self.transports.remove(&scope);
                self.emit(UploadEvent::QueueChanged);
            }
            Err(e) => tracing::error!(error = %e, "could not park the workspace's rows"),
        }
    }

    /// One last bounded pass (quitting, signing out), whatever the back-off.
    fn final_drain(&mut self, deadline: Instant) {
        if self.current.is_none() {
            return;
        }
        if self.drain(Some(deadline), &mut || false) == PassEnd::Waiting {
            tracing::info!("the last uploads were cut short; they go up on the next launch");
        }
    }

    /// The transport of `scope` (made on first use), taken out of the cache
    /// for one row; [`Self::drain`] puts it back.
    fn take_transport(&mut self, scope: &OutboxScope) -> Result<HttpDeliveryTransport, String> {
        if let Some(transport) = self.transports.remove(scope) {
            return Ok(transport);
        }
        let Some(current) = self.current.as_ref() else {
            return Err("no account to upload with".into());
        };
        let account = &current.account;
        HttpDeliveryTransport::new_shared(TransportConfig {
            server: account.server.clone(),
            account: current.owner.clone(),
            // Never a silent password sign-in (refused refresh = expired).
            password: None,
            tokens: account.tokens.clone(),
            workspace_id: scope.workspace_id.clone(),
            device_id: account.device_id.clone(),
            signing_key: account.signing_key,
            clock: self.clock.clone(),
            allow_insecure_http: account.allow_insecure_http,
        })
    }

    /// Upload the account's queued rows, scope by scope and oldest first, one
    /// at a time (each can carry a screenshot), until the queue is empty, a
    /// row must wait, `deadline` passes or `interrupt` says to stop.
    fn drain(&mut self, deadline: Option<Instant>, interrupt: &mut dyn FnMut() -> bool) -> PassEnd {
        let Some(owner) = self.current.as_ref().map(|c| c.owner.clone()) else {
            return PassEnd::Idle;
        };
        let tracked = self
            .current
            .as_ref()
            .and_then(|c| c.account.workspace_id.clone());
        let scopes = match self
            .store
            .as_ref()
            .map(|store| store.pending_scopes(&owner))
        {
            None => return PassEnd::Idle,
            Some(Ok(scopes)) => scopes,
            Some(Err(e)) => {
                tracing::error!(error = %e, "could not read the local queue");
                self.not_before = Some(Instant::now() + STORE_RETRY);
                return PassEnd::StoreFailed;
            }
        };
        let (mut uploaded, mut parked, mut stripped) = (0u32, 0u32, 0u32);
        let mut end = PassEnd::Drained;
        'scopes: for scope in scopes {
            if uuid::Uuid::parse_str(scope.workspace_id.trim()).is_err() {
                // Queued by the foundation build under a workspace that is not
                // a GUID: the server can never take these rows.
                match self
                    .store
                    .as_ref()
                    .map(|s| s.park_scope(&scope, "the workspace id is not a GUID"))
                {
                    Some(Ok(count)) => parked += count as u32,
                    Some(Err(e)) => {
                        tracing::error!(error = %e, "could not park the rows of an invalid workspace")
                    }
                    None => {}
                }
                continue;
            }
            loop {
                if deadline.is_some_and(|by| Instant::now() >= by) || interrupt() {
                    end = PassEnd::Waiting;
                    break 'scopes;
                }
                let head = match self.store.as_ref().map(|s| s.peek_ready(&scope, 1)) {
                    Some(Ok(mut rows)) => rows.pop(),
                    Some(Err(e)) => {
                        tracing::error!(error = %e, "could not read the local queue");
                        self.not_before = Some(Instant::now() + STORE_RETRY);
                        end = PassEnd::StoreFailed;
                        break 'scopes;
                    }
                    None => break 'scopes,
                };
                let Some(head) = head else {
                    // This scope is drained.
                    break;
                };
                let config = SyncPassConfig {
                    max_attempts: 1,
                    batch_limit: 1,
                    deadline,
                };
                let mut transport = match self.take_transport(&scope) {
                    Ok(transport) => transport,
                    Err(reason) => {
                        self.wait(Wait::Backoff {
                            reason,
                            retry_after: None,
                        });
                        end = PassEnd::Waiting;
                        break 'scopes;
                    }
                };
                let Some(store) = self.store.as_ref() else {
                    break 'scopes;
                };
                let report =
                    SyncPassCoordinator::new(store, config).run_pass(&scope, &mut transport);
                self.transports.insert(scope.clone(), transport);
                let report = match report {
                    Ok(report) => report,
                    Err(e) => {
                        tracing::error!(error = %e, "the sync pass failed");
                        self.not_before = Some(Instant::now() + STORE_RETRY);
                        end = PassEnd::StoreFailed;
                        break 'scopes;
                    }
                };
                if report
                    .persisted_acknowledged
                    .contains(&head.client_activity_id)
                {
                    uploaded += 1;
                    self.backoff.reset();
                    self.emit(UploadEvent::RowState {
                        row_id: head.id,
                        state: UploadState::Uploaded,
                    });
                    self.emit(UploadEvent::Synced(Utc::now()));
                    self.emit(UploadEvent::Issue(None));
                    tracing::debug!(id = head.id, "activity uploaded");
                }
                for row in &report.parked {
                    tracing::warn!(reason = %row.reason, "the server will not take the block; parked");
                    parked += 1;
                    if row.client_activity_id == head.client_activity_id {
                        self.emit(UploadEvent::RowState {
                            row_id: head.id,
                            state: UploadState::Rejected,
                        });
                    }
                }
                stripped += report.stripped.len() as u32;
                match report.stop_reason {
                    None | Some(SyncStopReason::MaxAttemptsReached { .. }) => {}
                    Some(SyncStopReason::DeadlineReached) => {
                        end = PassEnd::Waiting;
                        break 'scopes;
                    }
                    Some(SyncStopReason::RetryLater {
                        reason,
                        kind,
                        retry_after,
                        ..
                    }) => {
                        self.wait(match kind {
                            RetryKind::Transient => Wait::Backoff {
                                reason,
                                retry_after,
                            },
                            RetryKind::Subscription => Wait::Subscription(reason),
                            RetryKind::ClockAhead => Wait::ClockAhead(reason),
                        });
                        end = PassEnd::Waiting;
                        break 'scopes;
                    }
                    Some(
                        SyncStopReason::TransientFailure { reason, .. }
                        | SyncStopReason::DefinitiveRejection { reason, .. },
                    ) => {
                        self.wait(Wait::Backoff {
                            reason,
                            retry_after: None,
                        });
                        end = PassEnd::Waiting;
                        break 'scopes;
                    }
                    Some(SyncStopReason::MismatchedAcknowledgement { expected, .. }) => {
                        self.wait(Wait::Backoff {
                            reason: format!(
                                "the server acknowledged another block than {expected}"
                            ),
                            retry_after: None,
                        });
                        end = PassEnd::Waiting;
                        break 'scopes;
                    }
                    Some(SyncStopReason::AuthRequired { reason, .. }) => {
                        self.emit(UploadEvent::SignInExpired {
                            owner: owner.clone(),
                            reason,
                        });
                        end = PassEnd::SignInExpired;
                        break 'scopes;
                    }
                    Some(SyncStopReason::ScopeBlocked { reason, block, .. }) => {
                        // Terminal for this workspace: its queued rows are
                        // parked (kept 14 days), the other workspaces go on.
                        let label = match block {
                            ScopeBlock::WorkspaceAccessLost => {
                                format!("workspace access lost: {reason}")
                            }
                            ScopeBlock::DeviceRevoked => "device revoked".to_owned(),
                        };
                        match self.store.as_ref().map(|s| s.park_scope(&scope, &label)) {
                            Some(Ok(count)) => parked += count as u32,
                            Some(Err(e)) => {
                                tracing::error!(error = %e, "could not park the workspace's rows");
                                self.not_before = Some(Instant::now() + STORE_RETRY);
                                end = PassEnd::StoreFailed;
                                break 'scopes;
                            }
                            None => {}
                        }
                        self.transports.remove(&scope);
                        let workspace_id = scope.workspace_id.clone();
                        match block {
                            ScopeBlock::WorkspaceAccessLost => {
                                tracing::warn!(%workspace_id, "no access to the rows' workspace; its rows are parked");
                                if tracked.as_deref() == Some(workspace_id.as_str()) {
                                    self.emit(UploadEvent::WorkspaceLost {
                                        owner: owner.clone(),
                                        workspace_id,
                                    });
                                }
                            }
                            ScopeBlock::DeviceRevoked => {
                                tracing::warn!(%workspace_id, "this device was revoked; the workspace's rows are parked");
                                self.emit(UploadEvent::DeviceRevoked {
                                    owner: owner.clone(),
                                    workspace_id,
                                });
                            }
                        }
                        break;
                    }
                    Some(
                        SyncStopReason::StoreError(_)
                        | SyncStopReason::AcknowledgementPersistenceFailed { .. }
                        | SyncStopReason::RetryMetadataPersistenceFailed { .. }
                        | SyncStopReason::ParkPersistenceFailed { .. },
                    ) => {
                        tracing::error!(stop = ?report.stop_reason, "the local queue could not be updated");
                        // Sent again later: the server takes it back idempotently.
                        self.not_before = Some(Instant::now() + STORE_RETRY);
                        end = PassEnd::StoreFailed;
                        break 'scopes;
                    }
                }
            }
        }

        if end == PassEnd::SignInExpired {
            // Nothing more until the password is entered again.
            self.current = None;
            self.transports.clear();
            self.emit(UploadEvent::Issue(None));
        }
        if uploaded + parked > 0 {
            if let Some(store) = self.store.as_ref() {
                // Uploaded rows (and their screenshots) leave free pages behind.
                if let Err(e) = store.reclaim_space() {
                    tracing::warn!(error = %e, "could not reclaim the local queue's free space");
                }
            }
        }
        self.emit(UploadEvent::QueueChanged);
        if uploaded > 0 {
            self.emit(UploadEvent::Uploaded(uploaded));
        }
        if parked > 0 {
            self.emit(UploadEvent::Parked {
                owner: owner.clone(),
                count: parked,
            });
        }
        if stripped > 0 {
            self.emit(UploadEvent::Stripped {
                owner,
                count: stripped,
            });
        }
        end
    }

    /// Keep the row, say why, and come back later.
    fn wait(&mut self, wait: Wait) {
        let at =
            |delay: Duration| Utc::now() + chrono::Duration::from_std(delay).unwrap_or_default();
        let (delay, issue, note) = match wait {
            Wait::Backoff {
                reason,
                retry_after,
            } => {
                let delay = self.backoff.next(retry_after);
                (
                    delay,
                    UploadIssue::Retrying {
                        next_attempt_at: at(delay),
                        reason: reason.clone(),
                    },
                    reason,
                )
            }
            Wait::Subscription(message) => (
                SUBSCRIPTION_RETRY,
                UploadIssue::SubscriptionInactive {
                    next_attempt_at: at(SUBSCRIPTION_RETRY),
                    message: message.clone(),
                },
                message,
            ),
            Wait::ClockAhead(message) => (
                CLOCK_AHEAD_RETRY,
                UploadIssue::ClockAhead {
                    next_attempt_at: at(CLOCK_AHEAD_RETRY),
                },
                message,
            ),
        };
        tracing::warn!(reason = %note, wait = ?delay, "upload must wait");
        self.not_before = Some(Instant::now() + delay);
        self.emit(UploadEvent::Issue(Some(issue)));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::model::Activity;
    use crate::outbox::{CaptureKind, PendingActivity, PendingCapture, RowState};
    use std::io::{Read, Write};
    use tokio::sync::mpsc::UnboundedReceiver;

    const W1: &str = "11111111-1111-4111-8111-111111111111";
    const W2: &str = "22222222-2222-4222-8222-222222222222";
    const P1: &str = "aaaaaaaa-0000-4000-8000-000000000001";
    const P2: &str = "aaaaaaaa-0000-4000-8000-000000000002";
    const GONE: &str = "aaaaaaaa-0000-4000-8000-0000000000ff";
    const KEY: [u8; 32] = [7u8; 32];

    #[test]
    fn the_backoff_doubles_from_thirty_seconds_to_thirty_minutes() {
        let mut backoff = Backoff::default();
        let waits: Vec<u64> = (0..9).map(|_| backoff.next(None).as_secs()).collect();
        assert_eq!(waits, [30, 60, 120, 240, 480, 960, 1800, 1800, 1800]);
        backoff.reset();
        assert_eq!(backoff.next(None), Duration::from_secs(30));
        // Never sooner than the server asked, even beyond the cap...
        assert_eq!(
            backoff.next(Some(Duration::from_secs(5))),
            Duration::from_secs(60)
        );
        assert_eq!(
            backoff.next(Some(Duration::from_secs(7200))),
            Duration::from_secs(7200)
        );
        // ...but a Retry-After beyond 6 h is capped there.
        assert_eq!(
            backoff.next(Some(Duration::from_secs(30 * 3600))),
            MAX_RETRY_AFTER
        );
    }

    #[test]
    fn commands_that_cut_a_pass_short_say_by_when() {
        let soon = Instant::now() + Duration::from_secs(15);
        assert_eq!(
            UploadCommand::SignOut { deadline: soon }.deadline(),
            Some(soon)
        );
        assert_eq!(
            UploadCommand::Shutdown { deadline: soon }.deadline(),
            Some(soon)
        );
        assert!(UploadCommand::Account(None).deadline().is_some());
        assert!(UploadCommand::RowsQueued.deadline().is_none());
        assert!(UploadCommand::Now.deadline().is_none());
    }

    // --- Against a local canned server and a real queue ---------------------

    fn http(status: &str, headers: &[(&str, &str)], body: &str) -> String {
        let extra: String = headers
            .iter()
            .map(|(k, v)| format!("{k}: {v}\r\n"))
            .collect();
        format!(
            "HTTP/1.1 {status}\r\nContent-Length: {}\r\nContent-Type: application/json\r\nConnection: close\r\n{extra}\r\n{body}",
            body.len()
        )
    }

    fn token_ok() -> String {
        http(
            "200 OK",
            &[],
            r#"{"access_token":"t0k3n","token_type":"Bearer","expires_in":3600,"refresh_token":"r2"}"#,
        )
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
                        name.eq_ignore_ascii_case("content-length")
                            .then(|| value.trim().parse::<usize>().ok())?
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
                let Ok((mut stream, _)) = listener.accept() else {
                    return;
                };
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

    fn header_of(request: &str, name: &str) -> Option<String> {
        let head = &request[..request.find("\r\n\r\n").unwrap_or(request.len())];
        head.lines().find_map(|line| {
            let (key, value) = line.split_once(':')?;
            key.trim()
                .eq_ignore_ascii_case(name)
                .then(|| value.trim().to_owned())
        })
    }

    struct Harness {
        uploader: Uploader,
        events: UnboundedReceiver<UploadEvent>,
        owner: String,
        _dir: tempfile::TempDir,
    }

    fn account(url: &str) -> Account {
        let mut account =
            Account::new(url.to_string(), "alice@acme.com".into(), "").in_workspace(W1, "Acme");
        account.tokens = crate::api::TokenCell::with_refresh_token(Some("r1".into()));
        account.signing_key = Some(KEY);
        account.device_id = Some("5b0c1b9e-8f2c-4a43-9a51-0c7f6f2d9b11".into());
        account
    }

    fn harness(url: &str) -> Harness {
        let (tx, events) = tokio::sync::mpsc::unbounded_channel();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("outbox.db");
        let store = Outbox::open(&path, OutboxLimits::default()).unwrap();
        let account = account(url);
        let owner = account.owner();
        let mut uploader = Uploader::new(path, Some(store), ServerClock::default(), tx);
        uploader.set_account(account);
        Harness {
            uploader,
            events,
            owner,
            _dir: dir,
        }
    }

    impl Harness {
        fn queue(&self, project: &str, with_images: bool) -> i64 {
            let now = Utc::now();
            let activity = Activity::new(project, now - chrono::Duration::minutes(10), now);
            self.queue_in(W1, activity, with_images)
        }

        fn queue_in(&self, workspace: &str, activity: Activity, with_images: bool) -> i64 {
            let captures = if with_images {
                let at = activity.started_at + chrono::Duration::minutes(5);
                vec![
                    PendingCapture {
                        kind: CaptureKind::Screen,
                        jpeg: vec![0xff, 0xd8, 0xff, 0x01],
                        captured_at: at,
                        blurred: false,
                    },
                    PendingCapture {
                        kind: CaptureKind::Thumb,
                        jpeg: vec![0xff, 0xd8, 0xff, 0x02],
                        captured_at: at,
                        blurred: false,
                    },
                ]
            } else {
                Vec::new()
            };
            let pending = PendingActivity::from_activity(&activity, captures).unwrap();
            let scope = OutboxScope::new(&self.owner, workspace).unwrap();
            match self
                .store()
                .enqueue_activity(&scope, &pending, &KEY)
                .unwrap()
            {
                crate::outbox::EnqueueResult::Inserted(id) => id,
                other => panic!("{other:?}"),
            }
        }

        fn store(&self) -> &Outbox {
            self.uploader.store.as_ref().unwrap()
        }

        fn rows(&self) -> Vec<crate::outbox::QueueRow> {
            self.store().recent_rows(&self.owner, 10).unwrap()
        }

        fn drain(&mut self) -> PassEnd {
            self.uploader.drain(None, &mut || false)
        }

        fn events(&mut self) -> Vec<UploadEvent> {
            std::iter::from_fn(|| self.events.try_recv().ok()).collect()
        }

        /// The latest upload issue the worker was told about.
        fn issue(&mut self) -> Option<UploadIssue> {
            self.events()
                .into_iter()
                .rev()
                .find_map(|e| match e {
                    UploadEvent::Issue(issue) => Some(issue),
                    _ => None,
                })
                .flatten()
        }

        /// The wait scheduled after the pass, rounded to seconds.
        fn waits_for(&self) -> Option<u64> {
            self.uploader.not_before.map(|at| {
                (at.saturating_duration_since(Instant::now()).as_millis() as f64 / 1000.0).round()
                    as u64
            })
        }
    }

    #[test]
    fn an_oversized_upload_is_sent_again_at_once_without_its_images() {
        let (url, requests) = canned_server(vec![
            token_ok(),
            http("413 Payload Too Large", &[], ""),
            http("200 OK", &[], "{}"),
        ]);
        let mut h = harness(&url);
        h.queue(P1, true);

        assert_eq!(h.drain(), PassEnd::Drained);
        let requests: Vec<String> = requests.try_iter().collect();
        assert_eq!(requests.len(), 3);
        assert!(
            body_of(&requests[1]).get("screenshotPngBase64").is_some(),
            "first try with the images"
        );
        let resent = body_of(&requests[2]);
        assert!(
            resent.get("screenshotPngBase64").is_none()
                && resent.get("screenshotThumbJpgBase64").is_none()
        );
        assert!(
            resent["sentAt"].is_string(),
            "sentAt is stamped on every attempt"
        );
        assert!(h.rows().is_empty(), "uploaded and removed");
        let events = h.events();
        assert!(events.contains(&UploadEvent::Uploaded(1)), "{events:?}");
        assert!(events.iter().any(|e| matches!(e, UploadEvent::Synced(_))));
        assert!(
            events
                .iter()
                .any(|e| matches!(e, UploadEvent::Stripped { count: 1, .. })),
            "the user is told the screenshot was removed: {events:?}"
        );
    }

    #[test]
    fn a_definitive_rejection_is_parked_with_the_server_reason_and_the_queue_moves_on() {
        let overlap = abp(
            "Tracker:ActivityOverlaps",
            "This block overlaps another one.",
        );
        let (url, _requests) = canned_server(vec![
            token_ok(),
            http("409 Conflict", &[], &overlap),
            http("200 OK", &[], "{}"),
        ]);
        let mut h = harness(&url);
        let parked = h.queue(P1, true);
        h.queue(P1, false);

        assert_eq!(h.drain(), PassEnd::Drained);
        let rows = h.rows();
        assert_eq!(rows.len(), 1, "the second row went up");
        assert_eq!(rows[0].id, parked);
        assert_eq!(rows[0].state, RowState::Parked);
        assert_eq!(
            rows[0].parked_reason.as_deref(),
            Some("This block overlaps another one.")
        );
        let summary = h.store().queue_summary(&h.owner).unwrap();
        assert_eq!((summary.pending, summary.parked), (0, 1));
        let events = h.events();
        assert!(
            events
                .iter()
                .any(|e| matches!(e, UploadEvent::Parked { count: 1, .. })),
            "{events:?}"
        );
        assert!(events.contains(&UploadEvent::Uploaded(1)), "{events:?}");
        assert!(events.contains(&UploadEvent::RowState {
            row_id: parked,
            state: UploadState::Rejected
        }));
    }

    #[test]
    fn a_clock_ahead_rejection_keeps_the_row_and_retries_in_five_minutes() {
        let future = abp("Tracker:ActivityInFuture", "The block ends in the future.");
        let (url, _requests) = canned_server(vec![token_ok(), http("425 Too Early", &[], &future)]);
        let mut h = harness(&url);
        h.queue(P1, false);

        assert_eq!(h.drain(), PassEnd::Waiting);
        assert_eq!(h.rows()[0].state, RowState::Pending);
        assert_eq!(
            h.rows()[0].last_error.as_deref(),
            Some("The block ends in the future.")
        );
        assert_eq!(h.waits_for(), Some(300));
        assert!(matches!(h.issue(), Some(UploadIssue::ClockAhead { .. })));
        assert!(!h.uploader.ready(), "waiting");
    }

    #[test]
    fn server_trouble_backs_off_and_honours_retry_after() {
        let (url, _requests) = canned_server(vec![
            token_ok(),
            http("503 Service Unavailable", &[], "<html>down</html>"),
            http("429 Too Many Requests", &[("Retry-After", "120")], ""),
            http("200 OK", &[], "{}"),
        ]);
        let mut h = harness(&url);
        h.queue(P1, false);

        assert_eq!(h.drain(), PassEnd::Waiting);
        assert_eq!(h.waits_for(), Some(30));
        match h.issue() {
            Some(UploadIssue::Retrying { reason, .. }) => assert_eq!(reason, "HTTP 503"),
            other => panic!("expected a retry, got {other:?}"),
        }
        assert_eq!(h.rows()[0].state, RowState::Pending, "kept");

        // "Sync now" retries at once; the server asks for two minutes.
        h.uploader.apply(UploadCommand::Now);
        assert!(h.uploader.ready());
        assert_eq!(h.drain(), PassEnd::Waiting);
        assert_eq!(h.waits_for(), Some(120), "Retry-After beats the 60 s step");

        h.uploader.apply(UploadCommand::Now);
        assert_eq!(h.drain(), PassEnd::Drained);
        assert!(h.issue().is_none(), "cleared by the success");
        assert_eq!(
            h.uploader.backoff.next(None),
            Duration::from_secs(30),
            "and the back-off starts over"
        );
    }

    #[test]
    fn an_inactive_subscription_keeps_the_row_and_retries_hourly() {
        let inactive = abp(
            "Tracker:SubscriptionInactive",
            "The subscription is inactive.",
        );
        let (url, _requests) = canned_server(vec![
            token_ok(),
            http("402 Payment Required", &[], &inactive),
        ]);
        let mut h = harness(&url);
        h.queue(P1, false);
        assert_eq!(h.drain(), PassEnd::Waiting);
        assert_eq!(h.waits_for(), Some(3600));
        assert!(matches!(
            h.issue(),
            Some(UploadIssue::SubscriptionInactive { .. })
        ));
        assert_eq!(h.rows()[0].state, RowState::Pending);
    }

    #[test]
    fn a_block_of_a_project_that_is_gone_is_parked_after_checking_the_projects() {
        let not_member = abp(
            "Tracker:NotProjectMember",
            "You are not a member of this project.",
        );
        let others = format!(
            r#"[{{"id":"{P2}","title":"Other","intervalMinutes":10,"allowScreenshot":true,"allowWebcam":false,
            "allowKeyboard":true,"allowMouse":true,"allowActiveWindow":true,"allowRunningPrograms":true}}]"#
        );
        let (url, requests) = canned_server(vec![
            token_ok(),
            http("403 Forbidden", &[], &not_member),
            http("200 OK", &[], &others),
            http("403 Forbidden", &[], &not_member),
            http("200 OK", &[], &others),
        ]);
        let mut h = harness(&url);
        h.queue(GONE, false);
        h.queue(GONE, false);

        assert_eq!(h.drain(), PassEnd::Drained);
        let rows = h.rows();
        assert!(rows.iter().all(|r| r.state == RowState::Parked), "{rows:?}");
        assert!(rows[0]
            .parked_reason
            .as_deref()
            .unwrap()
            .contains("You are not a member of this project."));
        let requests: Vec<String> = requests.try_iter().collect();
        let lookups: Vec<&String> = requests
            .iter()
            .filter(|r| r.starts_with("GET /api/app/project/my-projects"))
            .collect();
        assert_eq!(
            lookups.len(),
            2,
            "each refusal is checked against the projects"
        );
        assert_eq!(header_of(lookups[0], "x-workspace-id").as_deref(), Some(W1));
        assert!(h
            .events()
            .iter()
            .any(|e| matches!(e, UploadEvent::Parked { count: 2, .. })));
    }

    #[test]
    fn a_block_of_a_project_still_listed_is_kept() {
        let archived = abp("Tracker:ProjectArchived", "The project is archived.");
        let listed = format!(
            r#"[{{"id":"{P1}","title":"P1","intervalMinutes":10,"allowScreenshot":true,"allowWebcam":false,
            "allowKeyboard":true,"allowMouse":true,"allowActiveWindow":true,"allowRunningPrograms":true}}]"#
        );
        let (url, _requests) = canned_server(vec![
            token_ok(),
            http("409 Conflict", &[], &archived),
            http("200 OK", &[], &listed),
        ]);
        let mut h = harness(&url);
        h.queue(P1, false);
        assert_eq!(h.drain(), PassEnd::Waiting);
        assert_eq!(h.rows()[0].state, RowState::Pending);
        assert_eq!(h.waits_for(), Some(30));
    }

    #[test]
    fn a_refused_sign_in_stops_uploads_and_tells_the_worker() {
        let refused = r#"{"error":"invalid_grant","error_description":"The refresh token is no longer valid."}"#;
        let (url, requests) = canned_server(vec![http("400 Bad Request", &[], refused)]);
        let mut h = harness(&url);
        h.queue(P1, false);

        assert_eq!(h.drain(), PassEnd::SignInExpired);
        let events = h.events();
        assert!(
            events
                .iter()
                .any(|e| matches!(e, UploadEvent::SignInExpired { .. })),
            "{events:?}"
        );
        assert!(
            h.uploader.current.is_none(),
            "no more attempts until the password is entered again"
        );
        assert_eq!(h.rows()[0].state, RowState::Pending, "the queue is kept");
        assert!(!h.uploader.ready());
        assert_eq!(h.drain(), PassEnd::Idle);
        let requests: Vec<String> = requests.try_iter().collect();
        assert_eq!(requests.len(), 1);
        assert!(
            !requests[0].contains("grant_type=password"),
            "never a silent password sign-in"
        );
    }

    #[test]
    fn rows_of_another_account_are_never_uploaded() {
        let (url, requests) = canned_server(vec![token_ok()]);
        let mut h = harness(&url);
        let now = Utc::now();
        let activity = Activity::new(P1, now - chrono::Duration::minutes(10), now);
        let pending = PendingActivity::from_activity(&activity, Vec::new()).unwrap();
        let bob = OutboxScope::new("bob@acme.com", W1).unwrap();
        h.store().enqueue_activity(&bob, &pending, &KEY).unwrap();
        assert!(!h.uploader.ready());
        assert_eq!(h.drain(), PassEnd::Drained);
        assert_eq!(requests.try_iter().count(), 0, "nothing was sent");
        assert_eq!(h.store().queue_summary("bob@acme.com").unwrap().pending, 1);
    }

    #[test]
    fn a_pass_past_its_deadline_stops_before_the_next_row() {
        let (url, requests) = canned_server(vec![token_ok()]);
        let mut h = harness(&url);
        h.queue(P1, false);
        assert_eq!(
            h.uploader.drain(Some(Instant::now()), &mut || false),
            PassEnd::Waiting
        );
        assert_eq!(requests.try_iter().count(), 0);
        assert_eq!(h.rows()[0].state, RowState::Pending);
    }

    #[test]
    fn a_command_that_must_cut_the_pass_short_stops_it_between_rows() {
        let (url, requests) = canned_server(vec![token_ok(), http("200 OK", &[], "{}")]);
        let mut h = harness(&url);
        h.queue(P1, false);
        h.queue(P1, false);
        let mut rows_sent = 0;
        let end = h.uploader.drain(None, &mut || {
            rows_sent += 1;
            rows_sent > 1
        });
        assert_eq!(end, PassEnd::Waiting);
        assert_eq!(h.rows().len(), 1, "one row went up, the second waits");
        assert_eq!(requests.try_iter().count(), 2);
    }

    #[test]
    fn rows_upload_to_the_workspace_they_were_recorded_in_after_a_switch() {
        let (url, requests) = canned_server(vec![
            token_ok(),
            http("200 OK", &[], "{}"),
            http("200 OK", &[], "{}"),
        ]);
        let mut h = harness(&url);
        let now = Utc::now();
        h.queue_in(
            W1,
            Activity::new(
                P1,
                now - chrono::Duration::minutes(20),
                now - chrono::Duration::minutes(10),
            ),
            false,
        );
        // Switched to workspace 2: the same user and token, another workspace.
        let switched = h
            .uploader
            .current
            .as_ref()
            .unwrap()
            .account
            .clone()
            .in_workspace(W2, "Globex");
        h.uploader.set_account(switched);
        h.queue_in(
            W2,
            Activity::new(P2, now - chrono::Duration::minutes(10), now),
            false,
        );

        assert_eq!(h.drain(), PassEnd::Drained);
        let requests: Vec<String> = requests.try_iter().collect();
        assert_eq!(requests.len(), 3, "one token shared by both workspaces");
        assert_eq!(
            header_of(&requests[0], "x-workspace-id"),
            None,
            "the token request"
        );
        assert_eq!(
            (
                header_of(&requests[1], "x-workspace-id").as_deref(),
                body_of(&requests[1])["projectId"].as_str()
            ),
            (Some(W1), Some(P1))
        );
        assert_eq!(
            (
                header_of(&requests[2], "x-workspace-id").as_deref(),
                body_of(&requests[2])["projectId"].as_str()
            ),
            (Some(W2), Some(P2))
        );
        assert!(h.rows().is_empty());
    }

    #[test]
    fn rows_of_a_lost_workspace_are_parked_and_the_rest_go_up() {
        let lost = abp(
            "Tracker:NotWorkspaceMember",
            "You are not a member of this workspace.",
        );
        let (url, requests) = canned_server(vec![
            token_ok(),
            http("403 Forbidden", &[], &lost),
            http("200 OK", &[], "{}"),
        ]);
        let mut h = harness(&url);
        let now = Utc::now();
        let first = h.queue(P1, false);
        h.queue_in(
            W1,
            Activity::new(
                P1,
                now - chrono::Duration::minutes(30),
                now - chrono::Duration::minutes(20),
            ),
            false,
        );
        h.queue_in(
            W2,
            Activity::new(
                P2,
                now - chrono::Duration::minutes(40),
                now - chrono::Duration::minutes(30),
            ),
            false,
        );

        assert_eq!(h.drain(), PassEnd::Drained);
        let requests: Vec<String> = requests.try_iter().collect();
        assert_eq!(
            requests.len(),
            3,
            "one try in workspace 1, then workspace 2's row; the second row of 1 is not tried"
        );
        assert_eq!(
            header_of(&requests[2], "x-workspace-id").as_deref(),
            Some(W2)
        );
        let rows = h.rows();
        assert_eq!(rows.len(), 2, "workspace 2's row went up");
        assert!(
            rows.iter().all(|r| r.state == RowState::Parked),
            "the terminal answer parks the workspace's rows: {rows:?}"
        );
        assert!(rows
            .iter()
            .find(|r| r.id == first)
            .unwrap()
            .parked_reason
            .as_deref()
            .unwrap()
            .starts_with("workspace access lost"));
        let events = h.events();
        assert!(
            events.iter().any(|e| matches!(e, UploadEvent::WorkspaceLost { workspace_id, .. } if workspace_id == W1)),
            "{events:?}"
        );
        assert!(events.contains(&UploadEvent::Uploaded(1)), "{events:?}");
    }

    #[test]
    fn a_revoked_device_parks_the_scope_and_other_workspaces_go_on() {
        let revoked = abp("Tracker:DeviceRevoked", "This device was revoked.");
        let (url, _requests) = canned_server(vec![
            token_ok(),
            http("403 Forbidden", &[], &revoked),
            http("200 OK", &[], "{}"),
        ]);
        let mut h = harness(&url);
        let now = Utc::now();
        h.queue(P1, true);
        h.queue_in(
            W1,
            Activity::new(
                P1,
                now - chrono::Duration::minutes(30),
                now - chrono::Duration::minutes(20),
            ),
            false,
        );
        h.queue_in(
            W2,
            Activity::new(
                P2,
                now - chrono::Duration::minutes(40),
                now - chrono::Duration::minutes(30),
            ),
            false,
        );
        assert_eq!(h.drain(), PassEnd::Drained);
        let rows = h.rows();
        assert_eq!(rows.len(), 2);
        assert!(
            rows.iter().all(|r| r.state == RowState::Parked
                && r.parked_reason
                    .as_deref()
                    .is_some_and(|reason| reason.to_lowercase().starts_with("device revoked"))
                && r.capture_bytes == 0),
            "{rows:?}"
        );
        let events = h.events();
        assert!(
            events.iter().any(|e| matches!(e, UploadEvent::DeviceRevoked { workspace_id, .. } if workspace_id == W1)),
            "{events:?}"
        );
        assert!(events.contains(&UploadEvent::Uploaded(1)));
    }

    #[test]
    fn rows_of_a_workspace_that_is_not_a_guid_are_parked_and_the_rest_go_up() {
        let (url, requests) = canned_server(vec![token_ok(), http("200 OK", &[], "{}")]);
        let mut h = harness(&url);
        let now = Utc::now();
        // Queued by an older build under a workspace name.
        h.queue_in(
            "acme-workspace",
            Activity::new(
                P1,
                now - chrono::Duration::minutes(30),
                now - chrono::Duration::minutes(20),
            ),
            false,
        );
        h.queue(P1, false);
        assert_eq!(h.drain(), PassEnd::Drained);
        let rows = h.rows();
        assert_eq!(rows.len(), 1, "the good row went up");
        assert_eq!(rows[0].state, RowState::Parked);
        assert_eq!(rows[0].workspace_id, "acme-workspace");
        let requests: Vec<String> = requests.try_iter().collect();
        assert_eq!(requests.len(), 2);
        assert_eq!(
            header_of(&requests[1], "x-workspace-id").as_deref(),
            Some(W1)
        );
    }

    #[test]
    fn the_worker_can_park_a_revoked_workspace_found_by_the_control_task() {
        let mut h = harness("http://127.0.0.1:9");
        h.queue(P1, false);
        h.queue(P1, true);
        h.uploader.apply(UploadCommand::ParkScope {
            workspace_id: W1.into(),
            reason: "device revoked".into(),
        });
        assert!(h.rows().iter().all(|r| r.state == RowState::Parked));
        assert!(h.events().contains(&UploadEvent::QueueChanged));
        assert!(!h.uploader.ready(), "nothing left to send");
    }

    #[test]
    fn a_tampered_row_is_parked_by_the_local_integrity_check_and_the_queue_moves_on() {
        let (url, requests) = canned_server(vec![token_ok(), http("200 OK", &[], "{}")]);
        let mut h = harness(&url);
        // Signed with another key than the account's: fails the check.
        let now = Utc::now();
        let activity = Activity::new(
            P1,
            now - chrono::Duration::minutes(20),
            now - chrono::Duration::minutes(10),
        );
        let pending = PendingActivity::from_activity(&activity, Vec::new()).unwrap();
        let scope = OutboxScope::new(&h.owner, W1).unwrap();
        h.store()
            .enqueue_activity(&scope, &pending, &[9u8; 32])
            .unwrap();
        h.queue(P1, false);
        assert_eq!(h.drain(), PassEnd::Drained);
        let rows = h.rows();
        assert_eq!(rows.len(), 1);
        assert_eq!(
            rows[0].parked_reason.as_deref(),
            Some(crate::api::INTEGRITY_FAILURE_REASON)
        );
        let requests: Vec<String> = requests.try_iter().collect();
        assert_eq!(requests.len(), 2, "only the good row was sent");
    }

    #[test]
    fn signing_out_drains_what_fits_then_lets_go_of_the_account() {
        let (url, requests) = canned_server(vec![token_ok(), http("200 OK", &[], "{}")]);
        let mut h = harness(&url);
        h.queue(P1, false);
        h.uploader.apply(UploadCommand::SignOut {
            deadline: Instant::now() + Duration::from_secs(15),
        });
        assert!(h.rows().is_empty(), "uploaded before letting go");
        assert!(h.uploader.current.is_none());
        assert_eq!(requests.try_iter().count(), 2);
        assert!(matches!(
            h.uploader.apply(UploadCommand::Shutdown {
                deadline: Instant::now()
            }),
            Flow::Exit
        ));
    }

    #[test]
    fn the_uploader_thread_uploads_queued_rows_and_ends_on_shutdown() {
        let (url, _requests) = canned_server(vec![token_ok(), http("200 OK", &[], "{}")]);
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("outbox.db");
        let account = account(&url);
        {
            let store = Outbox::open(&path, OutboxLimits::default()).unwrap();
            let now = Utc::now();
            let activity = Activity::new(P1, now - chrono::Duration::minutes(10), now);
            let pending = PendingActivity::from_activity(&activity, Vec::new()).unwrap();
            let scope = OutboxScope::new(account.owner(), W1).unwrap();
            store.enqueue_activity(&scope, &pending, &KEY).unwrap();
        }
        let (tx, mut events) = tokio::sync::mpsc::unbounded_channel();
        let mut handle = spawn(path.clone(), ServerClock::default(), tx);
        handle.send(UploadCommand::Account(Some(account)));
        let deadline = Instant::now() + Duration::from_secs(10);
        let mut uploaded = false;
        while !uploaded && Instant::now() < deadline {
            match events.try_recv() {
                Ok(UploadEvent::Uploaded(1)) => uploaded = true,
                Ok(_) => {}
                Err(_) => std::thread::sleep(Duration::from_millis(20)),
            }
        }
        assert!(uploaded, "the thread uploaded the queued row");
        handle.send(UploadCommand::Shutdown {
            deadline: Instant::now() + Duration::from_secs(1),
        });
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(handle.finish(Instant::now() + Duration::from_secs(5)));
        assert!(handle.thread.is_none(), "joined");
    }
}
