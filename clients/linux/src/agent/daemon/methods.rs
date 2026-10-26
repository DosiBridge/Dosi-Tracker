//! The daemon's IPC methods (design 10): each request is checked against the
//! snapshot (preconditions give the stable error codes), turned into a
//! worker [`Command`], and answered once the worker applied it (the command's
//! sequence number is reached in `SharedState::commands_handled`) and the
//! state shows the outcome. Commands are idempotent: a request that would
//! change nothing answers `changed: false` without bothering the worker.
//!
//! Sign-in waits at most [`SIGN_IN_TIMEOUT`]; its password is zeroized
//! (`Secret`) and never logged. Consent widenings write a receipt to
//! `consent-log.jsonl` and post a notification.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use base64::Engine as _;
use chrono::{DateTime, Utc};
use serde_json::{json, Value};

use super::relay::RelaySender;
use super::CommandSink;
use crate::agent::config::AppConfig;
use crate::agent::state::{
    Command, LoginErrorKind, PurgeQueue, SharedState, StateHandle, TrackerStatus, WorkspaceChoice,
};
use crate::agent::worker::{owner_key, TASK_NOTE_MAX_CHARS, WEBCAM_UNAVAILABLE};
use crate::consent::{ConsentedPermissions, LocalConsent};
use crate::credentials::{CredentialStore, CredentialStoreKind};
use crate::ipc::server::{BoxFuture, Dispatch, Reply};
use crate::ipc::{
    clamp_at, method, snapshot_value, AtParams, AwayParams, ConfigKeyParams, ConfigSetParams,
    ConsentUpdateParams, ErrorBody, ErrorCode, HelloResult, PauseParams, ProjectListParams,
    ProjectSelectParams, QueueListParams, QueuePurgeParams, SignInParams, SignInResult,
    SignOutParams, TaskSetParams, WorkspaceChooseParams, MAX_QUEUE_LIST, PROTOCOL_VERSION,
    SIGN_IN_TIMEOUT,
};
use crate::paths::LinuxPaths;
use crate::tracking::session_probe::SessionKind;

/// How long a plain command may take to be applied.
const APPLY_TIMEOUT: Duration = Duration::from_secs(10);

/// How long `tracking.sync_now` waits for the upload pass.
const SYNC_WAIT: Duration = Duration::from_secs(30);

/// How long `project.select` waits for a project list still on its way.
const PROJECTS_WAIT: Duration = Duration::from_secs(10);

/// How long workspace lookups may take.
const LOOKUP_TIMEOUT: Duration = Duration::from_secs(20);

/// What [`Methods`] is built from.
pub struct MethodsConfig {
    pub state: StateHandle,
    pub sink: CommandSink,
    pub paths: LinuxPaths,
    pub cfg: AppConfig,
    pub credentials: Arc<dyn CredentialStore>,
    /// Why there is no usable credential store (sign-in is refused).
    pub credential_problem: Option<String>,
    pub started_at: DateTime<Utc>,
    pub socket: PathBuf,
    pub relay: RelaySender,
    pub session_kind: SessionKind,
    /// The machine's clock (`at` is clamped against it).
    pub clock: super::Clock,
}

/// The daemon's [`Dispatch`].
pub struct Methods {
    inner: Arc<MethodsConfig>,
}

impl Methods {
    pub fn new(config: MethodsConfig) -> Self {
        Self {
            inner: Arc::new(config),
        }
    }
}

fn bad_request(message: impl Into<String>) -> ErrorBody {
    ErrorBody::new(ErrorCode::BadRequest, message)
}

fn optional<T: serde::de::DeserializeOwned + Default>(value: Value) -> Result<T, ErrorBody> {
    if value.is_null() {
        return Ok(T::default());
    }
    serde_json::from_value(value).map_err(|e| bad_request(format!("Bad parameters: {e}")))
}

fn required<T: serde::de::DeserializeOwned>(value: Value) -> Result<T, ErrorBody> {
    serde_json::from_value(value).map_err(|e| bad_request(format!("Bad parameters: {e}")))
}

fn signed_in(s: &SharedState) -> bool {
    !s.display_name.is_empty() && s.status != TrackerStatus::SignedOut
}

fn not_signed_in() -> ErrorBody {
    ErrorBody::new(ErrorCode::NotSignedIn, "You are not signed in.").with_hint("dosi-tracker login")
}

/// The first reason `s` cannot take commands for the server-side session.
fn require_account(s: &SharedState) -> Result<(), ErrorBody> {
    if !signed_in(s) {
        return Err(not_signed_in());
    }
    Ok(())
}

fn status_error(s: &SharedState) -> Option<ErrorBody> {
    Some(match s.status {
        TrackerStatus::SignedOut => not_signed_in(),
        TrackerStatus::SignInExpired => {
            ErrorBody::new(ErrorCode::SignInExpired, "Your sign-in has expired.")
                .with_hint("dosi-tracker login")
        }
        TrackerStatus::DeviceRevoked => ErrorBody::new(
            ErrorCode::DeviceRevoked,
            "This computer was removed from the workspace.",
        )
        .with_hint("dosi-tracker workspace use <another workspace>, or dosi-tracker logout"),
        TrackerStatus::ConsentRequired => ErrorBody::new(
            ErrorCode::ConsentRequired,
            "Choose what Dosi Tracker may record first.",
        )
        .with_hint("dosi-tracker consent grant --time-only (or --all, --keyboard, ...)"),
        TrackerStatus::NoSession => ErrorBody::new(
            ErrorCode::NoSession,
            "No graphical session is attached; tracking needs a desktop login.",
        ),
        TrackerStatus::ChooseProject => {
            ErrorBody::new(ErrorCode::ChoiceRequired, "Choose a project first.")
                .with_hint("dosi-tracker project use <project>")
                .with_data(json!({ "projects": s.projects }))
        }
        TrackerStatus::Stopped => ErrorBody::new(
            ErrorCode::Internal,
            s.last_error
                .clone()
                .unwrap_or_else(|| "Tracking is stopped.".into()),
        ),
        _ => return None,
    })
}

