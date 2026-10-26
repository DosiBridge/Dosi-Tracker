//! Activity screen: week chart, latest screenshot exactly as queued, recent
//! blocks with their upload state.
//!
//! Ported from the Windows agent's `ui/activity.rs`. The latest screenshot
//! is fetched from the daemon (`screenshot.latest`) when a new one is taken:
//! it is the thumbnail exactly as it was queued, so people see what their
//! workspace sees. The blocks of earlier sessions still on this computer are
//! listed with the server's reason when it refused one, and waiting blocks
//! can be purged (confirmed first). Apps used in the last hour: v1.1.

use std::collections::HashSet;

use chrono::{DateTime, Datelike, Local, Utc};
use eframe::egui;
use egui::{Align, Color32, CornerRadius, Layout, Pos2, Rect, Sense, Vec2};
use serde_json::json;

use super::timer::{activity_color, activity_text_color};
use super::widgets::{self, ButtonKind, DayBar, Icon, Tone};
use super::{format, section, theme, two_columns, TrackerApp};
use crate::agent::state::{QueuedRow, RowStatus, SharedState, UploadState};
use crate::ipc::method;

const WEEKDAYS: [&str; 7] = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];

/// How long rejected blocks are kept on this computer (the worker's purge).
pub const REJECTED_KEPT_DAYS: u32 = 14;

/// The latest screenshot as the window shows it.
pub enum Shot {
    /// Asked for the one taken at this time.
    Loading(DateTime<Utc>),
    Shown(DateTime<Utc>, egui::TextureHandle),
    /// It could not be fetched or decoded.
    Failed(DateTime<Utc>),
}

impl Shot {
    pub fn taken_at(&self) -> DateTime<Utc> {
        match self {
            Shot::Loading(at) | Shot::Shown(at, _) | Shot::Failed(at) => *at,
        }
    }
}

impl TrackerApp {
    pub(super) fn activity_view(
        &mut self,
        ui: &mut egui::Ui,
        snapshot: &SharedState,
        now: DateTime<Utc>,
        wide: bool,
    ) {
        if wide {
            two_columns(ui, |column, ui| {
                if column == 0 {
                    self.week_card(ui, snapshot, now);
                    self.recent_card(ui, snapshot);
                } else {
                    self.screenshot_card(ui, snapshot);
                    self.earlier_card(ui, snapshot, now);
                }
            });
        } else {
            self.week_card(ui, snapshot, now);
            self.screenshot_card(ui, snapshot);
            self.recent_card(ui, snapshot);
            self.earlier_card(ui, snapshot, now);
        }
    }

    fn week_card(&mut self, ui: &mut egui::Ui, snapshot: &SharedState, now: DateTime<Utc>) {
        let (today_secs, week_secs) = format::live_totals(snapshot, now);
        let today = now.with_timezone(&Local).weekday().num_days_from_monday() as usize;
        let hours = week_hours(&snapshot.week_day_secs, today, today_secs);
        let bars: Vec<DayBar<'_>> = (0..7)
            .map(|day| DayBar {
                label: WEEKDAYS[day],
                hours: hours[day],
                today: day == today,
                future: day > today,
            })
            .collect();

        let goal = self.weekly_goal_secs();
        let trailing = format!(
            "{} of {}",
            format::secs_label(week_secs),
            widgets::hours_label(goal / 3600.0)
        );
        let daily_goal_hours = self.daily_goal_secs() / 3600.0;
        section(ui, "This week", Some(&trailing), |ui| {
            widgets::week_chart(ui, &bars, daily_goal_hours, 168.0);

            let worked: Vec<(usize, f64)> = hours[..=today]
                .iter()
                .copied()
                .enumerate()
                .filter(|(_, h)| *h > 0.0)
                .collect();
            let average = if worked.is_empty() {
                0.0
            } else {
                worked.iter().map(|(_, h)| h).sum::<f64>() / worked.len() as f64
            };
            let best = worked.iter().copied().max_by(|a, b| a.1.total_cmp(&b.1));
            ui.add_space(2.0);
            super::equal_row(ui, 3, 8.0, |i, ui| match i {
                0 => super::metric(
                    ui,
                    "Days worked",
                    &format!("{} of {}", worked.len(), today + 1),
                    theme::INK,
                ),
                1 => super::metric(
                    ui,
                    "Daily average",
                    &widgets::hours_label(average),
                    theme::INK,
                ),
                _ => super::metric(
                    ui,
                    "Best day",
                    &best.map_or("—".to_owned(), |(day, h)| {
                        format!("{} · {}", WEEKDAYS[day], widgets::hours_label(h))
                    }),
                    theme::INK,
                ),
            });
        });
    }

