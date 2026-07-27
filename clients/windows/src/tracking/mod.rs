//! Tracking subsystem.
//!
//! Design goal: keep idle CPU near zero. Input listening is event-driven
//! (the OS pushes events; we only increment atomic counters). Heavy work
//! (screenshot, webcam) happens once per interval and then the agent sleeps.

pub mod active_window;
pub mod input;
pub mod screenshot;
pub mod webcam;

use std::sync::Arc;

use chrono::Utc;

use crate::config::CapturePermissions;
use crate::model::{Activity, WindowInfo};
use input::InputCounter;

/// Collects one activity snapshot for the given project time block.
///
/// Capture permissions are passed per snapshot because the effective set
/// (server-side project settings ∧ local config) may only become known after
/// the first successful sync with the backend.
pub struct Tracker {
    input: Arc<InputCounter>,
}

impl Tracker {
    pub fn new() -> Self {
        let input = Arc::new(InputCounter::new());
        // Start the global input listener once; it runs on its own thread and
        // is fully event-driven, so it costs ~0% CPU while idle.
        input::spawn_listener(input.clone());
        Self { input }
    }

    /// Build an activity for the elapsed interval, then reset counters.
    pub fn snapshot(
        &self,
        perms: &CapturePermissions,
        project_id: &str,
        started_at: chrono::DateTime<Utc>,
    ) -> Activity {
        let (keyboard_hits, mouse_clicks) = self.input.take();

        let active_windows: Vec<WindowInfo> = if perms.active_window {
            active_window::current().into_iter().collect()
        } else {
            Vec::new()
        };

        let running_programs: Vec<WindowInfo> = if perms.running_programs {
            active_window::running_programs().unwrap_or_default()
        } else {
            Vec::new()
        };

        let screenshot_png_base64 = if perms.screenshot {
            screenshot::capture_primary_png_base64().ok()
        } else {
            None
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
            ended_at: Utc::now(),
            description: None,
            mouse_clicks: if perms.mouse { mouse_clicks } else { 0 },
            keyboard_hits: if perms.keyboard { keyboard_hits } else { 0 },
            active_windows,
            running_programs,
            screenshot_png_base64,
            webcam_jpg_base64,
        }
    }
}
