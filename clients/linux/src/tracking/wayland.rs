//! Honest Wayland unsupported active window provider.
//!
//! Enforces:
//! - Unprivileged Wayland protocol isolation invariant: window hierarchy and titles
//!   are private to the compositor and cannot be queried by unprivileged clients.
//! - Truthful reporting of `TrackingError::UnsupportedSession` without synthetic or fabricated telemetry.
//! - Anti-keylogger / anti-surveillance invariant: never escalates to `root` or `evdev`.

use super::model::WindowInfo;
use super::traits::{ActiveWindowProvider, TrackingError};

/// Truthful Wayland active window provider reporting architectural unprivileged limits.
#[derive(Debug, Clone, Default)]
pub struct WaylandActiveWindowProvider;

impl WaylandActiveWindowProvider {
    pub fn new() -> Self {
        Self
    }
}

impl ActiveWindowProvider for WaylandActiveWindowProvider {
    fn current_active_window(&self) -> Result<Option<WindowInfo>, TrackingError> {
        Err(TrackingError::UnsupportedSession(
            "Active window tracking is architecturally unsupported on unprivileged Wayland sessions without compositor-specific private extensions"
                .into(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wayland_provider_honestly_reports_unsupported_session() {
        let provider = WaylandActiveWindowProvider::new();
        let res = provider.current_active_window();
        match res {
            Err(TrackingError::UnsupportedSession(reason)) => {
                assert!(reason.contains("unprivileged Wayland sessions"));
            }
            _ => panic!("Wayland provider must return TrackingError::UnsupportedSession"),
        }
    }
}
