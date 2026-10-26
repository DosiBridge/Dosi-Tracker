#![cfg(all(target_os = "linux", feature = "gui"))]
//! The window's pure parts, ported from the Windows agent's UI tests: the
//! alert rules, the consent view model (with this desktop's availability),
//! formatting, sign-in problems, the activity helpers and the tray's
//! `TrayView` — plus the link to a fake daemon (hello, subscribe, requests,
//! reconnect). Nothing here opens a window.

mod support;

use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use chrono::{DateTime, FixedOffset, Utc};
use dosi_tracker_linux::agent::state::{
    AwayPrompt, CapabilityInfo, CapabilityState, CaptureFlags, DaemonInfo, IndicatorInfo,
    LoginErrorKind, PendingAction, PlatformAvailability, ProjectOption, QueuedRow, RecentBlock,
    RowStatus, SessionKind, SessionSummary, SharedState, TrackerStatus, UploadIssue, UploadState,
    WorkspaceChoice,
};
use dosi_tracker_linux::agent::tray::{Badge, TrayView};
use dosi_tracker_linux::consent::{ConsentedPermissions, LocalConsent};
use dosi_tracker_linux::gui::alerts::{alerts, Alert, AlertAction, MAX_DETAIL_CHARS};
use dosi_tracker_linux::gui::consent::{
    desktop_support, rows, state_badge, Capability, CapabilityStatus, ConsentViewModel,
    DesktopSupport,
};
use dosi_tracker_linux::gui::link::{Link, LinkStatus};
use dosi_tracker_linux::gui::login::{workspace_label, SignInProblem};
use dosi_tracker_linux::gui::widgets::Tone;
use dosi_tracker_linux::gui::{activity, format, settings, timer};
use serde_json::json;

fn at(s: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc)
}

/// Tracking on X11 with a tray, every input kind measured.
fn tracking() -> SharedState {
    let on = CapabilityInfo::evaluate(
        true,
        true,
        true,
        &PlatformAvailability::Available,
        false,
        Some("xi2"),
    );
    let mut s = SharedState {
        status: TrackerStatus::Tracking,
        display_name: "ayesha@acme.test".into(),
        capture: CaptureFlags {
            keyboard: true,
            mouse: true,
            ..CaptureFlags::default()
        },
        indicator: IndicatorInfo {
            tray: true,
            notifications: true,
        },
        daemon: Some(DaemonInfo {
            version: env!("CARGO_PKG_VERSION").into(),
            pid: 1,
        }),
        session: SessionSummary {
            kind: SessionKind::X11,
            ..SessionSummary::default()
        },
        ..SharedState::default()
    };
    s.capabilities.keyboard = on.clone();
    s.capabilities.mouse = on;
    s
}

fn titles(list: &[Alert]) -> Vec<&str> {
    list.iter().map(|a| a.title.as_str()).collect()
}

// ---- Alerts ------------------------------------------------------------------------------

#[test]
fn a_healthy_tracker_has_nothing_to_say() {
    assert!(alerts(&tracking()).is_empty(), "{:?}", alerts(&tracking()));
}

#[test]
fn a_stopped_queue_is_explained_first() {
    let s = SharedState {
        status: TrackerStatus::Stopped,
        last_error: Some("The local queue cannot be opened (disk full).".into()),
        rejected_uploads: 2,
        ..tracking()
    };
    let list = alerts(&s);
    assert_eq!(list[0].tone, Tone::Danger);
    assert_eq!(list[0].title, "Tracking has stopped");
    assert!(list[0].body.contains("disk full"));
    assert_eq!(
        list.len(),
        2,
        "the error is not repeated as a second banner"
    );
}

#[test]
fn unavailable_input_is_reported_only_while_tracking() {
    let mut s = tracking();
    s.capabilities.keyboard = CapabilityInfo::evaluate(
        true,
        true,
        true,
        &PlatformAvailability::Unavailable("XI2 stopped".into()),
        false,
        Some("xi2"),
    );
    assert_eq!(
        titles(&alerts(&s)),
        ["Keyboard and mouse activity can't be measured"]
    );
    s.status = TrackerStatus::Paused;
    assert!(alerts(&s).is_empty(), "paused: nothing is recorded anyway");
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
    let list = alerts(&s);
    assert_eq!(list[0].title, "Workspace subscription inactive");
    assert!(list[0].body.contains("kept on this computer"));
    assert_eq!(list[0].detail.as_deref(), Some("The trial has ended."));
}

#[test]
fn the_clock_warning_says_how_far_and_which_way() {
    let ahead = SharedState {
        clock_skew_minutes: Some(-7),
        ..tracking()
    };
    assert_eq!(
        titles(&alerts(&ahead)),
        ["Your computer's clock is 7 min ahead"]
    );
    assert!(
        alerts(&ahead)[0].body.contains("timedatectl"),
        "Linux words"
    );
    let behind = SharedState {
        clock_skew_minutes: Some(75),
        ..tracking()
    };
    assert_eq!(
        titles(&alerts(&behind)),
        ["Your computer's clock is 1 h 15 min behind"]
    );
    let refused = SharedState {
        upload_issue: Some(UploadIssue::ClockAhead {
            next_attempt_at: Utc::now(),
        }),
        ..tracking()
    };
    let list = alerts(&refused);
    assert_eq!(titles(&list), ["Your computer's clock is ahead"]);
    assert!(list[0].body.contains("Uploads wait"));
}

