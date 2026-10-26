import XCTest
import SQLite3
@testable import DosiTrackerCore

/// `SQLITE_CONSTRAINT_TRIGGER` (`SQLITE_CONSTRAINT | (7<<8)`), what a trigger's
/// `RAISE(ABORT, …)` fails a statement with. Swift does not import SQLite's
/// composite extended-result-code macros, only the primary codes.
private let sqliteConstraintTrigger: Int32 = SQLITE_CONSTRAINT | (7 << 8)

/// One upload pass (port of the Windows worker's `sync_pending`, with the
/// CONTRACT §1 handling of each answer), against a real queue file.
final class SyncPassTests: XCTestCase {
    private let owner = "acme\u{1F}alice@acme.com"
    private var url: URL!
    private var queue: Queue!

    override func setUpWithError() throws {
        url = temporaryFileURL("dosi-sync")
        queue = try Queue(path: url.path, legacyOwner: nil)
    }

    override func tearDown() {
        queue = nil
        removeDatabase(at: url)
        super.tearDown()
    }

    private func payload(_ project: String, captures: Bool = false) -> ActivityPayload {
        var payload = ActivityPayload(projectId: project, startedAt: at(t0), endedAt: at(t0 + 600), productivity: 50)
        if captures {
            payload.screenshotPngBase64 = "c2NyZWVu"
            payload.screenshotThumbJpgBase64 = "dGh1bWI="
        }
        return payload
    }

    @discardableResult
    private func enqueue(_ projects: [String], captures: Bool = false) throws -> [Int64] {
        try projects.map { try queue.enqueue(payload($0, captures: captures), owner: owner) }
    }

    private func http(_ status: Int, _ code: String? = nil, _ message: String = "refused") -> UploadResponse {
        .failed(.http(HTTPFailure(status: status, error: code.map { AbpError(code: $0, message: message) })))
    }

    /// Answers uploads from a script keyed by project id; records what was sent.
    private final class FakeServer {
        var answers: [String: [UploadResponse]] = [:]
        var sent: [ActivityPayload] = []
        var verified: [String] = []
        var verdict: ProjectVerdict = .gone

        func upload(_ payload: ActivityPayload) -> UploadResponse {
            sent.append(payload)
            guard var queued = answers[payload.projectId], !queued.isEmpty else { return .accepted(status: 200) }
            let answer = queued.removeFirst()
            answers[payload.projectId] = queued
            return answer
        }

        func verify(_ projectId: String) -> ProjectVerdict {
            verified.append(projectId)
            return verdict
        }
    }

    private func run(_ server: FakeServer, maxUploads: Int = 100, batchSize: Int = 20) async -> SyncReport {
        await SyncPass.run(queue: queue, owner: owner, maxUploads: maxUploads, batchSize: batchSize,
                           upload: { server.upload($0) }, verifyProject: { server.verify($0) })
    }

    func testAcceptedBlocksLeaveTheQueueOldestFirst() async throws {
        let ids = try enqueue(["p1", "p2", "p3"])
        try queue.enqueue(payload("bob's"), owner: "acme\u{1F}bob@acme.com")
        let server = FakeServer()
        let report = await run(server)
        XCTAssertEqual(report.uploaded, ids)
        XCTAssertEqual(server.sent.map { $0.projectId }, ["p1", "p2", "p3"], "never another account's rows")
        XCTAssertTrue(report.drained)
        XCTAssertNil(report.stop)
        XCTAssertEqual(try queue.pendingCount(owner: owner), 0)
    }

