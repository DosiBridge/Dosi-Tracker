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
//!   - `TransientFailure`: Increments retry count and updates last error message; entry remains in queue.
//!   - `AuthRequired`: Stops pass immediately without removing entries or claiming token refresh.
//!   - `DefinitiveRejection`: Halts pass and visibly reports rejection; entry is retained.
//!   - `MismatchedAcknowledgement`: Halts pass; entry is preserved.
//! - Distinction between transport acceptance and local persistence:
//!   If the transport accepts an activity, but the local outbox deletion fails (e.g. SQLite disk error),
//!   the pass halts and reports [`SyncStopReason::AcknowledgementPersistenceFailed`]. The entry is reported
//!   in `transport_accepted` but **not** in `persisted_acknowledged`.

use crate::outbox::{Outbox, OutboxEntry, OutboxError, OutboxScope, MAX_ERROR_MESSAGE_BYTES};

/// Default maximum number of queued rows attempted per sync pass.
pub const DEFAULT_MAX_ATTEMPTS: usize = 50;

/// Absolute maximum allowed attempts per single pass.
pub const MAX_ALLOWED_ATTEMPTS: usize = 200;

/// Default batch read limit when querying pending records from outbox.
pub const DEFAULT_BATCH_LIMIT: usize = 20;

/// Absolute maximum allowed batch read limit.
pub const MAX_ALLOWED_BATCH_LIMIT: usize = 100;

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
}

impl Default for SyncPassConfig {
    fn default() -> Self {
        Self {
            max_attempts: DEFAULT_MAX_ATTEMPTS,
            batch_limit: DEFAULT_BATCH_LIMIT,
        }
    }
}

impl SyncPassConfig {
    /// Creates and validates a new sync pass configuration.
    pub fn new(max_attempts: usize, batch_limit: usize) -> Result<Self, SyncError> {
        let config = Self {
            max_attempts,
            batch_limit,
        };
        config.validate()?;
        Ok(config)
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
    /// Outbox store read or mutation error.
    StoreError(String),
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
    /// Identifiers and reasons for activities that experienced transient failures.
    pub transient_failures: Vec<ActivityFailure>,
    /// Identifiers and reasons for activities that experienced definitive rejections.
    pub definitive_rejections: Vec<ActivityFailure>,
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
            stop_reason: None,
            drained: false,
        }
    }
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
                        report.stop_reason = Some(SyncStopReason::StoreError(err.to_string()));
                        break;
                    }
                }
            };

            if batch.is_empty() {
                report.drained = true;
                break;
            }

            for entry in batch {
                report.attempted_count += 1;

                // Delivery invocation happens with SQLite mutex released
                let outcome = transport.deliver(&entry);

                match outcome {
                    DeliveryOutcome::Accepted { client_activity_id } => {
                        if client_activity_id != entry.client_activity_id {
                            report.stop_reason = Some(SyncStopReason::MismatchedAcknowledgement {
                                expected: entry.client_activity_id.clone(),
                                received: client_activity_id,
                            });
                            break;
                        }

                        report
                            .transport_accepted
                            .push(entry.client_activity_id.clone());

                        match self.outbox.ack_delivered(scope, &entry.client_activity_id) {
                            Ok(()) => {
                                report.persisted_acknowledged.push(entry.client_activity_id);
                            }
                            Err(err) => {
                                report.stop_reason =
                                    Some(SyncStopReason::AcknowledgementPersistenceFailed {
                                        client_activity_id: entry.client_activity_id,
                                        error: err.to_string(),
                                    });
                                break;
                            }
                        }
                    }
                    DeliveryOutcome::TransientFailure { reason } => {
                        let bounded_reason =
                            truncate_to_char_boundary(&reason, MAX_ERROR_MESSAGE_BYTES).to_string();
                        let _ = self.outbox.mark_attempt_failed(
                            scope,
                            &entry.client_activity_id,
                            &bounded_reason,
                        );
                        report.transient_failures.push(ActivityFailure {
                            client_activity_id: entry.client_activity_id.clone(),
                            reason: reason.clone(),
                        });
                        report.stop_reason = Some(SyncStopReason::TransientFailure {
                            client_activity_id: entry.client_activity_id,
                            reason: bounded_reason,
                        });
                        break;
                    }
                    DeliveryOutcome::AuthRequired { reason } => {
                        report.stop_reason = Some(SyncStopReason::AuthRequired {
                            client_activity_id: entry.client_activity_id,
                            reason,
                        });
                        break;
                    }
                    DeliveryOutcome::Rejected { reason } => {
                        report.definitive_rejections.push(ActivityFailure {
                            client_activity_id: entry.client_activity_id.clone(),
                            reason: reason.clone(),
                        });
                        report.stop_reason = Some(SyncStopReason::DefinitiveRejection {
                            client_activity_id: entry.client_activity_id,
                            reason,
                        });
                        break;
                    }
                }
            }

            if report.stop_reason.is_some() {
                break;
            }
        }

        // If loop finished without a stop reason, check if queue is drained or max attempts reached
        if report.stop_reason.is_none() && !report.drained {
            match self.outbox.count(Some(scope)) {
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
                    report.stop_reason = Some(SyncStopReason::StoreError(err.to_string()));
                }
            }
        }

        Ok(report)
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
