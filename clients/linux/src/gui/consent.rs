//! Consent screen (port of the Windows `ui/consent.rs`) with availability on
//! this desktop, the never-collected panel and time-only tracking.
//!
//! Enforces the privacy invariant:
//! - Every capability is off until the local user turns it on.
//! - What is collected, and what is never collected, is said plainly.
//! - Input COUNTS, the active window, running programs and screenshots are
//!   chosen one by one; the webcam cannot be chosen on Linux in v1.
//! - Effective = workspace policy AND local consent AND local configuration
//!   AND what this desktop can provide; each row says which one is missing.
//!
//! The view model and the rows are pure ([`ConsentViewModel`], [`rows`]);
//! the window only draws them.

use eframe::egui;
use egui::{CornerRadius, Rect, Sense, Stroke, Vec2};

use super::theme;
use super::widgets::{self, ButtonKind, Icon, Tone};
use crate::agent::session::Availability;
use crate::agent::state::{CapabilityInfo, CapabilityState, SessionKind, SharedState};
use crate::consent::{ConsentedPermissions, LocalConsent};

/// The webcam row's note.
pub const WEBCAM_NOTE: &str = "Not available on Linux yet.";

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
    pub draft: ConsentedPermissions,
}

impl ConsentViewModel {
    /// Construct view model from current consent state.
    /// Invariant: when consent is Absent or Revoked, draft starts with all permissions OFF.
    pub fn new(current: LocalConsent) -> Self {
        let mut draft = match &current {
            LocalConsent::Granted(p) => *p,
            LocalConsent::Absent | LocalConsent::Revoked => ConsentedPermissions::all_disabled(),
        };
        draft.webcam = false;
        Self { current, draft }
    }

    pub fn is_decided(&self) -> bool {
        !matches!(self.current, LocalConsent::Absent)
    }

    pub fn is_granted(&self) -> bool {
        matches!(self.current, LocalConsent::Granted(_))
    }

    pub fn is_revoked(&self) -> bool {
        matches!(self.current, LocalConsent::Revoked)
    }

    /// The draft differs from what is saved.
    pub fn is_dirty(&self) -> bool {
        match &self.current {
            LocalConsent::Granted(p) => *p != self.draft,
            _ => self.draft != ConsentedPermissions::all_disabled(),
        }
    }

    pub fn toggle_input_counts(&mut self, enabled: bool) {
        self.draft.keyboard = enabled;
        self.draft.mouse = enabled;
    }

    pub fn toggle_keyboard(&mut self, enabled: bool) {
        self.draft.keyboard = enabled;
    }

    pub fn toggle_mouse(&mut self, enabled: bool) {
        self.draft.mouse = enabled;
    }

    pub fn toggle_active_window(&mut self, enabled: bool) {
        self.draft.active_window = enabled;
        self.draft.running_programs = enabled;
    }

    pub fn toggle_window_titles(&mut self, enabled: bool) {
        self.draft.active_window = enabled;
    }

    pub fn toggle_running_programs(&mut self, enabled: bool) {
        self.draft.running_programs = enabled;
    }

    pub fn toggle_screenshot(&mut self, enabled: bool) {
        self.draft.screenshot = enabled;
    }

    pub fn toggle_blur_screenshots(&mut self, enabled: bool) {
        self.draft.blur_screenshots = enabled;
    }

    /// The webcam cannot be granted on Linux in v1: the draft stays off.
    pub fn toggle_webcam(&mut self, _enabled: bool) {
        self.draft.webcam = false;
    }

    /// Grant consent using current draft toggles (never the webcam).
    pub fn grant(&mut self) -> LocalConsent {
        self.draft.webcam = false;
        let consent = LocalConsent::Granted(self.draft);
        self.current = consent;
        consent
    }

    /// Track time only: consent with every capability off.
    pub fn grant_time_only(&mut self) -> LocalConsent {
        self.draft = ConsentedPermissions::all_disabled();
        self.grant()
    }

    /// Explicitly decline consent.
    pub fn decline(&mut self) -> LocalConsent {
        self.draft = ConsentedPermissions::all_disabled();
        self.current = LocalConsent::Revoked;
        LocalConsent::Revoked
    }

    /// Explicitly revoke previously granted consent.
    pub fn revoke(&mut self) -> LocalConsent {
        self.decline()
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

/// One capability of the consent screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Capability {
    Keyboard,
    Mouse,
    ActiveWindow,
    RunningPrograms,
    Screenshot,
    Webcam,
}

impl Capability {
    pub const ALL: [Capability; 6] = [
        Capability::Keyboard,
        Capability::Mouse,
        Capability::ActiveWindow,
        Capability::RunningPrograms,
        Capability::Screenshot,
        Capability::Webcam,
    ];

