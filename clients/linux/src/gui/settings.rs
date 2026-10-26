//! Settings screen: general, goals, what is recorded, privacy, this device,
//! account, service and help.
//!
//! Ported from the Windows agent's `ui/settings.rs`. Preferences live in the
//! agent's configuration (`config.set` through the daemon, validated there);
//! starting at login and the service controls run the `dosi-tracker` command
//! itself (`service enable/disable/start/restart/stop`), so the window and
//! the command line share one implementation and its safeguards.

use chrono::{DateTime, Utc};
use eframe::egui;
use egui::{Align, Layout};
use serde_json::{json, Value};

use super::consent::{self, Capability, ConsentAction};
use super::widgets::{self, ButtonKind, Icon, Tone};
use super::{format, section, theme, JobKind, TrackerApp};
use crate::agent::state::{SessionKind, SharedState, TrackerStatus};
use crate::consent::LocalConsent;
use crate::ipc::method;

/// How many consent receipts the history lists.
pub const HISTORY_SHOWN: usize = 8;

/// One line of `consent-log.jsonl`, in words.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Receipt {
    pub at: DateTime<Utc>,
    pub text: String,
}

/// What a consent allows, in a few words.
pub fn consent_words(consent: &LocalConsent) -> String {
    match consent {
        LocalConsent::Absent => "not decided".to_owned(),
        LocalConsent::Revoked => "declined".to_owned(),
        LocalConsent::Granted(p) => {
            let mut parts = Vec::new();
            if p.keyboard {
                parts.push("keyboard counts");
            }
            if p.mouse {
                parts.push("mouse counts");
            }
            if p.active_window {
                parts.push("active window");
            }
            if p.running_programs {
                parts.push("running programs");
            }
            if p.screenshot {
                parts.push(if p.blur_screenshots {
                    "blurred screenshots"
                } else {
                    "screenshots"
                });
            }
            if parts.is_empty() {
                "time only".to_owned()
            } else {
                parts.join(", ")
            }
        }
    }
}

/// The newest `limit` receipts of a `consent-log.jsonl` text, newest first.
/// Lines that are not receipts are skipped.
pub fn parse_receipts(text: &str, limit: usize) -> Vec<Receipt> {
    let mut receipts: Vec<Receipt> = text
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .filter_map(|line| {
            let at = serde_json::from_value::<DateTime<Utc>>(line.get("at")?.clone()).ok()?;
            let to = serde_json::from_value::<LocalConsent>(line.get("to")?.clone()).ok()?;
            let origin = match line.get("origin").and_then(Value::as_str) {
                Some("cli") => "from the command line",
                Some("gui") => "in this window",
                Some("tray") => "from the tray",
                _ => "on this computer",
            };
            Some(Receipt {
                at,
                text: format!("Changed to {} {origin}", consent_words(&to)),
            })
        })
        .collect();
    receipts.reverse();
    receipts.truncate(limit);
    receipts
}

impl TrackerApp {
    pub(super) fn settings_view(&mut self, ui: &mut egui::Ui, snapshot: &SharedState) {
        if let Some(error) = self.settings_error.clone() {
            widgets::banner(ui, Tone::Danger, &error);
        }
        self.general_settings(ui, snapshot);
        self.goal_settings(ui);
        self.recorded_card(ui, snapshot);
        self.privacy_card(ui, snapshot);
        self.device_card(ui, snapshot);
        self.account_card(ui, snapshot);
        self.service_card(ui);
        self.support_card(ui);
    }

    fn general_settings(&mut self, ui: &mut egui::Ui, snapshot: &SharedState) {
        let writable = self.writable();
        section(ui, "General", None, |ui| {
            let mut autostart = self.autostart_enabled;
            if setting_row(
                ui,
                "Start at login",
                "Start Dosi Tracker when you log in to this desktop.",
                &mut autostart,
            ) {
                self.run_cli(if autostart {
                    JobKind::Enable
                } else {
                    JobKind::Disable
                });
            }
            divider(ui);
            let mut open = self.config.gui.open_window_at_login;
            if setting_row(
                ui,
                "Open this window at login",
                "Show the window as well when Dosi Tracker starts at login.",
                &mut open,
            ) && writable
            {
                self.set_config("gui.open_window_at_login", open.to_string());
            }
            divider(ui);
            let mut notify = self.config.notifications.enabled;
            if setting_row(
                ui,
                "Show notifications",
                "When you come back from time away, when tracking starts by itself, and when your sign-in expires. Takes effect when the agent restarts.",
                &mut notify,
            ) && writable
            {
                self.set_config("notifications.enabled", notify.to_string());
            }
            ui.add_space(2.0);
            theme::inset(10).show(ui, |ui| {
                ui.set_width(ui.available_width());
                let tray = if snapshot.indicator.tray {
                    "The tray icon shows whether tracking is on."
                } else {
                    "This desktop shows no tray icon; on GNOME, the AppIndicator extension adds one."
                };
                widgets::muted(
                    ui,
                    &format!(
                        "Closing this window does not stop tracking: the agent keeps running. {tray} To stop it, use Stop under Service below or dosi-tracker service stop."
                    ),
                );
            });
        });
    }

