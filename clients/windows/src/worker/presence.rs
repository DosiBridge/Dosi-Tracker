//! Whether the user is at the computer and tracking: the block started or
//! stopped to match the facts, idle and the return from it, timed breaks,
//! the screen lock, sleep and shutdown, and the clock tick that notices
//! them.

use chrono::{DateTime, TimeDelta, Utc};
use tokio::time::Instant;

use crate::config::CapturePermissions;
use crate::model::EndReason;
use crate::session_events::{SystemEvent, SystemNotice};
use crate::state::NoticeKind;
use crate::tracking::MIN_BLOCK;

use super::account::Account;
use super::control::ControlRequest;
use super::queue::{maintain_queue, publish_queue};
use super::recording::Speed;
use super::uploader::UploadCommand;
use super::{
    Away, PendingPrompt, Worker, MAINTENANCE_EVERY, PROJECTS_REFRESH, RECONNECT_EVERY, TOTALS_EVERY, TOTALS_SPACING,
};

/// The input hooks confirm a return when they see the user's own input
/// within this long of Windows' idle clock first moving: a tick (2 s) to
/// notice it, up to 3 s to attach, a tick to look. Input the clock saw
/// earlier than that was not confirmed (a jiggler, say) and is not work.
pub(super) const CONFIRMED_WITHIN: TimeDelta = TimeDelta::seconds(10);

/// Hooks asked for to confirm a return that are still not attached this
/// long after the idle clock moved will not be soon (they back off for up
/// to 5 minutes): the idle clock decides, as it does without them.
pub(super) const HOOKS_ATTACH_WAIT: TimeDelta = TimeDelta::seconds(10);

impl Worker {
    // ---- Tracking on or off ------------------------------------------------------

    pub(super) fn should_track(&self) -> bool {
        self.cfg.consent.is_granted()
            && !self.consent_unsynced
            && self.account.is_some()
            && !self.expired
            && self.session.is_some()
            && self.pause.is_none()
            && self.away.is_none()
            && !self.locked
            && !self.disconnected
            && self.suspended.is_none()
            && self.shutting_down.is_none()
    }

    /// Returns the currently active effective permissions consented by the user and permitted
    /// by session/policy. If consent is absent, revoked, or unsynced (fail-closed), all permissions
    /// are strictly false.
    pub(super) fn current_effective_perms(&self) -> CapturePermissions {
        if !self.cfg.consent.is_granted() || self.consent_unsynced {
            return CapturePermissions::all_disabled();
        }
        self.session
            .as_ref()
            .map(|s| s.policy.perms)
            .unwrap_or_else(CapturePermissions::all_disabled)
    }

    /// Start or stop the stretch to match the facts; a stop ends the block
    /// at `at` by `reason`.
    pub(super) fn reconcile(&mut self, at: DateTime<Utc>, reason: EndReason) {
        let now = self.now();
        if self.should_track() {
            if self.blocks.is_open() {
                return;
            }
            let Some(session) = self.session.as_ref() else {
                return;
            };
            let (policy, title) = (session.policy.clone(), session.title.clone());
            let fraction = self.machine.random_fraction();
            self.blocks.start(at.min(now), policy, self.task_note.clone(), fraction);
            if std::mem::take(&mut self.announce_autostart) {
                self.state.update(|s| s.push_notice(NoticeKind::TrackingStartedAutomatically { project_title: title }, now));
            }
        } else if self.blocks.is_open() {
            let finished = self.blocks.end(at, now, reason);
            self.record(finished, Speed::for_reason(reason));
        }
    }

    // ---- The clock ---------------------------------------------------------------------

    pub(super) fn on_tick(&mut self) {
        let now = self.now();
        if let Some(before_gap) = self.sleep.observe(now, self.machine.awake_millis()) {
            tracing::info!(%before_gap, "the computer slept without announcing it");
            self.missed_suspend(before_gap, now);
        }
        // Input after a sleep or shutdown was announced proves the machine is
        // awake and the session goes on (the resume or the cancellation was
        // never heard): tracking must not stay off for good.
        if let Some(announced) = self.suspended.or(self.shutting_down) {
            if let Some(back) = self.machine.first_input_after(announced, now) {
                tracing::warn!(%announced, "input after an announced sleep or shutdown; tracking again");
                self.suspended = None;
                self.shutting_down = None;
                self.back_at_computer(back);
            }
        }
        self.check_return(now);
        self.check_break_end(now);
        if let Some(id) = self.blocks.capture_due(now) {
            self.capture(id);
        }
        self.save_rotated_token();
        let last_input = self.machine.last_input(now);
        let machine = self.machine.clone();
        let outcome = self.blocks.tick(now, last_input, &mut || machine.random_fraction());
        self.record(outcome.finished, Speed::Normal);
        if let Some(since) = outcome.went_idle {
            self.went_away(since);
        }
        self.housekeeping();
    }