    pub fn title(self) -> &'static str {
        match self {
            Capability::Keyboard => "Keyboard counts",
            Capability::Mouse => "Mouse click counts",
            Capability::ActiveWindow => "Active window",
            Capability::RunningPrograms => "Running programs",
            Capability::Screenshot => "Screenshots",
            Capability::Webcam => "Webcam photo",
        }
    }

    /// What is recorded, in the words of what is collected.
    pub fn detail(self) -> &'static str {
        match self {
            Capability::Keyboard => {
                "How many keys are pressed per minute — never which keys or what you type"
            }
            Capability::Mouse => "How many clicks per minute — never where",
            Capability::ActiveWindow => "The app in front and its window title, and for how long",
            Capability::RunningPrograms => {
                "The names of programs you run — never their window titles"
            }
            Capability::Screenshot => {
                "One per block, at a random moment, of the screen you're working on"
            }
            Capability::Webcam => "A periodic photo",
        }
    }

    pub fn icon(self) -> Icon {
        match self {
            Capability::Keyboard => Icon::Keyboard,
            Capability::Mouse => Icon::Mouse,
            Capability::ActiveWindow => Icon::Window,
            Capability::RunningPrograms => Icon::Briefcase,
            Capability::Screenshot => Icon::Camera,
            Capability::Webcam => Icon::Webcam,
        }
    }

    pub fn info(self, s: &SharedState) -> &CapabilityInfo {
        let c = &s.capabilities;
        match self {
            Capability::Keyboard => &c.keyboard,
            Capability::Mouse => &c.mouse,
            Capability::ActiveWindow => &c.active_window,
            Capability::RunningPrograms => &c.running_programs,
            Capability::Screenshot => &c.screenshot,
            Capability::Webcam => &c.webcam,
        }
    }

    pub fn chosen(self, p: &ConsentedPermissions) -> bool {
        match self {
            Capability::Keyboard => p.keyboard,
            Capability::Mouse => p.mouse,
            Capability::ActiveWindow => p.active_window,
            Capability::RunningPrograms => p.running_programs,
            Capability::Screenshot => p.screenshot,
            Capability::Webcam => false,
        }
    }

    pub fn set(self, vm: &mut ConsentViewModel, on: bool) {
        match self {
            Capability::Keyboard => vm.toggle_keyboard(on),
            Capability::Mouse => vm.toggle_mouse(on),
            Capability::ActiveWindow => vm.toggle_window_titles(on),
            Capability::RunningPrograms => vm.toggle_running_programs(on),
            Capability::Screenshot => vm.toggle_screenshot(on),
            Capability::Webcam => vm.toggle_webcam(on),
        }
    }
}

/// Whether this desktop can provide a capability.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DesktopSupport {
    Available,
    /// It can never be recorded here; the reason says why.
    Unsupported(String),
    /// The desktop is waiting for the user to approve it.
    NeedsApproval,
    /// The user refused it in the desktop's dialog.
    Denied,
    /// It failed for now.
    Unavailable(String),
}

impl DesktopSupport {
    pub fn badge(&self) -> (Tone, &'static str) {
        match self {
            DesktopSupport::Available => (Tone::Success, "Available here"),
            DesktopSupport::Unsupported(_) => (Tone::Neutral, "Not on this desktop"),
            DesktopSupport::NeedsApproval => (Tone::Info, "Needs approval"),
            DesktopSupport::Denied => (Tone::Warning, "Refused"),
            DesktopSupport::Unavailable(_) => (Tone::Warning, "Unavailable now"),
        }
    }

    pub fn note(&self) -> Option<&str> {
        match self {
            DesktopSupport::Unsupported(why) | DesktopSupport::Unavailable(why) => Some(why),
            DesktopSupport::NeedsApproval => {
                Some("The desktop asks once whether Dosi Tracker may take screenshots.")
            }
            DesktopSupport::Denied => Some(
                "Screenshots were refused in the desktop's dialog; test the permission to ask again.",
            ),
            DesktopSupport::Available => None,
        }
    }
}

/// One row of the consent screen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConsentRow {
    pub capability: Capability,
    /// The draft's choice.
    pub chosen: bool,
    /// The row can be switched on.
    pub selectable: bool,
    /// The current project records it (`None`: no project known yet).
    pub workspace: Option<bool>,
    /// The configuration on this computer allows it.
    pub config_allows: bool,
    pub desktop: DesktopSupport,
}