    fn goal_settings(&mut self, ui: &mut egui::Ui) {
        let writable = self.writable();
        section(ui, "Goals", None, |ui| {
            let goal = self.config.gui.daily_goal_hours;
            let mut set_goal = None;
            ui.horizontal(|ui| {
                label_block(ui, "Daily goal", "Hours you aim to track each working day.");
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.spacing_mut().item_spacing.x = 6.0;
                    if widgets::chip(ui, "+", false, writable && goal < 24)
                        .on_hover_text("An hour more")
                        .clicked()
                    {
                        set_goal = Some(goal + 1);
                    }
                    ui.label(
                        egui::RichText::new(widgets::hours_label(f64::from(goal)))
                            .font(theme::semibold(14.0))
                            .color(theme::INK),
                    );
                    if widgets::chip(ui, "−", false, writable && goal > 1)
                        .on_hover_text("An hour less")
                        .clicked()
                    {
                        set_goal = Some(goal - 1);
                    }
                });
            });
            divider(ui);
            let mut set_days = None;
            ui.horizontal(|ui| {
                label_block(ui, "Working days", "Days a week; sets the weekly goal.");
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.spacing_mut().item_spacing.x = 4.0;
                    for days in (5..=7).rev() {
                        let selected = self.config.gui.work_days_per_week == days;
                        if widgets::chip(ui, &format!("{days} days"), selected, writable).clicked()
                            && !selected
                        {
                            set_days = Some(days);
                        }
                    }
                });
            });
            ui.add_space(2.0);
            widgets::muted(
                ui,
                &format!(
                    "Weekly goal: {}. Goals are personal and only shown on this computer.",
                    widgets::hours_label(self.weekly_goal_secs() / 3600.0)
                ),
            );
            if let Some(goal) = set_goal {
                self.set_config("gui.daily_goal_hours", goal.to_string());
            }
            if let Some(days) = set_days {
                self.set_config("gui.work_days_per_week", days.to_string());
            }
        });
    }

    /// The effective matrix: each capability's state with the first reason
    /// it is off (design 10).
    fn recorded_card(&mut self, ui: &mut egui::Ui, snapshot: &SharedState) {
        let project = snapshot
            .current_project()
            .map(|p| p.title.clone())
            .unwrap_or_else(|| "No project".to_owned());
        section(ui, "What is recorded", Some(&project), |ui| {
            for (i, capability) in Capability::ALL.into_iter().enumerate() {
                if i > 0 {
                    divider(ui);
                }
                let info = capability.info(snapshot);
                let (tone, label) = consent::state_badge(info.state);
                let reason = if capability == Capability::Webcam {
                    Some(consent::WEBCAM_NOTE.to_owned())
                } else {
                    info.reason.clone()
                };
                ui.horizontal_top(|ui| {
                    ui.spacing_mut().item_spacing.x = 10.0;
                    widgets::icon(ui, capability.icon(), 18.0, theme::INK_SOFT);
                    let pill_w = 150.0;
                    ui.allocate_ui_with_layout(
                        egui::Vec2::new((ui.available_width() - pill_w).max(0.0), 0.0),
                        Layout::top_down(Align::Min),
                        |ui| {
                            ui.spacing_mut().item_spacing.y = 0.0;
                            ui.label(
                                egui::RichText::new(capability.title())
                                    .font(theme::semibold(13.0))
                                    .color(theme::INK),
                            );
                            if let Some(reason) = &reason {
                                widgets::muted(ui, reason);
                            }
                        },
                    );
                    ui.with_layout(Layout::right_to_left(Align::Min), |ui| {
                        widgets::status_pill(ui, tone, label);
                    });
                });
            }
            ui.add_space(4.0);
            theme::inset(10).show(ui, |ui| {
                ui.set_width(ui.available_width());
                widgets::muted(
                    ui,
                    &format!(
                        "A block is saved every {} minutes; time with the screen locked or the computer asleep is never recorded. {}",
                        snapshot.interval_minutes,
                        consent::NEVER_COLLECTED
                    ),
                );
            });
        });
    }

    /// The consent editor, the queue purge and the consent history.
    fn privacy_card(&mut self, ui: &mut egui::Ui, snapshot: &SharedState) {
        let writable = self.writable();
        section(ui, "Privacy", None, |ui| {
            // Follow a consent changed elsewhere (the CLI, another window).
            if self.consent_vm.current != snapshot.consent {
                self.consent_vm = consent::ConsentViewModel::new(snapshot.consent);
            }
            let (tone, text) = match &snapshot.consent {
                LocalConsent::Granted(_) => (Tone::Success, "Consented on this computer"),
                LocalConsent::Revoked => (Tone::Warning, "Declined"),
                LocalConsent::Absent => (Tone::Warning, "Consent required before tracking"),
            };
            ui.horizontal(|ui| {
                ui.label(
                    egui::RichText::new("Your choice:")
                        .font(theme::semibold(13.0))
                        .color(theme::INK),
                );
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    widgets::status_pill(ui, tone, text);
                });
            });
            if let Some(error) = &snapshot.consent_error {
                widgets::banner(ui, Tone::Danger, error);
            }
            if snapshot.consent_unsynced {
                widgets::muted(
                    ui,
                    "The last change could not be saved to disk; nothing more than before is recorded until it is.",
                );
            }
            divider(ui);
            consent::render_rows(ui, snapshot, &mut self.consent_vm);
            divider(ui);
            let mut action = None;
            ui.horizontal(|ui| {
                let dirty = self.consent_vm.is_dirty() || !self.consent_vm.is_granted();
                if widgets::button(
                    ui,
                    ButtonKind::Primary,
                    Some(Icon::Check),
                    "Save permissions",
                    None,
                    writable && dirty,
                )
                .clicked()
                {
                    action = Some(ConsentAction::Save(self.consent_vm.grant()));
                }
                if widgets::button(
                    ui,
                    ButtonKind::Secondary,
                    None,
                    "Revoke all",
                    None,
                    writable && snapshot.consent.is_granted(),
                )
                .clicked()
                {
                    action = Some(ConsentAction::Save(self.consent_vm.revoke()));
                }
                if snapshot.session.kind == SessionKind::Wayland
                    && self.consent_vm.draft.screenshot
                    && widgets::button(
                        ui,
                        ButtonKind::Secondary,
                        Some(Icon::Camera),
                        "Test screenshot permission",
                        None,
                        writable,
                    )
                    .clicked()
                {
                    action = Some(ConsentAction::TestScreenshotPermission);
                }
            });
            if let Some(action) = action {
                self.consent_action(action);
            }
            ui.add_space(4.0);
            divider(ui);
            ui.label(
                egui::RichText::new("Consent history")
                    .font(theme::semibold(13.0))
                    .color(theme::INK_SOFT),
            );
            let receipts = self.consent_history();
            if receipts.is_empty() {
                widgets::muted(ui, "No change was recorded on this computer yet.");
            }
            let now = Utc::now();
            for receipt in receipts {
                widgets::muted(
                    ui,
                    &format!(
                        "{} — {}",
                        format::day_and_time(receipt.at, now),
                        receipt.text
                    ),
                );
            }
        });
    }

    /// This computer as the workspace sees it.
    fn device_card(&mut self, ui: &mut egui::Ui, snapshot: &SharedState) {
        let zone = self.zone_label();
        let device = self.device.clone();
        let session = match snapshot.session.kind {
            SessionKind::X11 => "X11",
            SessionKind::Wayland => "Wayland",
            SessionKind::Tty => "Text console",
            SessionKind::None => "None",
        };
        let session = match snapshot.session.desktop.as_deref() {
            Some(desktop) if !desktop.trim().is_empty() => format!("{session} · {desktop}"),
            _ => session.to_owned(),
        };
        let mut copy = None;
        section(ui, "This device", None, |ui| {
            info_row(
                ui,
                Icon::Monitor,
                "Computer name",
                device.name.as_deref().unwrap_or("Unknown"),
            );
            divider(ui);
            ui.horizontal(|ui| {
                label_cell(ui, Icon::Info, "Device ID");
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.spacing_mut().item_spacing.x = 6.0;
                    match &device.id {
                        Some(id) => {
                            if widgets::icon_button(
                                ui,
                                Icon::Copy,
                                16.0,
                                theme::INK_SOFT,
                                "Copy the device ID",
                            )
                            .clicked()
                            {
                                copy = Some(id.clone());
                            }
                            value_label(ui, id);
                        }
                        None => value_label(ui, "Created when the agent first starts"),
                    }
                });
            });
            divider(ui);
            info_row(
                ui,
                Icon::Briefcase,
                "Agent version",
                env!("CARGO_PKG_VERSION"),
            );
            divider(ui);
            info_row(ui, Icon::Window, "System", &device.os);
            divider(ui);
            info_row(ui, Icon::Monitor, "Session", &session);
            divider(ui);
            info_row(ui, Icon::Globe, "Time zone", &zone);
            ui.add_space(2.0);
            widgets::muted(
                ui,
                "Sent with your tracked time and while you're online, so your workspace can tell your computers apart and show your days in your own time zone.",
            );
        });
        if let Some(id) = copy {
            ui.ctx().copy_text(id);
            self.show_toast(Tone::Success, "Device ID copied.");
        }
    }

    fn account_card(&mut self, ui: &mut egui::Ui, snapshot: &SharedState) {
        let server = if snapshot.server_url.trim().is_empty() {
            self.config.api_base_url.clone().unwrap_or_default()
        } else {
            snapshot.server_url.clone()
        };
        let dashboard_ready = self.dashboard_available();
        let dashboard = match &self.config.dashboard_url {
            Some(url) => format::server_label(url),
            None => "Not set on this computer".to_owned(),
        };
        let writable = self.writable();
        section(ui, "Account", None, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 12.0;
                widgets::avatar(ui, &snapshot.display_name, 42.0);
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 1.0;
                    ui.add_space(3.0);
                    ui.add(
                        egui::Label::new(
                            egui::RichText::new(&snapshot.display_name)
                                .font(theme::semibold(14.0))
                                .color(theme::INK),
                        )
                        .truncate(),
                    );
                    let workspace = if snapshot.workspace.trim().is_empty() {
                        "No workspace chosen".to_owned()
                    } else {
                        format!("Workspace: {}", snapshot.workspace.trim())
                    };
                    ui.label(
                        egui::RichText::new(workspace)
                            .font(theme::regular(12.0))
                            .color(theme::MUTED_FG),
                    );
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
                    ui.label(
                        egui::RichText::new("Sign out of Dosi Tracker?")
                            .font(theme::semibold(13.0))
                            .color(theme::INK),
                    );
                    widgets::muted(
                        ui,
                        "The block in progress is saved and uploaded first. Tracking stops on this computer until someone signs in again.",
                    );
                    ui.add_space(4.0);
                    let mut cancel = false;
                    let mut confirm = false;
                    super::equal_row(ui, 2, 8.0, |i, ui| {
                        let width = Some(ui.available_width());
                        if i == 0 {
                            cancel = widgets::button(ui, ButtonKind::Secondary, None, "Cancel", width, true)
                                .clicked();
                        } else {
                            confirm = widgets::button(
                                ui,
                                ButtonKind::Danger,
                                Some(Icon::SignOut),
                                "Sign out",
                                width,
                                writable,
                            )
                            .clicked();
                        }
                    });
                    if cancel {
                        self.confirm_sign_out = false;
                    }
                    if confirm {
                        self.confirm_sign_out = false;
                        self.link
                            .call(method::AUTH_SIGN_OUT, json!({ "at": Utc::now() }));
                    }
                });
            } else {
                let mut open_dashboard = false;
                let mut sign_out = false;
                super::equal_row(ui, 2, 8.0, |i, ui| {
                    let width = Some(ui.available_width());
                    if i == 0 {
                        let response = widgets::button(
                            ui,
                            ButtonKind::Secondary,
                            Some(Icon::External),
                            "Web dashboard",
                            width,
                            dashboard_ready,
                        );
                        open_dashboard = if dashboard_ready {
                            response.clicked()
                        } else {
                            response.on_hover_text(super::DASHBOARD_UNSET).clicked()
                        };
                    } else {
                        sign_out = widgets::button(
                            ui,
                            ButtonKind::Danger,
                            Some(Icon::SignOut),
                            "Sign out",
                            width,
                            writable,
                        )
                        .clicked();
                    }
                });
                if open_dashboard {
                    self.open_dashboard();
                }
                if sign_out {
                    self.confirm_sign_out = true;
                }
            }
        });
    }

    /// "Switch workspace": the user's workspaces; one click moves tracking there.
    fn switch_workspace(&mut self, ui: &mut egui::Ui, snapshot: &SharedState) {
        if snapshot.display_name.trim().is_empty()
            || snapshot.status == TrackerStatus::SignInExpired
        {
            return;
        }
        if let Some(note) = &snapshot.workspace_note {
            widgets::muted(ui, note);
        }
        let hidden = self.login.hidden_choices.as_ref() == Some(&snapshot.workspace_choices);
        if snapshot.workspace_choices.is_empty() || hidden {
            let finding = snapshot.finding_workspaces;
            let label = if finding {
                "Looking up workspaces…"
            } else {
                "Switch workspace"
            };
            if widgets::button(
                ui,
                ButtonKind::Secondary,
                None,
                label,
                Some(ui.available_width()),
                !finding && self.writable(),
            )
            .clicked()
            {
                self.login.hidden_choices = None;
                self.link.call(method::WORKSPACE_LIST, json!({}));
            }
            return;
        }
        let mut action = None;
        theme::inset(12).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 6.0;
            ui.label(
                egui::RichText::new("Switch workspace")
                    .font(theme::semibold(15.0))
                    .color(theme::INK),
            );
            for (index, choice) in snapshot.workspace_choices.iter().enumerate() {
                let label = super::login::workspace_label(choice);
                if widgets::button(
                    ui,
                    ButtonKind::Secondary,
                    None,
                    &label,
                    Some(ui.available_width()),
                    !choice.is_current && self.writable(),
                )
                .clicked()
                {
                    action = Some(Some(index));
                }
            }
            if snapshot.workspace_lost.is_none()
                && ui
                    .vertical_centered(|ui| {
                        widgets::link(ui, "Back", theme::PRIMARY_STRONG).clicked()
                    })
                    .inner
            {
                action = Some(None);
            }
        });
        match action {
            None => {}
            Some(None) => self.login.hidden_choices = Some(snapshot.workspace_choices.clone()),
            Some(Some(index)) => self.link.call(
                method::WORKSPACE_CHOOSE,
                json!({ "index": index, "at": Utc::now() }),
            ),
        }
    }

    /// The agent's service: installed, started at login, running; restart,
    /// stop and the logs.
    fn service_card(&mut self, ui: &mut egui::Ui) {
        if self.service.is_none() && !self.job_running(JobKind::Status) {
            self.run_cli(JobKind::Status);
        }
        let busy = self.jobs_running();
        section(ui, "Service", None, |ui| {
            match &self.service {
                Some(status) => {
                    let yes_no = |key: &str| {
                        if status.get(key).and_then(Value::as_bool) == Some(true) {
                            "Yes"
                        } else {
                            "No"
                        }
                    };
                    info_row(
                        ui,
                        Icon::Briefcase,
                        "Installed as a user service",
                        yes_no("installed"),
                    );
                    divider(ui);
                    info_row(
                        ui,
                        Icon::Timer,
                        "Starts at login",
                        yes_no("enabled_at_login"),
                    );
                    divider(ui);
                    let running = match status.get("daemon").and_then(|d| d.get("pid")) {
                        Some(pid) if pid.is_u64() => format!("Yes (process {pid})"),
                        _ if status.get("active").and_then(Value::as_bool) == Some(true) => {
                            "Yes".to_owned()
                        }
                        _ => "No".to_owned(),
                    };
                    info_row(ui, Icon::Sync, "Running", &running);
                }
                None => {
                    widgets::muted(ui, "Checking the service…");
                }
            }
            ui.add_space(4.0);
            let mut chosen = None;
            super::equal_row(ui, 3, 8.0, |i, ui| {
                let width = Some(ui.available_width());
                match i {
                    0 => {
                        if widgets::button(
                            ui,
                            ButtonKind::Secondary,
                            Some(Icon::Sync),
                            "Restart",
                            width,
                            !busy,
                        )
                        .on_hover_text("Restart the agent; the block in progress is saved first.")
                        .clicked()
                        {
                            chosen = Some(JobKind::Restart);
                        }
                    }
                    1 => {
                        if widgets::button(ui, ButtonKind::Danger, None, "Stop", width, !busy)
                            .on_hover_text(
                                "Stop the agent: nothing is tracked until it starts again.",
                            )
                            .clicked()
                        {
                            chosen = Some(JobKind::Stop);
                        }
                    }
                    _ => {
                        if widgets::button(
                            ui,
                            ButtonKind::Secondary,
                            Some(Icon::Folder),
                            "Logs",
                            width,
                            true,
                        )
                        .on_hover_text(
                            "The agent's log files — attach them when reporting a problem.",
                        )
                        .clicked()
                        {
                            crate::agent::tray::xdg_open(
                                &self.paths.logs_dir().display().to_string(),
                            );
                        }
                    }
                }
            });
            if let Some(job) = chosen {
                self.run_cli(job);
            }
        });
    }

    fn support_card(&mut self, ui: &mut egui::Ui) {
        let running = self.job_running(JobKind::Doctor);
        section(
            ui,
            "Help",
            Some(concat!("Version ", env!("CARGO_PKG_VERSION"))),
            |ui| {
                match &self.doctor {
                    Some(problems) if problems.is_empty() => {
                        widgets::banner(ui, Tone::Success, "No problems found on this computer.");
                    }
                    Some(problems) => {
                        for (tone, text) in problems {
                            widgets::banner(ui, *tone, text);
                        }
                    }
                    None => {
                        widgets::muted(
                            ui,
                            "Check this computer's session, keyring, tray, notifications and queue. Nothing is changed.",
                        );
                    }
                }
                let label = if running {
                    "Checking…"
                } else {
                    "Run diagnostics"
                };
                if widgets::button(
                    ui,
                    ButtonKind::Secondary,
                    Some(Icon::Info),
                    label,
                    Some(ui.available_width()),
                    !running,
                )
                .clicked()
                {
                    self.run_cli(JobKind::Doctor);
                }
            },
        );
    }
}

