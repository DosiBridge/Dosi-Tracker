//! Publishing: the status and clocks the window shows, and the presence
//! heartbeat.

use chrono::{DateTime, Utc};
use tokio::time::Instant;

use crate::model::{Heartbeat, PresenceStatus};
use crate::state::{AwayPrompt, CaptureFlags, TrackerStatus};

use super::blocks::Stretch;
use super::control::ControlRequest;
use super::session::{self, Facts, Session};
use super::{Worker, ENQUEUE_FAILURES_SHOWN, HEARTBEAT_EVERY};

/// What [`Worker::publish`] writes; compared to skip unchanged frames.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct View {
    status: TrackerStatus,
    tracking_since: Option<DateTime<Utc>>,
    block_started_at: Option<DateTime<Utc>>,
    unsaved_since: Option<DateTime<Utc>>,
    next_capture_at: Option<DateTime<Utc>>,
    paused_since: Option<DateTime<Utc>>,
    resume_at: Option<DateTime<Utc>>,
    away_since: Option<DateTime<Utc>>,
    away_prompt: Option<AwayPrompt>,
    selected_project: Option<String>,
    interval_minutes: u64,
    idle_threshold_minutes: Option<u32>,
    capture: CaptureFlags,
    task_note: String,
    server_reachable: bool,
    clock_skew_minutes: Option<i64>,
    consent: crate::config::LocalConsent,
    consent_error: Option<String>,
    consent_unsynced: bool,
}

impl Worker {
    pub(super) fn facts(&self) -> Facts {
        Facts {
            storage_failed: self.enqueue_failures >= ENQUEUE_FAILURES_SHOWN,
            signed_in: self.account.is_some(),
            sign_in_expired: self.expired,
            paused: self.pause.is_some(),
            has_session: self.session.is_some(),
            no_workspace: self.account.as_ref().is_some_and(|a| a.workspace_id.is_none()),
            projects_known: self.projects.is_some(),
            contacting: self.contacting || self.signing_in,
            locked: self.locked || self.disconnected,
            away: self.away.is_some(),
            suspended: self.suspended.is_some() || self.shutting_down.is_some(),
            tracking: self.blocks.is_open(),
            consent: self.cfg.consent,
        }
    }

    pub(super) fn view(&self) -> View {
        let status = session::status(&self.facts());
        let stretch = self.blocks.stretch().filter(|_| status == TrackerStatus::Tracking);
        let pause = self.pause.filter(|_| status == TrackerStatus::Paused);
        View {
            status,
            tracking_since: stretch.map(|s| s.since),
            block_started_at: stretch.map(|s| s.block.started_at),
            unsaved_since: stretch.map(Stretch::unsaved_since),
            next_capture_at: stretch.map(|s| s.block.planned_end()),
            paused_since: pause.map(|p| p.since),
            resume_at: pause.and_then(|p| p.until),
            away_since: self.away.as_ref().filter(|_| status == TrackerStatus::Away).map(|a| a.since),
            away_prompt: self.prompt.as_ref().map(|p| AwayPrompt { from: p.from, to: p.to, project_title: p.session.title.clone() }),
            selected_project: self.session.as_ref().map(|s| s.project_id().to_owned()),
            // Before a project is resolved, the configured interval.
            interval_minutes: self.session.as_ref().map_or(self.cfg.interval().as_secs() / 60, Session::interval_minutes),
            idle_threshold_minutes: self.session.as_ref().and_then(Session::idle_threshold_minutes),
            capture: self.session.as_ref().map(Session::capture_flags).unwrap_or_default(),
            task_note: self.task_note.clone().unwrap_or_default(),
            server_reachable: self.reachable,
            clock_skew_minutes: self.clock.skew_minutes(),
            consent: self.cfg.consent,
            consent_error: self.consent_error.clone(),
            consent_unsynced: self.consent_unsynced,
        }
    }