/// What this desktop says about `capability`: the static matrix of the
/// session (design 7), narrowed by what the sensors reported.
pub fn desktop_support(s: &SharedState, capability: Capability) -> DesktopSupport {
    if capability == Capability::Webcam {
        return DesktopSupport::Unsupported(WEBCAM_NOTE.to_owned());
    }
    let kind = s.session.kind;
    let available = Availability::for_session(kind);
    let statically = match capability {
        Capability::Keyboard => available.keyboard,
        Capability::Mouse => available.mouse,
        Capability::ActiveWindow => available.active_window,
        Capability::RunningPrograms => available.running_programs,
        Capability::Screenshot => available.screenshot,
        Capability::Webcam => false,
    };
    if !statically {
        let why = match (kind, capability) {
            (SessionKind::Wayland, Capability::Keyboard | Capability::Mouse) => {
                "Keyboard and mouse counts are not available on Wayland. Dosi Tracker never uses root or input devices to get around this."
                    .to_owned()
            }
            (SessionKind::Wayland, Capability::ActiveWindow) => {
                "The active window is not available on Wayland.".to_owned()
            }
            _ => "Not available without a desktop session.".to_owned(),
        };
        return DesktopSupport::Unsupported(why);
    }
    let info = capability.info(s);
    match info.state {
        CapabilityState::Unsupported => DesktopSupport::Unsupported(
            info.reason
                .clone()
                .unwrap_or_else(|| "Not available on this desktop.".to_owned()),
        ),
        CapabilityState::NeedsApproval => DesktopSupport::NeedsApproval,
        CapabilityState::Denied => DesktopSupport::Denied,
        CapabilityState::Unavailable => DesktopSupport::Unavailable(
            info.reason
                .clone()
                .unwrap_or_else(|| "It failed for now and is retried.".to_owned()),
        ),
        _ => DesktopSupport::Available,
    }
}

/// The consent screen's rows for the snapshot and the draft.
pub fn rows(s: &SharedState, draft: &ConsentedPermissions) -> Vec<ConsentRow> {
    let known_project = s.current_project().is_some();
    Capability::ALL
        .iter()
        .map(|&capability| {
            let info = capability.info(s);
            ConsentRow {
                capability,
                chosen: capability.chosen(draft),
                selectable: capability != Capability::Webcam,
                workspace: known_project.then_some(info.project_allows),
                config_allows: info.config_allows || capability == Capability::Webcam,
                desktop: desktop_support(s, capability),
            }
        })
        .collect()
}

/// The badge of a capability's effective state (design 10's taxonomy).
pub fn state_badge(state: CapabilityState) -> (Tone, &'static str) {
    match state {
        CapabilityState::On => (Tone::Success, "On"),
        CapabilityState::OffByYou => (Tone::Neutral, "Off by you"),
        CapabilityState::OffByWorkspace => (Tone::Neutral, "Off by workspace"),
        CapabilityState::OffByConfig => (Tone::Neutral, "Off by configuration"),
        CapabilityState::Unsupported => (Tone::Neutral, "Not on this desktop"),
        CapabilityState::NeedsApproval => (Tone::Info, "Needs approval"),
        CapabilityState::Denied => (Tone::Warning, "Refused"),
        CapabilityState::Unavailable => (Tone::Warning, "Unavailable"),
        CapabilityState::Paused => (Tone::Warning, "Paused"),
    }
}

/// What the screen does when a button is pressed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConsentAction {
    /// Save this consent (`consent.update`).
    Save(LocalConsent),
    /// Ask the screenshot portal now (`capture.retry_permission`).
    TestScreenshotPermission,
}

/// What is never collected, whatever is chosen.
pub const NEVER_COLLECTED: &str = "Never collected: the keys you press or what you type, \
    passwords, the clipboard, audio, your microphone or camera, or your files.";

