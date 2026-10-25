//! The Dosi-Tracker backend's REST API, as the agent uses it.
//!
//! This module holds the client and its calls; signing in and the shared
//! token are in [`tokens`], the classification of failed answers in
//! [`errors`], and the server's clock in [`clock`].

mod clock;
mod errors;
mod tokens;

use std::time::Duration;

use chrono::{DateTime, Utc};
use reqwest::header::DATE;
use reqwest::StatusCode;
use serde::de::DeserializeOwned;
use serde::Deserialize;

use crate::model::{Activity, Heartbeat, HeartbeatResult, Project};

use clock::{clock_offset_from_date, retry_after_of};
pub use clock::ServerClock;
use errors::{describe, http_error, RequestError};
pub use errors::{classify_upload_failure, ApiError, AuthError, SubmitError, WorkspaceRefusal};
pub use tokens::{classify_token_failure, Revocation, TokenCell};

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

/// Names the workspace a workspace-scoped request acts in.
pub const WORKSPACE_HEADER: &str = "X-Workspace-Id";

/// Give up on a TCP/TLS connect that has not completed by then, so a dead or
/// black-holed server can never hang the worker (and with it the UI's
/// pause/sync commands) indefinitely.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
/// Whole-request budget for sign-in, projects and totals.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
/// Whole-request budget for an upload, which carries a screenshot (and maybe
/// a webcam frame) over what may be a slow uplink.
const UPLOAD_TIMEOUT: Duration = Duration::from_secs(120);

/// The server's business error codes the agent acts on (CONTRACT §1).
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
}

