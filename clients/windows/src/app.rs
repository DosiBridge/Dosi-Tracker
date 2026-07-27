//! The desktop window: sign-in, live tracking status, and settings.
//!
//! Rendering is immediate-mode (egui) and reads only a cheap state snapshot, so
//! the UI stays responsive while the worker does network/capture work.
//! Visual language mirrors the web dashboard — see `theme.rs`.

use eframe::egui;
use tokio::sync::mpsc::UnboundedSender;

use crate::autostart;
use crate::state::{Command, SharedState, StateHandle, TrackerStatus};
use crate::theme;

#[derive(PartialEq, Eq)]
enum View {
    Main,
    Settings,
}

pub struct TrackerApp {
    state: StateHandle,
    commands: UnboundedSender<Command>,
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
        quit_requested: std::sync::Arc<std::sync::atomic::AtomicBool>,
    ) -> Self {
        Self {
            state,
            commands,
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

/// Colour + label for the current tracker status.
fn status_visual(status: TrackerStatus) -> (egui::Color32, &'static str) {
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
            self.main_view(ui, &snapshot);
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
                    let (color, label) = status_visual(snapshot.status);
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
        // Stat cards
        let gap = 8.0;
        let card_w = (ui.available_width() - gap * 2.0) / 3.0;
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = gap;
            stat_card(ui, card_w, "Today", &format_minutes(snapshot.tracked_today_minutes), theme::INK);
            stat_card(ui, card_w, "This week", &format_minutes(snapshot.tracked_week_minutes), theme::INK);
            stat_card(
                ui,
                card_w,
                "Activity",
                &format!("{}%", snapshot.last_productivity),
                theme::PRIMARY,
            );
        });

        ui.add_space(14.0);

        theme::card(14).show(ui, |ui| {
            ui.set_width(ui.available_width());
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

                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    meta(ui, &format!("Snapshot every {} min", snapshot.interval_minutes));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if snapshot.pending_uploads > 0 {
                            meta(
                                ui,
                                &format!(
                                    "{} pending upload{}",
                                    snapshot.pending_uploads,
                                    if snapshot.pending_uploads == 1 { "" } else { "s" }
                                ),
                            );
                        } else if let Some(synced) = snapshot.last_sync {
                            meta(ui, &format!("Synced {}", synced.format("%H:%M UTC")));
                        }
                    });
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

        ui.add_space(14.0);

        // Primary action: pause / resume. Plain text only — egui's bundled font
        // has no play/pause glyphs and would render tofu boxes.
        let paused = snapshot.status == TrackerStatus::Paused;
        let (label, fill) = if paused {
            ("Resume tracking", theme::SUCCESS)
        } else {
            ("Pause tracking", theme::WARNING)
        };
        if theme::filled_button(ui, label, fill, true).clicked() {
            self.send(if paused { Command::Resume } else { Command::Pause });
        }

        ui.add_space(8.0);
        ui.horizontal(|ui| {
            let half = (ui.available_width() - 8.0) / 2.0;
            if theme::ghost_button(ui, "Sync now", half).clicked() {
                self.send(Command::SyncNow);
            }
            if theme::ghost_button(ui, "Settings", half).clicked() {
                self.autostart_enabled = autostart::is_enabled();
                self.view = View::Settings;
            }
        });

        if !snapshot.display_name.is_empty() {
            ui.add_space(10.0);
            ui.vertical_centered(|ui| {
                ui.label(
                    egui::RichText::new(format!("Signed in as {}", snapshot.display_name))
                        .size(11.0)
                        .color(theme::MUTED_FG),
                );
            });
        }
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