#[test]
fn waiting_uploads_quote_the_error_and_offer_a_retry() {
    let s = SharedState {
        server_reachable: false,
        upload_issue: Some(UploadIssue::Retrying {
            next_attempt_at: Utc::now(),
            reason: "HTTP 503".into(),
        }),
        ..tracking()
    };
    let list = alerts(&s);
    assert_eq!(
        list[0].title,
        "Offline — your work is kept on this computer"
    );
    assert_eq!(list[0].detail.as_deref(), Some("Last error: HTTP 503"));
    assert_eq!(list[0].action, Some(AlertAction::SyncNow));
    let online = SharedState {
        server_reachable: true,
        ..s
    };
    assert_eq!(alerts(&online)[0].title, "Uploads are waiting");
}

#[test]
fn rejected_blocks_point_to_the_reasons() {
    let one = SharedState {
        rejected_uploads: 1,
        ..tracking()
    };
    let list = alerts(&one);
    assert_eq!(list[0].title, "1 block wasn't accepted by the server");
    assert!(list[0].body.starts_with("It stays"));
    assert_eq!(list[0].action, Some(AlertAction::ShowActivity));
    let three = SharedState {
        rejected_uploads: 3,
        ..tracking()
    };
    assert_eq!(
        alerts(&three)[0].title,
        "3 blocks weren't accepted by the server"
    );
}

#[test]
fn other_errors_show_as_they_are_unless_the_chooser_explains_them() {
    let s = SharedState {
        last_error: Some("Could not save locally: disk full".into()),
        ..tracking()
    };
    let list = alerts(&s);
    assert_eq!(list[0].body, "Could not save locally: disk full");
    assert!(list[0].title.is_empty());

    let no_projects = SharedState {
        status: TrackerStatus::ChooseProject,
        last_error: Some("You are not a member of any active project.".into()),
        ..tracking()
    };
    assert!(alerts(&no_projects).is_empty());
    let with_projects = SharedState {
        projects: vec![ProjectOption {
            id: "p".into(),
            title: "P".into(),
        }],
        ..no_projects
    };
    assert_eq!(alerts(&with_projects).len(), 1);
}

#[test]
fn long_server_text_is_cut() {
    let s = SharedState {
        upload_issue: Some(UploadIssue::Retrying {
            next_attempt_at: Utc::now(),
            reason: "x".repeat(500),
        }),
        ..tracking()
    };
    let detail = alerts(&s)[0].detail.clone().unwrap();
    assert!(detail.chars().count() <= "Last error: ".len() + MAX_DETAIL_CHARS);
}

#[test]
fn linux_states_have_their_own_banners() {
    let revoked = SharedState {
        status: TrackerStatus::DeviceRevoked,
        workspace: "Acme".into(),
        ..tracking()
    };
    let list = alerts(&revoked);
    assert_eq!(list[0].title, "This computer was removed from Acme");
    assert_eq!(list[0].action, Some(AlertAction::ShowSettings));

    let no_session = SharedState {
        status: TrackerStatus::NoSession,
        ..tracking()
    };
    assert_eq!(titles(&alerts(&no_session)), ["No desktop session"]);

    let locked = SharedState {
        credentials_locked: true,
        ..tracking()
    };
    let list = alerts(&locked);
    assert_eq!(list[0].title, "Your keyring is locked");
    assert_eq!(list[0].action, Some(AlertAction::Unlock));

    let manual = SharedState {
        manual_mode: true,
        ..tracking()
    };
    assert_eq!(
        titles(&alerts(&manual)),
        ["No idle detection on this desktop"]
    );
}

#[test]
fn screenshot_permission_banners_offer_to_ask_again() {
    let mut s = tracking();
    s.capabilities.screenshot = CapabilityInfo::evaluate(
        true,
        true,
        true,
        &PlatformAvailability::Denied,
        false,
        Some("portal"),
    );
    let list = alerts(&s);
    assert_eq!(list[0].title, "Screenshot permission refused");
    assert_eq!(list[0].action, Some(AlertAction::RetryScreenshot));
    s.capabilities.screenshot.state = CapabilityState::NeedsApproval;
    assert_eq!(alerts(&s)[0].title, "Screenshots need your permission");
}

#[test]
fn wayland_limits_show_only_for_what_was_chosen() {
    let unsupported = PlatformAvailability::Unsupported("Not on Wayland.".into());
    let mut s = tracking();
    s.session.kind = SessionKind::Wayland;
    s.capabilities.keyboard = CapabilityInfo::evaluate(true, true, true, &unsupported, false, None);
    let list = alerts(&s);
    assert_eq!(titles(&list), ["Some activity isn't available on Wayland"]);
    assert!(list[0].body.contains("never uses root or input devices"));
    // Not chosen: nothing is missing.
    s.capabilities.keyboard =
        CapabilityInfo::evaluate(false, true, true, &unsupported, false, None);
    assert!(alerts(&s).is_empty());
}

