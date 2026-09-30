//! Background tracking worker.
//!
//! Owns everything slow or fallible (network, capture, SQLite) on its own thread
//! with a single-threaded tokio runtime, so the UI thread never blocks. The UI
//! talks to it only through a command channel and reads a shared state snapshot.

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

/// Spawn the worker thread and return the channel the UI uses to control it.
pub fn spawn(state: StateHandle, cfg: AppConfig, repaint: impl Fn() + Send + 'static) -> mpsc::UnboundedSender<Command> {
    let (tx, rx) = mpsc::unbounded_channel();

    std::thread::Builder::new()
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
                runtime.block_on(run(state_for_run, cfg, rx, &repaint));
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

    tx
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

async fn run(
    state: StateHandle,
    cfg: AppConfig,
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

    let tracker = Tracker::new();
    let mut client: Option<ApiClient> = None;
    let mut session: Option<Session> = None;
    let mut paused = false;
    // Real start of the block currently being accumulated. Captures are timed
    // from this, never from an assumed "exactly one interval ago".
    let mut block_started_at = Utc::now();

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
                client = Some(restored);
            }
            Err(e) => {
                tracing::warn!(?e, "saved sign-in is no longer valid");
                state.update(|s| s.status = TrackerStatus::SignedOut);
            }
        }
        apply_status(&state, &client, &session, paused);
        repaint();
    }

    let mut ticker = new_ticker(
        session.as_ref().map(|s| s.interval).unwrap_or_else(|| cfg.interval()),
    );
    ticker.tick().await; // skip the immediate first tick

    loop {
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
                                if let Some(active) = &session {
                                    ticker = new_ticker(active.interval);
                                    ticker.tick().await;
                                }
                                // Nothing from before this sign-in (signed-out
                                // time, another account) belongs to this block.
                                block_started_at = start_new_block(&tracker);
                                let owner = candidate.owner_key();
                                maintain_queue(&store, Some(&owner));
                                refresh_queue_counts(&store, &state, Some(&owner));
                                client = Some(candidate);
                                paused = false;
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
                        let _ = credentials::clear();
                        client = None;
                        session = None;
                        // Input made while signed out must not reach the next
                        // account's first block.
                        block_started_at = start_new_block(&tracker);
                        // This account's queued rows stay (owner-tagged) until it
                        // signs in again; they are just not shown to anyone else.
                        refresh_queue_counts(&store, &state, None);
                        state.update(|s| {
                            s.display_name.clear();
                            s.projects.clear();
                            s.selected_project = None;
                            s.tracked_today_minutes = 0;
                            s.tracked_week_minutes = 0;
                            s.status = TrackerStatus::SignedOut;
                        });
                    }
                    Command::SelectProject(project_id) => {
                        if let Some(api) = client.as_mut() {
                            session = resolve_session(api, &cfg, Some(&project_id), &state).await;
                            if let Some(active) = &session {
                                ticker = new_ticker(active.interval);
                                ticker.tick().await;
                            }
                            // The new project starts a fresh block.
                            block_started_at = start_new_block(&tracker);
                            // Remember the choice across restarts.
                            if let Some(mut saved) = credentials::load() {
                                saved.project_id = Some(project_id);
                                let _ = credentials::save(&saved);
                            }
                        }
                    }
                    // Both are guarded: the tray offers Pause and Resume at all
                    // times, and a stray Resume while tracking must not throw
                    // away the block in progress.
                    Command::Pause if !paused => {
                        paused = true;
                        // End the block here: the partial block is discarded, so
                        // no paused time can ever be attributed to it.
                        block_started_at = start_new_block(&tracker);
                    }
                    Command::Resume if paused => {
                        paused = false;
                        // Do not bill the paused stretch (or input made during
                        // it): start a fresh block.
                        block_started_at = start_new_block(&tracker);
                    }
                    Command::Pause | Command::Resume => {}
                    Command::SyncNow => {
                        if let Some(api) = client.as_mut() {
                            match session.as_ref() {
                                Some(active) if !paused => {
                                    block_started_at =
                                        capture_and_sync(&tracker, api, &store, active, &state, block_started_at).await;
                                }
                                // Paused means nothing is captured, not even on
                                // demand: only already-queued rows are uploaded.
                                _ => sync_pending(api, &store, &state).await,
                            }
                            refresh_totals(api, &state).await;
                        }
                    }
                    Command::Shutdown => {
                        if let Some(api) = client.as_mut() {
                            sync_pending(api, &store, &state).await;
                        }
                        return;
                    }
                }
                apply_status(&state, &client, &session, paused);
                repaint();
            }

            _ = ticker.tick() => {
                if client.is_some() && !paused {
                    if session.is_none() {
                        if let Some(api) = client.as_mut() {
                            // Backend was unreachable (or no project yet) — keep retrying.
                            session = resolve_session(api, &cfg, None, &state).await;
                        }
                    }

                    if let (Some(api), Some(active)) = (client.as_mut(), session.as_ref()) {
                        block_started_at =
                            capture_and_sync(&tracker, api, &store, active, &state, block_started_at).await;
                        refresh_totals(api, &state).await;
                    }
                }
                apply_status(&state, &client, &session, paused);
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

/// Begin a new block now, discarding input counted before it (while paused,
/// signed out, or on another project). Returns the block's start.
fn start_new_block(tracker: &Tracker) -> DateTime<Utc> {
    tracker.discard_input();
    Utc::now()
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

fn apply_status(
    state: &StateHandle,
    client: &Option<ApiClient>,
    session: &Option<Session>,
    paused: bool,
) {
    let status = if client.is_none() {
        TrackerStatus::SignedOut
    } else if paused {
        TrackerStatus::Paused
    } else if session.is_some() {
        TrackerStatus::Tracking
    } else {
        TrackerStatus::Offline
    };
    state.update(|s| s.status = status);
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
    });
    tracing::info!(project = %chosen.title, "tracking session resolved");

    Some(Session {
        project_id: chosen.id.clone(),
        perms,
        interval: Duration::from_secs(interval_minutes * 60),
    })
}

