//! IPC protocol v1 between the daemon and its clients (CLI, GUI): newline-
//! delimited JSON over a same-uid Unix socket; envelope, methods, error codes
//! and the state snapshot (design 10).
//!
//! - One UTF-8 JSON object per line, at most [`MAX_FRAME_BYTES`]. A frame
//!   that grows past the cap is refused before it is parsed.
//! - `v` is the protocol major. Unknown fields are ignored, additions are
//!   additive; another major is answered with `unsupported_version` and the
//!   connection is closed.
//! - Requests carry an `id`; the response repeats it. Events (`state`,
//!   `show`) carry `event` and a sequence number instead.
//! - Error codes map onto the CLI's stable exit codes ([`ErrorCode::exit_code`]).
//! - The snapshot is the serde mirror of [`SharedState`], bounded
//!   ([`SharedState::bounded`]); it never carries a secret or image bytes.
//!
//! Frames that may carry a password (`auth.sign_in`) are decoded into
//! zeroizing buffers ([`FrameDecoder`]); nothing in this module logs a frame.

#[cfg(target_os = "linux")]
pub mod client;
#[cfg(target_os = "linux")]
pub mod server;

use std::time::Duration;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::Value;
use zeroize::{Zeroize, Zeroizing};

use crate::agent::state::{Secret, SharedState, WorkspaceChoice};
use crate::consent::LocalConsent;

/// The protocol major this build speaks.
pub const PROTOCOL_VERSION: u32 = 1;

/// The largest frame, newline excluded.
pub const MAX_FRAME_BYTES: usize = 1024 * 1024;

/// A frame that has started must be complete within this long.
pub const FRAME_TIMEOUT: Duration = Duration::from_secs(30);

/// At most this many clients at once.
pub const MAX_CONNECTIONS: usize = 16;

/// A connection that does not subscribe is closed after this long without a
/// request.
pub const IDLE_TIMEOUT: Duration = Duration::from_secs(5 * 60);

/// State events go out at most this often per subscriber (4/s).
pub const EVENT_MIN_INTERVAL: Duration = Duration::from_millis(250);

/// The user's action time (`at`) is clamped to `[now - 60 s, now + 2 s]`.
pub const AT_PAST_SLACK: chrono::TimeDelta = chrono::TimeDelta::seconds(60);
pub const AT_FUTURE_SLACK: chrono::TimeDelta = chrono::TimeDelta::seconds(2);

/// The longest sign-in wait (`auth.sign_in`).
pub const SIGN_IN_TIMEOUT: Duration = Duration::from_secs(45);

/// `queue.list` returns at most this many rows.
pub const MAX_QUEUE_LIST: usize = 50;

/// Clamp a client's `at` to the window the daemon accepts; a missing one is
/// `now`.
pub fn clamp_at(at: Option<DateTime<Utc>>, now: DateTime<Utc>) -> DateTime<Utc> {
    match at {
        Some(at) => at.clamp(now - AT_PAST_SLACK, now + AT_FUTURE_SLACK),
        None => now,
    }
}

// ---- Error codes ---------------------------------------------------------------------------

/// Why a request failed (design 10). Unknown codes from a newer daemon read
/// as [`ErrorCode::Internal`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ErrorCode {
    BadRequest,
    UnknownMethod,
    UnsupportedVersion,
    NotSignedIn,
    SignInFailed,
    SignInExpired,
    KeyringLocked,
    ConsentRequired,
    CapabilityUnavailable,
    ChoiceRequired,
    NotFound,
    Precondition,
    NoSession,
    DeviceRevoked,
    Offline,
    Timeout,
    Busy,
    PermissionDenied,
    Internal,
}

