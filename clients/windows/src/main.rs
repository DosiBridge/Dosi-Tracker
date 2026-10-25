//! Dosi-Tracker Windows agent.
//!
//! A lightweight background activity tracker with a system-tray presence and a
//! desktop window (sign-in, live timer, activity, settings).
//!
//! Architecture:
//!   • UI thread  — egui window + tray icon; renders a cheap state snapshot.
//!   • Worker thread — decides when blocks start and end and queues them in
//!     the offline SQLite queue; a task beside it talks to the server
//!     (sign-in, projects, presence), and an uploader thread drains the queue.
//!   • Session events thread — a hidden window that hears the screen lock,
//!     sleep and shutdown.
//! They communicate through command channels and a shared state snapshot, so
//! the window never blocks on network or capture work.
//!
//! Resource profile: input listening is event-driven and heavy captures only run
//! once per interval, so idle CPU stays near zero. Closing (or minimizing) the
//! window hides it to the tray; tracking continues until the user quits from
//! the tray menu.

// Release builds are a GUI app: no console window should flash on launch.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

// The data layer marks items the worker was to start using with
// `expect(dead_code)`; the worker now uses them, which makes those
// expectations unfulfilled. Allowed here, module by module, until the
// markers themselves are removed.
#[allow(unfulfilled_lint_expectations)]
mod api;
mod autostart;
#[allow(unfulfilled_lint_expectations)]
mod config;
mod credentials;
#[allow(unfulfilled_lint_expectations)]
mod model;
mod power;
mod prefs;
mod session_events;
#[allow(unfulfilled_lint_expectations)]
mod shell;
mod signing;
mod single_instance;
mod state;
#[allow(unfulfilled_lint_expectations)]
mod storage;
mod theme;
#[allow(unfulfilled_lint_expectations)]
mod tracking;
mod tray;
mod ui;
mod worker;

use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::Result;
use eframe::egui;

use config::AppConfig;
use single_instance::Signal;
use state::{AppSignal, Command, StateHandle};
use tracking::Tracker;

/// Window title, also used to find an existing instance.
const WINDOW_TITLE: &str = "Dosi Tracker";

/// The window's preferred inner size, shrunk to fit smaller screens.
pub const DEFAULT_WINDOW_SIZE: [f32; 2] = [480.0, 780.0];
/// Small enough for a 1366×768 laptop at 125% scaling, large enough that
/// nothing is cramped (the content scrolls below this).
const MIN_WINDOW_SIZE: [f32; 2] = [400.0, 520.0];

/// How long quitting waits for the worker to save the block in progress, say
/// goodbye to the server and upload. The block is saved locally first, so a
/// slow network only delays its upload to the next launch.
const SHUTDOWN_GRACE: Duration = Duration::from_secs(20);

