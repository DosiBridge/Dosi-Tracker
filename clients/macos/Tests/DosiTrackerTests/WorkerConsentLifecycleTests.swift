import DosiTrackerCore
import Foundation
import XCTest
@testable import DosiTracker

/// Production-path regression suite for TrackerWorker lifecycle gating,
/// driver policy coordination, immediate stops, asynchronous off-actor persistence,
/// and sample ticket invalidation across revoke/regrant.
final class WorkerConsentLifecycleTests: XCTestCase {

    // MARK: - Test Doubles & Protocol Stubs

    /// Isolated URLProtocol preventing any network traffic from executing during tests.
    private final class OfflineTestURLProtocol: URLProtocol, @unchecked Sendable {
        override class func canInit(with request: URLRequest) -> Bool { true }
        override class func canonicalRequest(for request: URLRequest) -> URLRequest { request }
        override func startLoading() {
            client?.urlProtocol(self, didFailWithError: URLError(.notConnectedToInternet))
        }
        override func stopLoading() {}
    }

    /// Blockable fake URLProtocol enabling deterministic in-flight transport pass testing.
    private final class BlockableURLProtocol: URLProtocol, @unchecked Sendable {
        private static let lock = NSLock()
        private static var isBlocked = false
        private static let requestEnteredSemaphore = DispatchSemaphore(value: 0)
        private static let requestReleaseSemaphore = DispatchSemaphore(value: 0)
        private static var requestCount = 0

        static func reset() {
            lock.lock()
            isBlocked = false
            requestCount = 0
            lock.unlock()
        }

        static func blockNextRequest() {
            lock.lock()
            isBlocked = true
            lock.unlock()
        }

        static func unblock() {
            lock.lock()
            isBlocked = false
            lock.unlock()
            requestReleaseSemaphore.signal()
        }

        static func waitForRequestEntered(timeout: TimeInterval = 3.0) -> Bool {
            requestEnteredSemaphore.wait(timeout: .now() + timeout) == .success
        }

        override class func canInit(with request: URLRequest) -> Bool { true }
        override class func canonicalRequest(for request: URLRequest) -> URLRequest { request }

        override func startLoading() {
            Self.lock.lock()
            let blocked = Self.isBlocked
            Self.requestCount += 1
            Self.lock.unlock()

            if blocked {
                Self.requestEnteredSemaphore.signal()
                _ = Self.requestReleaseSemaphore.wait(timeout: .now() + 5.0)
            }

            client?.urlProtocol(self, didFailWithError: URLError(.notConnectedToInternet))
        }

        override func stopLoading() {}
    }

    private struct TestCaptureError: Error, LocalizedError {
        let message: String
        var errorDescription: String? { message }
    }

    /// Blockable fake consent store enabling deterministic testing of in-flight persistence
    /// without relying on arbitrary sleeps.
    private final class BlockedConsentStore: ConsentStorage, @unchecked Sendable {
        private let lock = NSLock()
        private var consent: LocalConsent
        private let saveEnteredSemaphore = DispatchSemaphore(value: 0)
        private let saveReleaseSemaphore = DispatchSemaphore(value: 0)
        private var isBlocked = false
        var shouldFailSave = false
        private(set) var saveCount = 0
        private(set) var timedOut = false

        init(initial: LocalConsent = .absent) {
            self.consent = initial
        }

        func blockNextSave() {
            lock.lock()
            isBlocked = true
            lock.unlock()
        }

        func unblock(fail: Bool = false) {
            lock.lock()
            shouldFailSave = fail
            isBlocked = false
            lock.unlock()
            saveReleaseSemaphore.signal()
        }

        func waitForSaveEntered(timeout: TimeInterval = 3.0) -> Bool {
            let result = saveEnteredSemaphore.wait(timeout: .now() + timeout)
            return result == .success
        }

        func load() throws -> LocalConsent {
            lock.lock()
            defer { lock.unlock() }
            return consent
        }

        func save(_ consent: LocalConsent) throws {
            lock.lock()
            let blocked = isBlocked
            saveCount += 1
            lock.unlock()

            if blocked {
                saveEnteredSemaphore.signal()
                let waitResult = saveReleaseSemaphore.wait(timeout: .now() + 5.0)
                if waitResult == .timedOut {
                    lock.lock()
                    timedOut = true
                    lock.unlock()
                    throw ConsentError.saveFailed("BlockedConsentStore barrier timed out waiting for release")
                }
            }

            lock.lock()
            let fail = shouldFailSave
            if !fail {
                self.consent = consent
            }
            lock.unlock()

            if fail {
                throw ConsentError.saveFailed("Simulated persistence failure in BlockedConsentStore")
            }
        }
    }

    /// Blockable fake screen capturer enabling deterministic in-flight sample invalidation tests.
    private final class BlockedScreenCapturer: ScreenCapturing, @unchecked Sendable {
        private let lock = NSLock()
        private let captureEnteredSemaphore = DispatchSemaphore(value: 0)
        private let captureReleaseSemaphore = DispatchSemaphore(value: 0)
        private var isBlocked = false
        private(set) var captureCount = 0
        private(set) var timedOut = false
        var nextCaptureData: Data?

        func blockNextCapture() {
            lock.lock()
            isBlocked = true
            lock.unlock()
        }

        func unblock() {
            lock.lock()
            isBlocked = false
            lock.unlock()
            captureReleaseSemaphore.signal()
        }

        func waitForCaptureEntered(timeout: TimeInterval = 3.0) -> Bool {
            let result = captureEnteredSemaphore.wait(timeout: .now() + timeout)
            return result == .success
        }

        func capture(blur: Bool) async throws -> ScreenCapture {
            lock.lock()
            let blocked = isBlocked
            captureCount += 1
            let count = captureCount
            let customData = nextCaptureData
            lock.unlock()

            if blocked {
                captureEnteredSemaphore.signal()
                let waitResult = captureReleaseSemaphore.wait(timeout: .now() + 5.0)
                if waitResult == .timedOut {
                    lock.lock()
                    timedOut = true
                    lock.unlock()
                    throw TestCaptureError(message: "BlockedScreenCapturer barrier timed out waiting for release")
                }
            }

            let data = customData ?? Data([UInt8(count & 0xFF), UInt8(count & 0xFF)])
            return ScreenCapture(
                displayID: CGDirectDisplayID(count),
                fullJpeg: data,
                thumbnailJpeg: data,
                capturedAt: Date(),
                blurred: blur
            )
        }
    }

    private final class TestScreenCapturer: ScreenCapturing, @unchecked Sendable {
        var captureCount = 0
        var lastBlur = false
        func capture(blur: Bool) async throws -> ScreenCapture {
            captureCount += 1
            lastBlur = blur
            return ScreenCapture(displayID: 1, fullJpeg: Data([0xFF, 0xD8]), thumbnailJpeg: Data([0xFF, 0xD8]),
                                 capturedAt: Date(), blurred: blur)
        }
    }

    private final class TestWebcamCapturer: WebcamCapturing, @unchecked Sendable {
        var captureCount = 0
        func capturePhoto() async throws -> Data {
            captureCount += 1
            return Data([0xFF, 0xD8])
        }
    }