impl ErrorCode {
    pub const ALL: [ErrorCode; 19] = [
        ErrorCode::BadRequest,
        ErrorCode::UnknownMethod,
        ErrorCode::UnsupportedVersion,
        ErrorCode::NotSignedIn,
        ErrorCode::SignInFailed,
        ErrorCode::SignInExpired,
        ErrorCode::KeyringLocked,
        ErrorCode::ConsentRequired,
        ErrorCode::CapabilityUnavailable,
        ErrorCode::ChoiceRequired,
        ErrorCode::NotFound,
        ErrorCode::Precondition,
        ErrorCode::NoSession,
        ErrorCode::DeviceRevoked,
        ErrorCode::Offline,
        ErrorCode::Timeout,
        ErrorCode::Busy,
        ErrorCode::PermissionDenied,
        ErrorCode::Internal,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            ErrorCode::BadRequest => "bad_request",
            ErrorCode::UnknownMethod => "unknown_method",
            ErrorCode::UnsupportedVersion => "unsupported_version",
            ErrorCode::NotSignedIn => "not_signed_in",
            ErrorCode::SignInFailed => "sign_in_failed",
            ErrorCode::SignInExpired => "sign_in_expired",
            ErrorCode::KeyringLocked => "keyring_locked",
            ErrorCode::ConsentRequired => "consent_required",
            ErrorCode::CapabilityUnavailable => "capability_unavailable",
            ErrorCode::ChoiceRequired => "choice_required",
            ErrorCode::NotFound => "not_found",
            ErrorCode::Precondition => "precondition",
            ErrorCode::NoSession => "no_session",
            ErrorCode::DeviceRevoked => "device_revoked",
            ErrorCode::Offline => "offline",
            ErrorCode::Timeout => "timeout",
            ErrorCode::Busy => "busy",
            ErrorCode::PermissionDenied => "permission_denied",
            ErrorCode::Internal => "internal",
        }
    }

    /// The code named `name`; an unknown one is [`ErrorCode::Internal`].
    pub fn parse(name: &str) -> ErrorCode {
        ErrorCode::ALL
            .into_iter()
            .find(|code| code.as_str() == name)
            .unwrap_or(ErrorCode::Internal)
    }

    /// The CLI exit code of this error (design 11.3).
    pub fn exit_code(self) -> u8 {
        match self {
            ErrorCode::Internal | ErrorCode::Busy | ErrorCode::Timeout => exit::FAILED,
            ErrorCode::BadRequest | ErrorCode::UnknownMethod => exit::USAGE,
            ErrorCode::UnsupportedVersion => exit::DAEMON,
            ErrorCode::NotSignedIn
            | ErrorCode::SignInFailed
            | ErrorCode::SignInExpired
            | ErrorCode::KeyringLocked => exit::AUTH,
            ErrorCode::ConsentRequired | ErrorCode::CapabilityUnavailable => exit::CONSENT,
            ErrorCode::Offline => exit::OFFLINE,
            ErrorCode::ChoiceRequired | ErrorCode::NotFound => exit::CHOICE,
            ErrorCode::Precondition | ErrorCode::NoSession | ErrorCode::DeviceRevoked => {
                exit::PRECONDITION
            }
            ErrorCode::PermissionDenied => exit::ENVIRONMENT,
        }
    }
}

impl Serialize for ErrorCode {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for ErrorCode {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Ok(ErrorCode::parse(&String::deserialize(deserializer)?))
    }
}

impl std::fmt::Display for ErrorCode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// The CLI's stable exit codes (design 11.3).
pub mod exit {
    pub const OK: u8 = 0;
    /// The operation failed.
    pub const FAILED: u8 = 1;
    /// Usage error, `--password` on the command line, missing input.
    pub const USAGE: u8 = 2;
    /// The daemon is not running, cannot be started, or speaks another major.
    pub const DAEMON: u8 = 3;
    /// Not signed in, refused, expired, keyring locked.
    pub const AUTH: u8 = 4;
    /// Consent required or capability unavailable.
    pub const CONSENT: u8 = 5;
    /// The server cannot be reached.
    pub const OFFLINE: u8 = 6;
    /// A choice is required, or the named thing does not exist.
    pub const CHOICE: u8 = 7;
    /// A precondition failed (no project, nothing to resume, already running,
    /// no graphical session, device revoked).
    pub const PRECONDITION: u8 = 8;
    /// Unsupported environment: root, no `XDG_RUNTIME_DIR`, unsafe
    /// permissions, no Secret Service without the opt-in.
    pub const ENVIRONMENT: u8 = 9;
    /// Daemon only: restart after the binary was replaced.
    pub const RESTART: u8 = 75;
    /// Interrupted.
    pub const INTERRUPTED: u8 = 130;
}

/// An error answer.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ErrorBody {
    pub code: ErrorCode,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hint: Option<String>,
    /// Extra data, e.g. the choices of `choice_required`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

impl ErrorBody {
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            hint: None,
            data: None,
        }
    }

    pub fn with_hint(mut self, hint: impl Into<String>) -> Self {
        self.hint = Some(hint.into());
        self
    }

    pub fn with_data(mut self, data: Value) -> Self {
        self.data = Some(data);
        self
    }
}

