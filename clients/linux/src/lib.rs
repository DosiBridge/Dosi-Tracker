//! Dosi-Tracker Linux Client Foundation.
//!
//! Reusable library providing:
//! - Non-mutating XDG storage path resolution ([`paths`])
//! - Truthful environment and capability diagnostics ([`diagnostics`])
//! - Default-inactive per-capability consent core with atomic persistence ([`consent`])
//! - Lifecycle state machine with generation-based ABA ticket invalidation ([`lifecycle`])

pub mod api;
pub mod consent;
pub mod credentials;
pub mod diagnostics;
pub mod lifecycle;
pub mod outbox;
pub mod paths;
pub mod signing;
pub mod sync;
pub mod tracking;

pub use api::{
    classify_token_failure, classify_upload_failure, ApiClient, ApiError, AuthError,
    CreateActivityDto, HeartbeatDto, HeartbeatResultDto, HttpDeliveryTransport, ServerClock,
    SubmitError, TokenCell, CLIENT_ID, HEARTBEAT_CADENCE, SCOPE, WORKSPACE_HEADER,
};

pub use consent::{
    ConsentError, ConsentManager, ConsentStorage, ConsentStore, ConsentedPermissions,
    FileConsentStore, LocalConsent, MemoryConsentStore,
};
#[cfg(target_os = "linux")]
pub use credentials::ZbusSecretServiceTransport;
pub use credentials::{
    CredentialError, CredentialStore, CredentialStoreKind, Credentials, FileCredentialStore,
    InMemoryCredentialStore, MockSecretServiceTransport, SecretServiceStore,
    SecretServiceTransport,
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
    inspect_outbox_status, inspect_outbox_status_bounded, inspect_outbox_status_instrumented,
    inspect_outbox_status_with_budget, ActivityEnvelope, DatabaseCorruptionKind,
    DurableQueueSummary, EnqueueResult, Outbox, OutboxEntry, OutboxError, OutboxLimits,
    OutboxScope, OutboxStatusError, OutboxStatusReport, RecordCorruptionKind, SchemaErrorKind,
    ScopeErrorKind, SqliteWorkBudget, DEFAULT_MAX_SQLITE_WORK_STEPS, MAX_ACCOUNT_ID_BYTES,
    MAX_CLIENT_ACTIVITY_ID_BYTES, MAX_ERROR_MESSAGE_BYTES, MAX_STATUS_ROW_BOUND,
    MAX_WORKSPACE_ID_BYTES, OUTBOX_SCHEMA_VERSION, PROGRESS_OPCODES_PER_STEP,
};
pub use paths::LinuxPaths;
pub use signing::{
    generate_signing_key, sign_activity_envelope, sign_payload, verify_activity_envelope,
    verify_payload,
};
pub use sync::{
    run_sync_pass, ActivityFailure, DeliveryOutcome, DeliveryTransport, SyncError, SyncPassConfig,
    SyncPassCoordinator, SyncPassReport, SyncStopReason, DEFAULT_BATCH_LIMIT, DEFAULT_MAX_ATTEMPTS,
    MAX_ALLOWED_ATTEMPTS, MAX_ALLOWED_BATCH_LIMIT, MAX_RECEIVED_ID_BYTES,
    MAX_REPORT_DIAGNOSTIC_BYTES,
};
pub use tracking::{
    file_name, is_kernel_thread, program_list, AcquisitionProvider, AcquisitionProviderExt,
    ActiveWindowProvider, ConsentGatedTracker, FakeAcquisitionProvider, MockProcFsReader,
    MockProcessEntry, MockX11PropertyReader, ProcFsReader, ProcFsRunningAppsProvider,
    RunningAppsProvider, SysProcFsReader, SysX11PropertyReader, TrackingError,
    WaylandActiveWindowProvider, WindowInfo, X11ActiveWindowProvider, X11PropertyReader,
    MAX_RUNNING_PROGRAMS,
};
