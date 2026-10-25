//! What can go wrong talking to the server, and how each answer is
//! classified (CONTRACT §1, terminal server errors).

use std::time::Duration;

use crate::model::AbpError;

use super::codes;

/// An error and its causes on one line, for logs ("error sending request …:
/// tcp connect error: …").
pub(super) fn describe(error: &(dyn std::error::Error + 'static)) -> String {
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

/// Why signing in (`POST /connect/token`) failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuthError {
    /// The server refused the credentials (OpenIddict `invalid_grant`, or
    /// HTTP 401/403): the sign-in is no longer valid. Carries the server's
    /// `error_description` ("Invalid username or password!", locked out, …).
    Rejected(String),
    /// The server could not be reached (DNS, connect, TLS, timeout). Says
    /// nothing about the credentials.
    Offline(String),
    /// The server answered but failed (5xx, throttling, an unexpected status
    /// or body). Says nothing about the credentials either.
    Server(String),
}

impl AuthError {
    pub fn is_rejected(&self) -> bool {
        matches!(self, AuthError::Rejected(_))
    }

    /// What the sign-in screen shows.
    pub fn user_message(&self) -> String {
        match self {
            AuthError::Rejected(reason) | AuthError::Server(reason) => reason.clone(),
            AuthError::Offline(_) => "Cannot reach the server. Check the server address and your connection.".to_string(),
        }
    }
}

impl std::fmt::Display for AuthError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AuthError::Rejected(reason) => write!(f, "sign-in refused: {reason}"),
            AuthError::Offline(reason) => write!(f, "server unreachable: {reason}"),
            AuthError::Server(reason) => write!(f, "sign-in failed: {reason}"),
        }
    }
}

impl std::error::Error for AuthError {}

/// Why the server refused the workspace a request named. Each is terminal
/// for that workspace (CONTRACT, "terminal server errors"): capture and
/// heartbeats stop there and another workspace is asked for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkspaceRefusal {
    /// 403 `Tracker:NotWorkspaceMember`: the account lost access to it.
    NotMember,
    /// 403 `Tracker:WorkspaceArchived`.
    Archived,
    /// 400 `Tracker:WorkspaceRequired`: the request reached the server
    /// without its workspace (a proxy stripped the header) — a setup
    /// problem, never a reason to park a block.
    Required,
}

impl WorkspaceRefusal {
    /// The refusal a business error code stands for.
    pub fn of_code(code: Option<&str>) -> Option<Self> {
        match code? {
            codes::NOT_WORKSPACE_MEMBER => Some(WorkspaceRefusal::NotMember),
            codes::WORKSPACE_ARCHIVED => Some(WorkspaceRefusal::Archived),
            codes::WORKSPACE_REQUIRED => Some(WorkspaceRefusal::Required),
            _ => None,
        }
    }
}

impl std::fmt::Display for WorkspaceRefusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            WorkspaceRefusal::NotMember => "not a workspace member",
            WorkspaceRefusal::Archived => "workspace archived",
            WorkspaceRefusal::Required => "workspace required",
        })
    }
}

/// Why a request other than an upload failed.
#[derive(Debug)]
pub enum ApiError {
    /// Signing in (again) failed.
    Auth(AuthError),
    /// The server could not be reached, or did not answer in time.
    Offline(String),
    /// The server answered with an error status (after one sign-in retry on
    /// a 401), and its ABP error body when it sent one.
    Http { status: u16, error: Option<AbpError> },
    /// The server refused the workspace the request named.
    Workspace { refusal: WorkspaceRefusal, message: String },
    /// 403 `Tracker:DeviceRevoked`: a workspace admin removed this computer.
    DeviceRevoked(String),
    /// The answer could not be read.
    Invalid(String),
}

impl ApiError {
    /// The saved credentials were refused: the sign-in has expired.
    #[cfg_attr(not(test), expect(dead_code, reason = "for the worker's sign-in-expired handling"))]
    pub fn is_auth_rejected(&self) -> bool {
        matches!(self, ApiError::Auth(AuthError::Rejected(_)))
    }

    /// The HTTP status, when the server answered with an error.
    #[cfg_attr(not(test), expect(dead_code, reason = "for the worker's error handling"))]
    pub fn status(&self) -> Option<u16> {
        match self {
            ApiError::Http { status, .. } => Some(*status),
            _ => None,
        }
    }
}

impl std::fmt::Display for ApiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ApiError::Auth(e) => e.fmt(f),
            ApiError::Offline(reason) => write!(f, "server unreachable: {reason}"),
            ApiError::Http { status, error } => match error.as_ref().and_then(AbpError::summary) {
                Some(summary) => write!(f, "HTTP {status}: {summary}"),
                None => write!(f, "HTTP {status}"),
            },
            ApiError::Workspace { refusal, message } => write!(f, "{refusal}: {message}"),
            ApiError::DeviceRevoked(message) => write!(f, "device revoked: {message}"),
            ApiError::Invalid(reason) => write!(f, "unexpected response: {reason}"),
        }
    }
}

impl std::error::Error for ApiError {}