    private final class TestForegroundSource: ForegroundWindowSource, @unchecked Sendable {
        var queryCount = 0
        var frontmostAppName: String? = "Xcode"
        func currentWindow() async -> ForegroundWindow? {
            queryCount += 1
            return ForegroundWindow(app: "Xcode", title: "Project.swift", processID: 123)
        }
    }

    private final class TestRunningPrograms: RunningProgramsSource, @unchecked Sendable {
        var queryCount = 0
        func runningProgramNames() -> [String] {
            queryCount += 1
            return ["Xcode", "Finder"]
        }
    }

    private final class TestIdleSource: SystemIdleSource, @unchecked Sendable {
        var lastInput: Date? = Date()
        func lastInputAt(now: Date) -> Date? { lastInput }
    }

    private final class TestCredentialsStore: CredentialsStore, @unchecked Sendable {
        var saved: StoredSignIn?
        func load() throws -> StoredSignIn? { saved }
        func save(_ signIn: StoredSignIn) throws { saved = signIn }
        func clear() throws { saved = nil }
    }

    private final class DriverPolicyRecorder: @unchecked Sendable {
        private let lock = NSLock()
        private var history: [ConsentedPermissions] = []
        var onRecord: (@Sendable (ConsentedPermissions) -> Void)?

        func record(_ perms: ConsentedPermissions) {
            lock.lock()
            history.append(perms)
            let callback = onRecord
            lock.unlock()
            callback?(perms)
        }

        var last: ConsentedPermissions? {
            lock.lock()
            defer { lock.unlock() }
            return history.last
        }

        var all: [ConsentedPermissions] {
            lock.lock()
            defer { lock.unlock() }
            return history
        }

        func clear() {
            lock.lock()
            history.removeAll()
            lock.unlock()
        }
    }

    private final class Box<T>: @unchecked Sendable {
        private let lock = NSLock()
        private var _value: T
        init(_ value: T) { self._value = value }
        var value: T {
            get {
                lock.lock()
                defer { lock.unlock() }
                return _value
            }
            set {
                lock.lock()
                _value = newValue
                lock.unlock()
            }
        }
    }

    // MARK: - Bounded Async Observation Helpers

    /// Starts an asynchronous operation inside an owned `Task` and bounds observation of its
    /// completion using an `XCTestExpectation` and `XCTWaiter.fulfillment`.
    ///
    /// Unlike structured task groups (`withTaskGroup`), `XCTWaiter.fulfillment` guarantees return
    /// after `timeout` even if the underlying operation remains suspended or blocked, avoiding the
    /// Swift structured concurrency trap where task groups implicitly await child tasks on scope exit.
    ///
    /// The caller retains ownership of the returned `Task` and must ensure any synthetic barriers
    /// are released in both success and failure paths before joining the task to prevent resource leaks.
    private func observeAsyncCompletion<T: Sendable>(
        description: String = "async operation",
        timeout: TimeInterval,
        operation: @escaping @Sendable () async -> T
    ) async -> (result: XCTWaiter.Result, task: Task<T, Never>) {
        let exp = XCTestExpectation(description: description)
        let task = Task {
            let val = await operation()
            exp.fulfill()
            return val
        }
        let waitResult = await XCTWaiter.fulfillment(of: [exp], timeout: timeout)
        return (waitResult, task)
    }

    /// Bounds observation of an already-started `Task`'s completion using an `XCTestExpectation`
    /// and `XCTWaiter.fulfillment`. Returns promptly after `timeout` without hanging if the task
    /// is still in flight.
    private func observeTaskCompletion<T: Sendable>(
        task: Task<T, Never>,
        description: String = "task completion",
        timeout: TimeInterval
    ) async -> XCTWaiter.Result {
        let exp = XCTestExpectation(description: description)
        let observerTask = Task {
            _ = await task.result
            exp.fulfill()
        }
        let waitResult = await XCTWaiter.fulfillment(of: [exp], timeout: timeout)
        observerTask.cancel()
        return waitResult
    }

    // MARK: - Harness Setup

    private func makeHarness(
        initialConsent: LocalConsent = .absent,
        customStorage: (any ConsentStorage)? = nil,
        screenCapturer: (any ScreenCapturing)? = nil,
        project: MyProject? = nil,
        clock: Box<Date>? = nil,
        onShutdownJoining: (@Sendable () -> Void)? = nil,
        onCaptureCompleted: (@Sendable (Int, UInt64) -> Void)? = nil,
        urlProtocolClass: AnyClass? = nil
    ) throws -> (TrackerWorker, ConsentManager, DriverPolicyRecorder, Box<[TrackerSnapshot]>, AppPaths, URLSession, String) {
        let tempDir = FileManager.default.temporaryDirectory
            .appendingPathComponent("worker-consent-\(UUID().uuidString.lowercased())", isDirectory: true)
        try FileManager.default.createDirectory(at: tempDir, withIntermediateDirectories: true)

        let paths = AppPaths.rooted(at: tempDir)
        try paths.createDirectories()

        let storage = customStorage ?? MemoryConsentStore(initial: initialConsent)
        let consentManager = ConsentManager(initial: initialConsent, storage: storage)

        let credentials = TestCredentialsStore()
        credentials.saved = StoredSignIn(account: ApiClient.Account(workspace: "test-ws", username: "user@example.com", password: "pwd"))

        let suiteName = "test-\(UUID().uuidString.lowercased())"
        let userDefaults = UserDefaults(suiteName: suiteName)!
        let preferences = Preferences(defaults: userDefaults)
        preferences.rememberedProjectId = project?.id ?? "p-test"

        let projectCache = ProjectCacheStore(paths: paths)
        let sampleProject = project ?? MyProject(
            id: "p-test",
            title: "Test Project",
            intervalMinutes: 10,
            allowScreenshot: true,
            allowWebcam: true,
            allowKeyboard: true,
            allowMouse: true,
            allowActiveWindow: true,
            allowRunningPrograms: true,
            idleThresholdMinutes: 5,
            blurScreenshots: false
        )
        projectCache.save(ProjectCache(owner: "test-ws:user@example.com", resolvedAt: Date(), projects: [sampleProject]))

        let driverRecorder = DriverPolicyRecorder()
        let snapshotsBox = Box<[TrackerSnapshot]>([])

        let sessionConfig = URLSessionConfiguration.ephemeral
        sessionConfig.protocolClasses = [urlProtocolClass ?? OfflineTestURLProtocol.self]
        let offlineSession = URLSession(configuration: sessionConfig)

        let deps = TrackerWorker.Dependencies(
            paths: paths,
            configSource: ConfigSource(paths: paths),
            credentials: credentials,
            preferences: preferences,
            projectCache: projectCache,
            consentManager: consentManager,
            device: DeviceInfo(deviceId: "dev-1", deviceName: "Mac", osVersion: "14.5", agentVersion: "1.0"),
            input: InputLog(),
            usage: UsageRecorder(),
            screen: screenCapturer ?? TestScreenCapturer(),
            webcam: TestWebcamCapturer(),
            foreground: TestForegroundSource(),
            running: TestRunningPrograms(),
            systemIdle: TestIdleSource(),
            hardwareIdle: TestIdleSource(),
            setReadsWindowTitles: { _ in },
            applyDriverPolicy: { perms in
                driverRecorder.record(perms)
            },
            onShutdownJoining: onShutdownJoining ?? {},
            onCaptureCompleted: onCaptureCompleted ?? { _, _ in },
            makeApiClient: { url, account in
                ApiClient(baseURL: url, account: account, session: offlineSession)
            },
            publish: { snapshot in
                snapshotsBox.value.append(snapshot)
            },
            now: { clock?.value ?? Date() },
            uptime: { 1000 }
        )

        let worker = TrackerWorker(dependencies: deps)
        return (worker, consentManager, driverRecorder, snapshotsBox, paths, offlineSession, suiteName)
    }

