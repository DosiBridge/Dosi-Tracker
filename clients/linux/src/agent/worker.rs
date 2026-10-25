//! The `dosi-worker` thread: ported Windows worker loop (system notices,
//! commands, control/upload/capture events, 2 s tick) publishing `SharedState`.
//!
//! Owns everything slow or fallible — capture, the local queue, talking to
//! the server — away from the clients, in parts so none waits on another:
//!
//! - the worker loop (this module, on its own thread) decides when blocks
//!   start and end — from the clients' commands, the session and power
//!   events, the user's input and the clocks — builds them and queues them
//!   locally. It never waits on the network, the capture thread, D-Bus or the
//!   portal, so a pause, a lock or a suspend is handled at once;
//! - the control task ([`super::control`]) signs in, fetches the workspaces
//!   and projects, sends the presence heartbeat, refreshes the totals and
//!   checks the device's status;
//! - the uploader thread ([`super::uploader`]) uploads the queue, with its
//!   own connection to it;
//! - screenshots are taken on `dosi-capture` and come back as
//!   [`CaptureEvent`]s.
//!
//! Everything the worker asks of the machine (input times, the screen, the
//! device, the sensors) goes through [`Machine`], so its rules are tested
//! without a display.
//!
//! Linux deltas (design 5 and 6):
//! - Storage is the outbox v2: captures travel next to the payload, rows are
//!   signed with the device key, queue counts and rows are published.
//! - The sign-in is split: `session.json` ([`super::session_store`]) keeps
//!   what is not secret; the account's Secret Service item keeps the tokens,
//!   the device signing key and (when remembered) the password. A rotated
//!   refresh token is saved by a persister thread through the
//!   [`TokenCell`] rotation listener. A refused refresh is "sign-in expired":
//!   the agent never signs in again silently with a saved password.
//! - Consent lives in the [`TrackerLifecycle`] shared with the tracker; an
//!   update narrows at once, persists, then widens (fail closed when the save
//!   fails).
//! - A locked keyring keeps tracking offline on the cached projects; blocks
//!   wait in memory for the signing key; uploads and heartbeats wait for an
//!   explicit unlock.
//! - Manual mode (no idle source): tracking never starts or resumes by
//!   itself and pauses after `tracking.manual_max_hours`.
//! - `no_session` never tracks; `device_revoked` stops tracking and
//!   heartbeats in that workspace and parks its queued rows.

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use chrono::{DateTime, Datelike, Local, NaiveDate, TimeDelta, TimeZone, Utc};
use tokio::sync::mpsc::{self, UnboundedReceiver, UnboundedSender};
use tokio::time::Instant;

use super::blocks::{self, BlockId, Blocks, FinishedBlock, Stretch};
use super::config::{validated_server_url, AppConfig};
use super::control::{self, ControlEvent, ControlLink, ControlRequest};
use super::model::{Activity, EndReason, Heartbeat, PresenceStatus, Project, Workspace, MIN_BLOCK};
use super::power::SleepDetector;
use super::session::{
    self, pick_project, restored_pause, session_for, Availability, Facts, Pick, Session,
};
use super::session_events::{SystemEvent, SystemNotice};
use super::session_store::{self, ProjectCache, SavedPause, SavedWorkspace, SessionData};
use super::state::{
    AwayPrompt, Capabilities, CapabilityInfo, CaptureFlags, Command, LastScreenshot, NoticeKind,
    PlatformAvailability, ProjectOption, QueuedRow, RecentBlock, RowStatus, Secret, SessionSummary,
    StateHandle, TrackerStatus, UploadState, WorkspaceChoice, RECENT_BLOCKS,
};
use super::uploader::{self, UploadCommand, UploadEvent, UploaderHandle};
use crate::api::{ApiClient, ApiError, AuthError, ServerClock, TokenCell};
use crate::consent::{ConsentedPermissions, LocalConsent};
use crate::credentials::{CredentialError, CredentialStore, Credentials};
use crate::lifecycle::TrackerLifecycle;
use crate::outbox::{
    EnqueueResult, Outbox, OutboxLimits, OutboxScope, PendingActivity, QueueLimits, RowState,
    PARKED_RETENTION_DAYS,
};
use crate::tracking::build::{BlockSpec, BuiltBlock, ProvenanceScreenCapture};
use crate::tracking::capture::ScreenCapture;
use crate::tracking::system::DeviceInfo;

/// How often the worker looks at the clocks and the user's input: block
/// boundaries, the screenshot moment, idle, a missed suspend, a timed break
/// running out. Cheap — a few comparisons — and quick enough that coming
/// back to the computer shows within a couple of seconds.
const TICK: Duration = Duration::from_secs(2);

/// How often opening the local queue is retried while it fails.
const STORAGE_RETRY: Duration = Duration::from_secs(60);

/// The project list (and with it each project's policy) is fetched again
/// this often (CONTRACT §2)…
const PROJECTS_REFRESH: Duration = Duration::from_secs(5 * 60);

/// …and this often while the server cannot be reached.
const RECONNECT_EVERY: Duration = Duration::from_secs(30);

/// A presence heartbeat goes out this often, and on every status change.
const HEARTBEAT_EVERY: Duration = Duration::from_secs(60);

/// The today / this-week totals are fetched again this often…
const TOTALS_EVERY: Duration = Duration::from_secs(5 * 60);

/// …and after uploads, but not more often than this.
const TOTALS_SPACING: Duration = Duration::from_secs(15);

/// The queue's housekeeping (parked rows purged after 14 days, the image
/// budget) runs this often in an agent that stays up for weeks.
const MAINTENANCE_EVERY: Duration = Duration::from_secs(6 * 60 * 60);

/// Whether this device was revoked is checked this often (and after a
/// workspace is chosen, and after any 403 `DeviceRevoked`).
const DEVICE_STATUS_EVERY: Duration = Duration::from_secs(6 * 60 * 60);

/// Quitting waits this long for the uploads; the rest go up on the next launch.
const QUIT_UPLOAD_GRACE: Duration = Duration::from_secs(15);

/// Signing out lets the uploader finish the account's queue this long.
const SIGN_OUT_UPLOAD_GRACE: Duration = Duration::from_secs(15);

/// Quitting waits this long for the "stopped" heartbeat.
const FAREWELL_WAIT: Duration = Duration::from_secs(2);

/// A screenshot taken this long after its block's end (a tick late) still
/// belongs to it; one taken later — after an idle cut — does not.
const CAPTURE_SLACK: TimeDelta = TimeDelta::seconds(10);

/// How many queue rows the clients list.
const QUEUE_ROWS_SHOWN: usize = 20;

/// Blocks kept in memory while the keyring is locked (no signing key yet):
/// about a day and a half at the default interval.
pub const MAX_UNSIGNED_BLOCKS: usize = 200;

/// The longest task note ("≤ 200 chars in the UI", CONTRACT §4).
pub const TASK_NOTE_MAX_CHARS: usize = 200;

const ONE_WORKSPACE: &str = "You aren't in any other workspace.";
pub const NO_WORKSPACE: &str =
    "You aren't in any workspace yet. Create one on the web dashboard or ask for an invite.";
const ARCHIVED_WORKSPACE: &str =
    "That workspace is archived. Choose another one or ask its owner to restore it.";
const NO_PROJECTS: &str = "You are not a member of any active project.";
const NO_SERVER: &str = "Enter the server address.";
pub const WEBCAM_UNAVAILABLE: &str = "Webcam capture is not available on Linux in this version.";

/// The signed-in user, the server it lives on and the workspace tracked in.
#[derive(Clone)]
pub struct Account {
    /// The API address, normalized.
    pub server: String,
    pub username: String,
    /// The password typed for a sign-in check; empty everywhere else, so no
    /// client ever signs in again silently when its refresh token is refused.
    pub password: Secret,
    /// `None` until a workspace is chosen (or after access to it was lost).
    pub workspace_id: Option<String>,
    pub workspace_name: String,
    /// The user's token, shared by every client of this sign-in.
    pub tokens: TokenCell,
    /// The install's device id, sent with every token request.
    pub device_id: Option<String>,
    /// The device signing key queued rows are signed with (from the keyring).
    pub signing_key: Option<[u8; 32]>,
    /// `allow_insecure_http` from the configuration.
    pub allow_insecure_http: bool,
}

impl Account {
    pub fn new(server: String, username: String, password: impl Into<String>) -> Self {
        Self {
            server,
            username,
            password: Secret::new(password),
            workspace_id: None,
            workspace_name: String::new(),
            tokens: TokenCell::default(),
            device_id: None,
            signing_key: None,
            allow_insecure_http: false,
        }
    }

    /// The same sign-in, tracking in another workspace.
    pub fn in_workspace(mut self, id: &str, name: &str) -> Self {
        self.workspace_id = Some(id.to_owned());
        self.workspace_name = name.to_owned();
        self
    }

    /// The same sign-in without the typed password.
    pub fn without_password(mut self) -> Self {
        self.password = Secret::default();
        self
    }

    /// The user, as the outbox tags its rows (trimmed, lowercased).
    pub fn owner(&self) -> String {
        owner_key(&self.username)
    }

    /// A client for this account. Every client shares `clock`, so the clock
    /// warning sees every answer.
    pub fn client(&self, clock: &ServerClock) -> ApiClient {
        ApiClient::new(
            self.server.clone(),
            self.username.clone(),
            self.password.expose().to_owned(),
        )
        .with_server_clock(clock.clone())
        .with_tokens(self.tokens.clone())
        .with_workspace(self.workspace_id.clone())
        .with_device_id(self.device_id.clone())
        .with_signing_key(self.signing_key)
        .with_insecure_http(self.allow_insecure_http)
    }
}

impl PartialEq for Account {
    fn eq(&self, other: &Self) -> bool {
        self.server == other.server
            && self.username == other.username
            && self.password.expose() == other.password.expose()
            && self.workspace_id == other.workspace_id
            && self.workspace_name == other.workspace_name
            && self.tokens.same_cell(&other.tokens)
            && self.device_id == other.device_id
            && self.signing_key == other.signing_key
            && self.allow_insecure_http == other.allow_insecure_http
    }
}

impl Eq for Account {}

impl std::fmt::Debug for Account {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Account")
            .field("server", &self.server)
            .field("workspace_id", &self.workspace_id)
            .field("username", &self.username)
            .field("password", &"<hidden>")
            .field("signing_key", &self.signing_key.map(|_| "<hidden>"))
            .finish()
    }
}

/// The outbox key of `username`.
pub fn owner_key(username: &str) -> String {
    username.trim().to_lowercase()
}

/// A screenshot taken on `dosi-capture` for `block_id`; `None` when it
/// failed or was invalidated (consent or policy changed while it ran).
#[derive(Debug)]
pub struct CaptureEvent {
    pub block_id: BlockId,
    pub capture: Option<ProvenanceScreenCapture>,
}

/// What the platform says about one capability, and the sensor behind it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PlatformCapability {
    pub availability: PlatformAvailability,
    pub backend: Option<String>,
}

impl PlatformCapability {
    pub fn available(backend: &str) -> Self {
        Self {
            availability: PlatformAvailability::Available,
            backend: Some(backend.to_owned()),
        }
    }

    pub fn unsupported(reason: &str) -> Self {
        Self {
            availability: PlatformAvailability::Unsupported(reason.to_owned()),
            backend: None,
        }
    }

    /// Can it be effective at all? A capability the desktop cannot provide,
    /// or that the user refused in the desktop's dialog, never is; one that
    /// fails for now or waits for approval stays effective (its data is
    /// simply missing until it works).
    pub fn usable(&self) -> bool {
        !matches!(
            self.availability,
            PlatformAvailability::Unsupported(_) | PlatformAvailability::Denied
        )
    }
}

/// The platform's view of every capability of the capture matrix (design 7).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PlatformCapabilities {
    pub keyboard: PlatformCapability,
    pub mouse: PlatformCapability,
    pub active_window: PlatformCapability,
    pub running_programs: PlatformCapability,
    pub screenshot: PlatformCapability,
}

impl PlatformCapabilities {
    /// Everything available (an X11 session with every sensor working).
    pub fn everything() -> Self {
        Self {
            keyboard: PlatformCapability::available("xi2"),
            mouse: PlatformCapability::available("xi2"),
            active_window: PlatformCapability::available("x11"),
            running_programs: PlatformCapability::available("procfs"),
            screenshot: PlatformCapability::available("x11"),
        }
    }

    /// Nothing (no graphical session).
    pub fn nothing(reason: &str) -> Self {
        Self {
            keyboard: PlatformCapability::unsupported(reason),
            mouse: PlatformCapability::unsupported(reason),
            active_window: PlatformCapability::unsupported(reason),
            running_programs: PlatformCapability::unsupported(reason),
            screenshot: PlatformCapability::unsupported(reason),
        }
    }

    /// What may become effective, for [`session_for`].
    pub fn availability(&self) -> Availability {
        Availability {
            screenshot: self.screenshot.usable(),
            keyboard: self.keyboard.usable(),
            mouse: self.mouse.usable(),
            active_window: self.active_window.usable(),
            running_programs: self.running_programs.usable(),
        }
    }
}

/// What the worker needs from the machine — the time, the user's input,
/// the screen, the device, the sensors — behind thin functions, so the rules
/// above them are tested without a display, a bus or a real clock.
pub trait Machine: Send + Sync {
    fn now(&self) -> DateTime<Utc>;
    /// Milliseconds the machine has been awake ([`super::power::awake_millis`]).
    fn awake_millis(&self) -> u64;
    /// The user's latest input. In manual mode (no presence source) this is
    /// `now`: the user counts as present.
    fn last_input(&self, now: DateTime<Utc>) -> Option<DateTime<Utc>>;
    /// The user's first input after `since`, up to `now` (coming back);
    /// `None` in manual mode.
    fn first_input_after(&self, since: DateTime<Utc>, now: DateTime<Utc>) -> Option<DateTime<Utc>>;
    /// Where finished captures go ([`Machine::request_capture`]).
    fn set_capture_sink(&self, sink: UnboundedSender<CaptureEvent>);
    /// Take a screenshot for `block` on `dosi-capture`; the result arrives as
    /// a [`CaptureEvent`]. `false` when none was started.
    fn request_capture(&self, block: BlockId, blur: bool) -> bool;
    /// A screenshot now, synchronously and bounded (X11 capture-at-end);
    /// `None` for the portal, which is never asked at a block's end.
    fn capture_screen(&self, blur: bool) -> Option<ProvenanceScreenCapture>;
    /// Build the block's payload (and its capture).
    fn snapshot(
        &self,
        block: &BlockSpec<'_>,
        screen: Option<ProvenanceScreenCapture>,
    ) -> BuiltBlock;
    /// The time away kept as work (CONTRACT §4).
    fn kept_idle_blocks(
        &self,
        project_id: &str,
        from: DateTime<Utc>,
        to: DateTime<Utc>,
        interval: Duration,
        description: Option<&str>,
    ) -> Vec<Activity>;
    fn device(&self) -> &DeviceInfo;
    /// The app in front now (its executable name): X11 with the active
    /// window effective only.
    fn current_app(&self) -> Option<String>;
    /// A uniformly random fraction in `[0, 1)` (the screenshot moment).
    fn random_fraction(&self) -> f64;
    /// Start or stop the sensors to match `perms` (the session's effective
    /// permissions) while tracking; everything off otherwise.
    fn apply_sensor_policy(&self, active: bool, perms: Option<&ConsentedPermissions>);
    /// The attached session and its idle source.
    fn session_summary(&self) -> SessionSummary;
    /// What each capability can do on this desktop now.
    fn platform(&self) -> PlatformCapabilities;
    /// Ask the screenshot portal again after the user refused it.
    fn retry_capture_permission(&self);
}

