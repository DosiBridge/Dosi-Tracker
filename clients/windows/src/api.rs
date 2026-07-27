use std::time::{Duration, Instant};

use anyhow::{anyhow, Context, Result};
use chrono::{DateTime, Utc};
use serde::Deserialize;

use crate::model::{Activity, Project};

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
    pub total_tracked_minutes: f64,
    pub average_productivity: f64,
}

/// OpenIddict client id seeded by the backend (`Tracker_App`, password grant enabled).
const CLIENT_ID: &str = "Tracker_App";
const SCOPE: &str = "Tracker";

/// Thin HTTP client wrapping the Dosi-Tracker backend REST API.
///
/// Authentication uses the backend's OpenIddict token endpoint
/// (`POST /connect/token`, resource-owner password flow). The token is cached
/// and transparently refreshed via re-login shortly before it expires.
pub struct ApiClient {
    http: reqwest::Client,
    base_url: String,
    username: String,
    password: String,
    /// Workspace (tenant) name. Empty means the host account.
    tenant: String,
    token: Option<Token>,
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

/// Outcome classification for activity upload, so the sync loop can tell
/// permanently-rejected payloads (park them) from transient failures (retry).
pub enum SubmitError {
    /// The server rejected this payload (validation, bad project, …). Do not retry it.
    Rejected(String),
    /// Network / server / auth trouble. Keep the payload queued and retry later.
    Transient(anyhow::Error),
}

impl ApiClient {
    pub fn new(
        base_url: impl Into<String>,
        username: String,
        password: String,
        tenant: String,
    ) -> Self {
        Self {
            http: reqwest::Client::builder()
                .user_agent(concat!("dosi-tracker/", env!("CARGO_PKG_VERSION")))
                .build()
                .expect("failed to build http client"),
            base_url: base_url.into(),
            username,
            password,
            tenant,
            token: None,
        }
    }

    /// Authenticate against the OpenIddict token endpoint and cache the bearer token.
    ///
    /// The workspace (tenant) MUST travel as a query parameter: ABP's tenant
    /// resolvers read route/query/header/cookie, **not** the POST body. Putting
    /// `__tenant` in the form body silently resolves to the host tenant, which
    /// makes a tenant user's sign-in fail (or, worse, authenticate as host).
    pub async fn login(&mut self) -> Result<()> {
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

        let response: TokenResponse = self
            .http
            .post(url)
            .form(&params)
            .send()
            .await?
            .error_for_status()?
            .json()
            .await
            .context("failed to parse token response")?;

        // Refresh one minute early so requests never race the expiry.
        let ttl = response.expires_in.saturating_sub(60).max(60);
        self.token = Some(Token {
            access_token: response.access_token,
            expires_at: Instant::now() + Duration::from_secs(ttl),
        });
        Ok(())
    }

    async fn ensure_token(&mut self) -> Result<String> {
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

    /// Fetch the active (non-archived) projects the user may track, with the
    /// per-project capture permissions the backend pushes down.
    pub async fn projects(&mut self) -> Result<Vec<Project>> {
        let token = self.ensure_token().await?;
        let url = format!("{}/api/app/project/my-projects", self.base_url);
        let projects = self
            .http
            .get(url)
            .bearer_auth(token)
            .send()
            .await?
            .error_for_status()?
            .json()
            .await
            .context("failed to parse projects response")?;
        Ok(projects)
    }

    /// Tracked-time totals for a window, used by the desktop UI's status cards.
    /// Non-admin callers are scoped to their own data server-side.
    pub async fn summary(
        &mut self,
        from: DateTime<Utc>,
        to: DateTime<Utc>,
    ) -> Result<ReportSummary> {
        let token = self.ensure_token().await?;
        let url = format!(
            "{}/api/app/reporting/summary?From={}&To={}",
            self.base_url,
            from.to_rfc3339(),
            to.to_rfc3339()
        );
        let summary = self
            .http
            .get(url)
            .bearer_auth(token)
            .send()
            .await?
            .error_for_status()?
            .json()
            .await
            .context("failed to parse reporting summary")?;
        Ok(summary)
    }

    /// Upload a completed activity time block.
    pub async fn submit_activity(&mut self, activity: &Activity) -> Result<(), SubmitError> {
        let token = self
            .ensure_token()
            .await
            .map_err(SubmitError::Transient)?;
        let url = format!("{}/api/app/activity", self.base_url);

        let response = self
            .http
            .post(url)
            .bearer_auth(token)
            .json(activity)
            .send()
            .await
            .map_err(|e| SubmitError::Transient(e.into()))?;

        let status = response.status();
        if status.is_success() {
            return Ok(());
        }

        if status.as_u16() == 401 {
            // Token got revoked/expired server-side; drop it so the next attempt re-logs-in.
            self.token = None;
        }

        // Auth (401), timeout (408) and throttling (429) are transient; other 4xx are
        // permanent rejections of this payload and must not block the queue forever.
        if status.is_client_error() && !matches!(status.as_u16(), 401 | 408 | 429) {
            let body = response.text().await.unwrap_or_default();
            return Err(SubmitError::Rejected(format!("HTTP {status}: {body}")));
        }

        Err(SubmitError::Transient(anyhow!("HTTP {status}")))
    }
}