    private func withHarness(
        initialConsent: LocalConsent = .absent,
        customStorage: (any ConsentStorage)? = nil,
        screenCapturer: (any ScreenCapturing)? = nil,
        project: MyProject? = nil,
        clock: Box<Date>? = nil,
        onShutdownJoining: (@Sendable () -> Void)? = nil,
        onCaptureCompleted: (@Sendable (Int, UInt64) -> Void)? = nil,
        urlProtocolClass: AnyClass? = nil,
        _ body: (TrackerWorker, ConsentManager, DriverPolicyRecorder, Box<[TrackerSnapshot]>, AppPaths) async throws -> Void
    ) async throws {
        let (worker, consentManager, driverRecorder, snapshotsBox, paths, session, suiteName) = try makeHarness(
            initialConsent: initialConsent,
            customStorage: customStorage,
            screenCapturer: screenCapturer,
            project: project,
            clock: clock,
            onShutdownJoining: onShutdownJoining,
            onCaptureCompleted: onCaptureCompleted,
            urlProtocolClass: urlProtocolClass
        )
        defer {
            session.invalidateAndCancel()
            UserDefaults.standard.removePersistentDomain(forName: suiteName)
            try? FileManager.default.removeItem(at: paths.root)
        }
        do {
            try await body(worker, consentManager, driverRecorder, snapshotsBox, paths)
        } catch {
            await worker.shutdown()
            throw error
        }
        await worker.shutdown()
    }

    // MARK: - 1. Positive Control & Absent Consent

    func testPositiveControlActiveTrackingStartsFakeDrivers() async throws {
        let allLocalGranted = ConsentedPermissions(
            screenshot: true,
            webcam: true,
            keyboard: true,
            mouse: true,
            activeWindow: true,
            runningPrograms: true,
            blurScreenshots: false
        )
        try await withHarness(initialConsent: .granted(allLocalGranted)) { worker, _, driverRecorder, _, _ in
            await worker.start(launchedAtLogin: false, startPeriodicTicks: false)

            guard let lastDrivers = driverRecorder.last else {
                return XCTFail("Driver policy recording must not be nil after start")
            }
            XCTAssertEqual(lastDrivers, allLocalGranted, "Positive control: active tracking must be established with fake drivers started")
            let snapshot = await worker.snapshot()
            XCTAssertTrue(snapshot.facts.consentGranted)
            XCTAssertEqual(snapshot.localConsent, .granted(allLocalGranted))
        }
    }

    func testAbsentConsentPreventsStartAndKeepsDriversStopped() async throws {
        try await withHarness(initialConsent: .absent) { worker, _, driverRecorder, _, _ in
            await worker.start(launchedAtLogin: false, startPeriodicTicks: false)

            guard let lastDrivers = driverRecorder.last else {
                return XCTFail("Driver policy recording must not be nil after start")
            }
            XCTAssertEqual(lastDrivers, ConsentedPermissions.allDisabled)
            let snapshot = await worker.snapshot()
            XCTAssertFalse(snapshot.facts.consentGranted)
            XCTAssertEqual(snapshot.localConsent, .absent)
        }
    }

    // MARK: - 2. Project Policy Deny Restricts Effective Permissions

    func testProjectPolicyDenyRestrictsEffectivePermissions() async throws {
        let restrictedProject = MyProject(
            id: "p-restricted",
            title: "Restricted",
            intervalMinutes: 10,
            allowScreenshot: false,
            allowWebcam: false,
            allowKeyboard: true,
            allowMouse: true,
            allowActiveWindow: false,
            allowRunningPrograms: false
        )

        let allLocalGranted = ConsentedPermissions(
            screenshot: true,
            webcam: true,
            keyboard: true,
            mouse: true,
            activeWindow: true,
            runningPrograms: true,
            blurScreenshots: false
        )

        try await withHarness(initialConsent: .granted(allLocalGranted), project: restrictedProject) { worker, _, driverRecorder, _, _ in
            await worker.start(launchedAtLogin: false, startPeriodicTicks: false)

            guard let last = driverRecorder.last else {
                return XCTFail("Driver policy recording must not be nil")
            }
            XCTAssertTrue(last.keyboard)
            XCTAssertTrue(last.mouse)
            XCTAssertFalse(last.screenshot, "Project policy denied screenshot")
            XCTAssertFalse(last.webcam, "Project policy denied webcam")
            XCTAssertFalse(last.activeWindow, "Project policy denied activeWindow")
            XCTAssertFalse(last.runningPrograms, "Project policy denied runningPrograms")
        }
    }

    // MARK: - 3. Pause / Signout / Lock / Sleep / Quit Stop Drivers Immediately

    func testPauseSignoutLockSleepQuitImmediatelyStopsDrivers() async throws {
        let allLocalGranted = ConsentedPermissions(
            screenshot: true,
            webcam: true,
            keyboard: true,
            mouse: true,
            activeWindow: true,
            runningPrograms: true,
            blurScreenshots: false
        )

        try await withHarness(initialConsent: .granted(allLocalGranted)) { worker, _, driverRecorder, _, _ in
            await worker.start(launchedAtLogin: false, startPeriodicTicks: false)
            guard let initial = driverRecorder.last else {
                return XCTFail("Driver policy recording must not be nil after start")
            }
            XCTAssertEqual(initial, allLocalGranted)

            // 1. Pause
            await worker.pause(resumeAfter: nil, at: Date())
            guard let paused = driverRecorder.last else {
                return XCTFail("Driver policy recording must not be nil after pause")
            }
            XCTAssertEqual(paused, ConsentedPermissions.allDisabled, "Pause must immediately stop drivers")

            // Resume
            await worker.resume(at: Date())
            guard let resumed = driverRecorder.last else {
                return XCTFail("Driver policy recording must not be nil after resume")
            }
            XCTAssertEqual(resumed, allLocalGranted, "Resume restores active drivers")

            // 2. Lock
            await worker.handleSystemEvent(SystemEvent(at: Date(), kind: .screenLocked))
            guard let locked = driverRecorder.last else {
                return XCTFail("Driver policy recording must not be nil after lock")
            }
            XCTAssertEqual(locked, ConsentedPermissions.allDisabled, "Screen lock must immediately stop drivers")

            // Unlock
            await worker.handleSystemEvent(SystemEvent(at: Date(), kind: .screenUnlocked))
            guard let unlocked = driverRecorder.last else {
                return XCTFail("Driver policy recording must not be nil after unlock")
            }
            XCTAssertEqual(unlocked, allLocalGranted, "Unlock restores active drivers")

            // 3. Sleep
            await worker.handleSystemEvent(SystemEvent(at: Date(), kind: .willSleep))
            guard let sleeping = driverRecorder.last else {
                return XCTFail("Driver policy recording must not be nil after sleep")
            }
            XCTAssertEqual(sleeping, ConsentedPermissions.allDisabled, "Sleep must immediately stop drivers")

            // Wake
            await worker.handleSystemEvent(SystemEvent(at: Date(), kind: .didWake))
            guard let awake = driverRecorder.last else {
                return XCTFail("Driver policy recording must not be nil after wake")
            }
            XCTAssertEqual(awake, allLocalGranted, "Wake restores active drivers")

            // 4. Sign out
            await worker.signOut(at: Date())
            guard let signedOut = driverRecorder.last else {
                return XCTFail("Driver policy recording must not be nil after sign out")
            }
            XCTAssertEqual(signedOut, ConsentedPermissions.allDisabled, "Sign out must immediately stop drivers")
        }
    }

