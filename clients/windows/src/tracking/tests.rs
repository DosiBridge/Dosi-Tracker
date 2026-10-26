use std::sync::Mutex;

use super::payload::{clean_description, split_span, MAX_DESCRIPTION_CHARS};
use super::usage::UsageSlice;
use super::*;
use crate::model::WindowInfo;

const T0: i64 = 1_790_000_000; // arbitrary Unix second
const T0_MS: i64 = T0 * 1000;

fn at(second: i64) -> DateTime<Utc> {
    DateTime::from_timestamp(second, 0).unwrap()
}

fn device() -> DeviceInfo {
    DeviceInfo {
        device_id: Some("5b0c1b9e-8f2c-4a43-9a51-0c7f6f2d9b11".into()),
        device_name: Some("DESKTOP-1".into()),
        os_version: Some("Windows 11 Pro 23H2 (10.0.22631)".into()),
        platform: "windows",
        agent_version: "9.9.9",
    }
}

fn zone() -> Zone {
    Zone { time_zone: Some("Asia/Dhaka".into()), utc_offset_minutes: 360 }
}

fn all_allowed() -> CapturePermissions {
    CapturePermissions::default()
}

fn slice(app: &str, from_s: i64, to_s: i64) -> UsageSlice {
    UsageSlice { app: app.into(), title: format!("{app} window"), start_ms: T0_MS + from_s * 1000, end_ms: T0_MS + to_s * 1000 }
}

/// Ten minutes of measured work: typing in minutes 0-5, clicks in 7,
/// nothing in 6, 8 and 9; three injected events.
fn measurements() -> Measurements {
    let key_presses: Vec<i64> = (0..6).map(|m| T0_MS + m * 60_000 + 5_000).collect();
    let clicks = vec![T0_MS + 7 * 60_000 + 1_000];
    let mut active_seconds: Vec<i64> = key_presses.iter().map(|ms| ms / 1000).collect();
    active_seconds.extend(clicks.iter().map(|ms| ms / 1000));
    Measurements {
        key_presses,
        clicks,
        active_seconds,
        injected: 3,
        input_available: true,
        slices: vec![slice("code.exe", 0, 400), slice("chrome.exe", 400, 600)],
        window_now: None,
        running_programs: vec![WindowInfo { app_name: "slack.exe".into(), ..Default::default() }],
        screen: None,
        webcam_jpg_base64: None,
    }
}

fn capture(blurred: bool) -> ScreenCapture {
    ScreenCapture {
        jpeg_base64: "/9j/screen".into(),
        thumb_jpg_base64: Some("/9j/thumb".into()),
        captured_at: at(T0 + 250),
        blurred,
    }
}

fn prov_cap(tracker: &Tracker, blurred: bool) -> ProvenanceScreenCapture {
    let generation = tracker.auth.read().unwrap().generations.screen;
    ProvenanceScreenCapture {
        capture: capture(blurred),
        generation,
    }
}

fn block<'a>(perms: &'a CapturePermissions, end_reason: Option<EndReason>) -> BlockSpec<'a> {
    BlockSpec {
        project_id: "p1",
        perms,
        started_at: at(T0),
        ended_at: at(T0 + 600),
        end_reason,
        description: Some("  Invoice export  "),
    }
}

#[test]
fn a_measured_block_carries_every_contract_field() {
    let perms = all_allowed();
    let activity = build_activity(&block(&perms, Some(EndReason::Interval)), measurements(), &device(), zone());

    // Active in minutes 0-5 and 7: 7 of 10 buckets.
    assert_eq!(activity.productivity, Some(70));
    assert_eq!(activity.idle_seconds, Some(3 * 60));
    assert_eq!(activity.injected_input_events, Some(3));
    assert_eq!((activity.keyboard_hits, activity.mouse_clicks), (Some(6), Some(1)));
    assert_eq!(activity.timeline.len(), 10);
    assert_eq!(activity.timeline[0].active_seconds, Some(1));
    assert_eq!(activity.timeline[6].active_seconds, Some(0));
    assert!(!activity.timeline[6].active);
    assert_eq!(activity.active_windows[0].app_name, "code.exe");
    assert_eq!(activity.running_programs.len(), 1);
    assert_eq!(activity.end_reason, Some(EndReason::Interval));
    assert_eq!(activity.description.as_deref(), Some("Invoice export"));

    // Device context.
    assert_eq!(activity.platform.as_deref(), Some("windows"));
    assert_eq!(activity.agent_version.as_deref(), Some("9.9.9"));
    assert_eq!(activity.device_id.as_deref(), Some("5b0c1b9e-8f2c-4a43-9a51-0c7f6f2d9b11"));
    assert_eq!(activity.device_name.as_deref(), Some("DESKTOP-1"));
    assert_eq!(activity.os_version.as_deref(), Some("Windows 11 Pro 23H2 (10.0.22631)"));
    assert_eq!(activity.time_zone.as_deref(), Some("Asia/Dhaka"));
    assert_eq!(activity.utc_offset_minutes, Some(360));
    // Stamped at send time, not here.
    assert!(activity.sent_at.is_none());
    assert!(uuid::Uuid::parse_str(&activity.client_activity_id).is_ok());
}

#[test]
fn activity_is_not_measured_when_no_input_kind_is_recorded() {
    let perms = CapturePermissions { keyboard: false, mouse: false, ..all_allowed() };
    let mut measured = measurements();
    measured.key_presses.clear();
    measured.clicks.clear();
    measured.active_seconds.clear();
    let activity = build_activity(&block(&perms, None), measured, &device(), zone());
    assert_eq!(activity.productivity, None);
    assert_eq!(activity.idle_seconds, None);
    assert_eq!(activity.injected_input_events, None);
    assert!(activity.timeline.is_empty(), "all-inactive minutes would read as idle");
    // Apps are still recorded.
    assert!(!activity.active_windows.is_empty());
}

#[test]
fn activity_is_not_measured_when_the_hooks_failed_during_the_block() {
    let perms = all_allowed();
    let measured = Measurements { input_available: false, ..measurements() };
    let activity = build_activity(&block(&perms, None), measured, &device(), zone());
    assert_eq!(activity.productivity, None);
    assert_eq!(activity.idle_seconds, None);
    assert!(activity.timeline.is_empty());
    // What was seen is only part of it: not measured, never an
    // understated count.
    assert_eq!((activity.keyboard_hits, activity.mouse_clicks), (None, None));
}

#[test]
fn only_the_kinds_of_input_recorded_are_counted_the_rest_are_null() {
    // Keys only: clicks are not measured, anywhere in the block.
    let perms = CapturePermissions { mouse: false, ..all_allowed() };
    let measured = Measurements { clicks: Vec::new(), ..measurements() };
    let activity = build_activity(&block(&perms, None), measured, &device(), zone());
    assert_eq!((activity.keyboard_hits, activity.mouse_clicks), (Some(6), None));
    assert!(!activity.timeline.is_empty());
    assert!(activity.timeline.iter().all(|m| m.keyboard_hits.is_some() && m.mouse_clicks.is_none()));
    assert!(activity.active_windows.iter().all(|w| w.mouse_clicks.is_none()));

    // Neither: both null, not 0.
    let none = CapturePermissions { keyboard: false, mouse: false, ..all_allowed() };
    let measured = Measurements { key_presses: Vec::new(), clicks: Vec::new(), ..measurements() };
    let activity = build_activity(&block(&none, None), measured, &device(), zone());
    assert_eq!((activity.keyboard_hits, activity.mouse_clicks), (None, None));
    assert!(activity.active_windows.iter().all(|w| w.keyboard_hits.is_none() && w.mouse_clicks.is_none()));

    // Measured and nothing typed: a real 0.
    let measured = Measurements { key_presses: Vec::new(), clicks: Vec::new(), ..measurements() };
    let activity = build_activity(&block(&all_allowed(), None), measured, &device(), zone());
    assert_eq!((activity.keyboard_hits, activity.mouse_clicks), (Some(0), Some(0)));
}

