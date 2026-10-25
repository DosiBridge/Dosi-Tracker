//! Opening the queue: the schema and its migrations, incremental vacuum, and
//! a damaged database moved aside.

use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::Result;
use chrono::{DateTime, Utc};
use rusqlite::{Connection, OptionalExtension};

use super::{Opened, Storage, SQLITE_SIDECARS};

/// The bytes of a row's images, computed from its JSON (for rows queued
/// before the size was recorded at enqueue time).
pub(super) const CAPTURE_BYTES_OF_PAYLOAD: &str = "CASE WHEN json_valid(payload) THEN \
     COALESCE(length(json_extract(payload, '$.screenshotPngBase64')), 0) \
     + COALESCE(length(json_extract(payload, '$.screenshotThumbJpgBase64')), 0) \
     + COALESCE(length(json_extract(payload, '$.webcamJpgBase64')), 0) \
     ELSE 0 END";

/// Live data up to which a queue created before incremental auto-vacuum is
/// switched to it at once. The switch is a one-off VACUUM, which rewrites all
/// live data (and needs as much free disk); a bigger backlog is switched by
/// [`Storage::reclaim_space`] once a drain has shrunk it, so it never stalls
/// startup.
pub(super) const VACUUM_SWITCH_MAX_LIVE_BYTES: i64 = 64 * 1024 * 1024;

/// The database is damaged (corrupt pages, or not a database at all).
#[derive(Debug)]
pub(super) struct DamagedDatabase(String);

impl std::fmt::Display for DamagedDatabase {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "the local queue is damaged: {}", self.0)
    }
}

impl std::error::Error for DamagedDatabase {}

impl Storage {
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
    pub(super) fn open_with(path: &str, legacy_owner: Option<&str>, vacuum_switch_max_live_bytes: i64) -> Result<Self> {
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

    pub(super) fn open_checked(path: &Path, legacy_owner: Option<&str>) -> Result<Self> {
        let storage = Self::open(&path.to_string_lossy(), legacy_owner)?;
        let verdict: String = storage.conn.query_row("PRAGMA quick_check", [], |row| row.get(0))?;
        if verdict != "ok" {
            return Err(DamagedDatabase(verdict).into());
        }
        Ok(storage)
    }
}

/// The key the previous build tagged `owner_key`'s user with (no server).
pub(super) fn user_only_key(owner_key: &str) -> Option<String> {
    let user = owner_key.strip_prefix("acct\u{1f}")?.rsplit_once('\u{1f}')?.1;
    Some(format!("user\u{1f}{user}"))
}

/// Make sure the database uses incremental auto-vacuum, so deleted rows'
/// pages can be returned to the disk; returns whether it does now. A new
/// database takes the mode as it is created. An existing one needs a one-off
/// rebuild (VACUUM, which also reclaims what it held), done only while its
/// live data is at most `max_live_bytes`.
pub(super) fn switch_to_incremental_vacuum(conn: &Connection, max_live_bytes: i64) -> Result<bool> {
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
pub(super) fn is_damaged(error: &anyhow::Error) -> bool {
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
pub(super) fn set_aside(path: &Path, now: DateTime<Utc>) -> Result<PathBuf> {
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

pub(super) fn with_suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut name = path.as_os_str().to_os_string();
    name.push(suffix);
    PathBuf::from(name)
}

/// Idempotent `ALTER TABLE activities ADD COLUMN`. Returns whether the column
/// was added (i.e. this database predates it).
pub(super) fn add_column_if_missing(conn: &Connection, column: &str, decl: &str) -> Result<bool> {
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
