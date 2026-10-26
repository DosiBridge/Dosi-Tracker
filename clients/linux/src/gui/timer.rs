//! Timer screen: status, project picker, task note, pause/resume, breaks,
//! totals against goals, sync now, away prompt.
//!
//! Ported from the Windows agent's `ui/timer.rs`. The window is a thin
//! client: everything here is read from the daemon's snapshot, and every
//! action is one IPC request. The block card shows only what this desktop
//! measures (on Wayland, for example, no input counts).

use chrono::{DateTime, Utc};
use eframe::egui;
use egui::{Color32, CornerRadius, Pos2, Rect, Sense, Stroke, Vec2};
use serde_json::json;

use super::alerts::{self, AlertAction};
use super::widgets::{self, ButtonKind, Callout, Icon};
use super::{
    equal_row, format, metric, section, theme, two_columns, TrackerApp, View, PAUSE_CHOICES,
};
use crate::agent::state::{CapabilityState, PendingAction, SharedState, TrackerStatus};
use crate::agent::worker::{clean_note, TASK_NOTE_MAX_CHARS};
use crate::ipc::method;

/// With more projects than this, the chooser gets a filter field.
const FILTER_FROM: usize = 6;

/// The task note's counter shows once this close to the limit.
const COUNTER_WITHIN: usize = 40;

/// The "What are you working on?" field between frames.
#[derive(Debug, Default)]
pub struct TaskField {
    /// What is typed; follows the saved note while the field is not in use.
    pub draft: String,
    /// The field has the keyboard.
    pub editing: bool,
    /// Give it the keyboard when next shown (the tray's "Set task…").
    pub focus_requested: bool,
}

impl TaskField {
    /// The field, under its header. `saved` is the note the daemon keeps.
    /// Returns a note to apply: Enter, or leaving the field, with a change
    /// (compared the way the worker tidies notes). Esc puts the saved note
    /// back.
    pub fn show(&mut self, ui: &mut egui::Ui, saved: &str) -> Option<String> {
        if !self.editing && self.draft != saved {
            self.draft = saved.to_owned();
        }
        let used = self.draft.chars().count();
        let counter = (self.editing && used + COUNTER_WITHIN >= TASK_NOTE_MAX_CHARS)
            .then(|| format!("{used}/{TASK_NOTE_MAX_CHARS}"));
        widgets::section_header(ui, "What are you working on?", counter.as_deref());
        let response = ui.add_sized(
            [ui.available_width(), theme::CONTROL_H],
            egui::TextEdit::singleline(&mut self.draft)
                .id_salt("task-note")
                .char_limit(TASK_NOTE_MAX_CHARS)
                .hint_text("e.g. Checkout page redesign")
                .font(theme::regular(13.5))
                .margin(egui::Margin::symmetric(12, 10)),
        );
        if std::mem::take(&mut self.focus_requested) {
            response.request_focus();
        }
        if response.has_focus() {
            self.editing = true;
        }
        let mut apply = None;
        if response.lost_focus() {
            self.editing = false;
            if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                self.draft = saved.to_owned();
            } else if clean_note(&self.draft) != clean_note(saved) {
                apply = Some(self.draft.clone());
            }
        }
        if self.editing && clean_note(&self.draft) != clean_note(saved) {
            widgets::muted(
                ui,
                "Press Enter to apply — a new block starts with it. Esc cancels.",
            );
        }
        apply
    }
}

impl TrackerApp {
    pub(super) fn timer_view(
        &mut self,
        ui: &mut egui::Ui,
        snapshot: &SharedState,
        now: DateTime<Utc>,
        wide: bool,
    ) {
        self.alerts(ui, snapshot);
        if !snapshot.consent.is_granted() {
            let mut action = None;
            super::consent::render_consent_card(ui, snapshot, &mut self.consent_vm, |a| {
                action = Some(a);
            });
            if let Some(action) = action {
                self.consent_action(action);
            }
        }
        let choosing = snapshot.status == TrackerStatus::ChooseProject;
        if wide {
            two_columns(ui, |column, ui| {
                if column == 0 {
                    if choosing {
                        self.choose_project_card(ui, snapshot);
                    } else {
                        self.session_card(ui, snapshot, now);
                    }
                    self.quick_actions(ui, snapshot);
                } else {
                    self.goal_tiles(ui, snapshot, now);
                    self.block_card(ui, snapshot, now);
                }
            });
        } else {
            if choosing {
                self.choose_project_card(ui, snapshot);
            } else {
                self.session_card(ui, snapshot, now);
            }
            self.goal_tiles(ui, snapshot, now);
            self.block_card(ui, snapshot, now);
            self.quick_actions(ui, snapshot);
        }
        if !shows_task_field(snapshot.status) {
            // Nowhere to put the keyboard: forget the request.
            self.task.focus_requested = false;
        }
    }

