//! Scoped offline sync-pass coordinator for Dosi-Tracker Linux.
//!
//! # Architecture & Scope Boundaries
//! - Operates strictly over the durable local [`Outbox`] via an injected [`DeliveryTransport`].
//! - Explicit, sequential, single-threaded execution per pass: one immutable [`OutboxScope`].
//! - No production HTTP client, no network stack, no daemon/scheduler, and no sensor dependencies.
//! - The injected delivery interface allows deterministic unit/integration testing and fake ledgers.
//! - Concurrency guarantee: The SQLite mutex is **never** held across injected transport delivery calls.
//! - Multi-process limitation: This library does not provide multi-process distributed locks. Concurrent
//!   external processes reading the same SQLite outbox file must rely on backend idempotency.
//!
//! # Delivery Contract & State Handling
//! - Exact positive acknowledgement: Only when the transport explicitly acknowledges the exact same
//!   `client_activity_id` is the entry removed from the local outbox.
//! - Non-accepted row preservation:
//!   - `TransientFailure`: When local persistence succeeds, increments retry count and updates last error message;
//!     entry remains in queue. If persisting retry metadata fails, reports [`SyncStopReason::RetryMetadataPersistenceFailed`]
//!     and halts without claiming retry metadata was saved.
//!   - `AuthRequired`: Stops pass immediately without removing entries or claiming token refresh.
//!   - `DefinitiveRejection`: Halts pass and visibly reports rejection; entry is retained.
//!   - `MismatchedAcknowledgement`: Halts pass; entry is preserved.
//! - Upload outcomes of the server contract (design 8.4):
//!   - `Park`: the row is parked with its reason and the pass **continues** with the next
//!     row, so one refused block never blocks its scope.
//!   - `StripCaptures`: the row's capture bytes are stripped and it is redelivered once at
//!     once; a second refusal (or a row with nothing to strip) parks it.
//!   - `RetryLater`: the attempt is recorded and the pass stops with
//!     [`SyncStopReason::RetryLater`]; the caller schedules the retry by its kind.
//!   - `WorkspaceAccessLost` / `DeviceRevoked`: the pass stops with
//!     [`SyncStopReason::ScopeBlocked`]; a revoked device's row is parked, a lost
//!     workspace's row is kept.
//! - Deadline: an optional [`SyncPassConfig::deadline`] is checked before each row.
//! - Captures: a v2 row's captures are loaded right before its delivery, with the SQLite
//!   lock released again before the transport runs.
//! - Bounded diagnostic retention:
//!   All diagnostic reasons and received IDs retained in [`SyncPassReport`] and [`SyncStopReason`] are truncated
//!   to explicit UTF-8 byte boundaries ([`MAX_REPORT_DIAGNOSTIC_BYTES`] and [`MAX_RECEIVED_ID_BYTES`]).
//!   Queue identities ([`OutboxEntry::client_activity_id`]) are never truncated. Total process memory is not
//!   bounded by this coordinator because injected transports may allocate memory before returning.
//! - Distinction between transport acceptance and local persistence:
//!   If the transport accepts an activity, but the local outbox deletion fails (e.g. SQLite disk error),
//!   the pass halts and reports [`SyncStopReason::AcknowledgementPersistenceFailed`]. The entry is reported
//!   in `transport_accepted` but **not** in `persisted_acknowledged`.

use std::time::{Duration, Instant};

use crate::outbox::{Outbox, OutboxEntry, OutboxError, OutboxScope};

/// Default maximum number of queued rows attempted per sync pass.
pub const DEFAULT_MAX_ATTEMPTS: usize = 50;

/// Absolute maximum allowed attempts per single pass.
pub const MAX_ALLOWED_ATTEMPTS: usize = 200;

/// Default batch read limit when querying pending records from outbox.
pub const DEFAULT_BATCH_LIMIT: usize = 20;

/// Absolute maximum allowed batch read limit.
pub const MAX_ALLOWED_BATCH_LIMIT: usize = 100;

/// Maximum UTF-8 bytes retained for diagnostic reason strings in reports.
pub const MAX_REPORT_DIAGNOSTIC_BYTES: usize = 1024;

/// Maximum UTF-8 bytes retained for unexpected received identifiers in reports.
pub const MAX_RECEIVED_ID_BYTES: usize = 256;