#[test]
fn no_tray_and_an_updated_agent_are_mentioned() {
    let mut s = tracking();
    s.indicator.tray = false;
    let list = alerts(&s);
    assert_eq!(titles(&list), ["No tray icon on this desktop"]);
    assert!(list[0].body.contains("notification"));
    s.indicator = IndicatorInfo::default();
    assert!(alerts(&s)[0].body.contains("dosi-tracker status"));
    let mut s = tracking();
    s.daemon = Some(DaemonInfo {
        version: "0.0.1".into(),
        pid: 7,
    });
    assert_eq!(titles(&alerts(&s)), ["Dosi Tracker was updated"]);
}

// ---- Consent -----------------------------------------------------------------------------

#[test]
fn fresh_model_defaults_all_off_when_consent_is_absent() {
    let model = ConsentViewModel::new(LocalConsent::Absent);
    assert!(!model.is_decided() && !model.is_granted() && !model.is_revoked());
    assert_eq!(model.draft, ConsentedPermissions::all_disabled());
}

#[test]
fn fresh_model_defaults_all_off_when_consent_is_revoked() {
    let model = ConsentViewModel::new(LocalConsent::Revoked);
    assert!(model.is_decided() && !model.is_granted() && model.is_revoked());
    assert!(!model.draft.keyboard && !model.draft.screenshot);
}

#[test]
fn grant_saves_exact_granular_selections() {
    let mut model = ConsentViewModel::new(LocalConsent::Absent);
    model.toggle_input_counts(true);
    model.toggle_active_window(false);
    model.toggle_screenshot(true);
    model.toggle_blur_screenshots(true);
    model.toggle_webcam(true);
    match model.grant() {
        LocalConsent::Granted(p) => {
            assert!(p.keyboard && p.mouse);
            assert!(!p.active_window && !p.running_programs);
            assert!(p.screenshot && p.blur_screenshots);
            assert!(!p.webcam, "the webcam cannot be granted on Linux");
        }
        other => panic!("expected Granted, got {other:?}"),
    }
    assert!(model.is_granted() && !model.is_dirty());
}

#[test]
fn capabilities_toggle_one_by_one_and_time_only_grants_nothing() {
    let mut model = ConsentViewModel::new(LocalConsent::Absent);
    model.toggle_keyboard(true);
    model.toggle_running_programs(true);
    assert!(model.draft.keyboard && !model.draft.mouse);
    assert!(model.draft.running_programs && !model.draft.active_window);
    assert!(model.is_dirty());
    assert_eq!(
        model.grant_time_only(),
        LocalConsent::Granted(ConsentedPermissions::all_disabled())
    );
}

#[test]
fn decline_and_revoke_clear_everything() {
    let mut model = ConsentViewModel::new(LocalConsent::Absent);
    model.toggle_input_counts(true);
    assert_eq!(model.decline(), LocalConsent::Revoked);
    assert!(!model.draft.keyboard);
    let mut initial = ConsentedPermissions::all_disabled();
    initial.keyboard = true;
    let mut model = ConsentViewModel::new(LocalConsent::Granted(initial));
    assert_eq!(model.revoke(), LocalConsent::Revoked);
    assert!(model.is_revoked() && !model.draft.keyboard);
}

#[test]
fn capability_effective_status_matrix() {
    assert_eq!(
        ConsentViewModel::capability_status(true, true),
        CapabilityStatus::Active
    );
    assert_eq!(
        ConsentViewModel::capability_status(false, true),
        CapabilityStatus::DeniedByUser
    );
    assert_eq!(
        ConsentViewModel::capability_status(true, false),
        CapabilityStatus::DisabledByProject
    );
    assert_eq!(
        ConsentViewModel::capability_status(false, false),
        CapabilityStatus::Disabled
    );
}

#[test]
fn availability_on_this_desktop_is_said_before_anything_is_chosen() {
    let mut s = SharedState::default();
    s.session.kind = SessionKind::Wayland;
    match desktop_support(&s, Capability::Keyboard) {
        DesktopSupport::Unsupported(why) => {
            assert!(why.contains("not available on Wayland"), "{why}");
            assert!(why.contains("never uses root or input devices"), "{why}");
        }
        other => panic!("{other:?}"),
    }
    assert!(matches!(
        desktop_support(&s, Capability::ActiveWindow),
        DesktopSupport::Unsupported(_)
    ));
    assert_eq!(
        desktop_support(&s, Capability::Screenshot),
        DesktopSupport::Available
    );
    assert_eq!(
        desktop_support(&s, Capability::RunningPrograms),
        DesktopSupport::Available
    );
    match desktop_support(&s, Capability::Webcam) {
        DesktopSupport::Unsupported(why) => assert_eq!(why, "Not available on Linux yet."),
        other => panic!("{other:?}"),
    }
    // What the sensors reported narrows it further.
    s.capabilities.screenshot.state = CapabilityState::Denied;
    assert_eq!(
        desktop_support(&s, Capability::Screenshot),
        DesktopSupport::Denied
    );
    s.session.kind = SessionKind::X11;
    assert_eq!(
        desktop_support(&s, Capability::Keyboard),
        DesktopSupport::Available
    );
}

