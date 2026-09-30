use anyhow::Result;
use rusqlite::{Connection, OptionalExtension};

use crate::model::Activity;

/// Identify the account a queued row belongs to: workspace (tenant) + user.
///
/// Rows are only ever uploaded under the token of the account that captured
/// them — otherwise signing out and in as someone else would upload the first
/// user's time and screenshots as the second user's. Both parts are
/// case-insensitive on the server, so they are normalised here. The separator
/// is a control character that cannot be typed into the sign-in form, so two
/// different accounts can never produce the same key.
pub fn owner_key(workspace: &str, username: &str) -> String {
    format!(
        "{}\u{1f}{}",
        workspace.trim().to_lowercase(),
        username.trim().to_lowercase()
    )
}

/// Bounds that keep the offline queue from growing without limit while the
/// backend is unreachable.
///
/// Screenshots/webcam frames are by far the bulk of a row (hundreds of KB of
/// base64 each), so they are shed first: pending rows beyond the newest
/// `capture_max_rows`, or older than `capture_max_age_days`, keep their time
/// data (start/end, counts, activity %, windows) but lose the images. Only
/// beyond `max_pending_rows` are the oldest rows dropped outright.
pub struct QueueLimits {
    pub capture_max_rows: u32,
    pub capture_max_age_days: u32,
    pub max_pending_rows: u32,
}

