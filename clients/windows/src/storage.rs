//! The offline queue: every block is saved here first, then uploaded.
//!
//! This module holds the rows' operations; opening and migrating the
//! database is in [`open`], its bounds and housekeeping in [`limits`].

mod limits;
mod open;

use std::path::PathBuf;

use anyhow::Result;
use chrono::{DateTime, Utc};
use rusqlite::{Connection, OptionalExtension};

use crate::model::Activity;
use crate::signing;

pub use limits::QueueLimits;

/// Files SQLite keeps next to a database (a hot journal is part of it).
pub const SQLITE_SIDECARS: [&str; 3] = ["-journal", "-wal", "-shm"];

/// Identify the account a queued row belongs to: the user *on a server* (the
/// workspace it belongs to is kept per row, in `workspace_id`).
///
/// Rows are only ever uploaded under the token of the user who captured
/// them, to the server they were captured for — otherwise signing out and in
/// as someone else would upload the first user's time and screenshots as the
/// second user's, and the same email on another organisation's server would
/// receive this one's. Emails are case-insensitive on the server and server
/// addresses are compared normalized. The prefix keeps keys of older builds
/// (user only, or workspace + user) from ever matching.
pub fn owner_key(server: &str, username: &str) -> String {
    let server = server.trim().trim_end_matches('/').to_lowercase();
    format!("acct\u{1f}{server}\u{1f}{}", username.trim().to_lowercase())
}

/// Where a queued row stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RowStatus {
    /// Waiting to upload.
    Queued,
    /// Parked: the server refused it; kept for diagnostics, never retried.
    Rejected,
}

/// A pending row to upload, with the workspace it was recorded in.
#[derive(Debug, Clone)]
pub struct PendingRow {
    pub id: i64,
    pub workspace_id: String,
    pub activity: Activity,
    /// Earlier attempts the server answered with an error about this row.
    pub attempts: u32,
}

/// One row of the queue as the window lists it.
#[derive(Debug, Clone, PartialEq)]
pub struct QueuedRow {
    pub id: i64,
    pub project_id: Option<String>,
    pub started_at: Option<DateTime<Utc>>,
    pub ended_at: Option<DateTime<Utc>>,
    pub status: RowStatus,
    /// Rejected: the server's reason. Queued: why the last attempt failed,
    /// if one did.
    pub reason: Option<String>,
    /// Still carries screenshot/webcam images.
    pub has_captures: bool,
}

/// A queue opened by [`Storage::open_or_recover`].
pub struct Opened {
    pub storage: Storage,
    /// Where a damaged database was moved before starting a fresh one.
    #[cfg_attr(not(test), expect(dead_code, reason = "for the window's notice that the queue was reset"))]
    pub moved_aside: Option<PathBuf>,
}

/// SQL expression that removes every image of a row (screenshot, its
/// thumbnail, webcam, and the capture's time and blur flag), leaving the time
/// data intact. Corrupt JSON is left as-is rather than failing the update.
const STRIP_CAPTURES: &str = "CASE WHEN json_valid(payload) \
     THEN json_remove(payload, '$.screenshotPngBase64', '$.screenshotThumbJpgBase64', \
     '$.webcamJpgBase64', '$.screenshotCapturedAt', '$.screenshotBlurred') \
     ELSE payload END";

/// SQL condition of a row the uploader may send now: pending, in a
/// workspace, not held, and not deferred past now.
const DUE: &str = "synced = 0 AND workspace_id IS NOT NULL AND held IS NULL \
     AND (retry_at IS NULL OR retry_at <= CAST(strftime('%s', 'now') AS INTEGER))";

