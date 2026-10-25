//! Background tracking worker.
//!
//! Owns everything slow or fallible — capture, the local queue, talking to
//! the server — away from the UI thread, in three parts so none waits on
//! another:
//!
//! - the worker loop (this module, on its own thread) decides when blocks
//!   start and end — from the window's commands, the session and power
//!   events, the user's input and the clocks — builds them and queues them
//!   locally. It never waits on the network, so a pause, a lock or a suspend
//!   is handled at once;
//! - the control task ([`control`]) signs in, fetches the projects, sends the
//!   presence heartbeat and refreshes the totals;
//! - the uploader thread ([`uploader`]) uploads the queue, with its own
//!   connection to it.
//!
//! The UI talks to it only through a command channel and reads a shared
//! state snapshot. Everything the worker asks of the machine (input times,
//! the screen, the device) goes through [`Machine`], so its rules are tested
//! without hooks or a screen.

mod blocks;
mod control;
mod session;
mod uploader;

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Datelike, Local, NaiveDate, TimeDelta, TimeZone, Utc};
use tokio::sync::mpsc::{self, UnboundedReceiver, UnboundedSender};
use tokio::time::Instant;

use crate::api::{ApiClient, ApiError, AuthError, ServerClock, TokenCell, Workspace};
use crate::config::{normalize_server_url, AppConfig, CapturePermissions, ConsentStore, FileConsentStore};
use crate::credentials::{self, Credentials, ProjectCache, SavedPause};
use crate::model::{Activity, EndReason, Heartbeat, PresenceStatus, Project};
use crate::power::{self, SleepDetector};
use crate::session_events::{SystemEvent, SystemNotice};
use crate::state::{
    AwayPrompt, CaptureFlags, Command, LastScreenshot, NoticeKind, ProjectOption, RecentBlock, StateHandle,
    TrackerStatus, UploadState, WorkspaceChoice, RECENT_BLOCKS,
};
use crate::storage::{self, QueueLimits, Storage};
use crate::tracking::system::DeviceInfo;
use crate::tracking::{BlockSpec, ProvenanceScreenCapture, Tracker, MIN_BLOCK};

use blocks::{BlockId, Blocks, FinishedBlock, Stretch};
use control::{ControlEvent, ControlLink, ControlRequest};
use session::{pick_project, restored_pause, session_for, Facts, Pick, Session};
use uploader::{UploadCommand, UploadEvent, UploaderHandle};

/// How often the worker looks at the clocks and the user's input: block
/// boundaries, the screenshot moment, idle, a missed suspend, a timed break
/// running out. Cheap — a few comparisons — and quick enough that coming
/// back to the computer shows within a couple of seconds.
const TICK: Duration = Duration::from_secs(2);

/// How long parked (server-rejected) rows are kept for diagnostics before the
/// queue reclaims their space. Queued rows of an account that is not signed in
/// wait this long for it to come back, then are purged too.
const REJECTED_RETENTION_DAYS: u32 = 14;

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

/// Quitting waits this long for the uploads; the rest go up on the next launch.
const QUIT_UPLOAD_GRACE: Duration = Duration::from_secs(15);

/// Signing out lets the uploader finish the account's queue this long.
const SIGN_OUT_UPLOAD_GRACE: Duration = Duration::from_secs(15);

/// Quitting waits this long for the "stopped" heartbeat.
const FAREWELL_WAIT: Duration = Duration::from_secs(2);

/// A screenshot taken this long after its block's end (a tick late) still
/// belongs to it; one taken later — after an idle cut — does not.
const CAPTURE_SLACK: TimeDelta = TimeDelta::seconds(10);

/// A block built this long after it ended (it was held while the user was
/// inactive) gets no webcam photo: one taken now would not show that block.
const LATE: TimeDelta = TimeDelta::seconds(30);

/// How many queue rows the window lists.
const QUEUE_ROWS_SHOWN: u32 = 20;

/// The longest task note ("≤ 200 chars in the UI", CONTRACT §4).
pub const TASK_NOTE_MAX_CHARS: usize = 200;

const ONE_WORKSPACE: &str = "You aren't in any other workspace.";
pub const NO_WORKSPACE: &str =
    "You aren't in any workspace yet. Create one on the web dashboard or ask for an invite.";

const NO_PROJECTS: &str = "You are not a member of any active project.";

/// The signed-in user, the server it lives on and the workspace tracked in.
#[derive(Clone, PartialEq, Eq)]
pub struct Account {
    /// The API address, normalized.
    pub server: String,
    pub username: String,
    pub password: String,
    /// `None` until a workspace is chosen (or after access to it was lost).
    pub workspace_id: Option<String>,
    pub workspace_name: String,
    /// The user's token, shared by every client of this sign-in.
    pub tokens: TokenCell,
}

impl Account {
    pub fn new(server: String, username: String, password: String) -> Self {
        Self { server, username, password, workspace_id: None, workspace_name: String::new(), tokens: TokenCell::default() }
    }

    /// The same sign-in, tracking in another workspace.
    pub fn in_workspace(mut self, id: &str, name: &str) -> Self {
        self.workspace_id = Some(id.to_owned());
        self.workspace_name = name.to_owned();
        self
    }

    fn of(saved: &Credentials, cfg: &AppConfig) -> Self {
        Self {
            server: cfg.effective_api_base_url(saved.server.as_deref()),
            username: saved.username.clone(),
            password: saved.password.clone(),
            workspace_id: saved.workspace_id.clone(),
            workspace_name: saved.workspace_name.clone(),
            tokens: TokenCell::with_refresh_token(saved.refresh_token.clone()),
        }
    }

    /// The user, as the offline queue tags its rows.
    pub fn owner(&self) -> String {
        storage::owner_key(&self.username)
    }

