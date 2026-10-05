import Foundation
import SQLite3

/// Bounds that keep the offline queue from growing without limit while the
/// server is unreachable.
///
/// Images are by far the bulk of a row, so they are shed first: pending rows
/// keep their images only while they are among the newest `captureMaxRows`,
/// the images of all newer rows plus their own stay within
/// `captureMaxBytes`, and they are younger than `captureMaxAgeDays`. Rows past
/// that keep their time data (start/end, counts, activity %, windows) but lose
/// the images. Only beyond `maxPendingRows` are the oldest rows dropped.
public struct QueueLimits: Equatable, Sendable {
    public var captureMaxRows: Int
    public var captureMaxBytes: Int64
    public var captureMaxAgeDays: Int
    public var maxPendingRows: Int

    public init(captureMaxRows: Int, captureMaxBytes: Int64, captureMaxAgeDays: Int, maxPendingRows: Int) {
        self.captureMaxRows = captureMaxRows
        self.captureMaxBytes = captureMaxBytes
        self.captureMaxAgeDays = captureMaxAgeDays
        self.maxPendingRows = maxPendingRows
    }

    /// ~500 rows is ~80 h of tracking at the default 10-minute interval — two
    /// working weeks offline with every screenshot intact — and 300 MB holds
    /// them at typical JPEG sizes. 10,000 time-only rows are a few tens of MB
    /// and years of normal use. (The Windows agent's `QueueLimits::DEFAULT`.)
    public static let standard = QueueLimits(
        captureMaxRows: 500,
        captureMaxBytes: 300 * 1024 * 1024,
        captureMaxAgeDays: 30,
        maxPendingRows: 10_000
    )
}

/// What ``Queue/enforceLimits(_:)`` did, for logging.
public struct LimitReport: Equatable, Sendable {
    public var stripped: Int
    public var dropped: Int

    public init(stripped: Int = 0, dropped: Int = 0) {
        self.stripped = stripped
        self.dropped = dropped
    }
}

/// What ``Queue/maintain(currentOwner:limits:retentionDays:)`` removed.
public struct MaintenanceReport: Equatable, Sendable {
    public var purgedRejected = 0
    public var purgedForeign = 0
    public var limits = LimitReport()

    public init() {}
}

/// A queued block ready to upload.
public struct QueuedActivity: Equatable, Sendable {
    public let id: Int64
    public let payload: ActivityPayload

    public init(id: Int64, payload: ActivityPayload) {
        self.id = id
        self.payload = payload
    }
}

/// One row of the queue as the window lists it, read from the summary
/// columns only (never the payload with its images).
public struct QueuedRow: Equatable, Sendable, Identifiable {
    public enum Status: Equatable, Sendable {
        /// Waiting to upload.
        case queued
        /// Parked: the server refused it; kept for diagnostics, never retried.
        case rejected
    }

    public let id: Int64
    public let projectId: String?
    public let startedAt: Date?
    public let endedAt: Date?
    public let status: Status
    /// Rejected: the server's reason. Queued: why the last attempt failed, if
    /// one did.
    public let reason: String?
    /// Still carries screenshot / webcam images.
    public let hasCaptures: Bool

    public init(id: Int64, projectId: String?, startedAt: Date?, endedAt: Date?, status: Status,
                reason: String?, hasCaptures: Bool) {
        self.id = id
        self.projectId = projectId
        self.startedAt = startedAt
        self.endedAt = endedAt
        self.status = status
        self.reason = reason
        self.hasCaptures = hasCaptures
    }
}

/// The offline-first local queue (port of the Windows agent's `storage.rs`):
/// every block is saved here first, then uploaded, and survives restarts and
/// outages. Backed by the system SQLite through its C API.
///
/// Row states (`synced` column): 0 = pending upload, 2 = permanently rejected
/// by the server (parked for diagnostics, never retried). Uploaded rows are
/// deleted, so the queue (and its embedded images) stays small; the database
/// uses incremental auto-vacuum and ``reclaimSpace()`` gives the freed pages
/// back to the disk.
///
/// Every row carries its `owner` (``ownerKey(workspace:username:)``); uploads
/// only ever read the signed-in account's rows.
///
/// Thread-safe: every call runs under one lock on one connection.
public final class Queue: @unchecked Sendable {
    /// File name inside ``AppPaths/applicationSupport``.
    public static let fileName = "dosi-tracker.db"

