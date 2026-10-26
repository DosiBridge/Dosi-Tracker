//! Shared GUI widgets (status pill, badges, cards), ported from the Windows
//! agent's `ui/widgets.rs`: painted icons, buttons, toggles, chips, progress
//! bars and the week chart.
//!
//! Everything is drawn with the painter rather than relying on font glyphs —
//! egui's bundled fonts lack play/pause and most UI symbols — so the controls
//! look the same on every desktop.

use eframe::egui;
use egui::{
    Align2, Color32, CornerRadius, CursorIcon, Painter, Pos2, Rect, Response, Sense, Shape, Stroke,
    StrokeKind, Ui, Vec2,
};

use super::theme;

// ---- Icons --------------------------------------------------------------------

/// A small line icon, drawn on a 16×16 grid scaled to its rect.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Icon {
    Play,
    Pause,
    Timer,
    Chart,
    Sliders,
    Sync,
    External,
    Folder,
    Check,
    Cross,
    SignOut,
    Camera,
    Webcam,
    Keyboard,
    Mouse,
    Window,
    Info,
    Briefcase,
    Eye,
    EyeOff,
    ChevronDown,
    ChevronRight,
    /// A server address.
    Globe,
    Copy,
    /// This computer.
    Monitor,
    /// A warning triangle.
    Alert,
}

