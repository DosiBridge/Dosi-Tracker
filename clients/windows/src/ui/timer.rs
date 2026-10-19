//! The Timer view: what needs attention, the project (or choosing one), the
//! task note, the session clock with the start/pause control and timed
//! breaks, progress against the daily and weekly goals, and what the block
//! in progress holds so far. Also the question asked after time away, which
//! every tab shows.

use chrono::{DateTime, Utc};
use eframe::egui;
use egui::{Color32, CornerRadius, Pos2, Rect, Sense, Stroke, Vec2};

use super::alerts::{self, AlertAction};
use super::widgets::{self, ButtonKind, Callout, Icon};
use super::{equal_row, format, metric, section, two_columns, TrackerApp, View, PAUSE_CHOICES};
use crate::state::{Command, PendingAction, SharedState, TrackerStatus};
use crate::theme;
use crate::tracking::input::{active_minutes, bucket_of, minute_buckets};
use crate::worker::{clean_note, TASK_NOTE_MAX_CHARS};

/// With more projects than this, the chooser gets a filter field.
const FILTER_FROM: usize = 6;

/// The task note's counter shows once this close to the limit.
const COUNTER_WITHIN: usize = 40;

/// The "What are you working on?" field between frames.
#[derive(Debug, Default)]
pub(super) struct TaskField {
    /// What is typed; follows the saved note while the field is not in use.
    pub draft: String,
    /// The field has the keyboard.
    pub editing: bool,
    /// Give it the keyboard when next shown (the tray's "Set task…").
    pub focus_requested: bool,
}

impl TaskField {
    /// The field, under its header. `saved` is the note the worker keeps.
    /// Returns a note to apply: Enter, or leaving the field, with a change
    /// (compared the way the worker tidies notes). Esc puts the saved note
    /// back.
    pub fn show(&mut self, ui: &mut egui::Ui, saved: &str) -> Option<String> {
        if !self.editing && self.draft != saved {
            self.draft = saved.to_owned();
        }
        let used = self.draft.chars().count();
        let counter = (self.editing && used + COUNTER_WITHIN >= TASK_NOTE_MAX_CHARS).then(|| format!("{used}/{TASK_NOTE_MAX_CHARS}"));
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
            widgets::muted(ui, "Press Enter to apply — a new block starts with it. Esc cancels.");
        }
        apply
    }
}

