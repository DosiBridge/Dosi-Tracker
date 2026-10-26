use chrono::{DateTime, Utc};
use serde::{Deserialize, Deserializer, Serialize};

/// The server's default idle threshold, used when it does not send one.
pub const DEFAULT_IDLE_THRESHOLD_MINUTES: u32 = 5;

/// Longest idle threshold a project can set.
pub const MAX_IDLE_THRESHOLD_MINUTES: u32 = 60;

/// A project the current user may track against, with server-side permissions.
///
/// The policy fields added later (`idleThresholdMinutes`, `blurScreenshots`)
/// fall back to the server's defaults, so this agent keeps working against a
/// server that does not send them yet.
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
    /// Minutes without input before the block is cut as idle; 0 turns idle
    /// detection off.
    #[serde(default = "default_idle_threshold_minutes", deserialize_with = "idle_threshold_or_default")]
    pub idle_threshold_minutes: u32,
    /// Screen captures must be blurred (text unreadable) before upload.
    #[serde(default, deserialize_with = "false_if_null")]
    pub blur_screenshots: bool,
}

impl Project {
    /// How long without input counts as idle, or `None` when the project
    /// turned idle detection off. Out-of-range values are clamped.
    #[cfg_attr(not(test), expect(dead_code, reason = "read by the worker's idle detection"))]
    pub fn idle_threshold(&self) -> Option<std::time::Duration> {
        let minutes = self.idle_threshold_minutes.min(MAX_IDLE_THRESHOLD_MINUTES);
        (minutes > 0).then(|| std::time::Duration::from_secs(u64::from(minutes) * 60))
    }
}

fn default_idle_threshold_minutes() -> u32 {
    DEFAULT_IDLE_THRESHOLD_MINUTES
}

/// A `null` threshold means the server did not set one: use its default.
fn idle_threshold_or_default<'de, D: Deserializer<'de>>(deserializer: D) -> Result<u32, D::Error> {
    Ok(Option::<u32>::deserialize(deserializer)?.unwrap_or(DEFAULT_IDLE_THRESHOLD_MINUTES))
}

fn false_if_null<'de, D: Deserializer<'de>>(deserializer: D) -> Result<bool, D::Error> {
    Ok(Option::<bool>::deserialize(deserializer)?.unwrap_or(false))
}

/// Why a block ended (`endReason`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum EndReason {
    /// The interval ran out.
    Interval,
    Pause,
    /// A timed break.
    Break,
    /// No input for the project's idle threshold.
    Idle,
    /// The session was locked.
    Lock,
    /// The machine went to sleep (or a missed suspend was detected).
    Sleep,
    /// The user switched project.
    Project,
    /// The user changed the task note.
    Task,
    SignOut,
    Quit,
    /// Windows is shutting down or the user is signing off.
    Shutdown,
}

impl EndReason {
    const ALL: [EndReason; 11] = [
        EndReason::Interval,
        EndReason::Pause,
        EndReason::Break,
        EndReason::Idle,
        EndReason::Lock,
        EndReason::Sleep,
        EndReason::Project,
        EndReason::Task,
        EndReason::SignOut,
        EndReason::Quit,
        EndReason::Shutdown,
    ];

    /// The wire name, e.g. `"signout"`.
    pub fn as_str(self) -> &'static str {
        match self {
            EndReason::Interval => "interval",
            EndReason::Pause => "pause",
            EndReason::Break => "break",
            EndReason::Idle => "idle",
            EndReason::Lock => "lock",
            EndReason::Sleep => "sleep",
            EndReason::Project => "project",
            EndReason::Task => "task",
            EndReason::SignOut => "signout",
            EndReason::Quit => "quit",
            EndReason::Shutdown => "shutdown",
        }
    }

    /// Parse a wire name; anything unknown is `None` (as on the server).
    pub fn parse(value: &str) -> Option<EndReason> {
        Self::ALL.into_iter().find(|reason| reason.as_str().eq_ignore_ascii_case(value.trim()))
    }

    /// May a capture be taken at the end of a block that ends this way? A
    /// locked, sleeping, idle or shutting-down machine is never captured.
    pub fn allows_capture_at_end(self) -> bool {
        !matches!(self, EndReason::Lock | EndReason::Sleep | EndReason::Idle | EndReason::Shutdown)
    }
}