/// The delivery outcome returned by an injected delivery transport for one queued activity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeliveryOutcome {
    /// Transport definitively accepted the activity and returned positive acknowledgement.
    /// Carries the positively acknowledged stable `client_activity_id`.
    Accepted { client_activity_id: String },
    /// Transient delivery failure (e.g. timeout, connection reset, 5xx).
    /// The entry should be retained for future retry with a bounded diagnostic message.
    TransientFailure { reason: String },
    /// Authentication or authorization was denied (e.g. 401/403 or expired token).
    /// The entry must be preserved, and the sync pass must halt immediately without claiming credential refresh.
    AuthRequired { reason: String },
    /// Definitive rejection (e.g. 400 bad schema, 422 unprocessable, forbidden payload).
    /// The entry must be retained and reported as rejected without silent removal.
    Rejected { reason: String },
    /// The server refused this block for good, or it failed its local
    /// integrity check: park it and continue with the next row.
    Park { reason: String },
    /// The captures are the problem (413, `CaptureTooLarge`, `InvalidCaptureData`):
    /// strip them and resend once at once.
    StripCaptures { reason: String },
    /// Keep the row, record the attempt and stop; retry by `kind`, never
    /// before `retry_after`.
    RetryLater {
        reason: String,
        kind: RetryKind,
        retry_after: Option<Duration>,
    },
    /// 403 `NotWorkspaceMember` / `WorkspaceArchived`, 400 `WorkspaceRequired`:
    /// the scope waits; the row is kept.
    WorkspaceAccessLost { reason: String },
    /// 403 `DeviceRevoked`: the row is parked and the scope stops.
    DeviceRevoked { reason: String },
}

/// When a [`DeliveryOutcome::RetryLater`] row should be retried.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RetryKind {
    /// Network, timeout, 5xx, throttling, unknown codes: back off 30 s
    /// doubling to 30 min.
    Transient,
    /// 402 `SubscriptionInactive`: retry hourly.
    Subscription,
    /// 425 `ActivityInFuture`: this computer's clock is ahead; retry in 5 min.
    ClockAhead,
}

/// Why a scope stopped uploading ([`SyncStopReason::ScopeBlocked`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScopeBlock {
    WorkspaceAccessLost,
    DeviceRevoked,
}

/// Abstract delivery transport interface.
///
/// Implementations receive the scoped queued entry and return a typed delivery outcome.
/// An injected transport does NOT perform ambient account or workspace lookups; all required
/// context is carried by the [`OutboxEntry`].
pub trait DeliveryTransport {
    /// Delivers a single queued activity to the transport destination.
    fn deliver(&mut self, entry: &OutboxEntry) -> DeliveryOutcome;
}

impl<F> DeliveryTransport for F
where
    F: FnMut(&OutboxEntry) -> DeliveryOutcome,
{
    fn deliver(&mut self, entry: &OutboxEntry) -> DeliveryOutcome {
        (self)(entry)
    }
}

/// Configuration options for a single sync pass.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyncPassConfig {
    /// Maximum number of queued rows to attempt during a single pass.
    pub max_attempts: usize,
    /// Batch read limit for `peek_pending`.
    pub batch_limit: usize,
    /// No new row is started at or after this instant (e.g. a logout drain
    /// bounded in time); `None` = no deadline.
    pub deadline: Option<Instant>,
}

impl Default for SyncPassConfig {
    fn default() -> Self {
        Self {
            max_attempts: DEFAULT_MAX_ATTEMPTS,
            batch_limit: DEFAULT_BATCH_LIMIT,
            deadline: None,
        }
    }
}

impl SyncPassConfig {
    /// Creates and validates a new sync pass configuration.
    pub fn new(max_attempts: usize, batch_limit: usize) -> Result<Self, SyncError> {
        let config = Self {
            max_attempts,
            batch_limit,
            deadline: None,
        };
        config.validate()?;
        Ok(config)
    }

    /// The same configuration, bounded by `deadline`.
    pub fn with_deadline(mut self, deadline: Option<Instant>) -> Self {
        self.deadline = deadline;
        self
    }

