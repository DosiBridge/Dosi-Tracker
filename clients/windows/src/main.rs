//! Dosi-Tracker Windows agent.
//!
//! A lightweight background activity tracker with a system-tray presence and a
//! small desktop window (sign-in, live status, project picker, settings).
//!
//! Architecture:
//!   • UI thread  — egui window + tray icon; renders a cheap state snapshot.
//!   • Worker thread — owns auth, capture, the offline SQLite queue and sync.
//! They communicate through a command channel and a shared state snapshot, so
//! the window never blocks on network or capture work.
//!
//! Resource profile: input listening is event-driven and heavy captures only run
//! once per interval, so idle CPU stays near zero. Closing the window hides to
//! the tray; tracking continues until the user quits from the tray menu.

// Release builds are a GUI app: no console window should flash on launch.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod api;
mod app;
mod autostart;
mod config;
mod credentials;
mod model;
mod single_instance;
mod state;
mod storage;
mod theme;
mod tracking;
mod worker;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::Result;
use eframe::egui;
use tray_icon::menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem};
use tray_icon::{TrayIcon, TrayIconBuilder};

use config::AppConfig;
use state::{Command, PendingAction, SharedState, StateHandle, TrackerStatus};
use tracking::Tracker;

/// Window title, also used to find an existing instance.
const WINDOW_TITLE: &str = "Dosi Tracker";

/// How long quitting waits for the worker to queue and upload the block in
/// progress. The block is saved locally first, so a slow network only delays
/// its upload to the next launch.
const SHUTDOWN_GRACE: Duration = Duration::from_secs(20);

fn main() -> Result<()> {
    // Keep the log guard alive for the whole process, or buffered lines are lost.
    let _log_guard = init_logging();

    // Two agents would each capture and upload with their own idempotency keys,
    // which the backend cannot deduplicate — double-counted, over-billed time.
    let Some(_instance) = single_instance::acquire() else {
        tracing::info!("another instance is already running; focusing it and exiting");
        single_instance::focus_existing(WINDOW_TITLE);
        return Ok(());
    };

    let cfg = AppConfig::load()?;
    tracing::info!(?cfg, "starting dosi-tracker agent");

    let state = StateHandle::new();
    let quit_requested = Arc::new(AtomicBool::new(false));
    // Starts the input listener and the foreground-window sampler.
    let tracker = Arc::new(Tracker::new());
    let input = tracker.input();

    // The worker needs a way to wake the UI when state changes; the egui context
    // only exists once the window is created, so it is injected here.
    let repaint_ctx: Arc<std::sync::Mutex<Option<egui::Context>>> =
        Arc::new(std::sync::Mutex::new(None));
    let repaint_for_worker = repaint_ctx.clone();
    let repaint = move || {
        if let Ok(guard) = repaint_for_worker.lock() {
            if let Some(ctx) = guard.as_ref() {
                ctx.request_repaint();
            }
        }
    };

    let (commands, worker) = worker::spawn(state.clone(), cfg, tracker, repaint);

    // Debug-only: `DOSI_UI_PREVIEW=main|settings` seeds demo state so the
    // signed-in views can be laid out and reviewed without a live backend.
    #[cfg(debug_assertions)]
    seed_ui_preview(&state);

    // Debug-only: drive a real sign-in from the environment for end-to-end tests
    // (`DOSI_WORKSPACE` / `DOSI_USERNAME` / `DOSI_PASSWORD`, plus `DOSI_AUTO_SYNC`
    // to capture immediately instead of waiting for the first interval).
    #[cfg(debug_assertions)]
    debug_auto_signin(&commands);

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([380.0, 640.0])
            .with_min_inner_size([340.0, 600.0])
            .with_resizable(false)
            .with_title(WINDOW_TITLE),
        ..Default::default()
    };

    let commands_for_app = commands.clone();
    let quit_for_app = quit_requested.clone();
    let state_for_app = state.clone();

    let result = eframe::run_native(
        WINDOW_TITLE,
        options,
        Box::new(move |cc| {
            // Publish the egui context so the worker can request repaints.
            if let Ok(mut guard) = repaint_ctx.lock() {
                *guard = Some(cc.egui_ctx.clone());
            }

            // The tray must be built on the UI thread (it needs the message loop).
            let tray = build_tray().map_err(|e| {
                tracing::error!(?e, "failed to create the tray icon");
                e
            });
            let tray = tray.ok();

            spawn_tray_event_pump(
                cc.egui_ctx.clone(),
                state_for_app.clone(),
                commands_for_app.clone(),
                quit_for_app.clone(),
            );

            Ok(Box::new(TrayHolder {
                tray,
                state: state_for_app.clone(),
                app: app::TrackerApp::new(state_for_app, commands_for_app, input, quit_for_app),
            }) as Box<dyn eframe::App>)
        }),
    );

    if let Err(e) = result {
        tracing::error!(%e, "the UI failed to start");
        // Without a window there is nothing the user can do, so exit rather than
        // leaving an invisible process tracking in the background.
        let _ = commands.send(Command::Shutdown);
        wait_for_worker(worker);
        return Err(anyhow::anyhow!("failed to start the UI: {e}"));
    }

    let _ = commands.send(Command::Shutdown);
    wait_for_worker(worker);
    Ok(())
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

