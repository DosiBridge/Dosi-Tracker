//! The Settings view: how the app behaves on this computer, the user's goals,
//! the capture policy their admin set (read-only), this device, the account,
//! and support.

use eframe::egui;
use egui::{Align, Layout};

use super::widgets::{self, ButtonKind, Icon, Tone};
use super::{format, section, TrackerApp};
use crate::prefs::{GOAL_MAX_HOURS, GOAL_MIN_HOURS};
use crate::shell;
use crate::state::{Command, SharedState, TrackerStatus};
use crate::theme;

impl TrackerApp {
    pub(super) fn settings_view(&mut self, ui: &mut egui::Ui, snapshot: &SharedState) {
        if let Some(error) = self.settings_error.clone() {
            widgets::banner(ui, Tone::Danger, &error);
        }
        self.general_settings(ui);
        self.goal_settings(ui);
        self.consent_and_policy_card(ui, snapshot);
        self.device_card(ui);
        self.account_card(ui, snapshot);
        self.support_card(ui);
    }

    fn general_settings(&mut self, ui: &mut egui::Ui) {
        let has_tray = self.tray.is_some();
        section(ui, "General", None, |ui| {
            let mut autostart = self.autostart_enabled;
            if setting_row(ui, "Start with Windows", "Open Dosi Tracker when you sign in to Windows.", &mut autostart) {
                self.set_autostart(autostart);
            }
            divider(ui);
            let mut changed = false;
            ui.add_enabled_ui(has_tray, |ui| {
                changed |= setting_row(
                    ui,
                    "Start in the tray",
                    "When Windows starts the app, keep the window closed. Tracking resumes in the background.",
                    &mut self.prefs.start_minimized,
                );
                divider(ui);
                changed |= setting_row(
                    ui,
                    "Minimize to the tray",
                    "Minimizing hides the window to the notification area instead of the taskbar.",
                    &mut self.prefs.minimize_to_tray,
                );
                divider(ui);
                changed |= setting_row(
                    ui,
                    "Show notifications",
                    "While the window is closed: when you come back from time away, when tracking starts by itself, \
                     and when your sign-in expires.",
                    &mut self.prefs.show_notifications,
                );
            });
            if changed {
                self.save_prefs();
            }
            ui.add_space(2.0);
            theme::inset(10).show(ui, |ui| {
                ui.set_width(ui.available_width());
                widgets::muted(
                    ui,
                    "Closing the window keeps Dosi Tracker running in the tray. To stop it completely, \
                     right-click the tray icon and choose Quit.",
                );
            });
        });
    }

