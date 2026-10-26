//! Signing in: the token grants (password once, then the refresh token),
//! the token shared by every client of a sign-in, and revoking it.

use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::{Duration, Instant};

use reqwest::header::ACCEPT;
use serde::Deserialize;

use super::errors::{describe, AuthError};
use super::ApiClient;

/// OpenIddict client id seeded by the backend (`Tracker_App`, password grant enabled).
pub(super) const CLIENT_ID: &str = "Tracker_App";
pub(super) const SCOPE: &str = "openid profile email roles offline_access Tracker";

/// OpenIddict's error body: `{"error":"invalid_grant","error_description":"…"}`.
#[derive(Debug, Default, Deserialize)]
struct OAuthError {
    #[serde(default)]
    error: Option<String>,
    #[serde(default)]
    error_description: Option<String>,
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

/// The bearer token (and refresh token) of one signed-in user, shared by
/// every client acting for that user, so a renewal by one is seen by all and
/// a rotated refresh token is never used twice.
#[derive(Clone, Default)]
pub struct TokenCell(Arc<TokenSlot>);

#[derive(Default)]
struct TokenSlot {
    token: std::sync::Mutex<Option<GrantedToken>>,
    /// Held while renewing, so concurrent clients renew once.
    renewing: tokio::sync::Mutex<()>,
    /// The server granted a token to this sign-in at least once. From then
    /// on the password is never sent again: without a refresh token the
    /// sign-in simply expires.
    granted: std::sync::atomic::AtomicBool,
}

impl TokenCell {
    /// A cell that renews with `refresh_token` before its first request.
    pub fn with_refresh_token(refresh_token: Option<String>) -> Self {
        let cell = Self::default();
        if let Some(refresh_token) = refresh_token.filter(|t| !t.is_empty()) {
            cell.set(GrantedToken { access_token: String::new(), refresh_token: Some(refresh_token), expires_at: Instant::now() });
        }
        cell
    }

    pub(super) fn lock(&self) -> std::sync::MutexGuard<'_, Option<GrantedToken>> {
        self.0.token.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub(super) fn set(&self, token: GrantedToken) {
        *self.lock() = Some(token);
    }

    /// The server answered a grant with `token`.
    pub(super) fn granted(&self, token: GrantedToken) {
        self.set(token);
        self.0.granted.store(true, Ordering::Relaxed);
    }

    /// Has the server granted a token to this sign-in yet?
    pub(super) fn was_granted(&self) -> bool {
        self.0.granted.load(Ordering::Relaxed)
    }

    /// Forget every token (signed out): nothing is sent with them again.
    pub(super) fn clear(&self) {
        *self.lock() = None;
    }

    /// The server granted `refresh_token` (or none) with a fresh access
    /// token, as a renewal by another task of the sign-in would.
    #[cfg(test)]
    pub fn grant_for_test(&self, refresh_token: Option<&str>) {
        self.granted(GrantedToken {
            access_token: "t0k3n".into(),
            refresh_token: refresh_token.map(str::to_owned),
            expires_at: Instant::now() + Duration::from_secs(3600),
        });
    }

    /// The latest refresh token, to save with the sign-in. The server rotates
    /// it on every renewal; the worker saves each new one at once.
    pub fn refresh_token(&self) -> Option<String> {
        self.lock().as_ref().and_then(|t| t.refresh_token.clone())
    }

    pub(super) fn valid_access_token(&self) -> Option<String> {
        self.lock()
            .as_ref()
            .filter(|t| !t.access_token.is_empty() && Instant::now() < t.expires_at)
            .map(|t| t.access_token.clone())
    }

