import XCTest
@testable import DosiTrackerCore

/// Ports of the Windows agent's `tracking/mod.rs` tests (`build_activity`,
/// `split_span`, `kept_idle_blocks`), plus the measuring step.
final class BlockAssemblerTests: XCTestCase {
    private let zone = BlockZone(timeZone: "Asia/Dhaka", utcOffsetMinutes: 360)

    private func slice(_ app: String, _ from: Int64, _ to: Int64) -> UsageSlice {
        UsageSlice(app: app, title: "\(app) window", startMs: t0Ms + from * 1000, endMs: t0Ms + to * 1000)
    }

    private func block(_ permissions: CapturePermissions = .all, blur: Bool = false,
                       endReason: EndReason? = nil) -> BlockSpec {
        BlockSpec(projectId: "p1", permissions: permissions, blurScreenshots: blur, startedAt: at(t0),
                  endedAt: at(t0 + 600), endReason: endReason, taskNote: "  Invoice export  ")
    }

    /// Ten minutes of measured work: typing in minutes 0-5, clicks in 7,
    /// nothing in 6, 8 and 9; three injected events.
    private func measurements() -> Measurements {
        let keyPresses = (0..<6).map { t0Ms + Int64($0) * 60_000 + 5_000 }
        let clicks = [t0Ms + 7 * 60_000 + 1_000]
        let activeSeconds = keyPresses.map { $0 / 1000 } + clicks.map { $0 / 1000 }
        return Measurements(
            keyPresses: keyPresses,
            clicks: clicks,
            activeSeconds: activeSeconds,
            injected: 3,
            inputAvailable: true,
            slices: [slice("Xcode", 0, 400), slice("Google Chrome", 400, 600)],
            windowNow: nil,
            runningPrograms: [WindowInfo(appName: "Slack", windowTitle: "")]
        )
    }

    private func capture(blurred: Bool) -> ScreenCapture {
        ScreenCapture(image: Data([0xFF, 0xD8, 0xFF, 0x01]), thumbnail: Data([0xFF, 0xD8, 0xFF, 0x02]),
                      capturedAt: at(t0 + 250), blurred: blurred)
    }

    func testAMeasuredBlockCarriesEveryContractField() {
        let payload = BlockAssembler.build(block(endReason: .interval), measured: measurements(),
                                           device: sampleDevice(), zone: zone)

        // Active in minutes 0-5 and 7: 7 of 10 buckets.
        XCTAssertEqual(payload.productivity, 70)
        XCTAssertEqual(payload.idleSeconds, 3 * 60)
        XCTAssertEqual(payload.injectedInputEvents, 3)
        XCTAssertEqual(payload.keyboardHits, 6)
        XCTAssertEqual(payload.mouseClicks, 1)
        XCTAssertEqual(payload.timeline.count, 10)
        XCTAssertEqual(payload.timeline[0].activeSeconds, 1)
        XCTAssertEqual(payload.timeline[6].activeSeconds, 0)
        XCTAssertFalse(payload.timeline[6].active)
        XCTAssertEqual(payload.activeWindows.first?.appName, "Xcode")
        XCTAssertEqual(payload.runningPrograms.count, 1)
        XCTAssertEqual(payload.endReason, .interval)
        XCTAssertEqual(payload.description, "Invoice export")

        // Device context.
        XCTAssertEqual(payload.platform, "macos")
        XCTAssertEqual(payload.agentVersion, "9.9.9")
        XCTAssertEqual(payload.deviceId, "5b0c1b9e-8f2c-4a43-9a51-0c7f6f2d9b11")
        XCTAssertEqual(payload.deviceName, "Ana's MacBook Pro")
        XCTAssertEqual(payload.osVersion, "macOS 14.5 (23F79)")
        XCTAssertEqual(payload.timeZone, "Asia/Dhaka")
        XCTAssertEqual(payload.utcOffsetMinutes, 360)
        // Stamped at send time, not here.
        XCTAssertNil(payload.sentAt)
        XCTAssertNotNil(UUID(uuidString: payload.clientActivityId))
        XCTAssertEqual(payload.clientActivityId, payload.clientActivityId.lowercased())
    }

