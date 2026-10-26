//! `build_activity` port: turns measurements into an `agent::model::Activity`
//! with the honest-null rules (unmeasured counts are null).
//!
//! Ported from the Windows `tracking/mod.rs` (`BlockSpec`, `Measurements`,
//! `Zone`, `build_activity`, `with_device_context`, `clean_description`,
//! `split_span`, `kept_idle_blocks`, with their tests). Linux deltas
//! (design 7 and 8.2, cross-lane contract API-05):
//!
//! - `keyboardHits` is `Some` only when the keyboard is effective and input
//!   was measured for the whole block; `mouseClicks` likewise. Otherwise it
//!   is `null`, never a fabricated 0.
//! - Activity %, `idleSeconds`, the per-minute timeline and
//!   `injectedInputEvents` exist only when input was measured; a kind that was
//!   not measured stays `null` inside every timeline minute and every usage row.
//! - The screen capture travels next to the payload as bytes
//!   ([`BuiltBlock::screen`]); the payload only carries its time and blur flag.
//!   It is kept only when screenshots are effective and, when blur is
//!   required, blurred.

use chrono::{DateTime, TimeDelta, Utc};

use super::capture::ScreenCapture;
use super::input;
use super::model::WindowInfo;
use super::system::DeviceInfo;
use super::usage::{self, UsageSlice};
use crate::agent::model::{Activity, EndReason, MIN_BLOCK};
use crate::consent::ConsentedPermissions;
use crate::lifecycle::LifecycleGeneration;

/// The longest task note the server stores.
pub const MAX_DESCRIPTION_CHARS: usize = 2048;

/// A screen capture with the lifecycle generation it was taken in: it may be
/// attached to a block only while that generation is still current.
#[derive(Debug, Clone)]
pub struct ProvenanceScreenCapture {
    pub capture: ScreenCapture,
    pub generation: LifecycleGeneration,
}

/// One block to record: its project and effective permissions, its time
/// window, and why it ended.
#[derive(Debug, Clone, Copy)]
pub struct BlockSpec<'a> {
    pub project_id: &'a str,
    pub perms: &'a ConsentedPermissions,
    pub started_at: DateTime<Utc>,
    pub ended_at: DateTime<Utc>,
    pub end_reason: Option<EndReason>,
    /// The task note ("What are you working on?").
    pub description: Option<&'a str>,
}

impl BlockSpec<'_> {
    /// May the block take a capture now that it ends? Not when the session
    /// is locked, asleep, idle or shutting down.
    pub fn captures_at_end(&self) -> bool {
        self.end_reason.is_none_or(EndReason::allows_capture_at_end)
    }
}

/// What the tracker measured for a block — read from the sensors by the
/// tracker's snapshot, turned into the payload by [`build_activity`].
#[derive(Debug, Clone, Default)]
pub struct Measurements {
    /// Unix ms of key presses; `None` when the keyboard was not recorded.
    pub key_presses: Option<Vec<i64>>,
    /// Unix ms of clicks; `None` when the mouse was not recorded.
    pub clicks: Option<Vec<i64>>,
    /// Unix seconds with physical input of the recorded kinds.
    pub active_seconds: Vec<i64>,
    /// Injected (XTEST) events of the recorded kinds.
    pub injected: u64,
    /// The input listener delivered for the whole block.
    pub input_available: bool,
    /// Foreground usage (empty when the active window is not recorded).
    pub slices: Vec<UsageSlice>,
    /// The window in front at the end of the block.
    pub window_now: Option<WindowInfo>,
    pub running_programs: Vec<WindowInfo>,
    pub screen: Option<ScreenCapture>,
}

/// The device's time zone when the block started.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Zone {
    pub time_zone: Option<String>,
    pub utc_offset_minutes: i32,
}

/// A finished block: the payload, plus its screen capture as bytes (queued
/// in the outbox's capture table, never inside the payload).
#[derive(Debug, Clone)]
pub struct BuiltBlock {
    pub activity: Activity,
    pub screen: Option<ScreenCapture>,
}

