import Foundation
import SQLite3

/// Offline-first local queue backed by the system SQLite3 (no external deps).
/// Activities are persisted before upload and survive restarts / outages.
final class Storage {
    private var db: OpaquePointer?
    private static let SQLITE_TRANSIENT = unsafeBitCast(-1, to: sqlite3_destructor_type.self)

    // Row states (`synced`): 0 = pending upload, 2 = permanently rejected by the
    // server (parked, never retried). Successfully synced rows are deleted so the
    // queue (and its embedded screenshots) stays small.
    init(path: String) throws {
        guard sqlite3_open(path, &db) == SQLITE_OK else {
            throw StorageError.open(String(cString: sqlite3_errmsg(db)))
        }
        let ddl = """
        CREATE TABLE IF NOT EXISTS activities (
            id         INTEGER PRIMARY KEY AUTOINCREMENT,
            payload    TEXT NOT NULL,
            synced     INTEGER NOT NULL DEFAULT 0,
            error      TEXT NULL,
            created_at TEXT NOT NULL DEFAULT (datetime('now'))
        );
        """
        try exec(ddl)
        // Older databases predate the error column; add it if missing.
        try? exec("ALTER TABLE activities ADD COLUMN error TEXT NULL")
    }

    deinit { sqlite3_close(db) }

    func enqueue(_ activity: Activity) throws {
        let payload = String(data: try JSONEncoder.iso.encode(activity), encoding: .utf8) ?? "{}"
        var stmt: OpaquePointer?
        guard sqlite3_prepare_v2(db, "INSERT INTO activities (payload, synced) VALUES (?, 0)", -1, &stmt, nil) == SQLITE_OK else {
            throw StorageError.prepare(String(cString: sqlite3_errmsg(db)))
        }
        defer { sqlite3_finalize(stmt) }
        sqlite3_bind_text(stmt, 1, payload, -1, Self.SQLITE_TRANSIENT)
        guard sqlite3_step(stmt) == SQLITE_DONE else {
            throw StorageError.step(String(cString: sqlite3_errmsg(db)))
        }
    }

    func pending() throws -> [(Int64, Activity)] {
        var stmt: OpaquePointer?
        guard sqlite3_prepare_v2(db, "SELECT id, payload FROM activities WHERE synced = 0 ORDER BY id", -1, &stmt, nil) == SQLITE_OK else {
            throw StorageError.prepare(String(cString: sqlite3_errmsg(db)))
        }
        defer { sqlite3_finalize(stmt) }

        var result: [(Int64, Activity)] = []
        while sqlite3_step(stmt) == SQLITE_ROW {
            let id = sqlite3_column_int64(stmt, 0)
            guard let cText = sqlite3_column_text(stmt, 1) else { continue }
            let json = String(cString: cText)
            if let data = json.data(using: .utf8),
               let activity = try? JSONDecoder.iso.decode(Activity.self, from: data) {
                result.append((id, activity))
            }
        }
        return result
    }

    /// The upload succeeded — the local copy (incl. base64 captures) is no longer needed.
    func removeSynced(_ id: Int64) throws {
        try exec("DELETE FROM activities WHERE id = \(id)")
    }

    /// The server permanently rejected this payload; park it so it never blocks the queue.
    func markRejected(_ id: Int64, reason: String) throws {
        var stmt: OpaquePointer?
        guard sqlite3_prepare_v2(db, "UPDATE activities SET synced = 2, error = ? WHERE id = ?", -1, &stmt, nil) == SQLITE_OK else {
            throw StorageError.prepare(String(cString: sqlite3_errmsg(db)))
        }
        defer { sqlite3_finalize(stmt) }
        sqlite3_bind_text(stmt, 1, reason, -1, Self.SQLITE_TRANSIENT)
        sqlite3_bind_int64(stmt, 2, id)
        guard sqlite3_step(stmt) == SQLITE_DONE else {
            throw StorageError.step(String(cString: sqlite3_errmsg(db)))
        }
    }

    private func exec(_ sql: String) throws {
        var errMsg: UnsafeMutablePointer<CChar>?
        guard sqlite3_exec(db, sql, nil, nil, &errMsg) == SQLITE_OK else {
            let message = errMsg.map { String(cString: $0) } ?? "unknown error"
            sqlite3_free(errMsg)
            throw StorageError.exec(message)
        }
    }

    enum StorageError: Error {
        case open(String), prepare(String), step(String), exec(String)
    }
}