#[test]
fn rows_carry_the_workspace_policy_and_the_draft() {
    let mut s = tracking();
    s.projects = vec![ProjectOption {
        id: "p1".into(),
        title: "Website".into(),
    }];
    s.selected_project = Some("p1".into());
    s.capabilities.screenshot = CapabilityInfo::evaluate(
        false,
        false,
        true,
        &PlatformAvailability::Available,
        false,
        None,
    );
    let mut draft = ConsentedPermissions::all_disabled();
    draft.keyboard = true;
    let all = rows(&s, &draft);
    assert_eq!(all.len(), 6);
    let keyboard = &all[0];
    assert_eq!(keyboard.capability, Capability::Keyboard);
    assert!(keyboard.chosen && keyboard.selectable);
    assert_eq!(keyboard.workspace, Some(true));
    let screenshot = all
        .iter()
        .find(|r| r.capability == Capability::Screenshot)
        .unwrap();
    assert_eq!(screenshot.workspace, Some(false));
    let webcam = all
        .iter()
        .find(|r| r.capability == Capability::Webcam)
        .unwrap();
    assert!(!webcam.selectable && !webcam.chosen);
    // No project yet: the workspace's policy is not known.
    s.selected_project = None;
    assert_eq!(rows(&s, &draft)[0].workspace, None);
}

#[test]
fn state_badges_name_every_state() {
    assert_eq!(state_badge(CapabilityState::On), (Tone::Success, "On"));
    assert_eq!(
        state_badge(CapabilityState::OffByWorkspace).1,
        "Off by workspace"
    );
    assert_eq!(state_badge(CapabilityState::Denied).0, Tone::Warning);
    assert_eq!(
        state_badge(CapabilityState::Unsupported).1,
        "Not on this desktop"
    );
}

// ---- Formatting --------------------------------------------------------------------------

#[test]
fn clock_reads_like_a_stopwatch() {
    assert_eq!(format::clock(chrono::Duration::seconds(0)), "00:00");
    assert_eq!(format::clock(chrono::Duration::seconds(309)), "05:09");
    assert_eq!(format::clock(chrono::Duration::seconds(3_909)), "1:05:09");
    assert_eq!(format::clock(chrono::Duration::seconds(-4)), "00:00");
}

#[test]
fn minute_labels() {
    assert_eq!(format::minutes_label(0), "0m");
    assert_eq!(format::minutes_label(42), "42m");
    assert_eq!(format::minutes_label(222), "3h 42m");
    assert_eq!(format::secs_label(f64::NAN), "0m");
    assert_eq!(format::secs_label(-30.0), "0m");
    assert_eq!(format::secs_label(3_659.0), "1h 00m");
}

#[test]
fn durations_read_as_words() {
    assert_eq!(format::minutes_words(42), "42 min");
    assert_eq!(format::minutes_words(72), "1 h 12 min");
    assert_eq!(format::minutes_words(180), "3 h");
    assert_eq!(format::minutes_words(-3), "0 min");
}

#[test]
fn time_away_reads_like_the_contract_example() {
    let dhaka = FixedOffset::east_opt(6 * 3600).unwrap();
    let summary = format::away_summary_in(
        at("2026-10-05T04:05:00Z"),
        at("2026-10-05T04:47:30Z"),
        &dhaka,
    );
    assert_eq!(summary, "42 min (10:05 – 10:47)");
    let overnight = format::away_summary_in(
        at("2026-10-05T12:00:00Z"),
        at("2026-10-06T03:00:00Z"),
        &dhaka,
    );
    assert_eq!(overnight, "15 h (Mon 18:00 – Tue 09:00)");
}

#[test]
fn list_times_name_the_day_when_it_is_not_today() {
    let utc = Utc;
    let now = at("2026-10-07T15:00:00Z");
    assert_eq!(
        format::day_and_time_in(at("2026-10-07T09:30:00Z"), now, &utc),
        "09:30"
    );
    assert_eq!(
        format::day_and_time_in(at("2026-10-06T09:30:00Z"), now, &utc),
        "Yesterday 09:30"
    );
    assert_eq!(
        format::day_and_time_in(at("2026-10-04T09:30:00Z"), now, &utc),
        "Sun 09:30"
    );
    assert_eq!(
        format::day_and_time_in(at("2026-09-12T09:30:00Z"), now, &utc),
        "12 Sep 09:30"
    );
    assert_eq!(
        format::day_and_time_in(at("2025-12-30T09:30:00Z"), now, &utc),
        "30 Dec 2025 09:30"
    );
}

#[test]
fn live_totals_add_the_running_block() {
    let now = Utc::now();
    let mut snapshot = SharedState {
        tracked_today_secs: 600.0,
        tracked_week_secs: 3_600.0,
        ..SharedState::default()
    };
    assert_eq!(format::live_totals(&snapshot, now), (600.0, 3_600.0));
    snapshot.block_started_at = Some(now - chrono::Duration::seconds(90));
    let (today, week) = format::live_totals(&snapshot, now);
    assert!(today > 600.0 && today <= 690.0);
    assert!(week > 3_600.0 && week <= 3_690.0);
}

#[test]
fn counts_pluralise_and_long_text_is_cut() {
    assert_eq!(format::count(1, "upload"), "1 upload");
    assert_eq!(format::count(0, "upload"), "0 uploads");
    let cut = format::truncate_chars(&"x".repeat(300), 127);
    assert_eq!(cut.chars().count(), 127);
    assert!(cut.ends_with('…'));
    assert_eq!(
        format::truncate_chars("Checkout page redesign", 10),
        "Checkout…"
    );
}