    func testActivityIsNotMeasuredWhenNoInputKindIsRecorded() {
        var permissions = CapturePermissions.all
        permissions.keyboard = false
        permissions.mouse = false
        var measured = measurements()
        measured.keyPresses = []
        measured.clicks = []
        measured.activeSeconds = []
        let payload = BlockAssembler.build(block(permissions), measured: measured, device: sampleDevice(), zone: zone)
        XCTAssertNil(payload.productivity)
        XCTAssertNil(payload.idleSeconds)
        XCTAssertNil(payload.injectedInputEvents)
        XCTAssertTrue(payload.timeline.isEmpty, "all-inactive minutes would read as idle")
        // Not measured is null, never a 0 that reads as "no input".
        XCTAssertNil(payload.keyboardHits)
        XCTAssertNil(payload.mouseClicks)
        // Apps are still recorded, without made-up counts.
        XCTAssertFalse(payload.activeWindows.isEmpty)
        XCTAssertTrue(payload.activeWindows.allSatisfy { $0.keyboardHits == nil && $0.mouseClicks == nil })
        let json = jsonDictionary(try! WireJSON.encoder().encode(payload))
        XCTAssertTrue(json["keyboardHits"] is NSNull, "sent as an explicit null")
        XCTAssertTrue(json["mouseClicks"] is NSNull, "sent as an explicit null")
    }

    func testEachInputKindIsMeasuredOnItsOwn() {
        // The keyboard is recorded, the mouse is not (the project or the
        // user's consent leaves it out).
        var permissions = CapturePermissions.all
        permissions.mouse = false
        var measured = measurements()
        measured.clicks = []
        let payload = BlockAssembler.build(block(permissions), measured: measured, device: sampleDevice(), zone: zone)
        XCTAssertEqual(payload.keyboardHits, 6)
        XCTAssertNil(payload.mouseClicks)
        XCTAssertNotNil(payload.productivity, "the keyboard alone still measures activity")
        XCTAssertEqual(payload.timeline.count, 10)
        XCTAssertTrue(payload.timeline.allSatisfy { $0.mouseClicks == nil && $0.keyboardHits != nil })
        XCTAssertEqual(payload.timeline[0].keyboardHits, 1)
        XCTAssertEqual(payload.timeline[6].keyboardHits, 0, "measured, and none: a real 0")
        XCTAssertTrue(payload.activeWindows.allSatisfy { $0.mouseClicks == nil && $0.keyboardHits != nil })

        let minute = jsonDictionary(try! WireJSON.encoder().encode(payload.timeline[0]))
        XCTAssertEqual(minute["keyboardHits"] as? Int, 1)
        XCTAssertTrue(minute["mouseClicks"] is NSNull)

        // Measured and zero: 0, not null.
        var quiet = measurements()
        quiet.keyPresses = []
        quiet.clicks = []
        quiet.activeSeconds = []
        let idle = BlockAssembler.build(block(), measured: quiet, device: sampleDevice(), zone: zone)
        XCTAssertEqual(idle.keyboardHits, 0)
        XCTAssertEqual(idle.mouseClicks, 0)
        XCTAssertEqual(idle.productivity, 0)
    }

    func testActivityIsNotMeasuredWhenTheCaptureFailedDuringTheBlock() {
        var measured = measurements()
        measured.inputAvailable = false
        let payload = BlockAssembler.build(block(), measured: measured, device: sampleDevice(), zone: zone)
        XCTAssertNil(payload.productivity)
        XCTAssertNil(payload.idleSeconds)
        XCTAssertTrue(payload.timeline.isEmpty)
        // A partial count would understate the input: not measured.
        XCTAssertNil(payload.keyboardHits)
        XCTAssertNil(payload.mouseClicks)
    }