/// Read an end reason leniently: an unknown value (e.g. queued by a newer
/// build) becomes `None` instead of making the whole row unreadable.
fn lenient_end_reason<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Option<EndReason>, D::Error> {
    Ok(Option::<String>::deserialize(deserializer)?.as_deref().and_then(EndReason::parse))
}

/// One tracked time block that gets uploaded to the backend
/// (`POST /api/app/activity`, CONTRACT §1).
///
/// Every field added after the first release is optional and left out of the
/// JSON when unset, so an older server (which does not know it) keeps
/// accepting the block, and defaulted on load, so rows queued by older builds
/// still parse.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Activity {
    pub client_activity_id: String,
    pub project_id: String,
    pub started_at: DateTime<Utc>,
    pub ended_at: DateTime<Utc>,
    /// The task note ("What are you working on?").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Activity % (0-100): share of the block's minutes that had keyboard or
    /// mouse input. `None` = not measured (input capture unavailable, or the
    /// project records neither keyboard nor mouse); left out of the JSON, so
    /// an older server stores its default instead of rejecting a null.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub productivity: Option<u8>,
    /// Clicks / key presses in the block. `None` (sent as `null`) = not
    /// measured: the project or the local consent does not record that kind
    /// of input, or capture was unavailable for part of the block. `0` only
    /// when measured as zero (CONTRACT: nullable counts).
    #[serde(default)]
    pub mouse_clicks: Option<u64>,
    #[serde(default)]
    pub keyboard_hits: Option<u64>,
    /// Foreground usage over the whole block, one entry per (app, window
    /// title), most-used first.
    pub active_windows: Vec<WindowInfo>,
    /// Programs with a window open, by name only (never their titles).
    pub running_programs: Vec<WindowInfo>,
    /// Per-minute breakdown of the block. Defaulted so rows queued by older
    /// builds still load.
    #[serde(default)]
    pub timeline: Vec<ActivityMinute>,
    /// The screen capture, base64. JPEG (older builds queued PNG); the server
    /// detects the format, the name is kept for compatibility.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub screenshot_png_base64: Option<String>,
    /// Base64-encoded small JPEG of the same screenshot, for list thumbnails.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub screenshot_thumb_jpg_base64: Option<String>,
    /// Base64-encoded JPEG, if webcam is allowed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub webcam_jpg_base64: Option<String>,

    /// `"windows"`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub platform: Option<String>,
    /// e.g. `"Windows 11 Pro 23H2 (10.0.22631)"`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub os_version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_version: Option<String>,
    /// Random UUID generated once per install.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub device_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub device_name: Option<String>,
    /// IANA id of the device's time zone, e.g. `"Asia/Dhaka"`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub time_zone: Option<String>,
    /// The device's UTC offset at `started_at`, in minutes (+360 for Dhaka).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub utc_offset_minutes: Option<i32>,
    /// Seconds of the block without input, at minute granularity (the
    /// lengths of the timeline's inactive buckets). `None` = not measured.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub idle_seconds: Option<u32>,
    #[serde(default, deserialize_with = "lenient_end_reason", skip_serializing_if = "Option::is_none")]
    pub end_reason: Option<EndReason>,
    /// The device clock when the upload was sent; set at send time, never
    /// stored in the queue.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sent_at: Option<DateTime<Utc>>,
    /// When the screen capture was taken (a random moment inside the block).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub screenshot_captured_at: Option<DateTime<Utc>>,
    /// The screen capture and its thumbnail were blurred before upload.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub screenshot_blurred: Option<bool>,
    /// Software-injected keyboard/mouse events seen during the block (mouse
    /// jigglers, remote tools). They are excluded from the counts and the
    /// activity %.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub injected_input_events: Option<u32>,
    /// HMAC-SHA256 of the payload bytes as queued locally (base64url, no pad),
    /// with a key that never leaves this computer ([`crate::signing`]). The
    /// server does **not** verify it (it has no copy of the key), so it
    /// proves nothing about the block. Absent on rows queued before signing
    /// was introduced or in tests that construct Activity values directly.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub payload_signature: Option<String>,
}

