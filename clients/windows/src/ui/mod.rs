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

pub mod format;
mod activity;
mod alerts;
mod login;
#[cfg(debug_assertions)]
mod preview;
mod settings;
mod timer;
mod widgets;

use std::sync::mpsc::Receiver;
use std::sync::Arc;
use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};
use eframe::egui;
use egui::{Align, Color32, Layout, Pos2, Rect, Stroke, UiBuilder, Vec2};
use tokio::sync::mpsc::UnboundedSender;

use crate::autostart;
use crate::prefs::Preferences;
use crate::shell;
use crate::state::{AppSignal, AwayPrompt, Command, NoticeKind, PendingAction, SharedState, StateHandle, TrackerStatus, UploadIssue};
use crate::theme;
use crate::tracking::input::InputCounter;
use crate::tracking::system::DeviceInfo;
use crate::tracking::usage::UsageRecorder;
use crate::tray::{Tray, TrayAction, TrayEvent, TrayView};
use widgets::{Icon, Tone};

/// The timed pauses offered next to the pause button (and in the tray).
pub const PAUSE_CHOICES: [(&str, Duration); 3] = [
    ("15 min", Duration::from_secs(15 * 60)),
    ("30 min", Duration::from_secs(30 * 60)),
    ("1 hour", Duration::from_secs(60 * 60)),
];

