use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// A project the current user may track against, with server-side permissions.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
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
#[serde(rename_all = "camelCase")]
pub struct Activity {
    pub client_activity_id: String,
    pub project_id: String,
    pub started_at: DateTime<Utc>,
    pub ended_at: DateTime<Utc>,
    pub description: Option<String>,
    /// Activity % (0-100): share of the block's minutes that had keyboard or
    /// mouse input. Defaulted so rows queued by older builds still load.
    #[serde(default)]
    pub productivity: u8,
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
#[serde(rename_all = "camelCase")]
pub struct WindowInfo {
    pub app_name: String,
    pub window_title: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn productivity_is_sent_as_a_camel_case_integer() {
        let now = Utc::now();
        let activity = Activity {
            client_activity_id: "id".into(),
            project_id: "p".into(),
            started_at: now,
            ended_at: now,
            description: None,
            productivity: 73,
            mouse_clicks: 0,
            keyboard_hits: 0,
            active_windows: Vec::new(),
            running_programs: Vec::new(),
            screenshot_png_base64: None,
            webcam_jpg_base64: None,
        };
        let json = serde_json::to_value(&activity).unwrap();
        assert_eq!(json["productivity"], 73);
    }

    #[test]
    fn payloads_queued_before_productivity_existed_still_load() {
        let legacy = r#"{"clientActivityId":"id","projectId":"p",
            "startedAt":"2026-01-01T00:00:00Z","endedAt":"2026-01-01T00:10:00Z",
            "description":null,"mouseClicks":1,"keyboardHits":2,
            "activeWindows":[],"runningPrograms":[],
            "screenshotPngBase64":null,"webcamJpgBase64":null}"#;
        let activity: Activity = serde_json::from_str(legacy).unwrap();
        assert_eq!(activity.productivity, 0);
    }
}