    fn goal_settings(&mut self, ui: &mut egui::Ui) {
        section(ui, "Goals", None, |ui| {
            let mut changed = false;
            ui.horizontal(|ui| {
                label_block(ui, "Daily goal", "Hours you aim to track each working day.");
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.spacing_mut().item_spacing.x = 6.0;
                    let goal = self.prefs.daily_goal_hours;
                    if widgets::chip(ui, "+", false, goal < GOAL_MAX_HOURS).on_hover_text("Half an hour more").clicked() {
                        self.prefs.daily_goal_hours = (goal + 0.5).min(GOAL_MAX_HOURS);
                        changed = true;
                    }
                    ui.label(
                        egui::RichText::new(widgets::hours_label(f64::from(goal)))
                            .font(theme::semibold(14.0))
                            .color(theme::INK),
                    );
                    if widgets::chip(ui, "−", false, goal > GOAL_MIN_HOURS).on_hover_text("Half an hour less").clicked() {
                        self.prefs.daily_goal_hours = (goal - 0.5).max(GOAL_MIN_HOURS);
                        changed = true;
                    }
                });
            });
            divider(ui);
            ui.horizontal(|ui| {
                label_block(ui, "Working days", "Days a week; sets the weekly goal.");
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.spacing_mut().item_spacing.x = 4.0;
                    for days in (5..=7).rev() {
                        let selected = self.prefs.work_days_per_week == days;
                        if widgets::chip(ui, &format!("{days} days"), selected, true).clicked() && !selected {
                            self.prefs.work_days_per_week = days;
                            changed = true;
                        }
                    }
                });
            });
            ui.add_space(2.0);
            widgets::muted(
                ui,
                &format!(
                    "Weekly goal: {}. Goals are personal and only shown on this computer.",
                    widgets::hours_label(self.prefs.weekly_goal_secs() / 3600.0)
                ),
            );
            if changed {
                self.save_prefs();
            }
        });
    }

    /// This computer as the workspace sees it: every block carries these.
    fn device_card(&mut self, ui: &mut egui::Ui) {
        let zone = self.zone_label();
        let device = &self.device;
        let name = device.device_name.clone().unwrap_or_else(|| "Unknown".to_owned());
        let os = device.os_version.clone().unwrap_or_else(|| "Windows".to_owned());
        let id = device.device_id.clone();
        let mut copy = None;
        section(ui, "This device", None, |ui| {
            info_row(ui, Icon::Monitor, "Device name", &name);
            divider(ui);
            ui.horizontal(|ui| {
                label_cell(ui, Icon::Info, "Device ID");
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.spacing_mut().item_spacing.x = 6.0;
                    match &id {
                        Some(id) => {
                            if widgets::icon_button(ui, Icon::Copy, 16.0, theme::INK_SOFT, "Copy the device ID").clicked() {
                                copy = Some(id.clone());
                            }
                            value_label(ui, id);
                        }
                        None => value_label(ui, "Not available"),
                    }
                });
            });
            divider(ui);
            info_row(ui, Icon::Briefcase, "Agent version", device.agent_version);
            divider(ui);
            info_row(ui, Icon::Window, "Windows", &os);
            divider(ui);
            info_row(ui, Icon::Globe, "Time zone", &zone);
            ui.add_space(2.0);
            widgets::muted(
                ui,
                "Sent with your tracked time and while you're online, so your workspace can tell your computers apart \
                 and show your days in your own time zone.",
            );
        });
        if let Some(id) = copy {
            ui.ctx().copy_text(id);
            self.show_toast(Tone::Success, "Device ID copied.");
        }
    }

    fn account_card(&mut self, ui: &mut egui::Ui, snapshot: &SharedState) {
        let server = if snapshot.server_url.trim().is_empty() { self.links.api_base_url.clone() } else { snapshot.server_url.clone() };
        let dashboard_ready = self.dashboard_available(snapshot);
        let dashboard = if dashboard_ready { format::server_label(&self.links.dashboard_url) } else { "Not set on this computer".to_owned() };
        section(ui, "Account", None, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 12.0;
                widgets::avatar(ui, &snapshot.display_name, 42.0);
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 1.0;
                    ui.add_space(3.0);
                    ui.add(egui::Label::new(egui::RichText::new(&snapshot.display_name).font(theme::semibold(14.0)).color(theme::INK)).truncate());
                    let workspace = if snapshot.workspace.trim().is_empty() {
                        "No workspace chosen".to_owned()
                    } else {
                        format!("Workspace: {}", snapshot.workspace.trim())
                    };
                    ui.label(egui::RichText::new(workspace).font(theme::regular(12.0)).color(theme::MUTED_FG));
                });
            });
            ui.add_space(2.0);
            info_row(ui, Icon::Globe, "Server", &format::server_label(&server));
            divider(ui);
            info_row(ui, Icon::External, "Web dashboard", &dashboard);
            ui.add_space(4.0);
            self.switch_workspace(ui, snapshot);

            if self.confirm_sign_out {
                theme::inset(12).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.label(egui::RichText::new("Sign out of Dosi Tracker?").font(theme::semibold(13.0)).color(theme::INK));
                    widgets::muted(
                        ui,
                        "The block in progress is saved and uploaded first. Tracking stops on this computer until \
                         someone signs in again.",
                    );
                    ui.add_space(4.0);
                    let mut cancel = false;
                    let mut confirm = false;
                    super::equal_row(ui, 2, 8.0, |i, ui| {
                        let width = Some(ui.available_width());
                        if i == 0 {
                            cancel = widgets::button(ui, ButtonKind::Secondary, None, "Cancel", width, true).clicked();
                        } else {
                            confirm = widgets::button(ui, ButtonKind::Danger, Some(Icon::SignOut), "Sign out", width, true).clicked();
                        }
                    });
                    if cancel {
                        self.confirm_sign_out = false;
                    }
                    if confirm {
                        self.confirm_sign_out = false;
                        self.send(Command::SignOut { at: chrono::Utc::now() });
                    }
                });
            } else {
                let mut open_dashboard = false;
                let mut sign_out = false;
                super::equal_row(ui, 2, 8.0, |i, ui| {
                    let width = Some(ui.available_width());
                    if i == 0 {
                        let response = widgets::button(ui, ButtonKind::Secondary, Some(Icon::External), "Web dashboard", width, dashboard_ready);
                        open_dashboard = if dashboard_ready { response.clicked() } else { response.on_hover_text(super::DASHBOARD_UNSET).clicked() };
                    } else {
                        sign_out = widgets::button(ui, ButtonKind::Danger, Some(Icon::SignOut), "Sign out", width, true).clicked();
                    }
                });
                if open_dashboard {
                    self.open_dashboard(snapshot);
                }
                if sign_out {
                    self.confirm_sign_out = true;
                }
            }
        });
    }

    /// "Switch workspace": the user's workspaces; one click moves tracking there.
    fn switch_workspace(&mut self, ui: &mut egui::Ui, snapshot: &SharedState) {
        if snapshot.display_name.trim().is_empty() || snapshot.status == TrackerStatus::SignInExpired {
            return;
        }
        if let Some(note) = &snapshot.workspace_note {
            widgets::muted(ui, note);
        }
        if snapshot.workspace_choices.is_empty() {
            let finding = snapshot.finding_workspaces;
            let label = if finding { "Looking up workspaces…" } else { "Switch workspace" };
            if widgets::button(ui, ButtonKind::Secondary, None, label, Some(ui.available_width()), !finding).clicked() {
                self.send(Command::SwitchWorkspace);
            }
            return;
        }
        let mut action = None;
        theme::inset(12).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 6.0;
            ui.label(egui::RichText::new("Switch workspace").font(theme::semibold(15.0)).color(theme::INK));
            for (index, choice) in snapshot.workspace_choices.iter().enumerate() {
                let label = super::login::workspace_label(choice);
                if widgets::button(ui, ButtonKind::Secondary, None, &label, Some(ui.available_width()), !choice.is_current).clicked() {
                    action = Some(Some(index));
                }
            }
            if snapshot.workspace_lost.is_none() && ui.vertical_centered(|ui| widgets::link(ui, "Back", theme::PRIMARY_STRONG).clicked()).inner {
                action = Some(None);
            }
        });
        match action {
            None => {}
            Some(None) => self.send(Command::CancelWorkspaceChoice),
            Some(Some(index)) => self.send(Command::SwitchToWorkspace { index, at: chrono::Utc::now() }),
        }
    }

    fn support_card(&mut self, ui: &mut egui::Ui) {
        section(ui, "Help & support", Some(concat!("Version ", env!("CARGO_PKG_VERSION"))), |ui| {
            let mut open_logs = false;
            let mut open_data = false;
            super::equal_row(ui, 2, 8.0, |i, ui| {
                let width = Some(ui.available_width());
                if i == 0 {
                    open_logs = widgets::button(ui, ButtonKind::Secondary, Some(Icon::Folder), "Open logs", width, true)
                        .on_hover_text("Daily log files — attach them when reporting a problem.")
                        .clicked();
                } else {
                    open_data = widgets::button(ui, ButtonKind::Secondary, Some(Icon::Folder), "Open data folder", width, true)
                        .on_hover_text("Where Dosi Tracker keeps its offline queue (with screenshots waiting to upload).")
                        .clicked();
                }
            });
            if open_logs {
                self.open_folder(shell::logs_dir());
            }
            if open_data {
                self.open_folder(shell::local_data_dir());
            }
        });
    }

    /// Privacy and local consent settings: explicit granular consent choices
    /// and effective capability status matrix (workspace policy ∩ local consent).
    fn consent_and_policy_card(&mut self, ui: &mut egui::Ui, snapshot: &SharedState) {
        let project = snapshot
            .current_project()
            .map(|p| p.title.clone())
            .unwrap_or_else(|| "your project".to_owned());

        section(ui, "Privacy & Permissions", Some(&project), |ui| {
            // Keep VM synchronized if snapshot changed externally
            if self.consent_vm.current != snapshot.consent {
                self.consent_vm = crate::ui::consent::ConsentViewModel::new(snapshot.consent.clone());
            }

            // Consent status pill
            let (status_tone, status_text) = match &snapshot.consent {
                crate::config::LocalConsent::Granted(_) => (Tone::Success, "Consented on this computer"),
                crate::config::LocalConsent::Revoked => (Tone::Warning, "Declined / Revoked"),
                crate::config::LocalConsent::Absent => (Tone::Warning, "Consent required before tracking"),
            };

            ui.horizontal(|ui| {
                ui.label(egui::RichText::new("Local consent:").font(theme::semibold(13.0)).color(theme::INK));
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    widgets::status_pill(ui, status_tone, status_text);
                });
            });

            ui.add_space(2.0);
            widgets::muted(
                ui,
                "Explicit local consent is required. Tracking only captures capabilities granted below AND permitted by your workspace policy.",
            );
            divider(ui);

            // Granular controls
            let mut input_counts = self.consent_vm.draft.keyboard || self.consent_vm.draft.mouse;
            if setting_row(
                ui,
                "Input activity (counts only)",
                "Count keyboard key presses and mouse clicks/moves — never raw keystrokes or text typed",
                &mut input_counts,
            ) {
                self.consent_vm.toggle_input_counts(input_counts);
            }
            divider(ui);

            let mut active_win = self.consent_vm.draft.active_window;
            if setting_row(
                ui,
                "Active window & application",
                "Record the foreground window title and active application name during tracked time",
                &mut active_win,
            ) {
                self.consent_vm.toggle_active_window(active_win);
            }
            divider(ui);

            let mut screenshot = self.consent_vm.draft.screenshot;
            if setting_row(
                ui,
                "Screenshots",
                "Capture periodic screenshots of the active display during tracked blocks",
                &mut screenshot,
            ) {
                self.consent_vm.toggle_screenshot(screenshot);
            }
            divider(ui);

            let mut blur = self.consent_vm.draft.blur_screenshots;
            if setting_row(
                ui,
                "Blur screenshots locally",
                "Blur captured screenshots on this computer before uploading to protect sensitive text",
                &mut blur,
            ) {
                self.consent_vm.toggle_blur_screenshots(blur);
            }
            divider(ui);

            let mut webcam = self.consent_vm.draft.webcam;
            if setting_row(
                ui,
                "Webcam photos",
                "Capture periodic webcam photos during tracked blocks (if enabled by project)",
                &mut webcam,
            ) {
                self.consent_vm.toggle_webcam(webcam);
            }
            divider(ui);

            // Buttons: Save & Revoke
            ui.horizontal(|ui| {
                if widgets::button(ui, ButtonKind::Primary, Some(Icon::Check), "Save permissions", None, true).clicked() {
                    let consent = self.consent_vm.grant();
                    self.send(Command::UpdateConsent { consent });
                }

                if widgets::button(ui, ButtonKind::Secondary, None, "Revoke all consent", None, true).clicked() {
                    let consent = self.consent_vm.revoke();
                    self.send(Command::UpdateConsent { consent });
                }
            });

            ui.add_space(6.0);
            divider(ui);

            // Effective capability status table
            ui.label(
                egui::RichText::new("Effective Recording Matrix")
                    .font(theme::semibold(13.0))
                    .color(theme::INK_SOFT),
            );
            ui.add_space(2.0);

            let c = snapshot.capture;
            let user_granted = match &snapshot.consent {
                crate::config::LocalConsent::Granted(p) => p.clone(),
                crate::config::LocalConsent::Absent | crate::config::LocalConsent::Revoked => {
                    crate::config::CapturePermissions::all_disabled()
                }
            };

            let capabilities = [
                (
                    Icon::Keyboard,
                    "Keyboard counts",
                    "Keys pressed as counts — never typed text",
                    user_granted.keyboard,
                    c.keyboard,
                ),
                (
                    Icon::Mouse,
                    "Mouse activity",
                    "Clicks and movement counts",
                    user_granted.mouse,
                    c.mouse,
                ),
                (
                    Icon::Window,
                    "Active window",
                    "Foreground app and window title",
                    user_granted.active_window,
                    c.active_window,
                ),
                (
                    Icon::Camera,
                    "Screenshots",
                    "Occasional screen capture",
                    user_granted.screenshot,
                    c.screenshot,
                ),
                (
                    Icon::Webcam,
                    "Webcam",
                    "Occasional webcam capture",
                    user_granted.webcam,
                    c.webcam,
                ),
            ];

            for (i, (icon, title, detail, user_ok, project_ok)) in capabilities.into_iter().enumerate() {
                if i > 0 {
                    divider(ui);
                }
                let status = crate::ui::consent::ConsentViewModel::capability_status(user_ok, project_ok);
                ui.horizontal_top(|ui| {
                    ui.spacing_mut().item_spacing.x = 10.0;
                    widgets::icon(
                        ui,
                        icon,
                        18.0,
                        if status == crate::ui::consent::CapabilityStatus::Active { theme::INK_SOFT } else { theme::FAINT },
                    );
                    let pill_w = 120.0;
                    ui.allocate_ui_with_layout(
                        egui::Vec2::new((ui.available_width() - pill_w).max(0.0), 0.0),
                        Layout::top_down(Align::Min),
                        |ui| {
                            ui.spacing_mut().item_spacing.y = 0.0;
                            ui.label(
                                egui::RichText::new(title)
                                    .font(theme::semibold(13.0))
                                    .color(if status == crate::ui::consent::CapabilityStatus::Active { theme::INK } else { theme::MUTED_FG }),
                            );
                            ui.add(egui::Label::new(egui::RichText::new(detail).font(theme::regular(11.5)).color(theme::MUTED_FG)).wrap());
                        },
                    );
                    ui.with_layout(Layout::right_to_left(Align::Min), |ui| {
                        widgets::status_pill(ui, status.tone(), status.label());
                    });
                });
            }

            ui.add_space(4.0);
            theme::inset(10).show(ui, |ui| {
                ui.set_width(ui.available_width());
                widgets::muted(
                    ui,
                    &format!(
                        "A block is saved every {} minutes; time with the screen locked or computer asleep is never recorded. \
                         Never collected: the keys you press or what you type, the clipboard, your microphone, or your personal files.",
                        snapshot.interval_minutes
                    ),
                );
            });
        });
    }
}