    func testTheScreenCaptureFollowsTheProjectPolicy() {
        var measured = measurements()
        measured.screen = capture(blurred: false)
        var payload = BlockAssembler.build(block(), measured: measured, device: sampleDevice(), zone: zone)
        XCTAssertEqual(payload.screenshotPngBase64, Data([0xFF, 0xD8, 0xFF, 0x01]).base64EncodedString())
        XCTAssertEqual(payload.screenshotThumbJpgBase64, Data([0xFF, 0xD8, 0xFF, 0x02]).base64EncodedString())
        XCTAssertEqual(payload.screenshotCapturedAt, at(t0 + 250))
        XCTAssertEqual(payload.screenshotBlurred, false)

        // Blur required: an unblurred capture is never sent.
        payload = BlockAssembler.build(block(blur: true), measured: measured, device: sampleDevice(), zone: zone)
        XCTAssertFalse(payload.hasCaptures)
        XCTAssertNil(payload.screenshotCapturedAt)
        XCTAssertNil(payload.screenshotBlurred)
        measured.screen = capture(blurred: true)
        payload = BlockAssembler.build(block(blur: true), measured: measured, device: sampleDevice(), zone: zone)
        XCTAssertEqual(payload.screenshotBlurred, true)

        // Screenshots not allowed: dropped, whatever the caller passed.
        var none = CapturePermissions.all
        none.screenshot = false
        none.webcam = false
        measured.webcamJpeg = Data([0xFF, 0xD8, 0xFF, 0x03])
        payload = BlockAssembler.build(block(none), measured: measured, device: sampleDevice(), zone: zone)
        XCTAssertFalse(payload.hasCaptures)
    }

    func testCapturesOverTheServerLimitsAreNeverAttached() {
        var measured = measurements()
        measured.screen = ScreenCapture(image: Data(count: CaptureLimits.maxScreenBytes + 1),
                                        thumbnail: Data([1]), capturedAt: at(t0 + 30), blurred: false)
        measured.webcamJpeg = Data(count: CaptureLimits.maxWebcamBytes + 1)
        let payload = BlockAssembler.build(block(), measured: measured, device: sampleDevice(), zone: zone)
        XCTAssertFalse(payload.hasCaptures)
        XCTAssertNil(payload.screenshotCapturedAt)

        // An oversized thumbnail alone is left out; the capture stays.
        measured.screen = ScreenCapture(image: Data([1, 2, 3]), thumbnail: Data(count: CaptureLimits.maxThumbnailBytes + 1),
                                        capturedAt: at(t0 + 30), blurred: false)
        let kept = BlockAssembler.build(block(), measured: measured, device: sampleDevice(), zone: zone)
        XCTAssertNotNil(kept.screenshotPngBase64)
        XCTAssertNil(kept.screenshotThumbJpgBase64)
    }

    func testACaptureTakenBeforeALockIsKept() {
        // The random moment came before the lock; only a capture at the end
        // is forbidden.
        let spec = block(endReason: .lock)
        XCTAssertFalse(spec.capturesAtEnd)
        var measured = measurements()
        measured.screen = capture(blurred: false)
        let payload = BlockAssembler.build(spec, measured: measured, device: sampleDevice(), zone: zone)
        XCTAssertNotNil(payload.screenshotPngBase64)
        XCTAssertEqual(payload.endReason, .lock)
        XCTAssertTrue(block().capturesAtEnd)
        XCTAssertTrue(block(endReason: .pause).capturesAtEnd)
    }

    func testAppUsageFollowsTheActiveWindowPermission() {
        var permissions = CapturePermissions.all
        permissions.activeWindow = false
        permissions.runningPrograms = false
        var measured = measurements()
        measured.slices = []
        measured.runningPrograms = []
        var payload = BlockAssembler.build(block(permissions), measured: measured, device: sampleDevice(), zone: zone)
        XCTAssertTrue(payload.activeWindows.isEmpty && payload.runningPrograms.isEmpty)
        XCTAssertTrue(payload.timeline.allSatisfy { $0.appName == nil })

        // A block too short for a sample: the window in front stands in.
        measured.windowNow = ForegroundWindow(appName: "Microsoft Excel", windowTitle: "Q3")
        payload = BlockAssembler.build(block(), measured: measured, device: sampleDevice(), zone: zone)
        XCTAssertEqual(payload.activeWindows, [WindowInfo(appName: "Microsoft Excel", windowTitle: "Q3")])
    }