/// Seconds as a delta, bounded to a year (idle times beyond that are noise).
pub fn seconds(secs: u64) -> TimeDelta {
    TimeDelta::seconds(secs.min(366 * 24 * 3600) as i64)
}

/// How the worker was started.
#[derive(Debug, Clone, Copy)]
pub struct Launch {
    /// By the login autostart hook (`service session-start`).
    pub autostart: bool,
    /// A visible indicator exists (tray item or notifications): tracking may
    /// resume by itself only then (invariant I5).
    pub indicator: bool,
}

impl Default for Launch {
    fn default() -> Self {
        Self {
            autostart: false,
            indicator: true,
        }
    }
}

/// Everything [`spawn`] needs.
pub struct WorkerConfig {
    pub state: StateHandle,
    pub cfg: AppConfig,
    pub machine: Arc<dyn Machine>,
    /// Shared with the tracker's consent gate; its consent manager persists.
    pub lifecycle: Arc<Mutex<TrackerLifecycle>>,
    pub credentials: Arc<dyn CredentialStore>,
    /// `$XDG_DATA_HOME/dosi-tracker/outbox.db`.
    pub outbox_path: PathBuf,
    /// `$XDG_STATE_HOME/dosi-tracker/session.json`; `None` keeps it in memory.
    pub session_file: Option<PathBuf>,
    pub launch: Launch,
}

/// The running worker.
pub struct WorkerHandle {
    /// The clients' commands.
    pub commands: UnboundedSender<Command>,
    /// Lock, sleep and shutdown events ([`super::session_events`]).
    pub system: UnboundedSender<SystemNotice>,
    pub thread: std::thread::JoinHandle<()>,
}

/// Spawn the `dosi-worker` thread.
pub fn spawn(config: WorkerConfig) -> std::io::Result<WorkerHandle> {
    let (commands, commands_rx) = mpsc::unbounded_channel();
    let (system, system_rx) = mpsc::unbounded_channel();
    let thread = std::thread::Builder::new()
        .name("dosi-worker".into())
        .spawn(move || {
            let state = config.state.clone();
            let runtime = match tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            {
                Ok(rt) => rt,
                Err(e) => {
                    tracing::error!(?e, "failed to start the worker runtime");
                    state.update(|s| {
                        s.status = TrackerStatus::Stopped;
                        s.last_error = Some(format!("Tracker could not start: {e}"));
                    });
                    return;
                }
            };
            // A panic in here would otherwise kill the worker silently while the
            // clients kept showing "Tracking" and nothing was captured.
            let machine = config.machine.clone();
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                runtime.block_on(run(config, commands_rx, system_rx));
            }));
            if result.is_err() {
                machine.apply_sensor_policy(false, None);
                tracing::error!("the tracking worker panicked; tracking has stopped");
                state.update(|s| {
                    s.status = TrackerStatus::Stopped;
                    s.last_error =
                        Some("Tracking stopped unexpectedly. Please restart Dosi Tracker.".into());
                });
            }
        })?;
    Ok(WorkerHandle {
        commands,
        system,
        thread,
    })
}

async fn run(
    config: WorkerConfig,
    mut commands: UnboundedReceiver<Command>,
    mut system: UnboundedReceiver<SystemNotice>,
) {
    let WorkerConfig {
        state,
        cfg,
        machine,
        lifecycle,
        credentials,
        outbox_path,
        session_file,
        launch,
    } = config;
    machine.apply_sensor_policy(false, None);

    // A damaged queue is moved aside and replaced. One that cannot be opened
    // at all (no permission, disk full, a newer store) leaves nothing to
    // record into: say so, and try again every minute.
    let opened = loop {
        match Outbox::open_or_recover(&outbox_path, OutboxLimits::default()) {
            Ok(opened) => break opened,
            Err(e) => {
                tracing::error!(error = %e, "could not open the local queue; retrying");
                state.update(|s| {
                    s.status = TrackerStatus::Stopped;
                    s.last_error = Some(format!(
                        "The local queue cannot be opened ({e}). Nothing is recorded until it can — retrying every minute."
                    ));
                });
                tokio::select! {
                    command = commands.recv() => match command {
                        None | Some(Command::Shutdown { .. }) => return,
                        Some(_) => tracing::warn!("command ignored while the local queue is unavailable"),
                    },
                    _ = tokio::time::sleep(STORAGE_RETRY) => {}
                }
            }
        }
    };
    state.update(|s| s.last_error = None);

    let now = machine.now();
    let (saved, session_moved) = match session_file.as_deref() {
        Some(path) => match session_store::load(path, now) {
            Ok(loaded) => (Some(loaded.data), loaded.moved_aside),
            Err(e) => {
                tracing::warn!(error = %e, "the session file cannot be used");
                state.update(|s| s.last_error = Some(e.to_string()));
                (None, None)
            }
        },
        None => (None, None),
    };

    let clock = ServerClock::default();
    let (control_events_tx, mut control_events) = mpsc::unbounded_channel();
    let control = control::spawn(
        state.clone(),
        clock.clone(),
        control_events_tx,
        machine.device().time_zone.clone(),
    );
    let (upload_events_tx, mut upload_events) = mpsc::unbounded_channel();
    let uploader = uploader::spawn(outbox_path, clock.clone(), upload_events_tx);
    let (capture_tx, mut capture_events) = mpsc::unbounded_channel();
    machine.set_capture_sink(capture_tx);

    let mut worker = Worker::new(WorkerParts {
        state,
        cfg,
        machine,
        lifecycle,
        credentials,
        session_file,
        launch,
        clock,
        outbox: opened.outbox,
        control,
        uploader,
    });
    worker.start(saved, opened.moved_aside);
    if let Some(path) = session_moved {
        let moved_to = path.display().to_string();
        let now = worker.now();
        worker
            .state
            .update(|s| s.push_notice(NoticeKind::QueueReset { moved_to }, now));
    }
    worker.publish();

    let mut tick = tokio::time::interval(TICK);
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        // Biased: session and power notices queued while the worker was busy
        // (one may have gone unanswered before the machine slept) are handled
        // before a tick, which would otherwise see the sleep as a gap first.
        tokio::select! {
            biased;
            Some(notice) = system.recv() => {
                let at = notice.at;
                if worker.on_system(notice) {
                    worker.quit(at).await;
                    return;
                }
            }
            command = commands.recv() => match command {
                Some(Command::Shutdown { at }) => {
                    worker.quit(at).await;
                    return;
                }
                Some(command) => worker.on_command(command),
                None => {
                    let now = worker.now();
                    worker.quit(now).await;
                    return;
                }
            },
            Some(event) = control_events.recv() => worker.on_control(event),
            Some(event) = upload_events.recv() => worker.on_upload(event),
            Some(event) = capture_events.recv() => worker.on_capture(event),
            _ = tick.tick() => worker.on_tick(),
        }
        worker.publish();
    }
}

/// Whether to take the time to capture: not before sleep or shutdown, when
/// the block must be saved within logind's delay (and the network is off
/// limits).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Speed {
    Normal,
    Fast,
}

impl Speed {
    fn for_reason(reason: EndReason) -> Self {
        match reason {
            EndReason::Sleep | EndReason::Shutdown => Speed::Fast,
            _ => Speed::Normal,
        }
    }
}

/// The user is away: no input for the idle threshold (or a break ran out
/// with nobody at the computer).
#[derive(Debug, Clone)]
struct Away {
    /// The last activity before they left.
    since: DateTime<Utc>,
    /// Where the time away stops counting: the session locked or the machine
    /// slept meanwhile, and that time is never tracked.
    until: Option<DateTime<Utc>>,
    /// After a break: tracking resumes with the next input, nothing to ask.
    after_break: bool,
    /// The project tracked when they left, for "Keep as work".
    session: Option<Session>,
}

/// The time away, waiting for "Keep as work" or "Discard".
#[derive(Debug, Clone)]
struct PendingPrompt {
    from: DateTime<Utc>,
    to: DateTime<Utc>,
    session: Session,
}

/// What [`Worker::publish`] writes; compared to skip unchanged frames.
#[derive(Debug, Clone, PartialEq)]
struct View {
    status: TrackerStatus,
    tracking_since: Option<DateTime<Utc>>,
    block_started_at: Option<DateTime<Utc>>,
    unsaved_since: Option<DateTime<Utc>>,
    next_capture_at: Option<DateTime<Utc>>,
    paused_since: Option<DateTime<Utc>>,
    resume_at: Option<DateTime<Utc>>,
    away_since: Option<DateTime<Utc>>,
    away_prompt: Option<AwayPrompt>,
    selected_project: Option<String>,
    interval_minutes: u64,
    idle_threshold_minutes: Option<u32>,
    capture: CaptureFlags,
    task_note: String,
    server_reachable: bool,
    clock_skew_minutes: Option<i64>,
    consent: LocalConsent,
    consent_error: Option<String>,
    consent_unsynced: bool,
    capabilities: Capabilities,
    session: SessionSummary,
    manual_mode: bool,
    credentials_locked: bool,
}

/// A block finished while no signing key was available (locked keyring).
struct UnsignedBlock {
    scope: OutboxScope,
    pending: PendingActivity,
}

/// Saves rotated refresh tokens on a thread of its own (`dosi-tokens`), so
/// the request that received one never waits on the keyring.
struct TokenPersister {
    sender: std::sync::mpsc::Sender<(String, String)>,
}

impl TokenPersister {
    fn spawn(store: Arc<dyn CredentialStore>) -> Self {
        let (sender, receiver) = std::sync::mpsc::channel::<(String, String)>();
        let spawned = std::thread::Builder::new()
            .name("dosi-tokens".into())
            .spawn(move || {
                while let Ok((account, refresh_token)) = receiver.recv() {
                    let saved = match store.load_account(&account) {
                        Ok(Some(mut credentials)) => {
                            credentials.refresh_token = Some(refresh_token);
                            store.save_account(&credentials)
                        }
                        // Signed out meanwhile: nothing to update.
                        Ok(None) => Ok(()),
                        Err(e) => Err(e),
                    };
                    if let Err(e) = saved {
                        tracing::warn!(error = %e, "could not save the rotated refresh token");
                    }
                }
            });
        if let Err(e) = spawned {
            tracing::error!(?e, "could not start the token persister");
        }
        Self { sender }
    }

    /// Save every rotation of `tokens` under `account`. Never blocks.
    fn watch(&self, tokens: &TokenCell, account: &str) {
        let sender = self.sender.clone();
        let account = account.to_owned();
        tokens.set_rotation_listener(Arc::new(move |refresh_token: String| {
            let _ = sender.send((account.clone(), refresh_token));
        }));
    }
}

/// What [`Worker::new`] is built from.
pub(crate) struct WorkerParts {
    pub state: StateHandle,
    pub cfg: AppConfig,
    pub machine: Arc<dyn Machine>,
    pub lifecycle: Arc<Mutex<TrackerLifecycle>>,
    pub credentials: Arc<dyn CredentialStore>,
    pub session_file: Option<PathBuf>,
    pub launch: Launch,
    pub clock: ServerClock,
    pub outbox: Outbox,
    pub control: ControlLink,
    pub uploader: UploaderHandle,
}

pub(crate) struct Worker {
    state: StateHandle,
    cfg: AppConfig,
    machine: Arc<dyn Machine>,
    lifecycle: Arc<Mutex<TrackerLifecycle>>,
    credentials: Arc<dyn CredentialStore>,
    persister: TokenPersister,
    session_file: Option<PathBuf>,
    launch: Launch,
    clock: ServerClock,
    store: Outbox,
    control: ControlLink,
    uploader: UploaderHandle,

    /// The consent the session rules use (the lifecycle's, as last applied).
    consent: LocalConsent,
    consent_error: Option<String>,
    consent_unsynced: bool,

    /// What is saved in `session.json`; `None` when signed out.
    saved: Option<SessionData>,
    account: Option<Account>,
    /// Signed in to the server, waiting for a workspace to be picked.
    pending: Option<Account>,
    /// The workspace (name or id) to pick after signing in, if listed.
    preselect: Option<String>,
    /// Workspaces offered to choose from.
    choices: Vec<Workspace>,
    /// The server refused the saved sign-in.
    expired: bool,
    /// The keyring is locked (or unreachable): tracking goes on offline.
    credentials_locked: bool,
    /// The workspace that revoked this device.
    revoked: Option<String>,
    /// A sign-in the user typed is being checked.
    signing_in: bool,
    /// The server's first answer since starting is awaited.
    contacting: bool,
    reachable: bool,
    /// The project list: the server's, or the saved one while offline.
    projects: Option<Vec<Project>>,
    session: Option<Session>,
    choose_notified: bool,
    /// Projects already told about asking for more than the consent allows.
    policy_notified: HashSet<String>,
    task_note: Option<String>,

    pause: Option<SavedPause>,
    /// Manual mode or a login without the conditions to resume: tracking
    /// waits for an explicit resume (never saved).
    hold: Option<DateTime<Utc>>,
    away: Option<Away>,
    prompt: Option<PendingPrompt>,
    locked: bool,
    disconnected: bool,
    /// Since when the machine is (announced to be) asleep.
    suspended: Option<DateTime<Utc>>,
    /// Since when the machine is (announced to be) shutting down.
    shutting_down: Option<DateTime<Utc>>,
    /// When the machine was last seen awake again: resumed, unlocked,
    /// reconnected, or back from a sleep nobody announced.
    awake_since: Option<DateTime<Utc>>,

