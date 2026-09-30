//! The desktop window: sign-in, live tracking status, and settings.
//!
//! Rendering is immediate-mode (egui) and reads only a cheap state snapshot, so
//! the UI stays responsive while the worker does network/capture work.
//! Visual language mirrors the web dashboard — see `theme.rs`.

use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Local, Utc};
use eframe::egui;
use tokio::sync::mpsc::UnboundedSender;

use crate::autostart;
use crate::state::{Command, PendingAction, SharedState, StateHandle, TrackerStatus};
use crate::theme;
use crate::tracking::input::InputCounter;
use crate::worker::{local_day_and_week_start, tracked_secs_since};

/// The timed pauses offered next to the pause button (and in the tray).
pub const PAUSE_CHOICES: [(&str, Duration); 3] = [
    ("15 min", Duration::from_secs(15 * 60)),
    ("30 min", Duration::from_secs(30 * 60)),
    ("1 hour", Duration::from_secs(60 * 60)),
];

#[derive(PartialEq, Eq)]
enum View {
    Main,
    Settings,
}

pub struct TrackerApp {
    state: StateHandle,
    commands: UnboundedSender<Command>,
    /// Live input of the block in progress (read-only), for the session card.
    input: Arc<InputCounter>,
    view: View,
    themed: bool,

    // Login form
    workspace: String,
    email: String,
    password: String,
    show_password: bool,

    // Settings
    autostart_enabled: bool,
    autostart_error: Option<String>,

    /// Set by the tray "Quit" item so the window close actually exits.
    quit_requested: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

impl TrackerApp {
    pub fn new(
        state: StateHandle,
        commands: UnboundedSender<Command>,
        input: Arc<InputCounter>,
        quit_requested: std::sync::Arc<std::sync::atomic::AtomicBool>,
    ) -> Self {
        Self {
            state,
            commands,
            input,
            view: View::Main,
            themed: false,
            workspace: String::new(),
            email: String::new(),
            password: String::new(),
            show_password: false,
            autostart_enabled: autostart::is_enabled(),
            autostart_error: None,
            quit_requested,
        }
    }

    fn send(&self, command: Command) {
        let _ = self.commands.send(command);
    }

