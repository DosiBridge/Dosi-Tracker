//! GUI theme ported from the Windows agent's `theme.rs`.
//!
//! Mirrors the web dashboard's design tokens (`frontend/src/app/globals.css`)
//! so the desktop app and the browser product read as one piece of software.
//! On Linux the window uses egui's bundled fonts (no system font is loaded or
//! required), with a "semibold" family for emphasis that maps onto them. No
//! image assets: everything is painted.

use eframe::egui;
use egui::{Color32, CornerRadius, FontFamily, FontId, Stroke, Vec2};

// ---- Palette (light) — matches :root in globals.css -------------------------
pub const BACKGROUND: Color32 = Color32::from_rgb(0xf5, 0xf6, 0xf8);
pub const CARD: Color32 = Color32::from_rgb(0xff, 0xff, 0xff);
pub const MUTED: Color32 = Color32::from_rgb(0xf3, 0xf4, 0xf6);
pub const MUTED_FG: Color32 = Color32::from_rgb(0x6b, 0x72, 0x80);
/// Axis labels, placeholders, idle bars — quieter than [`MUTED_FG`].
pub const FAINT: Color32 = Color32::from_rgb(0x9c, 0xa3, 0xaf);
pub const BORDER: Color32 = Color32::from_rgb(0xe5, 0xe7, 0xeb);
pub const BORDER_STRONG: Color32 = Color32::from_rgb(0xd1, 0xd5, 0xdb);
pub const INK: Color32 = Color32::from_rgb(0x1a, 0x1a, 0x1a);
pub const INK_SOFT: Color32 = Color32::from_rgb(0x4d, 0x50, 0x55);

pub const PRIMARY: Color32 = Color32::from_rgb(0x00, 0x6b, 0xff);
pub const PRIMARY_HOVER: Color32 = Color32::from_rgb(0x00, 0x5b, 0xe6);
pub const PRIMARY_PRESSED: Color32 = Color32::from_rgb(0x00, 0x4f, 0xc7);
/// `--primary-strong`: brand blue for small text and links.
pub const PRIMARY_STRONG: Color32 = Color32::from_rgb(0x00, 0x55, 0xcc);
pub const ACCENT: Color32 = Color32::from_rgb(0xee, 0xf4, 0xff);

pub const SUCCESS: Color32 = Color32::from_rgb(0x00, 0xa3, 0x89);
pub const SUCCESS_HOVER: Color32 = Color32::from_rgb(0x00, 0x8f, 0x78);
pub const WARNING: Color32 = Color32::from_rgb(0xf0, 0xa8, 0x00);
pub const DANGER: Color32 = Color32::from_rgb(0xd9, 0x36, 0x3e);

// Text on the matching tinted backgrounds (pills, banners). The vivid hues
// above are for fills and icons; as small text on their own tint they fall
// under WCAG AA, so these darker variants are used instead (same values as
// the web app's `--*-on-tint` tokens).
pub const SUCCESS_ON_TINT: Color32 = Color32::from_rgb(0x00, 0x70, 0x5d);
pub const WARNING_ON_TINT: Color32 = Color32::from_rgb(0x8a, 0x5a, 0x00);
pub const DANGER_ON_TINT: Color32 = Color32::from_rgb(0xb3, 0x15, 0x1d);
pub const NEUTRAL_ON_TINT: Color32 = Color32::from_rgb(0x5b, 0x62, 0x70);

// ---- Rhythm -----------------------------------------------------------------
/// `--radius-control: 0.75rem`
pub const RADIUS: u8 = 10;
/// Cards are a touch rounder than controls.
pub const CARD_RADIUS: u8 = 12;
/// `--control-h: 2.5rem`
pub const CONTROL_H: f32 = 40.0;
pub const PAGE_PAD: f32 = 18.0;
/// Space between cards.
pub const GAP: f32 = 12.0;

/// The font family registered for emphasis.
const SEMIBOLD: &str = "semibold";

/// Apply the theme and fonts once at startup.
///
/// egui keeps a style per OS theme; the same one goes into both slots so the
/// agent's appearance is stable whether the desktop is light or dark.
pub fn apply(ctx: &egui::Context) {
    install_fonts(ctx);
    ctx.all_styles_mut(style_mut);
}

/// Regular text.
pub fn regular(size: f32) -> FontId {
    FontId::proportional(size)
}

/// Emphasised text (headings, values, buttons).
pub fn semibold(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name(SEMIBOLD.into()))
}

/// egui's bundled fonts, plus the "semibold" family over the same faces
/// (egui panics on an unknown family, so it must always be defined).
fn install_fonts(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();
    let proportional = fonts
        .families
        .get(&FontFamily::Proportional)
        .cloned()
        .unwrap_or_default();
    fonts
        .families
        .insert(FontFamily::Name(SEMIBOLD.into()), proportional);
    ctx.set_fonts(fonts);
}