#[test]
fn offsets_servers_and_levels_read_as_labels() {
    assert_eq!(format::utc_offset_label(360), "UTC+06:00");
    assert_eq!(format::utc_offset_label(-330), "UTC\u{2212}05:30");
    assert_eq!(
        format::server_label("https://tracker.acme.com"),
        "tracker.acme.com"
    );
    assert_eq!(
        format::server_label("http://10.0.0.5:6006/tracker/"),
        "10.0.0.5:6006/tracker"
    );
    assert_eq!(format::percent_label(Some(42)), "42%");
    assert_eq!(format::percent_label(None), "—", "not measured is never 0%");
}

#[test]
fn goal_captions_and_activity_colours() {
    assert_eq!(timer::goal_caption(0.0, 8.0 * 3600.0), "0% of 8h goal");
    assert_eq!(
        timer::goal_caption(3.7 * 3600.0, 8.0 * 3600.0),
        "46% of 8h goal"
    );
    assert_eq!(
        timer::goal_caption(9.0 * 3600.0, 8.0 * 3600.0),
        "Goal reached"
    );
    assert_eq!(timer::goal_caption(1.0, 0.0), "");
    assert_eq!(
        timer::activity_color(85),
        dosi_tracker_linux::gui::theme::SUCCESS
    );
    assert_eq!(
        timer::activity_color(10),
        dosi_tracker_linux::gui::theme::DANGER
    );
}

#[test]
fn the_task_field_shows_wherever_a_next_block_can_follow() {
    for status in [
        TrackerStatus::Tracking,
        TrackerStatus::Paused,
        TrackerStatus::Away,
        TrackerStatus::Offline,
        TrackerStatus::Locked,
        TrackerStatus::NoSession,
    ] {
        assert!(timer::shows_task_field(status), "{status:?}");
    }
    for status in [
        TrackerStatus::SignedOut,
        TrackerStatus::SignInExpired,
        TrackerStatus::Stopped,
        TrackerStatus::ChooseProject,
        TrackerStatus::ConsentRequired,
        TrackerStatus::DeviceRevoked,
    ] {
        assert!(!timer::shows_task_field(status), "{status:?}");
    }
}

#[test]
fn the_block_card_names_only_what_is_measured() {
    let mut s = tracking();
    assert_eq!(
        timer::measured_kinds(&s),
        ["keyboard counts", "mouse click counts"]
    );
    s.capabilities.keyboard.state = CapabilityState::Unsupported;
    s.capabilities.mouse.state = CapabilityState::Unsupported;
    assert!(timer::measured_kinds(&s).is_empty(), "Wayland: time only");
}

// ---- Sign-in -----------------------------------------------------------------------------

#[test]
fn sign_in_problems_follow_the_daemons_error_kind() {
    use SignInProblem as P;
    assert_eq!(P::of(Some(LoginErrorKind::Offline), "x"), P::Unreachable);
    assert_eq!(
        P::of(
            Some(LoginErrorKind::Refused),
            "Invalid username or password!"
        ),
        P::Refused
    );
    assert_eq!(
        P::of(Some(LoginErrorKind::Server), "HTTP 503"),
        P::ServerError
    );
    assert_eq!(
        P::of(
            Some(LoginErrorKind::Server),
            "Too many sign-in attempts. Try again later."
        ),
        P::Throttled
    );
    assert_eq!(
        P::of(Some(LoginErrorKind::InvalidServer), "x"),
        P::ServerAddress
    );
    assert_eq!(
        P::of(Some(LoginErrorKind::NoServer), "Enter the server address."),
        P::ServerAddress
    );
    assert_eq!(
        P::of(Some(LoginErrorKind::NoWorkspace), "x"),
        P::NoWorkspace
    );
    assert_eq!(
        P::of(Some(LoginErrorKind::ArchivedWorkspace), "x"),
        P::ArchivedWorkspace
    );
    assert_eq!(
        P::of(
            None,
            "Your sign-in has expired (refused). Enter your password"
        ),
        P::Expired
    );
    assert_eq!(P::of(None, "Something else"), P::Refused);
    assert!(P::Unreachable.concerns_server() && P::ServerAddress.concerns_server());
    assert!(!P::Refused.concerns_server());
    assert_eq!(P::Expired.tone(), Tone::Warning);
    assert!(P::Expired.title().is_empty(), "the message says it all");
    assert_eq!(P::Refused.tone(), Tone::Danger);
}

#[test]
fn the_picker_names_the_role_and_the_current_workspace() {
    let choice = |name: &str, role: Option<&str>, is_current| WorkspaceChoice {
        id: String::new(),
        name: name.into(),
        role: role.map(Into::into),
        is_current,
    };
    assert_eq!(workspace_label(&choice("Acme", None, false)), "Acme");
    assert_eq!(
        workspace_label(&choice("Acme", Some("Owner"), false)),
        "Acme  ·  Owner"
    );
    assert_eq!(
        workspace_label(&choice("Globex", Some("Member"), true)),
        "Globex  ·  Member  (current)"
    );
    assert_eq!(
        workspace_label(&choice(" ", Some(" "), false)),
        "Unnamed workspace"
    );
}

// ---- Activity and settings ---------------------------------------------------------------

fn row(id: i64, status: RowStatus, reason: Option<&str>) -> QueuedRow {
    let now = Utc::now();
    QueuedRow {
        id,
        project_id: Some("p1".into()),
        started_at: Some(now - chrono::Duration::minutes(10)),
        ended_at: Some(now),
        status,
        reason: reason.map(str::to_owned),
        has_captures: false,
    }
}