/// Capture one block covering `block_started_at .. now`, then sync.
/// Returns the instant the next block starts from.
async fn capture_and_sync(
    tracker: &Tracker,
    client: &mut ApiClient,
    store: &Storage,
    session: &Session,
    state: &StateHandle,
    block_started_at: chrono::DateTime<Utc>,
) -> chrono::DateTime<Utc> {
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
        Ok(id) => tracing::debug!(id, "activity queued"),
        Err(e) => {
            tracing::error!(?e, "failed to queue activity");
            state.update(|s| s.last_error = Some(format!("Could not save locally: {e}")));
        }
    }
    enforce_queue_limits(store);

    sync_pending(client, store, state).await;

    // The next block starts where this one ended.
    now
}

/// Refresh the today / this-week totals shown in the window.
async fn refresh_totals(client: &mut ApiClient, state: &StateHandle) {
    let now = Utc::now();
    let (today_start, week_start) = local_day_and_week_start(now, &Local);

    if let Ok(today) = client.summary(today_start, now).await {
        state.update(|s| {
            s.tracked_today_minutes = today.total_tracked_minutes.max(0.0) as u64;
            s.last_productivity = today.average_productivity.clamp(0.0, 100.0) as u8;
        });
    }
    if let Ok(week) = client.summary(week_start, now).await {
        state.update(|s| s.tracked_week_minutes = week.total_tracked_minutes.max(0.0) as u64);
    }
}

/// Local midnight today and on this week's Monday, as UTC instants for the
/// API. "Today" follows the user's wall clock, not UTC — otherwise the cards
/// would reset mid-day (at 06:00 in UTC+6, at 19:00 the day before in UTC-5).
fn local_day_and_week_start<Tz: TimeZone>(now: DateTime<Utc>, tz: &Tz) -> (DateTime<Utc>, DateTime<Utc>) {
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
}
