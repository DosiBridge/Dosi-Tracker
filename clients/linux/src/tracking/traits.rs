//! Abstract traits and error types for telemetry acquisition on Linux.
//!
//! Enforces:
//! - Fakeable traits matching Windows (`AcquisitionProvider`, `ActiveWindowProvider`, `RunningAppsProvider`).
//! - Complete decoupling of OS queries from lifecycle and consent decision boundaries.
//! - Truthful error propagation without synthetic fallback.

use super::model::WindowInfo;
use crate::lifecycle::AcquisitionCapability;

/// Errors arising during native telemetry acquisition and gating.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TrackingError {
    /// Tracking requires affirmative local consent and active lifecycle state.
    ConsentRequired,
    /// The requested capability is not permitted under the effective policy.
    CapabilityDenied(AcquisitionCapability),
    /// In-flight acquisition was invalidated due to lifecycle generation transition (ABA protection).
    GenerationMismatch { ticket_gen: u64, current_gen: u64 },
    /// Session type does not support unprivileged tracking (e.g. Wayland active window).
    UnsupportedSession(String),
    /// Display server is missing or inaccessible (e.g. no DISPLAY variable or socket failure).
    DisplayUnavailable(String),
    /// File system or OS query failed (e.g. reading /proc).
    IoError(String),
    /// Protocol or property decoding error.
    ProtocolError(String),
}

impl std::fmt::Display for TrackingError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TrackingError::ConsentRequired => {
                write!(f, "Tracking requires affirmative local user consent")
            }
            TrackingError::CapabilityDenied(cap) => {
                write!(f, "Acquisition capability {:?} is not permitted", cap)
            }
            TrackingError::GenerationMismatch {
                ticket_gen,
                current_gen,
            } => {
                write!(
                    f,
                    "In-flight acquisition invalidated: ticket generation {} != current generation {}",
                    ticket_gen, current_gen
                )
            }
            TrackingError::UnsupportedSession(reason) => {
                write!(f, "Unsupported session: {}", reason)
            }
            TrackingError::DisplayUnavailable(reason) => {
                write!(f, "Display unavailable: {}", reason)
            }
            TrackingError::IoError(reason) => {
                write!(f, "I/O error during acquisition: {}", reason)
            }
            TrackingError::ProtocolError(reason) => {
                write!(f, "Protocol error during acquisition: {}", reason)
            }
        }
    }
}

impl std::error::Error for TrackingError {}

/// Abstract provider for querying the currently active foreground window.
pub trait ActiveWindowProvider: Send + Sync {
    /// Return the currently focused window (app + title), or `None` if no window has focus.
    fn current_active_window(&self) -> Result<Option<WindowInfo>, TrackingError>;
}

/// Abstract provider for enumerating running applications / processes.
pub trait RunningAppsProvider: Send + Sync {
    /// Return the list of running top-level programs (names only, capped at 200).
    fn running_programs(&self) -> Result<Vec<WindowInfo>, TrackingError>;
}

/// Abstract acquisition provider combining active window and process tracking, matching Windows.
pub trait AcquisitionProvider: ActiveWindowProvider + RunningAppsProvider + Send + Sync {}

// Blanket implementation for any type implementing both ActiveWindowProvider and RunningAppsProvider
impl<T> AcquisitionProvider for T where T: ActiveWindowProvider + RunningAppsProvider + Send + Sync {}

/// Extension trait providing Windows-compatible ergonomic helper methods.
pub trait AcquisitionProviderExt: AcquisitionProvider {
    /// Query the foreground window info, returning `None` on error or when no window is focused.
    fn current_window(&self) -> Option<WindowInfo> {
        self.current_active_window().ok().flatten()
    }
}

impl<T: AcquisitionProvider + ?Sized> AcquisitionProviderExt for T {}
