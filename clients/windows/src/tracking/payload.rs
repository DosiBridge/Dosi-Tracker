//! A block's upload payload (CONTRACT §1), from what the tracker measured,
//! and the time away kept as work.

use chrono::{DateTime, TimeDelta, Utc};

use crate::model::{Activity, EndReason, WindowInfo};

use super::input;
use super::screenshot::ScreenCapture;
use super::system::{self, DeviceInfo};
use super::usage::{self, UsageSlice};
use super::{BlockSpec, MIN_BLOCK};

/// The longest task note the server stores.
pub(super) const MAX_DESCRIPTION_CHARS: usize = 2048;

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
/// whole block. Each count (block, minute, window) of a kind of input that is
/// not measured is `null`, never a made-up 0.
pub fn build_activity(block: &BlockSpec<'_>, measured: Measurements, device: &DeviceInfo, zone: Zone) -> Activity {
    let perms = block.perms;
    let (start, end) = (block.started_at.timestamp(), block.ended_at.timestamp());
    let keyboard_measured = perms.keyboard && measured.input_available;
    let mouse_measured = perms.mouse && measured.input_available;
    let input_measured = keyboard_measured || mouse_measured;
    let only_measured = |keys: &mut Option<u64>, clicks: &mut Option<u64>| {
        if !keyboard_measured {
            *keys = None;
        }
        if !mouse_measured {
            *clicks = None;
        }
    };

    let mut active_windows: Vec<WindowInfo> = if !perms.active_window {
        Vec::new()
    } else if measured.slices.is_empty() {
        // Too short for a single sample: fall back to the window in front.
        measured.window_now.into_iter().collect()
    } else {
        usage::app_usage(&measured.slices, &measured.key_presses, &measured.clicks)
    };
    for window in &mut active_windows {
        only_measured(&mut window.keyboard_hits, &mut window.mouse_clicks);
    }
    let mut timeline = if input_measured {
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
    for minute in &mut timeline {
        only_measured(&mut minute.keyboard_hits, &mut minute.mouse_clicks);
    }

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
        mouse_clicks: mouse_measured.then_some(measured.clicks.len() as u64),
        keyboard_hits: keyboard_measured.then_some(measured.key_presses.len() as u64),
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
pub(super) fn with_device_context(activity: Activity, device: &DeviceInfo, zone: Zone) -> Activity {
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
pub(super) fn clean_description(description: Option<&str>) -> Option<String> {
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
