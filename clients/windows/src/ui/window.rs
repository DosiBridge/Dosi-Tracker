//! The window and the tray: showing, hiding and quitting, the tray's menu,
//! a tray icon that could not be created, and the worker's notices.

use std::time::{Duration, Instant};

use chrono::Utc;
use eframe::egui;

use crate::shell;
use crate::state::{AwayPrompt, Command, NoticeKind, SharedState, TrackerStatus};
use crate::tray::{Tray, TrayAction, TrayEvent};

use super::widgets::Tone;
use super::{format, TrackerApp, View};

/// A tray icon that could not be created (Explorer not ready at autostart)
/// is tried again after this, doubling…
pub(super) const TRAY_RETRY_FIRST: Duration = Duration::from_secs(15);
/// …up to this.
pub(super) const TRAY_RETRY_MAX: Duration = Duration::from_secs(5 * 60);

/// The wait before the next attempt at the tray icon, after `wait`.
pub(super) fn next_tray_retry(wait: Duration) -> Duration {
    (wait * 2).clamp(TRAY_RETRY_FIRST, TRAY_RETRY_MAX)
}

/// What the window's close button (X) does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum CloseAction {
    /// Hide to the tray: tracking goes on, the icon brings it back.
    HideToTray,
    /// No tray icon to come back from: minimize to the taskbar instead, so the
    /// agent never becomes invisible (and unquittable) while it tracks.
    Minimize,
}

pub(super) fn close_action(has_tray: bool) -> CloseAction {
    if has_tray {
        CloseAction::HideToTray
    } else {
        CloseAction::Minimize
    }
}

impl TrackerApp {
    /// A window that started off-screen (an autostart into the tray) is
    /// centred the first time it is shown, however that happens.
    pub(super) fn recenter_if_needed(&mut self, ctx: &egui::Context) {
        if std::mem::take(&mut self.window.recenter_on_show) {
            let size = ctx
                .input(|i| i.viewport().outer_rect)
                .map(|r| [r.width(), r.height()])
                .unwrap_or(crate::DEFAULT_WINDOW_SIZE);
            if let Some(area) = shell::work_area() {
                ctx.send_viewport_cmd(egui::ViewportCommand::OuterPosition(area.centered(size).into()));
            }
        }
    }

    pub(super) fn show_window(&mut self, ctx: &egui::Context) {
        self.recenter_if_needed(ctx);
        ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
        ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
        ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
        self.window.hidden = false;
        // Paint straight away: a window shown from the tray must never sit
        // blank until the mouse happens to move over it.
        ctx.request_repaint();
    }

    /// Without a tray icon (Explorer was not ready when the app started at
    /// sign-in, or it restarted), try to create it again now and then.
    pub(super) fn retry_tray(&mut self, ctx: &egui::Context) {
        let Some((at, wait)) = self.tray_retry else { return };
        if Instant::now() < at {
            ctx.request_repaint_after(at - Instant::now());
            return;
        }
        match Tray::build(self.tray_sender.clone(), ctx.clone()) {
            Ok(tray) => {
                tracing::info!("the tray icon is up");
                self.tray = Some(tray);
                self.tray_retry = None;
            }
            Err(e) => {
                let wait = next_tray_retry(wait);
                tracing::warn!(?e, retry_in_secs = wait.as_secs(), "the tray icon still cannot be created");
                self.tray_retry = Some((Instant::now() + wait, wait));
            }
        }
    }

    pub(super) fn hide_window(&mut self, ctx: &egui::Context) {
        ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
        self.window.hidden = true;
    }

    pub(super) fn quit(&mut self, ctx: &egui::Context) {
        if !self.quitting {
            self.quitting = true;
            self.send(Command::Shutdown { at: Utc::now() });
        }
        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        ctx.request_repaint();
    }