fn login_error(s: &SharedState) -> Option<ErrorBody> {
    let message = s.login_error.clone()?;
    let code = match s.login_error_kind {
        Some(LoginErrorKind::Offline) => ErrorCode::Offline,
        Some(LoginErrorKind::InvalidServer) | Some(LoginErrorKind::NoServer) => {
            ErrorCode::BadRequest
        }
        Some(LoginErrorKind::NoWorkspace) => ErrorCode::NotFound,
        Some(LoginErrorKind::ArchivedWorkspace) => ErrorCode::Precondition,
        Some(LoginErrorKind::Expired) => ErrorCode::SignInExpired,
        Some(LoginErrorKind::Refused) | Some(LoginErrorKind::Server) | None => {
            ErrorCode::SignInFailed
        }
    };
    let error = ErrorBody::new(code, message);
    Some(match s.login_error_kind {
        Some(LoginErrorKind::NoServer) => error.with_hint("dosi-tracker login --server URL"),
        _ => error,
    })
}

/// Which capabilities `to` adds over `from`, in words.
fn widened(from: &LocalConsent, to: &LocalConsent) -> Vec<&'static str> {
    let LocalConsent::Granted(to) = to else {
        return Vec::new();
    };
    let from: ConsentedPermissions = from.permissions();
    let mut added = Vec::new();
    let pairs: [(bool, bool, &'static str); 5] = [
        (from.screenshot, to.screenshot, "Screenshots"),
        (from.keyboard, to.keyboard, "Keyboard counts"),
        (from.mouse, to.mouse, "Mouse click counts"),
        (from.active_window, to.active_window, "Active window titles"),
        (
            from.running_programs,
            to.running_programs,
            "Running program names",
        ),
    ];
    for (before, after, name) in pairs {
        if after && !before {
            added.push(name);
        }
    }
    added
}

fn origin_words(origin: &str) -> &'static str {
    match origin {
        "cli" => "from the command line",
        "gui" => "in the Dosi Tracker window",
        "tray" => "from the tray menu",
        _ => "on this computer",
    }
}

/// Append one consent receipt (`{at, from, to, origin}`) to
/// `consent-log.jsonl` (0600 in the 0700 state dir).
pub fn append_consent_receipt(
    paths: &LinuxPaths,
    from: &LocalConsent,
    to: &LocalConsent,
    origin: &str,
) -> std::io::Result<()> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    crate::paths::ensure_private_dir(&paths.state_dir)?;
    let line = json!({ "at": Utc::now(), "from": from, "to": to, "origin": origin });
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(paths.consent_log_file())?;
    let mut bytes = serde_json::to_vec(&line).map_err(std::io::Error::other)?;
    bytes.push(b'\n');
    file.write_all(&bytes)?;
    file.sync_all()
}

impl Dispatch for Methods {
    fn hello(&self) -> HelloResult {
        HelloResult {
            protocol: PROTOCOL_VERSION,
            daemon_version: env!("CARGO_PKG_VERSION").into(),
            pid: std::process::id(),
            started_at: self.inner.started_at,
        }
    }

    fn call(&self, method: &str, params: Value) -> BoxFuture<Result<Reply, ErrorBody>> {
        let inner = self.inner.clone();
        let method = method.to_owned();
        Box::pin(async move { inner.call(&method, params).await })
    }

    fn changes(&self) -> tokio::sync::watch::Receiver<u64> {
        self.inner.state.subscribe()
    }

    fn snapshot(&self) -> Value {
        snapshot_value(self.inner.state.snapshot())
    }
}

impl MethodsConfig {
    fn now(&self) -> DateTime<Utc> {
        (self.clock)()
    }