    fn screenshot_card(&mut self, ui: &mut egui::Ui, snapshot: &SharedState) {
        if !snapshot.capture.screenshot {
            return;
        }
        let taken = snapshot.last_screenshot.as_ref().map(|shot| shot.taken_at);
        let trailing = taken.map(|at| format!("Taken at {}", format::local_hm(at)));
        // A new screenshot: fetch its thumbnail once.
        if let Some(at) = taken {
            if self.shot.as_ref().map(Shot::taken_at) != Some(at) && self.writable() {
                self.shot = Some(Shot::Loading(at));
                self.link.call(method::SCREENSHOT_LATEST, json!({}));
            }
        }
        section(ui, "Latest screenshot", trailing.as_deref(), |ui| {
            match (taken, &self.shot) {
                (None, _) => {
                    // The moment of the next one is random and not announced.
                    widgets::muted(
                        ui,
                        "No screenshot yet this session. One is taken at a random moment in each block.",
                    );
                }
                (Some(_), Some(Shot::Shown(_, texture))) => {
                    let size = texture.size_vec2();
                    let width = ui.available_width();
                    let height = (width * size.y / size.x.max(1.0)).min(320.0);
                    let width = height * size.x / size.y.max(1.0);
                    let image = egui::Image::from_texture(egui::load::SizedTexture::new(
                        texture.id(),
                        Vec2::new(width, height),
                    ))
                    .corner_radius(CornerRadius::same(8));
                    ui.vertical_centered(|ui| {
                        let response = ui.add(image);
                        ui.painter().rect_stroke(
                            response.rect,
                            CornerRadius::same(8),
                            egui::Stroke::new(1.0, theme::BORDER),
                            egui::StrokeKind::Inside,
                        );
                    });
                    let note = if snapshot.capture.blur_screenshots {
                        "This is what your workspace sees: screenshots are blurred on this computer before upload, so no text can be read. One is taken at a random moment in each block."
                    } else {
                        "This is what your workspace sees. One screenshot is taken at a random moment in each block, of the screen you're working on."
                    };
                    widgets::muted(ui, note);
                }
                (Some(_), Some(Shot::Failed(_))) => {
                    widgets::muted(ui, "The latest screenshot could not be shown.");
                }
                (Some(_), _) => {
                    widgets::muted(ui, "Loading the latest screenshot…");
                }
            }
        });
    }

    /// The answer to `screenshot.latest`.
    pub(super) fn on_screenshot(&mut self, ctx: &egui::Context, result: Option<serde_json::Value>) {
        let Some(Shot::Loading(at)) = &self.shot else {
            return;
        };
        let at = *at;
        let decoded = result
            .as_ref()
            .and_then(|value| value.get("jpeg_base64"))
            .and_then(serde_json::Value::as_str)
            .and_then(|b64| {
                use base64::Engine as _;
                base64::engine::general_purpose::STANDARD.decode(b64).ok()
            })
            .and_then(|bytes| decode_jpeg(&bytes));
        self.shot = Some(match decoded {
            Some(image) => Shot::Shown(
                at,
                ctx.load_texture("latest-screenshot", image, egui::TextureOptions::LINEAR),
            ),
            None => Shot::Failed(at),
        });
    }