/// Set up logging to a rolling file under `%APPDATA%\DosiTracker\logs`.
///
/// Release builds are a GUI subsystem app with no console, so a stdout-only
/// subscriber would silently discard every line — leaving no evidence at all
/// when a user reports that tracking stopped. Debug builds also keep stdout.
fn init_logging() -> Option<tracing_appender::non_blocking::WorkerGuard> {
    use tracing_subscriber::EnvFilter;

    let filter = || EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into());

    let log_dir = dirs::config_dir().map(|d| d.join("DosiTracker").join("logs"));
    let Some(log_dir) = log_dir else {
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
fn seed_ui_preview(state: &StateHandle) {
    if std::env::var("DOSI_UI_PREVIEW").is_err() {
        return;
    }
    let now = chrono::Utc::now();
    let paused = std::env::var("DOSI_UI_PREVIEW").is_ok_and(|v| v == "paused");
    state.update(|s| {
        s.status = if paused { TrackerStatus::Paused } else { TrackerStatus::Tracking };
        s.display_name = "ayesha@acme.com".into();
        s.projects = vec![
            state::ProjectOption { id: "p1".into(), title: "Website Redesign".into() },
            state::ProjectOption { id: "p2".into(), title: "Mobile App".into() },
        ];
        s.selected_project = Some("p1".into());
        s.tracked_today_secs = 222.0 * 60.0;
        s.tracked_week_secs = 1085.0 * 60.0;
        s.last_productivity = 72;
        s.interval_minutes = 10;
        s.pending_uploads = 2;
        if paused {
            s.paused_since = Some(now - chrono::Duration::seconds(252));
            s.resume_at = Some(now + chrono::Duration::minutes(12));
        } else {
            s.tracking_since = Some(now - chrono::Duration::seconds(4_997));
            s.block_started_at = Some(now - chrono::Duration::seconds(197));
            s.next_capture_at = Some(now + chrono::Duration::seconds(403));
        }
    });
}

/// Sign in from environment variables so an automated end-to-end run can exercise
/// the real login → project → capture → upload path. Never built into a release.
#[cfg(debug_assertions)]
fn debug_auto_signin(commands: &tokio::sync::mpsc::UnboundedSender<Command>) {
    let (Ok(username), Ok(password)) = (
        std::env::var("DOSI_USERNAME"),
        std::env::var("DOSI_PASSWORD"),
    ) else {
        return;
    };
    let workspace = std::env::var("DOSI_WORKSPACE").unwrap_or_default();
    tracing::info!(%workspace, %username, "debug auto sign-in");
    let _ = commands.send(Command::SignIn {
        workspace,
        username,
        password,
    });
    if std::env::var("DOSI_AUTO_SYNC").is_ok() {
        // Queued behind SignIn; the worker processes commands in order.
        let _ = commands.send(Command::SyncNow);
    }
}

/// Keeps the tray icon alive for the lifetime of the app, keeps it in step
/// with the tracker, and forwards `eframe::App`.
struct TrayHolder {
    tray: Option<Tray>,
    state: StateHandle,
    app: app::TrackerApp,
}

impl eframe::App for TrayHolder {
    fn clear_color(&self, visuals: &egui::Visuals) -> [f32; 4] {
        self.app.clear_color(visuals)
    }

    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        self.app.ui(ui, frame);
        if let Some(tray) = &mut self.tray {
            tray.sync(&self.state.snapshot());
        }
    }
}

// Menu item ids, matched in the event pump.
const MENU_SHOW: &str = "show";
const MENU_PAUSE: &str = "pause";
const MENU_PAUSE_30: &str = "pause-30";
const MENU_PAUSE_60: &str = "pause-60";
const MENU_RESUME: &str = "resume";
const MENU_QUIT: &str = "quit";

/// The tray icon and the menu items whose state follows the tracker.
struct Tray {
    icon: TrayIcon,
    pause: [MenuItem; 3],
    resume: MenuItem,
    /// What was last applied, so the icon is only touched on a change.
    applied: Option<(bool, bool, String)>,
}

impl Tray {
    /// Enable only the actions that apply now, and show the status (with the
    /// session time, to the minute) in the tooltip.
    fn sync(&mut self, snapshot: &SharedState) {
        let idle = snapshot.pending_action.is_none();
        let can_pause = idle && app::can_pause(snapshot.status);
        let can_resume = idle && snapshot.status == TrackerStatus::Paused;
        let tooltip = tray_tooltip(snapshot, chrono::Utc::now());
        let wanted = (can_pause, can_resume, tooltip);
        if self.applied.as_ref() == Some(&wanted) {
            return;
        }
        for item in &self.pause {
            item.set_enabled(wanted.0);
        }
        self.resume.set_enabled(wanted.1);
        if let Err(e) = self.icon.set_tooltip(Some(&wanted.2)) {
            tracing::debug!(?e, "could not update the tray tooltip");
        }
        self.applied = Some(wanted);
    }
}