    /// Wait (at most `timeout`) until `pred` holds for the snapshot.
    async fn until(
        &self,
        timeout: Duration,
        pred: impl Fn(&SharedState) -> bool,
    ) -> Result<SharedState, ErrorBody> {
        let mut changes = self.state.subscribe();
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            let snapshot = self.state.snapshot();
            if pred(&snapshot) {
                return Ok(snapshot);
            }
            tokio::select! {
                changed = changes.changed() => {
                    if changed.is_err() {
                        return Err(ErrorBody::new(ErrorCode::Internal, "The agent is stopping."));
                    }
                }
                _ = tokio::time::sleep_until(deadline) => {
                    return Err(ErrorBody::new(ErrorCode::Timeout, "The agent did not finish in time."));
                }
            }
        }
    }

    /// Send `command` and wait until the worker applied it.
    async fn apply(&self, command: Command) -> Result<SharedState, ErrorBody> {
        let seq = self.sink.send(command);
        self.until(APPLY_TIMEOUT, move |s| s.commands_handled >= seq)
            .await
    }

    async fn call(&self, method: &str, params: Value) -> Result<Reply, ErrorBody> {
        match method {
            method::STATE_GET => Ok(Reply::value(snapshot_value(self.state.snapshot()))),
            method::CAPABILITIES_GET => {
                let s = self.state.snapshot();
                Ok(Reply::value(json!({
                    "capabilities": s.capabilities,
                    "session": s.session,
                    "manual_mode": s.manual_mode,
                    "indicator": s.indicator,
                })))
            }
            method::AUTH_SIGN_IN => self.sign_in(required(params)?).await,
            method::AUTH_SIGN_OUT => self.sign_out(optional(params)?).await,
            method::AUTH_UNLOCK_KEYRING => self.unlock().await,
            method::WORKSPACE_LIST => self.workspace_list().await,
            method::WORKSPACE_CHOOSE => self.workspace_choose(optional(params)?).await,
            method::PROJECT_LIST => self.project_list(optional(params)?).await,
            method::PROJECT_SELECT => self.project_select(required(params)?).await,
            method::TASK_SET => self.task_set(required(params)?).await,
            method::TRACKING_START | method::TRACKING_RESUME => {
                self.resume(optional::<AtParams>(params)?).await
            }
            method::TRACKING_PAUSE => self.pause(optional(params)?).await,
            method::TRACKING_SYNC_NOW => self.sync_now(optional(params)?).await,
            method::AWAY_RESOLVE => self.away(required(params)?).await,
            method::CONSENT_GET => {
                let s = self.state.snapshot();
                Ok(Reply::value(json!({
                    "consent": s.consent,
                    "capabilities": s.capabilities,
                    "consent_unsynced": s.consent_unsynced,
                    "consent_error": s.consent_error,
                })))
            }
            method::CONSENT_UPDATE => self.consent_update(required(params)?).await,
            method::CAPTURE_RETRY_PERMISSION => {
                let s = self.apply(Command::RetryCapture).await?;
                Ok(Reply::changed(
                    json!({ "screenshot": s.capabilities.screenshot }),
                    true,
                ))
            }
            method::QUEUE_STATUS => {
                let s = self.state.snapshot();
                Ok(Reply::value(json!({
                    "source": "daemon",
                    "pending": s.pending_uploads,
                    "parked": s.rejected_uploads,
                    "last_sync": s.last_sync,
                    "upload_issue": s.upload_issue,
                })))
            }
            method::QUEUE_LIST => self.queue_list(optional(params)?).await,
            method::QUEUE_PURGE => self.queue_purge(required(params)?).await,
            method::CONFIG_GET => {
                let p: ConfigKeyParams = required(params)?;
                if !crate::agent::config::keys().any(|k| k == p.key) {
                    return Err(bad_request(format!(
                        "unknown configuration key `{}`",
                        p.key
                    )));
                }
                let value = self
                    .cfg
                    .value(&p.key)
                    .map(|v| serde_json::to_value(v).unwrap_or(Value::Null))
                    .unwrap_or(Value::Null);
                Ok(Reply::value(json!({ "key": p.key, "value": value })))
            }
            method::CONFIG_SET => {
                let p: ConfigSetParams = required(params)?;
                crate::agent::config::set_user_value(
                    &self.paths.config_file(),
                    &p.key,
                    &p.value,
                    &self.cfg,
                )
                .map_err(config_error)?;
                Ok(Reply::changed(
                    json!({ "key": p.key, "restart_required": true }),
                    true,
                ))
            }
            method::CONFIG_UNSET => {
                let p: ConfigKeyParams = required(params)?;
                let removed =
                    crate::agent::config::unset_user_value(&self.paths.config_file(), &p.key)
                        .map_err(config_error)?;
                Ok(Reply::changed(
                    json!({ "key": p.key, "restart_required": removed }),
                    removed,
                ))
            }
            method::CONFIG_RELOAD => Ok(Reply::value(json!({
                "reloaded": false,
                "restart_required": true,
                "hint": "dosi-tracker service restart",
            }))),
            method::SCREENSHOT_LATEST => {
                let s = self.state.snapshot();
                match s.last_screenshot {
                    Some(shot) if !shot.jpeg.is_empty() => Ok(Reply::value(json!({
                        "taken_at": shot.taken_at,
                        "jpeg_base64": base64::engine::general_purpose::STANDARD.encode(&shot.jpeg),
                    }))),
                    _ => Err(ErrorBody::new(
                        ErrorCode::NotFound,
                        "No screenshot was taken in this run.",
                    )),
                }
            }
            method::DAEMON_INFO => {
                let s = self.state.snapshot();
                Ok(Reply::value(json!({
                    "version": env!("CARGO_PKG_VERSION"),
                    "protocol": PROTOCOL_VERSION,
                    "pid": std::process::id(),
                    "started_at": self.started_at,
                    "socket": self.socket,
                    "session": s.session,
                    "indicator": s.indicator,
                    "credentials_store": self.credential_problem.as_ref().map_or_else(
                        || match self.credentials.kind() {
                            CredentialStoreKind::SecretService => "secret-service",
                            CredentialStoreKind::FileFallback => "file",
                            CredentialStoreKind::Memory => "memory",
                        },
                        |_| "unavailable",
                    ),
                    "allow_insecure_http": self.cfg.allow_insecure_http,
                })))
            }
            method::DAEMON_SHUTDOWN => {
                let p: AtParams = optional(params)?;
                self.sink.send(Command::Shutdown {
                    at: clamp_at(p.at, self.now()),
                });
                Ok(Reply::changed(json!({ "stopping": true }), true))
            }
            _ => Err(ErrorBody::new(
                ErrorCode::UnknownMethod,
                "This agent does not know that request.",
            )),
        }
    }

    async fn sign_in(&self, p: SignInParams) -> Result<Reply, ErrorBody> {
        if let Some(problem) = &self.credential_problem {
            return Err(ErrorBody::new(ErrorCode::PermissionDenied, problem.clone())
                .with_hint(crate::credentials::NO_SECRET_SERVICE_HINT));
        }
        let email = p.email.trim().to_owned();
        if email.is_empty() {
            return Err(bad_request("The email address is missing."));
        }
        if p.password.is_empty() {
            return Err(bad_request("The password is missing."));
        }
        let s = self.state.snapshot();
        if signed_in(&s) && s.status != TrackerStatus::SignInExpired {
            if owner_key(&s.display_name) == owner_key(&email) {
                return Ok(Reply::changed(
                    serde_json::to_value(SignInResult {
                        status: s.status,
                        workspace_choices: Vec::new(),
                        workspace: Some(s.workspace.clone()),
                    })
                    .unwrap_or(Value::Null),
                    false,
                ));
            }
            return Err(ErrorBody::new(
                ErrorCode::Precondition,
                format!("Signed in as {}; sign out first.", s.display_name),
            )
            .with_hint("dosi-tracker logout"));
        }
        tracing::info!("sign-in requested");
        self.apply(Command::SignIn {
            workspace: p.workspace.clone(),
            username: email,
            password: p.password,
            server: p.server.clone(),
            remember_password: p.remember_password,
        })
        .await?;
        let s = self
            .until(SIGN_IN_TIMEOUT, |s| {
                !s.signing_in
                    && (s.login_error.is_some() || !s.workspace_choices.is_empty() || signed_in(s))
            })
            .await?;
        if let Some(error) = login_error(&s) {
            return Err(error);
        }
        if !signed_in(&s) {
            return Ok(Reply::changed(
                serde_json::to_value(SignInResult {
                    status: s.status,
                    workspace_choices: s.workspace_choices.clone(),
                    workspace: None,
                })
                .unwrap_or(Value::Null),
                false,
            ));
        }
        self.signed_in_cleanup();
        Ok(Reply::changed(
            serde_json::to_value(SignInResult {
                status: s.status,
                workspace_choices: Vec::new(),
                workspace: Some(s.workspace.clone()),
            })
            .unwrap_or(Value::Null),
            true,
        ))
    }

    /// After a successful sign-in: the foundation login's plaintext file goes.
    fn signed_in_cleanup(&self) {
        match crate::credentials::remove_legacy_credentials(&self.paths) {
            Ok(true) => tracing::info!("removed the legacy plaintext credentials file"),
            Ok(false) => {}
            Err(e) => tracing::warn!(error = %e, "could not remove the legacy credentials file"),
        }
    }

    async fn sign_out(&self, p: SignOutParams) -> Result<Reply, ErrorBody> {
        let s = self.state.snapshot();
        if s.display_name.is_empty() {
            return Ok(Reply::changed(json!({ "status": s.status }), false));
        }
        if p.discard_unsent {
            self.apply(Command::PurgeQueue {
                which: PurgeQueue::All,
            })
            .await?;
        }
        let at = clamp_at(p.at, self.now());
        let s = self.apply(Command::SignOut { at }).await?;
        Ok(Reply::changed(
            json!({ "status": s.status, "discarded_unsent": p.discard_unsent }),
            true,
        ))
    }

    async fn unlock(&self) -> Result<Reply, ErrorBody> {
        let s = self.state.snapshot();
        if !s.credentials_locked {
            return Ok(Reply::changed(json!({ "unlocked": true }), false));
        }
        if self.credentials.kind() != CredentialStoreKind::SecretService
            || self.credential_problem.is_some()
        {
            return Err(ErrorBody::new(
                ErrorCode::KeyringLocked,
                s.last_error
                    .clone()
                    .unwrap_or_else(|| "The credential store cannot be read.".into()),
            ));
        }
        let account = owner_key(&s.display_name);
        let unlocked = tokio::task::spawn_blocking(move || {
            crate::credentials::SecretServiceStore::default().unlock(&account)
        })
        .await
        .map_err(|e| ErrorBody::new(ErrorCode::Internal, e.to_string()))?;
        match unlocked {
            Ok(true) => {
                let s = self.apply(Command::ReloadCredentials).await?;
                Ok(Reply::changed(
                    json!({ "unlocked": !s.credentials_locked }),
                    true,
                ))
            }
            Ok(false) => Err(ErrorBody::new(
                ErrorCode::KeyringLocked,
                "The keyring was not unlocked.",
            )),
            Err(e) => Err(ErrorBody::new(ErrorCode::KeyringLocked, e.to_string())),
        }
    }

    /// The workspaces to choose from: those of a pending sign-in, or the
    /// signed-in user's (looked up now).
    async fn choices(&self) -> Result<(SharedState, Vec<WorkspaceChoice>), ErrorBody> {
        let s = self.state.snapshot();
        if !s.workspace_choices.is_empty() {
            let choices = s.workspace_choices.clone();
            return Ok((s, choices));
        }
        require_account(&s)?;
        if let Some(error) = status_error(&s).filter(|e| e.code == ErrorCode::SignInExpired) {
            return Err(error);
        }
        self.apply(Command::SwitchWorkspace).await?;
        let s = self
            .until(LOOKUP_TIMEOUT, |s| !s.finding_workspaces)
            .await?;
        let choices = s.workspace_choices.clone();
        Ok((s, choices))
    }

    async fn workspace_list(&self) -> Result<Reply, ErrorBody> {
        let (s, choices) = self.choices().await?;
        Ok(Reply::value(json!({
            "current": (!s.workspace.is_empty()).then(|| s.workspace.clone()),
            "choices": choices,
            "note": s.workspace_note,
        })))
    }

    async fn workspace_choose(&self, p: WorkspaceChooseParams) -> Result<Reply, ErrorBody> {
        let pending_sign_in = {
            let s = self.state.snapshot();
            !signed_in(&s) && !s.workspace_choices.is_empty()
        };
        let (s, choices) = self.choices().await?;
        let wanted = p
            .name
            .as_deref()
            .or(p.id.as_deref())
            .map(str::trim)
            .filter(|w| !w.is_empty());
        // The current workspace is never offered when switching.
        if let Some(wanted) = wanted {
            if signed_in(&s) && s.workspace.trim().eq_ignore_ascii_case(wanted) {
                return Ok(Reply::changed(json!({ "workspace": s.workspace }), false));
            }
        }
        let index = match (p.index, wanted) {
            (Some(index), _) if index < choices.len() => index,
            (Some(_), _) => {
                return Err(
                    ErrorBody::new(ErrorCode::NotFound, "There is no such choice.")
                        .with_data(json!({ "choices": choices })),
                )
            }
            (None, Some(wanted)) => {
                let by_id = choices
                    .iter()
                    .position(|c| !c.id.is_empty() && c.id.eq_ignore_ascii_case(wanted));
                let named: Vec<usize> = choices
                    .iter()
                    .enumerate()
                    .filter(|(_, c)| c.name.trim().eq_ignore_ascii_case(wanted))
                    .map(|(i, _)| i)
                    .collect();
                match (by_id, named.as_slice()) {
                    (Some(i), _) => i,
                    (None, [i]) => *i,
                    (None, []) => {
                        if !signed_in(&s) || choices.is_empty() {
                            if let Some(note) = &s.workspace_note {
                                return Err(ErrorBody::new(ErrorCode::NotFound, note.clone()));
                            }
                        }
                        return Err(ErrorBody::new(
                            ErrorCode::NotFound,
                            format!("You are not in a workspace named {wanted}."),
                        )
                        .with_data(json!({ "choices": choices })));
                    }
                    (None, _) => {
                        return Err(ErrorBody::new(
                            ErrorCode::ChoiceRequired,
                            format!("Several workspaces are named {wanted}; choose by number."),
                        )
                        .with_data(json!({ "choices": choices })))
                    }
                }
            }
            (None, None) => {
                return Err(
                    ErrorBody::new(ErrorCode::ChoiceRequired, "Choose a workspace.")
                        .with_data(json!({ "choices": choices })),
                )
            }
        };
        let name = choices[index].name.clone();
        if pending_sign_in || !signed_in(&s) {
            self.apply(Command::ChooseWorkspace { index }).await?;
            let s = self
                .until(LOOKUP_TIMEOUT, |s| signed_in(s) || s.login_error.is_some())
                .await?;
            if let Some(error) = login_error(&s) {
                return Err(error);
            }
            self.signed_in_cleanup();
            return Ok(Reply::changed(json!({ "workspace": s.workspace }), true));
        }
        let at = clamp_at(p.at, self.now());
        let s = self.apply(Command::SwitchToWorkspace { index, at }).await?;
        Ok(Reply::changed(
            json!({ "workspace": if s.workspace.is_empty() { name } else { s.workspace } }),
            true,
        ))
    }

    async fn project_list(&self, p: ProjectListParams) -> Result<Reply, ErrorBody> {
        let s = self.state.snapshot();
        require_account(&s)?;
        let s = if p.refresh {
            let before = s.projects.clone();
            self.apply(Command::RefreshProjects).await?;
            // The server's answer arrives asynchronously: take a changed
            // list as soon as it comes, else what is there after a while.
            self.until(Duration::from_secs(3), move |s| s.projects != before)
                .await
                .unwrap_or_else(|_| self.state.snapshot())
        } else {
            s
        };
        Ok(Reply::value(json!({
            "projects": s.projects,
            "selected": s.selected_project,
            "server_reachable": s.server_reachable,
        })))
    }

    async fn project_select(&self, p: ProjectSelectParams) -> Result<Reply, ErrorBody> {
        let s = self.state.snapshot();
        require_account(&s)?;
        if let Some(error) = status_error(&s).filter(|e| {
            matches!(
                e.code,
                ErrorCode::SignInExpired | ErrorCode::DeviceRevoked | ErrorCode::NotSignedIn
            )
        }) {
            return Err(error);
        }
        let wanted = p.project.trim();
        if wanted.is_empty() {
            return Err(bad_request("Name the project."));
        }
        // Right after signing in the project list may still be on its way.
        let s = if s.projects.is_empty() {
            self.until(PROJECTS_WAIT, |s| !s.projects.is_empty())
                .await
                .unwrap_or_else(|_| self.state.snapshot())
        } else {
            s
        };
        let by_id = s
            .projects
            .iter()
            .find(|o| o.id.eq_ignore_ascii_case(wanted));
        let titled: Vec<_> = s
            .projects
            .iter()
            .filter(|o| o.title.trim().eq_ignore_ascii_case(wanted))
            .collect();
        let project = match (by_id, titled.as_slice()) {
            (Some(project), _) => project.clone(),
            (None, [project]) => (*project).clone(),
            (None, []) => {
                return Err(ErrorBody::new(
                    ErrorCode::NotFound,
                    format!("No project named {wanted}."),
                )
                .with_data(json!({ "projects": s.projects })))
            }
            (None, _) => {
                return Err(ErrorBody::new(
                    ErrorCode::ChoiceRequired,
                    format!("Several projects are named {wanted}; use the id."),
                )
                .with_data(json!({ "projects": titled })))
            }
        };
        let tracking_it = s.selected_project.as_deref() == Some(project.id.as_str())
            && matches!(
                s.status,
                TrackerStatus::Tracking | TrackerStatus::Away | TrackerStatus::Locked
            );
        if tracking_it {
            return Ok(Reply::changed(
                json!({ "project": project, "status": s.status }),
                false,
            ));
        }
        let at = clamp_at(p.at, self.now());
        let s = self
            .apply(Command::SelectProject {
                project_id: project.id.clone(),
                at,
            })
            .await?;
        Ok(Reply::changed(
            json!({ "project": project, "status": s.status }),
            true,
        ))
    }

    async fn task_set(&self, p: TaskSetParams) -> Result<Reply, ErrorBody> {
        let s = self.state.snapshot();
        require_account(&s)?;
        if p.note.chars().count() > TASK_NOTE_MAX_CHARS {
            return Err(bad_request(format!(
                "The task note is longer than {TASK_NOTE_MAX_CHARS} characters."
            )));
        }
        let note = crate::agent::worker::clean_note(&p.note).unwrap_or_default();
        if note == s.task_note {
            return Ok(Reply::changed(json!({ "task_note": note }), false));
        }
        let at = clamp_at(p.at, self.now());
        let s = self.apply(Command::SetTaskNote { note, at }).await?;
        Ok(Reply::changed(json!({ "task_note": s.task_note }), true))
    }

    async fn resume(&self, p: AtParams) -> Result<Reply, ErrorBody> {
        let s = self.state.snapshot();
        require_account(&s)?;
        if let Some(error) = status_error(&s) {
            return Err(error);
        }
        if matches!(
            s.status,
            TrackerStatus::Tracking | TrackerStatus::Away | TrackerStatus::Locked
        ) {
            return Ok(Reply::changed(
                json!({ "status": s.status, "indicator": s.indicator }),
                false,
            ));
        }
        if matches!(s.status, TrackerStatus::Connecting | TrackerStatus::Offline)
            && s.selected_project.is_none()
        {
            return Err(ErrorBody::new(
                ErrorCode::Offline,
                "The server cannot be reached and no project list is saved yet.",
            ));
        }
        let at = clamp_at(p.at, self.now());
        let s = self.apply(Command::Resume { at }).await?;
        Ok(Reply::changed(
            json!({ "status": s.status, "indicator": s.indicator, "manual_mode": s.manual_mode }),
            true,
        ))
    }

    async fn pause(&self, p: PauseParams) -> Result<Reply, ErrorBody> {
        let s = self.state.snapshot();
        require_account(&s)?;
        if let Some(error) = status_error(&s).filter(|e| {
            matches!(
                e.code,
                ErrorCode::SignInExpired | ErrorCode::DeviceRevoked | ErrorCode::NotSignedIn
            )
        }) {
            return Err(error);
        }
        let now = self.now();
        let at = clamp_at(p.at, now);
        let resume_after = p.resume_after_secs.map(Duration::from_secs);
        if s.status == TrackerStatus::Paused && resume_after.is_none() && s.resume_at.is_none() {
            return Ok(Reply::changed(
                json!({ "status": s.status, "resume_at": s.resume_at }),
                false,
            ));
        }
        let s = self.apply(Command::Pause { resume_after, at }).await?;
        Ok(Reply::changed(
            json!({ "status": s.status, "resume_at": s.resume_at }),
            true,
        ))
    }

    async fn sync_now(&self, p: AtParams) -> Result<Reply, ErrorBody> {
        let before = self.state.snapshot();
        require_account(&before)?;
        if let Some(error) = status_error(&before)
            .filter(|e| matches!(e.code, ErrorCode::SignInExpired | ErrorCode::NotSignedIn))
        {
            return Err(error);
        }
        let at = clamp_at(p.at, self.now());
        let applied = self.apply(Command::SyncNow { at }).await?;
        let last_sync = before.last_sync;
        let issue = before.upload_issue.clone();
        let s = match self
            .until(SYNC_WAIT, move |s| {
                s.last_sync != last_sync
                    || (s.upload_issue.is_some() && s.upload_issue != issue)
                    || (s.pending_uploads == 0 && s.commands_handled >= applied.commands_handled)
            })
            .await
        {
            Ok(s) => s,
            Err(_) => self.state.snapshot(),
        };
        if s.pending_uploads > 0 && !s.server_reachable {
            return Err(ErrorBody::new(
                ErrorCode::Offline,
                format!(
                    "The server cannot be reached; {} blocks wait and upload later.",
                    s.pending_uploads
                ),
            ));
        }
        Ok(Reply::changed(
            json!({
                "pending": s.pending_uploads,
                "parked": s.rejected_uploads,
                "last_sync": s.last_sync,
                "upload_issue": s.upload_issue,
            }),
            true,
        ))
    }

    async fn away(&self, p: AwayParams) -> Result<Reply, ErrorBody> {
        let s = self.state.snapshot();
        if s.away_prompt.is_none() {
            return Err(ErrorBody::new(
                ErrorCode::Precondition,
                "There is no time away to keep or discard.",
            ));
        }
        let s = self.apply(Command::ResolveAway { keep: p.keep }).await?;
        Ok(Reply::changed(
            json!({ "kept": p.keep, "status": s.status }),
            true,
        ))
    }

    async fn consent_update(&self, p: ConsentUpdateParams) -> Result<Reply, ErrorBody> {
        if let LocalConsent::Granted(granted) = &p.consent {
            if granted.webcam {
                return Err(ErrorBody::new(
                    ErrorCode::CapabilityUnavailable,
                    WEBCAM_UNAVAILABLE,
                ));
            }
        }
        let before = self.state.snapshot();
        if before.consent == p.consent && !before.consent_unsynced {
            return Ok(Reply::changed(json!({ "consent": before.consent }), false));
        }
        let s = self
            .apply(Command::UpdateConsent { consent: p.consent })
            .await?;
        if s.consent_unsynced || s.consent != p.consent {
            return Err(ErrorBody::new(
                ErrorCode::Internal,
                s.consent_error
                    .clone()
                    .unwrap_or_else(|| "The consent change was not saved.".into()),
            ));
        }
        let origin = p.origin.as_deref().unwrap_or("unknown");
        if let Err(e) = append_consent_receipt(&self.paths, &before.consent, &s.consent, origin) {
            tracing::warn!(error = %e, "could not write the consent receipt");
        }
        let added = widened(&before.consent, &s.consent);
        if !added.is_empty() {
            self.relay.receipt(format!(
                "{} {} turned on for Dosi Tracker {}.",
                added.join(", "),
                if added.len() == 1 { "was" } else { "were" },
                origin_words(origin)
            ));
        } else if !before.consent.is_granted() && s.consent.is_granted() {
            self.relay.receipt(format!(
                "Time tracking was allowed for Dosi Tracker {}.",
                origin_words(origin)
            ));
        }
        if added.contains(&"Screenshots") && self.session_kind == SessionKind::Wayland {
            // Ask the portal once now, while the user is at the computer.
            let _ = self.apply(Command::RetryCapture).await;
        }
        Ok(Reply::changed(
            json!({ "consent": s.consent, "capabilities": s.capabilities }),
            true,
        ))
    }

    async fn queue_list(&self, p: QueueListParams) -> Result<Reply, ErrorBody> {
        let s = self.state.snapshot();
        require_account(&s)?;
        let owner = owner_key(&s.display_name);
        let limit = p.limit.unwrap_or(20).clamp(1, MAX_QUEUE_LIST);
        let path = self.paths.outbox_db();
        let parked = p.parked;
        let rows = tokio::task::spawn_blocking(move || {
            let outbox =
                crate::outbox::Outbox::open(&path, crate::outbox::OutboxLimits::default())?;
            // Parked rows are a subset: read the newest MAX and filter.
            let rows =
                outbox.recent_rows(&owner, if parked { MAX_QUEUE_LIST * 4 } else { limit })?;
            Ok::<_, crate::outbox::OutboxError>(rows)
        })
        .await
        .map_err(|e| ErrorBody::new(ErrorCode::Internal, e.to_string()))?
        .map_err(|e| ErrorBody::new(ErrorCode::Internal, e.to_string()))?;
        let rows: Vec<Value> = rows
            .into_iter()
            .filter(|row| !parked || row.state == crate::outbox::RowState::Parked)
            .take(limit)
            .map(|row| {
                json!({
                    "id": row.id,
                    "workspace_id": row.workspace_id,
                    "project_id": row.project_id,
                    "started_at": row.started_at,
                    "ended_at": row.ended_at,
                    "state": match row.state {
                        crate::outbox::RowState::Pending => "pending",
                        crate::outbox::RowState::Parked => "parked",
                    },
                    "retry_count": row.retry_count,
                    "last_error": row.last_error,
                    "parked_reason": row.parked_reason,
                    "parked_at": row.parked_at,
                    "capture_bytes": row.capture_bytes,
                })
            })
            .collect();
        Ok(Reply::value(json!({ "rows": rows })))
    }

    async fn queue_purge(&self, p: QueuePurgeParams) -> Result<Reply, ErrorBody> {
        if !p.confirmed {
            return Err(
                bad_request("Purging deletes queued blocks; confirm it.").with_hint("add --yes")
            );
        }
        let s = self.state.snapshot();
        require_account(&s)?;
        let s = self.apply(Command::PurgeQueue { which: p.which }).await?;
        Ok(Reply::changed(
            json!({ "pending": s.pending_uploads, "parked": s.rejected_uploads }),
            true,
        ))
    }
}

