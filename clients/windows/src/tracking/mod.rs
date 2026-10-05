//! Tracking subsystem.
//!
//! Design goal: keep idle CPU near zero. Input listening is event-driven
//! (the OS pushes events; we only record timestamps), the foreground window
//! is sampled every few seconds with a couple of Win32 calls, and heavy work
//! (screenshot, webcam) happens once per block, then the agent sleeps.

pub mod active_window;
pub mod input;
pub mod screenshot;
pub mod system;
pub mod usage;
pub mod webcam;

use std::sync::Arc;

use chrono::{DateTime, TimeDelta, Utc};

use crate::config::CapturePermissions;
use crate::model::{Activity, EndReason, WindowInfo};
use input::InputCounter;
use screenshot::ScreenCapture;
use system::DeviceInfo;
use usage::{UsageRecorder, UsageSlice};

/// A block shorter than this is never recorded (CONTRACT §1).
pub const MIN_BLOCK: TimeDelta = TimeDelta::seconds(60);

/// The longest task note the server stores.
const MAX_DESCRIPTION_CHARS: usize = 2048;

/// One block to record: its project and capture policy, its time window, and
/// why it ended.
#[derive(Debug, Clone, Copy)]
pub struct BlockSpec<'a> {
    pub project_id: &'a str,
    pub perms: &'a CapturePermissions,
    pub started_at: DateTime<Utc>,
    pub ended_at: DateTime<Utc>,
    pub end_reason: Option<EndReason>,
    /// The task note ("What are you working on?").
    pub description: Option<&'a str>,
}

impl BlockSpec<'_> {
    /// May the block take a capture now that it ends? Not when the machine
    /// is locked, asleep, idle or shutting down.
    pub fn captures_at_end(&self) -> bool {
        self.end_reason.is_none_or(EndReason::allows_capture_at_end)
    }
}

/// Collects one activity snapshot for the given project time block.
///
/// Capture permissions are passed per snapshot because the effective set
/// (server-side project settings ∧ local config) may only become known after
/// the first successful sync with the backend.
pub struct Tracker {
    input: Arc<InputCounter>,
    usage: Arc<UsageRecorder>,
    device: DeviceInfo,
}

impl Tracker {
    pub fn new() -> Self {
        let input = Arc::new(InputCounter::new());
        // Start the global input hooks once; they run on their own thread and
        // are fully event-driven, so they cost ~0% CPU while idle.
        input::spawn_listener(input.clone());
        let usage = Arc::new(UsageRecorder::new());
        usage::spawn_sampler(usage.clone());
        Self { input, usage, device: DeviceInfo::collect() }
    }

    /// Read-only access to input for the live view (counts and activity % of
    /// the block in progress, whether input capture works).
    pub fn input(&self) -> Arc<InputCounter> {
        self.input.clone()
    }

    /// Read-only access to foreground-window usage, for the window's "top
    /// apps" list (shown only when the project records app usage).
    pub fn usage(&self) -> Arc<UsageRecorder> {
        self.usage.clone()
    }

    /// This device: install id, name, OS and agent version (also for the
    /// presence heartbeat).
    #[expect(dead_code, reason = "for the worker's presence heartbeat")]
    pub fn device(&self) -> &DeviceInfo {
        &self.device
    }

    /// Capture the screen now. The caller picks the moment — a random point
    /// inside the block — and hands the capture to [`Tracker::snapshot`] when
    /// the block ends. A failure is logged and gives `None`: a missing
    /// capture never costs the block.
    pub fn capture_screen(&self, blur: bool) -> Option<ScreenCapture> {
        match screenshot::capture_foreground_monitor(blur) {
            Ok(capture) => Some(capture),
            Err(e) => {
                tracing::warn!(?e, "screenshot capture failed");
                None
            }
        }
    }

