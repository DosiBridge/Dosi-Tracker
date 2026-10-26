//! The sensors follow tracking: on while tracking, off otherwise.

use super::*;

#[test]
fn tracking_active_follows_pause_and_resume_lifecycle() {
    let h_unsigned = harness_at(at("10:00:00"), None, false, None);
    assert!(!h_unsigned.machine.is_tracking_active(), "sensors must start inactive before sign-in");
    assert!(!h_unsigned.machine.is_input_running());
    assert!(!h_unsigned.machine.is_usage_running());

    let mut h = tracking();
    assert!(h.machine.is_tracking_active(), "sensors must activate when tracking project session");
    assert!(h.machine.is_input_running());
    assert!(h.machine.is_usage_running());
    assert_eq!(h.machine.input_starts(), 1);
    assert_eq!(h.machine.usage_starts(), 1);

    h.command(at("10:05:00"), Command::Pause { resume_after: None, at: at("10:05:00") });
    assert!(!h.machine.is_tracking_active(), "sensors must deactivate immediately when paused");
    assert!(!h.machine.is_input_running());
    assert!(!h.machine.is_usage_running());
    assert_eq!(h.machine.input_stops(), 1);
    assert_eq!(h.machine.usage_stops(), 1);

    h.command(at("10:10:00"), Command::Resume { at: at("10:10:00") });
    assert!(h.machine.is_tracking_active(), "sensors must reactivate upon resume");
    assert!(h.machine.is_input_running());
    assert!(h.machine.is_usage_running());
    assert_eq!(h.machine.input_starts(), 2);
    assert_eq!(h.machine.usage_starts(), 2);

    h.command(at("10:15:00"), Command::SignOut { at: at("10:15:00") });
    assert!(!h.machine.is_tracking_active(), "sensors must deactivate upon sign-out");
    assert!(!h.machine.is_input_running());
    assert!(!h.machine.is_usage_running());
    assert_eq!(h.machine.input_stops(), 2);
    assert_eq!(h.machine.usage_stops(), 2);
}

#[test]
fn sensor_lifecycle_stops_on_lock_and_resumes_on_unlock() {
    let mut h = tracking();
    assert!(h.machine.is_tracking_active());
    assert!(h.machine.is_input_running());
    assert!(h.machine.is_usage_running());

    // Machine locks
    h.worker.on_system(crate::session_events::SystemNotice {
        event: crate::session_events::SystemEvent::Locked,
        at: at("10:05:00"),
        done: None,
    });
    h.worker.publish();
    assert_eq!(h.status(), TrackerStatus::Locked);
    assert!(!h.machine.is_tracking_active(), "sensors must stop when machine is locked");
    assert!(!h.machine.is_input_running(), "input hooks must detach on lock");
    assert!(!h.machine.is_usage_running(), "window sampler must stop on lock");

    // Machine unlocks
    h.worker.on_system(crate::session_events::SystemNotice {
        event: crate::session_events::SystemEvent::Unlocked,
        at: at("10:06:00"),
        done: None,
    });
    h.worker.publish();
    assert_eq!(h.status(), TrackerStatus::Tracking);
    assert!(h.machine.is_tracking_active(), "sensors must resume when machine unlocks");
    assert!(h.machine.is_input_running());
    assert!(h.machine.is_usage_running());
}

#[test]
fn sensor_lifecycle_stops_on_away_and_resumes_on_return() {
    let mut h = tracking();
    assert!(h.machine.is_tracking_active());

    // Advance clock past idle threshold (default 10 min)
    h.machine.set(at("10:15:00"));
    h.worker.on_tick();
    h.worker.publish();
    assert_eq!(h.status(), TrackerStatus::Away);
    assert!(!h.machine.is_tracking_active(), "sensors must stop while user is away");
    assert!(!h.machine.is_input_running(), "input hooks must detach when away");
    assert!(!h.machine.is_usage_running(), "window sampler must stop when away");

    // Input at 10:16:00, seen by Windows' idle clock: only the input hooks
    // attach, to confirm it is the user's own.
    h.machine.input_at(at("10:16:00"));
    h.tick(at("10:16:01"));
    assert_eq!(h.status(), TrackerStatus::Away);
    assert!(h.machine.is_input_running(), "the hooks confirm the return");
    assert!(!h.machine.is_usage_running() && !h.machine.is_screenshot_allowed() && !h.machine.is_webcam_allowed(), "nothing else runs");

    // The user is still typing: back, from where the idle clock saw it.
    h.machine.input_at(at("10:16:02"));
    h.tick(at("10:16:03"));
    assert_eq!(h.status(), TrackerStatus::Tracking);
    assert_eq!(h.tracking_since(), Some(at("10:16:00")));
    assert!(h.machine.is_tracking_active(), "sensors reactivate upon return");
    assert!(h.machine.is_input_running());
    assert!(h.machine.is_usage_running());
}

#[test]
fn sensor_lifecycle_stops_on_workspace_lost() {
    let mut h = tracking();
    assert!(h.machine.is_tracking_active());

    h.worker.on_control(ControlEvent::WorkspaceLost("w1".into(), crate::api::WorkspaceRefusal::NotMember));
    h.worker.publish();
    assert_ne!(h.status(), TrackerStatus::Tracking);
    assert!(!h.machine.is_tracking_active(), "sensors must stop when workspace access is lost");
    assert!(!h.machine.is_input_running());
    assert!(!h.machine.is_usage_running());
}

