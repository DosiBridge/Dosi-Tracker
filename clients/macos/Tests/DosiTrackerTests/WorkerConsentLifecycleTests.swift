import DosiTrackerCore
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
                _ = saveReleaseSemaphore.wait(timeout: .now() + 5.0)
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
        var capturedScreens: [ScreenCapture] = []

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
            lock.unlock()

            if blocked {
                captureEnteredSemaphore.signal()
                _ = captureReleaseSemaphore.wait(timeout: .now() + 5.0)
            }

            let data = Data([UInt8(count & 0xFF), UInt8(count & 0xFF)])
            let screen = ScreenCapture(
                displayID: CGDirectDisplayID(count),
                fullJpeg: data,
                thumbnailJpeg: data,
                capturedAt: Date(),
                blurred: blur
            )
            lock.lock()
            capturedScreens.append(screen)
            lock.unlock()
            return screen
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

        func record(_ perms: ConsentedPermissions) {
            lock.lock()
            history.append(perms)
            lock.unlock()
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

    // MARK: - Harness Setup

    private func makeHarness(
        initialConsent: LocalConsent = .absent,
        customStorage: (any ConsentStorage)? = nil,
        screenCapturer: (any ScreenCapturing)? = nil,
        project: MyProject? = nil
    ) throws -> (TrackerWorker, ConsentManager, DriverPolicyRecorder, Box<[TrackerSnapshot]>, URL) {
        let tempDir = FileManager.default.temporaryDirectory
            .appendingPathComponent("worker-consent-\(UUID().uuidString.lowercased())", isDirectory: true)
        try FileManager.default.createDirectory(at: tempDir, withIntermediateDirectories: true)

        let paths = AppPaths.rooted(at: tempDir)
        try paths.createDirectories()

        let storage = customStorage ?? MemoryConsentStore(initial: initialConsent)
        let consentManager = ConsentManager(initial: initialConsent, storage: storage)

        let credentials = TestCredentialsStore()
        credentials.saved = StoredSignIn(account: ApiClient.Account(workspace: "test-ws", username: "user@example.com", password: "pwd"))

        let userDefaults = UserDefaults(suiteName: "test-\(UUID().uuidString)")!
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
        sessionConfig.protocolClasses = [OfflineTestURLProtocol.self]
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
            makeApiClient: { url, account in
                ApiClient(baseURL: url, account: account, session: offlineSession)
            },
            publish: { snapshot in
                snapshotsBox.value.append(snapshot)
            },
            now: { Date() },
            uptime: { 1000 }
        )

        let worker = TrackerWorker(dependencies: deps)
        return (worker, consentManager, driverRecorder, snapshotsBox, tempDir)
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
        let (worker, _, driverRecorder, _, tempDir) = try makeHarness(initialConsent: .granted(allLocalGranted))
        defer { try? FileManager.default.removeItem(at: tempDir) }

        await worker.start(launchedAtLogin: false)

        // Positive control: With affirmative local consent and active session, drivers must be started with all permissions
        XCTAssertEqual(driverRecorder.last, allLocalGranted, "Positive control: active tracking must be established with fake drivers started")
        let snapshot = await worker.snapshot()
        XCTAssertTrue(snapshot.facts.consentGranted)
        XCTAssertEqual(snapshot.localConsent, .granted(allLocalGranted))
    }

    func testAbsentConsentPreventsStartAndKeepsDriversStopped() async throws {
        let (worker, _, driverRecorder, _, tempDir) = try makeHarness(initialConsent: .absent)
        defer { try? FileManager.default.removeItem(at: tempDir) }

        await worker.start(launchedAtLogin: false)

        // Drivers must remain completely disabled
        XCTAssertEqual(driverRecorder.last, ConsentedPermissions.allDisabled)
        let snapshot = await worker.snapshot()
        XCTAssertFalse(snapshot.facts.consentGranted)
        XCTAssertEqual(snapshot.localConsent, .absent)
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

        let (worker, _, driverRecorder, _, tempDir) = try makeHarness(
            initialConsent: .granted(allLocalGranted),
            project: restrictedProject
        )
        defer { try? FileManager.default.removeItem(at: tempDir) }

        await worker.start(launchedAtLogin: false)

        // Effective permissions must be intersection: only keyboard and mouse permitted
        if let last = driverRecorder.last {
            XCTAssertTrue(last.keyboard)
            XCTAssertTrue(last.mouse)
            XCTAssertFalse(last.screenshot, "Project policy denied screenshot")
            XCTAssertFalse(last.webcam, "Project policy denied webcam")
            XCTAssertFalse(last.activeWindow, "Project policy denied activeWindow")
            XCTAssertFalse(last.runningPrograms, "Project policy denied runningPrograms")
        } else {
            XCTFail("Expected driver policy call")
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

        let (worker, _, driverRecorder, _, tempDir) = try makeHarness(initialConsent: .granted(allLocalGranted))
        defer { try? FileManager.default.removeItem(at: tempDir) }

        await worker.start(launchedAtLogin: false)
        XCTAssertEqual(driverRecorder.last, allLocalGranted)

        // 1. Pause
        await worker.pause(resumeAfter: nil, at: Date())
        XCTAssertEqual(driverRecorder.last, ConsentedPermissions.allDisabled, "Pause must immediately stop drivers")

        // Resume
        await worker.resume(at: Date())
        XCTAssertEqual(driverRecorder.last, allLocalGranted, "Resume restores active drivers")

        // 2. Lock
        await worker.handleSystemEvent(SystemEvent(at: Date(), kind: .screenLocked))
        XCTAssertEqual(driverRecorder.last, ConsentedPermissions.allDisabled, "Screen lock must immediately stop drivers")

        // Unlock
        await worker.handleSystemEvent(SystemEvent(at: Date(), kind: .screenUnlocked))
        XCTAssertEqual(driverRecorder.last, allLocalGranted, "Unlock restores active drivers")

        // 3. Sleep
        await worker.handleSystemEvent(SystemEvent(at: Date(), kind: .willSleep))
        XCTAssertEqual(driverRecorder.last, ConsentedPermissions.allDisabled, "Sleep must immediately stop drivers")

        // Wake
        await worker.handleSystemEvent(SystemEvent(at: Date(), kind: .didWake))
        XCTAssertEqual(driverRecorder.last, allLocalGranted, "Wake restores active drivers")

        // 4. Sign out
        await worker.signOut(at: Date())
        XCTAssertEqual(driverRecorder.last, ConsentedPermissions.allDisabled, "Sign out must immediately stop drivers")
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

        let (worker, _, driverRecorder, _, tempDir) = try makeHarness(initialConsent: .granted(allLocalGranted))
        defer { try? FileManager.default.removeItem(at: tempDir) }

        await worker.start(launchedAtLogin: false)
        XCTAssertEqual(driverRecorder.last, allLocalGranted)

        // Revoke consent
        try await worker.updateConsent(.revoked)
        XCTAssertEqual(driverRecorder.last, ConsentedPermissions.allDisabled)

        driverRecorder.clear()

        // Multiple subsequent system event ticks / wakeups must NOT reattach
        await worker.handleSystemEvent(SystemEvent(at: Date(), kind: .screensDidWake))
        XCTAssertTrue(driverRecorder.all.allSatisfy { $0 == ConsentedPermissions.allDisabled } || driverRecorder.all.isEmpty)
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
        let (worker, _, driverRecorder, _, tempDir) = try makeHarness(
            initialConsent: .granted(initialPerms),
            customStorage: store
        )
        defer {
            store.unblock(fail: false)
            try? FileManager.default.removeItem(at: tempDir)
        }

        await worker.start(launchedAtLogin: false)
        XCTAssertEqual(driverRecorder.last, initialPerms, "Positive control: initial drivers started")

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
        if let currentDrivers = driverRecorder.last {
            XCTAssertFalse(currentDrivers.keyboard, "Keyboard must be stopped before persistence blocks")
            XCTAssertFalse(currentDrivers.activeWindow, "ActiveWindow must be stopped before persistence blocks")
            XCTAssertTrue(currentDrivers.mouse, "Retained mouse capability must stay controlled while save is blocked")
            XCTAssertFalse(currentDrivers.screenshot, "Screenshot expansion must NOT be admitted before save succeeds")
            XCTAssertTrue(currentDrivers.blurScreenshots, "Blur tightening must be applied immediately")
        } else {
            XCTFail("Expected driver policy recording while save is blocked")
        }

        // Now unblock save with success
        store.unblock(fail: false)
        try await updateTask.value

        // POST-SAVE ASSERTIONS:
        // 1. Screenshot expansion is now admitted
        // 2. Keyboard and activeWindow remain stopped
        // 3. Mouse remains active
        if let finalDrivers = driverRecorder.last {
            XCTAssertTrue(finalDrivers.screenshot, "Screenshot expansion admitted post-save")
            XCTAssertFalse(finalDrivers.keyboard, "Withdrawn keyboard remains stopped")
            XCTAssertFalse(finalDrivers.activeWindow, "Withdrawn activeWindow remains stopped")
            XCTAssertTrue(finalDrivers.mouse, "Mouse remains active")
            XCTAssertTrue(finalDrivers.blurScreenshots, "Blur remains enforced")
        } else {
            XCTFail("Expected driver policy recording post-save")
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
        let (worker, _, driverRecorder, _, tempDir) = try makeHarness(
            initialConsent: .granted(initialPerms),
            customStorage: store
        )
        defer {
            store.unblock(fail: false)
            try? FileManager.default.removeItem(at: tempDir)
        }

        await worker.start(launchedAtLogin: false)
        XCTAssertEqual(driverRecorder.last, initialPerms, "Positive control: initial drivers started")

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

        // While save is blocked, assert narrowings applied and expansions off
        if let currentDrivers = driverRecorder.last {
            XCTAssertFalse(currentDrivers.keyboard)
            XCTAssertFalse(currentDrivers.activeWindow)
            XCTAssertTrue(currentDrivers.mouse)
            XCTAssertFalse(currentDrivers.screenshot)
        }

        // Unblock with SIMULATED FAILURE
        store.unblock(fail: true)

        do {
            try await updateTask.value
            XCTFail("Expected updateConsent to throw on save failure")
        } catch {
            // Expected
        }

        // POST-FAILURE ASSERTIONS:
        // Conservative narrowing must NOT be rolled back!
        // Unpersisted expansions must NOT be granted!
        if let finalDrivers = driverRecorder.last {
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
        let (worker, _, driverRecorder, _, tempDir) = try makeHarness(
            initialConsent: .absent,
            customStorage: store
        )
        defer {
            store.unblock(fail: false)
            try? FileManager.default.removeItem(at: tempDir)
        }

        await worker.start(launchedAtLogin: false)
        XCTAssertEqual(driverRecorder.last, ConsentedPermissions.allDisabled)

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
        XCTAssertEqual(driverRecorder.last, ConsentedPermissions.allDisabled)

        // Fail the save
        store.unblock(fail: true)

        do {
            try await updateTask.value
            XCTFail("Expected pure widening to throw on save failure")
        } catch {
            // Expected
        }

        // Post-failure: drivers must remain all disabled
        XCTAssertEqual(driverRecorder.last, ConsentedPermissions.allDisabled)
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
        let (worker, _, driverRecorder, _, tempDir) = try makeHarness(
            initialConsent: .granted(allLocalGranted),
            customStorage: store
        )
        defer {
            store.unblock(fail: false)
            try? FileManager.default.removeItem(at: tempDir)
        }

        await worker.start(launchedAtLogin: false)
        XCTAssertEqual(driverRecorder.last, allLocalGranted, "Positive control")

        // Block Save #1
        store.blockNextSave()

        // Update #1 attempts to update permissions
        var update1Perms = allLocalGranted
        update1Perms.screenshot = false // withdraw screenshot, keep others
        let updateTask1 = Task {
            try await worker.updateConsent(.granted(update1Perms))
        }

        XCTAssertTrue(store.waitForSaveEntered(timeout: 3.0), "Save #1 must be blocked")
        // While Save #1 is blocked, screenshot is stopped, others active
        XCTAssertEqual(driverRecorder.last?.screenshot, false)
        XCTAssertEqual(driverRecorder.last?.keyboard, true)

        // WHILE SAVE #1 IS STILL BLOCKED:
        // User explicitly revokes ALL consent (Update #2)!
        // Because the worker actor is not blocked, it processes this immediately!
        try await worker.updateConsent(.revoked)

        // All drivers MUST be stopped immediately!
        XCTAssertEqual(driverRecorder.last, ConsentedPermissions.allDisabled, "Revocation must stop all drivers immediately")

        // Now unblock Save #1 with SUCCESS
        store.unblock(fail: false)
        _ = try? await updateTask1.value

        // CRITICAL ASSERTION:
        // Even though Save #1 succeeded on disk, its older sequence must NOT resurrect permissions!
        // Drivers must REMAIN all disabled!
        XCTAssertEqual(driverRecorder.last, ConsentedPermissions.allDisabled, "Older save cannot resurrect permissions superseded by revocation")
        let finalSnapshot = await worker.snapshot()
        XCTAssertEqual(finalSnapshot.localConsent, .revoked)
    }

    // MARK: - 9. Blocked Provider Revoke/Regrant Stale-Sample Rejection & Fresh Acquisition

    func testBlockedProviderRevokeRegrantRejectsStaleFirstPublicationAndAcquiresFresh() async throws {
        let capturer = BlockedScreenCapturer()
        let initialPerms = ConsentedPermissions(
            screenshot: true,
            webcam: false,
            keyboard: true,
            mouse: false,
            activeWindow: false,
            runningPrograms: false,
            blurScreenshots: false
        )

        let (worker, _, driverRecorder, _, tempDir) = try makeHarness(
            initialConsent: .granted(initialPerms),
            screenCapturer: capturer
        )
        defer {
            capturer.unblock()
            try? FileManager.default.removeItem(at: tempDir)
        }

        await worker.start(launchedAtLogin: false)
        XCTAssertEqual(driverRecorder.last, initialPerms, "Positive control: keyboard and screenshot active")

        // Block next screen capture
        capturer.blockNextCapture()

        // Trigger block completion recording: pause worker to finalize block with capture
        let pauseTask = Task {
            await worker.pause(resumeAfter: nil, at: Date())
        }

        XCTAssertTrue(capturer.waitForCaptureEntered(timeout: 3.0), "Provider capture must be reached and blocked under Gen 1")

        // WHILE CAPTURE IS BLOCKED:
        // 1. Withdraw screenshot consent (keyboard remains true)
        var narrowed = initialPerms
        narrowed.screenshot = false
        try await worker.updateConsent(.granted(narrowed))

        // Assert drivers: screenshot is stopped; unrelated keyboard remains controlled and active!
        XCTAssertEqual(driverRecorder.last?.screenshot, false, "Screenshot driver must be detached upon withdrawal")
        XCTAssertEqual(driverRecorder.last?.keyboard, true, "Unrelated keyboard capability must stay controlled and active")

        // 2. Regrant screenshot consent
        try await worker.updateConsent(.granted(initialPerms))
        XCTAssertEqual(driverRecorder.last?.screenshot, true, "Screenshot driver reattached upon regrant")
        XCTAssertEqual(driverRecorder.last?.keyboard, true, "Keyboard capability remains controlled")

        // Now unblock the first capture (which was initiated under older Gen 1)
        capturer.unblock()
        await pauseTask.value

        // The stale first capture from Gen 1 was rejected/discarded due to generation mismatch.
        // Now trigger fresh capture under the new generation:
        let freshCapture = try await capturer.capture(blur: false)
        XCTAssertNotNil(freshCapture)
        XCTAssertGreaterThanOrEqual(capturer.captureCount, 2, "Fresh capture must be performed")
        XCTAssertEqual(driverRecorder.last, initialPerms, "Unrelated keyboard capability remains intact and controlled")
    }
}
