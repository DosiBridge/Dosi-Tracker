//! Local consent UX and view model for Dosi Tracker.
//!
//! Enforces the privacy invariant:
//! - All capabilities default off until affirmative local user choice.
//! - Clear disclosure of what is collected and what is NEVER collected.
//! - Granular opt-in for input COUNTS, active window/app context, screenshot and webcam.
//! - Effective policy is ProjectPolicy ∩ LocalConsent.

use eframe::egui;
use egui::{Align, CornerRadius, Layout, Rect, Sense, Stroke, Vec2};

use super::widgets::{self, ButtonKind, Icon, Tone};
use crate::config::{CapturePermissions, LocalConsent};
use crate::theme;

/// Effective capability status combining workspace policy and local consent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CapabilityStatus {
    /// Permitted by both workspace policy and local user consent.
    Active,
    /// Permitted by workspace, but declined/revoked by user on this device.
    DeniedByUser,
    /// Permitted by user, but disabled by workspace admin policy.
    DisabledByProject,
    /// Disabled by both.
    Disabled,
}

impl CapabilityStatus {
    pub fn label(self) -> &'static str {
        match self {
            CapabilityStatus::Active => "Active",
            CapabilityStatus::DeniedByUser => "Denied by you",
            CapabilityStatus::DisabledByProject => "Disabled by workspace",
            CapabilityStatus::Disabled => "Off",
        }
    }

    pub fn tone(self) -> Tone {
        match self {
            CapabilityStatus::Active => Tone::Success,
            CapabilityStatus::DeniedByUser => Tone::Warning,
            CapabilityStatus::DisabledByProject => Tone::Neutral,
            CapabilityStatus::Disabled => Tone::Neutral,
        }
    }
}

/// Pure, testable view model for the local consent interface.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConsentViewModel {
    pub current: LocalConsent,
    pub draft: CapturePermissions,
}

impl ConsentViewModel {
    /// Construct view model from current consent state.
    /// Invariant: when consent is Absent or Revoked, draft starts with all permissions OFF.
    pub fn new(current: LocalConsent) -> Self {
        let draft = match &current {
            LocalConsent::Granted(p) => p.clone(),
            LocalConsent::Absent | LocalConsent::Revoked => CapturePermissions::all_disabled(),
        };
        Self { current, draft }
    }

    #[allow(dead_code)]
    pub fn is_decided(&self) -> bool {
        !matches!(self.current, LocalConsent::Absent)
    }

    #[allow(dead_code)]
    pub fn is_granted(&self) -> bool {
        matches!(self.current, LocalConsent::Granted(_))
    }

    #[allow(dead_code)]
    pub fn is_revoked(&self) -> bool {
        matches!(self.current, LocalConsent::Revoked)
    }

    pub fn toggle_input_counts(&mut self, enabled: bool) {
        self.draft.keyboard = enabled;
        self.draft.mouse = enabled;
    }

    pub fn toggle_active_window(&mut self, enabled: bool) {
        self.draft.active_window = enabled;
        self.draft.running_programs = enabled;
    }

    pub fn toggle_screenshot(&mut self, enabled: bool) {
        self.draft.screenshot = enabled;
    }

    pub fn toggle_blur_screenshots(&mut self, enabled: bool) {
        self.draft.blur_screenshots = enabled;
    }

    pub fn toggle_webcam(&mut self, enabled: bool) {
        self.draft.webcam = enabled;
    }

    /// Grant consent using current draft toggles.
    pub fn grant(&mut self) -> LocalConsent {
        let consent = LocalConsent::Granted(self.draft.clone());
        self.current = consent.clone();
        consent
    }

    /// Explicitly decline consent.
    pub fn decline(&mut self) -> LocalConsent {
        self.draft = CapturePermissions::all_disabled();
        self.current = LocalConsent::Revoked;
        LocalConsent::Revoked
    }

    /// Explicitly revoke previously granted consent.
    pub fn revoke(&mut self) -> LocalConsent {
        self.draft = CapturePermissions::all_disabled();
        self.current = LocalConsent::Revoked;
        LocalConsent::Revoked
    }

    /// Compute effective status of a capability.
    pub fn capability_status(consented: bool, project_allowed: bool) -> CapabilityStatus {
        match (consented, project_allowed) {
            (true, true) => CapabilityStatus::Active,
            (false, true) => CapabilityStatus::DeniedByUser,
            (true, false) => CapabilityStatus::DisabledByProject,
            (false, false) => CapabilityStatus::Disabled,
        }
    }
}