    /// Validates configuration bounds against finite non-zero ceilings.
    pub fn validate(&self) -> Result<(), SyncError> {
        if self.max_attempts == 0 {
            return Err(SyncError::InvalidConfig(
                "max_attempts must be greater than zero".into(),
            ));
        }
        if self.max_attempts > MAX_ALLOWED_ATTEMPTS {
            return Err(SyncError::InvalidConfig(format!(
                "max_attempts exceeds maximum allowed ceiling of {MAX_ALLOWED_ATTEMPTS}"
            )));
        }
        if self.batch_limit == 0 {
            return Err(SyncError::InvalidConfig(
                "batch_limit must be greater than zero".into(),
            ));
        }
        if self.batch_limit > MAX_ALLOWED_BATCH_LIMIT {
            return Err(SyncError::InvalidConfig(format!(
                "batch_limit exceeds maximum allowed ceiling of {MAX_ALLOWED_BATCH_LIMIT}"
            )));
        }
        Ok(())
    }
}

/// Information about an activity delivery failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActivityFailure {
    /// Client activity identifier.
    pub client_activity_id: String,
    /// Detailed diagnostic reason.
    pub reason: String,
}

/// Detailed reason why a sync pass stopped execution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SyncStopReason {
    /// Reached the configured max_attempts ceiling while pending rows remain.
    MaxAttemptsReached { limit: usize },
    /// Transport requested retry due to transient delivery failure.
    TransientFailure {
        client_activity_id: String,
        reason: String,
    },
    /// Authentication or authorization credentials required / rejected.
    AuthRequired {
        client_activity_id: String,
        reason: String,
    },
    /// Transport definitively rejected the activity (e.g. 400 bad data).
    DefinitiveRejection {
        client_activity_id: String,
        reason: String,
    },
    /// Transport returned an acknowledgement with an ID that did not match the sent activity.
    MismatchedAcknowledgement { expected: String, received: String },
    /// Transport accepted delivery, but local SQLite deletion failed.
    AcknowledgementPersistenceFailed {
        client_activity_id: String,
        error: String,
    },
    /// Transport reported transient delivery failure, but local SQLite failed to persist updated retry metadata.
    RetryMetadataPersistenceFailed {
        client_activity_id: String,
        transport_reason: String,
        error: String,
    },
    /// Outbox store read or mutation error.
    StoreError(String),
    /// The transport asked to retry this row later; the attempt was recorded.
    RetryLater {
        client_activity_id: String,
        reason: String,
        kind: RetryKind,
        retry_after: Option<Duration>,
    },
    /// The scope cannot upload until the user acts (re-sign-in, workspace switch).
    ScopeBlocked {
        client_activity_id: String,
        reason: String,
        block: ScopeBlock,
    },
    /// The transport asked to park this row, but parking it failed locally.
    ParkPersistenceFailed {
        client_activity_id: String,
        reason: String,
        error: String,
    },
    /// The configured deadline passed before the next row was started.
    DeadlineReached,
}

/// Comprehensive outcome report of a completed sync pass.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyncPassReport {
    /// The scope of this sync pass.
    pub scope: OutboxScope,
    /// Total rows attempted in this pass.
    pub attempted_count: usize,
    /// Identifiers of activities that the transport positively accepted.
    pub transport_accepted: Vec<String>,
    /// Identifiers of activities whose deletion was actually committed in local SQLite outbox.
    pub persisted_acknowledged: Vec<String>,
    /// Identifiers and reasons for activities that experienced transient failures
    /// (including rows the transport asked to retry later).
    pub transient_failures: Vec<ActivityFailure>,
    /// Identifiers and reasons for activities that experienced definitive rejections.
    pub definitive_rejections: Vec<ActivityFailure>,
    /// Rows parked in this pass, with the reason they were parked for.
    pub parked: Vec<ActivityFailure>,
    /// Rows whose capture bytes were stripped in this pass.
    pub stripped: Vec<String>,
    /// Why the pass ended early; `None` when it ran out of rows cleanly.
    pub stop_reason: Option<SyncStopReason>,
    /// True if the queue has no more pending entries for this scope.
    pub drained: bool,
}

impl SyncPassReport {
    /// Initializes an empty report for the given scope.
    pub fn new(scope: OutboxScope) -> Self {
        Self {
            scope,
            attempted_count: 0,
            transport_accepted: Vec::new(),
            persisted_acknowledged: Vec::new(),
            transient_failures: Vec::new(),
            definitive_rejections: Vec::new(),
            parked: Vec::new(),
            stripped: Vec::new(),
            stop_reason: None,
            drained: false,
        }
    }
}

