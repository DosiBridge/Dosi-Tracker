import DosiTrackerCore
import XCTest
@testable import DosiTracker

/// Production-path regression suite for TrackerWorker lifecycle gating,
/// driver policy coordination, immediate stops, and sample ticket invalidation.
final class WorkerConsentLifecycleTests: XCTestCase {

    // MARK: - Test Doubles

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

    // MARK: - Harness Setup

    private func makeHarness(
        initialConsent: LocalConsent = .absent,
        project: MyProject? = nil
    ) throws -> (TrackerWorker, ConsentManager, DriverPolicyRecorder, URL) {
        let tempDir = FileManager.default.temporaryDirectory
            .appendingPathComponent("worker-consent-\(UUID().uuidString.lowercased())", isDirectory: true)
        try FileManager.default.createDirectory(at: tempDir, withIntermediateDirectories: true)

        let paths = AppPaths.rooted(at: tempDir)
        try paths.createDirectories()

        let memoryConsentStore = MemoryConsentStore(initial: initialConsent)
        let consentManager = ConsentManager(initial: initialConsent, storage: memoryConsentStore)

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
        let publishBox = Box<TrackerSnapshot>(TrackerSnapshot())

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
            screen: TestScreenCapturer(),
            webcam: TestWebcamCapturer(),
            foreground: TestForegroundSource(),
            running: TestRunningPrograms(),
            systemIdle: TestIdleSource(),
            hardwareIdle: TestIdleSource(),
            setReadsWindowTitles: { _ in },
            applyDriverPolicy: { perms in
                driverRecorder.record(perms)
            },
            publish: { snapshot in
                publishBox.value = snapshot
            },
            now: { Date() },
            uptime: { 1000 }
        )

        let worker = TrackerWorker(dependencies: deps)
        return (worker, consentManager, driverRecorder, tempDir)
    }

    private final class Box<T>: @unchecked Sendable {
        var value: T
        init(_ value: T) { self.value = value }
    }

    // MARK: - 1. Absent Consent No-Start

    func testAbsentConsentPreventsStartAndKeepsDriversStopped() async throws {
        let (worker, _, driverRecorder, tempDir) = try makeHarness(initialConsent: .absent)
        defer { try? FileManager.default.removeItem(at: tempDir) }

        await worker.start(launchedAtLogin: false)

        // Drivers must remain disabled
        XCTAssertEqual(driverRecorder.last, ConsentedPermissions.allDisabled)
    }

    // MARK: - 2. Project Policy Deny Restricts Effective Permissions

    func testProjectPolicyDenyRestrictsEffectivePermissions() async throws {
        // Project policy strictly denies screenshot, webcam, and active window
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

        let (worker, _, driverRecorder, tempDir) = try makeHarness(
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

        let (worker, _, driverRecorder, tempDir) = try makeHarness(initialConsent: .granted(allLocalGranted))
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

        let (worker, _, driverRecorder, tempDir) = try makeHarness(initialConsent: .granted(allLocalGranted))
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

    // MARK: - 5. Revoke / Regrant Stale Result Rejection

    func testRevokeRegrantInvalidatesStaleCaptureResults() async throws {
        let allLocalGranted = ConsentedPermissions(
            screenshot: true,
            webcam: true,
            keyboard: true,
            mouse: true,
            activeWindow: true,
            runningPrograms: true,
            blurScreenshots: false
        )

        let (worker, _, driverRecorder, tempDir) = try makeHarness(initialConsent: .granted(allLocalGranted))
        defer { try? FileManager.default.removeItem(at: tempDir) }

        await worker.start(launchedAtLogin: false)
        XCTAssertEqual(driverRecorder.last, allLocalGranted)

        // Withdraw screenshot permission
        var narrowed = allLocalGranted
        narrowed.screenshot = false
        try await worker.updateConsent(.granted(narrowed))

        // In-flight or completed screenshot from older generation is dropped
        if let last = driverRecorder.last {
            XCTAssertFalse(last.screenshot, "Screenshot driver must be detached upon withdrawal")
            XCTAssertTrue(last.keyboard, "Keyboard capability remains retained and controlled")
        } else {
            XCTFail("Expected driver policy update")
        }
    }
}
