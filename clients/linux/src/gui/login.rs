//! Sign-in, sign-in-expired and keyring-unlock screens; workspace picker.
//!
//! Ported from the Windows agent's `ui/login.rs`. The password goes to the
//! daemon in `auth.sign_in` and is wiped from the form right after; the
//! daemon keeps it only in the keyring, and only when "Remember the
//! password" is on. When no keyring (Secret Service) is running, the form
//! explains the opt-in file store instead of failing silently.

use eframe::egui;
use egui::{Align, Layout, Vec2};
use serde_json::json;
use zeroize::Zeroize;

use super::widgets::{self, ButtonKind, Callout, Icon, Tone};
use super::{format, theme, TrackerApp};
use crate::agent::config::{normalize_server_url, ServerUrlError};
use crate::agent::state::{LoginErrorKind, Secret, SharedState, TrackerStatus, WorkspaceChoice};
use crate::ipc::{method, ErrorBody, ErrorCode, SignInParams};

/// What the sign-in form holds between frames.
pub struct LoginForm {
    pub email: String,
    /// Wiped as soon as it is sent, and when the form goes away.
    pub password: String,
    pub show_password: bool,
    /// The server address (under "Advanced"). It follows the address the
    /// account uses until the user edits it.
    pub server: String,
    pub server_edited: bool,
    /// A workspace to sign in to (name or id); empty lets the server list them.
    pub workspace: String,
    pub remember_password: bool,
    pub advanced_open: bool,
    /// The server the last attempt went to, for "Can't reach …".
    pub attempted_server: Option<String>,
    /// The sign-in error last seen: a new one about the server opens
    /// "Advanced" once.
    pub error_seen: Option<String>,
    /// An answer of `auth.sign_in` that is not in the snapshot (no keyring,
    /// signed in as someone else, a bad request).
    pub reply_error: Option<ErrorBody>,
    /// An expired sign-in: "Use a different account" waits for a confirmation.
    pub confirm_switch: bool,
    /// "Back" closed the workspace picker for these choices.
    pub hidden_choices: Option<Vec<WorkspaceChoice>>,
    /// The password field's undo history still holds typed text.
    forget_typed: bool,
}

/// The password field's id (its undo history is cleared after a sign-in).
const PASSWORD_ID: &str = "dosi-login-password";

impl LoginForm {
    /// "Advanced" starts open when no server is configured on this computer:
    /// the address must be typed.
    pub fn new(configured_server: Option<&str>, remember_password: bool) -> Self {
        Self {
            email: String::new(),
            password: String::new(),
            show_password: false,
            server: configured_server.unwrap_or_default().to_owned(),
            server_edited: false,
            workspace: String::new(),
            remember_password,
            advanced_open: configured_server.is_none_or(|s| s.trim().is_empty()),
            attempted_server: None,
            error_seen: None,
            reply_error: None,
            confirm_switch: false,
            hidden_choices: None,
            forget_typed: false,
        }
    }

    /// Forget the typed password.
    pub fn wipe_password(&mut self) {
        self.password.zeroize();
        self.forget_typed = true;
    }
}

impl Drop for LoginForm {
    fn drop(&mut self) {
        self.wipe_password();
    }
}

/// What a sign-in error is about: it decides the banner's title and what the
/// form offers next.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SignInProblem {
    /// The server address typed cannot be used.
    ServerAddress,
    /// No answer from the server (DNS, connection, certificate, timeout).
    Unreachable,
    /// Signed in, but the account is in no workspace yet.
    NoWorkspace,
    /// The workspace named is archived.
    ArchivedWorkspace,
    /// Too many attempts in a short time.
    Throttled,
    /// The server failed, or answered with something that is not a sign-in.
    ServerError,
    /// The saved sign-in stopped working.
    Expired,
    /// The server refused the email or password; its own words say why.
    Refused,
}

/// How the API client words a 429.
const THROTTLED_PREFIX: &str = "Too many";
/// How the worker words an expired sign-in.
const EXPIRED_PREFIX: &str = "Your sign-in has expired";