    /// Files SQLite keeps next to a database (a hot journal is part of it).
    public static let sidecarSuffixes = ["-journal", "-wal", "-shm"]

    /// SQL expression that removes every image of a row (screen capture, its
    /// thumbnail, webcam, and the capture's time and blur flag), leaving the
    /// time data intact. Corrupt JSON is left as-is rather than failing the
    /// update.
    static let stripCapturesSQL = """
        CASE WHEN json_valid(payload) THEN json_remove(payload, '$.screenshotPngBase64', \
        '$.screenshotThumbJpgBase64', '$.webcamJpgBase64', '$.screenshotCapturedAt', \
        '$.screenshotBlurred') ELSE payload END
        """

    /// The bytes of a row's images, computed from its JSON (for rows queued
    /// before the size was recorded at enqueue time).
    static let captureBytesOfPayloadSQL = """
        CASE WHEN json_valid(payload) THEN \
        COALESCE(length(json_extract(payload, '$.screenshotPngBase64')), 0) \
        + COALESCE(length(json_extract(payload, '$.screenshotThumbJpgBase64')), 0) \
        + COALESCE(length(json_extract(payload, '$.webcamJpgBase64')), 0) \
        ELSE 0 END
        """

    let db: SQLiteDatabase
    private let lock = NSLock()

    /// Set when ``open(at:legacyOwner:now:)`` found a damaged database and
    /// moved it aside to start a fresh one: where the old file now is (for the
    /// window's notice that the queue was reset).
    public private(set) var movedAside: URL?

    /// Identify the account a row belongs to: workspace (tenant) + user, both
    /// trimmed and lower-cased (the server compares them case-insensitively),
    /// joined by U+001F, a control character that cannot be typed into the
    /// sign-in form, so two different accounts never produce the same key.
    public static func ownerKey(workspace: String, username: String) -> String {
        workspace.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
            + "\u{1F}"
            + username.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
    }

    // MARK: Opening

    /// Open the queue at `url`, checking that it is intact. A damaged file
    /// (corrupt pages, or not a database at all) is renamed aside as
    /// `<name>.corrupt-<UTC time>` together with its journal, and a fresh,
    /// empty queue takes its place (``movedAside`` says where the old file
    /// went), so the agent keeps tracking. Other failures (locked, no
    /// permission, disk full, a directory in the way) are thrown: renaming
    /// the file would not help.
    public static func open(at url: URL, legacyOwner: String?, now: Date = Date()) throws -> Queue {
        do {
            return try openChecked(path: url.path, legacyOwner: legacyOwner)
        } catch let error as QueueError where error.isDamage {
            let aside = try setAside(databaseAt: url, now: now)
            CoreLog.error("queue", "the local queue was damaged (\(error)); moved it to \(aside.lastPathComponent) and started a new one")
            let queue = try openChecked(path: url.path, legacyOwner: legacyOwner)
            queue.movedAside = aside
            return queue
        }
    }

    private static func openChecked(path: String, legacyOwner: String?) throws -> Queue {
        let queue = try Queue(path: path, legacyOwner: legacyOwner)
        let verdict = try queue.quickCheck()
        guard verdict == "ok" else {
            throw QueueError(code: SQLITE_CORRUPT, message: "integrity check failed: \(verdict)")
        }
        return queue
    }

