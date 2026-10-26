//! The rules around a tracking session, as pure functions: which project to
//! track (CONTRACT §2), what it may capture, what the status and the presence
//! heartbeat say (§3), and how a saved pause comes back after a restart (§4).
//!
//! Ported from the Windows worker (`worker/session.rs`, with its tests).
//! Linux deltas:
//! - Effective permissions = project policy AND local consent AND local
//!   config AND platform availability (blur = OR), see [`session_for`].
//!   The webcam is never effective in v1.
//! - Two more statuses in the design 5.1 order: `device_revoked` right after
//!   `sign_in_expired`, `no_session` right after `consent_required`.
//! - Presence: `consent_required` reports `choosing` and `no_session`
//!   reports `stopped` (design 5.1); `device_revoked` sends nothing.

use chrono::{DateTime, TimeDelta, Utc};

use super::config::AppConfig;
use super::model::{PresenceStatus, Project};
use super::session_store::SavedPause;
use super::state::{CaptureFlags, SessionKind, TrackerStatus};
use crate::consent::{ConsentedPermissions, LocalConsent};

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

    /// What the clients show the project captures.
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
        self.policy
            .idle_threshold
            .map(|t| t.num_minutes().max(0) as u32)
    }
}

/// What this desktop session can provide at all (design 7). A capability the
/// platform cannot provide is never effective, whatever the policy and the
/// consent say.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Availability {
    pub screenshot: bool,
    pub keyboard: bool,
    pub mouse: bool,
    pub active_window: bool,
    pub running_programs: bool,
}

impl Availability {
    /// Everything (an X11 session with every sensor working).
    pub fn everything() -> Self {
        Self {
            screenshot: true,
            keyboard: true,
            mouse: true,
            active_window: true,
            running_programs: true,
        }
    }

    /// Nothing (no graphical session).
    pub fn nothing() -> Self {
        Self {
            screenshot: false,
            keyboard: false,
            mouse: false,
            active_window: false,
            running_programs: false,
        }
    }

    /// What a session of `kind` can provide in v1: everything on X11; on
    /// Wayland only running programs (app scopes) and portal screenshots.
    pub fn for_session(kind: SessionKind) -> Self {
        match kind {
            SessionKind::X11 => Self::everything(),
            SessionKind::Wayland => Self {
                screenshot: true,
                running_programs: true,
                ..Self::nothing()
            },
            SessionKind::Tty | SessionKind::None => Self::nothing(),
        }
    }
}

/// The effective permissions: the server's policy AND the local consent AND
/// the local config AND what the platform can provide; blur is ORed. Absent
/// or revoked consent fails closed (everything off, blur on).
pub fn effective_permissions(
    project: &Project,
    consent: &LocalConsent,
    cfg: &AppConfig,
    available: &Availability,
) -> ConsentedPermissions {
    let LocalConsent::Granted(granted) = consent else {
        return ConsentedPermissions::all_disabled();
    };
    let local = &cfg.capture;
    ConsentedPermissions {
        screenshot: project.allow_screenshot
            && local.screenshot
            && granted.screenshot
            && available.screenshot,
        // Not available on Linux in v1: never effective.
        webcam: false,
        keyboard: project.allow_keyboard
            && local.keyboard
            && granted.keyboard
            && available.keyboard,
        mouse: project.allow_mouse && local.mouse && granted.mouse && available.mouse,
        active_window: project.allow_active_window
            && local.active_window
            && granted.active_window
            && available.active_window,
        running_programs: project.allow_running_programs
            && local.running_programs
            && granted.running_programs
            && available.running_programs,
        blur_screenshots: project.blur_screenshots
            || local.blur_screenshots
            || granted.blur_screenshots,
    }
}

/// The session for `project`: the server's permissions win, but never enable
/// what the local config forbade or the platform cannot provide, and require
/// affirmative local user consent. If consent is absent or revoked, all
/// captures fail closed (disabled).
pub fn session_for(
    project: &Project,
    consent: &LocalConsent,
    cfg: &AppConfig,
    available: &Availability,
) -> Session {
    let perms = effective_permissions(project, consent, cfg, available);
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
/// meanwhile, or any break when the login autostart started the agent, stays
/// paused until the user resumes — tracking never restarts silently from a
/// saved pause.
pub fn restored_pause(
    saved: Option<SavedPause>,
    now: DateTime<Utc>,
    launched_at_login: bool,
) -> Option<SavedPause> {
    let pause = saved?;
    let still_running = pause.until.is_some_and(|until| until > now);
    Some(if still_running && !launched_at_login {
        pause
    } else {
        SavedPause {
            since: pause.since,
            until: None,
        }
    })
}

/// What decides the status.
#[derive(Debug, Clone, Copy)]
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
    /// Asleep, or the machine is shutting down.
    pub suspended: bool,
    pub tracking: bool,
    /// Affirmative local user consent state.
    pub consent: LocalConsent,
    /// Linux: the workspace removed this device.
    pub device_revoked: bool,
    /// Linux: no X11/Wayland session is attached (TTY, SSH, before login).
    pub no_graphical_session: bool,
}