    fn recent_card(&mut self, ui: &mut egui::Ui, snapshot: &SharedState) {
        let trailing = (!snapshot.recent_blocks.is_empty())
            .then(|| format::count(snapshot.recent_blocks.len() as u32, "block"));
        section(ui, "Recorded this session", trailing.as_deref(), |ui| {
            if snapshot.recent_blocks.is_empty() {
                widgets::muted(
                    ui,
                    &format!(
                        "A block is saved every {} minutes, and when you pause, switch project or change the task.",
                        snapshot.interval_minutes
                    ),
                );
                return;
            }
            let mut any_rejected = false;
            for (i, block) in snapshot.recent_blocks.iter().enumerate() {
                if i > 0 {
                    row_divider(ui);
                }
                let queued = snapshot
                    .queue_rows
                    .iter()
                    .find(|row| row.id == block.row_id);
                let note = block_note(block.upload, queued.and_then(|row| row.reason.as_deref()));
                any_rejected |= block.upload == UploadState::Rejected;
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 8.0;
                    upload_icon(ui, block.upload);
                    ui.vertical(|ui| {
                        ui.spacing_mut().item_spacing.y = 0.0;
                        let span = format!(
                            "{} – {}  ·  {}",
                            format::local_hm(block.started_at),
                            format::local_hm(block.ended_at),
                            format::minutes_label(
                                (block.ended_at - block.started_at).num_minutes().max(0) as u64
                            )
                        );
                        ui.label(
                            egui::RichText::new(span)
                                .font(theme::semibold(12.5))
                                .color(theme::INK),
                        );
                        let mut detail = block.project.clone();
                        if block.screenshot {
                            detail.push_str(" · screenshot");
                        }
                        detail.push_str(match block.upload {
                            UploadState::Queued => " · waiting to upload",
                            UploadState::Uploaded => " · uploaded",
                            UploadState::Rejected => " · not accepted by the server",
                        });
                        ui.add(
                            egui::Label::new(
                                egui::RichText::new(detail)
                                    .font(theme::regular(11.5))
                                    .color(theme::MUTED_FG),
                            )
                            .truncate(),
                        );
                    });
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        percent_pill(ui, block.productivity);
                    });
                });
                if let Some((text, color)) = note {
                    reason_line(ui, &text, color);
                }
            }
            if any_rejected {
                ui.add_space(2.0);
                widgets::muted(
                    ui,
                    &format!(
                        "Blocks the server did not accept stay on this computer for {REJECTED_KEPT_DAYS} days for reference, then are deleted."
                    ),
                );
            }
        });
    }

    /// Blocks of earlier sessions still on this computer: waiting to upload,
    /// or not accepted (with the server's reason) — and purging the waiting
    /// ones, confirmed first.
    fn earlier_card(&mut self, ui: &mut egui::Ui, snapshot: &SharedState, now: DateTime<Utc>) {
        let rows = earlier_rows(snapshot);
        if rows.is_empty() && snapshot.pending_uploads == 0 {
            return;
        }
        let waiting = rows
            .iter()
            .filter(|row| row.status == RowStatus::Queued)
            .count() as u32;
        let rejected = rows.len() as u32 - waiting;
        let mut summary = Vec::new();
        if waiting > 0 {
            summary.push(format!("{waiting} waiting"));
        }
        if rejected > 0 {
            summary.push(format!("{rejected} not accepted"));
        }
        let trailing = summary.join(" · ");
        section(ui, "From earlier sessions", Some(&trailing), |ui| {
            for (i, row) in rows.iter().enumerate() {
                if i > 0 {
                    row_divider(ui);
                }
                let upload = if row.status == RowStatus::Parked {
                    UploadState::Rejected
                } else {
                    UploadState::Queued
                };
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 8.0;
                    upload_icon(ui, upload);
                    ui.vertical(|ui| {
                        ui.spacing_mut().item_spacing.y = 0.0;
                        ui.label(
                            egui::RichText::new(row_span(row, now))
                                .font(theme::semibold(12.5))
                                .color(theme::INK),
                        );
                        let mut detail = project_title(snapshot, row.project_id.as_deref());
                        if row.has_captures {
                            detail.push_str(" · with screenshot");
                        }
                        ui.add(
                            egui::Label::new(
                                egui::RichText::new(detail)
                                    .font(theme::regular(11.5))
                                    .color(theme::MUTED_FG),
                            )
                            .truncate(),
                        );
                    });
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if upload == UploadState::Rejected {
                            widgets::status_pill(ui, Tone::Danger, "Not accepted");
                        } else {
                            widgets::status_pill(ui, Tone::Warning, "Waiting");
                        }
                    });
                });
                if let Some((text, color)) = block_note(upload, row.reason.as_deref()) {
                    reason_line(ui, &text, color);
                }
            }
            if snapshot.pending_uploads > 0 {
                ui.add_space(4.0);
                self.purge_waiting(ui, snapshot);
            }
        });
    }

    /// "Purge waiting blocks": deletes this account's unsent blocks for good,
    /// after a confirmation.
    fn purge_waiting(&mut self, ui: &mut egui::Ui, snapshot: &SharedState) {
        if !self.confirm_purge {
            if widgets::button(
                ui,
                ButtonKind::Danger,
                None,
                "Purge waiting blocks",
                Some(ui.available_width()),
                self.writable(),
            )
            .on_hover_text("Delete the blocks waiting to upload from this computer.")
            .clicked()
            {
                self.confirm_purge = true;
            }
            return;
        }
        theme::inset(12).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(
                egui::RichText::new(format!(
                    "Delete {}?",
                    format::count(snapshot.pending_uploads, "waiting block")
                ))
                .font(theme::semibold(13.0))
                .color(theme::INK),
            );
            widgets::muted(
                ui,
                "They are removed from this computer and never uploaded. This cannot be undone.",
            );
            let mut cancel = false;
            let mut confirm = false;
            super::equal_row(ui, 2, 8.0, |i, ui| {
                let width = Some(ui.available_width());
                if i == 0 {
                    cancel =
                        widgets::button(ui, ButtonKind::Secondary, None, "Cancel", width, true)
                            .clicked();
                } else {
                    confirm =
                        widgets::button(ui, ButtonKind::Danger, None, "Delete them", width, true)
                            .clicked();
                }
            });
            if cancel {
                self.confirm_purge = false;
            }
            if confirm {
                self.confirm_purge = false;
                self.link.call(
                    method::QUEUE_PURGE,
                    json!({ "which": "pending", "confirmed": true }),
                );
            }
        });
    }
}