    // MARK: - 4. Denial Retry Cannot Reattach

    func testDenialRetryCannotReattach() async throws {
        let allLocalGranted = ConsentedPermissions(
            screenshot: true,
            webcam: true,
            keyboard: true,
            mouse: true,
            activeWindow: true,
            runningPrograms: true,
            blurScreenshots: false
        )

        try await withHarness(initialConsent: .granted(allLocalGranted)) { worker, _, driverRecorder, _, _ in
            await worker.start(launchedAtLogin: false, startPeriodicTicks: false)
            guard let initial = driverRecorder.last else {
                return XCTFail("Driver policy recording must not be nil after start")
            }
            XCTAssertEqual(initial, allLocalGranted)

            // Revoke consent
            try await worker.updateConsent(.revoked)
            guard let revoked = driverRecorder.last else {
                return XCTFail("Driver policy recording must not be nil after revoke")
            }
            XCTAssertEqual(revoked, ConsentedPermissions.allDisabled)

            driverRecorder.clear()

            // Multiple subsequent system event ticks / wakeups must NOT reattach
            await worker.handleSystemEvent(SystemEvent(at: Date(), kind: .screensDidWake))
            XCTAssertTrue(driverRecorder.all.allSatisfy { $0 == ConsentedPermissions.allDisabled } || driverRecorder.all.isEmpty)
        }
    }

    // MARK: - 5. Blocked Save: Mixed Narrow/Expand with Pre-Save Detach and Post-Save Expansion

    func testBlockedSaveMixedNarrowExpandStopsWithdrawnDriversBeforeSaveAndAdmitsExpansionsOnSuccess() async throws {
        let initialPerms = ConsentedPermissions(
            screenshot: false,
            webcam: false,
            keyboard: true,
            mouse: true,
            activeWindow: true,
            runningPrograms: false,
            blurScreenshots: false
        )
        let store = BlockedConsentStore(initial: .granted(initialPerms))
        defer { store.unblock(fail: false) }

        try await withHarness(initialConsent: .granted(initialPerms), customStorage: store) { worker, _, driverRecorder, _, _ in
            await worker.start(launchedAtLogin: false, startPeriodicTicks: false)
            guard let initial = driverRecorder.last else {
                return XCTFail("Driver policy recording must not be nil after start")
            }
            XCTAssertEqual(initial, initialPerms, "Positive control: initial drivers started")

            // Block next save
            store.blockNextSave()

            // Mixed update: withdraw keyboard and activeWindow, expand screenshot, keep mouse, tighten blur
            let mixedPerms = ConsentedPermissions(
                screenshot: true,
                webcam: false,
                keyboard: false,
                mouse: true,
                activeWindow: false,
                runningPrograms: false,
                blurScreenshots: true
            )

            let updateTask = Task {
                try await worker.updateConsent(.granted(mixedPerms))
            }

            // Wait until save is blocked inside the persistence thread
            XCTAssertTrue(store.waitForSaveEntered(timeout: 3.0), "Save must be reached and blocked")

            // ASSERTIONS WHILE SAVE IS BLOCKED:
            // 1. Withdrawn keyboard driver must be STOPPED before save!
            // 2. Withdrawn activeWindow must be STOPPED before save!
            // 3. Retained mouse capability stays CONTROLLED and ACTIVE!
            // 4. Expanded screenshot capability must NOT be admitted yet!
            // 5. Blur must be tightened immediately as defense-in-depth!
            guard let currentDrivers = driverRecorder.last else {
                store.unblock(fail: false)
                _ = try? await updateTask.value
                return XCTFail("Expected driver policy recording while save is blocked")
            }
            XCTAssertFalse(currentDrivers.keyboard, "Keyboard must be stopped before persistence blocks")
            XCTAssertFalse(currentDrivers.activeWindow, "ActiveWindow must be stopped before persistence blocks")
            XCTAssertTrue(currentDrivers.mouse, "Retained mouse capability must stay controlled while save is blocked")
            XCTAssertFalse(currentDrivers.screenshot, "Screenshot expansion must NOT be admitted before save succeeds")
            XCTAssertTrue(currentDrivers.blurScreenshots, "Blur tightening must be applied immediately")

            // Now unblock save with success
            store.unblock(fail: false)
            try await updateTask.value
            XCTAssertFalse(store.timedOut, "Store barrier must not have timed out")

            // POST-SAVE ASSERTIONS:
            // 1. Screenshot expansion is now admitted
            // 2. Keyboard and activeWindow remain stopped
            // 3. Mouse remains active
            guard let finalDrivers = driverRecorder.last else {
                return XCTFail("Expected driver policy recording post-save")
            }
            XCTAssertTrue(finalDrivers.screenshot, "Screenshot expansion admitted post-save")
            XCTAssertFalse(finalDrivers.keyboard, "Withdrawn keyboard remains stopped")
            XCTAssertFalse(finalDrivers.activeWindow, "Withdrawn activeWindow remains stopped")
            XCTAssertTrue(finalDrivers.mouse, "Mouse remains active")
            XCTAssertTrue(finalDrivers.blurScreenshots, "Blur remains enforced")
        }
    }

    // MARK: - 6. Blocked Save: Mixed Narrow/Expand Preserves Conservative State on Save Failure

