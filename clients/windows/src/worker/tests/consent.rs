//! Local consent: granting, narrowing and revoking it, and saving it.

use super::*;

// ---- Local consent UX and lifecycle integration tests ---------------------------------

#[test]
fn consent_1_fresh_config_absent() {
    let mut h = harness_with_consent(
        at("10:00:00"),
        Some(saved(vec![project("p1", 5)], Some("p1"))),
        LocalConsent::Absent,
    );
    assert_eq!(h.status(), TrackerStatus::ConsentRequired);
    assert!(h.tracking_since().is_none());
    assert!(!h.machine.is_tracking_active());
    assert!(!h.machine.is_input_running());
    assert!(!h.machine.is_usage_running());
    assert_eq!(h.machine.input_starts(), 0);
    assert_eq!(h.machine.usage_starts(), 0);

    h.tick(at("10:01:00"));
    assert_eq!(h.status(), TrackerStatus::ConsentRequired);
    assert_eq!(h.machine.input_starts(), 0);
    assert_eq!(h.machine.usage_starts(), 0);
}

#[test]
fn consent_2_legacy_config_absent() {
    let cfg: AppConfig = serde_json::from_str("{}").expect("deserialization from empty json must succeed");
    assert_eq!(cfg.consent, LocalConsent::Absent, "legacy config must fail closed to Absent");

    let nonexistent = PathBuf::from("dosi_nonexistent_consent_file_12345.json");
    let loaded = LocalConsent::load_from(&nonexistent);
    assert_eq!(loaded, LocalConsent::Absent, "missing consent file must fail closed to Absent");

    let h = harness_with_consent(
        at("10:00:00"),
        Some(saved(vec![project("p1", 5)], Some("p1"))),
        cfg.consent,
    );
    assert_eq!(h.status(), TrackerStatus::ConsentRequired);
    assert!(!h.machine.is_tracking_active());
    assert_eq!(h.machine.input_starts(), 0);
    assert_eq!(h.machine.usage_starts(), 0);
}

#[test]
fn consent_3_explicit_granular_grant() {
    let mut h = harness_with_consent(
        at("10:00:00"),
        Some(saved(vec![project("p1", 5)], Some("p1"))),
        LocalConsent::Absent,
    );
    assert_eq!(h.status(), TrackerStatus::ConsentRequired);
    assert_eq!(h.machine.input_starts(), 0);

    let granular = ConsentedPermissions {
        keyboard: true,
        mouse: false,
        active_window: false,
        running_programs: false,
        screenshot: false,
        webcam: false,
        blur_screenshots: false,
    };
    h.command(at("10:00:30"), Command::UpdateConsent { consent: LocalConsent::Granted(granular) });
    assert_eq!(h.status(), TrackerStatus::Tracking);
    assert!(h.machine.is_input_running(), "input hooks attached for keyboard");
    assert!(h.machine.last_keyboard.load(std::sync::atomic::Ordering::SeqCst));
    assert!(!h.machine.last_mouse.load(std::sync::atomic::Ordering::SeqCst), "mouse not consented");
    assert!(!h.machine.is_usage_running(), "active window not consented");
    assert_eq!(h.machine.usage_starts(), 0);
}

#[test]
fn consent_4_decline_prevents_tracking() {
    let mut h = harness_with_consent(
        at("10:00:00"),
        Some(saved(vec![project("p1", 5)], Some("p1"))),
        LocalConsent::Absent,
    );
    h.command(at("10:00:30"), Command::UpdateConsent { consent: LocalConsent::Revoked });
    assert_eq!(h.status(), TrackerStatus::ConsentRequired);

    h.command(at("10:01:00"), Command::SelectProject { project_id: "p1".into(), at: at("10:01:00") });
    h.command(at("10:01:30"), Command::Resume { at: at("10:01:30") });
    assert_eq!(h.status(), TrackerStatus::ConsentRequired);
    assert_eq!(h.machine.input_starts(), 0);
    assert_eq!(h.machine.usage_starts(), 0);
}

#[test]
fn consent_5_revoke_while_active() {
    let mut h = harness_with_consent(
        at("10:00:00"),
        Some(saved(vec![project("p1", 5)], Some("p1"))),
        LocalConsent::Granted(CapturePermissions::default().into()),
    );
    assert_eq!(h.status(), TrackerStatus::Tracking);
    assert!(h.machine.is_input_running());
    assert!(h.machine.is_usage_running());

    h.command(at("10:02:00"), Command::UpdateConsent { consent: LocalConsent::Revoked });
    assert_eq!(h.status(), TrackerStatus::ConsentRequired);
    assert!(!h.machine.is_tracking_active());
    assert!(!h.machine.is_input_running());
    assert!(!h.machine.is_usage_running());
    assert!(h.machine.input_stops() >= 1);
    assert!(h.machine.usage_stops() >= 1);
}

