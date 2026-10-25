//! The queue's bounds: images shed and old rows dropped, parked and foreign
//! rows purged, and freed space given back to the disk.

use anyhow::Result;

use super::open::{switch_to_incremental_vacuum, VACUUM_SWITCH_MAX_LIVE_BYTES};
use super::{Storage, STRIP_CAPTURES};

/// Bounds that keep the offline queue from growing without limit while the
/// backend is unreachable.
///
/// Screenshots/webcam frames are by far the bulk of a row, so they are shed
/// first: pending rows keep their images only while they are among the newest
/// `capture_max_rows`, the images of all newer rows plus their own stay within
/// `capture_max_bytes`, and they are younger than `capture_max_age_days`.
/// Rows past that keep their time data (start/end, counts, activity %,
/// windows) but lose the images. Only beyond `max_pending_rows` are the
/// oldest rows dropped outright.
pub struct QueueLimits {
    pub capture_max_rows: u32,
    pub capture_max_bytes: u64,
    pub capture_max_age_days: u32,
    pub max_pending_rows: u32,
}

impl QueueLimits {
    /// ~500 rows is ~80 h of tracking at the default 10-minute interval — two
    /// working weeks offline with every screenshot intact — and 300 MB holds
    /// them at typical JPEG sizes. 10,000 time-only rows is a few tens of MB
    /// and years of normal use.
    pub const DEFAULT: QueueLimits = QueueLimits {
        capture_max_rows: 500,
        capture_max_bytes: 300 * 1024 * 1024,
        capture_max_age_days: 30,
        max_pending_rows: 10_000,
    };
}

/// What [`Storage::enforce_limits`] did, for logging.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct LimitReport {
    pub stripped: usize,
    pub dropped: usize,
}

impl Storage {
    /// Drop parked rows, and rows held for a workspace that never came back,
    /// older than `days`. Without this the queue — and the base64 captures
    /// inside it — would grow without bound for the life of the install.
    /// Returns how many rows were removed.
    pub fn purge_rejected_older_than(&self, days: u32) -> Result<usize> {
        let removed = self.conn.execute(
            "DELETE FROM activities \
             WHERE (synced = 2 OR (synced = 0 AND held IS NOT NULL)) AND created_at < datetime('now', ?1)",
            [format!("-{days} days")],
        )?;
        Ok(removed)
    }

    /// Drop pending rows that belong to anyone other than `current_owner` (or
    /// to no one) once they are older than `days`. Another account's rows wait
    /// for that account to sign in again, but not forever. With nobody signed
    /// in, every pending row older than `days` goes.
    pub fn purge_foreign_pending_older_than(&self, current_owner: Option<&str>, days: u32) -> Result<usize> {
        let removed = self.conn.execute(
            "DELETE FROM activities \
             WHERE synced = 0 AND (?1 IS NULL OR owner IS NOT ?1) \
               AND created_at < datetime('now', ?2)",
            rusqlite::params![current_owner, format!("-{days} days")],
        )?;
        Ok(removed)
    }

    /// Apply [`QueueLimits`]: shed images from old/excess pending rows (newest
    /// first keep theirs, within the row and byte budgets), then drop the
    /// oldest rows beyond the hard cap.
    pub fn enforce_limits(&self, limits: &QueueLimits) -> Result<LimitReport> {
        let stripped = self.conn.execute(
            &format!(
                "UPDATE activities SET payload = {STRIP_CAPTURES}, stripped = 1, capture_bytes = 0 \
                 WHERE id IN ( \
                     SELECT id FROM ( \
                         SELECT id, stripped, created_at, \
                                ROW_NUMBER() OVER newest AS rank, \
                                SUM(capture_bytes) OVER newest AS kept_bytes \
                         FROM activities WHERE synced = 0 \
                         WINDOW newest AS (ORDER BY id DESC ROWS BETWEEN UNBOUNDED PRECEDING AND CURRENT ROW)) \
                     WHERE stripped = 0 \
                       AND (rank > ?2 OR kept_bytes > ?3 OR created_at < datetime('now', ?1)))"
            ),
            rusqlite::params![
                format!("-{} days", limits.capture_max_age_days),
                limits.capture_max_rows,
                i64::try_from(limits.capture_max_bytes).unwrap_or(i64::MAX)
            ],
        )?;
        let dropped = self.conn.execute(
            "DELETE FROM activities WHERE synced = 0 AND id NOT IN \
             (SELECT id FROM activities WHERE synced = 0 ORDER BY id DESC LIMIT ?1)",
            [limits.max_pending_rows],
        )?;
        Ok(LimitReport { stripped, dropped })
    }

    /// Give the pages freed by uploads, parked rows and shed images back to
    /// the disk. Cheap when there is nothing to free; run it after a drain.
    /// A queue from before incremental vacuum is switched to it here once its
    /// live data is small enough to rebuild quickly.
    pub fn reclaim_space(&self) -> Result<()> {
        self.reclaim_space_with(VACUUM_SWITCH_MAX_LIVE_BYTES)
    }

    pub(super) fn reclaim_space_with(&self, vacuum_switch_max_live_bytes: i64) -> Result<()> {
        if !switch_to_incremental_vacuum(&self.conn, vacuum_switch_max_live_bytes)? {
            // Still a large backlog: its free pages are reused by new rows
            // meanwhile, and the switch is tried after the next drain.
            return Ok(());
        }
        // incremental_vacuum frees one page per step, so it is stepped to the end.
        let mut stmt = self.conn.prepare("PRAGMA incremental_vacuum")?;
        let mut rows = stmt.query([])?;
        while rows.next()?.is_some() {}
        Ok(())
    }
}
