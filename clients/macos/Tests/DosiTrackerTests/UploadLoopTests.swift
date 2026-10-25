import DosiTrackerCore
import Foundation
import XCTest
@testable import DosiTracker

/// The uploader against a scripted server: each workspace's rows wait out
/// their own back-off, so a workspace whose subscription lapsed (or whose
/// server answers fail) never holds back the rows of another.
final class UploadLoopTests: XCTestCase {

    // MARK: - A scripted server

    /// Answers uploads by the workspace they name (`X-Workspace-Id`); grants
    /// a token on any token request. Records every upload's workspace. One
    /// test at a time uses it.
    private final class WorkspaceServer: URLProtocol {
        private static let lock = NSLock()
        private static var answers: [String: (status: Int, body: String)] = [:]
        private static var seen: [String] = []

        static func reset(_ replies: [String: (status: Int, body: String)]) {
            lock.lock()
            answers = replies
            seen = []
            lock.unlock()
        }

        /// The workspace of every upload, in order.
        static var uploads: [String] {
            lock.lock()
            defer { lock.unlock() }
            return seen
        }

        override class func canInit(with request: URLRequest) -> Bool { true }
        override class func canonicalRequest(for request: URLRequest) -> URLRequest { request }

        override func startLoading() {
            guard let url = request.url else {
                client?.urlProtocol(self, didFailWithError: URLError(.badURL))
                return
            }
            var status = 200
            var body = "{}"
            if url.path == "/connect/token" {
                body = #"{"access_token":"t0k3n","token_type":"Bearer","expires_in":3600,"refresh_token":"refresh-2"}"#
            } else {
                let workspace = request.value(forHTTPHeaderField: ApiClient.workspaceHeader) ?? ""
                Self.lock.lock()
                Self.seen.append(workspace)
                if let answer = Self.answers[workspace] {
                    status = answer.status
                    body = answer.body
                }
                Self.lock.unlock()
            }
            guard let response = HTTPURLResponse(url: url, statusCode: status, httpVersion: "HTTP/1.1",
                                                 headerFields: ["Content-Type": "application/json"])
            else {
                client?.urlProtocol(self, didFailWithError: URLError(.badServerResponse))
                return
            }
            client?.urlProtocol(self, didReceive: response, cacheStoragePolicy: .notAllowed)
            client?.urlProtocol(self, didLoad: Data(body.utf8))
            client?.urlProtocolDidFinishLoading(self)
        }

        override func stopLoading() {}
    }

    /// What the uploader told the worker.
    private final class Events: @unchecked Sendable {
        private let lock = NSLock()
        private var items: [UploadEvent] = []

        func add(_ event: UploadEvent) {
            lock.lock()
            items.append(event)
            lock.unlock()
        }

        /// The issue of every finished pass of `owner`, in order.
        func issues(of owner: String) -> [UploadIssue?] {
            lock.lock()
            defer { lock.unlock() }
            return items.compactMap { event -> UploadIssue?? in
                guard case .finished(let passOwner, _, let issue, _) = event, passOwner == owner else { return nil }
                return .some(issue)
            }
        }
    }

    // MARK: - Harness

    private static let username = "user@example.com"
    private let current = UploadScope(owner: Queue.ownerKey(workspaceId: "ws-now", username: UploadLoopTests.username),
                                      workspaceId: "ws-now")
    private let earlier = Queue.ownerKey(workspaceId: "ws-old", username: UploadLoopTests.username)

    private var root: URL!
    private var queue: Queue!
    private var session: URLSession!
    private var loop: UploadLoop!
    private var runner: Task<Void, Never>?
    private let events = Events()

    override func setUpWithError() throws {
        root = FileManager.default.temporaryDirectory
            .appendingPathComponent("upload-loop-\(UUID().uuidString.lowercased())", isDirectory: true)
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        queue = try Queue(path: root.appendingPathComponent("queue.sqlite").path, legacyOwner: nil)
        let configuration = URLSessionConfiguration.ephemeral
        configuration.protocolClasses = [WorkspaceServer.self]
        session = URLSession(configuration: configuration)
        let sink = events
        loop = UploadLoop(queue: queue, events: { sink.add($0) })
        let started = loop!
        runner = Task { await started.run() }
    }

    override func tearDown() async throws {
        await loop?.shutdown()
        await runner?.value
        session?.invalidateAndCancel()
        loop = nil
        queue = nil
        try? FileManager.default.removeItem(at: root)
        try await super.tearDown()
    }