    /// The server refused `access_token`: renew before the next request
    /// (unless another client already did).
    pub(super) fn invalidate(&self, access_token: &str) {
        if let Some(token) = self.lock().as_mut().filter(|t| t.access_token == access_token) {
            token.access_token.clear();
        }
    }
}

impl PartialEq for TokenCell {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

impl Eq for TokenCell {}

impl std::fmt::Debug for TokenCell {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("TokenCell(<hidden>)")
    }
}

#[derive(Clone)]
pub(super) struct GrantedToken {
    pub(super) access_token: String,
    pub(super) refresh_token: Option<String>,
    pub(super) expires_at: Instant,
}

impl GrantedToken {
    fn of(response: TokenResponse, previous_refresh: Option<String>) -> Self {
        // Renew one minute early so requests never race the expiry.
        let ttl = response.expires_in.saturating_sub(60).max(60);
        Self {
            access_token: response.access_token,
            refresh_token: response.refresh_token.filter(|t| !t.is_empty()).or(previous_refresh),
            expires_at: Instant::now() + Duration::from_secs(ttl),
        }
    }
}

#[derive(Deserialize)]
pub(super) struct TokenResponse {
    access_token: String,
    #[serde(default)]
    refresh_token: Option<String>,
    #[serde(default = "default_expires_in")]
    expires_in: u64,
}

pub(super) fn default_expires_in() -> u64 {
    3600
}

impl ApiClient {
    /// `POST /connect/token` with `params`; the token is stored on success.
    pub(super) async fn token_request(&self, params: &[(&str, &str)], previous_refresh: Option<String>) -> Result<(), AuthError> {
        self.check_transport()?;
        let mut params = params.to_vec();
        if let Some(device_id) = self.device_id.as_deref() {
            params.push(("device_id", device_id));
        }
        let params = params.as_slice();
        let url = format!("{}/connect/token", self.base_url);
        let response = self
            .execute(self.http.post(url).header(ACCEPT, "application/json").form(params))
            .await
            .map_err(|e| AuthError::Offline(describe(&e)))?;
        let status = response.status();
        let body = response.text().await.map_err(|e| AuthError::Offline(describe(&e)))?;
        if !status.is_success() {
            return Err(classify_token_failure(status.as_u16(), &body));
        }
        let token: TokenResponse = serde_json::from_str(&body)
            .map_err(|e| AuthError::Server(format!("The server sent an unexpected sign-in response ({e}).")))?;
        if token.refresh_token.as_deref().is_none_or(str::is_empty) && previous_refresh.is_none() {
            tracing::warn!("the server granted no refresh token: the sign-in ends when this access token expires");
        }
        self.tokens.granted(GrantedToken::of(token, previous_refresh));
        Ok(())
    }

    /// Sign in with the email (or user name) and password. The password is
    /// forgotten once the server accepted it.
    pub async fn login(&mut self) -> Result<(), AuthError> {
        let params = [
            ("grant_type", "password"),
            ("client_id", CLIENT_ID),
            ("username", self.username.as_str()),
            ("password", self.password.as_str()),
            ("scope", SCOPE),
        ];
        self.token_request(&params, None).await?;
        self.password.clear();
        Ok(())
    }

    /// Revoke this sign-in's refresh token at the server (signing out), then
    /// forget every token of it. Best effort, bounded by `timeout`: the
    /// sign-in is forgotten locally whatever the answer.
    ///
    /// The endpoint is the one the server's discovery document advertises
    /// (`/.well-known/openid-configuration`, `revocation_endpoint`) when it is
    /// on this server; otherwise ABP's default `/connect/revocat`. A server
    /// without revocation answers 404 (or `unsupported_token_type`):
    /// [`Revocation::Unsupported`].
    pub async fn revoke_refresh_token(&self, timeout: Duration) -> Revocation {
        let Some(refresh_token) = self.tokens.refresh_token() else {
            self.tokens.clear();
            return Revocation::NothingToRevoke;
        };
        let outcome = match tokio::time::timeout(timeout, self.revoke(&refresh_token)).await {
            Ok(outcome) => outcome,
            Err(_) => Revocation::Failed(format!("no answer within {} ms", timeout.as_millis())),
        };
        self.tokens.clear();
        outcome
    }