#[test]
fn earlier_rows_leave_out_this_sessions_blocks() {
    let now = Utc::now();
    let snapshot = SharedState {
        recent_blocks: vec![RecentBlock {
            row_id: 2,
            project: "P".into(),
            started_at: now,
            ended_at: now,
            productivity: None,
            keyboard_hits: None,
            mouse_clicks: None,
            screenshot: false,
            upload: UploadState::Rejected,
        }],
        queue_rows: vec![
            row(1, RowStatus::Queued, None),
            row(2, RowStatus::Parked, Some("Overlaps.")),
        ],
        ..SharedState::default()
    };
    let earlier: Vec<i64> = activity::earlier_rows(&snapshot)
        .iter()
        .map(|r| r.id)
        .collect();
    assert_eq!(earlier, vec![1]);
}

#[test]
fn notes_quote_the_server_or_the_failed_attempt() {
    let (text, _) = activity::block_note(
        UploadState::Rejected,
        Some(" This block overlaps another one. "),
    )
    .unwrap();
    assert_eq!(text, "Server: This block overlaps another one.");
    assert_eq!(
        activity::block_note(UploadState::Queued, Some("HTTP 503"))
            .unwrap()
            .0,
        "Last try failed: HTTP 503"
    );
    assert!(activity::block_note(UploadState::Queued, None).is_none());
    assert!(activity::block_note(UploadState::Uploaded, Some("old")).is_none());
    assert!(
        activity::block_note(UploadState::Rejected, None).is_some(),
        "a refusal is never silent"
    );
}

#[test]
fn rows_name_their_project_and_survive_missing_times() {
    let snapshot = SharedState {
        projects: vec![ProjectOption {
            id: "p1".into(),
            title: "Website".into(),
        }],
        ..SharedState::default()
    };
    assert_eq!(activity::project_title(&snapshot, Some("p1")), "Website");
    assert_eq!(
        activity::project_title(&snapshot, Some("old")),
        "A project no longer listed"
    );
    let now = Utc::now();
    let mut r = row(1, RowStatus::Queued, None);
    assert!(activity::row_span(&r, now).contains("10m"));
    r.ended_at = None;
    assert!(!activity::row_span(&r, now).is_empty());
    r.started_at = None;
    assert_eq!(activity::row_span(&r, now), "Time unknown");
}

#[test]
fn the_week_chart_shows_past_days_today_live_and_nothing_ahead() {
    let mut days = [0.0; 7];
    days[0] = 7200.0;
    days[1] = 3600.0;
    days[3] = 9999.0;
    assert_eq!(
        activity::week_hours(&days, 2, 1800.0),
        vec![2.0, 1.0, 0.5, 0.0, 0.0, 0.0, 0.0]
    );
}

#[test]
fn colours_match_the_web_dashboards_hash() {
    assert_eq!(activity::fnv1a(""), 0x811c_9dc5);
    assert_eq!(activity::fnv1a("a"), 0xe40c_292c);
    assert_eq!(
        activity::name_color("Website"),
        activity::name_color(" website ")
    );
}

#[test]
fn a_jpeg_thumbnail_decodes_to_an_image() {
    let mut jpeg = Vec::new();
    let source = image::RgbImage::from_pixel(8, 4, image::Rgb([10, 107, 255]));
    image::DynamicImage::ImageRgb8(source)
        .write_to(
            &mut std::io::Cursor::new(&mut jpeg),
            image::ImageFormat::Jpeg,
        )
        .unwrap();
    let decoded = activity::decode_jpeg(&jpeg).expect("decodes");
    assert_eq!(decoded.size, [8, 4]);
    assert!(activity::decode_jpeg(b"not a jpeg").is_none());
}

#[test]
fn consent_receipts_read_newest_first() {
    let log = concat!(
        r#"{"at":"2026-10-05T08:00:00Z","from":"Absent","to":{"Granted":{"screenshot":false,"webcam":false,"keyboard":false,"mouse":false,"active_window":false,"running_programs":false,"blur_screenshots":false}},"origin":"cli"}"#,
        "\nnot json\n",
        r#"{"at":"2026-10-05T09:00:00Z","from":"Absent","to":{"Granted":{"screenshot":true,"webcam":false,"keyboard":true,"mouse":false,"active_window":false,"running_programs":false,"blur_screenshots":true}},"origin":"gui"}"#,
        "\n",
        r#"{"at":"2026-10-05T10:00:00Z","from":"Absent","to":"Revoked","origin":"tray"}"#,
    );
    let receipts = settings::parse_receipts(log, 8);
    let texts: Vec<&str> = receipts.iter().map(|r| r.text.as_str()).collect();
    assert_eq!(
        texts,
        [
            "Changed to declined from the tray",
            "Changed to keyboard counts, blurred screenshots in this window",
            "Changed to time only from the command line",
        ]
    );
    assert_eq!(settings::parse_receipts(log, 1).len(), 1);
}

