//! Recording: finished blocks built (with their consented captures) and
//! queued locally.

use std::sync::Arc;

use chrono::{DateTime, Local, Utc};

use crate::model::{Activity, EndReason};
use crate::state::LastScreenshot;
use crate::tracking::{BlockSpec, ProvenanceScreenCapture};

use super::blocks::{BlockId, FinishedBlock, Stretch};
use super::calendar::{local_day_and_week_start, tracked_secs_since};
use super::queue::{decode_thumbnail, enforce_queue_limits, publish_queue, push_recent_block, recent_block};
use super::uploader::UploadCommand;
use super::{Worker, CAPTURE_SLACK, LATE};

/// Whether to take the time to capture: not before sleep or shutdown, when
/// the block must be saved within Windows' patience (and the network is off
/// limits).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Speed {
    Normal,
    Fast,
}

impl Speed {
    pub(super) fn for_reason(reason: EndReason) -> Self {
        match reason {
            EndReason::Sleep | EndReason::Shutdown => Speed::Fast,
            _ => Speed::Normal,
        }
    }
}

impl Worker {
    /// Build the finished blocks and queue them locally.
    pub(super) fn record(&mut self, finished: Vec<FinishedBlock>, speed: Speed) {
        let tagged = self.account.as_ref().and_then(|a| Some((a.owner(), a.workspace_id.clone()?)));
        let Some((owner, workspace)) = tagged.filter(|_| !finished.is_empty()) else {
            self.prune_screens();
            return;
        };
        let now = self.now();
        let current_perms = self.current_effective_perms();
        let mut queued = 0;
        for block in finished {
            let late = now - block.ended_at > LATE;
            let taken = self
                .take_screen(block.id)
                .filter(|capture| capture.capture.captured_at <= block.ended_at + CAPTURE_SLACK);
            // Ended before its screenshot moment: taken at the end instead —
            // unless the machine is locked, asleep, idle or shutting down,
            // or screenshot permission has been withdrawn.
            let capture_now = taken.is_none()
                && speed == Speed::Normal
                && !late
                && block.capture_at_end
                && block.policy.perms.screenshot
                && current_perms.screenshot
                && block.end_reason.allows_capture_at_end();
            let screen = if capture_now {
                self.machine.capture_screen(block.policy.perms.blur_screenshots || current_perms.blur_screenshots)
            } else if current_perms.screenshot {
                taken
            } else {
                None
            };
            let mut perms = block.policy.perms;
            if speed == Speed::Fast || late || !current_perms.webcam {
                perms.webcam = false;
            }
            if !current_perms.running_programs {
                perms.running_programs = false;
            }
            let spec = BlockSpec {
                project_id: &block.policy.project_id,
                perms: &perms,
                started_at: block.started_at,
                ended_at: block.ended_at,
                end_reason: Some(block.end_reason),
                description: block.description.as_deref(),
            };
            let activity = self.machine.snapshot(&spec, screen);
            if self.enqueue(&activity, &owner, &workspace, now) {
                queued += 1;
            }
        }
        self.prune_screens();
        enforce_queue_limits(&self.store);
        publish_queue(&self.store, &self.state, Some(&owner));
        // Before sleep or shutdown nothing goes over the network.
        if queued > 0 && speed == Speed::Normal {
            self.uploader.send(UploadCommand::RowsQueued);
        }
        (self.repaint)();
    }

    /// Queue one block and show it (and count it in the totals at once; the
    /// server's totals, refreshed after the upload, replace that estimate).
    pub(super) fn enqueue(&mut self, activity: &Activity, owner: &str, workspace: &str, now: DateTime<Utc>) -> bool {
        let id = match self.store.enqueue(activity, owner, workspace) {
            Ok(id) => id,
            Err(e) => {
                self.enqueue_failures = self.enqueue_failures.saturating_add(1);
                tracing::error!(?e, failures = self.enqueue_failures, "failed to queue activity");
                self.state.update(|s| s.last_error = Some(format!("Could not save locally: {e}")));
                return false;
            }
        };
        if std::mem::take(&mut self.enqueue_failures) > 0 {
            tracing::info!("blocks are saved locally again");
            self.state.update(|s| {
                if s.last_error.as_deref().is_some_and(|e| e.starts_with("Could not save locally")) {
                    s.last_error = None;
                }
            });
        }
        tracing::debug!(id, end_reason = ?activity.end_reason, "activity queued");
        let (today_start, week_start) = local_day_and_week_start(now, &Local);
        let project = self
            .projects
            .iter()
            .flatten()
            .find(|p| p.id == activity.project_id)
            .map(|p| p.title.clone())
            .unwrap_or_default();
        let screenshot = decode_thumbnail(activity);
        let captured_at = activity.screenshot_captured_at.unwrap_or(now);
        let stretch = self.blocks.stretch();
        let (block_started_at, unsaved_since) = (stretch.map(|s| s.block.started_at), stretch.map(Stretch::unsaved_since));
        let (started_at, ended_at) = (activity.started_at, activity.ended_at);
        let block = recent_block(id, project, activity);
        self.state.update(|s| {
            s.tracked_today_secs += tracked_secs_since(started_at, ended_at, today_start);
            s.tracked_week_secs += tracked_secs_since(started_at, ended_at, week_start);
            // Moved in the same update, so the live total never counts this
            // block twice.
            if s.unsaved_since.is_some() {
                s.block_started_at = block_started_at;
                s.unsaved_since = unsaved_since;
            }
            push_recent_block(&mut s.recent_blocks, block);
            if let Some(jpeg) = screenshot {
                s.last_screenshot = Some(Arc::new(LastScreenshot { taken_at: captured_at, jpeg }));
            }
        });
        true
    }

    pub(super) fn take_screen(&mut self, id: BlockId) -> Option<ProvenanceScreenCapture> {
        let index = self.screens.iter().position(|(block, _)| *block == id)?;
        Some(self.screens.swap_remove(index).1)
    }

    /// Forget screenshots of blocks that are gone (dropped by an idle cut).
    pub(super) fn prune_screens(&mut self) {
        let live = self.blocks.stretch().map(Stretch::block_ids).unwrap_or_default();
        self.screens.retain(|(id, _)| live.contains(id));
    }

    pub(super) fn capture(&mut self, id: BlockId) {
        let current_perms = self.current_effective_perms();
        if !current_perms.screenshot {
            return;
        }
        let blur = current_perms.blur_screenshots
            || self.blocks.stretch().is_some_and(|s| s.block.policy.perms.blur_screenshots);
        if let Some(screen) = self.machine.capture_screen(blur) {
            self.screens.push((id, screen));
        }
        self.blocks.mark_captured(id);
    }
}