    /// The wall clock jumped ahead of the awake clock: the machine slept and
    /// nobody said so. The block ends at the last tick before the gap.
    pub(super) fn missed_suspend(&mut self, before_gap: DateTime<Utc>, now: DateTime<Utc>) {
        self.awake_since = Some(now);
        self.freeze_away(before_gap);
        if self.blocks.stretch().is_some_and(|s| s.since < before_gap) {
            let finished = self.blocks.end(before_gap, now, EndReason::Sleep);
            self.record(finished, Speed::Fast);
            self.uploader.send(UploadCommand::RowsQueued);
            // A new block from now on.
            self.reconcile(now, EndReason::Sleep);
        }
    }

    /// No input for the idle threshold: the block ended at the last input.
    pub(super) fn went_away(&mut self, since: DateTime<Utc>) {
        tracing::info!(%since, "no input for the idle threshold; away");
        // An earlier question nobody answered: no answer means discard.
        self.prompt = None;
        self.away = Some(Away { since, until: None, after_break: false, session: self.session.clone(), noticed: None });
    }

    pub(super) fn present(&self) -> bool {
        !(self.locked || self.disconnected || self.suspended.is_some() || self.shutting_down.is_some())
    }

    /// Back at the computer? While away the input hooks are off, so Windows'
    /// idle clock says first. It is also moved by mouse jigglers and remote
    /// tools, so when the project records input the hooks confirm it: they
    /// are attached (counting only, nothing else runs) and the user's own
    /// next input ends the time away. It ends where the idle clock first saw
    /// input only when the hooks confirm it right after ([`CONFIRMED_WITHIN`]);
    /// a jiggler that moved the clock long before the user came back gives
    /// none of that time. Without input permission nothing can confirm it,
    /// and the idle clock decides alone — as it does when the hooks cannot
    /// attach ([`HOOKS_ATTACH_WAIT`]), or the user would stay away for good.
    pub(super) fn check_return(&mut self, now: DateTime<Utc>) {
        let Some((since, noticed)) = self.away.as_ref().map(|away| (away.since, away.noticed)) else {
            return;
        };
        if !self.present() {
            return;
        }
        let Some(seen) = self.machine.first_input_after(since, now) else {
            return;
        };
        // Where the idle clock first moved, if it was just now: earlier, the
        // hooks were checking and the user was not back yet.
        let first = noticed.filter(|noticed| seen - *noticed <= CONFIRMED_WITHIN);
        if !self.hooks_confirm_return() {
            self.come_back(first.map_or(seen, |noticed| noticed.min(seen)));
            return;
        }
        if !self.machine.input_hooks_running() {
            // `seen` is the idle clock's: the hooks are asked for to confirm it.
            let Some(noticed) = noticed else {
                tracing::debug!(%seen, "input while away; confirming it with the input hooks");
                if let Some(away) = self.away.as_mut() {
                    away.noticed = Some(seen);
                }
                return;
            };
            if self.machine.input_hooks_failed() || now - noticed >= HOOKS_ATTACH_WAIT {
                tracing::warn!(%noticed, "the input hooks cannot attach to confirm the return; Windows' idle clock decides");
                self.come_back(noticed.min(seen));
            }
            return;
        }
        // The hooks saw the user's own input at `seen`.
        self.come_back(first.map_or(seen, |noticed| noticed.min(seen)));
    }

    /// Can the input hooks tell the user's own input from injected input on
    /// their return? Only when the project (and consent) lets them see both
    /// keys and the mouse.
    pub(super) fn hooks_confirm_return(&self) -> bool {
        let perms = self.current_effective_perms();
        perms.keyboard && perms.mouse
    }

    /// While away, after Windows' idle clock showed input: the input hooks,
    /// alone, to confirm the return.
    pub(super) fn return_probe_perms(&self) -> Option<CapturePermissions> {
        let probing = self.away.as_ref().is_some_and(|away| away.noticed.is_some()) && self.present();
        let perms = self.current_effective_perms();
        (probing && perms.keyboard && perms.mouse)
            .then(|| CapturePermissions { keyboard: true, mouse: true, ..CapturePermissions::all_disabled() })
    }

    /// The user is back at `at`: a new block starts there, and — unless they
    /// were on a break — they are asked about the time away.
    pub(super) fn come_back(&mut self, at: DateTime<Utc>) {
        let Some(away) = self.away.take() else {
            return;
        };
        let at = at.min(self.now()).max(away.since);
        if let (false, Some(session)) = (away.after_break, away.session) {
            let to = away.until.unwrap_or(at);
            if to - away.since >= MIN_BLOCK {
                self.prompt = Some(PendingPrompt { from: away.since, to, session });
            }
        }
        tracing::info!(%at, "back at the computer");
        self.reconcile(at, EndReason::Interval);
    }

