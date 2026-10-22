//! Lifecycle state machine and acquisition generation invalidation for Dosi-Tracker Linux.
//!
//! Enforces:
//! - Absent, denied, or revoked consent CANNOT start tracking.
//! - Explicit per-capability grant PLUS explicit start required.
//! - Pause, stop, quit, lock, and consent revocation immediately invalidate acquisition admission.
//! - Monotonic generation invalidation protecting against ABA race conditions on in-flight acquisitions.
//! - Restart behavior defined truthfully: starts in Stopped/Unconfigured, never auto-tracking on load.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::consent::{ConsentManager, ConsentedPermissions, LocalConsent};

/// Monotonic generation counter for acquisition ticket invalidation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct LifecycleGeneration(pub u64);

impl LifecycleGeneration {
    pub fn next(&self) -> Self {
        Self(self.0 + 1)
    }
}

/// Supported acquisition capability categories.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum AcquisitionCapability {
    ScreenCapture,
    KeyboardCount,
    MouseCount,
    ActiveWindow,
    RunningPrograms,
    Webcam,
}

/// Reason for entering paused state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PauseReason {
    User,
    IdleTimeout,
    Break,
    ConsentChanged,
}

/// State of the tracker lifecycle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TrackerState {
    /// Initial unconfigured state; no workspace project assigned.
    Unconfigured,
    /// Configured with project policy and local consent, but tracking is stopped (safe idle).
    Stopped,
    /// Actively tracking with valid project policy and valid affirmative local consent.
    Tracking,
    /// Tracking temporarily suspended by user or idle timeout.
    Paused(PauseReason),
    /// Screen or workstation is locked.
    Locked,
    /// Tracker process has terminated.
    Terminated,
}

/// Errors occurring during lifecycle operations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LifecycleError {
    ConsentRequired,
    NoActiveCapabilitiesConsented,
    InvalidStateTransition {
        from: TrackerState,
        to: &'static str,
    },
    Unconfigured,
    ConsentSaveError(String),
}

impl std::fmt::Display for LifecycleError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LifecycleError::ConsentRequired => {
                write!(f, "Tracking requires affirmative local user consent")
            }
            LifecycleError::NoActiveCapabilitiesConsented => {
                write!(
                    f,
                    "All capabilities are disabled in effective policy; tracking cannot start"
                )
            }
            LifecycleError::InvalidStateTransition { from, to } => {
                write!(f, "Invalid lifecycle transition from {:?} to {}", from, to)
            }
            LifecycleError::Unconfigured => {
                write!(f, "Tracker is not configured with a project policy")
            }
            LifecycleError::ConsentSaveError(s) => {
                write!(f, "Failed to persist consent during lifecycle: {}", s)
            }
        }
    }
}

impl std::error::Error for LifecycleError {}

/// Admission ticket issued for a bounded telemetry acquisition sample.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AcquisitionTicket {
    pub generation: LifecycleGeneration,
    pub capability: AcquisitionCapability,
    pub issued_at: DateTime<Utc>,
}

/// Errors during acquisition admission verification.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AcquisitionError {
    NotTracking(TrackerState),
    CapabilityNotPermitted(AcquisitionCapability),
    GenerationMismatch { ticket_gen: u64, current_gen: u64 },
}

impl std::fmt::Display for AcquisitionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AcquisitionError::NotTracking(state) => {
                write!(
                    f,
                    "Acquisition rejected: tracker is in {:?} state, not Tracking",
                    state
                )
            }
            AcquisitionError::CapabilityNotPermitted(cap) => {
                write!(
                    f,
                    "Acquisition rejected: capability {:?} is not permitted by effective policy",
                    cap
                )
            }
            AcquisitionError::GenerationMismatch {
                ticket_gen,
                current_gen,
            } => {
                write!(
                    f,
                    "Acquisition rejected: ticket generation {} is invalid (current generation is {})",
                    ticket_gen, current_gen
                )
            }
        }
    }
}

impl std::error::Error for AcquisitionError {}

/// Production-core tracker lifecycle state machine.
pub struct TrackerLifecycle {
    state: TrackerState,
    generation: LifecycleGeneration,
    project_policy: Option<ConsentedPermissions>,
    consent_manager: ConsentManager,
}

impl TrackerLifecycle {
    /// Initialize a new lifecycle instance.
    /// Invariant: Starts in `TrackerState::Unconfigured` or `TrackerState::Stopped`.
    /// NEVER starts tracking automatically on load.
    pub fn new(consent_manager: ConsentManager) -> Self {
        Self {
            state: TrackerState::Unconfigured,
            generation: LifecycleGeneration(1),
            project_policy: None,
            consent_manager,
        }
    }

    pub fn state(&self) -> TrackerState {
        self.state
    }

    pub fn generation(&self) -> LifecycleGeneration {
        self.generation
    }