pub fn paint_icon(painter: &Painter, rect: Rect, icon: Icon, color: Color32) {
    let s = rect.width().min(rect.height()) / 16.0;
    let origin = rect.center() - Vec2::splat(8.0 * s);
    let p = |x: f32, y: f32| origin + Vec2::new(x * s, y * s);
    let stroke = Stroke::new((1.5 * s).max(1.0), color);
    let line =
        |points: &[(f32, f32)]| Shape::line(points.iter().map(|&(x, y)| p(x, y)).collect(), stroke);
    let closed = |points: &[(f32, f32)]| {
        Shape::closed_line(points.iter().map(|&(x, y)| p(x, y)).collect(), stroke)
    };
    let round_rect = |x0: f32, y0: f32, x1: f32, y1: f32, r: f32| {
        painter.rect_stroke(
            Rect::from_min_max(p(x0, y0), p(x1, y1)),
            CornerRadius::same((r * s) as u8),
            stroke,
            StrokeKind::Middle,
        );
    };

    match icon {
        Icon::Play => {
            painter.add(Shape::convex_polygon(
                vec![p(5.0, 3.0), p(13.5, 8.0), p(5.0, 13.0)],
                color,
                Stroke::new(s, color),
            ));
        }
        Icon::Pause => {
            for x in [4.0, 9.5] {
                painter.rect_filled(
                    Rect::from_min_max(p(x, 3.0), p(x + 2.75, 13.0)),
                    CornerRadius::same(s as u8),
                    color,
                );
            }
        }
        Icon::Timer => {
            painter.circle_stroke(p(8.0, 9.0), 5.5 * s, stroke);
            painter.add(line(&[(8.0, 9.0), (8.0, 6.0)]));
            painter.add(line(&[(8.0, 9.0), (10.25, 10.5)]));
            painter.add(line(&[(6.5, 1.75), (9.5, 1.75)]));
        }
        Icon::Chart => {
            for (x, top) in [(2.5, 9.0), (6.75, 5.0), (11.0, 2.0)] {
                painter.rect_filled(
                    Rect::from_min_max(p(x, top), p(x + 2.5, 14.0)),
                    CornerRadius::same(s as u8),
                    color,
                );
            }
        }
        Icon::Sliders => {
            for (y, knob) in [(4.0, 10.5), (8.0, 5.0), (12.0, 9.0)] {
                painter.add(line(&[(2.0, y), (14.0, y)]));
                painter.circle_filled(p(knob, y), 2.1 * s, color);
            }
        }
        Icon::Sync => {
            arc_with_head(
                painter,
                p(8.0, 8.0),
                5.25 * s,
                200f32.to_radians(),
                335f32.to_radians(),
                stroke,
            );
            arc_with_head(
                painter,
                p(8.0, 8.0),
                5.25 * s,
                20f32.to_radians(),
                155f32.to_radians(),
                stroke,
            );
        }
        Icon::External => {
            painter.add(line(&[
                (7.5, 3.5),
                (3.0, 3.5),
                (3.0, 13.0),
                (12.5, 13.0),
                (12.5, 8.5),
            ]));
            painter.add(line(&[(7.5, 8.5), (13.5, 2.5)]));
            painter.add(line(&[(9.5, 2.5), (13.5, 2.5), (13.5, 6.5)]));
        }
        Icon::Folder => {
            painter.add(closed(&[
                (1.75, 4.0),
                (6.0, 4.0),
                (7.5, 5.75),
                (14.25, 5.75),
                (14.25, 13.0),
                (1.75, 13.0),
            ]));
        }
        Icon::Check => {
            painter.add(line(&[(3.25, 8.5), (6.5, 11.75), (12.75, 4.5)]));
        }
        Icon::Cross => {
            painter.add(line(&[(4.5, 4.5), (11.5, 11.5)]));
            painter.add(line(&[(11.5, 4.5), (4.5, 11.5)]));
        }
        Icon::SignOut => {
            painter.add(line(&[(7.0, 2.5), (3.0, 2.5), (3.0, 13.5), (7.0, 13.5)]));
            painter.add(line(&[(6.5, 8.0), (13.5, 8.0)]));
            painter.add(line(&[(10.75, 5.25), (13.5, 8.0), (10.75, 10.75)]));
        }
        Icon::Camera => {
            round_rect(1.75, 4.75, 14.25, 13.25, 2.0);
            painter.add(line(&[
                (5.25, 4.75),
                (6.5, 2.75),
                (9.5, 2.75),
                (10.75, 4.75),
            ]));
            painter.circle_stroke(p(8.0, 9.0), 2.5 * s, stroke);
        }
        Icon::Webcam => {
            painter.circle_stroke(p(8.0, 6.75), 4.75 * s, stroke);
            painter.circle_filled(p(8.0, 6.75), 1.6 * s, color);
            painter.add(line(&[(8.0, 11.5), (8.0, 13.75)]));
            painter.add(line(&[(4.75, 14.0), (11.25, 14.0)]));
        }
        Icon::Keyboard => {
            round_rect(1.5, 4.0, 14.5, 12.5, 2.0);
            for x in [4.25, 6.75, 9.25, 11.75] {
                painter.circle_filled(p(x, 6.75), 0.75 * s, color);
            }
            painter.add(line(&[(5.0, 9.75), (11.0, 9.75)]));
        }
        Icon::Mouse => {
            round_rect(4.5, 1.75, 11.5, 14.25, 3.5);
            painter.add(line(&[(8.0, 1.75), (8.0, 6.0)]));
        }
        Icon::Window => {
            round_rect(1.75, 2.75, 14.25, 13.25, 2.0);
            painter.add(line(&[(1.75, 6.0), (14.25, 6.0)]));
        }
        Icon::Info => {
            painter.circle_stroke(p(8.0, 8.0), 6.25 * s, stroke);
            painter.add(line(&[(8.0, 7.25), (8.0, 11.25)]));
            painter.circle_filled(p(8.0, 4.9), 0.9 * s, color);
        }
        Icon::Briefcase => {
            round_rect(1.75, 5.0, 14.25, 13.5, 2.0);
            painter.add(line(&[
                (5.75, 5.0),
                (5.75, 2.75),
                (10.25, 2.75),
                (10.25, 5.0),
            ]));
            painter.add(line(&[(1.75, 8.75), (14.25, 8.75)]));
        }
        Icon::Eye | Icon::EyeOff => {
            let lid = |sign: f32| -> Vec<Pos2> {
                (0..=12)
                    .map(|i| {
                        let t = i as f32 / 12.0;
                        let x = 1.5 + 13.0 * t;
                        let y = 8.0 + sign * 4.75 * (std::f32::consts::PI * t).sin();
                        p(x, y)
                    })
                    .collect()
            };
            painter.add(Shape::line(lid(-1.0), stroke));
            painter.add(Shape::line(lid(1.0), stroke));
            painter.circle_stroke(p(8.0, 8.0), 2.0 * s, stroke);
            if icon == Icon::EyeOff {
                painter.add(line(&[(2.5, 13.5), (13.5, 2.5)]));
            }
        }
        Icon::ChevronDown => {
            painter.add(line(&[(4.0, 6.0), (8.0, 10.0), (12.0, 6.0)]));
        }
        Icon::ChevronRight => {
            painter.add(line(&[(6.0, 4.0), (10.0, 8.0), (6.0, 12.0)]));
        }
        Icon::Globe => {
            painter.circle_stroke(p(8.0, 8.0), 6.25 * s, stroke);
            let meridian: Vec<Pos2> = (0..=24)
                .map(|i| {
                    let t = i as f32 / 24.0 * std::f32::consts::TAU;
                    p(8.0 + 2.75 * t.cos(), 8.0 + 6.25 * t.sin())
                })
                .collect();
            painter.add(Shape::line(meridian, stroke));
            painter.add(line(&[(1.75, 8.0), (14.25, 8.0)]));
        }
        Icon::Copy => {
            painter.add(line(&[
                (5.5, 4.5),
                (5.5, 2.0),
                (13.5, 2.0),
                (13.5, 10.5),
                (11.0, 10.5),
            ]));
            round_rect(2.5, 5.0, 10.5, 14.0, 1.5);
        }
        Icon::Monitor => {
            round_rect(1.75, 2.75, 14.25, 11.25, 1.5);
            painter.add(line(&[(8.0, 11.25), (8.0, 13.75)]));
            painter.add(line(&[(5.0, 13.75), (11.0, 13.75)]));
        }
        Icon::Alert => {
            painter.add(closed(&[(8.0, 2.0), (14.5, 13.25), (1.5, 13.25)]));
            painter.add(line(&[(8.0, 6.25), (8.0, 9.5)]));
            painter.circle_filled(p(8.0, 11.3), 0.9 * s, color);
        }
    }
}

