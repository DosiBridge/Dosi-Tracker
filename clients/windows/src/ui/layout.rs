//! The signed-in page's frame — header, tabs, footer and the scrolling
//! content between them — and the layout pieces the views share.

use chrono::{DateTime, Utc};
use eframe::egui;
use egui::{Align, Color32, Layout, Pos2, Rect, Stroke, UiBuilder, Vec2};

use crate::state::{PendingAction, SharedState, TrackerStatus, UploadIssue};
use crate::theme;

use super::widgets::{self, Icon, Tone};
use super::{format, TrackerApp, View};

/// Content wider than this gets a two-column layout.
pub(super) const WIDE_LAYOUT: f32 = 760.0;
pub(super) const HEADER_H: f32 = 60.0;
pub(super) const TABS_H: f32 = 56.0;
pub(super) const FOOTER_H: f32 = 30.0;

impl TrackerApp {
    pub(super) fn signed_in_layout(&mut self, ui: &mut egui::Ui, snapshot: &SharedState, now: DateTime<Utc>) {
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

    pub(super) fn header(&self, ui: &mut egui::Ui, snapshot: &SharedState) {
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
    pub(super) fn tabs(&mut self, ui: &mut egui::Ui) {
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

    pub(super) fn footer(&self, ui: &mut egui::Ui, snapshot: &SharedState) {
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
pub(super) fn status_visual(status: TrackerStatus, pending: Option<PendingAction>) -> (Tone, &'static str) {
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
        TrackerStatus::ConsentRequired => (Tone::Warning, status.label()),
    }
}

/// The footer's one-line upload state: the most important thing first.
pub(super) fn sync_status(snapshot: &SharedState) -> (Tone, String) {
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

/// A card with a section header; the body fills the card's width.
pub(super) fn section<R>(ui: &mut egui::Ui, title: &str, trailing: Option<&str>, body: impl FnOnce(&mut egui::Ui) -> R) -> R {
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
pub(super) fn two_columns(ui: &mut egui::Ui, mut column: impl FnMut(usize, &mut egui::Ui)) {
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
pub(super) fn equal_row(ui: &mut egui::Ui, n: usize, gap: f32, mut cell: impl FnMut(usize, &mut egui::Ui)) {
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
pub(super) fn metric(ui: &mut egui::Ui, label: &str, value: &str, color: Color32) {
    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing.y = 1.0;
        ui.label(egui::RichText::new(label.to_uppercase()).font(theme::semibold(10.0)).color(theme::MUTED_FG));
        ui.label(egui::RichText::new(value).font(theme::semibold(15.0)).color(color));
    });
}