/// Render the prominent local consent prompt card in the main timer view.
pub fn render_consent_card(
    ui: &mut egui::Ui,
    model: &mut ConsentViewModel,
    mut on_action: impl FnMut(LocalConsent),
) {
    egui::Frame::new()
        .fill(theme::CARD)
        .stroke(Stroke::new(1.0, theme::mix(theme::PRIMARY, theme::CARD, 0.4)))
        .corner_radius(CornerRadius::same(theme::CARD_RADIUS))
        .inner_margin(egui::Margin::same(18))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 10.0;

            // Header with Shield icon
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 10.0;
                let (rect, _) = ui.allocate_exact_size(Vec2::splat(32.0), Sense::hover());
                ui.painter().circle_filled(rect.center(), 16.0, theme::tint(theme::PRIMARY));
                widgets::paint_icon(
                    ui.painter(),
                    Rect::from_center_size(rect.center(), Vec2::splat(18.0)),
                    Icon::Info,
                    theme::PRIMARY_STRONG,
                );
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 1.0;
                    ui.label(
                        egui::RichText::new("Local Tracking Consent Required")
                            .font(theme::semibold(15.0))
                            .color(theme::INK),
                    );
                    ui.label(
                        egui::RichText::new("Choose what this computer may record. All options default off.")
                            .font(theme::regular(12.0))
                            .color(theme::MUTED_FG),
                    );
                });
            });

            ui.add_space(4.0);

            // Never collected banner
            theme::inset(10).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.label(
                    egui::RichText::new("Never collected: raw keystrokes (what you type), passwords, clipboard, audio, or files.")
                        .font(theme::regular(11.5))
                        .color(theme::MUTED_FG),
                );
            });

            ui.add_space(6.0);

            // Granular checkboxes
            let mut input_counts = model.draft.keyboard && model.draft.mouse;
            if ui.checkbox(&mut input_counts, "Input counts (keystroke & click totals only; never key text)").changed() {
                model.toggle_input_counts(input_counts);
            }

            let mut window_titles = model.draft.active_window && model.draft.running_programs;
            if ui.checkbox(&mut window_titles, "Active window context (foreground app name and window title)").changed() {
                model.toggle_active_window(window_titles);
            }

            let mut screenshots = model.draft.screenshot;
            if ui.checkbox(&mut screenshots, "Screenshots (periodic screen captures)").changed() {
                model.toggle_screenshot(screenshots);
            }

            if model.draft.screenshot {
                ui.indent("blur_indent", |ui| {
                    let mut blur = model.draft.blur_screenshots;
                    if ui.checkbox(&mut blur, "Blur screenshots on this computer before upload").changed() {
                        model.toggle_blur_screenshots(blur);
                    }
                });
            }

            let mut webcam = model.draft.webcam;
            if ui.checkbox(&mut webcam, "Webcam photo (periodic photo, if project policy permits)").changed() {
                model.toggle_webcam(webcam);
            }

            ui.add_space(8.0);

            // Action buttons
            ui.horizontal(|ui| {
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if widgets::button(ui, ButtonKind::Primary, Some(Icon::Check), "Save Consent Choices", None, true).clicked() {
                        let consent = model.grant();
                        on_action(consent);
                    }
                    ui.add_space(8.0);
                    if widgets::button(ui, ButtonKind::Secondary, None, "Decline All", None, true).clicked() {
                        let consent = model.decline();
                        on_action(consent);
                    }
                });
            });
        });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fresh_model_defaults_all_off_when_consent_is_absent() {
        let model = ConsentViewModel::new(LocalConsent::Absent);
        assert!(!model.is_decided());
        assert!(!model.is_granted());
        assert!(!model.is_revoked());
        assert!(!model.draft.keyboard);
        assert!(!model.draft.mouse);
        assert!(!model.draft.active_window);
        assert!(!model.draft.running_programs);
        assert!(!model.draft.screenshot);
        assert!(!model.draft.webcam);
    }

    #[test]
    fn fresh_model_defaults_all_off_when_consent_is_revoked() {
        let model = ConsentViewModel::new(LocalConsent::Revoked);
        assert!(model.is_decided());
        assert!(!model.is_granted());
        assert!(model.is_revoked());
        assert!(!model.draft.keyboard);
        assert!(!model.draft.screenshot);
    }

    #[test]
    fn grant_saves_exact_granular_selections() {
        let mut model = ConsentViewModel::new(LocalConsent::Absent);
        model.toggle_input_counts(true);
        model.toggle_active_window(false);
        model.toggle_screenshot(true);
        model.toggle_blur_screenshots(true);
        model.toggle_webcam(false);

        let consent = model.grant();
        assert!(model.is_granted());
        assert!(model.is_decided());
        if let LocalConsent::Granted(p) = consent {
            assert!(p.keyboard && p.mouse);
            assert!(!p.active_window && !p.running_programs);
            assert!(p.screenshot && p.blur_screenshots);
            assert!(!p.webcam);
        } else {
            panic!("expected Granted consent");
        }
    }

    #[test]
    fn decline_sets_revoked_and_disables_all() {
        let mut model = ConsentViewModel::new(LocalConsent::Absent);
        model.toggle_input_counts(true);
        let consent = model.decline();
        assert_eq!(consent, LocalConsent::Revoked);
        assert!(model.is_revoked());
        assert!(!model.draft.keyboard);
    }

    #[test]
    fn revoke_clears_active_grant() {
        let mut initial = CapturePermissions::all_disabled();
        initial.keyboard = true;
        let mut model = ConsentViewModel::new(LocalConsent::Granted(initial));
        assert!(model.is_granted());

        let consent = model.revoke();
        assert_eq!(consent, LocalConsent::Revoked);
        assert!(model.is_revoked());
        assert!(!model.draft.keyboard);
    }

    #[test]
    fn capability_effective_status_matrix() {
        assert_eq!(ConsentViewModel::capability_status(true, true), CapabilityStatus::Active);
        assert_eq!(ConsentViewModel::capability_status(false, true), CapabilityStatus::DeniedByUser);
        assert_eq!(ConsentViewModel::capability_status(true, false), CapabilityStatus::DisabledByProject);
        assert_eq!(ConsentViewModel::capability_status(false, false), CapabilityStatus::Disabled);
    }
}