    func testATransientFailureKeepsTheRowNotesWhyAndStopsThePass() async throws {
        let ids = try enqueue(["p1", "p2"])
        let server = FakeServer()
        server.answers["p1"] = [.failed(.transport(.timedOut))]
        let report = await run(server)
        XCTAssertEqual(report.stop, .retry(RetryPlan(reason: .offline)))
        XCTAssertTrue(report.uploaded.isEmpty)
        XCTAssertEqual(server.sent.count, 1, "the rest waits for the retry")
        XCTAssertEqual(try queue.pendingCount(owner: owner), 2)
        let rows = try queue.recentRows(owner: owner, limit: 5)
        XCTAssertEqual(rows.first { $0.id == ids[0] }?.reason, RetryReason.offline.userMessage)
    }

    func testADefinitiveRejectionParksTheRowAndMovesOn() async throws {
        let ids = try enqueue(["p1", "p2"], captures: true)
        let server = FakeServer()
        server.answers["p1"] = [http(409, UploadPolicy.Code.activityOverlaps, "Overlaps another block.")]
        let report = await run(server)
        XCTAssertEqual(report.parked, [ParkedRow(id: ids[0], projectId: "p1", reason: "Overlaps another block.")])
        XCTAssertEqual(report.uploaded, [ids[1]])
        XCTAssertEqual(try queue.rejectedCount(owner: owner), 1)
        XCTAssertEqual(try queue.recentRows(owner: owner, limit: 5).first?.reason, "Overlaps another block.")
    }

    func testCapturesTheServerRefusesAreStrippedAndResentAtOnce() async throws {
        let ids = try enqueue(["p1"], captures: true)
        let server = FakeServer()
        server.answers["p1"] = [http(413)]
        let report = await run(server)
        XCTAssertEqual(report.stripped, ids)
        XCTAssertEqual(report.uploaded, ids)
        XCTAssertEqual(server.sent.count, 2)
        XCTAssertTrue(server.sent[0].hasCaptures)
        XCTAssertFalse(server.sent[1].hasCaptures, "the time data goes up without the images")
        XCTAssertEqual(server.sent[1].clientActivityId, server.sent[0].clientActivityId, "the same block")
        XCTAssertEqual(try queue.pendingCount(owner: owner), 0)
    }

    func testARefusalWithoutImagesIsKeptForLater() async throws {
        try enqueue(["p1"], captures: true)
        let server = FakeServer()
        server.answers["p1"] = [http(413), http(413)]
        let report = await run(server)
        XCTAssertEqual(report.stop, .retry(RetryPlan(reason: .unexpectedResponse(status: 413))))
        XCTAssertEqual(try queue.pendingCount(owner: owner), 1, "the time data is never lost")
    }

    func testAProjectThatIsGoneParksItsRowsAfterOneCheck() async throws {
        let ids = try enqueue(["gone", "gone", "kept"])
        let server = FakeServer()
        server.answers["gone"] = [http(409, UploadPolicy.Code.projectArchived, "The project is archived."),
                                  http(409, UploadPolicy.Code.projectArchived, "The project is archived.")]
        server.verdict = .gone
        let report = await run(server)
        XCTAssertEqual(server.verified, ["gone"], "the project list is refreshed once per pass")
        XCTAssertEqual(report.parked.map { $0.id }, [ids[0], ids[1]])
        XCTAssertEqual(report.parked.first?.reason, "The project is archived.")
        XCTAssertEqual(report.goneProjects, ["gone"])
        XCTAssertEqual(report.uploaded, [ids[2]])
    }

    func testAProjectThatIsStillListedKeepsItsRows() async throws {
        try enqueue(["p1", "p2"])
        for verdict in [ProjectVerdict.available, .unknown] {
            let server = FakeServer()
            server.answers["p1"] = [http(403, UploadPolicy.Code.notProjectMember, "Not a member")]
            server.verdict = verdict
            let report = await run(server)
            XCTAssertEqual(report.stop, .retry(RetryPlan(reason: .projectUnavailable)), "\(verdict)")
            XCTAssertTrue(report.parked.isEmpty)
            XCTAssertTrue(report.goneProjects.isEmpty)
            XCTAssertEqual(try queue.pendingCount(owner: owner), 2)
        }
    }