    /// The screen locked or the machine slept while away: that time is never
    /// tracked, so the time away ends there.
    pub(super) fn freeze_away(&mut self, at: DateTime<Utc>) {
        if let Some(away) = self.away.as_mut() {
            away.until.get_or_insert(at.max(away.since));
        }
    }

    /// A timed break ran out: tracking resumes where it ended — or, when
    /// nobody is at the computer, with the next input (CONTRACT §4).
    pub(super) fn check_break_end(&mut self, now: DateTime<Utc>) {
        let Some(pause) = self.pause else {
            return;
        };
        let Some(until) = pause.until.filter(|until| *until <= now) else {
            return;
        };
        self.pause = None;
        self.persist_pause();
        let threshold = self.session.as_ref().and_then(|s| s.policy.idle_threshold);
        let last = self.machine.last_input(now).map_or(pause.since, |last| last.max(pause.since));
        if self.session.is_some() && threshold.is_some_and(|threshold| now - last >= threshold) {
            tracing::info!("the break is over, but nobody is at the computer; resuming with the next input");
            self.away = Some(Away { since: last, until: None, after_break: true, session: None, noticed: None });
        } else {
            // Not inside a sleep (or a lock) that outlasted the break: the
            // time before the machine was last awake again is not work.
            let resume = self.awake_since.map_or(until, |awake| until.max(awake)).min(now);
            tracing::info!("timed pause is over; resuming tracking");
            self.reconcile(resume, EndReason::Interval);
        }
    }

    /// What is due: the queue's housekeeping, refreshing the projects and
    /// the totals.
    pub(super) fn housekeeping(&mut self) {
        let now = Instant::now();
        if now >= self.next_maintenance {
            self.next_maintenance = now + MAINTENANCE_EVERY;
            let owner = self.account.as_ref().map(Account::owner);
            maintain_queue(&self.store, owner.as_deref());
            publish_queue(&self.store, &self.state, owner.as_deref());
        }
        let in_workspace = self.account.as_ref().is_some_and(|a| a.workspace_id.is_some());
        if !in_workspace || self.expired || self.suspended.is_some() || self.shutting_down.is_some() {
            return;
        }
        if now >= self.next_projects_refresh {
            self.control.send(ControlRequest::RefreshProjects);
            let online = self.reachable && !self.contacting;
            self.next_projects_refresh = now + if online { PROJECTS_REFRESH } else { RECONNECT_EVERY };
        }
        if now >= self.totals_due_at {
            self.totals_wanted = true;
        }
        if self.totals_wanted && now >= self.next_totals {
            self.control.send(ControlRequest::RefreshTotals);
            self.totals_wanted = false;
            self.next_totals = now + TOTALS_SPACING;
            self.totals_due_at = now + TOTALS_EVERY;
        }
    }

    // ---- Session and power events ------------------------------------------------------

    pub(super) fn on_system(&mut self, mut notice: SystemNotice) {
        let at = notice.at;
        match notice.event {
            SystemEvent::Locked | SystemEvent::Disconnected => {
                if notice.event == SystemEvent::Locked {
                    self.locked = true;
                } else {
                    self.disconnected = true;
                }
                self.freeze_away(at);
                self.reconcile(at, EndReason::Lock);
            }
            SystemEvent::Unlocked | SystemEvent::Connected => {
                if notice.event == SystemEvent::Unlocked {
                    self.locked = false;
                } else {
                    self.disconnected = false;
                }
                self.back_at_computer(at);
            }
            SystemEvent::Suspending => {
                self.suspended.get_or_insert(at);
                self.freeze_away(at);
                self.reconcile(at, EndReason::Sleep);
            }
            SystemEvent::Resumed => {
                if self.suspended.take().is_some() {
                    // Handled: the tick after the wake must not report it again.
                    self.sleep.reset();
                    self.back_at_computer(at);
                    self.uploader.send(UploadCommand::RowsQueued);
                }
            }
            SystemEvent::SessionEnding => {
                if self.shutting_down.is_none() {
                    self.shutting_down = Some(at);
                    self.freeze_away(at);
                    self.reconcile(at, EndReason::Shutdown);
                }
            }
            SystemEvent::SessionContinues => {
                if self.shutting_down.take().is_some() {
                    self.back_at_computer(at);
                }
            }
        }
        // The block is saved: the window may let Windows go on.
        notice.acknowledge();
    }

    /// Unlocked, reconnected, awake again: someone is at the computer (the
    /// password typed on the lock screen is input the hooks never see).
    pub(super) fn back_at_computer(&mut self, at: DateTime<Utc>) {
        self.awake_since = Some(at);
        if self.away.is_some() && self.present() {
            self.come_back(at);
        } else {
            self.reconcile(at, EndReason::Interval);
        }
    }

    /// A click in the window is input too: an away stretch ends there.
    pub(super) fn user_acted(&mut self, at: DateTime<Utc>) {
        if self.away.is_some() && self.present() {
            self.come_back(at);
        }
    }
}