impl std::fmt::Display for ErrorBody {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} ({})", self.message, self.code)
    }
}

impl std::error::Error for ErrorBody {}

// ---- Envelope ------------------------------------------------------------------------------

/// A client's request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Request {
    pub v: u32,
    pub id: u64,
    pub method: String,
    #[serde(default)]
    pub params: Value,
}

/// The daemon's answer to one request.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Response {
    pub v: u32,
    pub id: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<ErrorBody>,
    /// For commands: whether anything changed (commands are idempotent).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub changed: Option<bool>,
}

impl Response {
    pub fn ok(id: u64, result: Value, changed: Option<bool>) -> Self {
        Self {
            v: PROTOCOL_VERSION,
            id,
            result: Some(result),
            error: None,
            changed,
        }
    }

    pub fn err(id: u64, error: ErrorBody) -> Self {
        Self {
            v: PROTOCOL_VERSION,
            id,
            result: None,
            error: Some(error),
            changed: None,
        }
    }
}

/// Something the daemon pushes: `state` (a snapshot) or `show` (to GUIs).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Event {
    pub v: u32,
    pub event: String,
    pub seq: u64,
    #[serde(default)]
    pub data: Value,
}

/// A frame from the daemon.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(untagged)]
pub enum ServerFrame {
    Event(Event),
    Response(Response),
}

/// Event names.
pub const EVENT_STATE: &str = "state";
pub const EVENT_SHOW: &str = "show";

// ---- Methods -------------------------------------------------------------------------------

/// Every method of protocol v1.
pub mod method {
    pub const HELLO: &str = "hello";
    pub const STATE_GET: &str = "state.get";
    pub const STATE_SUBSCRIBE: &str = "state.subscribe";
    pub const CAPABILITIES_GET: &str = "capabilities.get";
    pub const AUTH_SIGN_IN: &str = "auth.sign_in";
    pub const AUTH_SIGN_OUT: &str = "auth.sign_out";
    pub const AUTH_UNLOCK_KEYRING: &str = "auth.unlock_keyring";
    pub const WORKSPACE_LIST: &str = "workspace.list";
    pub const WORKSPACE_CHOOSE: &str = "workspace.choose";
    pub const PROJECT_LIST: &str = "project.list";
    pub const PROJECT_SELECT: &str = "project.select";
    pub const TASK_SET: &str = "task.set";
    pub const TRACKING_START: &str = "tracking.start";
    pub const TRACKING_PAUSE: &str = "tracking.pause";
    pub const TRACKING_RESUME: &str = "tracking.resume";
    pub const TRACKING_SYNC_NOW: &str = "tracking.sync_now";
    pub const AWAY_RESOLVE: &str = "away.resolve";
    pub const CONSENT_GET: &str = "consent.get";
    pub const CONSENT_UPDATE: &str = "consent.update";
    pub const CAPTURE_RETRY_PERMISSION: &str = "capture.retry_permission";
    pub const QUEUE_STATUS: &str = "queue.status";
    pub const QUEUE_LIST: &str = "queue.list";
    pub const QUEUE_PURGE: &str = "queue.purge";
    pub const CONFIG_GET: &str = "config.get";
    pub const CONFIG_SET: &str = "config.set";
    pub const CONFIG_UNSET: &str = "config.unset";
    pub const CONFIG_RELOAD: &str = "config.reload";
    pub const SCREENSHOT_LATEST: &str = "screenshot.latest";
    pub const UI_SHOW: &str = "ui.show";
    pub const DAEMON_INFO: &str = "daemon.info";
    pub const DAEMON_SHUTDOWN: &str = "daemon.shutdown";

    pub const ALL: [&str; 31] = [
        HELLO,
        STATE_GET,
        STATE_SUBSCRIBE,
        CAPABILITIES_GET,
        AUTH_SIGN_IN,
        AUTH_SIGN_OUT,
        AUTH_UNLOCK_KEYRING,
        WORKSPACE_LIST,
        WORKSPACE_CHOOSE,
        PROJECT_LIST,
        PROJECT_SELECT,
        TASK_SET,
        TRACKING_START,
        TRACKING_PAUSE,
        TRACKING_RESUME,
        TRACKING_SYNC_NOW,
        AWAY_RESOLVE,
        CONSENT_GET,
        CONSENT_UPDATE,
        CAPTURE_RETRY_PERMISSION,
        QUEUE_STATUS,
        QUEUE_LIST,
        QUEUE_PURGE,
        CONFIG_GET,
        CONFIG_SET,
        CONFIG_UNSET,
        CONFIG_RELOAD,
        SCREENSHOT_LATEST,
        UI_SHOW,
        DAEMON_INFO,
        DAEMON_SHUTDOWN,
    ];
}

