use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};
use reqwest::header::{HeaderMap, ACCEPT, DATE, RETRY_AFTER};
use reqwest::StatusCode;
use serde::de::DeserializeOwned;
use serde::Deserialize;

use crate::model::{AbpError, Activity, Heartbeat, HeartbeatResult, Project};

/// Percent-encode a query-parameter value (workspace names may contain spaces
/// or other characters that would otherwise break the URL).
fn urlencode(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for byte in value.as_bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(*byte as char)
            }
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

/// Subset of `GET /api/app/reporting/summary` the desktop UI displays.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReportSummary {
    #[serde(default)]
    pub total_tracked_minutes: f64,
    /// `None` when nothing in the range was measured.
    #[serde(default)]
    pub average_productivity: Option<f64>,
}

/// Query parameters for `GET /api/app/reporting/summary`. `TimeZone` (an
/// IANA id) makes the server's "days" the device's calendar days.
///
/// Passed through `RequestBuilder::query` so they are percent-encoded: a raw
/// `+00:00` offset in a hand-built URL is decoded by the server as a space,
/// which fails model binding (HTTP 400) and left the status cards at zero.
fn summary_query(from: DateTime<Utc>, to: DateTime<Utc>, time_zone: Option<&str>) -> Vec<(&'static str, String)> {
    let iso = |t: DateTime<Utc>| t.to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
    let mut query = vec![("From", iso(from)), ("To", iso(to))];
    if let Some(zone) = time_zone.map(str::trim).filter(|zone| !zone.is_empty()) {
        query.push(("TimeZone", zone.to_string()));
    }
    query
}

/// OpenIddict client id seeded by the backend (`Tracker_App`, password grant enabled).
const CLIENT_ID: &str = "Tracker_App";
const SCOPE: &str = "Tracker";

/// Give up on a TCP/TLS connect that has not completed by then, so a dead or
/// black-holed server can never hang the worker (and with it the UI's
/// pause/sync commands) indefinitely.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
/// Whole-request budget for sign-in, projects and totals.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
/// Whole-request budget for an upload, which carries a screenshot (and maybe
/// a webcam frame) over what may be a slow uplink.
const UPLOAD_TIMEOUT: Duration = Duration::from_secs(120);
/// Longest `Retry-After` honoured: a broken or hostile value must not stall
/// the queue for days.
const MAX_RETRY_AFTER: Duration = Duration::from_secs(6 * 60 * 60);

/// A device clock off by more than this gets a warning (CONTRACT §1).
#[cfg_attr(not(test), expect(dead_code, reason = "read by the window's clock warning"))]
pub const CLOCK_SKEW_WARNING_SECS: i64 = 120;

/// The server's business error codes the agent acts on (CONTRACT §1).
pub mod codes {
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
}

/// Build the shared HTTP client.
///
/// TLS trusts the Windows certificate store (so corporate CAs and the ASP.NET
/// dev certificate installed by `dotnet dev-certs https --trust` work) *in
/// addition to* the bundled Mozilla roots (so public CAs Windows has not
/// fetched yet still work). Certificate verification is always on.
fn build_http_client() -> reqwest::Client {
    let builder = || {
        reqwest::Client::builder()
            .user_agent(concat!("dosi-tracker/", env!("CARGO_PKG_VERSION")))
            .connect_timeout(CONNECT_TIMEOUT)
            .timeout(REQUEST_TIMEOUT)
    };
    builder().build().unwrap_or_else(|e| {
        // An unreadable Windows store must not take tracking down: fall back to
        // the bundled roots only (still fully verified).
        tracing::warn!(?e, "could not load the Windows certificate store; using bundled roots");
        builder()
            .tls_built_in_native_certs(false)
            .build()
            .expect("failed to build http client")
    })
}

/// The server's clock relative to this device's, read from the `Date`
/// header of the latest response: server minus device, in whole seconds
/// (positive = this computer's clock is behind). Clones share the value, so
/// the window can read what the worker's client last saw.
#[derive(Debug, Clone)]
pub struct ServerClock(Arc<AtomicI64>);

/// Marks "no response seen yet".
const UNKNOWN_OFFSET: i64 = i64::MIN;

impl Default for ServerClock {
    fn default() -> Self {
        Self(Arc::new(AtomicI64::new(UNKNOWN_OFFSET)))
    }
}

impl ServerClock {
    /// Server minus device, in seconds, from the latest response.
    #[cfg_attr(not(test), expect(dead_code, reason = "read by the window's clock warning"))]
    pub fn offset_secs(&self) -> Option<i64> {
        Some(self.0.load(Ordering::Relaxed)).filter(|&offset| offset != UNKNOWN_OFFSET)
    }

    pub fn record(&self, offset_secs: i64) {
        self.0.store(offset_secs.max(UNKNOWN_OFFSET + 1), Ordering::Relaxed);
    }

    /// Minutes the device clock is off by, once that is worth a warning:
    /// positive = behind the server, negative = ahead.
    #[cfg_attr(not(test), expect(dead_code, reason = "read by the window's clock warning"))]
    pub fn skew_minutes(&self) -> Option<i64> {
        self.offset_secs().and_then(skew_warning_minutes)
    }
}

/// The "clock is off by N min" warning for an offset, if it exceeds
/// [`CLOCK_SKEW_WARNING_SECS`]. Timestamps are never rewritten.
#[cfg_attr(not(test), expect(dead_code, reason = "read by the window's clock warning"))]
pub fn skew_warning_minutes(offset_secs: i64) -> Option<i64> {
    (offset_secs.abs() > CLOCK_SKEW_WARNING_SECS).then(|| (offset_secs as f64 / 60.0).round() as i64)
}