/// An arc from `start` to `end` (radians, clockwise on screen) with an
/// arrowhead at its end.
fn arc_with_head(
    painter: &Painter,
    center: Pos2,
    radius: f32,
    start: f32,
    end: f32,
    stroke: Stroke,
) {
    let steps = 14;
    let point = |a: f32| center + Vec2::new(a.cos(), a.sin()) * radius;
    let points: Vec<Pos2> = (0..=steps)
        .map(|i| point(start + (end - start) * i as f32 / steps as f32))
        .collect();
    painter.add(Shape::line(points, stroke));
    let tip = point(end);
    let tangent = Vec2::new(-end.sin(), end.cos());
    let back = -tangent * radius * 0.55;
    let wing = |turn: f32| egui::emath::Rot2::from_angle(turn) * back;
    painter.add(Shape::line(
        vec![tip + wing(0.6), tip, tip + wing(-0.6)],
        stroke,
    ));
}

/// An icon on its own, laid out like a widget.
pub fn icon(ui: &mut Ui, icon: Icon, size: f32, color: Color32) -> Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(size), Sense::hover());
    paint_icon(ui.painter(), rect, icon, color);
    response
}

// ---- Buttons ------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ButtonKind {
    /// Solid brand blue.
    Primary,
    /// White with a border.
    Secondary,
    /// Red text and border, for sign-out.
    Danger,
}

pub const BUTTON_H: f32 = 38.0;

/// A painted button with an optional leading icon. `width` of `None` sizes it
/// to its label.
pub fn button(
    ui: &mut Ui,
    kind: ButtonKind,
    icon: Option<Icon>,
    label: &str,
    width: Option<f32>,
    enabled: bool,
) -> Response {
    let font = theme::semibold(13.5);
    let galley = ui
        .painter()
        .layout_no_wrap(label.to_owned(), font, Color32::PLACEHOLDER);
    let icon_w = if icon.is_some() { 16.0 + 8.0 } else { 0.0 };
    let content_w = galley.size().x + icon_w;
    let width = width.unwrap_or(content_w + 32.0).max(content_w + 16.0);
    let sense = if enabled {
        Sense::click()
    } else {
        Sense::hover()
    };
    let (rect, response) = ui.allocate_exact_size(Vec2::new(width, BUTTON_H), sense);
    let response = if enabled {
        response.on_hover_cursor(CursorIcon::PointingHand)
    } else {
        response
    };

    if ui.is_rect_visible(rect) {
        let hovered = enabled && response.hovered();
        let pressed = enabled && response.is_pointer_button_down_on();
        let (fill, border, text) = match kind {
            ButtonKind::Primary => (
                if pressed {
                    theme::PRIMARY_PRESSED
                } else if hovered {
                    theme::PRIMARY_HOVER
                } else {
                    theme::PRIMARY
                },
                Color32::TRANSPARENT,
                Color32::WHITE,
            ),
            ButtonKind::Secondary => (
                if pressed {
                    theme::MUTED
                } else if hovered {
                    theme::ACCENT
                } else {
                    theme::CARD
                },
                if hovered {
                    theme::PRIMARY
                } else {
                    theme::BORDER_STRONG
                },
                if hovered {
                    theme::PRIMARY_STRONG
                } else {
                    theme::INK_SOFT
                },
            ),
            ButtonKind::Danger => (
                if hovered || pressed {
                    theme::tint(theme::DANGER)
                } else {
                    theme::CARD
                },
                if hovered {
                    theme::DANGER
                } else {
                    theme::mix(theme::DANGER, theme::CARD, 0.45)
                },
                theme::DANGER_ON_TINT,
            ),
        };
        let fade = |c: Color32| if enabled { c } else { c.gamma_multiply(0.45) };
        let painter = ui.painter();
        painter.rect(
            rect,
            CornerRadius::same(theme::RADIUS),
            fade(fill),
            Stroke::new(1.0, fade(border)),
            StrokeKind::Inside,
        );

        let start = rect.center().x - content_w / 2.0;
        if let Some(icon) = icon {
            let icon_rect =
                Rect::from_center_size(Pos2::new(start + 8.0, rect.center().y), Vec2::splat(16.0));
            paint_icon(painter, icon_rect, icon, fade(text));
        }
        let text_pos = Pos2::new(start + icon_w, rect.center().y - galley.size().y / 2.0);
        painter.galley(text_pos, galley, fade(text));
    }
    response
}