    func testBlockedSaveMixedNarrowExpandPreservesConservativeStateOnSaveFailure() async throws {
        let initialPerms = ConsentedPermissions(
            screenshot: false,
            webcam: false,
            keyboard: true,
            mouse: true,
            activeWindow: true,
            runningPrograms: false,
            blurScreenshots: false
        )
        let store = BlockedConsentStore(initial: .granted(initialPerms))
        defer { store.unblock(fail: false) }

        try await withHarness(initialConsent: .granted(initialPerms), customStorage: store) { worker, _, driverRecorder, _, _ in
            await worker.start(launchedAtLogin: false, startPeriodicTicks: false)
            guard let initial = driverRecorder.last else {
                return XCTFail("Driver policy recording must not be nil after start")
            }
            XCTAssertEqual(initial, initialPerms, "Positive control: initial drivers started")

            // Block next save
            store.blockNextSave()

            let mixedPerms = ConsentedPermissions(
                screenshot: true,
                webcam: false,
                keyboard: false,
                mouse: true,
                activeWindow: false,
                runningPrograms: false,
                blurScreenshots: true
            )

            let updateTask = Task {
                try await worker.updateConsent(.granted(mixedPerms))
            }

            XCTAssertTrue(store.waitForSaveEntered(timeout: 3.0), "Save must be reached and blocked")

            guard let currentDrivers = driverRecorder.last else {
                store.unblock(fail: false)
                _ = try? await updateTask.value
                return XCTFail("Expected driver policy recording while save is blocked")
            }
            XCTAssertFalse(currentDrivers.keyboard)
            XCTAssertFalse(currentDrivers.activeWindow)
            XCTAssertTrue(currentDrivers.mouse)
            XCTAssertFalse(currentDrivers.screenshot)

            // Unblock with SIMULATED FAILURE
            store.unblock(fail: true)

            do {
                try await updateTask.value
                XCTFail("Expected updateConsent to throw on save failure")
            } catch {
                // Expected
            }

            XCTAssertFalse(store.timedOut, "Store barrier must not have timed out")

            // POST-FAILURE ASSERTIONS:
            // Conservative narrowing must NOT be rolled back!
            // Unpersisted expansions must NOT be granted!
            guard let finalDrivers = driverRecorder.last else {
                return XCTFail("Driver policy recording must not be nil after failure")
            }
            XCTAssertFalse(finalDrivers.keyboard, "Withdrawn keyboard must remain stopped after save failure")
            XCTAssertFalse(finalDrivers.activeWindow, "Withdrawn activeWindow must remain stopped after save failure")
            XCTAssertTrue(finalDrivers.mouse, "Retained mouse capability remains controlled")
            XCTAssertFalse(finalDrivers.screenshot, "Unpersisted screenshot expansion was never admitted")
            XCTAssertTrue(finalDrivers.blurScreenshots, "Tightened blur remains enforced")
        }
    }

    // MARK: - 7. Blocked Save: Pure Widening Failure Does Not Admit Permissions

    func testBlockedSavePureWideningFailureDoesNotAdmitPermissions() async throws {
        let store = BlockedConsentStore(initial: .absent)
        defer { store.unblock(fail: false) }

        try await withHarness(initialConsent: .absent, customStorage: store) { worker, _, driverRecorder, _, _ in
            await worker.start(launchedAtLogin: false, startPeriodicTicks: false)
            guard let initial = driverRecorder.last else {
                return XCTFail("Driver policy recording must not be nil after start")
            }
            XCTAssertEqual(initial, ConsentedPermissions.allDisabled)

            store.blockNextSave()

            let wideningGrant = ConsentedPermissions(
                screenshot: true,
                webcam: true,
                keyboard: true,
                mouse: true,
                activeWindow: true,
                runningPrograms: true,
                blurScreenshots: false
            )

            let updateTask = Task {
                try await worker.updateConsent(.granted(wideningGrant))
            }

            XCTAssertTrue(store.waitForSaveEntered(timeout: 3.0))

            // While save is blocked: pure widening means drivers MUST remain disabled!
            guard let duringSave = driverRecorder.last else {
                store.unblock(fail: false)
                _ = try? await updateTask.value
                return XCTFail("Driver policy recording must not be nil during save")
            }
            XCTAssertEqual(duringSave, ConsentedPermissions.allDisabled)

            // Fail the save
            store.unblock(fail: true)

            do {
                try await updateTask.value
                XCTFail("Expected pure widening to throw on save failure")
            } catch {
                // Expected
            }

            XCTAssertFalse(store.timedOut, "Store barrier must not have timed out")

            // Post-failure: drivers must remain all disabled
            guard let postFailure = driverRecorder.last else {
                return XCTFail("Driver policy recording must not be nil after failure")
            }
            XCTAssertEqual(postFailure, ConsentedPermissions.allDisabled)
        }
    }

    // MARK: - 8. Blocked Save: Newer Revocation Supersedes Older Outstanding Save

    func testBlockedSaveNewerRevocationSupersedesOlderOutstandingSave() async throws {
        let allLocalGranted = ConsentedPermissions(
            screenshot: true,
            webcam: true,
            keyboard: true,
            mouse: true,
            activeWindow: true,
            runningPrograms: true,
            blurScreenshots: false
        )
        let store = BlockedConsentStore(initial: .granted(allLocalGranted))
        defer { store.unblock(fail: false) }

        try await withHarness(initialConsent: .granted(allLocalGranted), customStorage: store) { worker, _, driverRecorder, _, _ in
            await worker.start(launchedAtLogin: false, startPeriodicTicks: false)
            guard let initial = driverRecorder.last else {
                return XCTFail("Driver policy recording must not be nil after start")
            }
            XCTAssertEqual(initial, allLocalGranted, "Positive control: all drivers active")

            // Block Save #1
            store.blockNextSave()

            // Update #1 attempts to withdraw screenshot
            var update1Perms = allLocalGranted
            update1Perms.screenshot = false
            let updateTask1 = Task {
                try await worker.updateConsent(.granted(update1Perms))
            }

            XCTAssertTrue(store.waitForSaveEntered(timeout: 3.0), "Save #1 must enter store and block")
            guard let duringSave1Drivers = driverRecorder.last else {
                store.unblock(fail: false)
                _ = try? await updateTask1.value
                return XCTFail("Driver policy recording must not be nil during Save #1")
            }
            XCTAssertEqual(duringSave1Drivers.screenshot, false)
            XCTAssertEqual(duringSave1Drivers.keyboard, true)

            // Set up deterministic acknowledgement for Update #2 (revocation) immediate driver stop
            let driverRevocationAck = DispatchSemaphore(value: 0)
            driverRecorder.onRecord = { perms in
                if perms == ConsentedPermissions.allDisabled {
                    driverRevocationAck.signal()
                }
            }

            // Launch Update #2 (revocation) as a separately tracked operation
            let updateTask2 = Task {
                try await worker.updateConsent(.revoked)
            }

            // Establish its immediate conservative transition and driver stop while Save #1 remains blocked!
            let ackResult = driverRevocationAck.wait(timeout: .now() + 3.0)
            XCTAssertEqual(ackResult, .success, "Update #2 must immediately stop all drivers while Save #1 is blocked")
            guard let duringSave2Drivers = driverRecorder.last else {
                store.unblock(fail: false)
                _ = try? await updateTask1.value
                _ = try? await updateTask2.value
                return XCTFail("Driver policy recording must not be nil after revocation")
            }
            XCTAssertEqual(duringSave2Drivers, ConsentedPermissions.allDisabled, "Revocation must stop all drivers immediately")

            // Now release Save #1
            store.unblock(fail: false)

            // Bounded join for both update tasks
            _ = try await updateTask1.value
            _ = try await updateTask2.value

            XCTAssertFalse(store.timedOut, "Store barrier must not have timed out")

            // Even though Save #1 succeeded on disk, its older sequence must NOT resurrect permissions!
            guard let finalDrivers = driverRecorder.last else {
                return XCTFail("Driver policy recording must not be nil at end")
            }
            XCTAssertEqual(finalDrivers, ConsentedPermissions.allDisabled, "Older save cannot resurrect permissions superseded by revocation")

            // Assert in-memory policy
            let finalSnapshot = await worker.snapshot()
            XCTAssertEqual(finalSnapshot.localConsent, .revoked)

            // Assert final persisted consent in store
            let persistedConsent = try store.load()
            XCTAssertEqual(persistedConsent, .revoked, "Final persisted consent in store must be revoked")
        }
    }