    /// Build the activity for `block`, with `screen` taken earlier by
    /// [`Tracker::capture_screen`] (if any).
    ///
    /// Everything is taken over the block's window, so input and window usage
    /// from before it (while paused, signed out, or on another project) can
    /// never be attributed to it. A webcam photo is taken now, unless the
    /// block ends with the machine locked, asleep, idle or shutting down.
    pub fn snapshot(&self, block: &BlockSpec<'_>, screen: Option<ScreenCapture>) -> Activity {
        let perms = block.perms;
        let (started_at, ended_at) = (block.started_at, block.ended_at);

        // Credit the stretch since the last sample, so usage runs right up to
        // the end of the block.
        let window_now = active_window::current();
        self.usage.record(ended_at.timestamp_millis(), window_now.clone());

        let measured = Measurements {
            key_presses: if perms.keyboard { self.input.key_presses_between(started_at, ended_at) } else { Vec::new() },
            clicks: if perms.mouse { self.input.clicks_between(started_at, ended_at) } else { Vec::new() },
            active_seconds: self.input.active_seconds(started_at, ended_at, perms.keyboard, perms.mouse),
            injected: self.input.injected_between(started_at, ended_at, perms.keyboard, perms.mouse),
            input_available: self.input.capture_available_between(started_at, ended_at),
            slices: if perms.active_window {
                self.usage.slices_between(started_at.timestamp_millis(), ended_at.timestamp_millis())
            } else {
                Vec::new()
            },
            window_now,
            running_programs: if perms.running_programs {
                active_window::running_programs().unwrap_or_else(|e| {
                    tracing::warn!(?e, "could not list running programs");
                    Vec::new()
                })
            } else {
                Vec::new()
            },
            screen,
            webcam_jpg_base64: if perms.webcam && block.captures_at_end() {
                webcam::capture_jpg_base64()
                    .map_err(|e| tracing::warn!(?e, "webcam capture failed"))
                    .ok()
            } else {
                None
            },
        };
        let zone = Zone { time_zone: system::time_zone(), utc_offset_minutes: system::utc_offset_minutes(started_at) };
        build_activity(block, measured, &self.device, zone)
    }

    /// The away span kept as work after the idle prompt; see [`kept_idle_blocks`].
    #[expect(dead_code, reason = "called by the worker's idle prompt")]
    pub fn kept_idle_blocks(
        &self,
        project_id: &str,
        from: DateTime<Utc>,
        to: DateTime<Utc>,
        interval: std::time::Duration,
        description: Option<&str>,
    ) -> Vec<Activity> {
        let max = TimeDelta::from_std(interval).unwrap_or(TimeDelta::minutes(10));
        kept_idle_blocks(project_id, from, to, max, description, &self.device, system::time_zone().as_deref())
    }
}

/// What the tracker measured for a block — read from the OS by
/// [`Tracker::snapshot`], turned into the payload by [`build_activity`].
pub struct Measurements {
    /// Unix ms of key presses (empty when the keyboard is not recorded).
    pub key_presses: Vec<i64>,
    /// Unix ms of clicks (empty when the mouse is not recorded).
    pub clicks: Vec<i64>,
    /// Unix seconds with input of the recorded kinds.
    pub active_seconds: Vec<i64>,
    /// Injected events of the recorded kinds.
    pub injected: u64,
    /// The input hooks worked for the whole block.
    pub input_available: bool,
    /// Foreground usage (empty when the active window is not recorded).
    pub slices: Vec<UsageSlice>,
    /// The window in front at the end of the block.
    pub window_now: Option<WindowInfo>,
    pub running_programs: Vec<WindowInfo>,
    pub screen: Option<ScreenCapture>,
    pub webcam_jpg_base64: Option<String>,
}

/// The device's time zone when the block started.
pub struct Zone {
    pub time_zone: Option<String>,
    pub utc_offset_minutes: i32,
}