/// One line of what a project records: what it is, and whether it is on.
#[cfg(test)]
struct PolicyRow {
    #[allow(dead_code)]
    icon: Icon,
    title: &'static str,
    on: bool,
    detail: String,
}

/// What the current project records, in the words of what is collected —
/// the admin's policy, read-only here.
#[cfg(test)]
fn policy_rows(snapshot: &SharedState) -> Vec<PolicyRow> {
    let c = snapshot.capture;
    let screenshot = if c.blur_screenshots {
        "One per block, at a random moment, of the screen you're working on — blurred on this computer so no text can be read"
    } else {
        "One per block, at a random moment, of the screen you're working on"
    };
    let idle = match snapshot.idle_threshold_minutes {
        Some(minutes) => PolicyRow {
            icon: Icon::Timer,
            title: "Idle detection",
            on: true,
            detail: format!(
                "After {} without keyboard or mouse input the block ends; when you're back you choose whether to keep \
                 the time away",
                format::minutes_words(i64::from(minutes))
            ),
        },
        None => PolicyRow {
            icon: Icon::Timer,
            title: "Idle detection",
            on: false,
            detail: "Off on this project: time away from the keyboard is recorded until the screen locks".to_owned(),
        },
    };
    vec![
        PolicyRow { icon: Icon::Camera, title: "Screenshots", on: c.screenshot, detail: screenshot.to_owned() },
        PolicyRow { icon: Icon::Webcam, title: "Webcam photo", on: c.webcam, detail: "One per block".to_owned() },
        PolicyRow {
            icon: Icon::Keyboard,
            title: "Keyboard activity",
            on: c.keyboard,
            detail: "When keys are pressed — never which keys, or what you type".to_owned(),
        },
        PolicyRow { icon: Icon::Mouse, title: "Mouse activity", on: c.mouse, detail: "Clicks, movement and scrolling, as counts".to_owned() },
        PolicyRow {
            icon: Icon::Window,
            title: "Apps & window titles",
            on: c.active_window,
            detail: "The app in front, its window title, and for how long".to_owned(),
        },
        PolicyRow {
            icon: Icon::Briefcase,
            title: "Running programs",
            on: c.running_programs,
            detail: "The names of open programs — never their window titles".to_owned(),
        },
        idle,
    ]
}