    // MARK: - 9. Blocked Provider Revoke/Regrant Stale-Sample Rejection & Fresh Acquisition

    func testBlockedProviderRevokeRegrantRejectsStaleFirstPublicationAndAcquiresFresh() async throws {
        let capturer = BlockedScreenCapturer()
        defer { capturer.unblock() }

        let initialPerms = ConsentedPermissions(
            screenshot: true,
            webcam: false,
            keyboard: true,
            mouse: false,
            activeWindow: false,
            runningPrograms: false,
            blurScreenshots: false
        )

        let staleData = Data("stale-first-bytes".utf8)
        let staleBase64 = staleData.base64EncodedString()
        let freshData = Data("fresh-second-bytes".utf8)
        let freshBase64 = freshData.base64EncodedString()

        let captureCompletedSemaphore = DispatchSemaphore(value: 0)
        let startTime = Date()
        let clock = Box<Date>(startTime)

        try await withHarness(
            initialConsent: .granted(initialPerms),
            screenCapturer: capturer,
            clock: clock,
            onCaptureCompleted: { _, _ in
                captureCompletedSemaphore.signal()
            }
        ) { worker, _, driverRecorder, _, paths in
            await worker.start(launchedAtLogin: false, startPeriodicTicks: false)
            guard let initialDrivers = driverRecorder.last else {
                return XCTFail("Driver policy recording must not be nil after start")
            }
            XCTAssertEqual(initialDrivers, initialPerms, "Positive control: keyboard and screenshot active")
            let startSnapshot = await worker.snapshot()
            XCTAssertTrue(startSnapshot.facts.trackingActive, "Lifecycle must be actively tracking")

            // Block next screen capture in fake provider and assign distinguishable stale bytes
            capturer.nextCaptureData = staleData
            capturer.blockNextCapture()

            // Trigger active acquisition through the real worker
            await worker.triggerActiveCapture()
            XCTAssertTrue(capturer.waitForCaptureEntered(timeout: 3.0), "Provider capture must be reached and blocked under Gen 1")

            // WHILE CAPTURE #1 IS BLOCKED IN PROVIDER:
            // 1. Withdraw screenshot consent ONLY (unrelated keyboard remains authorized and lifecycle active)
            var narrowed = initialPerms
            narrowed.screenshot = false
            try await worker.updateConsent(.granted(narrowed))

            guard let narrowedDrivers = driverRecorder.last else {
                capturer.unblock()
                return XCTFail("Driver policy recording must not be nil after withdrawal")
            }
            XCTAssertFalse(narrowedDrivers.screenshot, "Screenshot driver must be detached upon withdrawal")
            XCTAssertTrue(narrowedDrivers.keyboard, "Unrelated keyboard capability must stay controlled and active")
            let narrowedSnapshot = await worker.snapshot()
            XCTAssertTrue(narrowedSnapshot.facts.trackingActive, "Worker lifecycle must remain actively tracking")

            // 2. Regrant screenshot consent (keyboard still authorized, lifecycle still active)
            try await worker.updateConsent(.granted(initialPerms))
            guard let regrantedDrivers = driverRecorder.last else {
                capturer.unblock()
                return XCTFail("Driver policy recording must not be nil after regrant")
            }
            XCTAssertTrue(regrantedDrivers.screenshot, "Screenshot driver reattached upon regrant")
            XCTAssertTrue(regrantedDrivers.keyboard, "Keyboard capability remains controlled")
            let regrantedSnapshot = await worker.snapshot()
            XCTAssertTrue(regrantedSnapshot.facts.trackingActive, "Worker lifecycle remains actively tracking")

            // Now release Capture #1 (which was initiated under older Gen 1)
            capturer.unblock()

            // Wait for old worker captureFinished actor execution to complete via production completion seam
            let oldCompleted = captureCompletedSemaphore.wait(timeout: .now() + 3.0) == .success
            XCTAssertTrue(oldCompleted, "Old capture task must finish executing captureFinished before checking publication absence")

            // Discard of stale first publication: assert stale bytes/identity are absent from publication & queue
            let interimSnapshot = await worker.snapshot()
            XCTAssertNil(interimSnapshot.lastScreenshot, "Stale first capture must not be published to lastScreenshot")
            XCTAssertTrue(interimSnapshot.recentBlocks.allSatisfy {
                $0.payload.screenshotThumbJpgBase64 != staleBase64
            }, "Stale first capture bytes must be absent from recentBlocks")

            let queue = try Queue.open(at: paths.database, legacyOwner: nil, now: clock.value)
            let interimQueueBatch = try queue.pendingBatch(owner: "test-ws:user@example.com", afterId: 0, limit: 10)
            XCTAssertTrue(interimQueueBatch.allSatisfy {
                $0.payload.screenshotThumbJpgBase64 != staleBase64
            }, "Stale first capture bytes must be absent from SQLite queue payload")

            // Now trigger fresh acquisition through the REAL WORKER:
            capturer.nextCaptureData = freshData
            await worker.triggerActiveCapture()

            let freshCompleted = captureCompletedSemaphore.wait(timeout: .now() + 3.0) == .success
            XCTAssertTrue(freshCompleted, "Fresh capture task must finish executing captureFinished")

            // Advance clock beyond AgentTiming.minimumBlock (60 s) and commit block through real worker
            clock.value = clock.value.addingTimeInterval(70)
            await worker.syncNow(at: clock.value)
            await worker.flushRecorder()

            // Assert fresh identity reaches its intended publication / queue boundary
            let freshSnapshot = await worker.snapshot()
            guard let lastScreenshot = freshSnapshot.lastScreenshot else {
                return XCTFail("Fresh screenshot must reach publication")
            }
            XCTAssertEqual(lastScreenshot.jpeg, freshData, "Fresh screenshot must reach lastScreenshot")

            guard let lastRecent = freshSnapshot.recentBlocks.first else {
                return XCTFail("Recent block must exist after syncNow")
            }
            XCTAssertEqual(lastRecent.payload.screenshotThumbJpgBase64, freshBase64)
            XCTAssertNotEqual(lastRecent.payload.screenshotThumbJpgBase64, staleBase64)

            let freshQueueBatch = try queue.pendingBatch(owner: "test-ws:user@example.com", afterId: 0, limit: 10)
            XCTAssertTrue(freshQueueBatch.contains {
                $0.payload.screenshotThumbJpgBase64 == freshBase64
            }, "Fresh capture must reach SQLite queue")
            XCTAssertTrue(freshQueueBatch.allSatisfy {
                $0.payload.screenshotThumbJpgBase64 != staleBase64
            }, "Stale capture must still be absent from SQLite queue")

            guard let finalDrivers = driverRecorder.last else {
                return XCTFail("Driver policy recording must not be nil at end")
            }
            XCTAssertTrue(finalDrivers.keyboard, "Keyboard capability remains controlled and authorized")
            XCTAssertTrue(finalDrivers.screenshot, "Screenshot capability active after fresh acquisition")
            XCTAssertFalse(capturer.timedOut, "Provider barrier must not have timed out")
        }
    }