/// The big round start/pause control.
pub fn round_button(
    ui: &mut Ui,
    diameter: f32,
    fill: Color32,
    hover_fill: Color32,
    icon: Icon,
    enabled: bool,
) -> Response {
    let halo = 8.0;
    let sense = if enabled {
        Sense::click()
    } else {
        Sense::hover()
    };
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(diameter + halo * 2.0), sense);
    let response = if enabled {
        response.on_hover_cursor(CursorIcon::PointingHand)
    } else {
        response
    };
    let hovered = enabled && response.hovered();
    let t = ui.ctx().animate_bool(response.id.with("hover"), hovered);
    let center = rect.center();
    let painter = ui.painter();
    let fill = if enabled {
        fill
    } else {
        fill.gamma_multiply(0.45)
    };
    painter.circle_filled(
        center,
        diameter / 2.0 + halo * t,
        theme::mix(fill, theme::CARD, 0.16),
    );
    painter.circle_filled(
        center,
        diameter / 2.0,
        if hovered { hover_fill } else { fill },
    );
    let icon_size = diameter * 0.40;
    // The play triangle's optical centre sits right of its box centre.
    let nudge = if icon == Icon::Play {
        diameter * 0.03
    } else {
        0.0
    };
    paint_icon(
        painter,
        Rect::from_center_size(center + Vec2::new(nudge, 0.0), Vec2::splat(icon_size)),
        icon,
        Color32::WHITE,
    );
    response
}

/// A small pill button (timed breaks, steppers).
pub fn chip(ui: &mut Ui, label: &str, selected: bool, enabled: bool) -> Response {
    let font = theme::semibold(12.0);
    let galley = ui
        .painter()
        .layout_no_wrap(label.to_owned(), font, Color32::PLACEHOLDER);
    let size = Vec2::new(galley.size().x + 24.0, 30.0);
    let sense = if enabled {
        Sense::click()
    } else {
        Sense::hover()
    };
    let (rect, response) = ui.allocate_exact_size(size, sense);
    let response = if enabled {
        response.on_hover_cursor(CursorIcon::PointingHand)
    } else {
        response
    };
    let hovered = enabled && response.hovered();
    let (fill, border, text) = if selected || hovered {
        (theme::ACCENT, theme::PRIMARY, theme::PRIMARY_STRONG)
    } else {
        (theme::CARD, theme::BORDER_STRONG, theme::INK_SOFT)
    };
    let fade = |c: Color32| if enabled { c } else { c.gamma_multiply(0.45) };
    let painter = ui.painter();
    painter.rect(
        rect,
        CornerRadius::same(15),
        fade(fill),
        Stroke::new(1.0, fade(border)),
        StrokeKind::Inside,
    );
    painter.galley(rect.center() - galley.size() / 2.0, galley, fade(text));
    response
}