/// Assemble the upload payload of a block (CONTRACT §1, design 8.2).
///
/// Input is "measured" only when the keyboard or the mouse is effective and
/// the listener worked for the whole block. Without that, `productivity`,
/// `idleSeconds`, `injectedInputEvents` and the timeline are left out and both
/// counts are `null`.
pub fn build_activity(
    block: &BlockSpec<'_>,
    measured: Measurements,
    device: &DeviceInfo,
    zone: Zone,
) -> BuiltBlock {
    let perms = block.perms;
    let (start, end) = (block.started_at.timestamp(), block.ended_at.timestamp());
    let input_measured = (perms.keyboard || perms.mouse) && measured.input_available;
    // Usage is attributed only when the active window is effective.
    let slices: &[UsageSlice] = if perms.active_window {
        &measured.slices
    } else {
        &[]
    };
    let keys = measured
        .key_presses
        .filter(|_| input_measured && perms.keyboard);
    let clicks = measured.clicks.filter(|_| input_measured && perms.mouse);

    let active_windows: Vec<WindowInfo> = if !perms.active_window {
        Vec::new()
    } else if slices.is_empty() {
        // Too short for a single sample: fall back to the window in front.
        measured.window_now.into_iter().collect()
    } else {
        let mut rows = usage::app_usage(
            slices,
            keys.as_deref().unwrap_or_default(),
            clicks.as_deref().unwrap_or_default(),
        );
        // A kind that was not measured has no per-window count either.
        for row in &mut rows {
            if keys.is_none() {
                row.keyboard_hits = None;
            }
            if clicks.is_none() {
                row.mouse_clicks = None;
            }
        }
        rows
    };
    let timeline = if input_measured {
        usage::minute_timeline(
            start,
            end,
            slices,
            keys.as_deref(),
            clicks.as_deref(),
            &measured.active_seconds,
        )
    } else {
        Vec::new()
    };

    // Only a capture the session allows, and blurred when it must be: the
    // server would drop an unblurred one anyway.
    let screen = measured
        .screen
        .filter(|_| perms.screenshot)
        .filter(|capture| capture.blurred || !perms.blur_screenshots);

    let activity = Activity {
        description: clean_description(block.description),
        productivity: input_measured
            .then(|| input::activity_percent(&measured.active_seconds, start, end)),
        keyboard_hits: keys.as_ref().map(|k| k.len() as u64),
        mouse_clicks: clicks.as_ref().map(|c| c.len() as u64),
        active_windows,
        running_programs: if perms.running_programs {
            measured.running_programs
        } else {
            Vec::new()
        },
        idle_seconds: input_measured.then(|| usage::idle_seconds(&timeline, start, end)),
        timeline,
        end_reason: block.end_reason,
        screenshot_captured_at: screen.as_ref().map(|capture| capture.captured_at),
        screenshot_blurred: screen.as_ref().map(|capture| capture.blurred),
        injected_input_events: input_measured
            .then(|| u32::try_from(measured.injected).unwrap_or(u32::MAX)),
        ..Activity::new(block.project_id, block.started_at, block.ended_at)
    };
    BuiltBlock {
        activity: with_device_context(activity, device, zone),
        screen,
    }
}

/// Stamp the device context (CONTRACT §1) on a block.
pub fn with_device_context(activity: Activity, device: &DeviceInfo, zone: Zone) -> Activity {
    let non_empty = |value: &str| (!value.trim().is_empty()).then(|| value.to_string());
    Activity {
        platform: non_empty(&device.platform),
        os_version: non_empty(&device.os_version),
        agent_version: non_empty(&device.agent_version),
        device_id: non_empty(&device.device_id),
        device_name: device.device_name.clone(),
        time_zone: zone.time_zone,
        utc_offset_minutes: Some(zone.utc_offset_minutes.clamp(-840, 840)),
        ..activity
    }
}

/// The task note as sent: trimmed, within the server's limit, `None` when blank.
pub fn clean_description(description: Option<&str>) -> Option<String> {
    let text: String = description?
        .trim()
        .chars()
        .take(MAX_DESCRIPTION_CHARS)
        .collect();
    let text = text.trim_end();
    (!text.is_empty()).then(|| text.to_string())
}

