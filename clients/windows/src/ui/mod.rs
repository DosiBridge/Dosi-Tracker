//! The desktop window: sign-in, the live timer, today's activity, and
//! settings — plus the tray icon, which it owns.
//!
//! Rendering is immediate-mode (egui) and reads only a cheap state snapshot, so
//! the UI stays responsive while the worker does network/capture work.
//!
//! Window behaviour lives in [`eframe::App::logic`], which eframe runs every
//! frame whether or not the window is on screen. [`eframe::App::ui`] is skipped
//! while the window is minimized, so anything the tray depends on (its status,
//! its menu actions, its notifications, quitting) must not live there — it did
//! once, and the tray went stale and "Quit" stopped working whenever the
//! window was minimized.
//!
//! This module holds the app's state and the frame loop; showing, hiding
//! and the tray are in [`window`], the page's frame and the shared layout
//! pieces in [`layout`], and each view (sign-in, timer, activity, settings)
//! in its own module.

pub mod consent;
pub mod format;
mod activity;
mod alerts;
mod layout;
mod login;
#[cfg(debug_assertions)]
mod preview;
mod settings;
mod timer;
mod widgets;
mod window;

use std::sync::mpsc::{Receiver, Sender};
use std::sync::Arc;
use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};
use eframe::egui;
use tokio::sync::mpsc::UnboundedSender;

use crate::autostart;
use crate::prefs::Preferences;
use crate::shell;
use crate::state::{AppSignal, Command, PendingAction, SharedState, StateHandle, TrackerStatus};
use crate::theme;
use crate::tracking::input::InputCounter;
use crate::tracking::system::DeviceInfo;
use crate::tracking::usage::UsageRecorder;
use crate::tray::{Tray, TrayEvent, TrayView};
use layout::{equal_row, metric, section, two_columns};
use widgets::Tone;
use window::{close_action, CloseAction, TRAY_RETRY_FIRST};

/// The timed pauses offered next to the pause button (and in the tray).
pub const PAUSE_CHOICES: [(&str, Duration); 3] = [
    ("15 min", Duration::from_secs(15 * 60)),
    ("30 min", Duration::from_secs(30 * 60)),
    ("1 hour", Duration::from_secs(60 * 60)),
];

/// Below this (either way) the window is not really on screen; nothing is laid out.
const MIN_LAYOUT_SIZE: f32 = 200.0;

/// How long a message at the top of the window stays once seen.
const TOAST_FOR: Duration = Duration::from_secs(8);
/// At most this many at once (the newest win).
const MAX_TOASTS: usize = 3;
/// The device's time zone is read again this often for Settings (it follows
/// the user when they travel).
const ZONE_REFRESH: Duration = Duration::from_secs(60);

/// Why the dashboard buttons are greyed out.
const DASHBOARD_UNSET: &str =
    "The web dashboard's address isn't set on this computer. Ask your admin, or set dashboard_url in config.toml.";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum View {
    Timer,
    Activity,
    Settings,
}

/// Endpoints the window links to.
pub struct Links {
    pub dashboard_url: String,
    pub api_base_url: String,
    /// `config.toml`'s `allow_insecure_http`: whether the sign-in form may
    /// accept a plain `http://` server on another computer.
    pub allow_insecure_http: bool,
}

/// How the window was asked to start.
pub struct Launch {
    /// Stay hidden in the tray (started by Windows at sign-in).
    pub start_hidden: bool,
}

/// A message at the top of the window for a few seconds.
struct Toast {
    tone: Tone,
    text: String,
    /// When it was first on screen: the countdown starts there, so a
    /// message that arrived while the window was closed is still seen.
    shown_at: Option<Instant>,
}

pub struct TrackerApp {
    state: StateHandle,
    commands: UnboundedSender<Command>,
    /// Live input of the block in progress (read-only), for the timer view.
    input: Arc<InputCounter>,
    /// Foreground-window usage (read-only), for the "top apps" list.
    usage: Arc<UsageRecorder>,
    links: Links,
    prefs: Preferences,
    view: View,

    tray: Option<Tray>,
    tray_events: Receiver<TrayEvent>,
    /// For building the tray icon again when it could not be created.
    tray_sender: Sender<TrayEvent>,
    /// Without a tray icon: when to try again, and the wait after that.
    tray_retry: Option<(Instant, Duration)>,
    /// Requests from another launch or the installer (show the window, quit).
    signals: Receiver<AppSignal>,

    login: login::LoginForm,
    task: timer::TaskField,
    /// The project chooser's filter.
    project_filter: String,

    // Consent
    pub consent_vm: consent::ConsentViewModel,

