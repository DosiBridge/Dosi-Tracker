//! Abstract traits and error types for telemetry acquisition on Linux.
//!
//! Enforces:
//! - Fakeable traits matching Windows (`AcquisitionProvider`, `ActiveWindowProvider`, `RunningAppsProvider`).
//! - Complete decoupling of OS queries from lifecycle and consent decision boundaries.
//! - Truthful error propagation without synthetic fallback.
//! - Sensor seams: [`IdleSource`], [`ScreenSource`] and the capability state
//!   [`Availability`] (available, unsupported, denied, needs approval, unavailable).

use chrono::{DateTime, Utc};

pub use super::capture::RawFrame;
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
    /// The user (or the desktop on their behalf) refused the request, e.g. a
    /// screenshot portal dialog answered "deny". Not retried until the user acts.
    PermissionDenied(String),
    /// The desktop is waiting for the user to approve the request (e.g. a
    /// portal dialog nobody answered in time).
    NeedsApproval(String),
    /// A captured image could not be decoded or encoded within its limits.
    EncodeError(String),
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
            TrackingError::PermissionDenied(reason) => {
                write!(f, "Permission denied: {}", reason)
            }
            TrackingError::NeedsApproval(reason) => {
                write!(f, "Waiting for the user's approval: {}", reason)
            }
            TrackingError::EncodeError(reason) => {
                write!(f, "Capture encoding failed: {}", reason)
            }
        }
    }
}

impl std::error::Error for TrackingError {}

/// Whether a sensor can deliver on this desktop right now — the capability
/// state shown by status, doctor and the GUI. Never a fabricated "working".
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Availability {
    /// The source works.
    Available,
    /// The session or desktop cannot provide it (e.g. no portal, Wayland).
    Unsupported(String),
    /// The user refused it; nothing is requested again until they act.
    Denied,
    /// The desktop asks the user first and nobody answered yet.
    NeedsApproval,
    /// Supported, but failing at the moment (connection lost, I/O error).
    Unavailable(String),
}

/// A source of "when did the user last give input" for idle and away
/// detection. Local only: never uploaded.
pub trait IdleSource: Send + Sync {
    /// When the user last gave input, as seen at `now`; `None` when the source
    /// has no input on record.
    fn last_input(&self, now: DateTime<Utc>) -> Result<Option<DateTime<Utc>>, TrackingError>;
    /// Short stable name of the source (`mutter`, `screensaver`, ...) for
    /// status and doctor.
    fn kind(&self) -> &'static str;
}

/// What a screen source should capture.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CaptureTarget {
    /// The monitor showing most of the active window (the whole screen when
    /// that cannot be told).
    #[default]
    ActiveMonitor,
    /// The whole screen.
    WholeScreen,
}

/// A source of screen frames (X11 GetImage, the Screenshot portal). Encoding
/// to JPEG is done afterwards by [`super::capture::encode_frame`].
pub trait ScreenSource: Send + Sync {
    /// Capture one frame.
    fn capture(&self, target: CaptureTarget) -> Result<RawFrame, TrackingError>;
    /// The source's current state, without capturing.
    fn availability(&self) -> Availability;
}

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