/// What the pass does after one row's (possibly repeated) delivery.
enum RowStep {
    /// Go on with the next row.
    Continue,
    /// Stop the pass (the report's stop reason is set).
    Stop,
}

/// Errors returned by the sync pass coordinator.
#[derive(Debug)]
pub enum SyncError {
    /// Scope validation failed.
    InvalidScope(String),
    /// Configuration validation failed.
    InvalidConfig(String),
    /// Underlying outbox error.
    Outbox(OutboxError),
}

impl std::fmt::Display for SyncError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidScope(msg) => write!(f, "Invalid sync scope: {msg}"),
            Self::InvalidConfig(msg) => write!(f, "Invalid sync config: {msg}"),
            Self::Outbox(err) => write!(f, "Outbox error during sync pass: {err}"),
        }
    }
}

impl std::error::Error for SyncError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Outbox(err) => Some(err),
            _ => None,
        }
    }
}

impl From<OutboxError> for SyncError {
    fn from(err: OutboxError) -> Self {
        Self::Outbox(err)
    }
}

/// Coordinator for executing bounded offline synchronization passes over the durable outbox.
#[derive(Debug)]
pub struct SyncPassCoordinator<'a> {
    outbox: &'a Outbox,
    config: SyncPassConfig,
}

impl<'a> SyncPassCoordinator<'a> {
    /// Creates a new coordinator instance.
    pub fn new(outbox: &'a Outbox, config: SyncPassConfig) -> Self {
        Self { outbox, config }
    }

    /// Executes a single, bounded synchronization pass for the given scope using the provided transport.
    ///
    /// # Concurrency & Lock Safety
    /// The SQLite database connection is NOT locked while calling [`DeliveryTransport::deliver`].
    /// Locks are acquired strictly within [`Outbox::peek_pending`], [`Outbox::ack_delivered`],
    /// and [`Outbox::mark_attempt_failed`], and released immediately thereafter.
    pub fn run_pass<T: DeliveryTransport>(
        &self,
        scope: &OutboxScope,
        transport: &mut T,
    ) -> Result<SyncPassReport, SyncError> {
        scope
            .validate()
            .map_err(|e| SyncError::InvalidScope(e.to_string()))?;
        self.config.validate()?;

        let mut report = SyncPassReport::new(scope.clone());

        while report.attempted_count < self.config.max_attempts {
            let remaining = self.config.max_attempts - report.attempted_count;
            let read_limit = remaining.min(self.config.batch_limit);

            let batch = match self.outbox.peek_pending(scope, read_limit) {
                Ok(entries) => entries,
                Err(err) => {
                    if report.attempted_count == 0 {
                        return Err(SyncError::Outbox(err));
                    } else {
                        let bounded_err = truncate_to_char_boundary(
                            &err.to_string(),
                            MAX_REPORT_DIAGNOSTIC_BYTES,
                        )
                        .to_string();
                        report.stop_reason = Some(SyncStopReason::StoreError(bounded_err));
                        break;
                    }
                }
            };

            if batch.is_empty() {
                report.drained = true;
                break;
            }

            for entry in batch {
                if self
                    .config
                    .deadline
                    .is_some_and(|deadline| Instant::now() >= deadline)
                {
                    report.stop_reason = Some(SyncStopReason::DeadlineReached);
                    break;
                }
                report.attempted_count += 1;
                if let RowStep::Stop = self.deliver_row(scope, entry, transport, &mut report) {
                    break;
                }
            }

            if report.stop_reason.is_some() {
                break;
            }
        }

        // If loop finished without a stop reason, check if queue is drained or max attempts reached
        if report.stop_reason.is_none() && !report.drained {
            match self.outbox.pending_count(scope) {
                Ok(0) => {
                    report.drained = true;
                }
                Ok(_) => {
                    if report.attempted_count >= self.config.max_attempts {
                        report.stop_reason = Some(SyncStopReason::MaxAttemptsReached {
                            limit: self.config.max_attempts,
                        });
                    }
                }
                Err(err) => {
                    let bounded_err =
                        truncate_to_char_boundary(&err.to_string(), MAX_REPORT_DIAGNOSTIC_BYTES)
                            .to_string();
                    report.stop_reason = Some(SyncStopReason::StoreError(bounded_err));
                }
            }
        }

        Ok(report)
    }