    // Settings
    autostart_enabled: bool,
    settings_error: Option<String>,
    confirm_sign_out: bool,
    /// The lost workspace last shown: a new loss opens Settings' picker once.
    lost_seen: Option<String>,
    /// This computer as the workspace sees it.
    device: DeviceInfo,
    /// "Asia/Dhaka (UTC+06:00)", and when it was read.
    zone: Option<(String, Instant)>,

    toasts: Vec<Toast>,
    /// The newest worker notice already shown.
    notices_seen: u64,
    /// The time-away question last announced from the tray.
    away_announced: Option<(DateTime<Utc>, DateTime<Utc>)>,
    /// The decoded latest screenshot, keyed by when it was taken.
    screenshot: Option<(DateTime<Utc>, egui::TextureHandle)>,
    /// Debug preview only: show the window as if the input hooks were down.
    input_down_preview: bool,

    window: WindowState,
    /// Set by "Quit": the next close is real instead of hiding to the tray.
    quitting: bool,
}

struct WindowState {
    /// Hidden to the tray (not just minimized).
    hidden: bool,
    /// Hide on the first frame (eframe always shows the window first).
    hide_on_start: bool,
    /// Started off-screen to avoid a flash; centre it when first shown.
    recenter_on_show: bool,
}

impl TrackerApp {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        ctx: &egui::Context,
        state: StateHandle,
        commands: UnboundedSender<Command>,
        input: Arc<InputCounter>,
        usage: Arc<UsageRecorder>,
        links: Links,
        launch: Launch,
        signals: Receiver<AppSignal>,
    ) -> Self {
        theme::apply(ctx);

        // The tray must be built on the UI thread (it needs the message loop).
        let (sender, tray_events) = std::sync::mpsc::channel();
        crate::tray::spawn_event_forwarders(ctx.clone(), sender.clone());
        let tray = match Tray::build(sender.clone(), ctx.clone()) {
            Ok(tray) => Some(tray),
            Err(e) => {
                tracing::error!(?e, "failed to create the tray icon; trying again shortly");
                None
            }
        };
        let tray_retry = tray.is_none().then(|| (Instant::now() + TRAY_RETRY_FIRST, TRAY_RETRY_FIRST));

        // Starting hidden needs somewhere to come back to: without a tray
        // icon the window must show.
        let start_hidden = launch.start_hidden && tray.is_some();

        // Debug-only: `DOSI_UI_VIEW=activity|settings` opens on that tab, and
        // `DOSI_UI_SCENE` adds a scene to the preview's demo data, for
        // reviewing layouts without clicking through.
        #[cfg(debug_assertions)]
        let (view, input_down_preview) = {
            let view = match std::env::var("DOSI_UI_VIEW").as_deref() {
                Ok("activity") => View::Activity,
                Ok("settings") => View::Settings,
                _ => View::Timer,
            };
            (view, preview::apply(&state).input_down)
        };
        #[cfg(not(debug_assertions))]
        let (view, input_down_preview) = (View::Timer, false);

        let login = login::LoginForm::new(&links.api_base_url);
        let initial_consent = state.snapshot().consent;
        Self {
            state,
            commands,
            input,
            usage,
            links,
            prefs: Preferences::load(),
            view,
            tray,
            tray_events,
            tray_sender: sender,
            tray_retry,
            signals,
            login,
            task: timer::TaskField::default(),
            project_filter: String::new(),
            consent_vm: consent::ConsentViewModel::new(initial_consent),
            autostart_enabled: autostart::is_enabled(),
            settings_error: None,
            confirm_sign_out: false,
            lost_seen: None,
            device: DeviceInfo::collect(),
            zone: None,
            toasts: Vec::new(),
            notices_seen: 0,
            away_announced: None,
            screenshot: None,
            input_down_preview,
            window: WindowState {
                hidden: false,
                hide_on_start: start_hidden,
                recenter_on_show: start_hidden,
            },
            quitting: false,
        }
    }

    fn send(&self, command: Command) {
        let _ = self.commands.send(command);
    }

    /// Pause or resume, showing it in the window at once (see
    /// [`PendingAction`]); the worker confirms when it has applied it.
    fn request(&self, action: PendingAction, command: Command) {
        self.state.update(|s| s.pending_action = Some(action));
        self.send(command);
    }

    /// Pause (or re-arm a pause). The click's time travels with the command:
    /// the block ends exactly there, however busy the worker is.
    fn pause(&self, snapshot: &SharedState, resume_after: Option<Duration>) {
        let idle = snapshot.pending_action.is_none();
        let at = Utc::now();
        if snapshot.status == TrackerStatus::Paused {
            // Already paused: only when (or whether) it ends changes.
            self.send(Command::Pause { resume_after, at });
        } else if idle && format::can_pause(snapshot.status) {
            self.request(PendingAction::Pausing, Command::Pause { resume_after, at });
        }
    }

    fn resume(&self, snapshot: &SharedState) {
        if snapshot.status == TrackerStatus::Paused && snapshot.pending_action.is_none() {
            self.request(PendingAction::Resuming, Command::Resume { at: Utc::now() });
        }
    }

    /// Are keyboard and mouse being measured?
    fn input_healthy(&self) -> bool {
        self.input.hook_healthy() && !self.input_down_preview
    }

    /// A message at the top of the window; the same one again only restarts
    /// its countdown.
    fn show_toast(&mut self, tone: Tone, text: impl Into<String>) {
        let text = text.into();
        self.toasts.retain(|toast| toast.text != text);
        self.toasts.push(Toast { tone, text, shown_at: None });
        let excess = self.toasts.len().saturating_sub(MAX_TOASTS);
        self.toasts.drain(..excess);
    }

    /// The pending messages, each with a close button.
    fn toasts(&mut self, ui: &mut egui::Ui) {
        let now = Instant::now();
        self.toasts.retain(|toast| toast.shown_at.is_none_or(|at| now.duration_since(at) < TOAST_FOR));
        let mut closed = None;
        for (i, toast) in self.toasts.iter_mut().enumerate() {
            toast.shown_at.get_or_insert(now);
            if widgets::toast(ui, toast.tone, &toast.text) {
                closed = Some(i);
            }
        }
        if let Some(i) = closed {
            self.toasts.remove(i);
        }
    }

    /// The server this account talks to (the configured one before sign-in).
    fn server_url<'a>(&'a self, snapshot: &'a SharedState) -> &'a str {
        if snapshot.server_url.trim().is_empty() {
            &self.links.api_base_url
        } else {
            &snapshot.server_url
        }
    }

    fn dashboard_available(&self, snapshot: &SharedState) -> bool {
        dashboard_configured(&self.links.dashboard_url, self.server_url(snapshot))
    }

    fn open_dashboard(&mut self, snapshot: &SharedState) {
        if !self.dashboard_available(snapshot) {
            self.show_toast(Tone::Warning, DASHBOARD_UNSET);
            return;
        }
        let url = self.links.dashboard_url.clone();
        if let Err(e) = shell::open(&url) {
            tracing::warn!(?e, %url, "could not open the dashboard");
            self.show_toast(Tone::Warning, format!("Could not open {url}"));
        }
    }

    fn open_folder(&mut self, folder: Option<std::path::PathBuf>) {
        let Some(folder) = folder else {
            self.show_toast(Tone::Warning, "That folder could not be found.");
            return;
        };
        let _ = std::fs::create_dir_all(&folder);
        if let Err(e) = shell::open(&folder.to_string_lossy()) {
            tracing::warn!(?e, "could not open a folder");
            self.show_toast(Tone::Warning, "Could not open the folder.");
        }
    }

    fn set_autostart(&mut self, enabled: bool) {
        match autostart::set_enabled(enabled) {
            Ok(()) => self.settings_error = None,
            Err(e) => self.settings_error = Some(e.to_string()),
        }
        // Reflect the real registry state, whether or not the write worked.
        self.autostart_enabled = autostart::is_enabled();
        if let Some(tray) = &mut self.tray {
            tray.invalidate();
        }
    }

    fn save_prefs(&mut self) {
        self.prefs = self.prefs.clone().normalized();
        if let Err(e) = self.prefs.save() {
            tracing::warn!(?e, "could not save preferences");
            self.settings_error = Some(format!("Could not save your preferences: {e}"));
        }
    }

    /// "Asia/Dhaka (UTC+06:00)": the device's zone, read again now and then.
    fn zone_label(&mut self) -> String {
        let fresh = self.zone.as_ref().is_some_and(|(_, at)| at.elapsed() < ZONE_REFRESH);
        if !fresh {
            let offset = format::utc_offset_label(crate::tracking::system::utc_offset_minutes(Utc::now()));
            let label = match crate::tracking::system::time_zone() {
                Some(zone) => format!("{zone} ({offset})"),
                None => offset,
            };
            self.zone = Some((label, Instant::now()));
        }
        self.zone.as_ref().map(|(label, _)| label.clone()).unwrap_or_default()
    }

}

