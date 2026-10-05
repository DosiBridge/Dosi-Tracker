import XCTest
@testable import DosiTrackerCore

/// Ports of the Windows agent's `storage.rs` tests, on temporary files.
final class QueueTests: XCTestCase {
    private let alice = "acme\u{1F}alice@acme.com"
    private let bob = "acme\u{1F}bob@acme.com"
    private var files: [URL] = []

    override func tearDown() {
        for file in files {
            removeDatabase(at: file)
        }
        files.removeAll()
        super.tearDown()
    }

    private func temporaryDatabase() -> URL {
        let url = temporaryFileURL("dosi-queue")
        files.append(url)
        return url
    }

    private func store() throws -> Queue {
        try Queue(path: temporaryDatabase().path, legacyOwner: nil)
    }

    private func sample(_ project: String) -> ActivityPayload {
        let now = Date()
        var payload = ActivityPayload(projectId: project, startedAt: now.addingTimeInterval(-600), endedAt: now)
        payload.productivity = 40
        payload.mouseClicks = 1
        payload.keyboardHits = 2
        return payload
    }

    private func withCaptures(_ project: String) -> ActivityPayload {
        var payload = sample(project)
        payload.screenshotPngBase64 = "c2NyZWVu"
        payload.screenshotThumbJpgBase64 = "dGh1bWI="
        payload.webcamJpgBase64 = "d2ViY2Ft"
        payload.screenshotCapturedAt = Date()
        payload.screenshotBlurred = false
        return payload
    }

    /// A row whose images weigh `bytes` in total.
    private func withImageBytes(_ project: String, _ bytes: Int) -> ActivityPayload {
        var payload = sample(project)
        payload.screenshotPngBase64 = String(repeating: "A", count: bytes)
        return payload
    }

    /// Everything pending for `owner`, drained through the batch API.
    private func pending(_ queue: Queue, _ owner: String) throws -> [QueuedActivity] {
        try queue.pendingBatch(owner: owner, afterId: 0, limit: Int(Int32.max))
    }

    private func backdate(_ queue: Queue, _ id: Int64, days: Int) throws {
        try queue.db.run("UPDATE activities SET created_at = datetime('now', ?2) WHERE id = ?1",
                         [.int(id), .text("-\(days) days")])
    }

    private func rawPayload(_ queue: Queue, _ id: Int64) throws -> ActivityPayload {
        let json = try XCTUnwrap(try queue.db.query("SELECT payload FROM activities WHERE id = ?1", [.int(id)]) {
            $0.text(0)
        }.first ?? nil)
        return try Queue.decode(json)
    }

    // MARK: Ownership and order

    func testOwnerKeyIsCaseAndWhitespaceInsensitive() {
        XCTAssertEqual(Queue.ownerKey(workspace: " Acme ", username: "Alice@Acme.com "), alice)
        XCTAssertNotEqual(Queue.ownerKey(workspace: "acme", username: "alice@acme.com"),
                          Queue.ownerKey(workspace: "", username: "alice@acme.com"))
        // The separator keeps the (workspace, user) split unambiguous.
        XCTAssertNotEqual(Queue.ownerKey(workspace: "a", username: "bc"), Queue.ownerKey(workspace: "ab", username: "c"))
        XCTAssertEqual(ApiClient.Account(workspace: "Acme", username: "Alice@Acme.com", password: "x").ownerKey, alice)
    }

    func testEnqueueThenPendingRoundTrips() throws {
        let queue = try store()
        let original = sample("p1")
        try queue.enqueue(original, owner: alice)
        let rows = try pending(queue, alice)
        XCTAssertEqual(rows.count, 1)
        XCTAssertEqual(rows[0].payload.projectId, "p1")
        XCTAssertEqual(rows[0].payload.productivity, 40)
        XCTAssertEqual(rows[0].payload.clientActivityId, original.clientActivityId)
    }