#[test]
fn doctor_problems_become_banners() {
    let report = json!({ "problems": [
        { "severity": "blocking", "code": "no_session", "message": "No session.", "hint": "log in" },
        { "severity": "warning", "code": "no_tray", "message": "No tray." }
    ]});
    assert_eq!(
        settings::doctor_problems(&report),
        vec![
            (Tone::Danger, "No session. (log in)".to_owned()),
            (Tone::Warning, "No tray.".to_owned())
        ]
    );
    assert!(settings::doctor_problems(&json!({})).is_empty());
}

// ---- The tray's view ---------------------------------------------------------------------

fn signed_in(status: TrackerStatus) -> SharedState {
    SharedState {
        status,
        display_name: "ayesha@acme.com".into(),
        projects: vec![
            ProjectOption {
                id: "p1".into(),
                title: "Website & SEO".into(),
            },
            ProjectOption {
                id: "p2".into(),
                title: "Mobile".into(),
            },
        ],
        selected_project: Some("p1".into()),
        ..SharedState::default()
    }
}

fn view(snapshot: &SharedState) -> TrayView {
    TrayView::new(snapshot, Utc::now(), (0.0, 0.0), false, true)
}

#[test]
fn tracking_view_offers_pause_and_shows_the_project() {
    let now = Utc::now();
    let mut snapshot = signed_in(TrackerStatus::Tracking);
    snapshot.tracking_since = Some(now - chrono::Duration::minutes(65));
    let view = TrayView::new(
        &snapshot,
        now,
        (3.0 * 3600.0 + 42.0 * 60.0, 18.0 * 3600.0),
        true,
        true,
    );
    assert_eq!(view.badge, Badge::Tracking);
    assert_eq!(view.toggle_label, "Pause tracking");
    assert!(
        view.toggle_enabled && view.breaks_enabled && view.projects_enabled && view.task_enabled
    );
    assert_eq!(view.status_line, "● Tracking · 1h 05m — Website & SEO");
    assert_eq!(view.totals_line, "Today 3h 42m · This week 18h 00m");
    assert!(view
        .tooltip
        .starts_with("Tracking · 1h 05m\nWebsite & SEO · Today 3h 42m"));
    assert_eq!(view.task_label, "Set task…");
    assert!(view.autostart && view.dashboard_enabled && !view.needs_attention);
}

#[test]
fn the_task_note_is_quoted_short() {
    let mut snapshot = signed_in(TrackerStatus::Tracking);
    snapshot.task_note = "Q&A for the checkout page redesign and the release notes".into();
    let label = view(&snapshot).task_label;
    assert!(label.starts_with("Task: Q&A for the checkout"), "{label}");
    assert!(label.ends_with('…'));
}

#[test]
fn paused_view_offers_resume_and_upload_only() {
    let now = Utc::now();
    let mut snapshot = signed_in(TrackerStatus::Paused);
    snapshot.paused_since = Some(now);
    let view = TrayView::new(&snapshot, now, (0.0, 0.0), false, true);
    assert_eq!(view.badge, Badge::Paused);
    assert_eq!(view.toggle_label, "Resume tracking");
    assert!(view.toggle_enabled && view.breaks_enabled);
    assert_eq!(view.sync_label, "Upload queued");
}

#[test]
fn away_view_says_since_when() {
    let now = Utc::now();
    let mut snapshot = signed_in(TrackerStatus::Away);
    snapshot.away_since = Some(now - chrono::Duration::minutes(7));
    snapshot.away_prompt = Some(AwayPrompt {
        from: now - chrono::Duration::minutes(7),
        to: now,
        project_title: "Website".into(),
    });
    let view = TrayView::new(&snapshot, now, (0.0, 0.0), false, true);
    assert_eq!(view.badge, Badge::Paused);
    assert!(
        view.status_line.starts_with("● Away since "),
        "{}",
        view.status_line
    );
    assert!(view.toggle_enabled, "a break can still be taken");
    assert!(view.needs_attention, "the away question wants an answer");
}

#[test]
fn a_request_in_flight_disables_the_actions() {
    let mut snapshot = signed_in(TrackerStatus::Tracking);
    snapshot.pending_action = Some(PendingAction::Pausing);
    let view = view(&snapshot);
    assert_eq!(view.toggle_label, "Pausing…");
    assert!(!view.toggle_enabled && !view.breaks_enabled && !view.projects_enabled);
}

#[test]
fn signed_out_view_invites_sign_in() {
    let view = view(&SharedState::default());
    assert_eq!(view.badge, Badge::SignedOut);
    assert_eq!(view.toggle_label, "Sign in…");
    assert!(view.toggle_enabled);
    assert!(
        !view.sync_enabled && !view.projects_enabled && !view.breaks_enabled && !view.task_enabled
    );
    assert_eq!(view.tooltip, "Signed out");
}

#[test]
fn an_expired_sign_in_or_a_removed_device_only_offers_the_way_out() {
    let view_expired = view(&signed_in(TrackerStatus::SignInExpired));
    assert_eq!(view_expired.badge, Badge::Stopped);
    assert_eq!(
        (view_expired.toggle_label, view_expired.toggle_enabled),
        ("Sign in…", true)
    );
    assert!(
        !view_expired.sync_enabled && !view_expired.projects_enabled && !view_expired.task_enabled
    );
    assert!(view_expired.needs_attention);
    let revoked = view(&signed_in(TrackerStatus::DeviceRevoked));
    assert_eq!(revoked.badge, Badge::Stopped);
    assert_eq!(revoked.toggle_label, "Choose a workspace…");
    assert!(!revoked.sync_enabled && !revoked.projects_enabled);
    let no_session = view(&signed_in(TrackerStatus::NoSession));
    assert_eq!(no_session.badge, Badge::Offline);
    assert!(!no_session.toggle_enabled, "nothing to pause");
}