/// Build the shared HTTP client.
///
/// TLS trusts the Windows certificate store (so corporate CAs and the ASP.NET
/// dev certificate installed by `dotnet dev-certs https --trust` work) *in
/// addition to* the bundled Mozilla roots (so public CAs Windows has not
/// fetched yet still work). Certificate verification is always on.
///
/// Redirects are never followed: no API call needs one, and following a 307
/// or 308 would resend the password (or a screenshot) to wherever it points,
/// downgraded to `http://` or on another host. A 3xx is an error answer.
fn build_http_client() -> reqwest::Client {
    let builder = || {
        reqwest::Client::builder()
            .user_agent(concat!("dosi-tracker/", env!("CARGO_PKG_VERSION")))
            .connect_timeout(CONNECT_TIMEOUT)
            .timeout(REQUEST_TIMEOUT)
            .redirect(reqwest::redirect::Policy::none())
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

/// A workspace the signed-in user belongs to (`GET /api/app/workspaces/mine`).
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Workspace {
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub role: Option<String>,
    #[serde(default)]
    pub plan_name: Option<String>,
    #[serde(default)]
    pub is_owner: bool,
    /// Archived workspaces are listed too, but refuse every tracker call.
    #[serde(default)]
    pub is_archived: bool,
}

/// Thin HTTP client wrapping the Dosi-Tracker backend REST API.
///
/// Authentication uses the backend's OpenIddict token endpoint
/// (`POST /connect/token`), naming this install's `device_id` on every grant.
/// The token is renewed with the refresh token shortly before it expires and
/// once when a request answers 401. A refused refresh token means "sign-in
/// expired": the password is never sent again behind the user's back. The
/// password is sent once, for the sign-in itself, and forgotten as soon as
/// the server granted a token ([`TokenCell`] remembers that for every client
/// of the sign-in). Signing out revokes the refresh token
/// ([`ApiClient::revoke_refresh_token`]). Workspace-scoped requests carry
/// [`WORKSPACE_HEADER`].
///
/// Nothing is sent to a plain `http://` server other than this computer
/// unless the admin opted in ([`ApiClient::with_insecure_http`]); see
/// [`crate::config::check_transport`].
pub struct ApiClient {
    http: reqwest::Client,
    base_url: String,
    username: String,
    password: String,
    tokens: TokenCell,
    workspace_id: Option<String>,
    clock: ServerClock,
    /// This install's id, sent with the password and refresh grants.
    device_id: Option<String>,
    /// `config.toml`'s `allow_insecure_http`.
    insecure_http_allowed: bool,
}

fn scoped(request: reqwest::RequestBuilder, workspace_id: Option<&str>) -> reqwest::RequestBuilder {
    match workspace_id {
        Some(id) => request.header(WORKSPACE_HEADER, id),
        None => request,
    }
}

impl ApiClient {
    pub fn new(base_url: impl Into<String>, username: String, password: String) -> Self {
        // Endpoints are appended as "/path": a trailing slash would make
        // every one of them "//path".
        let base_url = base_url.into().trim().trim_end_matches('/').to_string();
        Self {
            http: build_http_client(),
            base_url,
            username,
            password,
            tokens: TokenCell::default(),
            workspace_id: None,
            clock: ServerClock::default(),
            device_id: None,
            insecure_http_allowed: false,
        }
    }

    /// Name this install (a UUID kept per install) on the token grants.
    pub fn with_device_id(mut self, device_id: Option<String>) -> Self {
        self.device_id = device_id.map(|id| id.trim().to_owned()).filter(|id| !id.is_empty());
        self
    }

    /// Allow a plain `http://` server that is not this computer (the admin's
    /// explicit `allow_insecure_http = true`).
    pub fn with_insecure_http(mut self, allowed: bool) -> Self {
        self.insecure_http_allowed = allowed;
        self
    }

    /// Refuse to talk to the server at all over a transport it may not use.
    fn check_transport(&self) -> Result<(), AuthError> {
        crate::config::check_transport(&self.base_url, self.insecure_http_allowed)
            .map_err(|e| AuthError::Server(e.to_string()))
    }

    /// Share `clock` with this client (e.g. one the window already reads), so
    /// it survives signing in again.
    #[cfg_attr(not(test), expect(dead_code, reason = "read by the window's clock warning"))]
    pub fn with_server_clock(mut self, clock: ServerClock) -> Self {
        self.clock = clock;
        self
    }

    /// Use (and share) `tokens` instead of signing in first.
    pub fn with_tokens(mut self, tokens: TokenCell) -> Self {
        self.tokens = tokens;
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

    pub fn workspace_id(&self) -> Option<&str> {
        self.workspace_id.as_deref()
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

    /// The user (on this server) this client acts as, in the form the offline
    /// queue tags rows with — so rows are only ever uploaded under their
    /// owner's token, to the server they were recorded for.
    pub fn owner_key(&self) -> String {
        crate::storage::owner_key(&self.base_url, &self.username)
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

    /// Send an authorized request built by `build` (given the HTTP client and
    /// a bearer token), in `workspace_id` when given. On a 401 the token was
    /// revoked or expired server-side: renew it once and resend.
    async fn send_authorized<F>(&mut self, workspace_id: Option<&str>, build: F) -> Result<reqwest::Response, RequestError>
    where
        F: Fn(&reqwest::Client, &str) -> reqwest::RequestBuilder,
    {
        self.check_transport().map_err(RequestError::Auth)?;
        let token = self.ensure_token().await.map_err(RequestError::Auth)?;
        let response = self
            .execute(scoped(build(&self.http, &token), workspace_id))
            .await
            .map_err(RequestError::Transport)?;
        if response.status() != StatusCode::UNAUTHORIZED {
            return Ok(response);
        }
        self.tokens.invalidate(&token);
        let token = self.ensure_token().await.map_err(RequestError::Auth)?;
        let response = self
            .execute(scoped(build(&self.http, &token), workspace_id))
            .await
            .map_err(RequestError::Transport)?;
        if response.status() == StatusCode::UNAUTHORIZED {
            self.tokens.invalidate(&token);
        }
        Ok(response)
    }

    /// The workspaces this user belongs to (`GET /api/app/workspaces/mine`).
    pub async fn workspaces_mine(&mut self) -> Result<Vec<Workspace>, ApiError> {
        let url = format!("{}/api/app/workspaces/mine", self.base_url);
        let response = self.send_authorized(None, |http, token| http.get(&url).bearer_auth(token)).await?;
        read_json(response).await
    }

    /// The active (non-archived) projects the user may track in the current
    /// workspace, with the per-project capture permissions and policy the
    /// backend pushes down (`GET /api/app/project/my-projects`).
    pub async fn my_projects(&mut self) -> Result<Vec<Project>, ApiError> {
        let url = format!("{}/api/app/project/my-projects", self.base_url);
        let workspace = self.workspace_id.clone();
        let response = self.send_authorized(workspace.as_deref(), |http, token| http.get(&url).bearer_auth(token)).await?;
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
        let workspace = self.workspace_id.clone();
        let response = self
            .send_authorized(workspace.as_deref(), |http, token| http.get(&url).query(&query).bearer_auth(token))
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
        let workspace = self.workspace_id.clone();
        let call = async {
            let response = self
                .send_authorized(workspace.as_deref(), |http, token| {
                    let mut body = body.clone();
                    body.sent_at = Some(Utc::now());
                    http.post(&url).bearer_auth(token).timeout(timeout).json(&body)
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
            .unwrap_or_else(|_| Err(ApiError::Offline(format!("no answer within {} ms", timeout.as_millis()))))
    }

    /// Upload a completed activity time block to the current workspace.
    /// `sentAt` is stamped at send time (the server derives the device's
    /// clock skew from it).
    pub async fn submit_activity(&mut self, activity: &Activity) -> Result<(), SubmitError> {
        let url = format!("{}/api/app/activity", self.base_url);
        let workspace = self.workspace_id.clone();
        let response = self
            .send_authorized(workspace.as_deref(), |http, token| {
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
        return Err(http_error(status.as_u16(), &body));
    }
    let body = response.text().await.map_err(|e| ApiError::Offline(describe(&e)))?;
    serde_json::from_str(&body).map_err(|e| ApiError::Invalid(e.to_string()))
}

#[cfg(test)]
mod tests;
