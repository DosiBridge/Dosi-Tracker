//! Async HTTP API client, token manager, and transport coordinator for Dosi-Tracker Linux.
//!
//! # Architecture & Security Invariants
//! - Async reqwest HTTP client built with rustls and system/bundled TLS roots.
//! - Strict TLS verification: certificate verification is NEVER disabled.
//! - Finite timeouts: connect timeout (10s), request timeout (30s), upload timeout (120s), heartbeat timeout (10s).
//! - Authentication follows OpenIddict password grant (`POST /connect/token`) with `client_id = Tracker_App`.
//! - Pre-expiry token renewal: tokens are refreshed 60 seconds before expiration.
//! - Bounded 401 retry: upon receiving HTTP 401, attempts token refresh once and retries once.
//! - Activity delivery follows `POST /api/app/activity` with exact `client_activity_id` idempotency.
//! - Presence heartbeat follows `POST /api/app/presence/heartbeat` with 60s cadence primitive.
//! - Zero secrets or tokens in logs, Display, or Debug representations.

use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};
use reqwest::header::{HeaderMap, ACCEPT, DATE, RETRY_AFTER};
use reqwest::StatusCode;
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex as TokioMutex;

use crate::credentials::{CredentialStore, Credentials};
use crate::outbox::{ActivityEnvelope, OutboxEntry};
use crate::signing::sign_activity_envelope;
use crate::sync::{DeliveryOutcome, DeliveryTransport};

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

/// Longest Retry-After header duration honored (6 hours).
pub const MAX_RETRY_AFTER: Duration = Duration::from_secs(6 * 60 * 60);

/// Skew warning threshold in seconds (2 minutes).
pub const CLOCK_SKEW_WARNING_SECS: i64 = 120;

/// Standard ABP business error codes returned by the backend.
pub mod codes {
    pub const NOT_WORKSPACE_MEMBER: &str = "Tracker:NotWorkspaceMember";
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

/// Builds the shared reqwest HTTP client with rustls-tls and strict verification.
pub fn build_http_client() -> reqwest::Client {
    reqwest::Client::builder()
        .user_agent(concat!("dosi-tracker-linux/", env!("CARGO_PKG_VERSION")))
        .connect_timeout(CONNECT_TIMEOUT)
        .timeout(REQUEST_TIMEOUT)
        .build()
        .expect("Failed to initialize secure HTTP client")
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
    /// Server error (5xx, unexpected response structure).
    Server(String),
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
    Auth(AuthError),
    Offline(String),
    Http { status: u16, message: String },
    NotWorkspaceMember(String),
    Invalid(String),
}

impl std::fmt::Display for ApiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Auth(err) => write!(f, "{err}"),
            Self::Offline(msg) => write!(f, "Network error: {msg}"),
            Self::Http { status, message } => write!(f, "HTTP {status}: {message}"),
            Self::NotWorkspaceMember(msg) => write!(f, "Not a workspace member: {msg}"),
            Self::Invalid(msg) => write!(f, "Invalid response: {msg}"),
        }
    }
}

impl std::error::Error for ApiError {}

impl From<AuthError> for ApiError {
    fn from(err: AuthError) -> Self {
        Self::Auth(err)
    }
}

/// Result of an activity submission attempt.
#[derive(Debug)]
pub enum SubmitError {
    /// Transient failure: retain entry and retry with backoff.
    Transient {
        reason: String,
        retry_after: Option<Duration>,
    },
    /// Authentication rejected: credentials invalid or revoked.
    AuthRejected(String),
    /// Definitive rejection: invalid block, overlap, validation failure.
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
            Self::Rejected(reason) => write!(f, "Activity rejected: {reason}"),
        }
    }
}

impl std::error::Error for SubmitError {}