    /// A client for this account. Every client shares `clock`, so the clock
    /// warning sees every answer.
    pub fn client(&self, clock: &ServerClock) -> ApiClient {
        ApiClient::new(self.server.clone(), self.username.clone(), self.password.clone())
            .with_server_clock(clock.clone())
            .with_tokens(self.tokens.clone())
            .with_workspace(self.workspace_id.clone())
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

/// What the worker needs from the machine — the time, the user's input,
/// the screen, the device — behind thin functions, so the rules above them
/// are tested without hooks, a screen or a real clock.
pub trait Machine: Send + Sync {
    fn now(&self) -> DateTime<Utc>;
    /// Milliseconds the machine has been awake ([`power::awake_millis`]).
    fn awake_millis(&self) -> u64;
    /// The user's latest keyboard or mouse input.
    fn last_input(&self, now: DateTime<Utc>) -> Option<DateTime<Utc>>;
    /// The user's first input after `since`, up to `now` (coming back).
    fn first_input_after(&self, since: DateTime<Utc>, now: DateTime<Utc>) -> Option<DateTime<Utc>>;
    fn capture_screen(&self, blur: bool) -> Option<crate::tracking::ProvenanceScreenCapture>;
    /// Build the block's upload payload.
    fn snapshot(&self, block: &BlockSpec<'_>, screen: Option<crate::tracking::ProvenanceScreenCapture>) -> Activity;

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
    /// The app in front now (its executable name).
    fn current_app(&self) -> Option<String>;
    /// A uniformly random fraction in `[0, 1)` (the screenshot moment).
    fn random_fraction(&self) -> f64;
    /// Turn active sensor collection on or off (usage sampler and input counting).
    #[allow(dead_code)]
    fn set_tracking_active(&self, active: bool);
    /// Apply effective sensor policy based on active tracking status and session permissions.
    fn apply_sensor_policy(&self, active: bool, perms: Option<&CapturePermissions>);
}

impl Machine for Tracker {
    fn now(&self) -> DateTime<Utc> {
        Utc::now()
    }

    fn awake_millis(&self) -> u64 {
        power::awake_millis()
    }

    /// The hooks' view (the user's own input) while they work; otherwise
    /// Windows' idle clock, which mouse jigglers also reset.
    fn last_input(&self, now: DateTime<Utc>) -> Option<DateTime<Utc>> {
        let input = self.input();
        if input.hook_healthy() {
            input.last_input_at()
        } else {
            crate::tracking::input::idle_seconds().map(|idle| now - seconds(idle))
        }
    }

    fn first_input_after(&self, since: DateTime<Utc>, now: DateTime<Utc>) -> Option<DateTime<Utc>> {
        let input = self.input();
        if input.hook_healthy() {
            input
                .active_seconds(since + TimeDelta::seconds(1), now, true, true)
                .into_iter()
                .min()
                .and_then(|second| DateTime::from_timestamp(second, 0))
        } else {
            self.last_input(now).filter(|last| *last > since)
        }
    }

    fn capture_screen(&self, blur: bool) -> Option<crate::tracking::ProvenanceScreenCapture> {
        Tracker::capture_screen(self, blur)
    }

    fn snapshot(&self, block: &BlockSpec<'_>, screen: Option<crate::tracking::ProvenanceScreenCapture>) -> Activity {
        Tracker::snapshot(self, block, screen)
    }


    fn kept_idle_blocks(
        &self,
        project_id: &str,
        from: DateTime<Utc>,
        to: DateTime<Utc>,
        interval: Duration,
        description: Option<&str>,
    ) -> Vec<Activity> {
        Tracker::kept_idle_blocks(self, project_id, from, to, interval, description)
    }

    fn device(&self) -> &DeviceInfo {
        Tracker::device(self)
    }

    fn current_app(&self) -> Option<String> {
        Tracker::current_app(self)
    }

    fn random_fraction(&self) -> f64 {
        blocks::random_fraction()
    }

    fn set_tracking_active(&self, active: bool) {
        Tracker::set_tracking_active(self, active);
    }

    fn apply_sensor_policy(&self, active: bool, perms: Option<&CapturePermissions>) {
        Tracker::apply_sensor_policy(self, active, perms);
    }
}

/// Seconds as a delta, bounded to a year (idle times beyond that are noise).
fn seconds(secs: u64) -> TimeDelta {
    TimeDelta::seconds(secs.min(366 * 24 * 3600) as i64)
}

/// How the worker was started.
#[derive(Debug, Clone, Copy, Default)]
pub struct Launch {
    /// By Windows at sign-in (`--autostart`).
    pub autostart: bool,
}

/// The running worker.
pub struct WorkerHandle {
    /// The window's commands.
    pub commands: UnboundedSender<Command>,
    /// Lock, sleep and shutdown events ([`crate::session_events`]).
    pub system: UnboundedSender<SystemNotice>,
    pub thread: std::thread::JoinHandle<()>,
}

/// Spawn the worker thread.
pub fn spawn(
    state: StateHandle,
    cfg: AppConfig,
    tracker: Arc<Tracker>,
    launch: Launch,
    repaint: Arc<dyn Fn() + Send + Sync>,
) -> WorkerHandle {
    let (commands, commands_rx) = mpsc::unbounded_channel();
    let (system, system_rx) = mpsc::unbounded_channel();

    let thread = std::thread::Builder::new()
        .name("dosi-worker".into())
        .spawn(move || {
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
                    repaint();
                    return;
                }
            };

            // A panic in here would otherwise kill the worker silently while the
            // window kept showing a green "Tracking" pill and captured nothing.
            // Catch it and surface the failure instead of faking success.
            let state_for_run = state.clone();
            let repaint_for_run = repaint.clone();
            let machine: Arc<dyn Machine> = tracker;
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                runtime.block_on(run(state_for_run, cfg, machine, launch, repaint_for_run, commands_rx, system_rx));
            }));

            if result.is_err() {
                tracing::error!("the tracking worker panicked; tracking has stopped");
                state.update(|s| {
                    s.status = TrackerStatus::Stopped;
                    s.last_error =
                        Some("Tracking stopped unexpectedly. Please restart Dosi Tracker.".into());
                });
                repaint();
            }
        })
        .expect("failed to spawn worker thread");

    WorkerHandle { commands, system, thread }
}

/// The offline queue: machine-local (`%LOCALAPPDATA%\DosiTracker`), never in
/// the roaming profile or the working directory.
fn queue_path() -> Option<PathBuf> {
    let path = crate::shell::queue_db_path()?;
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    Some(path)
}

