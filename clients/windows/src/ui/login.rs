//! The sign-in screen — and, when the server stops accepting the saved
//! password, the same screen asking for the password alone.

use eframe::egui;
use egui::{Align, Layout, Vec2};

use super::widgets::{self, ButtonKind, Callout, Icon, Tone};
use super::{format, TrackerApp};
use crate::api::{classify_token_failure, AuthError};
use crate::config::{check_transport, normalize_server_url, AppConfig, ServerUrlError};
use crate::state::{Command, SharedState, TrackerStatus, WorkspaceChoice};
use crate::theme;

/// What the sign-in form holds between frames.
pub(super) struct LoginForm {
    pub email: String,
    pub password: String,
    pub show_password: bool,
    /// The server address (under "Advanced"). It follows the address the
    /// account uses until the user edits it.
    pub server: String,
    pub server_edited: bool,
    pub advanced_open: bool,
    /// The server the last attempt went to, for "Can't reach …".
    pub attempted_server: Option<String>,
    /// The sign-in error last seen: a new one about the server opens
    /// "Advanced" once.
    pub error_seen: Option<String>,
    /// An expired sign-in: "Use a different account" waits for a confirmation.
    pub confirm_switch: bool,
}

impl LoginForm {
    /// "Advanced" starts open when no server was ever set on this computer
    /// (no `config.toml`, no installer option): the address must be typed.
    pub fn new(configured_server: &str) -> Self {
        Self {
            email: String::new(),
            password: String::new(),
            show_password: false,
            server: String::new(),
            server_edited: false,
            advanced_open: configured_server == AppConfig::default().api_base_url,
            attempted_server: None,
            error_seen: None,
            confirm_switch: false,
        }
    }
}

/// What a sign-in error is about: it decides the banner's title and what
/// the form offers next. Read from the messages the API client and the
/// worker produce (the tests below build them with the same functions).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum SignInProblem {
    /// The server address typed cannot be used.
    ServerAddress,
    /// No answer from the server (DNS, connection, certificate, timeout).
    Unreachable,
    /// Signed in, but the account is in no workspace yet.
    NoWorkspace,
    /// Too many attempts in a short time.
    Throttled,
    /// The server failed, or answered with something that is not a sign-in.
    ServerError,
    /// The saved sign-in stopped working (the worker's own message).
    Expired,
    /// A workspace admin removed this computer; it was signed out.
    DeviceRemoved,
    /// The server refused the email or password; its own words say why
    /// (wrong password, locked out, not allowed).
    Refused,
}

/// A 429 worded with its `Retry-After`.
const THROTTLED_PREFIX: &str = "Too many attempts";
const SERVER_ERROR_PREFIXES: [&str; 3] =
    ["The server had a problem", "Sign-in failed (HTTP", "The server sent an unexpected sign-in response"];
/// How `Worker::expire` words it.
const EXPIRED_PREFIX: &str = "Your sign-in has expired";
const SCHEME_PREFIX: &str = "The server address must start with";

impl SignInProblem {
    pub fn of(message: &str) -> Self {
        let message = message.trim();
        if message == AuthError::Offline(String::new()).user_message() {
            SignInProblem::Unreachable
        } else if message == classify_token_failure(429, "").user_message() || message.starts_with(THROTTLED_PREFIX) {
            SignInProblem::Throttled
        } else if message == crate::worker::NO_WORKSPACE || message == crate::worker::ALL_ARCHIVED {
            SignInProblem::NoWorkspace
        } else if message == crate::state::DEVICE_REVOKED_TEXT {
            SignInProblem::DeviceRemoved
        } else if SERVER_ERROR_PREFIXES.iter().any(|prefix| message.starts_with(prefix)) {
            SignInProblem::ServerError
        } else if message.starts_with(EXPIRED_PREFIX) {
            SignInProblem::Expired
        } else if message.starts_with(SCHEME_PREFIX)
            || message == ServerUrlError::Empty.to_string()
            || message == ServerUrlError::Invalid.to_string()
            || message == ServerUrlError::Insecure.to_string()
        {
            SignInProblem::ServerAddress
        } else {
            SignInProblem::Refused
        }
    }