    func testOnlyTheSignedInAccountsRowsAreUploadable() throws {
        let queue = try store()
        try queue.enqueue(sample("alice-1"), owner: alice)
        try queue.enqueue(sample("bob-1"), owner: bob)
        try queue.enqueue(sample("alice-2"), owner: alice)

        XCTAssertEqual(try pending(queue, alice).map { $0.payload.projectId }, ["alice-1", "alice-2"])
        XCTAssertEqual(try pending(queue, bob).map { $0.payload.projectId }, ["bob-1"])
        XCTAssertEqual(try queue.pendingCount(owner: alice), 2)
        XCTAssertEqual(try queue.pendingCount(owner: bob), 1)
        XCTAssertTrue(try pending(queue, "acme\u{1F}mallory@acme.com").isEmpty)
    }

    func testBatchesPageThroughTheQueueOldestFirst() throws {
        let queue = try store()
        for index in 0..<5 {
            try queue.enqueue(sample("p\(index)"), owner: alice)
        }
        try queue.enqueue(sample("bob"), owner: bob)

        var seen: [String] = []
        var cursor: Int64 = 0
        while true {
            let batch = try queue.pendingBatch(owner: alice, afterId: cursor, limit: 2)
            guard let last = batch.last else { break }
            XCTAssertLessThanOrEqual(batch.count, 2)
            cursor = last.id
            seen += batch.map { $0.payload.projectId }
        }
        XCTAssertEqual(seen, ["p0", "p1", "p2", "p3", "p4"])
    }

    func testPendingPreservesEnqueueCountAndFifoOrderForAnyBatchSize() throws {
        // The property the whole offline-sync loop relies on, over seeded
        // random sequences.
        var generator = SplitMix(seed: 7)
        for round in 0..<12 {
            let queue = try store()
            let count = Int.random(in: 0..<24, using: &generator)
            let projects = (0..<count).map { "r\(round)-p\($0)" }
            for project in projects {
                try queue.enqueue(sample(project), owner: alice)
            }
            let batchSize = Int.random(in: 1..<8, using: &generator)
            var order: [String] = []
            var cursor: Int64 = 0
            while true {
                let rows = try queue.pendingBatch(owner: alice, afterId: cursor, limit: batchSize)
                guard let last = rows.last else { break }
                XCTAssertLessThanOrEqual(rows.count, batchSize)
                cursor = last.id
                order += rows.map { $0.payload.projectId }
            }
            XCTAssertEqual(order, projects)
        }
    }

    func testUnreadablePayloadsAreParkedNotSkippedForever() throws {
        let queue = try store()
        try queue.db.run("INSERT INTO activities (payload, synced, owner) VALUES ('{not json', 0, ?1)", [.text(alice)])
        try queue.enqueue(sample("ok"), owner: alice)

        let batch = try pending(queue, alice)
        XCTAssertEqual(batch.map { $0.payload.projectId }, ["ok"])
        XCTAssertEqual(try queue.pendingCount(owner: alice), 1)
        XCTAssertEqual(try queue.rejectedCount(owner: alice), 1)
    }

    // MARK: Leaving the queue

    func testSyncedRowsAreRemovedFromTheQueue() throws {
        let queue = try store()
        let id = try queue.enqueue(sample("p1"), owner: alice)
        try queue.removeSynced(id: id)
        XCTAssertTrue(try pending(queue, alice).isEmpty)
    }

    func testRejectedRowsAreParkedNotRetried() throws {
        let queue = try store()
        let id = try queue.enqueue(withCaptures("p1"), owner: alice)
        try queue.markRejected(id: id, reason: "HTTP 400: bad project")

        // Parked rows must never come back as pending — that is what used to
        // block the whole queue behind one poison payload.
        XCTAssertTrue(try pending(queue, alice).isEmpty)
        XCTAssertEqual(try queue.rejectedCount(owner: alice), 1)
        XCTAssertEqual(try queue.rejectedCount(owner: bob), 0)

        // The images are dropped; the time data stays for diagnostics.
        let parked = try rawPayload(queue, id)
        XCTAssertFalse(parked.hasCaptures)
        XCTAssertNil(parked.screenshotCapturedAt)
        XCTAssertNil(parked.screenshotBlurred)
        XCTAssertEqual(parked.projectId, "p1")
    }