    func testTaskNotesAreCleanedAndBounded() {
        XCTAssertNil(BlockAssembler.cleanDescription(nil))
        XCTAssertNil(BlockAssembler.cleanDescription("   "))
        XCTAssertEqual(BlockAssembler.cleanDescription(" Fix login "), "Fix login")
        XCTAssertEqual(BlockAssembler.cleanDescription("Fix\n\tlogin  page\u{0007}"), "Fix login page")
        let long = String(repeating: "x", count: 5000)
        XCTAssertEqual(BlockAssembler.cleanDescription(long)?.utf16.count, BlockAssembler.maxDescriptionLength)
        // Characters outside the BMP count twice, as on the server.
        let emoji = String(repeating: "😀", count: 1500)
        XCTAssertLessThanOrEqual(BlockAssembler.cleanDescription(emoji)?.utf16.count ?? 0, 2048)
        XCTAssertEqual(TaskNote.description(from: "  Review PR  "), "Review PR")
        XCTAssertTrue(TaskNote.changesDescription(from: "Review PR", to: "Review PR 2"))
        XCTAssertFalse(TaskNote.changesDescription(from: "Review PR", to: " Review  PR "))
    }

    func testKeptIdleTimeIsRecordedAsIdleBlocks() {
        let span = AwaySpan(start: at(t0), end: at(t0 + 25 * 60))
        let blocks = BlockAssembler.keptIdleBlocks(projectId: "p1", span: span, interval: 600, description: "Client call",
                                                   device: sampleDevice(), timeZone: TimeZone(identifier: "Asia/Dhaka")!)
        XCTAssertEqual(blocks.count, 3)
        for payload in blocks {
            let length = Int(payload.endedAt.timeIntervalSince(payload.startedAt))
            XCTAssertLessThanOrEqual(length, 600)
            XCTAssertEqual(payload.productivity, 0)
            XCTAssertEqual(payload.idleSeconds, length)
            XCTAssertEqual(payload.endReason, .idle)
            XCTAssertTrue(payload.activeWindows.isEmpty && payload.timeline.isEmpty && !payload.hasCaptures)
            XCTAssertEqual(payload.description, "Client call")
            XCTAssertEqual(payload.timeZone, "Asia/Dhaka")
            XCTAssertEqual(payload.utcOffsetMinutes, 360)
            XCTAssertEqual(payload.deviceId, "5b0c1b9e-8f2c-4a43-9a51-0c7f6f2d9b11")
            XCTAssertEqual(payload.mouseClicks, 0)
            XCTAssertEqual(payload.keyboardHits, 0)
        }
        XCTAssertEqual(blocks[0].startedAt, at(t0))
        XCTAssertEqual(blocks[2].endedAt, at(t0 + 25 * 60))
        XCTAssertTrue(zip(blocks, blocks.dropFirst()).allSatisfy { $0.endedAt == $1.startedAt }, "no gaps")
        // Distinct blocks, distinct idempotency keys.
        XCTAssertNotEqual(blocks[0].clientActivityId, blocks[1].clientActivityId)
        let short = AwaySpan(start: at(t0), end: at(t0 + 30))
        XCTAssertTrue(BlockAssembler.keptIdleBlocks(projectId: "p1", span: short, interval: 600, description: nil,
                                                    device: sampleDevice()).isEmpty)
    }

    func testRunningProgramsAreNamesOnlyOncePerAppAndBounded() {
        let rows = BlockAssembler.runningProgramRows(["Safari", "", "safari", "Xcode", "Slack", "XCODE"])
        XCTAssertEqual(rows, [
            WindowInfo(appName: "Safari", windowTitle: ""),
            WindowInfo(appName: "Xcode", windowTitle: ""),
            WindowInfo(appName: "Slack", windowTitle: ""),
        ])
        let many = (0..<300).map { "App \($0)" }
        XCTAssertEqual(BlockAssembler.runningProgramRows(many).count, BlockAssembler.maxRunningPrograms)
    }

