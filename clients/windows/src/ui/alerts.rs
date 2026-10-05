//! What needs the user's attention, as banners at the top of the Timer tab:
//! tracking that stopped, input that cannot be measured, uploads held back
//! (an inactive subscription, a clock ahead of the server's, a server out of
//! reach) and blocks the server did not accept. Decided here from the state
//! alone, so the rules are tested without a window.

use crate::state::{SharedState, TrackerStatus, UploadIssue};

use super::format;
use super::widgets::Tone;

/// A server's error text is quoted at most this long.
const MAX_DETAIL_CHARS: usize = 160;

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
}

impl AlertAction {
    pub fn label(self) -> &'static str {
        match self {
            AlertAction::SyncNow => "Try again now",
            AlertAction::ShowActivity => "See why",
        }
    }
}

/// The banners for `s`, most serious first. `input_healthy` says whether the
/// input hooks deliver (they are read live, not from the state).
pub fn alerts(s: &SharedState, input_healthy: bool) -> Vec<Alert> {
    let mut list = Vec::new();

    if s.status == TrackerStatus::Stopped {
        list.push(Alert {
            tone: Tone::Danger,
            title: "Tracking has stopped".to_owned(),
            body: s
                .last_error
                .clone()
                .unwrap_or_else(|| "Tracking stopped unexpectedly. Please restart Dosi Tracker.".to_owned()),
            detail: None,
            action: None,
        });
    } else if let Some(error) = s.last_error.as_deref() {
        // Not a member of any project: the project chooser says so itself.
        let explained = s.status == TrackerStatus::ChooseProject && s.projects.is_empty();
        if !explained {
            list.push(Alert { tone: Tone::Warning, title: String::new(), body: error.to_owned(), detail: None, action: None });
        }
    }

    let measures_input = s.capture.keyboard || s.capture.mouse;
    if !input_healthy && measures_input && s.status == TrackerStatus::Tracking {
        list.push(Alert {
            tone: Tone::Warning,
            title: "Keyboard and mouse activity can't be measured".to_owned(),
            body: "Windows isn't passing input to Dosi Tracker right now. Your time is still recorded; its activity shows \
                   as not measured until this recovers."
                .to_owned(),
            detail: None,
            action: None,
        });
    }

    if let Some(UploadIssue::SubscriptionInactive { next_attempt_at, message }) = &s.upload_issue {
        list.push(Alert {
            tone: Tone::Warning,
            title: "Workspace subscription inactive".to_owned(),
            body: format!(
                "Your work is kept on this computer and uploads once the subscription is renewed. Next try at {}.",
                format::local_hm(*next_attempt_at)
            ),
            detail: quoted(message),
            action: None,
        });
    }

    let clock_ahead_issue = match &s.upload_issue {
        Some(UploadIssue::ClockAhead { next_attempt_at }) => Some(*next_attempt_at),
        _ => None,
    };
    if s.clock_skew_minutes.is_some() || clock_ahead_issue.is_some() {
        let title = match s.clock_skew_minutes {
            Some(minutes) if minutes < 0 => format!("Your computer's clock is {} ahead", format::minutes_words(-minutes)),
            Some(minutes) => format!("Your computer's clock is {} behind", format::minutes_words(minutes)),
            None => "Your computer's clock is ahead".to_owned(),
        };
        let mut body = "Times are recorded as this computer shows them. Turn on \u{201c}Set the time automatically\u{201d} in \
                        Windows Settings \u{2192} Time & language."
            .to_owned();
        if let Some(next) = clock_ahead_issue {
            body.push_str(&format!(
                " Uploads wait until the server's time catches up — next try at {}.",
                format::local_hm(next)
            ));
        }
        list.push(Alert { tone: Tone::Warning, title, body, detail: None, action: None });
    }

    if let Some(UploadIssue::Retrying { next_attempt_at, reason }) = &s.upload_issue {
        let title = if s.server_reachable { "Uploads are waiting" } else { "Offline — your work is kept on this computer" };
        list.push(Alert {
            tone: Tone::Warning,
            title: title.to_owned(),
            body: format!(
                "Blocks upload by themselves once the server answers. Next try at {}.",
                format::local_hm(*next_attempt_at)
            ),
            detail: quoted(reason).map(|reason| format!("Last error: {reason}")),
            action: Some(AlertAction::SyncNow),
        });
    }

    if s.rejected_uploads > 0 {
        let blocks = format::count(s.rejected_uploads, "block");
        let (verb, body) = if s.rejected_uploads == 1 {
            ("wasn't", "It stays on this computer for 14 days for reference but is never uploaded.")
        } else {
            ("weren't", "They stay on this computer for 14 days for reference but are never uploaded.")
        };
        list.push(Alert {
            tone: Tone::Warning,
            title: format!("{blocks} {verb} accepted by the server"),
            body: body.to_owned(),
            detail: None,
            action: Some(AlertAction::ShowActivity),
        });
    }

    list
}