    /// Rename a damaged database to `<name>.corrupt-<UTC time>`, its sidecar
    /// files with it (as `<name>.corrupt-<time>-journal`, so SQLite still
    /// pairs them). A sidecar that cannot be moved is deleted: a stale journal
    /// next to the fresh database would be rolled back into it. Returns the
    /// new location of the main file.
    @discardableResult
    public static func setAside(databaseAt url: URL, now: Date = Date()) throws -> URL {
        let fileManager = FileManager.default
        let base = url.path + ".corrupt-" + WireDate.compactStamp(from: now)
        var asidePath = base
        var attempt = 1
        while fileManager.fileExists(atPath: asidePath) {
            asidePath = base + "-\(attempt)"
            attempt += 1
        }
        try fileManager.moveItem(atPath: url.path, toPath: asidePath)
        for suffix in sidecarSuffixes {
            let sidecar = url.path + suffix
            guard fileManager.fileExists(atPath: sidecar) else { continue }
            do {
                try fileManager.moveItem(atPath: sidecar, toPath: asidePath + suffix)
            } catch {
                try fileManager.removeItem(atPath: sidecar)
            }
        }
        return URL(fileURLWithPath: asidePath)
    }

    /// Open (and migrate) the queue at `path` (`:memory:` for an in-memory
    /// queue), without the integrity check of ``open(at:legacyOwner:now:)``.
    ///
    /// `legacyOwner` is the account signed in when this build first opens a
    /// database created before rows were tagged with an owner: those untagged
    /// rows were captured by that account, so they are assigned to it — once,
    /// at migration time. Without a saved account they stay untagged, are
    /// never uploaded, and expire with
    /// ``purgeForeignPending(currentOwner:olderThanDays:)``.
    public init(path: String, legacyOwner: String?) throws {
        db = try SQLiteDatabase(path: path)
        try migrate(legacyOwner: legacyOwner)
    }

    private func migrate(legacyOwner: String?) throws {
        try enableIncrementalVacuum()
        try db.execute("""
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
            """)

        // Older databases predate some columns; add them if missing. One
        // transaction, so a crash can never leave the owner column added but
        // the legacy rows unclaimed-and-claimable by a later, different account.
        try db.execute("BEGIN IMMEDIATE;")
        do {
            try addColumnIfMissing("error", "TEXT NULL")
            if try addColumnIfMissing("owner", "TEXT NULL"), let owner = legacyOwner {
                let claimed = try db.run("UPDATE activities SET owner = ?1 WHERE owner IS NULL", [.text(owner)])
                if claimed > 0 {
                    CoreLog.info("queue", "assigned \(claimed) queued activities to the saved account")
                }
            }
            try addColumnIfMissing("stripped", "INTEGER NOT NULL DEFAULT 0")
            if try addColumnIfMissing("capture_bytes", "INTEGER NOT NULL DEFAULT 0") {
                try db.run("UPDATE activities SET capture_bytes = \(Queue.captureBytesOfPayloadSQL) WHERE stripped = 0")
            }
            let summaryColumnsAdded = try addColumnIfMissing("project_id", "TEXT NULL")
            try addColumnIfMissing("started_at", "TEXT NULL")
            try addColumnIfMissing("ended_at", "TEXT NULL")
            if summaryColumnsAdded {
                try db.run("""
                    UPDATE activities SET
                        project_id = json_extract(payload, '$.projectId'),
                        started_at = json_extract(payload, '$.startedAt'),
                        ended_at = json_extract(payload, '$.endedAt')
                    WHERE json_valid(payload)
                    """)
            }
            try db.execute("CREATE INDEX IF NOT EXISTS ix_activities_owner_synced ON activities (owner, synced, id);")
            try db.execute("COMMIT;")
        } catch {
            try? db.execute("ROLLBACK;")
            throw error
        }
    }

    /// Switch the database to incremental auto-vacuum, so deleted rows' pages
    /// can be returned to the disk. A new database takes the mode at once; an
    /// existing one is rebuilt once (VACUUM), which also reclaims what it held.
    private func enableIncrementalVacuum() throws {
        let incremental: Int64 = 2
        if try db.scalar("PRAGMA auto_vacuum") == incremental {
            return
        }
        try db.execute("PRAGMA auto_vacuum = INCREMENTAL;")
        let pages = try db.scalar("PRAGMA page_count") ?? 0
        guard pages > 0 else { return }
        do {
            try db.execute("VACUUM;")
            CoreLog.info("queue", "local queue rebuilt for incremental vacuum")
        } catch {
            // Needs free disk space about the database's size; it is retried
            // at the next start, and the queue works either way.
            CoreLog.warning("queue", "could not switch the local queue to incremental vacuum: \(error)")
        }
    }

