//! The Activity view: this week day by day, the apps used in the last hour,
//! the latest screenshot exactly as captured, the blocks recorded so far
//! with their upload status — and the server's reason when it refused one.

use std::collections::HashSet;

use chrono::{DateTime, Datelike, Local, Utc};
use eframe::egui;
use egui::{Align, Color32, CornerRadius, Layout, Pos2, Rect, Sense, Vec2};

use super::timer::{activity_color, activity_text_color};
use super::widgets::{self, DayBar, Icon, Tone};
use super::{format, section, two_columns, TrackerApp};
use crate::state::{SharedState, UploadState};
use crate::storage::{QueuedRow, RowStatus};
use crate::theme;
use crate::tracking::usage::top_apps;

const WEEKDAYS: [&str; 7] = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];

/// How far back the "top apps" list looks.
const APPS_WINDOW_MS: i64 = 60 * 60 * 1000;

/// How long rejected blocks are kept on this computer (the worker's purge).
const REJECTED_KEPT_DAYS: u32 = 14;

impl TrackerApp {
    pub(super) fn activity_view(&mut self, ui: &mut egui::Ui, snapshot: &SharedState, now: DateTime<Utc>, wide: bool) {
        if wide {
            two_columns(ui, |column, ui| {
                if column == 0 {
                    self.week_card(ui, snapshot, now);
                    self.apps_card(ui, snapshot, now);
                } else {
                    self.screenshot_card(ui, snapshot);
                    self.recent_card(ui, snapshot);
                    self.earlier_card(ui, snapshot, now);
                }
            });
        } else {
            self.week_card(ui, snapshot, now);
            self.screenshot_card(ui, snapshot);
            self.apps_card(ui, snapshot, now);
            self.recent_card(ui, snapshot);
            self.earlier_card(ui, snapshot, now);
        }
    }

    fn week_card(&mut self, ui: &mut egui::Ui, snapshot: &SharedState, now: DateTime<Utc>) {
        let (today_secs, week_secs) = format::live_totals(snapshot, now);
        let today = now.with_timezone(&Local).weekday().num_days_from_monday() as usize;
        let hours: Vec<f64> = (0..7)
            .map(|day| match day.cmp(&today) {
                std::cmp::Ordering::Less => snapshot.week_day_secs[day] / 3600.0,
                std::cmp::Ordering::Equal => today_secs / 3600.0,
                std::cmp::Ordering::Greater => 0.0,
            })
            .collect();
        let bars: Vec<DayBar<'_>> = (0..7)
            .map(|day| DayBar { label: WEEKDAYS[day], hours: hours[day], today: day == today, future: day > today })
            .collect();

        let goal = self.prefs.weekly_goal_secs();
        let trailing = format!("{} of {}", format::secs_label(week_secs), widgets::hours_label(goal / 3600.0));
        section(ui, "This week", Some(&trailing), |ui| {
            widgets::week_chart(ui, &bars, f64::from(self.prefs.daily_goal_hours), 168.0);

            let worked: Vec<(usize, f64)> = hours[..=today].iter().copied().enumerate().filter(|(_, h)| *h > 0.0).collect();
            let average = if worked.is_empty() { 0.0 } else { worked.iter().map(|(_, h)| h).sum::<f64>() / worked.len() as f64 };
            let best = worked.iter().copied().max_by(|a, b| a.1.total_cmp(&b.1));
            ui.add_space(2.0);
            super::equal_row(ui, 3, 8.0, |i, ui| match i {
                0 => super::metric(ui, "Days worked", &format!("{} of {}", worked.len(), today + 1), theme::INK),
                1 => super::metric(ui, "Daily average", &widgets::hours_label(average), theme::INK),
                _ => super::metric(
                    ui,
                    "Best day",
                    &best.map_or("—".to_owned(), |(day, h)| format!("{} · {}", WEEKDAYS[day], widgets::hours_label(h))),
                    theme::INK,
                ),
            });
        });
    }

