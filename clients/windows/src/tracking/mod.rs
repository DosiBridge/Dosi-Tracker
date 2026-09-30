//! Tracking subsystem.
//!
//! Design goal: keep idle CPU near zero. Input listening is event-driven
//! (the OS pushes events; we only record timestamps), the foreground window
//! is sampled every few seconds with a couple of Win32 calls, and heavy work
//! (screenshot, webcam) happens once per interval, then the agent sleeps.

pub mod active_window;
pub mod input;
pub mod screenshot;
pub mod usage;
pub mod webcam;

use std::sync::Arc;

use chrono::Utc;

use crate::config::CapturePermissions;
use crate::model::{Activity, WindowInfo};
use input::InputCounter;
use usage::UsageRecorder;

/// Collects one activity snapshot for the given project time block.
///
/// Capture permissions are passed per snapshot because the effective set
/// (server-side project settings ∧ local config) may only become known after
/// the first successful sync with the backend.
pub struct Tracker {
    input: Arc<InputCounter>,
    usage: Arc<UsageRecorder>,
}

impl Tracker {
    pub fn new() -> Self {
        let input = Arc::new(InputCounter::new());
        // Start the global input listener once; it runs on its own thread and
        // is fully event-driven, so it costs ~0% CPU while idle.
        input::spawn_listener(input.clone());
        let usage = Arc::new(UsageRecorder::new());
        usage::spawn_sampler(usage.clone());
        Self { input, usage }
    }

    /// Read-only access to input for the live view (counts and activity % of
    /// the block in progress).
    pub fn input(&self) -> Arc<InputCounter> {
        self.input.clone()
    }

    /// Build an activity for the block `started_at ..= ended_at`.
    ///
    /// Everything is taken over that window, so input and window usage from
    /// before the block (while paused, signed out, or on another project) can
    /// never be attributed to it.
    pub fn snapshot(
        &self,
        perms: &CapturePermissions,
        project_id: &str,
        started_at: chrono::DateTime<Utc>,
        ended_at: chrono::DateTime<Utc>,
    ) -> Activity {
        // Credit the stretch since the last sample, so usage runs right up to
        // the end of the block.
        let now_window = active_window::current();
        self.usage.record(ended_at.timestamp_millis(), now_window.clone());

        let key_presses = if perms.keyboard {
            self.input.key_presses_between(started_at, ended_at)
        } else {
            Vec::new()
        };
        let clicks = if perms.mouse {
            self.input.clicks_between(started_at, ended_at)
        } else {
            Vec::new()
        };
        let active_seconds =
            self.input
                .active_seconds(started_at, ended_at, perms.keyboard, perms.mouse);
        let productivity = input::activity_percent(
            &active_seconds,
            started_at.timestamp(),
            ended_at.timestamp(),
        );

        let slices = if perms.active_window {
            self.usage
                .slices_between(started_at.timestamp_millis(), ended_at.timestamp_millis())
        } else {
            Vec::new()
        };
        let active_windows: Vec<WindowInfo> = if !perms.active_window {
            Vec::new()
        } else if slices.is_empty() {
            // Too short for a single sample: fall back to the window in front.
            now_window.into_iter().collect()
        } else {
            usage::app_usage(&slices, &key_presses, &clicks)
        };
        let timeline = usage::minute_timeline(
            started_at.timestamp(),
            ended_at.timestamp(),
            &slices,
            &key_presses,
            &clicks,
            &active_seconds,
        );

        let running_programs: Vec<WindowInfo> = if perms.running_programs {
            active_window::running_programs().unwrap_or_default()
        } else {
            Vec::new()
        };

        let screen = if perms.screenshot {
            match screenshot::capture_primary() {
                Ok(capture) => Some(capture),
                Err(e) => {
                    tracing::warn!(?e, "screenshot capture failed");
                    None
                }
            }
        } else {
            None
        };
        let (screenshot_png_base64, screenshot_thumb_jpg_base64) = match screen {
            Some(capture) => (Some(capture.png_base64), capture.thumb_jpg_base64),
            None => (None, None),
        };

        let webcam_jpg_base64 = if perms.webcam {
            webcam::capture_jpg_base64().ok()
        } else {
            None
        };

        Activity {
            client_activity_id: uuid::Uuid::new_v4().to_string(),
            project_id: project_id.to_string(),
            started_at,
            ended_at,
            description: None,
            productivity,
            mouse_clicks: clicks.len() as u64,
            keyboard_hits: key_presses.len() as u64,
            active_windows,
            running_programs,
            timeline,
            screenshot_png_base64,
            screenshot_thumb_jpg_base64,
            webcam_jpg_base64,
        }
    }
}
