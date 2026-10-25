import DosiTrackerCore
import Foundation
import XCTest
@testable import DosiTracker

/// The worker against a scripted server, per the cross-agent contract: the
/// sign-in names no workspace, the workspace is chosen from
/// `/api/app/workspaces/mine` and sent as `X-Workspace-Id`, the saved sign-in
/// keeps the rotated refresh token (never the password), and a revoked device
/// or a lost workspace stops tracking there at once.
final class WorkerWorkspaceTests: XCTestCase {

    // MARK: - A scripted server

    /// Answers by path from a script (the last answer of a path repeats);
    /// records every request. One test at a time uses it.
    private final class ScriptedURLProtocol: URLProtocol {
        struct Reply {
            let status: Int
            let body: String
        }

        struct Seen {
            let path: String
            let headers: [String: String]
            let body: String

            func header(_ name: String) -> String? {
                headers.first { $0.key.caseInsensitiveCompare(name) == .orderedSame }?.value
            }

            var form: [String: String] {
                var fields: [String: String] = [:]
                for pair in body.split(separator: "&") {
                    let parts = pair.split(separator: "=", maxSplits: 1, omittingEmptySubsequences: false)
                    guard parts.count == 2 else { continue }
                    fields[String(parts[0]).removingPercentEncoding ?? ""] = String(parts[1]).removingPercentEncoding
                }
                return fields
            }
        }

        private static let lock = NSLock()
        private static var script: [String: [Reply]] = [:]
        private static var seen: [Seen] = []

        static func reset(_ replies: [String: [Reply]]) {
            lock.lock()
            script = replies
            seen = []
            lock.unlock()
        }

        static var requests: [Seen] {
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
            let path = url.path
            Self.lock.lock()
            Self.seen.append(Seen(path: path, headers: request.allHTTPHeaderFields ?? [:],
                                  body: String(decoding: Self.body(of: request), as: UTF8.self)))
            var reply: Reply?
            if var queued = Self.script[path], !queued.isEmpty {
                reply = queued.count > 1 ? queued.removeFirst() : queued[0]
                Self.script[path] = queued
            }
            Self.lock.unlock()
            guard let answer = reply,
                  let response = HTTPURLResponse(url: url, statusCode: answer.status, httpVersion: "HTTP/1.1",
                                                 headerFields: ["Content-Type": "application/json"])
            else {
                client?.urlProtocol(self, didFailWithError: URLError(.notConnectedToInternet))
                return
            }
            client?.urlProtocol(self, didReceive: response, cacheStoragePolicy: .notAllowed)
            client?.urlProtocol(self, didLoad: Data(answer.body.utf8))
            client?.urlProtocolDidFinishLoading(self)
        }

        override func stopLoading() {}

        private static func body(of request: URLRequest) -> Data {
            if let body = request.httpBody {
                return body
            }
            guard let stream = request.httpBodyStream else { return Data() }
            stream.open()
            defer { stream.close() }
            var data = Data()
            var buffer = [UInt8](repeating: 0, count: 16_384)
            while stream.hasBytesAvailable {
                let read = stream.read(&buffer, maxLength: buffer.count)
                if read <= 0 { break }
                data.append(buffer, count: read)
            }
            return data
        }
    }

    // MARK: - Fakes

    private final class NoScreen: ScreenCapturing, @unchecked Sendable {
        func capture(blur: Bool) async throws -> ScreenCapture {
            throw CaptureError.unavailable("no screen in tests")
        }
    }

    private final class NoWebcam: WebcamCapturing, @unchecked Sendable {
        func capturePhoto() async throws -> Data {
            throw CaptureError.unavailable("no camera in tests")
        }
    }

    private final class FixedForeground: ForegroundWindowSource, @unchecked Sendable {
        var frontmostAppName: String? = "Xcode"
        func currentWindow() async -> ForegroundWindow? {
            ForegroundWindow(appName: "Xcode", windowTitle: "Project.swift")
        }
    }

    private final class NoPrograms: RunningProgramsSource, @unchecked Sendable {
        func runningProgramNames() -> [String] { [] }
    }

    private final class RecentInput: SystemIdleSource, @unchecked Sendable {
        func lastInputAt(now: Date) -> Date? { now }
    }