/// A label with its icon on the left and a value on the right.
fn info_row(ui: &mut egui::Ui, icon: Icon, label: &str, value: &str) {
    ui.horizontal(|ui| {
        label_cell(ui, icon, label);
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| value_label(ui, value));
    });
}

fn label_cell(ui: &mut egui::Ui, icon: Icon, label: &str) {
    ui.spacing_mut().item_spacing.x = 10.0;
    widgets::icon(ui, icon, 16.0, theme::MUTED_FG);
    ui.label(egui::RichText::new(label).font(theme::semibold(12.5)).color(theme::INK_SOFT));
}

/// A value on the right of a row, cut with "…" when it would not fit.
fn value_label(ui: &mut egui::Ui, value: &str) {
    ui.add(egui::Label::new(egui::RichText::new(value).font(theme::regular(12.5)).color(theme::INK)).truncate());
}

/// A setting with its title and description on the left and a switch on the
/// right. Returns whether it was flipped.
fn setting_row(ui: &mut egui::Ui, title: &str, description: &str, value: &mut bool) -> bool {
    let mut changed = false;
    ui.horizontal(|ui| {
        let toggle_w = 48.0;
        ui.allocate_ui_with_layout(
            egui::Vec2::new(ui.available_width() - toggle_w, 0.0),
            Layout::top_down(Align::Min),
            |ui| label_column(ui, title, description),
        );
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            changed = widgets::toggle(ui, value).changed();
        });
    });
    changed
}