    fn apps_card(&mut self, ui: &mut egui::Ui, snapshot: &SharedState, now: DateTime<Utc>) {
        section(ui, "Top apps", Some("Last hour"), |ui| {
            if !snapshot.capture.active_window {
                widgets::muted(ui, "App usage isn't recorded on this project.");
                return;
            }
            let end = now.timestamp_millis();
            let apps = top_apps(&self.usage.slices_between(end - APPS_WINDOW_MS, end), 6);
            let Some(&(_, longest)) = apps.first() else {
                widgets::muted(ui, "Apps you use appear here as you work.");
                return;
            };
            for (app, ms) in &apps {
                let name = format::app_display_name(app);
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 10.0;
                    app_badge(ui, &name, app_color(&name));
                    ui.vertical(|ui| {
                        ui.spacing_mut().item_spacing.y = 4.0;
                        ui.horizontal(|ui| {
                            ui.label(egui::RichText::new(&name).font(theme::semibold(13.0)).color(theme::INK));
                            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                ui.label(
                                    egui::RichText::new(if *ms < 60_000 { "<1m".to_owned() } else { format::minutes_label((*ms / 60_000) as u64) })
                                        .font(theme::regular(12.0))
                                        .color(theme::MUTED_FG),
                                );
                            });
                        });
                        widgets::progress_bar(ui, *ms as f32 / longest as f32, 5.0, theme::mix(theme::PRIMARY, theme::CARD, 0.75));
                    });
                });
            }
            ui.add_space(2.0);
            widgets::muted(ui, "Foreground time on this computer. Shared with your workspace only as part of tracked blocks.");
        });
    }

    fn screenshot_card(&mut self, ui: &mut egui::Ui, snapshot: &SharedState) {
        if !snapshot.capture.screenshot {
            return;
        }
        let trailing = snapshot
            .last_screenshot
            .as_ref()
            .map(|shot| format!("Taken at {}", format::local_hm(shot.taken_at)));
        section(ui, "Latest screenshot", trailing.as_deref(), |ui| {
            let Some(shot) = snapshot.last_screenshot.clone() else {
                // The moment of the next one is random and not announced.
                widgets::muted(ui, "No screenshot yet this session. One is taken at a random moment in each block.");
                return;
            };
            if self.screenshot.as_ref().map(|(at, _)| *at) != Some(shot.taken_at) {
                self.screenshot = decode_jpeg(&shot.jpeg).map(|image| {
                    let texture = ui.ctx().load_texture("latest-screenshot", image, egui::TextureOptions::LINEAR);
                    (shot.taken_at, texture)
                });
            }
            match &self.screenshot {
                Some((_, texture)) => {
                    let size = texture.size_vec2();
                    let width = ui.available_width();
                    let height = (width * size.y / size.x.max(1.0)).min(320.0);
                    let width = height * size.x / size.y.max(1.0);
                    let image = egui::Image::from_texture(egui::load::SizedTexture::new(texture.id(), Vec2::new(width, height)))
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
                        "This is what your workspace sees: screenshots are blurred on this computer before upload, so no \
                         text can be read. One is taken at a random moment in each block."
                    } else {
                        "This is what your workspace sees. One screenshot is taken at a random moment in each block, of \
                         the screen you're working on."
                    };
                    widgets::muted(ui, note);
                }
                None => {
                    widgets::muted(ui, "The latest screenshot could not be shown.");
                }
            }
        });
    }

    fn recent_card(&mut self, ui: &mut egui::Ui, snapshot: &SharedState) {
        let trailing = (!snapshot.recent_blocks.is_empty()).then(|| format::count(snapshot.recent_blocks.len() as u32, "block"));
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
                let queued = snapshot.queue_rows.iter().find(|row| row.id == block.row_id);
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
                            format::minutes_label((block.ended_at - block.started_at).num_minutes().max(0) as u64)
                        );
                        ui.label(egui::RichText::new(span).font(theme::semibold(12.5)).color(theme::INK));
                        let mut detail = block.project.clone();
                        if block.screenshot {
                            detail.push_str(" · screenshot");
                        }
                        detail.push_str(match block.upload {
                            UploadState::Queued => " · waiting to upload",
                            UploadState::Uploaded => " · uploaded",
                            UploadState::Rejected => " · not accepted by the server",
                        });
                        ui.add(egui::Label::new(egui::RichText::new(detail).font(theme::regular(11.5)).color(theme::MUTED_FG)).truncate());
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
                    &format!("Blocks the server did not accept stay on this computer for {REJECTED_KEPT_DAYS} days for reference, then are deleted."),
                );
            }
        });
    }

    /// Blocks of earlier sessions still on this computer: waiting to upload,
    /// or not accepted (with the server's reason).
    fn earlier_card(&mut self, ui: &mut egui::Ui, snapshot: &SharedState, now: DateTime<Utc>) {
        let rows = earlier_rows(snapshot);
        if rows.is_empty() {
            return;
        }
        let waiting = rows.iter().filter(|row| row.status == RowStatus::Queued).count() as u32;
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
                let upload = if row.status == RowStatus::Rejected { UploadState::Rejected } else { UploadState::Queued };
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 8.0;
                    upload_icon(ui, upload);
                    ui.vertical(|ui| {
                        ui.spacing_mut().item_spacing.y = 0.0;
                        ui.label(egui::RichText::new(row_span(row, now)).font(theme::semibold(12.5)).color(theme::INK));
                        let mut detail = project_title(snapshot, row.project_id.as_deref());
                        if row.has_captures {
                            detail.push_str(" · with screenshot");
                        }
                        ui.add(egui::Label::new(egui::RichText::new(detail).font(theme::regular(11.5)).color(theme::MUTED_FG)).truncate());
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
        });
    }
}

