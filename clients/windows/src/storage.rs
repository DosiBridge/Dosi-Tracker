use anyhow::Result;
use rusqlite::Connection;

use crate::model::Activity;

/// Offline-first local store. Activities are persisted here first, then synced
/// to the backend. Unsynced rows survive restarts and network outages.
///
/// Row states (`synced` column): 0 = pending upload, 2 = permanently rejected
/// by the server (parked for diagnostics, never retried). Successfully synced
/// rows are deleted so the queue (and its embedded screenshots) stays small.
pub struct Storage {
    conn: Connection,
}

impl Storage {
    pub fn open(path: &str) -> Result<Self> {
        let conn = Connection::open(path)?;
        conn.execute_batch(
            r#"
            CREATE TABLE IF NOT EXISTS activities (
                id           INTEGER PRIMARY KEY AUTOINCREMENT,
                payload      TEXT NOT NULL,
                synced       INTEGER NOT NULL DEFAULT 0,
                error        TEXT NULL,
                created_at   TEXT NOT NULL DEFAULT (datetime('now'))
            );
            "#,
        )?;
        // Older databases predate the error column; add it if missing.
        let _ = conn.execute("ALTER TABLE activities ADD COLUMN error TEXT NULL", []);
        Ok(Self { conn })
    }

    /// Queue an activity for upload.
    pub fn enqueue(&self, activity: &Activity) -> Result<i64> {
        let payload = serde_json::to_string(activity)?;
        self.conn.execute(
            "INSERT INTO activities (payload, synced) VALUES (?1, 0)",
            [payload],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    /// Return all not-yet-synced activities as (row_id, Activity).
    pub fn pending(&self) -> Result<Vec<(i64, Activity)>> {
        let mut stmt = self
            .conn
            .prepare("SELECT id, payload FROM activities WHERE synced = 0 ORDER BY id")?;
        let rows = stmt.query_map([], |row| {
            let id: i64 = row.get(0)?;
            let payload: String = row.get(1)?;
            Ok((id, payload))
        })?;

        let mut out = Vec::new();
        for r in rows {
            let (id, payload) = r?;
            if let Ok(activity) = serde_json::from_str::<Activity>(&payload) {
                out.push((id, activity));
            }
        }
        Ok(out)
    }

    /// The upload succeeded — the local copy (incl. base64 captures) is no longer needed.
    pub fn remove_synced(&self, id: i64) -> Result<()> {
        self.conn
            .execute("DELETE FROM activities WHERE id = ?1", [id])?;
        Ok(())
    }

    /// The server permanently rejected this payload; park it so it never blocks the queue.
    pub fn mark_rejected(&self, id: i64, reason: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE activities SET synced = 2, error = ?2 WHERE id = ?1",
            rusqlite::params![id, reason],
        )?;
        Ok(())
    }

    /// How many payloads the server permanently rejected (surfaced in the UI so
    /// dropped data is never invisible).
    pub fn rejected_count(&self) -> Result<u32> {
        let count: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM activities WHERE synced = 2",
            [],
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    fn sample(project: &str) -> Activity {
        let now = Utc::now();
        Activity {
            client_activity_id: uuid::Uuid::new_v4().to_string(),
            project_id: project.to_string(),
            started_at: now,
            ended_at: now,
            description: None,
            mouse_clicks: 1,
            keyboard_hits: 2,
            active_windows: Vec::new(),
            running_programs: Vec::new(),
            screenshot_png_base64: None,
            webcam_jpg_base64: None,
        }
    }

    fn store() -> Storage {
        Storage::open(":memory:").expect("in-memory sqlite")
    }

    #[test]
    fn enqueue_then_pending_roundtrips() {
        let s = store();
        s.enqueue(&sample("p1")).unwrap();
        let pending = s.pending().unwrap();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].1.project_id, "p1");
    }

    #[test]
    fn synced_rows_are_removed_from_the_queue() {
        let s = store();
        let id = s.enqueue(&sample("p1")).unwrap();
        s.remove_synced(id).unwrap();
        assert!(s.pending().unwrap().is_empty());
    }

    #[test]
    fn rejected_rows_are_parked_not_retried() {
        let s = store();
        let id = s.enqueue(&sample("p1")).unwrap();
        s.mark_rejected(id, "HTTP 400: bad project").unwrap();

        // Parked rows must never come back as pending — that is what used to
        // block the whole queue behind one poison payload.
        assert!(s.pending().unwrap().is_empty());
        assert_eq!(s.rejected_count().unwrap(), 1);
    }

    #[test]
    fn purge_keeps_recent_rejects_and_drops_old_ones() {
        let s = store();
        let id = s.enqueue(&sample("p1")).unwrap();
        s.mark_rejected(id, "nope").unwrap();

        // Just-parked rows survive a 30-day purge.
        assert_eq!(s.purge_rejected_older_than(30).unwrap(), 0);
        assert_eq!(s.rejected_count().unwrap(), 1);

        // Back-date it and it is collected.
        s.conn
            .execute(
                "UPDATE activities SET created_at = datetime('now', '-60 days') WHERE id = ?1",
                [id],
            )
            .unwrap();
        assert_eq!(s.purge_rejected_older_than(30).unwrap(), 1);
        assert_eq!(s.rejected_count().unwrap(), 0);
    }

    // Property-based tests: the queue invariants must hold for ANY sequence of operations,
    // not just the hand-picked cases above.
    use proptest::prelude::*;

    proptest! {
        // pending() returns exactly what was enqueued, in FIFO (insertion) order — the property
        // the whole offline-sync loop relies on to upload oldest-first without gaps or reordering.
        #[test]
        fn pending_preserves_enqueue_count_and_fifo_order(
            projects in prop::collection::vec("[a-z][a-z0-9]{0,7}", 0..24)
        ) {
            let s = store();
            for p in &projects {
                s.enqueue(&sample(p)).unwrap();
            }
            let order: Vec<String> = s.pending().unwrap().into_iter().map(|(_, a)| a.project_id).collect();
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
                s.enqueue(&sample("p")).unwrap();
            }

            let ids: Vec<i64> = s.pending().unwrap().into_iter().map(|(id, _)| id).collect();
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

            prop_assert!(s.pending().unwrap().is_empty());
            prop_assert_eq!(s.rejected_count().unwrap(), rejected);
        }
    }
}