/// `hello` params.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HelloParams {
    /// `cli` or `gui`.
    pub client: String,
    pub version: String,
}

/// `hello` result.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HelloResult {
    pub protocol: u32,
    pub daemon_version: String,
    pub pid: u32,
    pub started_at: DateTime<Utc>,
}

/// Params of the commands that only carry the user's action time.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AtParams {
    #[serde(default)]
    pub at: Option<DateTime<Utc>>,
}

/// `auth.sign_in` params. The password is zeroized when dropped and never
/// printed by `Debug`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SignInParams {
    #[serde(default)]
    pub server: Option<String>,
    pub email: String,
    pub password: Secret,
    #[serde(default)]
    pub workspace: Option<String>,
    #[serde(default)]
    pub remember_password: Option<bool>,
}

/// `auth.sign_in` result.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SignInResult {
    pub status: crate::agent::state::TrackerStatus,
    /// Several workspaces and none named: choose one (`workspace.choose`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub workspace_choices: Vec<WorkspaceChoice>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace: Option<String>,
}

/// `auth.sign_out` params.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SignOutParams {
    #[serde(default)]
    pub at: Option<DateTime<Utc>>,
    /// Also delete this account's queued rows (confirmed by the client).
    #[serde(default)]
    pub discard_unsent: bool,
}

/// `workspace.choose` params: one of index, id or name.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceChooseParams {
    #[serde(default)]
    pub index: Option<usize>,
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub at: Option<DateTime<Utc>>,
}

/// `project.list` params.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectListParams {
    #[serde(default)]
    pub refresh: bool,
}

/// `project.select` params: an id or a title.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectSelectParams {
    pub project: String,
    #[serde(default)]
    pub at: Option<DateTime<Utc>>,
}

/// `task.set` params (an empty note clears it).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskSetParams {
    pub note: String,
    #[serde(default)]
    pub at: Option<DateTime<Utc>>,
}

/// `tracking.pause` params.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PauseParams {
    #[serde(default)]
    pub resume_after_secs: Option<u64>,
    #[serde(default)]
    pub at: Option<DateTime<Utc>>,
}

/// `away.resolve` params.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AwayParams {
    pub keep: bool,
}

/// `consent.update` params. `origin` says where the change came from
/// (`cli`, `gui`, `tray`) for the consent receipt.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConsentUpdateParams {
    pub consent: LocalConsent,
    #[serde(default)]
    pub origin: Option<String>,
}

/// `queue.list` params.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct QueueListParams {
    /// Only parked rows.
    #[serde(default)]
    pub parked: bool,
    #[serde(default)]
    pub limit: Option<usize>,
}

/// Which queued rows `queue.purge` deletes.
pub use crate::agent::state::PurgeQueue as PurgeWhich;

/// `queue.purge` params; `confirmed` must be true.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QueuePurgeParams {
    pub which: PurgeWhich,
    #[serde(default)]
    pub confirmed: bool,
}

/// `config.get` / `config.unset` params.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConfigKeyParams {
    pub key: String,
}

/// `config.set` params.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConfigSetParams {
    pub key: String,
    pub value: String,
}

/// `ui.show` params.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct UiShowParams {
    #[serde(default)]
    pub view: Option<String>,
}

/// The snapshot as it goes over IPC: the bounded [`SharedState`].
pub fn snapshot_value(state: SharedState) -> Value {
    serde_json::to_value(state.bounded()).unwrap_or(Value::Null)
}

// ---- Codec ---------------------------------------------------------------------------------

/// Why a frame could not be read or written.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FrameError {
    /// Longer than [`MAX_FRAME_BYTES`].
    TooLarge,
    /// Not UTF-8 or not a JSON object of the expected shape.
    Malformed(String),
}

impl std::fmt::Display for FrameError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FrameError::TooLarge => write!(f, "frame larger than {MAX_FRAME_BYTES} bytes"),
            FrameError::Malformed(why) => write!(f, "malformed frame: {why}"),
        }
    }
}

impl std::error::Error for FrameError {}