/// Hours per weekday of this week, Monday first: past days from the server,
/// today live, the rest nothing yet.
pub fn week_hours(week_day_secs: &[f64; 7], today: usize, today_secs: f64) -> Vec<f64> {
    (0..7)
        .map(|day| match day.cmp(&today) {
            std::cmp::Ordering::Less => week_day_secs[day] / 3600.0,
            std::cmp::Ordering::Equal => today_secs / 3600.0,
            std::cmp::Ordering::Greater => 0.0,
        })
        .collect()
}

/// What to say under a block about its upload: the server's reason for
/// refusing it, or why the last attempt to send it failed.
pub fn block_note(upload: UploadState, reason: Option<&str>) -> Option<(String, Color32)> {
    let reason = reason.map(str::trim).filter(|r| !r.is_empty());
    match (upload, reason) {
        (UploadState::Rejected, Some(reason)) => {
            Some((format!("Server: {reason}"), theme::DANGER_ON_TINT))
        }
        (UploadState::Rejected, None) => Some((
            "The server did not say why.".to_owned(),
            theme::DANGER_ON_TINT,
        )),
        (UploadState::Queued, Some(reason)) => {
            Some((format!("Last try failed: {reason}"), theme::MUTED_FG))
        }
        _ => None,
    }
}

/// The queue's rows that are not blocks of this session (those are listed
/// with their session), oldest last as the queue lists them.
pub fn earlier_rows(snapshot: &SharedState) -> Vec<&QueuedRow> {
    let this_session: HashSet<i64> = snapshot.recent_blocks.iter().map(|b| b.row_id).collect();
    snapshot
        .queue_rows
        .iter()
        .filter(|row| !this_session.contains(&row.id))
        .collect()
}

/// "Mon 14:05 – 14:15 · 10m" (the day when it is not today).
pub fn row_span(row: &QueuedRow, now: DateTime<Utc>) -> String {
    match (row.started_at, row.ended_at) {
        (Some(start), Some(end)) => format!(
            "{} – {}  ·  {}",
            format::day_and_time(start, now),
            format::local_hm(end),
            format::minutes_label((end - start).num_minutes().max(0) as u64)
        ),
        (Some(start), None) => format::day_and_time(start, now),
        _ => "Time unknown".to_owned(),
    }
}

/// The project's name, if it is still one of the user's projects.
pub fn project_title(snapshot: &SharedState, project_id: Option<&str>) -> String {
    project_id
        .and_then(|id| snapshot.projects.iter().find(|p| p.id == id))
        .map(|p| p.title.clone())
        .unwrap_or_else(|| "A project no longer listed".to_owned())
}

fn row_divider(ui: &mut egui::Ui) {
    let y = ui.cursor().top() - 4.0;
    let x = ui.max_rect().x_range();
    ui.painter()
        .hline(x, y, egui::Stroke::new(1.0, theme::MUTED));
}