    func testRefusedCredentialsStopThePassWithSignInExpired() async throws {
        try enqueue(["p1", "p2"])
        let rejection = AuthRejection(status: 400, error: "invalid_grant", errorDescription: "Invalid username or password!")
        let server = FakeServer()
        server.answers["p1"] = [.failed(.authRejected(rejection))]
        let report = await run(server)
        XCTAssertEqual(report.stop, .signInExpired(rejection))
        XCTAssertEqual(server.sent.count, 1)
        XCTAssertEqual(try queue.pendingCount(owner: owner), 2, "the queue is kept for the next sign-in")
    }

    func testARevokedDeviceOrALostWorkspaceStopsThePassAndKeepsEveryRow() async throws {
        for (status, code) in [(403, UploadPolicy.Code.deviceRevoked), (403, UploadPolicy.Code.notWorkspaceMember),
                               (403, UploadPolicy.Code.workspaceArchived), (400, UploadPolicy.Code.workspaceRequired)] {
            try queue.reassignOwner(from: owner, to: "done")
            let ids = try enqueue(["p1", "p2"])
            let server = FakeServer()
            server.answers["p1"] = [http(status, code, "No access.")]
            let report = await run(server)
            guard case .some(.accessLost(let loss)) = report.stop else {
                return XCTFail("\(code): expected an access loss, got \(String(describing: report.stop))")
            }
            XCTAssertEqual(loss.message, "No access.")
            XCTAssertEqual(server.sent.count, 1, "\(code): nothing more is sent")
            XCTAssertTrue(report.parked.isEmpty, "\(code): never parked as rejected")
            XCTAssertEqual(try queue.pendingCount(owner: owner), 2, "\(code): the rows wait")
            XCTAssertEqual(try queue.recentRows(owner: owner, limit: 5).first { $0.id == ids[0] }?.reason, "No access.")
        }
    }

    func testAPassIsBoundedAndReadsInBatches() async throws {
        try enqueue((0..<7).map { "p\($0)" })
        let server = FakeServer()
        let report = await run(server, maxUploads: 5, batchSize: 2)
        XCTAssertEqual(report.attempted, 5)
        XCTAssertEqual(report.uploaded.count, 5)
        XCTAssertFalse(report.drained)
        XCTAssertNil(report.stop)
        XCTAssertEqual(try queue.pendingCount(owner: owner), 2, "drained over the next pass")

        let next = await run(server, maxUploads: 5, batchSize: 2)
        XCTAssertEqual(next.uploaded.count, 2)
        XCTAssertTrue(next.drained)
    }

    func testAnEmptyQueueIsDrainedAtOnce() async {
        let report = await run(FakeServer())
        XCTAssertTrue(report.drained)
        XCTAssertEqual(report.attempted, 0)
    }

    func testRecentBlocksFollowThePass() async throws {
        let ids = try enqueue(["p1", "p2"])
        var recent = RecentBlocks()
        for (id, project) in zip(ids, ["p1", "p2"]) {
            recent.push(RecentBlock(rowId: id, projectTitle: project, payload: payload(project)))
        }
        let server = FakeServer()
        server.answers["p2"] = [http(422, UploadPolicy.Code.activityTooLong, "Too long")]
        recent.apply(await run(server))
        XCTAssertEqual(recent.items.first { $0.rowId == ids[0] }?.upload, .uploaded)
        XCTAssertEqual(recent.items.first { $0.rowId == ids[1] }?.upload, .rejected)
        XCTAssertEqual(recent.items.first { $0.rowId == ids[1] }?.message, "Too long")
    }

    // MARK: - Persistence Failure Regressions