#[test]
fn the_screen_capture_follows_the_project_policy() {
    let perms = all_allowed();
    let measured = Measurements { screen: Some(capture(false)), ..measurements() };
    let activity = build_activity(&block(&perms, None), measured, &device(), zone());
    assert_eq!(activity.screenshot_png_base64.as_deref(), Some("/9j/screen"));
    assert_eq!(activity.screenshot_thumb_jpg_base64.as_deref(), Some("/9j/thumb"));
    assert_eq!(activity.screenshot_captured_at, Some(at(T0 + 250)));
    assert_eq!(activity.screenshot_blurred, Some(false));

    // Blur required: an unblurred capture is never sent.
    let blur = CapturePermissions { blur_screenshots: true, ..all_allowed() };
    let measured = Measurements { screen: Some(capture(false)), ..measurements() };
    let activity = build_activity(&block(&blur, None), measured, &device(), zone());
    assert!(!activity.has_captures());
    assert!(activity.screenshot_captured_at.is_none() && activity.screenshot_blurred.is_none());
    let measured = Measurements { screen: Some(capture(true)), ..measurements() };
    let activity = build_activity(&block(&blur, None), measured, &device(), zone());
    assert_eq!(activity.screenshot_blurred, Some(true));

    // Screenshots not allowed: dropped, whatever the caller passed.
    let none = CapturePermissions { screenshot: false, webcam: false, ..all_allowed() };
    let measured = Measurements { screen: Some(capture(true)), webcam_jpg_base64: Some("/9j/cam".into()), ..measurements() };
    let activity = build_activity(&block(&none, None), measured, &device(), zone());
    assert!(!activity.has_captures());
}

#[test]
fn a_capture_taken_before_a_lock_is_kept() {
    // The random moment came before the lock; only a capture at the end
    // is forbidden.
    let perms = all_allowed();
    let spec = block(&perms, Some(EndReason::Lock));
    assert!(!spec.captures_at_end());
    let measured = Measurements { screen: Some(capture(false)), ..measurements() };
    let activity = build_activity(&spec, measured, &device(), zone());
    assert!(activity.screenshot_png_base64.is_some());
    assert_eq!(activity.end_reason, Some(EndReason::Lock));
    assert!(block(&perms, None).captures_at_end());
    assert!(block(&perms, Some(EndReason::Pause)).captures_at_end());
}

#[test]
fn app_usage_follows_the_active_window_permission() {
    let perms = CapturePermissions { active_window: false, running_programs: false, ..all_allowed() };
    let measured = Measurements { slices: Vec::new(), running_programs: Vec::new(), ..measurements() };
    let activity = build_activity(&block(&perms, None), measured, &device(), zone());
    assert!(activity.active_windows.is_empty() && activity.running_programs.is_empty());
    assert!(activity.timeline.iter().all(|m| m.app_name.is_none()));

    // A block too short for a sample: the window in front stands in.
    let perms = all_allowed();
    let front = WindowInfo { app_name: "excel.exe".into(), window_title: "Q3".into(), ..Default::default() };
    let measured = Measurements { slices: Vec::new(), window_now: Some(front.clone()), ..measurements() };
    let activity = build_activity(&block(&perms, None), measured, &device(), zone());
    assert_eq!(activity.active_windows, vec![front]);
}

#[test]
fn task_notes_are_trimmed_and_bounded() {
    assert_eq!(clean_description(None), None);
    assert_eq!(clean_description(Some("   ")), None);
    assert_eq!(clean_description(Some(" Fix login ")).as_deref(), Some("Fix login"));
    let long = "x".repeat(5000);
    assert_eq!(clean_description(Some(&long)).map(|d| d.chars().count()), Some(MAX_DESCRIPTION_CHARS));
}

#[test]
fn spans_split_into_equal_pieces_of_at_most_one_interval() {
    let ten = TimeDelta::minutes(10);
    // 42 minutes: five pieces of 8 min 24 s, back to back.
    let pieces = split_span(at(T0), at(T0 + 42 * 60), ten);
    assert_eq!(pieces.len(), 5);
    assert_eq!(pieces[0].0, at(T0));
    assert_eq!(pieces[4].1, at(T0 + 42 * 60));
    assert!(pieces.windows(2).all(|w| w[0].1 == w[1].0), "no gaps");
    assert!(pieces.iter().all(|(s, e)| *e - *s <= ten && *e - *s >= TimeDelta::minutes(8)));
    // Exactly one interval: one piece. Under a minute: nothing.
    assert_eq!(split_span(at(T0), at(T0 + 600), ten).len(), 1);
    assert!(split_span(at(T0), at(T0 + 59), ten).is_empty());
    assert!(split_span(at(T0), at(T0 + 600), TimeDelta::zero()).is_empty());
}

#[test]
fn kept_idle_time_is_recorded_as_idle_blocks() {
    let blocks = kept_idle_blocks(
        "p1",
        at(T0),
        at(T0 + 25 * 60),
        TimeDelta::minutes(10),
        Some("Client call"),
        &device(),
        Some("Asia/Dhaka"),
    );
    assert_eq!(blocks.len(), 3);
    for activity in &blocks {
        let length = (activity.ended_at - activity.started_at).num_seconds() as u32;
        assert!(length <= 600);
        assert_eq!(activity.productivity, Some(0));
        assert_eq!(activity.idle_seconds, Some(length));
        assert_eq!(activity.end_reason, Some(EndReason::Idle));
        assert!(activity.active_windows.is_empty() && activity.timeline.is_empty() && !activity.has_captures());
        assert_eq!(activity.description.as_deref(), Some("Client call"));
        assert_eq!(activity.time_zone.as_deref(), Some("Asia/Dhaka"));
        assert_eq!(activity.device_id, device().device_id);
        assert!(activity.utc_offset_minutes.is_some());
    }
    assert_eq!(blocks[0].started_at, at(T0));
    assert_eq!(blocks[2].ended_at, at(T0 + 25 * 60));
    // Distinct blocks, distinct idempotency keys.
    assert_ne!(blocks[0].client_activity_id, blocks[1].client_activity_id);
    assert!(kept_idle_blocks("p1", at(T0), at(T0 + 30), TimeDelta::minutes(10), None, &device(), None).is_empty());
}

#[test]
fn tracker_active_gate_controls_both_usage_and_input() {
    let (tracker, driver) = Tracker::new_fake();
    assert!(!tracker.is_tracking_active(), "Tracker must default to inactive (opt-in)");
    assert!(!driver.is_input_running());
    assert!(!driver.is_usage_running());

    tracker.set_tracking_active(true);
    assert!(tracker.is_tracking_active());
    assert!(driver.is_input_running());
    assert!(driver.is_usage_running());
    assert!(tracker.usage().is_active());
    assert!(tracker.input().is_active());

    tracker.set_tracking_active(false);
    assert!(!tracker.is_tracking_active());
    assert!(!driver.is_input_running());
    assert!(!driver.is_usage_running());
    assert!(!tracker.usage().is_active());
    assert!(!tracker.input().is_active());
}