#[test]
fn consent_6_policy_narrowing() {
    let mut h = harness_with_consent(
        at("10:00:00"),
        Some(saved(vec![project("p1", 5)], Some("p1"))),
        LocalConsent::Granted(CapturePermissions::default().into()),
    );
    assert_eq!(h.status(), TrackerStatus::Tracking);
    assert!(h.machine.is_input_running());
    assert!(h.machine.is_usage_running());

    // Narrowing by user consent (uncheck active_window)
    let mut user_narrowed: ConsentedPermissions = CapturePermissions::default().into();
    user_narrowed.active_window = false;
    h.command(at("10:01:00"), Command::UpdateConsent { consent: LocalConsent::Granted(user_narrowed) });
    assert_eq!(h.status(), TrackerStatus::Tracking);
    assert!(h.machine.is_input_running());
    assert!(!h.machine.is_usage_running(), "usage stopped after user narrowed consent");
    assert_eq!(h.machine.usage_stops(), 1);

    // Narrowing by server project policy (disallow keyboard and mouse)
    let restricted_project: Project = serde_json::from_value(serde_json::json!({
        "id": "p1", "title": "Project p1", "intervalMinutes": 10,
        "allowScreenshot": true, "allowWebcam": false,
        "allowKeyboard": false, "allowMouse": false,
        "allowActiveWindow": false, "allowRunningPrograms": false,
        "blurScreenshots": true,
    })).unwrap();
    h.worker.on_control(ControlEvent::Projects(vec![restricted_project]));
    h.worker.publish();
    assert!(!h.machine.is_input_running(), "input stopped when workspace policy disallowed it");
    assert!(h.machine.input_stops() >= 1);
}

#[test]
fn consent_7_project_workspace_logout_transition() {
    let mut h = harness_with_consent(
        at("10:00:00"),
        Some(saved(vec![project("p1", 5), project("p2", 5)], Some("p1"))),
        LocalConsent::Granted(CapturePermissions::default().into()),
    );
    assert_eq!(h.status(), TrackerStatus::Tracking);
    assert!(h.machine.is_tracking_active());

    // Logout immediately stops sensors
    h.command(at("10:05:00"), Command::SignOut { at: at("10:05:00") });
    assert_eq!(h.status(), TrackerStatus::SignedOut);
    assert!(!h.machine.is_tracking_active());
    assert!(!h.machine.is_input_running());
    assert!(!h.machine.is_usage_running());

    // Logging in when consent is revoked does not auto-start
    let h_revoked = harness_with_consent(
        at("10:10:00"),
        Some(saved(vec![project("p2", 5)], Some("p2"))),
        LocalConsent::Revoked,
    );
    assert_eq!(h_revoked.status(), TrackerStatus::ConsentRequired);
    assert!(!h_revoked.machine.is_tracking_active());
    assert_eq!(h_revoked.machine.input_starts(), 0);
    assert_eq!(h_revoked.machine.usage_starts(), 0);
}