impl SignInProblem {
    /// The problem of a failed sign-in: the snapshot's `login_error_kind`
    /// when the daemon gave one, else read from the message.
    pub fn of(kind: Option<LoginErrorKind>, message: &str) -> Self {
        let message = message.trim();
        if message.starts_with(THROTTLED_PREFIX) {
            return SignInProblem::Throttled;
        }
        match kind {
            Some(LoginErrorKind::Refused) => SignInProblem::Refused,
            Some(LoginErrorKind::Offline) => SignInProblem::Unreachable,
            Some(LoginErrorKind::Server) => SignInProblem::ServerError,
            Some(LoginErrorKind::InvalidServer | LoginErrorKind::NoServer) => {
                SignInProblem::ServerAddress
            }
            Some(LoginErrorKind::NoWorkspace) => SignInProblem::NoWorkspace,
            Some(LoginErrorKind::ArchivedWorkspace) => SignInProblem::ArchivedWorkspace,
            Some(LoginErrorKind::Expired) => SignInProblem::Expired,
            None if message.starts_with(EXPIRED_PREFIX) => SignInProblem::Expired,
            None if message == ServerUrlError::Empty.to_string()
                || message == ServerUrlError::Invalid.to_string() =>
            {
                SignInProblem::ServerAddress
            }
            None => SignInProblem::Refused,
        }
    }

    /// The banner's title; empty when the message says it all.
    pub fn title(self) -> &'static str {
        match self {
            SignInProblem::ServerAddress => "Check the server address",
            SignInProblem::Unreachable => "Can't reach the server",
            SignInProblem::NoWorkspace => "No workspace yet",
            SignInProblem::ArchivedWorkspace => "Workspace archived",
            SignInProblem::Throttled => "Too many sign-in attempts",
            SignInProblem::ServerError => "The server couldn't sign you in",
            SignInProblem::Expired => "",
            SignInProblem::Refused => "Sign-in refused",
        }
    }

    pub fn tone(self) -> Tone {
        if matches!(
            self,
            SignInProblem::Expired | SignInProblem::NoWorkspace | SignInProblem::ArchivedWorkspace
        ) {
            Tone::Warning
        } else {
            Tone::Danger
        }
    }

    /// Is the server address the thing to look at?
    pub fn concerns_server(self) -> bool {
        matches!(
            self,
            SignInProblem::ServerAddress | SignInProblem::Unreachable
        )
    }
}

/// How the pickers name a workspace: its name, the user's role, and whether
/// it is the current one.
pub fn workspace_label(choice: &WorkspaceChoice) -> String {
    let name = if choice.name.trim().is_empty() {
        "Unnamed workspace"
    } else {
        choice.name.trim()
    };
    let mut label = name.to_owned();
    if let Some(role) = choice
        .role
        .as_deref()
        .map(str::trim)
        .filter(|r| !r.is_empty())
    {
        label.push_str(&format!("  ·  {role}"));
    }
    if choice.is_current {
        label.push_str("  (current)");
    }
    label
}

/// The explanation shown when no keyring is running.
pub const NO_KEYRING: &str = "No keyring (Secret Service) is running in this session, so \
    Dosi Tracker has nowhere safe to keep your sign-in. Start or unlock GNOME Keyring, \
    KWallet or KeePassXC, or allow the file store with \
    dosi-tracker config set credentials.store file (it keeps tokens in a private file, \
    never the password).";