async fn run(
    state: StateHandle,
    cfg: AppConfig,
    machine: Arc<dyn Machine>,
    launch: Launch,
    repaint: Arc<dyn Fn() + Send + Sync>,
    mut commands: UnboundedReceiver<Command>,
    mut system: UnboundedReceiver<SystemNotice>,
) {
    let Some(path) = queue_path() else {
        tracing::error!("no local data folder for the queue");
        state.update(|s| {
            s.status = TrackerStatus::Stopped;
            s.last_error = Some("Windows did not give Dosi Tracker a folder for its local queue.".into());
        });
        repaint();
        return;
    };
    let credentials_file = credentials::default_path().ok();
    let saved = credentials_file.as_deref().and_then(credentials::load_from);
    // The saved sign-in claims rows queued by builds that predate per-account
    // tagging.
    let legacy_owner = saved.as_ref().map(Credentials::owner_key);

    // A damaged queue is moved aside and replaced. One that cannot be opened
    // at all (locked by another program, no permission, disk full) leaves
    // nothing to record into: say so, and try again every minute.
    let opened = loop {
        match Storage::open_or_recover(&path, legacy_owner.as_deref()) {
            Ok(opened) => break opened,
            Err(e) => {
                tracing::error!(?e, "could not open the local queue; retrying");
                state.update(|s| {
                    s.status = TrackerStatus::Stopped;
                    s.last_error = Some(format!(
                        "The local queue cannot be opened ({e}). Nothing is recorded until it can — retrying every minute."
                    ));
                });
                repaint();
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

    // Attach the device signing key so every enqueued payload gets an HMAC tag.
    let mut storage = opened.storage;
    match crate::signing::load_or_create_key() {
        Ok(key) => storage.set_signing_key(key),
        Err(e) => tracing::warn!(?e, "could not load device signing key; payloads will not be signed"),
    }
    // Reconstruct Opened with the updated storage so the rest of the worker
    // can use it through the original variable name.
    let opened = crate::storage::Opened { storage, moved_aside: opened.moved_aside };

    let clock = ServerClock::default();
    let (control_events_tx, mut control_events) = mpsc::unbounded_channel();
    let control = control::spawn(state.clone(), clock.clone(), control_events_tx);
    let (upload_events_tx, mut upload_events) = mpsc::unbounded_channel();
    let uploader = uploader::spawn(path, state.clone(), clock.clone(), repaint.clone(), upload_events_tx);

    let consent_store: Arc<dyn ConsentStore> = Arc::new(
        FileConsentStore::production().unwrap_or_else(|| {
            FileConsentStore::new(PathBuf::from("consent.json"))
        }),
    );

    let mut worker = Worker::new(
        state,
        cfg,
        machine,
        launch,
        repaint,
        clock,
        opened.storage,
        control,
        uploader,
        credentials_file,
        consent_store,
    );
    worker.start(saved, opened.moved_aside);
    worker.publish();

    let mut tick = tokio::time::interval(TICK);
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        // Biased: session and power notices queued while the worker was busy
        // (one may have gone unanswered before the machine slept) are handled
        // before a tick, which would otherwise see the sleep as a gap first.
        tokio::select! {
            biased;
            Some(notice) = system.recv() => worker.on_system(notice),
            command = commands.recv() => match command {
                Some(Command::Shutdown { at }) => {
                    worker.quit(at).await;
                    return;
                }
                Some(command) => worker.on_command(command),
                None => {
                    worker.quit(worker.now()).await;
                    return;
                }
            },
            Some(event) = control_events.recv() => worker.on_control(event),
            Some(event) = upload_events.recv() => worker.on_upload(event),
            _ = tick.tick() => worker.on_tick(),
        }
        worker.publish();
    }
}

/// Whether to take the time to capture: not before sleep or shutdown, when
/// the block must be saved within Windows' patience (and the network is off
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
    /// Where the time away stops counting: the screen locked or the machine
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
    consent: crate::config::LocalConsent,
    consent_error: Option<String>,
    consent_unsynced: bool,
}

struct Worker {
    state: StateHandle,
    cfg: AppConfig,
    machine: Arc<dyn Machine>,
    repaint: Arc<dyn Fn() + Send + Sync>,
    autostart: bool,
    clock: ServerClock,
    store: Storage,
    control: ControlLink,
    uploader: UploaderHandle,
    /// Where the sign-in is saved; `None` keeps it in memory only (tests).
    credentials_file: Option<PathBuf>,
    consent_store: Arc<dyn ConsentStore>,
    consent_error: Option<String>,
    consent_unsynced: bool,

    /// The saved sign-in, with what it remembers; `None` when signed out.
    saved: Option<Credentials>,
    account: Option<Account>,
    /// Signed in to the server, waiting for a workspace to be picked.
    pending: Option<Account>,
    /// The workspace (name or id) to pick after signing in, if listed.
    preselect: Option<String>,
    /// Workspaces offered to choose from.
    choices: Vec<Workspace>,
    /// The server refused the saved password.
    expired: bool,
    /// A sign-in the user typed is being checked.
    signing_in: bool,
    /// The server's first answer since starting is awaited.
    contacting: bool,
    reachable: bool,
    /// The project list: the server's, or the saved one while offline.
    projects: Option<Vec<Project>>,
    session: Option<Session>,
    choose_notified: bool,
    task_note: Option<String>,

    pause: Option<SavedPause>,
    away: Option<Away>,
    prompt: Option<PendingPrompt>,
    locked: bool,
    disconnected: bool,
    /// Since when the machine is (announced to be) asleep.
    suspended: Option<DateTime<Utc>>,
    /// Since when Windows is (announced to be) shutting down or signing out.
    shutting_down: Option<DateTime<Utc>>,
    /// When the machine was last seen awake again: resumed, unlocked,
    /// reconnected, or back from a sleep nobody announced.
    awake_since: Option<DateTime<Utc>>,

    blocks: Blocks,
    /// Screenshots taken for blocks not finished yet.
    screens: Vec<(BlockId, ProvenanceScreenCapture)>,
    sleep: SleepDetector,
    /// Tell the user when tracking starts by itself after an autostart.
    announce_autostart: bool,

    next_projects_refresh: Instant,
    next_heartbeat: Instant,
    next_maintenance: Instant,
    next_totals: Instant,
    totals_due_at: Instant,
    totals_wanted: bool,
    presence_sent: Option<(PresenceStatus, Option<String>)>,
    presence_since: DateTime<Utc>,
    published: Option<View>,
}

impl Worker {
    #[allow(clippy::too_many_arguments)]
    fn new(
        state: StateHandle,
        cfg: AppConfig,
        machine: Arc<dyn Machine>,
        launch: Launch,
        repaint: Arc<dyn Fn() + Send + Sync>,
        clock: ServerClock,
        store: Storage,
        control: ControlLink,
        uploader: UploaderHandle,
        credentials_file: Option<PathBuf>,
        consent_store: Arc<dyn ConsentStore>,
    ) -> Self {
        let now = Instant::now();
        let presence_since = machine.now();
        Self {
            state,
            cfg,
            machine,
            repaint,
            autostart: launch.autostart,
            clock,
            store,
            control,
            uploader,
            credentials_file,
            consent_store,
            consent_error: None,
            consent_unsynced: false,
            saved: None,
            account: None,
            pending: None,
            preselect: None,
            choices: Vec::new(),
            expired: false,
            signing_in: false,
            contacting: false,
            reachable: true,
            projects: None,
            session: None,
            choose_notified: false,
            task_note: None,
            pause: None,
            away: None,
            prompt: None,
            locked: false,
            disconnected: false,
            suspended: None,
            shutting_down: None,
            awake_since: None,
            blocks: Blocks::new(),
            screens: Vec::new(),
            sleep: SleepDetector::new(),
            announce_autostart: false,
            next_projects_refresh: now + PROJECTS_REFRESH,
            next_heartbeat: now,
            next_maintenance: now + MAINTENANCE_EVERY,
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

    // ---- Starting ------------------------------------------------------------

    /// Pick up where the last run left off: the saved sign-in, its pause and
    /// task note, and — when the server cannot be reached — the project list
    /// saved with it (at most a week old), so tracking starts offline too.
    fn start(&mut self, saved: Option<Credentials>, moved_aside: Option<PathBuf>) {
        self.cfg.consent = self.consent_store.load();
        if !self.cfg.consent.is_granted() {
            self.machine.apply_sensor_policy(false, None);
        }

        let now = self.now();
        let owner = saved.as_ref().map(Credentials::owner_key);
        // Reclaim space (old rejects, other accounts' expired rows, over-limit
        // images), and show what is left.
        maintain_queue(&self.store, owner.as_deref());
        publish_queue(&self.store, &self.state, owner.as_deref());
        if let Some(path) = moved_aside {
            let moved_to = path.display().to_string();
            self.state.update(|s| s.push_notice(NoticeKind::QueueReset { moved_to }, now));
        }

        // A sign-in saved by an older build names no workspace: sign in again.
        let Some(mut saved) = saved.filter(|saved| saved.workspace_id.is_some()) else {
            let server = self.cfg.api_base_url.clone();
            self.state.update(|s| s.server_url = server);
            return;
        };
        let pause = restored_pause(saved.pause, now, self.autostart);
        let pause_changed = pause != saved.pause;
        saved.pause = pause;
        self.pause = pause;
        self.task_note = saved.task_note.as_deref().and_then(clean_note);
        let account = Account::of(&saved, &self.cfg);
        // Offline first: the saved list is used at once; the server's answer
        // replaces it shortly.
        self.projects = saved.usable_projects(now).map(<[Project]>::to_vec);
        let (name, workspace, server) = (saved.username.clone(), saved.workspace_name.clone(), account.server.clone());
        self.state.update(|s| {
            s.display_name = name;
            s.workspace = workspace;
            s.server_url = server;
        });
        self.saved = Some(saved);
        if pause_changed {
            self.persist();
        }

        self.account = Some(account.clone());
        self.contacting = true;
        self.control.send(ControlRequest::UseAccount(Some(account.clone())));
        self.control.send(ControlRequest::RefreshProjects);
        self.next_projects_refresh = Instant::now() + RECONNECT_EVERY;
        self.uploader.send(UploadCommand::Account(Some(account)));
        self.totals_wanted = true;
        self.announce_autostart = self.autostart;
        self.choose_session(now);
        self.reconcile(now, EndReason::Interval);
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
        let remembered = self.saved.as_ref().and_then(|saved| saved.project_id.clone());
        match pick_project(projects, remembered.as_deref()) {
            Pick::Project(project) => {
                let project = project.clone();
                self.select(&project);
            }
            Pick::Choose => {
                self.clear_error(NO_PROJECTS);
                if !std::mem::replace(&mut self.choose_notified, true) {
                    self.state.update(|s| s.push_notice(NoticeKind::ChooseProject, now));
                }
            }
            Pick::NoProjects => {
                tracing::warn!("signed in, but not a member of any active project");
                self.state.update(|s| s.last_error = Some(NO_PROJECTS.into()));
            }
        }
    }

    /// Track `project` from the next block on, and remember it.
    fn select(&mut self, project: &Project) {
        tracing::info!(project = %project.title, "tracking session resolved");
        self.session = Some(session_for(project, &self.cfg));
        self.choose_notified = false;
        self.clear_error(NO_PROJECTS);
        if let Some(saved) = self.saved.as_mut() {
            if saved.project_id.as_deref() != Some(project.id.as_str()) {
                saved.project_id = Some(project.id.clone());
                self.persist();
            }
        }
    }

    fn publish_projects(&self) {
        let options: Vec<ProjectOption> = self
            .projects
            .iter()
            .flatten()
            .map(|p| ProjectOption { id: p.id.clone(), title: p.title.clone() })
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
            saved.projects = Some(ProjectCache { resolved_at: now, projects: projects.clone() });
        }
        self.persist();
        self.projects = Some(projects);

        if let Some(current) = self.session.as_ref().map(|s| s.project_id().to_owned()) {
            let listed = self.projects.iter().flatten().find(|p| p.id == current).cloned();
            match listed {
                Some(project) => {
                    let refreshed = session_for(&project, &self.cfg);
                    self.blocks.update_policy(refreshed.policy.clone());
                    self.session = Some(refreshed);
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
        self.state.update(|s| s.push_notice(NoticeKind::ProjectUnavailable { title: gone.title, now_tracking: next }, now));
    }

    // ---- Tracking on or off ------------------------------------------------------

    fn should_track(&self) -> bool {
        self.cfg.consent.is_granted()
            && !self.consent_unsynced
            && self.account.is_some()
            && !self.expired
            && self.session.is_some()
            && self.pause.is_none()
            && self.away.is_none()
            && !self.locked
            && !self.disconnected
            && self.suspended.is_none()
            && self.shutting_down.is_none()
    }

    /// Returns the currently active effective permissions consented by the user and permitted
    /// by session/policy. If consent is absent, revoked, or unsynced (fail-closed), all permissions
    /// are strictly false.
    fn current_effective_perms(&self) -> CapturePermissions {
        if !self.cfg.consent.is_granted() || self.consent_unsynced {
            return CapturePermissions::all_disabled();
        }
        self.session
            .as_ref()
            .map(|s| s.policy.perms)
            .unwrap_or_else(CapturePermissions::all_disabled)
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
            self.blocks.start(at.min(now), policy, self.task_note.clone(), fraction);
            if std::mem::take(&mut self.announce_autostart) {
                self.state.update(|s| s.push_notice(NoticeKind::TrackingStartedAutomatically { project_title: title }, now));
            }
        } else if self.blocks.is_open() {
            let finished = self.blocks.end(at, now, reason);
            self.record(finished, Speed::for_reason(reason));
        }
    }

    // ---- Recording -----------------------------------------------------------------

    /// Build the finished blocks and queue them locally.
    fn record(&mut self, finished: Vec<FinishedBlock>, speed: Speed) {
        let tagged = self.account.as_ref().and_then(|a| Some((a.owner(), a.workspace_id.clone()?)));
        let Some((owner, workspace)) = tagged.filter(|_| !finished.is_empty()) else {
            self.prune_screens();
            return;
        };
        let now = self.now();
        let current_perms = self.current_effective_perms();
        let mut queued = 0;
        for block in finished {
            let late = now - block.ended_at > LATE;
            let taken = self
                .take_screen(block.id)
                .filter(|capture| capture.capture.captured_at <= block.ended_at + CAPTURE_SLACK);
            // Ended before its screenshot moment: taken at the end instead —
            // unless the machine is locked, asleep, idle or shutting down,
            // or screenshot permission has been withdrawn.
            let capture_now = taken.is_none()
                && speed == Speed::Normal
                && !late
                && block.capture_at_end
                && block.policy.perms.screenshot
                && current_perms.screenshot
                && block.end_reason.allows_capture_at_end();
            let screen = if capture_now {
                self.machine.capture_screen(block.policy.perms.blur_screenshots || current_perms.blur_screenshots)
            } else if current_perms.screenshot {
                taken
            } else {
                None
            };
            let mut perms = block.policy.perms;
            if speed == Speed::Fast || late || !current_perms.webcam {
                perms.webcam = false;
            }
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
            let activity = self.machine.snapshot(&spec, screen);
            if self.enqueue(&activity, &owner, &workspace, now) {
                queued += 1;
            }
        }
        self.prune_screens();
        enforce_queue_limits(&self.store);
        publish_queue(&self.store, &self.state, Some(&owner));
        // Before sleep or shutdown nothing goes over the network.
        if queued > 0 && speed == Speed::Normal {
            self.uploader.send(UploadCommand::RowsQueued);
        }
        (self.repaint)();
    }

    /// Queue one block and show it (and count it in the totals at once; the
    /// server's totals, refreshed after the upload, replace that estimate).
    fn enqueue(&mut self, activity: &Activity, owner: &str, workspace: &str, now: DateTime<Utc>) -> bool {
        let id = match self.store.enqueue(activity, owner, workspace) {
            Ok(id) => id,
            Err(e) => {
                tracing::error!(?e, "failed to queue activity");
                self.state.update(|s| s.last_error = Some(format!("Could not save locally: {e}")));
                return false;
            }
        };
        tracing::debug!(id, end_reason = ?activity.end_reason, "activity queued");
        let (today_start, week_start) = local_day_and_week_start(now, &Local);
        let project = self
            .projects
            .iter()
            .flatten()
            .find(|p| p.id == activity.project_id)
            .map(|p| p.title.clone())
            .unwrap_or_default();
        let screenshot = decode_thumbnail(activity);
        let captured_at = activity.screenshot_captured_at.unwrap_or(now);
        let stretch = self.blocks.stretch();
        let (block_started_at, unsaved_since) = (stretch.map(|s| s.block.started_at), stretch.map(Stretch::unsaved_since));
        let (started_at, ended_at) = (activity.started_at, activity.ended_at);
        let block = recent_block(id, project, activity);
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
            if let Some(jpeg) = screenshot {
                s.last_screenshot = Some(Arc::new(LastScreenshot { taken_at: captured_at, jpeg }));
            }
        });
        true
    }

    fn take_screen(&mut self, id: BlockId) -> Option<ProvenanceScreenCapture> {
        let index = self.screens.iter().position(|(block, _)| *block == id)?;
        Some(self.screens.swap_remove(index).1)
    }

    /// Forget screenshots of blocks that are gone (dropped by an idle cut).
    fn prune_screens(&mut self) {
        let live = self.blocks.stretch().map(Stretch::block_ids).unwrap_or_default();
        self.screens.retain(|(id, _)| live.contains(id));
    }

    fn capture(&mut self, id: BlockId) {
        let current_perms = self.current_effective_perms();
        if !current_perms.screenshot {
            return;
        }
        let blur = current_perms.blur_screenshots
            || self.blocks.stretch().is_some_and(|s| s.block.policy.perms.blur_screenshots);
        if let Some(screen) = self.machine.capture_screen(blur) {
            self.screens.push((id, screen));
        }
        self.blocks.mark_captured(id);
    }

    // ---- The clock ---------------------------------------------------------------------

    fn on_tick(&mut self) {
        let now = self.now();
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
        if let Some(id) = self.blocks.capture_due(now) {
            self.capture(id);
        }
        let last_input = self.machine.last_input(now);
        let machine = self.machine.clone();
        let outcome = self.blocks.tick(now, last_input, &mut || machine.random_fraction());
        self.record(outcome.finished, Speed::Normal);
        if let Some(since) = outcome.went_idle {
            self.went_away(since);
        }
        self.housekeeping();
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
            // A new block from now on.
            self.reconcile(now, EndReason::Sleep);
        }
    }

    /// No input for the idle threshold: the block ended at the last input.
    fn went_away(&mut self, since: DateTime<Utc>) {
        tracing::info!(%since, "no input for the idle threshold; away");
        // An earlier question nobody answered: no answer means discard.
        self.prompt = None;
        self.away = Some(Away { since, until: None, after_break: false, session: self.session.clone() });
    }

    fn present(&self) -> bool {
        !(self.locked || self.disconnected || self.suspended.is_some() || self.shutting_down.is_some())
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
                self.prompt = Some(PendingPrompt { from: away.since, to, session });
            }
        }
        tracing::info!(%at, "back at the computer");
        self.reconcile(at, EndReason::Interval);
    }

    /// The screen locked or the machine slept while away: that time is never
    /// tracked, so the time away ends there.
    fn freeze_away(&mut self, at: DateTime<Utc>) {
        if let Some(away) = self.away.as_mut() {
            away.until.get_or_insert(at.max(away.since));
        }
    }

    /// A timed break ran out: tracking resumes where it ended — or, when
    /// nobody is at the computer, with the next input (CONTRACT §4).
    fn check_break_end(&mut self, now: DateTime<Utc>) {
        let Some(pause) = self.pause else {
            return;
        };
        let Some(until) = pause.until.filter(|until| *until <= now) else {
            return;
        };
        self.pause = None;
        self.persist_pause();
        let threshold = self.session.as_ref().and_then(|s| s.policy.idle_threshold);
        let last = self.machine.last_input(now).map_or(pause.since, |last| last.max(pause.since));
        if self.session.is_some() && threshold.is_some_and(|threshold| now - last >= threshold) {
            tracing::info!("the break is over, but nobody is at the computer; resuming with the next input");
            self.away = Some(Away { since: last, until: None, after_break: true, session: None });
        } else {
            // Not inside a sleep (or a lock) that outlasted the break: the
            // time before the machine was last awake again is not work.
            let resume = self.awake_since.map_or(until, |awake| until.max(awake)).min(now);
            tracing::info!("timed pause is over; resuming tracking");
            self.reconcile(resume, EndReason::Interval);
        }
    }

    /// What is due: the queue's housekeeping, refreshing the projects and
    /// the totals.
    fn housekeeping(&mut self) {
        let now = Instant::now();
        if now >= self.next_maintenance {
            self.next_maintenance = now + MAINTENANCE_EVERY;
            let owner = self.account.as_ref().map(Account::owner);
            maintain_queue(&self.store, owner.as_deref());
            publish_queue(&self.store, &self.state, owner.as_deref());
        }
        let in_workspace = self.account.as_ref().is_some_and(|a| a.workspace_id.is_some());
        if !in_workspace || self.expired || self.suspended.is_some() || self.shutting_down.is_some() {
            return;
        }
        if now >= self.next_projects_refresh {
            self.control.send(ControlRequest::RefreshProjects);
            let online = self.reachable && !self.contacting;
            self.next_projects_refresh = now + if online { PROJECTS_REFRESH } else { RECONNECT_EVERY };
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

    fn on_system(&mut self, mut notice: SystemNotice) {
        let at = notice.at;
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
            SystemEvent::SessionEnding => {
                if self.shutting_down.is_none() {
                    self.shutting_down = Some(at);
                    self.freeze_away(at);
                    self.reconcile(at, EndReason::Shutdown);
                }
            }
            SystemEvent::SessionContinues => {
                if self.shutting_down.take().is_some() {
                    self.back_at_computer(at);
                }
            }
        }
        // The block is saved: the window may let Windows go on.
        notice.acknowledge();
    }

    /// Unlocked, reconnected, awake again: someone is at the computer (the
    /// password typed on the lock screen is input the hooks never see).
    fn back_at_computer(&mut self, at: DateTime<Utc>) {
        self.awake_since = Some(at);
        if self.away.is_some() && self.present() {
            self.come_back(at);
        } else {
            self.reconcile(at, EndReason::Interval);
        }
    }

    // ---- The window's commands -------------------------------------------------------------

    fn on_command(&mut self, command: Command) {
        // The user is here: tracking starting later is no surprise.
        self.announce_autostart = false;
        match command {
            Command::SignIn { workspace, username, password, server } => self.sign_in(workspace, username, password, server),
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
        }
        self.state.update(|s| s.pending_action = None);
    }

    fn update_consent(&mut self, consent: crate::config::LocalConsent) {
        tracing::info!(?consent, "local consent update requested by user");
        let now = self.now();
        let prior_consent = self.cfg.consent;

        // Step 1: Enforce withdrawn permissions BEFORE persistence can block.
        // Monotonic per-capability narrowing:
        // - Any capability withdrawn in `consent` becomes false immediately.
        // - No capability newly expanded in `consent` is granted yet.
        // - If `consent` is Revoked/Absent, all permissions are withdrawn immediately.
        let staged_consent = self.cfg.consent.narrow_to(&consent);
        self.cfg.consent = staged_consent;

        // Apply narrowed consent to session policy and active blocks
        if let Some(current_session) = self.session.as_ref() {
            let project_id = current_session.project_id().to_owned();
            if let Some(project) = self.projects.iter().flatten().find(|p| p.id == project_id).cloned() {
                let refreshed = session_for(&project, &self.cfg);
                self.blocks.narrow_block_permissions(&refreshed.policy.perms);
                self.session = Some(refreshed);
            }
        }

        // Quiesce active capture path: clear any pending screenshots if screenshot permission was withdrawn
        let screenshot_permitted = self.session.as_ref().is_some_and(|s| s.policy.perms.screenshot);
        if !screenshot_permitted {
            self.screens.clear();
        }

        // Apply narrowed policy to native sensor drivers immediately BEFORE reconcile / record / persistence
        if self.should_track() {
            if let Some(session) = self.session.as_ref() {
                self.machine.apply_sensor_policy(true, Some(&session.policy.perms));
            }
            self.reconcile(now, EndReason::Interval);
        } else {
            self.machine.apply_sensor_policy(false, None);
            self.reconcile(now, EndReason::Pause);
        }

        // Step 2: Now invoke persistence (which may block on slow I/O or fail)
        match self.consent_store.save(&consent) {
            Ok(()) => {
                // Safely persisted! Now apply the full target consent (including any persisted expansions)
                self.cfg.consent = consent;
                self.consent_error = None;
                self.consent_unsynced = false;
                tracing::info!("consent successfully persisted to disk");

                if let Some(current_session) = self.session.as_ref() {
                    let project_id = current_session.project_id().to_owned();
                    if let Some(project) = self.projects.iter().flatten().find(|p| p.id == project_id).cloned() {
                        let refreshed = session_for(&project, &self.cfg);
                        self.blocks.update_policy(refreshed.policy.clone());
                        self.session = Some(refreshed);
                    }
                } else {
                    self.choose_session(now);
                }

                if self.should_track() {
                    if let Some(session) = self.session.as_ref() {
                        self.machine.apply_sensor_policy(true, Some(&session.policy.perms));
                    }
                    self.reconcile(now, EndReason::Interval);
                } else {
                    self.machine.apply_sensor_policy(false, None);
                    self.reconcile(now, EndReason::Pause);
                }
            }
            Err(e) => {
                // Persistence failed!
                // Establish fail-closed effective state:
                // - Stop all sensors immediately
                // - Halt tracking
                // - Mark consent_unsynced = true to prevent resume
                // - Set displayed consent to fail-closed state (Absent if previously Absent, otherwise Revoked)
                // - Clear actionable error with restart durability warning
                let err_msg = format!(
                    "Failed to persist consent changes to disk: {e}. Tracking stopped (fail-closed) to prevent non-durable sensor operation. Changes/revocation are non-durable across app restart: prior on-disk consent remains and will reload on restart until disk save succeeds."
                );
                tracing::error!(%err_msg);
                self.consent_error = Some(err_msg);
                self.consent_unsynced = true;

                self.cfg.consent = match prior_consent {
                    crate::config::LocalConsent::Absent => crate::config::LocalConsent::Absent,
                    _ => crate::config::LocalConsent::Revoked,
                };
                if let Some(current_session) = self.session.as_ref() {
                    let project_id = current_session.project_id().to_owned();
                    if let Some(project) = self.projects.iter().flatten().find(|p| p.id == project_id).cloned() {
                        let refreshed = session_for(&project, &self.cfg);
                        self.blocks.update_policy(refreshed.policy.clone());
                        self.session = Some(refreshed);
                    }
                }

                self.machine.apply_sensor_policy(false, None);
                self.reconcile(now, EndReason::Pause);
                self.screens.clear();
            }
        }

        self.publish();
    }

    /// A click in the window is input too: an away stretch ends there.
    fn user_acted(&mut self, at: DateTime<Utc>) {
        if self.away.is_some() && self.present() {
            self.come_back(at);
        }
    }

    fn sign_in(&mut self, workspace: Option<String>, username: String, password: String, server: Option<String>) {
        if self.account.is_some() && !self.expired {
            tracing::warn!("sign-in ignored: already signed in");
            return;
        }
        let typed = match server.as_deref().map(str::trim).filter(|s| !s.is_empty()).map(normalize_server_url).transpose() {
            Ok(typed) => typed,
            Err(e) => {
                self.state.update(|s| s.login_error = Some(e.to_string()));
                return;
            }
        };
        // The same user signing in again keeps the server and workspace saved.
        let same = self.saved.as_ref().filter(|saved| saved.same_account(&username));
        let saved_server = same.and_then(|saved| saved.server.clone());
        let saved_workspace = same.and_then(|saved| saved.workspace_id.clone());
        let server = self.cfg.effective_api_base_url(typed.or(saved_server).as_deref());
        self.clear_choices();
        self.preselect = workspace.map(|w| w.trim().to_owned()).filter(|w| !w.is_empty()).or(saved_workspace);
        self.signing_in = true;
        self.state.update(|s| {
            s.signing_in = true;
            s.login_error = None;
        });
        self.control.send(ControlRequest::SignIn(Account::new(server, username, password)));
    }

    /// The server accepted the sign-in: pick the workspace to track in.
    fn signed_in_to_server(&mut self, account: Account, workspaces: Result<Vec<Workspace>, ApiError>) {
        if !self.signing_in {
            return;
        }
        self.signing_in = false;
        let preselect = self.preselect.take();
        let workspaces = match workspaces {
            Ok(workspaces) => workspaces,
            Err(error) => {
                let message = match &error {
                    ApiError::Auth(e) => e.user_message(),
                    ApiError::Offline(_) => AuthError::Offline(String::new()).user_message(),
                    other => format!("Signed in, but your workspaces could not be loaded ({other})."),
                };
                return self.state.update(|s| {
                    s.signing_in = false;
                    s.login_error = Some(message);
                });
            }
        };
        let preselected = preselect.as_deref().and_then(|wanted| {
            workspaces.iter().find(|w| w.id.eq_ignore_ascii_case(wanted)).or_else(|| workspaces.iter().find(|w| w.name.trim().eq_ignore_ascii_case(wanted)))
        });
        match (preselected, workspaces.as_slice()) {
            (_, []) => self.state.update(|s| {
                s.signing_in = false;
                s.login_error = Some(NO_WORKSPACE.to_owned());
            }),
            (Some(workspace), _) | (None, [workspace]) => {
                let account = account.in_workspace(&workspace.id, &workspace.name);
                self.state.update(|s| s.signing_in = false);
                self.signed_in(account);
            }
            (None, _) => {
                self.offer(&workspaces, None);
                self.choices = workspaces;
                self.pending = Some(account);
                self.state.update(|s| s.signing_in = false);
            }
        }
    }

    /// Show `workspaces` in the picker.
    fn offer(&self, workspaces: &[Workspace], current: Option<&str>) {
        let choices: Vec<WorkspaceChoice> = workspaces
            .iter()
            .map(|w| WorkspaceChoice { name: w.name.clone(), role: w.role.clone(), is_current: current == Some(w.id.as_str()) })
            .collect();
        self.state.update(|s| s.workspace_choices = choices);
    }

    /// The workspace picked after signing in.
    fn choose_workspace(&mut self, index: usize) {
        let (Some(workspace), Some(account)) = (self.choices.get(index).cloned(), self.pending.take()) else { return };
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
        if self.account.is_none() || self.expired {
            return;
        }
        self.clear_choices();
        self.state.update(|s| s.finding_workspaces = true);
        self.control.send(ControlRequest::MyWorkspaces);
    }

    fn my_workspaces_found(&mut self, found: Result<Vec<Workspace>, String>) {
        self.state.update(|s| s.finding_workspaces = false);
        let Some(current) = self.account.as_ref().filter(|_| !self.expired).map(|a| a.workspace_id.clone()) else { return };
        let others = |items: &[Workspace]| items.iter().filter(|w| current.as_deref() != Some(w.id.as_str())).count();
        let items = match found {
            Err(message) => return self.state.update(|s| s.workspace_note = Some(message)),
            Ok(items) if items.is_empty() => return self.state.update(|s| s.workspace_note = Some(NO_WORKSPACE.to_owned())),
            Ok(items) if others(&items) == 0 => return self.state.update(|s| s.workspace_note = Some(ONE_WORKSPACE.to_owned())),
            Ok(items) => items,
        };
        self.offer(&items, current.as_deref());
        self.choices = items;
    }

    /// Track in `workspace_choices[index]` from now on: the block so far is
    /// queued in the workspace it was recorded in, and the project is chosen
    /// again. The sign-in (and its token) stays.
    fn switch_to(&mut self, index: usize, at: DateTime<Utc>) {
        let Some(old) = self.account.clone().filter(|_| !self.expired) else { return };
        let Some(target) = self.choices.get(index).cloned() else { return };
        if old.workspace_id.as_deref() == Some(target.id.as_str()) {
            return;
        }
        let now = self.now();
        let finished = self.blocks.end(at.min(now), now, EndReason::SignOut);
        self.record(finished, Speed::Normal);
        self.clear_choices();
        self.use_workspace(old.in_workspace(&target.id, &target.name), now);
    }

    /// Carry on in `account`'s workspace: what belonged to the previous one
    /// (projects, the chosen project, the question about time away) is dropped.
    fn use_workspace(&mut self, account: Account, now: DateTime<Utc>) {
        if let Some(saved) = self.saved.as_mut() {
            saved.workspace_id = account.workspace_id.clone();
            saved.workspace_name = account.workspace_name.clone();
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
        let (name, lost) = (account.workspace_name.clone(), account.workspace_id.is_none());
        self.state.update(|s| {
            s.workspace = if lost { String::new() } else { name };
            s.projects.clear();
            s.selected_project = None;
            s.tracked_today_secs = 0.0;
            s.tracked_week_secs = 0.0;
            s.week_day_secs = [0.0; 7];
            s.week_days_fetched = None;
            s.last_productivity = 0;
            s.capture = CaptureFlags::default();
            s.last_error = None;
            if !lost {
                s.workspace_lost = None;
            }
        });
        self.account = Some(account.clone());
        self.control.send(ControlRequest::UseAccount(Some(account.clone())));
        self.uploader.send(UploadCommand::Account(Some(account.clone())));
        if account.workspace_id.is_some() {
            self.control.send(ControlRequest::RefreshProjects);
            self.next_projects_refresh = Instant::now() + RECONNECT_EVERY;
            self.totals_wanted = true;
        }
        self.choose_session(now);
        self.reconcile(now, EndReason::Interval);
    }

    /// The account lost access to the workspace it tracks in: tracking stops
    /// (what was recorded waits in the queue) and another workspace is asked for.
    fn workspace_lost(&mut self, workspace_id: &str) {
        let Some(account) = self.account.clone().filter(|_| !self.expired) else { return };
        if account.workspace_id.as_deref() != Some(workspace_id) {
            return;
        }
        let now = self.now();
        let name = if account.workspace_name.trim().is_empty() { "this workspace".to_owned() } else { account.workspace_name.clone() };
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

    /// A workspace was chosen after signing in. The same user in the same
    /// workspace (the password entered again after it expired) keeps
    /// everything it remembered; otherwise it starts afresh.
    fn signed_in(&mut self, account: Account) {
        let now = self.now();
        self.signing_in = false;
        let same = self
            .saved
            .as_ref()
            .is_some_and(|saved| saved.owner_key() == account.owner() && saved.workspace_id == account.workspace_id);
        if !same && self.account.is_some() {
            self.forget_account(now);
        }
        let mut saved = match self.saved.take() {
            Some(saved) if same => saved,
            _ => Credentials::new(String::new(), String::new(), None),
        };
        saved.workspace_id = account.workspace_id.clone();
        saved.workspace_name = account.workspace_name.clone();
        saved.username = account.username.clone();
        saved.password = account.password.clone();
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
        publish_queue(&self.store, &self.state, Some(&owner));
        let (name, workspace, server) = (account.username.clone(), account.workspace_name.clone(), account.server.clone());
        self.state.update(|s| {
            s.display_name = name;
            s.workspace = workspace;
            s.server_url = server;
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

    fn sign_in_failed(&mut self, error: &AuthError) {
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

    /// The server refused the saved password (changed, reset, the account
    /// deactivated): tracking stops, the queue is kept for when it is entered
    /// again (CONTRACT §1).
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
        if self.account.is_some() {
            // The work so far is this account's: queue it (and upload it)
            // before letting go.
            let finished = self.blocks.end(at, now, EndReason::SignOut);
            self.record(finished, Speed::Normal);
            let heartbeat = if self.expired { None } else { self.heartbeat(PresenceStatus::SignedOut, now, now) };
            self.control.farewell_later(heartbeat);
            self.uploader.send(UploadCommand::SignOut { deadline: std::time::Instant::now() + SIGN_OUT_UPLOAD_GRACE });
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
    fn forget_account(&mut self, now: DateTime<Utc>) {
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
            s.server_url = server;
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

    fn select_project(&mut self, project_id: &str, at: DateTime<Utc>) {
        if self.session.as_ref().is_some_and(|s| s.project_id() == project_id) {
            return;
        }
        // The saved list works offline too.
        let Some(project) = self.projects.iter().flatten().find(|p| p.id == project_id).cloned() else {
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
        let until = resume_after.and_then(|d| TimeDelta::from_std(d).ok()).map(|d| at + d);
        match self.pause.as_mut() {
            // Already paused: only when (or whether) it ends changes.
            Some(pause) => pause.until = until,
            None => {
                self.away = None;
                self.pause = Some(SavedPause { since: at, until });
                let reason = if until.is_some() { EndReason::Break } else { EndReason::Pause };
                self.reconcile(at, reason);
            }
        }
        self.persist_pause();
    }

    fn resume(&mut self, at: DateTime<Utc>) {
        if self.pause.take().is_some() {
            self.persist_pause();
            // Not a second of the pause (or input made during it) is billed:
            // a fresh block starts at the click.
            self.reconcile(at, EndReason::Interval);
        }
    }

    /// Save the block in progress now (if it is a minute old) and upload at once.
    fn sync_now(&mut self, at: DateTime<Utc>) {
        let now = self.now();
        let old_enough = self.blocks.stretch().is_some_and(|s| at.min(now) - s.block.started_at >= MIN_BLOCK);
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
        match self.blocks.stretch().map(|s| at.min(now) - s.block.started_at < MIN_BLOCK) {
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
            tracing::info!(minutes = (prompt.to - prompt.from).num_minutes(), "time away discarded");
            return;
        }
        // Also while the sign-in is expired: the queue keeps the account's
        // rows for when the password is entered again.
        let Some((owner, workspace)) = self.account.as_ref().and_then(|a| Some((a.owner(), a.workspace_id.clone()?))) else {
            tracing::info!("time away not kept: signed out");
            return;
        };
        let now = self.now();
        let interval = prompt.session.policy.interval.to_std().unwrap_or(Duration::from_secs(600));
        let blocks = self.machine.kept_idle_blocks(prompt.session.project_id(), prompt.from, prompt.to, interval, self.task_note.as_deref());
        let mut queued = 0;
        for activity in &blocks {
            if self.enqueue(activity, &owner, &workspace, now) {
                queued += 1;
            }
        }
        enforce_queue_limits(&self.store);
        publish_queue(&self.store, &self.state, Some(&owner));
        if queued > 0 {
            self.uploader.send(UploadCommand::RowsQueued);
        }
        tracing::info!(queued, "time away kept as work");
    }

    /// Quit: save the block in progress first — so whatever happens next it
    /// goes up on the next launch — then say "stopped" and upload what fits.
    async fn quit(&mut self, at: DateTime<Utc>) {
        // Immediately halt all OS sensors before farewell heartbeats and upload draining
        self.machine.apply_sensor_policy(false, None);
        let started = std::time::Instant::now();
        let now = self.now();
        let finished = self.blocks.end(at, now, EndReason::Quit);
        self.record(finished, Speed::Normal);
        // An unanswered question about time away: no answer means discard.
        self.prompt = None;
        let heartbeat = if self.account.is_some() && !self.expired {
            self.heartbeat(PresenceStatus::Stopped, now, now)
        } else {
            None
        };
        self.control.farewell(heartbeat, FAREWELL_WAIT).await;
        let deadline = started + QUIT_UPLOAD_GRACE;
        self.uploader.send(UploadCommand::Shutdown { deadline });
        self.uploader.finish(deadline + Duration::from_secs(1)).await;
    }

    // ---- What the other parts report ------------------------------------------------------------

    fn on_control(&mut self, event: ControlEvent) {
        match event {
            ControlEvent::SignedIn { account, workspaces } => self.signed_in_to_server(account, workspaces),
            ControlEvent::SignInFailed(error) => self.sign_in_failed(&error),
            ControlEvent::MyWorkspaces(found) => self.my_workspaces_found(found),
            ControlEvent::WorkspaceLost(workspace) => self.workspace_lost(&workspace),
            ControlEvent::Projects(projects) => {
                if self.account.as_ref().is_some_and(|a| a.workspace_id.is_some()) && !self.expired {
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

    fn on_upload(&mut self, event: UploadEvent) {
        match event {
            // A pass of the previous account (a sign-out's final drain) may
            // report after another account signed in: not about this one.
            UploadEvent::SignInExpired { owner, .. }
            | UploadEvent::Projects { owner, .. }
            | UploadEvent::Parked { owner, .. }
            | UploadEvent::WorkspaceLost { owner, .. }
                if !self.is_current_owner(&owner) =>
            {
                tracing::debug!("an upload event of the previous account ignored");
            }
            UploadEvent::SignInExpired { reason, .. } => self.expire(&reason),
            UploadEvent::WorkspaceLost { workspace_id, .. } => self.workspace_lost(&workspace_id),
            UploadEvent::Projects { projects, .. } => {
                if !self.expired && self.account.as_ref().is_some_and(|a| a.workspace_id.is_some()) {
                    self.apply_projects(projects);
                }
            }
            UploadEvent::Uploaded(count) => {
                tracing::debug!(count, "blocks reached the server");
                self.totals_wanted = true;
            }
            // After a 403/409 the projects are fetched again (CONTRACT §2).
            UploadEvent::Parked { count, .. } => {
                tracing::info!(count, "blocks rejected by the server; checking the projects again");
                self.next_projects_refresh = Instant::now();
            }
        }
    }

    /// Whether `owner` is the account signed in now.
    fn is_current_owner(&self, owner: &str) -> bool {
        self.account.as_ref().is_some_and(|account| account.owner() == owner)
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
            paused: self.pause.is_some(),
            has_session: self.session.is_some(),
            no_workspace: self.account.as_ref().is_some_and(|a| a.workspace_id.is_none()),
            projects_known: self.projects.is_some(),
            contacting: self.contacting || self.signing_in,
            locked: self.locked || self.disconnected,
            away: self.away.is_some(),
            suspended: self.suspended.is_some() || self.shutting_down.is_some(),
            tracking: self.blocks.is_open(),
            consent: self.cfg.consent,
        }
    }

    fn view(&self) -> View {
        let status = session::status(&self.facts());
        let stretch = self.blocks.stretch().filter(|_| status == TrackerStatus::Tracking);
        let pause = self.pause.filter(|_| status == TrackerStatus::Paused);
        View {
            status,
            tracking_since: stretch.map(|s| s.since),
            block_started_at: stretch.map(|s| s.block.started_at),
            unsaved_since: stretch.map(Stretch::unsaved_since),
            next_capture_at: stretch.map(|s| s.block.planned_end()),
            paused_since: pause.map(|p| p.since),
            resume_at: pause.and_then(|p| p.until),
            away_since: self.away.as_ref().filter(|_| status == TrackerStatus::Away).map(|a| a.since),
            away_prompt: self.prompt.as_ref().map(|p| AwayPrompt { from: p.from, to: p.to, project_title: p.session.title.clone() }),
            selected_project: self.session.as_ref().map(|s| s.project_id().to_owned()),
            // Before a project is resolved, the configured interval.
            interval_minutes: self.session.as_ref().map_or(self.cfg.interval().as_secs() / 60, Session::interval_minutes),
            idle_threshold_minutes: self.session.as_ref().and_then(Session::idle_threshold_minutes),
            capture: self.session.as_ref().map(Session::capture_flags).unwrap_or_default(),
            task_note: self.task_note.clone().unwrap_or_default(),
            server_reachable: self.reachable,
            clock_skew_minutes: self.clock.skew_minutes(),
            consent: self.cfg.consent,
            consent_error: self.consent_error.clone(),
            consent_unsynced: self.consent_unsynced,
        }
    }

    /// Publish the status and the clocks the window animates (only what
    /// changed wakes it), and send a heartbeat when the presence changed or
    /// one is due.
    fn publish(&mut self) {
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
            });
            self.published = Some(view);
            (self.repaint)();
        }
        self.maybe_heartbeat(status);
    }

    /// Presence (CONTRACT §3): every minute while signed in, and at once on
    /// every change of status or project. Never queued.
    fn maybe_heartbeat(&mut self, status: TrackerStatus) {
        let timed_break = self.pause.is_some_and(|p| p.until.is_some());
        let in_workspace = self.account.as_ref().is_some_and(|a| a.workspace_id.is_some());
        let online = in_workspace && !self.expired && self.suspended.is_none() && self.shutting_down.is_none();
        let Some(presence) = session::presence(status, timed_break).filter(|_| online) else {
            return;
        };
        let now = self.now();
        let key = (presence, self.session.as_ref().map(|s| s.project_id().to_owned()));
        let changed = self.presence_sent.as_ref() != Some(&key);
        if changed {
            self.presence_since = now;
        }
        if changed || Instant::now() >= self.next_heartbeat {
            if let Some(heartbeat) = self.heartbeat(presence, self.presence_since, now) {
                self.control.send(ControlRequest::Heartbeat(Box::new(heartbeat)));
            }
            self.presence_sent = Some(key);
            self.next_heartbeat = Instant::now() + HEARTBEAT_EVERY;
        }
    }

    /// A heartbeat saying `status` since `since`; `None` without a device id
    /// (the server needs one).
    fn heartbeat(&self, status: PresenceStatus, since: DateTime<Utc>, now: DateTime<Utc>) -> Option<Heartbeat> {
        let device = self.machine.device();
        let device_id = device.device_id.clone()?;
        let session = self.session.as_ref();
        // The app in front only while tracking, and only when the project
        // records the active window.
        let current_app = if status == PresenceStatus::Tracking && session.is_some_and(|s| s.policy.perms.active_window) {
            self.machine.current_app()
        } else {
            None
        };
        Some(Heartbeat {
            device_id,
            device_name: device.device_name.clone(),
            platform: Some(device.platform.to_owned()),
            os_version: device.os_version.clone(),
            agent_version: Some(device.agent_version.to_owned()),
            status,
            project_id: session.map(|s| s.project_id().to_owned()),
            status_since: Some(since),
            current_app,
            time_zone: crate::tracking::system::time_zone(),
            utc_offset_minutes: Some(crate::tracking::system::utc_offset_minutes(now)),
            sent_at: None,
        })
    }

    // ---- Persistence ----------------------------------------------------------------------------------

    fn persist(&self) {
        let (Some(path), Some(saved)) = (&self.credentials_file, &self.saved) else {
            return;
        };
        let mut saved = saved.clone();
        if let Some(refresh_token) = self.account.as_ref().and_then(|a| a.tokens.refresh_token()) {
            saved.refresh_token = Some(refresh_token);
        }
        if let Err(e) = credentials::save_at(path, &saved) {
            tracing::warn!(?e, "could not save the sign-in");
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

/// Did the server refuse these credentials (as opposed to being unreachable
/// or failing)?
fn is_auth_rejection(e: &AuthError) -> bool {
    e.is_rejected()
}

fn friendly_login_error(e: &AuthError) -> String {
    e.user_message()
}

/// Housekeeping that bounds the local queue: expire parked rows and other
/// accounts' stale rows, then shed images/rows beyond [`QueueLimits::DEFAULT`].
fn maintain_queue(store: &Storage, owner: Option<&str>) {
    match store.purge_rejected_older_than(REJECTED_RETENTION_DAYS) {
        Ok(n) if n > 0 => tracing::info!(purged = n, "removed old rejected activities"),
        Err(e) => tracing::warn!(?e, "could not purge rejected activities"),
        _ => {}
    }
    match store.purge_foreign_pending_older_than(owner, REJECTED_RETENTION_DAYS) {
        Ok(n) if n > 0 => tracing::info!(purged = n, "removed expired activities of other accounts"),
        Err(e) => tracing::warn!(?e, "could not purge other accounts' activities"),
        _ => {}
    }
    enforce_queue_limits(store);
}

fn enforce_queue_limits(store: &Storage) {
    match store.enforce_limits(&QueueLimits::DEFAULT) {
        Ok(report) if report.stripped > 0 || report.dropped > 0 => tracing::warn!(
            stripped = report.stripped,
            dropped = report.dropped,
            "offline queue over its limits; shed images / oldest activities"
        ),
        Err(e) => tracing::warn!(?e, "could not enforce offline queue limits"),
        _ => {}
    }
}

/// Show the signed-in account's queue: pending / rejected counts and the
/// newest rows with the server's reasons (nothing when signed out).
fn publish_queue(store: &Storage, state: &StateHandle, owner: Option<&str>) {
    let (pending, rejected, rows) = match owner {
        Some(owner) => (
            store.pending_count(owner).unwrap_or(0),
            store.rejected_count(owner).unwrap_or(0),
            store.recent_rows(owner, QUEUE_ROWS_SHOWN).unwrap_or_default(),
        ),
        None => (0, 0, Vec::new()),
    };
    state.update(|s| {
        s.pending_uploads = pending;
        s.rejected_uploads = rejected;
        s.queue_rows = rows;
    });
}

/// The window's summary of a block just queued.
fn recent_block(row_id: i64, project: String, activity: &Activity) -> RecentBlock {
    RecentBlock {
        row_id,
        project,
        started_at: activity.started_at,
        ended_at: activity.ended_at,
        productivity: activity.productivity.unwrap_or(0),
        keyboard_hits: activity.keyboard_hits,
        mouse_clicks: activity.mouse_clicks,
        screenshot: activity.screenshot_png_base64.is_some(),
        upload: UploadState::Queued,
    }
}

/// Newest first, bounded to [`RECENT_BLOCKS`].
fn push_recent_block(blocks: &mut Vec<RecentBlock>, block: RecentBlock) {
    blocks.insert(0, block);
    blocks.truncate(RECENT_BLOCKS);
}

/// Record where a queued block is now (uploaded / rejected), if it is listed.
fn set_upload_state(state: &StateHandle, row_id: i64, upload: UploadState) {
    state.update(|s| {
        if let Some(block) = s.recent_blocks.iter_mut().find(|b| b.row_id == row_id) {
            block.upload = upload;
        }
    });
}

/// The screenshot's small JPEG, decoded for the window's preview.
fn decode_thumbnail(activity: &Activity) -> Option<Vec<u8>> {
    use base64::Engine as _;
    let encoded = activity.screenshot_thumb_jpg_base64.as_deref()?;
    base64::engine::general_purpose::STANDARD.decode(encoded).ok()
}

/// Each local day of this week before today as (slot, start, end): the slot
/// is the day's index from Monday, and the day runs `[start, end)`. Empty on
/// a Monday.
pub fn past_days_of_week<Tz: TimeZone>(now: DateTime<Utc>, tz: &Tz) -> Vec<(usize, DateTime<Utc>, DateTime<Utc>)> {
    let today = now.with_timezone(tz).date_naive();
    let monday = today - chrono::Days::new(u64::from(today.weekday().num_days_from_monday()));
    monday
        .iter_days()
        .take_while(|day| *day < today)
        .enumerate()
        .filter_map(|(slot, day)| Some((slot, local_midnight(day, tz)?, local_midnight(day.succ_opt()?, tz)?)))
        .collect()
}

/// Seconds of `started_at .. ended_at` that fall on or after `since` (e.g.
/// today's local midnight), so a block spanning midnight only adds today's
/// part to "today".
pub fn tracked_secs_since(started_at: DateTime<Utc>, ended_at: DateTime<Utc>, since: DateTime<Utc>) -> f64 {
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
pub fn local_day_and_week_start<Tz: TimeZone>(now: DateTime<Utc>, tz: &Tz) -> (DateTime<Utc>, DateTime<Utc>) {
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
        .find_map(|hour| tz.from_local_datetime(&date.and_hms_opt(hour, 0, 0)?).earliest())
        .map(|start| start.with_timezone(&Utc))
}

#[cfg(test)]
mod tests;

