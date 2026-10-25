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

    // The user is really back: the hooks see it at once. The jiggler's 50
    // seconds before it are not work.
    h.machine.input_at(at("10:11:00"));
    h.tick(at("10:11:01"));
    assert_eq!(h.status(), TrackerStatus::Tracking);
    assert_eq!(h.tracking_since(), Some(at("10:11:00")), "from the user's own input, not the jiggler's");
    let prompt = h.state.snapshot().away_prompt.expect("asked about the time away");
    assert_eq!((prompt.from, prompt.to), (at("10:02:00"), at("10:11:00")));
}

#[test]
fn an_hour_of_mouse_jiggling_before_the_user_returns_is_never_work() {
    let mut h = tracking();
    h.machine.input_at(at("10:02:00"));
    h.tick(at("10:07:00"));
    assert_eq!(h.status(), TrackerStatus::Away);

    // A jiggler from 10:05 until noon, a tick every 2 seconds.
    let mut when = at("10:05:00");
    while when < at("12:00:00") {
        h.machine.injected_at(when);
        when += TimeDelta::seconds(2);
        h.tick(when);
        assert_eq!(h.status(), TrackerStatus::Away, "at {when}");
    }
    assert_eq!(h.spans().len(), 1, "nothing recorded while away");

    h.come_back(at("12:00:30"));
    assert_eq!(h.status(), TrackerStatus::Tracking);
    assert_eq!(h.tracking_since(), Some(at("12:00:30")), "the user's own input, not the jiggler's 10:05");
    let prompt = h.state.snapshot().away_prompt.expect("asked about the time away");
    assert_eq!((prompt.from, prompt.to), (at("10:02:00"), at("12:00:30")), "the jiggling is time away, kept only if they say so");

    // Ten minutes on, the first block is the user's own.
    for minute in 1..=12 {
        let when = at("12:00:30") + TimeDelta::minutes(minute);
        h.machine.input_at(when - TimeDelta::seconds(5));
        h.tick(when);
    }
    let spans = h.spans();
    assert_eq!(spans.len(), 2, "{spans:?}");
    assert_eq!(spans[1].0, at("12:00:30"), "nothing before the real return");
}

#[test]
fn a_return_the_hooks_confirm_at_once_starts_where_the_idle_clock_saw_it() {
    let mut h = tracking();
    h.machine.input_at(at("10:02:00"));
    h.tick(at("10:07:00"));
    assert_eq!(h.status(), TrackerStatus::Away);
    // Windows' idle clock sees the user at 10:20:00; the hooks, attached to
    // check, see them still at it two seconds later.
    h.come_back(at("10:20:00"));
    assert_eq!(h.status(), TrackerStatus::Tracking);
    assert_eq!(h.tracking_since(), Some(at("10:20:00")));
}

#[test]
fn a_return_is_noticed_when_the_input_hooks_cannot_attach() {
    for silently in [false, true] {
        let mut h = tracking();
        h.machine.input_at(at("10:02:00"));
        h.tick(at("10:07:00"));
        assert_eq!(h.status(), TrackerStatus::Away);
        // Security software blocks the hooks from now on.
        h.machine.block_hooks(silently);

        h.machine.input_at(at("10:20:00"));
        h.tick(at("10:20:01"));
        assert_eq!(h.status(), TrackerStatus::Away, "silently {silently}: the hooks are asked for first");
        assert!(!h.machine.is_input_running(), "silently {silently}: and do not attach");
        // The user keeps working; the hooks never come. Windows' idle clock
        // decides, as it does without input permission.
        let mut when = at("10:20:01");
        while h.status() == TrackerStatus::Away && when < at("10:21:00") {
            when += TimeDelta::seconds(2);
            h.machine.input_at(when - TimeDelta::seconds(1));
            h.tick(when);
        }
        assert_eq!(h.status(), TrackerStatus::Tracking, "silently {silently}: not stuck away");
        assert!(when <= at("10:20:01") + HOOKS_ATTACH_WAIT + TimeDelta::seconds(2), "silently {silently}: within {when}");
        assert_eq!(h.tracking_since(), Some(at("10:20:00")), "silently {silently}: from when the idle clock first saw them");
        if !silently {
            assert_eq!(when, at("10:20:03"), "a reported failure: on the next tick");
        }
    }
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

#[test]
fn jiggling_the_hooks_rejected_is_not_work_once_input_counts_are_turned_off() {
    let mut h = tracking();
    h.machine.input_at(at("10:02:00"));
    h.tick(at("10:07:00"));
    assert_eq!(h.status(), TrackerStatus::Away);
    let mut when = at("10:10:00");
    while when < at("11:00:00") {
        h.machine.injected_at(when);
        when += TimeDelta::seconds(2);
        h.tick(when);
    }
    assert_eq!(h.status(), TrackerStatus::Away);

    // The admin stops recording keyboard and mouse: nothing can tell the
    // jiggler apart any more, and the idle clock decides — from now, not
    // from the jiggling the hooks saw through.
    let no_input: Project = serde_json::from_value(serde_json::json!({
        "id": "p1", "title": "Project p1", "intervalMinutes": 10,
        "allowScreenshot": true, "allowWebcam": false, "allowKeyboard": false, "allowMouse": false,
        "allowActiveWindow": true, "allowRunningPrograms": true, "idleThresholdMinutes": 5
    }))
    .unwrap();
    h.worker.on_control(ControlEvent::Projects(vec![no_input]));
    h.worker.publish();
    h.machine.injected_at(when);
    h.tick(when + TimeDelta::seconds(1));
    assert_eq!(h.status(), TrackerStatus::Tracking);
    assert_eq!(h.tracking_since(), Some(when), "not from 10:10, when the jiggling began");
}
