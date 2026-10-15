//! The rules around a tracking session, as pure functions: which project to
//! track (CONTRACT §2), what it may capture, what the status and the presence
//! heartbeat say (§3), and how a saved pause comes back after a restart (§4).

use chrono::{DateTime, TimeDelta, Utc};

use crate::config::{AppConfig, CapturePermissions};
use crate::credentials::SavedPause;
use crate::model::{PresenceStatus, Project};
use crate::state::{CaptureFlags, TrackerStatus};

use super::blocks::BlockPolicy;

/// The project being tracked, as resolved from the server's (or the saved)
/// project list.
#[derive(Debug, Clone)]
pub struct Session {
    pub title: String,
    pub policy: BlockPolicy,
}

impl Session {
    pub fn project_id(&self) -> &str {
        &self.policy.project_id
    }

    /// What the window shows the project captures.
    pub fn capture_flags(&self) -> CaptureFlags {
        let perms = &self.policy.perms;
        CaptureFlags {
            screenshot: perms.screenshot,
            webcam: perms.webcam,
            keyboard: perms.keyboard,
            mouse: perms.mouse,
            active_window: perms.active_window,
            running_programs: perms.running_programs,
            blur_screenshots: perms.blur_screenshots,
        }
    }

    pub fn interval_minutes(&self) -> u64 {
        self.policy.interval.num_minutes().max(0) as u64
    }

    pub fn idle_threshold_minutes(&self) -> Option<u32> {
        self.policy.idle_threshold.map(|t| t.num_minutes().max(0) as u32)
    }
}

/// The session for `project`: the server's permissions win, but never enable
/// what the local config forbade; blur is added by either.
pub fn session_for(project: &Project, cfg: &AppConfig) -> Session {
    let local = &cfg.capture;
    let perms = CapturePermissions {
        screenshot: project.allow_screenshot && local.screenshot,
        webcam: project.allow_webcam && local.webcam,
        keyboard: project.allow_keyboard && local.keyboard,
        mouse: project.allow_mouse && local.mouse,
        active_window: project.allow_active_window && local.active_window,
        running_programs: project.allow_running_programs && local.running_programs,
        blur_screenshots: project.blur_screenshots || local.blur_screenshots,
    };
    let interval_minutes = project.interval_minutes.clamp(5, 60) as i64;
    Session {
        title: project.title.clone(),
        policy: BlockPolicy {
            project_id: project.id.clone(),
            perms,
            interval: TimeDelta::minutes(interval_minutes),
            idle_threshold: project
                .idle_threshold()
                .and_then(|threshold| TimeDelta::from_std(threshold).ok()),
        },
    }
}

/// Which project to track.
#[derive(Debug, Clone, Copy)]
pub enum Pick<'a> {
    Project(&'a Project),
    /// Several projects and none to go on: the user must choose.
    Choose,
    /// Not a member of any active project.
    NoProjects,
}

/// CONTRACT §2: the only project when there is exactly one; otherwise the
/// remembered one if it is still listed; otherwise the user chooses. Never
/// silently the first of several.
pub fn pick_project<'a>(projects: &'a [Project], remembered: Option<&str>) -> Pick<'a> {
    match projects {
        [] => Pick::NoProjects,
        [only] => Pick::Project(only),
        _ => remembered
            .and_then(|id| projects.iter().find(|p| p.id == id))
            .map_or(Pick::Choose, Pick::Project),
    }
}

/// The pause to start with, from the one saved when the app last ran.
///
/// A pause survives a restart. A timed break still running when the user
/// opens the app goes on and resumes by itself; but a break that ran out
/// meanwhile, or any break when Windows started the app at sign-in, stays
/// paused until the user resumes — tracking never restarts silently from a
/// saved pause.
pub fn restored_pause(saved: Option<SavedPause>, now: DateTime<Utc>, launched_at_login: bool) -> Option<SavedPause> {
    let pause = saved?;
    let still_running = pause.until.is_some_and(|until| until > now);
    Some(if still_running && !launched_at_login {
        pause
    } else {
        SavedPause { since: pause.since, until: None }
    })
}

