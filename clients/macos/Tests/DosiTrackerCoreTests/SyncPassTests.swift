import XCTest
@testable import DosiTrackerCore

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
}