impl Activity {
    /// A block with no input, windows or captures yet — the base the tracker
    /// fills in.
    pub fn new(project_id: &str, started_at: DateTime<Utc>, ended_at: DateTime<Utc>) -> Self {
        Self {
            client_activity_id: uuid::Uuid::new_v4().to_string(),
            project_id: project_id.to_string(),
            started_at,
            ended_at,
            description: None,
            productivity: None,
            mouse_clicks: None,
            keyboard_hits: None,
            active_windows: Vec::new(),
            running_programs: Vec::new(),
            timeline: Vec::new(),
            screenshot_png_base64: None,
            screenshot_thumb_jpg_base64: None,
            webcam_jpg_base64: None,
            platform: None,
            os_version: None,
            agent_version: None,
            device_id: None,
            device_name: None,
            time_zone: None,
            utc_offset_minutes: None,
            idle_seconds: None,
            end_reason: None,
            sent_at: None,
            screenshot_captured_at: None,
            screenshot_blurred: None,
            injected_input_events: None,
            payload_signature: None,
        }
    }

    /// Does this block still carry any image?
    #[cfg_attr(not(test), expect(dead_code, reason = "for the worker's upload handling"))]
    pub fn has_captures(&self) -> bool {
        self.screenshot_png_base64.is_some() || self.screenshot_thumb_jpg_base64.is_some() || self.webcam_jpg_base64.is_some()
    }
}

/// A window (app + title). For foreground usage it also carries how long it
/// was in front during the block and the input made while it was; running
/// programs leave those out.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WindowInfo {
    pub app_name: String,
    pub window_title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seconds: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub keyboard_hits: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mouse_clicks: Option<u64>,
}

/// One minute of a block: input counts, whether there was any activity, and
/// the app mostly in front. Buckets match the activity % minutes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ActivityMinute {
    /// 0-based minute of the block.
    pub minute: u32,
    /// `None` (`null`) when that kind of input is not measured.
    #[serde(default)]
    pub keyboard_hits: Option<u64>,
    #[serde(default)]
    pub mouse_clicks: Option<u64>,
    pub active: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub app_name: Option<String>,
    /// Seconds of the bucket with any input (0..=90: the last bucket can run
    /// up to 89 s).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_seconds: Option<u32>,
}

/// What the agent is doing, for the live "who is working now" view.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
#[cfg_attr(not(test), expect(dead_code, reason = "sent by the worker's presence loop"))]
pub enum PresenceStatus {
    Tracking,
    Paused,
    /// A timed break.
    Break,
    Idle,
    Locked,
    /// Signed in, but no project is chosen.
    Choosing,
    SignedOut,
    /// The agent is quitting.
    Stopped,
}

/// `POST /api/app/presence/heartbeat` (CONTRACT §3). Sent every 60 s while
/// signed in and on every status change; never queued.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(not(test), expect(dead_code, reason = "sent by the worker's presence loop"))]
pub struct Heartbeat {
    pub device_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub device_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub platform: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub os_version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_version: Option<String>,
    pub status: PresenceStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status_since: Option<DateTime<Utc>>,
    /// The app in front — only when the project records the active window.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub current_app: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub time_zone: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub utc_offset_minutes: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sent_at: Option<DateTime<Utc>>,
}

impl Heartbeat {
    /// Trim the text fields and cut them to the server's limits, so a long
    /// computer or app name never turns a heartbeat into a validation error.
    /// Blank values are dropped.
    pub fn normalized(mut self) -> Self {
        fn fit(value: Option<String>, max_chars: usize) -> Option<String> {
            value
                .map(|v| v.trim().chars().take(max_chars).collect::<String>())
                .map(|v| v.trim_end().to_string())
                .filter(|v| !v.is_empty())
        }
        self.device_name = fit(self.device_name, 128);
        self.platform = fit(self.platform, 16);
        self.os_version = fit(self.os_version, 64);
        self.agent_version = fit(self.agent_version, 32);
        self.current_app = fit(self.current_app, 256);
        self.time_zone = fit(self.time_zone, 64);
        self.utc_offset_minutes = self.utc_offset_minutes.map(|m| m.clamp(-840, 840));
        self
    }
}

/// `HeartbeatResultDto`.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(not(test), expect(dead_code, reason = "sent by the worker's presence loop"))]
pub struct HeartbeatResult {
    #[serde(default, deserialize_with = "lenient_utc")]
    pub server_time: Option<DateTime<Utc>>,
}