    /// Publish the status and the clocks the window animates (only what
    /// changed wakes it), and send a heartbeat when the presence changed or
    /// one is due.
    pub(super) fn publish(&mut self) {
        let view = self.view();
        let status = view.status;
        let is_tracking = status == TrackerStatus::Tracking;
        let probe = (status == TrackerStatus::Away).then(|| self.return_probe_perms()).flatten();
        let perms = if is_tracking {
            self.session.as_ref().map(|s| &s.policy.perms)
        } else {
            probe.as_ref()
        };
        self.machine.apply_sensor_policy(perms.is_some(), perms);
        if self.published.as_ref() != Some(&view) {
            let v = view.clone();
            self.state.update(|s| {
                s.status = v.status;
                s.tracking_since = v.tracking_since;
                s.block_started_at = v.block_started_at;
                s.unsaved_since = v.unsaved_since;
                s.next_capture_at = v.next_capture_at;
                s.paused_since = v.paused_since;
                s.resume_at = v.resume_at;
                s.away_since = v.away_since;
                s.away_prompt = v.away_prompt;
                s.selected_project = v.selected_project;
                s.interval_minutes = v.interval_minutes;
                s.idle_threshold_minutes = v.idle_threshold_minutes;
                s.capture = v.capture;
                s.task_note = v.task_note;
                s.server_reachable = v.server_reachable;
                s.clock_skew_minutes = v.clock_skew_minutes;
                s.consent = v.consent;
                s.consent_error = v.consent_error;
                s.consent_unsynced = v.consent_unsynced;
            });
            self.published = Some(view);
            (self.repaint)();
        }
        self.maybe_heartbeat(status);
    }

    /// Presence (CONTRACT §3): every minute while signed in, and at once on
    /// every change of status or project. Never queued.
    pub(super) fn maybe_heartbeat(&mut self, status: TrackerStatus) {
        let timed_break = self.pause.is_some_and(|p| p.until.is_some());
        let in_workspace = self.account.as_ref().is_some_and(|a| a.workspace_id.is_some());
        let online = in_workspace && !self.expired && self.suspended.is_none() && self.shutting_down.is_none();
        let Some(presence) = session::presence(status, timed_break).filter(|_| online) else {
            return;
        };
        let now = self.now();
        let key = (presence, self.session.as_ref().map(|s| s.project_id().to_owned()));
        let changed = self.presence_sent.as_ref() != Some(&key);
        if changed {
            self.presence_since = now;
        }
        if changed || Instant::now() >= self.next_heartbeat {
            if let Some(heartbeat) = self.heartbeat(presence, self.presence_since, now) {
                self.control.send(ControlRequest::Heartbeat(Box::new(heartbeat)));
            }
            self.presence_sent = Some(key);
            self.next_heartbeat = Instant::now() + HEARTBEAT_EVERY;
        }
    }

    /// A heartbeat saying `status` since `since`; `None` without a device id
    /// (the server needs one).
    pub(super) fn heartbeat(&self, status: PresenceStatus, since: DateTime<Utc>, now: DateTime<Utc>) -> Option<Heartbeat> {
        let device = self.machine.device();
        let device_id = device.device_id.clone()?;
        let session = self.session.as_ref();
        // The app in front only while tracking, and only when the project
        // records the active window.
        let current_app = if status == PresenceStatus::Tracking && session.is_some_and(|s| s.policy.perms.active_window) {
            self.machine.current_app()
        } else {
            None
        };
        Some(Heartbeat {
            device_id,
            device_name: device.device_name.clone(),
            platform: Some(device.platform.to_owned()),
            os_version: device.os_version.clone(),
            agent_version: Some(device.agent_version.to_owned()),
            status,
            project_id: session.map(|s| s.project_id().to_owned()),
            status_since: Some(since),
            current_app,
            time_zone: crate::tracking::system::time_zone(),
            utc_offset_minutes: Some(crate::tracking::system::utc_offset_minutes(now)),
            sent_at: None,
        })
    }

    pub(super) fn clear_error(&self, message: &str) {
        self.state.update(|s| {
            if s.last_error.as_deref() == Some(message) {
                s.last_error = None;
            }
        });
    }
}