    /// Idempotent `ALTER TABLE activities ADD COLUMN`. Returns whether the
    /// column was added (this database predates it).
    @discardableResult
    private func addColumnIfMissing(_ column: String, _ declaration: String) throws -> Bool {
        let exists = try db.scalar("SELECT 1 FROM pragma_table_info('activities') WHERE name = ?1", [.text(column)]) != nil
        if exists {
            return false
        }
        try db.execute("ALTER TABLE activities ADD COLUMN \(column) \(declaration);")
        return true
    }

    /// `PRAGMA quick_check`: `ok` for an intact database.
    func quickCheck() throws -> String {
        lock.lock()
        defer { lock.unlock() }
        let verdicts = try db.query("PRAGMA quick_check") { $0.text(0) ?? "" }
        return verdicts.isEmpty ? "ok" : verdicts.joined(separator: "; ")
    }

    // MARK: Writing

    /// Queue a block for upload on behalf of `owner`. Returns the row id.
    @discardableResult
    public func enqueue(_ payload: ActivityPayload, owner: String) throws -> Int64 {
        let json = try Queue.encode(payload)
        lock.lock()
        defer { lock.unlock() }
        try db.run(
            """
            INSERT INTO activities (payload, synced, owner, capture_bytes, project_id, started_at, ended_at)
            VALUES (?1, 0, ?2, ?3, ?4, ?5, ?6)
            """,
            [
                .text(json), .text(owner), .int(payload.captureBytes), .text(payload.projectId),
                .text(WireDate.string(from: payload.startedAt)), .text(WireDate.string(from: payload.endedAt)),
            ])
        return db.lastInsertRowId
    }

    /// The upload succeeded: the local copy (with its images) is no longer
    /// needed.
    public func removeSynced(id: Int64) throws {
        lock.lock()
        defer { lock.unlock() }
        try db.run("DELETE FROM activities WHERE id = ?1", [.int(id)])
    }

    /// Remember why the last upload attempt of a pending row failed (shown
    /// with the row; nil clears it). The row stays queued; a parked row's
    /// reason is never overwritten.
    public func noteAttempt(id: Int64, reason: String?) throws {
        lock.lock()
        defer { lock.unlock() }
        try db.run("UPDATE activities SET error = ?2 WHERE id = ?1 AND synced = 0",
                   [.int(id), reason.map { SQLiteValue.text($0) } ?? .null])
    }

    /// The server permanently rejected this block: park it so it never blocks
    /// the queue. The reason and time data stay for diagnostics; the images
    /// are dropped, since a parked row is never uploaded again.
    public func markRejected(id: Int64, reason: String) throws {
        lock.lock()
        defer { lock.unlock() }
        try markRejectedLocked(id: id, reason: reason)
    }

    private func markRejectedLocked(id: Int64, reason: String) throws {
        try db.run(
            """
            UPDATE activities SET synced = 2, error = ?2, stripped = 1, capture_bytes = 0,
                payload = \(Queue.stripCapturesSQL) WHERE id = ?1
            """,
            [.int(id), .text(reason)])
    }

    /// Remove the images from one pending row, keeping its time data — what a
    /// 413 (the request was too large) calls for before resending it. Returns
    /// whether the row was still queued.
    @discardableResult
    public func stripCaptures(id: Int64) throws -> Bool {
        lock.lock()
        defer { lock.unlock() }
        let changed = try db.run(
            """
            UPDATE activities SET payload = \(Queue.stripCapturesSQL), stripped = 1, capture_bytes = 0
            WHERE id = ?1 AND synced = 0
            """,
            [.int(id)])
        return changed > 0
    }

    // MARK: Reading