/// The problems of a `doctor --json` report, as banners.
pub fn doctor_problems(report: &Value) -> Vec<(Tone, String)> {
    report
        .get("problems")
        .and_then(Value::as_array)
        .map(|problems| {
            problems
                .iter()
                .map(|p| {
                    let tone = if p.get("severity").and_then(Value::as_str) == Some("blocking") {
                        Tone::Danger
                    } else {
                        Tone::Warning
                    };
                    let message = p.get("message").and_then(Value::as_str).unwrap_or("?");
                    let text = match p.get("hint").and_then(Value::as_str) {
                        Some(hint) => format!("{message} ({hint})"),
                        None => message.to_owned(),
                    };
                    (tone, text)
                })
                .collect()
        })
        .unwrap_or_default()
}

/// A label with its icon on the left and a value on the right.
fn info_row(ui: &mut egui::Ui, icon: Icon, label: &str, value: &str) {
    ui.horizontal(|ui| {
        label_cell(ui, icon, label);
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            value_label(ui, value)
        });
    });
}

fn label_cell(ui: &mut egui::Ui, icon: Icon, label: &str) {
    ui.spacing_mut().item_spacing.x = 10.0;
    widgets::icon(ui, icon, 16.0, theme::MUTED_FG);
    ui.label(
        egui::RichText::new(label)
            .font(theme::semibold(12.5))
            .color(theme::INK_SOFT),
    );
}

/// A value on the right of a row, cut with "…" when it would not fit.
fn value_label(ui: &mut egui::Ui, value: &str) {
    ui.add(
        egui::Label::new(
            egui::RichText::new(value)
                .font(theme::regular(12.5))
                .color(theme::INK),
        )
        .truncate(),
    );
}

/// A setting with its title and description on the left and a switch on
/// the right. Returns whether it was flipped.
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
    ui.allocate_ui_with_layout(
        egui::Vec2::new(width, 0.0),
        Layout::top_down(Align::Min),
        |ui| label_column(ui, title, description),
    );
}

fn label_column(ui: &mut egui::Ui, title: &str, description: &str) {
    ui.spacing_mut().item_spacing.y = 1.0;
    ui.label(
        egui::RichText::new(title)
            .font(theme::semibold(13.5))
            .color(theme::INK),
    );
    ui.add(
        egui::Label::new(
            egui::RichText::new(description)
                .font(theme::regular(12.0))
                .color(theme::MUTED_FG),
        )
        .wrap(),
    );
}

fn divider(ui: &mut egui::Ui) {
    consent::divider(ui);
}
