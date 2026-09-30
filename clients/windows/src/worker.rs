//! Background tracking worker.
//!
//! Owns everything slow or fallible (network, capture, SQLite) on its own thread
//! with a single-threaded tokio runtime, so the UI thread never blocks. The UI
//! talks to it only through a command channel and reads a shared state snapshot.

use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Datelike, Local, NaiveDate, TimeZone, Utc};
use tokio::sync::mpsc;

use crate::api::{ApiClient, SubmitError};
use crate::config::{AppConfig, CapturePermissions};
use crate::credentials::{self, Credentials};
use crate::state::{Command, ProjectOption, StateHandle, TrackerStatus};
use crate::storage::{self, QueueLimits, Storage};
use crate::tracking::Tracker;

/// Server-resolved tracking session: which project, which permissions, how often.
struct Session {
    project_id: String,
    perms: CapturePermissions,
    interval: Duration,
}

/// Spawn the worker thread. Returns the channel the UI uses to control it and
/// the thread's handle, so quitting can wait for the last block to be saved.
pub fn spawn(
    state: StateHandle,
    cfg: AppConfig,
    tracker: Arc<Tracker>,
    repaint: impl Fn() + Send + 'static,
) -> (mpsc::UnboundedSender<Command>, std::thread::JoinHandle<()>) {
    let (tx, rx) = mpsc::unbounded_channel();

    let handle = std::thread::Builder::new()
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
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                runtime.block_on(run(state_for_run, cfg, tracker, rx, &repaint));
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

    (tx, handle)
}

/// How long parked (server-rejected) rows are kept for diagnostics before the
/// queue reclaims their space. Queued rows of an account that is not signed in
/// wait this long for it to come back, then are purged too.
const REJECTED_RETENTION_DAYS: u32 = 14;

/// Rows read from the queue per round trip — each can embed a base64
/// screenshot, so the backlog is never loaded into memory at once.
const UPLOAD_BATCH: u32 = 20;

/// Upper bound on uploads per sync pass. Commands (Pause, Quit) wait while a
/// pass runs, so a large offline backlog drains over a few intervals instead
/// of stalling them for minutes.
const MAX_UPLOADS_PER_PASS: u32 = 100;

/// A block shorter than this is not a session of its own: "Sync now" leaves it
/// running, and pausing, switching project, signing out or quitting drops it
/// instead of recording a seconds-long block scored 100% from a single click.
const MIN_BLOCK: chrono::TimeDelta = chrono::TimeDelta::seconds(60);

/// Stand-in wait for the auto-resume branch while no timed pause is armed
/// (the branch is disabled then; the value only has to be valid).
const NO_AUTO_RESUME: Duration = Duration::from_secs(24 * 60 * 60);

/// The worker's clocks: whether and until when tracking is paused, and where
/// the current stretch and block began. Published to the window by
/// [`apply_status`], which animates the timers from them.
struct Clocks {
    /// `Some` while paused.
    paused_since: Option<DateTime<Utc>>,
    /// When a timed pause ends by itself.
    pause_until: Option<DateTime<Utc>>,
    /// Start of the uninterrupted tracking stretch (the session timer).
    tracking_since: DateTime<Utc>,
    /// Real start of the block currently being accumulated. Captures are timed
    /// from this, never from an assumed "exactly one interval ago".
    block_started_at: DateTime<Utc>,
    next_capture_at: DateTime<Utc>,
}

impl Clocks {
    fn new() -> Self {
        let now = Utc::now();
        Self {
            paused_since: None,
            pause_until: None,
            tracking_since: now,
            block_started_at: now,
            next_capture_at: now,
        }
    }

    fn paused(&self) -> bool {
        self.paused_since.is_some()
    }

    /// Begin a fresh tracking stretch now (sign-in, resume, project switch):
    /// a new block, the session timer from zero, and a full interval until the
    /// first automatic snapshot. Input made before now never counts, because
    /// every capture only reads input inside its own block.
    fn start_tracking(&mut self, ticker: &mut tokio::time::Interval) {
        let now = Utc::now();
        self.paused_since = None;
        self.pause_until = None;
        self.tracking_since = now;
        self.block_started_at = now;
        ticker.reset();
        self.next_capture_at = next_capture(ticker);
    }
}