/// Assemble the upload payload of a block (CONTRACT §1).
///
/// Activity is "not measured" — `productivity`, `idleSeconds` and
/// `injectedInputEvents` left out, and no per-minute timeline (whose
/// `active: false` minutes would read as idle) — when the project records
/// neither keyboard nor mouse, or the input hooks were not working for the
/// whole block.
pub fn build_activity(block: &BlockSpec<'_>, measured: Measurements, device: &DeviceInfo, zone: Zone) -> Activity {
    let perms = block.perms;
    let (start, end) = (block.started_at.timestamp(), block.ended_at.timestamp());
    let input_measured = (perms.keyboard || perms.mouse) && measured.input_available;

    let active_windows: Vec<WindowInfo> = if !perms.active_window {
        Vec::new()
    } else if measured.slices.is_empty() {
        // Too short for a single sample: fall back to the window in front.
        measured.window_now.into_iter().collect()
    } else {
        usage::app_usage(&measured.slices, &measured.key_presses, &measured.clicks)
    };
    let timeline = if input_measured {
        usage::minute_timeline(
            start,
            end,
            &measured.slices,
            &measured.key_presses,
            &measured.clicks,
            &measured.active_seconds,
        )
    } else {
        Vec::new()
    };

    // Only a capture the project allows, and blurred when it must be: the
    // server would drop an unblurred one anyway.
    let screen = measured
        .screen
        .filter(|_| perms.screenshot)
        .filter(|capture| capture.blurred || !perms.blur_screenshots);
    let screenshot_captured_at = screen.as_ref().map(|capture| capture.captured_at);
    let screenshot_blurred = screen.as_ref().map(|capture| capture.blurred);
    let (screenshot, thumb) = match screen {
        Some(capture) => (Some(capture.jpeg_base64), capture.thumb_jpg_base64),
        None => (None, None),
    };

    let activity = Activity {
        description: clean_description(block.description),
        productivity: input_measured.then(|| input::activity_percent(&measured.active_seconds, start, end)),
        mouse_clicks: measured.clicks.len() as u64,
        keyboard_hits: measured.key_presses.len() as u64,
        active_windows,
        running_programs: measured.running_programs,
        idle_seconds: input_measured.then(|| usage::idle_seconds(&timeline, start, end)),
        timeline,
        screenshot_png_base64: screenshot,
        screenshot_thumb_jpg_base64: thumb,
        webcam_jpg_base64: measured.webcam_jpg_base64.filter(|_| perms.webcam),
        end_reason: block.end_reason,
        screenshot_captured_at,
        screenshot_blurred,
        injected_input_events: input_measured.then(|| u32::try_from(measured.injected).unwrap_or(u32::MAX)),
        ..Activity::new(block.project_id, block.started_at, block.ended_at)
    };
    with_device_context(activity, device, zone)
}

/// Stamp the device context (CONTRACT §1) on a block.
fn with_device_context(activity: Activity, device: &DeviceInfo, zone: Zone) -> Activity {
    Activity {
        platform: Some(device.platform.to_string()),
        os_version: device.os_version.clone(),
        agent_version: Some(device.agent_version.to_string()),
        device_id: device.device_id.clone(),
        device_name: device.device_name.clone(),
        time_zone: zone.time_zone,
        utc_offset_minutes: Some(zone.utc_offset_minutes.clamp(-840, 840)),
        ..activity
    }
}

/// The task note as sent: trimmed, within the server's limit, `None` when blank.
fn clean_description(description: Option<&str>) -> Option<String> {
    let text: String = description?.trim().chars().take(MAX_DESCRIPTION_CHARS).collect();
    let text = text.trim_end();
    (!text.is_empty()).then(|| text.to_string())
}

/// Split `from .. to` into back-to-back pieces of at most `max` each, as
/// equal as possible (no sliver at the end). Nothing when the span is shorter
/// than [`MIN_BLOCK`].
pub fn split_span(from: DateTime<Utc>, to: DateTime<Utc>, max: TimeDelta) -> Vec<(DateTime<Utc>, DateTime<Utc>)> {
    let total_ms = (to - from).num_milliseconds();
    let max_ms = max.num_milliseconds();
    if to - from < MIN_BLOCK || max_ms <= 0 {
        return Vec::new();
    }
    let pieces = (total_ms + max_ms - 1) / max_ms;
    let boundary = |i: i64| {
        if i == pieces {
            to
        } else {
            from + TimeDelta::milliseconds(total_ms * i / pieces)
        }
    };
    (0..pieces).map(|i| (boundary(i), boundary(i + 1))).collect()
}

/// The away span kept as work after the idle prompt (CONTRACT §4): blocks of
/// at most one interval, 0% active and idle for their whole length, ended by
/// `idle`, without windows, timeline or captures, carrying the task note.
pub fn kept_idle_blocks(
    project_id: &str,
    from: DateTime<Utc>,
    to: DateTime<Utc>,
    interval: TimeDelta,
    description: Option<&str>,
    device: &DeviceInfo,
    time_zone: Option<&str>,
) -> Vec<Activity> {
    split_span(from, to, interval)
        .into_iter()
        .map(|(start, end)| {
            let activity = Activity {
                description: clean_description(description),
                productivity: Some(0),
                idle_seconds: Some(u32::try_from((end - start).num_seconds()).unwrap_or(u32::MAX)),
                end_reason: Some(EndReason::Idle),
                ..Activity::new(project_id, start, end)
            };
            let zone = Zone {
                time_zone: time_zone.map(str::to_string),
                utc_offset_minutes: system::utc_offset_minutes(start),
            };
            with_device_context(activity, device, zone)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

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
        assert_eq!((activity.keyboard_hits, activity.mouse_clicks), (6, 1));
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
        // What was seen is still reported.
        assert_eq!(activity.keyboard_hits, 6);
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
}
