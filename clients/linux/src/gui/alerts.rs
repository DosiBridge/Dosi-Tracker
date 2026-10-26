//! Alert banners (port of the Windows `ui/alerts.rs` plus Linux alerts).
//!
//! What needs the user's attention, as banners at the top of the Timer tab:
//! tracking that stopped, a device removed from the workspace, no desktop
//! session, a locked keyring, input or screenshots that cannot be recorded,
//! Wayland's limits, manual mode, uploads held back (an inactive
//! subscription, a clock ahead of the server's, a server out of reach),
//! blocks the server did not accept, no tray icon and an agent that was
//! updated. Decided here from the snapshot alone (design 5.1), so the rules
//! are tested without a window.

use crate::agent::state::{CapabilityState, SessionKind, SharedState, TrackerStatus, UploadIssue};

use super::format;
use super::widgets::Tone;

/// A server's error text is quoted at most this long.
pub const MAX_DETAIL_CHARS: usize = 160;

/// One banner.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Alert {
    pub tone: Tone,
    /// Empty for a banner that is only its message.
    pub title: String,
    pub body: String,
    /// A quieter line: the server's own words.
    pub detail: Option<String>,
    pub action: Option<AlertAction>,
}

/// What a banner's link does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AlertAction {
    SyncNow,
    /// The Activity tab lists each rejected block with the server's reason.
    ShowActivity,
    /// Settings: switch workspace or sign out.
    ShowSettings,
    /// The keyring's own unlock prompt (`auth.unlock_keyring`).
    Unlock,
    /// Ask the screenshot portal again (`capture.retry_permission`).
    RetryScreenshot,
}

impl AlertAction {
    pub fn label(self) -> &'static str {
        match self {
            AlertAction::SyncNow => "Try again now",
            AlertAction::ShowActivity => "See why",
            AlertAction::ShowSettings => "Choose a workspace",
            AlertAction::Unlock => "Unlock",
            AlertAction::RetryScreenshot => "Ask again",
        }
    }
}

fn alert(tone: Tone, title: &str, body: impl Into<String>, action: Option<AlertAction>) -> Alert {
    Alert {
        tone,
        title: title.to_owned(),
        body: body.into(),
        detail: None,
        action,
    }
}