/// The toggles of the consent rows, with their badges. Returns whether the
/// draft changed.
pub fn render_rows(ui: &mut egui::Ui, s: &SharedState, model: &mut ConsentViewModel) -> bool {
    let mut changed = false;
    for (i, row) in rows(s, &model.draft).into_iter().enumerate() {
        if i > 0 {
            divider(ui);
        }
        ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing.x = 10.0;
            let available = row.desktop == DesktopSupport::Available;
            let color = if available {
                theme::INK_SOFT
            } else {
                theme::FAINT
            };
            widgets::icon(ui, row.capability.icon(), 18.0, color);
            let toggle_w = 48.0;
            ui.allocate_ui_with_layout(
                Vec2::new((ui.available_width() - toggle_w).max(0.0), 0.0),
                egui::Layout::top_down(egui::Align::Min),
                |ui| {
                    ui.spacing_mut().item_spacing.y = 2.0;
                    ui.label(
                        egui::RichText::new(row.capability.title())
                            .font(theme::semibold(13.0))
                            .color(theme::INK),
                    );
                    widgets::muted(ui, row.capability.detail());
                    ui.horizontal_wrapped(|ui| {
                        ui.spacing_mut().item_spacing = Vec2::new(4.0, 4.0);
                        let (tone, label) = match row.workspace {
                            Some(true) => (Tone::Success, "Workspace records it"),
                            Some(false) => (Tone::Neutral, "Workspace: off"),
                            None => (Tone::Neutral, "Workspace: not known yet"),
                        };
                        widgets::status_pill(ui, tone, label);
                        if !row.config_allows {
                            widgets::status_pill(ui, Tone::Neutral, "Off by configuration");
                        }
                        let (tone, label) = row.desktop.badge();
                        widgets::status_pill(ui, tone, label);
                    });
                    if let Some(note) = row.desktop.note() {
                        widgets::muted(ui, note);
                    }
                },
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
                let mut on = row.chosen;
                if row.selectable {
                    if widgets::toggle(ui, &mut on).changed() {
                        row.capability.set(model, on);
                        changed = true;
                    }
                } else {
                    ui.add_enabled_ui(false, |ui| widgets::toggle(ui, &mut on));
                }
            });
        });
    }
    if model.draft.screenshot {
        divider(ui);
        let mut blur = model.draft.blur_screenshots;
        ui.horizontal(|ui| {
            ui.label(
                egui::RichText::new("Blur screenshots on this computer before upload")
                    .font(theme::regular(12.5))
                    .color(theme::INK_SOFT),
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if widgets::toggle(ui, &mut blur).changed() {
                    model.toggle_blur_screenshots(blur);
                    changed = true;
                }
            });
        });
    }
    changed
}

/// Render the prominent local consent prompt card in the main timer view.
pub fn render_consent_card(
    ui: &mut egui::Ui,
    s: &SharedState,
    model: &mut ConsentViewModel,
    mut on_action: impl FnMut(ConsentAction),
) {
    egui::Frame::new()
        .fill(theme::CARD)
        .stroke(Stroke::new(
            1.0,
            theme::mix(theme::PRIMARY, theme::CARD, 0.4),
        ))
        .corner_radius(CornerRadius::same(theme::CARD_RADIUS))
        .inner_margin(egui::Margin::same(18))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 10.0;

            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 10.0;
                let (rect, _) = ui.allocate_exact_size(Vec2::splat(32.0), Sense::hover());
                ui.painter()
                    .circle_filled(rect.center(), 16.0, theme::tint(theme::PRIMARY));
                widgets::paint_icon(
                    ui.painter(),
                    Rect::from_center_size(rect.center(), Vec2::splat(18.0)),
                    Icon::Info,
                    theme::PRIMARY_STRONG,
                );
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 1.0;
                    ui.label(
                        egui::RichText::new("Choose what this computer may record")
                            .font(theme::semibold(15.0))
                            .color(theme::INK),
                    );
                    widgets::muted(
                        ui,
                        "Everything starts off. Nothing is tracked until you decide.",
                    );
                });
            });

            theme::inset(10).show(ui, |ui| {
                ui.set_width(ui.available_width());
                widgets::muted(ui, NEVER_COLLECTED);
            });

            render_rows(ui, s, model);

            if s.session.kind == SessionKind::Wayland
                && model.draft.screenshot
                && widgets::button(
                    ui,
                    ButtonKind::Secondary,
                    Some(Icon::Camera),
                    "Test screenshot permission",
                    Some(ui.available_width()),
                    true,
                )
                .on_hover_text(
                    "Ask the desktop now, while you are here, whether screenshots may be taken.",
                )
                .clicked()
            {
                on_action(ConsentAction::TestScreenshotPermission);
            }

            ui.add_space(4.0);
            super::equal_row(ui, 3, 8.0, |i, ui| {
                let width = Some(ui.available_width());
                match i {
                    0 => {
                        if widgets::button(ui, ButtonKind::Secondary, None, "Decline", width, true)
                            .clicked()
                        {
                            on_action(ConsentAction::Save(model.decline()));
                        }
                    }
                    1 => {
                        if widgets::button(
                            ui,
                            ButtonKind::Secondary,
                            None,
                            "Time only",
                            width,
                            true,
                        )
                        .on_hover_text("Track time only: nothing about your activity is recorded.")
                        .clicked()
                        {
                            on_action(ConsentAction::Save(model.grant_time_only()));
                        }
                    }
                    _ => {
                        if widgets::button(
                            ui,
                            ButtonKind::Primary,
                            Some(Icon::Check),
                            "Save choices",
                            width,
                            true,
                        )
                        .clicked()
                        {
                            on_action(ConsentAction::Save(model.grant()));
                        }
                    }
                }
            });
        });
}

pub(super) fn divider(ui: &mut egui::Ui) {
    let y = ui.cursor().top();
    ui.painter().hline(
        ui.max_rect().x_range(),
        y,
        egui::Stroke::new(1.0, theme::MUTED),
    );
    ui.add_space(1.0);
}