impl TrackerApp {
    pub(super) fn timer_view(&mut self, ui: &mut egui::Ui, snapshot: &SharedState, now: DateTime<Utc>, wide: bool) {
        self.alerts(ui, snapshot);
        if !snapshot.consent.is_granted() {
            let mut action = None;
            super::consent::render_consent_card(ui, &mut self.consent_vm, |consent| {
                action = Some(consent);
            });
            if let Some(consent) = action {
                self.send(Command::UpdateConsent { consent });
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
                    ui.painter().circle_filled(rect.center(), 17.0, theme::tint(theme::WARNING));
                    widgets::paint_icon(ui.painter(), Rect::from_center_size(rect.center(), Vec2::splat(18.0)), Icon::Timer, theme::WARNING_ON_TINT);
                    ui.vertical(|ui| {
                        ui.set_width(ui.available_width());
                        ui.spacing_mut().item_spacing.y = 4.0;
                        let title = format!("You were away for {}.", format::away_summary(prompt.from, prompt.to));
                        ui.add(egui::Label::new(egui::RichText::new(title).font(theme::semibold(14.0)).color(theme::INK)).wrap());
                        widgets::muted(
                            ui,
                            &format!(
                                "Keep it as work on {}, recorded with no activity, or discard it. Nothing is recorded unless \
                                 you keep it.",
                                prompt.project_title
                            ),
                        );
                        ui.add_space(4.0);
                        equal_row(ui, 2, 8.0, |i, ui| {
                            let width = Some(ui.available_width());
                            if i == 0 {
                                if widgets::button(ui, ButtonKind::Primary, Some(Icon::Check), "Keep as work", width, true).clicked() {
                                    answer = Some(true);
                                }
                            } else if widgets::button(ui, ButtonKind::Secondary, None, "Discard", width, true).clicked() {
                                answer = Some(false);
                            }
                        });
                    });
                });
            });
        if let Some(keep) = answer {
            self.send(Command::ResolveAway { keep });
            // The worker confirms by publishing the prompt gone; until then
            // the card must not offer the choice twice.
            self.state.update(|s| s.away_prompt = None);
        }
    }

    /// Problems that need the user's attention, most serious first.
    fn alerts(&mut self, ui: &mut egui::Ui, snapshot: &SharedState) {
        for alert in alerts::alerts(snapshot, self.input_healthy()) {
            let content = Callout {
                title: (!alert.title.is_empty()).then_some(alert.title.as_str()),
                body: &alert.body,
                detail: alert.detail.as_deref(),
                action: alert.action.map(AlertAction::label),
            };
            if widgets::callout(ui, alert.tone, &content) {
                match alert.action {
                    Some(AlertAction::SyncNow) => self.send(Command::SyncNow { at: Utc::now() }),
                    Some(AlertAction::ShowActivity) => self.view = View::Activity,
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
            let trailing = (!projects.is_empty()).then(|| format::count(projects.len() as u32, "project"));
            widgets::section_header(ui, "Choose a project", trailing.as_deref());

            if projects.is_empty() {
                ui.add_space(6.0);
                ui.vertical_centered(|ui| {
                    ui.spacing_mut().item_spacing.y = 4.0;
                    widgets::icon(ui, Icon::Briefcase, 28.0, theme::FAINT);
                    ui.label(egui::RichText::new("You're not on any active project").font(theme::semibold(14.0)).color(theme::INK));
                    widgets::muted(
                        ui,
                        "Ask your workspace admin to add you to a project. Dosi Tracker checks again every few minutes \
                         and starts as soon as there is one.",
                    );
                });
                return;
            }

            widgets::muted(ui, "Pick what you're working on to start tracking. You can switch any time, here or from the tray menu.");
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
            let enabled = snapshot.pending_action.is_none();
            let mut shown = 0;
            for project in projects.iter().filter(|p| filter.is_empty() || p.title.to_lowercase().contains(&filter)) {
                shown += 1;
                if project_row(ui, &project.title, enabled).clicked() {
                    self.send(Command::SelectProject { project_id: project.id.clone(), at: Utc::now() });
                    self.project_filter.clear();
                }
            }
            if shown == 0 {
                widgets::muted(ui, &format!("No project matches \u{201c}{}\u{201d}.", self.project_filter.trim()));
            }
        });
    }

    /// Project picker, the task note, the big clock, and the start/pause
    /// control.
    fn session_card(&mut self, ui: &mut egui::Ui, snapshot: &SharedState, now: DateTime<Utc>) {
        theme::card(18).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 6.0;

            widgets::section_header(ui, "Project", Some(&format!("Saved every {} min", snapshot.interval_minutes)));
            self.project_picker(ui, snapshot);
            if shows_task_field(snapshot.status) {
                ui.add_space(8.0);
                self.task_field(ui, snapshot);
            }
            ui.add_space(14.0);

            let paused = snapshot.status == TrackerStatus::Paused;
            let (caption, caption_color, digits, digits_color, detail) = match snapshot.status {
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
                        Some(at) => format!("Resumes at {} · in {}", format::local_hm(at), format::clock(at - now)),
                        None => "Nothing is recorded until you resume.".to_owned(),
                    };
                    ("On a break", theme::WARNING_ON_TINT, format::clock(now - since), theme::WARNING_ON_TINT, detail)
                }
                TrackerStatus::Connecting => ("Connecting", theme::PRIMARY_STRONG, "--:--".to_owned(), theme::FAINT, "Connecting to your workspace…".to_owned()),
                TrackerStatus::Offline => (
                    "Offline",
                    theme::WARNING_ON_TINT,
                    "--:--".to_owned(),
                    theme::FAINT,
                    "The server can't be reached and no recent project list is saved here. Tracking starts as soon as it \
                     answers."
                        .to_owned(),
                ),
                TrackerStatus::Stopped => (
                    "Stopped",
                    theme::DANGER_ON_TINT,
                    "--:--".to_owned(),
                    theme::FAINT,
                    "Nothing is recorded until this is fixed — see the message above.".to_owned(),
                ),
                TrackerStatus::SignedOut => ("Signed out", theme::MUTED_FG, "--:--".to_owned(), theme::FAINT, String::new()),
                TrackerStatus::Away => {
                    let since = snapshot.away_since.map(format::local_hm).unwrap_or_default();
                    (
                        "Away",
                        theme::WARNING_ON_TINT,
                        "--:--".to_owned(),
                        theme::FAINT,
                        format!("No input since {since}. Tracking resumes when you are back."),
                    )
                }
                TrackerStatus::Locked => ("Locked", theme::WARNING_ON_TINT, "--:--".to_owned(), theme::FAINT, "Nothing is recorded while the screen is locked.".to_owned()),
                TrackerStatus::ChooseProject => (
                    "Choose a project",
                    theme::PRIMARY_STRONG,
                    "--:--".to_owned(),
                    theme::FAINT,
                    "Pick the project you are working on to start tracking.".to_owned(),
                ),
                TrackerStatus::SignInExpired => ("Sign-in expired", theme::DANGER_ON_TINT, "--:--".to_owned(), theme::FAINT, "Enter your password to continue.".to_owned()),
                TrackerStatus::ConsentRequired => (
                    "Consent required",
                    theme::WARNING_ON_TINT,
                    "--:--".to_owned(),
                    theme::FAINT,
                    "Explicit local consent is required before tracking can start. Choose your permissions above.".to_owned(),
                ),
            };

            ui.vertical_centered(|ui| {
                ui.spacing_mut().item_spacing.y = 2.0;
                ui.label(egui::RichText::new(caption.to_uppercase()).font(theme::semibold(11.0)).color(caption_color));
                ui.label(egui::RichText::new(digits).font(theme::semibold(46.0)).color(digits_color));
                ui.add(egui::Label::new(egui::RichText::new(detail).font(theme::regular(12.0)).color(theme::MUTED_FG)).wrap());
                ui.add_space(10.0);

                let pending = snapshot.pending_action;
                let idle = pending.is_none();
                let (fill, hover, icon, enabled, label) = if paused {
                    let label = if pending == Some(PendingAction::Resuming) { "Resuming…" } else { "Resume tracking" };
                    (theme::SUCCESS, theme::SUCCESS_HOVER, Icon::Play, idle, label)
                } else {
                    let can = idle && format::can_pause(snapshot.status);
                    let label = match pending {
                        Some(PendingAction::Pausing) => "Pausing…",
                        _ if can => "Pause tracking",
                        _ => "Tracking is not running",
                    };
                    (theme::PRIMARY, theme::PRIMARY_HOVER, Icon::Pause, can, label)
                };
                let response = widgets::round_button(ui, 76.0, fill, hover, icon, enabled);
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
                ui.label(egui::RichText::new(label).font(theme::semibold(12.5)).color(theme::INK_SOFT));
            });

            // Timed breaks. While paused the same choices re-arm the
            // auto-resume from now, and "Never" turns it off.
            if paused || format::can_pause(snapshot.status) {
                ui.add_space(10.0);
                let idle = snapshot.pending_action.is_none();
                let show_never = paused && snapshot.resume_at.is_some();
                let caption = if paused { "Resume after" } else { "Take a break" };
                let mut labels: Vec<&str> = PAUSE_CHOICES.iter().map(|(label, _)| *label).collect();
                if show_never {
                    labels.push("Never");
                }
                let caption_w = ui.painter().layout_no_wrap(caption.to_owned(), theme::regular(12.0), Color32::PLACEHOLDER).size().x;
                let chips_w: f32 = labels
                    .iter()
                    .map(|l| ui.painter().layout_no_wrap((*l).to_owned(), theme::semibold(12.0), Color32::PLACEHOLDER).size().x + 24.0)
                    .sum();
                let total = caption_w + chips_w + 6.0 * labels.len() as f32 + 4.0;
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 6.0;
                    ui.add_space(((ui.available_width() - total) / 2.0).max(0.0));
                    ui.label(egui::RichText::new(caption).font(theme::regular(12.0)).color(theme::MUTED_FG));
                    ui.add_space(4.0);
                    for (label, duration) in PAUSE_CHOICES {
                        let hint = if paused {
                            format!("Stay on a break, then resume automatically in {label}")
                        } else {
                            format!("Pause now and resume automatically in {label}")
                        };
                        if widgets::chip(ui, label, false, idle).on_hover_text(hint).clicked() {
                            self.pause(snapshot, Some(duration));
                        }
                    }
                    if show_never && widgets::chip(ui, "Never", false, idle).on_hover_text("Stay paused until you resume").clicked() {
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
        let enabled = snapshot.pending_action.is_none() && !snapshot.projects.is_empty();
        ui.add_enabled_ui(enabled, |ui| {
            egui::ComboBox::from_id_salt("project-picker")
                .width(ui.available_width())
                .height(260.0)
                .selected_text(egui::RichText::new(current).font(theme::semibold(14.0)).color(theme::INK))
                .show_ui(ui, |ui| {
                    ui.set_min_width(220.0);
                    for project in &snapshot.projects {
                        let selected = snapshot.selected_project.as_deref() == Some(&project.id);
                        let text = egui::RichText::new(&project.title).font(theme::regular(13.5));
                        if ui.selectable_label(selected, text).clicked() && !selected {
                            self.send(Command::SelectProject { project_id: project.id.clone(), at: Utc::now() });
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
    }

    /// "What are you working on?" — sent as each block's description. A
    /// new note ends the block in progress; the next starts with it.
    fn task_field(&mut self, ui: &mut egui::Ui, snapshot: &SharedState) {
        let Some(note) = self.task.show(ui, &snapshot.task_note) else {
            return;
        };
        self.send(Command::SetTaskNote { note: note.clone(), at: Utc::now() });
        // Shown at once; the worker publishes the same tidy note.
        let shown = clean_note(&note).unwrap_or_default();
        self.task.draft = shown.clone();
        self.state.update(|s| s.task_note = shown);
    }

    /// Today and this week against the goals, and today's activity level.
    fn goal_tiles(&mut self, ui: &mut egui::Ui, snapshot: &SharedState, now: DateTime<Utc>) {
        let (today, week) = format::live_totals(snapshot, now);
        let daily_goal = self.prefs.daily_goal_secs();
        let weekly_goal = self.prefs.weekly_goal_secs();
        let activity = snapshot.last_productivity;
        let activity_color = activity_color(activity);
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
            ("Activity", format!("{activity}%"), f32::from(activity) / 100.0, activity_color, "Today's average".to_owned()),
        ];
        equal_row(ui, tiles.len(), theme::GAP - 2.0, |i, ui| {
            let (label, value, fraction, color, caption) = &tiles[i];
            theme::card(14).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.spacing_mut().item_spacing.y = 4.0;
                ui.label(egui::RichText::new(label.to_uppercase()).font(theme::semibold(10.0)).color(theme::MUTED_FG));
                ui.label(egui::RichText::new(value.as_str()).font(theme::semibold(19.0)).color(theme::INK));
                ui.add_space(2.0);
                widgets::progress_bar(ui, *fraction, 6.0, *color);
                ui.add(egui::Label::new(egui::RichText::new(caption.as_str()).font(theme::regular(11.0)).color(theme::MUTED_FG)).truncate());
            });
        });
    }

    /// The block in progress: when it is saved, per-minute input, and
    /// counts — or what a pause means.
    fn block_card(&mut self, ui: &mut egui::Ui, snapshot: &SharedState, now: DateTime<Utc>) {
        match snapshot.status {
            TrackerStatus::Tracking => {
                let block = snapshot.block_started_at.unwrap_or(now);
                // When the block is saved — never the screenshot moment,
                // which is random within the block and not announced.
                let next = snapshot.next_capture_at.unwrap_or(now);
                let trailing = format!("Saved in {}", format::clock(next - now));
                section(ui, "Current block", Some(&trailing), |ui| {
                    let interval = (snapshot.interval_minutes.max(1) * 60) as f32;
                    let elapsed = (now - block).num_seconds().max(0) as f32;
                    widgets::progress_bar(ui, elapsed / interval, 6.0, theme::PRIMARY);
                    ui.add_space(4.0);

                    let counts_input = snapshot.capture.keyboard || snapshot.capture.mouse;
                    if counts_input {
                        let minutes = self.block_minutes(snapshot, block, now);
                        widgets::minute_bars(ui, &minutes, snapshot.interval_minutes as usize, 38.0);
                        ui.add_space(4.0);
                    }

                    let (keys, clicks) = self.input.counts_between(block, now);
                    let percent = self
                        .input
                        .activity_percent(block, now, snapshot.capture.keyboard, snapshot.capture.mouse);
                    let mut metrics: Vec<(&str, String, Color32)> = Vec::new();
                    if counts_input {
                        if self.input_healthy() {
                            metrics.push(("Active", format!("{percent}%"), activity_text_color(percent)));
                        } else {
                            metrics.push(("Active", "—".to_owned(), theme::MUTED_FG));
                        }
                    }
                    if snapshot.capture.keyboard {
                        metrics.push(("Keys", keys.to_string(), theme::INK));
                    }
                    if snapshot.capture.mouse {
                        metrics.push(("Clicks", clicks.to_string(), theme::INK));
                    }
                    metrics.push(("Since", format::local_hm(block), theme::INK));
                    equal_row(ui, metrics.len(), 8.0, |i, ui| {
                        let (label, value, color) = &metrics[i];
                        metric(ui, label, value, *color);
                    });

                    if snapshot.capture.active_window {
                        if let Some(window) = crate::tracking::active_window::current() {
                            ui.add_space(2.0);
                            ui.separator();
                            ui.horizontal(|ui| {
                                ui.spacing_mut().item_spacing.x = 6.0;
                                widgets::icon(ui, Icon::Window, 14.0, theme::MUTED_FG);
                                ui.label(egui::RichText::new("Now").font(theme::semibold(11.5)).color(theme::MUTED_FG));
                                let app = format::app_display_name(&window.app_name);
                                let text = if window.window_title.trim().is_empty() {
                                    app
                                } else {
                                    format!("{app} — {}", window.window_title.trim())
                                };
                                ui.add(egui::Label::new(egui::RichText::new(text).font(theme::regular(12.0)).color(theme::INK_SOFT)).truncate());
                            });
                        }
                    }
                });
            }
            TrackerStatus::Paused => {
                section(ui, "On a break", None, |ui| {
                    widgets::muted(
                        ui,
                        "The block you were working on was saved when you paused. Nothing — no screenshots, \
                         input or apps — is recorded until you resume.",
                    );
                });
            }
            TrackerStatus::Away => {
                section(ui, "Away from the computer", None, |ui| {
                    widgets::muted(
                        ui,
                        "The block ended at your last keyboard or mouse input. When you are back, a new block starts \
                         and you choose whether to keep the time away as work.",
                    );
                });
            }
            _ => {}
        }
    }

    /// Per-minute (active?, input count) for the block so far, counting only
    /// the input kinds the project records.
    fn block_minutes(&self, snapshot: &SharedState, block: DateTime<Utc>, now: DateTime<Utc>) -> Vec<(bool, u64)> {
        let (start, end) = (block.timestamp(), now.timestamp());
        let minutes = minute_buckets(start, end);
        if minutes == 0 {
            return Vec::new();
        }
        let (keyboard, mouse) = (snapshot.capture.keyboard, snapshot.capture.mouse);
        let active = active_minutes(&self.input.active_seconds(block, now, keyboard, mouse), start, end);
        let mut counts = vec![0u64; minutes as usize];
        let mut add = |stamps: Vec<i64>| {
            for ms in stamps {
                counts[bucket_of(ms / 1000, start, minutes)] += 1;
            }
        };
        if keyboard {
            add(self.input.key_presses_between(block, now));
        }
        if mouse {
            add(self.input.clicks_between(block, now));
        }
        active.into_iter().zip(counts).collect()
    }

    /// Sync and the web dashboard.
    fn quick_actions(&mut self, ui: &mut egui::Ui, snapshot: &SharedState) {
        let paused = snapshot.status == TrackerStatus::Paused;
        // While paused the button never captures — it only uploads what is
        // already queued — so it says so rather than promising a sync.
        let (label, hint) = if paused {
            ("Upload queued", "Paused: uploads activity already recorded. Nothing new is captured.")
        } else {
            ("Sync now", "Save the current block now and upload it.")
        };
        let dashboard_ready = self.dashboard_available(snapshot);
        let mut sync = false;
        let mut dashboard = false;
        equal_row(ui, 2, 8.0, |i, ui| {
            let width = Some(ui.available_width());
            if i == 0 {
                let enabled = !matches!(snapshot.status, TrackerStatus::Stopped | TrackerStatus::SignInExpired);
                sync = widgets::button(ui, ButtonKind::Secondary, Some(Icon::Sync), label, width, enabled)
                    .on_hover_text(hint)
                    .clicked();
            } else {
                let response = widgets::button(ui, ButtonKind::Secondary, Some(Icon::External), "Web dashboard", width, dashboard_ready);
                dashboard = if dashboard_ready {
                    response.on_hover_text("Open your timesheets and reports in the browser.").clicked()
                } else {
                    response.on_hover_text(super::DASHBOARD_UNSET).clicked()
                };
            }
        });
        if sync {
            self.send(Command::SyncNow { at: Utc::now() });
        }
        if dashboard {
            self.open_dashboard(snapshot);
        }
    }
}

/// Statuses whose Timer card has the task note field (it applies to the
/// next block even while paused, away or offline).
fn shows_task_field(status: TrackerStatus) -> bool {
    !matches!(
        status,
        TrackerStatus::SignedOut | TrackerStatus::SignInExpired | TrackerStatus::Stopped | TrackerStatus::ChooseProject | TrackerStatus::ConsentRequired
    )
}

/// One project in the chooser: a coloured dot, its name and a chevron, on a
/// row that lights up under the pointer.
fn project_row(ui: &mut egui::Ui, title: &str, enabled: bool) -> egui::Response {
    let sense = if enabled { Sense::click() } else { Sense::hover() };
    let (rect, response) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 42.0), sense);
    let response = if enabled { response.on_hover_cursor(egui::CursorIcon::PointingHand) } else { response };
    let hovered = enabled && response.hovered();
    let painter = ui.painter();
    painter.rect(
        rect,
        CornerRadius::same(theme::RADIUS),
        if hovered { theme::ACCENT } else { theme::CARD },
        Stroke::new(1.0, if hovered { theme::PRIMARY } else { theme::BORDER }),
        egui::StrokeKind::Inside,
    );
    let dot = Pos2::new(rect.left() + 18.0, rect.center().y);
    painter.circle_filled(dot, 5.0, super::activity::name_color(title));
    let color = if hovered { theme::PRIMARY_STRONG } else { theme::INK };
    // A long name ends in "…" before the chevron.
    let mut job = egui::text::LayoutJob::simple_singleline(title.to_owned(), theme::semibold(13.5), color);
    job.wrap = egui::text::TextWrapping::truncate_at_width((rect.width() - 64.0).max(0.0));
    let galley = painter.layout_job(job);
    painter.galley(Pos2::new(rect.left() + 32.0, rect.center().y - galley.size().y / 2.0), galley, color);
    widgets::paint_icon(
        painter,
        Rect::from_center_size(Pos2::new(rect.right() - 18.0, rect.center().y), Vec2::splat(14.0)),
        Icon::ChevronRight,
        if hovered { theme::PRIMARY } else { theme::FAINT },
    );
    response
}

/// Green when mostly active, amber when mixed, red when mostly idle.
pub(super) fn activity_color(percent: u8) -> Color32 {
    match percent {
        60.. => theme::SUCCESS,
        30.. => theme::WARNING,
        _ => theme::DANGER,
    }
}

/// [`activity_color`] for text: the darker on-tint variants stay readable at
/// small sizes.
pub(super) fn activity_text_color(percent: u8) -> Color32 {
    match percent {
        60.. => theme::SUCCESS_ON_TINT,
        30.. => theme::WARNING_ON_TINT,
        _ => theme::DANGER_ON_TINT,
    }
}

/// "46% of 8h goal", or "Goal reached" once met.
fn goal_caption(secs: f64, goal: f64) -> String {
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

    #[test]
    fn goal_captions() {
        assert_eq!(goal_caption(0.0, 8.0 * 3600.0), "0% of 8h goal");
        assert_eq!(goal_caption(3.7 * 3600.0, 8.0 * 3600.0), "46% of 8h goal");
        assert_eq!(goal_caption(9.0 * 3600.0, 8.0 * 3600.0), "Goal reached");
        assert_eq!(goal_caption(1.0, 37.5 * 3600.0), "0% of 37h 30m goal");
        assert_eq!(goal_caption(1.0, 0.0), "");
    }

    #[test]
    fn activity_colors_follow_the_level() {
        assert_eq!(activity_color(85), theme::SUCCESS);
        assert_eq!(activity_color(45), theme::WARNING);
        assert_eq!(activity_color(10), theme::DANGER);
    }

    /// An egui context with the app's fonts (the "semibold" family must exist).
    fn context() -> egui::Context {
        let ctx = egui::Context::default();
        theme::apply(&ctx);
        ctx
    }

    /// One frame of the field with `events`, as the window would run it.
    fn frame(ctx: &egui::Context, field: &mut TaskField, saved: &str, events: Vec<egui::Event>) -> Option<String> {
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
        egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::NONE }
    }

    fn click(pos: Pos2, pressed: bool) -> egui::Event {
        egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE }
    }

    /// The field with the keyboard (as the tray's "Set task…" leaves it).
    fn focused(ctx: &egui::Context, saved: &str) -> TaskField {
        let mut field = TaskField { focus_requested: true, ..TaskField::default() };
        assert_eq!(frame(ctx, &mut field, saved, Vec::new()), None);
        assert!(field.editing && field.draft == saved);
        field
    }

    #[test]
    fn enter_applies_a_changed_task_note() {
        let ctx = context();
        let mut field = focused(&ctx, "Invoices");
        assert_eq!(frame(&ctx, &mut field, "Invoices", vec![egui::Event::Text(" and QA".into())]), None, "not on every keystroke");
        assert_eq!(field.draft, "Invoices and QA");
        assert_eq!(frame(&ctx, &mut field, "Invoices", vec![key(egui::Key::Enter)]), Some("Invoices and QA".into()));
        assert!(!field.editing);
    }

    #[test]
    fn escape_puts_the_saved_note_back() {
        let ctx = context();
        let mut field = focused(&ctx, "Invoices");
        frame(&ctx, &mut field, "Invoices", vec![egui::Event::Text("!!".into())]);
        assert_eq!(frame(&ctx, &mut field, "Invoices", vec![key(egui::Key::Escape)]), None);
        assert_eq!(field.draft, "Invoices");
        assert!(!field.editing);
    }

    #[test]
    fn leaving_the_field_applies_it_too() {
        let ctx = context();
        let mut field = focused(&ctx, "");
        frame(&ctx, &mut field, "", vec![egui::Event::Text("Client call".into())]);
        let away = Pos2::new(240.0, 300.0);
        let applied = frame(&ctx, &mut field, "", vec![egui::Event::PointerMoved(away), click(away, true)]);
        let applied = applied.or_else(|| frame(&ctx, &mut field, "", vec![click(away, false)]));
        assert_eq!(applied.as_deref(), Some("Client call"));
    }

    #[test]
    fn an_unchanged_note_is_not_sent_again() {
        let ctx = context();
        let mut field = focused(&ctx, "Client call");
        // Only spacing differs: the worker would store the same note.
        frame(&ctx, &mut field, "Client call", vec![egui::Event::Text("  ".into())]);
        assert_eq!(frame(&ctx, &mut field, "Client call", vec![key(egui::Key::Enter)]), None);
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

    #[test]
    fn the_task_note_field_shows_wherever_a_next_block_can_follow() {
        for status in [TrackerStatus::Tracking, TrackerStatus::Paused, TrackerStatus::Away, TrackerStatus::Offline, TrackerStatus::Locked] {
            assert!(shows_task_field(status), "{status:?}");
        }
        for status in [TrackerStatus::SignedOut, TrackerStatus::SignInExpired, TrackerStatus::Stopped, TrackerStatus::ChooseProject, TrackerStatus::ConsentRequired] {
            assert!(!shows_task_field(status), "{status:?}");
        }
    }
}