impl Default for Facts {
    fn default() -> Self {
        Self {
            storage_failed: false,
            signed_in: false,
            sign_in_expired: false,
            paused: false,
            has_session: false,
            no_workspace: false,
            projects_known: false,
            contacting: false,
            locked: false,
            away: false,
            suspended: false,
            tracking: false,
            consent: LocalConsent::Granted(ConsentedPermissions::default()),
            device_revoked: false,
            no_graphical_session: false,
        }
    }
}

/// The status the clients and the tray show; the first reason tracking is
/// not running wins (design 5.1 order).
pub fn status(facts: &Facts) -> TrackerStatus {
    if facts.storage_failed {
        TrackerStatus::Stopped
    } else if !facts.signed_in {
        TrackerStatus::SignedOut
    } else if facts.sign_in_expired {
        TrackerStatus::SignInExpired
    } else if facts.device_revoked {
        TrackerStatus::DeviceRevoked
    } else if !facts.consent.is_granted() {
        TrackerStatus::ConsentRequired
    } else if facts.no_graphical_session {
        TrackerStatus::NoSession
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

/// What the presence heartbeat reports for `status` (CONTRACT §3, design
/// 5.1); `None` when no heartbeat can or should be sent (signed out: the
/// farewell is sent separately; the sign-in expired; the device was removed;
/// no answer from the server yet).
pub fn presence(status: TrackerStatus, timed_break: bool) -> Option<PresenceStatus> {
    Some(match status {
        TrackerStatus::Tracking => PresenceStatus::Tracking,
        TrackerStatus::Paused if timed_break => PresenceStatus::Break,
        TrackerStatus::Paused => PresenceStatus::Paused,
        TrackerStatus::Away => PresenceStatus::Idle,
        TrackerStatus::Locked => PresenceStatus::Locked,
        TrackerStatus::ChooseProject | TrackerStatus::ConsentRequired => PresenceStatus::Choosing,
        TrackerStatus::Stopped | TrackerStatus::NoSession => PresenceStatus::Stopped,
        TrackerStatus::SignedOut
        | TrackerStatus::SignInExpired
        | TrackerStatus::DeviceRevoked
        | TrackerStatus::Connecting
        | TrackerStatus::Offline => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::config::CaptureRestrictions;

    /// What the Windows tests' `CapturePermissions::default()` grants:
    /// everything but the webcam, no blur.
    fn granted_defaults() -> ConsentedPermissions {
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

    fn everywhere() -> Availability {
        Availability::everything()
    }

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
        let consent = LocalConsent::Granted(granted_defaults());
        let cfg = AppConfig {
            capture: CaptureRestrictions {
                blur_screenshots: true,
                ..CaptureRestrictions::default()
            },
            ..AppConfig::default()
        };
        let mut source = project("p1");
        source.interval_minutes = 90;
        source.idle_threshold_minutes = 0;
        let session = session_for(&source, &consent, &cfg, &everywhere());
        let perms = &session.policy.perms;
        assert!(
            perms.screenshot && !perms.webcam,
            "local config can only take away"
        );
        assert!(!perms.mouse && perms.keyboard && !perms.running_programs);
        assert!(perms.blur_screenshots, "local config can add blur");
        assert_eq!(
            session.policy.interval,
            TimeDelta::minutes(60),
            "clamped to 5..60"
        );
        assert_eq!(
            session.policy.idle_threshold, None,
            "0 turns idle detection off"
        );
        assert_eq!(session.project_id(), "p1");
        assert_eq!(session.interval_minutes(), 60);
        assert!(session.capture_flags().blur_screenshots);

        let session = session_for(
            &project("p2"),
            &consent,
            &AppConfig::default(),
            &everywhere(),
        );
        assert_eq!(
            session.idle_threshold_minutes(),
            Some(5),
            "the server's default"
        );
    }

    #[test]
    fn session_for_fails_closed_when_consent_is_absent() {
        let cfg = AppConfig::default();
        let session = session_for(&project("p1"), &LocalConsent::Absent, &cfg, &everywhere());
        let perms = &session.policy.perms;
        assert!(!perms.screenshot);
        assert!(!perms.keyboard);
        assert!(!perms.mouse);
        assert!(!perms.active_window);
        assert!(!perms.webcam);
        assert!(!perms.running_programs);
        assert!(perms.blur_screenshots, "blur stays on when failing closed");
    }

    #[test]
    fn session_for_fails_closed_when_consent_is_revoked() {
        let cfg = AppConfig::default();
        let session = session_for(&project("p1"), &LocalConsent::Revoked, &cfg, &everywhere());
        let perms = &session.policy.perms;
        assert!(!perms.screenshot);
        assert!(!perms.keyboard);
        assert!(!perms.mouse);
        assert!(!perms.active_window);
    }

    #[test]
    fn session_for_respects_granular_consent_toggles() {
        let consent = ConsentedPermissions {
            keyboard: true,
            mouse: false,
            active_window: false,
            screenshot: false,
            ..granted_defaults()
        };
        let session = session_for(
            &project("p1"),
            &LocalConsent::Granted(consent),
            &AppConfig::default(),
            &everywhere(),
        );
        let perms = &session.policy.perms;
        assert!(perms.keyboard, "keyboard was granted");
        assert!(!perms.mouse, "mouse was denied");
        assert!(!perms.active_window, "active window was denied");
        assert!(!perms.screenshot, "screenshot was denied");
    }

    #[test]
    fn the_platform_and_the_webcam_rule_narrow_further() {
        let consent = LocalConsent::Granted(ConsentedPermissions {
            webcam: true,
            ..granted_defaults()
        });
        let mut all = project("p1");
        all.allow_mouse = true;
        all.allow_running_programs = true;
        // Wayland: no input counts, no active window; screenshots through the
        // portal and running programs remain.
        let wayland = Availability::for_session(SessionKind::Wayland);
        let perms = session_for(&all, &consent, &AppConfig::default(), &wayland)
            .policy
            .perms;
        assert!(!perms.keyboard && !perms.mouse && !perms.active_window);
        assert!(perms.screenshot && perms.running_programs);
        assert!(!perms.webcam, "the webcam is never effective in v1");
        // X11 has everything but the webcam.
        let x11 = Availability::for_session(SessionKind::X11);
        let perms = session_for(&all, &consent, &AppConfig::default(), &x11)
            .policy
            .perms;
        assert!(perms.keyboard && perms.mouse && perms.active_window && !perms.webcam);
        // No graphical session: nothing.
        for kind in [SessionKind::Tty, SessionKind::None] {
            let available = Availability::for_session(kind);
            let perms = session_for(&all, &consent, &AppConfig::default(), &available)
                .policy
                .perms;
            assert!(!perms.has_any_active_capability(), "{kind:?}");
        }
        // Consent and project add blur on their own.
        let blur_consent = LocalConsent::Granted(ConsentedPermissions {
            blur_screenshots: true,
            ..granted_defaults()
        });
        let perms = session_for(&all, &blur_consent, &AppConfig::default(), &x11)
            .policy
            .perms;
        assert!(perms.blur_screenshots);
        all.blur_screenshots = true;
        let perms = session_for(&all, &consent, &AppConfig::default(), &x11)
            .policy
            .perms;
        assert!(perms.blur_screenshots);
    }

    fn at(s: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc)
    }

    #[test]
    fn a_saved_pause_survives_a_restart() {
        let now = at("2026-10-05T09:00:00Z");
        let open = SavedPause {
            since: at("2026-10-04T18:00:00Z"),
            until: None,
        };
        assert_eq!(restored_pause(Some(open), now, false), Some(open));
        assert_eq!(
            restored_pause(Some(open), now, true),
            Some(open),
            "after a reboot it starts paused"
        );
        assert_eq!(restored_pause(None, now, true), None);
    }

    #[test]
    fn a_timed_break_never_resumes_silently_after_an_autostart() {
        let now = at("2026-10-05T09:00:00Z");
        let running = SavedPause {
            since: at("2026-10-05T08:50:00Z"),
            until: Some(at("2026-10-05T09:05:00Z")),
        };
        // Opened by the user mid-break: the break goes on and ends by itself.
        assert_eq!(restored_pause(Some(running), now, false), Some(running));
        // Started by the login autostart: paused until the user resumes.
        assert_eq!(
            restored_pause(Some(running), now, true),
            Some(SavedPause {
                since: running.since,
                until: None
            })
        );
        // The break ran out while the app was closed: paused, never resumed silently.
        let over = SavedPause {
            since: at("2026-10-05T08:00:00Z"),
            until: Some(at("2026-10-05T08:15:00Z")),
        };
        assert_eq!(
            restored_pause(Some(over), now, false),
            Some(SavedPause {
                since: over.since,
                until: None
            })
        );
    }

    fn tracking() -> Facts {
        Facts {
            signed_in: true,
            has_session: true,
            projects_known: true,
            tracking: true,
            ..Facts::default()
        }
    }

    #[test]
    fn the_status_names_the_first_reason_tracking_is_not_running() {
        assert_eq!(status(&tracking()), TrackerStatus::Tracking);
        assert_eq!(status(&Facts::default()), TrackerStatus::SignedOut);
        assert_eq!(
            status(&Facts {
                storage_failed: true,
                ..tracking()
            }),
            TrackerStatus::Stopped
        );
        assert_eq!(
            status(&Facts {
                sign_in_expired: true,
                paused: true,
                ..tracking()
            }),
            TrackerStatus::SignInExpired
        );
        assert_eq!(
            status(&Facts {
                paused: true,
                locked: true,
                ..tracking()
            }),
            TrackerStatus::Paused
        );
        assert_eq!(
            status(&Facts {
                locked: true,
                away: true,
                ..tracking()
            }),
            TrackerStatus::Locked
        );
        assert_eq!(
            status(&Facts {
                suspended: true,
                ..tracking()
            }),
            TrackerStatus::Locked
        );
        assert_eq!(
            status(&Facts {
                away: true,
                tracking: false,
                ..tracking()
            }),
            TrackerStatus::Away
        );
        let no_session = Facts {
            has_session: false,
            tracking: false,
            ..tracking()
        };
        assert_eq!(status(&no_session), TrackerStatus::ChooseProject);
        assert_eq!(
            status(&Facts {
                projects_known: false,
                contacting: true,
                ..no_session
            }),
            TrackerStatus::Connecting
        );
        assert_eq!(
            status(&Facts {
                projects_known: false,
                ..no_session
            }),
            TrackerStatus::Offline
        );
    }

    #[test]
    fn linux_statuses_follow_the_design_order() {
        // device_revoked comes right after sign_in_expired ...
        let revoked = Facts {
            device_revoked: true,
            paused: true,
            consent: LocalConsent::Absent,
            ..tracking()
        };
        assert_eq!(status(&revoked), TrackerStatus::DeviceRevoked);
        assert_eq!(
            status(&Facts {
                sign_in_expired: true,
                ..revoked
            }),
            TrackerStatus::SignInExpired
        );
        // ... and no_session right after consent_required, before paused.
        let tty = Facts {
            no_graphical_session: true,
            paused: true,
            locked: true,
            ..tracking()
        };
        assert_eq!(status(&tty), TrackerStatus::NoSession);
        assert_eq!(
            status(&Facts {
                consent: LocalConsent::Revoked,
                ..tty
            }),
            TrackerStatus::ConsentRequired
        );
        assert_eq!(
            status(&Facts {
                signed_in: false,
                ..tty
            }),
            TrackerStatus::SignedOut
        );
        assert_eq!(
            status(&Facts {
                storage_failed: true,
                ..tty
            }),
            TrackerStatus::Stopped
        );
        // Time-only consent (nothing granted) is still consent.
        let time_only = LocalConsent::Granted(ConsentedPermissions::all_disabled());
        assert_eq!(
            status(&Facts {
                consent: time_only,
                ..tracking()
            }),
            TrackerStatus::Tracking
        );
    }

    #[test]
    fn presence_follows_the_status() {
        assert_eq!(
            presence(TrackerStatus::Tracking, false),
            Some(PresenceStatus::Tracking)
        );
        assert_eq!(
            presence(TrackerStatus::Paused, false),
            Some(PresenceStatus::Paused)
        );
        assert_eq!(
            presence(TrackerStatus::Paused, true),
            Some(PresenceStatus::Break)
        );
        assert_eq!(
            presence(TrackerStatus::Away, false),
            Some(PresenceStatus::Idle)
        );
        assert_eq!(
            presence(TrackerStatus::Locked, false),
            Some(PresenceStatus::Locked)
        );
        assert_eq!(
            presence(TrackerStatus::ChooseProject, false),
            Some(PresenceStatus::Choosing)
        );
        assert_eq!(
            presence(TrackerStatus::Stopped, false),
            Some(PresenceStatus::Stopped)
        );
        for silent in [
            TrackerStatus::SignedOut,
            TrackerStatus::SignInExpired,
            TrackerStatus::Connecting,
            TrackerStatus::Offline,
        ] {
            assert_eq!(presence(silent, false), None, "{silent:?}");
        }
        // Linux (design 5.1).
        assert_eq!(
            presence(TrackerStatus::ConsentRequired, false),
            Some(PresenceStatus::Choosing)
        );
        assert_eq!(
            presence(TrackerStatus::NoSession, false),
            Some(PresenceStatus::Stopped)
        );
        assert_eq!(
            presence(TrackerStatus::DeviceRevoked, false),
            None,
            "never sent for a removed device"
        );
    }
}
