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

use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};
use reqwest::header::{HeaderMap, ACCEPT, DATE, RETRY_AFTER};
use reqwest::StatusCode;
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex as TokioMutex;

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

#[derive(Clone)]
pub(crate) struct GrantedToken {
    access_token: String,
    refresh_token: Option<String>,
    expires_at: Instant,
}

impl GrantedToken {
    fn from_response(response: TokenResponse, previous_refresh: Option<String>) -> Self {
        // Pre-expiry renewal: 60s early buffer
        let ttl = response.expires_in.saturating_sub(60).max(60);
        Self {
            access_token: response.access_token,
            refresh_token: response
                .refresh_token
                .filter(|t| !t.is_empty())
                .or(previous_refresh),
            expires_at: Instant::now() + Duration::from_secs(ttl),
        }
    }

    fn is_valid(&self) -> bool {
        Instant::now() < self.expires_at
    }
}

#[derive(Deserialize)]
struct TokenResponse {
    access_token: String,
    #[serde(default)]
    refresh_token: Option<String>,
    #[serde(default = "default_expires_in")]
    expires_in: u64,
}

fn default_expires_in() -> u64 {
    3600
}

struct TokenCellInner {
    token: std::sync::Mutex<Option<GrantedToken>>,
    renewing: TokioMutex<()>,
}

#[derive(Clone)]
pub struct TokenCell(Arc<TokenCellInner>);

impl Default for TokenCell {
    fn default() -> Self {
        Self(Arc::new(TokenCellInner {
            token: std::sync::Mutex::new(None),
            renewing: TokioMutex::new(()),
        }))
    }
}

impl TokenCell {
    pub fn with_refresh_token(refresh_token: Option<String>) -> Self {
        let cell = Self::default();
        if let Some(refresh_token) = refresh_token.filter(|t| !t.is_empty()) {
            cell.set(GrantedToken {
                access_token: String::new(),
                refresh_token: Some(refresh_token),
                expires_at: Instant::now(),
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

    pub fn refresh_token(&self) -> Option<String> {
        self.lock().as_ref().and_then(|t| t.refresh_token.clone())
    }

    pub fn valid_access_token(&self) -> Option<String> {
        self.lock()
            .as_ref()
            .filter(|t| !t.access_token.is_empty() && t.is_valid())
            .map(|t| t.access_token.clone())
    }

    pub fn invalidate(&self, current: &str) {
        if let Some(token) = self.lock().as_mut().filter(|t| t.access_token == current) {
            token.access_token.clear();
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

        self.tokens
            .set(GrantedToken::from_response(token_resp, previous_refresh));
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
        if let Some(token) = self.tokens.valid_access_token() {
            return Ok(token);
        }

        let slot = self.tokens.0.clone();
        let _guard = slot.renewing.lock().await;

        if let Some(token) = self.tokens.valid_access_token() {
            return Ok(token);
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
                Err(e) => return Err(e),
            }
        } else if self.password.is_empty() {
            return Err(AuthError::Rejected(
                "Credentials required. Please sign in.".into(),
            ));
        } else {
            self.login().await?;
        }

        self.tokens.valid_access_token().ok_or_else(|| {
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
}