fn style_mut(style: &mut egui::Style) {
    // Typography: a clear size hierarchy instead of egui's flat defaults.
    use egui::TextStyle;
    style.text_styles = [
        (TextStyle::Heading, semibold(19.0)),
        (TextStyle::Body, regular(13.5)),
        (TextStyle::Button, regular(13.5)),
        (TextStyle::Small, regular(11.5)),
        (
            TextStyle::Monospace,
            FontId::new(12.5, FontFamily::Monospace),
        ),
    ]
    .into();

    // Spacing rhythm.
    style.spacing.item_spacing = Vec2::new(8.0, 8.0);
    style.spacing.button_padding = Vec2::new(14.0, 10.0);
    style.spacing.interact_size = Vec2::new(40.0, CONTROL_H);
    style.spacing.menu_margin = egui::Margin::same(6);
    style.spacing.combo_height = 260.0;
    style.spacing.scroll = egui::style::ScrollStyle::thin();

    let mut visuals = egui::Visuals::light();
    visuals.override_text_color = Some(INK);
    visuals.panel_fill = BACKGROUND;
    visuals.window_fill = CARD;
    visuals.extreme_bg_color = CARD; // text-edit background
    visuals.faint_bg_color = MUTED;
    visuals.window_stroke = Stroke::new(1.0, BORDER);
    visuals.window_corner_radius = CornerRadius::same(RADIUS);
    visuals.menu_corner_radius = CornerRadius::same(RADIUS);
    visuals.selection.bg_fill = ACCENT;
    visuals.selection.stroke = Stroke::new(1.0, PRIMARY);
    visuals.hyperlink_color = PRIMARY_STRONG;
    // Popups float on a soft shadow, like the web app's menus.
    visuals.popup_shadow = egui::epaint::Shadow {
        offset: [0, 6],
        blur: 18,
        spread: 0,
        color: Color32::from_black_alpha(28),
    };
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
    visuals.widgets.inactive.bg_stroke = Stroke::new(1.0, BORDER_STRONG);
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

/// A white surface card with a hairline border and a soft lift — the app's
/// primary container.
pub fn card(padding: i8) -> egui::Frame {
    egui::Frame::new()
        .fill(CARD)
        .stroke(Stroke::new(1.0, BORDER))
        .corner_radius(CornerRadius::same(CARD_RADIUS))
        .inner_margin(egui::Margin::same(padding))
        .shadow(egui::epaint::Shadow {
            offset: [0, 1],
            blur: 3,
            spread: 0,
            color: Color32::from_black_alpha(10),
        })
}

/// A recessed grey panel inside a card (notes, read-only details).
pub fn inset(padding: i8) -> egui::Frame {
    egui::Frame::new()
        .fill(MUTED)
        .corner_radius(CornerRadius::same(RADIUS))
        .inner_margin(egui::Margin::same(padding))
}

/// ~14% wash of a colour over the card background, for pill/banner fills.
pub fn tint(color: Color32) -> Color32 {
    mix(color, CARD, 0.14)
}

/// `amount` of `color` over `base` (0 = all base, 1 = all colour).
pub fn mix(color: Color32, base: Color32, amount: f32) -> Color32 {
    let blend =
        |c: u8, bg: u8| -> u8 { (c as f32 * amount + bg as f32 * (1.0 - amount)).round() as u8 };
    Color32::from_rgb(
        blend(color.r(), base.r()),
        blend(color.g(), base.g()),
        blend(color.b(), base.b()),
    )
}

/// The brand mark: a rounded-square tile with concentric "radar" rings,
/// echoing the web app's Radar icon.
pub fn logo_mark(ui: &mut egui::Ui, size: f32) {
    let (rect, _) = ui.allocate_exact_size(Vec2::splat(size), egui::Sense::hover());
    let painter = ui.painter();
    painter.rect_filled(rect, CornerRadius::same((size * 0.28) as u8), PRIMARY);
    let center = rect.center();
    painter.circle_stroke(
        center,
        size * 0.30,
        Stroke::new(size * 0.06, Color32::WHITE),
    );
    painter.circle_filled(center, size * 0.11, Color32::WHITE);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mixing_spans_base_to_colour() {
        assert_eq!(mix(PRIMARY, CARD, 0.0), CARD);
        assert_eq!(mix(PRIMARY, CARD, 1.0), PRIMARY);
        let half = mix(Color32::BLACK, Color32::WHITE, 0.5);
        assert_eq!(half, Color32::from_rgb(128, 128, 128));
    }

    #[test]
    fn the_semibold_family_always_exists() {
        let ctx = egui::Context::default();
        apply(&ctx);
        // Laying text out in the family would panic if it were missing.
        let _ = ctx.run_ui(egui::RawInput::default(), |ui| {
            ui.label(egui::RichText::new("Dosi Tracker").font(semibold(15.0)));
        });
    }
}