    /// The question after time away (CONTRACT §4): keep it as work, or not.
    /// Shown on every tab until answered; no answer records nothing.
    pub(super) fn away_prompt_card(&mut self, ui: &mut egui::Ui, snapshot: &SharedState) {
        let Some(prompt) = snapshot.away_prompt.clone() else {
            return;
        };
        let mut answer = None;
        egui::Frame::new()
            .fill(theme::CARD)
            .stroke(Stroke::new(1.0, theme::mix(theme::WARNING, theme::CARD, 0.55)))
            .corner_radius(CornerRadius::same(theme::CARD_RADIUS))
            .inner_margin(egui::Margin::same(16))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal_top(|ui| {
                    ui.spacing_mut().item_spacing.x = 12.0;
                    let (rect, _) = ui.allocate_exact_size(Vec2::splat(34.0), Sense::hover());
                    ui.painter()
                        .circle_filled(rect.center(), 17.0, theme::tint(theme::WARNING));
                    widgets::paint_icon(
                        ui.painter(),
                        Rect::from_center_size(rect.center(), Vec2::splat(18.0)),
                        Icon::Timer,
                        theme::WARNING_ON_TINT,
                    );
                    ui.vertical(|ui| {
                        ui.set_width(ui.available_width());
                        ui.spacing_mut().item_spacing.y = 4.0;
                        let title = format!(
                            "You were away for {}.",
                            format::away_summary(prompt.from, prompt.to)
                        );
                        ui.add(
                            egui::Label::new(
                                egui::RichText::new(title)
                                    .font(theme::semibold(14.0))
                                    .color(theme::INK),
                            )
                            .wrap(),
                        );
                        widgets::muted(
                            ui,
                            &format!(
                                "Keep it as work on {}, recorded with no activity, or discard it. Nothing is recorded unless you keep it.",
                                prompt.project_title
                            ),
                        );
                        ui.add_space(4.0);
                        let writable = self.writable();
                        equal_row(ui, 2, 8.0, |i, ui| {
                            let width = Some(ui.available_width());
                            if i == 0 {
                                if widgets::button(
                                    ui,
                                    ButtonKind::Primary,
                                    Some(Icon::Check),
                                    "Keep as work",
                                    width,
                                    writable,
                                )
                                .clicked()
                                {
                                    answer = Some(true);
                                }
                            } else if widgets::button(
                                ui,
                                ButtonKind::Secondary,
                                None,
                                "Discard",
                                width,
                                writable,
                            )
                            .clicked()
                            {
                                answer = Some(false);
                            }
                        });
                    });
                });
            });
        if let Some(keep) = answer {
            self.link
                .call(method::AWAY_RESOLVE, json!({ "keep": keep }));
            // The daemon confirms by publishing the prompt gone; until then
            // the card must not offer the choice twice.
            self.link.update_local(|s| s.away_prompt = None);
        }
    }

    /// Problems that need the user's attention, most serious first.
    fn alerts(&mut self, ui: &mut egui::Ui, snapshot: &SharedState) {
        for alert in alerts::alerts(snapshot) {
            let content = Callout {
                title: (!alert.title.is_empty()).then_some(alert.title.as_str()),
                body: &alert.body,
                detail: alert.detail.as_deref(),
                action: alert.action.map(AlertAction::label),
            };
            if widgets::callout(ui, alert.tone, &content) && self.writable() {
                match alert.action {
                    Some(AlertAction::SyncNow) => self.sync_now(),
                    Some(AlertAction::ShowActivity) => self.view = View::Activity,
                    Some(AlertAction::ShowSettings) => self.view = View::Settings,
                    Some(AlertAction::Unlock) => {
                        self.link.call(method::AUTH_UNLOCK_KEYRING, json!({}));
                    }
                    Some(AlertAction::RetryScreenshot) => {
                        self.link.call(method::CAPTURE_RETRY_PERMISSION, json!({}));
                    }
                    None => {}
                }
            }
        }
    }

    /// Signed in, but no project chosen: the projects to pick from, or why
    /// there are none (CONTRACT §2 — never silently the first of several).
    fn choose_project_card(&mut self, ui: &mut egui::Ui, snapshot: &SharedState) {
        theme::card(18).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 6.0;
            let projects = &snapshot.projects;
            let trailing =
                (!projects.is_empty()).then(|| format::count(projects.len() as u32, "project"));
            widgets::section_header(ui, "Choose a project", trailing.as_deref());

            if projects.is_empty() {
                ui.add_space(6.0);
                ui.vertical_centered(|ui| {
                    ui.spacing_mut().item_spacing.y = 4.0;
                    widgets::icon(ui, Icon::Briefcase, 28.0, theme::FAINT);
                    ui.label(
                        egui::RichText::new("You're not on any active project")
                            .font(theme::semibold(14.0))
                            .color(theme::INK),
                    );
                    widgets::muted(
                        ui,
                        "Ask your workspace admin to add you to a project. Dosi Tracker checks again every few minutes and starts as soon as there is one.",
                    );
                });
                return;
            }

            widgets::muted(
                ui,
                "Pick what you're working on to start tracking. You can switch any time, here or from the tray menu.",
            );
            if projects.len() > FILTER_FROM {
                ui.add_space(2.0);
                ui.add_sized(
                    [ui.available_width(), 34.0],
                    egui::TextEdit::singleline(&mut self.project_filter)
                        .hint_text("Filter projects")
                        .font(theme::regular(13.0))
                        .margin(egui::Margin::symmetric(10, 8)),
                );
            }
            ui.add_space(2.0);
            let filter = self.project_filter.trim().to_lowercase();
            let enabled = snapshot.pending_action.is_none() && self.writable();
            let mut shown = 0;
            let mut picked = None;
            for project in projects
                .iter()
                .filter(|p| filter.is_empty() || p.title.to_lowercase().contains(&filter))
            {
                shown += 1;
                if project_row(ui, &project.title, enabled).clicked() {
                    picked = Some(project.id.clone());
                }
            }
            if shown == 0 {
                widgets::muted(
                    ui,
                    &format!(
                        "No project matches \u{201c}{}\u{201d}.",
                        self.project_filter.trim()
                    ),
                );
            }
            if let Some(id) = picked {
                self.select_project(id);
                self.project_filter.clear();
            }
        });
    }

    /// Project picker, the task note, the big clock, and the start/pause
    /// control.
    fn session_card(&mut self, ui: &mut egui::Ui, snapshot: &SharedState, now: DateTime<Utc>) {
        theme::card(18).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 6.0;

            widgets::section_header(
                ui,
                "Project",
                Some(&format!("Saved every {} min", snapshot.interval_minutes)),
            );
            self.project_picker(ui, snapshot);
            if shows_task_field(snapshot.status) {
                ui.add_space(8.0);
                self.task_field(ui, snapshot);
            }
            ui.add_space(14.0);

            let paused = snapshot.status == TrackerStatus::Paused;
            let (caption, caption_color, digits, digits_color, detail) = clock_face(snapshot, now);

            ui.vertical_centered(|ui| {
                ui.spacing_mut().item_spacing.y = 2.0;
                ui.label(
                    egui::RichText::new(caption.to_uppercase())
                        .font(theme::semibold(11.0))
                        .color(caption_color),
                );
                ui.label(
                    egui::RichText::new(digits)
                        .font(theme::semibold(46.0))
                        .color(digits_color),
                );
                ui.add(
                    egui::Label::new(
                        egui::RichText::new(detail)
                            .font(theme::regular(12.0))
                            .color(theme::MUTED_FG),
                    )
                    .wrap(),
                );
                ui.add_space(10.0);

                let pending = snapshot.pending_action;
                let idle = pending.is_none() && self.writable();
                let (fill, hover, icon, enabled, label) = if paused {
                    let label = if pending == Some(PendingAction::Resuming) {
                        "Resuming…"
                    } else {
                        "Resume tracking"
                    };
                    (
                        theme::SUCCESS,
                        theme::SUCCESS_HOVER,
                        Icon::Play,
                        idle,
                        label,
                    )
                } else {
                    let can = idle && format::can_pause(snapshot.status);
                    let label = match pending {
                        Some(PendingAction::Pausing) => "Pausing…",
                        _ if can => "Pause tracking",
                        _ => "Tracking is not running",
                    };
                    (
                        theme::PRIMARY,
                        theme::PRIMARY_HOVER,
                        Icon::Pause,
                        can,
                        label,
                    )
                };
                let response = widgets::round_button(ui, 76.0, fill, hover, icon, enabled);
                self.probe("toggle", response.rect);
                let hint = if paused {
                    "Start a new block now. The break is not counted."
                } else {
                    "Pause tracking. The work so far is saved and uploaded."
                };
                if response.on_hover_text(hint).clicked() {
                    if paused {
                        self.resume(snapshot);
                    } else {
                        self.pause(snapshot, None);
                    }
                }
                ui.label(
                    egui::RichText::new(label)
                        .font(theme::semibold(12.5))
                        .color(theme::INK_SOFT),
                );
            });

            // Timed breaks. While paused the same choices re-arm the
            // auto-resume from now, and "Never" turns it off.
            if paused || format::can_pause(snapshot.status) {
                ui.add_space(10.0);
                let idle = snapshot.pending_action.is_none() && self.writable();
                let show_never = paused && snapshot.resume_at.is_some();
                let caption = if paused {
                    "Resume after"
                } else {
                    "Take a break"
                };
                let mut labels: Vec<&str> = PAUSE_CHOICES.iter().map(|(label, _)| *label).collect();
                if show_never {
                    labels.push("Never");
                }
                let caption_w = ui
                    .painter()
                    .layout_no_wrap(
                        caption.to_owned(),
                        theme::regular(12.0),
                        Color32::PLACEHOLDER,
                    )
                    .size()
                    .x;
                let chips_w: f32 = labels
                    .iter()
                    .map(|l| {
                        ui.painter()
                            .layout_no_wrap(
                                (*l).to_owned(),
                                theme::semibold(12.0),
                                Color32::PLACEHOLDER,
                            )
                            .size()
                            .x
                            + 24.0
                    })
                    .sum();
                let total = caption_w + chips_w + 6.0 * labels.len() as f32 + 4.0;
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 6.0;
                    ui.add_space(((ui.available_width() - total) / 2.0).max(0.0));
                    ui.label(
                        egui::RichText::new(caption)
                            .font(theme::regular(12.0))
                            .color(theme::MUTED_FG),
                    );
                    ui.add_space(4.0);
                    for (label, duration) in PAUSE_CHOICES {
                        let hint = if paused {
                            format!("Stay on a break, then resume automatically in {label}")
                        } else {
                            format!("Pause now and resume automatically in {label}")
                        };
                        if widgets::chip(ui, label, false, idle)
                            .on_hover_text(hint)
                            .clicked()
                        {
                            self.pause(snapshot, Some(duration));
                        }
                    }
                    if show_never
                        && widgets::chip(ui, "Never", false, idle)
                            .on_hover_text("Stay paused until you resume")
                            .clicked()
                    {
                        self.pause(snapshot, None);
                    }
                });
            }
        });
    }

    fn project_picker(&mut self, ui: &mut egui::Ui, snapshot: &SharedState) {
        let current = snapshot
            .current_project()
            .map(|p| p.title.clone())
            .unwrap_or_else(|| "No project selected".to_owned());
        let enabled =
            snapshot.pending_action.is_none() && !snapshot.projects.is_empty() && self.writable();
        let mut picked = None;
        ui.add_enabled_ui(enabled, |ui| {
            egui::ComboBox::from_id_salt("project-picker")
                .width(ui.available_width())
                .height(260.0)
                .selected_text(
                    egui::RichText::new(current)
                        .font(theme::semibold(14.0))
                        .color(theme::INK),
                )
                .show_ui(ui, |ui| {
                    ui.set_min_width(220.0);
                    for project in &snapshot.projects {
                        let selected = snapshot.selected_project.as_deref() == Some(&project.id);
                        let text = egui::RichText::new(&project.title).font(theme::regular(13.5));
                        if ui.selectable_label(selected, text).clicked() && !selected {
                            picked = Some(project.id.clone());
                        }
                    }
                });
        })
        .response
        .on_disabled_hover_text(if snapshot.projects.is_empty() {
            "You are not a member of any active project yet."
        } else {
            "Wait for the pause/resume to finish."
        });
        if let Some(id) = picked {
            self.select_project(id);
        }
    }

    /// "What are you working on?" — sent as each block's description. A new
    /// note ends the block in progress; the next starts with it.
    fn task_field(&mut self, ui: &mut egui::Ui, snapshot: &SharedState) {
        let Some(note) = self.task.show(ui, &snapshot.task_note) else {
            return;
        };
        if !self.writable() {
            return;
        }
        self.link.call(
            method::TASK_SET,
            json!({ "note": note, "at": chrono::Utc::now() }),
        );
        // Shown at once; the daemon publishes the same tidy note.
        let shown = clean_note(&note).unwrap_or_default();
        self.task.draft = shown.clone();
        self.link.update_local(|s| s.task_note = shown);
    }

    /// Today and this week against the goals, and today's activity level.
    fn goal_tiles(&mut self, ui: &mut egui::Ui, snapshot: &SharedState, now: DateTime<Utc>) {
        let (today, week) = format::live_totals(snapshot, now);
        let daily_goal = self.daily_goal_secs();
        let weekly_goal = self.weekly_goal_secs();
        let activity = snapshot.last_productivity;
        let tiles: [(&str, String, f32, Color32, String); 3] = [
            (
                "Today",
                format::secs_label(today),
                (today / daily_goal) as f32,
                theme::PRIMARY,
                goal_caption(today, daily_goal),
            ),
            (
                "This week",
                format::secs_label(week),
                (week / weekly_goal) as f32,
                theme::PRIMARY,
                goal_caption(week, weekly_goal),
            ),
            (
                "Activity",
                format::percent_label(activity),
                activity.map_or(0.0, |a| f32::from(a) / 100.0),
                activity.map_or(theme::FAINT, activity_color),
                if activity.is_some() {
                    "Today's average".to_owned()
                } else {
                    "Not measured".to_owned()
                },
            ),
        ];
        equal_row(ui, tiles.len(), theme::GAP - 2.0, |i, ui| {
            let (label, value, fraction, color, caption) = &tiles[i];
            theme::card(14).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.spacing_mut().item_spacing.y = 4.0;
                ui.label(
                    egui::RichText::new(label.to_uppercase())
                        .font(theme::semibold(10.0))
                        .color(theme::MUTED_FG),
                );
                ui.label(
                    egui::RichText::new(value.as_str())
                        .font(theme::semibold(19.0))
                        .color(theme::INK),
                );
                ui.add_space(2.0);
                widgets::progress_bar(ui, *fraction, 6.0, *color);
                ui.add(
                    egui::Label::new(
                        egui::RichText::new(caption.as_str())
                            .font(theme::regular(11.0))
                            .color(theme::MUTED_FG),
                    )
                    .truncate(),
                );
            });
        });
    }

    /// The block in progress: when it is saved and what this desktop
    /// measures in it — or what a pause means.
    fn block_card(&mut self, ui: &mut egui::Ui, snapshot: &SharedState, now: DateTime<Utc>) {
        match snapshot.status {
            TrackerStatus::Tracking => {
                let block = snapshot.block_started_at.unwrap_or(now);
                // When the block is saved — never the screenshot moment, which
                // is random within the block and not announced.
                let next = snapshot.next_capture_at.unwrap_or(now);
                let trailing = format!("Saved in {}", format::clock(next - now));
                section(ui, "Current block", Some(&trailing), |ui| {
                    let interval = (snapshot.interval_minutes.max(1) * 60) as f32;
                    let elapsed = (now - block).num_seconds().max(0) as f32;
                    widgets::progress_bar(ui, elapsed / interval, 6.0, theme::PRIMARY);
                    ui.add_space(4.0);
                    let measured = measured_kinds(snapshot);
                    let metrics: Vec<(&str, String, Color32)> = vec![
                        ("Since", format::local_hm(block), theme::INK),
                        ("Saved at", format::local_hm(next), theme::INK),
                    ];
                    equal_row(ui, metrics.len(), 8.0, |i, ui| {
                        let (label, value, color) = &metrics[i];
                        metric(ui, label, value, *color);
                    });
                    ui.add_space(2.0);
                    if measured.is_empty() {
                        widgets::muted(
                            ui,
                            "Only time is recorded in this block: nothing about your activity.",
                        );
                    } else {
                        widgets::muted(
                            ui,
                            &format!("Recorded with this block: {}.", measured.join(", ")),
                        );
                    }
                });
            }
            TrackerStatus::Paused => {
                section(ui, "On a break", None, |ui| {
                    widgets::muted(
                        ui,
                        "The block you were working on was saved when you paused. Nothing — no screenshots, input or apps — is recorded until you resume.",
                    );
                });
            }
            TrackerStatus::Away => {
                section(ui, "Away from the computer", None, |ui| {
                    widgets::muted(
                        ui,
                        "The block ended at your last keyboard or mouse input. When you are back, a new block starts and you choose whether to keep the time away as work.",
                    );
                });
            }
            _ => {}
        }
    }

    /// Sync and the web dashboard.
    fn quick_actions(&mut self, ui: &mut egui::Ui, snapshot: &SharedState) {
        let paused = snapshot.status == TrackerStatus::Paused;
        // While paused the button never captures — it only uploads what is
        // already queued — so it says so rather than promising a sync.
        let (label, hint) = if paused {
            (
                "Upload queued",
                "Paused: uploads activity already recorded. Nothing new is captured.",
            )
        } else {
            ("Sync now", "Save the current block now and upload it.")
        };
        let dashboard_ready = self.dashboard_available();
        let writable = self.writable();
        let mut sync = false;
        let mut dashboard = false;
        equal_row(ui, 2, 8.0, |i, ui| {
            let width = Some(ui.available_width());
            if i == 0 {
                let enabled = writable
                    && !matches!(
                        snapshot.status,
                        TrackerStatus::Stopped | TrackerStatus::SignInExpired
                    );
                sync = widgets::button(
                    ui,
                    ButtonKind::Secondary,
                    Some(Icon::Sync),
                    label,
                    width,
                    enabled,
                )
                .on_hover_text(hint)
                .clicked();
            } else {
                let response = widgets::button(
                    ui,
                    ButtonKind::Secondary,
                    Some(Icon::External),
                    "Web dashboard",
                    width,
                    dashboard_ready,
                );
                dashboard = if dashboard_ready {
                    response
                        .on_hover_text("Open your timesheets and reports in the browser.")
                        .clicked()
                } else {
                    response.on_hover_text(super::DASHBOARD_UNSET).clicked()
                };
            }
        });
        if sync {
            self.sync_now();
        }
        if dashboard {
            self.open_dashboard();
        }
    }
}