async fn run(
    state: StateHandle,
    cfg: AppConfig,
    tracker: Arc<Tracker>,
    mut rx: mpsc::UnboundedReceiver<Command>,
    repaint: &(impl Fn() + Send),
) {
    // The saved sign-in decides whose queued rows are shown and uploadable, and
    // claims rows queued by builds that predate per-account tagging.
    let saved = credentials::load();
    let saved_owner = saved
        .as_ref()
        .map(|c| storage::owner_key(&c.workspace, &c.username));

    let store = match Storage::open(&storage_path(), saved_owner.as_deref()) {
        Ok(s) => s,
        Err(e) => {
            tracing::error!(?e, "failed to open the local queue");
            state.update(|s| s.last_error = Some(format!("Local storage unavailable: {e}")));
            repaint();
            return;
        }
    };

    // Reclaim space (old rejects, other accounts' expired rows, over-limit
    // images), and show what is left.
    maintain_queue(&store, saved_owner.as_deref());
    refresh_queue_counts(&store, &state, saved_owner.as_deref());

    let mut client: Option<ApiClient> = None;
    let mut session: Option<Session> = None;
    let mut clocks = Clocks::new();

    // Restore a previous sign-in so a relaunch (e.g. at login) resumes silently.
    if let Some(saved) = saved {
        state.update(|s| s.status = TrackerStatus::Connecting);
        repaint();
        let mut restored = ApiClient::new(
            cfg.api_base_url.clone(),
            saved.username.clone(),
            saved.password.clone(),
            saved.workspace.clone(),
        );
        match restored.login().await {
            Ok(()) => {
                state.update(|s| {
                    s.display_name = saved.username.clone();
                    s.selected_project = saved.project_id.clone();
                });
                session = resolve_session(&mut restored, &cfg, saved.project_id.as_deref(), &state).await;
                refresh_totals(&mut restored, &state).await;
                client = Some(restored);
            }
            Err(e) => {
                tracing::warn!(?e, "saved sign-in is no longer valid");
                state.update(|s| s.status = TrackerStatus::SignedOut);
            }
        }
    }

    let mut ticker = new_ticker(
        session.as_ref().map(|s| s.interval).unwrap_or_else(|| cfg.interval()),
    );
    ticker.tick().await; // skip the immediate first tick
    clocks.start_tracking(&mut ticker);
    // Only a restored sign-in has anything to show; otherwise the window
    // stays on its initial (signed-out, or debug preview) state.
    if client.is_some() {
        apply_status(&state, &client, &session, &clocks);
        repaint();
    }

    loop {
        // A timed pause resumes by itself; otherwise this branch stays disabled.
        let auto_resume_in = clocks
            .pause_until
            .filter(|_| clocks.paused())
            .map(|at| (at - Utc::now()).to_std().unwrap_or_default());

        tokio::select! {
            Some(command) = rx.recv() => {
                match command {
                    Command::SignIn { workspace, username, password } => {
                        state.update(|s| { s.signing_in = true; s.login_error = None; s.status = TrackerStatus::Connecting; });
                        repaint();

                        let mut candidate = ApiClient::new(
                            cfg.api_base_url.clone(),
                            username.clone(),
                            password.clone(),
                            workspace.clone(),
                        );
                        match candidate.login().await {
                            Ok(()) => {
                                // Only persist once the backend has accepted the credentials.
                                let _ = credentials::save(&Credentials {
                                    workspace,
                                    username: username.clone(),
                                    password,
                                    project_id: None,
                                });
                                state.update(|s| { s.display_name = username; s.signing_in = false; });
                                session = resolve_session(&mut candidate, &cfg, None, &state).await;
                                ticker = new_ticker(
                                    session.as_ref().map(|s| s.interval).unwrap_or_else(|| cfg.interval()),
                                );
                                ticker.tick().await;
                                // Nothing from before this sign-in (signed-out
                                // time, another account) belongs to this block.
                                clocks.start_tracking(&mut ticker);
                                let owner = candidate.owner_key();
                                maintain_queue(&store, Some(&owner));
                                refresh_queue_counts(&store, &state, Some(&owner));
                                refresh_totals(&mut candidate, &state).await;
                                client = Some(candidate);
                            }
                            Err(e) => {
                                state.update(|s| {
                                    s.signing_in = false;
                                    s.status = TrackerStatus::SignedOut;
                                    s.login_error = Some(friendly_login_error(&e));
                                });
                            }
                        }
                    }
                    Command::SignOut => {
                        if let Some(api) = client.as_mut() {
                            // The work so far is this account's: queue and upload
                            // it before letting go.
                            if let Some(active) = session.as_ref().filter(|_| !clocks.paused()) {
                                save_block_in_progress(&tracker, api, &store, active, &state, clocks.block_started_at);
                            }
                            sync_pending(api, &store, &state).await;
                        }
                        let _ = credentials::clear();
                        client = None;
                        session = None;
                        // A pause belongs to this sign-in, not the next one.
                        clocks.paused_since = None;
                        clocks.pause_until = None;
                        // This account's queued rows stay (owner-tagged) until it
                        // signs in again; they are just not shown to anyone else.
                        refresh_queue_counts(&store, &state, None);
                        state.update(|s| {
                            s.display_name.clear();
                            s.projects.clear();
                            s.selected_project = None;
                            s.tracked_today_secs = 0.0;
                            s.tracked_week_secs = 0.0;
                            s.last_productivity = 0;
                            s.status = TrackerStatus::SignedOut;
                        });
                    }
                    Command::SelectProject(project_id) => {
                        if let Some(api) = client.as_mut() {
                            // The work so far belongs to the project it was done on.
                            if let Some(active) = session.as_ref().filter(|_| !clocks.paused()) {
                                save_block_in_progress(&tracker, api, &store, active, &state, clocks.block_started_at);
                            }
                            session = resolve_session(api, &cfg, Some(&project_id), &state).await;
                            if let Some(active) = &session {
                                ticker = new_ticker(active.interval);
                                ticker.tick().await;
                            }
                            // The new project starts a fresh block (and, unless
                            // paused, a fresh session timer).
                            if !clocks.paused() {
                                clocks.start_tracking(&mut ticker);
                            }
                            // Remember the choice across restarts.
                            if let Some(mut saved) = credentials::load() {
                                saved.project_id = Some(project_id);
                                let _ = credentials::save(&saved);
                            }
                            sync_pending(api, &store, &state).await;
                            refresh_totals(api, &state).await;
                        }
                    }
                    Command::Pause { resume_after } => {
                        let now = Utc::now();
                        clocks.pause_until = resume_after
                            .and_then(|d| chrono::Duration::from_std(d).ok())
                            .map(|d| now + d);
                        if !clocks.paused() {
                            // Keep the work done so far instead of discarding it:
                            // the block ends here and is queued at once...
                            if let (Some(api), Some(active)) = (client.as_ref(), session.as_ref()) {
                                save_block_in_progress(&tracker, api, &store, active, &state, clocks.block_started_at);
                            }
                            clocks.paused_since = Some(Utc::now());
                            // ...the window shows the pause immediately...
                            state.update(|s| s.pending_action = None);
                            apply_status(&state, &client, &session, &clocks);
                            repaint();
                            // ...and the upload happens while already paused.
                            if let Some(api) = client.as_mut() {
                                sync_pending(api, &store, &state).await;
                                refresh_totals(api, &state).await;
                            }
                        }
                    }
                    Command::Resume => {
                        if clocks.paused() {
                            // Not a second of the paused stretch (or input made
                            // during it) is billed: a fresh block starts now.
                            clocks.start_tracking(&mut ticker);
                        }
                    }
                    Command::SyncNow => {
                        if let Some(api) = client.as_mut() {
                            match session.as_ref() {
                                Some(active) if !clocks.paused() && Utc::now() - clocks.block_started_at >= MIN_BLOCK => {
                                    clocks.block_started_at =
                                        capture_and_sync(&tracker, api, &store, active, &state, clocks.block_started_at).await;
                                    // The next automatic block runs a full interval from
                                    // this cut, instead of a sliver up to the old tick.
                                    ticker.reset();
                                    clocks.next_capture_at = next_capture(&ticker);
                                }
                                // Paused means nothing is captured, not even on
                                // demand: only already-queued rows are uploaded.
                                // So is a block under a minute old: it keeps
                                // running rather than becoming a seconds-long
                                // session scored 100% from a single click.
                                _ => sync_pending(api, &store, &state).await,
                            }
                            refresh_totals(api, &state).await;
                        }
                    }
                    Command::Shutdown => {
                        if let Some(api) = client.as_mut() {
                            // Quitting must not throw the block in progress away.
                            // It is queued locally first, so even if the upload
                            // is cut short it goes up on the next launch.
                            if let Some(active) = session.as_ref().filter(|_| !clocks.paused()) {
                                save_block_in_progress(&tracker, api, &store, active, &state, clocks.block_started_at);
                            }
                            sync_pending(api, &store, &state).await;
                        }
                        return;
                    }
                }
                state.update(|s| s.pending_action = None);
                apply_status(&state, &client, &session, &clocks);
                repaint();
            }

            _ = ticker.tick() => {
                clocks.next_capture_at = next_capture(&ticker);
                if client.is_some() && !clocks.paused() {
                    if session.is_none() {
                        if let Some(api) = client.as_mut() {
                            // Backend was unreachable (or no project yet) — keep retrying.
                            session = resolve_session(api, &cfg, None, &state).await;
                        }
                    }

                    if let (Some(api), Some(active)) = (client.as_mut(), session.as_ref()) {
                        clocks.block_started_at =
                            capture_and_sync(&tracker, api, &store, active, &state, clocks.block_started_at).await;
                        refresh_totals(api, &state).await;
                    }
                }
                apply_status(&state, &client, &session, &clocks);
                repaint();
            }

            _ = tokio::time::sleep(auto_resume_in.unwrap_or(NO_AUTO_RESUME)), if auto_resume_in.is_some() => {
                tracing::info!("timed pause is over; resuming tracking");
                clocks.start_tracking(&mut ticker);
                apply_status(&state, &client, &session, &clocks);
                repaint();
            }
        }
    }
}