/// What decides the status.
#[derive(Debug, Clone, Copy, Default)]
pub struct Facts {
    /// The local queue is unusable: nothing can be recorded.
    pub storage_failed: bool,
    pub signed_in: bool,
    pub sign_in_expired: bool,
    pub paused: bool,
    pub has_session: bool,
    /// Signed in, but no workspace to track in (access to it was lost).
    pub no_workspace: bool,
    /// A project list is known (from the server, or saved).
    pub projects_known: bool,
    /// The first answer from the server is still awaited.
    pub contacting: bool,
    pub locked: bool,
    pub away: bool,
    /// Asleep, or Windows is shutting down.
    pub suspended: bool,
    pub tracking: bool,
}

/// The status the window and the tray show; the first reason tracking is
/// not running wins.
pub fn status(facts: &Facts) -> TrackerStatus {
    if facts.storage_failed {
        TrackerStatus::Stopped
    } else if !facts.signed_in {
        TrackerStatus::SignedOut
    } else if facts.sign_in_expired {
        TrackerStatus::SignInExpired
    } else if facts.paused {
        TrackerStatus::Paused
    } else if !facts.has_session {
        if facts.projects_known || facts.no_workspace {
            TrackerStatus::ChooseProject
        } else if facts.contacting {
            TrackerStatus::Connecting
        } else {
            TrackerStatus::Offline
        }
    } else if facts.locked || facts.suspended {
        TrackerStatus::Locked
    } else if facts.away {
        TrackerStatus::Away
    } else if facts.tracking {
        TrackerStatus::Tracking
    } else {
        TrackerStatus::Connecting
    }
}

