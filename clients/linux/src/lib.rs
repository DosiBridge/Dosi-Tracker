//! Dosi-Tracker Linux Client Foundation.
//!
//! Reusable library providing:
//! - Non-mutating XDG storage path resolution ([`paths`])
//! - Truthful environment and capability diagnostics ([`diagnostics`])
//! - Default-inactive per-capability consent core with atomic persistence ([`consent`])
//! - Lifecycle state machine with generation-based ABA ticket invalidation ([`lifecycle`])

pub mod consent;
pub mod diagnostics;
pub mod lifecycle;
pub mod outbox;
pub mod paths;
pub mod sync;

pub use consent::{
    ConsentError, ConsentManager, ConsentStorage, ConsentStore, ConsentedPermissions,
    FileConsentStore, LocalConsent, MemoryConsentStore,
};
pub use diagnostics::{
    ArchitecturalSupport, CapabilityDiagnosis, DiagnosticStatus, DoctorReport, SessionEnvironment,
    SessionType,
};
pub use lifecycle::{
    AcquisitionCapability, AcquisitionError, AcquisitionTicket, LifecycleError,
    LifecycleGeneration, PauseReason, TrackerLifecycle, TrackerState,
};
pub use outbox::{
    ActivityEnvelope, EnqueueResult, Outbox, OutboxEntry, OutboxError, OutboxLimits, OutboxScope,
    MAX_ACCOUNT_ID_BYTES, MAX_CLIENT_ACTIVITY_ID_BYTES, MAX_ERROR_MESSAGE_BYTES,
    MAX_WORKSPACE_ID_BYTES, OUTBOX_SCHEMA_VERSION,
};
pub use paths::LinuxPaths;
pub use sync::{
    run_sync_pass, ActivityFailure, DeliveryOutcome, DeliveryTransport, SyncError, SyncPassConfig,
    SyncPassCoordinator, SyncPassReport, SyncStopReason, DEFAULT_BATCH_LIMIT, DEFAULT_MAX_ATTEMPTS,
    MAX_ALLOWED_ATTEMPTS, MAX_ALLOWED_BATCH_LIMIT,
};