    blocks: Blocks,
    /// Screenshots taken for blocks not finished yet.
    screens: Vec<(BlockId, ProvenanceScreenCapture)>,
    /// Screenshots asked of `dosi-capture` and not back yet.
    captures_in_flight: Vec<BlockId>,
    /// Blocks waiting for the signing key (locked keyring).
    unsigned: Vec<UnsignedBlock>,
    sleep: SleepDetector,
    /// Tell the user when tracking starts by itself after an autostart.
    announce_autostart: bool,

    session_summary: SessionSummary,
    platform: PlatformCapabilities,
    availability: Availability,

    next_projects_refresh: Instant,
    next_heartbeat: Instant,
    next_maintenance: Instant,
    next_device_status: Instant,
    next_totals: Instant,
    totals_due_at: Instant,
    totals_wanted: bool,
    presence_sent: Option<(PresenceStatus, Option<String>)>,
    presence_since: DateTime<Utc>,
    published: Option<View>,
}

impl Worker {
    pub(crate) fn new(parts: WorkerParts) -> Self {
        let now = Instant::now();
        let presence_since = parts.machine.now();
        let session_summary = parts.machine.session_summary();
        let platform = parts.machine.platform();
        let availability = platform.availability();
        let persister = TokenPersister::spawn(parts.credentials.clone());
        Self {
            state: parts.state,
            cfg: parts.cfg,
            machine: parts.machine,
            lifecycle: parts.lifecycle,
            credentials: parts.credentials,
            persister,
            session_file: parts.session_file,
            launch: parts.launch,
            clock: parts.clock,
            store: parts.outbox,
            control: parts.control,
            uploader: parts.uploader,
            consent: LocalConsent::Absent,
            consent_error: None,
            consent_unsynced: false,
            saved: None,
            account: None,
            pending: None,
            preselect: None,
            choices: Vec::new(),
            expired: false,
            credentials_locked: false,
            revoked: None,
            signing_in: false,
            contacting: false,
            reachable: true,
            projects: None,
            session: None,
            choose_notified: false,
            policy_notified: HashSet::new(),
            task_note: None,
            pause: None,
            hold: None,
            away: None,
            prompt: None,
            locked: false,
            disconnected: false,
            suspended: None,
            shutting_down: None,
            awake_since: None,
            blocks: Blocks::new(),
            screens: Vec::new(),
            captures_in_flight: Vec::new(),
            unsigned: Vec::new(),
            sleep: SleepDetector::new(),
            announce_autostart: false,
            session_summary,
            platform,
            availability,
            next_projects_refresh: now + PROJECTS_REFRESH,
            next_heartbeat: now,
            next_maintenance: now + MAINTENANCE_EVERY,
            next_device_status: now + DEVICE_STATUS_EVERY,
            next_totals: now,
            totals_due_at: now + TOTALS_EVERY,
            totals_wanted: false,
            presence_sent: None,
            presence_since,
            published: None,
        }
    }

    fn now(&self) -> DateTime<Utc> {
        self.machine.now()
    }

    /// No idle source: the idle threshold cannot be enforced (design 5.2).
    fn manual_mode(&self) -> bool {
        self.session_summary.kind.is_graphical() && self.session_summary.idle_source.is_none()
    }