#[test]
fn tracker_per_sensor_permission_gating_with_fake_driver() {
    let (tracker, driver) = Tracker::new_fake();
    // Only active_window permitted
    let perms = CapturePermissions {
        keyboard: false,
        mouse: false,
        active_window: true,
        screenshot: false,
        webcam: false,
        running_programs: false,
        blur_screenshots: false,
    };
    tracker.apply_sensor_policy(true, Some(&perms));
    assert!(!driver.is_input_running(), "input hooks must not start when keyboard and mouse are denied");
    assert!(driver.is_usage_running(), "usage sampler should start when active_window is allowed");

    // Only keyboard permitted
    let perms2 = CapturePermissions {
        keyboard: true,
        mouse: false,
        active_window: false,
        ..perms
    };
    tracker.apply_sensor_policy(true, Some(&perms2));
    assert!(driver.is_input_running(), "input hooks should start when keyboard is allowed");
    assert!(!driver.is_usage_running(), "usage sampler must stop when active_window is denied");
    assert!(driver.last_keyboard_allowed());
    assert!(!driver.last_mouse_allowed());
}

#[test]
fn tracker_consent_webcam_withdrawn_while_mouse_retained_stale_spec_zero_camera_calls() {
    let (tracker, driver, acq) = Tracker::new_fake_with_acquisition();
    // 1. Initial grant with mouse and webcam
    let mut initial_perms = CapturePermissions::all_disabled();
    initial_perms.mouse = true;
    initial_perms.webcam = true;
    tracker.apply_sensor_policy(true, Some(&initial_perms));
    assert!(tracker.is_tracking_active());
    assert!(driver.is_input_running());

    // 2. User narrows consent: webcam is withdrawn, mouse is retained
    let mut narrowed_perms = initial_perms;
    narrowed_perms.webcam = false;
    tracker.apply_sensor_policy(true, Some(&narrowed_perms));
    assert!(tracker.is_tracking_active());
    assert!(driver.is_input_running());

    // 3. Stale block spec with webcam = true passed to snapshot
    let mut stale_perms = CapturePermissions::all_disabled();
    stale_perms.mouse = true;
    stale_perms.webcam = true; // STALE!
    let spec = block(&stale_perms, Some(EndReason::Interval));

    let activity = tracker.snapshot(&spec, None);

    // 4. Zero camera calls on acquisition provider, and webcam_jpg_base64 is None
    assert_eq!(acq.webcam_calls(), 0, "must NOT invoke camera when webcam is withdrawn in current effective policy");
    assert!(activity.webcam_jpg_base64.is_none());
}

#[test]
fn tracker_consent_screenshot_withdrawn_direct_capture_screen_zero_calls() {
    let (tracker, _driver, acq) = Tracker::new_fake_with_acquisition();
    // 1. Initially active with screenshot
    let mut perms = CapturePermissions::all_disabled();
    perms.screenshot = true;
    tracker.apply_sensor_policy(true, Some(&perms));

    let cap = tracker.capture_screen(false);
    assert!(cap.is_some());
    assert_eq!(acq.screen_calls(), 1);

    // 2. User withdraws screenshot
    perms.screenshot = false;
    tracker.apply_sensor_policy(true, Some(&perms));

    // 3. Direct capture_screen call
    let cap_denied = tracker.capture_screen(false);
    assert!(cap_denied.is_none(), "capture_screen must return None when screenshot withdrawn");
    assert_eq!(acq.screen_calls(), 1, "provider screen_calls must remain 1 (zero new calls)");
}

#[test]
fn tracker_consent_running_programs_withdrawn_active_window_retained_zero_process_calls() {
    let (tracker, driver, acq) = Tracker::new_fake_with_acquisition();
    // 1. Initially active with active_window AND running_programs
    let mut initial_perms = CapturePermissions::all_disabled();
    initial_perms.active_window = true;
    initial_perms.running_programs = true;
    tracker.apply_sensor_policy(true, Some(&initial_perms));
    assert!(driver.is_usage_running());

    // 2. Narrowed: running_programs withdrawn, active_window retained
    let mut narrowed_perms = initial_perms;
    narrowed_perms.running_programs = false;
    tracker.apply_sensor_policy(true, Some(&narrowed_perms));
    assert!(driver.is_usage_running(), "usage driver still running for active_window");

    // 3. Stale block spec with running_programs = true
    let mut stale_perms = CapturePermissions::all_disabled();
    stale_perms.active_window = true;
    stale_perms.running_programs = true; // STALE!
    let spec = block(&stale_perms, Some(EndReason::Interval));

    let activity = tracker.snapshot(&spec, None);

    // 4. Zero process queries on acquisition provider
    assert_eq!(acq.process_calls(), 0, "must NOT query running programs when withdrawn in current effective policy");
    assert!(activity.running_programs.is_empty());
    // Meanwhile, window query was permitted because active_window was retained
    assert_eq!(acq.window_calls(), 1);
    assert!(!activity.active_windows.is_empty());
}

#[test]
fn tracker_consent_inactive_or_paused_with_stale_spec_zero_acquisitions() {
    let (tracker, driver, acq) = Tracker::new_fake_with_acquisition();
    // Tracking is stopped / paused / inactive (default or explicitly paused)
    tracker.apply_sensor_policy(false, None);
    assert!(!tracker.is_tracking_active());
    assert!(!driver.is_input_running());
    assert!(!driver.is_usage_running());

    // Stale block spec with everything allowed
    let stale_perms = CapturePermissions::default(); // all true
    let spec = block(&stale_perms, Some(EndReason::Pause));

    // Direct capture_screen attempt
    let cap = tracker.capture_screen(false);
    assert!(cap.is_none());
    assert_eq!(acq.screen_calls(), 0);

    // Direct current_app attempt
    let app = tracker.current_app();
    assert!(app.is_none());
    assert_eq!(acq.window_calls(), 0);

    // Direct snapshot attempt
    let activity = tracker.snapshot(&spec, None);
    assert_eq!(acq.webcam_calls(), 0, "0 webcam calls when inactive");
    assert_eq!(acq.window_calls(), 0, "0 window calls when inactive");
    assert_eq!(acq.process_calls(), 0, "0 process calls when inactive");
    assert!(activity.webcam_jpg_base64.is_none());
    assert!(activity.screenshot_png_base64.is_none());
    assert!(activity.running_programs.is_empty());
}

#[test]
fn tracker_consent_legitimately_active_webcam_only_policy_invokes_camera_without_input_or_usage_drivers() {
    let (tracker, driver, acq) = Tracker::new_fake_with_acquisition();
    // Webcam only active: input and active_window denied
    let mut webcam_only = CapturePermissions::all_disabled();
    webcam_only.webcam = true;
    tracker.apply_sensor_policy(true, Some(&webcam_only));

    // Tracking is active, but input and usage drivers are NOT running!
    assert!(tracker.is_tracking_active(), "tracking must be active for authorized webcam-only policy");
    assert!(!driver.is_input_running(), "input driver must NOT run");
    assert!(!driver.is_usage_running(), "usage driver must NOT run");

    let spec = block(&webcam_only, Some(EndReason::Interval));
    let activity = tracker.snapshot(&spec, None);

    // Synthetic camera provider WAS called!
    assert_eq!(acq.webcam_calls(), 1, "synthetic camera provider MUST be called for authorized webcam");
    assert!(activity.webcam_jpg_base64.is_some());
    assert_eq!(acq.window_calls(), 0);
    assert_eq!(acq.process_calls(), 0);
    assert_eq!(acq.screen_calls(), 0);
}