    func testDisposedRowsNeverReappearAndCountsAreConserved() throws {
        var generator = SplitMix(seed: 11)
        for _ in 0..<10 {
            let queue = try store()
            let keep = (0..<Int.random(in: 0..<24, using: &generator)).map { _ in Bool.random(using: &generator) }
            for _ in keep {
                try queue.enqueue(sample("p"), owner: alice)
            }
            let ids = try pending(queue, alice).map { $0.id }
            XCTAssertEqual(ids.count, keep.count)
            var rejected = 0
            for (id, uploaded) in zip(ids, keep) {
                if uploaded {
                    try queue.removeSynced(id: id)
                } else {
                    try queue.markRejected(id: id, reason: "server rejected")
                    rejected += 1
                }
            }
            XCTAssertTrue(try pending(queue, alice).isEmpty)
            XCTAssertEqual(try queue.rejectedCount(owner: alice), rejected)
        }
    }

    func testRecentRowsListBothStatesWithTheServerReason() throws {
        let queue = try store()
        let first = sample("p1")
        let a = try queue.enqueue(first, owner: alice)
        let b = try queue.enqueue(withCaptures("p2"), owner: alice)
        let c = try queue.enqueue(sample("p3"), owner: alice)
        try queue.enqueue(sample("bob"), owner: bob)
        try queue.markRejected(id: b, reason: "Overlaps another block.")
        try queue.noteAttempt(id: c, reason: "Your computer clock is ahead")

        var rows = try queue.recentRows(owner: alice, limit: 10)
        XCTAssertEqual(rows.map { $0.id }, [c, b, a], "newest first, Alice's only")
        XCTAssertEqual(rows[0].status, .queued)
        XCTAssertEqual(rows[0].reason, "Your computer clock is ahead")
        XCTAssertEqual(rows[1].status, .rejected)
        XCTAssertEqual(rows[1].reason, "Overlaps another block.")
        XCTAssertFalse(rows[1].hasCaptures, "parking drops the images")
        XCTAssertEqual(rows[2].projectId, "p1")
        // Millisecond precision is what the column keeps.
        XCTAssertEqual(rows[2].startedAt.map { EpochTime.milliseconds($0) }, EpochTime.milliseconds(first.startedAt))
        XCTAssertEqual(rows[2].endedAt.map { EpochTime.milliseconds($0) }, EpochTime.milliseconds(first.endedAt))

        XCTAssertEqual(try queue.recentRows(owner: alice, limit: 1).count, 1)
        // Clearing the note; a parked row's reason cannot be overwritten by it.
        try queue.noteAttempt(id: c, reason: nil)
        try queue.noteAttempt(id: b, reason: "ignored")
        rows = try queue.recentRows(owner: alice, limit: 10)
        XCTAssertNil(rows[0].reason)
        XCTAssertEqual(rows[1].reason, "Overlaps another block.")
    }

    func testStrippingOneRowKeepsItsTimeDataAndPlaceInTheQueue() throws {
        let queue = try store()
        let id = try queue.enqueue(withCaptures("p1"), owner: alice)
        let other = try queue.enqueue(withCaptures("p2"), owner: alice)
        XCTAssertTrue(try queue.stripCaptures(id: id))

        let row = try rawPayload(queue, id)
        XCTAssertFalse(row.hasCaptures)
        XCTAssertEqual(row.projectId, "p1")
        XCTAssertEqual(row.productivity, 40)
        XCTAssertEqual(row.keyboardHits, 2)
        XCTAssertTrue(try rawPayload(queue, other).hasCaptures, "only that row")
        XCTAssertEqual(try pending(queue, alice).count, 2, "still queued, to be resent")
        XCTAssertFalse(try queue.recentRows(owner: alice, limit: 10).first { $0.id == id }?.hasCaptures ?? true)

        // A parked row is not pending: nothing to strip.
        try queue.markRejected(id: other, reason: "nope")
        XCTAssertFalse(try queue.stripCaptures(id: other))
    }

    // MARK: Retention