/// Server minus device seconds, from an HTTP `Date` header
/// (`Tue, 05 Oct 2026 10:00:00 GMT`).
pub fn clock_offset_from_date(date_header: &str, device_now: DateTime<Utc>) -> Option<i64> {
    let server = DateTime::parse_from_rfc2822(date_header.trim()).ok()?.with_timezone(&Utc);
    Some((server - device_now).num_seconds())
}

/// How long a `Retry-After` header asks to wait: delta-seconds or an HTTP
/// date. A date is measured against the server's own clock when it sent one,
/// so a wrong device clock does not distort the wait.
pub fn parse_retry_after(value: &str, server_now: Option<DateTime<Utc>>, device_now: DateTime<Utc>) -> Option<Duration> {
    let value = value.trim();
    let wait = match value.parse::<u64>() {
        Ok(seconds) => Duration::from_secs(seconds),
        Err(_) => {
            let at = DateTime::parse_from_rfc2822(value).ok()?.with_timezone(&Utc);
            (at - server_now.unwrap_or(device_now)).to_std().unwrap_or_default()
        }
    };
    Some(wait.min(MAX_RETRY_AFTER))
}

fn retry_after_of(headers: &HeaderMap) -> Option<Duration> {
    let value = headers.get(RETRY_AFTER)?.to_str().ok()?;
    let server_now = headers
        .get(DATE)
        .and_then(|date| date.to_str().ok())
        .and_then(|date| DateTime::parse_from_rfc2822(date.trim()).ok())
        .map(|date| date.with_timezone(&Utc));
    parse_retry_after(value, server_now, Utc::now())
}