    /// The banner's title; empty when the message says it all.
    pub fn title(self) -> &'static str {
        match self {
            SignInProblem::ServerAddress => "Check the server address",
            SignInProblem::Unreachable => "Can't reach the server",
            SignInProblem::NoWorkspace => "No workspace yet",
            SignInProblem::Throttled => "Too many sign-in attempts",
            SignInProblem::ServerError => "The server couldn't sign you in",
            SignInProblem::Expired => "",
            SignInProblem::DeviceRemoved => "This computer was removed",
            SignInProblem::Refused => "Sign-in refused",
        }
    }

    pub fn tone(self) -> Tone {
        if matches!(self, SignInProblem::Expired | SignInProblem::NoWorkspace) {
            Tone::Warning
        } else {
            Tone::Danger
        }
    }

    /// Is the server address the thing to look at?
    pub fn concerns_server(self) -> bool {
        matches!(self, SignInProblem::ServerAddress | SignInProblem::Unreachable)
    }
}

impl TrackerApp {
    pub(super) fn login_view(&mut self, ui: &mut egui::Ui, snapshot: &SharedState) {
        let full = ui.max_rect();
        let mut content = ui.new_child(egui::UiBuilder::new().max_rect(full).layout(Layout::top_down(Align::Min)));
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
        if !self.login.server_edited {
            let server = if snapshot.server_url.trim().is_empty() { &self.links.api_base_url } else { &snapshot.server_url };
            if self.login.server != *server {
                self.login.server = server.clone();
            }
        }
        if snapshot.login_error != self.login.error_seen {
            self.login.error_seen = snapshot.login_error.clone();
            if snapshot.login_error.as_deref().map(SignInProblem::of).is_some_and(SignInProblem::concerns_server) {
                self.login.advanced_open = true;
            }
        }
    }