    pub(super) async fn revoke(&self, refresh_token: &str) -> Revocation {
        if let Err(e) = self.check_transport() {
            return Revocation::Failed(e.to_string());
        }
        let url = self.revocation_endpoint().await;
        let params = [("token", refresh_token), ("token_type_hint", "refresh_token"), ("client_id", CLIENT_ID)];
        let response = match self.execute(self.http.post(&url).header(ACCEPT, "application/json").form(&params)).await {
            Ok(response) => response,
            Err(e) => return Revocation::Failed(describe(&e)),
        };
        let status = response.status();
        if status.is_success() {
            return Revocation::Revoked;
        }
        let body = response.text().await.unwrap_or_default();
        let oauth: OAuthError = serde_json::from_str(&body).unwrap_or_default();
        let unsupported = matches!(status.as_u16(), 404 | 405 | 501)
            || matches!(oauth.error.as_deref(), Some("unsupported_token_type" | "unauthorized_client"));
        let reason = oauth.error_description.or(oauth.error).unwrap_or_else(|| format!("HTTP {status}"));
        if unsupported {
            Revocation::Unsupported(reason)
        } else {
            Revocation::Failed(reason)
        }
    }

    /// Where to revoke tokens: the discovery document's endpoint when it is
    /// on this same server (a token is never sent to another host), else
    /// ABP's default path.
    pub(super) async fn revocation_endpoint(&self) -> String {
        #[derive(Deserialize)]
        struct Discovery {
            #[serde(default)]
            revocation_endpoint: Option<String>,
        }
        let fallback = format!("{}/connect/revocat", self.base_url);
        let url = format!("{}/.well-known/openid-configuration", self.base_url);
        let response = match self.execute(self.http.get(url).header(ACCEPT, "application/json")).await {
            Ok(response) if response.status().is_success() => response,
            _ => return fallback,
        };
        let advertised = response
            .text()
            .await
            .ok()
            .and_then(|body| serde_json::from_str::<Discovery>(&body).ok())
            .and_then(|discovery| discovery.revocation_endpoint);
        match advertised {
            Some(endpoint) if same_origin(&endpoint, &self.base_url) => endpoint,
            Some(endpoint) => {
                tracing::warn!(%endpoint, "the advertised revocation endpoint is on another server; using this server's");
                fallback
            }
            None => fallback,
        }
    }

    pub(super) async fn refresh(&self, refresh_token: &str) -> Result<(), AuthError> {
        let params = [("grant_type", "refresh_token"), ("client_id", CLIENT_ID), ("refresh_token", refresh_token)];
        self.token_request(&params, Some(refresh_token.to_owned())).await
    }

    pub(super) async fn ensure_token(&mut self) -> Result<String, AuthError> {
        if let Some(token) = self.tokens.valid_access_token() {
            return Ok(token);
        }
        let slot = self.tokens.0.clone();
        let _renewing = slot.renewing.lock().await;
        if let Some(token) = self.tokens.valid_access_token() {
            return Ok(token);
        }
        if let Some(refresh_token) = self.tokens.refresh_token() {
            // A refused refresh token (`invalid_grant`: revoked, expired, the
            // password changed) means "sign-in expired" — never a silent
            // sign-in with a password behind the user's back.
            self.refresh(&refresh_token).await?;
        } else if self.password.is_empty() || self.tokens.was_granted() {
            // No refresh token: the sign-in was never kept, or the server
            // granted none and the access token ran out.
            return Err(AuthError::Rejected("Please sign in again.".to_owned()));
        } else {
            self.login().await?;
        }
        self.tokens
            .valid_access_token()
            .ok_or_else(|| AuthError::Server("The server sent an unusable token.".to_owned()))
    }
}

/// How revoking the refresh token at sign-out went (it is forgotten locally
/// either way).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Revocation {
    /// The server confirmed it (RFC 7009: also for a token already invalid).
    Revoked,
    /// This sign-in held no refresh token.
    NothingToRevoke,
    /// The server has no revocation endpoint (or refuses this client one).
    Unsupported(String),
    /// Unreachable, too slow, or another error: the token expires by itself.
    Failed(String),
}

/// Do `url` and `base` name the same scheme, host and port?
pub(super) fn same_origin(url: &str, base: &str) -> bool {
    match (reqwest::Url::parse(url), reqwest::Url::parse(base)) {
        (Ok(a), Ok(b)) => {
            a.scheme() == b.scheme()
                && a.host_str().map(str::to_ascii_lowercase) == b.host_str().map(str::to_ascii_lowercase)
                && a.port_or_known_default() == b.port_or_known_default()
        }
        _ => false,
    }
}