    func testPurgeKeepsRecentRejectsAndDropsOldOnes() throws {
        let queue = try store()
        let id = try queue.enqueue(sample("p1"), owner: alice)
        try queue.markRejected(id: id, reason: "nope")

        // Just-parked rows survive a 30-day purge.
        XCTAssertEqual(try queue.purgeRejected(olderThanDays: 30), 0)
        XCTAssertEqual(try queue.rejectedCount(owner: alice), 1)

        // Back-date it and it is collected.
        try backdate(queue, id, days: 60)
        XCTAssertEqual(try queue.purgeRejected(olderThanDays: 30), 1)
        XCTAssertEqual(try queue.rejectedCount(owner: alice), 0)
    }

    func testAnotherAccountsRowsWaitThenExpire() throws {
        let queue = try store()
        let bobRecent = try queue.enqueue(sample("bob-recent"), owner: bob)
        let bobOld = try queue.enqueue(sample("bob-old"), owner: bob)
        let aliceOld = try queue.enqueue(sample("alice-old"), owner: alice)
        try backdate(queue, bobOld, days: 20)
        try backdate(queue, aliceOld, days: 20)

        // Alice signed in: Bob's old row goes, his recent one and all of hers stay.
        XCTAssertEqual(try queue.purgeForeignPending(currentOwner: alice, olderThanDays: 14), 1)
        XCTAssertEqual(try queue.pendingCount(owner: alice), 1)
        XCTAssertEqual(try pending(queue, bob).map { $0.id }, [bobRecent])

        // No known owner (signed out, or the saved sign-in unreadable): the
        // rows may be the user's own, so nothing goes.
        XCTAssertEqual(try queue.purgeForeignPending(currentOwner: nil, olderThanDays: 14), 0)
        XCTAssertEqual(try queue.pendingCount(owner: alice), 1)
        XCTAssertEqual(try queue.pendingCount(owner: bob), 1)
        XCTAssertEqual(queue.maintain(currentOwner: nil, retentionDays: 14).purgedForeign, 0)
        XCTAssertEqual(try queue.pendingCount(owner: alice), 1)
    }

    func testMaintenanceRunsEveryHousekeepingStep() throws {
        let queue = try store()
        let parked = try queue.enqueue(sample("p1"), owner: alice)
        try queue.markRejected(id: parked, reason: "nope")
        try backdate(queue, parked, days: 30)
        let foreign = try queue.enqueue(sample("bob"), owner: bob)
        try backdate(queue, foreign, days: 30)
        for _ in 0..<3 {
            try queue.enqueue(withCaptures("p"), owner: alice)
        }
        let limits = QueueLimits(captureMaxRows: 1, captureMaxBytes: Int64.max, captureMaxAgeDays: 30, maxPendingRows: 100)
        let report = queue.maintain(currentOwner: alice, limits: limits, retentionDays: 14)
        XCTAssertEqual(report.purgedRejected, 1)
        XCTAssertEqual(report.purgedForeign, 1)
        XCTAssertEqual(report.limits, LimitReport(stripped: 2, dropped: 0))
    }

    // MARK: Migration from older databases

    /// Create a database with the schema shipped before owners existed.
    private func legacyDatabase(rows: Int) throws -> URL {
        let url = temporaryDatabase()
        let db = try SQLiteDatabase(path: url.path)
        try db.execute("""
            CREATE TABLE activities (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                payload TEXT NOT NULL,
                synced INTEGER NOT NULL DEFAULT 0,
                error TEXT NULL,
                created_at TEXT NOT NULL DEFAULT (datetime('now')));
            """)
        for _ in 0..<rows {
            try db.run("INSERT INTO activities (payload, synced) VALUES (?1, 0)", [.text(try Queue.encode(withCaptures("legacy")))])
        }
        return url
    }

    func testMigrationAssignsLegacyRowsToTheSavedAccountOnce() throws {
        let url = try legacyDatabase(rows: 2)
        do {
            let queue = try Queue(path: url.path, legacyOwner: alice)
            XCTAssertEqual(try queue.pendingCount(owner: alice), 2)
        }
        do {
            // Re-opening (even as someone else) is a no-op: the migration ran once.
            let queue = try Queue(path: url.path, legacyOwner: bob)
            XCTAssertEqual(try queue.pendingCount(owner: alice), 2)
            XCTAssertEqual(try queue.pendingCount(owner: bob), 0)
        }
    }