impl QueueLimits {
    /// ~500 rows is ~80 h of tracking at the default 10-minute interval — two
    /// working weeks offline with every screenshot intact. 10,000 time-only
    /// rows is a few tens of MB and years of normal use.
    pub const DEFAULT: QueueLimits = QueueLimits {
        capture_max_rows: 500,
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

/// SQL expression that nulls out every image payload of a row (screenshot,
/// its thumbnail, webcam), leaving the time data intact. Corrupt JSON is left
/// as-is rather than failing the update.
const STRIP_CAPTURES: &str = "CASE WHEN json_valid(payload) \
     THEN json_set(payload, '$.screenshotPngBase64', NULL, \
     '$.screenshotThumbJpgBase64', NULL, '$.webcamJpgBase64', NULL) \
     ELSE payload END";

/// Offline-first local store. Activities are persisted here first, then synced
/// to the backend. Unsynced rows survive restarts and network outages.
///
/// Row states (`synced` column): 0 = pending upload, 2 = permanently rejected
/// by the server (parked for diagnostics, never retried). Successfully synced
/// rows are deleted so the queue (and its embedded screenshots) stays small.
///
/// Every row carries its `owner` ([`owner_key`]); uploads only ever read the
/// signed-in account's rows.
pub struct Storage {
    conn: Connection,
}

impl Storage {
    /// Open (and migrate) the queue.
    ///
    /// `legacy_owner` is the account signed in when this build first opens a
    /// database created before rows were tagged with an owner: those untagged
    /// rows were captured by that account, so they are assigned to it — once,
    /// at migration time. Without a saved account they stay untagged, are never
    /// uploaded, and are purged by [`Storage::purge_foreign_pending_older_than`].
    pub fn open(path: &str, legacy_owner: Option<&str>) -> Result<Self> {
        let conn = Connection::open(path)?;
        conn.execute_batch(
            r#"
            CREATE TABLE IF NOT EXISTS activities (
                id           INTEGER PRIMARY KEY AUTOINCREMENT,
                payload      TEXT NOT NULL,
                synced       INTEGER NOT NULL DEFAULT 0,
                error        TEXT NULL,
                created_at   TEXT NOT NULL DEFAULT (datetime('now')),
                owner        TEXT NULL,
                stripped     INTEGER NOT NULL DEFAULT 0
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
        tx.execute_batch(
            "CREATE INDEX IF NOT EXISTS ix_activities_owner_synced \
             ON activities (owner, synced, id);",
        )?;
        tx.commit()?;

        Ok(Self { conn })
    }

    /// Queue an activity for upload on behalf of `owner`.
    pub fn enqueue(&self, activity: &Activity, owner: &str) -> Result<i64> {
        let payload = serde_json::to_string(activity)?;
        self.conn.execute(
            "INSERT INTO activities (payload, synced, owner) VALUES (?1, 0, ?2)",
            rusqlite::params![payload, owner],
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
    pub fn pending_batch(&self, owner: &str, after_id: i64, limit: u32) -> Result<Vec<(i64, Activity)>> {
        let rows: Vec<(i64, String)> = {
            let mut stmt = self.conn.prepare(
                "SELECT id, payload FROM activities \
                 WHERE synced = 0 AND owner = ?1 AND id > ?2 \
                 ORDER BY id LIMIT ?3",
            )?;
            let mapped = stmt.query_map(rusqlite::params![owner, after_id, limit], |row| {
                Ok((row.get(0)?, row.get(1)?))
            })?;
            mapped.collect::<rusqlite::Result<_>>()?
        };

        let mut out = Vec::with_capacity(rows.len());
        for (id, payload) in rows {
            match serde_json::from_str::<Activity>(&payload) {
                Ok(activity) => out.push((id, activity)),
                Err(e) => {
                    tracing::warn!(id, ?e, "queued activity is unreadable; parking it");
                    self.mark_rejected(id, &format!("unreadable local payload: {e}"))?;
                }
            }
        }
        Ok(out)
    }

    /// How many of `owner`'s activities are waiting to upload.
    pub fn pending_count(&self, owner: &str) -> Result<u32> {
        let count: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM activities WHERE synced = 0 AND owner = ?1",
            [owner],
            |row| row.get(0),
        )?;
        Ok(count.max(0) as u32)
    }

    /// The upload succeeded — the local copy (incl. base64 captures) is no longer needed.
    pub fn remove_synced(&self, id: i64) -> Result<()> {
        self.conn
            .execute("DELETE FROM activities WHERE id = ?1", [id])?;
        Ok(())
    }

    /// The server permanently rejected this payload; park it so it never blocks
    /// the queue. The reason and time data are kept for diagnostics; the images
    /// are dropped, since a parked row is never uploaded again.
    pub fn mark_rejected(&self, id: i64, reason: &str) -> Result<()> {
        self.conn.execute(
            &format!(
                "UPDATE activities SET synced = 2, error = ?2, stripped = 1, \
                 payload = {STRIP_CAPTURES} WHERE id = ?1"
            ),
            rusqlite::params![id, reason],
        )?;
        Ok(())
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

    /// Drop parked rows older than `days`. Without this the queue — and the
    /// base64 captures inside it — would grow without bound for the life of the
    /// install. Returns how many rows were removed.
    pub fn purge_rejected_older_than(&self, days: u32) -> Result<usize> {
        let removed = self.conn.execute(
            "DELETE FROM activities \
             WHERE synced = 2 AND created_at < datetime('now', ?1)",
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

    /// Apply [`QueueLimits`]: shed images from old/excess pending rows, then
    /// drop the oldest rows beyond the hard cap.
    pub fn enforce_limits(&self, limits: &QueueLimits) -> Result<LimitReport> {
        let stripped = self.conn.execute(
            &format!(
                "UPDATE activities SET payload = {STRIP_CAPTURES}, stripped = 1 \
                 WHERE synced = 0 AND stripped = 0 AND ( \
                     created_at < datetime('now', ?1) \
                     OR id NOT IN (SELECT id FROM activities WHERE synced = 0 \
                                   ORDER BY id DESC LIMIT ?2))"
            ),
            rusqlite::params![
                format!("-{} days", limits.capture_max_age_days),
                limits.capture_max_rows
            ],
        )?;
        let dropped = self.conn.execute(
            "DELETE FROM activities WHERE synced = 0 AND id NOT IN \
             (SELECT id FROM activities WHERE synced = 0 ORDER BY id DESC LIMIT ?1)",
            [limits.max_pending_rows],
        )?;
        Ok(LimitReport { stripped, dropped })
    }
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
    use chrono::Utc;

    const ALICE: &str = "acme\u{1f}alice@acme.com";
    const BOB: &str = "acme\u{1f}bob@acme.com";

    fn sample(project: &str) -> Activity {
        let now = Utc::now();
        Activity {
            client_activity_id: uuid::Uuid::new_v4().to_string(),
            project_id: project.to_string(),
            started_at: now,
            ended_at: now,
            description: None,
            productivity: 40,
            mouse_clicks: 1,
            keyboard_hits: 2,
            active_windows: Vec::new(),
            running_programs: Vec::new(),
            timeline: Vec::new(),
            screenshot_png_base64: None,
            screenshot_thumb_jpg_base64: None,
            webcam_jpg_base64: None,
        }
    }

    fn with_captures(project: &str) -> Activity {
        Activity {
            screenshot_png_base64: Some("c2NyZWVu".into()),
            screenshot_thumb_jpg_base64: Some("dGh1bWI=".into()),
            webcam_jpg_base64: Some("d2ViY2Ft".into()),
            ..sample(project)
        }
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
        assert_eq!(owner_key(" Acme ", "Alice@Acme.com "), ALICE);
        assert_ne!(owner_key("acme", "alice@acme.com"), owner_key("", "alice@acme.com"));
        // The separator keeps the (workspace, user) split unambiguous.
        assert_ne!(owner_key("a", "bc"), owner_key("ab", "c"));
    }

    #[test]
    fn enqueue_then_pending_roundtrips() {
        let s = store();
        s.enqueue(&sample("p1"), ALICE).unwrap();
        let pending = pending(&s, ALICE);
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].1.project_id, "p1");
        assert_eq!(pending[0].1.productivity, 40);
    }

    #[test]
    fn only_the_signed_in_accounts_rows_are_uploadable() {
        let s = store();
        s.enqueue(&sample("alice-1"), ALICE).unwrap();
        s.enqueue(&sample("bob-1"), BOB).unwrap();
        s.enqueue(&sample("alice-2"), ALICE).unwrap();

        let alice: Vec<String> = pending(&s, ALICE).into_iter().map(|(_, a)| a.project_id).collect();
        assert_eq!(alice, ["alice-1", "alice-2"]);
        let bob: Vec<String> = pending(&s, BOB).into_iter().map(|(_, a)| a.project_id).collect();
        assert_eq!(bob, ["bob-1"]);
        assert_eq!(s.pending_count(ALICE).unwrap(), 2);
        assert_eq!(s.pending_count(BOB).unwrap(), 1);
        assert!(pending(&s, "acme\u{1f}mallory@acme.com").is_empty());
    }

    #[test]
    fn batches_page_through_the_queue_oldest_first() {
        let s = store();
        for i in 0..5 {
            s.enqueue(&sample(&format!("p{i}")), ALICE).unwrap();
        }
        s.enqueue(&sample("bob"), BOB).unwrap();

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
                "INSERT INTO activities (payload, synced, owner) VALUES ('{not json', 0, ?1)",
                [ALICE],
            )
            .unwrap();
        s.enqueue(&sample("ok"), ALICE).unwrap();

        let batch = pending(&s, ALICE);
        assert_eq!(batch.len(), 1);
        assert_eq!(batch[0].1.project_id, "ok");
        assert_eq!(s.pending_count(ALICE).unwrap(), 1);
        assert_eq!(s.rejected_count(ALICE).unwrap(), 1);
    }

    #[test]
    fn synced_rows_are_removed_from_the_queue() {
        let s = store();
        let id = s.enqueue(&sample("p1"), ALICE).unwrap();
        s.remove_synced(id).unwrap();
        assert!(pending(&s, ALICE).is_empty());
    }

    #[test]
    fn rejected_rows_are_parked_not_retried() {
        let s = store();
        let id = s.enqueue(&with_captures("p1"), ALICE).unwrap();
        s.mark_rejected(id, "HTTP 400: bad project").unwrap();

        // Parked rows must never come back as pending — that is what used to
        // block the whole queue behind one poison payload.
        assert!(pending(&s, ALICE).is_empty());
        assert_eq!(s.rejected_count(ALICE).unwrap(), 1);
        assert_eq!(s.rejected_count(BOB).unwrap(), 0);

        // The images are dropped; the time data stays for diagnostics.
        let parked = raw_payload(&s, id);
        assert!(parked.screenshot_png_base64.is_none());
        assert!(parked.screenshot_thumb_jpg_base64.is_none());
        assert!(parked.webcam_jpg_base64.is_none());
        assert_eq!(parked.project_id, "p1");
    }

    #[test]
    fn purge_keeps_recent_rejects_and_drops_old_ones() {
        let s = store();
        let id = s.enqueue(&sample("p1"), ALICE).unwrap();
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
        let bob_recent = s.enqueue(&sample("bob-recent"), BOB).unwrap();
        let bob_old = s.enqueue(&sample("bob-old"), BOB).unwrap();
        let alice_old = s.enqueue(&sample("alice-old"), ALICE).unwrap();
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
        for _ in 0..rows {
            conn.execute(
                "INSERT INTO activities (payload, synced) VALUES (?1, 0)",
                [serde_json::to_string(&sample("legacy")).unwrap()],
            )
            .unwrap();
        }
    }

    #[test]
    fn migration_assigns_legacy_rows_to_the_saved_account_once() {
        let path = temp_db_path();
        legacy_db_with_rows(&path, 2);
        let path_str = path.to_string_lossy().into_owned();

        {
            let s = Storage::open(&path_str, Some(ALICE)).unwrap();
            assert_eq!(s.pending_count(ALICE).unwrap(), 2);
        }
        {
            // Re-opening (even as someone else) is a no-op: the migration ran once.
            let s = Storage::open(&path_str, Some(BOB)).unwrap();
            assert_eq!(s.pending_count(ALICE).unwrap(), 2);
            assert_eq!(s.pending_count(BOB).unwrap(), 0);
        }
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
            .map(|i| s.enqueue(&with_captures(&format!("p{i}")), ALICE).unwrap())
            .collect();
        backdate(&s, ids[3], 40); // newest row, but too old to keep images

        let limits = QueueLimits { capture_max_rows: 2, capture_max_age_days: 30, max_pending_rows: 100 };
        let report = s.enforce_limits(&limits).unwrap();
        assert_eq!(report, LimitReport { stripped: 3, dropped: 0 });

        // ids[0], ids[1]: beyond the newest two. ids[3]: too old. ids[2] keeps its images.
        for (i, &id) in ids.iter().enumerate() {
            let row = raw_payload(&s, id);
            assert_eq!(row.screenshot_png_base64.is_some(), i == 2, "row {i}");
            assert_eq!(row.screenshot_thumb_jpg_base64.is_some(), i == 2, "row {i}");
            assert_eq!(row.webcam_jpg_base64.is_some(), i == 2, "row {i}");
            assert_eq!(row.project_id, format!("p{i}"));
            assert_eq!(row.productivity, 40);
        }
        assert_eq!(s.pending_count(ALICE).unwrap(), 4);

        // Idempotent: already-stripped rows are not rewritten again.
        assert_eq!(s.enforce_limits(&limits).unwrap(), LimitReport::default());
    }

    #[test]
    fn limits_drop_the_oldest_rows_beyond_the_hard_cap() {
        let s = store();
        for i in 0..5 {
            s.enqueue(&sample(&format!("p{i}")), ALICE).unwrap();
        }
        let parked = s.enqueue(&sample("parked"), ALICE).unwrap();
        s.mark_rejected(parked, "nope").unwrap();

        let limits = QueueLimits { capture_max_rows: 10, capture_max_age_days: 30, max_pending_rows: 3 };
        assert_eq!(s.enforce_limits(&limits).unwrap().dropped, 2);

        let left: Vec<String> = pending(&s, ALICE).into_iter().map(|(_, a)| a.project_id).collect();
        assert_eq!(left, ["p2", "p3", "p4"]);
        // Parked rows follow their own retention, not the pending cap.
        assert_eq!(s.rejected_count(ALICE).unwrap(), 1);
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
                s.enqueue(&sample(p), ALICE).unwrap();
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
                s.enqueue(&sample("p"), ALICE).unwrap();
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
    }
}