impl TrackerApp {
    pub(super) fn login_view(&mut self, ui: &mut egui::Ui, snapshot: &SharedState) {
        let full = ui.max_rect();
        let mut content = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(full)
                .layout(Layout::top_down(Align::Min)),
        );
        egui::ScrollArea::vertical()
            .id_salt("login")
            .auto_shrink([false, false])
            .show(&mut content, |ui| {
                let available = ui.available_width();
                let width = (available - 2.0 * theme::PAGE_PAD).clamp(0.0, 380.0);
                let side = ((available - width) / 2.0).max(0.0);
                // Sit a little above centre on tall windows.
                let top = ((full.height() - 600.0) / 3.0).max(theme::PAGE_PAD + 8.0);
                ui.add_space(top);
                ui.horizontal_top(|ui| {
                    ui.add_space(side - ui.spacing().item_spacing.x);
                    ui.vertical(|ui| {
                        ui.set_width(width);
                        ui.spacing_mut().item_spacing.y = theme::GAP;
                        self.toasts(ui);
                        if snapshot.status == TrackerStatus::SignInExpired {
                            self.expired_form(ui, snapshot);
                        } else {
                            self.login_form(ui, snapshot);
                        }
                    });
                });
                ui.add_space(theme::PAGE_PAD);
            });
    }

    /// Follow the server the account uses (until edited), and open
    /// "Advanced" once for a new error about the server.
    fn sync_login_form(&mut self, snapshot: &SharedState) {
        if !self.login.server_edited
            && !snapshot.server_url.trim().is_empty()
            && self.login.server != snapshot.server_url
        {
            self.login.server = snapshot.server_url.clone();
        }
        if snapshot.login_error != self.login.error_seen {
            self.login.error_seen = snapshot.login_error.clone();
            let concerns_server = snapshot
                .login_error
                .as_deref()
                .map(|m| SignInProblem::of(snapshot.login_error_kind, m))
                .is_some_and(SignInProblem::concerns_server);
            if concerns_server {
                self.login.advanced_open = true;
            }
        }
    }

    fn login_form(&mut self, ui: &mut egui::Ui, snapshot: &SharedState) {
        self.sync_login_form(snapshot);
        let hidden = self.login.hidden_choices.as_ref() == Some(&snapshot.workspace_choices);
        if !snapshot.workspace_choices.is_empty() && !hidden {
            theme::card(20).show(ui, |ui| {
                ui.set_width(ui.available_width());
                self.workspace_picker(ui, snapshot);
            });
            return;
        }
        ui.vertical_centered(|ui| {
            theme::logo_mark(ui, 52.0);
            ui.add_space(10.0);
            ui.label(
                egui::RichText::new("Welcome to Dosi Tracker")
                    .font(theme::semibold(21.0))
                    .color(theme::INK),
            );
            ui.add_space(2.0);
            ui.label(
                egui::RichText::new("Sign in to your workspace to start tracking.")
                    .font(theme::regular(13.0))
                    .color(theme::MUTED_FG),
            );
        });
        ui.add_space(6.0);

        if self.no_keyring() {
            widgets::callout(
                ui,
                Tone::Warning,
                &Callout {
                    title: Some("No keyring on this desktop"),
                    body: NO_KEYRING,
                    ..Callout::default()
                },
            );
        }

        let mut submit = false;
        theme::card(20).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 6.0;

            field_label(ui, "Email");
            let email = text_field(ui, &mut self.login.email, "you@company.com");
            submit |= enter_pressed(ui, &email);
            ui.add_space(8.0);

            field_label(ui, "Password");
            submit |= self.password_row(ui);
            ui.add_space(2.0);
            ui.checkbox(
                &mut self.login.remember_password,
                "Remember the password in the keyring",
            )
            .on_hover_text(
                "Kept only in your keyring. Without it, an expired sign-in asks for the password again.",
            );
            ui.add_space(4.0);

            let server = normalize_server_url(&self.login.server);
            submit |= self.advanced(ui, &server);

            if let Some(error) = &snapshot.login_error {
                ui.add_space(4.0);
                self.error_callout(ui, snapshot.login_error_kind, error);
                if SignInProblem::of(snapshot.login_error_kind, error) == SignInProblem::NoWorkspace {
                    let ready = self.dashboard_available();
                    let response = widgets::button(
                        ui,
                        ButtonKind::Secondary,
                        Some(Icon::External),
                        "Open dashboard",
                        Some(ui.available_width()),
                        ready,
                    );
                    let clicked = if ready {
                        response.clicked()
                    } else {
                        response.on_hover_text(super::DASHBOARD_UNSET).clicked()
                    };
                    if clicked {
                        self.open_dashboard();
                    }
                }
            } else if let Some(error) = self.login.reply_error.clone() {
                ui.add_space(4.0);
                reply_callout(ui, &error);
            }

            ui.add_space(10.0);
            let ready = !self.login.email.trim().is_empty()
                && !self.login.password.is_empty()
                && server.is_ok();
            let can_submit = !snapshot.signing_in && ready && self.writable();
            let label = if snapshot.signing_in {
                "Signing in…"
            } else {
                "Sign in"
            };
            if widgets::button(
                ui,
                ButtonKind::Primary,
                None,
                label,
                Some(ui.available_width()),
                can_submit,
            )
            .clicked()
            {
                submit = true;
            }
            if submit && can_submit {
                if let Ok(server) = server {
                    self.login.attempted_server = Some(server.clone());
                    let workspace = self.login.workspace.trim().to_owned();
                    let email = self.login.email.trim().to_owned();
                    let remember = self.login.remember_password;
                    self.sign_in(Some(server), email, (!workspace.is_empty()).then_some(workspace), remember);
                }
            }
        });

        ui.vertical_centered(|ui| {
            ui.label(
                egui::RichText::new("Your workspace admin decides what is captured; you choose what this computer may record.")
                    .font(theme::regular(11.5))
                    .color(theme::MUTED_FG),
            );
        });
    }

    /// Send `auth.sign_in` with the typed password, then wipe it.
    fn sign_in(
        &mut self,
        server: Option<String>,
        email: String,
        workspace: Option<String>,
        remember_password: bool,
    ) {
        self.login.reply_error = None;
        let params = SignInParams {
            server,
            email,
            password: Secret::new(std::mem::take(&mut self.login.password)),
            workspace,
            remember_password: Some(remember_password),
        };
        self.login.wipe_password();
        // The Secret is wiped when `params` drops; the JSON copy is wiped by
        // the IPC client once sent.
        let value = serde_json::to_value(&params).unwrap_or_default();
        drop(params);
        self.link.update_local(|s| s.signing_in = true);
        self.link.call(method::AUTH_SIGN_IN, value);
    }

    /// The password field and its show/hide toggle. Returns whether Enter
    /// was pressed in it.
    fn password_row(&mut self, ui: &mut egui::Ui) -> bool {
        let toggle_w = theme::CONTROL_H;
        let mut entered = false;
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            let id = egui::Id::new(PASSWORD_ID);
            if std::mem::take(&mut self.login.forget_typed) {
                if let Some(mut typed) = egui::text_edit::TextEditState::load(ui.ctx(), id) {
                    typed.clear_undoer();
                    typed.store(ui.ctx(), id);
                }
            }
            let password = ui.add_sized(
                [ui.available_width() - toggle_w - 6.0, theme::CONTROL_H],
                egui::TextEdit::singleline(&mut self.login.password)
                    .id(id)
                    .password(!self.login.show_password)
                    .hint_text("Your password")
                    .font(theme::regular(13.5))
                    .margin(egui::Margin::symmetric(12, 10)),
            );
            entered = enter_pressed(ui, &password);
            let (rect, response) =
                ui.allocate_exact_size(Vec2::splat(toggle_w), egui::Sense::click());
            let hovered = response.hovered();
            ui.painter().rect(
                rect,
                egui::CornerRadius::same(theme::RADIUS),
                if hovered { theme::ACCENT } else { theme::CARD },
                egui::Stroke::new(
                    1.0,
                    if hovered {
                        theme::PRIMARY
                    } else {
                        theme::BORDER_STRONG
                    },
                ),
                egui::StrokeKind::Inside,
            );
            let icon = if self.login.show_password {
                Icon::EyeOff
            } else {
                Icon::Eye
            };
            widgets::paint_icon(
                ui.painter(),
                egui::Rect::from_center_size(rect.center(), Vec2::splat(17.0)),
                icon,
                theme::INK_SOFT,
            );
            if response
                .on_hover_cursor(egui::CursorIcon::PointingHand)
                .on_hover_text(if self.login.show_password {
                    "Hide password"
                } else {
                    "Show password"
                })
                .clicked()
            {
                self.login.show_password = !self.login.show_password;
            }
        });
        entered
    }

    /// "Advanced": the server address and a workspace. Returns whether Enter
    /// was pressed in a field.
    fn advanced(&mut self, ui: &mut egui::Ui, server: &Result<String, ServerUrlError>) -> bool {
        let summary = if self.login.server.trim().is_empty() {
            "Server: not set".to_owned()
        } else {
            format!("Server: {}", format::server_label(&self.login.server))
        };
        if widgets::disclosure(ui, self.login.advanced_open, "Advanced", Some(&summary)).clicked() {
            self.login.advanced_open = !self.login.advanced_open;
        }
        if !self.login.advanced_open {
            return false;
        }
        field_label(ui, "Server address");
        let field = text_field(ui, &mut self.login.server, "https://tracker.example.com");
        if field.changed() {
            self.login.server_edited = true;
        }
        let mut entered = enter_pressed(ui, &field);
        match server {
            Err(e) if self.login.server_edited => {
                ui.add(
                    egui::Label::new(
                        egui::RichText::new(e.to_string())
                            .font(theme::regular(11.5))
                            .color(theme::DANGER_ON_TINT),
                    )
                    .wrap(),
                );
            }
            // A bare host becomes https://…: say where it will go.
            Ok(normalized) if normalized != self.login.server.trim() => {
                widgets::muted(ui, &format!("Signs in to {normalized}"));
            }
            _ => {
                widgets::muted(
                    ui,
                    "The address your admin gave you — it is shown on the web dashboard's Download page.",
                );
            }
        }
        ui.add_space(4.0);
        field_label(ui, "Workspace (optional)");
        let workspace = text_field(
            ui,
            &mut self.login.workspace,
            "Name or id; empty lists yours",
        );
        entered |= enter_pressed(ui, &workspace);
        entered
    }

    fn error_callout(&self, ui: &mut egui::Ui, kind: Option<LoginErrorKind>, error: &str) {
        let problem = SignInProblem::of(kind, error);
        let tried = self
            .login
            .attempted_server
            .as_deref()
            .map(|server| format!("Tried {server}"));
        let detail = match problem {
            SignInProblem::Unreachable => tried.as_deref(),
            _ => None,
        };
        let title = problem.title();
        widgets::callout(
            ui,
            problem.tone(),
            &Callout {
                title: (!title.is_empty()).then_some(title),
                body: error,
                detail,
                action: None,
            },
        );
    }

    /// "Choose a workspace" after signing in to an account in several.
    pub(super) fn workspace_picker(&mut self, ui: &mut egui::Ui, snapshot: &SharedState) {
        ui.spacing_mut().item_spacing.y = 6.0;
        ui.label(
            egui::RichText::new("Choose a workspace")
                .font(theme::semibold(15.0))
                .color(theme::INK),
        );
        widgets::muted(
            ui,
            "Your account is in more than one workspace. You can switch later in Settings.",
        );
        ui.add_space(4.0);
        let mut chosen = None;
        for (index, choice) in snapshot.workspace_choices.iter().enumerate() {
            let label = workspace_label(choice);
            let enabled = !choice.is_current && !snapshot.signing_in && self.writable();
            if widgets::button(
                ui,
                ButtonKind::Secondary,
                None,
                &label,
                Some(ui.available_width()),
                enabled,
            )
            .clicked()
            {
                chosen = Some(index);
            }
        }
        ui.add_space(4.0);
        let back = ui
            .vertical_centered(|ui| widgets::link(ui, "Back", theme::PRIMARY_STRONG).clicked())
            .inner;
        if let Some(index) = chosen {
            self.link.call(
                method::WORKSPACE_CHOOSE,
                json!({ "index": index, "at": chrono::Utc::now() }),
            );
        } else if back {
            self.login.hidden_choices = Some(snapshot.workspace_choices.clone());
        }
    }

    /// The saved sign-in stopped working: ask for the password again, for
    /// the same account — or let someone else sign in.
    fn expired_form(&mut self, ui: &mut egui::Ui, snapshot: &SharedState) {
        ui.vertical_centered(|ui| {
            theme::logo_mark(ui, 52.0);
            ui.add_space(10.0);
            ui.label(
                egui::RichText::new("Sign-in expired")
                    .font(theme::semibold(21.0))
                    .color(theme::INK),
            );
            ui.add_space(2.0);
            ui.add(
                egui::Label::new(
                    egui::RichText::new(
                        "Enter your password to keep tracking. Your recorded time is safe on this computer.",
                    )
                    .font(theme::regular(13.0))
                    .color(theme::MUTED_FG),
                )
                .wrap(),
            );
        });
        ui.add_space(6.0);

        if snapshot.credentials_locked {
            let unlock = widgets::callout(
                ui,
                Tone::Warning,
                &Callout {
                    title: Some("Your keyring is locked"),
                    body: "Unlocking it may be enough: the saved sign-in is read again afterwards.",
                    action: Some("Unlock the keyring"),
                    ..Callout::default()
                },
            );
            if unlock && self.writable() {
                self.link.call(method::AUTH_UNLOCK_KEYRING, json!({}));
            }
        }

        let mut submit = false;
        theme::card(20).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 6.0;

            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 12.0;
                widgets::avatar(ui, &snapshot.display_name, 40.0);
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 1.0;
                    ui.add_space(2.0);
                    ui.add(
                        egui::Label::new(
                            egui::RichText::new(&snapshot.display_name)
                                .font(theme::semibold(14.0))
                                .color(theme::INK),
                        )
                        .truncate(),
                    );
                    let mut account = Vec::new();
                    if !snapshot.workspace.trim().is_empty() {
                        account.push(format!("Workspace: {}", snapshot.workspace.trim()));
                    }
                    if !snapshot.server_url.trim().is_empty() {
                        account.push(format::server_label(&snapshot.server_url));
                    }
                    ui.add(
                        egui::Label::new(
                            egui::RichText::new(account.join(" · "))
                                .font(theme::regular(12.0))
                                .color(theme::MUTED_FG),
                        )
                        .truncate(),
                    );
                });
            });
            ui.add_space(6.0);

            field_label(ui, "Password");
            submit |= self.password_row(ui);

            if let Some(error) = &snapshot.login_error {
                ui.add_space(4.0);
                self.error_callout(ui, snapshot.login_error_kind, error);
            } else if let Some(error) = self.login.reply_error.clone() {
                ui.add_space(4.0);
                reply_callout(ui, &error);
            }

            ui.add_space(10.0);
            let can_submit =
                !snapshot.signing_in && !self.login.password.is_empty() && self.writable();
            let label = if snapshot.signing_in {
                "Signing in…"
            } else {
                "Sign in"
            };
            if widgets::button(
                ui,
                ButtonKind::Primary,
                None,
                label,
                Some(ui.available_width()),
                can_submit,
            )
            .clicked()
            {
                submit = true;
            }
            if submit && can_submit {
                self.login.attempted_server =
                    Some(snapshot.server_url.clone()).filter(|s| !s.trim().is_empty());
                // The server and workspace saved with this account are used again.
                let remember = self.login.remember_password;
                self.sign_in(
                    None,
                    snapshot.display_name.trim().to_owned(),
                    None,
                    remember,
                );
            }
            ui.add_space(4.0);
            self.switch_account(ui, snapshot);
        });
    }

    /// "Use a different account": signing out leaves this account's unsent
    /// work on the computer, so it is confirmed first.
    fn switch_account(&mut self, ui: &mut egui::Ui, snapshot: &SharedState) {
        if !self.login.confirm_switch {
            ui.vertical_centered(|ui| {
                if widgets::link(ui, "Use a different account", theme::PRIMARY_STRONG).clicked() {
                    self.login.confirm_switch = true;
                }
            });
            return;
        }
        theme::inset(12).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(
                egui::RichText::new("Sign out of this account?")
                    .font(theme::semibold(13.0))
                    .color(theme::INK),
            );
            let waiting = snapshot.pending_uploads;
            let detail = if waiting > 0 {
                format!(
                    "{} recorded for {} stay on this computer. They upload if this account signs in again within 14 days.",
                    format::count(waiting, "block"),
                    snapshot.display_name
                )
            } else {
                "Then sign in with another email.".to_owned()
            };
            widgets::muted(ui, &detail);
            ui.add_space(4.0);
            let mut cancel = false;
            let mut confirm = false;
            super::equal_row(ui, 2, 8.0, |i, ui| {
                let width = Some(ui.available_width());
                if i == 0 {
                    cancel = widgets::button(ui, ButtonKind::Secondary, None, "Cancel", width, true)
                        .clicked();
                } else {
                    confirm = widgets::button(
                        ui,
                        ButtonKind::Danger,
                        Some(Icon::SignOut),
                        "Sign out",
                        width,
                        true,
                    )
                    .clicked();
                }
            });
            if cancel {
                self.login.confirm_switch = false;
            }
            if confirm {
                self.login.confirm_switch = false;
                self.login.email.clear();
                self.login.wipe_password();
                self.link
                    .call(method::AUTH_SIGN_OUT, json!({ "at": chrono::Utc::now() }));
            }
        });
    }
}