    func testMigrationFillsTheNewColumnsAndSwitchesToIncrementalVacuum() throws {
        let url = try legacyDatabase(rows: 1)
        let queue = try Queue(path: url.path, legacyOwner: alice)

        let rows = try queue.recentRows(owner: alice, limit: 5)
        XCTAssertEqual(rows.count, 1)
        XCTAssertEqual(rows[0].projectId, "legacy")
        XCTAssertNotNil(rows[0].startedAt)
        XCTAssertNotNil(rows[0].endedAt)
        XCTAssertTrue(rows[0].hasCaptures, "the legacy row's images were measured")
        let bytes = try queue.db.scalar("SELECT capture_bytes FROM activities")
        XCTAssertEqual(bytes, Int64("c2NyZWVu".count + "dGh1bWI=".count + "d2ViY2Ft".count))
        XCTAssertEqual(try queue.db.scalar("PRAGMA auto_vacuum"), 2, "INCREMENTAL")
    }

    func testLegacyRowsWithoutASavedAccountAreNeverUploaded() throws {
        let url = try legacyDatabase(rows: 1)
        let queue = try Queue(path: url.path, legacyOwner: nil)
        // A later sign-in must not inherit them...
        let later = try Queue(path: url.path, legacyOwner: bob)
        XCTAssertTrue(try pending(later, bob).isEmpty)
        // ...and they expire like any other foreign row.
        try queue.db.run("UPDATE activities SET created_at = datetime('now', '-20 days')")
        XCTAssertEqual(try queue.purgeForeignPending(currentOwner: bob, olderThanDays: 14), 1)
    }

    // MARK: Limits

    func testLimitsShedImagesFirstAndKeepTimeData() throws {
        let queue = try store()
        var ids: [Int64] = []
        for index in 0..<4 {
            ids.append(try queue.enqueue(withCaptures("p\(index)"), owner: alice))
        }
        try backdate(queue, ids[3], days: 40) // newest row, but too old to keep images

        let limits = QueueLimits(captureMaxRows: 2, captureMaxBytes: Int64.max, captureMaxAgeDays: 30, maxPendingRows: 100)
        XCTAssertEqual(try queue.enforceLimits(limits), LimitReport(stripped: 3, dropped: 0))

        // ids[0], ids[1]: beyond the newest two. ids[3]: too old. ids[2] keeps its images.
        for (index, id) in ids.enumerated() {
            let row = try rawPayload(queue, id)
            XCTAssertEqual(row.screenshotPngBase64 != nil, index == 2, "row \(index)")
            XCTAssertEqual(row.screenshotThumbJpgBase64 != nil, index == 2, "row \(index)")
            XCTAssertEqual(row.webcamJpgBase64 != nil, index == 2, "row \(index)")
            XCTAssertEqual(row.projectId, "p\(index)")
            XCTAssertEqual(row.productivity, 40)
        }
        XCTAssertEqual(try queue.pendingCount(owner: alice), 4)

        // Idempotent: already-stripped rows are not rewritten again.
        XCTAssertEqual(try queue.enforceLimits(limits), LimitReport())
    }

    func testImagesAreKeptNewestFirstWithinTheByteBudget() throws {
        let queue = try store()
        // Oldest to newest: 400 + 300 + 500 + 200 bytes of images, one row without.
        var ids: [Int64] = []
        for (index, bytes) in [400, 300, 500, 0, 200].enumerated() {
            let row = bytes == 0 ? sample("p\(index)") : withImageBytes("p\(index)", bytes)
            ids.append(try queue.enqueue(row, owner: bob))
        }

        let limits = QueueLimits(captureMaxRows: 100, captureMaxBytes: 1_000, captureMaxAgeDays: 30, maxPendingRows: 100)
        // Newest first: 200 (200), 0 (200), 500 (700), 300 (1000), 400 (1400 > 1000).
        XCTAssertEqual(try queue.enforceLimits(limits), LimitReport(stripped: 1, dropped: 0))
        XCTAssertFalse(try rawPayload(queue, ids[0]).hasCaptures, "the oldest goes over the budget")
        for id in ids[1...] {
            XCTAssertEqual(try rawPayload(queue, id).screenshotPngBase64 != nil, id != ids[3])
        }

        // A tighter budget sheds the next-oldest too, never a newer row first.
        var tighter = limits
        tighter.captureMaxBytes = 700
        XCTAssertEqual(try queue.enforceLimits(tighter).stripped, 1)
        XCTAssertFalse(try rawPayload(queue, ids[1]).hasCaptures)
        XCTAssertTrue(try rawPayload(queue, ids[2]).hasCaptures)
        XCTAssertTrue(try rawPayload(queue, ids[4]).hasCaptures)
    }