/// An error and its causes on one line, for logs ("error sending request …:
/// tcp connect error: …").
fn describe(error: &(dyn std::error::Error + 'static)) -> String {
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

/// OpenIddict's error body: `{"error":"invalid_grant","error_description":"…"}`.
#[derive(Debug, Default, Deserialize)]
struct OAuthError {
    #[serde(default)]
    error: Option<String>,
    #[serde(default)]
    error_description: Option<String>,
}

/// The header ABP's multi-tenancy sets when it cannot open the workspace a
/// request names (unknown or inactive), with its reason, HTML-encoded.
const TENANT_RESOLVE_ERROR: &str = "Abp-Tenant-Resolve-Error";

/// The longest server reason quoted in a sign-in message.
const MAX_REASON_CHARS: usize = 200;

/// The sign-in failed because the server could not open the workspace
/// (`Abp-Tenant-Resolve-Error`). Not a refusal of the credentials: ABP
/// answers the same way when looking the workspace up fails on its side, so
/// a saved sign-in is kept and retried; the sign-in form shows the reason.
pub fn workspace_unavailable(header: &str) -> AuthError {
    let reason: String = decode_html_text(header).trim().chars().take(MAX_REASON_CHARS).collect();
    if reason.is_empty() {
        AuthError::Server("The server could not open this workspace. Check the workspace name.".to_string())
    } else {
        AuthError::Server(format!("The server could not open this workspace ({reason}). Check the workspace name."))
    }
}

/// Undo HTML encoding (`&amp;`, `&#x27;`, `&#39;`, …); anything that is not a
/// well-formed entity is kept as written.
fn decode_html_text(text: &str) -> String {
    fn entity(name: &str) -> Option<char> {
        match name {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            _ => {
                let number = name.strip_prefix('#')?;
                let code = match number.strip_prefix(['x', 'X']) {
                    Some(hex) => u32::from_str_radix(hex, 16).ok()?,
                    None => number.parse().ok()?,
                };
                char::from_u32(code)
            }
        }
    }

    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find('&') {
        out.push_str(&rest[..start]);
        let tail = &rest[start..];
        let decoded = tail
            .find(';')
            .filter(|&end| end > 1 && end <= 10)
            .and_then(|end| entity(&tail[1..end]).map(|c| (c, end + 1)));
        match decoded {
            Some((c, length)) => {
                out.push(c);
                rest = &tail[length..];
            }
            None => {
                out.push('&');
                rest = &tail[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

/// Classify a token-endpoint failure. Only a refusal of the credentials is
/// [`AuthError::Rejected`]; a bare 400 from a proxy, a 404 or a 5xx is not.
pub fn classify_token_failure(status: u16, body: &str) -> AuthError {
    let oauth: OAuthError = serde_json::from_str(body).unwrap_or_default();
    let description = oauth
        .error_description
        .map(|d| d.trim().to_string())
        .filter(|d| !d.is_empty());
    if oauth.error.as_deref() == Some("invalid_grant") || matches!(status, 401 | 403) {
        return AuthError::Rejected(description.unwrap_or_else(|| "Incorrect email or password.".to_string()));
    }
    match status {
        429 => AuthError::Server("Too many sign-in attempts. Try again in a minute.".to_string()),
        500.. => AuthError::Server(format!("The server had a problem (HTTP {status}). Try again shortly.")),
        _ => AuthError::Server(description.unwrap_or_else(|| format!("Sign-in failed (HTTP {status})."))),
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
    /// Network trouble, a timeout, 408/429/5xx/404/405, a 4xx without an ABP
    /// error code, or a business error this agent does not know: keep the row
    /// and retry with backoff, no sooner than `retry_after` when the server
    /// sent one.
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
        // later rather than guess that it is permanent.
        Some(other) => transient(format!("HTTP {status} {other}: {message}")),
        // 404, 405, 408, 429, or any 4xx without an ABP error code (a proxy,
        // a missing route).
        None => transient(format!("HTTP {status}")),
    }
}

/// Why an authorized request could not get an answer.
enum RequestError {
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

/// Thin HTTP client wrapping the Dosi-Tracker backend REST API.
///
/// Authentication uses the backend's OpenIddict token endpoint
/// (`POST /connect/token`, resource-owner password flow). The token is cached
/// and transparently refreshed via re-login shortly before it expires, and
/// once more when a request answers 401.
pub struct ApiClient {
    http: reqwest::Client,
    base_url: String,
    username: String,
    password: String,
    /// Workspace (tenant) name. Empty means the host account.
    tenant: String,
    token: Option<Token>,
    clock: ServerClock,
}

struct Token {
    access_token: String,
    expires_at: Instant,
}

#[derive(Deserialize)]
struct TokenResponse {
    access_token: String,
    #[serde(default = "default_expires_in")]
    expires_in: u64,
}

fn default_expires_in() -> u64 {
    3600
}

impl ApiClient {
    pub fn new(
        base_url: impl Into<String>,
        username: String,
        password: String,
        tenant: String,
    ) -> Self {
        // Endpoints are appended as "/path": a trailing slash would make
        // every one of them "//path".
        let base_url = base_url.into().trim().trim_end_matches('/').to_string();
        Self {
            http: build_http_client(),
            base_url,
            username,
            password,
            tenant,
            token: None,
            clock: ServerClock::default(),
        }
    }

    /// Share `clock` with this client (e.g. one the window already reads), so
    /// it survives signing in again.
    #[cfg_attr(not(test), expect(dead_code, reason = "read by the window's clock warning"))]
    pub fn with_server_clock(mut self, clock: ServerClock) -> Self {
        self.clock = clock;
        self
    }

    /// The server clock offset seen in the latest response's `Date` header.
    #[cfg_attr(not(test), expect(dead_code, reason = "read by the window's clock warning"))]
    pub fn server_clock(&self) -> ServerClock {
        self.clock.clone()
    }

    /// The server this client talks to, without a trailing slash.
    #[cfg_attr(not(test), expect(dead_code, reason = "for the worker's saved server address"))]
    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    /// The account this client acts as, in the form the offline queue tags rows
    /// with — so rows are only ever uploaded under their owner's token.
    pub fn owner_key(&self) -> String {
        crate::storage::owner_key(&self.tenant, &self.username)
    }

    /// Send a request and note the server's clock from its `Date` header.
    async fn execute(&self, request: reqwest::RequestBuilder) -> Result<reqwest::Response, reqwest::Error> {
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

    /// Authenticate against the OpenIddict token endpoint and cache the bearer token.
    ///
    /// The workspace (tenant) MUST travel as a query parameter: ABP's tenant
    /// resolvers read route/query/header/cookie, **not** the POST body. Putting
    /// `__tenant` in the form body silently resolves to the host tenant, which
    /// makes a tenant user's sign-in fail (or, worse, authenticate as host).
    pub async fn login(&mut self) -> Result<(), AuthError> {
        let url = if self.tenant.is_empty() {
            format!("{}/connect/token", self.base_url)
        } else {
            format!(
                "{}/connect/token?__tenant={}",
                self.base_url,
                urlencode(&self.tenant)
            )
        };
        let params = [
            ("grant_type", "password"),
            ("username", self.username.as_str()),
            ("password", self.password.as_str()),
            ("client_id", CLIENT_ID),
            ("scope", SCOPE),
        ];

        let response = self
            .execute(self.http.post(url).header(ACCEPT, "application/json").form(&params))
            .await
            .map_err(|e| AuthError::Offline(describe(&e)))?;
        let status = response.status();
        let tenant_error = response
            .headers()
            .get(TENANT_RESOLVE_ERROR)
            .map(|value| String::from_utf8_lossy(value.as_bytes()).into_owned());
        let body = response
            .text()
            .await
            .map_err(|e| AuthError::Offline(describe(&e)))?;
        if !status.is_success() {
            return Err(match tenant_error {
                Some(header) => workspace_unavailable(&header),
                None => classify_token_failure(status.as_u16(), &body),
            });
        }
        let token: TokenResponse = serde_json::from_str(&body)
            .map_err(|e| AuthError::Server(format!("The server sent an unexpected sign-in response ({e}).")))?;

        // Refresh one minute early so requests never race the expiry.
        let ttl = token.expires_in.saturating_sub(60).max(60);
        self.token = Some(Token {
            access_token: token.access_token,
            expires_at: Instant::now() + Duration::from_secs(ttl),
        });
        Ok(())
    }

    async fn ensure_token(&mut self) -> Result<String, AuthError> {
        let expired = match &self.token {
            Some(token) => Instant::now() >= token.expires_at,
            None => true,
        };
        if expired {
            self.login().await?;
        }
        Ok(self
            .token
            .as_ref()
            .map(|t| t.access_token.clone())
            .expect("token present after login"))
    }

    /// Send an authorized request built by `build` (given the HTTP client and
    /// a bearer token). On a 401 the token was revoked or expired server-side:
    /// sign in again once and resend, so a stale token never costs a request.
    async fn send_authorized<F>(&mut self, build: F) -> Result<reqwest::Response, RequestError>
    where
        F: Fn(&reqwest::Client, &str) -> reqwest::RequestBuilder,
    {
        let token = self.ensure_token().await.map_err(RequestError::Auth)?;
        let response = self
            .execute(build(&self.http, &token))
            .await
            .map_err(RequestError::Transport)?;
        if response.status() != StatusCode::UNAUTHORIZED {
            return Ok(response);
        }
        self.token = None;
        let token = self.ensure_token().await.map_err(RequestError::Auth)?;
        let response = self
            .execute(build(&self.http, &token))
            .await
            .map_err(RequestError::Transport)?;
        if response.status() == StatusCode::UNAUTHORIZED {
            self.token = None;
        }
        Ok(response)
    }

    /// The active (non-archived) projects the user may track, with the
    /// per-project capture permissions and policy the backend pushes down
    /// (`GET /api/app/project/my-projects`).
    pub async fn my_projects(&mut self) -> Result<Vec<Project>, ApiError> {
        let url = format!("{}/api/app/project/my-projects", self.base_url);
        let response = self.send_authorized(|http, token| http.get(&url).bearer_auth(token)).await?;
        read_json(response).await
    }

    /// Tracked-time totals for a window, used by the desktop UI's status cards.
    /// Non-admin callers are scoped to their own data server-side. With a
    /// `time_zone` (IANA id) the server's days are that zone's calendar days.
    pub async fn summary(
        &mut self,
        from: DateTime<Utc>,
        to: DateTime<Utc>,
        time_zone: Option<&str>,
    ) -> Result<ReportSummary, ApiError> {
        let url = format!("{}/api/app/reporting/summary", self.base_url);
        let query = summary_query(from, to, time_zone);
        let response = self
            .send_authorized(|http, token| http.get(&url).query(&query).bearer_auth(token))
            .await?;
        read_json(response).await
    }

    /// Report presence (`POST /api/app/presence/heartbeat`). The whole call,
    /// a sign-in included, is bounded by `timeout` — 1-2 s for the
    /// best-effort one sent when quitting. `sentAt` is stamped here. A failed
    /// heartbeat is never retried or queued.
    #[cfg_attr(not(test), expect(dead_code, reason = "called by the worker's presence loop"))]
    pub async fn heartbeat(&mut self, heartbeat: &Heartbeat, timeout: Duration) -> Result<HeartbeatResult, ApiError> {
        let url = format!("{}/api/app/presence/heartbeat", self.base_url);
        let body = heartbeat.clone().normalized();
        let call = async {
            let response = self
                .send_authorized(|http, token| {
                    let mut body = body.clone();
                    body.sent_at = Some(Utc::now());
                    http.post(&url).bearer_auth(token).timeout(timeout).json(&body)
                })
                .await?;
            let status = response.status();
            if !status.is_success() {
                let text = response.text().await.unwrap_or_default();
                return Err(ApiError::Http { status: status.as_u16(), error: AbpError::from_body(&text) });
            }
            // The heartbeat counted even if its answer cannot be read.
            let text = response.text().await.unwrap_or_default();
            Ok(serde_json::from_str(&text).unwrap_or_default())
        };
        tokio::time::timeout(timeout, call)
            .await
            .unwrap_or_else(|_| Err(ApiError::Offline(format!("no answer within {} ms", timeout.as_millis()))))
    }

    /// Upload a completed activity time block. `sentAt` is stamped at send
    /// time (the server derives the device's clock skew from it).
    pub async fn submit_activity(&mut self, activity: &Activity) -> Result<(), SubmitError> {
        let url = format!("{}/api/app/activity", self.base_url);
        let response = self
            .send_authorized(|http, token| {
                let mut body = activity.clone();
                body.sent_at = Some(Utc::now());
                http.post(&url).bearer_auth(token).timeout(UPLOAD_TIMEOUT).json(&body)
            })
            .await;

        let response = match response {
            Ok(response) => response,
            Err(RequestError::Auth(AuthError::Rejected(reason))) => return Err(SubmitError::AuthRejected(reason)),
            Err(RequestError::Auth(e)) => {
                return Err(SubmitError::Transient { reason: e.to_string(), retry_after: None })
            }
            Err(RequestError::Transport(e)) => {
                return Err(SubmitError::Transient { reason: describe(&e), retry_after: None })
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
}

/// Read a JSON answer, or the error the server sent instead.
async fn read_json<T: DeserializeOwned>(response: reqwest::Response) -> Result<T, ApiError> {
    let status = response.status();
    if !status.is_success() {
        let body = response.text().await.unwrap_or_default();
        return Err(ApiError::Http { status: status.as_u16(), error: AbpError::from_body(&body) });
    }
    let body = response.text().await.map_err(|e| ApiError::Offline(describe(&e)))?;
    serde_json::from_str(&body).map_err(|e| ApiError::Invalid(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};

    #[test]
    fn http_client_builds_with_native_roots_and_timeouts() {
        // Loads the Windows certificate store; must never panic on a normal machine.
        let _client = build_http_client();
    }

    #[test]
    fn owner_key_matches_the_queue_tagging() {
        let client = ApiClient::new(
            "https://example.invalid",
            "Alice@Acme.com".into(),
            "secret".into(),
            "Acme".into(),
        );
        assert_eq!(client.owner_key(), crate::storage::owner_key("acme", "alice@acme.com"));
    }

    #[test]
    fn the_base_url_never_ends_with_a_slash() {
        let client = ApiClient::new(" https://tracker.example.com// ", "a".into(), "b".into(), String::new());
        assert_eq!(client.base_url(), "https://tracker.example.com");
    }

    #[test]
    fn summary_query_survives_url_encoding() {
        let from = DateTime::parse_from_rfc3339("2026-09-29T18:00:00+00:00").unwrap().with_timezone(&Utc);
        let to = DateTime::parse_from_rfc3339("2026-09-30T10:29:24.976+00:00").unwrap().with_timezone(&Utc);
        let request = reqwest::Client::new()
            .get("https://example.invalid/api/app/reporting/summary")
            .query(&summary_query(from, to, Some("America/Argentina/Buenos_Aires")))
            .build()
            .unwrap();

        // What the server sees after decoding must still parse to the same instants.
        let pairs: std::collections::HashMap<_, _> = request.url().query_pairs().into_owned().collect();
        let parse = |key: &str| DateTime::parse_from_rfc3339(&pairs[key]).unwrap().with_timezone(&Utc);
        assert_eq!(parse("From"), from);
        assert_eq!(parse("To"), to);
        assert_eq!(pairs["TimeZone"], "America/Argentina/Buenos_Aires");

        // No zone: the parameter is left out (the server's default applies).
        assert_eq!(summary_query(from, to, None).len(), 2);
        assert_eq!(summary_query(from, to, Some("  ")).len(), 2);
    }

    #[test]
    fn workspace_names_are_percent_encoded() {
        assert_eq!(urlencode("Acme Corp/EU"), "Acme%20Corp%2FEU");
        assert_eq!(urlencode("plain-name_1.x~"), "plain-name_1.x~");
    }

    #[test]
    fn report_summaries_tolerate_unmeasured_productivity() {
        let summary: ReportSummary =
            serde_json::from_str(r#"{"totalTrackedMinutes":90.5,"averageProductivity":null,"totalIdleMinutes":12}"#).unwrap();
        assert_eq!(summary.total_tracked_minutes, 90.5);
        assert_eq!(summary.average_productivity, None);
        // An older server always measured.
        let older: ReportSummary = serde_json::from_str(r#"{"totalTrackedMinutes":5,"averageProductivity":61.2}"#).unwrap();
        assert_eq!(older.average_productivity, Some(61.2));
    }

    fn abp(code: Option<&str>, message: &str) -> String {
        let code = code.map(|c| format!("\"{c}\"")).unwrap_or_else(|| "null".to_string());
        format!(r#"{{"error":{{"code":{code},"message":"{message}","details":null}}}}"#)
    }

    #[test]
    fn upload_failures_are_classified_as_the_contract_says() {
        let wait = Some(Duration::from_secs(30));
        let kind = |status: u16, body: &str| match classify_upload_failure(status, body, None) {
            SubmitError::Transient { .. } => "transient",
            SubmitError::AuthRejected(_) => "auth",
            SubmitError::SubscriptionInactive(_) => "subscription",
            SubmitError::ProjectUnavailable { .. } => "project",
            SubmitError::ClockAhead(_) => "clock",
            SubmitError::StripCaptures(_) => "strip",
            SubmitError::Rejected(_) => "park",
        };

        // Transient: infrastructure, throttling and code-less 4xx.
        for status in [404, 405, 408, 429, 500, 502, 503] {
            assert_eq!(kind(status, ""), "transient", "{status}");
        }
        assert_eq!(kind(400, "<html>bad request</html>"), "transient");
        assert_eq!(kind(403, r#"{"title":"Forbidden"}"#), "transient");
        assert_eq!(kind(422, &abp(Some("Tracker:SomethingNew"), "?")), "transient");
        // A 5xx is transient even with a business code.
        assert_eq!(kind(500, &abp(Some(codes::ACTIVITY_OVERLAPS), "x")), "transient");

        // Special handling.
        assert_eq!(kind(402, &abp(Some(codes::SUBSCRIPTION_INACTIVE), "Inactive")), "subscription");
        assert_eq!(kind(402, ""), "transient", "402 without a code");
        assert_eq!(kind(403, &abp(Some(codes::NOT_PROJECT_MEMBER), "Not a member")), "project");
        assert_eq!(kind(409, &abp(Some(codes::PROJECT_ARCHIVED), "Archived")), "project");
        assert_eq!(kind(425, &abp(Some(codes::ACTIVITY_IN_FUTURE), "In the future")), "clock");
        assert_eq!(kind(413, ""), "strip");
        assert_eq!(kind(413, "<html>Request Entity Too Large</html>"), "strip");
        assert_eq!(kind(413, &abp(Some(codes::CAPTURE_TOO_LARGE), "Too big")), "strip");
        assert_eq!(kind(400, &abp(Some(codes::INVALID_CAPTURE_DATA), "Bad image")), "strip");

        // Park: definitive rejections of the block.
        assert_eq!(kind(409, &abp(Some(codes::ACTIVITY_OVERLAPS), "Overlaps")), "park");
        assert_eq!(kind(409, &abp(Some(codes::ACTIVITY_CLIENT_ID_CONFLICT), "Conflict")), "park");
        assert_eq!(kind(422, &abp(Some(codes::ACTIVITY_TOO_LONG), "Too long")), "park");
        assert_eq!(kind(422, &abp(Some(codes::ACTIVITY_TOO_OLD), "Too old")), "park");
        assert_eq!(kind(422, &abp(Some(codes::ACTIVITY_TIME_RANGE_INVALID), "Bad range")), "park");
        let validation = r#"{"error":{"code":null,"message":"Your request is not valid!",
            "validationErrors":[{"message":"Description is too long.","members":["description"]}]}}"#;
        assert_eq!(kind(400, validation), "park");
        // An older server answers 403 for every business error: the code decides.
        assert_eq!(kind(403, &abp(Some(codes::ACTIVITY_TOO_LONG), "Too long")), "park");
        // Validation errors park the block even if a code comes with them.
        let coded_validation = r#"{"error":{"code":"Volo.Abp.Validation","message":"Your request is not valid!",
            "validationErrors":[{"message":"The field TimeZone is invalid.","members":["timeZone"]}]}}"#;
        assert_eq!(kind(400, coded_validation), "park");
        // ...but only on a 400: elsewhere they are not about this payload.
        assert_eq!(kind(422, coded_validation), "transient");

        // The server's message is kept for the window; Retry-After is passed on.
        match classify_upload_failure(400, validation, None) {
            SubmitError::Rejected(message) => assert_eq!(message, "Your request is not valid! Description is too long."),
            other => panic!("expected a rejection, got {other:?}"),
        }
        match classify_upload_failure(429, "", wait) {
            SubmitError::Transient { retry_after, .. } => assert_eq!(retry_after, wait),
            other => panic!("expected transient, got {other:?}"),
        }
        match classify_upload_failure(403, &abp(Some(codes::NOT_PROJECT_MEMBER), "Not a member"), None) {
            SubmitError::ProjectUnavailable { code, message } => {
                assert_eq!(code, codes::NOT_PROJECT_MEMBER);
                assert_eq!(message, "Not a member");
            }
            other => panic!("expected project unavailable, got {other:?}"),
        }
    }

    #[test]
    fn only_refused_credentials_are_a_rejected_sign_in() {
        let invalid_grant = r#"{"error":"invalid_grant","error_description":"Invalid username or password!"}"#;
        assert_eq!(
            classify_token_failure(400, invalid_grant),
            AuthError::Rejected("Invalid username or password!".into())
        );
        let locked = r#"{"error":"invalid_grant","error_description":"The user account has been locked out."}"#;
        assert_eq!(classify_token_failure(400, locked).user_message(), "The user account has been locked out.");
        assert!(classify_token_failure(401, "").is_rejected());
        assert!(classify_token_failure(403, "").is_rejected());
        assert_eq!(classify_token_failure(401, ""), AuthError::Rejected("Incorrect email or password.".into()));

        // Not about the credentials: a proxy's bare 400, a wrong client, a
        // missing endpoint, throttling and server failures.
        assert!(!classify_token_failure(400, "").is_rejected());
        assert!(!classify_token_failure(400, r#"{"error":"invalid_client"}"#).is_rejected());
        assert!(!classify_token_failure(404, "").is_rejected());
        assert!(matches!(classify_token_failure(429, ""), AuthError::Server(m) if m.contains("Too many")));
        assert!(matches!(classify_token_failure(503, ""), AuthError::Server(m) if m.contains("503")));
        assert!(AuthError::Offline("tcp connect error".into()).user_message().contains("Cannot reach the server"));
    }

    #[test]
    fn an_unknown_workspace_is_named_but_not_a_refusal() {
        let error = workspace_unavailable("Tenant not found!");
        assert!(!error.is_rejected(), "a saved sign-in is kept and retried");
        assert_eq!(
            error.user_message(),
            "The server could not open this workspace (Tenant not found!). Check the workspace name."
        );
        // ABP HTML-encodes the reason.
        let encoded = workspace_unavailable("There is no tenant with the tenant id or name: Acme &amp; Co &#x27;EU&#39;");
        assert!(encoded.user_message().contains("name: Acme & Co 'EU')"), "{}", encoded.user_message());
        assert_eq!(workspace_unavailable("  ").user_message(), "The server could not open this workspace. Check the workspace name.");
        let long = workspace_unavailable(&"x".repeat(5_000)).user_message();
        assert!(long.chars().count() < MAX_REASON_CHARS + 80);
    }

    #[test]
    fn html_entities_are_decoded_and_anything_else_is_kept() {
        assert_eq!(decode_html_text("a &lt;b&gt; &quot;c&quot; &#65;&#x42;&#X43;"), "a <b> \"c\" ABC");
        assert_eq!(decode_html_text("R&D & co &bogus; &#xZZ; &; &#99999999;"), "R&D & co &bogus; &#xZZ; &; &#99999999;");
        assert_eq!(decode_html_text("trailing &amp"), "trailing &amp");
        assert_eq!(decode_html_text("plain"), "plain");
    }

    #[test]
    fn retry_after_takes_seconds_or_a_date_and_is_bounded() {
        let now = DateTime::parse_from_rfc3339("2026-10-05T10:00:00Z").unwrap().with_timezone(&Utc);
        assert_eq!(parse_retry_after("120", None, now), Some(Duration::from_secs(120)));
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
        assert_eq!(parse_retry_after("Mon, 05 Oct 2026 09:00:00 GMT", None, now), Some(Duration::ZERO));
        assert_eq!(parse_retry_after("99999999", None, now), Some(MAX_RETRY_AFTER));
        assert_eq!(parse_retry_after("soon", None, now), None);
    }

    #[test]
    fn the_server_clock_offset_comes_from_the_date_header() {
        let device = DateTime::parse_from_rfc3339("2026-10-05T10:00:00Z").unwrap().with_timezone(&Utc);
        assert_eq!(clock_offset_from_date("Mon, 05 Oct 2026 10:03:30 GMT", device), Some(210));
        assert_eq!(clock_offset_from_date("Mon, 05 Oct 2026 09:59:00 GMT", device), Some(-60));
        assert_eq!(clock_offset_from_date("yesterday", device), None);

        let clock = ServerClock::default();
        assert_eq!(clock.offset_secs(), None);
        assert_eq!(clock.skew_minutes(), None);
        clock.record(-60);
        assert_eq!(clock.clone().offset_secs(), Some(-60), "clones share the value");
        assert_eq!(clock.skew_minutes(), None, "within two minutes: no warning");
        clock.record(-7 * 60 - 20);
        assert_eq!(clock.skew_minutes(), Some(-7), "device clock 7 min ahead");
        assert_eq!(skew_warning_minutes(121), Some(2));
        assert_eq!(skew_warning_minutes(120), None);
    }

    // --- Against a local canned server ------------------------------------

    /// One raw HTTP/1.1 response.
    fn http(status: &str, headers: &[(&str, &str)], body: &str) -> String {
        let extra: String = headers.iter().map(|(k, v)| format!("{k}: {v}\r\n")).collect();
        format!(
            "HTTP/1.1 {status}\r\nContent-Length: {}\r\nContent-Type: application/json\r\nConnection: close\r\n{extra}\r\n{body}",
            body.len()
        )
    }

    fn token_ok() -> String {
        http("200 OK", &[], r#"{"access_token":"t0k3n","token_type":"Bearer","expires_in":3600}"#)
    }

    /// Read one request (headers and a Content-Length body) off the stream.
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
                        name.eq_ignore_ascii_case("content-length").then(|| value.trim().parse::<usize>().ok())?
                    })
                    .unwrap_or(0);
                if data.len() >= end + 4 + length {
                    break;
                }
            }
        }
        String::from_utf8_lossy(&data).into_owned()
    }

    /// Serve `responses` in order, one per connection, and pass each request
    /// on as text.
    fn canned_server(responses: Vec<String>) -> (String, std::sync::mpsc::Receiver<String>) {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            for response in responses {
                let Ok((mut stream, _)) = listener.accept() else { return };
                let _ = tx.send(read_request(&mut stream));
                let _ = stream.write_all(response.as_bytes());
                let _ = stream.flush();
            }
        });
        (url, rx)
    }

    fn client(url: &str) -> ApiClient {
        ApiClient::new(format!("{url}/"), "alice@acme.com".into(), "pw".into(), "acme".into())
    }

    fn sample_activity() -> Activity {
        let now = Utc::now();
        Activity::new("0b9a7c55-2f37-4f40-a7e5-5a8f0f6b2d11", now - chrono::Duration::minutes(10), now)
    }

    #[tokio::test]
    async fn a_401_upload_signs_in_again_and_resends_with_sent_at() {
        let (url, requests) = canned_server(vec![
            token_ok(),
            http("401 Unauthorized", &[], ""),
            token_ok(),
            http("200 OK", &[("Date", "Mon, 05 Oct 2026 10:00:00 GMT")], "{}"),
        ]);
        let mut api = client(&url);
        api.submit_activity(&sample_activity()).await.expect("uploaded on the second try");

        let requests: Vec<String> = requests.try_iter().collect();
        assert_eq!(requests.len(), 4);
        assert!(requests[0].starts_with("POST /connect/token?__tenant=acme "), "{}", requests[0]);
        assert!(requests[1].starts_with("POST /api/app/activity "), "no double slash: {}", requests[1]);
        assert!(requests[2].starts_with("POST /connect/token"));
        let upload = &requests[3];
        assert!(upload.contains("authorization: Bearer t0k3n") || upload.contains("Authorization: Bearer t0k3n"));
        let body = &upload[upload.find("\r\n\r\n").unwrap() + 4..];
        let json: serde_json::Value = serde_json::from_str(body).unwrap();
        let sent_at = json["sentAt"].as_str().expect("sentAt is stamped at send time");
        assert!(DateTime::parse_from_rfc3339(sent_at).is_ok());
        assert!(json.get("productivity").is_none(), "unmeasured: left out");
        // The Date header of the last answer set the clock offset.
        assert!(api.server_clock().offset_secs().is_some());
    }

    #[tokio::test]
    async fn an_unknown_workspace_at_sign_in_says_so() {
        let (url, requests) = canned_server(vec![http(
            "404 Not Found",
            &[("Abp-Tenant-Resolve-Error", "Tenant not found!")],
            "<html><body><h3>Tenant not found!</h3></body></html>",
        )]);
        match client(&url).login().await {
            Err(error @ AuthError::Server(_)) => {
                assert!(error.user_message().contains("Tenant not found!"), "{}", error.user_message())
            }
            other => panic!("expected a workspace error, got {other:?}"),
        }
        let request = requests.try_iter().next().unwrap().to_ascii_lowercase();
        assert!(request.contains("accept: application/json"), "{request}");
    }

    #[tokio::test]
    async fn a_refused_sign_in_on_any_call_means_sign_in_expired() {
        // The cached token expired server-side (401) and the saved password
        // no longer works.
        let (url, _requests) = canned_server(vec![
            token_ok(),
            http("401 Unauthorized", &[], ""),
            http("400 Bad Request", &[], r#"{"error":"invalid_grant","error_description":"Invalid username or password!"}"#),
        ]);
        let error = client(&url).my_projects().await.unwrap_err();
        assert!(error.is_auth_rejected(), "{error}");
        assert_eq!(error.status(), None);

        // Unreachable is not a refusal.
        let mut offline = ApiClient::new("http://127.0.0.1:9", "a@b.c".into(), "x".into(), String::new());
        assert!(!offline.my_projects().await.unwrap_err().is_auth_rejected());
    }

    #[tokio::test]
    async fn a_shared_server_clock_sees_every_response() {
        let (url, _requests) = canned_server(vec![
            http("200 OK", &[("Date", "Mon, 05 Oct 2026 10:00:00 GMT")], r#"{"access_token":"t","expires_in":3600}"#),
            http("200 OK", &[("Date", "Mon, 05 Oct 2026 10:00:00 GMT")], "[]"),
        ]);
        // What the window reads, handed to a client created later (a new
        // sign-in): it is updated by that client's answers.
        let window_clock = ServerClock::default();
        let mut api = client(&url).with_server_clock(window_clock.clone());
        assert_eq!(window_clock.offset_secs(), None);
        assert!(api.my_projects().await.unwrap().is_empty());
        let fixed = DateTime::parse_from_rfc3339("2026-10-05T10:00:00Z").unwrap().with_timezone(&Utc);
        let expected = (fixed - Utc::now()).num_seconds();
        let seen = window_clock.offset_secs().expect("recorded from the Date header");
        assert!((seen - expected).abs() <= 5, "{seen} vs {expected}");
        assert_eq!(api.server_clock().offset_secs(), Some(seen), "one shared value");
    }

    #[tokio::test]
    async fn refused_credentials_during_an_upload_mean_sign_in_expired() {
        let (url, _requests) = canned_server(vec![http(
            "400 Bad Request",
            &[],
            r#"{"error":"invalid_grant","error_description":"Invalid username or password!"}"#,
        )]);
        match client(&url).submit_activity(&sample_activity()).await {
            Err(SubmitError::AuthRejected(reason)) => assert_eq!(reason, "Invalid username or password!"),
            other => panic!("expected sign-in expired, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn an_oversized_upload_asks_for_its_captures_to_be_stripped() {
        let (url, _requests) = canned_server(vec![token_ok(), http("413 Payload Too Large", &[], "")]);
        assert!(matches!(
            client(&url).submit_activity(&sample_activity()).await,
            Err(SubmitError::StripCaptures(_))
        ));
    }

    #[tokio::test]
    async fn throttled_uploads_carry_the_retry_after() {
        let (url, _requests) = canned_server(vec![token_ok(), http("429 Too Many Requests", &[("Retry-After", "90")], "")]);
        match client(&url).submit_activity(&sample_activity()).await {
            Err(SubmitError::Transient { retry_after, .. }) => assert_eq!(retry_after, Some(Duration::from_secs(90))),
            other => panic!("expected transient, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn an_unreachable_server_is_transient_and_offline() {
        // Nothing listens on port 9 (discard) locally: a real connect error.
        let mut api = ApiClient::new("http://127.0.0.1:9", "a@b.c".into(), "x".into(), String::new());
        assert!(matches!(api.login().await, Err(AuthError::Offline(_))));
        assert!(matches!(api.submit_activity(&sample_activity()).await, Err(SubmitError::Transient { .. })));
        assert!(matches!(api.my_projects().await, Err(ApiError::Offline(_))));
    }

    #[tokio::test]
    async fn projects_parse_with_the_new_policy_fields_and_errors_keep_the_abp_body() {
        let projects = r#"[{"id":"p1","title":"Website","intervalMinutes":10,"allowScreenshot":true,
            "allowWebcam":false,"allowKeyboard":true,"allowMouse":true,"allowActiveWindow":true,
            "allowRunningPrograms":true,"idleThresholdMinutes":15,"blurScreenshots":true}]"#;
        let (url, requests) = canned_server(vec![
            token_ok(),
            http("200 OK", &[], projects),
            http("403 Forbidden", &[], &abp(Some("Volo.Authorization:010001"), "Authorization failed!")),
        ]);
        let mut api = client(&url);
        let list = api.my_projects().await.unwrap();
        assert_eq!(list[0].idle_threshold_minutes, 15);
        assert!(list[0].blur_screenshots);

        let error = api
            .summary(Utc::now() - chrono::Duration::hours(1), Utc::now(), Some("Asia/Dhaka"))
            .await
            .unwrap_err();
        assert_eq!(error.status(), Some(403));
        assert!(error.to_string().contains("Authorization failed!"));
        let summary_request = requests.try_iter().nth(2).unwrap();
        assert!(summary_request.contains("TimeZone=Asia%2FDhaka"), "{summary_request}");
    }

    #[tokio::test]
    async fn heartbeats_are_bounded_by_their_timeout() {
        // Accepts connections but never answers.
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        std::thread::spawn(move || {
            let mut held = Vec::new();
            for stream in listener.incoming() {
                held.push(stream);
            }
        });
        let heartbeat = Heartbeat {
            device_id: uuid::Uuid::new_v4().to_string(),
            device_name: None,
            platform: Some("windows".into()),
            os_version: None,
            agent_version: None,
            status: crate::model::PresenceStatus::Stopped,
            project_id: None,
            status_since: None,
            current_app: None,
            time_zone: None,
            utc_offset_minutes: None,
            sent_at: None,
        };
        let started = Instant::now();
        let result = client(&url).heartbeat(&heartbeat, Duration::from_millis(800)).await;
        assert!(matches!(result, Err(ApiError::Offline(_))), "{result:?}");
        assert!(started.elapsed() < Duration::from_secs(3));
    }

    #[tokio::test]
    async fn heartbeats_post_the_contract_body_and_read_the_server_time() {
        let (url, requests) = canned_server(vec![
            token_ok(),
            http("200 OK", &[], r#"{"serverTime":"2026-10-05T10:00:00Z"}"#),
        ]);
        let heartbeat = Heartbeat {
            device_id: "5b0c1b9e-8f2c-4a43-9a51-0c7f6f2d9b11".into(),
            device_name: Some("DESKTOP-1".into()),
            platform: Some("windows".into()),
            os_version: None,
            agent_version: Some("0.2.0".into()),
            status: crate::model::PresenceStatus::Tracking,
            project_id: Some("p1".into()),
            status_since: None,
            current_app: Some("code.exe".into()),
            time_zone: Some("Asia/Dhaka".into()),
            utc_offset_minutes: Some(360),
            sent_at: None,
        };
        let result = client(&url).heartbeat(&heartbeat, Duration::from_secs(5)).await.unwrap();
        assert_eq!(result.server_time, crate::model::parse_server_time("2026-10-05T10:00:00Z"));
        let request = requests.try_iter().nth(1).unwrap();
        assert!(request.starts_with("POST /api/app/presence/heartbeat "), "{request}");
        let body: serde_json::Value = serde_json::from_str(&request[request.find("\r\n\r\n").unwrap() + 4..]).unwrap();
        assert_eq!(body["status"], "tracking");
        assert_eq!(body["currentApp"], "code.exe");
        assert!(body["sentAt"].is_string());
    }
}
