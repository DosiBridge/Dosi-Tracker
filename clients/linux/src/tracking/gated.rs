//! Consent-gated acquisition wrapper enforcing lifecycle state and generation tickets.
//!
//! Enforces:
//! - Default-deny: Absent, stopped, paused, or unconfigured tracker CANNOT acquire telemetry.
//! - Per-capability consent gating: `ActiveWindow` and `RunningPrograms` require independent affirmative permission.
//! - In-flight ABA ticket invalidation: If lifecycle generation changes while an acquisition query
//!   is in flight (e.g. consent revoked, paused, or policy tightened), the sample is discarded and
//!   `TrackingError::GenerationMismatch` is returned.
//! - Decoupled concurrency: Lifecycle mutex lock is released during underlying I/O queries.

use std::sync::{Arc, Mutex};

use super::model::WindowInfo;
use super::traits::{ActiveWindowProvider, RunningAppsProvider, TrackingError};
use crate::lifecycle::{AcquisitionCapability, AcquisitionError, TrackerLifecycle};

/// Wrapper that enforces affirmative consent and ticket generation checks on every acquisition.
pub struct ConsentGatedTracker {
    active_window: Arc<dyn ActiveWindowProvider>,
    running_apps: Arc<dyn RunningAppsProvider>,
    lifecycle: Arc<Mutex<TrackerLifecycle>>,
}

impl ConsentGatedTracker {
    pub fn new(
        active_window: Arc<dyn ActiveWindowProvider>,
        running_apps: Arc<dyn RunningAppsProvider>,
        lifecycle: Arc<Mutex<TrackerLifecycle>>,
    ) -> Self {
        Self {
            active_window,
            running_apps,
            lifecycle,
        }
    }

    fn map_acquisition_error(err: AcquisitionError) -> TrackingError {
        match err {
            AcquisitionError::NotTracking(_) => TrackingError::ConsentRequired,
            AcquisitionError::CapabilityNotPermitted(cap) => TrackingError::CapabilityDenied(cap),
            AcquisitionError::GenerationMismatch {
                ticket_gen,
                current_gen,
            } => TrackingError::GenerationMismatch {
                ticket_gen,
                current_gen,
            },
        }
    }
}

impl ActiveWindowProvider for ConsentGatedTracker {
    fn current_active_window(&self) -> Result<Option<WindowInfo>, TrackingError> {
        // 1. Pre-acquisition: acquire admission ticket
        let ticket = {
            let lifecycle = self
                .lifecycle
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            lifecycle
                .acquire_ticket(AcquisitionCapability::ActiveWindow)
                .map_err(Self::map_acquisition_error)?
        };

        // 2. Acquisition execution: read without holding the lifecycle lock
        let sample = self.active_window.current_active_window()?;

        // 3. Post-acquisition: validate ticket generation before returning
        {
            let lifecycle = self
                .lifecycle
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            lifecycle
                .validate_ticket(&ticket)
                .map_err(Self::map_acquisition_error)?;
        }

        Ok(sample)
    }
}

impl RunningAppsProvider for ConsentGatedTracker {
    fn running_programs(&self) -> Result<Vec<WindowInfo>, TrackingError> {
        // 1. Pre-acquisition: acquire admission ticket
        let ticket = {
            let lifecycle = self
                .lifecycle
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            lifecycle
                .acquire_ticket(AcquisitionCapability::RunningPrograms)
                .map_err(Self::map_acquisition_error)?
        };

        // 2. Acquisition execution: read without holding the lifecycle lock
        let sample = self.running_apps.running_programs()?;

        // 3. Post-acquisition: validate ticket generation before returning
        {
            let lifecycle = self
                .lifecycle
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            lifecycle
                .validate_ticket(&ticket)
                .map_err(Self::map_acquisition_error)?;
        }

        Ok(sample)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::consent::{ConsentManager, ConsentedPermissions, LocalConsent};
    use crate::lifecycle::{PauseReason, TrackerLifecycle};
    use crate::tracking::fake::FakeAcquisitionProvider;

    fn create_test_lifecycle() -> Arc<Mutex<TrackerLifecycle>> {
        let consent_mgr = ConsentManager::new(LocalConsent::Absent, None);
        let lifecycle = TrackerLifecycle::new(consent_mgr);
        Arc::new(Mutex::new(lifecycle))
    }

    #[test]
    fn default_deny_refuses_window_and_running_apps() {
        let lifecycle = create_test_lifecycle();
        let fake = Arc::new(FakeAcquisitionProvider::new());
        let tracker = ConsentGatedTracker::new(fake.clone(), fake.clone(), lifecycle);

        // Not configured and not tracking -> ConsentRequired
        assert_eq!(
            tracker.current_active_window(),
            Err(TrackingError::ConsentRequired)
        );
        assert_eq!(
            tracker.running_programs(),
            Err(TrackingError::ConsentRequired)
        );
        assert_eq!(fake.window_calls(), 0);
        assert_eq!(fake.process_calls(), 0);
    }

    #[test]
    fn capability_specific_consent_permits_only_consented_capability() {
        let lifecycle = create_test_lifecycle();
        let fake = Arc::new(FakeAcquisitionProvider::new());
        let tracker = ConsentGatedTracker::new(fake.clone(), fake.clone(), lifecycle.clone());

        // Configure project policy permitting both
        let policy = ConsentedPermissions {
            active_window: true,
            running_programs: true,
            ..ConsentedPermissions::all_disabled()
        };

        // Grant local consent ONLY for active_window
        let local = ConsentedPermissions {
            active_window: true,
            running_programs: false,
            ..ConsentedPermissions::all_disabled()
        };

        {
            let mut lc = lifecycle.lock().unwrap();
            lc.configure(policy);
            lc.update_consent(LocalConsent::Granted(local)).unwrap();
            lc.start().unwrap();
        }

        // Active window succeeds
        let win = tracker
            .current_active_window()
            .expect("Window must succeed");
        assert!(win.is_some());
        assert_eq!(fake.window_calls(), 1);

        // Running programs fails closed with CapabilityDenied
        let err = tracker.running_programs();
        assert_eq!(
            err,
            Err(TrackingError::CapabilityDenied(
                AcquisitionCapability::RunningPrograms
            ))
        );
        assert_eq!(fake.process_calls(), 0);
    }

    #[test]
    fn in_flight_consent_revocation_invalidates_ticket_and_discards_sample() {
        let lifecycle = create_test_lifecycle();
        let fake = Arc::new(FakeAcquisitionProvider::new());

        let policy = ConsentedPermissions {
            active_window: true,
            ..ConsentedPermissions::all_disabled()
        };
        let local = ConsentedPermissions {
            active_window: true,
            ..ConsentedPermissions::all_disabled()
        };

        {
            let mut lc = lifecycle.lock().unwrap();
            lc.configure(policy);
            lc.update_consent(LocalConsent::Granted(local)).unwrap();
            lc.start().unwrap();
        }

        // Set hook on fake to revoke consent during the in-flight window query
        let lc_hook = lifecycle.clone();
        fake.set_on_current_window(Box::new(move || {
            let mut lc = lc_hook.lock().unwrap();
            lc.pause(PauseReason::User).unwrap();
        }));

        let tracker = ConsentGatedTracker::new(fake.clone(), fake.clone(), lifecycle);
        let res = tracker.current_active_window();

        // Must fail with GenerationMismatch (sample discarded)
        assert!(matches!(
            res,
            Err(TrackingError::GenerationMismatch { .. }) | Err(TrackingError::ConsentRequired)
        ));
    }
}