/// Caption, its colour, the digits, their colour and the line under them.
fn clock_face(
    snapshot: &SharedState,
    now: DateTime<Utc>,
) -> (&'static str, Color32, String, Color32, String) {
    let blank = || "--:--".to_owned();
    match snapshot.status {
        TrackerStatus::Tracking => {
            let started = snapshot.tracking_since.unwrap_or(now);
            (
                "Tracking time",
                theme::SUCCESS_ON_TINT,
                format::clock(now - started),
                theme::INK,
                format!("Session started at {}", format::local_hm(started)),
            )
        }
        TrackerStatus::Paused => {
            let since = snapshot.paused_since.unwrap_or(now);
            let detail = match snapshot.resume_at {
                Some(at) => format!(
                    "Resumes at {} · in {}",
                    format::local_hm(at),
                    format::clock(at - now)
                ),
                None => "Nothing is recorded until you resume.".to_owned(),
            };
            (
                "On a break",
                theme::WARNING_ON_TINT,
                format::clock(now - since),
                theme::WARNING_ON_TINT,
                detail,
            )
        }
        TrackerStatus::Connecting => (
            "Connecting",
            theme::PRIMARY_STRONG,
            blank(),
            theme::FAINT,
            "Connecting to your workspace…".to_owned(),
        ),
        TrackerStatus::Offline => (
            "Offline",
            theme::WARNING_ON_TINT,
            blank(),
            theme::FAINT,
            "The server can't be reached and no recent project list is saved here. Tracking starts as soon as it answers."
                .to_owned(),
        ),
        TrackerStatus::Stopped => (
            "Stopped",
            theme::DANGER_ON_TINT,
            blank(),
            theme::FAINT,
            "Nothing is recorded until this is fixed — see the message above.".to_owned(),
        ),
        TrackerStatus::SignedOut => (
            "Signed out",
            theme::MUTED_FG,
            blank(),
            theme::FAINT,
            String::new(),
        ),
        TrackerStatus::Away => {
            let since = snapshot.away_since.map(format::local_hm).unwrap_or_default();
            (
                "Away",
                theme::WARNING_ON_TINT,
                blank(),
                theme::FAINT,
                format!("No input since {since}. Tracking resumes when you are back."),
            )
        }
        TrackerStatus::Locked => (
            "Locked",
            theme::WARNING_ON_TINT,
            blank(),
            theme::FAINT,
            "Nothing is recorded while the screen is locked.".to_owned(),
        ),
        TrackerStatus::ChooseProject => (
            "Choose a project",
            theme::PRIMARY_STRONG,
            blank(),
            theme::FAINT,
            "Pick the project you are working on to start tracking.".to_owned(),
        ),
        TrackerStatus::SignInExpired => (
            "Sign-in expired",
            theme::DANGER_ON_TINT,
            blank(),
            theme::FAINT,
            "Enter your password to continue.".to_owned(),
        ),
        TrackerStatus::ConsentRequired => (
            "Consent required",
            theme::WARNING_ON_TINT,
            blank(),
            theme::FAINT,
            "Choose above what this computer may record before tracking can start.".to_owned(),
        ),
        TrackerStatus::NoSession => (
            "No desktop session",
            theme::WARNING_ON_TINT,
            blank(),
            theme::FAINT,
            "Tracking needs a graphical login on this computer.".to_owned(),
        ),
        TrackerStatus::DeviceRevoked => (
            "Device removed",
            theme::DANGER_ON_TINT,
            blank(),
            theme::FAINT,
            "Choose another workspace or sign out in Settings.".to_owned(),
        ),
    }
}