#[test]
fn consent_8_persisted_explicit_choice() {
    let dir = std::env::temp_dir().join(format!("dosi-test-consent-{}", uuid::Uuid::new_v4()));
    let path = dir.join("consent.json");

    let grant = LocalConsent::Granted(ConsentedPermissions {
        screenshot: true,
        webcam: false,
        keyboard: true,
        mouse: false,
        active_window: true,
        running_programs: false,
        blur_screenshots: true,
    });
    grant.save_to(&path).expect("save_to must succeed");

    let loaded = LocalConsent::load_from(&path);
    assert_eq!(loaded, grant, "loaded consent must match saved consent exactly");

    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn consent_9_corrupted_config_fail_closed() {
    let dir = std::env::temp_dir().join(format!("dosi-test-corrupt-{}", uuid::Uuid::new_v4()));
    let path = dir.join("consent.json");
    std::fs::create_dir_all(&dir).unwrap();

    // Empty file
    std::fs::write(&path, b"").unwrap();
    assert_eq!(LocalConsent::load_from(&path), LocalConsent::Absent);

    // Corrupted JSON
    std::fs::write(&path, b"{ not valid json ::: ").unwrap();
    assert_eq!(LocalConsent::load_from(&path), LocalConsent::Absent);

    // Invalid schema
    std::fs::write(&path, b"{\"Granted\": 99999}").unwrap();
    assert_eq!(LocalConsent::load_from(&path), LocalConsent::Absent);

    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn consent_10_pause_resume_quit() {
    let mut h = harness_with_consent(
        at("10:00:00"),
        Some(saved(vec![project("p1", 5)], Some("p1"))),
        LocalConsent::Granted(CapturePermissions::default().into()),
    );
    assert_eq!(h.status(), TrackerStatus::Tracking);
    assert!(h.machine.is_tracking_active());

    // Pause
    h.command(at("10:02:00"), Command::Pause { resume_after: None, at: at("10:02:00") });
    assert_eq!(h.status(), TrackerStatus::Paused);
    assert!(!h.machine.is_tracking_active());

    // Revoke consent while paused
    h.command(at("10:02:30"), Command::UpdateConsent { consent: LocalConsent::Revoked });
    assert_eq!(h.status(), TrackerStatus::ConsentRequired);

    // Attempt to resume fails closed
    h.command(at("10:03:00"), Command::Resume { at: at("10:03:00") });
    assert_eq!(h.status(), TrackerStatus::ConsentRequired);
    assert!(!h.machine.is_tracking_active());

    // Re-grant and resume
    h.command(at("10:03:30"), Command::UpdateConsent { consent: LocalConsent::Granted(CapturePermissions::default().into()) });
    assert_eq!(h.status(), TrackerStatus::Tracking);
    assert!(h.machine.is_tracking_active());

    // System lock stops tracking
    h.system(SystemEvent::Locked, at("10:04:00"));
    assert_eq!(h.status(), TrackerStatus::Locked);
    assert!(!h.machine.is_tracking_active());

    // Revoke during lock
    h.command(at("10:04:30"), Command::UpdateConsent { consent: LocalConsent::Revoked });
    h.system(SystemEvent::Unlocked, at("10:05:00"));
    assert_eq!(h.status(), TrackerStatus::ConsentRequired);
    assert!(!h.machine.is_tracking_active(), "sensors must not restart after permission loss during lock");
}

#[test]
fn consent_11_no_duplicate_thread_start_on_repeated_user_actions() {
    let mut h = harness_with_consent(
        at("10:00:00"),
        Some(saved(vec![project("p1", 5)], Some("p1"))),
        LocalConsent::Granted(CapturePermissions::default().into()),
    );
    assert_eq!(h.status(), TrackerStatus::Tracking);
    let initial_input_starts = h.machine.input_starts();
    let initial_usage_starts = h.machine.usage_starts();

    // Repeated identical consent updates
    for i in 1..=5 {
        h.command(
            at(&format!("10:01:{:02}", i)),
            Command::UpdateConsent { consent: LocalConsent::Granted(CapturePermissions::default().into()) },
        );
    }
    assert_eq!(h.machine.input_starts(), initial_input_starts, "no duplicate input starts on repeated grant");
    assert_eq!(h.machine.usage_starts(), initial_usage_starts, "no duplicate usage starts on repeated grant");

    // Repeated resume and project selection
    for i in 1..=5 {
        h.command(at(&format!("10:02:{:02}", i)), Command::Resume { at: at(&format!("10:02:{:02}", i)) });
        h.command(
            at(&format!("10:03:{:02}", i)),
            Command::SelectProject { project_id: "p1".into(), at: at(&format!("10:03:{:02}", i)) },
        );
    }
    assert_eq!(h.machine.input_starts(), initial_input_starts, "repeated redundant commands do not duplicate threads");
    assert_eq!(h.machine.usage_starts(), initial_usage_starts);
}

#[test]
fn consent_12_grant_persistence_failure_blocks_sensors_and_reports_error() {
    let mut h = harness_with_consent(
        at("10:00:00"),
        Some(saved(vec![project("p1", 5)], Some("p1"))),
        LocalConsent::Absent,
    );
    assert_eq!(h.status(), TrackerStatus::ConsentRequired);
    assert!(!h.machine.is_tracking_active());

    // Inject disk write fault
    h.consent_store.set_fail_saves(true);

    let grant = ConsentedPermissions {
        keyboard: true,
        mouse: true,
        screenshot: false,
        webcam: false,
        active_window: false,
        running_programs: false,
        blur_screenshots: true,
    };
    h.command(at("10:00:30"), Command::UpdateConsent { consent: LocalConsent::Granted(grant) });

    // Invariant: granting must not start sensors until safely accepted/persisted!
    assert_eq!(h.status(), TrackerStatus::ConsentRequired);
    assert!(!h.machine.is_tracking_active(), "sensors must NOT start on persistence failure");
    assert!(!h.machine.is_input_running(), "input hooks must NOT start");
    assert!(!h.machine.is_usage_running(), "usage sampler must NOT start");
    assert_eq!(h.machine.input_starts(), 0);
    assert_eq!(h.machine.usage_starts(), 0);

    // Error state surfaced actionably
    let snapshot = h.state.snapshot();
    assert!(snapshot.consent_error.is_some(), "persistence error must be reported");
    assert!(snapshot.consent_unsynced, "consent_unsynced flag must be true");
    assert_eq!(snapshot.consent, LocalConsent::Absent, "grant not accepted into state on write failure");
}

#[test]
fn consent_13_revoke_persistence_failure_stops_sensors_and_reports_non_durable() {
    let mut h = harness_with_consent(
        at("10:00:00"),
        Some(saved(vec![project("p1", 5)], Some("p1"))),
        LocalConsent::Granted(CapturePermissions::default().into()),
    );
    assert_eq!(h.status(), TrackerStatus::Tracking);
    assert!(h.machine.is_tracking_active());
    assert!(h.machine.is_input_running());

    // Inject disk write fault
    h.consent_store.set_fail_saves(true);

    // Revoke consent
    h.command(at("10:01:00"), Command::UpdateConsent { consent: LocalConsent::Revoked });

    // Invariant: revocation must stop sensors IMMEDIATELY even if disk fails!
    assert_eq!(h.status(), TrackerStatus::ConsentRequired);
    assert!(!h.machine.is_tracking_active(), "sensors must stop immediately on revoke");
    assert!(!h.machine.is_input_running(), "input hooks must be detached");
    assert!(!h.machine.is_usage_running(), "usage sampler must be stopped");
    assert!(h.machine.input_stops() >= 1);

    // State reports non-durable revocation clearly
    let snapshot = h.state.snapshot();
    assert!(snapshot.consent_error.is_some(), "error message must describe non-durable disk write");
    let err_msg = snapshot.consent_error.as_ref().unwrap();
    assert!(err_msg.contains("non-durable across app restart"), "error describes restart durability limit honestly");
    assert!(snapshot.consent_unsynced, "consent_unsynced flag must be true");

    // Unintended resume must be prevented while unsynced/revoked
    h.command(at("10:02:00"), Command::Resume { at: at("10:02:00") });
    assert_eq!(h.status(), TrackerStatus::ConsentRequired);
    assert!(!h.machine.is_tracking_active(), "resume must be blocked while consent is unsynced/revoked");
}

#[test]
fn consent_14_fake_worker_isolation_proves_no_disk_access() {
    // Proves that fake worker constructors and tests interact strictly with MemoryConsentStore.
    let mut h = harness_with_consent(
        at("10:00:00"),
        Some(saved(vec![project("p1", 5)], Some("p1"))),
        LocalConsent::Absent,
    );
    assert_eq!(h.consent_store.load_count(), 1, "loaded from MemoryConsentStore on start");
    assert_eq!(h.consent_store.save_count(), 0);

    let grant = ConsentedPermissions::default();
    h.command(at("10:00:30"), Command::UpdateConsent { consent: LocalConsent::Granted(grant) });
    assert_eq!(h.consent_store.save_count(), 1, "saved to MemoryConsentStore on update");

    h.command(at("10:01:00"), Command::UpdateConsent { consent: LocalConsent::Revoked });
    assert_eq!(h.consent_store.save_count(), 2, "second save went to MemoryConsentStore");

    // No real disk consent file touched: MemoryConsentStore keeps state in memory mutex
    assert_eq!(h.consent_store.load(), LocalConsent::Revoked);
}

#[test]
fn consent_15_partial_grant_omitted_capabilities_strictly_default_deny() {
    // Prove that partial grant schema defaults ALL omitted capabilities strictly to false
    let json_only_mouse = r#"{"Granted": {"mouse": true}}"#;
    let consent: LocalConsent = serde_json::from_str(json_only_mouse).expect("partial json must deserialize");

    let mut h = harness_with_consent(
        at("10:00:00"),
        Some(saved(vec![project("p1", 5)], Some("p1"))),
        LocalConsent::Absent,
    );
    h.command(at("10:00:30"), Command::UpdateConsent { consent });
    assert_eq!(h.status(), TrackerStatus::Tracking);

    // Mouse was granted
    assert!(h.machine.last_mouse.load(std::sync::atomic::Ordering::SeqCst), "mouse granted");
    // All omitted capabilities are false:
    assert!(!h.machine.last_keyboard.load(std::sync::atomic::Ordering::SeqCst), "keyboard omitted -> denied");
    assert!(!h.machine.is_usage_running(), "active window omitted -> denied");
    assert_eq!(h.machine.usage_starts(), 0);
}

#[test]
fn consent_16_narrowing_stops_withdrawn_sensors_before_blocked_save_completes() {
    let broad = ConsentedPermissions {
        screenshot: true,
        webcam: true,
        keyboard: true,
        mouse: true,
        active_window: true,
        running_programs: true,
        blur_screenshots: true,
    };
    let mut h = harness_with_consent(
        at("10:00:00"),
        Some(saved(vec![project_allowing_all("p1", 5)], Some("p1"))),
        LocalConsent::Granted(broad),
    );

    // Initial state: broad tracking active
    assert_eq!(h.status(), TrackerStatus::Tracking);
    assert!(h.machine.is_tracking_active());
    assert!(h.machine.last_keyboard.load(std::sync::atomic::Ordering::SeqCst));
    assert!(h.machine.last_mouse.load(std::sync::atomic::Ordering::SeqCst));
    assert!(h.machine.is_usage_running());
    assert!(h.machine.is_screenshot_allowed());
    assert!(h.machine.is_webcam_allowed());

    // Setup injected blocking hook: inspect machine state WHILE save is in flight!
    let machine = h.machine.clone();
    let hook_called = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let hook_called_clone = hook_called.clone();
    h.consent_store.set_save_hook(move |_target| {
        hook_called_clone.store(true, std::sync::atomic::Ordering::SeqCst);
        // Invariant: Withdrawn permissions (keyboard, window, screen, webcam) MUST be stopped BEFORE save blocks!
        assert!(!machine.last_keyboard.load(std::sync::atomic::Ordering::SeqCst), "keyboard MUST stop before save blocks");
        assert!(!machine.is_usage_running(), "usage sampler MUST stop before save blocks");
        assert!(!machine.is_screenshot_allowed(), "screenshot MUST be disallowed before save blocks");
        assert!(!machine.is_webcam_allowed(), "webcam MUST be disallowed before save blocks");

        // Retained permission (mouse) remains active without drop
        assert!(machine.last_mouse.load(std::sync::atomic::Ordering::SeqCst), "mouse remains active");
        assert!(machine.is_input_running(), "input running for mouse");
        assert_eq!(machine.captures(), 0, "no screen captures occurred");
        Ok(())
    });

    let narrower = ConsentedPermissions {
        screenshot: false,
        webcam: false,
        keyboard: false,
        mouse: true,
        active_window: false,
        running_programs: false,
        blur_screenshots: true,
    };
    h.command(at("10:01:00"), Command::UpdateConsent { consent: LocalConsent::Granted(narrower) });

    // Ensure hook executed synchronously during save
    assert!(hook_called.load(std::sync::atomic::Ordering::SeqCst), "hook must have executed during save");
    assert_eq!(h.consent_store.save_count(), 1);

    // After save completed: state matches narrower grant
    assert_eq!(h.status(), TrackerStatus::Tracking);
    assert!(h.machine.last_mouse.load(std::sync::atomic::Ordering::SeqCst));
    assert!(!h.machine.last_keyboard.load(std::sync::atomic::Ordering::SeqCst));
    assert!(!h.machine.is_usage_running());
    assert!(!h.machine.is_screenshot_allowed());
    assert!(!h.machine.is_webcam_allowed());
}

#[test]
fn consent_17_mixed_narrowing_and_expansion_request_under_blocked_store() {
    // Start with mouse + keyboard, but NO webcam
    let initial = ConsentedPermissions {
        screenshot: false,
        webcam: false,
        keyboard: true,
        mouse: true,
        active_window: false,
        running_programs: false,
        blur_screenshots: true,
    };
    let mut h = harness_with_consent(
        at("10:00:00"),
        Some(saved(vec![project_allowing_all("p1", 5)], Some("p1"))),
        LocalConsent::Granted(initial),
    );
    assert_eq!(h.status(), TrackerStatus::Tracking);
    assert!(h.machine.last_keyboard.load(std::sync::atomic::Ordering::SeqCst));
    assert!(!h.machine.is_webcam_allowed());

    // Request mixed change: withdraw keyboard, expand webcam
    let machine = h.machine.clone();
    let hook_called = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let hook_called_clone = hook_called.clone();
    h.consent_store.set_save_hook(move |_target| {
        hook_called_clone.store(true, std::sync::atomic::Ordering::SeqCst);
        // Withdrawn keyboard MUST be stopped
        assert!(!machine.last_keyboard.load(std::sync::atomic::Ordering::SeqCst), "withdrawn keyboard stopped before save");
        // Newly requested webcam MUST NOT be active before storage confirms
        assert!(!machine.is_webcam_allowed(), "unpersisted expansion must NOT be active while save is blocked");
        // Retained mouse is active
        assert!(machine.last_mouse.load(std::sync::atomic::Ordering::SeqCst), "retained mouse active");
        Ok(())
    });

    let mixed = ConsentedPermissions {
        screenshot: false,
        webcam: true,
        keyboard: false,
        mouse: true,
        active_window: false,
        running_programs: false,
        blur_screenshots: true,
    };
    h.command(at("10:01:00"), Command::UpdateConsent { consent: LocalConsent::Granted(mixed) });

    assert!(hook_called.load(std::sync::atomic::Ordering::SeqCst));
    assert_eq!(h.status(), TrackerStatus::Tracking);
    // After persistence succeeded: webcam is now safely active!
    assert!(h.machine.is_webcam_allowed(), "webcam activated after successful persistence");
    assert!(!h.machine.last_keyboard.load(std::sync::atomic::Ordering::SeqCst));
    assert!(h.machine.last_mouse.load(std::sync::atomic::Ordering::SeqCst));
}

#[test]
fn consent_18_blocked_save_failure_fails_closed_and_prevents_resume() {
    let broad = ConsentedPermissions {
        screenshot: true,
        webcam: true,
        keyboard: true,
        mouse: true,
        active_window: true,
        running_programs: true,
        blur_screenshots: true,
    };
    let mut h = harness_with_consent(
        at("10:00:00"),
        Some(saved(vec![project_allowing_all("p1", 5)], Some("p1"))),
        LocalConsent::Granted(broad),
    );
    assert_eq!(h.status(), TrackerStatus::Tracking);

    // Save hook returns error to simulate write failure
    let machine = h.machine.clone();
    h.consent_store.set_save_hook(move |_target| {
        // Assert withdrawn permissions stopped before the write error
        assert!(!machine.last_keyboard.load(std::sync::atomic::Ordering::SeqCst));
        anyhow::bail!("simulated disk I/O timeout during consent save")
    });

    let narrower = ConsentedPermissions {
        screenshot: false,
        webcam: false,
        keyboard: false,
        mouse: true,
        active_window: false,
        running_programs: false,
        blur_screenshots: true,
    };
    h.command(at("10:01:00"), Command::UpdateConsent { consent: LocalConsent::Granted(narrower) });

    // Invariant: Save failure must establish fail-closed state!
    assert_eq!(h.status(), TrackerStatus::ConsentRequired);
    assert!(!h.machine.is_tracking_active(), "all sensors stopped on write failure");
    assert!(!h.machine.is_input_running());
    assert!(!h.machine.is_usage_running());
    assert!(!h.machine.is_screenshot_allowed());
    assert!(!h.machine.is_webcam_allowed());

    let snapshot = h.state.snapshot();
    assert!(snapshot.consent_unsynced, "must report unsynced");
    assert_eq!(snapshot.consent, LocalConsent::Revoked, "fail-closed state does not falsely claim success");
    let err = snapshot.consent_error.as_ref().unwrap();
    assert!(err.contains("non-durable across app restart"), "honestly reports non-durable restart limit");

    // Resume attempt must be strictly blocked
    h.command(at("10:02:00"), Command::Resume { at: at("10:02:00") });
    assert_eq!(h.status(), TrackerStatus::ConsentRequired);
    assert!(!h.machine.is_tracking_active(), "resume blocked while unsynced");
}

#[test]
fn consent_19_multithreaded_barrier_proves_withdrawn_sensors_stopped_while_store_blocked() {
    let broad = ConsentedPermissions {
        screenshot: true,
        webcam: true,
        keyboard: true,
        mouse: true,
        active_window: true,
        running_programs: true,
        blur_screenshots: true,
    };
    let mut h = harness_with_consent(
        at("10:00:00"),
        Some(saved(vec![project_allowing_all("p1", 5)], Some("p1"))),
        LocalConsent::Granted(broad),
    );
    assert_eq!(h.status(), TrackerStatus::Tracking);

    let enter_barrier = Arc::new(std::sync::Barrier::new(2));
    let exit_barrier = Arc::new(std::sync::Barrier::new(2));

    let enter_clone = enter_barrier.clone();
    let exit_clone = exit_barrier.clone();
    h.consent_store.set_save_hook(move |_target| {
        enter_clone.wait(); // Signals test thread that save is currently blocked
        exit_clone.wait();  // Waits for test thread to complete its assertions
        Ok(())
    });

    let machine = h.machine.clone();
    let narrower = ConsentedPermissions {
        screenshot: false,
        webcam: false,
        keyboard: false,
        mouse: true,
        active_window: false,
        running_programs: false,
        blur_screenshots: true,
    };

    // Run command on worker in a separate thread
    let worker_thread = std::thread::spawn(move || {
        h.command(at("10:01:00"), Command::UpdateConsent { consent: LocalConsent::Granted(narrower) });
        h
    });

    // Test coordinator thread: wait until store is BLOCKED in save()
    enter_barrier.wait();

    // At this EXACT moment, worker thread is literally blocked inside consent_store.save!
    // Assert withdrawn sensors are ALREADY stopped:
    assert!(!machine.last_keyboard.load(std::sync::atomic::Ordering::SeqCst), "keyboard stopped while store blocked");
    assert!(!machine.is_usage_running(), "usage stopped while store blocked");
    assert!(!machine.is_screenshot_allowed(), "screenshot disallowed while store blocked");
    assert!(!machine.is_webcam_allowed(), "webcam disallowed while store blocked");
    assert!(machine.last_mouse.load(std::sync::atomic::Ordering::SeqCst), "retained mouse still running");
    assert_eq!(machine.captures(), 0, "no screen captures scheduled/taken");

    // Release worker thread to finish save
    exit_barrier.wait();

    let h = worker_thread.join().expect("worker thread must finish cleanly");
    assert_eq!(h.status(), TrackerStatus::Tracking);
    assert_eq!(h.consent_store.save_count(), 1);
}

#[test]
fn consent_20_withdrawn_sensors_stopped_before_reconcile_record_blocks() {
    let broad = ConsentedPermissions {
        screenshot: true,
        webcam: true,
        keyboard: true,
        mouse: true,
        active_window: true,
        running_programs: true,
        blur_screenshots: true,
    };
    let mut h = harness_with_consent(
        at("10:00:00"),
        Some(saved(vec![project_allowing_all("p1", 5)], Some("p1"))),
        LocalConsent::Granted(broad),
    );
    assert_eq!(h.status(), TrackerStatus::Tracking);
    assert!(h.machine.is_tracking_active());
    assert!(h.machine.is_input_running());
    assert!(h.machine.is_usage_running());

    // Deterministic channel-based synchronization asserting stopped drivers
    // WHILE finalization/record is actively executing in machine.snapshot!
    let (enter_tx, enter_rx) = std::sync::mpsc::channel::<()>();
    let (exit_tx, exit_rx) = std::sync::mpsc::channel::<()>();
    let exit_rx = Mutex::new(exit_rx);

    h.machine.set_on_snapshot(move || {
        let _ = enter_tx.send(());
        let _ = exit_rx.lock().unwrap().recv();
    });

    let machine = h.machine.clone();

    let worker_thread = std::thread::spawn(move || {
        h.command(at("10:01:00"), Command::UpdateConsent { consent: LocalConsent::Revoked });
        h
    });

    // Test coordinator: wait until worker reaches machine.snapshot during finalization/record
    enter_rx.recv().expect("worker thread must enter snapshot during finalization");

    // AT THIS EXACT MOMENT:
    // The worker is actively executing record() -> snapshot().
    // consent_store.save has NOT even been called yet.
    // Assert native sensor drivers were ALREADY stopped ahead of reconcile/record:
    assert!(!machine.is_tracking_active(), "tracking stopped before record/snapshot");
    assert!(!machine.is_input_running(), "input driver stopped before record/snapshot");
    assert!(!machine.is_usage_running(), "usage driver stopped before record/snapshot");
    assert_eq!(machine.captures(), 0, "no screen captures taken while record is blocked");
    assert_eq!(machine.webcam_captures(), 0, "no webcam captures taken while record is blocked");
    assert_eq!(machine.window_queries(), 0, "no window queries taken while record is blocked");
    assert_eq!(machine.process_queries(), 0, "no process queries taken while record is blocked");

    // Release worker thread cleanly
    drop(exit_tx);

    let h = worker_thread.join().expect("worker thread must finish cleanly");
    assert_eq!(h.status(), TrackerStatus::ConsentRequired);
    assert_eq!(h.consent_store.save_count(), 1);
    assert_eq!(h.consent_store.load(), LocalConsent::Revoked);
    assert_eq!(h.machine.snapshot_calls(), 1, "exactly one snapshot during finalization");

    assert_eq!(h.machine.captures(), 0, "final screen captures remain zero");
    assert_eq!(h.machine.webcam_captures(), 0, "final webcam captures remain zero");
    assert_eq!(h.machine.window_queries(), 0, "final window queries remain zero");
    assert_eq!(h.machine.process_queries(), 0, "final process queries remain zero");
}

#[test]
fn consent_21_held_blocks_do_not_acquire_withdrawn_screen_or_webcam_at_finalization() {
    let broad = ConsentedPermissions {
        screenshot: true,
        webcam: true,
        keyboard: true,
        mouse: true,
        active_window: true,
        running_programs: true,
        blur_screenshots: true,
    };
    let mut h = harness_with_consent(
        at("10:00:00"),
        Some(saved(vec![project_allowing_all("p1", 30)], Some("p1"))),
        LocalConsent::Granted(broad),
    );
    assert_eq!(h.status(), TrackerStatus::Tracking);

    // Record user input at 10:03:00 during the active block
    h.machine.input_at(at("10:03:00"));

    // Tick past block boundary (10:10:01) with last input at 10:03:00.
    // Under 30-min idle threshold, the 10:00-10:10 block is held in stretch.held
    h.tick(at("10:10:01"));
    assert_eq!(h.queued().len(), 0, "first block is held awaiting user return");
    let captures_before_revoke = h.machine.captures();

    // At 10:12:00, user explicitly revokes consent while the block is held!
    h.command(at("10:12:00"), Command::UpdateConsent { consent: LocalConsent::Revoked });

    assert_eq!(h.status(), TrackerStatus::ConsentRequired);
    assert_eq!(h.machine.snapshot_calls(), 2, "snapshots taken for both held and open block");

    // Assert actual acquisition call counts on fake machine:
    // Withdrawn sensors must NEVER perform fresh captures or queries at finalization
    assert_eq!(h.machine.captures(), captures_before_revoke, "no new screen captures taken at finalization");
    assert_eq!(h.machine.webcam_captures(), 0, "no webcam photos taken for held or open blocks");
    assert_eq!(h.machine.window_queries(), 0, "no foreground window sampling after withdrawal");
    assert_eq!(h.machine.process_queries(), 0, "no running programs listing after withdrawal");

    // Verify retained historical work:
    // Both the held block (10:00-10:10) and final sliver (10:10-10:12) are queued,
    // retaining already-authorized historical time and input counts without fresh capture data.
    let queued = h.queued();
    assert_eq!(queued.len(), 2, "both held block and closing sliver were finalized");

    let held_block = &queued[0];
    assert_eq!(held_block.started_at, at("10:00:00"));
    assert_eq!(held_block.ended_at, at("10:10:00"));
    assert_eq!(held_block.keyboard_hits, Some(1), "authorized key hits in held block preserved");
    assert_eq!(held_block.mouse_clicks, Some(1), "authorized clicks in held block preserved");
    assert_eq!(held_block.active_windows.len(), 1, "authorized usage slice preserved");
    assert_eq!(held_block.screenshot_png_base64, None, "no screenshot acquired on held block");
    assert_eq!(held_block.webcam_jpg_base64, None, "no webcam acquired on held block");
    assert!(held_block.running_programs.is_empty(), "no process sampling on held block");

    let open_block = &queued[1];
    assert_eq!(open_block.started_at, at("10:10:00"));
    assert_eq!(open_block.ended_at, at("10:12:00"));
    assert_eq!(open_block.screenshot_png_base64, None, "no screenshot acquired on open block");
    assert_eq!(open_block.webcam_jpg_base64, None, "no webcam acquired on open block");
    assert!(open_block.running_programs.is_empty(), "no process sampling on open block");
}

#[test]
fn consent_22_mixed_narrowing_stops_withdrawn_sensors_before_reconcile_record_blocks() {
    let broad = ConsentedPermissions {
        screenshot: true,
        webcam: true,
        keyboard: true,
        mouse: true,
        active_window: true,
        running_programs: true,
        blur_screenshots: true,
    };
    let mut h = harness_with_consent(
        at("10:00:00"),
        Some(saved(vec![project_allowing_all("p1", 5)], Some("p1"))),
        LocalConsent::Granted(broad),
    );
    assert_eq!(h.status(), TrackerStatus::Tracking);

    // Record user input at 10:02:00
    h.machine.input_at(at("10:02:00"));

    let narrower = ConsentedPermissions {
        screenshot: false,
        webcam: false,
        keyboard: false,
        mouse: true,
        active_window: false,
        running_programs: false,
        blur_screenshots: true,
    };

    // User narrows permissions while tracking:
    h.command(at("10:05:00"), Command::UpdateConsent { consent: LocalConsent::Granted(narrower) });
    assert_eq!(h.status(), TrackerStatus::Tracking);

    // Native sensors for withdrawn capabilities are stopped immediately:
    assert!(!h.machine.last_keyboard.load(std::sync::atomic::Ordering::SeqCst), "keyboard stopped");
    assert!(!h.machine.is_usage_running(), "usage sampler stopped");
    assert!(!h.machine.is_screenshot_allowed(), "screenshot disallowed");
    assert!(!h.machine.is_webcam_allowed(), "webcam disallowed");
    assert!(h.machine.last_mouse.load(std::sync::atomic::Ordering::SeqCst), "retained mouse still running");

    // Now tick past 10:10 to finalize the block:
    h.tick(at("10:10:01"));

    // Assert zero acquisitions for withdrawn capabilities:
    assert_eq!(h.machine.captures(), 0, "no screen captures taken at block boundary");
    assert_eq!(h.machine.webcam_captures(), 0, "no webcam photos taken at block boundary");
    assert_eq!(h.machine.window_queries(), 0, "no foreground window sampling at block boundary");
    assert_eq!(h.machine.process_queries(), 0, "no process queries at block boundary");

    let queued = h.queued();
    assert_eq!(queued.len(), 1, "interval block recorded");
    assert_eq!(queued[0].keyboard_hits, None, "withdrawn keyboard: not measured (null), never a made-up 0");
    assert_eq!(queued[0].mouse_clicks, Some(1), "retained mouse clicks preserved");
    assert_eq!(queued[0].screenshot_png_base64, None, "no screenshot attached");
    assert_eq!(queued[0].webcam_jpg_base64, None, "no webcam attached");
    assert!(queued[0].running_programs.is_empty(), "no running programs attached");
}
