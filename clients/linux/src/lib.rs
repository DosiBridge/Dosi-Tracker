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
pub mod paths;

pub use consent::{ConsentError, ConsentManager, ConsentStore, ConsentedPermissions, LocalConsent};
pub use diagnostics::{
    ArchitecturalSupport, CapabilityDiagnosis, DiagnosticStatus, DoctorReport, SessionEnvironment,
    SessionType,
};
pub use lifecycle::{
    AcquisitionCapability, AcquisitionError, AcquisitionTicket, LifecycleError,
    LifecycleGeneration, PauseReason, TrackerLifecycle, TrackerState,
};
pub use paths::LinuxPaths;