/// ABP Framework Error Response.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct AbpErrorResponse {
    #[serde(default)]
    pub error: Option<AbpErrorDetail>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct AbpErrorDetail {
    #[serde(default)]
    pub code: Option<String>,
    #[serde(default)]
    pub message: Option<String>,
    #[serde(default)]
    pub details: Option<String>,
    #[serde(default, rename = "validationErrors")]
    pub validation_errors: Option<Vec<AbpValidationError>>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct AbpValidationError {
    #[serde(default)]
    pub message: Option<String>,
    #[serde(default)]
    pub members: Option<Vec<String>>,
}

impl AbpErrorResponse {
    pub fn summary(&self) -> Option<String> {
        let err = self.error.as_ref()?;
        let mut msg = err.message.clone().unwrap_or_default();
        if let Some(ref val_errors) = err.validation_errors {
            for v in val_errors {
                if let Some(ref vm) = v.message {
                    if !msg.is_empty() {
                        msg.push(' ');
                    }
                    msg.push_str(vm);
                }
            }
        }
        if msg.trim().is_empty() {
            None
        } else {
            Some(msg.trim().to_string())
        }
    }
}

pub fn classify_upload_failure(
    status: u16,
    body: &str,
    retry_after: Option<Duration>,
) -> SubmitError {
    let abp: AbpErrorResponse = serde_json::from_str(body).unwrap_or_default();
    let code = abp.error.as_ref().and_then(|e| e.code.as_deref());
    let summary = abp.summary().unwrap_or_else(|| format!("HTTP {status}"));

    // Check specific business error codes
    if let Some(code) = code {
        match code {
            codes::ACTIVITY_OVERLAPS
            | codes::ACTIVITY_CLIENT_ID_CONFLICT
            | codes::ACTIVITY_TOO_LONG
            | codes::ACTIVITY_TOO_OLD
            | codes::ACTIVITY_TIME_RANGE_INVALID => {
                return SubmitError::Rejected(summary);
            }
            codes::NOT_WORKSPACE_MEMBER => {
                return SubmitError::Rejected(summary);
            }
            _ => {}
        }
    }

    // Check HTTP status classifications
    match status {
        400 => {
            // Validation errors or invalid format
            SubmitError::Rejected(summary)
        }
        409 | 422 => SubmitError::Rejected(summary),
        429 | 500.. => SubmitError::Transient {
            reason: summary,
            retry_after,
        },
        _ => SubmitError::Transient {
            reason: summary,
            retry_after,
        },
    }
}

/// DTO for activity upload matching backend `CreateActivityDto`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateActivityDto {
    pub client_activity_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project_id: Option<String>,
    pub started_at: DateTime<Utc>,
    pub ended_at: DateTime<Utc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub productivity: Option<u32>,
    pub mouse_clicks: u32,
    pub keyboard_hits: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub device_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub platform: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sent_at: Option<DateTime<Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub payload_signature: Option<String>,
}

impl From<&ActivityEnvelope> for CreateActivityDto {
    fn from(env: &ActivityEnvelope) -> Self {
        Self {
            client_activity_id: env.client_activity_id.clone(),
            project_id: env.project_id.clone(),
            started_at: env.started_at,
            ended_at: env.ended_at,
            productivity: env.productivity,
            mouse_clicks: env.mouse_clicks.unwrap_or(0),
            keyboard_hits: env.keyboard_hits.unwrap_or(0),
            description: env.description.clone(),
            device_id: env.device_id.clone(),
            platform: Some(env.platform.clone()),
            agent_version: Some(env.agent_version.clone()),
            sent_at: Some(Utc::now()),
            payload_signature: env.payload_signature.clone(),
        }
    }
}

/// DTO for presence heartbeat matching backend `HeartbeatDto`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HeartbeatDto {
    pub device_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub device_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub platform: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub os_version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_version: Option<String>,
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status_since: Option<DateTime<Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sent_at: Option<DateTime<Utc>>,
}

/// DTO response from backend for presence heartbeat.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HeartbeatResultDto {
    pub server_time: Option<DateTime<Utc>>,
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

struct TokenCellInner {
    token: std::sync::Mutex<Option<GrantedToken>>,
    renewing: TokioMutex<()>,
    time_offset_ms: AtomicU64,
    clock_override: std::sync::Mutex<Option<TokenClockFn>>,
}

#[derive(Clone)]
pub struct TokenCell(Arc<TokenCellInner>);