    /// Pause or resume, showing it in the window at once (see
    /// [`PendingAction`]); the worker confirms when it has applied it.
    fn request(&self, action: PendingAction, command: Command) {
        request_action(&self.state, &self.commands, action, command);
    }
}

/// Mark `action` as pending and send `command` — shared by the window and the
/// tray so both give the same instant feedback.
pub fn request_action(
    state: &StateHandle,
    commands: &UnboundedSender<Command>,
    action: PendingAction,
    command: Command,
) {
    state.update(|s| s.pending_action = Some(action));
    let _ = commands.send(command);
}

fn format_minutes(total: u64) -> String {
    let hours = total / 60;
    let minutes = total % 60;
    if hours > 0 {
        format!("{hours}h {minutes:02}m")
    } else {
        format!("{minutes}m")
    }
}

/// A stopwatch reading: `1:05:09`, or `05:09` under an hour.
fn format_clock(elapsed: chrono::Duration) -> String {
    let total = elapsed.num_seconds().max(0);
    let (hours, minutes, seconds) = (total / 3600, total % 3600 / 60, total % 60);
    if hours > 0 {
        format!("{hours}:{minutes:02}:{seconds:02}")
    } else {
        format!("{minutes:02}:{seconds:02}")
    }
}

/// Today / this week including the block in progress, in seconds. The worker
/// publishes the saved totals; the running block is added here every frame.
fn live_totals(snapshot: &SharedState, now: DateTime<Utc>) -> (f64, f64) {
    let (mut today, mut week) = (snapshot.tracked_today_secs, snapshot.tracked_week_secs);
    if let Some(block) = snapshot.block_started_at {
        let (today_start, week_start) = local_day_and_week_start(now, &Local);
        today += tracked_secs_since(block, now, today_start);
        week += tracked_secs_since(block, now, week_start);
    }
    (today, week)
}

/// Pausing applies while signed in and not already paused — including while
/// offline, so a flaky connection never stops someone from taking a break.
pub fn can_pause(status: TrackerStatus) -> bool {
    matches!(status, TrackerStatus::Tracking | TrackerStatus::Offline)
}

/// Colour + label for the status pill; a pause/resume in flight shows first.
fn status_visual(status: TrackerStatus, pending: Option<PendingAction>) -> (egui::Color32, &'static str) {
    match pending {
        Some(PendingAction::Pausing) => return (theme::WARNING, "Pausing…"),
        Some(PendingAction::Resuming) => return (theme::SUCCESS, "Resuming…"),
        None => {}
    }
    match status {
        TrackerStatus::Tracking => (theme::SUCCESS, status.label()),
        TrackerStatus::Paused => (theme::WARNING, status.label()),
        TrackerStatus::Offline => (theme::WARNING, status.label()),
        TrackerStatus::Connecting => (theme::PRIMARY, status.label()),
        TrackerStatus::SignedOut => (theme::MUTED_FG, status.label()),
        TrackerStatus::Stopped => (theme::DANGER, status.label()),
    }
}

impl eframe::App for TrackerApp {
    /// Clear the framebuffer to the app surface colour. Doing this here (rather
    /// than painting a rect) guarantees full coverage — any area egui does not
    /// draw would otherwise show through as black.
    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        let c = theme::BACKGROUND;
        [
            c.r() as f32 / 255.0,
            c.g() as f32 / 255.0,
            c.b() as f32 / 255.0,
            1.0,
        ]
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        if !self.themed {
            theme::apply(&ctx);
            self.themed = true;
        }

        let snapshot = self.state.snapshot();
        let quitting = self.quit_requested.load(std::sync::atomic::Ordering::Relaxed);

        if quitting {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        } else if ctx.input(|i| i.viewport().close_requested()) {
            // Closing the window hides to tray instead of exiting — this is a
            // background agent, so the X button must not stop tracking.
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
        }

        // eframe 0.35 hands us a Ui with no background, so paint the app surface
        // across the whole viewport, then lay content into a padded inner rect.
        // (Padding via a Frame margin would make children measure the *unpadded*
        // width and overflow the right edge.)
        let viewport = ui.max_rect();
        ui.painter()
            .rect_filled(viewport, egui::CornerRadius::ZERO, theme::BACKGROUND);

        let mut content = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(viewport.shrink(theme::PAGE_PAD))
                .layout(egui::Layout::top_down(egui::Align::Min)),
        );
        let ui = &mut content;

        self.header(ui, &snapshot);
        ui.add_space(14.0);

        if snapshot.status == TrackerStatus::SignedOut {
            self.login_view(ui, &snapshot);
        } else if self.view == View::Settings {
            self.settings_view(ui);
        } else {
            // Warning banners can push the content past the fixed window
            // height; scroll rather than clip the controls.
            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .show(ui, |ui| self.main_view(ui, &snapshot));
        }

        ctx.request_repaint_after(std::time::Duration::from_secs(1));
    }
}