    func testAcceptedResponseRemoveFailureHaltsPassRetainsQueuedRowWithoutReportingUploadedAndRecovers() async throws {
        let otherOwner = "acme\u{1F}bob@acme.com"
        let ids = try enqueue(["p1", "p2", "p3"])
        let bobSentinel = try queue.enqueue(payload("bob-project"), owner: otherOwner)

        var recent = RecentBlocks()
        for (id, project) in zip(ids, ["p1", "p2", "p3"]) {
            recent.push(RecentBlock(rowId: id, projectTitle: project, payload: payload(project)))
        }

        // Deterministic isolated SQLite failure injection: distinctive private/long/control-character trigger text
        try queue.db.execute("CREATE TRIGGER fail_delete BEFORE DELETE ON activities BEGIN SELECT RAISE(ABORT, 'CONFIDENTIAL_DB_PATH=/Users/victim/Library/dosi.db\nUSER_SECRET=tok_1234567890\t\u{1F}--unbounded-control-payload-message-padding-XXXX'); END;")

        let server = FakeServer()
        let report = await run(server)

        // Truthful outcome reporting:
        // Even though the server accepted p1, local removeSynced failed, so p1 MUST NOT be reported uploaded
        XCTAssertTrue(report.uploaded.isEmpty, "Row that failed local queue removal must not be reported as uploaded")
        XCTAssertFalse(report.drained, "Pass must not report drained when queue removal fails")
        if case .some(.queueError(let message)) = report.stop {
            XCTAssertEqual(message, "queue_remove_failed (sqlite code \(sqliteConstraintTrigger))", "Stop reason must match finite sanitized category and code")
            XCTAssertLessThanOrEqual(message.count, 64, "Error message must be short and bounded")
            XCTAssertFalse(message.contains("CONFIDENTIAL_DB_PATH"), "Raw SQL/trigger text must never leak into report")
            XCTAssertFalse(message.contains("/Users/victim"), "Database paths must never leak into report")
            XCTAssertFalse(message.contains("tok_1234567890"), "Secrets/payloads must never leak into report")
            XCTAssertFalse(message.contains("\n"), "Newlines must never appear in report")
            XCTAssertFalse(message.contains("\t"), "Tabs must never appear in report")
            XCTAssertFalse(message.contains("\u{1F}"), "Control characters must never appear in report")
        } else {
            XCTFail("Expected .queueError stop reason, got \(String(describing: report.stop))")
        }

        // Halt on fatal mutation failure: subsequent rows must NOT be uploaded
        XCTAssertEqual(server.sent.count, 1, "Must halt immediately; p2 and p3 must not be uploaded after local mutation failure")
        XCTAssertEqual(server.sent[0].projectId, "p1")

        // Preserved pending payload and clientActivityId
        XCTAssertEqual(try queue.pendingCount(owner: owner), 3, "All 3 rows must remain pending in the queue")
        let pending = try queue.pendingBatch(owner: owner, afterId: 0, limit: 1)
        XCTAssertEqual(pending.count, 1)
        XCTAssertEqual(pending[0].id, ids[0])
        XCTAssertEqual(pending[0].payload.clientActivityId, server.sent[0].clientActivityId, "clientActivityId must be preserved for idempotent retry")
        XCTAssertEqual(pending[0].payload.projectId, "p1")

        // No false RecentBlocks uploaded transition
        recent.apply(report)
        for id in ids {
            XCTAssertEqual(recent.items.first { $0.rowId == id }?.upload, .queued, "Recent blocks must remain .queued on mutation failure")
        }

        // Other-owner sentinel intact
        XCTAssertEqual(try queue.pendingCount(owner: otherOwner), 1)
        let bobPending = try queue.pendingBatch(owner: otherOwner, afterId: 0, limit: 5)
        XCTAssertEqual(bobPending.map { $0.id }, [bobSentinel])

        // Recovery: remove injected trigger, verify queue drains successfully with idempotent server retry
        try queue.db.execute("DROP TRIGGER fail_delete;")
        let recoveryReport = await run(server)
        XCTAssertEqual(recoveryReport.uploaded, ids, "All rows must upload and leave the queue after failure is resolved")
        XCTAssertTrue(recoveryReport.drained)
        XCTAssertNil(recoveryReport.stop)
        XCTAssertEqual(try queue.pendingCount(owner: owner), 0)
        XCTAssertEqual(try queue.pendingCount(owner: otherOwner), 1, "Other-owner sentinel preserved")

        // Sequence verification: 1 initial attempt for p1 + 3 recovery uploads (p1, p2, p3)
        XCTAssertEqual(server.sent.count, 4, "Total uploads: 1 initial failed remove + 3 recovery uploads")
        XCTAssertEqual(server.sent[0].clientActivityId, server.sent[1].clientActivityId, "clientActivityId must match on idempotent retry")
        XCTAssertEqual(server.sent[2].projectId, "p2")
        XCTAssertEqual(server.sent[3].projectId, "p3")

        recent.apply(recoveryReport)
        for id in ids {
            XCTAssertEqual(recent.items.first { $0.rowId == id }?.upload, .uploaded)
        }
    }