/// How long `--quit` waits for the running instance to be gone.
const QUIT_WAIT: Duration = Duration::from_secs(30);

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    // The installer asks the running agent to quit (saving the block in
    // progress) before it replaces the program.
    if single_instance::quit_requested(&args) {
        return quit_running_instance();
    }

    // Earlier builds kept the queue and the logs in Roaming AppData; they are
    // machine-local now. Moved before the logger and the queue open them.
    let migration = shell::migrate_to_local();
    // Keep the log guard alive for the whole process, or buffered lines are lost.
    let _log_guard = init_logging();
    log_migration(&migration);

    // Two agents would each capture and upload with their own idempotency keys,
    // which the backend cannot deduplicate — double-counted, over-billed time.
    let Some(instance) = single_instance::acquire() else {
        tracing::info!("another instance is already running; showing it and exiting");
        if !single_instance::signal(Signal::Show) {
            // A running build too old to listen.
            single_instance::focus_existing(WINDOW_TITLE);
        }
        return Ok(());
    };

    let cfg = AppConfig::load()?;
    tracing::info!(?cfg, "starting dosi-tracker agent");

    // Windows starting us at sign-in should not throw a window in the user's
    // face — if they asked for that, and there is a saved sign-in to resume.
    // Otherwise (first run, signed out) the window is needed to sign in.
    let autostart = autostart::launched_at_login();
    let start_hidden = autostart
        && prefs::Preferences::load().start_minimized
        && credentials::load().is_some();

    let links = ui::Links {
        dashboard_url: cfg.dashboard_url.clone(),
        api_base_url: cfg.api_base_url.clone(),
        allow_insecure_http: cfg.allow_insecure_http,
    };

    let state = StateHandle::new();
    // Starts the input listener and the foreground-window sampler.
    let tracker = Arc::new(Tracker::new());
    let input = tracker.input();
    let usage = tracker.usage();

    // The worker needs a way to wake the UI when state changes; the egui context
    // only exists once the window is created, so it is injected here.
    let repaint_ctx: Arc<std::sync::Mutex<Option<egui::Context>>> = Arc::new(std::sync::Mutex::new(None));
    let repaint_for_worker = repaint_ctx.clone();
    let repaint: Arc<dyn Fn() + Send + Sync> = Arc::new(move || {
        if let Ok(guard) = repaint_for_worker.lock() {
            if let Some(ctx) = guard.as_ref() {
                ctx.request_repaint();
            }
        }
    });

    // Another launch asking this one to show its window, or to quit.
    let (signals, signal_events) = std::sync::mpsc::channel();
    let repaint_for_signals = repaint.clone();
    instance.listen(move |signal| {
        let signal = match signal {
            Signal::Show => AppSignal::Show,
            Signal::Quit => AppSignal::Quit,
        };
        if signals.send(signal).is_ok() {
            repaint_for_signals();
        }
    });

    // Debug-only: `DOSI_UI_PREVIEW=main|paused|empty` seeds demo state and
    // runs without a worker, so the signed-in views can be laid out and
    // reviewed without a live backend (or touching the saved sign-in).
    #[cfg(debug_assertions)]
    let preview = std::env::var("DOSI_UI_PREVIEW").ok();
    #[cfg(not(debug_assertions))]
    let preview: Option<String> = None;

    let (commands, worker) = match preview.as_deref() {
        Some(kind) => {
            #[cfg(debug_assertions)]
            seed_ui_preview(&state, kind);
            let _ = kind;
            preview_worker()
        }
        None => {
            let worker = worker::spawn(state.clone(), cfg, tracker, worker::Launch { autostart }, repaint);
            // The screen locking, sleep and shutdown: ending the block at the
            // right moment, and saving it before the machine goes away.
            if let Err(e) = session_events::spawn(worker.system.clone()) {
                tracing::error!(?e, "lock, sleep and shutdown events are unavailable");
            }
            (worker.commands, worker.thread)
        }
    };

    // Debug-only: drive a real sign-in from the environment for end-to-end tests
    // (`DOSI_WORKSPACE` / `DOSI_USERNAME` / `DOSI_PASSWORD`, plus `DOSI_AUTO_SYNC`
    // to capture immediately instead of waiting for the first interval).
    #[cfg(debug_assertions)]
    debug_auto_signin(&commands);

    let mut viewport = egui::ViewportBuilder::default()
        .with_inner_size(initial_window_size())
        .with_min_inner_size(MIN_WINDOW_SIZE)
        .with_resizable(true)
        .with_title(WINDOW_TITLE)
        .with_icon(Arc::new(tray::window_icon()));
    if start_hidden {
        // eframe shows the window after its first frame, before it can be
        // hidden; off-screen, that is invisible. It is centred when first shown.
        viewport = viewport.with_position([-32_000.0, -32_000.0]);
    }
    let options = eframe::NativeOptions {
        viewport,
        centered: !start_hidden,
        ..Default::default()
    };

    let commands_for_app = commands.clone();
    let state_for_app = state.clone();

    let result = eframe::run_native(
        WINDOW_TITLE,
        options,
        Box::new(move |cc| {
            // Publish the egui context so the worker can request repaints.
            if let Ok(mut guard) = repaint_ctx.lock() {
                *guard = Some(cc.egui_ctx.clone());
            }
            Ok(Box::new(ui::TrackerApp::new(
                &cc.egui_ctx,
                state_for_app,
                commands_for_app,
                input,
                usage,
                links,
                ui::Launch { start_hidden },
                signal_events,
            )) as Box<dyn eframe::App>)
        }),
    );

    if let Err(e) = result {
        tracing::error!(%e, "the UI failed to start");
        // Without a window there is nothing the user can do, so exit rather than
        // leaving an invisible process tracking in the background.
        let _ = commands.send(Command::Shutdown { at: chrono::Utc::now() });
        wait_for_worker(worker);
        return Err(anyhow::anyhow!("failed to start the UI: {e}"));
    }

    // The window sent its own Shutdown when the user quit; this one is for
    // any other way the window closed (the worker handles the first one).
    let _ = commands.send(Command::Shutdown { at: chrono::Utc::now() });
    wait_for_worker(worker);
    Ok(())
}