impl TrackerApp {
    fn header(&self, ui: &mut egui::Ui, snapshot: &SharedState) {
        ui.horizontal(|ui| {
            theme::logo_mark(ui, 26.0);
            ui.add_space(2.0);
            ui.label(
                egui::RichText::new("Dosi Tracker")
                    .size(15.0)
                    .color(theme::INK)
                    .strong(),
            );

            if snapshot.status != TrackerStatus::SignedOut {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let (color, label) = status_visual(snapshot.status, snapshot.pending_action);
                    theme::status_pill(ui, color, label);
                });
            }
        });
    }

    fn login_view(&mut self, ui: &mut egui::Ui, snapshot: &SharedState) {
        ui.add_space(6.0);
        theme::card(18).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.vertical(|ui| {
                ui.label(
                    egui::RichText::new("Welcome back")
                        .size(19.0)
                        .color(theme::INK)
                        .strong(),
                );
                ui.add_space(2.0);
                ui.label(
                    egui::RichText::new("Sign in to start tracking your work.")
                        .size(12.5)
                        .color(theme::MUTED_FG),
                );
                ui.add_space(18.0);

                theme::field_label(ui, "Workspace");
                ui.add_sized(
                    [ui.available_width(), theme::CONTROL_H],
                    egui::TextEdit::singleline(&mut self.workspace)
                        .hint_text("acme")
                        .margin(egui::Margin::symmetric(10, 10)),
                );
                ui.add_space(12.0);

                theme::field_label(ui, "Email");
                ui.add_sized(
                    [ui.available_width(), theme::CONTROL_H],
                    egui::TextEdit::singleline(&mut self.email)
                        .hint_text("you@company.com")
                        .margin(egui::Margin::symmetric(10, 10)),
                );
                ui.add_space(12.0);

                theme::field_label(ui, "Password");
                ui.horizontal(|ui| {
                    let toggle_w = 38.0;
                    ui.add_sized(
                        [ui.available_width() - toggle_w - 8.0, theme::CONTROL_H],
                        egui::TextEdit::singleline(&mut self.password)
                            .password(!self.show_password)
                            .hint_text("••••••••")
                            .margin(egui::Margin::symmetric(10, 10)),
                    );
                    if ui
                        .add_sized(
                            [toggle_w, theme::CONTROL_H],
                            egui::Button::new(
                                egui::RichText::new(if self.show_password { "🙈" } else { "👁" })
                                    .size(13.0),
                            )
                            .corner_radius(egui::CornerRadius::same(theme::RADIUS)),
                        )
                        .on_hover_text(if self.show_password { "Hide password" } else { "Show password" })
                        .clicked()
                    {
                        self.show_password = !self.show_password;
                    }
                });

                if let Some(error) = &snapshot.login_error {
                    ui.add_space(12.0);
                    banner(ui, theme::DANGER, error);
                }

                ui.add_space(18.0);
                let can_submit = !snapshot.signing_in
                    && !self.email.trim().is_empty()
                    && !self.password.is_empty();
                let label = if snapshot.signing_in { "Signing in…" } else { "Sign in" };
                if theme::primary_button(ui, label, can_submit).clicked() {
                    self.send(Command::SignIn {
                        workspace: self.workspace.trim().to_string(),
                        username: self.email.trim().to_string(),
                        password: self.password.clone(),
                    });
                    self.password.clear();
                }
            });
        });

        ui.add_space(10.0);
        ui.vertical_centered(|ui| {
            ui.label(
                egui::RichText::new("Your workspace admin controls what is captured.")
                    .size(11.0)
                    .color(theme::MUTED_FG),
            );
        });
    }

    fn main_view(&mut self, ui: &mut egui::Ui, snapshot: &SharedState) {
        let now = Utc::now();
        self.session_card(ui, snapshot, now);
        ui.add_space(10.0);

        // Stat cards — live: the block in progress is added every second.
        let (today_secs, week_secs) = live_totals(snapshot, now);
        let gap = 8.0;
        let card_w = (ui.available_width() - gap * 2.0) / 3.0;
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = gap;
            stat_card(ui, card_w, "Today", &format_minutes((today_secs / 60.0) as u64), theme::INK);
            stat_card(ui, card_w, "This week", &format_minutes((week_secs / 60.0) as u64), theme::INK);
            stat_card(
                ui,
                card_w,
                "Activity",
                &format!("{}%", snapshot.last_productivity),
                theme::PRIMARY,
            );
        });

        ui.add_space(10.0);

        theme::card(12).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 4.0;
            ui.vertical(|ui| {
                theme::field_label(ui, "Project");

                let current_title = snapshot
                    .selected_project
                    .as_ref()
                    .and_then(|id| snapshot.projects.iter().find(|p| &p.id == id))
                    .map(|p| p.title.clone())
                    .unwrap_or_else(|| "No project selected".to_string());

                egui::ComboBox::from_id_salt("project-picker")
                    .width(ui.available_width())
                    .height(200.0)
                    .selected_text(egui::RichText::new(current_title).size(13.5))
                    .show_ui(ui, |ui| {
                        if snapshot.projects.is_empty() {
                            ui.label(
                                egui::RichText::new("No projects available")
                                    .color(theme::MUTED_FG),
                            );
                        }
                        for project in &snapshot.projects {
                            let selected = snapshot.selected_project.as_deref() == Some(&project.id);
                            if ui.selectable_label(selected, &project.title).clicked() && !selected {
                                self.send(Command::SelectProject(project.id.clone()));
                            }
                        }
                    });
            });
        });

        // A dead worker must never look like healthy tracking.
        if snapshot.status == TrackerStatus::Stopped {
            ui.add_space(10.0);
            banner(
                ui,
                theme::DANGER,
                snapshot
                    .last_error
                    .as_deref()
                    .unwrap_or("Tracking stopped unexpectedly. Please restart Dosi Tracker."),
            );
        } else if let Some(error) = &snapshot.last_error {
            ui.add_space(10.0);
            banner(ui, theme::WARNING, error);
        }

        // Dropped uploads are surfaced rather than silently discarded.
        if snapshot.rejected_uploads > 0 {
            ui.add_space(8.0);
            banner(
                ui,
                theme::WARNING,
                &format!(
                    "{} capture{} were rejected by the server and will not be uploaded.",
                    snapshot.rejected_uploads,
                    if snapshot.rejected_uploads == 1 { "" } else { "s" }
                ),
            );
        }

        ui.add_space(10.0);
        self.pause_controls(ui, snapshot);
        let paused = snapshot.status == TrackerStatus::Paused;

        ui.add_space(8.0);
        ui.horizontal(|ui| {
            let half = (ui.available_width() - 8.0) / 2.0;
            // While paused the button never captures — it only uploads what is
            // already queued — so it says so rather than promising a sync.
            let (sync_label, sync_hint) = if paused {
                (
                    "Upload queued",
                    "Paused: uploads activity already recorded. Nothing new is captured.",
                )
            } else {
                ("Sync now", "Record the current block now and upload it.")
            };
            if theme::ghost_button(ui, sync_label, half)
                .on_hover_text(sync_hint)
                .clicked()
            {
                self.send(Command::SyncNow);
            }
            if theme::ghost_button(ui, "Settings", half).clicked() {
                self.autostart_enabled = autostart::is_enabled();
                self.view = View::Settings;
            }
        });

        if !snapshot.display_name.is_empty() {
            ui.add_space(10.0);
            // Upload state rides along with the account line: it matters at a
            // glance, but not enough for a row of its own.
            let upload_state = if snapshot.pending_uploads > 0 {
                format!(
                    " · {} pending upload{}",
                    snapshot.pending_uploads,
                    if snapshot.pending_uploads == 1 { "" } else { "s" }
                )
            } else if let Some(synced) = snapshot.last_sync {
                format!(" · synced {}", local_hm(synced))
            } else {
                String::new()
            };
            ui.vertical_centered(|ui| {
                ui.label(
                    egui::RichText::new(format!("Signed in as {}{upload_state}", snapshot.display_name))
                        .size(11.0)
                        .color(theme::MUTED_FG),
                );
            });
        }
    }

    /// The live card at the top: a stopwatch for the current tracking stretch
    /// and what the block in progress holds so far, or — while paused — how
    /// long the pause has lasted and when it ends.
    fn session_card(&self, ui: &mut egui::Ui, snapshot: &SharedState, now: DateTime<Utc>) {
        theme::card(12).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 3.0;
            ui.vertical(|ui| match snapshot.status {
                TrackerStatus::Paused => {
                    let since = snapshot.paused_since.unwrap_or(now);
                    card_heading(ui, "Paused", theme::WARNING, &format!("since {}", local_hm(since)));
                    ui.add_space(2.0);
                    timer_text(ui, &format_clock(now - since), theme::WARNING);
                    ui.add_space(4.0);
                    meta(
                        ui,
                        &match snapshot.resume_at {
                            Some(at) => format!(
                                "Resumes automatically at {} · in {}",
                                local_hm(at),
                                format_clock(at - now)
                            ),
                            None => "Nothing is recorded until you resume.".to_string(),
                        },
                    );
                }
                TrackerStatus::Tracking => {
                    let started = snapshot.tracking_since.unwrap_or(now);
                    let next = match snapshot.next_capture_at {
                        Some(at) => format!(
                            "every {} min · next in {}",
                            snapshot.interval_minutes,
                            format_clock(at - now)
                        ),
                        None => format!("snapshot every {} min", snapshot.interval_minutes),
                    };
                    card_heading(ui, "Current session", theme::MUTED_FG, &next);
                    ui.add_space(2.0);
                    timer_text(ui, &format_clock(now - started), theme::INK);
                    ui.add_space(4.0);
                    let block = snapshot.block_started_at.unwrap_or(now);
                    meta(ui, &self.block_summary(snapshot, block, now));
                }
                other => {
                    card_heading(ui, "Current session", theme::MUTED_FG, "");
                    ui.add_space(2.0);
                    timer_text(ui, "--:--", theme::MUTED_FG);
                    ui.add_space(4.0);
                    meta(
                        ui,
                        match other {
                            TrackerStatus::Connecting => "Connecting to your workspace…",
                            TrackerStatus::Stopped => "Tracking has stopped.",
                            _ => "Not tracking — waiting for the server or a project.",
                        },
                    );
                }
            });
        });
    }

    /// "72% active · 120 keys · 45 clicks since 16:40" for the block in
    /// progress, counting only the input kinds the project records.
    fn block_summary(&self, snapshot: &SharedState, block: DateTime<Utc>, now: DateTime<Utc>) -> String {
        let (keys, clicks) = self.input.counts_between(block, now);
        let mut parts = Vec::new();
        if snapshot.counts_keyboard || snapshot.counts_mouse {
            let percent = self
                .input
                .activity_percent(block, now, snapshot.counts_keyboard, snapshot.counts_mouse);
            parts.push(format!("{percent}% active"));
        }
        if snapshot.counts_keyboard {
            parts.push(format!("{keys} keys"));
        }
        if snapshot.counts_mouse {
            parts.push(format!("{clicks} clicks"));
        }
        parts.push(format!("since {}", local_hm(block)));
        parts.join(" · ")
    }

    /// Pause / resume, plus timed pauses. A request in flight disables the
    /// buttons and is shown at once, so a click never looks ignored.
    fn pause_controls(&mut self, ui: &mut egui::Ui, snapshot: &SharedState) {
        let pending = snapshot.pending_action;
        let paused = snapshot.status == TrackerStatus::Paused;
        let idle = pending.is_none();

        // Primary action. Plain text only — egui's bundled font has no
        // play/pause glyphs and would render tofu boxes.
        if paused {
            let label = if pending == Some(PendingAction::Resuming) { "Resuming…" } else { "Resume tracking" };
            if theme::filled_button(ui, label, theme::SUCCESS, idle).clicked() {
                self.request(PendingAction::Resuming, Command::Resume);
            }
        } else {
            let can_pause = idle && can_pause(snapshot.status);
            let label = if pending == Some(PendingAction::Pausing) { "Pausing…" } else { "Pause tracking" };
            if theme::filled_button(ui, label, theme::WARNING, can_pause).clicked() {
                self.request(PendingAction::Pausing, Command::Pause { resume_after: None });
            }
        }

        if !paused && !can_pause(snapshot.status) {
            return;
        }

        // Timed pauses. While paused the same choices re-arm the auto-resume
        // from now, and "Never" turns it off. The label sits inline when three
        // buttons leave room for it, and above the row when "Never" makes four.
        ui.add_space(8.0);
        let show_never = paused && snapshot.resume_at.is_some();
        let count = PAUSE_CHOICES.len() + usize::from(show_never);
        let caption = if paused { "Auto-resume" } else { "Pause for" };
        let inline_label = count <= PAUSE_CHOICES.len();
        if !inline_label {
            meta(ui, caption);
            ui.add_space(2.0);
        }
        let gap = 6.0;
        let label_w = if inline_label { 74.0 } else { 0.0 };
        let gaps = if inline_label { count } else { count - 1 };
        let width = (ui.available_width() - label_w - gap * gaps as f32) / count as f32;
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = gap;
            if inline_label {
                ui.add_sized(
                    [label_w, 36.0],
                    egui::Label::new(egui::RichText::new(caption).size(11.0).color(theme::MUTED_FG)),
                );
            }
            for (label, duration) in PAUSE_CHOICES {
                let hint = if paused {
                    format!("Stay paused, then resume automatically in {label}")
                } else {
                    format!("Pause now and resume automatically in {label}")
                };
                if ui
                    .add_enabled_ui(idle, |ui| theme::ghost_button(ui, label, width))
                    .inner
                    .on_hover_text(hint)
                    .clicked()
                {
                    let command = Command::Pause { resume_after: Some(duration) };
                    if paused {
                        // Already paused: only the deadline changes.
                        self.send(command);
                    } else {
                        self.request(PendingAction::Pausing, command);
                    }
                }
            }
            if show_never
                && ui
                    .add_enabled_ui(idle, |ui| theme::ghost_button(ui, "Never", width))
                    .inner
                    .on_hover_text("Stay paused until you resume")
                    .clicked()
            {
                self.send(Command::Pause { resume_after: None });
            }
        });
    }

    fn settings_view(&mut self, ui: &mut egui::Ui) {
        theme::card(16).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.vertical(|ui| {
                ui.label(
                    egui::RichText::new("Settings")
                        .size(16.0)
                        .color(theme::INK)
                        .strong(),
                );
                ui.add_space(14.0);

                if ui
                    .checkbox(
                        &mut self.autostart_enabled,
                        egui::RichText::new("  Start when I sign in to Windows").size(13.0),
                    )
                    .changed()
                {
                    match autostart::set_enabled(self.autostart_enabled) {
                        Ok(()) => self.autostart_error = None,
                        Err(e) => {
                            // Reflect the real registry state if the write failed.
                            self.autostart_enabled = autostart::is_enabled();
                            self.autostart_error = Some(e.to_string());
                        }
                    }
                }
                if let Some(error) = &self.autostart_error {
                    ui.add_space(8.0);
                    banner(ui, theme::DANGER, error);
                }

                ui.add_space(14.0);
                egui::Frame::new()
                    .fill(theme::MUTED)
                    .corner_radius(egui::CornerRadius::same(theme::RADIUS))
                    .inner_margin(egui::Margin::same(11))
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.label(
                            egui::RichText::new(
                                "Capture settings — screenshots, webcam, and keyboard/mouse \
                                 counts — are configured per project by your workspace admin.",
                            )
                            .size(11.5)
                            .color(theme::MUTED_FG),
                        );
                    });
            });
        });

        ui.add_space(12.0);
        ui.horizontal(|ui| {
            let half = (ui.available_width() - 8.0) / 2.0;
            if theme::ghost_button(ui, "Back", half).clicked() {
                self.view = View::Main;
            }
            if ui
                .add(
                    egui::Button::new(
                        egui::RichText::new("Sign out").color(theme::DANGER).strong(),
                    )
                    .fill(theme::tint(theme::DANGER))
                    .corner_radius(egui::CornerRadius::same(theme::RADIUS))
                    .stroke(egui::Stroke::new(1.0, theme::DANGER))
                    .min_size(egui::Vec2::new(half, 36.0)),
                )
                .clicked()
            {
                self.send(Command::SignOut);
                self.view = View::Main;
            }
        });
    }
}