#[test]
fn tracker_consent_held_block_safe_finalization_preserves_historical_accumulators_without_new_samples() {
    let (tracker, driver, acq) = Tracker::new_fake_with_acquisition();
    // 1. Block ran while keyboard, mouse, and active_window were authorized
    let mut authorized_perms = CapturePermissions::all_disabled();
    authorized_perms.keyboard = true;
    authorized_perms.mouse = true;
    authorized_perms.active_window = true;
    authorized_perms.webcam = true;
    tracker.apply_sensor_policy(true, Some(&authorized_perms));

    // Record some historical input and usage slices during the block
    tracker.input().record_key_press(at(T0 + 10).timestamp_millis());
    tracker.input().record_click(at(T0 + 20).timestamp_millis());
    tracker.usage().record(at(T0 + 30).timestamp_millis(), Some(WindowInfo {
        app_name: "code.exe".into(),
        window_title: "code".into(),
        ..Default::default()
    }));
    tracker.usage().record(at(T0 + 60).timestamp_millis(), Some(WindowInfo {
        app_name: "code.exe".into(),
        window_title: "code".into(),
        ..Default::default()
    }));

    // 2. User revokes all consent before held block finalization
    tracker.apply_sensor_policy(false, None);
    assert!(!tracker.is_tracking_active());
    assert!(!driver.is_input_running());
    assert!(!driver.is_usage_running());

    // 3. Held block is finalized with its historical block perms
    let spec = block(&authorized_perms, Some(EndReason::Interval));
    let activity = tracker.snapshot(&spec, None);

    // 4. Assert NO new samples or captures were taken
    assert_eq!(acq.webcam_calls(), 0, "zero new webcam calls upon finalized held block after revocation");
    assert_eq!(acq.screen_calls(), 0, "zero new screen calls");
    assert_eq!(acq.window_calls(), 0, "zero new window queries");
    assert_eq!(acq.process_calls(), 0, "zero new process queries");

    // 5. But historical accumulators (keys, clicks, slices) are preserved!
    assert_eq!(activity.keyboard_hits, Some(1), "historical key presses preserved");
    assert_eq!(activity.mouse_clicks, Some(1), "historical clicks preserved");
    assert!(!activity.active_windows.is_empty(), "historical usage slices preserved");
    assert!(activity.webcam_jpg_base64.is_none());
}

#[test]
fn tracker_consent_conservative_blur_applies_if_either_current_or_block_requires_blur() {
    let (tracker, _driver, acq) = Tracker::new_fake_with_acquisition();
    let mut blur_perms = CapturePermissions::all_disabled();
    blur_perms.screenshot = true;
    blur_perms.blur_screenshots = true;
    tracker.apply_sensor_policy(true, Some(&blur_perms));

    // 1. capture_screen with blur=false must force blur=true because current_perms requires blur
    let cap = tracker.capture_screen(false);
    assert!(cap.is_some());
    assert!(acq.last_blur(), "capture_screen must pass blur=true to provider when policy requires blur");

    // 2. snapshot with unblurred capture must drop it when current_perms requires blur
    let unblurred_cap = prov_cap(&tracker, false);
    let spec = block(&blur_perms, Some(EndReason::Interval));
    let activity = tracker.snapshot(&spec, Some(unblurred_cap));
    assert!(activity.screenshot_png_base64.is_none(), "unblurred capture must be dropped when policy requires blur");

    // 3. snapshot with blurred capture is kept
    let blurred_cap = prov_cap(&tracker, true);
    let activity2 = tracker.snapshot(&spec, Some(blurred_cap));
    assert!(activity2.screenshot_png_base64.is_some());
    assert_eq!(activity2.screenshot_blurred, Some(true));

    // 4. snapshot with unblurred capture must drop it if historical block requires blur, even if current policy relaxes blur
    let mut no_blur_perms = blur_perms;
    no_blur_perms.blur_screenshots = false;
    tracker.apply_sensor_policy(true, Some(&no_blur_perms));
    let unblurred_cap2 = prov_cap(&tracker, false);
    let activity3 = tracker.snapshot(&spec, Some(unblurred_cap2));
    assert!(activity3.screenshot_png_base64.is_none(), "unblurred capture must be dropped when block requires blur even if current policy relaxed blur");
}

#[test]
fn tracker_consent_snapshot_interleaving_revocation_prevents_new_webcam_admission() {
    let (tracker, driver, acq) = Tracker::new_fake_with_acquisition();
    let tracker = Arc::new(tracker);

    // 1. Initial grant with active_window and webcam
    let mut initial_perms = CapturePermissions::all_disabled();
    initial_perms.active_window = true;
    initial_perms.webcam = true;
    tracker.apply_sensor_policy(true, Some(&initial_perms));
    assert!(tracker.is_tracking_active());
    assert!(driver.is_usage_running());

    // 2. Set up deterministic channels to pause inside current_window
    let (window_entered_tx, window_entered_rx) = std::sync::mpsc::channel();
    let (revoke_done_tx, revoke_done_rx) = std::sync::mpsc::channel();
    let revoke_done_rx = Arc::new(Mutex::new(revoke_done_rx));
    let rx_for_hook = revoke_done_rx.clone();

    acq.set_on_current_window(move || {
        window_entered_tx.send(()).unwrap();
        rx_for_hook.lock().unwrap().recv_timeout(std::time::Duration::from_secs(5)).expect("timeout waiting for revocation");
    });

    // 3. Stale block spec with active_window and webcam
    let spec_perms = initial_perms;
    let tracker_clone = tracker.clone();
    let handle = std::thread::spawn(move || {
        let spec = block(&spec_perms, Some(EndReason::Interval));
        tracker_clone.snapshot(&spec, None)
    });

    // 4. Wait for snapshot to enter current_window
    window_entered_rx.recv_timeout(std::time::Duration::from_secs(5)).expect("timeout waiting for current_window entry");

    // 5. Concurrently revoke all consent while snapshot is paused in current_window
    tracker.apply_sensor_policy(false, None);
    assert!(!tracker.is_tracking_active());
    assert!(!driver.is_usage_running());

    // 6. Release current_window so snapshot can proceed
    revoke_done_tx.send(()).unwrap();

    // 7. Await snapshot completion
    let activity = handle.join().expect("snapshot thread panicked");

    // 8. In-flight window call completed, but was discarded on exit because revocation completed in flight
    assert_eq!(acq.window_calls(), 1);
    assert!(activity.active_windows.is_empty(), "in-flight window result must be dropped upon revocation");

    // 9. PROVE ZERO NEW WEBCAM ADMISSIONS: capture_webcam must NEVER be admitted after withdrawal!
    assert_eq!(acq.webcam_calls(), 0, "must NOT admit webcam call after withdrawal completed");
    assert!(activity.webcam_jpg_base64.is_none());
    assert_eq!(acq.process_calls(), 0);
    assert_eq!(acq.screen_calls(), 0);
}

#[test]
fn tracker_consent_process_only_positive_regression() {
    let (tracker, driver, acq) = Tracker::new_fake_with_acquisition();

    // 1. Process-only active policy: running_programs = true, active_window/input denied
    let mut process_only = CapturePermissions::all_disabled();
    process_only.running_programs = true;
    tracker.apply_sensor_policy(true, Some(&process_only));

    // 2. Tracking is active, but usage and input drivers remain strictly stopped
    assert!(tracker.is_tracking_active(), "tracker must be active for process-only grant");
    assert!(!driver.is_usage_running(), "usage driver must NOT run when active_window is denied");
    assert!(!driver.is_input_running(), "input driver must NOT run when input is denied");

    // 3. Snapshot with block permitting running_programs
    let spec = block(&process_only, Some(EndReason::Interval));
    let activity = tracker.snapshot(&spec, None);

    // 4. Exactly one synthetic process query on provider; zero window/camera/screen calls
    assert_eq!(acq.process_calls(), 1, "exactly one process query admitted");
    assert_eq!(acq.window_calls(), 0, "zero window calls");
    assert_eq!(acq.webcam_calls(), 0, "zero webcam calls");
    assert_eq!(acq.screen_calls(), 0, "zero screen calls");

    // 5. Returned fixture is present in emitted activity
    assert_eq!(activity.running_programs.len(), 1);
    assert_eq!(activity.running_programs[0].app_name, "code.exe");
    assert!(activity.active_windows.is_empty());
}