/// Content wider than this gets a two-column layout.
const WIDE_LAYOUT: f32 = 760.0;
const HEADER_H: f32 = 60.0;
const TABS_H: f32 = 56.0;
const FOOTER_H: f32 = 30.0;
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
    /// Requests from another launch or the installer (show the window, quit).
    signals: Receiver<AppSignal>,

    login: login::LoginForm,
    task: timer::TaskField,
    /// The project chooser's filter.
    project_filter: String,

    // Settings
    autostart_enabled: bool,
    settings_error: Option<String>,
    confirm_sign_out: bool,
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
        let tray = match Tray::build(sender, ctx.clone()) {
            Ok(tray) => Some(tray),
            Err(e) => {
                tracing::error!(?e, "failed to create the tray icon");
                None
            }
        };

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
            signals,
            login,
            task: timer::TaskField::default(),
            project_filter: String::new(),
            autostart_enabled: autostart::is_enabled(),
            settings_error: None,
            confirm_sign_out: false,
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

    // ---- Window management ------------------------------------------------------

    /// A window that started off-screen (an autostart into the tray) is
    /// centred the first time it is shown, however that happens.
    fn recenter_if_needed(&mut self, ctx: &egui::Context) {
        if std::mem::take(&mut self.window.recenter_on_show) {
            let size = ctx
                .input(|i| i.viewport().outer_rect)
                .map(|r| [r.width(), r.height()])
                .unwrap_or(crate::DEFAULT_WINDOW_SIZE);
            if let Some(area) = shell::work_area() {
                ctx.send_viewport_cmd(egui::ViewportCommand::OuterPosition(area.centered(size).into()));
            }
        }
    }

    fn show_window(&mut self, ctx: &egui::Context) {
        self.recenter_if_needed(ctx);
        ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
        ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
        ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
        self.window.hidden = false;
        // Paint straight away: a window shown from the tray must never sit
        // blank until the mouse happens to move over it.
        ctx.request_repaint();
    }

    fn hide_window(&mut self, ctx: &egui::Context) {
        ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
        self.window.hidden = true;
    }

    fn quit(&mut self, ctx: &egui::Context) {
        if !self.quitting {
            self.quitting = true;
            self.send(Command::Shutdown { at: Utc::now() });
        }
        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        ctx.request_repaint();
    }

    fn on_tray_event(&mut self, ctx: &egui::Context, event: TrayEvent, snapshot: &SharedState) {
        match event {
            TrayEvent::IconClicked | TrayEvent::Action(TrayAction::Open) => self.show_window(ctx),
            // Notifications are about the timer: the time-away question, a
            // project to choose, tracking that started.
            TrayEvent::NotificationClicked => {
                self.view = View::Timer;
                self.show_window(ctx);
            }
            TrayEvent::Action(TrayAction::ToggleTracking) => match snapshot.status {
                TrackerStatus::SignedOut | TrackerStatus::SignInExpired | TrackerStatus::ChooseProject => {
                    self.view = View::Timer;
                    self.show_window(ctx);
                }
                TrackerStatus::Paused => self.resume(snapshot),
                _ => self.pause(snapshot, None),
            },
            TrayEvent::Action(TrayAction::Break(duration)) => self.pause(snapshot, Some(duration)),
            TrayEvent::Action(TrayAction::SelectProject(id)) => {
                if snapshot.selected_project.as_deref() != Some(id.as_str()) {
                    self.send(Command::SelectProject { project_id: id, at: Utc::now() });
                }
            }
            TrayEvent::Action(TrayAction::SetTask) => {
                self.view = View::Timer;
                self.task.focus_requested = true;
                self.show_window(ctx);
            }
            TrayEvent::Action(TrayAction::SyncNow) => self.send(Command::SyncNow { at: Utc::now() }),
            TrayEvent::Action(TrayAction::OpenDashboard) => self.open_dashboard(snapshot),
            TrayEvent::Action(TrayAction::OpenSettings) => {
                self.view = View::Settings;
                self.show_window(ctx);
            }
            TrayEvent::Action(TrayAction::ToggleAutostart) => self.set_autostart(!self.autostart_enabled),
            TrayEvent::Action(TrayAction::Quit) => self.quit(ctx),
        }
        if let Some(tray) = &mut self.tray {
            tray.invalidate();
        }
    }

    /// Tell the user what the worker reported, once: in the window, and —
    /// while the window is not in front — from the tray as well. The
    /// time-away question is announced from the tray the same way (the
    /// window shows it until it is answered).
    fn deliver_notices(&mut self, snapshot: &SharedState, in_front: bool) {
        let fresh: Vec<_> = snapshot.notices.iter().filter(|n| n.id > self.notices_seen).cloned().collect();
        for notice in fresh {
            self.notices_seen = notice.id;
            let tone = notice_tone(&notice.kind);
            let text = notice.text();
            if !in_front && self.prefs.show_notifications {
                let warning = matches!(tone, Tone::Warning | Tone::Danger);
                if let Some(tray) = &mut self.tray {
                    tray.notify(notice_title(&notice.kind), &text, warning);
                }
            }
            // An expired sign-in replaces the window with its own form.
            if notice.kind != NoticeKind::SignInExpired {
                self.show_toast(tone, text);
            }
        }

        if let Some(prompt) = &snapshot.away_prompt {
            let key = (prompt.from, prompt.to);
            if self.away_announced != Some(key) {
                self.away_announced = Some(key);
                if !in_front && self.prefs.show_notifications {
                    if let Some(tray) = &mut self.tray {
                        tray.notify("Welcome back", &away_notification(prompt), false);
                    }
                }
            }
        }
    }

    // ---- Layout -----------------------------------------------------------------

    fn signed_in_layout(&mut self, ui: &mut egui::Ui, snapshot: &SharedState, now: DateTime<Utc>) {
        let full = ui.max_rect();
        let painter = ui.painter().clone();

        // Header bar.
        let header = Rect::from_min_size(full.min, Vec2::new(full.width(), HEADER_H));
        painter.rect_filled(header, 0, theme::CARD);
        painter.line_segment([header.left_bottom(), header.right_bottom()], Stroke::new(1.0, theme::BORDER));
        let mut header_ui = ui.new_child(
            UiBuilder::new()
                .max_rect(header.shrink2(Vec2::new(theme::PAGE_PAD, 0.0)))
                .layout(Layout::left_to_right(Align::Center)),
        );
        self.header(&mut header_ui, snapshot);

        // Footer bar.
        let footer = Rect::from_min_max(Pos2::new(full.left(), full.bottom() - FOOTER_H), full.max);
        painter.rect_filled(footer, 0, theme::CARD);
        painter.line_segment([footer.left_top(), footer.right_top()], Stroke::new(1.0, theme::BORDER));
        let mut footer_ui = ui.new_child(
            UiBuilder::new()
                .max_rect(footer.shrink2(Vec2::new(theme::PAGE_PAD, 0.0)))
                .layout(Layout::left_to_right(Align::Center)),
        );
        self.footer(&mut footer_ui, snapshot);

        // Tabs, fixed under the header.
        let tabs = Rect::from_min_max(Pos2::new(full.left(), header.bottom()), Pos2::new(full.right(), header.bottom() + TABS_H));
        let mut tabs_ui = ui.new_child(UiBuilder::new().max_rect(tabs).layout(Layout::top_down(Align::Center)));
        self.tabs(&mut tabs_ui);

        // Scrolling content.
        let body = Rect::from_min_max(Pos2::new(full.left(), tabs.bottom()), Pos2::new(full.right(), footer.top()));
        let mut body_ui = ui.new_child(UiBuilder::new().max_rect(body).layout(Layout::top_down(Align::Min)));
        egui::ScrollArea::vertical()
            .id_salt(("content", self.view as u8))
            .auto_shrink([false, false])
            .show(&mut body_ui, |ui| {
                let available = ui.available_width();
                let max_width = if available - 2.0 * theme::PAGE_PAD >= WIDE_LAYOUT { 1040.0 } else { 640.0 };
                let width = (available - 2.0 * theme::PAGE_PAD).min(max_width).max(0.0);
                let side = ((available - width) / 2.0).max(0.0);
                ui.add_space(2.0);
                ui.horizontal_top(|ui| {
                    ui.add_space(side - ui.spacing().item_spacing.x);
                    ui.vertical(|ui| {
                        ui.set_width(width);
                        ui.spacing_mut().item_spacing.y = theme::GAP;
                        self.toasts(ui);
                        self.away_prompt_card(ui, snapshot);
                        let wide = width >= WIDE_LAYOUT;
                        match self.view {
                            View::Timer => self.timer_view(ui, snapshot, now, wide),
                            View::Activity => self.activity_view(ui, snapshot, now, wide),
                            View::Settings => self.settings_view(ui, snapshot),
                        }
                        ui.add_space(theme::PAGE_PAD);
                    });
                });
            });
    }

    fn header(&self, ui: &mut egui::Ui, snapshot: &SharedState) {
        theme::logo_mark(ui, 32.0);
        ui.add_space(4.0);
        ui.vertical(|ui| {
            ui.add_space(12.0);
            ui.spacing_mut().item_spacing.y = 0.0;
            ui.label(egui::RichText::new("Dosi Tracker").font(theme::semibold(15.0)).color(theme::INK));
            let account = match (snapshot.workspace.trim(), snapshot.display_name.trim()) {
                ("", "") => String::new(),
                ("", name) => name.to_owned(),
                (workspace, "") => workspace.to_owned(),
                (workspace, name) => format!("{workspace} · {name}"),
            };
            ui.add(egui::Label::new(egui::RichText::new(account).font(theme::regular(11.5)).color(theme::MUTED_FG)).truncate());
        });
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            let (tone, label) = status_visual(snapshot.status, snapshot.pending_action);
            widgets::status_pill(ui, tone, label);
        });
    }

    /// The segmented control that switches views.
    fn tabs(&mut self, ui: &mut egui::Ui) {
        let tabs = [(View::Timer, Icon::Timer, "Timer"), (View::Activity, Icon::Chart, "Activity"), (View::Settings, Icon::Sliders, "Settings")];
        let width = (ui.available_width() - 2.0 * theme::PAGE_PAD).min(440.0);
        let height = 38.0;
        ui.add_space(10.0);
        let (outer, _) = ui.allocate_exact_size(Vec2::new(width, height), egui::Sense::hover());
        let painter = ui.painter();
        painter.rect_filled(outer, egui::CornerRadius::same(theme::RADIUS), theme::mix(theme::BORDER, theme::BACKGROUND, 0.55));
        let inner = outer.shrink(3.0);
        let slot = inner.width() / tabs.len() as f32;
        for (i, (view, icon, label)) in tabs.into_iter().enumerate() {
            let rect = Rect::from_min_size(Pos2::new(inner.left() + slot * i as f32, inner.top()), Vec2::new(slot, inner.height()));
            let response = ui
                .interact(rect, ui.id().with(("tab", i)), egui::Sense::click())
                .on_hover_cursor(egui::CursorIcon::PointingHand);
            if response.clicked() {
                self.view = view;
                self.confirm_sign_out = false;
            }
            let selected = self.view == view;
            let painter = ui.painter();
            if selected {
                painter.rect(
                    rect,
                    egui::CornerRadius::same(theme::RADIUS - 2),
                    theme::CARD,
                    Stroke::new(1.0, theme::BORDER),
                    egui::StrokeKind::Inside,
                );
            } else if response.hovered() {
                painter.rect_filled(rect, egui::CornerRadius::same(theme::RADIUS - 2), theme::mix(theme::CARD, theme::BORDER, 0.5));
            }
            let color = if selected { theme::INK } else { theme::MUTED_FG };
            let font = if selected { theme::semibold(13.0) } else { theme::regular(13.0) };
            let galley = painter.layout_no_wrap(label.to_owned(), font, color);
            let content_w = 16.0 + 7.0 + galley.size().x;
            let x = rect.center().x - content_w / 2.0;
            let icon_color = if selected { theme::PRIMARY } else { theme::MUTED_FG };
            widgets::paint_icon(painter, Rect::from_center_size(Pos2::new(x + 8.0, rect.center().y), Vec2::splat(15.0)), icon, icon_color);
            painter.galley(Pos2::new(x + 23.0, rect.center().y - galley.size().y / 2.0), galley, color);
        }
    }

    fn footer(&self, ui: &mut egui::Ui, snapshot: &SharedState) {
        let (tone, text) = sync_status(snapshot);
        let (dot, _, _) = tone.colors();
        let (rect, _) = ui.allocate_exact_size(Vec2::splat(8.0), egui::Sense::hover());
        ui.painter().circle_filled(rect.center(), 3.5, dot);
        let label = ui.label(egui::RichText::new(text).font(theme::regular(11.5)).color(theme::MUTED_FG));
        // Why uploads are held back, and when they are tried again.
        if let Some(issue) = &snapshot.upload_issue {
            label.on_hover_text(format!("{} Next try at {}.", issue.message(), format::local_hm(issue.next_attempt_at())));
        }
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            ui.label(egui::RichText::new(concat!("v", env!("CARGO_PKG_VERSION"))).font(theme::regular(11.5)).color(theme::FAINT));
        });
    }
}