#[test]
fn choosing_a_project_offers_the_list() {
    let mut snapshot = signed_in(TrackerStatus::ChooseProject);
    snapshot.selected_project = None;
    let view = view(&snapshot);
    assert_eq!(view.toggle_label, "Choose a project…");
    assert!(view.projects_enabled && view.task_enabled);
    assert_eq!(view.selected_project, None);
    assert_eq!(view.badge, Badge::Offline);
}

#[test]
fn an_unconfigured_dashboard_is_greyed_out() {
    let snapshot = signed_in(TrackerStatus::Tracking);
    assert!(!TrayView::new(&snapshot, Utc::now(), (0.0, 0.0), false, false).dashboard_enabled);
}

// ---- The link ----------------------------------------------------------------------------

fn wait(what: &str, mut ok: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !ok() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn socket_in(dir: &Path) -> std::path::PathBuf {
    dir.join("dosi-tracker").join("daemon.sock")
}

#[test]
fn the_link_subscribes_sends_requests_and_reconnects() {
    let temp = tempfile::tempdir().unwrap();
    let socket = socket_in(temp.path());
    let snapshot = serde_json::to_value(tracking()).unwrap();
    let (daemon, methods) = support::fake_daemon::recording(&socket, snapshot.clone());
    let repaints = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let counter = repaints.clone();
    let link = Link::start(
        socket.clone(),
        Arc::new(move || {
            counter.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        }),
    );
    wait("connected", || {
        matches!(link.status(), LinkStatus::Connected { .. })
    });
    wait("the snapshot", || {
        link.snapshot().status == TrackerStatus::Tracking
    });
    assert_eq!(link.snapshot().display_name, "ayesha@acme.test");
    assert!(
        repaints.load(std::sync::atomic::Ordering::SeqCst) > 0,
        "events repaint"
    );

    link.call("tracking.pause", json!({ "at": Utc::now() }));
    wait("the answer", || {
        link.try_reply()
            .is_some_and(|r| r.method == "tracking.pause" && r.outcome.is_ok())
    });
    {
        let seen = methods.lock().unwrap();
        assert_eq!(seen[0], "hello");
        assert!(seen.contains(&"state.subscribe".to_owned()));
        assert!(seen.contains(&"tracking.pause".to_owned()));
    }

    // The daemon dies: the window must not show stale data as live.
    drop(daemon);
    wait("not running", || {
        matches!(link.status(), LinkStatus::NotRunning { .. })
    });
    assert!(!link.status().is_live());
    // It comes back: the link reconnects by itself (back-off at most 5 s).
    let (_daemon, methods) = support::fake_daemon::recording(&socket, snapshot);
    wait("reconnected", || {
        matches!(link.status(), LinkStatus::Connected { .. })
    });
    assert!(methods
        .lock()
        .unwrap()
        .contains(&"state.subscribe".to_owned()));
}

#[test]
fn a_daemon_of_another_protocol_makes_the_link_read_only() {
    let temp = tempfile::tempdir().unwrap();
    let socket = socket_in(temp.path());
    let _daemon = support::fake_daemon::FakeDaemon::start(
        &socket,
        Arc::new(|request: &serde_json::Value| {
            Some(json!({
                "v": 2, "id": request["id"],
                "error": { "code": "unsupported_version", "message": "This agent speaks IPC protocol 2." }
            }))
        }),
    );
    let link = Link::start(socket, Arc::new(|| {}));
    wait("mismatch", || {
        matches!(link.status(), LinkStatus::Mismatch { .. })
    });
    match link.status() {
        LinkStatus::Mismatch { message } => assert!(message.contains("protocol 2"), "{message}"),
        other => panic!("{other:?}"),
    }
    assert!(!link.status().is_live());
}

#[test]
fn errors_come_back_with_the_daemons_code() {
    let temp = tempfile::tempdir().unwrap();
    let socket = socket_in(temp.path());
    let _daemon = support::fake_daemon::FakeDaemon::start_with(
        &socket,
        Arc::new(|request: &serde_json::Value| {
            let id = request["id"].clone();
            match request["method"].as_str() {
                Some("hello") => vec![support::fake_daemon::hello_result(&id)],
                _ => vec![json!({
                    "v": 1, "id": id,
                    "error": { "code": "consent_required", "message": "Choose what Dosi Tracker may record first." }
                })],
            }
        }),
    );
    let link = Link::start(socket, Arc::new(|| {}));
    link.call("tracking.resume", json!({}));
    let deadline = Instant::now() + Duration::from_secs(10);
    let reply = loop {
        if let Some(reply) = link.try_reply() {
            break reply;
        }
        assert!(Instant::now() < deadline, "no answer");
        std::thread::sleep(Duration::from_millis(20));
    };
    let error = reply.outcome.expect_err("refused");
    assert_eq!(
        error.code,
        dosi_tracker_linux::ipc::ErrorCode::ConsentRequired
    );
    assert_eq!(error.code.exit_code(), 5, "the CLI's exit-code class");
}