/// Is the web dashboard's address known? The address shipped by default
/// (`http://localhost:6005`) next to a server elsewhere is one nobody set —
/// opening it would only show an error page.
fn dashboard_configured(dashboard_url: &str, server_url: &str) -> bool {
    match (host_of(dashboard_url), host_of(server_url)) {
        (None, _) => false,
        (Some(dashboard), Some(server)) => !is_loopback(&dashboard) || is_loopback(&server),
        (Some(_), None) => true,
    }
}

fn host_of(url: &str) -> Option<String> {
    let parsed = reqwest::Url::parse(url.trim()).ok()?;
    parsed.host_str().map(str::to_lowercase)
}

/// This computer itself: `localhost`, `*.localhost`, 127.0.0.0/8 or ::1.
fn is_loopback(host: &str) -> bool {
    let host = host.trim_start_matches('[').trim_end_matches(']');
    host == "localhost" || host.ends_with(".localhost") || host.parse::<std::net::IpAddr>().is_ok_and(|ip| ip.is_loopback())
}

impl eframe::App for TrackerApp {
    /// Clear the framebuffer to the app surface colour. Doing this here (rather
    /// than painting a rect) guarantees full coverage — any area egui does not
    /// draw would otherwise show through as black.
    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        let c = theme::BACKGROUND;
        [c.r() as f32 / 255.0, c.g() as f32 / 255.0, c.b() as f32 / 255.0, 1.0]
    }

    /// Runs every frame, shown or not: window behaviour and the tray.
    fn logic(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        let snapshot = self.state.snapshot();
        let now = Utc::now();

        // The window can be brought back without us (a second launch focuses
        // it), so trust what Windows says over our own flag — and a window
        // that started off-screen is centred whichever way it comes back.
        if let Some(shown) = shell::is_window_shown(frame) {
            if shown && self.window.hidden && !self.window.hide_on_start {
                self.window.hidden = false;
                self.recenter_if_needed(ctx);
                ctx.request_repaint();
            }
        }
        if std::mem::take(&mut self.window.hide_on_start) {
            self.hide_window(ctx);
        }

        self.retry_tray(ctx);
        while let Ok(event) = self.tray_events.try_recv() {
            self.on_tray_event(ctx, event, &snapshot);
        }
        // Another launch wants the window (it re-centres a window that started
        // off-screen), or the installer wants the app to quit.
        while let Ok(signal) = self.signals.try_recv() {
            match signal {
                AppSignal::Show => self.show_window(ctx),
                AppSignal::Quit => self.quit(ctx),
            }
        }

        let minimized = ctx.input(|i| i.viewport().minimized).unwrap_or(false);
        if self.quitting {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        } else if ctx.input(|i| i.viewport().close_requested()) {
            // Closing the window hides to tray instead of exiting — this is a
            // background agent, so the X button must not stop tracking. With
            // no tray icon to come back from, it only minimizes.
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            match close_action(self.tray.is_some()) {
                CloseAction::HideToTray => self.hide_window(ctx),
                CloseAction::Minimize => ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(true)),
            }
        } else if minimized && self.prefs.minimize_to_tray && !self.window.hidden && self.tray.is_some() {
            self.hide_window(ctx);
        }

        let focused = ctx.input(|i| i.viewport().focused).unwrap_or(false);
        let in_front = !self.window.hidden && !minimized && focused;
        self.deliver_notices(&snapshot, in_front);

        let dashboard = self.dashboard_available(&snapshot);
        let autostart = self.autostart_enabled;
        if let Some(tray) = &mut self.tray {
            let totals = format::live_totals(&snapshot, now);
            tray.sync(TrayView::new(&snapshot, now, totals, autostart, dashboard));
        }

        // Timers tick every second on screen; hidden, only the tray tooltip
        // (to the minute) needs refreshing.
        let idle = self.window.hidden || minimized;
        ctx.request_repaint_after(if idle { Duration::from_secs(5) } else { Duration::from_secs(1) });
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let snapshot = self.state.snapshot();
        let now = Utc::now();

        // eframe 0.35 hands us a Ui with no background, so paint the app surface
        // across the whole viewport first.
        let viewport = ui.max_rect();
        ui.painter().rect_filled(viewport, egui::CornerRadius::ZERO, theme::BACKGROUND);

        // A window starting minimized can report a sliver of a size for a
        // frame. Laying the page out into it inverts the content area and
        // leaves the scroll position stranded; there is nothing to show anyway.
        if viewport.width() < MIN_LAYOUT_SIZE || viewport.height() < MIN_LAYOUT_SIZE {
            return;
        }

        // An expired sign-in asks for the password on the same screen.
        if matches!(snapshot.status, TrackerStatus::SignedOut | TrackerStatus::SignInExpired) {
            self.confirm_sign_out = false;
            self.view = View::Timer;
            self.login_view(ui, &snapshot);
        } else {
            self.login.confirm_switch = false;
            if snapshot.workspace_lost != self.lost_seen {
                self.lost_seen = snapshot.workspace_lost.clone();
                if self.lost_seen.is_some() {
                    self.view = View::Settings;
                }
            }
            self.signed_in_layout(ui, &snapshot, now);
        }
    }
}

#[cfg(test)]
mod tests;
