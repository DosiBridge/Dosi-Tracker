//! Visual theme for the agent window.
//!
//! Mirrors the web dashboard's design tokens (`frontend/src/app/globals.css`) so
//! the desktop app and the browser product read as one piece of software.
//! Pure egui styling — no extra dependencies, no image assets.

use eframe::egui;
use egui::{Color32, CornerRadius, Stroke, Vec2};

// ---- Palette (light) — matches :root in globals.css -------------------------
pub const BACKGROUND: Color32 = Color32::from_rgb(0xf8, 0xf9, 0xfa);
pub const CARD: Color32 = Color32::from_rgb(0xff, 0xff, 0xff);
pub const MUTED: Color32 = Color32::from_rgb(0xf3, 0xf4, 0xf6);
pub const MUTED_FG: Color32 = Color32::from_rgb(0x6b, 0x72, 0x80);
pub const BORDER: Color32 = Color32::from_rgb(0xe5, 0xe7, 0xeb);
pub const INK: Color32 = Color32::from_rgb(0x1a, 0x1a, 0x1a);
pub const INK_SOFT: Color32 = Color32::from_rgb(0x4d, 0x50, 0x55);

pub const PRIMARY: Color32 = Color32::from_rgb(0x00, 0x6b, 0xff);
pub const PRIMARY_HOVER: Color32 = Color32::from_rgb(0x00, 0x5b, 0xe6);
pub const ACCENT: Color32 = Color32::from_rgb(0xee, 0xf4, 0xff);

pub const SUCCESS: Color32 = Color32::from_rgb(0x00, 0xa3, 0x89);
pub const WARNING: Color32 = Color32::from_rgb(0xf0, 0xa8, 0x00);
pub const DANGER: Color32 = Color32::from_rgb(0xd9, 0x36, 0x3e);

// ---- Rhythm -----------------------------------------------------------------
/// `--radius-control: 0.75rem`
pub const RADIUS: u8 = 10;
/// `--control-h: 2.5rem`
pub const CONTROL_H: f32 = 40.0;
pub const PAGE_PAD: f32 = 18.0;

/// Apply the theme once at startup.
///
/// egui 0.35 keeps a style per OS theme; we write the same one into both slots so
/// the agent's appearance is stable whether Windows is in light or dark mode.
pub fn apply(ctx: &egui::Context) {
    ctx.all_styles_mut(style_mut);
}

fn style_mut(style: &mut egui::Style) {
    // Typography: a clear size hierarchy instead of egui's flat defaults.
    use egui::{FontFamily::Proportional, FontId, TextStyle};
    style.text_styles = [
        (TextStyle::Heading, FontId::new(19.0, Proportional)),
        (TextStyle::Body, FontId::new(13.5, Proportional)),
        (TextStyle::Button, FontId::new(13.5, Proportional)),
        (TextStyle::Small, FontId::new(11.5, Proportional)),
        (TextStyle::Monospace, FontId::new(12.5, egui::FontFamily::Monospace)),
    ]
    .into();

    // Spacing rhythm.
    style.spacing.item_spacing = Vec2::new(8.0, 8.0);
    style.spacing.button_padding = Vec2::new(14.0, 10.0);
    style.spacing.interact_size = Vec2::new(40.0, CONTROL_H);
    style.spacing.menu_margin = egui::Margin::same(6);
    style.spacing.combo_height = 220.0;

    let mut visuals = egui::Visuals::light();
    visuals.override_text_color = Some(INK);
    visuals.panel_fill = BACKGROUND;
    visuals.window_fill = CARD;
    visuals.extreme_bg_color = CARD; // text-edit background
    visuals.faint_bg_color = MUTED;
    visuals.window_stroke = Stroke::new(1.0, BORDER);
    visuals.window_corner_radius = CornerRadius::same(RADIUS);
    visuals.selection.bg_fill = ACCENT;
    visuals.selection.stroke = Stroke::new(1.0, PRIMARY);
    // Flat, modern surfaces: no drop shadows on popups.
    visuals.popup_shadow = egui::epaint::Shadow::NONE;
    visuals.window_shadow = egui::epaint::Shadow::NONE;

    let radius = CornerRadius::same(RADIUS);

    // Non-interactive surfaces (labels, frames).
    visuals.widgets.noninteractive.bg_fill = CARD;
    visuals.widgets.noninteractive.weak_bg_fill = MUTED;
    visuals.widgets.noninteractive.bg_stroke = Stroke::new(1.0, BORDER);
    visuals.widgets.noninteractive.fg_stroke = Stroke::new(1.0, INK);
    visuals.widgets.noninteractive.corner_radius = radius;

    // Resting controls: white with a hairline border (not egui's grey slabs).
    visuals.widgets.inactive.bg_fill = CARD;
    visuals.widgets.inactive.weak_bg_fill = CARD;
    visuals.widgets.inactive.bg_stroke = Stroke::new(1.0, BORDER);
    visuals.widgets.inactive.fg_stroke = Stroke::new(1.0, INK_SOFT);
    visuals.widgets.inactive.corner_radius = radius;
    visuals.widgets.inactive.expansion = 0.0;

    // Hover: tinted brand wash + brand border.
    visuals.widgets.hovered.bg_fill = ACCENT;
    visuals.widgets.hovered.weak_bg_fill = ACCENT;
    visuals.widgets.hovered.bg_stroke = Stroke::new(1.0, PRIMARY);
    visuals.widgets.hovered.fg_stroke = Stroke::new(1.0, INK);
    visuals.widgets.hovered.corner_radius = radius;
    visuals.widgets.hovered.expansion = 0.0;

    visuals.widgets.active.bg_fill = ACCENT;
    visuals.widgets.active.weak_bg_fill = ACCENT;
    visuals.widgets.active.bg_stroke = Stroke::new(1.5, PRIMARY);
    visuals.widgets.active.fg_stroke = Stroke::new(1.0, INK);
    visuals.widgets.active.corner_radius = radius;
    visuals.widgets.active.expansion = 0.0;

    visuals.widgets.open.bg_fill = CARD;
    visuals.widgets.open.weak_bg_fill = CARD;
    visuals.widgets.open.bg_stroke = Stroke::new(1.0, PRIMARY);
    visuals.widgets.open.fg_stroke = Stroke::new(1.0, INK);
    visuals.widgets.open.corner_radius = radius;

    style.visuals = visuals;
}