fn config_error(e: crate::agent::config::ConfigError) -> ErrorBody {
    use crate::agent::config::ConfigError;
    match e {
        ConfigError::UnknownKey(_) | ConfigError::InvalidValue { .. } => bad_request(e.to_string()),
        ConfigError::UserFileUnusable(_) => {
            ErrorBody::new(ErrorCode::PermissionDenied, e.to_string())
        }
        ConfigError::Io(_) => ErrorBody::new(ErrorCode::Internal, e.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::state::{ProjectOption, Secret};

    fn signed_in_state(status: TrackerStatus) -> SharedState {
        SharedState {
            status,
            display_name: "ayesha@acme.test".into(),
            workspace: "Acme".into(),
            ..SharedState::default()
        }
    }

    #[test]
    fn statuses_map_to_their_error_codes() {
        let code = |status| status_error(&signed_in_state(status)).map(|e| e.code);
        assert_eq!(code(TrackerStatus::SignedOut), Some(ErrorCode::NotSignedIn));
        assert_eq!(
            code(TrackerStatus::SignInExpired),
            Some(ErrorCode::SignInExpired)
        );
        assert_eq!(
            code(TrackerStatus::DeviceRevoked),
            Some(ErrorCode::DeviceRevoked)
        );
        assert_eq!(
            code(TrackerStatus::ConsentRequired),
            Some(ErrorCode::ConsentRequired)
        );
        assert_eq!(code(TrackerStatus::NoSession), Some(ErrorCode::NoSession));
        assert_eq!(
            code(TrackerStatus::ChooseProject),
            Some(ErrorCode::ChoiceRequired)
        );
        assert_eq!(code(TrackerStatus::Stopped), Some(ErrorCode::Internal));
        assert_eq!(code(TrackerStatus::Paused), None);
        assert_eq!(code(TrackerStatus::Tracking), None);
    }

    #[test]
    fn login_errors_map_to_exit_classes() {
        let with = |kind| SharedState {
            login_error: Some("x".into()),
            login_error_kind: kind,
            ..SharedState::default()
        };
        let code = |kind| login_error(&with(kind)).unwrap().code;
        assert_eq!(code(Some(LoginErrorKind::Refused)), ErrorCode::SignInFailed);
        assert_eq!(code(Some(LoginErrorKind::Offline)), ErrorCode::Offline);
        assert_eq!(code(Some(LoginErrorKind::NoServer)), ErrorCode::BadRequest);
        assert_eq!(
            code(Some(LoginErrorKind::InvalidServer)),
            ErrorCode::BadRequest
        );
        assert_eq!(code(Some(LoginErrorKind::NoWorkspace)), ErrorCode::NotFound);
        assert_eq!(
            code(Some(LoginErrorKind::ArchivedWorkspace)),
            ErrorCode::Precondition
        );
        assert_eq!(code(None), ErrorCode::SignInFailed);
        assert!(login_error(&SharedState::default()).is_none());
    }

    #[test]
    fn widenings_name_what_was_added() {
        let granted = |screenshot, keyboard| {
            LocalConsent::Granted(ConsentedPermissions {
                screenshot,
                keyboard,
                ..ConsentedPermissions::all_disabled()
            })
        };
        assert_eq!(
            widened(&LocalConsent::Absent, &granted(true, true)),
            vec!["Screenshots", "Keyboard counts"]
        );
        assert_eq!(
            widened(&granted(true, false), &granted(true, true)),
            vec!["Keyboard counts"]
        );
        assert!(widened(&granted(true, true), &granted(false, true)).is_empty());
        assert!(widened(&granted(true, true), &LocalConsent::Revoked).is_empty());
        assert_eq!(origin_words("cli"), "from the command line");
    }

    #[test]
    fn receipts_are_appended_privately() {
        use std::os::unix::fs::PermissionsExt;
        let temp = tempfile::tempdir().unwrap();
        let paths = LinuxPaths::resolve(Some(temp.path()), None, None, None);
        let to = LocalConsent::Granted(ConsentedPermissions::all_disabled());
        append_consent_receipt(&paths, &LocalConsent::Absent, &to, "cli").unwrap();
        append_consent_receipt(&paths, &to, &LocalConsent::Revoked, "gui").unwrap();
        let text = std::fs::read_to_string(paths.consent_log_file()).unwrap();
        let lines: Vec<Value> = text
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect();
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0]["origin"], "cli");
        assert_eq!(lines[0]["from"], "Absent");
        assert_eq!(lines[1]["to"], "Revoked");
        let mode = std::fs::metadata(paths.consent_log_file())
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600);
    }

    fn methods(
        state: StateHandle,
    ) -> (MethodsConfig, tokio::sync::mpsc::UnboundedReceiver<Command>) {
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        let temp = std::env::temp_dir();
        (
            MethodsConfig {
                state,
                sink: CommandSink::new(tx),
                paths: LinuxPaths::resolve(Some(&temp), None, None, None),
                cfg: AppConfig::default(),
                credentials: Arc::new(crate::credentials::InMemoryCredentialStore::new()),
                credential_problem: None,
                started_at: Utc::now(),
                socket: temp.join("sock"),
                relay: RelaySender::none(),
                session_kind: SessionKind::X11,
                clock: Arc::new(Utc::now),
            },
            rx,
        )
    }

    #[tokio::test]
    async fn commands_that_change_nothing_never_reach_the_worker() {
        let state = StateHandle::new();
        state.update(|s| {
            *s = signed_in_state(TrackerStatus::Tracking);
            s.projects = vec![ProjectOption {
                id: "p1".into(),
                title: "Website".into(),
            }];
            s.selected_project = Some("p1".into());
        });
        let (methods, mut rx) = methods(state);
        let resumed = methods
            .call(method::TRACKING_RESUME, json!({}))
            .await
            .unwrap();
        assert_eq!(resumed.changed, Some(false));
        let selected = methods
            .call(method::PROJECT_SELECT, json!({"project": "website"}))
            .await
            .unwrap();
        assert_eq!(selected.changed, Some(false));
        let note = methods
            .call(method::TASK_SET, json!({"note": "   "}))
            .await
            .unwrap();
        assert_eq!(note.changed, Some(false));
        assert!(rx.try_recv().is_err(), "nothing was sent to the worker");
    }

    #[tokio::test]
    async fn preconditions_answer_with_their_codes() {
        let (methods_out, _rx) = methods(StateHandle::new());
        let err = methods_out
            .call(method::TRACKING_START, json!({}))
            .await
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::NotSignedIn);
        let err = methods_out
            .call(method::AWAY_RESOLVE, json!({"keep": true}))
            .await
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::Precondition);
        let err = methods_out
            .call(method::QUEUE_PURGE, json!({"which": "all"}))
            .await
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::BadRequest, "unconfirmed purge");
        let err = methods_out
            .call(
                method::CONSENT_UPDATE,
                json!({"consent": {"Granted": {"webcam": true}}, "origin": "cli"}),
            )
            .await
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::CapabilityUnavailable);
        let err = methods_out
            .call(method::CONFIG_GET, json!({"key": "nope"}))
            .await
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::BadRequest);

        let state = StateHandle::new();
        state.update(|s| {
            *s = signed_in_state(TrackerStatus::ChooseProject);
            s.projects = vec![
                ProjectOption {
                    id: "p1".into(),
                    title: "Same".into(),
                },
                ProjectOption {
                    id: "p2".into(),
                    title: "same".into(),
                },
            ];
        });
        let (signed, _rx) = methods(state);
        let err = signed
            .call(method::PROJECT_SELECT, json!({"project": "Same"}))
            .await
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::ChoiceRequired);
        let err = signed
            .call(method::PROJECT_SELECT, json!({"project": "Other"}))
            .await
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::NotFound);
        let err = signed
            .call(method::TRACKING_START, json!({}))
            .await
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::ChoiceRequired);
        let err = signed
            .call(
                method::AUTH_SIGN_IN,
                serde_json::to_value(SignInParams {
                    server: None,
                    email: "someone@else.test".into(),
                    password: Secret::new("pw"),
                    workspace: None,
                    remember_password: None,
                })
                .unwrap(),
            )
            .await
            .unwrap_err();
        assert_eq!(
            err.code,
            ErrorCode::Precondition,
            "signed in as someone else"
        );
    }

    #[tokio::test]
    async fn a_command_is_answered_once_the_worker_counted_it() {
        let state = StateHandle::new();
        state.update(|s| *s = signed_in_state(TrackerStatus::Tracking));
        let (methods, mut rx) = methods(state.clone());
        let worker = tokio::spawn(async move {
            let command = rx.recv().await.unwrap();
            assert!(matches!(command, Command::Pause { .. }));
            tokio::time::sleep(Duration::from_millis(50)).await;
            state.update(|s| {
                s.status = TrackerStatus::Paused;
                s.commands_handled += 1;
            });
        });
        let reply = methods
            .call(method::TRACKING_PAUSE, json!({}))
            .await
            .unwrap();
        assert_eq!(reply.changed, Some(true));
        assert_eq!(reply.result["status"], "paused");
        worker.await.unwrap();
    }

    #[tokio::test]
    async fn sign_in_is_refused_without_a_credential_store() {
        let (mut config, _rx) = methods(StateHandle::new());
        config.credential_problem = Some("no Secret Service".into());
        let err = config
            .call(
                method::AUTH_SIGN_IN,
                json!({"email": "a@b.test", "password": "pw"}),
            )
            .await
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::PermissionDenied);
        assert_eq!(err.code.exit_code(), 9);
    }
}
