use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::Result;
use chrono::{DateTime, Utc};
use rusqlite::{Connection, OptionalExtension};

use crate::model::Activity;
use crate::signing;

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

/// The key the previous build tagged `owner_key`'s user with (no server).
fn user_only_key(owner_key: &str) -> Option<String> {
    let user = owner_key.strip_prefix("acct\u{1f}")?.rsplit_once('\u{1f}')?.1;
    Some(format!("user\u{1f}{user}"))
}

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

/// The bytes of a row's images, computed from its JSON (for rows queued
/// before the size was recorded at enqueue time).
const CAPTURE_BYTES_OF_PAYLOAD: &str = "CASE WHEN json_valid(payload) THEN \
     COALESCE(length(json_extract(payload, '$.screenshotPngBase64')), 0) \
     + COALESCE(length(json_extract(payload, '$.screenshotThumbJpgBase64')), 0) \
     + COALESCE(length(json_extract(payload, '$.webcamJpgBase64')), 0) \
     ELSE 0 END";

/// Live data up to which a queue created before incremental auto-vacuum is
/// switched to it at once. The switch is a one-off VACUUM, which rewrites all
/// live data (and needs as much free disk); a bigger backlog is switched by
/// [`Storage::reclaim_space`] once a drain has shrunk it, so it never stalls
/// startup.
const VACUUM_SWITCH_MAX_LIVE_BYTES: i64 = 64 * 1024 * 1024;

/// The database is damaged (corrupt pages, or not a database at all).
#[derive(Debug)]
struct DamagedDatabase(String);

impl std::fmt::Display for DamagedDatabase {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "the local queue is damaged: {}", self.0)
    }
}