#[tokio::test]
async fn sensor_lifecycle_stops_on_quit_before_upload_grace() {
    let mut h = tracking();
    assert!(h.machine.is_tracking_active());
    assert!(h.machine.is_input_running());
    assert!(h.machine.is_usage_running());

    let machine = h.machine.clone();
    let quit_task = tokio::spawn(async move {
        h.worker.quit(at("10:05:00")).await;
        h
    });

    // While quit is awaiting farewell and upload grace, assert that sensors were ALREADY halted!
    tokio::time::sleep(Duration::from_millis(15)).await;
    assert!(!machine.is_tracking_active(), "sensors must stop BEFORE waiting for farewell/upload grace");
    assert!(!machine.is_input_running(), "input hooks must detach BEFORE waiting for farewell/upload grace");
    assert!(!machine.is_usage_running(), "usage sampler must stop BEFORE waiting for farewell/upload grace");
    assert_eq!(machine.input_stops(), 1);
    assert_eq!(machine.usage_stops(), 1);

    let h = quit_task.await.unwrap();
    assert!(!h.machine.is_tracking_active());
}

#[tokio::test]
async fn quitting_keeps_what_the_last_block_captured_and_takes_no_new_photo() {
    let mut h = harness_with_consent(
        at("10:00:00"),
        Some(saved(vec![project_allowing_all("p1", 5)], None)),
        LocalConsent::Granted(CapturePermissions { webcam: true, ..CapturePermissions::default() }.into()),
    );
    // The block's screenshot moment (50%: 10:05) passes.
    h.machine.input_at(at("10:05:00"));
    h.tick(at("10:05:01"));
    assert_eq!(h.machine.captures(), 1);
    h.machine.input_at(at("10:06:00"));
    h.machine.set(at("10:06:30"));
    h.worker.quit(at("10:06:30")).await;

    let queued = h.queued();
    assert_eq!(queued.len(), 1);
    assert_eq!(queued[0].screenshot_captured_at, Some(at("10:05:01")), "taken with consent: kept");
    assert!(!queued[0].running_programs.is_empty(), "the running programs too");
    assert!(queued[0].webcam_jpg_base64.is_none(), "no webcam photo on the way out");
    assert_eq!(h.machine.webcam_captures(), 0);
    assert!(!h.machine.is_tracking_active(), "the sensors stop once it is saved");
}

#[test]
fn worker_fails_closed_when_consent_is_absent() {
    let machine = FakeMachine::new(at("10:00:00"));
    let state = StateHandle::new();
    let (control, _, _) = ControlLink::detached();
    let (uploader, _) = UploaderHandle::detached();
    let store = Storage::open(":memory:", None).unwrap();
    let cfg = AppConfig::default(); // consent: LocalConsent::Absent
    let consent_store = Arc::new(MemoryConsentStore::new(LocalConsent::Absent));
    let mut worker = Worker::new(
        state,
        cfg,
        machine.clone(),
        Launch { autostart: false },
        Arc::new(|| {}),
        ServerClock::default(),
        store,
        control,
        uploader,
        None,
        consent_store,
    );
    worker.start(Some(saved(vec![project("p1", 5)], None)), None);
    worker.publish();
    assert!(!machine.is_tracking_active(), "absent consent must fail closed");
    assert!(!machine.is_input_running(), "input hooks must not start without consent");
    assert!(!machine.is_usage_running(), "sampler must not start without consent");
    assert_eq!(machine.input_starts(), 0);
    assert_eq!(machine.usage_starts(), 0);
}

#[test]
fn sensor_lifecycle_per_sensor_denied_policy() {
    let h = tracking();
    // Update session with keyboard allowed, mouse denied, active window denied
    let perms = CapturePermissions {
        keyboard: true,
        mouse: false,
        active_window: false,
        screenshot: false,
        webcam: false,
        running_programs: false,
        blur_screenshots: false,
    };
    h.machine.apply_sensor_policy(true, Some(&perms));
    assert!(h.machine.is_input_running(), "input hooks running for keyboard");
    assert!(!h.machine.is_usage_running(), "usage sampler must remain stopped when active_window is denied");
}

#[test]
fn sensor_lifecycle_handles_repeated_transitions_accurately() {
    let mut h = tracking();
    assert!(h.machine.is_tracking_active());
    assert_eq!(h.machine.input_starts(), 1);
    assert_eq!(h.machine.usage_starts(), 1);

    for i in 1..=5 {
        let t_pause = at(&format!("10:{:02}:00", i * 2));
        let t_resume = at(&format!("10:{:02}:00", i * 2 + 1));

        h.command(t_pause, Command::Pause { resume_after: None, at: t_pause });
        assert!(!h.machine.is_tracking_active());
        assert!(!h.machine.is_input_running());
        assert!(!h.machine.is_usage_running());
        assert_eq!(h.machine.input_stops(), i);
        assert_eq!(h.machine.usage_stops(), i);

        h.command(t_resume, Command::Resume { at: t_resume });
        assert!(h.machine.is_tracking_active());
        assert!(h.machine.is_input_running());
        assert!(h.machine.is_usage_running());
        assert_eq!(h.machine.input_starts(), i + 1);
        assert_eq!(h.machine.usage_starts(), i + 1);
    }
}