/// Tone + label for the status pill; a pause/resume in flight shows first.
fn status_visual(status: TrackerStatus, pending: Option<PendingAction>) -> (Tone, &'static str) {
    match pending {
        Some(PendingAction::Pausing) => return (Tone::Warning, "Pausing…"),
        Some(PendingAction::Resuming) => return (Tone::Success, "Resuming…"),
        None => {}
    }
    match status {
        TrackerStatus::Tracking => (Tone::Success, status.label()),
        TrackerStatus::Paused => (Tone::Warning, status.label()),
        TrackerStatus::Offline => (Tone::Warning, "Offline"),
        TrackerStatus::Connecting => (Tone::Info, status.label()),
        TrackerStatus::SignedOut => (Tone::Neutral, status.label()),
        TrackerStatus::Stopped => (Tone::Danger, status.label()),
        TrackerStatus::Away | TrackerStatus::Locked => (Tone::Warning, status.label()),
        TrackerStatus::ChooseProject => (Tone::Info, status.label()),
        TrackerStatus::SignInExpired => (Tone::Danger, status.label()),
    }
}

/// The footer's one-line upload state: the most important thing first.
fn sync_status(snapshot: &SharedState) -> (Tone, String) {
    let waiting = snapshot.pending_uploads;
    let offline = snapshot.status == TrackerStatus::Offline || !snapshot.server_reachable;
    if snapshot.rejected_uploads > 0 {
        (Tone::Danger, format!("{} not accepted by the server", format::count(snapshot.rejected_uploads, "block")))
    } else if offline {
        if waiting > 0 {
            (Tone::Warning, format!("Offline — {} waiting to sync", format::count(waiting, "block")))
        } else {
            (Tone::Warning, "Offline — uploads will retry".to_owned())
        }
    } else if let Some(UploadIssue::SubscriptionInactive { .. }) = snapshot.upload_issue {
        (Tone::Warning, format!("Subscription inactive — {} kept here", format::count(waiting, "block")))
    } else if let Some(UploadIssue::ClockAhead { .. }) = snapshot.upload_issue {
        (Tone::Warning, format!("Clock ahead — {} waiting to sync", format::count(waiting, "block")))
    } else if waiting > 0 {
        (Tone::Warning, format!("{} waiting to sync", format::count(waiting, "block")))
    } else if let Some(synced) = snapshot.last_sync {
        (Tone::Success, format!("All synced · {}", format::local_hm(synced)))
    } else {
        (Tone::Neutral, "Nothing to sync yet".to_owned())
    }
}

