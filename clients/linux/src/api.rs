//! Async HTTP API client, token manager, and transport coordinator for Dosi-Tracker Linux.
//!
//! # Architecture & Security Invariants
//! - Async reqwest HTTP client built with rustls and system/bundled TLS roots.
//! - Strict TLS verification: certificate verification is NEVER disabled. Redirects are
//!   never followed, and plain `http://` is refused except for loopback unless the admin
//!   opted in (`allow_insecure_http`).
//! - Finite timeouts: connect timeout (10s), request timeout (30s), upload timeout (120s), heartbeat timeout (10s).
//! - Authentication follows OpenIddict password grant (`POST /connect/token`) with
//!   `client_id = Tracker_App` and the install's `device_id`; no tenant query parameter and no
//!   workspace header on the token endpoint.
//! - Header rules (Windows parity): `GET /api/app/workspaces/mine` is sent with the bearer
//!   token only; every workspace-scoped call carries `X-Workspace-Id`.
//! - Pre-expiry token renewal: tokens are refreshed 60 seconds before expiration. A refused
//!   refresh always means the sign-in has expired: it is never a cue to silently fall back
//!   to a remembered password. A rotated refresh token is handed to the [`TokenCell`]
//!   rotation listener.
//! - Bounded 401 retry: upon receiving HTTP 401, attempts token refresh once and retries once.
//! - Activity delivery follows `POST /api/app/activity` with exact `client_activity_id` idempotency;
//!   `sentAt` is stamped per attempt. Upload failures are classified exactly as on Windows
//!   ([`classify_upload_failure`]) plus `DeviceRevoked`, `WorkspaceArchived` and `WorkspaceRequired`.
//! - Presence heartbeat follows `POST /api/app/presence/heartbeat`, bounded as a whole by its timeout.
//! - Zero secrets or tokens in logs, Display, or Debug representations.

use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use base64::Engine as _;
use chrono::{DateTime, Utc};
use reqwest::header::{HeaderMap, ACCEPT, DATE, RETRY_AFTER};
use reqwest::StatusCode;
use serde::de::DeserializeOwned;
use serde::Deserialize;
use tokio::sync::Mutex as TokioMutex;
use zeroize::Zeroizing;

use crate::agent::model::{
    AbpError, Activity, DeviceStatus, Heartbeat, HeartbeatResult, Project, ReportSummary, Workspace,
};
use crate::credentials::{CredentialStore, Credentials};
use crate::outbox::{
    capture_digests, ActivityEnvelope, CaptureKind, OutboxEntry, OutboxScope, StoredCapture,
};
use crate::signing::{sign_activity_envelope, verify_v2};
use crate::sync::{DeliveryOutcome, DeliveryTransport, RetryKind};

/// OpenIddict client identifier seeded by the Dosi-Tracker backend.
pub const CLIENT_ID: &str = "Tracker_App";

/// OpenIddict scope for the tracker application.
pub const SCOPE: &str = "openid profile email roles offline_access Tracker";

/// HTTP header naming the active workspace for workspace-scoped requests.
pub const WORKSPACE_HEADER: &str = "X-Workspace-Id";

/// Default presence heartbeat cadence primitive (60 seconds).
pub const HEARTBEAT_CADENCE: Duration = Duration::from_secs(60);

/// TCP/TLS connection timeout.
pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

/// General REST request timeout budget.
pub const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

/// Upload request timeout budget (for activities containing metadata/captures).
pub const UPLOAD_TIMEOUT: Duration = Duration::from_secs(120);

/// Heartbeat timeout budget.
pub const HEARTBEAT_TIMEOUT: Duration = Duration::from_secs(10);

/// How long [`HttpDeliveryTransport`] waits for its worker thread beyond the
/// upload timeout before reporting the delivery as a transient failure.
pub const DELIVERY_GRACE: Duration = Duration::from_secs(10);

/// Longest Retry-After header duration honored (6 hours).
pub const MAX_RETRY_AFTER: Duration = Duration::from_secs(6 * 60 * 60);

/// Skew warning threshold in seconds (2 minutes).
pub const CLOCK_SKEW_WARNING_SECS: i64 = 120;

/// Reason a row is parked with when its local signature does not verify.
pub const INTEGRITY_FAILURE_REASON: &str = "failed local integrity check";

/// Standard ABP business error codes returned by the backend.
pub mod codes {
    pub const NOT_WORKSPACE_MEMBER: &str = "Tracker:NotWorkspaceMember";
    pub const WORKSPACE_ARCHIVED: &str = "Tracker:WorkspaceArchived";
    pub const WORKSPACE_REQUIRED: &str = "Tracker:WorkspaceRequired";
    pub const DEVICE_REVOKED: &str = "Tracker:DeviceRevoked";
    pub const ACTIVITY_TIME_RANGE_INVALID: &str = "Tracker:ActivityTimeRangeInvalid";
    pub const ACTIVITY_CLIENT_ID_CONFLICT: &str = "Tracker:ActivityClientIdConflict";
    pub const ACTIVITY_IN_FUTURE: &str = "Tracker:ActivityInFuture";
    pub const ACTIVITY_TOO_LONG: &str = "Tracker:ActivityTooLong";
    pub const ACTIVITY_TOO_OLD: &str = "Tracker:ActivityTooOld";
    pub const ACTIVITY_OVERLAPS: &str = "Tracker:ActivityOverlaps";
    pub const PROJECT_ARCHIVED: &str = "Tracker:ProjectArchived";
    pub const NOT_PROJECT_MEMBER: &str = "Tracker:NotProjectMember";
    pub const INVALID_CAPTURE_DATA: &str = "Tracker:InvalidCaptureData";
    pub const CAPTURE_TOO_LARGE: &str = "Tracker:CaptureTooLarge";
    pub const SUBSCRIPTION_INACTIVE: &str = "Tracker:SubscriptionInactive";

    /// Every code this agent acts on (checked against the backend by the contract drift test).
    pub const ALL: [&str; 15] = [
        NOT_WORKSPACE_MEMBER,
        WORKSPACE_ARCHIVED,
        WORKSPACE_REQUIRED,
        DEVICE_REVOKED,
        ACTIVITY_TIME_RANGE_INVALID,
        ACTIVITY_CLIENT_ID_CONFLICT,
        ACTIVITY_IN_FUTURE,
        ACTIVITY_TOO_LONG,
        ACTIVITY_TOO_OLD,
        ACTIVITY_OVERLAPS,
        PROJECT_ARCHIVED,
        NOT_PROJECT_MEMBER,
        INVALID_CAPTURE_DATA,
        CAPTURE_TOO_LARGE,
        SUBSCRIPTION_INACTIVE,
    ];
}

/// Builds the shared reqwest HTTP client with rustls-tls and strict verification.
/// Redirects are never followed: a 3xx is answered like any other unexpected status.
pub fn build_http_client() -> reqwest::Client {
    reqwest::Client::builder()
        .user_agent(concat!("dosi-tracker-linux/", env!("CARGO_PKG_VERSION")))
        .connect_timeout(CONNECT_TIMEOUT)
        .timeout(REQUEST_TIMEOUT)
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .expect("Failed to initialize secure HTTP client")
}

/// May credentials be sent to `base_url`? `https://` always; `http://` only to
/// a loopback host (`localhost`, 127.0.0.0/8, `::1`) unless the admin opted in
/// with `allow_insecure_http`. Anything else is refused.
pub fn check_server_url(base_url: &str, allow_insecure_http: bool) -> Result<(), String> {
    let url = reqwest::Url::parse(base_url.trim())
        .map_err(|e| format!("The server address is not a valid URL ({e})."))?;
    match url.scheme() {
        "https" => Ok(()),
        "http" if allow_insecure_http || is_loopback_host(&url) => Ok(()),
        "http" => Err(format!(
            "Refusing to sign in over plain http:// to {}: use https://, or set allow_insecure_http = true.",
            url.host_str().unwrap_or("the server")
        )),
        other => Err(format!(
            "The server address must start with https:// (got {other}://)."
        )),
    }
}

fn is_loopback_host(url: &reqwest::Url) -> bool {
    let Some(host) = url.host_str() else {
        return false;
    };
    let host = host.trim_start_matches('[').trim_end_matches(']');
    host.eq_ignore_ascii_case("localhost")
        || host
            .parse::<std::net::IpAddr>()
            .is_ok_and(|ip| ip.is_loopback())
}

/// Query parameters for `GET /api/app/reporting/summary`. `TimeZone` (an
/// IANA id) makes the server's "days" the device's calendar days.
///
/// Passed through `RequestBuilder::query` so they are percent-encoded: a raw
/// `+00:00` offset in a hand-built URL is decoded by the server as a space,
/// which fails model binding (HTTP 400).
fn summary_query(
    from: DateTime<Utc>,
    to: DateTime<Utc>,
    time_zone: Option<&str>,
) -> Vec<(&'static str, String)> {
    let iso = |t: DateTime<Utc>| t.to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
    let mut query = vec![("From", iso(from)), ("To", iso(to))];
    if let Some(zone) = time_zone.map(str::trim).filter(|zone| !zone.is_empty()) {
        query.push(("TimeZone", zone.to_string()));
    }
    query
}

/// Normalized server clock tracker based on HTTP `Date` response headers.
#[derive(Debug, Clone)]
pub struct ServerClock(Arc<AtomicI64>);

const UNKNOWN_OFFSET: i64 = i64::MIN;

impl Default for ServerClock {
    fn default() -> Self {
        Self(Arc::new(AtomicI64::new(UNKNOWN_OFFSET)))
    }
}

impl ServerClock {
    pub fn offset_secs(&self) -> Option<i64> {
        Some(self.0.load(Ordering::Relaxed)).filter(|&o| o != UNKNOWN_OFFSET)
    }

    pub fn record(&self, offset_secs: i64) {
        self.0
            .store(offset_secs.max(UNKNOWN_OFFSET + 1), Ordering::Relaxed);
    }

    pub fn skew_minutes(&self) -> Option<i64> {
        self.offset_secs().and_then(skew_warning_minutes)
    }
}

pub fn skew_warning_minutes(offset_secs: i64) -> Option<i64> {
    (offset_secs.abs() > CLOCK_SKEW_WARNING_SECS)
        .then(|| (offset_secs as f64 / 60.0).round() as i64)
}

pub fn clock_offset_from_date(date_header: &str, device_now: DateTime<Utc>) -> Option<i64> {
    let server = DateTime::parse_from_rfc2822(date_header.trim())
        .ok()?
        .with_timezone(&Utc);
    Some((server - device_now).num_seconds())
}

pub fn parse_retry_after(
    value: &str,
    server_now: Option<DateTime<Utc>>,
    device_now: DateTime<Utc>,
) -> Option<Duration> {
    let value = value.trim();
    let wait = match value.parse::<u64>() {
        Ok(seconds) => Duration::from_secs(seconds),
        Err(_) => {
            let at = DateTime::parse_from_rfc2822(value)
                .ok()?
                .with_timezone(&Utc);
            (at - server_now.unwrap_or(device_now))
                .to_std()
                .unwrap_or_default()
        }
    };
    Some(wait.min(MAX_RETRY_AFTER))
}

fn retry_after_of(headers: &HeaderMap) -> Option<Duration> {
    let value = headers.get(RETRY_AFTER)?.to_str().ok()?;
    let server_now = headers
        .get(DATE)
        .and_then(|d| d.to_str().ok())
        .and_then(|d| DateTime::parse_from_rfc2822(d.trim()).ok())
        .map(|d| d.with_timezone(&Utc));
    parse_retry_after(value, server_now, Utc::now())
}

fn describe_error(error: &(dyn std::error::Error + 'static)) -> String {
    let mut text = error.to_string();
    let mut source = error.source();
    while let Some(cause) = source {
        let cause_text = cause.to_string();
        if !text.contains(&cause_text) {
            text.push_str(": ");
            text.push_str(&cause_text);
        }
        source = cause.source();
    }
    text
}

/// Authentication and token error conditions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuthError {
    /// Server refused credentials (OpenIddict `invalid_grant`, HTTP 401/403).
    Rejected(String),
    /// Server unreachable (network, connect timeout, DNS failure).
    Offline(String),
    /// Server error (5xx, unexpected response structure), or a server this
    /// agent refuses to send credentials to.
    Server(String),
}

impl AuthError {
    /// The saved sign-in is no longer valid ("sign-in expired").
    pub fn is_rejected(&self) -> bool {
        matches!(self, AuthError::Rejected(_))
    }
}

impl std::fmt::Display for AuthError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Rejected(msg) => write!(f, "Authentication rejected: {msg}"),
            Self::Offline(msg) => write!(f, "Server unreachable: {msg}"),
            Self::Server(msg) => write!(f, "Authentication server error: {msg}"),
        }
    }
}

impl std::error::Error for AuthError {}

#[derive(Debug, Default, Deserialize)]
struct OAuthErrorResponse {
    #[serde(default)]
    error: Option<String>,
    #[serde(default)]
    error_description: Option<String>,
}

/// Classify a token-endpoint failure. Only a refusal of the credentials is
/// [`AuthError::Rejected`]; a bare 400 from a proxy, a 404 or a 5xx is not.
pub fn classify_token_failure(status: u16, body: &str) -> AuthError {
    let oauth: OAuthErrorResponse = serde_json::from_str(body).unwrap_or_default();
    let description = oauth
        .error_description
        .map(|d| d.trim().to_string())
        .filter(|d| !d.is_empty());

    if oauth.error.as_deref() == Some("invalid_grant") || matches!(status, 401 | 403) {
        return AuthError::Rejected(
            description.unwrap_or_else(|| "Incorrect username or password.".to_string()),
        );
    }
    match status {
        429 => AuthError::Server("Too many sign-in attempts. Try again later.".to_string()),
        500.. => AuthError::Server(format!("The server encountered an error (HTTP {status}).")),
        _ => AuthError::Server(
            description.unwrap_or_else(|| format!("Sign-in failed (HTTP {status}).")),
        ),
    }
}

/// General API request error.
#[derive(Debug)]
pub enum ApiError {
    /// Signing in (again) failed.
    Auth(AuthError),
    /// The server could not be reached, or did not answer in time.
    Offline(String),
    /// The server answered with an error status (after one sign-in retry on a
    /// 401): the ABP error summary when it sent one, else `HTTP <status>`.
    Http { status: u16, message: String },
    /// 403 `Tracker:NotWorkspaceMember`: the account lost access to the workspace.
    NotWorkspaceMember(String),
    /// 403 `Tracker:WorkspaceArchived`: the workspace was archived.
    WorkspaceArchived(String),
    /// 400 `Tracker:WorkspaceRequired`: no valid workspace was named.
    WorkspaceRequired(String),
    /// 403 `Tracker:DeviceRevoked`: an admin revoked this device in the workspace.
    DeviceRevoked(String),
    /// The answer could not be read, or the request could not be built.
    Invalid(String),
}

impl ApiError {
    /// The saved credentials were refused: the sign-in has expired.
    pub fn is_auth_rejected(&self) -> bool {
        matches!(self, ApiError::Auth(AuthError::Rejected(_)))
    }

    /// The HTTP status, when the server answered with an error.
    pub fn status(&self) -> Option<u16> {
        match self {
            ApiError::Http { status, .. } => Some(*status),
            _ => None,
        }
    }

    /// One of the terminal answers that stop the agent in this workspace
    /// until the user acts (re-sign-in or a workspace switch).
    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            ApiError::NotWorkspaceMember(_)
                | ApiError::WorkspaceArchived(_)
                | ApiError::WorkspaceRequired(_)
                | ApiError::DeviceRevoked(_)
        )
    }
}

impl std::fmt::Display for ApiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Auth(err) => write!(f, "{err}"),
            Self::Offline(msg) => write!(f, "Network error: {msg}"),
            Self::Http { status, message } => write!(f, "HTTP {status}: {message}"),
            Self::NotWorkspaceMember(msg) => write!(f, "Not a workspace member: {msg}"),
            Self::WorkspaceArchived(msg) => write!(f, "Workspace archived: {msg}"),
            Self::WorkspaceRequired(msg) => write!(f, "Workspace required: {msg}"),
            Self::DeviceRevoked(msg) => write!(f, "Device revoked: {msg}"),
            Self::Invalid(msg) => write!(f, "Invalid response: {msg}"),
        }
    }
}

impl std::error::Error for ApiError {}

impl From<AuthError> for ApiError {
    fn from(err: AuthError) -> Self {
        match err {
            // Unreachable is unreachable, whichever request found out.
            AuthError::Offline(reason) => Self::Offline(reason),
            other => Self::Auth(other),
        }
    }
}