    func testKeptImagesNeverExceedTheByteBudget() throws {
        var generator = SplitMix(seed: 23)
        for _ in 0..<10 {
            let queue = try store()
            let sizes = (0..<Int.random(in: 1..<20, using: &generator)).map { _ in Int.random(in: 0..<400, using: &generator) }
            let budget = Int64.random(in: 0..<2_000, using: &generator)
            var ids: [Int64] = []
            for size in sizes {
                ids.append(try queue.enqueue(size == 0 ? sample("p") : withImageBytes("p", size), owner: alice))
            }
            try queue.enforceLimits(QueueLimits(captureMaxRows: 1_000, captureMaxBytes: budget, captureMaxAgeDays: 30,
                                                maxPendingRows: 1_000))
            let kept = try ids.map { try rawPayload(queue, $0).hasCaptures }
            let keptBytes = zip(sizes, kept).filter { $0.1 }.reduce(Int64(0)) { $0 + Int64($1.0) }
            XCTAssertLessThanOrEqual(keptBytes, budget)
            // Kept rows are a newest-first run: once a row with images is shed,
            // every older one is too.
            var shedSeen = false
            for (size, isKept) in zip(sizes, kept).reversed() {
                if size > 0 && !isKept {
                    shedSeen = true
                }
                XCTAssertFalse(shedSeen && isKept)
            }
        }
    }

    func testTheDefaultBudgetIs300MBWithinTheNewest500Rows() {
        XCTAssertEqual(QueueLimits.standard.captureMaxBytes, 300 * 1024 * 1024)
        XCTAssertEqual(QueueLimits.standard.captureMaxRows, 500)
        XCTAssertEqual(QueueLimits.standard.captureMaxAgeDays, 30)
        XCTAssertEqual(QueueLimits.standard.maxPendingRows, 10_000)
    }

    func testLimitsDropTheOldestRowsBeyondTheHardCap() throws {
        let queue = try store()
        for index in 0..<5 {
            try queue.enqueue(sample("p\(index)"), owner: alice)
        }
        let parked = try queue.enqueue(sample("parked"), owner: alice)
        try queue.markRejected(id: parked, reason: "nope")

        let limits = QueueLimits(captureMaxRows: 10, captureMaxBytes: Int64.max, captureMaxAgeDays: 30, maxPendingRows: 3)
        XCTAssertEqual(try queue.enforceLimits(limits).dropped, 2)
        XCTAssertEqual(try pending(queue, alice).map { $0.payload.projectId }, ["p2", "p3", "p4"])
        // Parked rows follow their own retention, not the pending cap.
        XCTAssertEqual(try queue.rejectedCount(owner: alice), 1)
    }

    // MARK: The file

    func testSpaceFreedByUploadsGoesBackToTheDisk() throws {
        let url = temporaryDatabase()
        let queue = try Queue(path: url.path, legacyOwner: nil)
        var ids: [Int64] = []
        for index in 0..<40 {
            ids.append(try queue.enqueue(withImageBytes("p\(index)", 64 * 1024), owner: alice))
        }
        let size = { () -> Int in
            ((try? FileManager.default.attributesOfItem(atPath: url.path)[.size]) as? NSNumber)?.intValue ?? 0
        }
        let full = size()
        for id in ids {
            try queue.removeSynced(id: id)
        }
        try queue.reclaimSpace()
        XCTAssertLessThan(size() * 4, full, "the file shrank from \(full) to \(size())")
    }