    func testRejectionParkFailureHaltsPassRetainsQueuedRowWithoutReportingParkedAndRecovers() async throws {
        let otherOwner = "acme\u{1F}bob@acme.com"
        let ids = try enqueue(["p1", "p2"])
        _ = try queue.enqueue(payload("bob-project"), owner: otherOwner)

        var recent = RecentBlocks()
        for (id, project) in zip(ids, ["p1", "p2"]) {
            recent.push(RecentBlock(rowId: id, projectTitle: project, payload: payload(project)))
        }

        let server = FakeServer()
        server.answers["p1"] = [http(409, UploadPolicy.Code.activityOverlaps, "Overlaps another block.")]

        // Deterministic isolated SQLite failure injection: distinctive private/long/control-character trigger text
        try queue.db.execute("CREATE TRIGGER fail_park BEFORE UPDATE OF synced ON activities BEGIN SELECT RAISE(ABORT, 'SECRET_PARK_LEAK=/var/db/app.db\nCREDENTIAL_BEARER=secret_jwt_xyz\t\u{1F}--unbounded-park-padding-XXXX'); END;")

        let report = await run(server)

        // Truthful outcome reporting:
        // Even though the server rejected p1, local markRejected failed, so p1 MUST NOT be reported parked
        XCTAssertTrue(report.parked.isEmpty, "Row that failed local queue parking must not be reported as parked")
        XCTAssertTrue(report.uploaded.isEmpty)
        XCTAssertFalse(report.drained)
        if case .some(.queueError(let message)) = report.stop {
            XCTAssertEqual(message, "queue_park_failed (sqlite code \(sqliteConstraintTrigger))", "Stop reason must match finite sanitized category and code")
            XCTAssertLessThanOrEqual(message.count, 64)
            XCTAssertFalse(message.contains("SECRET_PARK_LEAK"), "Raw trigger text must never leak into report")
            XCTAssertFalse(message.contains("secret_jwt_xyz"), "Secrets must never leak into report")
            XCTAssertFalse(message.contains("\n"))
            XCTAssertFalse(message.contains("\t"))
            XCTAssertFalse(message.contains("\u{1F}"))
        } else {
            XCTFail("Expected .queueError stop reason, got \(String(describing: report.stop))")
        }

        // Halt on mutation failure: p2 must NOT be uploaded
        XCTAssertEqual(server.sent.count, 1, "Must halt immediately; p2 must not be uploaded after local parking failure")

        // Queued data retained as pending, not marked rejected
        XCTAssertEqual(try queue.pendingCount(owner: owner), 2)
        XCTAssertEqual(try queue.rejectedCount(owner: owner), 0, "Row must not be marked rejected when write fails")
        let pending = try queue.pendingBatch(owner: owner, afterId: 0, limit: 1)
        XCTAssertEqual(pending[0].payload.clientActivityId, server.sent[0].clientActivityId)

        // No false RecentBlocks rejected transition
        recent.apply(report)
        XCTAssertEqual(recent.items.first { $0.rowId == ids[0] }?.upload, .queued, "Recent block must not be marked rejected when write fails")
        XCTAssertEqual(recent.items.first { $0.rowId == ids[1] }?.upload, .queued)

        // Other-owner sentinel intact
        XCTAssertEqual(try queue.pendingCount(owner: otherOwner), 1)

        // Recovery: remove injected trigger, explicitly rearm server response script for p1, and run pass
        try queue.db.execute("DROP TRIGGER fail_park;")
        server.answers["p1"] = [http(409, UploadPolicy.Code.activityOverlaps, "Overlaps another block.")]
        let recoveryReport = await run(server)

        // Sequence verification: 1 initial attempt for p1 + 1 recovery park for p1 + 1 recovery upload for p2
        XCTAssertEqual(server.sent.count, 3, "Total uploads: 1 initial failed park + 1 recovery park + 1 recovery upload of p2")
        XCTAssertEqual(server.sent[0].clientActivityId, server.sent[1].clientActivityId, "clientActivityId must be preserved on retry")
        XCTAssertEqual(server.sent[2].projectId, "p2")

        XCTAssertEqual(recoveryReport.parked.map { $0.id }, [ids[0]])
        XCTAssertEqual(recoveryReport.parked.first?.reason, "Overlaps another block.")
        XCTAssertEqual(recoveryReport.uploaded, [ids[1]])
        XCTAssertTrue(recoveryReport.drained)
        XCTAssertNil(recoveryReport.stop)
        XCTAssertEqual(try queue.rejectedCount(owner: owner), 1)
        XCTAssertEqual(try queue.pendingCount(owner: owner), 0)
        XCTAssertEqual(try queue.pendingCount(owner: otherOwner), 1)

        recent.apply(recoveryReport)
        XCTAssertEqual(recent.items.first { $0.rowId == ids[0] }?.upload, .rejected)
        XCTAssertEqual(recent.items.first { $0.rowId == ids[1] }?.upload, .uploaded)
    }