/// `--quit`: ask the running instance to quit gracefully and wait until it is
/// gone. Fails (exit code 1) when it did not quit in time, so the installer
/// can fall back to ending it.
fn quit_running_instance() -> Result<()> {
    if !single_instance::signal(Signal::Quit) {
        // Nothing runs (or a build too old to listen; the installer's
        // fallback ends it).
        return Ok(());
    }
    if single_instance::wait_until_gone(QUIT_WAIT) {
        Ok(())
    } else {
        Err(anyhow::anyhow!("Dosi Tracker did not quit within {} s", QUIT_WAIT.as_secs()))
    }
}

/// Report what the move to Local AppData did (once the logger exists).
fn log_migration(migration: &shell::Migration) {
    for moved in &migration.moved {
        tracing::info!(path = %moved.display(), "moved to the machine-local data folder");
    }
    for (path, reason) in &migration.skipped {
        tracing::warn!(path = %path.display(), %reason, "left in the roaming data folder");
    }
}

/// The default size, shrunk to the screen's usable area (minus room for the
/// title bar), so the whole window — bottom buttons included — is on screen
/// even on a small or highly scaled display.
fn initial_window_size() -> [f32; 2] {
    let [width, height] = DEFAULT_WINDOW_SIZE;
    match shell::work_area() {
        Some(area) => [
            width.min(area.width - 40.0).max(MIN_WINDOW_SIZE[0]),
            height.min(area.height - 72.0).max(MIN_WINDOW_SIZE[1]),
        ],
        None => DEFAULT_WINDOW_SIZE,
    }
}

