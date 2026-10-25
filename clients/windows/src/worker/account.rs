//! The signed-in account: signing in and out, a sign-in that expired, a
//! computer an admin removed, and the saved sign-in.

use chrono::{DateTime, Utc};
use tokio::time::Instant;

use crate::api::{ApiClient, AuthError, ServerClock, TokenCell};
use crate::config::AppConfig;
use crate::credentials::{self, Credentials};
use crate::model::{EndReason, PresenceStatus};
use crate::state::{CaptureFlags, NoticeKind};
use crate::storage;

use super::control::ControlRequest;
use super::queue::{maintain_queue, publish_queue};
use super::recording::Speed;
use super::uploader::UploadCommand;
use super::{Worker, RECONNECT_EVERY, SIGN_OUT_UPLOAD_GRACE};

/// The signed-in user, the server it lives on and the workspace tracked in.
#[derive(Clone, PartialEq, Eq)]
pub struct Account {
    /// The API address, normalized.
    pub server: String,
    pub username: String,
    /// Only until the server has accepted it (signing in, or a sign-in saved
    /// by an older build): never saved, and never sent again after that.
    pub password: String,
    /// `None` until a workspace is chosen (or after access to it was lost).
    pub workspace_id: Option<String>,
    pub workspace_name: String,
    /// The user's token, shared by every client of this sign-in.
    pub tokens: TokenCell,
    /// This install's id, named on every token grant.
    pub device_id: Option<String>,
    /// `config.toml`'s `allow_insecure_http`.
    pub allow_insecure_http: bool,
}

impl Account {
    pub fn new(server: String, username: String, password: String) -> Self {
        Self {
            server,
            username,
            password,
            workspace_id: None,
            workspace_name: String::new(),
            tokens: TokenCell::default(),
            device_id: None,
            allow_insecure_http: false,
        }
    }

    /// This install (`device_id`) under this configuration.
    pub fn on_device(mut self, device_id: Option<String>, cfg: &AppConfig) -> Self {
        self.device_id = device_id;
        self.allow_insecure_http = cfg.allow_insecure_http;
        self
    }

    /// The same sign-in, tracking in another workspace.
    pub fn in_workspace(mut self, id: &str, name: &str) -> Self {
        self.workspace_id = Some(id.to_owned());
        self.workspace_name = name.to_owned();
        self
    }

    pub(super) fn of(saved: &Credentials, cfg: &AppConfig, device_id: Option<String>) -> Self {
        let refresh_token = saved.refresh_token.clone().filter(|t| !t.is_empty());
        Self {
            server: cfg.effective_api_base_url(saved.server.as_deref()),
            username: saved.username.clone(),
            // A sign-in with a refresh token never falls back to a password.
            // One saved by an older build with the password only exchanges it
            // once for a refresh token (the file no longer holds it).
            password: if refresh_token.is_some() { String::new() } else { saved.legacy_password.clone() },
            workspace_id: saved.workspace_id.clone(),
            workspace_name: saved.workspace_name.clone(),
            tokens: TokenCell::with_refresh_token(refresh_token),
            device_id,
            allow_insecure_http: cfg.allow_insecure_http,
        }
    }

    /// The account, as the offline queue tags its rows: the user on this
    /// server.
    pub fn owner(&self) -> String {
        storage::owner_key(&self.server, &self.username)
    }

    /// A client for this account. Every client shares `clock`, so the clock
    /// warning sees every answer.
    pub fn client(&self, clock: &ServerClock) -> ApiClient {
        ApiClient::new(self.server.clone(), self.username.clone(), self.password.clone())
            .with_server_clock(clock.clone())
            .with_tokens(self.tokens.clone())
            .with_workspace(self.workspace_id.clone())
            .with_device_id(self.device_id.clone())
            .with_insecure_http(self.allow_insecure_http)
    }
}

impl std::fmt::Debug for Account {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Account")
            .field("server", &self.server)
            .field("workspace_id", &self.workspace_id)
            .field("username", &self.username)
            .field("password", &"<hidden>")
            .finish()
    }
}

impl Worker {
    pub(super) fn sign_in(&mut self, workspace: Option<String>, username: String, password: String, server: Option<String>) {
        if self.account.is_some() && !self.expired {
            tracing::warn!("sign-in ignored: already signed in");
            return;
        }
        // A typed address must be one the agent may use: never plain http://
        // to another computer without the admin's opt-in.
        let typed = match server.as_deref().map(str::trim).filter(|s| !s.is_empty()).map(|s| self.cfg.server_url(s)).transpose() {
            Ok(typed) => typed,
            Err(e) => {
                self.state.update(|s| s.login_error = Some(e.to_string()));
                return;
            }
        };
        // The same user signing in again keeps the server saved — and, on
        // that server, the workspace.
        let same_user = self.saved.as_ref().filter(|saved| saved.same_user(&username));
        let saved_server = same_user.map(|saved| self.cfg.effective_api_base_url(saved.server.as_deref()));
        let server = typed.or(saved_server.clone()).unwrap_or_else(|| self.cfg.api_base_url.clone());
        let saved_workspace =
            same_user.filter(|_| saved_server.as_deref() == Some(server.as_str())).and_then(|saved| saved.workspace_id.clone());
        self.clear_choices();
        self.preselect = workspace.map(|w| w.trim().to_owned()).filter(|w| !w.is_empty()).or(saved_workspace);
        self.signing_in = true;
        self.state.update(|s| {
            s.signing_in = true;
            s.login_error = None;
        });
        let account = Account::new(server, username, password).on_device(self.machine.device().device_id.clone(), &self.cfg);
        self.control.send(ControlRequest::SignIn(account));
    }

