//! The window's commands: pause and resume, "Sync now", the task note,
//! the time away kept or discarded, and quitting.

use std::time::Duration;

use chrono::{DateTime, TimeDelta, Utc};
use tokio::time::Instant;

use crate::credentials::SavedPause;
use crate::model::{EndReason, PresenceStatus};
use crate::state::Command;
use crate::tracking::MIN_BLOCK;

use super::queue::{enforce_queue_limits, publish_queue};
use super::recording::Speed;
use super::uploader::UploadCommand;
use super::{Worker, FAREWELL_WAIT, QUIT_UPLOAD_GRACE};

/// The longest task note ("≤ 200 chars in the UI", CONTRACT §4).
pub const TASK_NOTE_MAX_CHARS: usize = 200;

impl Worker {
    pub(super) fn on_command(&mut self, command: Command) {
        // The user is here: tracking starting later is no surprise.
        self.announce_autostart = false;
        match command {
            Command::SignIn { workspace, username, password, server } => self.sign_in(workspace, username, password, server),
            Command::ChooseWorkspace { index } => self.choose_workspace(index),
            Command::CancelWorkspaceChoice => self.clear_choices(),
            Command::SwitchWorkspace => self.switch_workspace(),
            Command::SwitchToWorkspace { index, at } => {
                self.user_acted(at);
                self.switch_to(index, at);
            }
            Command::SignOut { at } => self.sign_out(at),
            Command::SelectProject { project_id, at } => {
                self.user_acted(at);
                self.select_project(&project_id, at);
            }
            Command::Pause { resume_after, at } => {
                self.user_acted(at);
                self.pause(resume_after, at);
            }
            Command::Resume { at } => {
                self.user_acted(at);
                self.resume(at);
            }
            Command::SyncNow { at } => {
                self.user_acted(at);
                self.sync_now(at);
            }
            Command::SetTaskNote { note, at } => {
                self.user_acted(at);
                self.set_task_note(&note, at);
            }
            Command::ResolveAway { keep } => self.resolve_away(keep),
            Command::Shutdown { .. } => {}
            Command::UpdateConsent { consent } => self.update_consent(consent),
        }
        self.state.update(|s| s.pending_action = None);
    }

    pub(super) fn pause(&mut self, resume_after: Option<Duration>, at: DateTime<Utc>) {
        let until = resume_after.and_then(|d| TimeDelta::from_std(d).ok()).map(|d| at + d);
        match self.pause.as_mut() {
            // Already paused: only when (or whether) it ends changes.
            Some(pause) => pause.until = until,
            None => {
                self.away = None;
                self.pause = Some(SavedPause { since: at, until });
                let reason = if until.is_some() { EndReason::Break } else { EndReason::Pause };
                self.reconcile(at, reason);
            }
        }
        self.persist_pause();
    }

    pub(super) fn resume(&mut self, at: DateTime<Utc>) {
        if self.pause.take().is_some() {
            self.persist_pause();
            // Not a second of the pause (or input made during it) is billed:
            // a fresh block starts at the click.
            self.reconcile(at, EndReason::Interval);
        }
    }

    /// Save the block in progress now (if it is a minute old) and upload at once.
    pub(super) fn sync_now(&mut self, at: DateTime<Utc>) {
        let now = self.now();
        let old_enough = self.blocks.stretch().is_some_and(|s| at.min(now) - s.block.started_at >= MIN_BLOCK);
        if old_enough {
            let fraction = self.machine.random_fraction();
            let finished = self.blocks.cut(at, now, EndReason::Interval, fraction);
            self.record(finished, Speed::Normal);
        }
        self.uploader.send(UploadCommand::Now);
        self.totals_wanted = true;
        self.next_totals = Instant::now();
    }