    /// Deliver one row and act on the outcome (redelivering once after a
    /// capture strip). The SQLite lock is only taken inside the outbox calls.
    fn deliver_row<T: DeliveryTransport>(
        &self,
        scope: &OutboxScope,
        mut entry: OutboxEntry,
        transport: &mut T,
        report: &mut SyncPassReport,
    ) -> RowStep {
        let bounded =
            |text: &str| truncate_to_char_boundary(text, MAX_REPORT_DIAGNOSTIC_BYTES).to_string();
        let store_error = |report: &mut SyncPassReport, err: OutboxError| {
            report.stop_reason = Some(SyncStopReason::StoreError(bounded(&err.to_string())));
            RowStep::Stop
        };

        // Every v2 row: a shed capture keeps its digest, which the row's
        // signature binds, even when no bytes are left.
        if entry.is_v2() {
            match self.outbox.load_captures(scope, entry.id) {
                Ok(captures) => entry.captures = captures,
                Err(err) => return store_error(report, err),
            }
        }

        let mut stripped_once = false;
        loop {
            // Delivery invocation happens with SQLite mutex released
            let outcome = transport.deliver(&entry);

            match outcome {
                DeliveryOutcome::Accepted { client_activity_id } => {
                    if client_activity_id != entry.client_activity_id {
                        let bounded_received =
                            truncate_to_char_boundary(&client_activity_id, MAX_RECEIVED_ID_BYTES)
                                .to_string();
                        report.stop_reason = Some(SyncStopReason::MismatchedAcknowledgement {
                            expected: entry.client_activity_id.clone(),
                            received: bounded_received,
                        });
                        return RowStep::Stop;
                    }

                    report
                        .transport_accepted
                        .push(entry.client_activity_id.clone());

                    return match self.outbox.ack_delivered(scope, &entry.client_activity_id) {
                        Ok(()) => {
                            report.persisted_acknowledged.push(entry.client_activity_id);
                            RowStep::Continue
                        }
                        Err(err) => {
                            report.stop_reason =
                                Some(SyncStopReason::AcknowledgementPersistenceFailed {
                                    client_activity_id: entry.client_activity_id,
                                    error: bounded(&err.to_string()),
                                });
                            RowStep::Stop
                        }
                    };
                }
                DeliveryOutcome::TransientFailure { reason } => {
                    let bounded_reason = bounded(&reason);
                    report.stop_reason = Some(
                        match self.record_attempt(scope, &entry, &bounded_reason, report) {
                            Ok(()) => SyncStopReason::TransientFailure {
                                client_activity_id: entry.client_activity_id,
                                reason: bounded_reason,
                            },
                            Err(stop) => stop,
                        },
                    );
                    return RowStep::Stop;
                }
                DeliveryOutcome::RetryLater {
                    reason,
                    kind,
                    retry_after,
                } => {
                    let bounded_reason = bounded(&reason);
                    report.stop_reason = Some(
                        match self.record_attempt(scope, &entry, &bounded_reason, report) {
                            Ok(()) => SyncStopReason::RetryLater {
                                client_activity_id: entry.client_activity_id,
                                reason: bounded_reason,
                                kind,
                                retry_after,
                            },
                            Err(stop) => stop,
                        },
                    );
                    return RowStep::Stop;
                }
                DeliveryOutcome::AuthRequired { reason } => {
                    report.stop_reason = Some(SyncStopReason::AuthRequired {
                        client_activity_id: entry.client_activity_id,
                        reason: bounded(&reason),
                    });
                    return RowStep::Stop;
                }
                DeliveryOutcome::Rejected { reason } => {
                    let bounded_reason = bounded(&reason);
                    report.definitive_rejections.push(ActivityFailure {
                        client_activity_id: entry.client_activity_id.clone(),
                        reason: bounded_reason.clone(),
                    });
                    report.stop_reason = Some(SyncStopReason::DefinitiveRejection {
                        client_activity_id: entry.client_activity_id,
                        reason: bounded_reason,
                    });
                    return RowStep::Stop;
                }
                DeliveryOutcome::Park { reason } => {
                    return self.park_row(scope, &entry, &bounded(&reason), report);
                }
                DeliveryOutcome::StripCaptures { reason } => {
                    let bounded_reason = bounded(&reason);
                    if stripped_once {
                        // Resending without captures was refused as well.
                        return self.park_row(scope, &entry, &bounded_reason, report);
                    }
                    match self.outbox.strip_captures(scope, &entry.client_activity_id) {
                        Ok(true) => {
                            report.stripped.push(entry.client_activity_id.clone());
                            for capture in &mut entry.captures {
                                capture.jpeg.clear();
                            }
                            entry.capture_bytes = 0;
                            stripped_once = true;
                        }
                        // Nothing to strip: resending cannot make it any smaller.
                        Ok(false) => return self.park_row(scope, &entry, &bounded_reason, report),
                        Err(err) => return store_error(report, err),
                    }
                }
                DeliveryOutcome::WorkspaceAccessLost { reason } => {
                    report.stop_reason = Some(SyncStopReason::ScopeBlocked {
                        client_activity_id: entry.client_activity_id,
                        reason: bounded(&reason),
                        block: ScopeBlock::WorkspaceAccessLost,
                    });
                    return RowStep::Stop;
                }
                DeliveryOutcome::DeviceRevoked { reason } => {
                    let bounded_reason = bounded(&reason);
                    if let RowStep::Stop = self.park_row(scope, &entry, &bounded_reason, report) {
                        return RowStep::Stop;
                    }
                    report.stop_reason = Some(SyncStopReason::ScopeBlocked {
                        client_activity_id: entry.client_activity_id,
                        reason: bounded_reason,
                        block: ScopeBlock::DeviceRevoked,
                    });
                    return RowStep::Stop;
                }
            }
        }
    }