    func testMeasuringReadsOnlyTheBlockAndTheAllowedKinds() {
        let input = InputLog()
        let usage = UsageRecorder()
        let filter = InputFilter(log: input)
        // Before the block (while paused): must not count.
        filter.apply(.keyDown(1), injected: false, atMs: t0Ms - 30_000)
        filter.apply(.keyUp(1), injected: false, atMs: t0Ms - 29_900)
        // Inside it.
        filter.apply(.keyDown(1), injected: false, atMs: t0Ms + 10_000)
        filter.apply(.buttonDown, injected: false, atMs: t0Ms + 70_000)
        filter.apply(.move, injected: true, atMs: t0Ms + 80_000)
        let front = ForegroundWindow(appName: "Xcode", windowTitle: "App.swift")
        // The sampler, every 3 s from before the block to just before its end.
        for second in stride(from: Int64(-3), through: 117, by: 3) {
            usage.record(nowMs: t0Ms + second * 1000, window: front)
        }

        let spec = BlockSpec(projectId: "p1", permissions: .all, blurScreenshots: false, startedAt: at(t0),
                             endedAt: at(t0 + 120), endReason: .pause, taskNote: nil)
        let measured = BlockAssembler.measure(spec, input: input, usage: usage, windowNow: front,
                                              runningPrograms: ["Xcode", "Finder"], screen: nil, webcamJpeg: nil)
        XCTAssertEqual(measured.keyPresses, [t0Ms + 10_000])
        XCTAssertEqual(measured.clicks, [t0Ms + 70_000])
        XCTAssertEqual(measured.injected, 1)
        XCTAssertTrue(measured.inputAvailable)
        // Usage runs right up to the end of the block, clipped to its start.
        XCTAssertEqual(measured.slices.first?.startMs, t0Ms)
        XCTAssertEqual(measured.slices.last?.endMs, t0Ms + 120_000)
        XCTAssertEqual(measured.runningPrograms.map { $0.appName }, ["Xcode", "Finder"])

        let payload = BlockAssembler.build(spec, measured: measured, device: sampleDevice(), zone: zone)
        XCTAssertEqual(payload.productivity, 100)
        XCTAssertEqual(payload.activeWindows, [WindowInfo(appName: "Xcode", windowTitle: "App.swift", seconds: 120,
                                                          keyboardHits: 1, mouseClicks: 1)])

        // Keyboard not allowed: no key presses, and its seconds do not count.
        var noKeys = spec
        noKeys.permissions.keyboard = false
        let mouseOnly = BlockAssembler.measure(noKeys, input: input, usage: usage, windowNow: front,
                                               runningPrograms: [], screen: nil, webcamJpeg: nil)
        XCTAssertTrue(mouseOnly.keyPresses.isEmpty)
        XCTAssertEqual(mouseOnly.activeSeconds, [t0 + 70])
        XCTAssertEqual(mouseOnly.injected, 1)
    }

    func testMeasuringReportsAnInputOutage() {
        let input = InputLog()
        input.markCaptureUnavailable(sinceMs: t0Ms + 30_000)
        let spec = BlockSpec(projectId: "p1", permissions: .all, blurScreenshots: false, startedAt: at(t0),
                             endedAt: at(t0 + 120), endReason: nil, taskNote: nil)
        let measured = BlockAssembler.measure(spec, input: input, usage: UsageRecorder(), windowNow: nil,
                                              runningPrograms: [], screen: nil, webcamJpeg: nil)
        XCTAssertFalse(measured.inputAvailable)
        XCTAssertNil(BlockAssembler.build(spec, measured: measured, device: sampleDevice(), zone: zone).productivity)
    }

    func testABlockForASessionUsesItsPolicy() {
        let project = MyProject(id: "p9", title: "Website", intervalMinutes: 15, allowScreenshot: true, allowWebcam: true,
                                allowKeyboard: true, allowMouse: false, allowActiveWindow: true,
                                allowRunningPrograms: false, blurScreenshots: true)
        let session = TrackingSession(project: project, localPolicy: .localDefault)
        let spec = BlockSpec(session: session, span: DateInterval(start: at(t0), end: at(t0 + 900)),
                             endReason: .project, taskNote: "Docs")
        XCTAssertEqual(spec.projectId, "p9")
        XCTAssertTrue(spec.blurScreenshots)
        XCTAssertFalse(spec.permissions.webcam, "the local default never enables the webcam")
        XCTAssertFalse(spec.permissions.mouse)
        XCTAssertEqual(spec.endedAt, at(t0 + 900))
    }
}