impl std::error::Error for DamagedDatabase {}

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

    /// Open (and migrate) the queue.
    ///
    /// `legacy_owner` is the [`owner_key`] of the account signed in when this
    /// build opens the queue. Rows queued by older builds were captured by
    /// that account, so they are assigned to it (and so to its server): rows
    /// never tagged with an owner (once, when the column is added), and rows
    /// tagged with its user alone, before owners named their server. Without
    /// a saved account they keep their old tags, are never uploaded, and are
    /// purged by [`Storage::purge_foreign_pending_older_than`].
    pub fn open(path: &str, legacy_owner: Option<&str>) -> Result<Self> {
        Self::open_with(path, legacy_owner, VACUUM_SWITCH_MAX_LIVE_BYTES)
    }

    /// [`Storage::open`], switching an older queue to incremental vacuum now
    /// only if it holds at most `vacuum_switch_max_live_bytes` of live data.
    fn open_with(path: &str, legacy_owner: Option<&str>, vacuum_switch_max_live_bytes: i64) -> Result<Self> {
        let conn = Connection::open(path)?;
        // Another connection (or antivirus) briefly holding the file must not
        // fail a write outright.
        conn.busy_timeout(Duration::from_secs(5))?;
        // Before any table exists, so a new queue is created in this mode.
        switch_to_incremental_vacuum(&conn, vacuum_switch_max_live_bytes)?;
        conn.execute_batch(
            r#"
            CREATE TABLE IF NOT EXISTS activities (
                id            INTEGER PRIMARY KEY AUTOINCREMENT,
                payload       TEXT NOT NULL,
                synced        INTEGER NOT NULL DEFAULT 0,
                error         TEXT NULL,
                created_at    TEXT NOT NULL DEFAULT (datetime('now')),
                owner         TEXT NULL,
                stripped      INTEGER NOT NULL DEFAULT 0,
                capture_bytes INTEGER NOT NULL DEFAULT 0,
                project_id    TEXT NULL,
                started_at    TEXT NULL,
                ended_at      TEXT NULL
            );
            "#,
        )?;

        // Older databases predate some columns; add them if missing. One
        // transaction, so a crash can never leave the owner column added but the
        // legacy rows unclaimed-and-claimable by a later, different account.
        let tx = conn.unchecked_transaction()?;
        add_column_if_missing(&tx, "error", "TEXT NULL")?;
        if add_column_if_missing(&tx, "owner", "TEXT NULL")? {
            if let Some(owner) = legacy_owner {
                let claimed = tx.execute(
                    "UPDATE activities SET owner = ?1 WHERE owner IS NULL",
                    [owner],
                )?;
                if claimed > 0 {
                    tracing::info!(claimed, "assigned queued activities to the saved account");
                }
            }
        }
        add_column_if_missing(&tx, "stripped", "INTEGER NOT NULL DEFAULT 0")?;
        if add_column_if_missing(&tx, "capture_bytes", "INTEGER NOT NULL DEFAULT 0")? {
            tx.execute(
                &format!("UPDATE activities SET capture_bytes = {CAPTURE_BYTES_OF_PAYLOAD} WHERE stripped = 0"),
                [],
            )?;
        }
        let summary_columns_added = add_column_if_missing(&tx, "project_id", "TEXT NULL")?;
        add_column_if_missing(&tx, "started_at", "TEXT NULL")?;
        add_column_if_missing(&tx, "ended_at", "TEXT NULL")?;
        add_column_if_missing(&tx, "sig", "TEXT NULL")?;
        add_column_if_missing(&tx, "workspace_id", "TEXT NULL")?;
        add_column_if_missing(&tx, "attempts", "INTEGER NOT NULL DEFAULT 0")?;
        add_column_if_missing(&tx, "retry_at", "INTEGER NULL")?;
        add_column_if_missing(&tx, "held", "TEXT NULL")?;
        // Owners once named the user alone: those rows were recorded for the
        // saved sign-in's server.
        if let Some((owner, user_only)) = legacy_owner.and_then(|owner| Some((owner, user_only_key(owner)?))) {
            let claimed = tx.execute("UPDATE activities SET owner = ?1 WHERE owner = ?2", [owner, user_only.as_str()])?;
            if claimed > 0 {
                tracing::info!(claimed, "assigned queued activities to the saved account's server");
            }
        }
        if summary_columns_added {
            tx.execute(
                "UPDATE activities SET \
                     project_id = json_extract(payload, '$.projectId'), \
                     started_at = json_extract(payload, '$.startedAt'), \
                     ended_at = json_extract(payload, '$.endedAt') \
                 WHERE json_valid(payload)",
                [],
            )?;
        }
        tx.execute_batch(
            "CREATE INDEX IF NOT EXISTS ix_activities_owner_synced \
             ON activities (owner, synced, id);",
        )?;
        tx.commit()?;

        Ok(Self { conn, signing_key: None })
    }

    /// Open the queue at `path`, checking that it is intact. A damaged file
    /// (corrupt pages, or not a database at all) is renamed aside as
    /// `<name>.corrupt-<UTC time>` together with its journal, and a fresh,
    /// empty queue takes its place, so the agent keeps tracking. Other
    /// failures (locked, no permission, disk full) are returned: renaming the
    /// file would not help.
    pub fn open_or_recover(path: &Path, legacy_owner: Option<&str>) -> Result<Opened> {
        match Self::open_checked(path, legacy_owner) {
            Ok(storage) => Ok(Opened { storage, moved_aside: None }),
            Err(e) if is_damaged(&e) => {
                let aside = set_aside(path, Utc::now())?;
                tracing::error!(error = %e, aside = %aside.display(), "the local queue was damaged; moved it aside and started a new one");
                let storage = Self::open_checked(path, legacy_owner)?;
                Ok(Opened { storage, moved_aside: Some(aside) })
            }
            Err(e) => Err(e),
        }
    }

    fn open_checked(path: &Path, legacy_owner: Option<&str>) -> Result<Self> {
        let storage = Self::open(&path.to_string_lossy(), legacy_owner)?;
        let verdict: String = storage.conn.query_row("PRAGMA quick_check", [], |row| row.get(0))?;
        if verdict != "ok" {
            return Err(DamagedDatabase(verdict).into());
        }
        Ok(storage)
    }

    /// Queue an activity for upload on behalf of `owner`, in `workspace_id`.
    pub fn enqueue(&self, activity: &Activity, owner: &str, workspace_id: &str) -> Result<i64> {
        // Sign the payload before writing it so the server can detect tampering.
        // The signature is computed over the canonical JSON (without the
        // `payloadSignature` field itself, which is always None at this point).
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
                    // Propagate the stored signature into the Activity so the
                    // upload includes it for server-side verification.
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

    fn reclaim_space_with(&self, vacuum_switch_max_live_bytes: i64) -> Result<()> {
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

/// Make sure the database uses incremental auto-vacuum, so deleted rows'
/// pages can be returned to the disk; returns whether it does now. A new
/// database takes the mode as it is created. An existing one needs a one-off
/// rebuild (VACUUM, which also reclaims what it held), done only while its
/// live data is at most `max_live_bytes`.
fn switch_to_incremental_vacuum(conn: &Connection, max_live_bytes: i64) -> Result<bool> {
    const INCREMENTAL: i64 = 2;
    let pragma = |name: &str| conn.query_row(&format!("PRAGMA {name}"), [], |row| row.get::<_, i64>(0));
    if pragma("auto_vacuum")? == INCREMENTAL {
        return Ok(true);
    }
    conn.execute_batch("PRAGMA auto_vacuum = INCREMENTAL;")?;
    let pages = pragma("page_count")?;
    if pages == 0 {
        return Ok(true);
    }
    let live_bytes = (pages - pragma("freelist_count")?) * pragma("page_size")?;
    if live_bytes > max_live_bytes {
        tracing::info!(live_bytes, "local queue switches to incremental vacuum once it is drained");
        return Ok(false);
    }
    let started = std::time::Instant::now();
    match conn.execute_batch("VACUUM;") {
        Ok(()) => {
            tracing::info!(elapsed = ?started.elapsed(), live_bytes, "local queue rebuilt for incremental vacuum");
            Ok(pragma("auto_vacuum")? == INCREMENTAL)
        }
        // Needs free disk space about the live data's size; it is tried
        // again later, and the queue works either way.
        Err(e) => {
            tracing::warn!(?e, "could not switch the local queue to incremental vacuum");
            Ok(false)
        }
    }
}

/// Is this error a damaged database (as opposed to a locked or unreachable one)?
fn is_damaged(error: &anyhow::Error) -> bool {
    error.chain().any(|cause| {
        cause.is::<DamagedDatabase>()
            || matches!(
                cause.downcast_ref::<rusqlite::Error>().and_then(rusqlite::Error::sqlite_error_code),
                Some(rusqlite::ErrorCode::DatabaseCorrupt | rusqlite::ErrorCode::NotADatabase)
            )
    })
}

/// Rename a damaged database to `<name>.corrupt-<UTC time>`, its sidecar
/// files with it (as `<name>.corrupt-<time>-journal`, so SQLite still pairs
/// them). A sidecar that cannot be moved is deleted: a stale journal next to
/// the fresh database would be rolled back into it.
fn set_aside(path: &Path, now: DateTime<Utc>) -> Result<PathBuf> {
    let aside = with_suffix(path, &format!(".corrupt-{}", now.format("%Y%m%dT%H%M%SZ")));
    std::fs::rename(path, &aside)?;
    for suffix in SQLITE_SIDECARS {
        let sidecar = with_suffix(path, suffix);
        if sidecar.exists() && std::fs::rename(&sidecar, with_suffix(&aside, suffix)).is_err() {
            std::fs::remove_file(&sidecar)?;
        }
    }
    Ok(aside)
}

fn with_suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut name = path.as_os_str().to_os_string();
    name.push(suffix);
    PathBuf::from(name)
}

/// Idempotent `ALTER TABLE activities ADD COLUMN`. Returns whether the column
/// was added (i.e. this database predates it).
fn add_column_if_missing(conn: &Connection, column: &str, decl: &str) -> Result<bool> {
    let exists = conn
        .query_row(
            "SELECT 1 FROM pragma_table_info('activities') WHERE name = ?1",
            [column],
            |_| Ok(()),
        )
        .optional()?
        .is_some();
    if exists {
        return Ok(false);
    }
    conn.execute_batch(&format!("ALTER TABLE activities ADD COLUMN {column} {decl};"))?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALICE: &str = "acct\u{1f}https://tracker.acme.com\u{1f}alice@acme.com";
    const BOB: &str = "acct\u{1f}https://tracker.acme.com\u{1f}bob@acme.com";
    /// The same email on another organisation's server.
    const ALICE_ELSEWHERE: &str = "acct\u{1f}https://tracker.globex.com\u{1f}alice@acme.com";
    const WS: &str = "w1";

    fn sample(project: &str) -> Activity {
        let now = Utc::now();
        Activity {
            productivity: Some(40),
            mouse_clicks: Some(1),
            keyboard_hits: Some(2),
            ..Activity::new(project, now, now)
        }
    }

    fn with_captures(project: &str) -> Activity {
        Activity {
            screenshot_png_base64: Some("c2NyZWVu".into()),
            screenshot_thumb_jpg_base64: Some("dGh1bWI=".into()),
            webcam_jpg_base64: Some("d2ViY2Ft".into()),
            screenshot_captured_at: Some(Utc::now()),
            screenshot_blurred: Some(false),
            ..sample(project)
        }
    }

    /// A row whose images weigh `bytes` in total.
    fn with_image_bytes(project: &str, bytes: usize) -> Activity {
        Activity { screenshot_png_base64: Some("A".repeat(bytes)), ..sample(project) }
    }

    fn store() -> Storage {
        Storage::open(":memory:", None).expect("in-memory sqlite")
    }

    /// Everything pending for `owner`, drained through the batch API.
    fn pending(s: &Storage, owner: &str) -> Vec<(i64, Activity)> {
        s.pending_batch(owner, 0, u32::MAX).unwrap()
    }

    fn backdate(s: &Storage, id: i64, days: u32) {
        s.conn
            .execute(
                "UPDATE activities SET created_at = datetime('now', ?2) WHERE id = ?1",
                rusqlite::params![id, format!("-{days} days")],
            )
            .unwrap();
    }

    fn raw_payload(s: &Storage, id: i64) -> Activity {
        let payload: String = s
            .conn
            .query_row("SELECT payload FROM activities WHERE id = ?1", [id], |r| r.get(0))
            .unwrap();
        serde_json::from_str(&payload).unwrap()
    }

    fn temp_db_path() -> std::path::PathBuf {
        std::env::temp_dir().join(format!("dosi-tracker-test-{}.db", uuid::Uuid::new_v4()))
    }

    #[test]
    fn owner_key_is_case_and_whitespace_insensitive() {
        assert_eq!(owner_key("https://Tracker.Acme.com/", " Alice@Acme.com "), ALICE);
        assert_ne!(owner_key("https://tracker.acme.com", "alice@acme.com"), owner_key("https://tracker.acme.com", "bob@acme.com"));
        // Keys of older builds (user only, or workspace + user) never match.
        assert_ne!(owner_key("https://tracker.acme.com", "alice@acme.com"), "user\u{1f}alice@acme.com");
        assert_ne!(owner_key("https://tracker.acme.com", "alice@acme.com"), "acme\u{1f}alice@acme.com");
        assert_eq!(user_only_key(ALICE).as_deref(), Some("user\u{1f}alice@acme.com"));
        assert_eq!(user_only_key("user\u{1f}alice@acme.com"), None);
    }

    #[test]
    fn the_same_email_on_another_server_is_another_account() {
        let s = store();
        s.enqueue(&with_captures("acme-work"), ALICE, WS).unwrap();
        assert_eq!(owner_key("https://tracker.globex.com", "alice@acme.com"), ALICE_ELSEWHERE);
        // Signed in with the same email on another organisation's server:
        // nothing recorded for the first one is ever sent there.
        assert!(pending(&s, ALICE_ELSEWHERE).is_empty());
        assert_eq!(s.due_count(ALICE_ELSEWHERE).unwrap(), 0);
        assert_eq!(s.pending_count(ALICE).unwrap(), 1);
        // ...and it expires there like any other account's rows.
        let id = pending(&s, ALICE)[0].0;
        backdate(&s, id, 20);
        assert_eq!(s.purge_foreign_pending_older_than(Some(ALICE_ELSEWHERE), 14).unwrap(), 1);
    }

    #[test]
    fn rows_tagged_with_the_user_alone_move_to_the_saved_accounts_server() {
        let path = temp_db_path();
        let path_str = path.to_string_lossy().into_owned();
        {
            let s = Storage::open(&path_str, None).unwrap();
            // As the previous build tagged them: the user, no server.
            for (owner, project) in [("user\u{1f}alice@acme.com", "alice"), ("user\u{1f}bob@acme.com", "bob")] {
                s.enqueue(&sample(project), owner, WS).unwrap();
            }
        }
        {
            let s = Storage::open(&path_str, Some(ALICE)).unwrap();
            let alice: Vec<String> = pending(&s, ALICE).into_iter().map(|(_, a)| a.project_id).collect();
            assert_eq!(alice, ["alice"], "the saved sign-in's rows, now on its server");
            assert_eq!(owned(&s, "user\u{1f}bob@acme.com"), 1, "another user's rows keep their old tag (never uploaded)");
        }
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn a_held_workspace_waits_without_holding_up_the_others() {
        let s = store();
        let w1 = s.enqueue(&sample("p1"), ALICE, "w1").unwrap();
        let w2 = s.enqueue(&sample("p2"), ALICE, "w2").unwrap();
        s.enqueue(&sample("bob"), BOB, "w1").unwrap();

        assert_eq!(s.hold_workspace(ALICE, "w1", "No access to Acme.").unwrap(), 1, "only Alice's rows");
        let due: Vec<i64> = pending(&s, ALICE).into_iter().map(|(id, _)| id).collect();
        assert_eq!(due, [w2]);
        assert_eq!(s.due_count(ALICE).unwrap(), 1);
        assert_eq!(s.pending_count(ALICE).unwrap(), 2, "still shown as waiting");
        assert_eq!(s.next_retry_at(ALICE).unwrap(), None, "no timer: it waits for the workspace");
        assert_eq!(s.recent_rows(ALICE, 10).unwrap().iter().find(|r| r.id == w1).unwrap().reason.as_deref(), Some("No access to Acme."));
        assert_eq!(s.due_count(BOB).unwrap(), 1);

        assert_eq!(s.release_workspace(ALICE, "w1").unwrap(), 1);
        assert_eq!(s.due_count(ALICE).unwrap(), 2);
        assert_eq!(s.release_workspace(ALICE, "w1").unwrap(), 0, "nothing left to release");

        // Held for longer than the retention: purged with the parked rows.
        s.hold_workspace(ALICE, "w1", "archived").unwrap();
        backdate(&s, w1, 20);
        backdate(&s, w2, 20);
        assert_eq!(s.purge_rejected_older_than(14).unwrap(), 1, "the held row only");
        assert_eq!(s.pending_count(ALICE).unwrap(), 1);
    }

    #[test]
    fn a_deferred_row_waits_for_its_time_and_counts_its_attempts() {
        let s = store();
        let first = s.enqueue(&sample("p1"), ALICE, WS).unwrap();
        let second = s.enqueue(&sample("p2"), ALICE, WS).unwrap();
        let later = Utc::now() + chrono::Duration::minutes(10);

        assert_eq!(s.defer(first, "HTTP 422 Tracker:New: ?", later).unwrap(), 1);
        let due = s.pending_rows(ALICE, 0, 10).unwrap();
        assert_eq!(due.iter().map(|r| r.id).collect::<Vec<_>>(), [second], "the next row goes on");
        assert_eq!(s.due_count(ALICE).unwrap(), 1);
        assert_eq!(s.next_retry_at(ALICE).unwrap().map(|t| t.timestamp()), Some(later.timestamp()));

        // Its time has come: due again, with the attempts so far.
        assert_eq!(s.defer(first, "again", Utc::now() - chrono::Duration::seconds(1)).unwrap(), 2);
        let row = s.pending_rows(ALICE, 0, 10).unwrap().into_iter().find(|r| r.id == first).unwrap();
        assert_eq!(row.attempts, 2);
        assert_eq!(s.recent_rows(ALICE, 10).unwrap().iter().find(|r| r.id == first).unwrap().reason.as_deref(), Some("again"));
        assert_eq!(s.defer(999, "gone", later).unwrap(), 0, "an unknown row");
    }

    #[test]
    fn each_row_keeps_its_workspace_and_rows_without_one_never_upload() {
        let s = store();
        s.enqueue(&sample("a"), ALICE, "w1").unwrap();
        s.enqueue(&sample("b"), ALICE, "w2").unwrap();
        s.conn
            .execute("INSERT INTO activities (payload, synced, owner) VALUES (?1, 0, ?2)", [serde_json::to_string(&sample("old")).unwrap(), ALICE.to_owned()])
            .unwrap();
        let rows = s.pending_rows(ALICE, 0, 10).unwrap();
        let seen: Vec<(&str, &str)> = rows.iter().map(|r| (r.activity.project_id.as_str(), r.workspace_id.as_str())).collect();
        assert_eq!(seen, [("a", "w1"), ("b", "w2")]);
        assert_eq!(s.pending_count(ALICE).unwrap(), 2, "a row without a workspace is not waiting to upload");
    }

    #[test]
    fn enqueue_then_pending_roundtrips() {
        let s = store();
        s.enqueue(&sample("p1"), ALICE, WS).unwrap();
        let pending = pending(&s, ALICE);
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].1.project_id, "p1");
        assert_eq!(pending[0].1.productivity, Some(40));
    }

    #[test]
    fn only_the_signed_in_accounts_rows_are_uploadable() {
        let s = store();
        s.enqueue(&sample("alice-1"), ALICE, WS).unwrap();
        s.enqueue(&sample("bob-1"), BOB, WS).unwrap();
        s.enqueue(&sample("alice-2"), ALICE, WS).unwrap();

        let alice: Vec<String> = pending(&s, ALICE).into_iter().map(|(_, a)| a.project_id).collect();
        assert_eq!(alice, ["alice-1", "alice-2"]);
        let bob: Vec<String> = pending(&s, BOB).into_iter().map(|(_, a)| a.project_id).collect();
        assert_eq!(bob, ["bob-1"]);
        assert_eq!(s.pending_count(ALICE).unwrap(), 2);
        assert_eq!(s.pending_count(BOB).unwrap(), 1);
        assert!(pending(&s, "user\u{1f}mallory@acme.com").is_empty());
    }

    #[test]
    fn batches_page_through_the_queue_oldest_first() {
        let s = store();
        for i in 0..5 {
            s.enqueue(&sample(&format!("p{i}")), ALICE, WS).unwrap();
        }
        s.enqueue(&sample("bob"), BOB, WS).unwrap();

        let mut seen = Vec::new();
        let mut cursor = 0;
        loop {
            let batch = s.pending_batch(ALICE, cursor, 2).unwrap();
            if batch.is_empty() {
                break;
            }
            assert!(batch.len() <= 2);
            cursor = batch.last().unwrap().0;
            seen.extend(batch.into_iter().map(|(_, a)| a.project_id));
        }
        assert_eq!(seen, ["p0", "p1", "p2", "p3", "p4"]);
    }

    #[test]
    fn unreadable_payloads_are_parked_not_skipped_forever() {
        let s = store();
        s.conn
            .execute(
                "INSERT INTO activities (payload, synced, owner, workspace_id) VALUES ('{not json', 0, ?1, 'w1')",
                [ALICE],
            )
            .unwrap();
        s.enqueue(&sample("ok"), ALICE, WS).unwrap();

        let batch = pending(&s, ALICE);
        assert_eq!(batch.len(), 1);
        assert_eq!(batch[0].1.project_id, "ok");
        assert_eq!(s.pending_count(ALICE).unwrap(), 1);
        assert_eq!(s.rejected_count(ALICE).unwrap(), 1);
    }

    #[test]
    fn synced_rows_are_removed_from_the_queue() {
        let s = store();
        let id = s.enqueue(&sample("p1"), ALICE, WS).unwrap();
        s.remove_synced(id).unwrap();
        assert!(pending(&s, ALICE).is_empty());
    }

    #[test]
    fn rejected_rows_are_parked_not_retried() {
        let s = store();
        let id = s.enqueue(&with_captures("p1"), ALICE, WS).unwrap();
        s.mark_rejected(id, "HTTP 400: bad project").unwrap();

        // Parked rows must never come back as pending — that is what used to
        // block the whole queue behind one poison payload.
        assert!(pending(&s, ALICE).is_empty());
        assert_eq!(s.rejected_count(ALICE).unwrap(), 1);
        assert_eq!(s.rejected_count(BOB).unwrap(), 0);

        // The images are dropped; the time data stays for diagnostics.
        let parked = raw_payload(&s, id);
        assert!(!parked.has_captures());
        assert!(parked.screenshot_captured_at.is_none() && parked.screenshot_blurred.is_none());
        assert_eq!(parked.project_id, "p1");
    }

    #[test]
    fn recent_rows_list_both_states_with_the_server_reason() {
        let s = store();
        let first = sample("p1");
        let a = s.enqueue(&first, ALICE, WS).unwrap();
        let b = s.enqueue(&with_captures("p2"), ALICE, WS).unwrap();
        let c = s.enqueue(&sample("p3"), ALICE, WS).unwrap();
        s.enqueue(&sample("bob"), BOB, WS).unwrap();
        s.mark_rejected(b, "Overlaps another block.").unwrap();
        s.note_attempt(c, Some("Your computer clock is ahead")).unwrap();

        let rows = s.recent_rows(ALICE, 10).unwrap();
        assert_eq!(rows.iter().map(|r| r.id).collect::<Vec<_>>(), [c, b, a], "newest first, Alice's only");
        assert_eq!(rows[0].status, RowStatus::Queued);
        assert_eq!(rows[0].reason.as_deref(), Some("Your computer clock is ahead"));
        assert_eq!(rows[1].status, RowStatus::Rejected);
        assert_eq!(rows[1].reason.as_deref(), Some("Overlaps another block."));
        assert!(!rows[1].has_captures, "parking drops the images");
        assert_eq!(rows[2].project_id.as_deref(), Some("p1"));
        // Millisecond precision is what the column keeps.
        let ms = |t: DateTime<Utc>| t.timestamp_millis();
        assert_eq!(rows[2].started_at.map(ms), Some(ms(first.started_at)));
        assert_eq!(rows[2].ended_at.map(ms), Some(ms(first.ended_at)));

        assert_eq!(s.recent_rows(ALICE, 1).unwrap().len(), 1);
        // Clearing the note; a parked row's reason cannot be overwritten by it.
        s.note_attempt(c, None).unwrap();
        s.note_attempt(b, Some("ignored")).unwrap();
        let rows = s.recent_rows(ALICE, 10).unwrap();
        assert_eq!(rows[0].reason, None);
        assert_eq!(rows[1].reason.as_deref(), Some("Overlaps another block."));
    }

    #[test]
    fn stripping_one_row_keeps_its_time_data_and_place_in_the_queue() {
        let s = store();
        let id = s.enqueue(&with_captures("p1"), ALICE, WS).unwrap();
        let other = s.enqueue(&with_captures("p2"), ALICE, WS).unwrap();
        assert!(s.strip_captures(id).unwrap());

        let row = raw_payload(&s, id);
        assert!(!row.has_captures());
        assert_eq!((row.project_id.as_str(), row.productivity, row.keyboard_hits), ("p1", Some(40), Some(2)));
        assert!(raw_payload(&s, other).has_captures(), "only that row");
        assert_eq!(pending(&s, ALICE).len(), 2, "still queued, to be resent");
        assert!(!s.recent_rows(ALICE, 10).unwrap().iter().find(|r| r.id == id).unwrap().has_captures);

        // Nothing left to strip: resending could not make it smaller.
        assert!(!s.strip_captures(id).unwrap());
        let bare = s.enqueue(&sample("p3"), ALICE, WS).unwrap();
        assert!(!s.strip_captures(bare).unwrap());
        // A parked row is not pending: nothing to strip.
        s.mark_rejected(other, "nope").unwrap();
        assert!(!s.strip_captures(other).unwrap());
    }

    #[test]
    fn purge_keeps_recent_rejects_and_drops_old_ones() {
        let s = store();
        let id = s.enqueue(&sample("p1"), ALICE, WS).unwrap();
        s.mark_rejected(id, "nope").unwrap();

        // Just-parked rows survive a 30-day purge.
        assert_eq!(s.purge_rejected_older_than(30).unwrap(), 0);
        assert_eq!(s.rejected_count(ALICE).unwrap(), 1);

        // Back-date it and it is collected.
        backdate(&s, id, 60);
        assert_eq!(s.purge_rejected_older_than(30).unwrap(), 1);
        assert_eq!(s.rejected_count(ALICE).unwrap(), 0);
    }

    #[test]
    fn another_accounts_rows_wait_then_expire() {
        let s = store();
        let bob_recent = s.enqueue(&sample("bob-recent"), BOB, WS).unwrap();
        let bob_old = s.enqueue(&sample("bob-old"), BOB, WS).unwrap();
        let alice_old = s.enqueue(&sample("alice-old"), ALICE, WS).unwrap();
        backdate(&s, bob_old, 20);
        backdate(&s, alice_old, 20);

        // Alice signed in: Bob's old row goes, his recent one and all of hers stay.
        assert_eq!(s.purge_foreign_pending_older_than(Some(ALICE), 14).unwrap(), 1);
        assert_eq!(s.pending_count(ALICE).unwrap(), 1);
        let bob: Vec<i64> = pending(&s, BOB).into_iter().map(|(id, _)| id).collect();
        assert_eq!(bob, [bob_recent]);

        // Nobody signed in: every old pending row expires.
        assert_eq!(s.purge_foreign_pending_older_than(None, 14).unwrap(), 1);
        assert_eq!(s.pending_count(ALICE).unwrap(), 0);
        assert_eq!(s.pending_count(BOB).unwrap(), 1);
    }

    /// Create a database with the schema shipped before owners existed.
    fn legacy_db_with_rows(path: &std::path::Path, rows: usize) {
        legacy_db_with(path, &vec![with_captures("legacy"); rows]);
    }

    /// A database with the schema shipped before owners existed (and before
    /// incremental vacuum), holding `rows`.
    fn legacy_db_with(path: &std::path::Path, rows: &[Activity]) {
        let conn = Connection::open(path).unwrap();
        conn.execute_batch(
            "CREATE TABLE activities (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                payload TEXT NOT NULL,
                synced INTEGER NOT NULL DEFAULT 0,
                error TEXT NULL,
                created_at TEXT NOT NULL DEFAULT (datetime('now')));",
        )
        .unwrap();
        for row in rows {
            conn.execute(
                "INSERT INTO activities (payload, synced) VALUES (?1, 0)",
                [serde_json::to_string(row).unwrap()],
            )
            .unwrap();
        }
    }

    fn owned(s: &Storage, owner: &str) -> i64 {
        s.conn.query_row("SELECT COUNT(*) FROM activities WHERE owner = ?1", [owner], |r| r.get(0)).unwrap()
    }

    fn auto_vacuum_mode(s: &Storage) -> i64 {
        s.conn.query_row("PRAGMA auto_vacuum", [], |r| r.get(0)).unwrap()
    }

    #[test]
    fn migration_assigns_legacy_rows_to_the_saved_account_once() {
        let path = temp_db_path();
        legacy_db_with_rows(&path, 2);
        let path_str = path.to_string_lossy().into_owned();

        {
            let s = Storage::open(&path_str, Some(ALICE)).unwrap();
            assert_eq!(owned(&s, ALICE), 2);
        }
        {
            // Re-opening (even as someone else) is a no-op: the migration ran once.
            let s = Storage::open(&path_str, Some(BOB)).unwrap();
            assert_eq!(owned(&s, ALICE), 2);
            assert_eq!(owned(&s, BOB), 0);
        }
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn migration_fills_the_new_columns_and_switches_to_incremental_vacuum() {
        let path = temp_db_path();
        legacy_db_with_rows(&path, 1);
        let s = Storage::open(&path.to_string_lossy(), Some(ALICE)).unwrap();

        let rows = s.recent_rows(ALICE, 5).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].project_id.as_deref(), Some("legacy"));
        assert!(rows[0].started_at.is_some() && rows[0].ended_at.is_some());
        assert!(rows[0].has_captures, "the legacy row's images were measured");
        let bytes: i64 = s.conn.query_row("SELECT capture_bytes FROM activities", [], |r| r.get(0)).unwrap();
        assert_eq!(bytes as usize, "c2NyZWVu".len() + "dGh1bWI=".len() + "d2ViY2Ft".len());

        assert_eq!(auto_vacuum_mode(&s), 2, "INCREMENTAL");
        drop(s);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn a_large_older_queue_switches_to_incremental_vacuum_once_drained() {
        let path = temp_db_path();
        let rows: Vec<Activity> = (0..24).map(|i| with_image_bytes(&format!("p{i}"), 64 * 1024)).collect();
        legacy_db_with(&path, &rows);
        let limit = 512 * 1024; // the backlog (~1.5 MB) is over it
        let s = Storage::open_with(&path.to_string_lossy(), Some(ALICE), limit).unwrap();
        s.conn.execute("UPDATE activities SET workspace_id = 'w1'", []).unwrap();
        assert_eq!(auto_vacuum_mode(&s), 0, "not rebuilt while the backlog is large");
        let ids: Vec<i64> = pending(&s, ALICE).into_iter().map(|(id, _)| id).collect();
        assert_eq!(ids.len(), 24, "the queue works meanwhile");

        // Half uploaded: still too much live data.
        for &id in &ids[..12] {
            s.remove_synced(id).unwrap();
        }
        s.reclaim_space_with(limit).unwrap();
        assert_eq!(auto_vacuum_mode(&s), 0);

        // Drained: switched, and the file gives the space back.
        let size = || std::fs::metadata(&path).unwrap().len();
        let before = size();
        for &id in &ids[12..] {
            s.remove_synced(id).unwrap();
        }
        s.reclaim_space_with(limit).unwrap();
        assert_eq!(auto_vacuum_mode(&s), 2);
        assert!(size() * 4 < before, "file shrank from {before} to {}", size());
        drop(s);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn legacy_rows_without_a_saved_account_are_never_uploaded() {
        let path = temp_db_path();
        legacy_db_with_rows(&path, 1);
        let path_str = path.to_string_lossy().into_owned();

        {
            let s = Storage::open(&path_str, None).unwrap();
            // A later sign-in must not inherit them...
            let s2 = Storage::open(&path_str, Some(BOB)).unwrap();
            assert!(pending(&s2, BOB).is_empty());
            // ...and they expire like any other foreign row.
            s.conn
                .execute("UPDATE activities SET created_at = datetime('now', '-20 days')", [])
                .unwrap();
            assert_eq!(s.purge_foreign_pending_older_than(Some(BOB), 14).unwrap(), 1);
        }
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn limits_shed_images_first_and_keep_time_data() {
        let s = store();
        let ids: Vec<i64> = (0..4)
            .map(|i| s.enqueue(&with_captures(&format!("p{i}")), ALICE, WS).unwrap())
            .collect();
        backdate(&s, ids[3], 40); // newest row, but too old to keep images

        let limits = QueueLimits {
            capture_max_rows: 2,
            capture_max_bytes: u64::MAX,
            capture_max_age_days: 30,
            max_pending_rows: 100,
        };
        let report = s.enforce_limits(&limits).unwrap();
        assert_eq!(report, LimitReport { stripped: 3, dropped: 0 });

        // ids[0], ids[1]: beyond the newest two. ids[3]: too old. ids[2] keeps its images.
        for (i, &id) in ids.iter().enumerate() {
            let row = raw_payload(&s, id);
            assert_eq!(row.screenshot_png_base64.is_some(), i == 2, "row {i}");
            assert_eq!(row.screenshot_thumb_jpg_base64.is_some(), i == 2, "row {i}");
            assert_eq!(row.webcam_jpg_base64.is_some(), i == 2, "row {i}");
            assert_eq!(row.project_id, format!("p{i}"));
            assert_eq!(row.productivity, Some(40));
        }
        assert_eq!(s.pending_count(ALICE).unwrap(), 4);

        // Idempotent: already-stripped rows are not rewritten again.
        assert_eq!(s.enforce_limits(&limits).unwrap(), LimitReport::default());
    }

    #[test]
    fn images_are_kept_newest_first_within_the_byte_budget() {
        let s = store();
        // Oldest to newest: 400 + 300 + 500 + 200 bytes of images, one row without.
        let ids: Vec<i64> = [400, 300, 500, 0, 200]
            .into_iter()
            .enumerate()
            .map(|(i, bytes)| {
                let row = if bytes == 0 { sample(&format!("p{i}")) } else { with_image_bytes(&format!("p{i}"), bytes) };
                s.enqueue(&row, BOB, WS).unwrap()
            })
            .collect();

        let limits = QueueLimits {
            capture_max_rows: 100,
            capture_max_bytes: 1_000,
            capture_max_age_days: 30,
            max_pending_rows: 100,
        };
        // Newest first: 200 (200), 0 (200), 500 (700), 300 (1000), 400 (1400 > 1000).
        assert_eq!(s.enforce_limits(&limits).unwrap(), LimitReport { stripped: 1, dropped: 0 });
        assert!(!raw_payload(&s, ids[0]).has_captures(), "the oldest goes over the budget");
        for &id in &ids[1..] {
            assert_eq!(raw_payload(&s, id).screenshot_png_base64.is_some(), id != ids[3]);
        }

        // A tighter budget sheds the next-oldest too, never a newer row first.
        let tighter = QueueLimits { capture_max_bytes: 700, ..limits };
        assert_eq!(s.enforce_limits(&tighter).unwrap().stripped, 1);
        assert!(!raw_payload(&s, ids[1]).has_captures());
        assert!(raw_payload(&s, ids[2]).has_captures() && raw_payload(&s, ids[4]).has_captures());
    }

    #[test]
    fn the_default_budget_is_300_mb_within_the_newest_500_rows() {
        assert_eq!(QueueLimits::DEFAULT.capture_max_bytes, 300 * 1024 * 1024);
        assert_eq!(QueueLimits::DEFAULT.capture_max_rows, 500);
    }

    #[test]
    fn limits_drop_the_oldest_rows_beyond_the_hard_cap() {
        let s = store();
        for i in 0..5 {
            s.enqueue(&sample(&format!("p{i}")), ALICE, WS).unwrap();
        }
        let parked = s.enqueue(&sample("parked"), ALICE, WS).unwrap();
        s.mark_rejected(parked, "nope").unwrap();

        let limits = QueueLimits {
            capture_max_rows: 10,
            capture_max_bytes: u64::MAX,
            capture_max_age_days: 30,
            max_pending_rows: 3,
        };
        assert_eq!(s.enforce_limits(&limits).unwrap().dropped, 2);

        let left: Vec<String> = pending(&s, ALICE).into_iter().map(|(_, a)| a.project_id).collect();
        assert_eq!(left, ["p2", "p3", "p4"]);
        // Parked rows follow their own retention, not the pending cap.
        assert_eq!(s.rejected_count(ALICE).unwrap(), 1);
    }

    #[test]
    fn space_freed_by_uploads_goes_back_to_the_disk() {
        let path = temp_db_path();
        let s = Storage::open(&path.to_string_lossy(), None).unwrap();
        let ids: Vec<i64> = (0..40).map(|i| s.enqueue(&with_image_bytes(&format!("p{i}"), 64 * 1024), ALICE, WS).unwrap()).collect();
        let size = || std::fs::metadata(&path).unwrap().len();
        let full = size();
        for id in ids {
            s.remove_synced(id).unwrap();
        }
        s.reclaim_space().unwrap();
        assert!(size() * 4 < full, "file shrank from {full} to {}", size());
        drop(s);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn a_damaged_queue_is_moved_aside_and_replaced() {
        let path = temp_db_path();
        // Several pages of something that is not an SQLite database, with a
        // leftover journal (SQLite discards one it cannot use when it reads
        // the file; any it leaves travels with the database).
        let damaged = b"not a database. ".repeat(1024);
        std::fs::write(&path, &damaged).unwrap();
        std::fs::write(with_suffix(&path, "-journal"), b"stale journal").unwrap();

        let opened = Storage::open_or_recover(&path, Some(ALICE)).unwrap();
        let aside = opened.moved_aside.clone().expect("moved aside");
        let name = aside.file_name().unwrap().to_string_lossy().into_owned();
        let stamp = name.strip_prefix("dosi-tracker-test-").and_then(|n| n.split_once(".db.corrupt-")).map(|(_, t)| t);
        assert!(
            stamp.is_some_and(|t| chrono::NaiveDateTime::parse_from_str(t, "%Y%m%dT%H%M%SZ").is_ok()),
            "{name}"
        );
        assert_eq!(std::fs::read(&aside).unwrap(), damaged, "kept as it was, for diagnosis");
        assert!(!with_suffix(&path, "-journal").exists(), "no stale journal next to the new queue");

        // The fresh queue works.
        opened.storage.enqueue(&sample("p1"), ALICE, WS).unwrap();
        assert_eq!(opened.storage.pending_count(ALICE).unwrap(), 1);
        drop(opened);

        // A healthy queue is opened as it is.
        let reopened = Storage::open_or_recover(&path, Some(ALICE)).unwrap();
        assert!(reopened.moved_aside.is_none());
        assert_eq!(reopened.storage.pending_count(ALICE).unwrap(), 1);
        drop(reopened);
        for file in [path.clone(), aside.clone(), with_suffix(&aside, "-journal")] {
            let _ = std::fs::remove_file(file);
        }
    }

    #[test]
    fn setting_aside_moves_the_sidecar_files_with_the_database() {
        let path = temp_db_path();
        std::fs::write(&path, b"db").unwrap();
        std::fs::write(with_suffix(&path, "-journal"), b"journal").unwrap();
        std::fs::write(with_suffix(&path, "-wal"), b"wal").unwrap();
        let at = DateTime::parse_from_rfc3339("2026-10-05T03:04:05Z").unwrap().with_timezone(&Utc);

        let aside = set_aside(&path, at).unwrap();
        assert_eq!(aside, with_suffix(&path, ".corrupt-20261005T030405Z"));
        assert_eq!(std::fs::read(&aside).unwrap(), b"db");
        // Still paired the way SQLite pairs them.
        assert_eq!(std::fs::read(with_suffix(&aside, "-journal")).unwrap(), b"journal");
        assert_eq!(std::fs::read(with_suffix(&aside, "-wal")).unwrap(), b"wal");
        for suffix in ["", "-journal", "-wal", "-shm"] {
            assert!(!with_suffix(&path, suffix).exists(), "{suffix} left behind");
        }
        for suffix in ["", "-journal", "-wal"] {
            let _ = std::fs::remove_file(with_suffix(&aside, suffix));
        }
    }

    #[test]
    fn a_queue_that_cannot_be_opened_for_other_reasons_is_left_alone() {
        // A directory where the file should be: not damage, so nothing is renamed.
        let dir = std::env::temp_dir().join(format!("dosi-tracker-test-dir-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        assert!(Storage::open_or_recover(&dir, None).is_err());
        assert!(dir.is_dir());
        let _ = std::fs::remove_dir_all(&dir);
    }

    // Property-based tests: the queue invariants must hold for ANY sequence of operations,
    // not just the hand-picked cases above.
    use proptest::prelude::*;

    proptest! {
        // The batch API returns exactly what was enqueued, in FIFO (insertion) order — the property
        // the whole offline-sync loop relies on to upload oldest-first without gaps or reordering —
        // whatever the batch size.
        #[test]
        fn pending_preserves_enqueue_count_and_fifo_order(
            projects in prop::collection::vec("[a-z][a-z0-9]{0,7}", 0..24),
            batch in 1u32..8,
        ) {
            let s = store();
            for p in &projects {
                s.enqueue(&sample(p), ALICE, WS).unwrap();
            }
            let mut order = Vec::new();
            let mut cursor = 0;
            loop {
                let rows = s.pending_batch(ALICE, cursor, batch).unwrap();
                let Some(&(last, _)) = rows.last() else { break };
                prop_assert!(rows.len() as u32 <= batch);
                cursor = last;
                order.extend(rows.into_iter().map(|(_, a)| a.project_id));
            }
            prop_assert_eq!(order, projects);
        }

        // Every row ends up either uploaded-and-removed or parked-as-rejected; none ever
        // reappears as pending, and rejected_count accounts for exactly the parked rows.
        #[test]
        fn disposed_rows_never_reappear_and_counts_are_conserved(
            keep_flags in prop::collection::vec(any::<bool>(), 0..24)
        ) {
            let s = store();
            for _ in &keep_flags {
                s.enqueue(&sample("p"), ALICE, WS).unwrap();
            }

            let ids: Vec<i64> = pending(&s, ALICE).into_iter().map(|(id, _)| id).collect();
            prop_assert_eq!(ids.len(), keep_flags.len());

            let mut rejected = 0u32;
            for (id, &synced_ok) in ids.iter().zip(&keep_flags) {
                if synced_ok {
                    s.remove_synced(*id).unwrap();
                } else {
                    s.mark_rejected(*id, "server rejected").unwrap();
                    rejected += 1;
                }
            }

            prop_assert!(pending(&s, ALICE).is_empty());
            prop_assert_eq!(s.rejected_count(ALICE).unwrap(), rejected);
        }

        // Whatever the image sizes, the newest rows keep their images and the kept total never
        // exceeds the budget.
        #[test]
        fn kept_images_never_exceed_the_byte_budget(
            sizes in prop::collection::vec(0usize..400, 1..20),
            budget in 0u64..2_000,
        ) {
            let s = store();
            let ids: Vec<i64> = sizes.iter().map(|&n| {
                let row = if n == 0 { sample("p") } else { with_image_bytes("p", n) };
                s.enqueue(&row, ALICE, WS).unwrap()
            }).collect();
            let limits = QueueLimits { capture_max_rows: 1_000, capture_max_bytes: budget, capture_max_age_days: 30, max_pending_rows: 1_000 };
            s.enforce_limits(&limits).unwrap();

            let kept: Vec<bool> = ids.iter().map(|&id| raw_payload(&s, id).has_captures()).collect();
            let kept_bytes: u64 = sizes.iter().zip(&kept).filter(|(_, &k)| k).map(|(&n, _)| n as u64).sum();
            prop_assert!(kept_bytes <= budget);
            // Kept rows are a newest-first run: once a row with images is shed, every older one is too.
            let mut shed_seen = false;
            for (i, (&n, &k)) in sizes.iter().zip(&kept).enumerate().rev() {
                if n > 0 && !k { shed_seen = true; }
                prop_assert!(!(shed_seen && k), "row {} kept after a newer one was shed", i);
            }
        }
    }
}
