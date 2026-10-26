//! Synthetic preview scenes (`--preview`, feature `test-support` only) for
//! the Xvfb smoke test and documentation screenshots.
//!
//! Every scene is made-up data: no daemon, no account, no capture. The
//! window runs on a [`super::link::Link::preview`], which sends nothing.

use chrono::{Duration, Utc};

use crate::agent::state::{
    Capabilities, CapabilityInfo, CapabilityState, DaemonInfo, IndicatorInfo, PlatformAvailability,
    ProjectOption, RecentBlock, SessionKind, SessionSummary, SharedState, TrackerStatus,
    UploadState,
};
use crate::consent::{ConsentedPermissions, LocalConsent};

/// The scene names `--preview` accepts.
pub const SCENES: [&str; 7] = [
    "main",
    "paused",
    "signed-out",
    "wayland",
    "no-tray",
    "keyring-locked",
    "portal-denied",
];

fn granted() -> ConsentedPermissions {
    ConsentedPermissions {
        screenshot: true,
        webcam: false,
        keyboard: true,
        mouse: true,
        active_window: true,
        running_programs: true,
        blur_screenshots: false,
    }
}

fn capabilities(kind: SessionKind, paused: bool) -> Capabilities {
    let ok = PlatformAvailability::Available;
    let wayland =
        |what: &str| PlatformAvailability::Unsupported(format!("{what} not available on Wayland."));
    let on = |platform: &PlatformAvailability, backend| {
        CapabilityInfo::evaluate(true, true, true, platform, paused, Some(backend))
    };
    match kind {
        SessionKind::Wayland => Capabilities {
            keyboard: on(&wayland("Keyboard counts are"), "xi2"),
            mouse: on(&wayland("Mouse counts are"), "xi2"),
            active_window: on(&wayland("The active window is"), "x11"),
            running_programs: on(&ok, "procfs"),
            screenshot: on(&ok, "portal"),
            webcam: CapabilityInfo::evaluate(false, false, true, &ok, paused, None),
        },
        _ => Capabilities {
            keyboard: on(&ok, "xi2"),
            mouse: on(&ok, "xi2"),
            active_window: on(&ok, "x11"),
            running_programs: on(&ok, "procfs"),
            screenshot: on(&ok, "x11"),
            webcam: CapabilityInfo::evaluate(false, false, true, &ok, paused, None),
        },
    }
}

/// Signed in and tracking on an X11 desktop with a tray.
fn main_scene() -> SharedState {
    let now = Utc::now();
    let block = now - Duration::minutes(4);
    SharedState {
        status: TrackerStatus::Tracking,
        display_name: "ayesha@acme.test".into(),
        workspace: "Acme".into(),
        server_url: "https://tracker.acme.test".into(),
        projects: ["Website Redesign", "Mobile App", "Internal tools"]
            .iter()
            .enumerate()
            .map(|(i, title)| ProjectOption {
                id: format!("p{}", i + 1),
                title: (*title).to_owned(),
            })
            .collect(),
        selected_project: Some("p1".into()),
        tracked_today_secs: 3.0 * 3600.0 + 12.0 * 60.0,
        tracked_week_secs: 14.0 * 3600.0,
        week_day_secs: [7.5 * 3600.0, 6.0 * 3600.0, 0.0, 0.0, 0.0, 0.0, 0.0],
        last_productivity: Some(72),
        task_note: "Checkout page redesign".into(),
        tracking_since: Some(now - Duration::minutes(64)),
        block_started_at: Some(block),
        unsaved_since: Some(block),
        next_capture_at: Some(block + Duration::minutes(10)),
        last_sync: Some(now - Duration::minutes(3)),
        recent_blocks: vec![RecentBlock {
            row_id: 1,
            project: "Website Redesign".into(),
            started_at: now - Duration::minutes(14),
            ended_at: block,
            productivity: Some(81),
            keyboard_hits: Some(412),
            mouse_clicks: Some(96),
            screenshot: true,
            upload: UploadState::Uploaded,
        }],
        consent: LocalConsent::Granted(granted()),
        capabilities: capabilities(SessionKind::X11, false),
        session: SessionSummary {
            kind: SessionKind::X11,
            desktop: Some("XFCE".into()),
            idle_source: Some("xi2".into()),
        },
        indicator: IndicatorInfo {
            tray: true,
            notifications: true,
        },
        daemon: Some(DaemonInfo {
            version: env!("CARGO_PKG_VERSION").into(),
            pid: 4242,
        }),
        capture: crate::agent::state::CaptureFlags {
            screenshot: true,
            keyboard: true,
            mouse: true,
            active_window: true,
            running_programs: true,
            ..Default::default()
        },
        ..SharedState::default()
    }
}

/// The snapshot of scene `name`, if there is one.
pub fn scene(name: &str) -> Option<SharedState> {
    let now = Utc::now();
    let mut s = main_scene();
    match name {
        "main" => {}
        "paused" => {
            s.status = TrackerStatus::Paused;
            s.paused_since = Some(now - Duration::minutes(6));
            s.resume_at = Some(now + Duration::minutes(24));
            s.block_started_at = None;
            s.unsaved_since = None;
            s.next_capture_at = None;
            s.capabilities = capabilities(SessionKind::X11, true);
        }
        "signed-out" => {
            s = SharedState {
                indicator: s.indicator,
                daemon: s.daemon,
                session: s.session,
                ..SharedState::default()
            };
        }
        "wayland" => {
            s.session = SessionSummary {
                kind: SessionKind::Wayland,
                desktop: Some("GNOME".into()),
                idle_source: Some("mutter".into()),
            };
            s.capabilities = capabilities(SessionKind::Wayland, false);
            s.last_productivity = None;
            s.recent_blocks[0].productivity = None;
            s.recent_blocks[0].keyboard_hits = None;
            s.recent_blocks[0].mouse_clicks = None;
        }
        "no-tray" => {
            s.indicator.tray = false;
        }
        "keyring-locked" => {
            s.credentials_locked = true;
            s.pending_uploads = 3;
        }
        "portal-denied" => {
            s.session = SessionSummary {
                kind: SessionKind::Wayland,
                desktop: Some("GNOME".into()),
                idle_source: Some("mutter".into()),
            };
            s.capabilities = capabilities(SessionKind::Wayland, false);
            s.capabilities.screenshot = CapabilityInfo::evaluate(
                true,
                true,
                true,
                &PlatformAvailability::Denied,
                false,
                Some("portal"),
            );
            debug_assert_eq!(s.capabilities.screenshot.state, CapabilityState::Denied);
        }
        _ => return None,
    }
    Some(s)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_scene_exists_and_unknown_ones_do_not() {
        for name in SCENES {
            assert!(scene(name).is_some(), "{name}");
        }
        assert!(scene("dancing").is_none());
        assert_eq!(
            scene("signed-out").unwrap().status,
            TrackerStatus::SignedOut
        );
        assert!(!scene("no-tray").unwrap().indicator.tray);
        assert!(scene("keyring-locked").unwrap().credentials_locked);
        assert_eq!(
            scene("portal-denied")
                .unwrap()
                .capabilities
                .screenshot
                .state,
            CapabilityState::Denied
        );
        let wayland = scene("wayland").unwrap();
        assert_eq!(
            wayland.capabilities.keyboard.state,
            CapabilityState::Unsupported
        );
        assert!(
            wayland.recent_blocks[0].keyboard_hits.is_none(),
            "unmeasured stays null"
        );
    }
}
