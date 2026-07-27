//! Background tracking worker.
//!
//! Owns everything slow or fallible (network, capture, SQLite) on its own thread
//! with a single-threaded tokio runtime, so the UI thread never blocks. The UI
//! talks to it only through a command channel and reads a shared state snapshot.

use std::time::Duration;

use chrono::{Datelike, TimeZone, Utc};
use tokio::sync::mpsc;

use crate::api::{ApiClient, SubmitError};
use crate::config::{AppConfig, CapturePermissions};
use crate::credentials::{self, Credentials};
use crate::state::{Command, ProjectOption, StateHandle, TrackerStatus};
use crate::storage::Storage;
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
/// queue reclaims their space.
const REJECTED_RETENTION_DAYS: u32 = 14;

async fn run(
    state: StateHandle,
    cfg: AppConfig,
    mut rx: mpsc::UnboundedReceiver<Command>,
    repaint: &(impl Fn() + Send),
) {
    let store = match Storage::open(&storage_path()) {
        Ok(s) => s,
        Err(e) => {
            tracing::error!(?e, "failed to open the local queue");
            state.update(|s| s.last_error = Some(format!("Local storage unavailable: {e}")));
            repaint();
            return;
        }
    };

    // Reclaim space from rows the server rejected long ago, and show the rest.
    match store.purge_rejected_older_than(REJECTED_RETENTION_DAYS) {
        Ok(n) if n > 0 => tracing::info!(purged = n, "removed old rejected activities"),
        Err(e) => tracing::warn!(?e, "could not purge rejected activities"),
        _ => {}
    }
    if let Ok(rejected) = store.rejected_count() {
        state.update(|s| s.rejected_uploads = rejected);
    }

    let tracker = Tracker::new();
    let mut client: Option<ApiClient> = None;
    let mut session: Option<Session> = None;
    let mut paused = false;
    // Real start of the block currently being accumulated. Captures are timed
    // from this, never from an assumed "exactly one interval ago".
    let mut block_started_at = Utc::now();

    // Restore a previous sign-in so a relaunch (e.g. at login) resumes silently.
    if let Some(saved) = credentials::load() {
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
                                    block_started_at = Utc::now();
                                }
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
                                block_started_at = Utc::now();
                            }
                            // Remember the choice across restarts.
                            if let Some(mut saved) = credentials::load() {
                                saved.project_id = Some(project_id);
                                let _ = credentials::save(&saved);
                            }
                        }
                    }
                    Command::Pause => paused = true,
                    Command::Resume => {
                        paused = false;
                        // Do not bill the paused stretch: start a fresh block.
                        block_started_at = Utc::now();
                    }
                    Command::SyncNow => {
                        if let (Some(api), Some(active)) = (client.as_mut(), session.as_ref()) {
                            block_started_at =
                                capture_and_sync(&tracker, api, &store, active, &state, block_started_at).await;
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

    let activity = tracker.snapshot(&session.perms, &session.project_id, started_at);

    match store.enqueue(&activity) {
        Ok(id) => tracing::debug!(id, "activity queued"),
        Err(e) => {
            tracing::error!(?e, "failed to queue activity");
            state.update(|s| s.last_error = Some(format!("Could not save locally: {e}")));
        }
    }

    sync_pending(client, store, state).await;

    // The next block starts where this one ended.
    now
}

/// Refresh the today / this-week totals shown in the window.
async fn refresh_totals(client: &mut ApiClient, state: &StateHandle) {
    let now = Utc::now();
    let today_start = Utc
        .with_ymd_and_hms(now.year(), now.month(), now.day(), 0, 0, 0)
        .single()
        .unwrap_or(now);
    // Monday-anchored week.
    let week_start = today_start
        - chrono::Duration::days(now.weekday().num_days_from_monday() as i64);

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

/// Upload queued activities. Permanently-rejected payloads are parked so they
/// never block the queue; transient failures are retried next interval.
async fn sync_pending(client: &mut ApiClient, store: &Storage, state: &StateHandle) {
    let pending = match store.pending() {
        Ok(p) => p,
        Err(e) => {
            tracing::error!(?e, "failed to read pending activities");
            return;
        }
    };

    let mut remaining = pending.len() as u32;
    for (id, activity) in pending {
        match client.submit_activity(&activity).await {
            Ok(()) => {
                let _ = store.remove_synced(id);
                remaining = remaining.saturating_sub(1);
                state.update(|s| { s.last_sync = Some(Utc::now()); s.last_error = None; });
                tracing::debug!(id, "activity synced");
            }
            Err(SubmitError::Rejected(reason)) => {
                tracing::warn!(id, %reason, "server rejected activity; parking it");
                let _ = store.mark_rejected(id, &reason);
                remaining = remaining.saturating_sub(1);
                if let Ok(rejected) = store.rejected_count() {
                    state.update(|s| s.rejected_uploads = rejected);
                }
            }
            Err(SubmitError::Transient(e)) => {
                tracing::warn!(?e, id, "sync failed; will retry next interval");
                state.update(|s| s.last_error = Some("Offline — uploads will retry.".into()));
                break;
            }
        }
    }

    state.update(|s| s.pending_uploads = remaining);
}