/// An on/off switch. Returns a response that is `changed()` when flipped.
pub fn toggle(ui: &mut Ui, on: &mut bool) -> Response {
    let size = Vec2::new(40.0, 22.0);
    let (rect, mut response) = ui.allocate_exact_size(size, Sense::click());
    let response_hovered = response.hovered();
    if response.clicked() {
        *on = !*on;
        response.mark_changed();
    }
    let t = ui.ctx().animate_bool(response.id, *on);
    let track = theme::mix(theme::PRIMARY, theme::BORDER_STRONG, t);
    let painter = ui.painter();
    painter.rect_filled(rect, CornerRadius::same(11), track);
    let knob_x = egui::lerp((rect.left() + 11.0)..=(rect.right() - 11.0), t);
    painter.circle_filled(
        Pos2::new(knob_x, rect.center().y + 0.5),
        8.5,
        Color32::from_black_alpha(25),
    );
    painter.circle_filled(Pos2::new(knob_x, rect.center().y), 8.5, Color32::WHITE);
    let response = response.on_hover_cursor(CursorIcon::PointingHand);
    if response_hovered {
        painter.rect_stroke(
            rect.expand(2.0),
            CornerRadius::same(13),
            Stroke::new(1.0, theme::mix(theme::PRIMARY, theme::CARD, 0.35)),
            StrokeKind::Inside,
        );
    }
    response
}

// ---- Status & text --------------------------------------------------------------

/// The four semantic tones used by pills and banners.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tone {
    Success,
    Warning,
    Danger,
    Info,
    Neutral,
}

impl Tone {
    /// (vivid colour for dots/icons, readable text colour on the tint, tint fill)
    pub fn colors(self) -> (Color32, Color32, Color32) {
        match self {
            Tone::Success => (
                theme::SUCCESS,
                theme::SUCCESS_ON_TINT,
                theme::tint(theme::SUCCESS),
            ),
            Tone::Warning => (
                theme::WARNING,
                theme::WARNING_ON_TINT,
                theme::tint(theme::WARNING),
            ),
            Tone::Danger => (
                theme::DANGER,
                theme::DANGER_ON_TINT,
                theme::tint(theme::DANGER),
            ),
            Tone::Info => (theme::PRIMARY, theme::PRIMARY_STRONG, theme::ACCENT),
            Tone::Neutral => (theme::FAINT, theme::NEUTRAL_ON_TINT, theme::MUTED),
        }
    }
}

/// A tinted status pill: coloured dot + label on a soft background.
pub fn status_pill(ui: &mut Ui, tone: Tone, text: &str) -> Response {
    let (dot, text_color, fill) = tone.colors();
    let galley = ui
        .painter()
        .layout_no_wrap(text.to_owned(), theme::semibold(11.5), text_color);
    let size = Vec2::new(galley.size().x + 30.0, 24.0);
    let (rect, response) = ui.allocate_exact_size(size, Sense::hover());
    let painter = ui.painter();
    painter.rect_filled(rect, CornerRadius::same(12), fill);
    let dot_center = Pos2::new(rect.left() + 13.0, rect.center().y);
    // A soft halo makes the "live" state read as live.
    if tone == Tone::Success {
        painter.circle_filled(dot_center, 6.0, theme::mix(dot, fill, 0.25));
    }
    painter.circle_filled(dot_center, 3.5, dot);
    painter.galley(
        Pos2::new(rect.left() + 22.0, rect.center().y - galley.size().y / 2.0),
        galley,
        text_color,
    );
    response
}

/// The icon a tone's notices carry: a triangle for problems, a tick for
/// good news, otherwise an "i".
fn tone_icon(tone: Tone) -> Icon {
    match tone {
        Tone::Danger | Tone::Warning => Icon::Alert,
        Tone::Success => Icon::Check,
        Tone::Info | Tone::Neutral => Icon::Info,
    }
}

/// Inline notice with a tinted background and an icon.
pub fn banner(ui: &mut Ui, tone: Tone, message: &str) {
    callout(
        ui,
        tone,
        &Callout {
            body: message,
            ..Callout::default()
        },
    );
}

/// What a [`callout`] says: an optional bold title, the message, an optional
/// quieter detail line (a server's own words), and an optional action link.
#[derive(Debug, Clone, Copy, Default)]
pub struct Callout<'a> {
    pub title: Option<&'a str>,
    pub body: &'a str,
    pub detail: Option<&'a str>,
    pub action: Option<&'a str>,
}