    fn login_form(&mut self, ui: &mut egui::Ui, snapshot: &SharedState) {
        self.sync_login_form(snapshot);
        if !snapshot.workspace_choices.is_empty() {
            theme::card(20).show(ui, |ui| {
                ui.set_width(ui.available_width());
                self.workspace_picker(ui, snapshot);
            });
            return;
        }
        ui.vertical_centered(|ui| {
            theme::logo_mark(ui, 52.0);
            ui.add_space(10.0);
            ui.label(egui::RichText::new("Welcome to Dosi Tracker").font(theme::semibold(21.0)).color(theme::INK));
            ui.add_space(2.0);
            ui.label(
                egui::RichText::new("Sign in to your workspace to start tracking.")
                    .font(theme::regular(13.0))
                    .color(theme::MUTED_FG),
            );
        });
        ui.add_space(6.0);

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
            ui.add_space(6.0);

            // Refused here already as the worker would: never http:// to
            // another computer without the admin's opt-in.
            let allow_insecure_http = self.links.allow_insecure_http;
            let server = normalize_server_url(&self.login.server)
                .and_then(|url| check_transport(&url, allow_insecure_http).map(|()| url));
            submit |= self.advanced(ui, &server);

            if let Some(error) = &snapshot.login_error {
                ui.add_space(4.0);
                self.error_callout(ui, error);
                if SignInProblem::of(error) == SignInProblem::NoWorkspace {
                    let ready = self.dashboard_available(snapshot);
                    let response = widgets::button(ui, ButtonKind::Secondary, Some(Icon::External), "Open dashboard", Some(ui.available_width()), ready);
                    let clicked = if ready { response.clicked() } else { response.on_hover_text(super::DASHBOARD_UNSET).clicked() };
                    if clicked {
                        self.open_dashboard(snapshot);
                    }
                }
            }

            ui.add_space(10.0);
            let ready = !self.login.email.trim().is_empty() && !self.login.password.is_empty() && server.is_ok();
            let can_submit = !snapshot.signing_in && ready;
            let label = if snapshot.signing_in { "Signing in…" } else { "Sign in" };
            if widgets::button(ui, ButtonKind::Primary, None, label, Some(ui.available_width()), can_submit).clicked() {
                submit = true;
            }
            if submit && can_submit {
                if let Ok(server) = server {
                    self.login.attempted_server = Some(server.clone());
                    self.send(Command::SignIn {
                        // The server lists this account's workspaces.
                        workspace: None,
                        username: self.login.email.trim().to_string(),
                        password: self.login.password.clone(),
                        // Saved with the sign-in when it differs from the configured one.
                        server: Some(server),
                    });
                    self.login.password.clear();
                }
            }
        });

        ui.vertical_centered(|ui| {
            ui.label(
                egui::RichText::new("Your workspace admin decides what is captured.")
                    .font(theme::regular(11.5))
                    .color(theme::MUTED_FG),
            );
        });
    }

    /// The password field and its show/hide toggle. Returns whether Enter
    /// was pressed in it.
    fn password_row(&mut self, ui: &mut egui::Ui) -> bool {
        let toggle_w = theme::CONTROL_H;
        let mut entered = false;
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            let password = ui.add_sized(
                [ui.available_width() - toggle_w - 6.0, theme::CONTROL_H],
                egui::TextEdit::singleline(&mut self.login.password)
                    .password(!self.login.show_password)
                    .hint_text("Your password")
                    .font(theme::regular(13.5))
                    .margin(egui::Margin::symmetric(12, 10)),
            );
            entered = enter_pressed(ui, &password);
            let (rect, response) = ui.allocate_exact_size(Vec2::splat(toggle_w), egui::Sense::click());
            let hovered = response.hovered();
            ui.painter().rect(
                rect,
                egui::CornerRadius::same(theme::RADIUS),
                if hovered { theme::ACCENT } else { theme::CARD },
                egui::Stroke::new(1.0, if hovered { theme::PRIMARY } else { theme::BORDER_STRONG }),
                egui::StrokeKind::Inside,
            );
            let icon = if self.login.show_password { Icon::EyeOff } else { Icon::Eye };
            widgets::paint_icon(ui.painter(), egui::Rect::from_center_size(rect.center(), Vec2::splat(17.0)), icon, theme::INK_SOFT);
            if response
                .on_hover_cursor(egui::CursorIcon::PointingHand)
                .on_hover_text(if self.login.show_password { "Hide password" } else { "Show password" })
                .clicked()
            {
                self.login.show_password = !self.login.show_password;
            }
        });
        entered
    }

    /// "Advanced": the server address. Returns whether Enter was pressed in it.
    fn advanced(&mut self, ui: &mut egui::Ui, server: &Result<String, ServerUrlError>) -> bool {
        let summary = format!("Server: {}", format::server_label(&self.login.server));
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
        let entered = enter_pressed(ui, &field);
        match server {
            Err(e) if self.login.server_edited => {
                ui.add(egui::Label::new(egui::RichText::new(e.to_string()).font(theme::regular(11.5)).color(theme::DANGER_ON_TINT)).wrap());
            }
            // Allowed by the admin, but say what it means.
            Ok(normalized) if crate::config::is_unencrypted_remote(normalized) => {
                let warning = format!("Not encrypted: {normalized} can be read on the network.");
                ui.add(egui::Label::new(egui::RichText::new(warning).font(theme::regular(11.5)).color(theme::DANGER_ON_TINT)).wrap());
            }
            // A bare host becomes https://…: say where it will go.
            Ok(normalized) if normalized != self.login.server.trim() => {
                widgets::muted(ui, &format!("Signs in to {normalized}"));
            }
            _ => {
                widgets::muted(ui, "The address your admin gave you — it is shown on the web dashboard's Download page.");
            }
        }
        entered
    }

    fn error_callout(&self, ui: &mut egui::Ui, error: &str) {
        let problem = SignInProblem::of(error);
        let tried = self.login.attempted_server.as_deref().map(|server| format!("Tried {server}"));
        let detail = match problem {
            SignInProblem::Unreachable => tried.as_deref(),
            _ => None,
        };
        let title = problem.title();
        widgets::callout(
            ui,
            problem.tone(),
            &Callout { title: (!title.is_empty()).then_some(title), body: error, detail, action: None },
        );
    }

    /// "Choose a workspace" after signing in to an account in several.
    pub(super) fn workspace_picker(&mut self, ui: &mut egui::Ui, snapshot: &SharedState) {
        ui.spacing_mut().item_spacing.y = 6.0;
        ui.label(egui::RichText::new("Choose a workspace").font(theme::semibold(15.0)).color(theme::INK));
        widgets::muted(ui, "Your account is in more than one workspace. You can switch later in Settings.");
        ui.add_space(4.0);
        let mut chosen = None;
        for (index, choice) in snapshot.workspace_choices.iter().enumerate() {
            let label = workspace_label(choice);
            let enabled = !choice.is_current && !snapshot.signing_in;
            if widgets::button(ui, ButtonKind::Secondary, None, &label, Some(ui.available_width()), enabled).clicked() {
                chosen = Some(index);
            }
        }
        ui.add_space(4.0);
        let back = ui.vertical_centered(|ui| widgets::link(ui, "Back", theme::PRIMARY_STRONG).clicked()).inner;
        if let Some(index) = chosen {
            self.send(Command::ChooseWorkspace { index });
        } else if back {
            self.send(Command::CancelWorkspaceChoice);
        }
    }

    /// The saved sign-in stopped working: ask for the password again, for the
    /// same account — or let someone else sign in.
    fn expired_form(&mut self, ui: &mut egui::Ui, snapshot: &SharedState) {
        ui.vertical_centered(|ui| {
            theme::logo_mark(ui, 52.0);
            ui.add_space(10.0);
            ui.label(egui::RichText::new("Sign-in expired").font(theme::semibold(21.0)).color(theme::INK));
            ui.add_space(2.0);
            ui.add(
                egui::Label::new(
                    egui::RichText::new("Enter your password to keep tracking. Your recorded time is safe on this computer.")
                        .font(theme::regular(13.0))
                        .color(theme::MUTED_FG),
                )
                .wrap(),
            );
        });
        ui.add_space(6.0);

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
                    ui.add(egui::Label::new(egui::RichText::new(&snapshot.display_name).font(theme::semibold(14.0)).color(theme::INK)).truncate());
                    let mut account = Vec::new();
                    if !snapshot.workspace.trim().is_empty() {
                        account.push(format!("Workspace: {}", snapshot.workspace.trim()));
                    }
                    if !snapshot.server_url.trim().is_empty() {
                        account.push(format::server_label(&snapshot.server_url));
                    }
                    ui.add(egui::Label::new(egui::RichText::new(account.join(" · ")).font(theme::regular(12.0)).color(theme::MUTED_FG)).truncate());
                });
            });
            ui.add_space(6.0);

            field_label(ui, "Password");
            submit |= self.password_row(ui);

            if let Some(error) = &snapshot.login_error {
                ui.add_space(4.0);
                self.error_callout(ui, error);
            }

            ui.add_space(10.0);
            let can_submit = !snapshot.signing_in && !self.login.password.is_empty();
            let label = if snapshot.signing_in { "Signing in…" } else { "Sign in" };
            if widgets::button(ui, ButtonKind::Primary, None, label, Some(ui.available_width()), can_submit).clicked() {
                submit = true;
            }
            if submit && can_submit {
                self.login.attempted_server = Some(snapshot.server_url.clone()).filter(|s| !s.trim().is_empty());
                self.send(Command::SignIn {
                    // The workspace saved with this account is picked again.
                    workspace: None,
                    username: snapshot.display_name.trim().to_string(),
                    password: self.login.password.clone(),
                    // The server saved with this account.
                    server: None,
                });
                self.login.password.clear();
            }
            ui.add_space(4.0);
            self.switch_account(ui, snapshot);
        });
    }

    /// "Use a different account": signing out leaves this account's
    /// unsent work on the computer, so it is confirmed first.
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
            ui.label(egui::RichText::new("Sign out of this account?").font(theme::semibold(13.0)).color(theme::INK));
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
                    cancel = widgets::button(ui, ButtonKind::Secondary, None, "Cancel", width, true).clicked();
                } else {
                    confirm = widgets::button(ui, ButtonKind::Danger, Some(Icon::SignOut), "Sign out", width, true).clicked();
                }
            });
            if cancel {
                self.login.confirm_switch = false;
            }
            if confirm {
                self.login.confirm_switch = false;
                self.login.email.clear();
                self.login.password.clear();
                self.send(Command::SignOut { at: chrono::Utc::now() });
            }
        });
    }
}