/// What the block in progress records besides time, by name.
pub fn measured_kinds(snapshot: &SharedState) -> Vec<&'static str> {
    let c = &snapshot.capabilities;
    [
        (&c.keyboard, "keyboard counts"),
        (&c.mouse, "mouse click counts"),
        (&c.active_window, "the active window"),
        (&c.running_programs, "running programs"),
        (&c.screenshot, "a screenshot"),
    ]
    .into_iter()
    .filter(|(info, _)| info.state == CapabilityState::On)
    .map(|(_, name)| name)
    .collect()
}

/// Statuses whose Timer card has the task note field (it applies to the
/// next block even while paused, away or offline).
pub fn shows_task_field(status: TrackerStatus) -> bool {
    !matches!(
        status,
        TrackerStatus::SignedOut
            | TrackerStatus::SignInExpired
            | TrackerStatus::Stopped
            | TrackerStatus::ChooseProject
            | TrackerStatus::ConsentRequired
            | TrackerStatus::DeviceRevoked
    )
}

/// One project in the chooser: a coloured dot, its name and a chevron, on a
/// row that lights up under the pointer.
fn project_row(ui: &mut egui::Ui, title: &str, enabled: bool) -> egui::Response {
    let sense = if enabled {
        Sense::click()
    } else {
        Sense::hover()
    };
    let (rect, response) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 42.0), sense);
    let response = if enabled {
        response.on_hover_cursor(egui::CursorIcon::PointingHand)
    } else {
        response
    };
    let hovered = enabled && response.hovered();
    let painter = ui.painter();
    painter.rect(
        rect,
        CornerRadius::same(theme::RADIUS),
        if hovered { theme::ACCENT } else { theme::CARD },
        Stroke::new(
            1.0,
            if hovered {
                theme::PRIMARY
            } else {
                theme::BORDER
            },
        ),
        egui::StrokeKind::Inside,
    );
    let dot = Pos2::new(rect.left() + 18.0, rect.center().y);
    painter.circle_filled(dot, 5.0, super::activity::name_color(title));
    let color = if hovered {
        theme::PRIMARY_STRONG
    } else {
        theme::INK
    };
    // A long name ends in "…" before the chevron.
    let mut job =
        egui::text::LayoutJob::simple_singleline(title.to_owned(), theme::semibold(13.5), color);
    job.wrap = egui::text::TextWrapping::truncate_at_width((rect.width() - 64.0).max(0.0));
    let galley = painter.layout_job(job);
    painter.galley(
        Pos2::new(rect.left() + 32.0, rect.center().y - galley.size().y / 2.0),
        galley,
        color,
    );
    widgets::paint_icon(
        painter,
        Rect::from_center_size(
            Pos2::new(rect.right() - 18.0, rect.center().y),
            Vec2::splat(14.0),
        ),
        Icon::ChevronRight,
        if hovered {
            theme::PRIMARY
        } else {
            theme::FAINT
        },
    );
    response
}