fn tray_tooltip(snapshot: &SharedState, now: chrono::DateTime<chrono::Utc>) -> String {
    let minutes = |since: Option<chrono::DateTime<chrono::Utc>>| {
        let total = since.map_or(0, |at| (now - at).num_minutes().max(0));
        if total >= 60 {
            format!("{}h {:02}m", total / 60, total % 60)
        } else {
            format!("{total}m")
        }
    };
    let detail = match (snapshot.pending_action, snapshot.status) {
        (Some(PendingAction::Pausing), _) => "Pausing…".to_string(),
        (Some(PendingAction::Resuming), _) => "Resuming…".to_string(),
        (None, TrackerStatus::Tracking) => format!("Tracking · {}", minutes(snapshot.tracking_since)),
        (None, TrackerStatus::Paused) => match snapshot.resume_at {
            Some(at) => format!(
                "Paused · resumes at {}",
                at.with_timezone(&chrono::Local).format("%H:%M")
            ),
            None => format!("Paused · {}", minutes(snapshot.paused_since)),
        },
        (None, status) => status.label().to_string(),
    };
    format!("Dosi Tracker — {detail}")
}

fn build_tray() -> Result<Tray> {
    let pause = [
        MenuItem::with_id(MENU_PAUSE, "Pause tracking", true, None),
        MenuItem::with_id(MENU_PAUSE_30, "Pause for 30 minutes", true, None),
        MenuItem::with_id(MENU_PAUSE_60, "Pause for 1 hour", true, None),
    ];
    let resume = MenuItem::with_id(MENU_RESUME, "Resume tracking", false, None);

    let menu = Menu::new();
    menu.append(&MenuItem::with_id(MENU_SHOW, "Open Dosi Tracker", true, None))?;
    menu.append(&PredefinedMenuItem::separator())?;
    for item in &pause {
        menu.append(item)?;
    }
    menu.append(&resume)?;
    menu.append(&PredefinedMenuItem::separator())?;
    menu.append(&MenuItem::with_id(MENU_QUIT, "Quit", true, None))?;

    let icon = TrayIconBuilder::new()
        .with_tooltip("Dosi Tracker")
        .with_menu(Box::new(menu))
        .with_icon(brand_icon())
        .build()?;

    Ok(Tray { icon, pause, resume, applied: None })
}

/// A small solid-brand icon drawn in code, so the binary stays self-contained
/// (no external .ico asset to ship or lose).
fn brand_icon() -> tray_icon::Icon {
    const SIZE: u32 = 32;
    let mut rgba = Vec::with_capacity((SIZE * SIZE * 4) as usize);
    let center = (SIZE as f32 - 1.0) / 2.0;
    for y in 0..SIZE {
        for x in 0..SIZE {
            let dx = x as f32 - center;
            let dy = y as f32 - center;
            let distance = (dx * dx + dy * dy).sqrt();
            // Filled brand circle with a hollow ring — reads well at 16px.
            let inside = distance <= center;
            let ring = (7.0..=10.5).contains(&distance);
            if inside && !ring {
                rgba.extend_from_slice(&[0x00, 0x6b, 0xff, 0xff]);
            } else if inside {
                rgba.extend_from_slice(&[0xff, 0xff, 0xff, 0xff]);
            } else {
                rgba.extend_from_slice(&[0, 0, 0, 0]);
            }
        }
    }
    tray_icon::Icon::from_rgba(rgba, SIZE, SIZE).expect("valid generated icon")
}

/// Tray menu clicks arrive on a global channel; translate them into commands and
/// window actions, then wake the UI.
fn spawn_tray_event_pump(
    ctx: egui::Context,
    state: StateHandle,
    commands: tokio::sync::mpsc::UnboundedSender<Command>,
    quit_requested: Arc<AtomicBool>,
) {
    std::thread::Builder::new()
        .name("tray-events".into())
        .spawn(move || {
            let receiver = MenuEvent::receiver();
            let pause = |resume_after: Option<Duration>| {
                app::request_action(&state, &commands, PendingAction::Pausing, Command::Pause { resume_after });
            };
            while let Ok(event) = receiver.recv() {
                match event.id.0.as_str() {
                    MENU_SHOW => {
                        ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
                        ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
                    }
                    MENU_PAUSE => pause(None),
                    MENU_PAUSE_30 => pause(Some(Duration::from_secs(30 * 60))),
                    MENU_PAUSE_60 => pause(Some(Duration::from_secs(60 * 60))),
                    MENU_RESUME => {
                        app::request_action(&state, &commands, PendingAction::Resuming, Command::Resume);
                    }
                    MENU_QUIT => {
                        quit_requested.store(true, Ordering::Relaxed);
                        let _ = commands.send(Command::Shutdown);
                    }
                    _ => {}
                }
                ctx.request_repaint();
            }
        })
        .expect("failed to spawn tray event pump");
}