/// Split `from .. to` into back-to-back pieces of at most `max` each, as
/// equal as possible (no sliver at the end). Nothing when the span is shorter
/// than [`MIN_BLOCK`].
pub fn split_span(
    from: DateTime<Utc>,
    to: DateTime<Utc>,
    max: TimeDelta,
) -> Vec<(DateTime<Utc>, DateTime<Utc>)> {
    let total_ms = (to - from).num_milliseconds();
    let max_ms = max.num_milliseconds();
    if to - from < MIN_BLOCK || max_ms <= 0 {
        return Vec::new();
    }
    // `i64::div_ceil` is not stable; both are positive here.
    let pieces = (total_ms + max_ms - 1) / max_ms;
    let boundary = |i: i64| {
        if i == pieces {
            to
        } else {
            from + TimeDelta::milliseconds(total_ms * i / pieces)
        }
    };
    (0..pieces)
        .map(|i| (boundary(i), boundary(i + 1)))
        .collect()
}

/// The away span kept as work after the idle prompt (CONTRACT §4): blocks of
/// at most one interval, 0% active and idle for their whole length, ended by
/// `idle`, without windows, timeline or captures, carrying the task note.
/// The counts stay `null`: nobody measured input while the user was away.
pub fn kept_idle_blocks(
    project_id: &str,
    from: DateTime<Utc>,
    to: DateTime<Utc>,
    interval: TimeDelta,
    description: Option<&str>,
    device: &DeviceInfo,
    zone: impl Fn(DateTime<Utc>) -> Zone,
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
            with_device_context(activity, device, zone(start))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const T0: i64 = 1_790_000_000; // arbitrary Unix second
    const T0_MS: i64 = T0 * 1000;
    const PROJECT: &str = "3f2a9c1e-6b7d-4e8f-9a0b-1c2d3e4f5a6b";

    fn at(second: i64) -> DateTime<Utc> {
        DateTime::from_timestamp(second, 0).unwrap()
    }

    fn device() -> DeviceInfo {
        DeviceInfo {
            device_id: "5b0c1b9e-8f2c-4a43-9a51-0c7f6f2d9b11".into(),
            device_name: Some("work-laptop".into()),
            os_version: "Ubuntu 24.04.1 LTS (Linux 6.8.0-45-generic)".into(),
            agent_version: "9.9.9".into(),
            platform: "linux".into(),
            time_zone: Some("Asia/Dhaka".into()),
        }
    }

    fn zone() -> Zone {
        Zone {
            time_zone: Some("Asia/Dhaka".into()),
            utc_offset_minutes: 360,
        }
    }

    fn all_allowed() -> ConsentedPermissions {
        ConsentedPermissions {
            screenshot: true,
            webcam: false,
            keyboard: true,
            mouse: true,
            active_window: true,
            running_programs: true,
            blur_screenshots: false,
        }
    }

    fn slice(app: &str, from_s: i64, to_s: i64) -> UsageSlice {
        UsageSlice {
            app: app.into(),
            title: format!("{app} window"),
            start_ms: T0_MS + from_s * 1000,
            end_ms: T0_MS + to_s * 1000,
        }
    }

    /// Ten minutes of measured work: typing in minutes 0-5, a click in 7,
    /// nothing in 6, 8 and 9; three injected events.
    fn measurements() -> Measurements {
        let key_presses: Vec<i64> = (0..6).map(|m| T0_MS + m * 60_000 + 5_000).collect();
        let clicks = vec![T0_MS + 7 * 60_000 + 1_000];
        let mut active_seconds: Vec<i64> = key_presses.iter().map(|ms| ms / 1000).collect();
        active_seconds.extend(clicks.iter().map(|ms| ms / 1000));
        Measurements {
            key_presses: Some(key_presses),
            clicks: Some(clicks),
            active_seconds,
            injected: 3,
            input_available: true,
            slices: vec![slice("code", 0, 400), slice("firefox", 400, 600)],
            window_now: None,
            running_programs: vec![WindowInfo {
                app_name: "slack".into(),
                ..Default::default()
            }],
            screen: None,
        }
    }

    fn capture(blurred: bool) -> ScreenCapture {
        ScreenCapture {
            jpeg: vec![0xff, 0xd8, 0x01],
            thumb_jpeg: Some(vec![0xff, 0xd8, 0x02]),
            captured_at: at(T0 + 250),
            blurred,
        }
    }

    fn block(perms: &ConsentedPermissions, end_reason: Option<EndReason>) -> BlockSpec<'_> {
        BlockSpec {
            project_id: PROJECT,
            perms,
            started_at: at(T0),
            ended_at: at(T0 + 600),
            end_reason,
            description: Some("  Invoice export  "),
        }
    }

    fn built(perms: &ConsentedPermissions, measured: Measurements) -> BuiltBlock {
        build_activity(&block(perms, None), measured, &device(), zone())
    }

    #[test]
    fn a_measured_block_carries_every_contract_field() {
        let perms = all_allowed();
        let activity = build_activity(
            &block(&perms, Some(EndReason::Interval)),
            measurements(),
            &device(),
            zone(),
        )
        .activity;

        // Active in minutes 0-5 and 7: 7 of 10 buckets.
        assert_eq!(activity.productivity, Some(70));
        assert_eq!(activity.idle_seconds, Some(3 * 60));
        assert_eq!(activity.injected_input_events, Some(3));
        assert_eq!(
            (activity.keyboard_hits, activity.mouse_clicks),
            (Some(6), Some(1))
        );
        assert_eq!(activity.timeline.len(), 10);
        assert_eq!(activity.timeline[0].active_seconds, Some(1));
        assert_eq!(activity.timeline[0].keyboard_hits, Some(1));
        assert_eq!(activity.timeline[6].active_seconds, Some(0));
        assert!(!activity.timeline[6].active);
        assert_eq!(activity.active_windows[0].app_name, "code");
        assert_eq!(activity.running_programs.len(), 1);
        assert_eq!(activity.end_reason, Some(EndReason::Interval));
        assert_eq!(activity.description.as_deref(), Some("Invoice export"));

        // Device context.
        assert_eq!(activity.platform.as_deref(), Some("linux"));
        assert_eq!(activity.agent_version.as_deref(), Some("9.9.9"));
        assert_eq!(
            activity.device_id.as_deref(),
            Some("5b0c1b9e-8f2c-4a43-9a51-0c7f6f2d9b11")
        );
        assert_eq!(activity.device_name.as_deref(), Some("work-laptop"));
        assert_eq!(
            activity.os_version.as_deref(),
            Some("Ubuntu 24.04.1 LTS (Linux 6.8.0-45-generic)")
        );
        assert_eq!(activity.time_zone.as_deref(), Some("Asia/Dhaka"));
        assert_eq!(activity.utc_offset_minutes, Some(360));
        // Stamped at send time, not here.
        assert!(activity.sent_at.is_none() && activity.payload_signature.is_none());
        assert!(uuid::Uuid::parse_str(&activity.client_activity_id).is_ok());
    }

    #[test]
    fn activity_is_not_measured_when_no_input_kind_is_recorded() {
        let perms = ConsentedPermissions {
            keyboard: false,
            mouse: false,
            ..all_allowed()
        };
        let measured = Measurements {
            key_presses: None,
            clicks: None,
            active_seconds: Vec::new(),
            ..measurements()
        };
        let activity = built(&perms, measured).activity;
        assert_eq!(activity.productivity, None);
        assert_eq!(activity.idle_seconds, None);
        assert_eq!(activity.injected_input_events, None);
        assert_eq!(
            (activity.keyboard_hits, activity.mouse_clicks),
            (None, None)
        );
        assert!(
            activity.timeline.is_empty(),
            "all-inactive minutes would read as idle"
        );
        // Apps are still recorded, without invented counts.
        assert!(!activity.active_windows.is_empty());
        assert!(activity
            .active_windows
            .iter()
            .all(|w| w.keyboard_hits.is_none() && w.mouse_clicks.is_none()));
    }

    #[test]
    fn activity_is_not_measured_when_the_listener_failed_during_the_block() {
        let perms = all_allowed();
        let measured = Measurements {
            input_available: false,
            ..measurements()
        };
        let activity = built(&perms, measured).activity;
        assert_eq!(activity.productivity, None);
        assert_eq!(activity.idle_seconds, None);
        assert!(activity.timeline.is_empty());
        // Linux: what a half-working listener saw is not a count either.
        assert_eq!(
            (activity.keyboard_hits, activity.mouse_clicks),
            (None, None)
        );
        assert_eq!(activity.injected_input_events, None);
    }

    #[test]
    fn a_kind_that_is_not_effective_stays_null_while_the_other_is_counted() {
        let perms = ConsentedPermissions {
            keyboard: false,
            ..all_allowed()
        };
        let activity = built(&perms, measurements()).activity;
        assert_eq!(activity.keyboard_hits, None, "keyboard not recorded");
        assert_eq!(activity.mouse_clicks, Some(1));
        assert!(activity.productivity.is_some(), "mouse input was measured");
        assert!(activity
            .timeline
            .iter()
            .all(|m| m.keyboard_hits.is_none() && m.mouse_clicks.is_some()));
        assert!(activity
            .active_windows
            .iter()
            .all(|w| w.keyboard_hits.is_none() && w.mouse_clicks.is_some()));
        let json = serde_json::to_value(&activity).unwrap();
        assert!(json["keyboardHits"].is_null(), "sent as null: {json}");
        assert_eq!(json["mouseClicks"], 1);
    }

    #[test]
    fn a_wayland_session_block_omits_every_unmeasured_field() {
        // Wayland: input and the active window are unsupported; running
        // programs (app scopes) and portal screenshots remain.
        let perms = ConsentedPermissions {
            keyboard: false,
            mouse: false,
            active_window: false,
            ..all_allowed()
        };
        let measured = Measurements {
            key_presses: None,
            clicks: None,
            active_seconds: Vec::new(),
            injected: 0,
            input_available: false,
            slices: Vec::new(),
            ..measurements()
        };
        let json = serde_json::to_value(built(&perms, measured).activity).unwrap();
        for absent in [
            "productivity",
            "idleSeconds",
            "timeline",
            "injectedInputEvents",
            "screenshotPngBase64",
            "screenshotThumbJpgBase64",
            "webcamJpgBase64",
        ] {
            assert!(json.get(absent).is_none(), "{absent} sent: {json}");
        }
        assert!(json["keyboardHits"].is_null() && json["mouseClicks"].is_null());
        assert_eq!(json["activeWindows"], serde_json::json!([]));
        assert_eq!(json["runningPrograms"][0]["appName"], "slack");
    }

    #[test]
    fn the_screen_capture_follows_the_effective_policy() {
        let perms = all_allowed();
        let measured = Measurements {
            screen: Some(capture(false)),
            ..measurements()
        };
        let block = built(&perms, measured);
        assert_eq!(block.screen.as_ref().unwrap().jpeg, vec![0xff, 0xd8, 0x01]);
        assert_eq!(block.activity.screenshot_captured_at, Some(at(T0 + 250)));
        assert_eq!(block.activity.screenshot_blurred, Some(false));
        assert!(
            !block.activity.has_captures(),
            "the bytes travel next to the payload"
        );

        // Blur required: an unblurred capture is never sent.
        let blur = ConsentedPermissions {
            blur_screenshots: true,
            ..all_allowed()
        };
        let measured = Measurements {
            screen: Some(capture(false)),
            ..measurements()
        };
        let block = built(&blur, measured);
        assert!(block.screen.is_none());
        assert!(block.activity.screenshot_captured_at.is_none());
        assert!(block.activity.screenshot_blurred.is_none());
        let measured = Measurements {
            screen: Some(capture(true)),
            ..measurements()
        };
        assert_eq!(
            built(&blur, measured).activity.screenshot_blurred,
            Some(true)
        );

        // Screenshots not effective: dropped, whatever the caller passed.
        let none = ConsentedPermissions {
            screenshot: false,
            ..all_allowed()
        };
        let measured = Measurements {
            screen: Some(capture(true)),
            ..measurements()
        };
        let block = built(&none, measured);
        assert!(block.screen.is_none() && block.activity.screenshot_captured_at.is_none());
        assert!(block.activity.webcam_jpg_base64.is_none(), "never on Linux");
    }

    #[test]
    fn a_capture_taken_before_a_lock_is_kept() {
        // The random moment came before the lock; only a capture at the end
        // is forbidden.
        let perms = all_allowed();
        let spec = block(&perms, Some(EndReason::Lock));
        assert!(!spec.captures_at_end());
        let measured = Measurements {
            screen: Some(capture(false)),
            ..measurements()
        };
        let built = build_activity(&spec, measured, &device(), zone());
        assert!(built.screen.is_some());
        assert_eq!(built.activity.end_reason, Some(EndReason::Lock));
        assert!(block(&perms, None).captures_at_end());
        assert!(block(&perms, Some(EndReason::Pause)).captures_at_end());
        for reason in [EndReason::Sleep, EndReason::Idle, EndReason::Shutdown] {
            assert!(!block(&perms, Some(reason)).captures_at_end(), "{reason:?}");
        }
    }

    #[test]
    fn app_usage_follows_the_active_window_permission() {
        let perms = ConsentedPermissions {
            active_window: false,
            running_programs: false,
            ..all_allowed()
        };
        let activity = built(&perms, measurements()).activity;
        assert!(activity.active_windows.is_empty() && activity.running_programs.is_empty());
        assert!(activity.timeline.iter().all(|m| m.app_name.is_none()));

        // A block too short for a sample: the window in front stands in.
        let perms = all_allowed();
        let front = WindowInfo {
            app_name: "libreoffice".into(),
            window_title: "Q3".into(),
            ..Default::default()
        };
        let measured = Measurements {
            slices: Vec::new(),
            window_now: Some(front.clone()),
            ..measurements()
        };
        assert_eq!(built(&perms, measured).activity.active_windows, vec![front]);
    }

    #[test]
    fn task_notes_are_trimmed_and_bounded() {
        assert_eq!(clean_description(None), None);
        assert_eq!(clean_description(Some("   ")), None);
        assert_eq!(
            clean_description(Some(" Fix login ")).as_deref(),
            Some("Fix login")
        );
        let long = "x".repeat(5000);
        assert_eq!(
            clean_description(Some(&long)).map(|d| d.chars().count()),
            Some(MAX_DESCRIPTION_CHARS)
        );
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
        assert!(pieces
            .iter()
            .all(|(s, e)| *e - *s <= ten && *e - *s >= TimeDelta::minutes(8)));
        // Exactly one interval: one piece. Under a minute: nothing.
        assert_eq!(split_span(at(T0), at(T0 + 600), ten).len(), 1);
        assert!(split_span(at(T0), at(T0 + 59), ten).is_empty());
        assert!(split_span(at(T0), at(T0 + 600), TimeDelta::zero()).is_empty());
    }

    #[test]
    fn kept_idle_time_is_recorded_as_idle_blocks() {
        let blocks = kept_idle_blocks(
            PROJECT,
            at(T0),
            at(T0 + 25 * 60),
            TimeDelta::minutes(10),
            Some("Client call"),
            &device(),
            |_| zone(),
        );
        assert_eq!(blocks.len(), 3);
        for activity in &blocks {
            let length = (activity.ended_at - activity.started_at).num_seconds() as u32;
            assert!(length <= 600);
            assert_eq!(activity.productivity, Some(0));
            assert_eq!(activity.idle_seconds, Some(length));
            assert_eq!(activity.end_reason, Some(EndReason::Idle));
            assert!(
                activity.active_windows.is_empty()
                    && activity.timeline.is_empty()
                    && !activity.has_captures()
            );
            assert_eq!(
                (activity.keyboard_hits, activity.mouse_clicks),
                (None, None),
                "nobody measured input while the user was away"
            );
            assert_eq!(activity.description.as_deref(), Some("Client call"));
            assert_eq!(activity.time_zone.as_deref(), Some("Asia/Dhaka"));
            assert_eq!(
                activity.device_id.as_deref(),
                Some(device().device_id.as_str())
            );
            assert!(activity.utc_offset_minutes.is_some());
        }
        assert_eq!(blocks[0].started_at, at(T0));
        assert_eq!(blocks[2].ended_at, at(T0 + 25 * 60));
        // Distinct blocks, distinct idempotency keys.
        assert_ne!(blocks[0].client_activity_id, blocks[1].client_activity_id);
        assert!(kept_idle_blocks(
            PROJECT,
            at(T0),
            at(T0 + 30),
            TimeDelta::minutes(10),
            None,
            &device(),
            |_| zone()
        )
        .is_empty());
    }

    #[test]
    fn an_empty_device_field_is_left_out_rather_than_sent_blank() {
        let mut blank = device();
        blank.device_id = String::new();
        blank.os_version = "  ".into();
        let activity = with_device_context(
            Activity::new(PROJECT, at(T0), at(T0 + 600)),
            &blank,
            Zone {
                time_zone: None,
                utc_offset_minutes: 9999,
            },
        );
        assert_eq!(activity.device_id, None);
        assert_eq!(activity.os_version, None);
        assert_eq!(activity.utc_offset_minutes, Some(840), "clamped");
    }
}