/// Green when mostly active, amber when mixed, red when mostly idle.
pub fn activity_color(percent: u8) -> Color32 {
    match percent {
        60.. => theme::SUCCESS,
        30.. => theme::WARNING,
        _ => theme::DANGER,
    }
}

/// [`activity_color`] for text: the darker on-tint variants stay readable at
/// small sizes.
pub fn activity_text_color(percent: u8) -> Color32 {
    match percent {
        60.. => theme::SUCCESS_ON_TINT,
        30.. => theme::WARNING_ON_TINT,
        _ => theme::DANGER_ON_TINT,
    }
}

/// "46% of 8h goal", or "Goal reached" once met.
pub fn goal_caption(secs: f64, goal: f64) -> String {
    if goal <= 0.0 {
        return String::new();
    }
    if secs >= goal {
        return "Goal reached".to_owned();
    }
    let percent = (secs / goal * 100.0).floor().max(0.0) as u32;
    format!("{percent}% of {} goal", widgets::hours_label(goal / 3600.0))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An egui context with the app's fonts (the "semibold" family must exist).
    fn context() -> egui::Context {
        let ctx = egui::Context::default();
        theme::apply(&ctx);
        ctx
    }

    /// One frame of the field with `events`, as the window would run it.
    fn frame(
        ctx: &egui::Context,
        field: &mut TaskField,
        saved: &str,
        events: Vec<egui::Event>,
    ) -> Option<String> {
        let mut applied = None;
        let input = egui::RawInput {
            events,
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(480.0, 320.0))),
            ..Default::default()
        };
        let _ = ctx.run_ui(input, |ui| applied = field.show(ui, saved));
        applied
    }

    fn key(key: egui::Key) -> egui::Event {
        egui::Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        }
    }

    fn click(pos: Pos2, pressed: bool) -> egui::Event {
        egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        }
    }

    /// The field with the keyboard (as the tray's "Set task…" leaves it).
    fn focused(ctx: &egui::Context, saved: &str) -> TaskField {
        let mut field = TaskField {
            focus_requested: true,
            ..TaskField::default()
        };
        assert_eq!(frame(ctx, &mut field, saved, Vec::new()), None);
        assert!(field.editing && field.draft == saved);
        field
    }

    #[test]
    fn enter_applies_a_changed_task_note() {
        let ctx = context();
        let mut field = focused(&ctx, "Invoices");
        assert_eq!(
            frame(
                &ctx,
                &mut field,
                "Invoices",
                vec![egui::Event::Text(" and QA".into())]
            ),
            None,
            "not on every keystroke"
        );
        assert_eq!(field.draft, "Invoices and QA");
        assert_eq!(
            frame(&ctx, &mut field, "Invoices", vec![key(egui::Key::Enter)]),
            Some("Invoices and QA".into())
        );
        assert!(!field.editing);
    }

    #[test]
    fn escape_puts_the_saved_note_back() {
        let ctx = context();
        let mut field = focused(&ctx, "Invoices");
        frame(
            &ctx,
            &mut field,
            "Invoices",
            vec![egui::Event::Text("!!".into())],
        );
        assert_eq!(
            frame(&ctx, &mut field, "Invoices", vec![key(egui::Key::Escape)]),
            None
        );
        assert_eq!(field.draft, "Invoices");
        assert!(!field.editing);
    }

    #[test]
    fn leaving_the_field_applies_it_too() {
        let ctx = context();
        let mut field = focused(&ctx, "");
        frame(
            &ctx,
            &mut field,
            "",
            vec![egui::Event::Text("Client call".into())],
        );
        let away = Pos2::new(240.0, 300.0);
        let applied = frame(
            &ctx,
            &mut field,
            "",
            vec![egui::Event::PointerMoved(away), click(away, true)],
        );
        let applied = applied.or_else(|| frame(&ctx, &mut field, "", vec![click(away, false)]));
        assert_eq!(applied.as_deref(), Some("Client call"));
    }

    #[test]
    fn an_unchanged_note_is_not_sent_again() {
        let ctx = context();
        let mut field = focused(&ctx, "Client call");
        // Only spacing differs: the worker would store the same note.
        frame(
            &ctx,
            &mut field,
            "Client call",
            vec![egui::Event::Text("  ".into())],
        );
        assert_eq!(
            frame(&ctx, &mut field, "Client call", vec![key(egui::Key::Enter)]),
            None
        );
    }

    #[test]
    fn the_field_follows_the_saved_note_until_someone_types() {
        let ctx = context();
        let mut field = TaskField::default();
        frame(&ctx, &mut field, "Restored after a restart", Vec::new());
        assert_eq!(field.draft, "Restored after a restart");
        frame(&ctx, &mut field, "Changed elsewhere", Vec::new());
        assert_eq!(field.draft, "Changed elsewhere");
    }
}