    private final class Credentials: CredentialsStore, @unchecked Sendable {
        private let lock = NSLock()
        private var stored: StoredSignIn?

        init(_ signIn: StoredSignIn?) {
            stored = signIn
        }

        var saved: StoredSignIn? {
            lock.lock()
            defer { lock.unlock() }
            return stored
        }

        func load() throws -> StoredSignIn? { saved }

        func save(_ signIn: StoredSignIn) throws {
            lock.lock()
            stored = signIn
            lock.unlock()
        }

        func clear() throws {
            lock.lock()
            stored = nil
            lock.unlock()
        }
    }

    private final class Drivers: @unchecked Sendable {
        private let lock = NSLock()
        private var history: [ConsentedPermissions] = []

        func record(_ permissions: ConsentedPermissions) {
            lock.lock()
            history.append(permissions)
            lock.unlock()
        }

        var last: ConsentedPermissions? {
            lock.lock()
            defer { lock.unlock() }
            return history.last
        }
    }

    private final class Snapshots: @unchecked Sendable {
        private let lock = NSLock()
        private var items: [TrackerSnapshot] = []

        func add(_ snapshot: TrackerSnapshot) {
            lock.lock()
            items.append(snapshot)
            lock.unlock()
        }

        var latest: TrackerSnapshot? {
            lock.lock()
            defer { lock.unlock() }
            return items.last
        }
    }

    // MARK: - Harness

    private static let consentAll = LocalConsent.granted(ConsentedPermissions(
        screenshot: true, webcam: false, keyboard: true, mouse: true, activeWindow: true, runningPrograms: true,
        blurScreenshots: false))

    private static func project(_ id: String) -> String {
        #"[{"id":"\#(id)","title":"Website","intervalMinutes":10,"allowScreenshot":true,"allowWebcam":false,"#
            + #""allowKeyboard":true,"allowMouse":true,"allowActiveWindow":true,"allowRunningPrograms":true}]"#
    }