    fn lifecycle(&self) -> std::sync::MutexGuard<'_, TrackerLifecycle> {
        self.lifecycle.lock().unwrap_or_else(|e| e.into_inner())
    }

    // ---- Starting ------------------------------------------------------------

    /// Pick up where the last run left off: the saved sign-in, its pause and
    /// task note, and — when the server cannot be reached or the keyring is
    /// locked — the project list saved with it (at most a week old), so
    /// tracking starts offline too.
    pub(crate) fn start(&mut self, saved: Option<SessionData>, moved_aside: Option<PathBuf>) {
        let consent = self.lifecycle().consent();
        self.consent = consent;
        if !self.consent.is_granted() {
            self.machine.apply_sensor_policy(false, None);
        }

        let now = self.now();
        let owner = saved
            .as_ref()
            .and_then(|s| s.account.as_deref())
            .map(owner_key);
        // Reclaim space (old parked rows, other accounts' expired rows,
        // over-limit images), and show what is left.
        self.maintain_queue(owner.as_deref());
        publish_queue(&self.store, &self.state, owner.as_deref());
        if let Some(path) = moved_aside {
            let moved_to = path.display().to_string();
            self.state
                .update(|s| s.push_notice(NoticeKind::QueueReset { moved_to }, now));
        }

        let prefill = saved.as_ref().and_then(|s| s.server.clone());
        let usable = saved.filter(|s| s.account.is_some() && s.workspace.is_some());
        let server = usable
            .as_ref()
            .and_then(|s| self.cfg.effective_api_base_url(s.server.as_deref()));
        let (Some(mut saved), Some(server)) = (usable, server) else {
            let server = self
                .cfg
                .effective_api_base_url(prefill.as_deref())
                .unwrap_or_default();
            self.state.update(|s| s.server_url = server);
            return;
        };
        let pause = restored_pause(saved.pause, now, self.launch.autostart);
        let pause_changed = pause != saved.pause;
        saved.pause = pause;
        self.pause = pause;
        self.task_note = saved.task_note.as_deref().and_then(clean_note);
        let username = saved.account.clone().unwrap_or_default();
        let workspace = saved.workspace.clone().unwrap_or(SavedWorkspace {
            id: String::new(),
            name: String::new(),
        });
        let account = self
            .new_account(server, username.clone(), "")
            .in_workspace(&workspace.id, &workspace.name);
        // Offline first: the saved list is used at once; the server's answer
        // replaces it shortly.
        self.projects = saved.usable_projects(now).map(<[Project]>::to_vec);
        let server = account.server.clone();
        self.state.update(|s| {
            s.display_name = username;
            s.workspace = workspace.name;
            s.server_url = server;
        });
        self.saved = Some(saved);
        if pause_changed {
            self.persist();
        }
        self.account = Some(account);
        self.load_secrets(now);

        // Tracking resumes by itself after a login autostart only when the
        // configuration wants it and an indicator shows it; in manual mode
        // never (design 5.2).
        let may_resume =
            !self.launch.autostart || (self.cfg.tracking.resume_at_login && self.launch.indicator);
        if self.pause.is_none() && (self.manual_mode() || !may_resume) {
            self.hold = Some(now);
        }
        self.announce_autostart = self.launch.autostart;
        self.choose_session(now);
        self.reconcile(now, EndReason::Interval);
    }

    /// An account of this install: its device id and the transport rule.
    fn new_account(&self, server: String, username: String, password: &str) -> Account {
        let mut account = Account::new(server, username, password);
        let device_id = self.machine.device().device_id.trim().to_owned();
        account.device_id = (!device_id.is_empty()).then_some(device_id);
        account.allow_insecure_http = self.cfg.allow_insecure_http;
        account
    }

    /// Read the account's secrets from the keyring: the tokens and the
    /// signing key. A locked keyring keeps tracking offline; a missing item
    /// means the sign-in must be entered again.
    fn load_secrets(&mut self, now: DateTime<Utc>) {
        let Some(owner) = self.account.as_ref().map(Account::owner) else {
            return;
        };
        match self.credentials.load_account(&owner) {
            Ok(Some(saved)) => {
                self.credentials_locked = false;
                let key = signing_key_of(&saved);
                let mut save_again = false;
                if let Some(account) = self.account.as_mut() {
                    account.tokens = TokenCell::with_refresh_token(saved.refresh_token.clone());
                    account.signing_key = key.or_else(|| {
                        save_again = true;
                        Some(crate::signing::generate_signing_key())
                    });
                    self.persister.watch(&account.tokens, &owner);
                }
                if save_again {
                    self.save_secrets(None);
                }
                self.flush_unsigned();
                self.connect_account();
            }
            Ok(None) => {
                tracing::warn!("the saved sign-in has no keyring item; it must be entered again");
                self.expired = true;
                self.state.update(|s| {
                    s.login_error = Some(
                        "Your sign-in has expired. Enter your password to continue — your tracked time is kept."
                            .into(),
                    );
                    s.push_notice(NoticeKind::SignInExpired, now);
                });
            }
            Err(CredentialError::Locked { .. }) => {
                tracing::info!("the keyring is locked; tracking offline until it is unlocked");
                self.credentials_locked = true;
            }
            Err(e) => {
                tracing::warn!(error = %e, "the keyring cannot be read; tracking offline");
                self.credentials_locked = true;
                let message = e.to_string();
                self.state.update(|s| s.last_error = Some(message));
            }
        }
    }

    /// Use the account (with its secrets) for the server from now on.
    fn connect_account(&mut self) {
        let Some(account) = self.account.clone() else {
            return;
        };
        self.contacting = account.workspace_id.is_some();
        self.control
            .send(ControlRequest::UseAccount(Some(account.clone())));
        self.uploader
            .send(UploadCommand::Account(Some(account.clone())));
        if account.workspace_id.is_some() {
            self.control.send(ControlRequest::RefreshProjects);
            self.control.send(ControlRequest::DeviceStatus);
            self.next_projects_refresh = Instant::now() + RECONNECT_EVERY;
            self.next_device_status = Instant::now() + DEVICE_STATUS_EVERY;
            self.totals_wanted = true;
        }
    }

    /// Write the account's secrets to the keyring: the tokens, the signing
    /// key, the device id and — when `password` is given and remembering is
    /// on — the password.
    fn save_secrets(&mut self, password: Option<&str>) {
        let Some(account) = self.account.as_ref() else {
            return;
        };
        let mut credentials = Credentials::new(account.owner(), None);
        credentials.password = password
            .filter(|_| self.cfg.credentials.remember_password)
            .filter(|p| !p.is_empty())
            .map(str::to_owned);
        if credentials.password.is_none() {
            // Keep a password remembered earlier.
            if let Ok(Some(old)) = self.credentials.load_account(&account.owner()) {
                credentials.password = old
                    .password
                    .filter(|_| self.cfg.credentials.remember_password);
            }
        }
        credentials.server_url = Some(account.server.clone());
        credentials.refresh_token = account.tokens.refresh_token();
        credentials.device_signing_key = account.signing_key.map(|k| k.to_vec());
        credentials.device_id = account.device_id.clone();
        if let Err(e) = self.credentials.save_account(&credentials) {
            tracing::warn!(error = %e, "could not save the sign-in in the keyring");
            let message = format!("Your sign-in could not be saved in the keyring ({e}); you will be asked again after a restart.");
            self.state.update(|s| s.last_error = Some(message));
        }
    }

    // ---- Projects --------------------------------------------------------------

    /// Without a session: pick the project to track (CONTRACT §2) — or ask.
    fn choose_session(&mut self, now: DateTime<Utc>) {
        self.publish_projects();
        if self.session.is_some() {
            return;
        }
        let Some(projects) = self.projects.as_ref() else {
            return;
        };
        let remembered = self
            .saved
            .as_ref()
            .and_then(|saved| saved.project_id.clone());
        match pick_project(projects, remembered.as_deref()) {
            Pick::Project(project) => {
                let project = project.clone();
                self.select(&project);
            }
            Pick::Choose => {
                self.clear_error(NO_PROJECTS);
                if !std::mem::replace(&mut self.choose_notified, true) {
                    self.state
                        .update(|s| s.push_notice(NoticeKind::ChooseProject, now));
                }
            }
            Pick::NoProjects => {
                tracing::warn!("signed in, but not a member of any active project");
                self.state
                    .update(|s| s.last_error = Some(NO_PROJECTS.into()));
            }
        }
    }

    fn session_of(&self, project: &Project) -> Session {
        session_for(project, &self.consent, &self.cfg, &self.availability)
    }

    /// Track `project` from the next block on, and remember it.
    fn select(&mut self, project: &Project) {
        tracing::info!(project = %project.title, "tracking session resolved");
        self.session = Some(self.session_of(project));
        self.choose_notified = false;
        self.clear_error(NO_PROJECTS);
        self.notify_policy_requests(project);
        if let Some(saved) = self.saved.as_mut() {
            if saved.project_id.as_deref() != Some(project.id.as_str()) {
                saved.project_id = Some(project.id.clone());
                self.persist();
            }
        }
    }

    /// The project asks to record more than the local consent allows: nothing
    /// more is captured (effective = intersection), but say so once.
    fn notify_policy_requests(&mut self, project: &Project) {
        let LocalConsent::Granted(granted) = self.consent else {
            return;
        };
        let mut asked = Vec::new();
        let mut want = |allowed: bool, consented: bool, name: &str| {
            if allowed && !consented {
                asked.push(name.to_owned());
            }
        };
        want(project.allow_screenshot, granted.screenshot, "screenshots");
        want(project.allow_keyboard, granted.keyboard, "keyboard counts");
        want(project.allow_mouse, granted.mouse, "mouse clicks");
        want(
            project.allow_active_window,
            granted.active_window,
            "the active window",
        );
        want(
            project.allow_running_programs,
            granted.running_programs,
            "running programs",
        );
        if asked.is_empty() || !self.policy_notified.insert(project.id.clone()) {
            return;
        }
        let now = self.now();
        let project_title = project.title.clone();
        self.state.update(|s| {
            s.push_notice(
                NoticeKind::PolicyRequestsConsent {
                    project_title,
                    capabilities: asked,
                },
                now,
            )
        });
    }

    fn publish_projects(&self) {
        let options: Vec<ProjectOption> = self
            .projects
            .iter()
            .flatten()
            .map(|p| ProjectOption {
                id: p.id.clone(),
                title: p.title.clone(),
            })
            .collect();
        self.state.update(|s| s.projects = options);
    }

    /// The server's project list: saved for offline starts, the tracked
    /// project's refreshed policy applied from the next block on — or, if
    /// that project is gone, tracking stopped there.
    fn apply_projects(&mut self, projects: Vec<Project>) {
        let now = self.now();
        self.contacting = false;
        self.set_reachable(true);
        self.next_projects_refresh = Instant::now() + PROJECTS_REFRESH;
        if let Some(saved) = self.saved.as_mut() {
            saved.projects = Some(ProjectCache {
                resolved_at: now,
                projects: projects.clone(),
            });
        }
        self.persist();
        self.projects = Some(projects);

        if let Some(current) = self.session.as_ref().map(|s| s.project_id().to_owned()) {
            let listed = self
                .projects
                .iter()
                .flatten()
                .find(|p| p.id == current)
                .cloned();
            match listed {
                Some(project) => {
                    let refreshed = self.session_of(&project);
                    self.blocks.update_policy(refreshed.policy.clone());
                    self.session = Some(refreshed);
                    self.notify_policy_requests(&project);
                }
                None => self.project_gone(now),
            }
        }
        self.choose_session(now);
        self.reconcile(now, EndReason::Project);
    }

    /// The tracked project was archived or the membership removed. What was
    /// tracked on it so far is kept (the server takes a block that started
    /// before the archiving); tracking stops, unless exactly one project is
    /// left to move to.
    fn project_gone(&mut self, now: DateTime<Utc>) {
        let Some(gone) = self.session.take() else {
            return;
        };
        tracing::warn!(project = %gone.title, "the tracked project is no longer available");
        let finished = self.blocks.end(now, now, EndReason::Project);
        self.record(finished, Speed::Normal);
        if let Some(saved) = self.saved.as_mut() {
            saved.project_id = None;
        }
        self.persist();
        let next = match self.projects.as_deref() {
            Some([only]) => Some(only.title.clone()),
            _ => None,
        };
        // The notice already asks to choose when that is needed.
        self.choose_notified = true;
        self.state.update(|s| {
            s.push_notice(
                NoticeKind::ProjectUnavailable {
                    title: gone.title,
                    now_tracking: next,
                },
                now,
            )
        });
    }

    /// The session's rules with the current consent and platform, for the
    /// next blocks.
    fn refresh_session(&mut self) {
        if let Some(project_id) = self.session.as_ref().map(|s| s.project_id().to_owned()) {
            if let Some(project) = self
                .projects
                .iter()
                .flatten()
                .find(|p| p.id == project_id)
                .cloned()
            {
                let refreshed = self.session_of(&project);
                self.blocks.update_policy(refreshed.policy.clone());
                self.session = Some(refreshed);
            }
        }
    }

    // ---- Tracking on or off ------------------------------------------------------

    fn revoked_here(&self) -> bool {
        self.revoked.is_some()
            && self
                .account
                .as_ref()
                .and_then(|a| a.workspace_id.as_deref())
                == self.revoked.as_deref()
    }

    fn should_track(&self) -> bool {
        self.consent.is_granted()
            && !self.consent_unsynced
            && self.account.is_some()
            && !self.expired
            && !self.revoked_here()
            && self.session_summary.kind.is_graphical()
            && self.session.is_some()
            && self.pause.is_none()
            && self.hold.is_none()
            && self.away.is_none()
            && !self.locked
            && !self.disconnected
            && self.suspended.is_none()
            && self.shutting_down.is_none()
    }

    /// The effective permissions now: consented and permitted by the session
    /// and its policy. Absent, revoked or unsynced consent: nothing.
    fn current_effective_perms(&self) -> ConsentedPermissions {
        if !self.consent.is_granted() || self.consent_unsynced {
            return ConsentedPermissions::all_disabled();
        }
        self.session
            .as_ref()
            .map(|s| s.policy.perms)
            .unwrap_or_else(ConsentedPermissions::all_disabled)
    }

    /// Start or stop the stretch to match the facts; a stop ends the block
    /// at `at` by `reason`.
    fn reconcile(&mut self, at: DateTime<Utc>, reason: EndReason) {
        let now = self.now();
        if self.should_track() {
            if self.blocks.is_open() {
                return;
            }
            let Some(session) = self.session.as_ref() else {
                return;
            };
            let (policy, title) = (session.policy.clone(), session.title.clone());
            let fraction = self.machine.random_fraction();
            self.blocks
                .start(at.min(now), policy, self.task_note.clone(), fraction);
            if std::mem::take(&mut self.announce_autostart) {
                self.state.update(|s| {
                    s.push_notice(
                        NoticeKind::TrackingStartedAutomatically {
                            project_title: title,
                        },
                        now,
                    )
                });
            }
        } else if self.blocks.is_open() {
            let finished = self.blocks.end(at, now, reason);
            self.record(finished, Speed::for_reason(reason));
            // Manual mode: nothing tells when the user is back, so tracking
            // waits for them to resume.
            if self.manual_mode()
                && matches!(
                    reason,
                    EndReason::Lock | EndReason::Sleep | EndReason::Shutdown
                )
            {
                self.hold.get_or_insert(at);
            }
        }
    }

    // ---- Recording -----------------------------------------------------------------

    /// Build the finished blocks and queue them locally.
    fn record(&mut self, finished: Vec<FinishedBlock>, speed: Speed) {
        let tagged = self
            .account
            .as_ref()
            .and_then(|a| Some((a.owner(), a.workspace_id.clone()?)));
        let Some((owner, workspace)) = tagged.filter(|_| !finished.is_empty()) else {
            self.prune_screens();
            return;
        };
        let now = self.now();
        let current_perms = self.current_effective_perms();
        let mut queued = 0;
        for block in finished {
            let taken = self
                .take_screen(block.id)
                .filter(|capture| capture.capture.captured_at <= block.ended_at + CAPTURE_SLACK);
            // Ended before its screenshot moment: taken at the end instead —
            // unless the session is locked, asleep, idle or shutting down,
            // or screenshot permission has been withdrawn.
            let capture_now = taken.is_none()
                && speed == Speed::Normal
                && block.capture_at_end
                && block.policy.perms.screenshot
                && current_perms.screenshot
                && block.end_reason.allows_capture_at_end();
            let screen = if capture_now {
                self.machine.capture_screen(
                    block.policy.perms.blur_screenshots || current_perms.blur_screenshots,
                )
            } else if current_perms.screenshot {
                taken
            } else {
                None
            };
            let mut perms = block.policy.perms;
            perms.webcam = false;
            if !current_perms.running_programs {
                perms.running_programs = false;
            }
            let spec = BlockSpec {
                project_id: &block.policy.project_id,
                perms: &perms,
                started_at: block.started_at,
                ended_at: block.ended_at,
                end_reason: Some(block.end_reason),
                description: block.description.as_deref(),
            };
            let built = self.machine.snapshot(&spec, screen);
            if self.enqueue(built, &owner, &workspace, now) {
                queued += 1;
            }
        }
        self.prune_screens();
        self.enforce_queue_limits();
        publish_queue(&self.store, &self.state, Some(&owner));
        // Before sleep or shutdown nothing goes over the network.
        if queued > 0 && speed == Speed::Normal {
            self.uploader.send(UploadCommand::RowsQueued);
        }
    }

    /// Queue one block and show it (and count it in the totals at once; the
    /// server's totals, refreshed after the upload, replace that estimate).
    fn enqueue(
        &mut self,
        built: BuiltBlock,
        owner: &str,
        workspace: &str,
        now: DateTime<Utc>,
    ) -> bool {
        let BuiltBlock { activity, screen } = built;
        let captures = screen
            .as_ref()
            .map(ScreenCapture::pending_captures)
            .unwrap_or_default();
        let queued = OutboxScope::new(owner, workspace).and_then(|scope| {
            let pending = PendingActivity::from_activity(&activity, captures)?;
            Ok((scope, pending))
        });
        let (scope, pending) = match queued {
            Ok(queued) => queued,
            Err(e) => {
                tracing::error!(error = %e, "failed to queue activity");
                self.state
                    .update(|s| s.last_error = Some(format!("Could not save locally: {e}")));
                return false;
            }
        };
        let key = self.account.as_ref().and_then(|a| a.signing_key);
        let row_id = match key {
            Some(key) => match self.store.enqueue_activity(&scope, &pending, &key) {
                Ok(EnqueueResult::Inserted(id) | EnqueueResult::Idempotent(id)) => id,
                Err(e) => {
                    tracing::error!(error = %e, "failed to queue activity");
                    self.state
                        .update(|s| s.last_error = Some(format!("Could not save locally: {e}")));
                    return false;
                }
            },
            None => {
                // The keyring is locked: the block waits in memory for the
                // signing key, and is shown as queued meanwhile.
                if self.unsigned.len() >= MAX_UNSIGNED_BLOCKS {
                    tracing::warn!("too many blocks wait for the keyring; dropping the oldest");
                    self.unsigned.remove(0);
                }
                self.unsigned.push(UnsignedBlock { scope, pending });
                0
            }
        };
        tracing::debug!(row_id, end_reason = ?activity.end_reason, "activity queued");
        let (today_start, week_start) = local_day_and_week_start(now, &Local);
        let project = self
            .projects
            .iter()
            .flatten()
            .find(|p| p.id == activity.project_id)
            .map(|p| p.title.clone())
            .unwrap_or_default();
        let thumbnail = screen
            .as_ref()
            .and_then(|s| s.thumb_jpeg.clone().map(|jpeg| (s.captured_at, jpeg)));
        let stretch = self.blocks.stretch();
        let (block_started_at, unsaved_since) = (
            stretch.map(|s| s.block.started_at),
            stretch.map(Stretch::unsaved_since),
        );
        let (started_at, ended_at) = (activity.started_at, activity.ended_at);
        let block = recent_block(row_id, project, &activity, screen.is_some());
        self.state.update(|s| {
            s.tracked_today_secs += tracked_secs_since(started_at, ended_at, today_start);
            s.tracked_week_secs += tracked_secs_since(started_at, ended_at, week_start);
            // Moved in the same update, so the live total never counts this
            // block twice.
            if s.unsaved_since.is_some() {
                s.block_started_at = block_started_at;
                s.unsaved_since = unsaved_since;
            }
            push_recent_block(&mut s.recent_blocks, block);
            if let Some((taken_at, jpeg)) = thumbnail {
                s.last_screenshot = Some(Arc::new(LastScreenshot { taken_at, jpeg }));
            }
        });
        true
    }

    /// The signing key is known again (keyring unlocked): queue the blocks
    /// that waited for it.
    fn flush_unsigned(&mut self) {
        let Some(key) = self.account.as_ref().and_then(|a| a.signing_key) else {
            return;
        };
        if self.unsigned.is_empty() {
            return;
        }
        let waiting = std::mem::take(&mut self.unsigned);
        let mut queued = 0;
        for block in waiting {
            match self
                .store
                .enqueue_activity(&block.scope, &block.pending, &key)
            {
                Ok(_) => queued += 1,
                Err(e) => {
                    tracing::error!(error = %e, "a block that waited for the keyring could not be queued")
                }
            }
        }
        tracing::info!(queued, "blocks that waited for the keyring were queued");
        let owner = self.account.as_ref().map(Account::owner);
        publish_queue(&self.store, &self.state, owner.as_deref());
        self.uploader.send(UploadCommand::RowsQueued);
    }

    fn take_screen(&mut self, id: BlockId) -> Option<ProvenanceScreenCapture> {
        let index = self.screens.iter().position(|(block, _)| *block == id)?;
        Some(self.screens.swap_remove(index).1)
    }

    /// Forget screenshots of blocks that are gone (dropped by an idle cut).
    fn prune_screens(&mut self) {
        let live = self
            .blocks
            .stretch()
            .map(Stretch::block_ids)
            .unwrap_or_default();
        self.screens.retain(|(id, _)| live.contains(id));
        self.captures_in_flight.retain(|id| live.contains(id));
    }

    /// The block's screenshot moment: ask `dosi-capture` for one.
    fn capture(&mut self, id: BlockId) {
        let current_perms = self.current_effective_perms();
        if current_perms.screenshot {
            let blur = current_perms.blur_screenshots
                || self
                    .blocks
                    .stretch()
                    .is_some_and(|s| s.block.policy.perms.blur_screenshots);
            if self.machine.request_capture(id, blur) {
                self.captures_in_flight.push(id);
            }
        }
        self.blocks.mark_captured(id);
    }

    /// A screenshot came back. It is kept only if it was still expected
    /// (consent not narrowed meanwhile) and its block is still open or held.
    pub(crate) fn on_capture(&mut self, event: CaptureEvent) {
        let Some(index) = self
            .captures_in_flight
            .iter()
            .position(|id| *id == event.block_id)
        else {
            tracing::debug!(
                block = event.block_id,
                "a screenshot nobody waits for was discarded"
            );
            return;
        };
        self.captures_in_flight.swap_remove(index);
        let live = self
            .blocks
            .stretch()
            .is_some_and(|s| s.block_ids().contains(&event.block_id));
        match event.capture {
            Some(capture) if live && self.current_effective_perms().screenshot => {
                self.screens.push((event.block_id, capture));
            }
            Some(_) => tracing::debug!("a screenshot of a finished block was discarded"),
            None => {}
        }
    }

    // ---- The clock ---------------------------------------------------------------------

    pub(crate) fn on_tick(&mut self) {
        let now = self.now();
        self.refresh_platform();
        if let Some(before_gap) = self.sleep.observe(now, self.machine.awake_millis()) {
            tracing::info!(%before_gap, "the computer slept without announcing it");
            self.missed_suspend(before_gap, now);
        }
        // Input after a sleep or shutdown was announced proves the machine is
        // awake and the session goes on (the resume or the cancellation was
        // never heard): tracking must not stay off for good.
        if let Some(announced) = self.suspended.or(self.shutting_down) {
            if let Some(back) = self.machine.first_input_after(announced, now) {
                tracing::warn!(%announced, "input after an announced sleep or shutdown; tracking again");
                self.suspended = None;
                self.shutting_down = None;
                self.back_at_computer(back);
            }
        }
        self.check_return(now);
        self.check_break_end(now);
        self.check_manual_limit(now);
        if let Some(id) = self.blocks.capture_due(now) {
            self.capture(id);
        }
        let last_input = self.machine.last_input(now);
        let machine = self.machine.clone();
        let outcome = self
            .blocks
            .tick(now, last_input, &mut || machine.random_fraction());
        self.record(outcome.finished, Speed::Normal);
        if let Some(since) = outcome.went_idle {
            self.went_away(since);
        }
        self.housekeeping();
    }

    /// The desktop's answers may change (the portal was refused, the idle
    /// source came back): the next blocks follow them.
    fn refresh_platform(&mut self) {
        let platform = self.machine.platform();
        let summary = self.machine.session_summary();
        if platform != self.platform {
            self.availability = platform.availability();
            self.platform = platform;
            self.refresh_session();
        }
        self.session_summary = summary;
    }

    /// Manual mode: tracking pauses after `tracking.manual_max_hours` of one
    /// uninterrupted stretch, since nothing can tell the user left.
    fn check_manual_limit(&mut self, now: DateTime<Utc>) {
        if !self.manual_mode() {
            return;
        }
        let hours = self.cfg.tracking.manual_max_hours.max(1);
        let limit = TimeDelta::hours(i64::from(hours));
        let Some(since) = self.blocks.stretch().map(|s| s.since) else {
            return;
        };
        if now - since < limit {
            return;
        }
        tracing::info!(
            hours,
            "manual mode: tracking ran for the maximum time; pausing"
        );
        let at = since + limit;
        self.pause = Some(SavedPause {
            since: at,
            until: None,
        });
        self.reconcile(at, EndReason::Pause);
        self.persist_pause();
        self.state
            .update(|s| s.push_notice(NoticeKind::ManualModePaused { hours }, now));
    }

    /// The wall clock jumped ahead of the awake clock: the machine slept and
    /// nobody said so. The block ends at the last tick before the gap.
    fn missed_suspend(&mut self, before_gap: DateTime<Utc>, now: DateTime<Utc>) {
        self.awake_since = Some(now);
        self.freeze_away(before_gap);
        if self.blocks.stretch().is_some_and(|s| s.since < before_gap) {
            let finished = self.blocks.end(before_gap, now, EndReason::Sleep);
            self.record(finished, Speed::Fast);
            self.uploader.send(UploadCommand::RowsQueued);
            if self.manual_mode() {
                self.hold.get_or_insert(before_gap);
            }
            // A new block from now on.
            self.reconcile(now, EndReason::Sleep);
        }
    }

    /// No input for the idle threshold: the block ended at the last input.
    fn went_away(&mut self, since: DateTime<Utc>) {
        tracing::info!(%since, "no input for the idle threshold; away");
        // An earlier question nobody answered: no answer means discard.
        self.prompt = None;
        self.away = Some(Away {
            since,
            until: None,
            after_break: false,
            session: self.session.clone(),
        });
    }

    fn present(&self) -> bool {
        !(self.locked
            || self.disconnected
            || self.suspended.is_some()
            || self.shutting_down.is_some())
    }

    fn check_return(&mut self, now: DateTime<Utc>) {
        let Some(since) = self.away.as_ref().map(|away| away.since) else {
            return;
        };
        if !self.present() {
            return;
        }
        if let Some(back) = self.machine.first_input_after(since, now) {
            self.come_back(back);
        }
    }

    /// The user is back at `at`: a new block starts there, and — unless they
    /// were on a break — they are asked about the time away.
    fn come_back(&mut self, at: DateTime<Utc>) {
        let Some(away) = self.away.take() else {
            return;
        };
        let at = at.min(self.now()).max(away.since);
        if let (false, Some(session)) = (away.after_break, away.session) {
            let to = away.until.unwrap_or(at);
            if to - away.since >= MIN_BLOCK {
                self.prompt = Some(PendingPrompt {
                    from: away.since,
                    to,
                    session,
                });
            }
        }
        tracing::info!(%at, "back at the computer");
        self.reconcile(at, EndReason::Interval);
    }

    /// The session locked or the machine slept while away: that time is
    /// never tracked, so the time away ends there.
    fn freeze_away(&mut self, at: DateTime<Utc>) {
        if let Some(away) = self.away.as_mut() {
            away.until.get_or_insert(at.max(away.since));
        }
    }

    /// A timed break ran out: tracking resumes where it ended — or, when
    /// nobody is at the computer, with the next input (CONTRACT §4). In
    /// manual mode it waits for the user to resume.
    fn check_break_end(&mut self, now: DateTime<Utc>) {
        let Some(pause) = self.pause else {
            return;
        };
        let Some(until) = pause.until.filter(|until| *until <= now) else {
            return;
        };
        self.pause = None;
        self.persist_pause();
        if self.manual_mode() {
            tracing::info!("the break is over; manual mode waits for the user to resume");
            self.hold.get_or_insert(until);
            return;
        }
        let threshold = self.session.as_ref().and_then(|s| s.policy.idle_threshold);
        let last = self
            .machine
            .last_input(now)
            .map_or(pause.since, |last| last.max(pause.since));
        if self.session.is_some() && threshold.is_some_and(|threshold| now - last >= threshold) {
            tracing::info!(
                "the break is over, but nobody is at the computer; resuming with the next input"
            );
            self.away = Some(Away {
                since: last,
                until: None,
                after_break: true,
                session: None,
            });
        } else {
            // Not inside a sleep (or a lock) that outlasted the break: the
            // time before the machine was last awake again is not work.
            let resume = self
                .awake_since
                .map_or(until, |awake| until.max(awake))
                .min(now);
            tracing::info!("timed pause is over; resuming tracking");
            self.reconcile(resume, EndReason::Interval);
        }
    }

    /// What is due: the queue's housekeeping, refreshing the projects, the
    /// totals and the device's status.
    fn housekeeping(&mut self) {
        let now = Instant::now();
        if now >= self.next_maintenance {
            self.next_maintenance = now + MAINTENANCE_EVERY;
            let owner = self.account.as_ref().map(Account::owner);
            self.maintain_queue(owner.as_deref());
            publish_queue(&self.store, &self.state, owner.as_deref());
        }
        let in_workspace = self
            .account
            .as_ref()
            .is_some_and(|a| a.workspace_id.is_some());
        if !in_workspace
            || self.expired
            || self.credentials_locked
            || self.suspended.is_some()
            || self.shutting_down.is_some()
        {
            return;
        }
        if now >= self.next_device_status {
            self.next_device_status = now + DEVICE_STATUS_EVERY;
            self.control.send(ControlRequest::DeviceStatus);
        }
        if self.revoked_here() {
            return;
        }
        if now >= self.next_projects_refresh {
            self.control.send(ControlRequest::RefreshProjects);
            let online = self.reachable && !self.contacting;
            self.next_projects_refresh = now
                + if online {
                    PROJECTS_REFRESH
                } else {
                    RECONNECT_EVERY
                };
        }
        if now >= self.totals_due_at {
            self.totals_wanted = true;
        }
        if self.totals_wanted && now >= self.next_totals {
            self.control.send(ControlRequest::RefreshTotals);
            self.totals_wanted = false;
            self.next_totals = now + TOTALS_SPACING;
            self.totals_due_at = now + TOTALS_EVERY;
        }
    }

    // ---- Session and power events ------------------------------------------------------

    /// Handle a session or power event; `true` when the session is gone and
    /// the worker must quit (design 4, step 5).
    pub(crate) fn on_system(&mut self, mut notice: SystemNotice) -> bool {
        let at = notice.at;
        let mut quit = false;
        match notice.event {
            SystemEvent::Locked | SystemEvent::Disconnected => {
                if notice.event == SystemEvent::Locked {
                    self.locked = true;
                } else {
                    self.disconnected = true;
                }
                self.freeze_away(at);
                self.reconcile(at, EndReason::Lock);
            }
            SystemEvent::Unlocked | SystemEvent::Connected => {
                if notice.event == SystemEvent::Unlocked {
                    self.locked = false;
                } else {
                    self.disconnected = false;
                }
                self.back_at_computer(at);
            }
            SystemEvent::Suspending => {
                self.suspended.get_or_insert(at);
                self.freeze_away(at);
                self.reconcile(at, EndReason::Sleep);
            }
            SystemEvent::Resumed => {
                if self.suspended.take().is_some() {
                    // Handled: the tick after the wake must not report it again.
                    self.sleep.reset();
                    self.back_at_computer(at);
                    self.uploader.send(UploadCommand::RowsQueued);
                }
            }
            SystemEvent::SessionEnding | SystemEvent::SessionRemoved => {
                if self.shutting_down.is_none() {
                    self.shutting_down = Some(at);
                    self.freeze_away(at);
                    self.reconcile(at, EndReason::Shutdown);
                }
                quit = notice.event == SystemEvent::SessionRemoved;
            }
            SystemEvent::SessionContinues => {
                if self.shutting_down.take().is_some() {
                    self.back_at_computer(at);
                }
            }
        }
        // The block is saved: logind may go on.
        notice.acknowledge();
        quit
    }

    /// Unlocked, reconnected, awake again: someone is at the computer (the
    /// password typed on the lock screen is input the listener never sees).
    fn back_at_computer(&mut self, at: DateTime<Utc>) {
        self.awake_since = Some(at);
        if self.away.is_some() && self.present() {
            self.come_back(at);
        } else {
            self.reconcile(at, EndReason::Interval);
        }
    }

    // ---- The clients' commands -------------------------------------------------------------

    pub(crate) fn on_command(&mut self, command: Command) {
        // The user is here: tracking starting later is no surprise.
        self.announce_autostart = false;
        match command {
            Command::SignIn {
                workspace,
                username,
                password,
                server,
            } => self.sign_in(workspace, username, password, server),
            Command::ChooseWorkspace { index } => self.choose_workspace(index),
            Command::CancelWorkspaceChoice => self.clear_choices(),
            Command::SwitchWorkspace => self.switch_workspace(),
            Command::SwitchToWorkspace { index, at } => {
                self.user_acted(at);
                self.switch_to(index, at);
            }
            Command::SignOut { at } => self.sign_out(at),
            Command::SelectProject { project_id, at } => {
                self.user_acted(at);
                self.select_project(&project_id, at);
            }
            Command::Pause { resume_after, at } => {
                self.user_acted(at);
                self.pause(resume_after, at);
            }
            Command::Resume { at } => {
                self.user_acted(at);
                self.resume(at);
            }
            Command::SyncNow { at } => {
                self.user_acted(at);
                self.sync_now(at);
            }
            Command::SetTaskNote { note, at } => {
                self.user_acted(at);
                self.set_task_note(&note, at);
            }
            Command::ResolveAway { keep } => self.resolve_away(keep),
            Command::Shutdown { .. } => {}
            Command::UpdateConsent { consent } => self.update_consent(consent),
            Command::ReloadCredentials => self.reload_credentials(),
            Command::RetryCapture => {
                self.machine.retry_capture_permission();
                self.refresh_platform();
            }
        }
        self.state.update(|s| s.pending_action = None);
    }

    /// The keyring was unlocked: read the sign-in's secrets again.
    fn reload_credentials(&mut self) {
        if !self.credentials_locked || self.account.is_none() {
            return;
        }
        let now = self.now();
        self.state.update(|s| s.last_error = None);
        self.load_secrets(now);
    }

    /// Update local consent: what is withdrawn stops before anything is
    /// saved; what is added starts only once it is saved; a failed save
    /// stops everything (fail closed) and says so.
    fn update_consent(&mut self, consent: LocalConsent) {
        tracing::info!(?consent, "local consent update requested");
        if let LocalConsent::Granted(granted) = consent {
            if granted.webcam {
                self.consent_error = Some(WEBCAM_UNAVAILABLE.into());
                return;
            }
        }
        let now = self.now();
        let prior = self.consent;

        // Step 1: withdraw at once (per capability), grant nothing new yet.
        self.consent = narrowed(prior, consent);
        if let Some(project_id) = self.session.as_ref().map(|s| s.project_id().to_owned()) {
            if let Some(project) = self
                .projects
                .iter()
                .flatten()
                .find(|p| p.id == project_id)
                .cloned()
            {
                let refreshed = self.session_of(&project);
                self.blocks
                    .narrow_block_permissions(&refreshed.policy.perms);
                self.session = Some(refreshed);
            }
        }
        // Quiesce the capture path: a screenshot taken or still in flight is
        // dropped when screenshots were withdrawn.
        if !self.current_effective_perms().screenshot {
            self.screens.clear();
            self.captures_in_flight.clear();
        }
        // The sensors follow the narrowed policy before anything is recorded
        // or saved.
        self.apply_policy_and_reconcile(now);

        // Step 2: persist (may be slow, may fail), then widen.
        let saved = self.lifecycle().update_consent(consent);
        match saved {
            Ok(()) => {
                self.consent = consent;
                self.consent_error = None;
                self.consent_unsynced = false;
                tracing::info!("consent saved");
                if self.session.is_some() {
                    self.refresh_session();
                } else {
                    self.choose_session(now);
                }
                self.apply_policy_and_reconcile(now);
            }
            Err(e) => {
                let err_msg = format!(
                    "Failed to save the consent change: {e}. Tracking stopped (fail-closed) to prevent non-durable sensor operation. The change is non-durable across app restart: the consent saved before remains on disk and will reload on restart until a save succeeds."
                );
                tracing::error!(%err_msg);
                self.consent_error = Some(err_msg);
                self.consent_unsynced = true;
                self.consent = match prior {
                    LocalConsent::Absent => LocalConsent::Absent,
                    _ => LocalConsent::Revoked,
                };
                self.refresh_session();
                self.machine.apply_sensor_policy(false, None);
                self.reconcile(now, EndReason::Pause);
                self.screens.clear();
                self.captures_in_flight.clear();
            }
        }
        self.publish();
    }

    fn apply_policy_and_reconcile(&mut self, now: DateTime<Utc>) {
        if self.should_track() {
            if let Some(session) = self.session.as_ref() {
                self.machine
                    .apply_sensor_policy(true, Some(&session.policy.perms));
            }
            self.reconcile(now, EndReason::Interval);
        } else {
            self.machine.apply_sensor_policy(false, None);
            self.reconcile(now, EndReason::Pause);
        }
    }

    /// A click in a client is input too: an away stretch ends there.
    fn user_acted(&mut self, at: DateTime<Utc>) {
        if self.away.is_some() && self.present() {
            self.come_back(at);
        }
    }

    fn sign_in(
        &mut self,
        workspace: Option<String>,
        username: String,
        password: Secret,
        server: Option<String>,
    ) {
        if self.account.is_some() && !self.expired {
            tracing::warn!("sign-in ignored: already signed in");
            return;
        }
        let typed = match server
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(|s| validated_server_url(s, self.cfg.allow_insecure_http))
            .transpose()
        {
            Ok(typed) => typed,
            Err(e) => {
                self.state.update(|s| s.login_error = Some(e.to_string()));
                return;
            }
        };
        // The same user signing in again keeps the server and workspace saved.
        let same = self
            .saved
            .as_ref()
            .filter(|saved| saved.account.as_deref().map(owner_key) == Some(owner_key(&username)));
        let saved_server = same.and_then(|saved| saved.server.clone());
        let saved_workspace = same.and_then(|saved| saved.workspace.as_ref().map(|w| w.id.clone()));
        let Some(server) =
            typed.or_else(|| self.cfg.effective_api_base_url(saved_server.as_deref()))
        else {
            self.state
                .update(|s| s.login_error = Some(NO_SERVER.into()));
            return;
        };
        self.clear_choices();
        self.preselect = workspace
            .map(|w| w.trim().to_owned())
            .filter(|w| !w.is_empty())
            .or(saved_workspace);
        self.signing_in = true;
        self.state.update(|s| {
            s.signing_in = true;
            s.login_error = None;
        });
        let account = self.new_account(server, username.trim().to_owned(), password.expose());
        self.control.send(ControlRequest::SignIn(account));
    }

    /// The server accepted the sign-in: pick the workspace to track in.
    fn signed_in_to_server(
        &mut self,
        account: Account,
        workspaces: Result<Vec<Workspace>, ApiError>,
    ) {
        if !self.signing_in {
            return;
        }
        self.signing_in = false;
        let preselect = self.preselect.take();
        let workspaces = match workspaces {
            Ok(workspaces) => workspaces,
            Err(error) => {
                let message = match &error {
                    ApiError::Auth(e) => auth_message(e),
                    ApiError::Offline(_) => auth_message(&AuthError::Offline(String::new())),
                    other => {
                        format!("Signed in, but your workspaces could not be loaded ({other}).")
                    }
                };
                return self.state.update(|s| {
                    s.signing_in = false;
                    s.login_error = Some(message);
                });
            }
        };
        let named = |wanted: &str| {
            workspaces
                .iter()
                .find(|w| w.id.eq_ignore_ascii_case(wanted))
                .or_else(|| {
                    workspaces
                        .iter()
                        .find(|w| w.name.trim().eq_ignore_ascii_case(wanted))
                })
                .cloned()
        };
        let preselected = preselect.as_deref().and_then(named);
        if preselected.as_ref().is_some_and(|w| !w.is_selectable()) {
            return self.state.update(|s| {
                s.signing_in = false;
                s.login_error = Some(ARCHIVED_WORKSPACE.to_owned());
            });
        }
        let selectable: Vec<Workspace> = workspaces
            .into_iter()
            .filter(Workspace::is_selectable)
            .collect();
        let chosen = preselected.or_else(|| match selectable.as_slice() {
            [only] => Some(only.clone()),
            _ => None,
        });
        match (chosen, selectable.as_slice()) {
            (None, []) => self.state.update(|s| {
                s.signing_in = false;
                s.login_error = Some(NO_WORKSPACE.to_owned());
            }),
            (Some(workspace), _) => {
                let account = account.in_workspace(&workspace.id, &workspace.name);
                self.state.update(|s| s.signing_in = false);
                self.signed_in(account);
            }
            (None, _) => {
                self.offer(&selectable, None);
                self.choices = selectable;
                self.pending = Some(account);
                self.state.update(|s| s.signing_in = false);
            }
        }
    }

    /// Show `workspaces` in the picker.
    fn offer(&self, workspaces: &[Workspace], current: Option<&str>) {
        let choices: Vec<WorkspaceChoice> = workspaces
            .iter()
            .map(|w| WorkspaceChoice {
                name: w.name.clone(),
                role: w.role.clone(),
                is_current: current == Some(w.id.as_str()),
            })
            .collect();
        self.state.update(|s| s.workspace_choices = choices);
    }

    /// The workspace picked after signing in.
    fn choose_workspace(&mut self, index: usize) {
        let (Some(workspace), Some(account)) =
            (self.choices.get(index).cloned(), self.pending.take())
        else {
            return;
        };
        self.clear_choices();
        self.signed_in(account.in_workspace(&workspace.id, &workspace.name));
    }

    fn clear_choices(&mut self) {
        self.pending = None;
        self.choices.clear();
        self.state.update(|s| {
            s.workspace_choices.clear();
            s.finding_workspaces = false;
            s.workspace_note = None;
        });
    }

    /// "Switch workspace": list the workspaces of the signed-in user.
    fn switch_workspace(&mut self) {
        if self.account.is_none() || self.expired || self.credentials_locked {
            return;
        }
        self.clear_choices();
        self.state.update(|s| s.finding_workspaces = true);
        self.control.send(ControlRequest::MyWorkspaces);
    }

    fn my_workspaces_found(&mut self, found: Result<Vec<Workspace>, String>) {
        self.state.update(|s| s.finding_workspaces = false);
        let Some(current) = self
            .account
            .as_ref()
            .filter(|_| !self.expired)
            .map(|a| a.workspace_id.clone())
        else {
            return;
        };
        let others = |items: &[Workspace]| {
            items
                .iter()
                .filter(|w| current.as_deref() != Some(w.id.as_str()))
                .count()
        };
        let items: Vec<Workspace> = match found {
            Err(message) => return self.state.update(|s| s.workspace_note = Some(message)),
            Ok(items) => items.into_iter().filter(Workspace::is_selectable).collect(),
        };
        if items.is_empty() {
            return self
                .state
                .update(|s| s.workspace_note = Some(NO_WORKSPACE.to_owned()));
        }
        if others(&items) == 0 {
            return self
                .state
                .update(|s| s.workspace_note = Some(ONE_WORKSPACE.to_owned()));
        }
        self.offer(&items, current.as_deref());
        self.choices = items;
    }

    /// Track in `workspace_choices[index]` from now on: the block so far is
    /// queued in the workspace it was recorded in, and the project is chosen
    /// again. The sign-in (and its token) stays.
    fn switch_to(&mut self, index: usize, at: DateTime<Utc>) {
        let Some(old) = self.account.clone().filter(|_| !self.expired) else {
            return;
        };
        let Some(target) = self.choices.get(index).cloned() else {
            return;
        };
        if old.workspace_id.as_deref() == Some(target.id.as_str()) {
            return;
        }
        let now = self.now();
        let finished = self.blocks.end(at.min(now), now, EndReason::SignOut);
        self.record(finished, Speed::Normal);
        self.clear_choices();
        let was_revoked = self.revoked.take().is_some();
        self.use_workspace(old.in_workspace(&target.id, &target.name), now);
        if was_revoked {
            // The revocation cleared the saved secrets: keep the sign-in for
            // the new workspace.
            self.save_secrets(None);
        }
    }

    /// Carry on in `account`'s workspace: what belonged to the previous one
    /// (projects, the chosen project, the question about time away) is dropped.
    fn use_workspace(&mut self, account: Account, now: DateTime<Utc>) {
        if let Some(saved) = self.saved.as_mut() {
            saved.workspace = account.workspace_id.clone().map(|id| SavedWorkspace {
                id,
                name: account.workspace_name.clone(),
            });
            saved.project_id = None;
            saved.projects = None;
        }
        self.persist();
        self.projects = None;
        self.session = None;
        self.choose_notified = false;
        self.away = None;
        self.prompt = None;
        self.presence_sent = None;
        self.contacting = account.workspace_id.is_some();
        let (name, lost) = (
            account.workspace_name.clone(),
            account.workspace_id.is_none(),
        );
        self.state.update(|s| {
            s.workspace = if lost { String::new() } else { name };
            s.projects.clear();
            s.selected_project = None;
            s.tracked_today_secs = 0.0;
            s.tracked_week_secs = 0.0;
            s.week_day_secs = [0.0; 7];
            s.week_days_fetched = None;
            s.last_productivity = None;
            s.capture = CaptureFlags::default();
            s.last_error = None;
            if !lost {
                s.workspace_lost = None;
            }
        });
        self.account = Some(account.clone());
        self.control
            .send(ControlRequest::UseAccount(Some(account.clone())));
        self.uploader
            .send(UploadCommand::Account(Some(account.clone())));
        if account.workspace_id.is_some() {
            self.control.send(ControlRequest::RefreshProjects);
            self.control.send(ControlRequest::DeviceStatus);
            self.next_projects_refresh = Instant::now() + RECONNECT_EVERY;
            self.next_device_status = Instant::now() + DEVICE_STATUS_EVERY;
            self.totals_wanted = true;
        }
        self.choose_session(now);
        self.reconcile(now, EndReason::Interval);
    }

    /// The account lost access to the workspace it tracks in (removed, the
    /// workspace archived, or no valid workspace): tracking stops (what was
    /// recorded is parked by the uploader) and another workspace is asked for.
    fn workspace_lost(&mut self, workspace_id: &str) {
        let Some(account) = self.account.clone().filter(|_| !self.expired) else {
            return;
        };
        if account.workspace_id.as_deref() != Some(workspace_id) {
            return;
        }
        let now = self.now();
        let name = if account.workspace_name.trim().is_empty() {
            "this workspace".to_owned()
        } else {
            account.workspace_name.clone()
        };
        tracing::warn!(%workspace_id, "no longer a member of the workspace; tracking stops");
        let finished = self.blocks.end(now, now, EndReason::SignOut);
        self.record(finished, Speed::Normal);
        self.uploader.send(UploadCommand::RowsQueued);
        let mut account = account;
        account.workspace_id = None;
        self.use_workspace(account, now);
        let note = format!("You no longer have access to {name}.");
        self.state.update(|s| {
            s.workspace_lost = Some(name.clone());
            s.workspace_note = Some(note);
            s.finding_workspaces = true;
            s.push_notice(NoticeKind::WorkspaceLost { name }, now);
        });
        self.control.send(ControlRequest::MyWorkspaces);
    }

    /// The workspace revoked this device: tracking and heartbeats stop there,
    /// its queued rows are parked, the saved secrets are cleared (a restart
    /// asks for the sign-in again); the user can switch workspace or sign out.
    fn device_revoked(&mut self, workspace_id: &str) {
        let Some(account) = self.account.clone() else {
            return;
        };
        if account.workspace_id.as_deref() != Some(workspace_id) || self.revoked_here() {
            return;
        }
        let now = self.now();
        tracing::warn!(%workspace_id, "this device was revoked in the workspace; tracking stops there");
        let finished = self.blocks.end(now, now, EndReason::SignOut);
        self.record(finished, Speed::Normal);
        self.revoked = Some(workspace_id.to_owned());
        self.uploader.send(UploadCommand::ParkScope {
            workspace_id: workspace_id.to_owned(),
            reason: "device revoked".into(),
        });
        if let Err(e) = self.credentials.clear_account(&account.owner()) {
            tracing::warn!(error = %e, "could not clear the saved sign-in of a revoked device");
        }
        let workspace = if account.workspace_name.trim().is_empty() {
            "this workspace".to_owned()
        } else {
            account.workspace_name.clone()
        };
        self.presence_sent = None;
        self.state
            .update(|s| s.push_notice(NoticeKind::DeviceRevoked { workspace }, now));
    }

    /// A workspace was chosen after signing in. The same user in the same
    /// workspace (the password entered again after it expired) keeps
    /// everything it remembered; otherwise it starts afresh.
    fn signed_in(&mut self, account: Account) {
        let now = self.now();
        self.signing_in = false;
        let owner = account.owner();
        let same = self.saved.as_ref().is_some_and(|saved| {
            saved.account.as_deref().map(owner_key) == Some(owner.clone())
                && saved.workspace.as_ref().map(|w| w.id.clone()) == account.workspace_id
        });
        if !same && self.account.is_some() {
            self.forget_account(now);
        }
        let mut saved = match self.saved.take() {
            Some(saved) if same => saved,
            _ => SessionData::default(),
        };
        saved.account = Some(owner.clone());
        saved.workspace = account.workspace_id.clone().map(|id| SavedWorkspace {
            id,
            name: account.workspace_name.clone(),
        });
        saved.server = Some(account.server.clone());
        if !same {
            self.projects = None;
        }
        self.saved = Some(saved);
        self.persist();

        // The signing key of this account, kept across sign-ins, or a new one.
        let previous = self.credentials.load_account(&owner).ok().flatten();
        let password = account.password.clone();
        let mut account = account.without_password();
        account.signing_key = previous
            .as_ref()
            .and_then(signing_key_of)
            .or_else(|| Some(crate::signing::generate_signing_key()));
        self.persister.watch(&account.tokens, &owner);
        self.account = Some(account.clone());
        self.save_secrets(Some(password.expose()));

        if !same {
            self.maintain_queue(Some(&owner));
        }
        publish_queue(&self.store, &self.state, Some(&owner));
        let (name, workspace, server) = (
            account.username.clone(),
            account.workspace_name.clone(),
            account.server.clone(),
        );
        self.state.update(|s| {
            s.display_name = name;
            s.workspace = workspace;
            s.server_url = server;
            s.signing_in = false;
            s.login_error = None;
            s.workspace_lost = None;
        });
        self.expired = false;
        self.credentials_locked = false;
        self.revoked = None;
        // Say where this device stands at once.
        self.presence_sent = None;
        self.set_reachable(true);
        self.flush_unsigned();
        self.connect_account();
        self.choose_session(now);
        self.reconcile(now, EndReason::Interval);
    }

    fn sign_in_failed(&mut self, error: &AuthError) {
        self.signing_in = false;
        if error.is_rejected() {
            tracing::info!(%error, "the server refused the sign-in");
        } else {
            tracing::warn!(%error, "could not sign in");
        }
        let message = auth_message(error);
        self.state.update(|s| {
            s.signing_in = false;
            s.login_error = Some(message);
        });
    }

    /// The server refused the saved sign-in (password changed, account
    /// deactivated, refresh token revoked): tracking stops, the queue is kept
    /// for when the password is entered again (CONTRACT §1).
    fn expire(&mut self, reason: &str) {
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

    fn sign_out(&mut self, at: DateTime<Utc>) {
        let now = self.now();
        if let Some(account) = self.account.clone() {
            // The work so far is this account's: queue it (and upload it)
            // before letting go.
            let finished = self.blocks.end(at, now, EndReason::SignOut);
            self.record(finished, Speed::Normal);
            let online = !self.expired && !self.credentials_locked && !self.revoked_here();
            let heartbeat = if online {
                self.heartbeat(PresenceStatus::SignedOut, now, now)
            } else {
                None
            };
            self.control.farewell_later(heartbeat);
            self.uploader.send(UploadCommand::SignOut {
                deadline: std::time::Instant::now() + SIGN_OUT_UPLOAD_GRACE,
            });
            if let Err(e) = self.credentials.clear_account(&account.owner()) {
                tracing::warn!(error = %e, "could not forget the saved sign-in");
            }
        }
        let signed_out = self.saved.as_ref().map(SessionData::signed_out);
        self.forget_account(now);
        if let Some(signed_out) = signed_out {
            self.saved = Some(signed_out);
            self.persist();
            self.saved = None;
        }
    }

    /// Let go of the account: nothing it had carries over to the next one.
    /// Its queued rows stay (tagged with it) until it signs in again.
    fn forget_account(&mut self, now: DateTime<Utc>) {
        let finished = self.blocks.end(now, now, EndReason::SignOut);
        self.record(finished, Speed::Normal);
        if !self.unsigned.is_empty() {
            tracing::warn!(
                blocks = self.unsigned.len(),
                "blocks that waited for the keyring are dropped with the sign-in"
            );
            self.unsigned.clear();
        }
        self.saved = None;
        self.account = None;
        self.expired = false;
        self.credentials_locked = false;
        self.revoked = None;
        self.contacting = false;
        self.projects = None;
        self.session = None;
        self.choose_notified = false;
        self.policy_notified.clear();
        self.task_note = None;
        self.pause = None;
        self.hold = None;
        self.away = None;
        self.prompt = None;
        self.screens.clear();
        self.captures_in_flight.clear();
        self.presence_sent = None;
        self.announce_autostart = false;
        self.pending = None;
        self.choices.clear();
        publish_queue(&self.store, &self.state, None);
        let server = self.cfg.api_base_url.clone().unwrap_or_default();
        self.state.update(|s| {
            s.display_name.clear();
            s.workspace.clear();
            s.server_url = server;
            s.projects.clear();
            s.selected_project = None;
            s.tracked_today_secs = 0.0;
            s.tracked_week_secs = 0.0;
            s.week_day_secs = [0.0; 7];
            s.week_days_fetched = None;
            s.last_productivity = None;
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

    fn select_project(&mut self, project_id: &str, at: DateTime<Utc>) {
        // Choosing what to track is an explicit start.
        let held = self.hold.take().is_some();
        if self
            .session
            .as_ref()
            .is_some_and(|s| s.project_id() == project_id)
        {
            if held {
                self.reconcile(at, EndReason::Interval);
            }
            return;
        }
        // The saved list works offline too.
        let Some(project) = self
            .projects
            .iter()
            .flatten()
            .find(|p| p.id == project_id)
            .cloned()
        else {
            tracing::warn!(project_id, "cannot switch to a project that is not listed");
            return;
        };
        let now = self.now();
        // The work so far belongs to the project it was done on.
        let finished = self.blocks.end(at, now, EndReason::Project);
        self.record(finished, Speed::Normal);
        self.select(&project);
        self.reconcile(at, EndReason::Project);
    }

    fn pause(&mut self, resume_after: Option<Duration>, at: DateTime<Utc>) {
        let until = resume_after
            .and_then(|d| TimeDelta::from_std(d).ok())
            .map(|d| at + d);
        match self.pause.as_mut() {
            // Already paused: only when (or whether) it ends changes.
            Some(pause) => pause.until = until,
            None => {
                self.away = None;
                self.hold = None;
                self.pause = Some(SavedPause { since: at, until });
                let reason = if until.is_some() {
                    EndReason::Break
                } else {
                    EndReason::Pause
                };
                self.reconcile(at, reason);
            }
        }
        self.persist_pause();
    }

    fn resume(&mut self, at: DateTime<Utc>) {
        let held = self.hold.take().is_some();
        if self.pause.take().is_some() {
            self.persist_pause();
            // Not a second of the pause (or input made during it) is billed:
            // a fresh block starts at the click.
            self.reconcile(at, EndReason::Interval);
        } else if held {
            self.reconcile(at, EndReason::Interval);
        }
    }

    /// Save the block in progress now (if it is a minute old) and upload at once.
    fn sync_now(&mut self, at: DateTime<Utc>) {
        let now = self.now();
        let old_enough = self
            .blocks
            .stretch()
            .is_some_and(|s| at.min(now) - s.block.started_at >= MIN_BLOCK);
        if old_enough {
            let fraction = self.machine.random_fraction();
            let finished = self.blocks.cut(at, now, EndReason::Interval, fraction);
            self.record(finished, Speed::Normal);
        }
        self.uploader.send(UploadCommand::Now);
        self.totals_wanted = true;
        self.next_totals = Instant::now();
    }

    /// A new task note: it is each block's description, so the block in
    /// progress ends with the old one (by `task`). A block under a minute old
    /// takes the new note instead, rather than being dropped as too short.
    fn set_task_note(&mut self, note: &str, at: DateTime<Utc>) {
        let note = clean_note(note);
        if note == self.task_note {
            return;
        }
        self.task_note = note.clone();
        if let Some(saved) = self.saved.as_mut() {
            saved.task_note = note.clone();
        }
        self.persist();
        let now = self.now();
        match self
            .blocks
            .stretch()
            .map(|s| at.min(now) - s.block.started_at < MIN_BLOCK)
        {
            Some(true) => self.blocks.set_description(note, true),
            Some(false) => {
                self.blocks.set_description(note, false);
                let fraction = self.machine.random_fraction();
                let finished = self.blocks.cut(at, now, EndReason::Task, fraction);
                self.record(finished, Speed::Normal);
            }
            None => {}
        }
    }

    /// "Keep as work": the time away is queued as idle blocks of at most one
    /// interval, on the project tracked then, with the current task note.
    fn resolve_away(&mut self, keep: bool) {
        let Some(prompt) = self.prompt.take() else {
            return;
        };
        if !keep {
            tracing::info!(
                minutes = (prompt.to - prompt.from).num_minutes(),
                "time away discarded"
            );
            return;
        }
        // Also while the sign-in is expired: the queue keeps the account's
        // rows for when the password is entered again.
        let Some((owner, workspace)) = self
            .account
            .as_ref()
            .and_then(|a| Some((a.owner(), a.workspace_id.clone()?)))
        else {
            tracing::info!("time away not kept: signed out");
            return;
        };
        let now = self.now();
        let interval = prompt
            .session
            .policy
            .interval
            .to_std()
            .unwrap_or(Duration::from_secs(600));
        let blocks = self.machine.kept_idle_blocks(
            prompt.session.project_id(),
            prompt.from,
            prompt.to,
            interval,
            self.task_note.as_deref(),
        );
        let mut queued = 0;
        for activity in blocks {
            let built = BuiltBlock {
                activity,
                screen: None,
            };
            if self.enqueue(built, &owner, &workspace, now) {
                queued += 1;
            }
        }
        self.enforce_queue_limits();
        publish_queue(&self.store, &self.state, Some(&owner));
        if queued > 0 {
            self.uploader.send(UploadCommand::RowsQueued);
        }
        tracing::info!(queued, "time away kept as work");
    }

    /// Quit: save the block in progress first — so whatever happens next it
    /// goes up on the next launch — then say "stopped" and upload what fits.
    pub(crate) async fn quit(&mut self, at: DateTime<Utc>) {
        // Halt every sensor before the farewell and the final uploads.
        self.machine.apply_sensor_policy(false, None);
        let started = std::time::Instant::now();
        let now = self.now();
        let finished = self.blocks.end(at, now, EndReason::Quit);
        self.record(finished, Speed::Normal);
        if !self.unsigned.is_empty() {
            tracing::warn!(
                blocks = self.unsigned.len(),
                "the keyring stayed locked: blocks that waited for it are lost"
            );
        }
        // An unanswered question about time away: no answer means discard.
        self.prompt = None;
        let online = self.account.is_some()
            && !self.expired
            && !self.credentials_locked
            && !self.revoked_here();
        let heartbeat = if online {
            self.heartbeat(PresenceStatus::Stopped, now, now)
        } else {
            None
        };
        self.control.farewell(heartbeat, FAREWELL_WAIT).await;
        let deadline = started + QUIT_UPLOAD_GRACE;
        self.uploader.send(UploadCommand::Shutdown { deadline });
        self.uploader
            .finish(deadline + Duration::from_secs(1))
            .await;
    }

    // ---- What the other parts report ------------------------------------------------------------

    pub(crate) fn on_control(&mut self, event: ControlEvent) {
        match event {
            ControlEvent::SignedIn {
                account,
                workspaces,
            } => self.signed_in_to_server(account, workspaces),
            ControlEvent::SignInFailed(error) => self.sign_in_failed(&error),
            ControlEvent::MyWorkspaces(found) => self.my_workspaces_found(found),
            ControlEvent::WorkspaceLost(workspace) => self.workspace_lost(&workspace),
            ControlEvent::DeviceRevoked(workspace) => self.device_revoked(&workspace),
            ControlEvent::Projects(projects) => {
                if self
                    .account
                    .as_ref()
                    .is_some_and(|a| a.workspace_id.is_some())
                    && !self.expired
                {
                    self.apply_projects(projects);
                }
            }
            ControlEvent::ProjectsFailed(error) => {
                self.contacting = false;
                if !error.is_auth_rejected() {
                    self.next_projects_refresh = Instant::now() + RECONNECT_EVERY;
                }
            }
            ControlEvent::SignInExpired(reason) => self.expire(&reason),
            ControlEvent::Reachable(reachable) => self.set_reachable(reachable),
        }
    }

    pub(crate) fn on_upload(&mut self, event: UploadEvent) {
        let now = self.now();
        match event {
            // A pass of the previous account (a sign-out's final drain) may
            // report after another account signed in: not about this one.
            UploadEvent::SignInExpired { owner, .. }
            | UploadEvent::Parked { owner, .. }
            | UploadEvent::Stripped { owner, .. }
            | UploadEvent::WorkspaceLost { owner, .. }
            | UploadEvent::DeviceRevoked { owner, .. }
                if !self.is_current_owner(&owner) =>
            {
                tracing::debug!("an upload event of the previous account ignored");
            }
            UploadEvent::SignInExpired { reason, .. } => self.expire(&reason),
            UploadEvent::WorkspaceLost { workspace_id, .. } => self.workspace_lost(&workspace_id),
            UploadEvent::DeviceRevoked { workspace_id, .. } => {
                self.device_revoked(&workspace_id);
                // Confirm with the device's status (design 5.2).
                self.control.send(ControlRequest::DeviceStatus);
            }
            UploadEvent::Uploaded(count) => {
                tracing::debug!(count, "blocks reached the server");
                self.totals_wanted = true;
            }
            // After a 403/409 the projects are fetched again (CONTRACT §2).
            UploadEvent::Parked { count, .. } => {
                tracing::info!(
                    count,
                    "blocks were not accepted; checking the projects again"
                );
                self.next_projects_refresh = Instant::now();
                self.state
                    .update(|s| s.push_notice(NoticeKind::BlocksNotAccepted { count }, now));
            }
            UploadEvent::Stripped { count, .. } => {
                self.state
                    .update(|s| s.push_notice(NoticeKind::ScreenshotsRemoved { count }, now));
            }
            UploadEvent::Issue(issue) => self.state.update(|s| s.upload_issue = issue),
            UploadEvent::Synced(at) => self.state.update(|s| s.last_sync = Some(at)),
            UploadEvent::RowState { row_id, state } => set_upload_state(&self.state, row_id, state),
            UploadEvent::QueueChanged => {
                let owner = self.account.as_ref().map(Account::owner);
                publish_queue(&self.store, &self.state, owner.as_deref());
            }
            UploadEvent::Failed(message) => self.state.update(|s| s.last_error = Some(message)),
        }
    }

    /// Whether `owner` is the account signed in now.
    fn is_current_owner(&self, owner: &str) -> bool {
        self.account
            .as_ref()
            .is_some_and(|account| account.owner() == owner)
    }

    /// Back online: retry the uploads now, and re-validate the projects.
    fn set_reachable(&mut self, reachable: bool) {
        if reachable && !self.reachable {
            tracing::info!("the server is reachable again");
            self.uploader.send(UploadCommand::Now);
            self.next_projects_refresh = Instant::now();
        }
        if !reachable {
            self.contacting = false;
        }
        self.reachable = reachable;
    }

    // ---- Publishing ---------------------------------------------------------------------------------

    fn facts(&self) -> Facts {
        Facts {
            storage_failed: false,
            signed_in: self.account.is_some(),
            sign_in_expired: self.expired,
            paused: self.pause.is_some() || self.hold.is_some(),
            has_session: self.session.is_some(),
            no_workspace: self
                .account
                .as_ref()
                .is_some_and(|a| a.workspace_id.is_none()),
            projects_known: self.projects.is_some(),
            contacting: self.contacting || self.signing_in,
            locked: self.locked || self.disconnected,
            away: self.away.is_some(),
            suspended: self.suspended.is_some() || self.shutting_down.is_some(),
            tracking: self.blocks.is_open(),
            consent: if self.consent_unsynced {
                // Fail closed: an unsaved change never counts as consent.
                match self.consent {
                    LocalConsent::Absent => LocalConsent::Absent,
                    _ => LocalConsent::Revoked,
                }
            } else {
                self.consent
            },
            device_revoked: self.revoked_here(),
            no_graphical_session: !self.session_summary.kind.is_graphical(),
        }
    }

    /// Every capability with its state and the first reason it is off.
    fn capabilities(&self, status: TrackerStatus) -> Capabilities {
        let consented = if self.consent_unsynced {
            ConsentedPermissions::all_disabled()
        } else {
            self.consent.permissions()
        };
        let project = self.session.as_ref().and_then(|s| {
            self.projects
                .iter()
                .flatten()
                .find(|p| p.id == s.project_id())
        });
        let allows = |f: fn(&Project) -> bool| project.is_some_and(f);
        let local = &self.cfg.capture;
        let paused = status != TrackerStatus::Tracking;
        let evaluate = |consented: bool,
                        project_allows: bool,
                        config_allows: bool,
                        platform: &PlatformCapability| {
            CapabilityInfo::evaluate(
                consented,
                project_allows,
                config_allows,
                &platform.availability,
                paused,
                platform.backend.as_deref(),
            )
        };
        let webcam = PlatformCapability::unsupported(WEBCAM_UNAVAILABLE);
        Capabilities {
            keyboard: evaluate(
                consented.keyboard,
                allows(|p| p.allow_keyboard),
                local.keyboard,
                &self.platform.keyboard,
            ),
            mouse: evaluate(
                consented.mouse,
                allows(|p| p.allow_mouse),
                local.mouse,
                &self.platform.mouse,
            ),
            active_window: evaluate(
                consented.active_window,
                allows(|p| p.allow_active_window),
                local.active_window,
                &self.platform.active_window,
            ),
            running_programs: evaluate(
                consented.running_programs,
                allows(|p| p.allow_running_programs),
                local.running_programs,
                &self.platform.running_programs,
            ),
            screenshot: evaluate(
                consented.screenshot,
                allows(|p| p.allow_screenshot),
                local.screenshot,
                &self.platform.screenshot,
            ),
            webcam: evaluate(consented.webcam, allows(|p| p.allow_webcam), true, &webcam),
        }
    }

    fn view(&self) -> View {
        let status = session::status(&self.facts());
        let stretch = self
            .blocks
            .stretch()
            .filter(|_| status == TrackerStatus::Tracking);
        let pause = self.pause.filter(|_| status == TrackerStatus::Paused);
        let held = self.hold.filter(|_| status == TrackerStatus::Paused);
        View {
            status,
            tracking_since: stretch.map(|s| s.since),
            block_started_at: stretch.map(|s| s.block.started_at),
            unsaved_since: stretch.map(Stretch::unsaved_since),
            next_capture_at: stretch.map(|s| s.block.planned_end()),
            paused_since: pause.map(|p| p.since).or(held),
            resume_at: pause.and_then(|p| p.until),
            away_since: self
                .away
                .as_ref()
                .filter(|_| status == TrackerStatus::Away)
                .map(|a| a.since),
            away_prompt: self.prompt.as_ref().map(|p| AwayPrompt {
                from: p.from,
                to: p.to,
                project_title: p.session.title.clone(),
            }),
            selected_project: self.session.as_ref().map(|s| s.project_id().to_owned()),
            // Before a project is resolved, the configured interval.
            interval_minutes: self.session.as_ref().map_or(
                self.cfg.interval().as_secs() / 60,
                Session::interval_minutes,
            ),
            idle_threshold_minutes: self
                .session
                .as_ref()
                .and_then(Session::idle_threshold_minutes),
            capture: self
                .session
                .as_ref()
                .map(Session::capture_flags)
                .unwrap_or_default(),
            task_note: self.task_note.clone().unwrap_or_default(),
            server_reachable: self.reachable,
            clock_skew_minutes: self.clock.skew_minutes(),
            consent: self.consent,
            consent_error: self.consent_error.clone(),
            consent_unsynced: self.consent_unsynced,
            capabilities: self.capabilities(status),
            session: self.session_summary.clone(),
            manual_mode: self.manual_mode(),
            credentials_locked: self.credentials_locked,
        }
    }

    /// Publish the status and the clocks the clients animate (only what
    /// changed wakes them), and send a heartbeat when the presence changed
    /// or one is due.
    pub(crate) fn publish(&mut self) {
        let view = self.view();
        let status = view.status;
        let is_tracking = status == TrackerStatus::Tracking;
        let perms = if is_tracking {
            self.session.as_ref().map(|s| &s.policy.perms)
        } else {
            None
        };
        self.machine.apply_sensor_policy(is_tracking, perms);
        if self.published.as_ref() != Some(&view) {
            let v = view.clone();
            self.state.update(|s| {
                s.status = v.status;
                s.tracking_since = v.tracking_since;
                s.block_started_at = v.block_started_at;
                s.unsaved_since = v.unsaved_since;
                s.next_capture_at = v.next_capture_at;
                s.paused_since = v.paused_since;
                s.resume_at = v.resume_at;
                s.away_since = v.away_since;
                s.away_prompt = v.away_prompt;
                s.selected_project = v.selected_project;
                s.interval_minutes = v.interval_minutes;
                s.idle_threshold_minutes = v.idle_threshold_minutes;
                s.capture = v.capture;
                s.task_note = v.task_note;
                s.server_reachable = v.server_reachable;
                s.clock_skew_minutes = v.clock_skew_minutes;
                s.consent = v.consent;
                s.consent_error = v.consent_error;
                s.consent_unsynced = v.consent_unsynced;
                s.capabilities = v.capabilities;
                s.session = v.session;
                s.manual_mode = v.manual_mode;
                s.credentials_locked = v.credentials_locked;
            });
            self.published = Some(view);
        }
        self.maybe_heartbeat(status);
    }

    /// Presence (CONTRACT §3): every minute while signed in, and at once on
    /// every change of status or project. Never queued; never while the
    /// sign-in is expired, the keyring locked or the device revoked.
    fn maybe_heartbeat(&mut self, status: TrackerStatus) {
        let timed_break = self.pause.is_some_and(|p| p.until.is_some());
        let in_workspace = self
            .account
            .as_ref()
            .is_some_and(|a| a.workspace_id.is_some());
        let online = in_workspace
            && !self.expired
            && !self.credentials_locked
            && !self.revoked_here()
            && self.suspended.is_none()
            && self.shutting_down.is_none();
        let Some(presence) = session::presence(status, timed_break).filter(|_| online) else {
            return;
        };
        let now = self.now();
        let key = (
            presence,
            self.session.as_ref().map(|s| s.project_id().to_owned()),
        );
        let changed = self.presence_sent.as_ref() != Some(&key);
        if changed {
            self.presence_since = now;
        }
        if changed || Instant::now() >= self.next_heartbeat {
            if let Some(heartbeat) = self.heartbeat(presence, self.presence_since, now) {
                self.control
                    .send(ControlRequest::Heartbeat(Box::new(heartbeat)));
            }
            self.presence_sent = Some(key);
            self.next_heartbeat = Instant::now() + HEARTBEAT_EVERY;
        }
    }

    /// A heartbeat saying `status` since `since`; `None` without a device id
    /// (the server needs one).
    fn heartbeat(
        &self,
        status: PresenceStatus,
        since: DateTime<Utc>,
        now: DateTime<Utc>,
    ) -> Option<Heartbeat> {
        let device = self.machine.device();
        let device_id = device.device_id.trim();
        if device_id.is_empty() {
            return None;
        }
        let session = self.session.as_ref();
        // The app in front only while tracking, and only when the session
        // records the active window.
        let current_app = if status == PresenceStatus::Tracking
            && session.is_some_and(|s| s.policy.perms.active_window)
        {
            self.machine.current_app()
        } else {
            None
        };
        Some(Heartbeat {
            device_id: device_id.to_owned(),
            device_name: device.device_name.clone(),
            platform: Some(device.platform.clone()),
            os_version: Some(device.os_version.clone()),
            agent_version: Some(device.agent_version.clone()),
            status,
            project_id: session.map(|s| s.project_id().to_owned()),
            status_since: Some(since),
            current_app,
            time_zone: device.time_zone.clone(),
            utc_offset_minutes: Some(device.utc_offset_minutes(now)),
            sent_at: None,
        })
    }

    // ---- Persistence ----------------------------------------------------------------------------------

    fn persist(&self) {
        let (Some(path), Some(saved)) = (&self.session_file, &self.saved) else {
            return;
        };
        if let Err(e) = session_store::save(path, saved) {
            tracing::warn!(error = %e, "could not save the session");
        }
    }

    /// A pause survives a restart (CONTRACT §4).
    fn persist_pause(&mut self) {
        if let Some(saved) = self.saved.as_mut() {
            saved.pause = self.pause;
        }
        self.persist();
    }

    fn clear_error(&self, message: &str) {
        self.state.update(|s| {
            if s.last_error.as_deref() == Some(message) {
                s.last_error = None;
            }
        });
    }

    /// Housekeeping that bounds the local queue: expire parked rows and other
    /// accounts' stale rows, then shed images/rows beyond [`QueueLimits::DEFAULT`].
    fn maintain_queue(&self, owner: Option<&str>) {
        match self.store.purge_parked_older_than(PARKED_RETENTION_DAYS) {
            Ok(n) if n > 0 => tracing::info!(purged = n, "removed old parked activities"),
            Err(e) => tracing::warn!(error = %e, "could not purge parked activities"),
            _ => {}
        }
        match self
            .store
            .purge_foreign_pending_older_than(owner, PARKED_RETENTION_DAYS)
        {
            Ok(n) if n > 0 => {
                tracing::info!(purged = n, "removed expired activities of other accounts")
            }
            Err(e) => tracing::warn!(error = %e, "could not purge other accounts' activities"),
            _ => {}
        }
        self.enforce_queue_limits();
    }

    fn enforce_queue_limits(&self) {
        match self.store.enforce_limits(&QueueLimits::DEFAULT) {
            Ok(report) if report.stripped > 0 || report.dropped() > 0 => {
                tracing::warn!(
                    stripped = report.stripped,
                    dropped = report.dropped(),
                    "offline queue over its limits; shed images / oldest activities"
                );
                if report.dropped() > 0 {
                    let dropped = report.dropped() as u64;
                    let now = self.now();
                    self.state
                        .update(|s| s.push_notice(NoticeKind::QueueTrimmed { dropped }, now));
                }
            }
            Err(e) => tracing::warn!(error = %e, "could not enforce offline queue limits"),
            _ => {}
        }
    }
}

/// `consent` as it may apply before it is saved: whatever it withdraws is
/// withdrawn, nothing it adds is granted, a tighter blur applies at once.
pub fn narrowed(current: LocalConsent, target: LocalConsent) -> LocalConsent {
    match (current, target) {
        (LocalConsent::Granted(old), LocalConsent::Granted(new)) => {
            LocalConsent::Granted(ConsentedPermissions {
                screenshot: old.screenshot && new.screenshot,
                webcam: old.webcam && new.webcam,
                keyboard: old.keyboard && new.keyboard,
                mouse: old.mouse && new.mouse,
                active_window: old.active_window && new.active_window,
                running_programs: old.running_programs && new.running_programs,
                blur_screenshots: old.blur_screenshots || new.blur_screenshots,
            })
        }
        // Widening from nothing waits for the save.
        (LocalConsent::Absent | LocalConsent::Revoked, LocalConsent::Granted(_)) => current,
        (_, target) => target,
    }
}

/// The 32-byte signing key of saved credentials, if they carry a valid one.
fn signing_key_of(credentials: &Credentials) -> Option<[u8; 32]> {
    credentials
        .device_signing_key
        .as_deref()
        .and_then(|key| <[u8; 32]>::try_from(key).ok())
}

/// The task note as kept: control characters gone, whitespace collapsed, at
/// most [`TASK_NOTE_MAX_CHARS`]; `None` when blank.
pub fn clean_note(note: &str) -> Option<String> {
    let collapsed = note
        .split(|c: char| c.is_whitespace() || c.is_control())
        .filter(|word| !word.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    let note: String = collapsed.chars().take(TASK_NOTE_MAX_CHARS).collect();
    let note = note.trim_end();
    (!note.is_empty()).then(|| note.to_string())
}

/// A sign-in failure, worded for the login form.
pub fn auth_message(error: &AuthError) -> String {
    match error {
        AuthError::Rejected(reason) => reason.clone(),
        AuthError::Offline(_) => {
            "The server cannot be reached. Check your connection and the server address.".into()
        }
        AuthError::Server(message) => message.clone(),
    }
}

/// Show the signed-in account's queue: pending / parked counts and the
/// newest rows with the server's reasons (nothing when signed out).
fn publish_queue(store: &Outbox, state: &StateHandle, owner: Option<&str>) {
    let (pending, parked, rows) = match owner {
        Some(owner) => {
            let summary = store.queue_summary(owner).unwrap_or_default();
            let rows = store
                .recent_rows(owner, QUEUE_ROWS_SHOWN)
                .unwrap_or_default()
                .into_iter()
                .map(|row| QueuedRow {
                    id: row.id,
                    project_id: row.project_id,
                    started_at: row.started_at,
                    ended_at: row.ended_at,
                    status: match row.state {
                        RowState::Pending => RowStatus::Queued,
                        RowState::Parked => RowStatus::Parked,
                    },
                    reason: row.parked_reason.or(row.last_error),
                    has_captures: row.capture_bytes > 0,
                })
                .collect();
            (summary.pending, summary.parked, rows)
        }
        None => (0, 0, Vec::new()),
    };
    state.update(|s| {
        s.pending_uploads = u32::try_from(pending).unwrap_or(u32::MAX);
        s.rejected_uploads = u32::try_from(parked).unwrap_or(u32::MAX);
        s.queue_rows = rows;
    });
}

/// The clients' summary of a block just queued.
fn recent_block(
    row_id: i64,
    project: String,
    activity: &Activity,
    screenshot: bool,
) -> RecentBlock {
    RecentBlock {
        row_id,
        project,
        started_at: activity.started_at,
        ended_at: activity.ended_at,
        productivity: activity.productivity,
        keyboard_hits: activity.keyboard_hits,
        mouse_clicks: activity.mouse_clicks,
        screenshot,
        upload: UploadState::Queued,
    }
}

/// Newest first, bounded to [`RECENT_BLOCKS`].
fn push_recent_block(blocks: &mut Vec<RecentBlock>, block: RecentBlock) {
    blocks.insert(0, block);
    blocks.truncate(RECENT_BLOCKS);
}

/// Record where a queued block is now (uploaded / parked), if it is listed.
fn set_upload_state(state: &StateHandle, row_id: i64, upload: UploadState) {
    state.update(|s| {
        if let Some(block) = s.recent_blocks.iter_mut().find(|b| b.row_id == row_id) {
            block.upload = upload;
        }
    });
}

/// Each local day of this week before today as (slot, start, end): the slot
/// is the day's index from Monday, and the day runs `[start, end)`. Empty on
/// a Monday.
pub fn past_days_of_week<Tz: TimeZone>(
    now: DateTime<Utc>,
    tz: &Tz,
) -> Vec<(usize, DateTime<Utc>, DateTime<Utc>)> {
    let today = now.with_timezone(tz).date_naive();
    let monday = today - chrono::Days::new(u64::from(today.weekday().num_days_from_monday()));
    monday
        .iter_days()
        .take_while(|day| *day < today)
        .enumerate()
        .filter_map(|(slot, day)| {
            Some((
                slot,
                local_midnight(day, tz)?,
                local_midnight(day.succ_opt()?, tz)?,
            ))
        })
        .collect()
}

/// Seconds of `started_at .. ended_at` that fall on or after `since` (e.g.
/// today's local midnight), so a block spanning midnight only adds today's
/// part to "today".
pub fn tracked_secs_since(
    started_at: DateTime<Utc>,
    ended_at: DateTime<Utc>,
    since: DateTime<Utc>,
) -> f64 {
    let from = started_at.max(since);
    if ended_at <= from {
        0.0
    } else {
        (ended_at - from).num_milliseconds() as f64 / 1000.0
    }
}

/// Local midnight today and on this week's Monday, as UTC instants for the
/// API. "Today" follows the user's wall clock, not UTC — otherwise the cards
/// would reset mid-day (at 06:00 in UTC+6, at 19:00 the day before in UTC-5).
pub fn local_day_and_week_start<Tz: TimeZone>(
    now: DateTime<Utc>,
    tz: &Tz,
) -> (DateTime<Utc>, DateTime<Utc>) {
    let today = now.with_timezone(tz).date_naive();
    // Monday-anchored week.
    let monday = today - chrono::Days::new(u64::from(today.weekday().num_days_from_monday()));
    (
        local_midnight(today, tz).unwrap_or(now),
        local_midnight(monday, tz).unwrap_or(now),
    )
}

/// The first instant of `date` in `tz`. Where a DST jump skips midnight, the
/// day starts at the first hour that exists.
fn local_midnight<Tz: TimeZone>(date: NaiveDate, tz: &Tz) -> Option<DateTime<Utc>> {
    (0..3)
        .find_map(|hour| {
            tz.from_local_datetime(&date.and_hms_opt(hour, 0, 0)?)
                .earliest()
        })
        .map(|start| start.with_timezone(&Utc))
}

/// The screenshot moment's random fraction, from the OS generator.
pub fn random_fraction() -> f64 {
    blocks::random_fraction()
}

#[cfg(test)]
mod tests;
