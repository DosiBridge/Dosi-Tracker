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

use anyhow::Result;
use eframe::egui;
use tray_icon::menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem};
use tray_icon::{TrayIcon, TrayIconBuilder};

use config::AppConfig;
use state::{Command, StateHandle};

/// Window title, also used to find an existing instance.
const WINDOW_TITLE: &str = "Dosi Tracker";

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

    let commands = worker::spawn(state.clone(), cfg, repaint);

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
            .with_inner_size([380.0, 460.0])
            .with_min_inner_size([340.0, 420.0])
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
                commands_for_app.clone(),
                quit_for_app.clone(),
            );

            Ok(Box::new(TrayHolder {
                _tray: tray,
                app: app::TrackerApp::new(state_for_app, commands_for_app, quit_for_app),
            }) as Box<dyn eframe::App>)
        }),
    );

    if let Err(e) = result {
        tracing::error!(%e, "the UI failed to start");
        // Without a window there is nothing the user can do, so exit rather than
        // leaving an invisible process tracking in the background.
        let _ = commands.send(Command::Shutdown);
        return Err(anyhow::anyhow!("failed to start the UI: {e}"));
    }

    let _ = commands.send(Command::Shutdown);
    Ok(())
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
    state.update(|s| {
        s.status = state::TrackerStatus::Tracking;
        s.display_name = "ayesha@acme.com".into();
        s.projects = vec![
            state::ProjectOption { id: "p1".into(), title: "Website Redesign".into() },
            state::ProjectOption { id: "p2".into(), title: "Mobile App".into() },
        ];
        s.selected_project = Some("p1".into());
        s.tracked_today_minutes = 222;
        s.tracked_week_minutes = 1085;
        s.last_productivity = 72;
        s.interval_minutes = 10;
        s.pending_uploads = 2;
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

/// Keeps the tray icon alive for the lifetime of the app and forwards `eframe::App`.
struct TrayHolder {
    _tray: Option<TrayIcon>,
    app: app::TrackerApp,
}

impl eframe::App for TrayHolder {
    fn clear_color(&self, visuals: &egui::Visuals) -> [f32; 4] {
        self.app.clear_color(visuals)
    }

    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        self.app.ui(ui, frame);
    }
}

// Menu item ids, matched in the event pump.
const MENU_SHOW: &str = "show";
const MENU_PAUSE: &str = "pause";
const MENU_RESUME: &str = "resume";
const MENU_QUIT: &str = "quit";

fn build_tray() -> Result<TrayIcon> {
    let menu = Menu::new();
    menu.append(&MenuItem::with_id(MENU_SHOW, "Open Dosi Tracker", true, None))?;
    menu.append(&PredefinedMenuItem::separator())?;
    menu.append(&MenuItem::with_id(MENU_PAUSE, "Pause tracking", true, None))?;
    menu.append(&MenuItem::with_id(MENU_RESUME, "Resume tracking", true, None))?;
    menu.append(&PredefinedMenuItem::separator())?;
    menu.append(&MenuItem::with_id(MENU_QUIT, "Quit", true, None))?;

    let tray = TrayIconBuilder::new()
        .with_tooltip("Dosi Tracker")
        .with_menu(Box::new(menu))
        .with_icon(brand_icon())
        .build()?;

    Ok(tray)
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
    commands: tokio::sync::mpsc::UnboundedSender<Command>,
    quit_requested: Arc<AtomicBool>,
) {
    std::thread::Builder::new()
        .name("tray-events".into())
        .spawn(move || {
            let receiver = MenuEvent::receiver();
            while let Ok(event) = receiver.recv() {
                match event.id.0.as_str() {
                    MENU_SHOW => {
                        ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
                        ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
                    }
                    MENU_PAUSE => {
                        let _ = commands.send(Command::Pause);
                    }
                    MENU_RESUME => {
                        let _ = commands.send(Command::Resume);
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