/// How a worker notice looks in the window.
fn notice_tone(kind: &NoticeKind) -> Tone {
    match kind {
        NoticeKind::TrackingStartedAutomatically { .. } | NoticeKind::ChooseProject => Tone::Info,
        NoticeKind::ProjectUnavailable { .. } | NoticeKind::QueueReset { .. } => Tone::Warning,
        NoticeKind::SignInExpired => Tone::Danger,
    }
}

/// A tray notification's title.
fn notice_title(kind: &NoticeKind) -> &'static str {
    match kind {
        NoticeKind::TrackingStartedAutomatically { .. } => "Tracking started",
        NoticeKind::ChooseProject => "Choose a project",
        NoticeKind::ProjectUnavailable { .. } => "Project no longer available",
        NoticeKind::QueueReset { .. } => "Local queue reset",
        NoticeKind::SignInExpired => "Sign-in expired",
    }
}

/// The tray's version of the time-away question.
fn away_notification(prompt: &AwayPrompt) -> String {
    format!(
        "You were away for {}. Open Dosi Tracker to keep it as work — otherwise it isn't recorded.",
        format::away_summary(prompt.from, prompt.to)
    )
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
            // background agent, so the X button must not stop tracking.
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.hide_window(ctx);
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
            self.signed_in_layout(ui, &snapshot, now);
        }
    }
}