/// What to say under a block about its upload: the server's reason for
/// refusing it, or why the last attempt to send it failed.
fn block_note(upload: UploadState, reason: Option<&str>) -> Option<(String, Color32)> {
    let reason = reason.map(str::trim).filter(|r| !r.is_empty());
    match (upload, reason) {
        (UploadState::Rejected, Some(reason)) => Some((format!("Server: {reason}"), theme::DANGER_ON_TINT)),
        (UploadState::Rejected, None) => Some(("The server did not say why.".to_owned(), theme::DANGER_ON_TINT)),
        (UploadState::Queued, Some(reason)) => Some((format!("Last try failed: {reason}"), theme::MUTED_FG)),
        _ => None,
    }
}

/// The queue's rows that are not blocks of this session (those are listed
/// with their session), oldest last as the queue lists them.
fn earlier_rows(snapshot: &SharedState) -> Vec<&QueuedRow> {
    let this_session: HashSet<i64> = snapshot.recent_blocks.iter().map(|b| b.row_id).collect();
    snapshot.queue_rows.iter().filter(|row| !this_session.contains(&row.id)).collect()
}

/// "Mon 14:05 – 14:15 · 10m" (the day when it is not today).
fn row_span(row: &QueuedRow, now: DateTime<Utc>) -> String {
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
fn project_title(snapshot: &SharedState, project_id: Option<&str>) -> String {
    project_id
        .and_then(|id| snapshot.projects.iter().find(|p| p.id == id))
        .map(|p| p.title.clone())
        .unwrap_or_else(|| "A project no longer listed".to_owned())
}

fn row_divider(ui: &mut egui::Ui) {
    let y = ui.cursor().top() - 4.0;
    let x = ui.max_rect().x_range();
    ui.painter().hline(x, y, egui::Stroke::new(1.0, theme::MUTED));
}

/// A wrapped line under a block, indented past its icon and tucked under
/// the block's own lines.
fn reason_line(ui: &mut egui::Ui, text: &str, color: Color32) {
    ui.add_space(-12.0);
    ui.horizontal_top(|ui| {
        ui.add_space(34.0);
        ui.add(egui::Label::new(egui::RichText::new(text).font(theme::regular(11.5)).color(color)).wrap());
    });
}

/// The web dashboard's app palette (`frontend/src/lib/apps.ts`): each colour
/// clears 3:1 on a white card, so an app looks the same here as there.
const APP_PALETTE: [Color32; 10] = [
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

/// Unnamed apps and "Other apps".
const OTHER_APP_COLOR: Color32 = Color32::from_rgb(0x76, 0x83, 0x90);

/// Fixed palette slots of the apps people most often run side by side (the
/// web's `PINNED_SLOTS`), so they never share a colour.
const PINNED_SLOTS: [(&str, usize); 35] = [
    ("visual studio code", 0),
    ("word", 0),
    ("microsoft teams", 1),
    ("notion", 1),
    ("linear", 1),
    ("discord", 1),
    ("visual studio", 2),
    ("zoom", 2),
    ("rider", 2),
    ("intellij idea", 2),
    ("slack", 3),
    ("onenote", 3),
    ("windows terminal", 4),
    ("powerpoint", 4),
    ("figma", 4),
    ("youtube", 4),
    ("opera", 4),
    ("powershell", 4),
    ("cmd", 4),
    ("firefox", 5),
    ("file explorer", 5),
    ("postman", 5),
    ("brave", 5),
    ("spotify", 5),
    ("google chrome", 6),
    ("excel", 7),
    ("terminal", 7),
    ("iterm2", 7),
    ("whatsapp", 7),
    ("microsoft edge", 8),
    ("finder", 8),
    ("outlook", 9),
    ("safari", 9),
    ("docker desktop", 9),
    ("telegram", 9),
];

/// 32-bit FNV-1a over UTF-16 code units — the web's hash, so a name falls
/// into the same palette slot here as on the dashboard.
fn fnv1a(text: &str) -> u32 {
    text.encode_utf16().fold(0x811c_9dc5_u32, |hash, unit| (hash ^ u32::from(unit)).wrapping_mul(0x0100_0193))
}

/// An app's colour (CONTRACT §8): a stable hash of its canonical name, never
/// its rank.
pub(super) fn app_color(name: &str) -> Color32 {
    let key = format::app_display_name(name).to_lowercase();
    if key == format::UNKNOWN_APP.to_lowercase() || key == "other apps" {
        return OTHER_APP_COLOR;
    }
    let slot = PINNED_SLOTS
        .iter()
        .find(|(app, _)| *app == key)
        .map_or_else(|| fnv1a(&key) as usize % APP_PALETTE.len(), |(_, slot)| *slot);
    APP_PALETTE[slot]
}

/// A stable colour for any other name (a project), from the same palette.
pub(super) fn name_color(name: &str) -> Color32 {
    APP_PALETTE[fnv1a(&name.trim().to_lowercase()) as usize % APP_PALETTE.len()]
}

/// A rounded square with the app's initial, in its colour.
fn app_badge(ui: &mut egui::Ui, name: &str, color: Color32) {
    let (rect, _) = ui.allocate_exact_size(Vec2::splat(30.0), Sense::hover());
    ui.painter().rect_filled(rect, CornerRadius::same(8), theme::mix(color, theme::CARD, 0.14));
    let initial: String = name.chars().next().map(|c| c.to_uppercase().collect()).unwrap_or_default();
    ui.painter().text(rect.center(), egui::Align2::CENTER_CENTER, initial, theme::semibold(14.0), color);
}

fn upload_icon(ui: &mut egui::Ui, upload: UploadState) {
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(26.0), Sense::hover());
    let (icon, color, hint) = match upload {
        UploadState::Uploaded => (Icon::Check, theme::SUCCESS, "Uploaded"),
        UploadState::Queued => (Icon::Sync, theme::WARNING, "Saved on this computer, waiting to upload"),
        UploadState::Rejected => (Icon::Cross, theme::DANGER, "Not accepted by the server"),
    };
    ui.painter().circle_filled(rect.center(), 13.0, theme::tint(color));
    widgets::paint_icon(ui.painter(), Rect::from_center_size(rect.center(), Vec2::splat(14.0)), icon, color);
    response.on_hover_text(hint);
}

fn percent_pill(ui: &mut egui::Ui, percent: u8) {
    let color = activity_color(percent);
    let text = format!("{percent}%");
    let galley = ui.painter().layout_no_wrap(text, theme::semibold(11.5), Color32::PLACEHOLDER);
    let (rect, response) = ui.allocate_exact_size(Vec2::new(galley.size().x + 16.0, 22.0), Sense::hover());
    ui.painter().rect_filled(rect, CornerRadius::same(11), theme::tint(color));
    let text_color = activity_text_color(percent);
    ui.painter().galley(Pos2::new(rect.center().x - galley.size().x / 2.0, rect.center().y - galley.size().y / 2.0), galley, text_color);
    response.on_hover_text("Activity: the share of minutes with keyboard or mouse input");
}

/// Decode the screenshot thumbnail for display.
fn decode_jpeg(bytes: &[u8]) -> Option<egui::ColorImage> {
    let image = image::load_from_memory_with_format(bytes, image::ImageFormat::Jpeg).ok()?.to_rgba8();
    let size = [image.width() as usize, image.height() as usize];
    Some(egui::ColorImage::from_rgba_unmultiplied(size, image.as_raw()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::{ProjectOption, RecentBlock};

    #[test]
    fn a_jpeg_thumbnail_decodes_to_an_image() {
        let mut jpeg = Vec::new();
        let source = image::RgbImage::from_pixel(8, 4, image::Rgb([10, 107, 255]));
        image::DynamicImage::ImageRgb8(source)
            .write_to(&mut std::io::Cursor::new(&mut jpeg), image::ImageFormat::Jpeg)
            .unwrap();
        let decoded = decode_jpeg(&jpeg).expect("decodes");
        assert_eq!(decoded.size, [8, 4]);
        assert!(decode_jpeg(b"not a jpeg").is_none());
    }

    #[test]
    fn the_hash_matches_the_web_dashboards() {
        // FNV-1a of "" and "a" (32-bit reference values).
        assert_eq!(fnv1a(""), 0x811c_9dc5);
        assert_eq!(fnv1a("a"), 0xe40c_292c);
    }

    #[test]
    fn apps_keep_one_colour_whatever_the_spelling() {
        assert_eq!(app_color("chrome.exe"), app_color("Google Chrome"));
        assert_eq!(app_color("chrome.exe"), APP_PALETTE[6], "pinned like the web");
        assert_eq!(app_color("Code.exe"), APP_PALETTE[0]);
        assert_eq!(app_color(""), OTHER_APP_COLOR);
        assert_eq!(app_color("Other apps"), OTHER_APP_COLOR);
        assert_eq!(app_color("mytool.exe"), app_color("MyTool"), "case and .exe do not matter");
        assert_eq!(name_color("Website"), name_color(" website "));
    }

    fn row(id: i64, status: RowStatus, reason: Option<&str>) -> QueuedRow {
        let now = Utc::now();
        QueuedRow {
            id,
            project_id: Some("p1".into()),
            started_at: Some(now - chrono::Duration::minutes(10)),
            ended_at: Some(now),
            status,
            reason: reason.map(str::to_owned),
            has_captures: false,
        }
    }

    #[test]
    fn earlier_rows_leave_out_this_sessions_blocks() {
        let now = Utc::now();
        let snapshot = SharedState {
            recent_blocks: vec![RecentBlock {
                row_id: 2,
                project: "P".into(),
                started_at: now,
                ended_at: now,
                productivity: 50,
                keyboard_hits: 0,
                mouse_clicks: 0,
                screenshot: false,
                upload: UploadState::Rejected,
            }],
            queue_rows: vec![row(1, RowStatus::Queued, None), row(2, RowStatus::Rejected, Some("Overlaps."))],
            ..SharedState::default()
        };
        let earlier: Vec<i64> = earlier_rows(&snapshot).iter().map(|r| r.id).collect();
        assert_eq!(earlier, vec![1]);
    }

    #[test]
    fn notes_quote_the_server_or_the_failed_attempt() {
        let (text, color) = block_note(UploadState::Rejected, Some(" This block overlaps another one. ")).unwrap();
        assert_eq!(text, "Server: This block overlaps another one.");
        assert_eq!(color, theme::DANGER_ON_TINT);
        assert_eq!(block_note(UploadState::Queued, Some("HTTP 503")).unwrap().0, "Last try failed: HTTP 503");
        assert!(block_note(UploadState::Queued, None).is_none());
        assert!(block_note(UploadState::Uploaded, Some("old")).is_none());
        assert!(block_note(UploadState::Rejected, None).is_some(), "a refusal is never silent");
    }

    #[test]
    fn rows_name_their_project_or_say_it_is_gone() {
        let snapshot = SharedState {
            projects: vec![ProjectOption { id: "p1".into(), title: "Website".into() }],
            ..SharedState::default()
        };
        assert_eq!(project_title(&snapshot, Some("p1")), "Website");
        assert_eq!(project_title(&snapshot, Some("old")), "A project no longer listed");
        assert_eq!(project_title(&snapshot, None), "A project no longer listed");
    }

    #[test]
    fn row_spans_survive_missing_times() {
        let now = Utc::now();
        let mut r = row(1, RowStatus::Queued, None);
        assert!(row_span(&r, now).contains("10m"));
        r.ended_at = None;
        assert!(!row_span(&r, now).is_empty());
        r.started_at = None;
        assert_eq!(row_span(&r, now), "Time unknown");
    }
}