    // MARK: - 10. Blocked Operation Shutdown Join Regressions & Negative Oracle

    func testBlockedActiveProviderShutdownWaitsAtJoiningBoundaryUntilRelease() async throws {
        let capturer = BlockedScreenCapturer()
        defer { capturer.unblock() }

        let initialPerms = ConsentedPermissions(
            screenshot: true,
            webcam: false,
            keyboard: true,
            mouse: false,
            activeWindow: false,
            runningPrograms: false,
            blurScreenshots: false
        )

        let joiningSemaphore = DispatchSemaphore(value: 0)

        try await withHarness(
            initialConsent: .granted(initialPerms),
            screenCapturer: capturer,
            onShutdownJoining: { joiningSemaphore.signal() }
        ) { worker, _, driverRecorder, _, paths in
            await worker.start(launchedAtLogin: false, startPeriodicTicks: false)
            guard let initialDrivers = driverRecorder.last else {
                return XCTFail("Driver policy recording must not be nil after start")
            }
            XCTAssertEqual(initialDrivers, initialPerms, "Positive control: keyboard and screenshot active")

            // Block next screen capture in fake provider and assign distinguishable bytes
            capturer.nextCaptureData = Data("active-blocked-bytes".utf8)
            capturer.blockNextCapture()

            // Trigger active acquisition through the real worker
            await worker.triggerActiveCapture()
            XCTAssertTrue(capturer.waitForCaptureEntered(timeout: 3.0), "Provider capture must enter and block")

            // Start worker shutdown inside an owned Task and observe with genuinely bounded async window
            let (initialWaitResult, shutdownTask) = await observeAsyncCompletion(
                description: "shutdown while active capture blocked",
                timeout: 0.1
            ) {
                await worker.shutdown()
            }

            // 1. Establish that shutdown has reached the real joining boundary
            let reachedBoundary = joiningSemaphore.wait(timeout: .now() + 3.0) == .success
            XCTAssertTrue(reachedBoundary, "Shutdown must reach the joining boundary")

            // 2. Establish that shutdown remains incomplete before release via bounded observation
            XCTAssertEqual(initialWaitResult, .timedOut, "Shutdown must remain incomplete while active provider capture is blocked")

            // 3. Release the blocked provider barrier (also guarded by test defer)
            capturer.unblock()

            // 4. Genuinely bounded join: observe shutdown completion using XCTWaiter (no withTaskGroup scope-exit hang)
            let completedResult = await observeTaskCompletion(
                task: shutdownTask,
                description: "shutdown completion after provider release",
                timeout: 3.0
            )
            XCTAssertEqual(completedResult, .completed, "Shutdown must complete within 3-second bound after provider release")
            _ = await shutdownTask.result

            // 5. Prove no further queue / publication after cleanup
            let finalSnapshot = await worker.snapshot()
            XCTAssertNil(finalSnapshot.lastScreenshot, "No screenshot must be published after shutdown cleanup")
            XCTAssertTrue(finalSnapshot.recentBlocks.isEmpty, "No recent blocks must be published after shutdown cleanup")

            let queue = try Queue.open(at: paths.database, legacyOwner: nil, now: Date())
            let pendingBatch = try queue.pendingBatch(owner: "test-ws:user@example.com", afterId: 0, limit: 10)
            XCTAssertTrue(pendingBatch.isEmpty, "No queue rows must be created after shutdown cleanup")

            // 6. Drivers stopped
            guard let finalDrivers = driverRecorder.last else {
                return XCTFail("Driver policy recording must not be nil at end")
            }
            XCTAssertEqual(finalDrivers, ConsentedPermissions.allDisabled, "All drivers must be stopped")
            XCTAssertFalse(capturer.timedOut, "Provider barrier must not have timed out")
        }
    }

    func testBlockedRetiredProviderShutdownWaitsAtJoiningBoundaryUntilRelease() async throws {
        let capturer = BlockedScreenCapturer()
        defer { capturer.unblock() }

        let initialPerms = ConsentedPermissions(
            screenshot: true,
            webcam: false,
            keyboard: true,
            mouse: false,
            activeWindow: false,
            runningPrograms: false,
            blurScreenshots: false
        )

        let joiningSemaphore = DispatchSemaphore(value: 0)

        try await withHarness(
            initialConsent: .granted(initialPerms),
            screenCapturer: capturer,
            onShutdownJoining: { joiningSemaphore.signal() }
        ) { worker, _, driverRecorder, _, paths in
            await worker.start(launchedAtLogin: false, startPeriodicTicks: false)
            guard let initialDrivers = driverRecorder.last else {
                return XCTFail("Driver policy recording must not be nil after start")
            }
            XCTAssertEqual(initialDrivers, initialPerms, "Positive control: keyboard and screenshot active")

            // Block screen capture in fake provider
            capturer.nextCaptureData = Data("retired-blocked-bytes".utf8)
            capturer.blockNextCapture()

            // Trigger active acquisition through the real worker
            await worker.triggerActiveCapture()
            XCTAssertTrue(capturer.waitForCaptureEntered(timeout: 3.0), "Provider capture must enter and block")

            // Narrow consent to withdraw screenshot while capture is blocked:
            // Must return immediately without waiting for blocked provider, retiring the task handle
            var narrowed = initialPerms
            narrowed.screenshot = false
            try await worker.updateConsent(.granted(narrowed))

            guard let narrowedDrivers = driverRecorder.last else {
                capturer.unblock()
                return XCTFail("Driver policy recording must not be nil after withdrawal")
            }
            XCTAssertFalse(narrowedDrivers.screenshot, "Screenshot driver must be detached upon withdrawal")
            XCTAssertTrue(narrowedDrivers.keyboard, "Unrelated keyboard capability must stay active")

            // Start worker shutdown inside an owned Task and observe with genuinely bounded async window
            let (initialWaitResult, shutdownTask) = await observeAsyncCompletion(
                description: "shutdown while retired capture blocked",
                timeout: 0.1
            ) {
                await worker.shutdown()
            }

            // 1. Establish that shutdown has reached the real joining boundary
            let reachedBoundary = joiningSemaphore.wait(timeout: .now() + 3.0) == .success
            XCTAssertTrue(reachedBoundary, "Shutdown must reach the joining boundary")

            // 2. Establish that shutdown remains incomplete before release via bounded observation
            XCTAssertEqual(initialWaitResult, .timedOut, "Shutdown must remain incomplete while retired provider task is blocked")

            // 3. Release the blocked provider barrier (also guarded by test defer)
            capturer.unblock()

            // 4. Genuinely bounded join: observe shutdown completion using XCTWaiter (no withTaskGroup scope-exit hang)
            let completedResult = await observeTaskCompletion(
                task: shutdownTask,
                description: "shutdown completion after provider release",
                timeout: 3.0
            )
            XCTAssertEqual(completedResult, .completed, "Shutdown must complete within 3-second bound after provider release")
            _ = await shutdownTask.result

            // 5. Prove no further queue / publication after cleanup
            let finalSnapshot = await worker.snapshot()
            XCTAssertNil(finalSnapshot.lastScreenshot, "No screenshot must be published after shutdown cleanup")
            XCTAssertTrue(finalSnapshot.recentBlocks.isEmpty, "No recent blocks must be published after shutdown cleanup")

            let queue = try Queue.open(at: paths.database, legacyOwner: nil, now: Date())
            let pendingBatch = try queue.pendingBatch(owner: "test-ws:user@example.com", afterId: 0, limit: 10)
            XCTAssertTrue(pendingBatch.isEmpty, "No queue rows must be created after shutdown cleanup")

            // 6. Drivers stopped
            guard let finalDrivers = driverRecorder.last else {
                return XCTFail("Driver policy recording must not be nil at end")
            }
            XCTAssertEqual(finalDrivers, ConsentedPermissions.allDisabled, "All drivers must be stopped")
            XCTAssertFalse(capturer.timedOut, "Provider barrier must not have timed out")
        }
    }