/// A card with a section header; the body fills the card's width.
fn section<R>(ui: &mut egui::Ui, title: &str, trailing: Option<&str>, body: impl FnOnce(&mut egui::Ui) -> R) -> R {
    theme::card(16)
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 8.0;
            widgets::section_header(ui, title, trailing);
            body(ui)
        })
        .inner
}

/// Two equal columns; `column` receives the column index and its Ui. (egui's
/// own columns are justified, which stretches the spacing of wrapped text.)
fn two_columns(ui: &mut egui::Ui, mut column: impl FnMut(usize, &mut egui::Ui)) {
    ui.columns(2, |columns| {
        for (i, col) in columns.iter_mut().enumerate() {
            col.with_layout(Layout::top_down(Align::Min), |ui| {
                ui.spacing_mut().item_spacing.y = theme::GAP;
                column(i, ui);
            });
        }
    });
}

/// A row of `n` equal cells across the available width; `cell` receives each
/// cell's index and a Ui sized to it.
fn equal_row(ui: &mut egui::Ui, n: usize, gap: f32, mut cell: impl FnMut(usize, &mut egui::Ui)) {
    let width = ui.available_width();
    let cell_w = ((width - gap * (n as f32 - 1.0)) / n as f32).max(0.0);
    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing.x = gap;
        for i in 0..n {
            ui.allocate_ui_with_layout(Vec2::new(cell_w, 0.0), Layout::top_down(Align::Min), |ui| {
                ui.set_width(cell_w);
                cell(i, ui);
            });
        }
    });
}