    /// Up to `limit` of `owner`'s pending blocks with a row id greater than
    /// `afterId`, oldest first.
    ///
    /// Reading in small batches keeps memory flat however large the backlog
    /// (each row can embed a screenshot). Paging by id rather than by offset
    /// means a row that fails to leave the queue is never handed back in the
    /// same pass. A row whose payload no longer decodes is parked as rejected
    /// instead of sitting in the queue forever.
    public func pendingBatch(owner: String, afterId: Int64, limit: Int) throws -> [QueuedActivity] {
        lock.lock()
        defer { lock.unlock() }
        let rows: [(Int64, String)] = try db.query(
            "SELECT id, payload FROM activities WHERE synced = 0 AND owner = ?1 AND id > ?2 ORDER BY id LIMIT ?3",
            [.text(owner), .int(afterId), .int(Int64(max(limit, 0)))]
        ) { row in
            (row.int64(0), row.text(1) ?? "")
        }

        var batch: [QueuedActivity] = []
        batch.reserveCapacity(rows.count)
        for (id, json) in rows {
            do {
                batch.append(QueuedActivity(id: id, payload: try Queue.decode(json)))
            } catch {
                CoreLog.warning("queue", "queued activity \(id) is unreadable; parking it")
                try markRejectedLocked(id: id, reason: "unreadable local payload: \(error)")
            }
        }
        return batch
    }

    /// How many of `owner`'s blocks are waiting to upload.
    public func pendingCount(owner: String) throws -> Int {
        lock.lock()
        defer { lock.unlock() }
        return Int(try db.scalar("SELECT COUNT(*) FROM activities WHERE synced = 0 AND owner = ?1", [.text(owner)]) ?? 0)
    }

    /// How many of `owner`'s blocks the server permanently rejected (shown in
    /// the window so dropped data is never invisible).
    public func rejectedCount(owner: String) throws -> Int {
        lock.lock()
        defer { lock.unlock() }
        return Int(try db.scalar("SELECT COUNT(*) FROM activities WHERE synced = 2 AND owner = ?1", [.text(owner)]) ?? 0)
    }

    /// `owner`'s newest rows, queued and parked, newest first — what the
    /// window lists, with each parked row's reason. Reads only the summary
    /// columns, never the payloads.
    public func recentRows(owner: String, limit: Int) throws -> [QueuedRow] {
        lock.lock()
        defer { lock.unlock() }
        return try db.query(
            """
            SELECT id, synced, error, project_id, started_at, ended_at, capture_bytes
            FROM activities WHERE owner = ?1 AND synced IN (0, 2)
            ORDER BY id DESC LIMIT ?2
            """,
            [.text(owner), .int(Int64(max(limit, 0)))]
        ) { row in
            QueuedRow(
                id: row.int64(0),
                projectId: row.text(3),
                startedAt: row.text(4).flatMap { WireDate.date(from: $0) },
                endedAt: row.text(5).flatMap { WireDate.date(from: $0) },
                status: row.int64(1) == 2 ? .rejected : .queued,
                reason: row.text(2),
                hasCaptures: row.int64(6) > 0
            )
        }
    }

    // MARK: Housekeeping

    /// Drop parked rows older than `days`. Without this the queue would grow
    /// for the life of the install. Returns how many rows were removed.
    @discardableResult
    public func purgeRejected(olderThanDays days: Int) throws -> Int {
        lock.lock()
        defer { lock.unlock() }
        return try db.run(
            "DELETE FROM activities WHERE synced = 2 AND created_at < datetime('now', ?1)",
            [.text("-\(days) days")])
    }

    /// Drop pending rows that belong to anyone other than `currentOwner` (or
    /// to no one) once they are older than `days`. Another account's rows wait
    /// for that account to sign in again, but not forever. Without a known
    /// owner nothing goes: the rows may be the user's own (the saved sign-in
    /// could not be read, or they signed out and come back), and the server
    /// still takes them for weeks.
    @discardableResult
    public func purgeForeignPending(currentOwner: String?, olderThanDays days: Int) throws -> Int {
        guard let currentOwner = currentOwner else { return 0 }
        lock.lock()
        defer { lock.unlock() }
        return try db.run(
            """
            DELETE FROM activities
            WHERE synced = 0 AND owner IS NOT ?1
              AND created_at < datetime('now', ?2)
            """,
            [.text(currentOwner), .text("-\(days) days")])
    }