/// What the presence heartbeat reports for `status` (CONTRACT §3); `None`
/// when no heartbeat can or should be sent (signed out, the sign-in expired,
/// no answer from the server yet).
pub fn presence(status: TrackerStatus, timed_break: bool) -> Option<PresenceStatus> {
    Some(match status {
        TrackerStatus::Tracking => PresenceStatus::Tracking,
        TrackerStatus::Paused if timed_break => PresenceStatus::Break,
        TrackerStatus::Paused => PresenceStatus::Paused,
        TrackerStatus::Away => PresenceStatus::Idle,
        TrackerStatus::Locked => PresenceStatus::Locked,
        TrackerStatus::ChooseProject => PresenceStatus::Choosing,
        TrackerStatus::Stopped => PresenceStatus::Stopped,
        TrackerStatus::SignedOut | TrackerStatus::SignInExpired | TrackerStatus::Connecting | TrackerStatus::Offline => {
            return None
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn project(id: &str) -> Project {
        serde_json::from_value(serde_json::json!({
            "id": id, "title": format!("Project {id}"), "intervalMinutes": 10,
            "allowScreenshot": true, "allowWebcam": true, "allowKeyboard": true, "allowMouse": false,
            "allowActiveWindow": true, "allowRunningPrograms": false
        }))
        .unwrap()
    }

    /// The picked project's id, or what to do instead.
    fn picked(pick: Pick<'_>) -> &str {
        match pick {
            Pick::Project(project) => &project.id,
            Pick::Choose => "<choose>",
            Pick::NoProjects => "<none>",
        }
    }

    #[test]
    fn exactly_one_project_is_picked_whatever_was_remembered() {
        let only = [project("p1")];
        assert_eq!(picked(pick_project(&only, None)), "p1");
        assert_eq!(picked(pick_project(&only, Some("gone"))), "p1");
    }

    #[test]
    fn several_projects_need_the_remembered_one_or_a_choice() {
        let several = [project("acme"), project("zeta")];
        assert_eq!(picked(pick_project(&several, Some("zeta"))), "zeta");
        // Never silently the first one.
        assert_eq!(picked(pick_project(&several, None)), "<choose>");
        assert_eq!(picked(pick_project(&several, Some("removed"))), "<choose>");
        assert_eq!(picked(pick_project(&[], Some("acme"))), "<none>");
    }

    #[test]
    fn the_session_combines_server_and_local_policy() {
        let mut cfg = AppConfig::default();
        cfg.capture.webcam = false;
        cfg.capture.blur_screenshots = true;
        let mut source = project("p1");
        source.interval_minutes = 90;
        source.idle_threshold_minutes = 0;
        let session = session_for(&source, &cfg);
        let perms = &session.policy.perms;
        assert!(perms.screenshot && !perms.webcam, "local config can only take away");
        assert!(!perms.mouse && perms.keyboard && !perms.running_programs);
        assert!(perms.blur_screenshots, "local config can add blur");
        assert_eq!(session.policy.interval, TimeDelta::minutes(60), "clamped to 5..60");
        assert_eq!(session.policy.idle_threshold, None, "0 turns idle detection off");
        assert_eq!(session.project_id(), "p1");
        assert_eq!(session.interval_minutes(), 60);
        assert!(session.capture_flags().blur_screenshots);

        let session = session_for(&project("p2"), &AppConfig::default());
        assert_eq!(session.idle_threshold_minutes(), Some(5), "the server's default");
    }

    fn at(s: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc)
    }

    #[test]
    fn a_saved_pause_survives_a_restart() {
        let now = at("2026-10-05T09:00:00Z");
        let open = SavedPause { since: at("2026-10-04T18:00:00Z"), until: None };
        assert_eq!(restored_pause(Some(open), now, false), Some(open));
        assert_eq!(restored_pause(Some(open), now, true), Some(open), "after a reboot it starts paused");
        assert_eq!(restored_pause(None, now, true), None);
    }

    #[test]
    fn a_timed_break_never_resumes_silently_after_an_autostart() {
        let now = at("2026-10-05T09:00:00Z");
        let running = SavedPause { since: at("2026-10-05T08:50:00Z"), until: Some(at("2026-10-05T09:05:00Z")) };
        // Opened by the user mid-break: the break goes on and ends by itself.
        assert_eq!(restored_pause(Some(running), now, false), Some(running));
        // Started by Windows: paused until the user resumes.
        assert_eq!(restored_pause(Some(running), now, true), Some(SavedPause { since: running.since, until: None }));
        // The break ran out while the app was closed: paused, never resumed silently.
        let over = SavedPause { since: at("2026-10-05T08:00:00Z"), until: Some(at("2026-10-05T08:15:00Z")) };
        assert_eq!(restored_pause(Some(over), now, false), Some(SavedPause { since: over.since, until: None }));
    }

    fn tracking() -> Facts {
        Facts { signed_in: true, has_session: true, projects_known: true, tracking: true, ..Facts::default() }
    }

    #[test]
    fn the_status_names_the_first_reason_tracking_is_not_running() {
        assert_eq!(status(&tracking()), TrackerStatus::Tracking);
        assert_eq!(status(&Facts::default()), TrackerStatus::SignedOut);
        assert_eq!(status(&Facts { storage_failed: true, ..tracking() }), TrackerStatus::Stopped);
        assert_eq!(status(&Facts { sign_in_expired: true, paused: true, ..tracking() }), TrackerStatus::SignInExpired);
        assert_eq!(status(&Facts { paused: true, locked: true, ..tracking() }), TrackerStatus::Paused);
        assert_eq!(status(&Facts { locked: true, away: true, ..tracking() }), TrackerStatus::Locked);
        assert_eq!(status(&Facts { suspended: true, ..tracking() }), TrackerStatus::Locked);
        assert_eq!(status(&Facts { away: true, tracking: false, ..tracking() }), TrackerStatus::Away);
        let no_session = Facts { has_session: false, tracking: false, ..tracking() };
        assert_eq!(status(&no_session), TrackerStatus::ChooseProject);
        assert_eq!(status(&Facts { projects_known: false, contacting: true, ..no_session }), TrackerStatus::Connecting);
        assert_eq!(status(&Facts { projects_known: false, ..no_session }), TrackerStatus::Offline);
    }

    #[test]
    fn presence_follows_the_status() {
        assert_eq!(presence(TrackerStatus::Tracking, false), Some(PresenceStatus::Tracking));
        assert_eq!(presence(TrackerStatus::Paused, false), Some(PresenceStatus::Paused));
        assert_eq!(presence(TrackerStatus::Paused, true), Some(PresenceStatus::Break));
        assert_eq!(presence(TrackerStatus::Away, false), Some(PresenceStatus::Idle));
        assert_eq!(presence(TrackerStatus::Locked, false), Some(PresenceStatus::Locked));
        assert_eq!(presence(TrackerStatus::ChooseProject, false), Some(PresenceStatus::Choosing));
        assert_eq!(presence(TrackerStatus::Stopped, false), Some(PresenceStatus::Stopped));
        for silent in [TrackerStatus::SignedOut, TrackerStatus::SignInExpired, TrackerStatus::Connecting, TrackerStatus::Offline] {
            assert_eq!(presence(silent, false), None, "{silent:?}");
        }
    }
}