    pub(super) fn on_tray_event(&mut self, ctx: &egui::Context, event: TrayEvent, snapshot: &SharedState) {
        match event {
            TrayEvent::IconClicked | TrayEvent::Action(TrayAction::Open) => self.show_window(ctx),
            // Notifications are about the timer: the time-away question, a
            // project to choose, tracking that started.
            TrayEvent::NotificationClicked => {
                self.view = View::Timer;
                self.show_window(ctx);
            }
            TrayEvent::Action(TrayAction::ToggleTracking) => match snapshot.status {
                TrackerStatus::SignedOut | TrackerStatus::SignInExpired | TrackerStatus::ChooseProject => {
                    self.view = View::Timer;
                    self.show_window(ctx);
                }
                TrackerStatus::Paused => self.resume(snapshot),
                _ => self.pause(snapshot, None),
            },
            TrayEvent::Action(TrayAction::Break(duration)) => self.pause(snapshot, Some(duration)),
            TrayEvent::Action(TrayAction::SelectProject(id)) => {
                if snapshot.selected_project.as_deref() != Some(id.as_str()) {
                    self.send(Command::SelectProject { project_id: id, at: Utc::now() });
                }
            }
            TrayEvent::Action(TrayAction::SetTask) => {
                self.view = View::Timer;
                self.task.focus_requested = true;
                self.show_window(ctx);
            }
            TrayEvent::Action(TrayAction::SyncNow) => self.send(Command::SyncNow { at: Utc::now() }),
            TrayEvent::Action(TrayAction::OpenDashboard) => self.open_dashboard(snapshot),
            TrayEvent::Action(TrayAction::OpenSettings) => {
                self.view = View::Settings;
                self.show_window(ctx);
            }
            TrayEvent::Action(TrayAction::ToggleAutostart) => self.set_autostart(!self.autostart_enabled),
            TrayEvent::Action(TrayAction::Quit) => self.quit(ctx),
        }
        if let Some(tray) = &mut self.tray {
            tray.invalidate();
        }
    }

    /// Tell the user what the worker reported, once: in the window, and —
    /// while the window is not in front — from the tray as well. The
    /// time-away question is announced from the tray the same way (the
    /// window shows it until it is answered).
    pub(super) fn deliver_notices(&mut self, snapshot: &SharedState, in_front: bool) {
        let fresh: Vec<_> = snapshot.notices.iter().filter(|n| n.id > self.notices_seen).cloned().collect();
        for notice in fresh {
            self.notices_seen = notice.id;
            let tone = notice_tone(&notice.kind);
            let text = notice.text();
            if !in_front && self.prefs.show_notifications {
                let warning = matches!(tone, Tone::Warning | Tone::Danger);
                if let Some(tray) = &mut self.tray {
                    tray.notify(notice_title(&notice.kind), &text, warning);
                }
            }
            // An expired sign-in (or a removed computer) replaces the window
            // with the sign-in form, which says why.
            if !matches!(notice.kind, NoticeKind::SignInExpired | NoticeKind::DeviceRevoked) {
                self.show_toast(tone, text);
            }
        }

        if let Some(prompt) = &snapshot.away_prompt {
            let key = (prompt.from, prompt.to);
            if self.away_announced != Some(key) {
                self.away_announced = Some(key);
                if !in_front && self.prefs.show_notifications {
                    if let Some(tray) = &mut self.tray {
                        tray.notify("Welcome back", &away_notification(prompt), false);
                    }
                }
            }
        }
    }
}

/// How a worker notice looks in the window.
pub(super) fn notice_tone(kind: &NoticeKind) -> Tone {
    match kind {
        NoticeKind::TrackingStartedAutomatically { .. } | NoticeKind::ChooseProject => Tone::Info,
        NoticeKind::ProjectUnavailable { .. } | NoticeKind::QueueReset { .. } | NoticeKind::WorkspaceLost { .. } => Tone::Warning,
        NoticeKind::SignInExpired | NoticeKind::DeviceRevoked => Tone::Danger,
    }
}

/// A tray notification's title.
pub(super) fn notice_title(kind: &NoticeKind) -> &'static str {
    match kind {
        NoticeKind::TrackingStartedAutomatically { .. } => "Tracking started",
        NoticeKind::ChooseProject => "Choose a project",
        NoticeKind::ProjectUnavailable { .. } => "Project no longer available",
        NoticeKind::QueueReset { .. } => "Local queue reset",
        NoticeKind::SignInExpired => "Sign-in expired",
        NoticeKind::WorkspaceLost { refusal: crate::api::WorkspaceRefusal::Archived, .. } => "Workspace archived",
        NoticeKind::WorkspaceLost { refusal: crate::api::WorkspaceRefusal::Required, .. } => "Workspace not received",
        NoticeKind::WorkspaceLost { .. } => "Workspace access removed",
        NoticeKind::DeviceRevoked => "Computer removed",
    }
}

/// The tray's version of the time-away question.
pub(super) fn away_notification(prompt: &AwayPrompt) -> String {
    format!(
        "You were away for {}. Open Dosi Tracker to keep it as work — otherwise it isn't recorded.",
        format::away_summary(prompt.from, prompt.to)
    )
}