    pub fn consent(&self) -> LocalConsent {
        self.consent_manager.current()
    }

    /// Configure with a workspace project policy.
    ///
    /// Invariants:
    /// - Does NOT auto-start; entering from `Unconfigured` moves to `TrackerState::Stopped`.
    /// - If policy changes while already configured, increments generation to invalidate
    ///   stale in-flight acquisition tickets (defeating ABA replay across policy withdrawal/re-grant).
    /// - If tracking and all capabilities are withdrawn by policy, safely transitions to `Stopped`.
    /// - If identical policy is republished, generation is NOT incremented unnecessarily.
    /// - Terminated state remains terminated.
    pub fn configure(&mut self, policy: ConsentedPermissions) {
        let prev_policy = self.project_policy;
        self.project_policy = Some(policy);

        if self.state == TrackerState::Unconfigured {
            self.state = TrackerState::Stopped;
            self.generation = self.generation.next();
            return;
        }

        if self.state == TrackerState::Terminated {
            return;
        }

        if prev_policy != Some(policy) {
            self.generation = self.generation.next();

            if self.state == TrackerState::Tracking
                && !self.effective_permissions().has_any_active_capability()
            {
                self.state = TrackerState::Stopped;
            }
        }
    }

    /// Compute current effective permissions (`ProjectPolicy ∩ LocalConsent`).
    pub fn effective_permissions(&self) -> ConsentedPermissions {
        match &self.project_policy {
            Some(project) => self.consent_manager.effective_permissions(project),
            None => ConsentedPermissions::all_disabled(),
        }
    }

    fn check_capability_permitted(&self, cap: AcquisitionCapability) -> bool {
        let eff = self.effective_permissions();
        match cap {
            AcquisitionCapability::ScreenCapture => eff.screenshot,
            AcquisitionCapability::KeyboardCount => eff.keyboard,
            AcquisitionCapability::MouseCount => eff.mouse,
            AcquisitionCapability::ActiveWindow => eff.active_window,
            AcquisitionCapability::RunningPrograms => eff.running_programs,
            AcquisitionCapability::Webcam => eff.webcam,
        }
    }

    /// Start active tracking.
    ///
    /// Invariants:
    /// - Absent, denied, or revoked consent CANNOT start.
    /// - Explicit per-capability grant + explicit start required.
    /// - At least one capability must be active in effective policy.
    pub fn start(&mut self) -> Result<(), LifecycleError> {
        if self.project_policy.is_none() {
            return Err(LifecycleError::Unconfigured);
        }

        match self.consent_manager.current() {
            LocalConsent::Absent | LocalConsent::Revoked => {
                return Err(LifecycleError::ConsentRequired)
            }
            LocalConsent::Granted(_) => {}
        }

        let effective = self.effective_permissions();
        if !effective.has_any_active_capability() {
            return Err(LifecycleError::NoActiveCapabilitiesConsented);
        }

        match self.state {
            TrackerState::Stopped | TrackerState::Paused(_) => {
                self.state = TrackerState::Tracking;
                self.generation = self.generation.next();
                Ok(())
            }
            TrackerState::Tracking => Ok(()), // Already tracking
            _ => Err(LifecycleError::InvalidStateTransition {
                from: self.state,
                to: "Tracking",
            }),
        }
    }

    /// Pause tracking with reason. Invalidates existing acquisition tickets.
    pub fn pause(&mut self, reason: PauseReason) -> Result<(), LifecycleError> {
        match self.state {
            TrackerState::Tracking => {
                self.state = TrackerState::Paused(reason);
                self.generation = self.generation.next();
                Ok(())
            }
            TrackerState::Paused(_) => Ok(()),
            _ => Err(LifecycleError::InvalidStateTransition {
                from: self.state,
                to: "Paused",
            }),
        }
    }

    /// Resume from paused state.
    pub fn resume(&mut self) -> Result<(), LifecycleError> {
        self.start()
    }

    /// Stop tracking. Invalidates existing acquisition tickets.
    pub fn stop(&mut self) -> Result<(), LifecycleError> {
        match self.state {
            TrackerState::Tracking | TrackerState::Paused(_) | TrackerState::Locked => {
                self.state = TrackerState::Stopped;
                self.generation = self.generation.next();
                Ok(())
            }
            TrackerState::Stopped => Ok(()),
            _ => Err(LifecycleError::InvalidStateTransition {
                from: self.state,
                to: "Stopped",
            }),
        }
    }

    /// Workstation lock event. Invalidates existing acquisition tickets.
    pub fn lock(&mut self) {
        if self.state == TrackerState::Tracking || matches!(self.state, TrackerState::Paused(_)) {
            self.state = TrackerState::Locked;
            self.generation = self.generation.next();
        }
    }