#[test]
fn tracker_consent_capture_screen_race_and_revocation_prevents_subsequent_admission() {
    let (tracker, _driver, acq) = Tracker::new_fake_with_acquisition();
    let tracker = Arc::new(tracker);

    let mut perms = CapturePermissions::all_disabled();
    perms.screenshot = true;
    tracker.apply_sensor_policy(true, Some(&perms));

    let (screen_entered_tx, screen_entered_rx) = std::sync::mpsc::channel();
    let (revoke_done_tx, revoke_done_rx) = std::sync::mpsc::channel();
    let revoke_done_rx = Arc::new(Mutex::new(revoke_done_rx));
    let rx_for_hook = revoke_done_rx.clone();

    acq.set_on_capture_screen(move |_blur| {
        screen_entered_tx.send(()).unwrap();
        rx_for_hook.lock().unwrap().recv_timeout(std::time::Duration::from_secs(5)).expect("timeout waiting for revocation");
    });

    let tracker_clone = tracker.clone();
    let handle = std::thread::spawn(move || {
        tracker_clone.capture_screen(false)
    });

    // Wait until provider is entered
    screen_entered_rx.recv_timeout(std::time::Duration::from_secs(5)).expect("timeout waiting for capture_screen entry");

    // Revoke screenshot capability while in-flight
    tracker.apply_sensor_policy(false, None);
    assert!(!tracker.is_tracking_active());

    // Release in-flight capture
    revoke_done_tx.send(()).unwrap();

    let cap = handle.join().expect("thread panicked");
    // In-flight capture completed on hardware/provider, but post-admission verification discards it
    assert!(cap.is_none(), "in-flight capture must be discarded if revoked while in flight");
    assert_eq!(acq.screen_calls(), 1);

    // Subsequent call to capture_screen: rejected at admission (zero new provider calls)
    let second_cap = tracker.capture_screen(false);
    assert!(second_cap.is_none());
    assert_eq!(acq.screen_calls(), 1, "zero subsequent admissions after withdrawal");
}

#[test]
fn tracker_consent_pause_and_partial_narrowing_during_snapshot() {
    let (tracker, driver, acq) = Tracker::new_fake_with_acquisition();
    let tracker = Arc::new(tracker);

    let mut initial_perms = CapturePermissions::all_disabled();
    initial_perms.active_window = true;
    initial_perms.running_programs = true;
    initial_perms.webcam = true;
    tracker.apply_sensor_policy(true, Some(&initial_perms));

    let (window_entered_tx, window_entered_rx) = std::sync::mpsc::channel();
    let (narrow_done_tx, narrow_done_rx) = std::sync::mpsc::channel();
    let narrow_done_rx = Arc::new(Mutex::new(narrow_done_rx));
    let rx_for_hook = narrow_done_rx.clone();

    acq.set_on_current_window(move || {
        window_entered_tx.send(()).unwrap();
        rx_for_hook.lock().unwrap().recv_timeout(std::time::Duration::from_secs(5)).expect("timeout waiting for narrowing");
    });

    let spec_perms = initial_perms;
    let tracker_clone = tracker.clone();
    let handle = std::thread::spawn(move || {
        let spec = block(&spec_perms, Some(EndReason::Interval));
        tracker_clone.snapshot(&spec, None)
    });

    window_entered_rx.recv_timeout(std::time::Duration::from_secs(5)).expect("timeout");

    // Partial narrowing: revoke active_window and webcam, but RETAIN running_programs
    let mut narrowed = CapturePermissions::all_disabled();
    narrowed.running_programs = true;
    tracker.apply_sensor_policy(true, Some(&narrowed));
    assert!(!driver.is_usage_running(), "usage driver stopped when active_window revoked");

    narrow_done_tx.send(()).unwrap();

    let activity = handle.join().expect("snapshot thread panicked");

    // 1. In-flight window call was discarded
    assert_eq!(acq.window_calls(), 1);
    assert!(activity.active_windows.is_empty());

    // 2. Retained running_programs capability WAS admitted!
    assert_eq!(acq.process_calls(), 1, "retained running_programs must be queried");
    assert_eq!(activity.running_programs.len(), 1);

    // 3. Withdrawn webcam was REJECTED at admission (zero calls!)
    assert_eq!(acq.webcam_calls(), 0, "withdrawn webcam must not be called");
    assert!(activity.webcam_jpg_base64.is_none());
}

#[test]
fn tracker_consent_no_deadlock_between_slow_provider_and_sensor_policy() {
    let (tracker, _driver, acq) = Tracker::new_fake_with_acquisition();
    let tracker = Arc::new(tracker);

    let mut perms = CapturePermissions::all_disabled();
    perms.active_window = true;
    tracker.apply_sensor_policy(true, Some(&perms));

    let (entered_tx, entered_rx) = std::sync::mpsc::channel();
    let (release_tx, release_rx) = std::sync::mpsc::channel();
    let release_rx = Arc::new(Mutex::new(release_rx));
    let rx_for_hook = release_rx.clone();

    acq.set_on_current_window(move || {
        entered_tx.send(()).unwrap();
        rx_for_hook.lock().unwrap().recv_timeout(std::time::Duration::from_secs(5)).expect("timeout waiting for release");
    });

    let tracker_clone = tracker.clone();
    let handle = std::thread::spawn(move || {
        tracker_clone.current_app()
    });

    entered_rx.recv_timeout(std::time::Duration::from_secs(5)).expect("timeout");

    // Crucial test: apply_sensor_policy must NOT block while current_window is executing!
    // We assert apply_sensor_policy completes immediately without deadlocking or waiting for release.
    tracker.apply_sensor_policy(false, None);
    assert!(!tracker.is_tracking_active());

    // Now release the provider
    release_tx.send(()).unwrap();
    let app = handle.join().expect("thread panicked");
    assert!(app.is_none(), "result discarded because revoked while in flight");
}

#[test]
fn tracker_consent_capture_screen_revocation_regrant_aba_rejects_old_capture_and_fresh_succeeds() {
    let (tracker, _driver, acq) = Tracker::new_fake_with_acquisition();
    let tracker = Arc::new(tracker);

    let mut perms = CapturePermissions::all_disabled();
    perms.screenshot = true;
    tracker.apply_sensor_policy(true, Some(&perms));

    let (screen_entered_tx, screen_entered_rx) = std::sync::mpsc::channel();
    let (release_tx, release_rx) = std::sync::mpsc::channel();
    let release_rx = Arc::new(Mutex::new(release_rx));
    let rx_for_hook = release_rx.clone();

    acq.set_on_capture_screen(move |_blur| {
        screen_entered_tx.send(()).unwrap();
        rx_for_hook.lock().unwrap().recv_timeout(std::time::Duration::from_secs(5)).expect("timeout waiting for release");
    });

    let tracker_clone = tracker.clone();
    let handle = std::thread::spawn(move || {
        tracker_clone.capture_screen(false)
    });

    // 1. Wait until provider is entered
    screen_entered_rx.recv_timeout(std::time::Duration::from_secs(5)).expect("timeout waiting for capture_screen entry");

    // 2. Revoke screenshot capability while in-flight
    tracker.apply_sensor_policy(false, None);
    assert!(!tracker.is_tracking_active());

    // 3. Regrant screenshot capability BEFORE releasing the old provider
    tracker.apply_sensor_policy(true, Some(&perms));
    assert!(tracker.is_tracking_active());

    // 4. Release old in-flight provider
    release_tx.send(()).unwrap();

    let old_cap = handle.join().expect("thread panicked");
    // 5. Assert: old capture rejected despite current policy having screenshot = true (ABA immunity)
    assert!(old_cap.is_none(), "old capture from invalidated consent interval must be rejected after regrant");
    assert_eq!(acq.screen_calls(), 1);

    // 6. Assert: genuinely fresh post-regrant acquisition succeeds!
    acq.set_on_capture_screen(|_| {});
    let fresh_cap = tracker.capture_screen(false);
    assert!(fresh_cap.is_some(), "fresh capture admitted and validated under current generation");
    assert_eq!(acq.screen_calls(), 2);
}