    private func client() -> ApiClient {
        ApiClient(baseURL: URL(string: "https://tracker.example.com")!,
                  identity: ApiClient.Identity(username: Self.username, deviceId: "5b0c1b9e-8f2c-4a43-9a51-0c7f6f2d9b11",
                                               workspaceId: current.workspaceId, refreshToken: "refresh-1"),
                  session: session)
    }

    private func enqueue(owner: String) throws {
        let start = Date().addingTimeInterval(-1200)
        _ = try queue.enqueue(ActivityPayload(projectId: "p-1", startedAt: start, endedAt: start.addingTimeInterval(600),
                                              productivity: 50),
                              owner: owner)
    }

    private func pending(_ owner: String) -> Int {
        (try? queue.pendingCount(owner: owner)) ?? -1
    }

    /// Wait (at most a few seconds) until `condition` holds.
    private func eventually(_ what: String, file: StaticString = #filePath, line: UInt = #line,
                            _ condition: () -> Bool) async {
        for _ in 0..<250 {
            if condition() {
                return
            }
            try? await Task.sleep(nanoseconds: 20_000_000)
        }
        XCTFail("timed out waiting for \(what)", file: file, line: line)
    }

    // MARK: - Tests

    func testAnEarlierWorkspaceWithALapsedSubscriptionNeverHoldsBackTheCurrentOne() async throws {
        WorkspaceServer.reset([
            "ws-old": (402, #"{"error":{"code":"Tracker:SubscriptionInactive","message":"Inactive."}}"#),
        ])
        try enqueue(owner: current.owner)
        try enqueue(owner: earlier)
        loop.send(.account(client: client(), user: UploadUser(username: Self.username, current: current)))

        await eventually("the earlier workspace refused") {
            !events.issues(of: earlier).isEmpty
        }
        XCTAssertEqual(pending(current.owner), 0, "the current workspace's row went up first")
        XCTAssertEqual(pending(earlier), 1, "the earlier workspace's row waits")
        guard case .some(.some(.subscriptionInactive)) = events.issues(of: earlier).last else {
            return XCTFail("the earlier workspace waits for its subscription: \(events.issues(of: earlier))")
        }
        XCTAssertEqual(events.issues(of: current.owner).last, .some(nil), "nothing waits in the current workspace")

        // New rows in the current workspace go up at once, the hour-long wait
        // of the earlier one notwithstanding — which is not asked again.
        try enqueue(owner: current.owner)
        loop.send(.rowsQueued)
        await eventually("the new row uploaded") {
            pending(current.owner) == 0
        }
        XCTAssertEqual(WorkspaceServer.uploads.filter { $0 == "ws-now" }.count, 2)
        XCTAssertEqual(WorkspaceServer.uploads.filter { $0 == "ws-old" }.count, 1,
                       "the lapsed workspace waits out its hour")
        XCTAssertEqual(pending(earlier), 1)
    }

    func testACurrentWorkspaceThatBacksOffNeverHoldsBackAnEarlierOne() async throws {
        WorkspaceServer.reset([
            "ws-now": (503, #"{"error":{"code":"Unavailable","message":"Down."}}"#),
        ])
        try enqueue(owner: current.owner)
        try enqueue(owner: earlier)
        loop.send(.account(client: client(), user: UploadUser(username: Self.username, current: current)))

        await eventually("the earlier workspace's row uploaded") {
            pending(earlier) == 0
        }
        XCTAssertEqual(pending(current.owner), 1, "the current workspace's row waits out its back-off")
        guard case .some(.some(.retrying)) = events.issues(of: current.owner).last else {
            return XCTFail("the current workspace backs off: \(events.issues(of: current.owner))")
        }

        // Its own back-off still holds: a new row does not go up before it ends.
        try enqueue(owner: current.owner)
        loop.send(.rowsQueued)
        try await Task.sleep(nanoseconds: 300_000_000)
        XCTAssertEqual(WorkspaceServer.uploads.filter { $0 == "ws-now" }.count, 1)
        XCTAssertEqual(pending(current.owner), 2)

        // "Sync now" ends every wait.
        WorkspaceServer.reset([:])
        loop.send(.now)
        await eventually("the current workspace's rows uploaded") {
            pending(current.owner) == 0
        }
    }
}