    func testBlockedTransportUploadShutdownWaitsAtJoiningBoundaryUntilRelease() async throws {
        defer { BlockableURLProtocol.unblock() }
        BlockableURLProtocol.reset()
        BlockableURLProtocol.blockNextRequest()

        let initialPerms = ConsentedPermissions(
            screenshot: true,
            webcam: false,
            keyboard: true,
            mouse: false,
            activeWindow: false,
            runningPrograms: false,
            blurScreenshots: false
        )

        let joiningSemaphore = DispatchSemaphore(value: 0)

        try await withHarness(
            initialConsent: .granted(initialPerms),
            onShutdownJoining: { joiningSemaphore.signal() },
            urlProtocolClass: BlockableURLProtocol.self
        ) { worker, _, driverRecorder, _, paths in
            // Seed one pending activity in the SQLite queue before starting worker so UploadLoop has immediate work
            let queue = try Queue.open(at: paths.database, legacyOwner: nil, now: Date())
            let payload = ActivityPayload(
                projectId: "p-test",
                clientActivityId: UUID().uuidString.lowercased(),
                startedAt: Date().addingTimeInterval(-600),
                endedAt: Date(),
                mouseClicks: 10,
                keyboardHits: 25,
                activeWindows: [],
                runningPrograms: [],
                timeline: []
            )
            _ = try queue.enqueue(payload, owner: "test-ws:user@example.com")

            await worker.start(launchedAtLogin: false, startPeriodicTicks: false)
            guard let initialDrivers = driverRecorder.last else {
                return XCTFail("Driver policy recording must not be nil after start")
            }
            XCTAssertEqual(initialDrivers, initialPerms, "Positive control: drivers active")

            // Kick an immediate upload pass
            worker.reconnect()

            // Establish that the upload pass reached transport and blocked in BlockableURLProtocol
            let transportEntered = BlockableURLProtocol.waitForRequestEntered(timeout: 3.0)
            XCTAssertTrue(transportEntered, "Upload pass must enter transport and block in BlockableURLProtocol")

            // Start worker shutdown inside an owned Task and observe with genuinely bounded async window
            let (initialWaitResult, shutdownTask) = await observeAsyncCompletion(
                description: "shutdown while upload transport is blocked",
                timeout: 0.1
            ) {
                await worker.shutdown()
            }

            // 1. Establish that shutdown has reached the joining boundary
            let reachedBoundary = joiningSemaphore.wait(timeout: .now() + 3.0) == .success
            XCTAssertTrue(reachedBoundary, "Shutdown must reach the joining boundary")

            // 2. Establish that shutdown remains incomplete before release via bounded observation
            XCTAssertEqual(initialWaitResult, .timedOut, "Shutdown must remain incomplete while upload pass is blocked in transport")

            // 3. Release the blocked transport barrier
            BlockableURLProtocol.unblock()

            // 4. Genuinely bounded join: observe shutdown completion using XCTWaiter
            let completedResult = await observeTaskCompletion(
                task: shutdownTask,
                description: "shutdown completion after transport release",
                timeout: 3.0
            )
            XCTAssertEqual(completedResult, .completed, "Shutdown must complete within 3-second bound after transport release")
            _ = await shutdownTask.result

            // 5. Prove completion/cleanup and no queue corruption or unintended mutation
            let finalQueue = try Queue.open(at: paths.database, legacyOwner: nil, now: Date())
            let pendingCount = try finalQueue.pendingCount(owner: "test-ws:user@example.com")
            XCTAssertEqual(pendingCount, 1, "The queued block must remain intact in local storage; failed transport does not drop rows")
            let rejectedCount = try finalQueue.rejectedCount(owner: "test-ws:user@example.com")
            XCTAssertEqual(rejectedCount, 0, "No blocks must be parked as rejected")

            // 6. Drivers stopped
            guard let finalDrivers = driverRecorder.last else {
                return XCTFail("Driver policy recording must not be nil at end")
            }
            XCTAssertEqual(finalDrivers, ConsentedPermissions.allDisabled, "All drivers must be stopped")
        }
    }

    func testNegativeOracleBoundedObservationTimesOutWhenOperationRemainsBlocked() async throws {
        let barrier = DispatchSemaphore(value: 0)
        let enteredSemaphore = DispatchSemaphore(value: 0)
        let released = Box<Bool>(false)
        defer {
            if !released.value {
                barrier.signal()
            }
        }

        // Start intentionally blocked synthetic operation and observe with bounded window
        let (initialResult, task) = await observeAsyncCompletion(
            description: "intentionally blocked synthetic operation",
            timeout: 0.1
        ) {
            enteredSemaphore.signal()
            _ = barrier.wait(timeout: .now() + 5.0)
        }

        // Establish operation entered blocked state
        let entered = enteredSemaphore.wait(timeout: .now() + 1.0) == .success
        XCTAssertTrue(entered, "Synthetic operation must enter blocked barrier")

        // Oracle demonstration: bounded observation reliably times out rather than hanging indefinitely
        XCTAssertEqual(initialResult, .timedOut, "Negative oracle: bounded observation must return .timedOut when operation is blocked")

        // Release barrier and verify task can now be cleanly joined without leaks
        released.value = true
        barrier.signal()

        let completionResult = await observeTaskCompletion(
            task: task,
            description: "join after releasing synthetic barrier",
            timeout: 1.0
        )
        XCTAssertEqual(completionResult, .completed, "Negative oracle: task must complete promptly after barrier release")
        _ = await task.result
    }
}