/// The error for a failed answer: the terminal workspace and device answers
/// have their own kinds.
fn http_error(status: u16, body: &str) -> ApiError {
    let error = AbpError::from_body(body);
    let message = error
        .as_ref()
        .and_then(AbpError::summary)
        .unwrap_or_else(|| format!("HTTP {status}"));
    match (status, error.as_ref().and_then(AbpError::code)) {
        (403, Some(codes::NOT_WORKSPACE_MEMBER)) => ApiError::NotWorkspaceMember(message),
        (403, Some(codes::WORKSPACE_ARCHIVED)) => ApiError::WorkspaceArchived(message),
        (403, Some(codes::DEVICE_REVOKED)) => ApiError::DeviceRevoked(message),
        (400, Some(codes::WORKSPACE_REQUIRED)) => ApiError::WorkspaceRequired(message),
        _ => ApiError::Http { status, message },
    }
}

/// Read a JSON answer, or the error the server sent instead.
async fn read_json<T: DeserializeOwned>(response: reqwest::Response) -> Result<T, ApiError> {
    let status = response.status();
    if !status.is_success() {
        let body = response.text().await.unwrap_or_default();
        return Err(http_error(status.as_u16(), &body));
    }
    let body = response
        .text()
        .await
        .map_err(|e| ApiError::Offline(describe_error(&e)))?;
    serde_json::from_str(&body).map_err(|e| ApiError::Invalid(e.to_string()))
}

/// How an upload failed, so the sync loop can act on it (Windows table plus
/// the Linux additions, design 8.4). Only [`SubmitError::Rejected`] parks the
/// row; every other outcome keeps it (a revoked device's row is parked by the
/// sync pass).
#[derive(Debug)]
pub enum SubmitError {
    /// Network trouble, a timeout, 408/429/5xx/404/405, a 4xx without an ABP
    /// error code, or a business error this agent does not know: keep the row
    /// and retry with backoff, no sooner than `retry_after` when the server
    /// sent one.
    Transient {
        reason: String,
        retry_after: Option<Duration>,
    },
    /// Signing in again was refused (the saved credentials no longer work):
    /// "Sign-in expired". Keep the queue and stop capturing.
    AuthRejected(String),
    /// 402 `Tracker:SubscriptionInactive`: keep the row, retry hourly.
    SubscriptionInactive(String),
    /// 403 `Tracker:NotProjectMember` / 409 `Tracker:ProjectArchived`: refresh
    /// the projects; park the row only if its project is really gone.
    ProjectUnavailable { code: String, message: String },
    /// 425 `Tracker:ActivityInFuture`: this computer's clock is ahead. Keep
    /// the row, retry in 5 minutes.
    ClockAhead(String),
    /// 413, `Tracker:CaptureTooLarge` or `Tracker:InvalidCaptureData`: the
    /// images are the problem. Strip them from the row and resend at once.
    StripCaptures(String),
    /// 403 `Tracker:NotWorkspaceMember`: the account lost access to the
    /// row's workspace. Keep the row; it waits for access to come back.
    NotWorkspaceMember(String),
    /// 403 `Tracker:WorkspaceArchived`: the row's workspace was archived.
    WorkspaceArchived(String),
    /// 400 `Tracker:WorkspaceRequired`: the request named no valid workspace.
    WorkspaceRequired(String),
    /// 403 `Tracker:DeviceRevoked`: an admin revoked this device in the
    /// row's workspace; there is no un-revoke.
    DeviceRevoked(String),
    /// A definitive rejection of this block (overlap, too long/old, invalid
    /// time range, id conflict, validation errors): park it with the server's
    /// message.
    Rejected(String),
}

impl std::fmt::Display for SubmitError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Transient {
                reason,
                retry_after: Some(wait),
            } => {
                write!(f, "{reason} (retry after {}s)", wait.as_secs())
            }
            Self::Transient {
                reason,
                retry_after: None,
            } => write!(f, "{reason}"),
            Self::AuthRejected(reason) => write!(f, "Authentication rejected: {reason}"),
            Self::SubscriptionInactive(message) => write!(f, "Subscription inactive: {message}"),
            Self::ProjectUnavailable { code, message } => write!(f, "{code}: {message}"),
            Self::ClockAhead(message) => write!(f, "Clock ahead: {message}"),
            Self::StripCaptures(message) => write!(f, "Captures refused: {message}"),
            Self::NotWorkspaceMember(message) => write!(f, "Not a workspace member: {message}"),
            Self::WorkspaceArchived(message) => write!(f, "Workspace archived: {message}"),
            Self::WorkspaceRequired(message) => write!(f, "Workspace required: {message}"),
            Self::DeviceRevoked(message) => write!(f, "Device revoked: {message}"),
            Self::Rejected(reason) => write!(f, "Activity rejected: {reason}"),
        }
    }
}

impl std::error::Error for SubmitError {}

/// Classify an upload answer that is not a success (ported from Windows).
/// Business error codes decide first (an older server answers 403 for every
/// unmapped business error), then the status. A 401 never gets here: the
/// client signs in again and resends first.
pub fn classify_upload_failure(
    status: u16,
    body: &str,
    retry_after: Option<Duration>,
) -> SubmitError {
    let error = AbpError::from_body(body);
    let code = error.as_ref().and_then(AbpError::code);
    let message = error
        .as_ref()
        .and_then(AbpError::summary)
        .unwrap_or_else(|| format!("HTTP {status}"));
    let transient = |reason: String| SubmitError::Transient {
        reason,
        retry_after,
    };

    if status == 413
        || matches!(
            code,
            Some(codes::CAPTURE_TOO_LARGE | codes::INVALID_CAPTURE_DATA)
        )
    {
        return SubmitError::StripCaptures(message);
    }
    if !(400..500).contains(&status) {
        // 5xx (or anything unexpected): the server's trouble, not this block's.
        return transient(format!("HTTP {status}"));
    }
    let invalid_payload =
        status == 400 && error.as_ref().is_some_and(AbpError::has_validation_errors);
    match code {
        Some(codes::SUBSCRIPTION_INACTIVE) => SubmitError::SubscriptionInactive(message),
        Some(codes::NOT_WORKSPACE_MEMBER) => SubmitError::NotWorkspaceMember(message),
        Some(codes::WORKSPACE_ARCHIVED) => SubmitError::WorkspaceArchived(message),
        Some(codes::WORKSPACE_REQUIRED) => SubmitError::WorkspaceRequired(message),
        Some(codes::DEVICE_REVOKED) => SubmitError::DeviceRevoked(message),
        Some(code @ (codes::NOT_PROJECT_MEMBER | codes::PROJECT_ARCHIVED)) => {
            SubmitError::ProjectUnavailable {
                code: code.to_string(),
                message,
            }
        }
        Some(codes::ACTIVITY_IN_FUTURE) => SubmitError::ClockAhead(message),
        Some(
            codes::ACTIVITY_OVERLAPS
            | codes::ACTIVITY_CLIENT_ID_CONFLICT
            | codes::ACTIVITY_TOO_LONG
            | codes::ACTIVITY_TOO_OLD
            | codes::ACTIVITY_TIME_RANGE_INVALID,
        ) => SubmitError::Rejected(message),
        // The block itself failed validation (whatever code came with it):
        // resending it unchanged can never succeed.
        _ if invalid_payload => SubmitError::Rejected(message),
        // A business error this agent does not know: keep the block and retry
        // later rather than guess that it is permanent.
        Some(other) => transient(format!("HTTP {status} {other}: {message}")),
        // 404, 405, 408, 429, or any 4xx without an ABP error code (a proxy,
        // a missing route).
        None => transient(format!("HTTP {status}")),
    }
}

/// The upload body: one wire model for both row formats.
pub type CreateActivityDto = Activity;

/// The heartbeat body (`HeartbeatDto`).
pub type HeartbeatDto = Heartbeat;

/// The heartbeat answer (`HeartbeatResultDto`).
pub type HeartbeatResultDto = HeartbeatResult;

impl From<&ActivityEnvelope> for Activity {
    /// The upload of a v1 row. Counts stay `None` (sent as `null`) when they
    /// were not measured; they are never fabricated as 0.
    fn from(env: &ActivityEnvelope) -> Self {
        let mut activity = Activity::new(
            env.project_id.as_deref().unwrap_or_default(),
            env.started_at,
            env.ended_at,
        );
        activity.client_activity_id = env.client_activity_id.clone();
        activity.productivity = env.productivity.map(|p| p.min(100) as u8);
        activity.mouse_clicks = env.mouse_clicks.map(u64::from);
        activity.keyboard_hits = env.keyboard_hits.map(u64::from);
        activity.description = env.description.clone();
        activity.device_id = env.device_id.clone();
        activity.platform = Some(env.platform.clone());
        activity.agent_version = Some(env.agent_version.clone());
        activity.payload_signature = env.payload_signature.clone();
        activity
    }
}

/// The upload of a v2 row: its stored payload plus the captures that still
/// carry bytes (base64), with their capture time and blur flag. `None` when
/// the payload cannot be read.
pub fn hydrate_v2_activity(
    payload_json: &str,
    captures: &[StoredCapture],
    signature: Option<&str>,
) -> Option<Activity> {
    let mut activity: Activity = serde_json::from_str(payload_json).ok()?;
    let b64 = |bytes: &[u8]| base64::engine::general_purpose::STANDARD.encode(bytes);
    for capture in captures.iter().filter(|c| c.has_bytes()) {
        match capture.kind {
            CaptureKind::Screen => {
                activity.screenshot_png_base64 = Some(b64(&capture.jpeg));
                activity.screenshot_captured_at = Some(capture.captured_at);
                activity.screenshot_blurred = Some(capture.blurred);
            }
            CaptureKind::Thumb => {
                activity.screenshot_thumb_jpg_base64 = Some(b64(&capture.jpeg));
                activity
                    .screenshot_captured_at
                    .get_or_insert(capture.captured_at);
                let blurred = activity.screenshot_blurred.unwrap_or(false) || capture.blurred;
                activity.screenshot_blurred = Some(blurred);
            }
        }
    }
    activity.payload_signature = signature.map(str::to_string);
    activity.sent_at = None;
    Some(activity)
}

/// Does a v2 row still verify: its signature against the payload and the
/// captures' digests, and every capture's bytes against its digest?
pub fn verify_v2_entry(key: &[u8], entry: &OutboxEntry) -> bool {
    let (Some(payload), Some(signature)) = (&entry.payload_json, &entry.payload_signature) else {
        return false;
    };
    entry.captures.iter().all(StoredCapture::bytes_match_digest)
        && verify_v2(
            key,
            payload.as_bytes(),
            &capture_digests(&entry.captures),
            signature,
        )
}

/// Maximum safe token TTL (1 year) to prevent Instant arithmetic overflows on malformed responses.
pub(crate) const MAX_TOKEN_TTL_SECS: u64 = 365 * 86400;

#[derive(Clone, Debug)]
pub(crate) struct GrantedToken {
    access_token: String,
    refresh_token: Option<String>,
    hard_expires_at: Instant,
    refresh_after: Instant,
    utc_expires_at: Option<DateTime<Utc>>,
}

impl GrantedToken {
    pub(crate) fn new(
        access_token: String,
        refresh_token: Option<String>,
        expires_in: Option<u64>,
        now_instant: Instant,
        now_utc: DateTime<Utc>,
    ) -> Self {
        match expires_in {
            Some(secs) if secs > 0 => {
                let safe_expires_in = secs.min(MAX_TOKEN_TTL_SECS);
                let hard_expires_at = now_instant
                    .checked_add(Duration::from_secs(safe_expires_in))
                    .unwrap_or_else(|| now_instant + Duration::from_secs(MAX_TOKEN_TTL_SECS));

                // Pre-expiry renewal buffer: up to 60s for standard tokens, proportional for short grants.
                // Ensures a 5s grant has an immediate valid window and refreshes at 4s before hard expiry at 5s,
                // never extending the token beyond its actual granted TTL.
                let renewal_buffer_secs = if safe_expires_in > 120 {
                    60
                } else if safe_expires_in > 1 {
                    (safe_expires_in / 5).max(1).min(safe_expires_in - 1)
                } else {
                    0
                };

                let refresh_after_secs = safe_expires_in.saturating_sub(renewal_buffer_secs);
                let refresh_after = now_instant
                    .checked_add(Duration::from_secs(refresh_after_secs))
                    .unwrap_or(hard_expires_at);

                let utc_expires_at =
                    now_utc.checked_add_signed(chrono::Duration::seconds(safe_expires_in as i64));

                Self {
                    access_token,
                    refresh_token,
                    hard_expires_at,
                    refresh_after,
                    utc_expires_at,
                }
            }
            _ => {
                // Expired or missing expiry: fails closed, immediately invalid without inventing arbitrary duration.
                Self {
                    access_token: String::new(),
                    refresh_token,
                    hard_expires_at: now_instant,
                    refresh_after: now_instant,
                    utc_expires_at: None,
                }
            }
        }
    }

    pub(crate) fn from_response(
        response: TokenResponse,
        previous_refresh: Option<String>,
        now_instant: Instant,
        now_utc: DateTime<Utc>,
    ) -> Self {
        let refresh_token = response
            .refresh_token
            .filter(|t| !t.is_empty())
            .or(previous_refresh);
        Self::new(
            response.access_token,
            refresh_token,
            response.expires_in,
            now_instant,
            now_utc,
        )
    }

    pub(crate) fn rehydrate(
        access_token: String,
        refresh_token: Option<String>,
        remaining_secs: u64,
        utc_expires_at: DateTime<Utc>,
        now_instant: Instant,
    ) -> Self {
        let safe_remaining = remaining_secs.min(MAX_TOKEN_TTL_SECS);
        let hard_expires_at = now_instant
            .checked_add(Duration::from_secs(safe_remaining))
            .unwrap_or_else(|| now_instant + Duration::from_secs(MAX_TOKEN_TTL_SECS));

        let renewal_buffer_secs = if safe_remaining > 120 {
            60
        } else if safe_remaining > 1 {
            (safe_remaining / 5).max(1).min(safe_remaining - 1)
        } else {
            0
        };

        let refresh_after_secs = safe_remaining.saturating_sub(renewal_buffer_secs);
        let refresh_after = now_instant
            .checked_add(Duration::from_secs(refresh_after_secs))
            .unwrap_or(hard_expires_at);

        Self {
            access_token,
            refresh_token,
            hard_expires_at,
            refresh_after,
            utc_expires_at: Some(utc_expires_at),
        }
    }

    pub(crate) fn is_valid_at(&self, now: Instant) -> bool {
        !self.access_token.is_empty() && now < self.hard_expires_at
    }

    pub(crate) fn needs_refresh_at(&self, now: Instant) -> bool {
        now >= self.refresh_after
    }
}

#[derive(Deserialize)]
pub(crate) struct TokenResponse {
    access_token: String,
    #[serde(default)]
    refresh_token: Option<String>,
    #[serde(default)]
    expires_in: Option<u64>,
}

type TokenClockFn = Arc<dyn Fn() -> (Instant, DateTime<Utc>) + Send + Sync>;

/// Called with a newly rotated refresh token (see [`TokenCell::set_rotation_listener`]).
pub type RotationListener = Arc<dyn Fn(String) + Send + Sync>;

struct TokenCellInner {
    token: std::sync::Mutex<Option<GrantedToken>>,
    renewing: TokioMutex<()>,
    time_offset_ms: AtomicU64,
    clock_override: std::sync::Mutex<Option<TokenClockFn>>,
    rotation_listener: std::sync::Mutex<Option<RotationListener>>,
}

/// The bearer token (and refresh token) of one signed-in user, shared by
/// every client acting for that user, so a renewal by one is seen by all and
/// a rotated refresh token is never used twice.
#[derive(Clone)]
pub struct TokenCell(Arc<TokenCellInner>);

impl Default for TokenCell {
    fn default() -> Self {
        Self(Arc::new(TokenCellInner {
            token: std::sync::Mutex::new(None),
            renewing: TokioMutex::new(()),
            time_offset_ms: AtomicU64::new(0),
            clock_override: std::sync::Mutex::new(None),
            rotation_listener: std::sync::Mutex::new(None),
        }))
    }
}

impl std::fmt::Debug for TokenCell {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("TokenCell(<hidden>)")
    }
}

impl TokenCell {
    /// Call `listener` whenever a grant returns a refresh token different from
    /// the previous one, so it can be persisted before it is needed again.
    /// The listener runs on the requesting task and must not block (send the
    /// token over a channel to a persister).
    pub fn set_rotation_listener(&self, listener: RotationListener) {
        *self
            .0
            .rotation_listener
            .lock()
            .unwrap_or_else(|e| e.into_inner()) = Some(listener);
    }

