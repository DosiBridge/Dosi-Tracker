use anyhow::{Context, Result};
use serde::Serialize;

use crate::model::{Activity, AuthSession, Project};

/// Thin HTTP client wrapping the Dosi-Tracker backend REST API.
pub struct ApiClient {
    http: reqwest::Client,
    base_url: String,
    token: Option<String>,
}

#[derive(Serialize)]
struct LoginRequest<'a> {
    username: &'a str,
    password: &'a str,
}

impl ApiClient {
    pub fn new(base_url: impl Into<String>) -> Self {
        Self {
            http: reqwest::Client::builder()
                .user_agent(concat!("dosi-tracker/", env!("CARGO_PKG_VERSION")))
                .build()
                .expect("failed to build http client"),
            base_url: base_url.into(),
            token: None,
        }
    }

    pub fn set_token(&mut self, token: String) {
        self.token = Some(token);
    }

    /// Authenticate and store the bearer token.
    pub async fn login(&mut self, username: &str, password: &str) -> Result<AuthSession> {
        let url = format!("{}/api/app/account/login", self.base_url);
        let session: AuthSession = self
            .http
            .post(url)
            .json(&LoginRequest { username, password })
            .send()
            .await?
            .error_for_status()?
            .json()
            .await
            .context("failed to parse login response")?;

        self.token = Some(session.access_token.clone());
        Ok(session)
    }

    /// Fetch the active (non-archived) projects the user can track.
    pub async fn projects(&self) -> Result<Vec<Project>> {
        let url = format!("{}/api/app/project/my-projects", self.base_url);
        let projects = self
            .authed(self.http.get(url))
            .send()
            .await?
            .error_for_status()?
            .json()
            .await
            .context("failed to parse projects response")?;
        Ok(projects)
    }

    /// Upload a completed activity time block.
    pub async fn submit_activity(&self, activity: &Activity) -> Result<()> {
        let url = format!("{}/api/app/activity", self.base_url);
        self.authed(self.http.post(url))
            .json(activity)
            .send()
            .await?
            .error_for_status()?;
        Ok(())
    }

    fn authed(&self, rb: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        match &self.token {
            Some(t) => rb.bearer_auth(t),
            None => rb,
        }
    }
}