    /// A workspace was chosen after signing in. The same user in the same
    /// workspace (the password entered again after it expired) keeps
    /// everything it remembered; otherwise it starts afresh.
    pub(super) fn signed_in(&mut self, mut account: Account) {
        let now = self.now();
        self.signing_in = false;
        let same = self.saved.as_ref().is_some_and(|saved| {
            saved.owner_key(&self.cfg.effective_api_base_url(saved.server.as_deref())) == account.owner()
                && saved.workspace_id == account.workspace_id
        });
        if !same && self.account.is_some() {
            self.forget_account(now);
        }
        // The server accepted the password: it is neither kept nor saved
        // (a refused or missing refresh token means "sign in again").
        account.password.clear();
        let mut saved = match self.saved.take() {
            Some(saved) if same => saved,
            _ => Credentials::new(String::new(), None),
        };
        saved.workspace_id = account.workspace_id.clone();
        saved.workspace_name = account.workspace_name.clone();
        saved.username = account.username.clone();
        saved.legacy_password.clear();
        saved.refresh_token = account.tokens.refresh_token();
        saved.server = (account.server != self.cfg.api_base_url).then(|| account.server.clone());
        if !same {
            self.projects = None;
        }
        self.saved = Some(saved);
        self.persist();

        let owner = account.owner();
        if !same {
            maintain_queue(&self.store, Some(&owner));
        }
        self.release_held(&account);
        publish_queue(&self.store, &self.state, Some(&owner));
        let (name, workspace, server) = (account.username.clone(), account.workspace_name.clone(), account.server.clone());
        self.state.update(|s| {
            s.display_name = name;
            s.workspace = workspace;
            s.set_server(server);
            s.signing_in = false;
            s.login_error = None;
            s.workspace_lost = None;
        });
        self.account = Some(account.clone());
        self.expired = false;
        self.contacting = true;
        // Say where this device stands at once.
        self.presence_sent = None;
        self.set_reachable(true);
        self.control.send(ControlRequest::UseAccount(Some(account.clone())));
        self.control.send(ControlRequest::RefreshProjects);
        self.next_projects_refresh = Instant::now() + RECONNECT_EVERY;
        self.uploader.send(UploadCommand::Account(Some(account)));
        self.totals_wanted = true;
        self.choose_session(now);
        self.reconcile(now, EndReason::Interval);
    }

    pub(super) fn sign_in_failed(&mut self, error: &AuthError) {
        self.signing_in = false;
        if is_auth_rejection(error) {
            tracing::info!(%error, "the server refused the sign-in");
        } else {
            tracing::warn!(%error, "could not sign in");
        }
        let message = friendly_login_error(error);
        self.state.update(|s| {
            s.signing_in = false;
            s.login_error = Some(message);
        });
    }

    /// The server refused the saved sign-in (its refresh token revoked or
    /// expired, the password changed, the account deactivated): tracking
    /// stops, the queue is kept for when the password is entered again
    /// (CONTRACT §1).
    pub(super) fn expire(&mut self, reason: &str) {
        if self.account.is_none() || self.expired {
            return;
        }
        let now = self.now();
        tracing::warn!(%reason, "the saved sign-in is no longer accepted");
        let finished = self.blocks.end(now, now, EndReason::SignOut);
        self.record(finished, Speed::Normal);
        self.expired = true;
        self.contacting = false;
        self.away = None;
        // An unanswered question about time away stays: "Keep as work" queues
        // it under this account, uploaded once the password is entered again.
        self.control.send(ControlRequest::UseAccount(None));
        self.uploader.send(UploadCommand::Account(None));
        let message = format!("Your sign-in has expired ({reason}). Enter your password to continue — your tracked time is kept.");
        self.state.update(|s| {
            s.login_error = Some(message);
            s.push_notice(NoticeKind::SignInExpired, now);
        });
    }

    /// A workspace admin removed this computer (403 `Tracker:DeviceRevoked`):
    /// capture and heartbeats stop at once, the saved sign-in for that server
    /// is forgotten, and the user is told. The queue is kept (tagged with the
    /// account), in case the computer is restored and they sign in again.
    pub(super) fn device_revoked(&mut self, reason: &str) {
        if self.account.is_none() {
            return;
        }
        let now = self.now();
        tracing::warn!(%reason, "this device was removed by a workspace admin; tracking stops and the sign-in is forgotten");
        self.machine.apply_sensor_policy(false, None);
        // The work so far is this account's: queued, never sent from here.
        let finished = self.blocks.end(now, now, EndReason::SignOut);
        self.record(finished, Speed::Fast);
        self.control.send(ControlRequest::UseAccount(None));
        self.uploader.send(UploadCommand::Account(None));
        if let Some(path) = &self.credentials_file {
            if let Err(e) = credentials::clear_at(path) {
                tracing::warn!(?e, "could not forget the saved sign-in");
            }
        }
        self.forget_account(now);
        self.state.update(|s| {
            s.login_error = Some(crate::state::DEVICE_REVOKED_TEXT.to_owned());
            s.push_notice(NoticeKind::DeviceRevoked, now);
        });
    }