/// One headline metric in a bordered surface card.
/// `width` is the card's OUTER width; the frame's padding is subtracted so a row
/// of cards adds up to exactly the space allocated for it.
fn stat_card(ui: &mut egui::Ui, width: f32, label: &str, value: &str, value_color: egui::Color32) {
    const PAD: f32 = 10.0;
    theme::card(PAD as i8).show(ui, |ui| {
        ui.set_width((width - PAD * 2.0).max(0.0));
        ui.vertical(|ui| {
            ui.label(
                egui::RichText::new(label.to_uppercase())
                    .size(9.5)
                    .color(theme::MUTED_FG)
                    .strong(),
            );
            ui.add_space(3.0);
            ui.label(
                egui::RichText::new(value)
                    .size(17.0)
                    .color(value_color)
                    .strong(),
            );
        });
    });
}

/// Inline notice with a tinted background (errors, warnings).
fn banner(ui: &mut egui::Ui, color: egui::Color32, message: &str) {
    egui::Frame::new()
        .fill(theme::tint(color))
        .corner_radius(egui::CornerRadius::same(8))
        .inner_margin(egui::Margin::same(10))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(egui::RichText::new(message).size(11.5).color(color));
        });
}

/// Small muted metadata text.
fn meta(ui: &mut egui::Ui, text: &str) {
    ui.label(egui::RichText::new(text).size(11.0).color(theme::MUTED_FG));
}