/// Offline-first local store. Activities are persisted here first, then synced
/// to the backend. Unsynced rows survive restarts and network outages.
///
/// Row states (`synced` column): 0 = pending upload, 2 = permanently rejected
/// by the server (parked for diagnostics, never retried). Successfully synced
/// rows are deleted so the queue (and its embedded screenshots) stays small;
/// the database uses incremental auto-vacuum, and [`Storage::reclaim_space`]
/// gives the freed pages back to the disk.
///
/// Every row carries its `owner` ([`owner_key`]); uploads only ever read the
/// signed-in account's rows.
///
/// A pending row the uploader must not send right now is either *held* —
/// its workspace refused it (`held` names why; [`Storage::hold_workspace`])
/// until that workspace is used again ([`Storage::release_workspace`]) — or
/// *deferred* until `retry_at` after the server answered an error about that
/// row alone ([`Storage::defer`]). Neither stops the rows behind it.
pub struct Storage {
    conn: Connection,
    /// Device signing key loaded at startup; `None` only in tests that call
    /// the low-level `open()` constructor without a key.
    signing_key: Option<Vec<u8>>,
}

impl Storage {
    /// Attach the device signing key so that [`Storage::enqueue`] can sign
    /// each payload. Call this once after opening the queue.
    pub fn set_signing_key(&mut self, key: Vec<u8>) {
        self.signing_key = Some(key);
    }