    /// Workstation unlock event. Enters safe stopped or paused state.
    pub fn unlock(&mut self) {
        if self.state == TrackerState::Locked {
            self.state = TrackerState::Paused(PauseReason::User);
            self.generation = self.generation.next();
        }
    }

    /// Quit tracker process.
    pub fn quit(&mut self) {
        self.state = TrackerState::Terminated;
        self.generation = self.generation.next();
    }

    /// Update local consent choices.
    ///
    /// Invariants:
    /// - Reconciles in-memory effective admission and invalidates generation EVEN IF
    ///   persistence fails with an error (`ConsentSaveError`).
    /// - If consent changes (e.g. withdrawal, narrowing, blur tightening), generation is incremented
    ///   so stale tickets from previous intervals can never validate.
    /// - If tracking and all capabilities are withdrawn, transitions safely to Stopped even on save error.
    /// - If consent is identical and unchanged, generation is not bumped unnecessarily.
    pub fn update_consent(&mut self, next: LocalConsent) -> Result<(), LifecycleError> {
        let prev_consent = self.consent_manager.current();
        let prev_effective = self.effective_permissions();

        let update_res = self.consent_manager.update(next);

        let new_consent = self.consent_manager.current();
        let new_effective = self.effective_permissions();

        let changed = prev_consent != new_consent || prev_effective != new_effective;

        if changed {
            self.generation = self.generation.next();

            if self.state == TrackerState::Tracking && !new_effective.has_any_active_capability() {
                self.state = TrackerState::Stopped;
            }
        }

        update_res.map_err(|e| LifecycleError::ConsentSaveError(e.to_string()))
    }

    /// Issue an acquisition ticket for an upcoming sensor sample.
    pub fn acquire_ticket(
        &self,
        cap: AcquisitionCapability,
    ) -> Result<AcquisitionTicket, AcquisitionError> {
        if self.state != TrackerState::Tracking {
            return Err(AcquisitionError::NotTracking(self.state));
        }

        if !self.check_capability_permitted(cap) {
            return Err(AcquisitionError::CapabilityNotPermitted(cap));
        }

        Ok(AcquisitionTicket {
            generation: self.generation,
            capability: cap,
            issued_at: Utc::now(),
        })
    }