/// `value` as one frame: compact JSON plus the newline.
pub fn encode<T: Serialize>(value: &T) -> Result<Vec<u8>, FrameError> {
    let mut bytes = serde_json::to_vec(value).map_err(|e| FrameError::Malformed(e.to_string()))?;
    if bytes.len() > MAX_FRAME_BYTES {
        bytes.zeroize();
        return Err(FrameError::TooLarge);
    }
    bytes.push(b'\n');
    Ok(bytes)
}

/// Parse one frame (without its newline).
pub fn decode<T: serde::de::DeserializeOwned>(frame: &[u8]) -> Result<T, FrameError> {
    let text = std::str::from_utf8(frame).map_err(|_| FrameError::Malformed("not UTF-8".into()))?;
    serde_json::from_str(text).map_err(|e| FrameError::Malformed(e.to_string()))
}

/// Splits a byte stream into frames, refusing any longer than
/// [`MAX_FRAME_BYTES`] as soon as that many bytes arrive without a newline.
/// Consumed bytes are wiped, and frames come out in zeroizing buffers.
#[derive(Default)]
pub struct FrameDecoder {
    buffer: Zeroizing<Vec<u8>>,
}

impl FrameDecoder {
    pub fn new() -> Self {
        Self::default()
    }

    /// Bytes of an incomplete frame are waiting.
    pub fn has_partial(&self) -> bool {
        !self.buffer.is_empty()
    }

    /// Add `bytes`; returns the complete frames (newline removed, empty lines
    /// skipped). After [`FrameError::TooLarge`] the stream must be closed.
    pub fn push(&mut self, bytes: &[u8]) -> Result<Vec<Zeroizing<Vec<u8>>>, FrameError> {
        let mut frames = Vec::new();
        let mut rest = bytes;
        while let Some(newline) = rest.iter().position(|b| *b == b'\n') {
            let (line, after) = rest.split_at(newline);
            if self.buffer.len() + line.len() > MAX_FRAME_BYTES {
                self.buffer.zeroize();
                self.buffer.clear();
                return Err(FrameError::TooLarge);
            }
            let mut frame = Zeroizing::new(std::mem::take(&mut *self.buffer));
            frame.extend_from_slice(line);
            if frame.last() == Some(&b'\r') {
                frame.pop();
            }
            if !frame.iter().all(u8::is_ascii_whitespace) {
                frames.push(frame);
            }
            rest = &after[1..];
        }
        if self.buffer.len() + rest.len() > MAX_FRAME_BYTES {
            self.buffer.zeroize();
            self.buffer.clear();
            return Err(FrameError::TooLarge);
        }
        self.buffer.extend_from_slice(rest);
        Ok(frames)
    }
}