/// Title + description, sized to leave room for a control on the right.
fn label_block(ui: &mut egui::Ui, title: &str, description: &str) {
    let width = (ui.available_width() * 0.55).max(160.0);
    ui.allocate_ui_with_layout(egui::Vec2::new(width, 0.0), Layout::top_down(Align::Min), |ui| {
        label_column(ui, title, description)
    });
}

fn label_column(ui: &mut egui::Ui, title: &str, description: &str) {
    ui.spacing_mut().item_spacing.y = 1.0;
    ui.label(egui::RichText::new(title).font(theme::semibold(13.5)).color(theme::INK));
    ui.add(egui::Label::new(egui::RichText::new(description).font(theme::regular(12.0)).color(theme::MUTED_FG)).wrap());
}

fn divider(ui: &mut egui::Ui) {
    let y = ui.cursor().top();
    ui.painter().hline(ui.max_rect().x_range(), y, egui::Stroke::new(1.0, theme::MUTED));
    ui.add_space(1.0);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::CaptureFlags;

    fn detail<'a>(rows: &'a [PolicyRow], title: &str) -> &'a PolicyRow {
        rows.iter().find(|r| r.title == title).expect(title)
    }

    #[test]
    fn the_privacy_text_matches_what_is_collected() {
        let snapshot = SharedState {
            capture: CaptureFlags { screenshot: true, running_programs: true, ..CaptureFlags::default() },
            idle_threshold_minutes: Some(5),
            ..SharedState::default()
        };
        let rows = policy_rows(&snapshot);
        // Running programs are names only (CONTRACT §1).
        assert!(detail(&rows, "Running programs").detail.contains("never their window titles"));
        assert!(detail(&rows, "Keyboard activity").detail.contains("never which keys"));
        let idle = detail(&rows, "Idle detection");
        assert!(idle.on && idle.detail.starts_with("After 5 min"));
        assert!(!detail(&rows, "Screenshots").detail.contains("blurred"));
    }

    #[test]
    fn blur_and_idle_detection_follow_the_project() {
        let snapshot = SharedState {
            capture: CaptureFlags { screenshot: true, blur_screenshots: true, ..CaptureFlags::default() },
            idle_threshold_minutes: None,
            ..SharedState::default()
        };
        let rows = policy_rows(&snapshot);
        assert!(detail(&rows, "Screenshots").detail.contains("blurred on this computer"));
        let idle = detail(&rows, "Idle detection");
        assert!(!idle.on && idle.detail.starts_with("Off on this project"));
    }
}