#[test]
fn tracker_consent_webcam_revocation_regrant_aba_rejects_old_capture_and_fresh_succeeds() {
    let (tracker, _driver, acq) = Tracker::new_fake_with_acquisition();
    let tracker = Arc::new(tracker);
    acq.set_canned_webcam(Some("valid_fake_webcam_data".into()));

    let mut perms = CapturePermissions::all_disabled();
    perms.webcam = true;
    tracker.apply_sensor_policy(true, Some(&perms));

    let (webcam_entered_tx, webcam_entered_rx) = std::sync::mpsc::channel();
    let (release_tx, release_rx) = std::sync::mpsc::channel();
    let release_rx = Arc::new(Mutex::new(release_rx));
    let rx_for_hook = release_rx.clone();

    acq.set_on_capture_webcam(move || {
        webcam_entered_tx.send(()).unwrap();
        rx_for_hook.lock().unwrap().recv_timeout(std::time::Duration::from_secs(5)).expect("timeout waiting for release");
    });

    let tracker_clone = tracker.clone();
    let spec_perms = perms;
    let handle = std::thread::spawn(move || {
        let spec = block(&spec_perms, Some(EndReason::Interval));
        tracker_clone.snapshot(&spec, None)
    });

    // 1. Wait until provider is entered
    webcam_entered_rx.recv_timeout(std::time::Duration::from_secs(5)).expect("timeout waiting for webcam entry");

    // 2. Revoke webcam capability while in-flight
    tracker.apply_sensor_policy(false, None);
    assert!(!tracker.is_tracking_active());

    // 3. Regrant webcam capability BEFORE releasing the old provider
    tracker.apply_sensor_policy(true, Some(&perms));
    assert!(tracker.is_tracking_active());

    // 4. Release old in-flight provider
    release_tx.send(()).unwrap();

    let activity = handle.join().expect("thread panicked");
    // 5. Assert: old webcam capture rejected despite current policy having webcam = true (ABA immunity)
    assert!(activity.webcam_jpg_base64.is_none(), "old webcam capture from invalidated interval must be rejected");
    assert_eq!(acq.webcam_calls(), 1);

    // 6. Assert: genuinely fresh post-regrant acquisition succeeds!
    acq.set_on_capture_webcam(|| {});
    let fresh_spec = block(&perms, Some(EndReason::Interval));
    let fresh_activity = tracker.snapshot(&fresh_spec, None);
    assert!(fresh_activity.webcam_jpg_base64.is_some(), "fresh webcam capture admitted and validated under current generation");
    assert_eq!(acq.webcam_calls(), 2);
}

#[test]
fn tracker_consent_current_app_and_snapshot_window_revocation_regrant_aba_rejected() {
    let (tracker, _driver, acq) = Tracker::new_fake_with_acquisition();
    let tracker = Arc::new(tracker);
    acq.set_canned_window(Some(WindowInfo {
        app_name: "test_editor.exe".into(),
        window_title: "test editor".into(),
        ..Default::default()
    }));

    let mut perms = CapturePermissions::all_disabled();
    perms.active_window = true;
    tracker.apply_sensor_policy(true, Some(&perms));

    // Part A: current_app ABA test
    {
        let (entered_tx, entered_rx) = std::sync::mpsc::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        let release_rx = Arc::new(Mutex::new(release_rx));
        let rx_for_hook = release_rx.clone();

        acq.set_on_current_window(move || {
            entered_tx.send(()).unwrap();
            rx_for_hook.lock().unwrap().recv_timeout(std::time::Duration::from_secs(5)).expect("timeout waiting for release");
        });

        let tracker_clone = tracker.clone();
        let handle = std::thread::spawn(move || {
            tracker_clone.current_app()
        });

        entered_rx.recv_timeout(std::time::Duration::from_secs(5)).expect("timeout waiting for current_app entry");

        // ABA transition: revoke then regrant before releasing provider
        tracker.apply_sensor_policy(false, None);
        tracker.apply_sensor_policy(true, Some(&perms));

        release_tx.send(()).unwrap();
        let app = handle.join().expect("thread panicked");
        assert!(app.is_none(), "old current_app result from invalidated consent interval must be rejected");
    }

    // Part B: snapshot current_window ABA test
    {
        let (entered_tx, entered_rx) = std::sync::mpsc::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        let release_rx = Arc::new(Mutex::new(release_rx));
        let rx_for_hook = release_rx.clone();

        acq.set_on_current_window(move || {
            entered_tx.send(()).unwrap();
            rx_for_hook.lock().unwrap().recv_timeout(std::time::Duration::from_secs(5)).expect("timeout waiting for release");
        });

        let tracker_clone = tracker.clone();
        let spec_perms = perms;
        let handle = std::thread::spawn(move || {
            let spec = block(&spec_perms, Some(EndReason::Interval));
            tracker_clone.snapshot(&spec, None)
        });

        entered_rx.recv_timeout(std::time::Duration::from_secs(5)).expect("timeout waiting for snapshot window entry");

        // ABA transition: revoke then regrant before releasing provider
        tracker.apply_sensor_policy(false, None);
        tracker.apply_sensor_policy(true, Some(&perms));

        release_tx.send(()).unwrap();
        let activity = handle.join().expect("thread panicked");
        assert!(activity.active_windows.is_empty(), "old window from invalidated consent interval must not be recorded");
    }

    // Part C: fresh current_app succeeds under current generation
    acq.set_on_current_window(|| {});
    let fresh_app = tracker.current_app();
    assert_eq!(fresh_app.as_deref(), Some("test_editor.exe"));
}

#[test]
fn tracker_consent_running_programs_revocation_regrant_aba_rejected() {
    let (tracker, _driver, acq) = Tracker::new_fake_with_acquisition();
    let tracker = Arc::new(tracker);
    acq.set_canned_processes(Some(vec![WindowInfo {
        app_name: "proc_worker.exe".into(),
        window_title: "proc worker".into(),
        ..Default::default()
    }]));

    let mut perms = CapturePermissions::all_disabled();
    perms.running_programs = true;
    tracker.apply_sensor_policy(true, Some(&perms));

    let (entered_tx, entered_rx) = std::sync::mpsc::channel();
    let (release_tx, release_rx) = std::sync::mpsc::channel();
    let release_rx = Arc::new(Mutex::new(release_rx));
    let rx_for_hook = release_rx.clone();

    acq.set_on_running_programs(move || {
        entered_tx.send(()).unwrap();
        rx_for_hook.lock().unwrap().recv_timeout(std::time::Duration::from_secs(5)).expect("timeout waiting for release");
    });

    let tracker_clone = tracker.clone();
    let spec_perms = perms;
    let handle = std::thread::spawn(move || {
        let spec = block(&spec_perms, Some(EndReason::Interval));
        tracker_clone.snapshot(&spec, None)
    });

    entered_rx.recv_timeout(std::time::Duration::from_secs(5)).expect("timeout waiting for running_programs entry");

    // ABA transition: revoke then regrant before releasing provider
    tracker.apply_sensor_policy(false, None);
    tracker.apply_sensor_policy(true, Some(&perms));

    release_tx.send(()).unwrap();
    let activity = handle.join().expect("thread panicked");
    assert!(activity.running_programs.is_empty(), "old processes from invalidated interval must be rejected");

    // Fresh acquisition succeeds
    acq.set_on_running_programs(|| {});
    let fresh_spec = block(&perms, Some(EndReason::Interval));
    let fresh_activity = tracker.snapshot(&fresh_spec, None);
    assert_eq!(fresh_activity.running_programs.len(), 1);
    assert_eq!(fresh_activity.running_programs[0].app_name, "proc_worker.exe");
}