    func testStripCapturesMutationFailureHaltsPassDoesNotReportStrippedAndRecovers() async throws {
        let otherOwner = "acme\u{1F}bob@acme.com"
        let ids = try enqueue(["p1", "p2"], captures: true)
        _ = try queue.enqueue(payload("bob-project"), owner: otherOwner)

        let server = FakeServer()
        server.answers["p1"] = [http(413)]

        // Deterministic isolated SQLite failure injection: distinctive private/long/control-character trigger text
        try queue.db.execute("CREATE TRIGGER fail_strip BEFORE UPDATE OF stripped ON activities BEGIN SELECT RAISE(ABORT, 'SECRET_STRIP_LEAK=/private/var/queue.db\nPAYLOAD_KEY=capture_token_abc\t\u{1F}--unbounded-strip-padding-XXXX'); END;")

        let report = await run(server)

        // Truthful outcome reporting:
        // Local strip mutation failed, so p1 MUST NOT be reported stripped
        XCTAssertTrue(report.stripped.isEmpty, "Row that failed local capture stripping must not be reported as stripped")
        XCTAssertTrue(report.uploaded.isEmpty)
        XCTAssertFalse(report.drained)
        if case .some(.queueError(let message)) = report.stop {
            XCTAssertEqual(message, "queue_strip_failed (sqlite code \(sqliteConstraintTrigger))", "Stop reason must match finite sanitized category and code")
            XCTAssertLessThanOrEqual(message.count, 64)
            XCTAssertFalse(message.contains("SECRET_STRIP_LEAK"), "Raw trigger text must never leak into report")
            XCTAssertFalse(message.contains("capture_token_abc"), "Payload keys must never leak into report")
            XCTAssertFalse(message.contains("\n"))
            XCTAssertFalse(message.contains("\t"))
            XCTAssertFalse(message.contains("\u{1F}"))
        } else {
            XCTFail("Expected .queueError stop reason, got \(String(describing: report.stop))")
        }

        // Resend must NOT be attempted because local strip mutation failed; p2 must not be attempted
        XCTAssertEqual(server.sent.count, 1, "Must not resend payload without images when local strip mutation fails")

        // Captures are still intact on the queued row
        XCTAssertEqual(try queue.pendingCount(owner: owner), 2)
        let pending = try queue.pendingBatch(owner: owner, afterId: 0, limit: 1)
        XCTAssertTrue(pending[0].payload.hasCaptures, "Captures must remain intact in local queue when stripping fails")
        XCTAssertEqual(pending[0].payload.clientActivityId, server.sent[0].clientActivityId)

        // Other-owner sentinel intact
        XCTAssertEqual(try queue.pendingCount(owner: otherOwner), 1)

        // Recovery: remove injected trigger, explicitly rearm 413 for p1, and run pass
        try queue.db.execute("DROP TRIGGER fail_strip;")
        server.answers["p1"] = [http(413)]
        let recoveryReport = await run(server)

        // Sequence verification:
        // 1 initial (with captures) + 1 recovery initial (with captures) + 1 recovery resend (stripped) + 1 recovery p2
        XCTAssertEqual(server.sent.count, 4, "Total uploads: 1 initial (with captures) + 1 recovery initial (with captures) + 1 recovery resend (stripped) + 1 recovery p2")
        XCTAssertTrue(server.sent[0].hasCaptures)
        XCTAssertTrue(server.sent[1].hasCaptures)
        XCTAssertFalse(server.sent[2].hasCaptures, "Recovery resend must strip captures")
        XCTAssertEqual(server.sent[0].clientActivityId, server.sent[1].clientActivityId)
        XCTAssertEqual(server.sent[1].clientActivityId, server.sent[2].clientActivityId, "Resent payload must preserve clientActivityId")
        XCTAssertEqual(server.sent[3].projectId, "p2")

        XCTAssertEqual(recoveryReport.stripped, [ids[0]])
        XCTAssertEqual(recoveryReport.uploaded, ids)
        XCTAssertTrue(recoveryReport.drained)
        XCTAssertNil(recoveryReport.stop)
        XCTAssertEqual(try queue.pendingCount(owner: owner), 0)
        XCTAssertEqual(try queue.pendingCount(owner: otherOwner), 1)
    }