/// A server's text, trimmed and kept short; `None` when there is none.
fn quoted(text: &str) -> Option<String> {
    let text = text.trim();
    (!text.is_empty()).then(|| format::truncate_chars(text, MAX_DETAIL_CHARS))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::{CaptureFlags, ProjectOption};
    use chrono::Utc;

    fn tracking() -> SharedState {
        SharedState {
            status: TrackerStatus::Tracking,
            capture: CaptureFlags { keyboard: true, mouse: true, ..CaptureFlags::default() },
            ..SharedState::default()
        }
    }

    fn titles(list: &[Alert]) -> Vec<&str> {
        list.iter().map(|a| a.title.as_str()).collect()
    }

    #[test]
    fn a_healthy_tracker_has_nothing_to_say() {
        assert!(alerts(&tracking(), true).is_empty());
    }

    #[test]
    fn a_stopped_queue_is_explained_first() {
        let s = SharedState {
            status: TrackerStatus::Stopped,
            last_error: Some("The local queue cannot be opened (disk full).".into()),
            rejected_uploads: 2,
            ..SharedState::default()
        };
        let list = alerts(&s, true);
        assert_eq!(list[0].tone, Tone::Danger);
        assert_eq!(list[0].title, "Tracking has stopped");
        assert!(list[0].body.contains("disk full"));
        assert_eq!(list.len(), 2, "the error is not repeated as a second banner");
    }

    #[test]
    fn missing_input_is_reported_only_while_it_matters() {
        let list = alerts(&tracking(), false);
        assert_eq!(titles(&list), ["Keyboard and mouse activity can't be measured"]);
        // The project records neither: nothing to measure.
        let neither = SharedState { capture: CaptureFlags { keyboard: false, mouse: false, ..CaptureFlags::default() }, ..tracking() };
        assert!(alerts(&neither, false).is_empty());
        // Paused: nothing is recorded anyway.
        assert!(alerts(&SharedState { status: TrackerStatus::Paused, ..tracking() }, false).is_empty());
    }

    #[test]
    fn an_inactive_subscription_says_the_work_is_kept() {
        let s = SharedState {
            upload_issue: Some(UploadIssue::SubscriptionInactive {
                next_attempt_at: Utc::now(),
                message: "The trial has ended.".into(),
            }),
            ..tracking()
        };
        let list = alerts(&s, true);
        assert_eq!(list[0].title, "Workspace subscription inactive");
        assert!(list[0].body.contains("kept on this computer"));
        assert_eq!(list[0].detail.as_deref(), Some("The trial has ended."));
    }

    #[test]
    fn the_clock_warning_says_how_far_and_which_way() {
        let ahead = SharedState { clock_skew_minutes: Some(-7), ..tracking() };
        assert_eq!(titles(&alerts(&ahead, true)), ["Your computer's clock is 7 min ahead"]);
        let behind = SharedState { clock_skew_minutes: Some(75), ..tracking() };
        assert_eq!(titles(&alerts(&behind, true)), ["Your computer's clock is 1 h 15 min behind"]);
        // A 425 from the server with no offset known yet: one banner, with the retry.
        let refused = SharedState { upload_issue: Some(UploadIssue::ClockAhead { next_attempt_at: Utc::now() }), ..tracking() };
        let list = alerts(&refused, true);
        assert_eq!(titles(&list), ["Your computer's clock is ahead"]);
        assert!(list[0].body.contains("Uploads wait"));
    }

    #[test]
    fn waiting_uploads_quote_the_error_and_offer_a_retry() {
        let s = SharedState {
            server_reachable: false,
            upload_issue: Some(UploadIssue::Retrying { next_attempt_at: Utc::now(), reason: "HTTP 503".into() }),
            ..tracking()
        };
        let list = alerts(&s, true);
        assert_eq!(list[0].title, "Offline — your work is kept on this computer");
        assert_eq!(list[0].detail.as_deref(), Some("Last error: HTTP 503"));
        assert_eq!(list[0].action, Some(AlertAction::SyncNow));
        let online = SharedState { server_reachable: true, ..s };
        assert_eq!(alerts(&online, true)[0].title, "Uploads are waiting");
    }

    #[test]
    fn rejected_blocks_point_to_the_reasons() {
        let one = SharedState { rejected_uploads: 1, ..tracking() };
        let list = alerts(&one, true);
        assert_eq!(list[0].title, "1 block wasn't accepted by the server");
        assert!(list[0].body.starts_with("It stays"));
        assert_eq!(list[0].action, Some(AlertAction::ShowActivity));
        let three = SharedState { rejected_uploads: 3, ..tracking() };
        let list = alerts(&three, true);
        assert_eq!(list[0].title, "3 blocks weren't accepted by the server");
        assert!(list[0].body.starts_with("They stay"));
    }

    #[test]
    fn other_errors_show_as_they_are_unless_the_chooser_explains_them() {
        let s = SharedState { last_error: Some("Could not save locally: disk full".into()), ..tracking() };
        let list = alerts(&s, true);
        assert_eq!(list[0].body, "Could not save locally: disk full");
        assert!(list[0].title.is_empty());

        let no_projects = SharedState {
            status: TrackerStatus::ChooseProject,
            last_error: Some("You are not a member of any active project.".into()),
            ..SharedState::default()
        };
        assert!(alerts(&no_projects, true).is_empty());
        let with_projects = SharedState {
            projects: vec![ProjectOption { id: "p".into(), title: "P".into() }],
            ..no_projects
        };
        assert_eq!(alerts(&with_projects, true).len(), 1);
    }

    #[test]
    fn long_server_text_is_cut() {
        let s = SharedState {
            upload_issue: Some(UploadIssue::Retrying { next_attempt_at: Utc::now(), reason: "x".repeat(500) }),
            ..tracking()
        };
        let detail = alerts(&s, true)[0].detail.clone().unwrap();
        assert!(detail.chars().count() <= "Last error: ".len() + MAX_DETAIL_CHARS);
    }
}