impl Default for TokenCell {
    fn default() -> Self {
        Self(Arc::new(TokenCellInner {
            token: std::sync::Mutex::new(None),
            renewing: TokioMutex::new(()),
            time_offset_ms: AtomicU64::new(0),
            clock_override: std::sync::Mutex::new(None),
        }))
    }
}

impl TokenCell {
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
pub struct ApiClient {
    http: reqwest::Client,
    base_url: String,
    username: String,
    password: String,
    workspace_id: Option<String>,
    signing_key: Option<[u8; 32]>,
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
            password,
            workspace_id: None,
            signing_key: None,
            tokens: TokenCell::default(),
            clock: ServerClock::default(),
        }
    }

    pub fn with_workspace(mut self, workspace_id: Option<String>) -> Self {
        self.workspace_id = workspace_id;
        self
    }

    pub fn with_signing_key(mut self, key: Option<[u8; 32]>) -> Self {
        self.signing_key = key;
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
    /// Reuses persisted tokens, workspace scope, device signing key, and server URL
    /// without requiring re-prompting for raw passwords.
    pub fn from_credentials(creds: &Credentials) -> Result<Self, String> {
        let base_url = creds
            .server_url
            .as_deref()
            .unwrap_or("https://api.dosi-tracker.com");
        let mut client = Self::new(
            base_url,
            creds.account_id.clone(),
            creds.password.clone().unwrap_or_default(),
        );
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

    pub fn server_clock(&self) -> ServerClock {
        self.clock.clone()
    }

    pub fn tokens(&self) -> TokenCell {
        self.tokens.clone()
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

    async fn token_request(
        &mut self,
        params: &[(&str, &str)],
        previous_refresh: Option<String>,
    ) -> Result<(), AuthError> {
        let url = if let Some(ref ws) = self.workspace_id {
            format!("{}/connect/token?__tenant={}", self.base_url, ws)
        } else {
            format!("{}/connect/token", self.base_url)
        };

        let response = self
            .execute(
                self.http
                    .post(&url)
                    .header(ACCEPT, "application/json")
                    .form(params),
            )
            .await
            .map_err(|e| AuthError::Offline(describe_error(&e)))?;

        let status = response.status();
        let body = response
            .text()
            .await
            .map_err(|e| AuthError::Offline(describe_error(&e)))?;

        if !status.is_success() {
            return Err(classify_token_failure(status.as_u16(), &body));
        }

        let token_resp: TokenResponse = serde_json::from_str(&body)
            .map_err(|e| AuthError::Server(format!("Failed to parse token response: {e}")))?;

        let (now_instant, now_utc) = self.tokens.current_time();
        self.tokens.set(GrantedToken::from_response(
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
                Err(AuthError::Rejected(_)) if self.password.is_empty() => {
                    return Err(AuthError::Rejected(
                        "Session expired. Please sign in again.".into(),
                    ));
                }
                Err(AuthError::Rejected(_)) => {
                    self.login().await?;
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

    /// Sends an authorized request with single-retry on HTTP 401.
    pub async fn send_authorized<F>(&mut self, build: F) -> Result<reqwest::Response, ApiError>
    where
        F: Fn(&reqwest::Client, &str) -> reqwest::RequestBuilder,
    {
        let token = self.ensure_token().await?;
        let mut req = build(&self.http, &token);
        if let Some(ref ws) = self.workspace_id {
            req = req.header(WORKSPACE_HEADER, ws);
        }

        let response = self
            .execute(req)
            .await
            .map_err(|e| ApiError::Offline(describe_error(&e)))?;

        if response.status() != StatusCode::UNAUTHORIZED {
            return Ok(response);
        }

        // Bounded retry: invalidate current token, obtain fresh token, retry once
        self.tokens.invalidate(&token);
        let fresh_token = self.ensure_token().await?;
        let mut retry_req = build(&self.http, &fresh_token);
        if let Some(ref ws) = self.workspace_id {
            retry_req = retry_req.header(WORKSPACE_HEADER, ws);
        }

        let retry_response = self
            .execute(retry_req)
            .await
            .map_err(|e| ApiError::Offline(describe_error(&e)))?;

        if retry_response.status() == StatusCode::UNAUTHORIZED {
            self.tokens.invalidate(&fresh_token);
            return Err(ApiError::Auth(AuthError::Rejected(
                "Unauthorized (HTTP 401) after re-authentication".to_string(),
            )));
        }

        Ok(retry_response)
    }

    /// Submits a queued activity envelope to `/api/app/activity`.
    pub async fn submit_activity(
        &mut self,
        envelope: &ActivityEnvelope,
    ) -> Result<String, SubmitError> {
        let url = format!("{}/api/app/activity", self.base_url);

        // Convert envelope to CreateActivityDto
        let mut dto = CreateActivityDto::from(envelope);

        // Sign payload if signing key is configured and signature is absent
        if let Some(ref key) = self.signing_key {
            if dto.payload_signature.is_none() {
                let mut env = envelope.clone();
                let sig = sign_activity_envelope(key, &mut env)
                    .map_err(|e| SubmitError::Rejected(format!("Failed to sign envelope: {e}")))?;
                dto.payload_signature = Some(sig);
            }
        }

        let body_json =
            serde_json::to_string(&dto).map_err(|e| SubmitError::Rejected(e.to_string()))?;

        let response = self
            .send_authorized(|http, token| {
                http.post(&url)
                    .bearer_auth(token)
                    .header("Content-Type", "application/json")
                    .timeout(UPLOAD_TIMEOUT)
                    .body(body_json.clone())
            })
            .await;

        let response = match response {
            Ok(r) => r,
            Err(ApiError::Auth(AuthError::Rejected(reason))) => {
                return Err(SubmitError::AuthRejected(reason))
            }
            Err(ApiError::Auth(e)) => {
                return Err(SubmitError::Transient {
                    reason: e.to_string(),
                    retry_after: None,
                })
            }
            Err(ApiError::Offline(msg)) => {
                return Err(SubmitError::Transient {
                    reason: msg,
                    retry_after: None,
                })
            }
            Err(ApiError::Http { status, message }) => {
                return Err(classify_upload_failure(status, &message, None));
            }
            Err(ApiError::NotWorkspaceMember(msg)) => return Err(SubmitError::Rejected(msg)),
            Err(ApiError::Invalid(msg)) => {
                return Err(SubmitError::Transient {
                    reason: msg,
                    retry_after: None,
                })
            }
        };

        let status = response.status();
        if status.is_success() {
            // Returns exact client_activity_id on success
            return Ok(envelope.client_activity_id.clone());
        }

        if status == StatusCode::UNAUTHORIZED {
            return Err(SubmitError::AuthRejected(
                "HTTP 401 after re-authentication".to_string(),
            ));
        }

        let retry_after = retry_after_of(response.headers());
        let body = response.text().await.unwrap_or_default();
        Err(classify_upload_failure(status.as_u16(), &body, retry_after))
    }

    /// Presence heartbeat reporting to `/api/app/presence/heartbeat`.
    pub async fn heartbeat(
        &mut self,
        heartbeat: &HeartbeatDto,
    ) -> Result<HeartbeatResultDto, ApiError> {
        let url = format!("{}/api/app/presence/heartbeat", self.base_url);
        let mut body = heartbeat.clone();
        body.sent_at = Some(Utc::now());

        let response = self
            .send_authorized(|http, token| {
                http.post(&url)
                    .bearer_auth(token)
                    .timeout(HEARTBEAT_TIMEOUT)
                    .json(&body)
            })
            .await?;

        let status = response.status();
        if !status.is_success() {
            let text = response.text().await.unwrap_or_default();
            return Err(ApiError::Http {
                status: status.as_u16(),
                message: text,
            });
        }

        let result = response
            .json::<HeartbeatResultDto>()
            .await
            .unwrap_or_default();
        Ok(result)
    }
}

enum WorkerMsg {
    Deliver {
        entry: Box<OutboxEntry>,
        reply: std::sync::mpsc::SyncSender<DeliveryOutcome>,
    },
    Shutdown,
}

/// Concrete HTTP Delivery Transport implementing [`DeliveryTransport`].
///
/// Runs delivery requests across a dedicated worker thread with an isolated
/// single-thread Tokio runtime, preventing runtime-nesting panics and guaranteeing
/// zero SQLite lock retention during HTTP network operations.
pub struct HttpDeliveryTransport {
    sender: std::sync::mpsc::SyncSender<WorkerMsg>,
    worker: Option<std::thread::JoinHandle<()>>,
}

impl HttpDeliveryTransport {
    pub fn new(mut client: ApiClient) -> Self {
        let (tx, rx) = std::sync::mpsc::sync_channel::<WorkerMsg>(32);

        let handle = std::thread::spawn(move || {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("Failed to initialize transport worker runtime");

            while let Ok(msg) = rx.recv() {
                match msg {
                    WorkerMsg::Deliver { entry, reply } => {
                        let outcome = rt.block_on(async {
                            match client.submit_activity(&entry.envelope).await {
                                Ok(acked_id) => DeliveryOutcome::Accepted {
                                    client_activity_id: acked_id,
                                },
                                Err(SubmitError::AuthRejected(reason)) => {
                                    DeliveryOutcome::AuthRequired { reason }
                                }
                                Err(SubmitError::Rejected(reason)) => {
                                    DeliveryOutcome::Rejected { reason }
                                }
                                Err(SubmitError::Transient { reason, .. }) => {
                                    DeliveryOutcome::TransientFailure { reason }
                                }
                            }
                        });
                        let _ = reply.send(outcome);
                    }
                    WorkerMsg::Shutdown => break,
                }
            }
        });

        Self {
            sender: tx,
            worker: Some(handle),
        }
    }

    /// Constructs an `HttpDeliveryTransport` from persisted credentials.
    pub fn from_credentials(creds: &Credentials) -> Result<Self, String> {
        let client = ApiClient::from_credentials(creds)?;
        Ok(Self::new(client))
    }

    /// Loads credentials from the specified store and constructs an `HttpDeliveryTransport`.
    pub fn from_store(
        store: &dyn CredentialStore,
        account_id: &str,
        workspace_id: Option<&str>,
    ) -> Result<Self, String> {
        let client = ApiClient::from_store(store, account_id, workspace_id)?;
        Ok(Self::new(client))
    }
}

impl DeliveryTransport for HttpDeliveryTransport {
    fn deliver(&mut self, entry: &OutboxEntry) -> DeliveryOutcome {
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

        reply_rx
            .recv()
            .unwrap_or_else(|_| DeliveryOutcome::TransientFailure {
                reason: "Transport worker failed to return outcome".to_string(),
            })
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

#[cfg(test)]
mod tests {
    use super::*;
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
        assert!(req.contains("POST /connect/token?__tenant=ws-1"));
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
        let result = client.submit_activity(&env).await;
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
        let result = client.submit_activity(&env).await;
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
        let ack_id = client.submit_activity(&env).await.unwrap();
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

        let hb = HeartbeatDto {
            device_id: "dev-uuid-1".to_string(),
            device_name: Some("linux-node-1".to_string()),
            platform: Some("linux".to_string()),
            os_version: Some("Ubuntu 24.04".to_string()),
            agent_version: Some("0.1.0".to_string()),
            status: "tracking".to_string(),
            project_id: Some("proj-uuid-1".to_string()),
            status_since: Some(Utc::now()),
            sent_at: None,
        };

        let result = client.heartbeat(&hb).await.unwrap();
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

        let mut transport = HttpDeliveryTransport::new(client);
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

        let mut transport = HttpDeliveryTransport::new(client);
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
        creds.server_url = Some("http://api.stored.local".into());
        creds.device_signing_key = Some(vec![99u8; 32]);
        creds.access_token = Some("valid-tok-123".into());
        creds.access_token_expires_at = Some(Utc::now() + chrono::Duration::seconds(1800));
        creds.refresh_token = Some("ref-tok-456".into());
        store.save(&creds).unwrap();

        let client =
            ApiClient::from_store(&store, "store-user@dosi.test", Some("ws-from-store")).unwrap();
        assert_eq!(client.base_url(), "http://api.stored.local");
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
}