    /// Apply ``QueueLimits``: shed images from old or excess pending rows (the
    /// newest keep theirs, within the row and byte budgets), then drop the
    /// oldest rows beyond the hard cap. Parked rows follow their own
    /// retention.
    @discardableResult
    public func enforceLimits(_ limits: QueueLimits) throws -> LimitReport {
        lock.lock()
        defer { lock.unlock() }
        let stripped = try db.run(
            """
            UPDATE activities SET payload = \(Queue.stripCapturesSQL), stripped = 1, capture_bytes = 0
            WHERE id IN (
                SELECT id FROM (
                    SELECT id, stripped, created_at,
                           ROW_NUMBER() OVER newest AS rank,
                           SUM(capture_bytes) OVER newest AS kept_bytes
                    FROM activities WHERE synced = 0
                    WINDOW newest AS (ORDER BY id DESC ROWS BETWEEN UNBOUNDED PRECEDING AND CURRENT ROW))
                WHERE stripped = 0
                  AND (rank > ?2 OR kept_bytes > ?3 OR created_at < datetime('now', ?1)))
            """,
            [
                .text("-\(limits.captureMaxAgeDays) days"),
                .int(Int64(limits.captureMaxRows)),
                .int(limits.captureMaxBytes),
            ])
        let dropped = try db.run(
            """
            DELETE FROM activities WHERE synced = 0 AND id NOT IN
                (SELECT id FROM activities WHERE synced = 0 ORDER BY id DESC LIMIT ?1)
            """,
            [.int(Int64(limits.maxPendingRows))])
        return LimitReport(stripped: stripped, dropped: dropped)
    }

    /// Give the pages freed by uploads, parked rows and shed images back to
    /// the disk. Cheap when there is nothing to free; run it after a drain.
    public func reclaimSpace() throws {
        lock.lock()
        defer { lock.unlock() }
        // sqlite3_exec steps the pragma to the end: every free page goes.
        try db.execute("PRAGMA incremental_vacuum;")
    }

    /// The housekeeping run at launch and after each sign-in: expire parked
    /// rows and other accounts' stale rows, then shed images / rows beyond
    /// `limits`. Failures are logged, never thrown: housekeeping must not stop
    /// tracking.
    @discardableResult
    public func maintain(currentOwner: String?, limits: QueueLimits = .standard,
                         retentionDays: Int = AgentTiming.rejectedRetentionDays) -> MaintenanceReport {
        var report = MaintenanceReport()
        do {
            report.purgedRejected = try purgeRejected(olderThanDays: retentionDays)
            if report.purgedRejected > 0 {
                CoreLog.info("queue", "removed \(report.purgedRejected) old rejected activities")
            }
        } catch {
            CoreLog.warning("queue", "could not purge rejected activities: \(error)")
        }
        do {
            report.purgedForeign = try purgeForeignPending(currentOwner: currentOwner, olderThanDays: retentionDays)
            if report.purgedForeign > 0 {
                CoreLog.info("queue", "removed \(report.purgedForeign) expired activities of other accounts")
            }
        } catch {
            CoreLog.warning("queue", "could not purge other accounts' activities: \(error)")
        }
        report.limits = enforceLimitsLogged(limits)
        return report
    }

    /// ``enforceLimits(_:)``, logging instead of throwing (run after every
    /// block queued).
    @discardableResult
    public func enforceLimitsLogged(_ limits: QueueLimits = .standard) -> LimitReport {
        do {
            let report = try enforceLimits(limits)
            if report.stripped > 0 || report.dropped > 0 {
                CoreLog.warning("queue", "offline queue over its limits; shed images of \(report.stripped) "
                                + "and dropped \(report.dropped) oldest activities")
            }
            return report
        } catch {
            CoreLog.warning("queue", "could not enforce offline queue limits: \(error)")
            return LimitReport()
        }
    }

    // MARK: Coding

    static func encode(_ payload: ActivityPayload) throws -> String {
        let data = try WireJSON.encoder().encode(payload)
        return String(decoding: data, as: UTF8.self)
    }

    static func decode(_ json: String) throws -> ActivityPayload {
        try WireJSON.decoder().decode(ActivityPayload.self, from: Data(json.utf8))
    }
}