    /// Verify an acquisition ticket before committing captured telemetry.
    ///
    /// Invariant:
    /// - Fails closed if generation does not match current lifecycle generation.
    /// - Fails closed if state is not Tracking or if capability is no longer permitted.
    pub fn validate_ticket(&self, ticket: &AcquisitionTicket) -> Result<(), AcquisitionError> {
        if self.state != TrackerState::Tracking {
            return Err(AcquisitionError::NotTracking(self.state));
        }

        if ticket.generation != self.generation {
            return Err(AcquisitionError::GenerationMismatch {
                ticket_gen: ticket.generation.0,
                current_gen: self.generation.0,
            });
        }

        if !self.check_capability_permitted(ticket.capability) {
            return Err(AcquisitionError::CapabilityNotPermitted(ticket.capability));
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup_lifecycle(consent: LocalConsent, policy: ConsentedPermissions) -> TrackerLifecycle {
        let manager = ConsentManager::new(consent, None);
        let mut lc = TrackerLifecycle::new(manager);
        lc.configure(policy);
        lc
    }

    #[test]
    fn starts_in_stopped_or_unconfigured_never_auto_tracking() {
        let manager = ConsentManager::new(LocalConsent::Absent, None);
        let lc = TrackerLifecycle::new(manager);
        assert_eq!(lc.state(), TrackerState::Unconfigured);
    }

    #[test]
    fn absent_or_revoked_consent_cannot_start() {
        let mut policy = ConsentedPermissions::all_disabled();
        policy.keyboard = true;

        let mut lc_absent = setup_lifecycle(LocalConsent::Absent, policy);
        assert_eq!(lc_absent.start(), Err(LifecycleError::ConsentRequired));

        let mut lc_revoked = setup_lifecycle(LocalConsent::Revoked, policy);
        assert_eq!(lc_revoked.start(), Err(LifecycleError::ConsentRequired));
    }

    #[test]
    fn explicit_grant_plus_explicit_start_transitions_to_tracking() {
        let mut user_perms = ConsentedPermissions::all_disabled();
        user_perms.keyboard = true;

        let mut policy = ConsentedPermissions::all_disabled();
        policy.keyboard = true;

        let mut lc = setup_lifecycle(LocalConsent::Granted(user_perms), policy);
        assert_eq!(lc.state(), TrackerState::Stopped);

        lc.start().expect("Start should succeed");
        assert_eq!(lc.state(), TrackerState::Tracking);
    }

    #[test]
    fn policy_cannot_grant_unconsented_capabilities() {
        let user_perms = ConsentedPermissions::all_disabled(); // User consents to nothing

        let mut policy = ConsentedPermissions::all_disabled();
        policy.keyboard = true;
        policy.screenshot = true;

        let mut lc = setup_lifecycle(LocalConsent::Granted(user_perms), policy);
        assert_eq!(
            lc.start(),
            Err(LifecycleError::NoActiveCapabilitiesConsented)
        );
    }

    #[test]
    fn pause_stop_lock_quit_remove_acquisition_admission() {
        let mut perms = ConsentedPermissions::all_disabled();
        perms.keyboard = true;

        let mut lc = setup_lifecycle(LocalConsent::Granted(perms), perms);
        lc.start().unwrap();
        assert_eq!(lc.state(), TrackerState::Tracking);

        // Ticket can be acquired in Tracking
        let ticket = lc
            .acquire_ticket(AcquisitionCapability::KeyboardCount)
            .unwrap();
        assert!(lc.validate_ticket(&ticket).is_ok());

        // Pause invalidates admission
        lc.pause(PauseReason::User).unwrap();
        assert_eq!(
            lc.acquire_ticket(AcquisitionCapability::KeyboardCount),
            Err(AcquisitionError::NotTracking(TrackerState::Paused(
                PauseReason::User
            )))
        );
        assert_eq!(
            lc.validate_ticket(&ticket),
            Err(AcquisitionError::NotTracking(TrackerState::Paused(
                PauseReason::User
            )))
        );

        // Resume, re-acquire, then Stop
        lc.resume().unwrap();
        let ticket2 = lc
            .acquire_ticket(AcquisitionCapability::KeyboardCount)
            .unwrap();
        lc.stop().unwrap();
        assert_eq!(
            lc.validate_ticket(&ticket2),
            Err(AcquisitionError::NotTracking(TrackerState::Stopped))
        );

        // Lock
        lc.start().unwrap();
        let ticket3 = lc
            .acquire_ticket(AcquisitionCapability::KeyboardCount)
            .unwrap();
        lc.lock();
        assert_eq!(
            lc.validate_ticket(&ticket3),
            Err(AcquisitionError::NotTracking(TrackerState::Locked))
        );

        // Quit
        lc.quit();
        assert_eq!(
            lc.validate_ticket(&ticket3),
            Err(AcquisitionError::NotTracking(TrackerState::Terminated))
        );
    }

    #[test]
    fn deterministic_aba_invalidation_test() {
        let mut perms = ConsentedPermissions::all_disabled();
        perms.keyboard = true;

        let mut lc = setup_lifecycle(LocalConsent::Granted(perms), perms);
        lc.start().unwrap();

        // 1. Generation A: Tracking with Keyboard consented
        let gen_a = lc.generation();
        let ticket_a = lc
            .acquire_ticket(AcquisitionCapability::KeyboardCount)
            .unwrap();
        assert_eq!(ticket_a.generation, gen_a);

        // 2. State transition (Pause) increments generation to B
        lc.pause(PauseReason::User).unwrap();
        let gen_b = lc.generation();
        assert!(gen_b > gen_a);

        // 3. State transition (Resume) increments generation to C
        lc.resume().unwrap();
        let gen_c = lc.generation();
        assert!(gen_c > gen_b);

        // 4. In generation C, tracker is once again Tracking and Keyboard is consented!
        assert_eq!(lc.state(), TrackerState::Tracking);
        assert!(lc.effective_permissions().keyboard);

        // 5. Verifying ticket_a (issued in generation A) MUST FAIL due to generation mismatch (ABA defeated)
        let err = lc.validate_ticket(&ticket_a).unwrap_err();
        match err {
            AcquisitionError::GenerationMismatch {
                ticket_gen,
                current_gen,
            } => {
                assert_eq!(ticket_gen, gen_a.0);
                assert_eq!(current_gen, gen_c.0);
            }
            other => panic!("Expected GenerationMismatch, got {:?}", other),
        }

        // A freshly acquired ticket in generation C works normally
        let ticket_c = lc
            .acquire_ticket(AcquisitionCapability::KeyboardCount)
            .unwrap();
        assert!(lc.validate_ticket(&ticket_c).is_ok());
    }

    #[test]
    fn consent_narrowing_stops_tracking_if_no_capabilities_remain() {
        let mut perms = ConsentedPermissions::all_disabled();
        perms.keyboard = true;

        let mut lc = setup_lifecycle(LocalConsent::Granted(perms), perms);
        lc.start().unwrap();
        assert_eq!(lc.state(), TrackerState::Tracking);

        // Narrow consent: revoke keyboard
        lc.update_consent(LocalConsent::Granted(ConsentedPermissions::all_disabled()))
            .unwrap();

        // Tracker should automatically fall back to Stopped
        assert_eq!(lc.state(), TrackerState::Stopped);
    }
}