/// The banners for `s`, most serious first.
pub fn alerts(s: &SharedState) -> Vec<Alert> {
    let mut list = Vec::new();
    let signed_in = !matches!(s.status, TrackerStatus::SignedOut);

    match s.status {
        TrackerStatus::Stopped => list.push(alert(
            Tone::Danger,
            "Tracking has stopped",
            s.last_error.clone().unwrap_or_else(|| {
                "Tracking stopped unexpectedly. Restart it with dosi-tracker service restart."
                    .to_owned()
            }),
            None,
        )),
        TrackerStatus::DeviceRevoked => {
            let workspace = if s.workspace.trim().is_empty() {
                "the workspace".to_owned()
            } else {
                s.workspace.trim().to_owned()
            };
            list.push(alert(
                Tone::Danger,
                &format!("This computer was removed from {workspace}"),
                "Tracking and uploads stopped there. Its unsent blocks stay on this computer \
                 for 14 days. Choose another workspace or sign out.",
                Some(AlertAction::ShowSettings),
            ));
        }
        TrackerStatus::NoSession => list.push(alert(
            Tone::Warning,
            "No desktop session",
            "Dosi Tracker is not attached to a graphical (X11 or Wayland) login, so nothing \
             is tracked. Sign in to the desktop and tracking can start there.",
            None,
        )),
        _ => {}
    }
    if s.status != TrackerStatus::Stopped {
        if let Some(error) = s.last_error.as_deref() {
            // Not a member of any project: the project chooser says so itself.
            let explained = s.status == TrackerStatus::ChooseProject && s.projects.is_empty();
            if !explained {
                list.push(alert(Tone::Warning, "", error, None));
            }
        }
    }

    if s.credentials_locked {
        list.push(alert(
            Tone::Warning,
            "Your keyring is locked",
            "Tracking goes on and blocks are kept on this computer; uploads wait until the \
             keyring is unlocked.",
            Some(AlertAction::Unlock),
        ));
    }

    let unavailable = |state: CapabilityState| state == CapabilityState::Unavailable;
    if s.status == TrackerStatus::Tracking
        && (unavailable(s.capabilities.keyboard.state) || unavailable(s.capabilities.mouse.state))
    {
        list.push(alert(
            Tone::Warning,
            "Keyboard and mouse activity can't be measured",
            "The desktop isn't passing input events to Dosi Tracker right now. Your time is \
             still recorded; its activity shows as not measured until this recovers.",
            None,
        ));
    }

    match s.capabilities.screenshot.state {
        CapabilityState::NeedsApproval => list.push(alert(
            Tone::Info,
            "Screenshots need your permission",
            "The desktop asks whether Dosi Tracker may take screenshots. Until you allow it, \
             blocks are recorded without them.",
            Some(AlertAction::RetryScreenshot),
        )),
        CapabilityState::Denied => list.push(alert(
            Tone::Warning,
            "Screenshot permission refused",
            "Screenshots were refused in the desktop's dialog, so blocks are recorded without \
             them. Nothing more is asked until you try again.",
            Some(AlertAction::RetryScreenshot),
        )),
        _ => {}
    }

    if s.session.kind == SessionKind::Wayland {
        let c = &s.capabilities;
        let missing = [&c.keyboard, &c.mouse, &c.active_window]
            .iter()
            .any(|cap| cap.consented && cap.state == CapabilityState::Unsupported);
        if missing {
            list.push(alert(
                Tone::Info,
                "Some activity isn't available on Wayland",
                WAYLAND_LIMITS,
                None,
            ));
        }
    }

    if s.manual_mode && signed_in {
        list.push(alert(
            Tone::Info,
            "No idle detection on this desktop",
            "This desktop does not say when you step away, so tracking never starts by itself \
             and pauses after a long stretch. Pause when you leave the computer.",
            None,
        ));
    }

    if let Some(UploadIssue::SubscriptionInactive {
        next_attempt_at,
        message,
    }) = &s.upload_issue
    {
        list.push(Alert {
            detail: quoted(message),
            ..alert(
                Tone::Warning,
                "Workspace subscription inactive",
                format!(
                    "Your work is kept on this computer and uploads once the subscription is \
                     renewed. Next try at {}.",
                    format::local_hm(*next_attempt_at)
                ),
                None,
            )
        });
    }

    let clock_ahead_issue = match &s.upload_issue {
        Some(UploadIssue::ClockAhead { next_attempt_at }) => Some(*next_attempt_at),
        _ => None,
    };
    if s.clock_skew_minutes.is_some() || clock_ahead_issue.is_some() {
        let title = match s.clock_skew_minutes {
            Some(minutes) if minutes < 0 => format!(
                "Your computer's clock is {} ahead",
                format::minutes_words(-minutes)
            ),
            Some(minutes) => format!(
                "Your computer's clock is {} behind",
                format::minutes_words(minutes)
            ),
            None => "Your computer's clock is ahead".to_owned(),
        };
        let mut body = "Times are recorded as this computer shows them. Turn on automatic time \
                        synchronisation (for example timedatectl set-ntp true)."
            .to_owned();
        if let Some(next) = clock_ahead_issue {
            body.push_str(&format!(
                " Uploads wait until the server's time catches up — next try at {}.",
                format::local_hm(next)
            ));
        }
        list.push(alert(Tone::Warning, &title, body, None));
    }

    if let Some(UploadIssue::Retrying {
        next_attempt_at,
        reason,
    }) = &s.upload_issue
    {
        let title = if s.server_reachable {
            "Uploads are waiting"
        } else {
            "Offline — your work is kept on this computer"
        };
        list.push(Alert {
            detail: quoted(reason).map(|reason| format!("Last error: {reason}")),
            ..alert(
                Tone::Warning,
                title,
                format!(
                    "Blocks upload by themselves once the server answers. Next try at {}.",
                    format::local_hm(*next_attempt_at)
                ),
                Some(AlertAction::SyncNow),
            )
        });
    }

    if s.rejected_uploads > 0 {
        let blocks = format::count(s.rejected_uploads, "block");
        let (verb, body) = if s.rejected_uploads == 1 {
            (
                "wasn't",
                "It stays on this computer for 14 days for reference but is never uploaded.",
            )
        } else {
            (
                "weren't",
                "They stay on this computer for 14 days for reference but are never uploaded.",
            )
        };
        list.push(alert(
            Tone::Warning,
            &format!("{blocks} {verb} accepted by the server"),
            body,
            Some(AlertAction::ShowActivity),
        ));
    }

    if signed_in && !s.indicator.tray {
        let body = if s.indicator.notifications {
            "A notification shows while tracking instead. On GNOME, the AppIndicator \
             extension adds the icon."
        } else {
            "Nothing on the desktop shows that tracking is on; keep this window or \
             dosi-tracker status at hand."
        };
        list.push(alert(
            Tone::Neutral,
            "No tray icon on this desktop",
            body,
            None,
        ));
    }

    if let Some(daemon) = &s.daemon {
        if daemon.version != env!("CARGO_PKG_VERSION") {
            list.push(alert(
                Tone::Info,
                "Dosi Tracker was updated",
                format!(
                    "The agent runs version {} and restarts with the new one at the end of \
                     the current block.",
                    daemon.version
                ),
                None,
            ));
        }
    }

    list
}

/// What Wayland cannot record, in the consent screen's words.
pub const WAYLAND_LIMITS: &str = "Keyboard and mouse counts and the active window are not \
    available on Wayland. Dosi Tracker never uses root or input devices to get around this.";

/// A server's text, trimmed and kept short; `None` when there is none.
fn quoted(text: &str) -> Option<String> {
    let text = text.trim();
    (!text.is_empty()).then(|| format::truncate_chars(text, MAX_DETAIL_CHARS))
}
