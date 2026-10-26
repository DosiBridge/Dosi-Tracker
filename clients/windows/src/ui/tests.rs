use super::layout::{status_visual, sync_status};
use super::window::{away_notification, next_tray_retry, notice_title, notice_tone};
use super::*;
use crate::state::{AwayPrompt, Notice, NoticeKind, UploadIssue};

#[test]
fn without_a_tray_icon_closing_minimizes_and_the_icon_is_tried_again() {
    assert_eq!(close_action(true), CloseAction::HideToTray);
    assert_eq!(close_action(false), CloseAction::Minimize, "never an invisible, unquittable agent");
    let waits: Vec<u64> = std::iter::successors(Some(TRAY_RETRY_FIRST), |w| Some(next_tray_retry(*w)))
        .take(7)
        .map(|w| w.as_secs())
        .collect();
    assert_eq!(waits, [15, 30, 60, 120, 240, 300, 300]);
}

#[test]
fn footer_reports_the_most_important_upload_state() {
    let mut s = SharedState { status: TrackerStatus::Tracking, ..SharedState::default() };
    assert_eq!(sync_status(&s), (Tone::Neutral, "Nothing to sync yet".to_owned()));
    s.pending_uploads = 2;
    assert_eq!(sync_status(&s), (Tone::Warning, "2 blocks waiting to sync".to_owned()));
    // Tracking goes on offline; the footer says where the blocks are.
    s.server_reachable = false;
    assert_eq!(sync_status(&s).1, "Offline — 2 blocks waiting to sync");
    s.status = TrackerStatus::Offline;
    s.pending_uploads = 0;
    assert_eq!(sync_status(&s).1, "Offline — uploads will retry");
    s.rejected_uploads = 1;
    assert_eq!(sync_status(&s), (Tone::Danger, "1 block not accepted by the server".to_owned()));
}

#[test]
fn held_back_uploads_say_why_in_the_footer() {
    let at = Utc::now();
    let mut s = SharedState { status: TrackerStatus::Tracking, pending_uploads: 3, ..SharedState::default() };
    s.upload_issue = Some(UploadIssue::SubscriptionInactive { next_attempt_at: at, message: String::new() });
    assert_eq!(sync_status(&s).1, "Subscription inactive — 3 blocks kept here");
    s.upload_issue = Some(UploadIssue::ClockAhead { next_attempt_at: at });
    assert_eq!(sync_status(&s).1, "Clock ahead — 3 blocks waiting to sync");
    s.upload_issue = None;
    s.pending_uploads = 0;
    s.last_sync = Some(at);
    assert_eq!(sync_status(&s).0, Tone::Success);
}

#[test]
fn a_request_in_flight_shows_before_the_status() {
    assert_eq!(status_visual(TrackerStatus::Tracking, Some(PendingAction::Pausing)), (Tone::Warning, "Pausing…"));
    assert_eq!(status_visual(TrackerStatus::Paused, None), (Tone::Warning, "Paused"));
    assert_eq!(status_visual(TrackerStatus::Stopped, None).0, Tone::Danger);
    assert_eq!(status_visual(TrackerStatus::SignInExpired, None), (Tone::Danger, "Sign-in expired"));
}

#[test]
fn notices_read_with_their_weight() {
    assert_eq!(notice_tone(&NoticeKind::SignInExpired), Tone::Danger);
    assert_eq!(notice_tone(&NoticeKind::QueueReset { moved_to: "x".into() }), Tone::Warning);
    let started = NoticeKind::TrackingStartedAutomatically { project_title: "Website".into() };
    assert_eq!(notice_tone(&started), Tone::Info);
    assert_eq!(notice_title(&started), "Tracking started");
    let notice = Notice { id: 1, at: Utc::now(), kind: started };
    assert!(notice.text().len() < 256, "fits a tray notification");
}

#[test]
fn the_away_notification_quotes_the_span() {
    let from = Utc::now() - chrono::Duration::minutes(42);
    let prompt = AwayPrompt { from, to: from + chrono::Duration::minutes(42), project_title: "Website".into() };
    let text = away_notification(&prompt);
    assert!(text.starts_with("You were away for 42 min ("), "{text}");
    assert!(text.chars().count() < 256);
}

#[test]
fn an_unconfigured_dashboard_next_to_a_real_server_is_not_offered() {
    let shipped = "http://localhost:6005";
    assert!(dashboard_configured(shipped, "https://localhost:6006"), "local development");
    assert!(dashboard_configured(shipped, "http://127.0.0.1:6006"));
    assert!(!dashboard_configured(shipped, "https://tracker.acme.com"));
    assert!(dashboard_configured("https://app.acme.com", "https://api.acme.com"));
    assert!(dashboard_configured("https://app.acme.com", "not a url"));
    assert!(!dashboard_configured("", "https://api.acme.com"));
    assert!(dashboard_configured("http://[::1]:6005", "http://[::1]:6006"));
}

#[test]
fn loopback_hosts_are_recognised() {
    for host in ["localhost", "app.localhost", "127.0.0.1", "127.1.2.3", "[::1]", "::1"] {
        assert!(is_loopback(host), "{host}");
    }
    for host in ["tracker.acme.com", "10.0.0.5", "localhost.acme.com"] {
        assert!(!is_loopback(host), "{host}");
    }
}