    /// A new task note: it is each block's description, so the block in
    /// progress ends with the old one (by `task`). A block under a minute old
    /// takes the new note instead, rather than being dropped as too short.
    pub(super) fn set_task_note(&mut self, note: &str, at: DateTime<Utc>) {
        let note = clean_note(note);
        if note == self.task_note {
            return;
        }
        self.task_note = note.clone();
        if let Some(saved) = self.saved.as_mut() {
            saved.task_note = note.clone();
        }
        self.persist();
        let now = self.now();
        match self.blocks.stretch().map(|s| at.min(now) - s.block.started_at < MIN_BLOCK) {
            Some(true) => self.blocks.set_description(note, true),
            Some(false) => {
                self.blocks.set_description(note, false);
                let fraction = self.machine.random_fraction();
                let finished = self.blocks.cut(at, now, EndReason::Task, fraction);
                self.record(finished, Speed::Normal);
            }
            None => {}
        }
    }

    /// "Keep as work": the time away is queued as idle blocks of at most one
    /// interval, on the project tracked then, with the current task note.
    pub(super) fn resolve_away(&mut self, keep: bool) {
        let Some(prompt) = self.prompt.take() else {
            return;
        };
        if !keep {
            tracing::info!(minutes = (prompt.to - prompt.from).num_minutes(), "time away discarded");
            return;
        }
        // Also while the sign-in is expired: the queue keeps the account's
        // rows for when the password is entered again.
        let Some((owner, workspace)) = self.account.as_ref().and_then(|a| Some((a.owner(), a.workspace_id.clone()?))) else {
            tracing::info!("time away not kept: signed out");
            return;
        };
        let now = self.now();
        let interval = prompt.session.policy.interval.to_std().unwrap_or(Duration::from_secs(600));
        let blocks = self.machine.kept_idle_blocks(prompt.session.project_id(), prompt.from, prompt.to, interval, self.task_note.as_deref());
        let mut queued = 0;
        for activity in &blocks {
            if self.enqueue(activity, &owner, &workspace, now) {
                queued += 1;
            }
        }
        enforce_queue_limits(&self.store);
        publish_queue(&self.store, &self.state, Some(&owner));
        if queued > 0 {
            self.uploader.send(UploadCommand::RowsQueued);
        }
        tracing::info!(queued, "time away kept as work");
    }

    /// Quit: save the block in progress first — so whatever happens next it
    /// goes up on the next launch — then say "stopped" and upload what fits.
    pub(super) async fn quit(&mut self, at: DateTime<Utc>) {
        let started = std::time::Instant::now();
        let now = self.now();
        let finished = self.blocks.end(at, now, EndReason::Quit);
        // The last block keeps what was captured for it with consent (its
        // screenshot, the running programs): the sensors stop only once it is
        // queued. Fast: no new capture (no webcam photo) on the way out; the
        // uploader's final drain below sends it.
        self.record(finished, Speed::Fast);
        // Then all OS sensors halt, before the farewell heartbeat and the
        // upload drain.
        self.machine.apply_sensor_policy(false, None);
        // An unanswered question about time away: no answer means discard.
        self.prompt = None;
        let heartbeat = if self.account.is_some() && !self.expired {
            self.heartbeat(PresenceStatus::Stopped, now, now)
        } else {
            None
        };
        self.control.farewell(heartbeat, FAREWELL_WAIT).await;
        let deadline = started + QUIT_UPLOAD_GRACE;
        self.uploader.send(UploadCommand::Shutdown { deadline });
        self.uploader.finish(deadline + Duration::from_secs(1)).await;
        // The final uploads may have renewed (and so rotated) the refresh token.
        self.save_rotated_token();
    }
}

/// The task note as kept: control characters gone, whitespace collapsed, at
/// most [`TASK_NOTE_MAX_CHARS`]; `None` when blank.
pub fn clean_note(note: &str) -> Option<String> {
    let collapsed = note
        .split(|c: char| c.is_whitespace() || c.is_control())
        .filter(|word| !word.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    let note: String = collapsed.chars().take(TASK_NOTE_MAX_CHARS).collect();
    let note = note.trim_end();
    (!note.is_empty()).then(|| note.to_string())
}