/// A server timestamp, with or without an offset (a bare one is UTC); an
/// unparseable value is `None` rather than an error.
#[cfg_attr(not(test), expect(dead_code, reason = "reached through HeartbeatResult's derived Deserialize"))]
fn lenient_utc<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Option<DateTime<Utc>>, D::Error> {
    Ok(Option::<String>::deserialize(deserializer)?.as_deref().and_then(parse_server_time))
}

/// Parse an ISO-8601 server timestamp; one without an offset is UTC.
pub fn parse_server_time(value: &str) -> Option<DateTime<Utc>> {
    let value = value.trim();
    DateTime::parse_from_rfc3339(value)
        .map(|t| t.with_timezone(&Utc))
        .ok()
        .or_else(|| {
            chrono::NaiveDateTime::parse_from_str(value, "%Y-%m-%dT%H:%M:%S%.f")
                .ok()
                .map(|naive| naive.and_utc())
        })
}

/// ABP's error response body: `{"error":{"code":…,"message":…,…}}`.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AbpErrorResponse {
    #[serde(default)]
    pub error: Option<AbpError>,
}

/// ABP's `RemoteServiceErrorInfo`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AbpError {
    /// The business error code, e.g. `"Tracker:ActivityOverlaps"`. Validation
    /// failures have none.
    #[serde(default)]
    pub code: Option<String>,
    #[serde(default)]
    pub message: Option<String>,
    #[serde(default)]
    pub details: Option<String>,
    #[serde(default)]
    pub validation_errors: Option<Vec<AbpValidationError>>,
}

impl AbpError {
    /// Parse an error body; `None` when it is not ABP's shape.
    pub fn from_body(body: &str) -> Option<AbpError> {
        serde_json::from_str::<AbpErrorResponse>(body).ok()?.error
    }

    /// The business error code, if the server sent a non-blank one.
    pub fn code(&self) -> Option<&str> {
        self.code.as_deref().map(str::trim).filter(|code| !code.is_empty())
    }

    pub fn has_validation_errors(&self) -> bool {
        self.validation_errors.as_ref().is_some_and(|errors| !errors.is_empty())
    }