/// A tinted notice with an icon. Returns whether its action was clicked.
pub fn callout(ui: &mut Ui, tone: Tone, content: &Callout<'_>) -> bool {
    let (vivid, text, fill) = tone.colors();
    let mut clicked = false;
    egui::Frame::new()
        .fill(fill)
        .corner_radius(CornerRadius::same(theme::RADIUS))
        .inner_margin(egui::Margin::symmetric(12, 10))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                icon(ui, tone_icon(tone), 16.0, vivid);
                ui.vertical(|ui| {
                    ui.set_width(ui.available_width());
                    ui.spacing_mut().item_spacing.y = 2.0;
                    if let Some(title) = content.title {
                        ui.add(
                            egui::Label::new(
                                egui::RichText::new(title)
                                    .font(theme::semibold(12.5))
                                    .color(text),
                            )
                            .wrap(),
                        );
                    }
                    if !content.body.is_empty() {
                        ui.add(
                            egui::Label::new(
                                egui::RichText::new(content.body)
                                    .font(theme::regular(12.5))
                                    .color(text),
                            )
                            .wrap(),
                        );
                    }
                    if let Some(detail) = content.detail {
                        let quiet = theme::mix(text, fill, 0.78);
                        ui.add(
                            egui::Label::new(
                                egui::RichText::new(detail)
                                    .font(theme::regular(11.5))
                                    .color(quiet),
                            )
                            .wrap(),
                        );
                    }
                    if let Some(action) = content.action {
                        ui.add_space(2.0);
                        clicked = link(ui, action, text).clicked();
                    }
                });
            });
        });
    clicked
}

/// A short-lived message with a close button. Returns whether it was closed.
pub fn toast(ui: &mut Ui, tone: Tone, message: &str) -> bool {
    let (vivid, text, fill) = tone.colors();
    let mut closed = false;
    egui::Frame::new()
        .fill(fill)
        .stroke(Stroke::new(1.0, theme::mix(vivid, fill, 0.35)))
        .corner_radius(CornerRadius::same(theme::RADIUS))
        .inner_margin(egui::Margin::symmetric(12, 10))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                icon(ui, tone_icon(tone), 16.0, vivid);
                let close_w = 18.0;
                let text_w = (ui.available_width() - close_w - 8.0).max(0.0);
                ui.allocate_ui_with_layout(
                    Vec2::new(text_w, 0.0),
                    egui::Layout::top_down(egui::Align::Min),
                    |ui| {
                        // The whole width, so the close button sits at the right edge.
                        ui.set_width(text_w);
                        ui.add(
                            egui::Label::new(
                                egui::RichText::new(message)
                                    .font(theme::regular(12.5))
                                    .color(text),
                            )
                            .wrap(),
                        )
                    },
                );
                closed = icon_button(ui, Icon::Cross, close_w, text, "Dismiss").clicked();
            });
        });
    closed
}

/// Text that acts as a button: the brand's link style, underlined on hover.
pub fn link(ui: &mut Ui, text: &str, color: Color32) -> Response {
    let galley = ui
        .painter()
        .layout_no_wrap(text.to_owned(), theme::semibold(12.5), color);
    let (rect, response) = ui.allocate_exact_size(galley.size(), Sense::click());
    let response = response.on_hover_cursor(CursorIcon::PointingHand);
    let painter = ui.painter();
    if response.hovered() {
        let y = rect.bottom() - 1.0;
        painter.line_segment(
            [Pos2::new(rect.left(), y), Pos2::new(rect.right(), y)],
            Stroke::new(1.0, color),
        );
    }
    painter.galley(rect.min, galley, color);
    response
}

/// A small square button holding only an icon, tinted on hover.
pub fn icon_button(ui: &mut Ui, glyph: Icon, size: f32, color: Color32, hint: &str) -> Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(size), Sense::click());
    let response = response
        .on_hover_cursor(CursorIcon::PointingHand)
        .on_hover_text(hint);
    if response.hovered() {
        ui.painter().rect_filled(
            rect.expand(3.0),
            CornerRadius::same(6),
            theme::mix(color, theme::CARD, 0.12),
        );
    }
    paint_icon(ui.painter(), rect.shrink(size * 0.12), glyph, color);
    response
}

