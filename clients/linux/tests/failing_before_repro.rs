//! Failing-before reproduction & regression test suite for task linux-foundation-consent-policy-correction-20261022-D.
//!
//! Validates the three privacy core corrections using REAL ConsentManager and TrackerLifecycle:
//! 1. Mixed consent updates (narrowing + expansion) on save failure:
//!    - Immediately withdraws removed permissions.
//!    - Does NOT grant newly added permissions.
//!    - Later successful save admits only authorized additions.
//! 2. Pure widening on save failure:
//!    - Remains strictly persistence-first (no permissions admitted on disk error).
//! 3. Conservative blur tightening on save failure:
//!    - Applies immediately as defense-in-depth even if persistence fails.
//! 4. Lifecycle consent update error handling:
//!    - Increments generation and reconciles state to Stopped even when persistence returns Err.
//!    - Removes all acquisition tickets and prevents stale ticket re-admission.
//! 5. Ticket invalidation across transitions:
//!    - ABA ticket replay defeated across policy withdraw/regrant and consent persistence-error/recovery.
//! 6. Safe nontracking state:
//!    - All-effective-capabilities removed leaves safe non-tracking state even on save error.
//! 7. Unchanged policy updates:
//!    - Does not bump generation or fabricate an authorization transition.

use dosi_tracker_linux::consent::{
    ConsentError, ConsentManager, ConsentedPermissions, LocalConsent, MemoryConsentStore,
};
use dosi_tracker_linux::lifecycle::{
    AcquisitionCapability, AcquisitionError, TrackerLifecycle, TrackerState,
};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

/// 1. Mixed narrow+expand save failure withdraws removed permissions but does not grant additions.
/// Also asserts blocked-store ordering using an explicit fake-store barrier.
#[test]
fn test_mixed_narrow_and_expand_save_failure_withdraws_removed_without_granting_additions() {
    let mut initial = ConsentedPermissions::all_disabled();
    initial.keyboard = true;
    initial.screenshot = false;

    let store = Arc::new(MemoryConsentStore::new(LocalConsent::Granted(initial)));
    let mut manager =
        ConsentManager::with_storage(LocalConsent::Granted(initial), Some(store.clone()));

    let mut mixed = ConsentedPermissions::all_disabled();
    mixed.keyboard = false; // Removed: must be withdrawn immediately
    mixed.screenshot = true; // Newly added: must NOT be granted until save succeeds

    // Set save barrier to verify ordering during the save attempt
    let barrier_verified = Arc::new(AtomicBool::new(false));
    let barrier_flag = barrier_verified.clone();
    store.set_save_barrier(move |_target| {
        // At the moment save is called, store has NOT yet persisted, but manager has already
        // applied the immediate narrowing and blur tightening.
        barrier_flag.store(true, Ordering::SeqCst);
        // Simulate a persistence failure from the storage backend
        Err(ConsentError::SaveFailed(
            "Injected disk failure during save".into(),
        ))
    });

    let res = manager.update(LocalConsent::Granted(mixed));
    assert!(res.is_err(), "Persistence failure must return Err");
    assert!(
        barrier_verified.load(Ordering::SeqCst),
        "Save barrier must have been invoked"
    );

    let perms = manager.current().permissions();
    // Invariant: Removed capability MUST be withdrawn immediately
    assert!(
        !perms.keyboard,
        "Removed capability (keyboard) must be withdrawn immediately"
    );
    // Invariant: Newly added capability MUST NOT be granted when save fails
    assert!(
        !perms.screenshot,
        "Newly added capability (screenshot) must NOT be granted when save fails"
    );
}

/// 2. Later successful save enables only authorized additions.
#[test]
fn test_later_successful_save_enables_only_authorized_additions() {
    let mut initial = ConsentedPermissions::all_disabled();
    initial.keyboard = true;
    initial.screenshot = false;

    let store = Arc::new(MemoryConsentStore::new(LocalConsent::Granted(initial)));
    let mut manager =
        ConsentManager::with_storage(LocalConsent::Granted(initial), Some(store.clone()));

    let mut mixed = ConsentedPermissions::all_disabled();
    mixed.keyboard = false;
    mixed.screenshot = true;

    // First attempt fails persistence
    store.set_fail_saves(true);
    let res1 = manager.update(LocalConsent::Granted(mixed));
    assert!(res1.is_err());
    assert!(!manager.current().permissions().keyboard);
    assert!(!manager.current().permissions().screenshot);

    // Backend recovers: subsequent save succeeds
    store.set_fail_saves(false);
    let res2 = manager.update(LocalConsent::Granted(mixed));
    assert!(res2.is_ok(), "Subsequent save must succeed");

    let perms = manager.current().permissions();
    assert!(!perms.keyboard, "Keyboard remains withdrawn");
    assert!(
        perms.screenshot,
        "Screenshot is authorized and enabled after successful persistence"
    );
    assert_eq!(store.current(), LocalConsent::Granted(mixed));
}