/// A white surface card with a hairline border — the app's primary container.
pub fn card(padding: i8) -> egui::Frame {
    egui::Frame::new()
        .fill(CARD)
        .stroke(Stroke::new(1.0, BORDER))
        .corner_radius(CornerRadius::same(RADIUS))
        .inner_margin(egui::Margin::same(padding))
}

/// Small uppercase field/section label.
pub fn field_label(ui: &mut egui::Ui, text: &str) {
    ui.label(
        egui::RichText::new(text.to_uppercase())
            .size(10.0)
            .color(MUTED_FG)
            .strong(),
    );
    ui.add_space(4.0);
}

/// Full-width brand button. Returns the click response.
pub fn primary_button(ui: &mut egui::Ui, label: &str, enabled: bool) -> egui::Response {
    let fill = if enabled { PRIMARY } else { Color32::from_rgb(0x9a, 0xc4, 0xff) };
    let response = filled_button(ui, label, fill, enabled);

    // Hover/press feedback: egui buttons take a static fill, so darken on top.
    if enabled && (response.hovered() || response.is_pointer_button_down_on()) {
        ui.painter().rect_filled(
            response.rect,
            CornerRadius::same(RADIUS),
            PRIMARY_HOVER.linear_multiply(if response.is_pointer_button_down_on() {
                0.35
            } else {
                0.18
            }),
        );
        ui.painter().text(
            response.rect.center(),
            egui::Align2::CENTER_CENTER,
            label,
            egui::FontId::proportional(14.0),
            Color32::WHITE,
        );
    }

    response
}

/// Full-width solid button with **centred** white text.
///
/// egui aligns button text using the parent layout, which is top-down/left in
/// this app — so a justified, centred sub-layout is needed to centre the label.
pub fn filled_button(
    ui: &mut egui::Ui,
    label: &str,
    fill: Color32,
    enabled: bool,
) -> egui::Response {
    let size = Vec2::new(ui.available_width(), 42.0);
    ui.allocate_ui_with_layout(
        size,
        egui::Layout::centered_and_justified(egui::Direction::LeftToRight),
        |ui| {
            ui.add_enabled(
                enabled,
                egui::Button::new(
                    egui::RichText::new(label)
                        .color(Color32::WHITE)
                        .size(14.0)
                        .strong(),
                )
                .fill(fill)
                .corner_radius(CornerRadius::same(RADIUS))
                .stroke(Stroke::NONE),
            )
        },
    )
    .inner
}

/// Secondary (outline) button.
pub fn ghost_button(ui: &mut egui::Ui, label: &str, width: f32) -> egui::Response {
    ui.add(
        egui::Button::new(egui::RichText::new(label).color(INK_SOFT))
            .fill(CARD)
            .corner_radius(CornerRadius::same(RADIUS))
            .stroke(Stroke::new(1.0, BORDER))
            .min_size(Vec2::new(width, 36.0)),
    )
}

/// A tinted status pill: coloured dot + label on a soft background.
pub fn status_pill(ui: &mut egui::Ui, color: Color32, text: &str) {
    egui::Frame::new()
        .fill(tint(color))
        .corner_radius(CornerRadius::same(11))
        .inner_margin(egui::Margin::symmetric(9, 4))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 5.0;
                // Painted dot rather than a "●" glyph — the bundled font renders
                // that as a tofu box.
                let (dot, _) = ui.allocate_exact_size(Vec2::splat(7.0), egui::Sense::hover());
                ui.painter().circle_filled(dot.center(), 3.5, color);
                ui.label(egui::RichText::new(text).size(11.0).color(color).strong());
            });
        });
}

/// ~12% opacity wash of a colour over the card background, for pill/banner fills.
pub fn tint(color: Color32) -> Color32 {
    let blend = |c: u8, bg: u8| -> u8 { (c as f32 * 0.14 + bg as f32 * 0.86) as u8 };
    Color32::from_rgb(
        blend(color.r(), CARD.r()),
        blend(color.g(), CARD.g()),
        blend(color.b(), CARD.b()),
    )
}

/// The brand mark: a rounded-square tile with the app glyph.
pub fn logo_mark(ui: &mut egui::Ui, size: f32) {
    let (rect, _) = ui.allocate_exact_size(Vec2::splat(size), egui::Sense::hover());
    let painter = ui.painter();
    painter.rect_filled(rect, CornerRadius::same(8), PRIMARY);
    // Concentric "radar" rings, echoing the web app's Radar icon.
    let center = rect.center();
    painter.circle_stroke(center, size * 0.30, Stroke::new(1.6, Color32::WHITE));
    painter.circle_filled(center, size * 0.11, Color32::WHITE);
}