/// Give the worker [`SHUTDOWN_GRACE`] to save and upload the block in
/// progress. Returning from `main` would otherwise kill it mid-way.
fn wait_for_worker(worker: std::thread::JoinHandle<()>) {
    let deadline = Instant::now() + SHUTDOWN_GRACE;
    while !worker.is_finished() {
        if Instant::now() >= deadline {
            tracing::warn!("worker did not finish in time; queued work uploads on the next launch");
            return;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    let _ = worker.join();
}

/// A stand-in worker for the UI preview: drains commands until shutdown.
fn preview_worker() -> (tokio::sync::mpsc::UnboundedSender<Command>, std::thread::JoinHandle<()>) {
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let handle = std::thread::spawn(move || {
        while let Some(command) = rx.blocking_recv() {
            if matches!(command, Command::Shutdown { .. }) {
                return;
            }
        }
    });
    (tx, handle)
}

/// Set up logging to a rolling file under `%LOCALAPPDATA%\DosiTracker\logs`.
///
/// Release builds are a GUI subsystem app with no console, so a stdout-only
/// subscriber would silently discard every line — leaving no evidence at all
/// when a user reports that tracking stopped. Debug builds also keep stdout.
fn init_logging() -> Option<tracing_appender::non_blocking::WorkerGuard> {
    use tracing_subscriber::EnvFilter;

    let filter = || EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into());

    let Some(log_dir) = shell::logs_dir() else {
        tracing_subscriber::fmt().with_env_filter(filter()).init();
        return None;
    };
    if std::fs::create_dir_all(&log_dir).is_err() {
        tracing_subscriber::fmt().with_env_filter(filter()).init();
        return None;
    }

    let appender = tracing_appender::rolling::Builder::new()
        .rotation(tracing_appender::rolling::Rotation::DAILY)
        .filename_prefix("dosi-tracker")
        .filename_suffix("log")
        .max_log_files(7) // bounded: never grows without limit
        .build(&log_dir);

    match appender {
        Ok(appender) => {
            let (writer, guard) = tracing_appender::non_blocking(appender);
            tracing_subscriber::fmt()
                .with_env_filter(filter())
                .with_ansi(false)
                .with_writer(writer)
                .init();
            Some(guard)
        }
        Err(_) => {
            tracing_subscriber::fmt().with_env_filter(filter()).init();
            None
        }
    }
}

/// Populate the shared state with representative values so the signed-in views
/// can be visually checked in a debug build (never compiled into a release).
#[cfg(debug_assertions)]
fn seed_ui_preview(state: &StateHandle, kind: &str) {
    use state::{CaptureFlags, ProjectOption, RecentBlock, TrackerStatus, UploadState};

    if kind == "signed-out" {
        return;
    }
    let now = chrono::Utc::now();
    let paused = kind == "paused";
    let minutes = |m: i64| chrono::Duration::minutes(m);
    state.update(|s| {
        s.status = if paused { TrackerStatus::Paused } else { TrackerStatus::Tracking };
        s.display_name = "ayesha@acme.com".into();
        s.workspace = "acme".into();
        s.projects = vec![
            ProjectOption { id: "p1".into(), title: "Website Redesign".into() },
            ProjectOption { id: "p2".into(), title: "Mobile App".into() },
        ];
        s.selected_project = Some("p1".into());
        s.tracked_today_secs = 222.0 * 60.0;
        s.tracked_week_secs = 1085.0 * 60.0;
        s.week_day_secs = [7.4 * 3600.0, 8.2 * 3600.0, 6.1 * 3600.0, 0.0, 0.0, 0.0, 0.0];
        s.last_productivity = 72;
        s.interval_minutes = 10;
        s.idle_threshold_minutes = Some(5);
        s.task_note = "Checkout page redesign".into();
        s.capture = CaptureFlags {
            screenshot: true,
            webcam: false,
            keyboard: true,
            mouse: true,
            active_window: true,
            running_programs: true,
            blur_screenshots: false,
        };
        s.pending_uploads = 1;
        s.last_sync = Some(now - minutes(3));
        s.recent_blocks = (0..4)
            .map(|i| RecentBlock {
                row_id: i,
                project: "Website Redesign".into(),
                started_at: now - minutes(10 * (i + 1) + 3),
                ended_at: now - minutes(10 * i + 3),
                productivity: [84, 67, 41, 92][i as usize],
                keyboard_hits: Some(300),
                mouse_clicks: Some(80),
                screenshot: true,
                upload: if i == 0 { UploadState::Queued } else { UploadState::Uploaded },
            })
            .collect();
        if paused {
            s.paused_since = Some(now - chrono::Duration::seconds(252));
            s.resume_at = Some(now + minutes(12));
        } else {
            s.tracking_since = Some(now - chrono::Duration::seconds(4_997));
            s.block_started_at = Some(now - chrono::Duration::seconds(197));
            s.unsaved_since = s.block_started_at;
            s.next_capture_at = Some(now + chrono::Duration::seconds(403));
        }
    });
}

/// Sign in from environment variables so an automated end-to-end run can exercise
/// the real login → project → capture → upload path. Never built into a release.
#[cfg(debug_assertions)]
fn debug_auto_signin(commands: &tokio::sync::mpsc::UnboundedSender<Command>) {
    let (Ok(username), Ok(password)) = (std::env::var("DOSI_USERNAME"), std::env::var("DOSI_PASSWORD")) else {
        return;
    };
    // Optional: the workspace (name or id) to pick after signing in.
    let workspace = std::env::var("DOSI_WORKSPACE").ok();
    tracing::info!(?workspace, %username, "debug auto sign-in");
    let server = std::env::var("DOSI_SERVER").ok();
    let _ = commands.send(Command::SignIn { workspace, username, password, server });
    if std::env::var("DOSI_AUTO_SYNC").is_ok() {
        // Queued behind SignIn; the worker processes commands in order.
        let _ = commands.send(Command::SyncNow { at: chrono::Utc::now() });
    }
}