    /// One line for the user: the message plus each validation error.
    pub fn summary(&self) -> Option<String> {
        let mut parts: Vec<String> = Vec::new();
        if let Some(message) = self.message.as_deref().map(str::trim).filter(|m| !m.is_empty()) {
            parts.push(message.to_string());
        }
        for error in self.validation_errors.iter().flatten() {
            if let Some(message) = error.message.as_deref().map(str::trim).filter(|m| !m.is_empty()) {
                parts.push(message.to_string());
            }
        }
        (!parts.is_empty()).then(|| parts.join(" "))
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AbpValidationError {
    #[serde(default)]
    pub message: Option<String>,
    #[serde(default)]
    pub members: Option<Vec<String>>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn block() -> Activity {
        let now = Utc::now();
        Activity::new("p", now, now)
    }

    #[test]
    fn productivity_is_sent_as_a_camel_case_integer() {
        let activity = Activity { productivity: Some(73), ..block() };
        let json = serde_json::to_value(&activity).unwrap();
        assert_eq!(json["productivity"], 73);
    }

    #[test]
    fn unset_optional_fields_are_left_out_for_older_servers() {
        // An older server binds `productivity` to a non-nullable int: a null
        // would fail the whole upload, an absent field takes its default.
        let json = serde_json::to_value(block()).unwrap();
        for absent in [
            "productivity",
            "description",
            "screenshotPngBase64",
            "screenshotThumbJpgBase64",
            "webcamJpgBase64",
            "platform",
            "osVersion",
            "agentVersion",
            "deviceId",
            "deviceName",
            "timeZone",
            "utcOffsetMinutes",
            "idleSeconds",
            "endReason",
            "sentAt",
            "screenshotCapturedAt",
            "screenshotBlurred",
            "injectedInputEvents",
        ] {
            assert!(json.get(absent).is_none(), "{absent} should be left out");
        }
        // The fields every server requires are always there.
        for present in ["clientActivityId", "projectId", "startedAt", "endedAt", "mouseClicks", "keyboardHits", "activeWindows", "runningPrograms", "timeline"] {
            assert!(json.get(present).is_some(), "{present} should be sent");
        }
    }

    #[test]
    fn unmeasured_counts_are_sent_as_null_and_measured_zeros_as_zero() {
        // Never a fabricated zero: not measured is null.
        let json = serde_json::to_value(block()).unwrap();
        assert!(json["mouseClicks"].is_null() && json["keyboardHits"].is_null(), "{json}");
        let measured = Activity { keyboard_hits: Some(0), mouse_clicks: Some(7), ..block() };
        let json = serde_json::to_value(&measured).unwrap();
        assert_eq!((json["keyboardHits"].as_u64(), json["mouseClicks"].as_u64()), (Some(0), Some(7)));
        // And they survive the local queue as they were.
        let back: Activity = serde_json::from_value(json).unwrap();
        assert_eq!((back.keyboard_hits, back.mouse_clicks), (Some(0), Some(7)));

        let minute = ActivityMinute { minute: 0, keyboard_hits: Some(4), mouse_clicks: None, active: true, app_name: None, active_seconds: None };
        let json = serde_json::to_value(&minute).unwrap();
        assert_eq!(json["keyboardHits"], 4);
        assert!(json["mouseClicks"].is_null());
    }

    #[test]
    fn new_fields_use_the_contract_names() {
        let at = DateTime::parse_from_rfc3339("2026-10-05T10:00:00Z").unwrap().with_timezone(&Utc);
        let activity = Activity {
            platform: Some("windows".into()),
            os_version: Some("Windows 11 Pro 23H2 (10.0.22631)".into()),
            agent_version: Some("0.2.0".into()),
            device_id: Some("5b0c1b9e-8f2c-4a43-9a51-0c7f6f2d9b11".into()),
            device_name: Some("DESKTOP-1".into()),
            time_zone: Some("Asia/Dhaka".into()),
            utc_offset_minutes: Some(360),
            idle_seconds: Some(120),
            end_reason: Some(EndReason::SignOut),
            sent_at: Some(at),
            screenshot_captured_at: Some(at),
            screenshot_blurred: Some(true),
            injected_input_events: Some(4),
            ..block()
        };
        let json = serde_json::to_value(&activity).unwrap();
        assert_eq!(json["platform"], "windows");
        assert_eq!(json["osVersion"], "Windows 11 Pro 23H2 (10.0.22631)");
        assert_eq!(json["agentVersion"], "0.2.0");
        assert_eq!(json["deviceId"], "5b0c1b9e-8f2c-4a43-9a51-0c7f6f2d9b11");
        assert_eq!(json["deviceName"], "DESKTOP-1");
        assert_eq!(json["timeZone"], "Asia/Dhaka");
        assert_eq!(json["utcOffsetMinutes"], 360);
        assert_eq!(json["idleSeconds"], 120);
        assert_eq!(json["endReason"], "signout");
        assert_eq!(json["sentAt"], "2026-10-05T10:00:00Z");
        assert_eq!(json["screenshotCapturedAt"], "2026-10-05T10:00:00Z");
        assert_eq!(json["screenshotBlurred"], true);
        assert_eq!(json["injectedInputEvents"], 4);

        // And they survive the local queue.
        let back: Activity = serde_json::from_value(json).unwrap();
        assert_eq!(back.end_reason, Some(EndReason::SignOut));
        assert_eq!(back.utc_offset_minutes, Some(360));
    }

    #[test]
    fn end_reasons_round_trip_and_unknown_ones_are_dropped() {
        for reason in EndReason::ALL {
            assert_eq!(serde_json::to_value(reason).unwrap(), reason.as_str());
            assert_eq!(EndReason::parse(reason.as_str()), Some(reason));
        }
        assert_eq!(EndReason::parse(" Lock "), Some(EndReason::Lock));
        assert_eq!(EndReason::parse("hibernate"), None);

        let json = serde_json::to_value(block()).unwrap();
        let mut json = json.as_object().unwrap().clone();
        json.insert("endReason".into(), "teleported".into());
        let parsed: Activity = serde_json::from_value(json.into()).unwrap();
        assert_eq!(parsed.end_reason, None);
    }

    #[test]
    fn only_lock_sleep_idle_and_shutdown_forbid_a_capture_at_the_end() {
        let forbidden: Vec<_> = EndReason::ALL.into_iter().filter(|r| !r.allows_capture_at_end()).collect();
        assert_eq!(forbidden, [EndReason::Idle, EndReason::Lock, EndReason::Sleep, EndReason::Shutdown]);
    }

    #[test]
    fn window_usage_is_sent_in_camel_case_and_bare_windows_stay_bare() {
        let used = WindowInfo {
            app_name: "code.exe".into(),
            window_title: "main.rs".into(),
            seconds: Some(240),
            keyboard_hits: Some(310),
            mouse_clicks: Some(12),
        };
        let json = serde_json::to_value(&used).unwrap();
        assert_eq!(json["seconds"], 240);
        assert_eq!(json["keyboardHits"], 310);
        assert_eq!(json["mouseClicks"], 12);

        let bare = WindowInfo { app_name: "slack.exe".into(), ..Default::default() };
        let json = serde_json::to_value(&bare).unwrap();
        assert!(json.get("seconds").is_none());
        assert!(json.get("keyboardHits").is_none());
    }

    #[test]
    fn timeline_minutes_carry_active_seconds_only_when_known() {
        let minute = ActivityMinute {
            minute: 0,
            keyboard_hits: Some(3),
            mouse_clicks: Some(1),
            active: true,
            app_name: None,
            active_seconds: Some(42),
        };
        assert_eq!(serde_json::to_value(&minute).unwrap()["activeSeconds"], 42);
        let legacy = ActivityMinute { active_seconds: None, ..minute };
        assert!(serde_json::to_value(&legacy).unwrap().get("activeSeconds").is_none());
    }

    #[test]
    fn payloads_queued_before_productivity_existed_still_load() {
        let legacy = r#"{"clientActivityId":"id","projectId":"p",
            "startedAt":"2026-01-01T00:00:00Z","endedAt":"2026-01-01T00:10:00Z",
            "description":null,"mouseClicks":1,"keyboardHits":2,
            "activeWindows":[],"runningPrograms":[],
            "screenshotPngBase64":null,"webcamJpgBase64":null}"#;
        let activity: Activity = serde_json::from_str(legacy).unwrap();
        // Never measured: not "0% active".
        assert_eq!(activity.productivity, None);
        assert!(activity.timeline.is_empty());
        assert!(activity.screenshot_thumb_jpg_base64.is_none());
        assert!(activity.device_id.is_none() && activity.end_reason.is_none());
    }

    #[test]
    fn payloads_queued_by_the_previous_build_keep_their_measurements() {
        let previous = r#"{"clientActivityId":"id","projectId":"p",
            "startedAt":"2026-01-01T00:00:00Z","endedAt":"2026-01-01T00:10:00Z",
            "description":null,"productivity":64,"mouseClicks":1,"keyboardHits":2,
            "activeWindows":[],"runningPrograms":[],
            "timeline":[{"minute":0,"keyboardHits":2,"mouseClicks":1,"active":true}],
            "screenshotPngBase64":"iVBORw0KGgo=","screenshotThumbJpgBase64":null,"webcamJpgBase64":null}"#;
        let activity: Activity = serde_json::from_str(previous).unwrap();
        assert_eq!(activity.productivity, Some(64));
        assert_eq!(activity.timeline[0].active_seconds, None);
        assert!(activity.has_captures());
    }