    func testProjectGonePolicyRecordedEvenWhenParkingPersistenceFails() async throws {
        let otherOwner = "acme\u{1F}bob@acme.com"
        let ids = try enqueue(["gone", "p2"])
        _ = try queue.enqueue(payload("bob-project"), owner: otherOwner)

        let server = FakeServer()
        server.answers["gone"] = [http(409, UploadPolicy.Code.projectArchived, "The project is archived.")]
        server.verdict = .gone

        // Deterministic isolated SQLite failure injection: distinctive private/long/control-character trigger text
        try queue.db.execute("CREATE TRIGGER fail_park BEFORE UPDATE OF synced ON activities BEGIN SELECT RAISE(ABORT, 'SECRET_GONE_LEAK=/path/to/db\nGONE_TOKEN=xyz\t\u{1F}--unbounded-gone-padding-XXXX'); END;")

        let report = await run(server)

        // Project-gone policy is separate from whether parking write persisted:
        // The server verdict holds, so goneProjects records the gone project
        XCTAssertEqual(report.goneProjects, ["gone"], "Server project-gone verdict must be recorded regardless of parking persistence")
        XCTAssertTrue(report.parked.isEmpty, "Parking write failed, so row must not be reported as parked")
        XCTAssertTrue(report.uploaded.isEmpty, "Subsequent row must not be uploaded after local mutation failure")
        if case .some(.queueError(let message)) = report.stop {
            XCTAssertEqual(message, "queue_park_failed (sqlite code \(sqliteConstraintTrigger))", "Stop reason must match finite sanitized category and code")
            XCTAssertLessThanOrEqual(message.count, 64)
            XCTAssertFalse(message.contains("SECRET_GONE_LEAK"))
            XCTAssertFalse(message.contains("GONE_TOKEN"))
            XCTAssertFalse(message.contains("\n"))
            XCTAssertFalse(message.contains("\t"))
            XCTAssertFalse(message.contains("\u{1F}"))
        } else {
            XCTFail("Expected .queueError stop reason, got \(String(describing: report.stop))")
        }

        XCTAssertEqual(try queue.pendingCount(owner: owner), 2)
        XCTAssertEqual(try queue.pendingCount(owner: otherOwner), 1)

        // Recovery: remove injected trigger, explicitly rearm 409 archived for gone, and run pass
        try queue.db.execute("DROP TRIGGER fail_park;")
        server.answers["gone"] = [http(409, UploadPolicy.Code.projectArchived, "The project is archived.")]
        let recoveryReport = await run(server)

        // Sequence verification: 1 initial gone + 1 recovery gone + 1 recovery p2
        XCTAssertEqual(server.sent.count, 3, "Total uploads: 1 initial gone + 1 recovery gone + 1 recovery p2")
        XCTAssertEqual(server.sent[0].clientActivityId, server.sent[1].clientActivityId)
        XCTAssertEqual(server.sent[2].projectId, "p2")

        XCTAssertEqual(recoveryReport.goneProjects, ["gone"])
        XCTAssertEqual(recoveryReport.parked.map { $0.id }, [ids[0]])
        XCTAssertEqual(recoveryReport.parked.first?.reason, "The project is archived.")
        XCTAssertEqual(recoveryReport.uploaded, [ids[1]])
        XCTAssertTrue(recoveryReport.drained)
        XCTAssertNil(recoveryReport.stop)
        XCTAssertEqual(try queue.rejectedCount(owner: owner), 1)
        XCTAssertEqual(try queue.pendingCount(owner: owner), 0)
        XCTAssertEqual(try queue.pendingCount(owner: otherOwner), 1)
    }

