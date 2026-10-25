//! The local queue as the worker keeps it: bounded, and shown in the window.

use crate::model::Activity;
use crate::state::{RecentBlock, StateHandle, UploadState, RECENT_BLOCKS};
use crate::storage::{QueueLimits, Storage};

use super::{QUEUE_ROWS_SHOWN, REJECTED_RETENTION_DAYS};

/// Housekeeping that bounds the local queue: expire parked rows and other
/// accounts' stale rows, then shed images/rows beyond [`QueueLimits::DEFAULT`].
pub(super) fn maintain_queue(store: &Storage, owner: Option<&str>) {
    match store.purge_rejected_older_than(REJECTED_RETENTION_DAYS) {
        Ok(n) if n > 0 => tracing::info!(purged = n, "removed old rejected activities"),
        Err(e) => tracing::warn!(?e, "could not purge rejected activities"),
        _ => {}
    }
    match store.purge_foreign_pending_older_than(owner, REJECTED_RETENTION_DAYS) {
        Ok(n) if n > 0 => tracing::info!(purged = n, "removed expired activities of other accounts"),
        Err(e) => tracing::warn!(?e, "could not purge other accounts' activities"),
        _ => {}
    }
    enforce_queue_limits(store);
}

pub(super) fn enforce_queue_limits(store: &Storage) {
    match store.enforce_limits(&QueueLimits::DEFAULT) {
        Ok(report) if report.stripped > 0 || report.dropped > 0 => tracing::warn!(
            stripped = report.stripped,
            dropped = report.dropped,
            "offline queue over its limits; shed images / oldest activities"
        ),
        Err(e) => tracing::warn!(?e, "could not enforce offline queue limits"),
        _ => {}
    }
}

/// Show the signed-in account's queue: pending / rejected counts and the
/// newest rows with the server's reasons (nothing when signed out).
pub(super) fn publish_queue(store: &Storage, state: &StateHandle, owner: Option<&str>) {
    let (pending, rejected, rows) = match owner {
        Some(owner) => (
            store.pending_count(owner).unwrap_or(0),
            store.rejected_count(owner).unwrap_or(0),
            store.recent_rows(owner, QUEUE_ROWS_SHOWN).unwrap_or_default(),
        ),
        None => (0, 0, Vec::new()),
    };
    state.update(|s| {
        s.pending_uploads = pending;
        s.rejected_uploads = rejected;
        s.queue_rows = rows;
    });
}

/// The window's summary of a block just queued.
pub(super) fn recent_block(row_id: i64, project: String, activity: &Activity) -> RecentBlock {
    RecentBlock {
        row_id,
        project,
        started_at: activity.started_at,
        ended_at: activity.ended_at,
        productivity: activity.productivity.unwrap_or(0),
        keyboard_hits: activity.keyboard_hits,
        mouse_clicks: activity.mouse_clicks,
        screenshot: activity.screenshot_png_base64.is_some(),
        upload: UploadState::Queued,
    }
}

/// Newest first, bounded to [`RECENT_BLOCKS`].
pub(super) fn push_recent_block(blocks: &mut Vec<RecentBlock>, block: RecentBlock) {
    blocks.insert(0, block);
    blocks.truncate(RECENT_BLOCKS);
}

/// Record where a queued block is now (uploaded / rejected), if it is listed.
pub(super) fn set_upload_state(state: &StateHandle, row_id: i64, upload: UploadState) {
    state.update(|s| {
        if let Some(block) = s.recent_blocks.iter_mut().find(|b| b.row_id == row_id) {
            block.upload = upload;
        }
    });
}

/// The screenshot's small JPEG, decoded for the window's preview.
pub(super) fn decode_thumbnail(activity: &Activity) -> Option<Vec<u8>> {
    use base64::Engine as _;
    let encoded = activity.screenshot_thumb_jpg_base64.as_deref()?;
    base64::engine::general_purpose::STANDARD.decode(encoded).ok()
}