/// Build the capture ticker.
///
/// `MissedTickBehavior::Delay` is essential: tokio's default (`Burst`) fires every
/// missed tick back-to-back after a slow sync or a machine suspend. Since each
/// firing records a block, a burst would invent tracked time that was never
/// worked — and that time is what payroll multiplies by an hourly rate.
fn new_ticker(period: Duration) -> tokio::time::Interval {
    let mut ticker = tokio::time::interval(period);
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    ticker
}

/// When the ticker, just ticked or reset, fires next.
fn next_capture(ticker: &tokio::time::Interval) -> DateTime<Utc> {
    Utc::now() + chrono::Duration::from_std(ticker.period()).unwrap_or_else(|_| chrono::Duration::minutes(10))
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

/// Show the signed-in account's pending / rejected counts (zero when signed out).
fn refresh_queue_counts(store: &Storage, state: &StateHandle, owner: Option<&str>) {
    let (pending, rejected) = match owner {
        Some(owner) => (
            store.pending_count(owner).unwrap_or(0),
            store.rejected_count(owner).unwrap_or(0),
        ),
        None => (0, 0),
    };
    state.update(|s| {
        s.pending_uploads = pending;
        s.rejected_uploads = rejected;
    });
}

/// Keep the local queue next to the per-user config, not the working directory —
/// the agent autostarts from arbitrary locations.
fn storage_path() -> String {
    dirs::config_dir()
        .map(|d| {
            let dir = d.join("DosiTracker");
            let _ = std::fs::create_dir_all(&dir);
            dir.join("dosi-tracker.db").to_string_lossy().into_owned()
        })
        .unwrap_or_else(|| "dosi-tracker.db".to_string())
}

fn friendly_login_error(e: &anyhow::Error) -> String {
    let text = e.to_string();
    if text.contains("400") || text.contains("401") {
        "Incorrect email or password.".to_string()
    } else if text.contains("dns") || text.contains("connect") || text.contains("tcp") {
        "Cannot reach the server. Check the API URL and your connection.".to_string()
    } else {
        format!("Sign-in failed: {text}")
    }
}

/// Publish the status and the clocks the window animates. Clocks that do not
/// apply to the status (e.g. the session timer while paused) are cleared.
fn apply_status(
    state: &StateHandle,
    client: &Option<ApiClient>,
    session: &Option<Session>,
    clocks: &Clocks,
) {
    let status = if client.is_none() {
        TrackerStatus::SignedOut
    } else if clocks.paused() {
        TrackerStatus::Paused
    } else if session.is_some() {
        TrackerStatus::Tracking
    } else {
        TrackerStatus::Offline
    };
    let tracking = status == TrackerStatus::Tracking;
    let paused = status == TrackerStatus::Paused;
    state.update(|s| {
        s.status = status;
        s.tracking_since = tracking.then_some(clocks.tracking_since);
        s.block_started_at = tracking.then_some(clocks.block_started_at);
        s.next_capture_at = tracking.then_some(clocks.next_capture_at);
        s.paused_since = if paused { clocks.paused_since } else { None };
        s.resume_at = if paused { clocks.pause_until } else { None };
    });
}

/// Log in (if needed) and pick a project — the requested one, the remembered one,
/// or the first available. Server-side capture permissions win, but never enable
/// what local config forbade.
async fn resolve_session(
    client: &mut ApiClient,
    cfg: &AppConfig,
    preferred: Option<&str>,
    state: &StateHandle,
) -> Option<Session> {
    let projects = match client.projects().await {
        Ok(p) => p,
        Err(e) => {
            tracing::warn!(?e, "could not resolve projects from backend");
            state.update(|s| s.last_error = Some(format!("Could not load projects: {e}")));
            return None;
        }
    };

    state.update(|s| {
        s.projects = projects
            .iter()
            .map(|p| ProjectOption { id: p.id.clone(), title: p.title.clone() })
            .collect();
        s.last_error = None;
    });

    if projects.is_empty() {
        tracing::warn!("logged in, but you are not a member of any active project");
        state.update(|s| s.last_error = Some("You are not a member of any active project.".into()));
        return None;
    }

    let chosen = preferred
        .and_then(|id| projects.iter().find(|p| p.id == id))
        .or_else(|| projects.first())?;

    let perms = CapturePermissions {
        screenshot: chosen.allow_screenshot && cfg.capture.screenshot,
        webcam: chosen.allow_webcam && cfg.capture.webcam,
        keyboard: chosen.allow_keyboard && cfg.capture.keyboard,
        mouse: chosen.allow_mouse && cfg.capture.mouse,
        active_window: chosen.allow_active_window && cfg.capture.active_window,
        running_programs: chosen.allow_running_programs && cfg.capture.running_programs,
    };
    let interval_minutes = chosen.interval_minutes.clamp(5, 60);

    state.update(|s| {
        s.selected_project = Some(chosen.id.clone());
        s.interval_minutes = interval_minutes;
        s.counts_keyboard = perms.keyboard;
        s.counts_mouse = perms.mouse;
    });
    tracing::info!(project = %chosen.title, "tracking session resolved");

    Some(Session {
        project_id: chosen.id.clone(),
        perms,
        interval: Duration::from_secs(interval_minutes * 60),
    })
}

/// Record the block covering `block_started_at .. now` into the local queue
/// and count it in the displayed totals at once (the backend's totals,
/// refreshed after the upload, replace that estimate). Returns the instant the
/// next block starts from.
fn record_block(
    tracker: &Tracker,
    client: &ApiClient,
    store: &Storage,
    session: &Session,
    state: &StateHandle,
    block_started_at: DateTime<Utc>,
) -> DateTime<Utc> {
    let now = Utc::now();

    // Use the real elapsed window, but never claim more than one interval: after
    // a suspend or a long stall the wall-clock gap can be hours, and the user was
    // not working through it.
    let max_span = chrono::Duration::from_std(session.interval).unwrap_or_else(|_| chrono::Duration::minutes(10));
    let earliest = now - max_span;
    let started_at = if block_started_at > earliest { block_started_at } else { earliest };

    // Degenerate window (clock jumped backwards, or a double-fire): skip rather
    // than record a zero/negative-length block the server would reject.
    if started_at >= now {
        tracing::warn!("skipping capture: non-positive time window");
        return now;
    }

    // The block ends exactly where the next one starts, so consecutive blocks
    // never overlap by the time a capture takes.
    let activity = tracker.snapshot(&session.perms, &session.project_id, started_at, now);

    match store.enqueue(&activity, &client.owner_key()) {
        Ok(id) => {
            tracing::debug!(id, "activity queued");
            let (today_start, week_start) = local_day_and_week_start(now, &Local);
            state.update(|s| {
                s.tracked_today_secs += tracked_secs_since(started_at, now, today_start);
                s.tracked_week_secs += tracked_secs_since(started_at, now, week_start);
                // Moved in the same update, so the live total never counts
                // this block twice.
                if s.block_started_at.is_some() {
                    s.block_started_at = Some(now);
                }
            });
        }
        Err(e) => {
            tracing::error!(?e, "failed to queue activity");
            state.update(|s| s.last_error = Some(format!("Could not save locally: {e}")));
        }
    }
    enforce_queue_limits(store);

    // The next block starts where this one ended.
    now
}

/// Capture one block covering `block_started_at .. now`, then sync.
/// Returns the instant the next block starts from.
async fn capture_and_sync(
    tracker: &Tracker,
    client: &mut ApiClient,
    store: &Storage,
    session: &Session,
    state: &StateHandle,
    block_started_at: DateTime<Utc>,
) -> DateTime<Utc> {
    let next = record_block(tracker, client, store, session, state, block_started_at);
    sync_pending(client, store, state).await;
    next
}

/// Queue the block in progress when tracking stops (pause, project switch,
/// sign-out, quit), so stopping never throws worked time away. A block under
/// [`MIN_BLOCK`] is dropped instead. The caller uploads.
fn save_block_in_progress(
    tracker: &Tracker,
    client: &ApiClient,
    store: &Storage,
    session: &Session,
    state: &StateHandle,
    block_started_at: DateTime<Utc>,
) {
    if Utc::now() - block_started_at >= MIN_BLOCK {
        record_block(tracker, client, store, session, state, block_started_at);
    }
}

/// Refresh the today / this-week totals shown in the window.
async fn refresh_totals(client: &mut ApiClient, state: &StateHandle) {
    let now = Utc::now();
    let (today_start, week_start) = local_day_and_week_start(now, &Local);

    // Failures keep the previous values on screen, but are logged: a silently
    // failing request once left these cards at zero with no trace of why.
    match client.summary(today_start, now).await {
        Ok(today) => state.update(|s| {
            s.tracked_today_secs = today.total_tracked_minutes.max(0.0) * 60.0;
            s.last_productivity = today.average_productivity.clamp(0.0, 100.0) as u8;
        }),
        Err(e) => tracing::warn!(?e, "could not load today's totals"),
    }
    match client.summary(week_start, now).await {
        Ok(week) => state.update(|s| s.tracked_week_secs = week.total_tracked_minutes.max(0.0) * 60.0),
        Err(e) => tracing::warn!(?e, "could not load this week's totals"),
    }
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

/// Upload the signed-in account's queued activities, oldest first, in batches.
/// Permanently-rejected payloads are parked so they never block the queue;
/// transient failures are retried next interval. Rows queued by any other
/// account are never read, let alone sent under this account's token.
async fn sync_pending(client: &mut ApiClient, store: &Storage, state: &StateHandle) {
    let owner = client.owner_key();
    let mut cursor = 0;
    let mut attempted = 0;

    'pass: while attempted < MAX_UPLOADS_PER_PASS {
        let limit = UPLOAD_BATCH.min(MAX_UPLOADS_PER_PASS - attempted);
        let batch = match store.pending_batch(&owner, cursor, limit) {
            Ok(batch) => batch,
            Err(e) => {
                tracing::error!(?e, "failed to read pending activities");
                break;
            }
        };
        if batch.is_empty() {
            break;
        }

        for (id, activity) in batch {
            // Paging by id: a row that fails to leave the queue is never
            // retried within the same pass.
            cursor = id;
            attempted += 1;
            match client.submit_activity(&activity).await {
                Ok(()) => {
                    let _ = store.remove_synced(id);
                    state.update(|s| { s.last_sync = Some(Utc::now()); s.last_error = None; });
                    tracing::debug!(id, "activity synced");
                }
                Err(SubmitError::Rejected(reason)) => {
                    tracing::warn!(id, %reason, "server rejected activity; parking it");
                    let _ = store.mark_rejected(id, &reason);
                }
                Err(SubmitError::Transient(e)) => {
                    tracing::warn!(?e, id, "sync failed; will retry next interval");
                    state.update(|s| s.last_error = Some("Offline — uploads will retry.".into()));
                    break 'pass;
                }
            }
        }
    }

    refresh_queue_counts(store, state, Some(&owner));
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::FixedOffset;

    fn utc(s: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc)
    }

    #[test]
    fn today_starts_at_local_midnight_ahead_of_utc() {
        // 02:00 on Thursday 24 Sep in UTC+6 is still Wednesday in UTC.
        let dhaka = FixedOffset::east_opt(6 * 3600).unwrap();
        let (today, week) = local_day_and_week_start(utc("2026-09-23T20:00:00Z"), &dhaka);
        assert_eq!(today, utc("2026-09-23T18:00:00Z")); // Thu 24 Sep 00:00 +06
        assert_eq!(week, utc("2026-09-20T18:00:00Z")); // Mon 21 Sep 00:00 +06
    }

    #[test]
    fn today_starts_at_local_midnight_behind_utc() {
        // 22:00 on Sunday 27 Sep in UTC-5 is already Monday in UTC.
        let bogota = FixedOffset::west_opt(5 * 3600).unwrap();
        let (today, week) = local_day_and_week_start(utc("2026-09-28T03:00:00Z"), &bogota);
        assert_eq!(today, utc("2026-09-27T05:00:00Z")); // Sun 27 Sep 00:00 -05
        assert_eq!(week, utc("2026-09-21T05:00:00Z")); // Mon 21 Sep 00:00 -05
    }

    #[test]
    fn utc_zone_keeps_utc_midnights() {
        let (today, week) = local_day_and_week_start(utc("2026-09-23T12:34:56Z"), &Utc);
        assert_eq!(today, utc("2026-09-23T00:00:00Z"));
        assert_eq!(week, utc("2026-09-21T00:00:00Z"));
    }

    #[test]
    fn only_the_part_of_a_block_after_the_cutoff_is_counted() {
        let midnight = utc("2026-09-30T18:00:00Z");
        // Entirely after: all of it.
        assert_eq!(tracked_secs_since(utc("2026-09-30T18:10:00Z"), utc("2026-09-30T18:15:00Z"), midnight), 300.0);
        // Spanning midnight: only the part after it.
        assert_eq!(tracked_secs_since(utc("2026-09-30T17:58:00Z"), utc("2026-09-30T18:03:30Z"), midnight), 210.0);
        // Entirely before: nothing.
        assert_eq!(tracked_secs_since(utc("2026-09-30T17:00:00Z"), utc("2026-09-30T17:05:00Z"), midnight), 0.0);
    }

    /// End-to-end against a live backend: real input + window sampling and
    /// screenshot → local queue → upload → today/week totals. Run with
    /// `DOSI_E2E_URL`, `DOSI_E2E_WORKSPACE`, `DOSI_E2E_USER`,
    /// `DOSI_E2E_PASSWORD`, `DOSI_E2E_PROJECT` set:
    /// `cargo test e2e_ -- --ignored --nocapture`.
    #[tokio::test]
    #[ignore = "needs a running backend and a desktop session"]
    async fn e2e_capture_upload_and_totals() {
        let env = |key: &str| std::env::var(key).unwrap_or_else(|_| panic!("{key} is not set"));
        let cfg = AppConfig { api_base_url: env("DOSI_E2E_URL"), ..AppConfig::default() };
        let mut client = ApiClient::new(
            cfg.api_base_url.clone(),
            env("DOSI_E2E_USER"),
            env("DOSI_E2E_PASSWORD"),
            env("DOSI_E2E_WORKSPACE"),
        );
        client.login().await.expect("sign-in");

        let state = StateHandle::new();
        let session = resolve_session(&mut client, &cfg, Some(&env("DOSI_E2E_PROJECT")), &state)
            .await
            .expect("project resolved");
        let tracker = Tracker::new();
        let store = Storage::open(":memory:", None).unwrap();
        let owner = client.owner_key();

        // Two minute buckets' worth of real sampling.
        let started = Utc::now();
        state.update(|s| s.block_started_at = Some(started));
        tokio::time::sleep(Duration::from_secs(125)).await;

        record_block(&tracker, &client, &store, &session, &state, started);
        assert_eq!(store.pending_count(&owner).unwrap(), 1, "block queued locally");
        let queued = store.pending_batch(&owner, 0, 1).unwrap().remove(0).1;
        println!(
            "queued: {} windows, {} timeline minutes, screenshot {}, thumb {}, keys {}, clicks {}, {}%",
            queued.active_windows.len(),
            queued.timeline.len(),
            queued.screenshot_png_base64.is_some(),
            queued.screenshot_thumb_jpg_base64.is_some(),
            queued.keyboard_hits,
            queued.mouse_clicks,
            queued.productivity,
        );
        assert_eq!(queued.timeline.len(), 2);
        assert!(queued.active_windows.iter().all(|w| w.seconds.is_some()));
        assert!(queued.screenshot_thumb_jpg_base64.is_some());

        sync_pending(&mut client, &store, &state).await;
        assert_eq!(store.pending_count(&owner).unwrap(), 0, "uploaded");
        assert_eq!(store.rejected_count(&owner).unwrap(), 0, "not rejected");

        state.update(|s| s.tracked_today_secs = 0.0);
        refresh_totals(&mut client, &state).await;
        let totals = state.snapshot();
        println!("server totals: today {:.0}s, week {:.0}s", totals.tracked_today_secs, totals.tracked_week_secs);
        assert!(totals.tracked_today_secs >= 120.0, "today's total includes the block");
    }

    /// End-to-end drive of the worker loop through the commands the window and
    /// tray send: sign-in, a timed pause that saves the block and resumes by
    /// itself, an open-ended pause + resume, a too-short "Sync now", and quit.
    /// Uses the per-user queue and saved sign-in, so run it only on a machine
    /// whose tracker data may be replaced. Same env vars as
    /// [`e2e_capture_upload_and_totals`].
    #[test]
    #[ignore = "needs a running backend; replaces the saved sign-in"]
    fn e2e_pause_resume_flow() {
        use std::time::Instant;
        let env = |key: &str| std::env::var(key).unwrap_or_else(|_| panic!("{key} is not set"));
        let cfg = AppConfig { api_base_url: env("DOSI_E2E_URL"), ..AppConfig::default() };
        let state = StateHandle::new();
        let (tx, worker) = spawn(state.clone(), cfg, Arc::new(Tracker::new()), || {});
        let wait_for = |what: &str, secs: u64, done: &dyn Fn(&crate::state::SharedState) -> bool| {
            let deadline = Instant::now() + Duration::from_secs(secs);
            loop {
                let s = state.snapshot();
                if done(&s) {
                    return s;
                }
                assert!(Instant::now() < deadline, "timed out waiting for {what}: {:?}", s.status);
                std::thread::sleep(Duration::from_millis(200));
            }
        };

        tx.send(Command::SignIn {
            workspace: env("DOSI_E2E_WORKSPACE"),
            username: env("DOSI_E2E_USER"),
            password: env("DOSI_E2E_PASSWORD"),
        })
        .unwrap();
        let s = wait_for("tracking", 30, &|s| s.status == TrackerStatus::Tracking);
        println!("signed in, tracking since {:?}, next capture {:?}", s.tracking_since, s.next_capture_at);

        // Work for over a minute, then take a 5-second timed pause.
        std::thread::sleep(Duration::from_secs(65));
        let synced_before = state.snapshot().last_sync;
        tx.send(Command::Pause { resume_after: Some(Duration::from_secs(5)) }).unwrap();
        let s = wait_for("paused", 20, &|s| s.status == TrackerStatus::Paused);
        assert!(s.resume_at.is_some() && s.tracking_since.is_none());
        let s = wait_for("the paused block uploaded", 60, &|s| s.last_sync != synced_before && s.pending_uploads == 0);
        println!("paused; block saved and uploaded; today {:.0}s", s.tracked_today_secs);
        let s = wait_for("auto-resume", 20, &|s| s.status == TrackerStatus::Tracking);
        assert!(s.resume_at.is_none() && s.paused_since.is_none());
        println!("auto-resumed");

        // Open-ended pause, then an explicit resume.
        tx.send(Command::Pause { resume_after: None }).unwrap();
        let s = wait_for("paused again", 20, &|s| s.status == TrackerStatus::Paused);
        assert!(s.resume_at.is_none());
        tx.send(Command::Resume).unwrap();
        wait_for("resumed", 20, &|s| s.status == TrackerStatus::Tracking);
        println!("paused and resumed");

        // A block under a minute is not cut by "Sync now".
        let block = state.snapshot().block_started_at;
        tx.send(Command::SyncNow).unwrap();
        std::thread::sleep(Duration::from_secs(3));
        assert_eq!(state.snapshot().block_started_at, block, "short block kept running");
        println!("short sync kept the block");

        tx.send(Command::Shutdown).unwrap();
        let deadline = Instant::now() + Duration::from_secs(30);
        while !worker.is_finished() {
            assert!(Instant::now() < deadline, "worker did not shut down");
            std::thread::sleep(Duration::from_millis(100));
        }
        println!("shut down cleanly");
    }

    #[tokio::test]
    async fn starting_a_stretch_resets_the_clocks_and_clears_a_pause() {
        let mut ticker = new_ticker(Duration::from_secs(300));
        ticker.tick().await;
        let mut clocks = Clocks::new();
        clocks.paused_since = Some(utc("2026-09-30T10:00:00Z"));
        clocks.pause_until = Some(utc("2026-09-30T10:30:00Z"));

        let before = Utc::now();
        clocks.start_tracking(&mut ticker);
        assert!(!clocks.paused());
        assert!(clocks.pause_until.is_none());
        assert!(clocks.tracking_since >= before && clocks.block_started_at == clocks.tracking_since);
        let until_next = clocks.next_capture_at - clocks.block_started_at;
        assert!((until_next - chrono::Duration::seconds(300)).num_seconds().abs() <= 1);
    }
}