    /// Store a fresh grant and tell the rotation listener when its refresh
    /// token differs from the one held before.
    pub(crate) fn set_granted(&self, token: GrantedToken) {
        let rotated = {
            let mut slot = self.lock();
            let previous = slot.as_ref().and_then(|t| t.refresh_token.clone());
            let rotated = token
                .refresh_token
                .clone()
                .filter(|new| previous.as_deref() != Some(new.as_str()));
            *slot = Some(token);
            rotated
        };
        if let Some(new_refresh) = rotated {
            let listener = self
                .0
                .rotation_listener
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .clone();
            if let Some(listener) = listener {
                listener(new_refresh);
            }
        }
    }

    /// Is `other` this very cell (a clone of it), not just an equal token?
    pub fn same_cell(&self, other: &TokenCell) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }

    pub fn with_refresh_token(refresh_token: Option<String>) -> Self {
        let cell = Self::default();
        if let Some(refresh_token) = refresh_token.filter(|t| !t.is_empty()) {
            let (now_instant, _) = cell.current_time();
            cell.set(GrantedToken {
                access_token: String::new(),
                refresh_token: Some(refresh_token),
                hard_expires_at: now_instant,
                refresh_after: now_instant,
                utc_expires_at: None,
            });
        }
        cell
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Option<GrantedToken>> {
        self.0.token.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub(crate) fn set(&self, token: GrantedToken) {
        *self.lock() = Some(token);
    }

    pub fn current_time(&self) -> (Instant, DateTime<Utc>) {
        if let Ok(guard) = self.0.clock_override.lock() {
            if let Some(ref clock) = *guard {
                return clock();
            }
        }
        let offset_ms = self.0.time_offset_ms.load(Ordering::Relaxed);
        let base_instant = Instant::now();
        let base_utc = Utc::now();
        if offset_ms == 0 {
            (base_instant, base_utc)
        } else {
            let dur = Duration::from_millis(offset_ms);
            let instant = base_instant.checked_add(dur).unwrap_or(base_instant);
            let utc = base_utc
                .checked_add_signed(chrono::Duration::milliseconds(offset_ms as i64))
                .unwrap_or(base_utc);
            (instant, utc)
        }
    }

    pub fn advance_time(&self, duration: Duration) {
        self.0
            .time_offset_ms
            .fetch_add(duration.as_millis() as u64, Ordering::Relaxed);
    }

    pub fn set_clock<F>(&self, clock: F)
    where
        F: Fn() -> (Instant, DateTime<Utc>) + Send + Sync + 'static,
    {
        if let Ok(mut guard) = self.0.clock_override.lock() {
            *guard = Some(Arc::new(clock));
        }
    }

    pub fn refresh_token(&self) -> Option<String> {
        self.lock().as_ref().and_then(|t| t.refresh_token.clone())
    }

    pub fn valid_access_token(&self) -> Option<String> {
        let (now, _) = self.current_time();
        self.valid_access_token_at(now)
    }

    pub fn valid_access_token_at(&self, now: Instant) -> Option<String> {
        self.lock()
            .as_ref()
            .filter(|t| t.is_valid_at(now))
            .map(|t| t.access_token.clone())
    }

    pub fn is_valid(&self) -> bool {
        let (now, _) = self.current_time();
        self.lock()
            .as_ref()
            .map(|t| t.is_valid_at(now))
            .unwrap_or(false)
    }

    pub fn needs_refresh(&self) -> bool {
        let (now, _) = self.current_time();
        self.needs_refresh_at(now)
    }

    pub fn needs_refresh_at(&self, now: Instant) -> bool {
        self.lock()
            .as_ref()
            .map(|t| t.needs_refresh_at(now))
            .unwrap_or(true)
    }

    pub fn access_token_expires_at(&self) -> Option<DateTime<Utc>> {
        self.lock().as_ref().and_then(|t| t.utc_expires_at)
    }

    pub fn hard_expires_at(&self) -> Option<Instant> {
        self.lock().as_ref().map(|t| t.hard_expires_at)
    }

    pub fn refresh_after(&self) -> Option<Instant> {
        self.lock().as_ref().map(|t| t.refresh_after)
    }

    pub fn invalidate(&self, current: &str) {
        let (now_instant, _) = self.current_time();
        if let Some(token) = self.lock().as_mut().filter(|t| t.access_token == current) {
            token.access_token.clear();
            token.hard_expires_at = now_instant;
            token.refresh_after = now_instant;
            token.utc_expires_at = None;
        }
    }
}

/// Concrete HTTP REST API Client for Dosi-Tracker.
///
/// Authentication uses the backend's OpenIddict token endpoint
/// (`POST /connect/token`, no tenant query parameter, with the install's
/// `device_id`).
/// The token is renewed with the refresh token shortly before it expires and
/// once when a request answers 401; only when the refresh token is refused
/// does it sign in again with the saved password (if one is remembered).
/// Workspace-scoped requests carry [`WORKSPACE_HEADER`]; the workspace list
/// and the token endpoint never do.
pub struct ApiClient {
    http: reqwest::Client,
    base_url: String,
    username: String,
    /// The remembered password; empty when none is kept.
    password: Zeroizing<String>,
    workspace_id: Option<String>,
    device_id: Option<String>,
    signing_key: Option<[u8; 32]>,
    allow_insecure_http: bool,
    tokens: TokenCell,
    clock: ServerClock,
}

impl ApiClient {
    pub fn new(base_url: impl Into<String>, username: String, password: String) -> Self {
        let base_url = base_url.into().trim().trim_end_matches('/').to_string();
        Self {
            http: build_http_client(),
            base_url,
            username,
            password: Zeroizing::new(password),
            workspace_id: None,
            device_id: None,
            signing_key: None,
            allow_insecure_http: false,
            tokens: TokenCell::default(),
            clock: ServerClock::default(),
        }
    }

    /// Use `password` (or none) for re-authentication when the refresh token
    /// is refused.
    pub fn with_password(mut self, password: Option<Zeroizing<String>>) -> Self {
        self.password = password.unwrap_or_default();
        self
    }

    pub fn with_workspace(mut self, workspace_id: Option<String>) -> Self {
        self.workspace_id = workspace_id;
        self
    }

    /// The workspace workspace-scoped requests act in from now on.
    pub fn set_workspace(&mut self, workspace_id: Option<String>) {
        self.workspace_id = workspace_id;
    }

    /// The install's random device id, sent with every token request.
    pub fn with_device_id(mut self, device_id: Option<String>) -> Self {
        self.device_id = device_id.filter(|id| !id.trim().is_empty());
        self
    }

    pub fn with_signing_key(mut self, key: Option<[u8; 32]>) -> Self {
        self.signing_key = key;
        self
    }

    /// Allow plain `http://` to a non-loopback server (explicit admin opt-in).
    pub fn with_insecure_http(mut self, allow: bool) -> Self {
        self.allow_insecure_http = allow;
        self
    }

    pub fn with_server_clock(mut self, clock: ServerClock) -> Self {
        self.clock = clock;
        self
    }

    pub fn with_tokens(mut self, tokens: TokenCell) -> Self {
        self.tokens = tokens;
        self
    }

    /// Constructs an `ApiClient` directly from persisted credentials.
    ///
    /// Reuses persisted tokens, workspace scope, device id, device signing key,
    /// and server URL without requiring re-prompting for raw passwords. There
    /// is no implicit default server: credentials without one are refused, as
    /// is a plain `http://` server that is not loopback.
    pub fn from_credentials(creds: &Credentials) -> Result<Self, String> {
        let base_url = creds
            .server_url
            .as_deref()
            .map(str::trim)
            .filter(|url| !url.is_empty())
            .ok_or_else(|| {
                "The saved sign-in names no server; sign in again with --server.".to_string()
            })?;
        check_server_url(base_url, false)?;
        let mut client = Self::new(
            base_url,
            creds.account_id.clone(),
            creds.password.clone().unwrap_or_default(),
        )
        .with_device_id(creds.device_id.clone());
        if let Some(ref ws) = creds.workspace_id {
            client = client.with_workspace(Some(ws.clone()));
        }
        if let Some(ref key) = creds.device_signing_key {
            let key_bytes: [u8; 32] = key
                .as_slice()
                .try_into()
                .map_err(|_| "Invalid device signing key length: expected 32 bytes".to_string())?;
            client = client.with_signing_key(Some(key_bytes));
        }

        let cell = TokenCell::with_refresh_token(creds.refresh_token.clone());
        if let Some(ref access_token) = creds.access_token {
            if !access_token.trim().is_empty() {
                if let Some(expires_at) = creds.access_token_expires_at {
                    let now = Utc::now();
                    if expires_at > now {
                        let remaining_secs = (expires_at - now).num_seconds().max(0) as u64;
                        if remaining_secs > 0 {
                            let (now_instant, _) = cell.current_time();
                            cell.set(GrantedToken::rehydrate(
                                access_token.clone(),
                                creds.refresh_token.clone(),
                                remaining_secs,
                                expires_at,
                                now_instant,
                            ));
                        }
                    }
                }
            }
        }
        client = client.with_tokens(cell);
        Ok(client)
    }

    /// Loads credentials from the specified store and constructs an `ApiClient`.
    pub fn from_store(
        store: &dyn CredentialStore,
        account_id: &str,
        workspace_id: Option<&str>,
    ) -> Result<Self, String> {
        let creds = store
            .load(account_id, workspace_id)
            .map_err(|e| format!("Failed to load credentials from store: {e}"))?
            .ok_or_else(|| {
                "No stored credentials found for the specified account and workspace".to_string()
            })?;
        Self::from_credentials(&creds)
    }

    /// Overrides or sets the target API server base URL.
    pub fn with_base_url(mut self, base_url: impl Into<String>) -> Self {
        self.base_url = base_url.into().trim().trim_end_matches('/').to_string();
        self
    }

    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    pub fn username(&self) -> &str {
        &self.username
    }

    pub fn workspace_id(&self) -> Option<&str> {
        self.workspace_id.as_deref()
    }

    pub fn device_id(&self) -> Option<&str> {
        self.device_id.as_deref()
    }

    pub fn server_clock(&self) -> ServerClock {
        self.clock.clone()
    }

    pub fn tokens(&self) -> TokenCell {
        self.tokens.clone()
    }

    /// May this client talk to its server at all (see [`check_server_url`])?
    fn transport_allowed(&self) -> Result<(), String> {
        check_server_url(&self.base_url, self.allow_insecure_http)
    }

    async fn execute(
        &self,
        request: reqwest::RequestBuilder,
    ) -> Result<reqwest::Response, reqwest::Error> {
        let response = request.send().await?;
        let offset = response
            .headers()
            .get(DATE)
            .and_then(|date| date.to_str().ok())
            .and_then(|date| clock_offset_from_date(date, Utc::now()));
        if let Some(offset) = offset {
            self.clock.record(offset);
        }
        Ok(response)
    }

    /// `POST /connect/token` with `params` (plus the device id); the token is
    /// stored on success. Never carries a tenant query parameter or a
    /// workspace header.
    async fn token_request(
        &mut self,
        params: &[(&str, &str)],
        previous_refresh: Option<String>,
    ) -> Result<(), AuthError> {
        self.transport_allowed().map_err(AuthError::Server)?;
        let url = format!("{}/connect/token", self.base_url);
        let mut form: Vec<(&str, &str)> = params.to_vec();
        if let Some(device_id) = self.device_id.as_deref() {
            form.push(("device_id", device_id));
        }

        let response = self
            .execute(
                self.http
                    .post(&url)
                    .header(ACCEPT, "application/json")
                    .form(&form),
            )
            .await
            .map_err(|e| AuthError::Offline(describe_error(&e)))?;

        let status = response.status();
        let body = Zeroizing::new(
            response
                .text()
                .await
                .map_err(|e| AuthError::Offline(describe_error(&e)))?,
        );

        if !status.is_success() {
            return Err(classify_token_failure(status.as_u16(), &body));
        }

        let token_resp: TokenResponse = serde_json::from_str(&body)
            .map_err(|e| AuthError::Server(format!("Failed to parse token response: {e}")))?;

        let (now_instant, now_utc) = self.tokens.current_time();
        self.tokens.set_granted(GrantedToken::from_response(
            token_resp,
            previous_refresh,
            now_instant,
            now_utc,
        ));
        Ok(())
    }

    pub async fn login(&mut self) -> Result<(), AuthError> {
        let username = self.username.clone();
        let password = self.password.clone();
        let params = [
            ("grant_type", "password"),
            ("client_id", CLIENT_ID),
            ("username", username.as_str()),
            ("password", password.as_str()),
            ("scope", SCOPE),
        ];
        self.token_request(&params, None).await
    }

    pub async fn refresh(&mut self, refresh_token: &str) -> Result<(), AuthError> {
        let params = [
            ("grant_type", "refresh_token"),
            ("client_id", CLIENT_ID),
            ("refresh_token", refresh_token),
        ];
        self.token_request(&params, Some(refresh_token.to_string()))
            .await
    }

    pub async fn ensure_token(&mut self) -> Result<String, AuthError> {
        let (now_instant, _) = self.tokens.current_time();
        if let Some(token) = self.tokens.valid_access_token_at(now_instant) {
            if !self.tokens.needs_refresh_at(now_instant) {
                return Ok(token);
            }
        }

        let slot = self.tokens.0.clone();
        let _guard = slot.renewing.lock().await;

        let (now_instant, _) = self.tokens.current_time();
        if let Some(token) = self.tokens.valid_access_token_at(now_instant) {
            if !self.tokens.needs_refresh_at(now_instant) {
                return Ok(token);
            }
        }

        if let Some(refresh_token) = self.tokens.refresh_token() {
            match self.refresh(&refresh_token).await {
                Ok(()) => {}
                Err(AuthError::Rejected(_)) => {
                    // The refresh was refused (OpenIddict `invalid_grant`): per the
                    // cross-lane contract this always means the sign-in has expired,
                    // never a cue to silently re-authenticate with a remembered
                    // password. Propagate it so the caller enters the SignInExpired
                    // state and requires explicit re-sign-in (or a workspace switch).
                    return Err(AuthError::Rejected(
                        "Session expired. Please sign in again.".into(),
                    ));
                }
                Err(e) => {
                    let (now_instant, _) = self.tokens.current_time();
                    if let Some(token) = self.tokens.valid_access_token_at(now_instant) {
                        return Ok(token);
                    }
                    return Err(e);
                }
            }
        } else if self.password.is_empty() {
            return Err(AuthError::Rejected(
                "Credentials required. Please sign in.".into(),
            ));
        } else {
            self.login().await?;
        }

        let (now_instant, _) = self.tokens.current_time();
        self.tokens
            .valid_access_token_at(now_instant)
            .ok_or_else(|| {
                AuthError::Server("No valid access token available after authentication".into())
            })
    }

    /// Send an authorized request built by `build` (given the HTTP client and
    /// a bearer token), with the workspace header when `scoped`. On a 401 the
    /// token was revoked or expired server-side: renew it once and resend.
    /// A second 401 is returned as is.
    async fn send<F>(&mut self, scoped: bool, build: F) -> Result<reqwest::Response, ApiError>
    where
        F: Fn(&reqwest::Client, &str) -> reqwest::RequestBuilder,
    {
        self.transport_allowed().map_err(ApiError::Invalid)?;
        let workspace = if scoped {
            let workspace = self
                .workspace_id
                .clone()
                .filter(|ws| !ws.trim().is_empty())
                .ok_or_else(|| ApiError::WorkspaceRequired("No workspace selected.".into()))?;
            Some(workspace)
        } else {
            None
        };
        let with_scope = |request: reqwest::RequestBuilder| match workspace.as_deref() {
            Some(ws) => request.header(WORKSPACE_HEADER, ws),
            None => request,
        };

        let token = self.ensure_token().await?;
        let response = self
            .execute(with_scope(build(&self.http, &token)))
            .await
            .map_err(|e| ApiError::Offline(describe_error(&e)))?;
        if response.status() != StatusCode::UNAUTHORIZED {
            return Ok(response);
        }

        // Bounded retry: invalidate current token, obtain fresh token, retry once
        self.tokens.invalidate(&token);
        let fresh_token = self.ensure_token().await?;
        let response = self
            .execute(with_scope(build(&self.http, &fresh_token)))
            .await
            .map_err(|e| ApiError::Offline(describe_error(&e)))?;
        if response.status() == StatusCode::UNAUTHORIZED {
            self.tokens.invalidate(&fresh_token);
        }
        Ok(response)
    }

    /// Sends an authorized request with the workspace header (single retry on HTTP 401).
    pub async fn send_scoped<F>(&mut self, build: F) -> Result<reqwest::Response, ApiError>
    where
        F: Fn(&reqwest::Client, &str) -> reqwest::RequestBuilder,
    {
        self.send(true, build).await
    }

    /// Sends an authorized request without the workspace header (single retry on HTTP 401).
    pub async fn send_unscoped<F>(&mut self, build: F) -> Result<reqwest::Response, ApiError>
    where
        F: Fn(&reqwest::Client, &str) -> reqwest::RequestBuilder,
    {
        self.send(false, build).await
    }

    /// The workspaces this user belongs to (`GET /api/app/workspaces/mine`,
    /// bearer only). Archived ones are listed but cannot be chosen.
    pub async fn workspaces_mine(&mut self) -> Result<Vec<Workspace>, ApiError> {
        let url = format!("{}/api/app/workspaces/mine", self.base_url);
        let response = self
            .send_unscoped(|http, token| http.get(&url).bearer_auth(token))
            .await?;
        read_json(response).await
    }

    /// The projects the user may track in the current workspace, with their
    /// capture policy (`GET /api/app/project/my-projects`).
    pub async fn my_projects(&mut self) -> Result<Vec<Project>, ApiError> {
        let url = format!("{}/api/app/project/my-projects", self.base_url);
        let response = self
            .send_scoped(|http, token| http.get(&url).bearer_auth(token))
            .await?;
        read_json(response).await
    }

    /// Tracked-time totals for a window (`GET /api/app/reporting/summary`),
    /// in `time_zone`'s calendar days when given. The query is percent-encoded.
    pub async fn summary(
        &mut self,
        from: DateTime<Utc>,
        to: DateTime<Utc>,
        time_zone: Option<&str>,
    ) -> Result<ReportSummary, ApiError> {
        let url = format!("{}/api/app/reporting/summary", self.base_url);
        let query = summary_query(from, to, time_zone);
        let response = self
            .send_scoped(|http, token| http.get(&url).query(&query).bearer_auth(token))
            .await?;
        read_json(response).await
    }

    /// Whether this device was revoked in the current workspace
    /// (`GET /api/app/device/{deviceId}/status`).
    pub async fn device_status(&mut self, device_id: &str) -> Result<DeviceStatus, ApiError> {
        let device_id = uuid::Uuid::parse_str(device_id.trim())
            .map_err(|_| ApiError::Invalid("The device id is not a UUID.".into()))?;
        let url = format!("{}/api/app/device/{device_id}/status", self.base_url);
        let response = self
            .send_scoped(|http, token| http.get(&url).bearer_auth(token))
            .await?;
        read_json(response).await
    }

    /// Report presence (`POST /api/app/presence/heartbeat`). The whole call, a
    /// sign-in included, is bounded by `timeout`. `sentAt` is stamped per
    /// attempt. A failed heartbeat is never retried or queued.
    pub async fn heartbeat(
        &mut self,
        heartbeat: &Heartbeat,
        timeout: Duration,
    ) -> Result<HeartbeatResult, ApiError> {
        let url = format!("{}/api/app/presence/heartbeat", self.base_url);
        let body = heartbeat.clone().normalized();
        let call = async {
            let response = self
                .send_scoped(|http, token| {
                    let mut body = body.clone();
                    body.sent_at = Some(Utc::now());
                    http.post(&url)
                        .bearer_auth(token)
                        .timeout(timeout)
                        .json(&body)
                })
                .await?;
            let status = response.status();
            if !status.is_success() {
                let text = response.text().await.unwrap_or_default();
                return Err(http_error(status.as_u16(), &text));
            }
            // The heartbeat counted even if its answer cannot be read.
            let text = response.text().await.unwrap_or_default();
            Ok(serde_json::from_str(&text).unwrap_or_default())
        };
        tokio::time::timeout(timeout, call)
            .await
            .unwrap_or_else(|_| {
                Err(ApiError::Offline(format!(
                    "no answer within {} ms",
                    timeout.as_millis()
                )))
            })
    }

    /// Upload a completed block to the current workspace (`POST
    /// /api/app/activity`). `sentAt` is stamped per attempt; `signature`
    /// (when given) is sent as `payloadSignature`.
    pub async fn submit_activity(
        &mut self,
        activity: &Activity,
        signature: Option<&str>,
    ) -> Result<(), SubmitError> {
        let url = format!("{}/api/app/activity", self.base_url);
        let mut body = activity.clone();
        if let Some(signature) = signature {
            body.payload_signature = Some(signature.to_string());
        }
        let response = self
            .send_scoped(|http, token| {
                let mut body = body.clone();
                body.sent_at = Some(Utc::now());
                http.post(&url)
                    .bearer_auth(token)
                    .timeout(UPLOAD_TIMEOUT)
                    .json(&body)
            })
            .await;

        let response = match response {
            Ok(response) => response,
            Err(ApiError::Auth(AuthError::Rejected(reason))) => {
                return Err(SubmitError::AuthRejected(reason))
            }
            Err(ApiError::WorkspaceRequired(reason)) => {
                return Err(SubmitError::WorkspaceRequired(reason))
            }
            Err(other) => {
                return Err(SubmitError::Transient {
                    reason: other.to_string(),
                    retry_after: None,
                })
            }
        };

        let status = response.status();
        if status.is_success() {
            return Ok(());
        }
        if status == StatusCode::UNAUTHORIZED {
            // Signing in worked, yet the fresh token was refused: retry later.
            return Err(SubmitError::Transient {
                reason: "HTTP 401 after signing in again".to_string(),
                retry_after: None,
            });
        }
        let retry_after = retry_after_of(response.headers());
        let body = response.text().await.unwrap_or_default();
        Err(classify_upload_failure(status.as_u16(), &body, retry_after))
    }

    /// Upload a v1 row (`ActivityEnvelope`) through the shared wire model,
    /// signing it with the device key when it carries no signature yet.
    /// Returns the acknowledged `client_activity_id`.
    pub async fn submit_envelope(
        &mut self,
        envelope: &ActivityEnvelope,
    ) -> Result<String, SubmitError> {
        let mut signed = envelope.clone();
        if let (Some(key), None) = (self.signing_key, envelope.payload_signature.as_ref()) {
            sign_activity_envelope(&key, &mut signed)
                .map_err(|e| SubmitError::Rejected(format!("Failed to sign envelope: {e}")))?;
        }
        let activity = Activity::from(&signed);
        self.submit_activity(&activity, None).await?;
        Ok(envelope.client_activity_id.clone())
    }

    /// Deliver one queued row and translate the answer into the sync pass's
    /// outcome (design 8.4).
    async fn deliver_entry(&mut self, entry: &OutboxEntry) -> DeliveryOutcome {
        let (result, project_id) = if entry.is_v2() {
            let Some(key) = self.signing_key else {
                return DeliveryOutcome::RetryLater {
                    reason: "The device signing key is not available; sign in again.".into(),
                    kind: RetryKind::Transient,
                    retry_after: None,
                };
            };
            if !verify_v2_entry(&key, entry) {
                return DeliveryOutcome::Park {
                    reason: INTEGRITY_FAILURE_REASON.to_string(),
                };
            }
            let Some(activity) = entry.payload_json.as_deref().and_then(|payload| {
                hydrate_v2_activity(payload, &entry.captures, entry.payload_signature.as_deref())
            }) else {
                return DeliveryOutcome::Park {
                    reason: INTEGRITY_FAILURE_REASON.to_string(),
                };
            };
            let result = self.submit_activity(&activity, None).await;
            (result, activity.project_id)
        } else {
            let result = self.submit_envelope(&entry.envelope).await.map(|_| ());
            (
                result,
                entry.envelope.project_id.clone().unwrap_or_default(),
            )
        };

        match result {
            Ok(()) => DeliveryOutcome::Accepted {
                client_activity_id: entry.client_activity_id.clone(),
            },
            Err(SubmitError::ProjectUnavailable { code, message }) => {
                // Park only if the project is really gone for this user.
                match self.my_projects().await {
                    Ok(projects)
                        if !projects
                            .iter()
                            .any(|p| p.id.eq_ignore_ascii_case(&project_id)) =>
                    {
                        DeliveryOutcome::Park {
                            reason: format!("{code}: {message}"),
                        }
                    }
                    _ => DeliveryOutcome::RetryLater {
                        reason: format!("{code}: {message}"),
                        kind: RetryKind::Transient,
                        retry_after: None,
                    },
                }
            }
            Err(error) => delivery_outcome_of(error),
        }
    }
}

/// The sync pass's outcome for an upload failure (everything but the project
/// check, which needs the server).
pub fn delivery_outcome_of(error: SubmitError) -> DeliveryOutcome {
    match error {
        SubmitError::Transient {
            reason,
            retry_after,
        } => DeliveryOutcome::RetryLater {
            reason,
            kind: RetryKind::Transient,
            retry_after,
        },
        SubmitError::AuthRejected(reason) => DeliveryOutcome::AuthRequired { reason },
        SubmitError::SubscriptionInactive(message) => DeliveryOutcome::RetryLater {
            reason: message,
            kind: RetryKind::Subscription,
            retry_after: None,
        },
        SubmitError::ClockAhead(message) => DeliveryOutcome::RetryLater {
            reason: message,
            kind: RetryKind::ClockAhead,
            retry_after: None,
        },
        error @ (SubmitError::NotWorkspaceMember(_)
        | SubmitError::WorkspaceArchived(_)
        | SubmitError::WorkspaceRequired(_)) => DeliveryOutcome::WorkspaceAccessLost {
            reason: error.to_string(),
        },
        error @ SubmitError::DeviceRevoked(_) => DeliveryOutcome::DeviceRevoked {
            reason: error.to_string(),
        },
        error @ SubmitError::StripCaptures(_) => DeliveryOutcome::StripCaptures {
            reason: error.to_string(),
        },
        SubmitError::Rejected(message) => DeliveryOutcome::Park { reason: message },
        SubmitError::ProjectUnavailable { code, message } => DeliveryOutcome::RetryLater {
            reason: format!("{code}: {message}"),
            kind: RetryKind::Transient,
            retry_after: None,
        },
    }
}

enum WorkerMsg {
    Deliver {
        entry: Box<OutboxEntry>,
        reply: std::sync::mpsc::SyncSender<DeliveryOutcome>,
    },
    Shutdown,
}

/// Everything [`HttpDeliveryTransport::new_shared`] binds a transport to.
pub struct TransportConfig {
    /// The server (normalized; `https://`, or loopback / opted-in `http://`).
    pub server: String,
    /// The signed-in account.
    pub account: String,
    /// The remembered password, if any (Secret Service with remember_password).
    pub password: Option<Zeroizing<String>>,
    /// The account's shared tokens (renewals and rotations are seen by all clients).
    pub tokens: TokenCell,
    /// The workspace GUID this transport uploads to.
    pub workspace_id: String,
    /// The install's device id, sent with token requests.
    pub device_id: Option<String>,
    /// The device signing key v2 rows are verified with.
    pub signing_key: Option<[u8; 32]>,
    /// The server clock shared with the rest of the agent.
    pub clock: ServerClock,
    /// `allow_insecure_http` from the configuration.
    pub allow_insecure_http: bool,
}

/// Concrete HTTP Delivery Transport implementing [`DeliveryTransport`].
///
/// Runs delivery requests across a dedicated worker thread with an isolated
/// single-thread Tokio runtime, preventing runtime-nesting panics and guaranteeing
/// zero SQLite lock retention during HTTP network operations. Waits at most
/// [`UPLOAD_TIMEOUT`] + [`DELIVERY_GRACE`] for each delivery.
pub struct HttpDeliveryTransport {
    bound_scope: OutboxScope,
    sender: std::sync::mpsc::SyncSender<WorkerMsg>,
    worker: Option<std::thread::JoinHandle<()>>,
}

impl HttpDeliveryTransport {
    pub fn new(mut client: ApiClient) -> Result<Self, String> {
        let account_id = client.username();
        let raw_workspace_id = client.workspace_id().ok_or_else(|| {
            "HttpDeliveryTransport requires a bound workspace_id; unscoped transport is forbidden"
                .to_string()
        })?;

        let bound_scope = OutboxScope::new(account_id, raw_workspace_id)
            .map_err(|e| format!("Invalid transport identity scope: {e}"))?;

        // Ensure client fields match canonicalized bound scope
        client.username = bound_scope.account_id.clone();
        client.workspace_id = Some(bound_scope.workspace_id.clone());

        let (tx, rx) = std::sync::mpsc::sync_channel::<WorkerMsg>(32);
        let worker_scope = bound_scope.clone();

        let handle = std::thread::spawn(move || {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("Failed to initialize transport worker runtime");

            while let Ok(msg) = rx.recv() {
                match msg {
                    WorkerMsg::Deliver { entry, reply } => {
                        if entry.scope != worker_scope {
                            let outcome = DeliveryOutcome::Rejected {
                                reason: format!(
                                    "Worker scope mismatch: transport bound to ({}, {}) rejected entry for ({}, {})",
                                    worker_scope.account_id,
                                    worker_scope.workspace_id,
                                    entry.scope.account_id,
                                    entry.scope.workspace_id,
                                ),
                            };
                            let _ = reply.send(outcome);
                            continue;
                        }

                        let outcome = rt.block_on(client.deliver_entry(&entry));
                        let _ = reply.send(outcome);
                    }
                    WorkerMsg::Shutdown => break,
                }
            }
        });

        Ok(Self {
            bound_scope,
            sender: tx,
            worker: Some(handle),
        })
    }

    /// A transport for one workspace of the signed-in account, sharing the
    /// account's tokens and the agent's server clock. The workspace must be a
    /// GUID and the server must pass [`check_server_url`].
    pub fn new_shared(config: TransportConfig) -> Result<Self, String> {
        check_server_url(&config.server, config.allow_insecure_http)?;
        let workspace = uuid::Uuid::parse_str(config.workspace_id.trim())
            .map_err(|_| "The workspace id must be a GUID".to_string())?;
        let client = ApiClient::new(config.server, config.account, String::new())
            .with_password(config.password)
            .with_tokens(config.tokens)
            .with_server_clock(config.clock)
            .with_device_id(config.device_id)
            .with_signing_key(config.signing_key)
            .with_insecure_http(config.allow_insecure_http)
            .with_workspace(Some(workspace.to_string()));
        Self::new(client)
    }

    /// Constructs an `HttpDeliveryTransport` from persisted credentials.
    pub fn from_credentials(creds: &Credentials) -> Result<Self, String> {
        let ws = creds.workspace_id.as_deref().ok_or_else(|| {
            "HttpDeliveryTransport requires credentials with a bound workspace_id".to_string()
        })?;
        if ws.trim().is_empty() {
            return Err("HttpDeliveryTransport requires a non-empty workspace_id".to_string());
        }
        let client = ApiClient::from_credentials(creds)?;
        Self::new(client)
    }

    /// Loads credentials from the specified store and constructs an `HttpDeliveryTransport`.
    pub fn from_store(
        store: &dyn CredentialStore,
        account_id: &str,
        workspace_id: Option<&str>,
    ) -> Result<Self, String> {
        let ws = workspace_id.ok_or_else(|| {
            "HttpDeliveryTransport requires an explicit bound workspace_id".to_string()
        })?;
        if ws.trim().is_empty() {
            return Err("HttpDeliveryTransport requires a non-empty workspace_id".to_string());
        }
        let client = ApiClient::from_store(store, account_id, Some(ws))?;
        Self::new(client)
    }

    /// Returns the bound [`OutboxScope`] of this transport.
    pub fn bound_scope(&self) -> &OutboxScope {
        &self.bound_scope
    }

    /// Returns the bound account identifier.
    pub fn bound_account_id(&self) -> &str {
        &self.bound_scope.account_id
    }

    /// Returns the bound workspace identifier.
    pub fn bound_workspace_id(&self) -> &str {
        &self.bound_scope.workspace_id
    }

    /// Checks if a candidate entry scope matches this transport's bound scope under
    /// exact canonicalization rules.
    pub fn matches_scope(&self, entry_scope: &OutboxScope) -> bool {
        if entry_scope.validate().is_err() {
            return false;
        }
        self.bound_scope.account_id == entry_scope.account_id.trim().to_lowercase()
            && self.bound_scope.workspace_id == entry_scope.workspace_id.trim()
    }
}

impl DeliveryTransport for HttpDeliveryTransport {
    fn deliver(&mut self, entry: &OutboxEntry) -> DeliveryOutcome {
        if !self.matches_scope(&entry.scope) {
            return DeliveryOutcome::Rejected {
                reason: format!(
                    "Scope mismatch: transport bound to scope ({}, {}) rejected entry for ({}, {})",
                    self.bound_scope.account_id,
                    self.bound_scope.workspace_id,
                    entry.scope.account_id,
                    entry.scope.workspace_id,
                ),
            };
        }

        let (reply_tx, reply_rx) = std::sync::mpsc::sync_channel(1);
        if self
            .sender
            .send(WorkerMsg::Deliver {
                entry: Box::new(entry.clone()),
                reply: reply_tx,
            })
            .is_err()
        {
            return DeliveryOutcome::TransientFailure {
                reason: "Transport worker thread disconnected".to_string(),
            };
        }

        match reply_rx.recv_timeout(UPLOAD_TIMEOUT + DELIVERY_GRACE) {
            Ok(outcome) => outcome,
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => DeliveryOutcome::RetryLater {
                reason: format!(
                    "The upload did not finish within {} s",
                    (UPLOAD_TIMEOUT + DELIVERY_GRACE).as_secs()
                ),
                kind: RetryKind::Transient,
                retry_after: None,
            },
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                DeliveryOutcome::TransientFailure {
                    reason: "Transport worker failed to return outcome".to_string(),
                }
            }
        }
    }
}