/// A card's small-caps heading, with an optional muted note on the right.
fn card_heading(ui: &mut egui::Ui, title: &str, color: egui::Color32, note: &str) {
    ui.horizontal(|ui| {
        ui.label(
            egui::RichText::new(title.to_uppercase())
                .size(9.5)
                .color(color)
                .strong(),
        );
        if !note.is_empty() {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| meta(ui, note));
        }
    });
}

/// The session card's big stopwatch digits (monospace, so they do not jitter
/// as they tick).
fn timer_text(ui: &mut egui::Ui, text: &str, color: egui::Color32) {
    ui.label(egui::RichText::new(text).size(28.0).color(color).strong().monospace());
}

/// Wall-clock `HH:MM` in the user's time zone.
fn local_hm(at: DateTime<Utc>) -> String {
    at.with_timezone(&Local).format("%H:%M").to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clock_reads_like_a_stopwatch() {
        assert_eq!(format_clock(chrono::Duration::seconds(0)), "00:00");
        assert_eq!(format_clock(chrono::Duration::seconds(309)), "05:09");
        assert_eq!(format_clock(chrono::Duration::seconds(3_909)), "1:05:09");
        // A deadline already passed never shows a negative time.
        assert_eq!(format_clock(chrono::Duration::seconds(-4)), "00:00");
    }

    #[test]
    fn live_totals_add_the_running_block() {
        let now = Utc::now();
        let mut snapshot = SharedState {
            tracked_today_secs: 600.0,
            tracked_week_secs: 3_600.0,
            ..SharedState::default()
        };
        assert_eq!(live_totals(&snapshot, now), (600.0, 3_600.0));

        // A block that began 90 s ago (and today, locally) is added to both.
        snapshot.block_started_at = Some(now - chrono::Duration::seconds(90));
        let (today_start, _) = local_day_and_week_start(now, &Local);
        let expected_today = 600.0 + tracked_secs_since(now - chrono::Duration::seconds(90), now, today_start);
        let (today, week) = live_totals(&snapshot, now);
        assert_eq!(today, expected_today);
        assert!(week > 3_600.0 && week <= 3_690.0);
    }
}