/// Paint a "label: value" metric pair (small caps label, strong value).
fn metric(ui: &mut egui::Ui, label: &str, value: &str, color: Color32) {
    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing.y = 1.0;
        ui.label(egui::RichText::new(label.to_uppercase()).font(theme::semibold(10.0)).color(theme::MUTED_FG));
        ui.label(egui::RichText::new(value).font(theme::semibold(15.0)).color(color));
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::Notice;

    #[test]
    fn footer_reports_the_most_important_upload_state() {
        let mut s = SharedState { status: TrackerStatus::Tracking, ..SharedState::default() };
        assert_eq!(sync_status(&s), (Tone::Neutral, "Nothing to sync yet".to_owned()));
        s.pending_uploads = 2;
        assert_eq!(sync_status(&s), (Tone::Warning, "2 blocks waiting to sync".to_owned()));
        // Tracking goes on offline; the footer says where the blocks are.
        s.server_reachable = false;
        assert_eq!(sync_status(&s).1, "Offline — 2 blocks waiting to sync");
        s.status = TrackerStatus::Offline;
        s.pending_uploads = 0;
        assert_eq!(sync_status(&s).1, "Offline — uploads will retry");
        s.rejected_uploads = 1;
        assert_eq!(sync_status(&s), (Tone::Danger, "1 block not accepted by the server".to_owned()));
    }

    #[test]
    fn held_back_uploads_say_why_in_the_footer() {
        let at = Utc::now();
        let mut s = SharedState { status: TrackerStatus::Tracking, pending_uploads: 3, ..SharedState::default() };
        s.upload_issue = Some(UploadIssue::SubscriptionInactive { next_attempt_at: at, message: String::new() });
        assert_eq!(sync_status(&s).1, "Subscription inactive — 3 blocks kept here");
        s.upload_issue = Some(UploadIssue::ClockAhead { next_attempt_at: at });
        assert_eq!(sync_status(&s).1, "Clock ahead — 3 blocks waiting to sync");
        s.upload_issue = None;
        s.pending_uploads = 0;
        s.last_sync = Some(at);
        assert_eq!(sync_status(&s).0, Tone::Success);
    }

    #[test]
    fn a_request_in_flight_shows_before_the_status() {
        assert_eq!(status_visual(TrackerStatus::Tracking, Some(PendingAction::Pausing)), (Tone::Warning, "Pausing…"));
        assert_eq!(status_visual(TrackerStatus::Paused, None), (Tone::Warning, "Paused"));
        assert_eq!(status_visual(TrackerStatus::Stopped, None).0, Tone::Danger);
        assert_eq!(status_visual(TrackerStatus::SignInExpired, None), (Tone::Danger, "Sign-in expired"));
    }

    #[test]
    fn notices_read_with_their_weight() {
        assert_eq!(notice_tone(&NoticeKind::SignInExpired), Tone::Danger);
        assert_eq!(notice_tone(&NoticeKind::QueueReset { moved_to: "x".into() }), Tone::Warning);
        let started = NoticeKind::TrackingStartedAutomatically { project_title: "Website".into() };
        assert_eq!(notice_tone(&started), Tone::Info);
        assert_eq!(notice_title(&started), "Tracking started");
        let notice = Notice { id: 1, at: Utc::now(), kind: started };
        assert!(notice.text().len() < 256, "fits a tray notification");
    }

    #[test]
    fn the_away_notification_quotes_the_span() {
        let from = Utc::now() - chrono::Duration::minutes(42);
        let prompt = AwayPrompt { from, to: from + chrono::Duration::minutes(42), project_title: "Website".into() };
        let text = away_notification(&prompt);
        assert!(text.starts_with("You were away for 42 min ("), "{text}");
        assert!(text.chars().count() < 256);
    }

    #[test]
    fn an_unconfigured_dashboard_next_to_a_real_server_is_not_offered() {
        let shipped = "http://localhost:6005";
        assert!(dashboard_configured(shipped, "https://localhost:6006"), "local development");
        assert!(dashboard_configured(shipped, "http://127.0.0.1:6006"));
        assert!(!dashboard_configured(shipped, "https://tracker.acme.com"));
        assert!(dashboard_configured("https://app.acme.com", "https://api.acme.com"));
        assert!(dashboard_configured("https://app.acme.com", "not a url"));
        assert!(!dashboard_configured("", "https://api.acme.com"));
        assert!(dashboard_configured("http://[::1]:6005", "http://[::1]:6006"));
    }

    #[test]
    fn loopback_hosts_are_recognised() {
        for host in ["localhost", "app.localhost", "127.0.0.1", "127.1.2.3", "[::1]", "::1"] {
            assert!(is_loopback(host), "{host}");
        }
        for host in ["tracker.acme.com", "10.0.0.5", "localhost.acme.com"] {
            assert!(!is_loopback(host), "{host}");
        }
    }
}