#[test]
fn tracker_consent_final_publication_rejects_earlier_results_after_later_provider_aba() {
    let (tracker, _driver, acq) = Tracker::new_fake_with_acquisition();
    let tracker = Arc::new(tracker);

    // 1. Initial grant authorizing all sensors: keyboard, mouse, active_window, running_programs, screenshot, webcam
    let mut perms = CapturePermissions::all_disabled();
    perms.keyboard = true;
    perms.mouse = true;
    perms.active_window = true;
    perms.running_programs = true;
    perms.screenshot = true;
    perms.webcam = true;
    tracker.apply_sensor_policy(true, Some(&perms));

    // 2. Populate genuinely authorized historical accumulators during the block [T0, T0+600]
    tracker.input().record_key_press(at(T0 + 10).timestamp_millis());
    tracker.input().record_click(at(T0 + 20).timestamp_millis());
    tracker.usage().record(
        at(T0 + 10).timestamp_millis(),
        Some(WindowInfo {
            app_name: "historical_app.exe".into(),
            window_title: "historical title".into(),
            ..Default::default()
        }),
    );
    tracker.usage().record(
        at(T0 + 20).timestamp_millis(),
        Some(WindowInfo {
            app_name: "historical_app.exe".into(),
            window_title: "historical title".into(),
            ..Default::default()
        }),
    );
    let historical_slice_count_before = tracker.usage().slices_between(at(T0).timestamp_millis(), at(T0 + 600).timestamp_millis()).len();
    assert_eq!(historical_slice_count_before, 1, "must have 1 authorized historical usage slice");

    // 3. Gather earlier screen capture under current generation
    let prov_cap = tracker.capture_screen(false);
    assert!(prov_cap.is_some(), "capture_screen under initial grant must succeed");

    // 4. Configure acquisition provider for window, processes, and webcam
    acq.set_canned_window(Some(WindowInfo {
        app_name: "early_app.exe".into(),
        window_title: "early title".into(),
        ..Default::default()
    }));
    acq.set_canned_processes(Some(vec![WindowInfo {
        app_name: "early_proc.exe".into(),
        window_title: "proc title".into(),
        ..Default::default()
    }]));

    // 5. Block inside webcam acquisition (which runs AFTER window and process queries)
    let (webcam_entered_tx, webcam_entered_rx) = std::sync::mpsc::channel();
    let (release_tx, release_rx) = std::sync::mpsc::channel();
    let release_rx = Arc::new(Mutex::new(release_rx));
    let rx_for_hook = release_rx.clone();

    acq.set_on_capture_webcam(move || {
        webcam_entered_tx.send(()).unwrap();
        rx_for_hook.lock().unwrap().recv_timeout(std::time::Duration::from_secs(5)).expect("timeout waiting for webcam release");
    });

    let tracker_clone = tracker.clone();
    let spec_perms = perms;
    let prov_cap_clone = prov_cap.clone();
    let handle = std::thread::spawn(move || {
        let spec = block(&spec_perms, Some(EndReason::Interval));
        tracker_clone.snapshot(&spec, prov_cap_clone)
    });

    // Wait until webcam provider is entered (meaning window, process, and screen checks are staged)
    webcam_entered_rx.recv_timeout(std::time::Duration::from_secs(5)).expect("timeout waiting for webcam entry");

    // 6. ABA transition: revoke then re-grant all capabilities BEFORE releasing webcam
    tracker.apply_sensor_policy(false, None);
    tracker.apply_sensor_policy(true, Some(&perms));

    // 7. Release webcam provider
    release_tx.send(()).unwrap();

    let activity = handle.join().expect("thread panicked");

    // 8. Prove invalidated NEW values are excluded at final activity assembly:
    assert!(activity.running_programs.is_empty(), "early running programs collected before webcam block must be rejected at final publication boundary");
    assert!(activity.screenshot_png_base64.is_none(), "early screenshot from invalidated consent interval must be rejected at final publication boundary");
    assert!(activity.webcam_jpg_base64.is_none(), "webcam captured in invalidated interval must be rejected");

    // 9. Prove usage is NOT contaminated:
    // The invalidated window sample from ended_at must NOT have been recorded via usage.record()
    let historical_slices_after = tracker.usage().slices_between(at(T0).timestamp_millis(), at(T0 + 600).timestamp_millis());
    assert_eq!(historical_slices_after.len(), historical_slice_count_before, "usage.record must NOT record invalidated window sample at block end");
    assert!(
        historical_slices_after.iter().all(|s| s.app != "early_app.exe"),
        "historical usage must not be contaminated with early_app.exe from invalidated consent interval"
    );

    // 10. Prove genuinely authorized historical accumulators ARE preserved:
    assert_eq!(activity.keyboard_hits, Some(1), "genuinely authorized historical keyboard events must be preserved");
    assert_eq!(activity.mouse_clicks, Some(1), "genuinely authorized historical mouse clicks must be preserved");
    assert_eq!(activity.active_windows.len(), 1, "active_windows from authorized historical slices must be preserved");
    assert_eq!(activity.active_windows[0].app_name, "historical_app.exe");
    assert!(activity.active_windows.iter().all(|w| w.app_name != "early_app.exe"), "early_app.exe must not appear in active_windows");

    // 11. Prove fresh post-regrant behavior:
    acq.set_on_capture_webcam(|| {});
    let fresh_prov_cap = tracker.capture_screen(false);
    assert!(fresh_prov_cap.is_some(), "fresh capture_screen under post-regrant generation succeeds");
    let fresh_spec = block(&perms, Some(EndReason::Interval));
    let fresh_activity = tracker.snapshot(&fresh_spec, fresh_prov_cap);
    assert!(fresh_activity.screenshot_png_base64.is_some(), "fresh screenshot accepted under current generation");
    assert!(fresh_activity.webcam_jpg_base64.is_some(), "fresh webcam accepted under current generation");
    assert_eq!(fresh_activity.running_programs.len(), 1, "fresh running programs accepted under current generation");
    assert_eq!(fresh_activity.running_programs[0].app_name, "early_proc.exe");
}

#[test]
fn tracker_consent_screenshot_revocation_regrant_aba_rejects_old_capture_and_fresh_succeeds() {
    let (tracker, _driver, _acq) = Tracker::new_fake_with_acquisition();
    let tracker = Arc::new(tracker);

    let mut perms = CapturePermissions::all_disabled();
    perms.screenshot = true;
    tracker.apply_sensor_policy(true, Some(&perms));

    // 1. Take capture_screen under initial grant
    let prov_cap = tracker.capture_screen(false);
    assert!(prov_cap.is_some());

    // 2. Revoke and regrant screenshot permission (ABA transition)
    tracker.apply_sensor_policy(false, None);
    tracker.apply_sensor_policy(true, Some(&perms));

    // 3. Snapshot with old prov_cap must reject the screenshot due to generation mismatch
    let spec = block(&perms, Some(EndReason::Interval));
    let activity = tracker.snapshot(&spec, prov_cap);
    assert!(activity.screenshot_png_base64.is_none(), "old screenshot from invalidated consent interval must be rejected");

    // 4. Fresh capture_screen under new generation succeeds
    let fresh_prov_cap = tracker.capture_screen(false);
    assert!(fresh_prov_cap.is_some());
    let activity2 = tracker.snapshot(&spec, fresh_prov_cap);
    assert!(activity2.screenshot_png_base64.is_some(), "fresh screenshot under current generation succeeds");
}