/// A wrapped line under a block, indented past its icon and tucked under
/// the block's own lines.
fn reason_line(ui: &mut egui::Ui, text: &str, color: Color32) {
    ui.add_space(-12.0);
    ui.horizontal_top(|ui| {
        ui.add_space(34.0);
        ui.add(
            egui::Label::new(
                egui::RichText::new(text)
                    .font(theme::regular(11.5))
                    .color(color),
            )
            .wrap(),
        );
    });
}

/// The web dashboard's palette (`frontend/src/lib/apps.ts`): each colour
/// clears 3:1 on a white card, so a name looks the same here as there.
pub const PALETTE: [Color32; 10] = [
    Color32::from_rgb(0x2d, 0x6f, 0xd4),
    Color32::from_rgb(0x93, 0x7c, 0xf1),
    Color32::from_rgb(0x9f, 0x4c, 0xb1),
    Color32::from_rgb(0xdd, 0x5d, 0x96),
    Color32::from_rgb(0xc4, 0x41, 0x32),
    Color32::from_rgb(0xd4, 0x7a, 0x05),
    Color32::from_rgb(0x97, 0x7e, 0x05),
    Color32::from_rgb(0x30, 0x8b, 0x15),
    Color32::from_rgb(0x05, 0xa8, 0x83),
    Color32::from_rgb(0x0a, 0xa0, 0xd2),
];

/// 32-bit FNV-1a over UTF-16 code units — the web's hash, so a name falls
/// into the same palette slot here as on the dashboard.
pub fn fnv1a(text: &str) -> u32 {
    text.encode_utf16().fold(0x811c_9dc5_u32, |hash, unit| {
        (hash ^ u32::from(unit)).wrapping_mul(0x0100_0193)
    })
}

/// A stable colour for a name (a project), from the palette.
pub fn name_color(name: &str) -> Color32 {
    PALETTE[fnv1a(&name.trim().to_lowercase()) as usize % PALETTE.len()]
}

fn upload_icon(ui: &mut egui::Ui, upload: UploadState) {
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(26.0), Sense::hover());
    let (icon, color, hint) = match upload {
        UploadState::Uploaded => (Icon::Check, theme::SUCCESS, "Uploaded"),
        UploadState::Queued => (
            Icon::Sync,
            theme::WARNING,
            "Saved on this computer, waiting to upload",
        ),
        UploadState::Rejected => (Icon::Cross, theme::DANGER, "Not accepted by the server"),
    };
    ui.painter()
        .circle_filled(rect.center(), 13.0, theme::tint(color));
    widgets::paint_icon(
        ui.painter(),
        Rect::from_center_size(rect.center(), Vec2::splat(14.0)),
        icon,
        color,
    );
    response.on_hover_text(hint);
}

/// The activity level of a block; "—" when it was not measured.
fn percent_pill(ui: &mut egui::Ui, percent: Option<u8>) {
    let (fill, text_color) = match percent {
        Some(p) => (theme::tint(activity_color(p)), activity_text_color(p)),
        None => (theme::MUTED, theme::NEUTRAL_ON_TINT),
    };
    let text = format::percent_label(percent);
    let galley = ui
        .painter()
        .layout_no_wrap(text, theme::semibold(11.5), Color32::PLACEHOLDER);
    let (rect, response) =
        ui.allocate_exact_size(Vec2::new(galley.size().x + 16.0, 22.0), Sense::hover());
    ui.painter().rect_filled(rect, CornerRadius::same(11), fill);
    ui.painter().galley(
        Pos2::new(
            rect.center().x - galley.size().x / 2.0,
            rect.center().y - galley.size().y / 2.0,
        ),
        galley,
        text_color,
    );
    response.on_hover_text(if percent.is_some() {
        "Activity: the share of minutes with keyboard or mouse input"
    } else {
        "Activity was not measured in this block"
    });
}

/// Decode the screenshot thumbnail for display.
pub fn decode_jpeg(bytes: &[u8]) -> Option<egui::ColorImage> {
    let image = image::load_from_memory_with_format(bytes, image::ImageFormat::Jpeg)
        .ok()?
        .to_rgba8();
    let size = [image.width() as usize, image.height() as usize];
    Some(egui::ColorImage::from_rgba_unmultiplied(
        size,
        image.as_raw(),
    ))
}