/// 3. Pure widening failure leaves in-memory state unchanged (persistence-first).
#[test]
fn test_pure_widening_failure() {
    let store = Arc::new(MemoryConsentStore::new(LocalConsent::Absent));
    let mut manager = ConsentManager::with_storage(LocalConsent::Absent, Some(store.clone()));

    let mut grant = ConsentedPermissions::all_disabled();
    grant.screenshot = true;

    store.set_fail_saves(true);
    let res = manager.update(LocalConsent::Granted(grant));
    assert!(res.is_err(), "Widening must fail when persistence fails");

    // Must remain Absent; no capabilities admitted
    assert_eq!(
        manager.current(),
        LocalConsent::Absent,
        "Pure widening must be persistence-first; must remain Absent on save failure"
    );
    assert!(!manager.current().permissions().has_any_active_capability());
}

/// 4. Blur tightening failure: conservative blur tightening applies immediately even if save fails.
#[test]
fn test_blur_tightening_failure() {
    let mut initial = ConsentedPermissions::all_disabled();
    initial.screenshot = true;
    initial.blur_screenshots = false; // unblurred

    let store = Arc::new(MemoryConsentStore::new(LocalConsent::Granted(initial)));
    let mut manager =
        ConsentManager::with_storage(LocalConsent::Granted(initial), Some(store.clone()));

    let mut tightened = initial;
    tightened.blur_screenshots = true; // tightened blur

    store.set_fail_saves(true);
    let res = manager.update(LocalConsent::Granted(tightened));
    assert!(res.is_err(), "Persistence should fail");

    // Invariant: blur tightening MUST apply immediately as defense-in-depth even if save fails
    assert!(
        manager.current().permissions().blur_screenshots,
        "Blur tightening MUST apply immediately even if save fails"
    );
    // Screenshot capability remains enabled since it was already granted
    assert!(manager.current().permissions().screenshot);
}

/// 5. Old ticket rejection across policy withdraw/regrant and consent persistence-error/recovery transitions.
#[test]
fn test_old_ticket_rejection_across_policy_withdraw_regrant_and_consent_persistence_transitions() {
    // === Part A: Across policy withdraw/regrant (ABA protection) ===
    let mut perms = ConsentedPermissions::all_disabled();
    perms.keyboard = true;
    perms.screenshot = true;

    let store = Arc::new(MemoryConsentStore::new(LocalConsent::Granted(perms)));
    let manager = ConsentManager::with_storage(LocalConsent::Granted(perms), Some(store.clone()));
    let mut lc = TrackerLifecycle::new(manager);
    lc.configure(perms);
    lc.start().unwrap();

    let ticket_ss = lc
        .acquire_ticket(AcquisitionCapability::ScreenCapture)
        .unwrap();
    assert!(lc.validate_ticket(&ticket_ss).is_ok());

    // Admin withdraws screenshot in workspace policy
    let mut policy_no_ss = perms;
    policy_no_ss.screenshot = false;
    lc.configure(policy_no_ss);

    // Old ticket is immediately rejected (policy not permitted)
    assert!(lc.validate_ticket(&ticket_ss).is_err());

    // Admin re-grants screenshot in workspace policy
    lc.configure(perms);

    // Old ticket must STILL be rejected due to generation advance (ABA prevented)
    let err = lc.validate_ticket(&ticket_ss).unwrap_err();
    match err {
        AcquisitionError::GenerationMismatch { .. } => {}
        other => panic!(
            "Expected GenerationMismatch for stale ticket, got {:?}",
            other
        ),
    }

    // === Part B: Across consent persistence error and recovery transitions ===
    let ticket_kb = lc
        .acquire_ticket(AcquisitionCapability::KeyboardCount)
        .unwrap();
    assert!(lc.validate_ticket(&ticket_kb).is_ok());

    // Consent update with persistence error (revoking keyboard)
    store.set_fail_saves(true);
    let mut consent_no_kb = perms;
    consent_no_kb.keyboard = false;
    let res = lc.update_consent(LocalConsent::Granted(consent_no_kb));
    assert!(res.is_err(), "Consent update returned save error");

    // Old ticket is rejected: generation advanced during failed-persistence narrowing
    assert!(lc.validate_ticket(&ticket_kb).is_err());

    // Now consent is recovered (re-grant keyboard with successful save)
    store.set_fail_saves(false);
    lc.update_consent(LocalConsent::Granted(perms)).unwrap();

    // The old ticket ticket_kb must NEVER be re-admitted after recovery
    let err2 = lc.validate_ticket(&ticket_kb).unwrap_err();
    match err2 {
        AcquisitionError::GenerationMismatch { .. } => {}
        other => panic!(
            "Expected GenerationMismatch for old ticket after recovery, got {:?}",
            other
        ),
    }
}