/// A full-width row that shows or hides a section: a chevron, its label,
/// and (while closed) a summary of what is inside on the right. The caller
/// flips `open` when it is clicked.
pub fn disclosure(ui: &mut Ui, open: bool, label: &str, summary: Option<&str>) -> Response {
    let (rect, response) =
        ui.allocate_exact_size(Vec2::new(ui.available_width(), 26.0), Sense::click());
    let response = response.on_hover_cursor(CursorIcon::PointingHand);
    let color = if response.hovered() {
        theme::PRIMARY_STRONG
    } else {
        theme::INK_SOFT
    };
    let painter = ui.painter();
    let chevron = if open {
        Icon::ChevronDown
    } else {
        Icon::ChevronRight
    };
    paint_icon(
        painter,
        Rect::from_center_size(
            Pos2::new(rect.left() + 7.0, rect.center().y),
            Vec2::splat(14.0),
        ),
        chevron,
        color,
    );
    let label_rect = painter.text(
        Pos2::new(rect.left() + 20.0, rect.center().y),
        Align2::LEFT_CENTER,
        label,
        theme::semibold(12.5),
        color,
    );
    if let Some(summary) = summary.filter(|_| !open) {
        let room = (rect.right() - label_rect.right() - 12.0).max(0.0);
        let galley =
            painter.layout_no_wrap(summary.to_owned(), theme::regular(12.0), theme::MUTED_FG);
        if galley.size().x <= room {
            painter.galley(
                Pos2::new(
                    rect.right() - galley.size().x,
                    rect.center().y - galley.size().y / 2.0,
                ),
                galley,
                theme::MUTED_FG,
            );
        }
    }
    response
}

/// Small uppercase section label, with optional trailing text on the right.
pub fn section_header(ui: &mut Ui, title: &str, trailing: Option<&str>) {
    // Painted into an exact-height row: a plain horizontal row would take the
    // 40 px minimum control height and push the card's content down.
    let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 18.0), Sense::hover());
    let painter = ui.painter();
    let title_rect = painter.text(
        rect.left_center(),
        Align2::LEFT_CENTER,
        title.to_uppercase(),
        theme::semibold(10.5),
        theme::MUTED_FG,
    );
    if let Some(trailing) = trailing {
        let room = (rect.right() - title_rect.right() - 12.0).max(0.0);
        let galley =
            painter.layout_no_wrap(trailing.to_owned(), theme::regular(11.5), theme::MUTED_FG);
        if galley.size().x <= room {
            painter.galley(
                Pos2::new(
                    rect.right() - galley.size().x,
                    rect.center().y - galley.size().y / 2.0,
                ),
                galley,
                theme::MUTED_FG,
            );
        }
    }
}

/// Muted secondary text.
pub fn muted(ui: &mut Ui, text: &str) -> Response {
    ui.add(
        egui::Label::new(
            egui::RichText::new(text)
                .font(theme::regular(12.0))
                .color(theme::MUTED_FG),
        )
        .wrap(),
    )
}

/// A circle with someone's initials.
pub fn avatar(ui: &mut Ui, name: &str, size: f32) {
    let (rect, _) = ui.allocate_exact_size(Vec2::splat(size), Sense::hover());
    let initials: String = name
        .split(|c: char| c == '@' || c == '.' || c.is_whitespace())
        .filter(|part| !part.is_empty())
        .take(2)
        .filter_map(|part| part.chars().next())
        .flat_map(char::to_uppercase)
        .collect();
    let painter = ui.painter();
    painter.circle_filled(rect.center(), size / 2.0, theme::ACCENT);
    painter.text(
        rect.center(),
        Align2::CENTER_CENTER,
        initials,
        theme::semibold(size * 0.38),
        theme::PRIMARY_STRONG,
    );
}

// ---- Progress & charts ----------------------------------------------------------

/// A rounded progress bar filling `fraction` (clamped) of its width.
pub fn progress_bar(ui: &mut Ui, fraction: f32, height: f32, color: Color32) {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), height), Sense::hover());
    paint_progress(ui.painter(), rect, fraction, color);
}

pub fn paint_progress(painter: &Painter, rect: Rect, fraction: f32, color: Color32) {
    let radius = CornerRadius::same((rect.height() / 2.0) as u8);
    painter.rect_filled(rect, radius, theme::MUTED);
    let fraction = if fraction.is_finite() {
        fraction.clamp(0.0, 1.0)
    } else {
        0.0
    };
    if fraction > 0.0 {
        let width = (rect.width() * fraction).max(rect.height());
        painter.rect_filled(
            Rect::from_min_size(rect.min, Vec2::new(width, rect.height())),
            radius,
            color,
        );
    }
}

/// One bar of the week chart.
pub struct DayBar<'a> {
    pub label: &'a str,
    pub hours: f64,
    pub today: bool,
    pub future: bool,
}