/// How the pickers name a workspace: its name, the user's role, and whether
/// it is the current one.
pub(super) fn workspace_label(choice: &WorkspaceChoice) -> String {
    let name = if choice.name.trim().is_empty() { "Unnamed workspace" } else { choice.name.trim() };
    let mut label = name.to_owned();
    if let Some(role) = choice.role.as_deref().map(str::trim).filter(|r| !r.is_empty()) {
        label.push_str(&format!("  ·  {role}"));
    }
    if choice.is_current {
        label.push_str("  (current)");
    }
    label
}

fn field_label(ui: &mut egui::Ui, text: &str) {
    ui.label(egui::RichText::new(text).font(theme::semibold(12.0)).color(theme::INK_SOFT));
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_failure_class_of_the_api_client_is_told_apart() {
        let of = |error: AuthError| SignInProblem::of(&error.user_message());
        assert_eq!(of(AuthError::Offline("tcp connect error".into())), SignInProblem::Unreachable);
        assert_eq!(of(classify_token_failure(429, "")), SignInProblem::Throttled);
        assert_eq!(of(classify_token_failure(503, "")), SignInProblem::ServerError);
        assert_eq!(of(classify_token_failure(404, "")), SignInProblem::ServerError);
        assert_eq!(SignInProblem::of(crate::worker::NO_WORKSPACE), SignInProblem::NoWorkspace);
        let refused = r#"{"error":"invalid_grant","error_description":"Invalid username or password!"}"#;
        assert_eq!(of(classify_token_failure(400, refused)), SignInProblem::Refused);
        let locked = r#"{"error":"invalid_grant","error_description":"The user account has been locked out."}"#;
        assert_eq!(of(classify_token_failure(400, locked)), SignInProblem::Refused);
        assert_eq!(of(classify_token_failure(401, "")), SignInProblem::Refused);
        assert_eq!(SignInProblem::of("The server sent an unexpected sign-in response (EOF)."), SignInProblem::ServerError);
    }

    #[test]
    fn server_address_mistakes_open_the_server_field() {
        for error in [
            ServerUrlError::Empty,
            ServerUrlError::Invalid,
            ServerUrlError::UnsupportedScheme("ftp".into()),
            ServerUrlError::Insecure,
        ] {
            let problem = SignInProblem::of(&error.to_string());
            assert_eq!(problem, SignInProblem::ServerAddress, "{error}");
            assert!(problem.concerns_server());
        }
        assert!(SignInProblem::Unreachable.concerns_server());
        assert!(!SignInProblem::Refused.concerns_server());
    }

    #[test]
    fn an_expired_sign_in_is_a_warning_with_the_servers_reason() {
        let message = "Your sign-in has expired (Invalid username or password!). Enter your password to continue — your tracked time is kept.";
        let problem = SignInProblem::of(message);
        assert_eq!(problem, SignInProblem::Expired);
        assert_eq!(problem.tone(), Tone::Warning);
        assert!(problem.title().is_empty(), "the message says it all");
        assert_eq!(SignInProblem::Refused.tone(), Tone::Danger);
    }

    #[test]
    fn a_removed_computer_and_archived_workspaces_are_named() {
        let removed = SignInProblem::of(crate::state::DEVICE_REVOKED_TEXT);
        assert_eq!((removed, removed.title(), removed.tone()), (SignInProblem::DeviceRemoved, "This computer was removed", Tone::Danger));
        assert!(!removed.concerns_server());
        assert_eq!(SignInProblem::of(crate::worker::ALL_ARCHIVED), SignInProblem::NoWorkspace);
    }

    #[test]
    fn throttled_workspace_lookups_read_as_throttling() {
        assert_eq!(SignInProblem::of("Too many attempts, try again in 42 s."), SignInProblem::Throttled);
        assert_eq!(SignInProblem::of("Invalid email or password."), SignInProblem::Refused);
    }

    #[test]
    fn the_picker_names_the_role_and_the_current_workspace() {
        let choice = |name: &str, role: Option<&str>, is_current| WorkspaceChoice { name: name.into(), role: role.map(Into::into), is_current };
        assert_eq!(workspace_label(&choice("Acme", None, false)), "Acme");
        assert_eq!(workspace_label(&choice("Acme", Some("Owner"), false)), "Acme  ·  Owner");
        assert_eq!(workspace_label(&choice("Globex", Some("Member"), true)), "Globex  ·  Member  (current)");
        assert_eq!(workspace_label(&choice(" ", Some(" "), false)), "Unnamed workspace");
    }

    #[test]
    fn advanced_starts_open_only_when_no_server_was_set() {
        assert!(LoginForm::new(&AppConfig::default().api_base_url).advanced_open);
        assert!(!LoginForm::new("https://tracker.acme.com").advanced_open);
    }
}