    func testSanitizedQueueErrorProducesFiniteSafeStringAndHandlesUnknownError() {
        XCTAssertEqual(SanitizedQueueError(operation: .read, sqliteCode: 1).description, "queue_read_failed (sqlite code 1)")
        XCTAssertEqual(SanitizedQueueError(operation: .strip, sqliteCode: 13).description, "queue_strip_failed (sqlite code 13)")
        XCTAssertEqual(SanitizedQueueError(operation: .remove, sqliteCode: 1).description, "queue_remove_failed (sqlite code 1)")
        XCTAssertEqual(SanitizedQueueError(operation: .park, sqliteCode: 5).description, "queue_park_failed (sqlite code 5)")

        let sensitiveError = QueueError(code: 1, message: "RAW_SECRET_QUERY_PATH=/path/to/db\nTOKEN=xyz\t\u{1F}")
        let sanitized = SanitizedQueueError(operation: .remove, error: sensitiveError).description
        XCTAssertEqual(sanitized, "queue_remove_failed (sqlite code 1)")
        XCTAssertFalse(sanitized.contains("RAW_SECRET_QUERY_PATH"))
        XCTAssertFalse(sanitized.contains("\n"))

        let unknownError = NSError(domain: "test.unknown", code: 999, userInfo: [NSLocalizedDescriptionKey: "unbounded private text"])
        let unknownSanitized = SanitizedQueueError(operation: .read, error: unknownError).description
        XCTAssertEqual(unknownSanitized, "queue_read_failed (unknown error)")
        XCTAssertFalse(unknownSanitized.contains("unbounded private text"))
    }
}