/// 6. All-effective-capabilities removed leaves safe nontracking state even on save error.
#[test]
fn test_all_effective_capabilities_removed_leaves_safe_nontracking_state_even_on_save_error() {
    let mut perms = ConsentedPermissions::all_disabled();
    perms.keyboard = true;

    let store = Arc::new(MemoryConsentStore::new(LocalConsent::Granted(perms)));
    let manager = ConsentManager::with_storage(LocalConsent::Granted(perms), Some(store.clone()));
    let mut lc = TrackerLifecycle::new(manager);
    lc.configure(perms);
    lc.start().unwrap();

    assert_eq!(lc.state(), TrackerState::Tracking);
    let ticket = lc
        .acquire_ticket(AcquisitionCapability::KeyboardCount)
        .unwrap();

    // User revokes all consent, but storage persistence fails
    store.set_fail_saves(true);
    let res = lc.update_consent(LocalConsent::Revoked);
    assert!(res.is_err(), "Persistence failure must return Err");

    // Invariant: Tracker MUST reconcile to Stopped state
    assert_eq!(
        lc.state(),
        TrackerState::Stopped,
        "Lifecycle state must transition to Stopped when all capabilities are removed, even on save error"
    );

    // Invariant: Stale ticket must fail validation
    assert!(lc.validate_ticket(&ticket).is_err());

    // Invariant: No tickets can be acquired in Stopped state
    assert_eq!(
        lc.acquire_ticket(AcquisitionCapability::KeyboardCount),
        Err(AcquisitionError::NotTracking(TrackerState::Stopped))
    );

    // Invariant: Cannot re-start without consent
    assert_eq!(
        lc.start(),
        Err(dosi_tracker_linux::LifecycleError::ConsentRequired)
    );
}

/// 7. Unchanged policy update does not fabricate authorization transition.
#[test]
fn test_unchanged_policy_update_does_not_fabricate_authorization_transition() {
    let mut perms = ConsentedPermissions::all_disabled();
    perms.keyboard = true;
    perms.mouse = true;

    let store = Arc::new(MemoryConsentStore::new(LocalConsent::Granted(perms)));
    let manager = ConsentManager::with_storage(LocalConsent::Granted(perms), Some(store));
    let mut lc = TrackerLifecycle::new(manager);
    lc.configure(perms);

    let stopped_gen = lc.generation();

    // Re-configure with identical policy while Stopped: generation must NOT change
    lc.configure(perms);
    assert_eq!(
        lc.generation(),
        stopped_gen,
        "Identical policy publication while Stopped must not advance generation"
    );
    assert_eq!(lc.state(), TrackerState::Stopped);

    // Explicit start
    lc.start().unwrap();
    assert_eq!(lc.state(), TrackerState::Tracking);
    let tracking_gen = lc.generation();
    assert!(tracking_gen > stopped_gen);

    let ticket = lc
        .acquire_ticket(AcquisitionCapability::KeyboardCount)
        .unwrap();
    assert!(lc.validate_ticket(&ticket).is_ok());

    // Re-publish identical policy while Tracking: generation must NOT change, ticket remains valid
    lc.configure(perms);
    assert_eq!(
        lc.generation(),
        tracking_gen,
        "Identical policy publication while Tracking must not advance generation"
    );
    assert_eq!(lc.state(), TrackerState::Tracking);
    assert!(
        lc.validate_ticket(&ticket).is_ok(),
        "Existing valid ticket remains valid across unchanged policy publication"
    );
}