    pub(super) fn sign_out(&mut self, at: DateTime<Utc>) {
        let now = self.now();
        if self.account.is_some() {
            // The work so far is this account's: queue it (and upload it)
            // before letting go.
            let finished = self.blocks.end(at, now, EndReason::SignOut);
            self.record(finished, Speed::Normal);
            let heartbeat = if self.expired { None } else { self.heartbeat(PresenceStatus::SignedOut, now, now) };
            self.control.farewell_later(heartbeat);
            // The uploader is the last to use the token: once its final
            // drain is over it revokes the refresh token at the server (an
            // expired sign-in has nothing left to revoke).
            self.uploader.send(UploadCommand::SignOut {
                deadline: std::time::Instant::now() + SIGN_OUT_UPLOAD_GRACE,
                revoke: !self.expired,
            });
        }
        if let Some(path) = &self.credentials_file {
            if let Err(e) = credentials::clear_at(path) {
                tracing::warn!(?e, "could not forget the saved sign-in");
            }
        }
        self.forget_account(now);
    }

    /// Let go of the account: nothing it had carries over to the next one.
    /// Its queued rows stay (tagged with it) until it signs in again.
    pub(super) fn forget_account(&mut self, now: DateTime<Utc>) {
        let finished = self.blocks.end(now, now, EndReason::SignOut);
        self.record(finished, Speed::Normal);
        self.saved = None;
        self.account = None;
        self.expired = false;
        self.contacting = false;
        self.projects = None;
        self.session = None;
        self.choose_notified = false;
        self.task_note = None;
        self.pause = None;
        self.away = None;
        self.prompt = None;
        self.screens.clear();
        self.presence_sent = None;
        self.announce_autostart = false;
        self.pending = None;
        self.choices.clear();
        publish_queue(&self.store, &self.state, None);
        let server = self.cfg.api_base_url.clone();
        self.state.update(|s| {
            s.display_name.clear();
            s.workspace.clear();
            s.set_server(server);
            s.projects.clear();
            s.selected_project = None;
            s.tracked_today_secs = 0.0;
            s.tracked_week_secs = 0.0;
            s.week_day_secs = [0.0; 7];
            s.week_days_fetched = None;
            s.last_productivity = 0;
            s.capture = CaptureFlags::default();
            // The next account must not see this one's work.
            s.recent_blocks.clear();
            s.last_screenshot = None;
            s.upload_issue = None;
            s.last_error = None;
            s.workspace_choices.clear();
            s.workspace_note = None;
            s.workspace_lost = None;
        });
    }

    // ---- Persistence ----------------------------------------------------------------------------------

    /// Save the sign-in (never with a password), with the latest refresh
    /// token.
    pub(super) fn persist(&mut self) {
        let live = self.account.as_ref().filter(|_| !self.expired).and_then(|a| a.tokens.refresh_token());
        let Some(saved) = self.saved.as_mut() else { return };
        if let Some(refresh_token) = live {
            saved.refresh_token = Some(refresh_token);
        }
        let Some(path) = &self.credentials_file else { return };
        match credentials::save_at(path, saved) {
            Ok(()) => saved.outdated_format = false,
            Err(e) => tracing::warn!(?e, "could not save the sign-in"),
        }
    }

    /// The server rotates the refresh token on every renewal (whichever task
    /// renewed): save the new one at once, so a crash or a power cut never
    /// leaves only the spent one on disk (which the server would refuse:
    /// "sign-in expired"). Once a refresh token exists, a password still in
    /// memory (the one-time exchange of an older build's sign-in) is dropped.
    pub(super) fn save_rotated_token(&mut self) {
        let Some(account) = self.account.as_mut().filter(|_| !self.expired) else { return };
        let Some(live) = account.tokens.refresh_token() else { return };
        account.password.clear();
        let Some(saved) = self.saved.as_mut() else { return };
        if saved.refresh_token.as_deref() == Some(live.as_str()) {
            return;
        }
        saved.legacy_password.clear();
        tracing::debug!("the refresh token was renewed; saving it");
        self.persist();
    }

    /// A pause survives a restart (CONTRACT §4).
    pub(super) fn persist_pause(&mut self) {
        if let Some(saved) = self.saved.as_mut() {
            saved.pause = self.pause;
        }
        self.persist();
    }
}

/// Did the server refuse these credentials (as opposed to being unreachable
/// or failing)?
pub(super) fn is_auth_rejection(e: &AuthError) -> bool {
    e.is_rejected()
}

pub(super) fn friendly_login_error(e: &AuthError) -> String {
    e.user_message()
}