/// Hours per day as rounded bars with the value above each, a dashed goal
/// line, and today's bar in brand blue.
pub fn week_chart(ui: &mut Ui, days: &[DayBar<'_>], goal_hours: f64, height: f32) -> Response {
    let (rect, response) =
        ui.allocate_exact_size(Vec2::new(ui.available_width(), height), Sense::hover());
    let painter = ui.painter_at(rect.expand(2.0));
    let label_h = 18.0;
    let value_h = 16.0;
    let plot = Rect::from_min_max(
        Pos2::new(rect.left(), rect.top() + value_h),
        Pos2::new(rect.right(), rect.bottom() - label_h),
    );
    let max = days
        .iter()
        .map(|d| d.hours)
        .fold(goal_hours.max(1.0), f64::max);
    let y_for = |hours: f64| plot.bottom() - (hours / max) as f32 * plot.height();

    painter.line_segment(
        [
            Pos2::new(plot.left(), plot.bottom()),
            Pos2::new(plot.right(), plot.bottom()),
        ],
        Stroke::new(1.0, theme::BORDER),
    );
    if goal_hours > 0.0 {
        let y = y_for(goal_hours);
        painter.extend(Shape::dashed_line(
            &[Pos2::new(plot.left(), y), Pos2::new(plot.right(), y)],
            Stroke::new(1.0, theme::mix(theme::SUCCESS, theme::CARD, 0.7)),
            4.0,
            4.0,
        ));
        painter.text(
            Pos2::new(plot.right(), y - 2.0),
            Align2::RIGHT_BOTTOM,
            format!("{} goal", hours_label(goal_hours)),
            theme::regular(10.0),
            theme::SUCCESS_ON_TINT,
        );
    }

    let n = days.len().max(1) as f32;
    let slot = plot.width() / n;
    let bar_w = (slot * 0.52).clamp(10.0, 34.0);
    let hover = response.hover_pos();
    for (i, day) in days.iter().enumerate() {
        let cx = plot.left() + slot * (i as f32 + 0.5);
        let top = y_for(day.hours);
        let bar = Rect::from_min_max(
            Pos2::new(cx - bar_w / 2.0, top.min(plot.bottom() - 3.0)),
            Pos2::new(cx + bar_w / 2.0, plot.bottom()),
        );
        let color = if day.today {
            theme::PRIMARY
        } else if day.future {
            theme::MUTED
        } else {
            theme::mix(theme::PRIMARY, theme::CARD, 0.35)
        };
        let hovered = hover.is_some_and(|p| (p.x - cx).abs() <= slot / 2.0);
        let color = if hovered && !day.future {
            theme::PRIMARY_HOVER
        } else {
            color
        };
        painter.rect_filled(
            bar,
            CornerRadius {
                nw: 5,
                ne: 5,
                sw: 0,
                se: 0,
            },
            color,
        );
        if day.hours > 0.0 || day.today {
            let value_color = if day.today {
                theme::PRIMARY_STRONG
            } else {
                theme::INK_SOFT
            };
            let galley =
                painter.layout_no_wrap(hours_label(day.hours), theme::semibold(10.5), value_color);
            let label =
                Align2::CENTER_BOTTOM.anchor_size(Pos2::new(cx, bar.top() - 3.0), galley.size());
            // A card-coloured plate keeps the goal line from striking through the value.
            painter.rect_filled(
                label.expand2(Vec2::new(3.0, 0.0)),
                CornerRadius::same(3),
                theme::CARD,
            );
            painter.galley(label.min, galley, value_color);
        }
        let label_color = if day.today {
            theme::INK
        } else {
            theme::MUTED_FG
        };
        let font = if day.today {
            theme::semibold(11.0)
        } else {
            theme::regular(11.0)
        };
        painter.text(
            Pos2::new(cx, rect.bottom() - label_h / 2.0),
            Align2::CENTER_CENTER,
            day.label,
            font,
            label_color,
        );
    }
    response
}

/// "7h 30m" → compact "7.5h"-style label for chart values.
pub fn hours_label(hours: f64) -> String {
    if !hours.is_finite() || hours <= 0.0 {
        return "0h".into();
    }
    let total_minutes = (hours * 60.0).round() as u64;
    let (h, m) = (total_minutes / 60, total_minutes % 60);
    match (h, m) {
        (0, m) => format!("{m}m"),
        (h, 0) => format!("{h}h"),
        (h, m) => format!("{h}h {m:02}m"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hour_labels_are_compact() {
        assert_eq!(hours_label(0.0), "0h");
        assert_eq!(hours_label(f64::NAN), "0h");
        assert_eq!(hours_label(0.25), "15m");
        assert_eq!(hours_label(8.0), "8h");
        assert_eq!(hours_label(7.5), "7h 30m");
        assert_eq!(hours_label(1.0 + 5.0 / 60.0), "1h 05m");
    }
}