    /// Queue an activity for upload on behalf of `owner`, in `workspace_id`.
    pub fn enqueue(&self, activity: &Activity, owner: &str, workspace_id: &str) -> Result<i64> {
        // Tag the payload before writing it (a local HMAC the server does not
        // verify; see `crate::signing`). The tag is computed over the canonical
        // JSON (without the `payloadSignature` field itself, which is always
        // None at this point).
        let payload = serde_json::to_string(activity)?;
        let sig: Option<String> = self
            .signing_key
            .as_deref()
            .map(|key| signing::sign_payload(key, payload.as_bytes()));

        let iso = |t: DateTime<Utc>| t.to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
        self.conn.execute(
            "INSERT INTO activities (payload, synced, owner, capture_bytes, project_id, started_at, ended_at, sig, workspace_id) \
             VALUES (?1, 0, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            rusqlite::params![
                payload,
                owner,
                capture_bytes(activity),
                activity.project_id,
                iso(activity.started_at),
                iso(activity.ended_at),
                sig,
                workspace_id
            ],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    /// Up to `limit` of `owner`'s pending activities with a row id greater than
    /// `after_id`, oldest first, as (row_id, Activity).
    ///
    /// Reading in small batches keeps memory flat however large the backlog
    /// (each row can embed a base64 screenshot). Paging by id rather than by
    /// offset means a row that fails to leave the queue can never be handed
    /// back in the same pass. A row whose payload no longer parses is parked as
    /// rejected instead of silently sitting in the queue forever.
    /// Make every later write fail (`broken`), as a full disk or a locked
    /// file would — or work again.
    #[cfg(test)]
    pub fn break_writes_for_test(&self, broken: bool) {
        let pragma = if broken { "PRAGMA query_only = ON;" } else { "PRAGMA query_only = OFF;" };
        self.conn.execute_batch(pragma).unwrap();
    }

    #[cfg(test)]
    pub fn pending_batch(&self, owner: &str, after_id: i64, limit: u32) -> Result<Vec<(i64, Activity)>> {
        Ok(self.pending_rows(owner, after_id, limit)?.into_iter().map(|row| (row.id, row.activity)).collect())
    }

    /// [`Storage::pending_batch`], with each row's workspace: the rows that
    /// may be sent now (neither held nor deferred). Rows without a workspace
    /// (queued by older builds) are never uploaded.
    pub fn pending_rows(&self, owner: &str, after_id: i64, limit: u32) -> Result<Vec<PendingRow>> {
        let rows: Vec<(i64, String, Option<String>, String, i64)> = {
            let mut stmt = self.conn.prepare(&format!(
                "SELECT id, payload, sig, workspace_id, attempts FROM activities \
                 WHERE {DUE} AND owner = ?1 AND id > ?2 \
                 ORDER BY id LIMIT ?3"
            ))?;
            let mapped = stmt.query_map(rusqlite::params![owner, after_id, limit], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?))
            })?;
            mapped.collect::<rusqlite::Result<_>>()?
        };

        let mut out = Vec::with_capacity(rows.len());
        for (id, payload, sig, workspace_id, attempts) in rows {
            match serde_json::from_str::<Activity>(&payload) {
                Ok(mut activity) => {
                    // Propagate the stored tag into the Activity (the server
                    // accepts the field but does not verify it).
                    if activity.payload_signature.is_none() {
                        activity.payload_signature = sig;
                    }
                    let attempts = u32::try_from(attempts).unwrap_or(0);
                    out.push(PendingRow { id, workspace_id, activity, attempts });
                }
                Err(e) => {
                    tracing::warn!(id, ?e, "queued activity is unreadable; parking it");
                    self.mark_rejected(id, &format!("unreadable local payload: {e}"))?;
                }
            }
        }
        Ok(out)
    }

    /// How many of `owner`'s activities are waiting to upload (held and
    /// deferred ones included).
    pub fn pending_count(&self, owner: &str) -> Result<u32> {
        let count: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM activities WHERE synced = 0 AND owner = ?1 AND workspace_id IS NOT NULL",
            [owner],
            |row| row.get(0),
        )?;
        Ok(count.max(0) as u32)
    }

    /// How many of `owner`'s activities may be sent now.
    pub fn due_count(&self, owner: &str) -> Result<u32> {
        let count: i64 = self.conn.query_row(
            &format!("SELECT COUNT(*) FROM activities WHERE {DUE} AND owner = ?1"),
            [owner],
            |row| row.get(0),
        )?;
        Ok(count.max(0) as u32)
    }

    /// When the earliest of `owner`'s deferred rows may be sent again.
    pub fn next_retry_at(&self, owner: &str) -> Result<Option<DateTime<Utc>>> {
        let at: Option<i64> = self.conn.query_row(
            "SELECT MIN(retry_at) FROM activities \
             WHERE synced = 0 AND owner = ?1 AND workspace_id IS NOT NULL AND held IS NULL AND retry_at IS NOT NULL",
            [owner],
            |row| row.get(0),
        )?;
        Ok(at.and_then(|secs| DateTime::from_timestamp(secs, 0)))
    }

    /// The server answered an error about this row alone: count the attempt,
    /// say why, and leave it until `retry_at`. Returns the attempts so far.
    pub fn defer(&self, id: i64, reason: &str, retry_at: DateTime<Utc>) -> Result<u32> {
        self.conn.execute(
            "UPDATE activities SET attempts = attempts + 1, error = ?2, retry_at = ?3 WHERE id = ?1 AND synced = 0",
            rusqlite::params![id, reason, retry_at.timestamp()],
        )?;
        let attempts: i64 = self
            .conn
            .query_row("SELECT attempts FROM activities WHERE id = ?1", [id], |row| row.get(0))
            .optional()?
            .unwrap_or(0);
        Ok(u32::try_from(attempts).unwrap_or(0))
    }

    /// The server refused `workspace` for `owner` (lost access, archived):
    /// hold all of its pending rows, with `reason`, until it is used again.
    /// They are not sent again meanwhile — not even on a timer.
    pub fn hold_workspace(&self, owner: &str, workspace: &str, reason: &str) -> Result<usize> {
        Ok(self.conn.execute(
            "UPDATE activities SET held = ?3, error = ?3 WHERE synced = 0 AND owner = ?1 AND workspace_id = ?2",
            rusqlite::params![owner, workspace, reason],
        )?)
    }

    /// `owner` uses `workspace` again (signed in to it, switched to it): its
    /// held rows are sent again.
    pub fn release_workspace(&self, owner: &str, workspace: &str) -> Result<usize> {
        Ok(self.conn.execute(
            "UPDATE activities SET held = NULL, retry_at = NULL \
             WHERE synced = 0 AND owner = ?1 AND workspace_id = ?2 AND held IS NOT NULL",
            rusqlite::params![owner, workspace],
        )?)
    }

    /// `owner`'s newest rows, queued and parked, newest first — what the
    /// window lists, with each parked row's reason. Reads only the summary
    /// columns, never the payloads.
    #[cfg_attr(not(test), expect(dead_code, reason = "listed by the window"))]
    pub fn recent_rows(&self, owner: &str, limit: u32) -> Result<Vec<QueuedRow>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, synced, error, project_id, started_at, ended_at, capture_bytes \
             FROM activities WHERE owner = ?1 AND synced IN (0, 2) \
             ORDER BY id DESC LIMIT ?2",
        )?;
        let parse_time = |value: Option<String>| {
            value
                .as_deref()
                .and_then(|v| DateTime::parse_from_rfc3339(v).ok())
                .map(|t| t.with_timezone(&Utc))
        };
        let rows = stmt.query_map(rusqlite::params![owner, limit], |row| {
            let synced: i64 = row.get(1)?;
            let capture_bytes: i64 = row.get(6)?;
            Ok(QueuedRow {
                id: row.get(0)?,
                status: if synced == 2 { RowStatus::Rejected } else { RowStatus::Queued },
                reason: row.get(2)?,
                project_id: row.get(3)?,
                started_at: parse_time(row.get(4)?),
                ended_at: parse_time(row.get(5)?),
                has_captures: capture_bytes > 0,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// The upload succeeded — the local copy (incl. base64 captures) is no longer needed.
    pub fn remove_synced(&self, id: i64) -> Result<()> {
        self.conn
            .execute("DELETE FROM activities WHERE id = ?1", [id])?;
        Ok(())
    }

    /// Remember why the last upload attempt of a pending row failed (shown
    /// with the row; `None` clears it). The row stays queued.
    #[cfg_attr(not(test), expect(dead_code, reason = "for the worker's upload handling"))]
    pub fn note_attempt(&self, id: i64, reason: Option<&str>) -> Result<()> {
        self.conn.execute(
            "UPDATE activities SET error = ?2 WHERE id = ?1 AND synced = 0",
            rusqlite::params![id, reason],
        )?;
        Ok(())
    }

    /// The server permanently rejected this payload; park it so it never blocks
    /// the queue. The reason and time data are kept for diagnostics; the images
    /// are dropped, since a parked row is never uploaded again.
    pub fn mark_rejected(&self, id: i64, reason: &str) -> Result<()> {
        self.conn.execute(
            &format!(
                "UPDATE activities SET synced = 2, error = ?2, stripped = 1, capture_bytes = 0, \
                 payload = {STRIP_CAPTURES} WHERE id = ?1"
            ),
            rusqlite::params![id, reason],
        )?;
        Ok(())
    }

    /// Remove the images from one pending row, keeping its time data — what a
    /// 413 (the request was too large) calls for before resending it. Returns
    /// whether there was anything to strip: `false` for a row without images
    /// (or no longer queued), which resending cannot make any smaller.
    pub fn strip_captures(&self, id: i64) -> Result<bool> {
        let changed = self.conn.execute(
            &format!(
                "UPDATE activities SET payload = {STRIP_CAPTURES}, stripped = 1, capture_bytes = 0 \
                 WHERE id = ?1 AND synced = 0 AND capture_bytes > 0"
            ),
            [id],
        )?;
        Ok(changed > 0)
    }

    /// How many of `owner`'s payloads the server permanently rejected (surfaced
    /// in the UI so dropped data is never invisible).
    pub fn rejected_count(&self, owner: &str) -> Result<u32> {
        let count: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM activities WHERE synced = 2 AND owner = ?1",
            [owner],
            |row| row.get(0),
        )?;
        Ok(count.max(0) as u32)
    }

}

/// The bytes of base64 image data a row carries.
fn capture_bytes(activity: &Activity) -> i64 {
    [
        &activity.screenshot_png_base64,
        &activity.screenshot_thumb_jpg_base64,
        &activity.webcam_jpg_base64,
    ]
    .into_iter()
    .flatten()
    .map(|image| image.len() as i64)
    .sum()
}

#[cfg(test)]
mod tests;