    private static func token(_ refresh: String) -> ScriptedURLProtocol.Reply {
        ScriptedURLProtocol.Reply(
            status: 200, body: #"{"access_token":"t0k3n","token_type":"Bearer","expires_in":3600,"refresh_token":"\#(refresh)"}"#)
    }

    private static func ok(_ body: String) -> ScriptedURLProtocol.Reply {
        ScriptedURLProtocol.Reply(status: 200, body: body)
    }

    private static func refused(_ status: Int, _ code: String) -> ScriptedURLProtocol.Reply {
        ScriptedURLProtocol.Reply(status: status, body: #"{"error":{"code":"\#(code)","message":"Refused."}}"#)
    }

    private struct Harness {
        let worker: TrackerWorker
        let credentials: Credentials
        let drivers: Drivers
        let snapshots: Snapshots
        let preferences: Preferences
        let session: URLSession
        let suiteName: String
        let root: URL
    }

    private func makeHarness(saved: StoredSignIn?, cachedProjectsFor owner: String? = nil) throws -> Harness {
        let root = FileManager.default.temporaryDirectory
            .appendingPathComponent("worker-workspace-\(UUID().uuidString.lowercased())", isDirectory: true)
        let paths = AppPaths.rooted(at: root)
        try paths.createDirectories()
        let suiteName = "test-\(UUID().uuidString.lowercased())"
        let defaults = try XCTUnwrap(UserDefaults(suiteName: suiteName))
        let preferences = Preferences(defaults: defaults)
        let projectCache = ProjectCacheStore(defaults: defaults)
        if let owner = owner {
            let list = try MyProject.decodeList(from: Data(Self.project("p-1").utf8))
            projectCache.save(ProjectCache(owner: owner, resolvedAt: Date(), projects: list))
        }
        let configuration = URLSessionConfiguration.ephemeral
        configuration.protocolClasses = [ScriptedURLProtocol.self]
        let session = URLSession(configuration: configuration)
        let credentials = Credentials(saved)
        let drivers = Drivers()
        let snapshots = Snapshots()
        let consent = ConsentManager(initial: Self.consentAll, storage: MemoryConsentStore(initial: Self.consentAll))
        let deps = TrackerWorker.Dependencies(
            paths: paths,
            configSource: ConfigSource(paths: paths),
            credentials: credentials,
            preferences: preferences,
            projectCache: projectCache,
            consentManager: consent,
            device: DeviceInfo(deviceId: "5b0c1b9e-8f2c-4a43-9a51-0c7f6f2d9b11", deviceName: "Mac", osVersion: "14.5",
                               agentVersion: "1.0"),
            input: InputLog(),
            usage: UsageRecorder(),
            screen: NoScreen(),
            webcam: NoWebcam(),
            foreground: FixedForeground(),
            running: NoPrograms(),
            systemIdle: RecentInput(),
            hardwareIdle: RecentInput(),
            setReadsWindowTitles: { _ in },
            applyDriverPolicy: { drivers.record($0) },
            makeApiClient: { url, identity in
                ApiClient(baseURL: url, identity: identity, session: session)
            },
            publish: { snapshots.add($0) },
            uptime: { 1000 }
        )
        return Harness(worker: TrackerWorker(dependencies: deps), credentials: credentials, drivers: drivers,
                       snapshots: snapshots, preferences: preferences, session: session, suiteName: suiteName, root: root)
    }

    private func tearDown(_ harness: Harness) async {
        await harness.worker.shutdown()
        harness.session.invalidateAndCancel()
        UserDefaults.standard.removePersistentDomain(forName: harness.suiteName)
        try? FileManager.default.removeItem(at: harness.root)
    }

    /// Wait (at most a few seconds) until `condition` holds.
    private func eventually(_ what: String, file: StaticString = #filePath, line: UInt = #line,
                            _ condition: () async -> Bool) async {
        for _ in 0..<250 {
            if await condition() {
                return
            }
            try? await Task.sleep(nanoseconds: 20_000_000)
        }
        XCTFail("timed out waiting for \(what)", file: file, line: line)
    }

    private func requests(_ path: String) -> [ScriptedURLProtocol.Seen] {
        ScriptedURLProtocol.requests.filter { $0.path == path }
    }

    // MARK: - Signing in

    func testASignInWithOneWorkspaceTracksThereAndKeepsNoPassword() async throws {
        ScriptedURLProtocol.reset([
            "/connect/token": [Self.token("refresh-1")],
            "/api/app/workspaces/mine": [Self.ok(#"[{"id":"ws-1","name":"Acme","role":"member","isArchived":false}]"#)],
            "/api/app/project/my-projects": [Self.ok(Self.project("p-1"))],
        ])
        let harness = try makeHarness(saved: nil)
        let worker = harness.worker
        await worker.start(launchedAtLogin: false, startPeriodicTicks: false)
        XCTAssertEqual(harness.snapshots.latest?.status, .signedOut)

        await worker.signIn(username: " user@example.com ", password: "pw-fixture", server: nil, newPassword: nil)
        await eventually("tracking in the only workspace") {
            await worker.facts().tracking
        }

        let token = try XCTUnwrap(requests("/connect/token").first)
        XCTAssertEqual(token.form["grant_type"], "password")
        XCTAssertEqual(token.form["device_id"], "5b0c1b9e-8f2c-4a43-9a51-0c7f6f2d9b11")
        XCTAssertNil(token.form["__tenant"])
        XCTAssertNil(requests("/api/app/workspaces/mine").first?.header("X-Workspace-Id"))
        XCTAssertEqual(requests("/api/app/project/my-projects").first?.header("X-Workspace-Id"), "ws-1")

        let saved = try XCTUnwrap(harness.credentials.saved)
        XCTAssertEqual(saved, StoredSignIn(username: "user@example.com", workspaceId: "ws-1", workspaceName: "Acme",
                                           refreshToken: "refresh-1"))
        XCTAssertFalse(String(decoding: try JSONEncoder().encode(saved), as: UTF8.self).contains("pw-fixture"),
                       "the password is never saved")
        XCTAssertEqual(harness.preferences.lastWorkspaceId, "ws-1")
        XCTAssertEqual(harness.snapshots.latest?.workspace, "Acme")
        await tearDown(harness)
    }

    func testSeveralWorkspacesAreOfferedWithTheirRolesUntilOneIsChosen() async throws {
        ScriptedURLProtocol.reset([
            "/connect/token": [Self.token("refresh-1")],
            "/api/app/workspaces/mine": [Self.ok("""
                [{"id":"ws-1","name":"Acme","role":"owner","isOwner":true},
                 {"id":"ws-2","name":"Globex","role":"member"},
                 {"id":"ws-3","name":"Initech","role":"member","isArchived":true}]
                """)],
            "/api/app/project/my-projects": [Self.ok(Self.project("p-1"))],
        ])
        let harness = try makeHarness(saved: nil)
        let worker = harness.worker
        await worker.start(launchedAtLogin: false, startPeriodicTicks: false)
        await worker.signIn(username: "user@example.com", password: "pw-fixture", server: nil, newPassword: nil)

        let choosing = try XCTUnwrap(harness.snapshots.latest)
        XCTAssertEqual(choosing.status, .choosingWorkspace)
        XCTAssertEqual(choosing.workspaceChoices.map { $0.name }, ["Acme", "Globex"], "never an archived one")
        XCTAssertEqual(choosing.workspaceChoices.map { $0.role }, ["Owner", "Member"])
        XCTAssertNil(harness.credentials.saved, "saved once a workspace is chosen")
        XCTAssertTrue(harness.drivers.last.map { $0 == .allDisabled } ?? true, "nothing is captured meanwhile")
        let notTracking = await worker.facts().tracking
        XCTAssertFalse(notTracking)
        XCTAssertTrue(requests("/api/app/project/my-projects").isEmpty)

        await worker.chooseWorkspace("WS-2", at: Date())
        await eventually("tracking in the chosen workspace") {
            await worker.facts().tracking
        }
        XCTAssertEqual(requests("/api/app/project/my-projects").first?.header("X-Workspace-Id"), "ws-2")
        XCTAssertEqual(harness.credentials.saved?.workspaceId, "ws-2")
        XCTAssertTrue(harness.snapshots.latest?.workspaceChoices.isEmpty ?? false)
        await tearDown(harness)
    }

    // MARK: - Terminal answers

    func testARevokedDeviceStopsTrackingAndForgetsTheSignIn() async throws {
        let owner = Queue.ownerKey(workspaceId: "ws-1", username: "user@example.com")
        ScriptedURLProtocol.reset([
            "/connect/token": [Self.token("refresh-2")],
            "/api/app/project/my-projects": [Self.refused(403, "Tracker:DeviceRevoked")],
        ])
        let harness = try makeHarness(
            saved: StoredSignIn(username: "user@example.com", workspaceId: "ws-1", workspaceName: "Acme",
                                refreshToken: "refresh-1"),
            cachedProjectsFor: owner)
        let worker = harness.worker
        await worker.start(launchedAtLogin: false, startPeriodicTicks: false)

        await eventually("the sign-in to be forgotten") {
            harness.credentials.saved == nil
        }
        await eventually("signed out") {
            harness.snapshots.latest?.status == .signedOut
        }
        let facts = await worker.facts()
        XCTAssertFalse(facts.tracking)
        XCTAssertEqual(harness.drivers.last, ConsentedPermissions.allDisabled, "capture stops at once")
        XCTAssertTrue(harness.snapshots.latest?.loginError?.contains("removed") ?? false)
        XCTAssertEqual(requests("/connect/token").first?.form["grant_type"], "refresh_token",
                       "resumed with the saved refresh token, never a password")
        XCTAssertEqual(requests("/connect/token").first?.form["device_id"], "5b0c1b9e-8f2c-4a43-9a51-0c7f6f2d9b11")
        await tearDown(harness)
    }

    func testALostWorkspaceStopsTrackingThereAndAnotherOneIsChosen() async throws {
        let owner = Queue.ownerKey(workspaceId: "ws-1", username: "user@example.com")
        ScriptedURLProtocol.reset([
            "/connect/token": [Self.token("refresh-rotated")],
            "/api/app/project/my-projects": [Self.refused(403, "Tracker:NotWorkspaceMember"), Self.ok(Self.project("p-2"))],
            "/api/app/workspaces/mine": [Self.ok(#"[{"id":"ws-2","name":"Globex","role":"member"}]"#)],
        ])
        let harness = try makeHarness(
            saved: StoredSignIn(username: "user@example.com", workspaceId: "ws-1", workspaceName: "Acme",
                                refreshToken: "refresh-1"),
            cachedProjectsFor: owner)
        let worker = harness.worker
        await worker.start(launchedAtLogin: false, startPeriodicTicks: false)
        // Offline first: the saved project list starts tracking at once.
        let startedTracking = await worker.facts().tracking
        XCTAssertTrue(startedTracking)

        // The workspace refused; the only other one is taken.
        await eventually("tracking in the other workspace") {
            harness.credentials.saved?.workspaceId == "ws-2"
        }
        XCTAssertNil(requests("/api/app/workspaces/mine").first?.header("X-Workspace-Id"))
        XCTAssertEqual(requests("/api/app/project/my-projects").map { $0.header("X-Workspace-Id") }.first, "ws-1")
        await eventually("the other workspace's projects") {
            requests("/api/app/project/my-projects").contains { $0.header("X-Workspace-Id") == "ws-2" }
        }
        XCTAssertEqual(harness.credentials.saved?.refreshToken, "refresh-rotated", "the rotated token is saved")
        XCTAssertEqual(harness.snapshots.latest?.workspace, "Globex")
        await tearDown(harness)
    }

    func testALostWorkspaceWithNoOtherAsksToChooseAndCapturesNothing() async throws {
        let owner = Queue.ownerKey(workspaceId: "ws-1", username: "user@example.com")
        ScriptedURLProtocol.reset([
            "/connect/token": [Self.token("refresh-2")],
            "/api/app/project/my-projects": [Self.refused(403, "Tracker:WorkspaceArchived")],
            "/api/app/workspaces/mine": [Self.ok(#"[{"id":"ws-1","name":"Acme","role":"owner","isArchived":true}]"#)],
        ])
        let harness = try makeHarness(
            saved: StoredSignIn(username: "user@example.com", workspaceId: "ws-1", workspaceName: "Acme",
                                refreshToken: "refresh-1"),
            cachedProjectsFor: owner)
        let worker = harness.worker
        await worker.start(launchedAtLogin: false, startPeriodicTicks: false)

        await eventually("the workspace list") {
            !requests("/api/app/workspaces/mine").isEmpty && !(harness.snapshots.latest?.findingWorkspaces ?? true)
        }
        await eventually("no workspace to choose") {
            harness.snapshots.latest?.workspaceNote == TrackerWorker.noWorkspaceMessage
        }
        let snapshot = try XCTUnwrap(harness.snapshots.latest)
        XCTAssertEqual(snapshot.status, .choosingWorkspace)
        XCTAssertTrue(snapshot.workspaceChoices.isEmpty)
        XCTAssertEqual(harness.drivers.last, ConsentedPermissions.allDisabled)
        let facts = await worker.facts()
        XCTAssertFalse(facts.tracking)
        XCTAssertNil(harness.credentials.saved?.workspaceId, "no workspace is saved")
        XCTAssertNotNil(harness.credentials.saved?.refreshToken, "the sign-in itself stays")
        await tearDown(harness)
    }

    func testTheRetiredSavedSignInIsRemovedAndAsksToSignInAgain() async throws {
        ScriptedURLProtocol.reset([:])
        let retired = try JSONDecoder().decode(StoredSignIn.self, from: Data(
            #"{"workspace":"acme","username":"user@example.com","password":"retired-fixture"}"#.utf8))
        let harness = try makeHarness(saved: retired)
        let worker = harness.worker
        await worker.start(launchedAtLogin: false, startPeriodicTicks: false)
        let snapshot = try XCTUnwrap(harness.snapshots.latest)
        XCTAssertEqual(snapshot.status, .signedOut)
        XCTAssertEqual(snapshot.suggestedUsername, "user@example.com")
        XCTAssertNotNil(snapshot.loginError)
        XCTAssertNil(harness.credentials.saved, "the item that held the password is gone")
        XCTAssertTrue(ScriptedURLProtocol.requests.isEmpty, "nothing is sent with the retired password")
        await tearDown(harness)
    }
}