/// The JSON envelope of one CLI result (`--json`, `dosi-tracker.cli/1`).
pub const CLI_SCHEMA: &str = "dosi-tracker.cli/1";

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn every_error_code_has_a_stable_name_and_exit_code() {
        let expected: [(&str, u8); 19] = [
            ("bad_request", 2),
            ("unknown_method", 2),
            ("unsupported_version", 3),
            ("not_signed_in", 4),
            ("sign_in_failed", 4),
            ("sign_in_expired", 4),
            ("keyring_locked", 4),
            ("consent_required", 5),
            ("capability_unavailable", 5),
            ("choice_required", 7),
            ("not_found", 7),
            ("precondition", 8),
            ("no_session", 8),
            ("device_revoked", 8),
            ("offline", 6),
            ("timeout", 1),
            ("busy", 1),
            ("permission_denied", 9),
            ("internal", 1),
        ];
        for (code, (name, exit_code)) in ErrorCode::ALL.into_iter().zip(expected) {
            assert_eq!(code.as_str(), name);
            assert_eq!(code.exit_code(), exit_code, "{name}");
            assert_eq!(ErrorCode::parse(name), code);
            assert_eq!(serde_json::to_value(code).unwrap(), json!(name));
        }
        let unknown: ErrorCode = serde_json::from_value(json!("from_the_future")).unwrap();
        assert_eq!(unknown, ErrorCode::Internal);
    }

    #[test]
    fn at_is_clamped_to_the_accepted_window() {
        let now = Utc::now();
        assert_eq!(clamp_at(None, now), now);
        assert_eq!(
            clamp_at(Some(now - chrono::TimeDelta::minutes(10)), now),
            now - AT_PAST_SLACK
        );
        assert_eq!(
            clamp_at(Some(now + chrono::TimeDelta::minutes(1)), now),
            now + AT_FUTURE_SLACK
        );
        let recent = now - chrono::TimeDelta::seconds(5);
        assert_eq!(clamp_at(Some(recent), now), recent);
    }

    #[test]
    fn frames_split_on_newlines_and_skip_blank_lines() {
        let mut decoder = FrameDecoder::new();
        let frames = decoder.push(b"{\"a\":1}\n\n  \n{\"b\"").unwrap();
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].as_slice(), b"{\"a\":1}");
        assert!(decoder.has_partial());
        let frames = decoder.push(b":2}\r\n").unwrap();
        assert_eq!(frames[0].as_slice(), b"{\"b\":2}");
        assert!(!decoder.has_partial());
    }

    #[test]
    fn a_frame_over_the_cap_is_refused_before_its_newline_arrives() {
        let mut decoder = FrameDecoder::new();
        let chunk = vec![b'x'; 512 * 1024];
        assert!(decoder.push(&chunk).unwrap().is_empty());
        assert!(
            decoder.push(&chunk).unwrap().is_empty(),
            "exactly the cap is fine"
        );
        assert_eq!(decoder.push(b"x"), Err(FrameError::TooLarge));

        let mut exact = vec![b' '; MAX_FRAME_BYTES];
        exact[0] = b'1';
        exact.push(b'\n');
        assert_eq!(FrameDecoder::new().push(&exact).unwrap().len(), 1);
        let mut over = vec![b' '; MAX_FRAME_BYTES + 1];
        over.push(b'\n');
        assert_eq!(FrameDecoder::new().push(&over), Err(FrameError::TooLarge));
    }

    #[test]
    fn encode_refuses_oversized_values_and_appends_the_newline() {
        let frame = encode(&json!({"v": 1})).unwrap();
        assert_eq!(frame, b"{\"v\":1}\n");
        let huge = "x".repeat(MAX_FRAME_BYTES);
        assert_eq!(encode(&json!({ "s": huge })), Err(FrameError::TooLarge));
    }

    #[test]
    fn requests_responses_and_events_round_trip() {
        let request: Request =
            decode(br#"{"v":1,"id":7,"method":"tracking.pause","params":{"resume_after_secs":900},"future":true}"#)
                .unwrap();
        assert_eq!(
            (request.v, request.id, request.method.as_str()),
            (1, 7, "tracking.pause")
        );
        let pause: PauseParams = serde_json::from_value(request.params).unwrap();
        assert_eq!(pause.resume_after_secs, Some(900));

        let no_params: Request = decode(br#"{"v":1,"id":1,"method":"state.get"}"#).unwrap();
        assert!(no_params.params.is_null());

        let ok: ServerFrame =
            decode(br#"{"v":1,"id":7,"result":{"status":"paused"},"changed":true}"#).unwrap();
        assert_eq!(
            ok,
            ServerFrame::Response(Response::ok(7, json!({"status": "paused"}), Some(true)))
        );
        let failed: ServerFrame = decode(
            br#"{"v":1,"id":8,"error":{"code":"consent_required","message":"m","hint":"h"}}"#,
        )
        .unwrap();
        match failed {
            ServerFrame::Response(response) => {
                let error = response.error.unwrap();
                assert_eq!(error.code, ErrorCode::ConsentRequired);
                assert_eq!(error.hint.as_deref(), Some("h"));
            }
            other => panic!("{other:?}"),
        }
        let event: ServerFrame = decode(br#"{"v":1,"event":"state","seq":42,"data":{}}"#).unwrap();
        assert!(matches!(event, ServerFrame::Event(Event { seq: 42, .. })));
    }

    #[test]
    fn sign_in_params_never_print_the_password() {
        let params: SignInParams = serde_json::from_value(json!({
            "email": "ayesha@acme.test", "password": "canary-pw-0042"
        }))
        .unwrap();
        assert_eq!(params.password.expose(), "canary-pw-0042");
        let debug = format!("{params:?}");
        assert!(!debug.contains("canary-pw-0042"), "{debug}");
        assert!(params.server.is_none() && params.workspace.is_none());
    }

    #[test]
    fn every_method_name_is_unique() {
        let mut names = method::ALL.to_vec();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), method::ALL.len());
    }

    #[test]
    fn the_snapshot_value_is_bounded_and_has_no_screenshot_bytes() {
        let value = snapshot_value(SharedState::default());
        assert_eq!(value["status"], "signed_out");
        assert!(value.get("last_screenshot_at").is_some());
        assert!(value.get("commands_handled").is_none(), "{value}");
    }
}