    func testADamagedQueueIsMovedAsideAndReplaced() throws {
        let url = temporaryDatabase()
        // Several pages of something that is not an SQLite database.
        try Data(String(repeating: "not a database. ", count: 1024).utf8).write(to: url)
        try Data("stale journal".utf8).write(to: URL(fileURLWithPath: url.path + "-journal"))

        let now = utc("2026-10-05T10:00:00Z")
        let queue = try Queue.open(at: url, legacyOwner: alice, now: now)
        let aside = try XCTUnwrap(queue.movedAside)
        files.append(aside)
        XCTAssertEqual(aside.lastPathComponent, url.lastPathComponent + ".corrupt-20261005T100000Z")
        XCTAssertEqual(try Data(contentsOf: aside).prefix(15), Data("not a database.".utf8), "the damaged file is kept")
        // SQLite itself rolls a junk journal back (and deletes it) before it
        // finds the file is not a database; either way none is left next to
        // the new queue.
        XCTAssertFalse(FileManager.default.fileExists(atPath: url.path + "-journal"),
                       "no stale journal next to the new queue")

        // The fresh queue works.
        try queue.enqueue(sample("p1"), owner: alice)
        XCTAssertEqual(try queue.pendingCount(owner: alice), 1)

        // A healthy queue is opened as it is.
        let reopened = try Queue.open(at: url, legacyOwner: alice, now: now)
        XCTAssertNil(reopened.movedAside)
        XCTAssertEqual(try reopened.pendingCount(owner: alice), 1)
    }

    func testSettingAsideMovesTheSidecarsWithTheDatabase() throws {
        let url = temporaryDatabase()
        try Data("damaged".utf8).write(to: url)
        try Data("journal".utf8).write(to: URL(fileURLWithPath: url.path + "-journal"))
        try Data("wal".utf8).write(to: URL(fileURLWithPath: url.path + "-wal"))
        let now = utc("2026-10-05T10:00:00Z")

        let aside = try Queue.setAside(databaseAt: url, now: now)
        files.append(aside)
        for suffix in ["", "-journal", "-wal"] {
            XCTAssertFalse(FileManager.default.fileExists(atPath: url.path + suffix), "\(suffix) left behind")
        }
        // Paired under the new name, so SQLite would still match them.
        XCTAssertEqual(try Data(contentsOf: URL(fileURLWithPath: aside.path + "-journal")), Data("journal".utf8))
        XCTAssertEqual(try Data(contentsOf: URL(fileURLWithPath: aside.path + "-wal")), Data("wal".utf8))

        // A second damaged file in the same second never overwrites the first.
        try Data("damaged again".utf8).write(to: url)
        let second = try Queue.setAside(databaseAt: url, now: now)
        files.append(second)
        XCTAssertEqual(second.lastPathComponent, aside.lastPathComponent + "-1")
        XCTAssertEqual(try Data(contentsOf: aside), Data("damaged".utf8))
    }

    func testAQueueThatCannotBeOpenedForOtherReasonsIsLeftAlone() throws {
        // A directory where the file should be: not damage, so nothing is renamed.
        let directory = temporaryDatabase()
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: directory) }
        XCTAssertThrowsError(try Queue.open(at: directory, legacyOwner: nil)) { error in
            XCTAssertFalse((error as? QueueError)?.isDamage ?? false)
        }
        var isDirectory: ObjCBool = false
        XCTAssertTrue(FileManager.default.fileExists(atPath: directory.path, isDirectory: &isDirectory))
        XCTAssertTrue(isDirectory.boolValue)
    }

    func testTheQueueIsSafeFromSeveralThreads() throws {
        let queue = try store()
        DispatchQueue.concurrentPerform(iterations: 4) { worker in
            for index in 0..<25 {
                _ = try? queue.enqueue(self.sample("w\(worker)-\(index)"), owner: self.alice)
                _ = try? queue.pendingCount(owner: self.alice)
            }
        }
        XCTAssertEqual(try queue.pendingCount(owner: alice), 100)
    }
}