    #[test]
    fn project_policy_fields_default_like_the_server() {
        let older_server = r#"{"id":"p","title":"Website","intervalMinutes":10,
            "allowScreenshot":true,"allowWebcam":false,"allowKeyboard":true,"allowMouse":true,
            "allowActiveWindow":true,"allowRunningPrograms":true}"#;
        let project: Project = serde_json::from_str(older_server).unwrap();
        assert_eq!(project.idle_threshold_minutes, 5, "not 0, which would turn idle detection off");
        assert!(!project.blur_screenshots);
        assert_eq!(project.idle_threshold(), Some(std::time::Duration::from_secs(300)));

        let nulls = older_server.replace("}", r#","idleThresholdMinutes":null,"blurScreenshots":null}"#);
        let project: Project = serde_json::from_str(&nulls).unwrap();
        assert_eq!(project.idle_threshold_minutes, 5);
        assert!(!project.blur_screenshots);

        let newer = older_server.replace("}", r#","idleThresholdMinutes":0,"blurScreenshots":true}"#);
        let project: Project = serde_json::from_str(&newer).unwrap();
        assert_eq!(project.idle_threshold(), None, "0 turns idle detection off");
        assert!(project.blur_screenshots);

        let project = Project { idle_threshold_minutes: 500, ..project };
        assert_eq!(project.idle_threshold(), Some(std::time::Duration::from_secs(3600)));
    }

    #[test]
    fn heartbeats_use_the_contract_shape_and_fit_the_server_limits() {
        let heartbeat = Heartbeat {
            device_id: "5b0c1b9e-8f2c-4a43-9a51-0c7f6f2d9b11".into(),
            device_name: Some(format!("  {}  ", "N".repeat(200))),
            platform: Some("windows".into()),
            os_version: None,
            agent_version: Some("0.2.0".into()),
            status: PresenceStatus::SignedOut,
            project_id: None,
            status_since: None,
            current_app: Some("   ".into()),
            time_zone: Some("Asia/Dhaka".into()),
            utc_offset_minutes: Some(9999),
            sent_at: None,
        }
        .normalized();
        assert_eq!(heartbeat.device_name.as_deref().map(str::len), Some(128));
        assert_eq!(heartbeat.current_app, None);
        assert_eq!(heartbeat.utc_offset_minutes, Some(840));

        let json = serde_json::to_value(&heartbeat).unwrap();
        assert_eq!(json["status"], "signedout");
        assert_eq!(json["deviceId"], "5b0c1b9e-8f2c-4a43-9a51-0c7f6f2d9b11");
        assert_eq!(json["timeZone"], "Asia/Dhaka");
        assert!(json.get("projectId").is_none() && json.get("osVersion").is_none());

        let statuses = [
            (PresenceStatus::Tracking, "tracking"),
            (PresenceStatus::Paused, "paused"),
            (PresenceStatus::Break, "break"),
            (PresenceStatus::Idle, "idle"),
            (PresenceStatus::Locked, "locked"),
            (PresenceStatus::Choosing, "choosing"),
            (PresenceStatus::Stopped, "stopped"),
        ];
        for (status, wire) in statuses {
            assert_eq!(serde_json::to_value(status).unwrap(), wire);
        }
    }

    #[test]
    fn heartbeat_results_accept_utc_and_unzoned_server_times() {
        let utc: HeartbeatResult = serde_json::from_str(r#"{"serverTime":"2026-10-05T10:00:00.123Z"}"#).unwrap();
        let bare: HeartbeatResult = serde_json::from_str(r#"{"serverTime":"2026-10-05T10:00:00.123"}"#).unwrap();
        assert_eq!(utc.server_time, bare.server_time);
        assert!(utc.server_time.is_some());
        let junk: HeartbeatResult = serde_json::from_str(r#"{"serverTime":"soon"}"#).unwrap();
        assert_eq!(junk.server_time, None);
        let empty: HeartbeatResult = serde_json::from_str("{}").unwrap();
        assert_eq!(empty.server_time, None);
    }

    #[test]
    fn abp_error_bodies_are_parsed() {
        let body = r#"{"error":{"code":"Tracker:ActivityOverlaps","message":"Overlaps another block.","details":null,"data":{},"validationErrors":null}}"#;
        let error = AbpError::from_body(body).unwrap();
        assert_eq!(error.code(), Some("Tracker:ActivityOverlaps"));
        assert!(!error.has_validation_errors());
        assert_eq!(error.summary().as_deref(), Some("Overlaps another block."));

        let validation = r#"{"error":{"code":null,"message":"Your request is not valid!",
            "validationErrors":[{"message":"The field Productivity must be between 0 and 100.","members":["productivity"]}]}}"#;
        let error = AbpError::from_body(validation).unwrap();
        assert_eq!(error.code(), None);
        assert!(error.has_validation_errors());
        assert_eq!(
            error.summary().as_deref(),
            Some("Your request is not valid! The field Productivity must be between 0 and 100.")
        );

        assert!(AbpError::from_body("<html>Bad gateway</html>").is_none());
        assert!(AbpError::from_body(r#"{"title":"Not Found","status":404}"#).is_none());
        assert_eq!(AbpError::from_body(r#"{"error":{"code":"  "}}"#).unwrap().code(), None);
    }
}