impl Drop for HttpDeliveryTransport {
    fn drop(&mut self) {
        let _ = self.sender.send(WorkerMsg::Shutdown);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

impl std::fmt::Debug for HttpDeliveryTransport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HttpDeliveryTransport")
            .field("bound_scope", &self.bound_scope)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// ABP's tenant query parameter, which the agent must never send (spelled
    /// in pieces so the source never carries the literal).
    const TENANT_PARAM: &str = concat!("__", "tenant");
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use tempfile::tempdir;

    use crate::outbox::{Outbox, OutboxLimits, OutboxScope};
    use crate::signing::generate_signing_key;
    use crate::sync::{SyncPassConfig, SyncPassCoordinator};

    fn http_resp(status: &str, headers: &[(&str, &str)], body: &str) -> String {
        let extra: String = headers
            .iter()
            .map(|(k, v)| format!("{k}: {v}\r\n"))
            .collect();
        format!(
            "HTTP/1.1 {status}\r\nContent-Length: {}\r\nContent-Type: application/json\r\nConnection: close\r\n{extra}\r\n{body}",
            body.len()
        )
    }

    fn token_ok() -> String {
        http_resp(
            "200 OK",
            &[],
            r#"{"access_token":"test-access-token","token_type":"Bearer","expires_in":3600,"refresh_token":"test-refresh-token"}"#,
        )
    }

    fn read_request(stream: &mut std::net::TcpStream) -> String {
        let mut data = Vec::new();
        let mut buf = [0u8; 8192];
        loop {
            let n = stream.read(&mut buf).unwrap_or(0);
            if n == 0 {
                break;
            }
            data.extend_from_slice(&buf[..n]);
            let text = String::from_utf8_lossy(&data);
            if let Some(end) = text.find("\r\n\r\n") {
                let length = text[..end]
                    .lines()
                    .find_map(|line| {
                        let (name, value) = line.split_once(':')?;
                        name.eq_ignore_ascii_case("content-length")
                            .then(|| value.trim().parse::<usize>().ok())?
                    })
                    .unwrap_or(0);
                if data.len() >= end + 4 + length {
                    break;
                }
            }
        }
        String::from_utf8_lossy(&data).into_owned()
    }

    fn canned_server(responses: Vec<String>) -> (String, std::sync::mpsc::Receiver<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            for response in responses {
                let Ok((mut stream, _)) = listener.accept() else {
                    return;
                };
                let _ = tx.send(read_request(&mut stream));
                let _ = stream.write_all(response.as_bytes());
                let _ = stream.flush();
            }
        });
        (url, rx)
    }

    fn sample_envelope(client_activity_id: &str) -> ActivityEnvelope {
        let now = Utc::now();
        ActivityEnvelope {
            format_version: 1,
            client_activity_id: client_activity_id.to_string(),
            project_id: Some("proj-uuid-1".to_string()),
            started_at: now - chrono::Duration::minutes(10),
            ended_at: now,
            productivity: Some(85),
            mouse_clicks: Some(42),
            keyboard_hits: Some(108),
            description: Some("Implementing Phase 1 transport".to_string()),
            device_id: Some("dev-uuid-1".to_string()),
            platform: "linux".to_string(),
            agent_version: "0.1.0".to_string(),
            payload_signature: None,
        }
    }

    #[test]
    fn test_client_id_and_scope_invariants() {
        assert_eq!(CLIENT_ID, "Tracker_App");
        assert!(SCOPE.contains("offline_access"));
        assert!(SCOPE.contains("Tracker"));
        assert_eq!(WORKSPACE_HEADER, "X-Workspace-Id");
    }

    #[test]
    fn test_classify_token_failure_variants() {
        let invalid_grant =
            r#"{"error":"invalid_grant","error_description":"Invalid username or password"}"#;
        assert_eq!(
            classify_token_failure(400, invalid_grant),
            AuthError::Rejected("Invalid username or password".to_string())
        );

        assert_eq!(
            classify_token_failure(401, ""),
            AuthError::Rejected("Incorrect username or password.".to_string())
        );

        assert_eq!(
            classify_token_failure(403, ""),
            AuthError::Rejected("Incorrect username or password.".to_string())
        );

        assert!(matches!(
            classify_token_failure(429, ""),
            AuthError::Server(_)
        ));
        assert!(matches!(
            classify_token_failure(500, ""),
            AuthError::Server(_)
        ));
    }

    #[test]
    fn test_classify_upload_failure_business_codes() {
        let overlap = r#"{"error":{"code":"Tracker:ActivityOverlaps","message":"Block overlaps another block"}}"#;
        match classify_upload_failure(409, overlap, None) {
            SubmitError::Rejected(msg) => assert!(msg.contains("overlaps")),
            other => panic!("Expected Rejected, got {other:?}"),
        }

        let conflict = r#"{"error":{"code":"Tracker:ActivityClientIdConflict","message":"ID already exists"}}"#;
        match classify_upload_failure(409, conflict, None) {
            SubmitError::Rejected(msg) => assert!(msg.contains("exists")),
            other => panic!("Expected Rejected, got {other:?}"),
        }

        let validation = r#"{"error":{"code":"Volo.Abp.Validation","message":"Validation failed","validationErrors":[{"message":"Duration is too long"}]}}"#;
        match classify_upload_failure(400, validation, None) {
            SubmitError::Rejected(msg) => {
                assert!(msg.contains("Validation failed"));
                assert!(msg.contains("Duration is too long"));
            }
            other => panic!("Expected Rejected, got {other:?}"),
        }

        match classify_upload_failure(500, "Internal error", None) {
            SubmitError::Transient { reason, .. } => assert!(reason.contains("500")),
            other => panic!("Expected Transient, got {other:?}"),
        }
    }

    #[test]
    fn test_clock_offset_from_date_and_skew_minutes() {
        let device = DateTime::parse_from_rfc3339("2026-10-24T10:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let offset = clock_offset_from_date("Sat, 24 Oct 2026 10:03:00 GMT", device);
        assert_eq!(offset, Some(180));

        let skew = skew_warning_minutes(180);
        assert_eq!(skew, Some(3));

        let small_skew = skew_warning_minutes(60);
        assert_eq!(small_skew, None);
    }

    #[test]
    fn test_parse_retry_after() {
        let now = DateTime::parse_from_rfc3339("2026-10-24T10:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        assert_eq!(
            parse_retry_after("120", None, now),
            Some(Duration::from_secs(120))
        );
        assert_eq!(parse_retry_after("0", None, now), Some(Duration::ZERO));
        assert_eq!(parse_retry_after("invalid", None, now), None);
    }

    #[tokio::test]
    async fn test_login_flow_canned_server() {
        let (url, rx) = canned_server(vec![token_ok()]);
        let mut client = ApiClient::new(&url, "user@dosi.test".into(), "test-pass".into())
            .with_workspace(Some("ws-1".into()));

        let res = client.login().await;
        assert!(res.is_ok(), "Login should succeed: {:?}", res.err());

        let req = rx.recv().unwrap();
        assert!(req.starts_with("POST /connect/token "), "{req}");
        assert!(!req.to_lowercase().contains(TENANT_PARAM), "{req}");
        assert!(!req.to_lowercase().contains("x-workspace-id"), "{req}");
        assert!(req.contains("grant_type=password"));
        assert!(req.contains("client_id=Tracker_App"));
        assert!(req.contains("username=user%40dosi.test"));

        let token = client.ensure_token().await.unwrap();
        assert_eq!(token, "test-access-token");
    }

    #[tokio::test]
    async fn test_refresh_token_pre_expiry_flow() {
        let (url, rx) = canned_server(vec![
            token_ok(),
            http_resp(
                "200 OK",
                &[],
                r#"{"access_token":"refreshed-token","token_type":"Bearer","expires_in":3600,"refresh_token":"new-refresh"}"#,
            ),
        ]);

        let mut client = ApiClient::new(&url, "user@dosi.test".into(), "test-pass".into());
        client.login().await.unwrap();
        let _ = rx.recv().unwrap();

        // Invalidate access token to force refresh
        client.tokens.invalidate("test-access-token");

        let refreshed = client.ensure_token().await.unwrap();
        assert_eq!(refreshed, "refreshed-token");

        let refresh_req = rx.recv().unwrap();
        assert!(refresh_req.contains("grant_type=refresh_token"));
        assert!(refresh_req.contains("refresh_token=test-refresh-token"));
    }

    #[tokio::test]
    async fn test_bounded_401_retry_flow() {
        let (url, rx) = canned_server(vec![
            token_ok(),
            // Activity request receives 401
            http_resp("401 Unauthorized", &[], r#"{"error":"invalid_token"}"#),
            // Client automatically refreshes
            http_resp(
                "200 OK",
                &[],
                r#"{"access_token":"fresh-after-401","token_type":"Bearer","expires_in":3600}"#,
            ),
            // Retry succeeds
            http_resp("200 OK", &[], r#"{"clientActivityId":"act-401"}"#),
        ]);

        let mut client = ApiClient::new(&url, "user@dosi.test".into(), "test-pass".into())
            .with_workspace(Some("ws-1".into()));

        let env = sample_envelope("act-401");
        let result = client.submit_envelope(&env).await;
        assert_eq!(result.unwrap(), "act-401");

        // Inspect sequence: login -> failed submit -> refresh -> successful submit
        let _login_req = rx.recv().unwrap();
        let submit_req1 = rx.recv().unwrap();
        assert!(submit_req1.contains("Bearer test-access-token"));
        let refresh_req = rx.recv().unwrap();
        assert!(refresh_req.contains("grant_type=refresh_token"));
        let submit_req2 = rx.recv().unwrap();
        assert!(submit_req2.contains("Bearer fresh-after-401"));
    }

    #[tokio::test]
    async fn test_bounded_401_retry_exhaustion_fails() {
        let (url, _rx) = canned_server(vec![
            token_ok(),
            // Activity request receives 401
            http_resp("401 Unauthorized", &[], r#"{"error":"invalid_token"}"#),
            // Refresh also fails with invalid_grant
            http_resp(
                "400 Bad Request",
                &[],
                r#"{"error":"invalid_grant","error_description":"Token revoked"}"#,
            ),
        ]);

        let mut client = ApiClient::new(&url, "user@dosi.test".into(), "".into())
            .with_workspace(Some("ws-1".into()));

        let env = sample_envelope("act-fail");
        let result = client.submit_envelope(&env).await;
        assert!(matches!(result, Err(SubmitError::AuthRejected(_))));
    }

    #[tokio::test]
    async fn test_submit_activity_with_signing_and_exact_ack() {
        let key = generate_signing_key();
        let (url, rx) = canned_server(vec![
            token_ok(),
            http_resp("200 OK", &[], r#"{"clientActivityId":"act-signed-1"}"#),
        ]);

        let mut client = ApiClient::new(&url, "user@dosi.test".into(), "test-pass".into())
            .with_workspace(Some("ws-1".into()))
            .with_signing_key(Some(key));

        let env = sample_envelope("act-signed-1");
        let ack_id = client.submit_envelope(&env).await.unwrap();
        assert_eq!(ack_id, "act-signed-1");

        let _login_req = rx.recv().unwrap();
        let submit_req = rx.recv().unwrap();
        assert!(submit_req.to_lowercase().contains("x-workspace-id: ws-1"));
        assert!(submit_req.contains("payloadSignature"));
    }

    #[tokio::test]
    async fn test_heartbeat_sends_contract_and_reads_server_time() {
        let (url, rx) = canned_server(vec![
            token_ok(),
            http_resp("200 OK", &[], r#"{"serverTime":"2026-10-24T10:00:00Z"}"#),
        ]);

        let mut client = ApiClient::new(&url, "user@dosi.test".into(), "test-pass".into())
            .with_workspace(Some("ws-1".into()));

        let hb = Heartbeat {
            device_id: "5b0c1b9e-8f2c-4a43-9a51-0c7f6f2d9b11".to_string(),
            device_name: Some("linux-node-1".to_string()),
            platform: Some("linux".to_string()),
            os_version: Some("Ubuntu 24.04".to_string()),
            agent_version: Some("0.1.0".to_string()),
            status: crate::agent::model::PresenceStatus::Tracking,
            project_id: Some("proj-uuid-1".to_string()),
            status_since: Some(Utc::now()),
            current_app: None,
            time_zone: Some("Asia/Dhaka".to_string()),
            utc_offset_minutes: Some(360),
            sent_at: None,
        };

        let result = client.heartbeat(&hb, HEARTBEAT_TIMEOUT).await.unwrap();
        assert!(result.server_time.is_some());

        let _login_req = rx.recv().unwrap();
        let hb_req = rx.recv().unwrap();
        assert!(hb_req.contains("POST /api/app/presence/heartbeat"));
        assert!(hb_req.contains("\"status\":\"tracking\""));
    }

    #[test]
    fn test_http_delivery_transport_with_sync_pass_coordinator() {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("outbox.db");
        let scope = OutboxScope::new("user-1", "workspace-1").unwrap();
        let outbox = Outbox::open(&db_path, OutboxLimits::default()).unwrap();

        let env1 = sample_envelope("act-1");
        let env2 = sample_envelope("act-2");
        outbox.enqueue(&scope, &env1).unwrap();
        outbox.enqueue(&scope, &env2).unwrap();

        let (url, _rx) = canned_server(vec![
            token_ok(),
            // Entry 1 succeeds
            http_resp("200 OK", &[], r#"{"clientActivityId":"act-1"}"#),
            // Entry 2 fails with 500 transient
            http_resp(
                "500 Internal Server Error",
                &[],
                r#"{"error":"db failure"}"#,
            ),
        ]);

        let client = ApiClient::new(&url, "user-1".into(), "test-pass".into())
            .with_workspace(Some("workspace-1".into()));

        let mut transport = HttpDeliveryTransport::new(client).unwrap();
        let coordinator = SyncPassCoordinator::new(&outbox, SyncPassConfig::default());

        let report = coordinator.run_pass(&scope, &mut transport).unwrap();
        assert_eq!(report.transport_accepted, vec!["act-1".to_string()]);
        assert_eq!(report.persisted_acknowledged, vec!["act-1".to_string()]);
        assert_eq!(report.transient_failures.len(), 1);
        assert_eq!(report.transient_failures[0].client_activity_id, "act-2");

        // Verify outbox state: act-1 removed, act-2 retained with retry_count = 1
        let pending = outbox.peek_pending(&scope, 10).unwrap();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].client_activity_id, "act-2");
        assert_eq!(pending[0].retry_count, 1);
    }

    #[test]
    fn test_http_delivery_transport_transient_failure_preserves_row_and_increments_attempts() {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("outbox.db");
        let scope = OutboxScope::new("user-1", "workspace-1").unwrap();
        let outbox = Outbox::open(&db_path, OutboxLimits::default()).unwrap();

        let env = sample_envelope("act-transient");
        outbox.enqueue(&scope, &env).unwrap();

        let (url, _rx) = canned_server(vec![
            token_ok(),
            http_resp("503 Service Unavailable", &[], r#"{}"#),
        ]);

        let client = ApiClient::new(&url, "user-1".into(), "test-pass".into())
            .with_workspace(Some("workspace-1".into()));

        let mut transport = HttpDeliveryTransport::new(client).unwrap();
        let coordinator = SyncPassCoordinator::new(&outbox, SyncPassConfig::default());

        let report = coordinator.run_pass(&scope, &mut transport).unwrap();
        assert_eq!(report.persisted_acknowledged.len(), 0);
        assert_eq!(report.transient_failures.len(), 1);

        let pending = outbox.peek_pending(&scope, 1).unwrap();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].retry_count, 1);
    }

    #[test]
    fn test_api_client_from_credentials_and_store() {
        use crate::credentials::InMemoryCredentialStore;

        let store = InMemoryCredentialStore::new();
        let mut creds = Credentials::new("store-user@dosi.test", Some("stored-pw".into()));
        creds.workspace_id = Some("ws-from-store".into());
        creds.server_url = Some("https://api.stored.local".into());
        creds.device_signing_key = Some(vec![99u8; 32]);
        creds.access_token = Some("valid-tok-123".into());
        creds.access_token_expires_at = Some(Utc::now() + chrono::Duration::seconds(1800));
        creds.refresh_token = Some("ref-tok-456".into());
        store.save(&creds).unwrap();

        let client =
            ApiClient::from_store(&store, "store-user@dosi.test", Some("ws-from-store")).unwrap();
        assert_eq!(client.base_url(), "https://api.stored.local");
        assert_eq!(client.username(), "store-user@dosi.test");
        assert_eq!(client.workspace_id(), Some("ws-from-store"));
        assert_eq!(
            client.tokens().valid_access_token().as_deref(),
            Some("valid-tok-123")
        );
        assert_eq!(
            client.tokens().refresh_token().as_deref(),
            Some("ref-tok-456")
        );

        let transport = HttpDeliveryTransport::from_store(
            &store,
            "store-user@dosi.test",
            Some("ws-from-store"),
        );
        assert!(transport.is_ok());

        // Negative check: missing scope in store returns Err
        let missing = ApiClient::from_store(&store, "store-user@dosi.test", Some("ws-nonexistent"));
        assert!(missing.is_err());
    }

    #[test]
    fn test_http_delivery_transport_scope_binding_validation() {
        use crate::credentials::InMemoryCredentialStore;

        // 1. Valid construction canonicalizes account and workspace
        let client_ok = ApiClient::new(
            "http://127.0.0.1:9999",
            "  User-A@Dosi.TEST  ".into(),
            "secret".into(),
        )
        .with_workspace(Some("  ws-tenant-1  ".into()));

        let transport = HttpDeliveryTransport::new(client_ok).unwrap();
        assert_eq!(transport.bound_account_id(), "user-a@dosi.test");
        assert_eq!(transport.bound_workspace_id(), "ws-tenant-1");
        assert_eq!(
            transport.bound_scope(),
            &OutboxScope::new("user-a@dosi.test", "ws-tenant-1").unwrap()
        );

        // Scope matching checks
        let exact_scope = OutboxScope::new("user-a@dosi.test", "ws-tenant-1").unwrap();
        assert!(transport.matches_scope(&exact_scope));

        // Matching with uncanonicalized input (case insensitive account, trimmed)
        let uncanonical_scope = OutboxScope {
            account_id: "  USER-A@DOSI.TEST  ".to_string(),
            workspace_id: "  ws-tenant-1  ".to_string(),
        };
        assert!(transport.matches_scope(&uncanonical_scope));

        // Mismatch checks
        let diff_account = OutboxScope::new("other@dosi.test", "ws-tenant-1").unwrap();
        assert!(!transport.matches_scope(&diff_account));

        let diff_workspace = OutboxScope::new("user-a@dosi.test", "ws-other").unwrap();
        assert!(!transport.matches_scope(&diff_workspace));

        let diff_both = OutboxScope::new("other@dosi.test", "ws-other").unwrap();
        assert!(!transport.matches_scope(&diff_both));

        // 2. Fail-closed: ApiClient without workspace cannot construct transport
        let client_no_ws = ApiClient::new(
            "http://127.0.0.1:9999",
            "user@dosi.test".into(),
            "secret".into(),
        );
        let err_no_ws = HttpDeliveryTransport::new(client_no_ws);
        assert!(err_no_ws.is_err());
        assert!(err_no_ws
            .unwrap_err()
            .contains("requires a bound workspace_id"));

        // 3. Fail-closed: ApiClient with empty/whitespace workspace
        let client_empty_ws = ApiClient::new(
            "http://127.0.0.1:9999",
            "user@dosi.test".into(),
            "secret".into(),
        )
        .with_workspace(Some("   ".into()));
        let err_empty_ws = HttpDeliveryTransport::new(client_empty_ws);
        assert!(err_empty_ws.is_err());

        // 4. Fail-closed: Credentials without workspace
        let mut creds_no_ws = Credentials::new("user@dosi.test", Some("secret".into()));
        creds_no_ws.workspace_id = None;
        let err_creds_no_ws = HttpDeliveryTransport::from_credentials(&creds_no_ws);
        assert!(err_creds_no_ws.is_err());
        assert!(err_creds_no_ws.unwrap_err().contains("bound workspace_id"));

        // 5. Fail-closed: Credentials with empty workspace
        let mut creds_empty_ws = Credentials::new("user@dosi.test", Some("secret".into()));
        creds_empty_ws.workspace_id = Some("   ".into());
        let err_creds_empty_ws = HttpDeliveryTransport::from_credentials(&creds_empty_ws);
        assert!(err_creds_empty_ws.is_err());
        assert!(err_creds_empty_ws
            .unwrap_err()
            .contains("non-empty workspace_id"));

        // 6. Fail-closed: from_store with None or empty workspace
        let store = InMemoryCredentialStore::new();
        let err_store_none = HttpDeliveryTransport::from_store(&store, "user@dosi.test", None);
        assert!(err_store_none.is_err());
        assert!(err_store_none
            .unwrap_err()
            .contains("explicit bound workspace_id"));

        let err_store_empty =
            HttpDeliveryTransport::from_store(&store, "user@dosi.test", Some("  "));
        assert!(err_store_empty.is_err());
        assert!(err_store_empty
            .unwrap_err()
            .contains("non-empty workspace_id"));
    }

    #[test]
    fn test_http_delivery_transport_scope_mismatch_rejection_at_boundary() {
        let client = ApiClient::new(
            "http://127.0.0.1:9999",
            "account-a@dosi.test".into(),
            "pass-a".into(),
        )
        .with_workspace(Some("ws-a".into()));

        let mut transport = HttpDeliveryTransport::new(client).unwrap();

        // Foreign scope entry
        let foreign_scope = OutboxScope::new("account-b@dosi.test", "ws-b").unwrap();
        let entry = OutboxEntry {
            id: 1,
            scope: foreign_scope,
            client_activity_id: "act-foreign-1".to_string(),
            enqueued_at: Utc::now(),
            retry_count: 0,
            last_error: None,
            envelope: sample_envelope("act-foreign-1"),
            format_version: crate::outbox::ROW_FORMAT_V1,
            payload_json: None,
            payload_signature: None,
            capture_bytes: 0,
            captures: Vec::new(),
        };

        // deliver() must definitively reject without touching network/worker
        let outcome = transport.deliver(&entry);
        match outcome {
            DeliveryOutcome::Rejected { reason } => {
                assert!(reason.contains("Scope mismatch"));
                assert!(reason.contains("account-a@dosi.test"));
                assert!(reason.contains("account-b@dosi.test"));
                assert!(reason.contains("ws-a"));
                assert!(reason.contains("ws-b"));
                // Invariant: no secrets in error messages
                assert!(!reason.contains("pass-a"));
            }
            other => panic!("Expected DeliveryOutcome::Rejected, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_clock_based_pre_expiry_vs_hard_expiry_renewal() {
        let (url, rx) = canned_server(vec![
            http_resp(
                "200 OK",
                &[],
                r#"{"access_token":"initial-300s-tok","token_type":"Bearer","expires_in":300,"refresh_token":"ref-300"}"#,
            ),
            http_resp(
                "200 OK",
                &[],
                r#"{"access_token":"refreshed-300s-tok","token_type":"Bearer","expires_in":300,"refresh_token":"ref-300"}"#,
            ),
        ]);

        let mut client = ApiClient::new(&url, "user@dosi.test".into(), "test-pass".into());
        client.login().await.unwrap();
        let _ = rx.recv().unwrap();

        // 1. Initial fresh state at t=0
        assert_eq!(
            client.tokens().valid_access_token().as_deref(),
            Some("initial-300s-tok")
        );
        assert!(!client.tokens().needs_refresh());
        assert!(client.tokens().is_valid());

        // 2. Advance time to t=100s: still well before renewal threshold (240s)
        client.tokens().advance_time(Duration::from_secs(100));
        assert!(!client.tokens().needs_refresh());
        assert!(client.tokens().is_valid());
        let tok = client.ensure_token().await.unwrap();
        assert_eq!(tok, "initial-300s-tok");

        // 3. Advance time to t=245s: passed renewal buffer (300-60=240s), but before hard expiry (300s)
        client.tokens().advance_time(Duration::from_secs(145));
        assert!(
            client.tokens().is_valid(),
            "Must be strictly valid before hard expiry"
        );
        assert!(
            client.tokens().needs_refresh(),
            "Must indicate renewal needed inside pre-expiry window"
        );

        // ensure_token proactively refreshes
        let refreshed = client.ensure_token().await.unwrap();
        assert_eq!(refreshed, "refreshed-300s-tok");
        let refresh_req = rx.recv().unwrap();
        assert!(refresh_req.contains("grant_type=refresh_token"));
        assert!(!client.tokens().needs_refresh());
        assert!(client.tokens().is_valid());

        // 4. Advance time past the refreshed token's hard expiry (+305s) with no more canned responses
        client.tokens().advance_time(Duration::from_secs(305));
        assert!(
            !client.tokens().is_valid(),
            "Token must hard-expire at server lifetime limit"
        );
        assert_eq!(client.tokens().valid_access_token(), None);
    }

    #[test]
    fn test_short_grant_pre_expiry_and_hard_expiry_boundaries() {
        let now_inst = Instant::now();
        let now_utc = Utc::now();
        let token = GrantedToken::new(
            "short-tok".into(),
            Some("ref".into()),
            Some(5),
            now_inst,
            now_utc,
        );

        // At t=0s
        assert!(token.is_valid_at(now_inst));
        assert!(!token.needs_refresh_at(now_inst));

        // At t=3s
        let t3 = now_inst + Duration::from_secs(3);
        assert!(token.is_valid_at(t3));
        assert!(!token.needs_refresh_at(t3));

        // At t=4s (proportional buffer is 1s: refresh_after is at 4s, hard expiry is at 5s)
        let t4 = now_inst + Duration::from_secs(4);
        assert!(token.is_valid_at(t4), "5s token must still be valid at 4s");
        assert!(
            token.needs_refresh_at(t4),
            "5s token must trigger proactive renewal at 4s"
        );

        // At t=5s: hard expired!
        let t5 = now_inst + Duration::from_secs(5);
        assert!(
            !token.is_valid_at(t5),
            "5s token must be invalid at 5s hard expiry"
        );
        assert!(token.needs_refresh_at(t5));

        // At t=60s: defect regression check!
        let t60 = now_inst + Duration::from_secs(60);
        assert!(
            !token.is_valid_at(t60),
            "5s token must NEVER be extended to 60 seconds!"
        );
    }

    #[test]
    fn test_zero_and_missing_and_huge_grant_boundaries() {
        let now_inst = Instant::now();
        let now_utc = Utc::now();

        // 1. Zero grant: immediately invalid, fails closed
        let zero = GrantedToken::new(
            "zero-tok".into(),
            Some("ref".into()),
            Some(0),
            now_inst,
            now_utc,
        );
        assert!(!zero.is_valid_at(now_inst));
        assert!(zero.needs_refresh_at(now_inst));
        assert_eq!(zero.utc_expires_at, None);

        // 2. Missing expiry (None): fails closed without inventing duration
        let missing = GrantedToken::new(
            "miss-tok".into(),
            Some("ref".into()),
            None,
            now_inst,
            now_utc,
        );
        assert!(!missing.is_valid_at(now_inst));
        assert!(missing.needs_refresh_at(now_inst));
        assert_eq!(missing.utc_expires_at, None);

        // 3. Huge grant (u64::MAX): safely clamped, zero panic, zero Instant overflow
        let huge = GrantedToken::new(
            "huge-tok".into(),
            Some("ref".into()),
            Some(u64::MAX),
            now_inst,
            now_utc,
        );
        assert!(huge.is_valid_at(now_inst));
        assert!(!huge.needs_refresh_at(now_inst));
        let expected_max_utc = now_utc + chrono::Duration::seconds(MAX_TOKEN_TTL_SECS as i64);
        assert_eq!(huge.utc_expires_at, Some(expected_max_utc));
    }

    #[test]
    fn test_persisted_session_rehydration_expiry_boundaries() {
        // 1. Rehydrate unexpired short token (15 seconds remaining)
        let mut creds = Credentials::new("rehydrate-user", None);
        creds.server_url = Some("https://tracker.example.com".into());
        creds.access_token = Some("tok-15s".into());
        creds.access_token_expires_at = Some(Utc::now() + chrono::Duration::seconds(15));
        creds.refresh_token = Some("ref-15s".into());

        let client = ApiClient::from_credentials(&creds).unwrap();
        assert_eq!(
            client.tokens().valid_access_token().as_deref(),
            Some("tok-15s")
        );
        assert!(client.tokens().is_valid());

        // Advance 16 seconds: must expire and NOT extend to 3600 seconds
        client.tokens().advance_time(Duration::from_secs(16));
        assert_eq!(
            client.tokens().valid_access_token(),
            None,
            "Rehydrated 15s token must expire after 16s, never extended to 1 hour"
        );
        assert!(!client.tokens().is_valid());

        // 2. Rehydrate already expired token: fails closed, access token not admitted
        let mut expired_creds = Credentials::new("expired-user", None);
        expired_creds.server_url = Some("https://tracker.example.com".into());
        expired_creds.access_token = Some("tok-expired".into());
        expired_creds.access_token_expires_at = Some(Utc::now() - chrono::Duration::seconds(10));
        expired_creds.refresh_token = Some("ref-still-valid".into());

        let exp_client = ApiClient::from_credentials(&expired_creds).unwrap();
        assert_eq!(
            exp_client.tokens().valid_access_token(),
            None,
            "Expired persisted token must not be admitted as valid"
        );
        assert!(!exp_client.tokens().is_valid());
        assert_eq!(
            exp_client.tokens().refresh_token().as_deref(),
            Some("ref-still-valid")
        );

        // 3. Rehydrate token with missing expiry: fails closed without inventing duration
        let mut missing_creds = Credentials::new("missing-exp-user", None);
        missing_creds.server_url = Some("https://tracker.example.com".into());
        missing_creds.access_token = Some("tok-no-exp".into());
        missing_creds.access_token_expires_at = None;
        missing_creds.refresh_token = Some("ref-backup".into());

        let miss_client = ApiClient::from_credentials(&missing_creds).unwrap();
        assert_eq!(
            miss_client.tokens().valid_access_token(),
            None,
            "Missing expiry must fail closed, never inventing 1 hour"
        );
        assert!(!miss_client.tokens().is_valid());
        assert_eq!(
            miss_client.tokens().refresh_token().as_deref(),
            Some("ref-backup")
        );
    }

    #[tokio::test]
    async fn test_transient_failure_during_pre_expiry_preserves_usable_token_until_hard_expiry() {
        let (url, rx) = canned_server(vec![
            http_resp(
                "200 OK",
                &[],
                r#"{"access_token":"token-60s","token_type":"Bearer","expires_in":60,"refresh_token":"ref-60s"}"#,
            ),
            // Transient 500 failure during refresh attempt
            http_resp(
                "500 Internal Server Error",
                &[],
                r#"{"error":"temporary_server_issue"}"#,
            ),
        ]);

        let mut client = ApiClient::new(&url, "user@dosi.test".into(), "test-pass".into());
        client.login().await.unwrap();
        let _ = rx.recv().unwrap();

        // Advance to t=50s (60s grant: buffer is 12s, refresh_after is 48s, hard expiry is 60s)
        client.tokens().advance_time(Duration::from_secs(50));
        assert!(client.tokens().needs_refresh());
        assert!(client.tokens().is_valid());

        // When refresh fails with transient 500, still-valid token is preserved
        let token = client.ensure_token().await.unwrap();
        assert_eq!(token, "token-60s");
        let refresh_req = rx.recv().unwrap();
        assert!(refresh_req.contains("grant_type=refresh_token"));

        // Advance to t=65s: now past hard expiry (60s)
        client.tokens().advance_time(Duration::from_secs(15));
        assert!(
            !client.tokens().is_valid(),
            "Token must be hard expired at 65s"
        );
        // Further ensure_token must fail closed
        let res = client.ensure_token().await;
        assert!(
            res.is_err(),
            "Must fail closed once hard expiry has elapsed"
        );
    }

    // --- Ported from the Windows agent (contract parity) -------------------

    fn abp(code: Option<&str>, message: &str) -> String {
        let code = code
            .map(|c| format!("\"{c}\""))
            .unwrap_or_else(|| "null".to_string());
        format!(r#"{{"error":{{"code":{code},"message":"{message}","details":null}}}}"#)
    }

    fn kind_of(error: &SubmitError) -> &'static str {
        match error {
            SubmitError::Transient { .. } => "transient",
            SubmitError::AuthRejected(_) => "auth",
            SubmitError::SubscriptionInactive(_) => "subscription",
            SubmitError::ProjectUnavailable { .. } => "project",
            SubmitError::ClockAhead(_) => "clock",
            SubmitError::NotWorkspaceMember(_) => "workspace",
            SubmitError::WorkspaceArchived(_) => "archived",
            SubmitError::WorkspaceRequired(_) => "workspace-required",
            SubmitError::DeviceRevoked(_) => "revoked",
            SubmitError::StripCaptures(_) => "strip",
            SubmitError::Rejected(_) => "park",
        }
    }

    #[test]
    fn upload_failures_are_classified_as_the_contract_says() {
        let wait = Some(Duration::from_secs(30));
        let kind = |status: u16, body: &str| kind_of(&classify_upload_failure(status, body, None));

        // Transient: infrastructure, throttling and code-less 4xx.
        for status in [301, 302, 404, 405, 408, 429, 500, 502, 503] {
            assert_eq!(kind(status, ""), "transient", "{status}");
        }
        assert_eq!(kind(400, "<html>bad request</html>"), "transient");
        assert_eq!(kind(403, r#"{"title":"Forbidden"}"#), "transient");
        assert_eq!(
            kind(422, &abp(Some("Tracker:SomethingNew"), "?")),
            "transient"
        );
        // A 5xx is transient even with a business code.
        assert_eq!(
            kind(500, &abp(Some(codes::ACTIVITY_OVERLAPS), "x")),
            "transient"
        );
        assert_eq!(
            kind(503, &abp(Some(codes::DEVICE_REVOKED), "x")),
            "transient"
        );

        // Special handling.
        assert_eq!(
            kind(402, &abp(Some(codes::SUBSCRIPTION_INACTIVE), "Inactive")),
            "subscription"
        );
        assert_eq!(kind(402, ""), "transient", "402 without a code");
        assert_eq!(
            kind(403, &abp(Some(codes::NOT_PROJECT_MEMBER), "Not a member")),
            "project"
        );
        assert_eq!(
            kind(409, &abp(Some(codes::PROJECT_ARCHIVED), "Archived")),
            "project"
        );
        assert_eq!(
            kind(425, &abp(Some(codes::ACTIVITY_IN_FUTURE), "In the future")),
            "clock"
        );
        assert_eq!(kind(413, ""), "strip");
        assert_eq!(kind(413, "<html>Request Entity Too Large</html>"), "strip");
        assert_eq!(
            kind(413, &abp(Some(codes::CAPTURE_TOO_LARGE), "Too big")),
            "strip"
        );
        assert_eq!(
            kind(400, &abp(Some(codes::INVALID_CAPTURE_DATA), "Bad image")),
            "strip"
        );

        // Linux additions: the terminal workspace and device answers.
        assert_eq!(
            kind(403, &abp(Some(codes::NOT_WORKSPACE_MEMBER), "No")),
            "workspace"
        );
        assert_eq!(
            kind(403, &abp(Some(codes::WORKSPACE_ARCHIVED), "Archived")),
            "archived"
        );
        assert_eq!(
            kind(400, &abp(Some(codes::WORKSPACE_REQUIRED), "Pick one")),
            "workspace-required"
        );
        assert_eq!(
            kind(403, &abp(Some(codes::DEVICE_REVOKED), "Revoked")),
            "revoked"
        );

        // Park: definitive rejections of the block.
        assert_eq!(
            kind(409, &abp(Some(codes::ACTIVITY_OVERLAPS), "Overlaps")),
            "park"
        );
        assert_eq!(
            kind(
                409,
                &abp(Some(codes::ACTIVITY_CLIENT_ID_CONFLICT), "Conflict")
            ),
            "park"
        );
        assert_eq!(
            kind(422, &abp(Some(codes::ACTIVITY_TOO_LONG), "Too long")),
            "park"
        );
        assert_eq!(
            kind(422, &abp(Some(codes::ACTIVITY_TOO_OLD), "Too old")),
            "park"
        );
        assert_eq!(
            kind(
                422,
                &abp(Some(codes::ACTIVITY_TIME_RANGE_INVALID), "Bad range")
            ),
            "park"
        );
        let validation = r#"{"error":{"code":null,"message":"Your request is not valid!",
            "validationErrors":[{"message":"Description is too long.","members":["description"]}]}}"#;
        assert_eq!(kind(400, validation), "park");
        // An older server answers 403 for every business error: the code decides.
        assert_eq!(
            kind(403, &abp(Some(codes::ACTIVITY_TOO_LONG), "Too long")),
            "park"
        );
        // Validation errors park the block even if a code comes with them.
        let coded_validation = r#"{"error":{"code":"Volo.Abp.Validation","message":"Your request is not valid!",
            "validationErrors":[{"message":"The field TimeZone is invalid.","members":["timeZone"]}]}}"#;
        assert_eq!(kind(400, coded_validation), "park");
        // ...but only on a 400: elsewhere they are not about this payload.
        assert_eq!(kind(422, coded_validation), "transient");

        // The server's message is kept; Retry-After is passed on.
        match classify_upload_failure(400, validation, None) {
            SubmitError::Rejected(message) => {
                assert_eq!(
                    message,
                    "Your request is not valid! Description is too long."
                )
            }
            other => panic!("expected a rejection, got {other:?}"),
        }
        match classify_upload_failure(429, "", wait) {
            SubmitError::Transient { retry_after, .. } => assert_eq!(retry_after, wait),
            other => panic!("expected transient, got {other:?}"),
        }
        match classify_upload_failure(
            403,
            &abp(Some(codes::NOT_PROJECT_MEMBER), "Not a member"),
            None,
        ) {
            SubmitError::ProjectUnavailable { code, message } => {
                assert_eq!(code, codes::NOT_PROJECT_MEMBER);
                assert_eq!(message, "Not a member");
            }
            other => panic!("expected project unavailable, got {other:?}"),
        }
    }

    #[test]
    fn upload_failures_map_to_the_delivery_outcomes_of_design_8_4() {
        let outcome = |status: u16, body: &str| {
            delivery_outcome_of(classify_upload_failure(
                status,
                body,
                Some(Duration::from_secs(7)),
            ))
        };
        assert_eq!(
            outcome(503, ""),
            DeliveryOutcome::RetryLater {
                reason: "HTTP 503".into(),
                kind: RetryKind::Transient,
                retry_after: Some(Duration::from_secs(7)),
            }
        );
        assert!(matches!(
            outcome(402, &abp(Some(codes::SUBSCRIPTION_INACTIVE), "x")),
            DeliveryOutcome::RetryLater {
                kind: RetryKind::Subscription,
                ..
            }
        ));
        assert!(matches!(
            outcome(425, &abp(Some(codes::ACTIVITY_IN_FUTURE), "x")),
            DeliveryOutcome::RetryLater {
                kind: RetryKind::ClockAhead,
                ..
            }
        ));
        for code in [codes::NOT_WORKSPACE_MEMBER, codes::WORKSPACE_ARCHIVED] {
            assert!(
                matches!(
                    outcome(403, &abp(Some(code), "x")),
                    DeliveryOutcome::WorkspaceAccessLost { .. }
                ),
                "{code}"
            );
        }
        assert!(matches!(
            outcome(400, &abp(Some(codes::WORKSPACE_REQUIRED), "x")),
            DeliveryOutcome::WorkspaceAccessLost { .. }
        ));
        assert!(matches!(
            outcome(403, &abp(Some(codes::DEVICE_REVOKED), "x")),
            DeliveryOutcome::DeviceRevoked { .. }
        ));
        assert!(matches!(
            outcome(413, ""),
            DeliveryOutcome::StripCaptures { .. }
        ));
        assert_eq!(
            outcome(409, &abp(Some(codes::ACTIVITY_OVERLAPS), "Overlaps")),
            DeliveryOutcome::Park {
                reason: "Overlaps".into()
            }
        );
        assert_eq!(
            delivery_outcome_of(SubmitError::AuthRejected("expired".into())),
            DeliveryOutcome::AuthRequired {
                reason: "expired".into()
            }
        );
    }

    #[test]
    fn only_refused_credentials_are_a_rejected_sign_in() {
        let invalid_grant =
            r#"{"error":"invalid_grant","error_description":"Invalid username or password!"}"#;
        assert_eq!(
            classify_token_failure(400, invalid_grant),
            AuthError::Rejected("Invalid username or password!".into())
        );
        assert!(classify_token_failure(401, "").is_rejected());
        assert!(classify_token_failure(403, "").is_rejected());
        // Not about the credentials: a proxy's bare 400, a wrong client, a
        // missing endpoint, throttling and server failures.
        assert!(!classify_token_failure(400, "").is_rejected());
        assert!(!classify_token_failure(400, r#"{"error":"invalid_client"}"#).is_rejected());
        assert!(!classify_token_failure(404, "").is_rejected());
        assert!(
            matches!(classify_token_failure(429, ""), AuthError::Server(m) if m.contains("Too many"))
        );
        assert!(
            matches!(classify_token_failure(503, ""), AuthError::Server(m) if m.contains("503"))
        );
    }

    #[test]
    fn retry_after_takes_seconds_or_a_date_and_is_bounded() {
        let now = DateTime::parse_from_rfc3339("2026-10-05T10:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        assert_eq!(
            parse_retry_after("120", None, now),
            Some(Duration::from_secs(120))
        );
        assert_eq!(parse_retry_after(" 0 ", None, now), Some(Duration::ZERO));
        assert_eq!(
            parse_retry_after("Mon, 05 Oct 2026 10:05:00 GMT", None, now),
            Some(Duration::from_secs(300))
        );
        // Measured against the server's clock when it is known.
        let server_now = now + chrono::Duration::minutes(2);
        assert_eq!(
            parse_retry_after("Mon, 05 Oct 2026 10:05:00 GMT", Some(server_now), now),
            Some(Duration::from_secs(180))
        );
        // In the past: retry now. Absurdly far: capped.
        assert_eq!(
            parse_retry_after("Mon, 05 Oct 2026 09:00:00 GMT", None, now),
            Some(Duration::ZERO)
        );
        assert_eq!(
            parse_retry_after("99999999", None, now),
            Some(MAX_RETRY_AFTER)
        );
        assert_eq!(parse_retry_after("soon", None, now), None);
    }

    #[test]
    fn summary_query_survives_url_encoding() {
        let from = DateTime::parse_from_rfc3339("2026-09-29T18:00:00+00:00")
            .unwrap()
            .with_timezone(&Utc);
        let to = DateTime::parse_from_rfc3339("2026-09-30T10:29:24.976+00:00")
            .unwrap()
            .with_timezone(&Utc);
        let request = reqwest::Client::new()
            .get("https://example.invalid/api/app/reporting/summary")
            .query(&summary_query(
                from,
                to,
                Some("America/Argentina/Buenos_Aires"),
            ))
            .build()
            .unwrap();

        // What the server sees after decoding must still parse to the same instants.
        let pairs: std::collections::HashMap<_, _> =
            request.url().query_pairs().into_owned().collect();
        let parse = |key: &str| {
            DateTime::parse_from_rfc3339(&pairs[key])
                .unwrap()
                .with_timezone(&Utc)
        };
        assert_eq!(parse("From"), from);
        assert_eq!(parse("To"), to);
        assert_eq!(pairs["TimeZone"], "America/Argentina/Buenos_Aires");
        assert!(!request.url().query().unwrap().contains('+'), "no raw '+'");

        // No zone: the parameter is left out (the server's default applies).
        assert_eq!(summary_query(from, to, None).len(), 2);
        assert_eq!(summary_query(from, to, Some("  ")).len(), 2);
    }

    #[test]
    fn report_summaries_tolerate_unmeasured_productivity() {
        let summary: ReportSummary = serde_json::from_str(
            r#"{"totalTrackedMinutes":90.5,"averageProductivity":null,"totalIdleMinutes":12}"#,
        )
        .unwrap();
        assert_eq!(summary.total_tracked_minutes, 90.5);
        assert_eq!(summary.average_productivity, None);
        let older: ReportSummary =
            serde_json::from_str(r#"{"totalTrackedMinutes":5,"averageProductivity":61.2}"#)
                .unwrap();
        assert_eq!(older.average_productivity, Some(61.2));
    }

    #[test]
    fn plain_http_is_refused_except_loopback_or_an_explicit_opt_in() {
        assert!(check_server_url("https://tracker.example.com", false).is_ok());
        for loopback in [
            "http://127.0.0.1:8080",
            "http://127.9.9.9",
            "http://localhost:44300",
            "http://[::1]:5000",
        ] {
            assert!(check_server_url(loopback, false).is_ok(), "{loopback}");
        }
        let refused = check_server_url("http://tracker.example.com", false).unwrap_err();
        assert!(refused.contains("allow_insecure_http"), "{refused}");
        assert!(check_server_url("http://10.0.0.5", false).is_err());
        assert!(check_server_url("http://tracker.example.com", true).is_ok());
        assert!(check_server_url("ftp://tracker.example.com", true).is_err());
        assert!(check_server_url("not a url", true).is_err());
    }

    #[tokio::test]
    async fn an_insecure_server_never_receives_a_request() {
        let mut api = ApiClient::new(
            "http://tracker.example.invalid",
            "a@b.c".into(),
            "pw".into(),
        );
        match api.login().await {
            Err(AuthError::Server(message)) => assert!(message.contains("http://")),
            other => panic!("expected a refusal, got {other:?}"),
        }
        assert!(matches!(
            api.workspaces_mine().await,
            Err(ApiError::Invalid(_))
        ));
        let mut creds = Credentials::new("a@b.c", None);
        creds.server_url = Some("http://tracker.example.invalid".into());
        assert!(ApiClient::from_credentials(&creds).is_err());
    }

    #[test]
    fn credentials_without_a_server_are_refused() {
        let creds = Credentials::new("a@b.c", None);
        let error = ApiClient::from_credentials(&creds).err().unwrap();
        assert!(error.contains("no server"), "{error}");
    }

    fn body_of(request: &str) -> &str {
        &request[request.find("\r\n\r\n").unwrap() + 4..]
    }

    fn header_of<'a>(request: &'a str, name: &str) -> Option<&'a str> {
        let head = &request[..request.find("\r\n\r\n").unwrap_or(request.len())];
        head.lines().find_map(|line| {
            let (key, value) = line.split_once(':')?;
            key.trim().eq_ignore_ascii_case(name).then(|| value.trim())
        })
    }

    const DEVICE: &str = "5b0c1b9e-8f2c-4a43-9a51-0c7f6f2d9b11";
    const W1: &str = "6f9619ff-8b86-d011-b42d-00c04fc964ff";
    const W2: &str = "7f9619ff-8b86-d011-b42d-00c04fc964ff";

    fn client(url: &str) -> ApiClient {
        ApiClient::new(format!("{url}/"), "alice@acme.com".into(), "pw".into())
            .with_workspace(Some(W1.into()))
            .with_device_id(Some(DEVICE.into()))
    }

    fn sample_activity() -> Activity {
        let now = Utc::now();
        Activity::new(
            "0b9a7c55-2f37-4f40-a7e5-5a8f0f6b2d11",
            now - chrono::Duration::minutes(10),
            now,
        )
    }

    fn heartbeat() -> Heartbeat {
        Heartbeat {
            device_id: DEVICE.into(),
            device_name: None,
            platform: Some("linux".into()),
            os_version: None,
            agent_version: None,
            status: crate::agent::model::PresenceStatus::Stopped,
            project_id: None,
            status_since: None,
            current_app: None,
            time_zone: None,
            utc_offset_minutes: None,
            sent_at: None,
        }
    }

    fn token_t0k3n() -> String {
        http_resp(
            "200 OK",
            &[],
            r#"{"access_token":"t0k3n","token_type":"Bearer","expires_in":3600}"#,
        )
    }

    #[tokio::test]
    async fn the_password_sign_in_sends_the_device_id_and_no_tenant_or_workspace() {
        let (url, requests) = canned_server(vec![token_t0k3n()]);
        client(&url).login().await.unwrap();
        let request = requests.try_iter().next().unwrap();
        assert!(request.starts_with("POST /connect/token "), "{request}");
        assert!(!request.to_lowercase().contains(TENANT_PARAM), "{request}");
        let form = body_of(&request);
        for part in [
            "grant_type=password",
            "client_id=Tracker_App",
            "username=alice%40acme.com",
            "password=pw",
            "offline_access",
            &format!("device_id={DEVICE}"),
        ] {
            assert!(form.contains(part), "{part} in {form}");
        }
        assert_eq!(header_of(&request, WORKSPACE_HEADER), None);
    }

    #[tokio::test]
    async fn workspace_calls_carry_the_header_and_token_and_mine_calls_do_not() {
        let mine = format!(
            r#"[{{"id":"{W1}","name":"Acme","role":"owner","planName":"Pro","isOwner":true,"isArchived":false}},
               {{"id":"{W2}","name":"Globex","role":"member","isArchived":true}}]"#
        );
        let (url, requests) = canned_server(vec![
            token_t0k3n(),
            http_resp("200 OK", &[], &mine),
            http_resp("200 OK", &[], "[]"),
            http_resp("200 OK", &[], r#"{"totalTrackedMinutes":1}"#),
            http_resp("200 OK", &[], "{}"),
            http_resp("200 OK", &[], "{}"),
            http_resp(
                "200 OK",
                &[],
                &format!(r#"{{"deviceId":"{DEVICE}","isRevoked":false}}"#),
            ),
        ]);
        let mut api = client(&url);
        let workspaces = api.workspaces_mine().await.unwrap();
        assert_eq!(workspaces.len(), 2);
        assert!(workspaces[0].is_selectable());
        assert!(!workspaces[1].is_selectable(), "archived");

        api.set_workspace(Some(W2.into()));
        api.my_projects().await.unwrap();
        api.summary(Utc::now() - chrono::Duration::hours(1), Utc::now(), None)
            .await
            .unwrap();
        api.heartbeat(&heartbeat(), Duration::from_secs(5))
            .await
            .unwrap();
        api.submit_activity(&sample_activity(), None).await.unwrap();
        let status = api.device_status(DEVICE).await.unwrap();
        assert!(!status.is_revoked);

        let requests: Vec<String> = requests.try_iter().collect();
        assert_eq!(requests.len(), 7);
        assert_eq!(
            header_of(&requests[0], WORKSPACE_HEADER),
            None,
            "the token request"
        );
        assert!(
            requests[1].starts_with("GET /api/app/workspaces/mine "),
            "{}",
            requests[1]
        );
        assert_eq!(
            header_of(&requests[1], WORKSPACE_HEADER),
            None,
            "listing the workspaces"
        );
        assert!(requests[2].starts_with("GET /api/app/project/my-projects "));
        assert!(requests[3].starts_with("GET /api/app/reporting/summary?"));
        assert!(requests[4].starts_with("POST /api/app/presence/heartbeat "));
        assert!(requests[5].starts_with("POST /api/app/activity "));
        assert!(requests[6].starts_with(&format!("GET /api/app/device/{DEVICE}/status ")));
        for request in &requests[2..] {
            assert_eq!(header_of(request, WORKSPACE_HEADER), Some(W2), "{request}");
            assert_eq!(header_of(request, "authorization"), Some("Bearer t0k3n"));
        }
    }

    #[tokio::test]
    async fn workspace_scoped_calls_need_a_workspace() {
        let mut api = ApiClient::new("http://127.0.0.1:9", "a@b.c".into(), "x".into());
        assert!(matches!(
            api.my_projects().await,
            Err(ApiError::WorkspaceRequired(_))
        ));
        assert!(matches!(
            api.submit_activity(&sample_activity(), None).await,
            Err(SubmitError::WorkspaceRequired(_))
        ));
        assert!(matches!(
            api.device_status("not-a-uuid").await,
            Err(ApiError::Invalid(_))
        ));
    }

    #[tokio::test]
    async fn a_401_upload_signs_in_again_and_resends_with_sent_at() {
        let (url, requests) = canned_server(vec![
            token_t0k3n(),
            http_resp("401 Unauthorized", &[], ""),
            token_t0k3n(),
            http_resp("200 OK", &[("Date", "Mon, 05 Oct 2026 10:00:00 GMT")], "{}"),
        ]);
        let mut api = client(&url);
        api.submit_activity(&sample_activity(), Some("c2ln"))
            .await
            .expect("uploaded on the second try");

        let requests: Vec<String> = requests.try_iter().collect();
        assert_eq!(requests.len(), 4);
        assert!(
            requests[0].starts_with("POST /connect/token "),
            "{}",
            requests[0]
        );
        assert!(
            requests[1].starts_with("POST /api/app/activity "),
            "no double slash: {}",
            requests[1]
        );
        assert!(requests[2].starts_with("POST /connect/token"));
        let upload = &requests[3];
        assert_eq!(header_of(upload, "authorization"), Some("Bearer t0k3n"));
        let json: serde_json::Value = serde_json::from_str(body_of(upload)).unwrap();
        let sent_at = json["sentAt"]
            .as_str()
            .expect("sentAt is stamped at send time");
        assert!(DateTime::parse_from_rfc3339(sent_at).is_ok());
        assert_eq!(json["payloadSignature"], "c2ln");
        assert!(json.get("productivity").is_none(), "unmeasured: left out");
        assert!(
            json["keyboardHits"].is_null() && json["mouseClicks"].is_null(),
            "unmeasured: null"
        );
        // The Date header of the last answer set the clock offset.
        assert!(api.server_clock().offset_secs().is_some());
    }

    #[tokio::test]
    async fn a_second_401_after_signing_in_again_is_transient() {
        let (url, _requests) = canned_server(vec![
            token_t0k3n(),
            http_resp("401 Unauthorized", &[], ""),
            token_t0k3n(),
            http_resp("401 Unauthorized", &[], ""),
        ]);
        assert!(matches!(
            client(&url).submit_activity(&sample_activity(), None).await,
            Err(SubmitError::Transient { .. })
        ));
    }

    #[tokio::test]
    async fn refused_credentials_during_an_upload_mean_sign_in_expired() {
        let (url, _requests) = canned_server(vec![http_resp(
            "400 Bad Request",
            &[],
            r#"{"error":"invalid_grant","error_description":"Invalid username or password!"}"#,
        )]);
        match client(&url).submit_activity(&sample_activity(), None).await {
            Err(SubmitError::AuthRejected(reason)) => {
                assert_eq!(reason, "Invalid username or password!")
            }
            other => panic!("expected sign-in expired, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn an_expired_token_is_renewed_with_the_refresh_token_not_the_password() {
        let refreshed = http_resp(
            "200 OK",
            &[],
            r#"{"access_token":"n3w","refresh_token":"r2","expires_in":3600}"#,
        );
        let (url, requests) = canned_server(vec![
            http_resp("401 Unauthorized", &[], ""),
            refreshed,
            http_resp("200 OK", &[], "[]"),
        ]);
        let tokens = TokenCell::with_refresh_token(Some("r1".into()));
        let (now, utc) = tokens.current_time();
        tokens.set(GrantedToken::new(
            "old".into(),
            Some("r1".into()),
            Some(600),
            now,
            utc,
        ));
        let rotated = Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
        let sink = rotated.clone();
        tokens.set_rotation_listener(Arc::new(move |token| sink.lock().unwrap().push(token)));
        let mut api = client(&url).with_tokens(tokens.clone());
        api.my_projects().await.unwrap();

        let requests: Vec<String> = requests.try_iter().collect();
        assert_eq!(header_of(&requests[0], "authorization"), Some("Bearer old"));
        let form = body_of(&requests[1]);
        assert!(
            form.contains("grant_type=refresh_token") && form.contains("refresh_token=r1"),
            "{form}"
        );
        assert!(form.contains(&format!("device_id={DEVICE}")), "{form}");
        assert!(!form.contains("password"), "{form}");
        assert_eq!(header_of(&requests[2], "authorization"), Some("Bearer n3w"));
        assert_eq!(
            tokens.refresh_token().as_deref(),
            Some("r2"),
            "the rotated refresh token is kept"
        );
        assert_eq!(
            *rotated.lock().unwrap(),
            vec!["r2".to_string()],
            "and handed to the listener"
        );
    }

    #[tokio::test]
    async fn an_unrotated_refresh_token_is_not_reported_again() {
        let same = http_resp(
            "200 OK",
            &[],
            r#"{"access_token":"n3w","refresh_token":"r1","expires_in":3600}"#,
        );
        let (url, _requests) = canned_server(vec![same, http_resp("200 OK", &[], "[]")]);
        let tokens = TokenCell::with_refresh_token(Some("r1".into()));
        let calls = Arc::new(AtomicU64::new(0));
        let counter = calls.clone();
        tokens.set_rotation_listener(Arc::new(move |_| {
            counter.fetch_add(1, Ordering::SeqCst);
        }));
        client(&url)
            .with_tokens(tokens)
            .workspaces_mine()
            .await
            .unwrap();
        assert_eq!(calls.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn a_refused_refresh_token_always_means_sign_in_expired() {
        // Cross-lane contract: "Refresh refused with invalid_grant -> 'sign-in
        // expired' state, never a silent password re-login." This must hold
        // whether or not a password is remembered.
        let refused = http_resp(
            "400 Bad Request",
            &[],
            r#"{"error":"invalid_grant","error_description":"The token is no longer valid."}"#,
        );

        // With a saved password: still rejected, never a silent password grant.
        let (url, requests) = canned_server(vec![refused.clone()]);
        let mut api = client(&url).with_tokens(TokenCell::with_refresh_token(Some("r1".into())));
        assert!(api.workspaces_mine().await.unwrap_err().is_auth_rejected());
        assert_eq!(
            requests.try_iter().count(),
            1,
            "no password grant was attempted even though a password is remembered"
        );

        // Without a saved password the refusal means signing in again, as before.
        let (url, requests) = canned_server(vec![refused]);
        let mut api = ApiClient::new(url, "alice@acme.com".into(), String::new())
            .with_password(None)
            .with_tokens(TokenCell::with_refresh_token(Some("r1".into())));
        assert!(api.workspaces_mine().await.unwrap_err().is_auth_rejected());
        assert_eq!(
            requests.try_iter().count(),
            1,
            "no password grant was attempted"
        );
    }

    #[tokio::test]
    async fn redirects_are_never_followed() {
        let (url, requests) = canned_server(vec![
            token_t0k3n(),
            http_resp(
                "307 Temporary Redirect",
                &[("Location", "http://127.0.0.1:9/steal")],
                "",
            ),
        ]);
        let mut api = client(&url);
        match api.submit_activity(&sample_activity(), None).await {
            Err(SubmitError::Transient { reason, .. }) => {
                assert!(reason.contains("307"), "{reason}")
            }
            other => panic!("expected a transient failure, got {other:?}"),
        }
        assert_eq!(requests.try_iter().count(), 2);
    }

    #[tokio::test]
    async fn heartbeats_are_bounded_by_their_timeout() {
        // Accepts connections but never answers.
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        std::thread::spawn(move || {
            let mut held = Vec::new();
            for stream in listener.incoming() {
                held.push(stream);
            }
        });
        let started = Instant::now();
        let result = client(&url)
            .heartbeat(&heartbeat(), Duration::from_millis(800))
            .await;
        assert!(matches!(result, Err(ApiError::Offline(_))), "{result:?}");
        assert!(started.elapsed() < Duration::from_secs(3));
    }

    #[tokio::test]
    async fn heartbeats_post_the_normalized_contract_body() {
        let (url, requests) = canned_server(vec![
            token_t0k3n(),
            http_resp("200 OK", &[], r#"{"serverTime":"2026-10-05T10:00:00Z"}"#),
        ]);
        let long = Heartbeat {
            device_name: Some(format!("  {}  ", "N".repeat(300))),
            status: crate::agent::model::PresenceStatus::Tracking,
            ..heartbeat()
        };
        let result = client(&url)
            .heartbeat(&long, Duration::from_secs(5))
            .await
            .unwrap();
        assert_eq!(
            result.server_time,
            crate::agent::model::parse_server_time("2026-10-05T10:00:00Z")
        );
        let request = requests.try_iter().nth(1).unwrap();
        assert!(
            request.starts_with("POST /api/app/presence/heartbeat "),
            "{request}"
        );
        let body: serde_json::Value = serde_json::from_str(body_of(&request)).unwrap();
        assert!(body["sentAt"].is_string());
        assert_eq!(body["status"], "tracking");
        assert_eq!(body["deviceName"].as_str().unwrap().len(), 128);
    }

    #[tokio::test]
    async fn terminal_workspace_and_device_answers_have_their_own_errors() {
        let (url, _requests) = canned_server(vec![
            token_t0k3n(),
            http_resp(
                "403 Forbidden",
                &[],
                &abp(Some(codes::NOT_WORKSPACE_MEMBER), "Gone"),
            ),
            http_resp(
                "403 Forbidden",
                &[],
                &abp(Some(codes::WORKSPACE_ARCHIVED), "Archived"),
            ),
            http_resp(
                "403 Forbidden",
                &[],
                &abp(Some(codes::DEVICE_REVOKED), "Revoked"),
            ),
            http_resp(
                "400 Bad Request",
                &[],
                &abp(Some(codes::WORKSPACE_REQUIRED), "Pick"),
            ),
            http_resp(
                "403 Forbidden",
                &[],
                &abp(Some("Volo.Authorization:010001"), "Authorization failed!"),
            ),
        ]);
        let mut api = client(&url);
        assert!(
            matches!(api.my_projects().await, Err(ApiError::NotWorkspaceMember(m)) if m == "Gone")
        );
        assert!(matches!(
            api.my_projects().await,
            Err(ApiError::WorkspaceArchived(_))
        ));
        let revoked = api
            .heartbeat(&heartbeat(), Duration::from_secs(5))
            .await
            .unwrap_err();
        assert!(matches!(revoked, ApiError::DeviceRevoked(_)) && revoked.is_terminal());
        assert!(matches!(
            api.my_projects().await,
            Err(ApiError::WorkspaceRequired(_))
        ));
        let other = api.my_projects().await.unwrap_err();
        assert_eq!(other.status(), Some(403));
        assert!(!other.is_terminal());
        assert!(other.to_string().contains("Authorization failed!"));
    }

    #[tokio::test]
    async fn an_unreachable_server_is_transient_and_offline() {
        // Nothing listens on port 9 (discard) locally: a real connect error.
        let mut api = client("http://127.0.0.1:9");
        assert!(matches!(api.login().await, Err(AuthError::Offline(_))));
        assert!(matches!(
            api.submit_activity(&sample_activity(), None).await,
            Err(SubmitError::Transient { .. })
        ));
        assert!(matches!(api.my_projects().await, Err(ApiError::Offline(_))));
    }

    #[test]
    fn v1_rows_upload_through_the_shared_model_with_null_counts() {
        let mut env = sample_envelope("act-v1");
        env.mouse_clicks = None;
        env.keyboard_hits = Some(0);
        env.productivity = None;
        let json = serde_json::to_value(Activity::from(&env)).unwrap();
        assert_eq!(json["clientActivityId"], "act-v1");
        assert_eq!(json["projectId"], "proj-uuid-1");
        assert!(json["mouseClicks"].is_null(), "not measured: null, never 0");
        assert_eq!(json["keyboardHits"], 0, "measured zero stays 0");
        assert!(json.get("productivity").is_none());
        assert_eq!(json["platform"], "linux");
    }

    #[test]
    fn new_shared_requires_a_guid_workspace_and_an_allowed_server() {
        let config = |server: &str, workspace: &str| TransportConfig {
            server: server.into(),
            account: "Alice@Acme.com".into(),
            password: None,
            tokens: TokenCell::default(),
            workspace_id: workspace.into(),
            device_id: Some(DEVICE.into()),
            signing_key: None,
            clock: ServerClock::default(),
            allow_insecure_http: false,
        };
        let transport = HttpDeliveryTransport::new_shared(config("https://t.example", W1)).unwrap();
        assert_eq!(transport.bound_account_id(), "alice@acme.com");
        assert_eq!(transport.bound_workspace_id(), W1);
        assert!(HttpDeliveryTransport::new_shared(config("https://t.example", "ws-1")).is_err());
        assert!(HttpDeliveryTransport::new_shared(config("http://t.example", W1)).is_err());
    }
}