    /// Record a failed attempt; on success the row is listed as a transient
    /// failure, otherwise the stop reason to report is returned.
    fn record_attempt(
        &self,
        scope: &OutboxScope,
        entry: &OutboxEntry,
        bounded_reason: &str,
        report: &mut SyncPassReport,
    ) -> Result<(), SyncStopReason> {
        match self
            .outbox
            .mark_attempt_failed(scope, &entry.client_activity_id, bounded_reason)
        {
            Ok(()) => {
                report.transient_failures.push(ActivityFailure {
                    client_activity_id: entry.client_activity_id.clone(),
                    reason: bounded_reason.to_string(),
                });
                Ok(())
            }
            Err(err) => Err(SyncStopReason::RetryMetadataPersistenceFailed {
                client_activity_id: entry.client_activity_id.clone(),
                transport_reason: bounded_reason.to_string(),
                error: truncate_to_char_boundary(&err.to_string(), MAX_REPORT_DIAGNOSTIC_BYTES)
                    .to_string(),
            }),
        }
    }

    /// Park the row and go on with the next one.
    fn park_row(
        &self,
        scope: &OutboxScope,
        entry: &OutboxEntry,
        bounded_reason: &str,
        report: &mut SyncPassReport,
    ) -> RowStep {
        match self
            .outbox
            .park(scope, &entry.client_activity_id, bounded_reason)
        {
            Ok(()) => {
                report.parked.push(ActivityFailure {
                    client_activity_id: entry.client_activity_id.clone(),
                    reason: bounded_reason.to_string(),
                });
                RowStep::Continue
            }
            Err(err) => {
                report.stop_reason = Some(SyncStopReason::ParkPersistenceFailed {
                    client_activity_id: entry.client_activity_id.clone(),
                    reason: bounded_reason.to_string(),
                    error: truncate_to_char_boundary(&err.to_string(), MAX_REPORT_DIAGNOSTIC_BYTES)
                        .to_string(),
                });
                RowStep::Stop
            }
        }
    }
}

/// Convenience function to execute a single sync pass using the coordinator.
pub fn run_sync_pass<T: DeliveryTransport>(
    outbox: &Outbox,
    scope: &OutboxScope,
    config: SyncPassConfig,
    transport: &mut T,
) -> Result<SyncPassReport, SyncError> {
    SyncPassCoordinator::new(outbox, config).run_pass(scope, transport)
}

fn truncate_to_char_boundary(s: &str, max_bytes: usize) -> &str {
    if s.len() <= max_bytes {
        s
    } else {
        let mut boundary = max_bytes;
        while boundary > 0 && !s.is_char_boundary(boundary) {
            boundary -= 1;
        }
        &s[..boundary]
    }
}