/// A refused `auth.sign_in` whose reason is not in the snapshot.
fn reply_callout(ui: &mut egui::Ui, error: &ErrorBody) {
    let (title, body) = match error.code {
        ErrorCode::PermissionDenied => ("No keyring on this desktop", NO_KEYRING.to_owned()),
        ErrorCode::Precondition => ("Already signed in", error.message.clone()),
        _ => ("Sign-in failed", error.message.clone()),
    };
    let hint = error.hint.as_deref().map(|h| format!("Hint: {h}"));
    widgets::callout(
        ui,
        Tone::Danger,
        &Callout {
            title: Some(title),
            body: &body,
            detail: hint.as_deref(),
            action: None,
        },
    );
}

fn field_label(ui: &mut egui::Ui, text: &str) {
    ui.label(
        egui::RichText::new(text)
            .font(theme::semibold(12.0))
            .color(theme::INK_SOFT),
    );
}

/// A full-width single-line field in the form's style.
fn text_field(ui: &mut egui::Ui, value: &mut String, hint: &str) -> egui::Response {
    ui.add_sized(
        [ui.available_width(), theme::CONTROL_H],
        egui::TextEdit::singleline(value)
            .hint_text(hint)
            .font(theme::regular(13.5))
            .margin(egui::Margin::symmetric(12, 10)),
    )
}

/// Enter in a field submits the form.
fn enter_pressed(ui: &egui::Ui, field: &egui::Response) -> bool {
    field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter))
}
