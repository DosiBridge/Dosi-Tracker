//! Idle detection without the input hooks (WIN-P0-01, WIN-P0-02).

use super::*;

// ---- Idle detection without the input hooks (WIN-P0-01, WIN-P0-02) --------------------

#[test]
fn the_real_tracker_reads_windows_idle_clock_while_the_hooks_are_off() {
    let (tracker, driver) = Tracker::new_fake();
    let now = Utc::now();
    // Hooks never attached (away, paused, or input not allowed): before the
    // fix, "healthy" hooks with no data answered None forever.
    driver.set_system_idle_seconds(Some(30));
    assert!(!Machine::input_hooks_running(&tracker));
    let last = Machine::last_input(&tracker, now).expect("Windows' idle clock answers");
    assert_eq!((now - last).num_seconds(), 30);
    assert_eq!(Machine::first_input_after(&tracker, now - TimeDelta::minutes(10), now), Some(last), "a return is seen");
    assert_eq!(Machine::first_input_after(&tracker, now - TimeDelta::seconds(5), now), None, "nothing newer");
    driver.set_system_idle_seconds(None);
    assert_eq!(Machine::last_input(&tracker, now), None, "Windows says nothing: nothing");

    // Hooks attached for both kinds: their view (the user's own input) wins.
    driver.set_system_idle_seconds(Some(0));
    Machine::apply_sensor_policy(&tracker, true, Some(&CapturePermissions::default()));
    assert!(Machine::input_hooks_running(&tracker));
    assert_eq!(Machine::last_input(&tracker, now), None, "no input seen by the hooks yet, whatever Windows says");
    assert_eq!(Machine::first_input_after(&tracker, now - TimeDelta::minutes(1), now), None);

    // Keys only: the hooks would miss someone scrolling, so the idle clock
    // decides.
    let keys_only = CapturePermissions { mouse: false, ..CapturePermissions::default() };
    Machine::apply_sensor_policy(&tracker, true, Some(&keys_only));
    assert!(Machine::input_hooks_running(&tracker));
    assert_eq!(Machine::last_input(&tracker, now).map(|t| (now - t).num_seconds()), Some(0));

    // Detached again: the idle clock.
    Machine::apply_sensor_policy(&tracker, false, None);
    driver.set_system_idle_seconds(Some(7));
    assert_eq!(Machine::last_input(&tracker, now).map(|t| (now - t).num_seconds()), Some(7));
}

#[test]
fn a_mouse_jiggler_never_ends_the_time_away() {
    let mut h = tracking();
    h.machine.input_at(at("10:02:00"));
    h.tick(at("10:07:00"));
    assert_eq!(h.status(), TrackerStatus::Away);
    h.heartbeats();

    // Injected input every few seconds: Windows' idle clock moves, the
    // hooks (attached to check) see none of the user's own.
    for second in (10..60).step_by(5) {
        let when = at("10:10:00") + TimeDelta::seconds(second);
        h.machine.injected_at(when);
        h.tick(when + TimeDelta::seconds(1));
        assert_eq!(h.status(), TrackerStatus::Away, "at {when}");
    }
    assert!(h.machine.is_input_running(), "the hooks are checking");
    assert!(!h.machine.is_usage_running() && !h.machine.is_screenshot_allowed(), "and only they run");
    assert!(h.tracking_since().is_none());
    assert!(!h.heartbeats().contains(&PresenceStatus::Tracking), "presence never claims tracking");
    assert_eq!(h.spans().len(), 1, "nothing recorded while away");

    // The user is really back.
    h.machine.input_at(at("10:11:00"));
    h.tick(at("10:11:01"));
    assert_eq!(h.status(), TrackerStatus::Tracking);
    assert_eq!(h.tracking_since(), Some(at("10:10:10")), "from when the idle clock first moved");
}

#[test]
fn a_break_that_ends_after_typing_during_it_resumes_at_its_end() {
    let mut h = tracking();
    h.command(at("10:05:00"), Command::Pause { resume_after: Some(Duration::from_secs(15 * 60)), at: at("10:05:00") });
    assert!(!h.machine.is_input_running(), "no hooks while on a break");
    // Typing during the break (nothing is counted, but Windows' idle clock
    // knows): someone is at the computer when it ends.
    h.machine.input_at(at("10:19:30"));
    h.tick(at("10:20:01"));
    assert_eq!(h.status(), TrackerStatus::Tracking, "not stuck in Away after a 15-minute break");
    assert_eq!(h.tracking_since(), Some(at("10:20:00")));
}

#[test]
fn input_after_an_announced_sleep_is_seen_without_the_hooks() {
    let mut h = tracking();
    h.system(SystemEvent::Suspending, at("10:05:00"));
    assert!(!h.machine.is_input_running());
    // The resume broadcast never arrived; the user types.
    h.machine.input_at(at("10:30:00"));
    h.tick(at("10:30:01"));
    assert_eq!(h.status(), TrackerStatus::Tracking);
}

/// Consent to everything except input counts.
fn screenshots_and_windows_only() -> LocalConsent {
    LocalConsent::Granted(ConsentedPermissions {
        screenshot: true,
        active_window: true,
        running_programs: true,
        ..ConsentedPermissions::default()
    })
}

#[test]
fn without_input_counts_time_is_still_recorded_and_idle_detection_works() {
    for consent in [screenshots_and_windows_only(), LocalConsent::Granted(ConsentedPermissions::all_disabled())] {
        let mut h = harness_with_consent(at("10:00:00"), Some(saved(vec![project("p1", 5)], None)), consent);
        assert_eq!(h.status(), TrackerStatus::Tracking, "{consent:?}");
        assert!(!h.machine.is_input_running(), "no hooks without input consent");

        // The user keeps working: Windows' idle clock sees it, nothing is counted.
        for minute in 1..=12 {
            let when = at("10:00:00") + TimeDelta::minutes(minute);
            h.machine.input_at(when - TimeDelta::seconds(20));
            h.tick(when);
            assert_eq!(h.status(), TrackerStatus::Tracking, "{consent:?} at {when}");
        }
        let spans = h.spans();
        assert_eq!(spans, [(at("10:00:00"), at("10:10:00"), Some(EndReason::Interval))], "{consent:?}: time is recorded");
        let block = &h.queued()[0];
        assert_eq!((block.keyboard_hits, block.mouse_clicks, block.productivity), (None, None, None), "not measured");

        // Gone for good: away after the threshold, at the last input.
        h.tick(at("10:18:00"));
        assert_eq!(h.status(), TrackerStatus::Away, "{consent:?}");
        assert_eq!(h.state.snapshot().away_since, Some(at("10:11:40")));
        // Back: nothing can tell injected input apart, so the idle clock decides at once.
        h.machine.input_at(at("10:30:00"));
        h.tick(at("10:30:01"));
        assert_eq!(h.status(), TrackerStatus::Tracking, "{consent:?}");
        assert_eq!(h.tracking_since(), Some(at("10:30:00")));
        assert!(!h.machine.is_input_running());
    }
}
