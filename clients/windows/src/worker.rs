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
//!
//! This module holds the loop, the [`Worker`] state and its start; the
//! worker's methods live beside it by concern: [`account`] (signing in and
//! out, the saved sign-in), [`workspaces`], [`projects`], [`presence`] (idle,
//! breaks, lock, sleep, the tick), [`recording`] and [`queue`], [`consent`],
//! [`commands`] (the window's), [`events`] (what the control task and the
//! uploader report) and [`publish`] (the window's view, the heartbeat). The
//! block clock is [`blocks`], the session rules [`session`], local days
//! [`calendar`], and the machine [`machine`].

mod account;
mod blocks;
mod calendar;
mod commands;
mod consent;
mod control;
mod events;
mod machine;
mod presence;
mod projects;
mod publish;
mod queue;
mod recording;
mod session;
mod uploader;
mod workspaces;

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, TimeDelta, Utc};
use tokio::sync::mpsc::{self, UnboundedReceiver, UnboundedSender};
use tokio::time::Instant;

use crate::api::{ServerClock, Workspace};
use crate::config::{AppConfig, ConsentStore, FileConsentStore};
use crate::credentials::{self, Credentials, SavedPause};
use crate::model::{EndReason, PresenceStatus, Project};
use crate::power::SleepDetector;
use crate::session_events::SystemNotice;
use crate::state::{Command, NoticeKind, StateHandle, TrackerStatus};
use crate::storage::Storage;
use crate::tracking::{ProvenanceScreenCapture, Tracker};

pub use account::Account;
use blocks::{BlockId, Blocks};
pub use calendar::{local_day_and_week_start, past_days_of_week, tracked_secs_since};
pub use commands::{clean_note, TASK_NOTE_MAX_CHARS};
use control::{ControlLink, ControlRequest};
pub use machine::Machine;
use publish::View;
use queue::{maintain_queue, publish_queue, set_upload_state};
use session::{restored_pause, Session};
use uploader::{UploadCommand, UploaderHandle};
pub use workspaces::{ALL_ARCHIVED, NO_WORKSPACE};

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

/// After this many blocks in a row could not be saved locally (a full disk,
/// a locked queue file), the status turns red: recording is not working.
const ENQUEUE_FAILURES_SHOWN: u32 = 2;

/// How many queue rows the window lists.
const QUEUE_ROWS_SHOWN: u32 = 20;

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
    // (and per-server) tagging.
    let legacy_owner = saved.as_ref().map(|saved| saved.owner_key(&cfg.effective_api_base_url(saved.server.as_deref())));

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
    /// When Windows' idle clock first showed input again, while the input
    /// hooks confirm it was the user's own (a mouse jiggler moves that clock
    /// too). The time away ends here when they do so right after; later, it
    /// ends at the input they saw.
    noticed: Option<DateTime<Utc>>,
}

/// The time away, waiting for "Keep as work" or "Discard".
#[derive(Debug, Clone)]
struct PendingPrompt {
    from: DateTime<Utc>,
    to: DateTime<Utc>,
    session: Session,
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
    /// The server refused the saved sign-in.
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
    /// Blocks in a row the local queue refused to save.
    enqueue_failures: u32,
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
            enqueue_failures: 0,
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
        let owner = saved.as_ref().map(|saved| saved.owner_key(&self.cfg.effective_api_base_url(saved.server.as_deref())));
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
            self.state.update(|s| s.set_server(server));
            return;
        };
        let pause = restored_pause(saved.pause, now, self.autostart);
        let pause_changed = pause != saved.pause;
        saved.pause = pause;
        self.pause = pause;
        self.task_note = saved.task_note.as_deref().and_then(clean_note);
        let account = Account::of(&saved, &self.cfg, self.machine.device().device_id.clone());
        self.release_held(&account);
        // Offline first: the saved list is used at once; the server's answer
        // replaces it shortly.
        self.projects = saved.usable_projects(now).map(<[Project]>::to_vec);
        let (name, workspace, server) = (saved.username.clone(), saved.workspace_name.clone(), account.server.clone());
        self.state.update(|s| {
            s.display_name = name;
            s.workspace = workspace;
            s.set_server(server);
        });
        // A file in an older format (one that held the password, or without
        // this app's DPAPI entropy) is rewritten at once, without it; the
        // password stays in memory only for its one-time exchange.
        let rewrite = pause_changed || saved.outdated_format;
        self.saved = Some(saved);
        if rewrite {
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
}
#[cfg(test)]
mod tests;