#[test]
fn tracker_consent_screenshot_rejected_after_later_webcam_aba_revocation() {
    let (tracker, _driver, acq) = Tracker::new_fake_with_acquisition();
    let tracker = Arc::new(tracker);
    acq.set_canned_webcam(Some("/9j/valid_webcam_during_blocked_screen_aba".into()));

    let mut perms = CapturePermissions::all_disabled();
    perms.screenshot = true;
    perms.webcam = true;
    tracker.apply_sensor_policy(true, Some(&perms));

    let prov_cap = tracker.capture_screen(false);
    assert!(prov_cap.is_some());

    // Block inside webcam acquisition (running after screenshot handoff)
    let (webcam_entered_tx, webcam_entered_rx) = std::sync::mpsc::channel();
    let (release_tx, release_rx) = std::sync::mpsc::channel();
    let release_rx = Arc::new(Mutex::new(release_rx));
    let rx_for_hook = release_rx.clone();

    acq.set_on_capture_webcam(move || {
        webcam_entered_tx.send(()).unwrap();
        rx_for_hook.lock().unwrap().recv_timeout(std::time::Duration::from_secs(5)).expect("timeout waiting for release");
    });

    let tracker_clone = tracker.clone();
    let spec_perms = perms;
    let prov_cap_clone = prov_cap.clone();
    let handle = std::thread::spawn(move || {
        let spec = block(&spec_perms, Some(EndReason::Interval));
        tracker_clone.snapshot(&spec, prov_cap_clone)
    });

    webcam_entered_rx.recv_timeout(std::time::Duration::from_secs(5)).expect("timeout waiting for webcam entry");

    // Isolated ABA transition: withdraw ONLY screenshot while keeping active=true and webcam=true
    let mut perms_webcam_only = perms;
    perms_webcam_only.screenshot = false;
    tracker.apply_sensor_policy(true, Some(&perms_webcam_only));
    assert!(tracker.is_tracking_active());

    // Regrant screenshot BEFORE releasing webcam
    tracker.apply_sensor_policy(true, Some(&perms));
    assert!(tracker.is_tracking_active());

    release_tx.send(()).unwrap();
    let activity = handle.join().expect("thread panicked");

    // Old screenshot from invalidated interval must be rejected
    assert!(activity.screenshot_png_base64.is_none(), "screenshot associated with revoked/ABA interval must be rejected at publication");
    // Continuously authorized webcam result must be present!
    assert!(activity.webcam_jpg_base64.is_some(), "continuously authorized webcam capture must be kept");
    assert_eq!(activity.webcam_jpg_base64.as_deref(), Some("/9j/valid_webcam_during_blocked_screen_aba"));

    // Assert exact provider call counts
    assert_eq!(acq.screen_calls(), 1, "exactly 1 screen call from earlier capture_screen");
    assert_eq!(acq.webcam_calls(), 1, "exactly 1 webcam call during snapshot");

    // Assert fresh screenshot under current generation succeeds
    acq.set_on_capture_webcam(|| {});
    let fresh_prov_cap = tracker.capture_screen(false);
    assert!(fresh_prov_cap.is_some(), "fresh capture_screen under current generation succeeds");
    let fresh_spec = block(&perms, Some(EndReason::Interval));
    let fresh_activity = tracker.snapshot(&fresh_spec, fresh_prov_cap);
    assert!(fresh_activity.screenshot_png_base64.is_some(), "fresh screenshot under current generation accepted");
    assert!(fresh_activity.webcam_jpg_base64.is_some(), "fresh webcam accepted");
    assert_eq!(acq.screen_calls(), 2);
    assert_eq!(acq.webcam_calls(), 2);
}

#[test]
fn tracker_consent_screenshot_blur_tightening_during_blocked_webcam() {
    let (tracker, _driver, acq) = Tracker::new_fake_with_acquisition();
    let tracker = Arc::new(tracker);

    let mut perms = CapturePermissions::all_disabled();
    perms.screenshot = true;
    perms.blur_screenshots = false;
    perms.webcam = true;
    tracker.apply_sensor_policy(true, Some(&perms));

    // Take unblurred capture under initial policy
    let prov_cap = tracker.capture_screen(false);
    assert!(prov_cap.as_ref().is_some_and(|c| !c.capture.blurred));

    let (webcam_entered_tx, webcam_entered_rx) = std::sync::mpsc::channel();
    let (release_tx, release_rx) = std::sync::mpsc::channel();
    let release_rx = Arc::new(Mutex::new(release_rx));
    let rx_for_hook = release_rx.clone();

    acq.set_on_capture_webcam(move || {
        webcam_entered_tx.send(()).unwrap();
        rx_for_hook.lock().unwrap().recv_timeout(std::time::Duration::from_secs(5)).expect("timeout waiting for release");
    });

    let tracker_clone = tracker.clone();
    let spec_perms = perms;
    let prov_cap_clone = prov_cap.clone();
    let handle = std::thread::spawn(move || {
        let spec = block(&spec_perms, Some(EndReason::Interval));
        tracker_clone.snapshot(&spec, prov_cap_clone)
    });

    webcam_entered_rx.recv_timeout(std::time::Duration::from_secs(5)).expect("timeout waiting for webcam entry");

    // Tighten policy to require blur while webcam is blocked
    let mut tight_perms = perms;
    tight_perms.blur_screenshots = true;
    tracker.apply_sensor_policy(true, Some(&tight_perms));

    release_tx.send(()).unwrap();
    let activity = handle.join().expect("thread panicked");

    assert!(activity.screenshot_png_base64.is_none(), "unblurred screenshot must be dropped when policy blur requirement is tightened during snapshot execution");
}

#[test]
fn tracker_consent_unchanged_policy_republish_does_not_invalidate_in_flight_acquisition() {
    let (tracker, _driver, acq) = Tracker::new_fake_with_acquisition();
    let tracker = Arc::new(tracker);

    let mut perms = CapturePermissions::all_disabled();
    perms.screenshot = true;
    perms.active_window = true;
    perms.running_programs = true;
    perms.webcam = true;
    tracker.apply_sensor_policy(true, Some(&perms));

    let gen_initial = tracker.auth.read().unwrap().generations;

    // Take screen capture under initial generation
    let prov_cap = tracker.capture_screen(false);
    assert!(prov_cap.is_some());
    assert_eq!(prov_cap.as_ref().unwrap().generation, gen_initial.screen);

    // Block inside webcam acquisition during snapshot
    let (webcam_entered_tx, webcam_entered_rx) = std::sync::mpsc::channel();
    let (release_tx, release_rx) = std::sync::mpsc::channel();
    let release_rx = Arc::new(Mutex::new(release_rx));
    let rx_for_hook = release_rx.clone();

    acq.set_on_capture_webcam(move || {
        webcam_entered_tx.send(()).unwrap();
        rx_for_hook.lock().unwrap().recv_timeout(std::time::Duration::from_secs(5)).expect("timeout waiting for release");
    });

    let tracker_clone = tracker.clone();
    let spec_perms = perms;
    let prov_cap_clone = prov_cap.clone();
    let handle = std::thread::spawn(move || {
        let spec = block(&spec_perms, Some(EndReason::Interval));
        tracker_clone.snapshot(&spec, prov_cap_clone)
    });

    webcam_entered_rx.recv_timeout(std::time::Duration::from_secs(5)).expect("timeout waiting for webcam entry");

    // Republish IDENTICAL policy (simulating routine worker publish tick)
    tracker.apply_sensor_policy(true, Some(&perms));

    let gen_republished = tracker.auth.read().unwrap().generations;
    assert_eq!(gen_initial, gen_republished, "generations must NOT advance on identical policy republish");

    release_tx.send(()).unwrap();
    let activity = handle.join().expect("thread panicked");

    // In-flight acquisitions must NOT be falsely invalidated by unchanged policy republish
    assert!(activity.screenshot_png_base64.is_some(), "screenshot must be kept on unchanged policy republish");
    assert!(activity.webcam_jpg_base64.is_some(), "webcam must be kept on unchanged policy republish");
}
