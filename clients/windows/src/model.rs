use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Credentials/token returned by the backend after login.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthSession {
    pub access_token: String,
    pub user_id: String,
    pub display_name: String,
    pub expires_at: DateTime<Utc>,
}

/// A project the current user may track against, with server-side permissions.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Project {
    pub id: String,
    pub title: String,
    pub interval_minutes: u64,
    pub allow_screenshot: bool,
    pub allow_webcam: bool,
    pub allow_keyboard: bool,
    pub allow_mouse: bool,
    pub allow_active_window: bool,
    pub allow_running_programs: bool,
}

/// One tracked time block that gets uploaded to the backend.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Activity {
    pub project_id: String,
    pub started_at: DateTime<Utc>,
    pub ended_at: DateTime<Utc>,
    pub description: Option<String>,
    pub mouse_clicks: u64,
    pub keyboard_hits: u64,
    pub active_windows: Vec<WindowInfo>,
    pub running_programs: Vec<WindowInfo>,
    /// Base64-encoded PNG, if screenshots are allowed.
    pub screenshot_png_base64: Option<String>,
    /// Base64-encoded JPEG, if webcam is allowed.
    pub webcam_jpg_base64: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WindowInfo {
    pub app_name: String,
    pub window_title: String,
}