/// How an upload failed, so the sync loop can act on it (CONTRACT §1, "How
/// agents treat upload responses"). Only [`SubmitError::Rejected`] parks the
/// row; every other outcome keeps it.
#[derive(Debug)]
pub enum SubmitError {
    /// Network trouble, a timeout, a redirect, 408/429/5xx/404/405 or a 4xx
    /// without an ABP error code: the server's (or the network's) trouble,
    /// not this row's. Keep the row and retry the queue with backoff, no
    /// sooner than `retry_after` when the server sent one.
    Transient { reason: String, retry_after: Option<Duration> },
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
    /// The server refused the row's workspace (access lost, archived, or the
    /// header never arrived). Keep the row: that workspace's rows are held
    /// until it is usable again, and the rest of the queue goes on.
    Workspace { refusal: WorkspaceRefusal, message: String },
    /// 403 `Tracker:DeviceRevoked`: a workspace admin removed this computer.
    /// Keep the queue; nothing more is sent from this device.
    DeviceRevoked(String),
    /// A business error this agent does not know, about this row: keep it and
    /// retry it later on its own schedule while the rows behind it go on;
    /// parked after too many attempts.
    RowRetry(String),
    /// A definitive rejection of this block (overlap, too long/old, invalid
    /// time range, id conflict, validation errors): park it with the server's
    /// message.
    Rejected(String),
}

impl std::fmt::Display for SubmitError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SubmitError::Transient { reason, retry_after: Some(wait) } => {
                write!(f, "{reason} (retry after {}s)", wait.as_secs())
            }
            SubmitError::Transient { reason, retry_after: None } => write!(f, "{reason}"),
            SubmitError::AuthRejected(reason) => write!(f, "sign-in refused: {reason}"),
            SubmitError::SubscriptionInactive(message) => write!(f, "subscription inactive: {message}"),
            SubmitError::ProjectUnavailable { code, message } => write!(f, "{code}: {message}"),
            SubmitError::ClockAhead(message) => write!(f, "clock ahead: {message}"),
            SubmitError::StripCaptures(message) => write!(f, "captures refused: {message}"),
            SubmitError::Workspace { refusal, message } => write!(f, "{refusal}: {message}"),
            SubmitError::DeviceRevoked(message) => write!(f, "device revoked: {message}"),
            SubmitError::RowRetry(message) => write!(f, "{message}"),
            SubmitError::Rejected(message) => write!(f, "rejected: {message}"),
        }
    }
}

/// Classify an upload answer that is not a success. Business error codes
/// decide first (an older server answers 403 for every unmapped business
/// error), then the status. A 401 never gets here: the client signs in again
/// and resends first.
pub fn classify_upload_failure(status: u16, body: &str, retry_after: Option<Duration>) -> SubmitError {
    let error = AbpError::from_body(body);
    let code = error.as_ref().and_then(AbpError::code);
    let message = error
        .as_ref()
        .and_then(AbpError::summary)
        .unwrap_or_else(|| format!("HTTP {status}"));
    let transient = |reason: String| SubmitError::Transient { reason, retry_after };

    if status == 413 || matches!(code, Some(codes::CAPTURE_TOO_LARGE | codes::INVALID_CAPTURE_DATA)) {
        return SubmitError::StripCaptures(message);
    }
    if !(400..500).contains(&status) {
        // 5xx (or anything unexpected): the server's trouble, not this block's.
        return transient(format!("HTTP {status}"));
    }
    // Terminal for the device or the workspace, whatever came with them.
    if code == Some(codes::DEVICE_REVOKED) {
        return SubmitError::DeviceRevoked(message);
    }
    if let Some(refusal) = WorkspaceRefusal::of_code(code) {
        return SubmitError::Workspace { refusal, message };
    }
    let invalid_payload = status == 400 && error.as_ref().is_some_and(AbpError::has_validation_errors);
    match code {
        Some(codes::SUBSCRIPTION_INACTIVE) => SubmitError::SubscriptionInactive(message),
        Some(code @ (codes::NOT_PROJECT_MEMBER | codes::PROJECT_ARCHIVED)) => {
            SubmitError::ProjectUnavailable { code: code.to_string(), message }
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
        // it later rather than guess that it is permanent — without holding
        // up the rows behind it.
        Some(other) => SubmitError::RowRetry(format!("HTTP {status} {other}: {message}")),
        // 404, 405, 408, 429, or any 4xx without an ABP error code (a proxy,
        // a missing route).
        None => transient(format!("HTTP {status}")),
    }
}

/// Why an authorized request could not get an answer.
pub(super) enum RequestError {
    Auth(AuthError),
    Transport(reqwest::Error),
}

impl From<RequestError> for ApiError {
    fn from(error: RequestError) -> Self {
        match error {
            // Unreachable is unreachable, whichever request found out.
            RequestError::Auth(AuthError::Offline(reason)) => ApiError::Offline(reason),
            RequestError::Auth(e) => ApiError::Auth(e),
            RequestError::Transport(e) => ApiError::Offline(describe(&e)),
        }
    }
}

/// The error for a failed answer: a refused workspace and a revoked device
/// have their own kinds (they are terminal).
pub(super) fn http_error(status: u16, body: &str) -> ApiError {
    let error = AbpError::from_body(body);
    let code = error.as_ref().and_then(AbpError::code);
    if (400..500).contains(&status) {
        let message = || error.as_ref().and_then(AbpError::summary).unwrap_or_else(|| format!("HTTP {status}"));
        if code == Some(codes::DEVICE_REVOKED) {
            return ApiError::DeviceRevoked(message());
        }
        if let Some(refusal) = WorkspaceRefusal::of_code(code) {
            return ApiError::Workspace { refusal, message: message() };
        }
    }
    ApiError::Http { status, error }
}
