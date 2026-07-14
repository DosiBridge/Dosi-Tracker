use anyhow::Result;
use rusqlite::Connection;

use crate::model::Activity;

/// Offline-first local store. Activities are persisted here first, then synced
/// to the backend. Unsynced rows survive restarts and network outages.
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
                created_at   TEXT NOT NULL DEFAULT (datetime('now'))
            );
            "#,
        )?;
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

    pub fn mark_synced(&self, id: i64) -> Result<()> {
        self.conn
            .execute("UPDATE activities SET synced = 1 WHERE id = ?1", [id])?;
        Ok(())
    }
}
