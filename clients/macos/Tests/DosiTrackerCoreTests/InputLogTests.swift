import XCTest
@testable import DosiTrackerCore

/// Ports of the Windows agent's `tracking/input.rs` counter, filter and
/// capture-health tests.
final class InputLogTests: XCTestCase {
    /// Feed `events` (input, injected, ms after t0) through a fresh filter.
    private func feed(_ events: [(RawInput, Bool, Int64)]) -> InputLog {
        let log = InputLog()
        let filter = InputFilter(log: log)
        for (input, injected, ms) in events {
            filter.apply(input, injected: injected, atMs: t0Ms + ms)
        }
        return log
    }

    func testActivityLogDedupesSecondsAndDropsOldEntries() {
        var log = ActivityLog()
        log.mark(t0)
        log.mark(t0) // same second: no duplicate
        log.mark(t0 + 1)
        XCTAssertEqual(log.seconds(from: t0, to: t0 + 10), [t0, t0 + 1])

        // Far in the future: everything older than the retention window goes.
        log.mark(t0 + InputLog.retentionSeconds + 10)
        XCTAssertEqual(log.seconds(from: t0 - 1, to: t0 + 2 * InputLog.retentionSeconds),
                       [t0 + InputLog.retentionSeconds + 10])
    }

    func testInputIsRememberedLongEnoughForAnIdleCutBlock() {
        // A 60-minute block ending at the last input, built when a 60-minute
        // idle threshold cuts it: its first minute is 120 minutes old by then.
        XCTAssertGreaterThanOrEqual(InputLog.retentionSeconds, 120 * 60)
        XCTAssertGreaterThanOrEqual(UsageRecorder.retentionMs, 120 * 60 * 1000)
    }

    func testCounterRespectsAllowedInputKinds() {
        let log = InputLog()
        log.markKeyboardActivity(second: t0 + 10)
        log.markMouseActivity(second: t0 + 70)
        let (start, end) = (at(t0), at(t0 + 120))

        XCTAssertEqual(log.activityPercent(from: start, to: end, keyboard: true, mouse: true), 100)
        XCTAssertEqual(log.activityPercent(from: start, to: end, keyboard: true, mouse: false), 50)
        XCTAssertEqual(log.activityPercent(from: start, to: end, keyboard: false, mouse: true), 50)
        XCTAssertEqual(log.activityPercent(from: start, to: end, keyboard: false, mouse: false), 0)
    }

    func testCountsAreTakenOverAWindow() {
        let log = InputLog()
        let now = EpochTime.milliseconds(Date())
        log.recordKeyPress(atMs: now)
        log.recordKeyPress(atMs: now)
        log.recordClick(atMs: now)
        let (before, after) = (atMs(now - 1_000), atMs(now + 1_000))
        XCTAssertEqual(log.counts(from: before, to: after), InputCounts(keyPresses: 2, clicks: 1))
        XCTAssertEqual(log.keyPresses(from: before, to: after).count, 2)
        XCTAssertEqual(log.clicks(from: before, to: after).count, 1)
        // Nothing before the window counts in it.
        XCTAssertEqual(log.counts(from: after, to: after.addingTimeInterval(5)), InputCounts(keyPresses: 0, clicks: 0))
    }

    func testEventWindowsAreHalfOpenSoBlocksNeverShareAnEvent() {
        var log = EventLog()
        for ms: Int64 in [1_000, 2_000, 3_000] {
            log.record(ms)
        }
        // Back-to-back blocks [1000, 2000) and [2000, 3001): each event once.
        XCTAssertEqual(log.between(1_000, 2_000), [1_000])
        XCTAssertEqual(log.between(2_000, 3_001), [2_000, 3_000])
        XCTAssertEqual(log.count(3_001, 9_000), 0)
        XCTAssertEqual(log.count(5_000, 1_000), 0) // inverted window
    }

    func testEventLogStaysSortedAndBounded() {
        var log = EventLog()
        log.record(10_000)
        log.record(9_000) // clock stepped back: filed with the newest
        XCTAssertEqual(log.between(0, 20_000), [10_000, 10_000])

        // Far in the future: everything older than the retention window goes.
        let later = 10_000 + InputLog.retentionSeconds * 1000 + 1
        log.record(later)
        XCTAssertEqual(log.between(0, Int64.max), [later])
    }

    func testAHeldKeyCountsOnceHoweverLongItRepeats() {
        // Held for 3 s: a 500 ms delay, then auto-repeat every 33 ms.
        var events: [(RawInput, Bool, Int64)] = [(.keyDown(0x00), false, 0)]
        events += (0..<76).map { (RawInput.keyDown(0x00), false, 500 + 33 * Int64($0)) }
        events.append((.keyUp(0x00), false, 3_000))
        // Pressed again: a new press.
        events.append((.keyDown(0x00), false, 3_200))
        let log = feed(events)
        XCTAssertEqual(log.counts(from: at(t0), to: at(t0 + 10)), InputCounts(keyPresses: 2, clicks: 0))
        // Only the seconds of real presses are active, not the held stretch.
        XCTAssertEqual(log.activeSeconds(from: at(t0), to: at(t0 + 10), keyboard: true, mouse: true), [t0, t0 + 3])
    }

    func testRepeatsFlaggedByTheSystemNeverCountAndKeepTheKeyHeld() {
        // macOS marks auto-repeat: even after a long initial delay (2 s, over
        // the missed-key-up re-arm) the repeats are not new presses.
        var events: [(RawInput, Bool, Int64)] = [(.keyDown(0x31), false, 0)]
        events += (0..<20).map { (RawInput.keyRepeat(0x31), false, 2_000 + 50 * Int64($0)) }
        events.append((.keyUp(0x31), false, 3_100))
        let log = feed(events)
        XCTAssertEqual(log.counts(from: at(t0), to: at(t0 + 10)).keyPresses, 1)
        XCTAssertEqual(log.activeSeconds(from: at(t0), to: at(t0 + 10), keyboard: true, mouse: false), [t0])
    }

    func testDifferentKeysAndChordsCountPerKey() {
        let log = feed([
            (.keyDown(0x3B), false, 0), // Control
            (.keyDown(0x08), false, 40), // C
            (.keyUp(0x08), false, 90),
            (.keyDown(0x09), false, 300), // V, Control still held
            (.keyDown(0x3B), false, 330), // Control again while held
            (.keyUp(0x09), false, 380),
            (.keyUp(0x3B), false, 400),
        ])
        XCTAssertEqual(log.counts(from: at(t0), to: at(t0 + 1)).keyPresses, 3)
    }

    func testAKeyWhoseReleaseWasMissedCountsAgainAfterAPause() {
        // The key-up happened while another app held the keyboard, unseen.
        let log = feed([
            (.keyDown(0x3B), false, 0),
            (.keyDown(0x3B), false, 30_000),
        ])
        XCTAssertEqual(log.counts(from: at(t0), to: at(t0 + 60)).keyPresses, 2)
    }

    func testClicksCountOnButtonDownAndMovesOnlyMarkActivity() {
        let log = feed([
            (.buttonDown, false, 0),
            (.buttonUp, false, 80),
            (.move, false, 61_000),
            (.wheel, false, 125_000),
        ])
        XCTAssertEqual(log.counts(from: at(t0), to: at(t0 + 180)), InputCounts(keyPresses: 0, clicks: 1))
        XCTAssertEqual(log.activeSeconds(from: at(t0), to: at(t0 + 180), keyboard: false, mouse: true),
                       [t0, t0 + 61, t0 + 125])
        XCTAssertEqual(log.activityPercent(from: at(t0), to: at(t0 + 180), keyboard: true, mouse: true), 100)
        // Mouse activity does not count when only the keyboard is recorded.
        XCTAssertEqual(log.activityPercent(from: at(t0), to: at(t0 + 180), keyboard: true, mouse: false), 0)
    }

    func testInjectedInputIsReportedAsideAndNeverCounted() {
        // A mouse jiggler and a remote tool typing.
        var events: [(RawInput, Bool, Int64)] = (0..<10).map { (RawInput.move, true, Int64($0) * 30_000) }
        events.append((.keyDown(0x00), true, 1_000))
        events.append((.keyUp(0x00), true, 1_050))
        events.append((.buttonDown, true, 2_000))
        let log = feed(events)

        let (start, end) = (at(t0), at(t0 + 300))
        XCTAssertEqual(log.counts(from: start, to: end), InputCounts(keyPresses: 0, clicks: 0))
        XCTAssertEqual(log.activityPercent(from: start, to: end, keyboard: true, mouse: true), 0)
        XCTAssertEqual(log.injectedCount(from: start, to: end, keyboard: true, mouse: true), 12)
        XCTAssertEqual(log.injectedCount(from: start, to: end, keyboard: true, mouse: false), 1)
        XCTAssertEqual(log.injectedCount(from: start, to: end, keyboard: false, mouse: true), 11)
        // Whole seconds, half-open: the block before or after sees none of them.
        XCTAssertEqual(log.injectedCount(from: at(t0 - 60), to: at(t0), keyboard: true, mouse: true), 0)
        XCTAssertEqual(log.injectedCount(from: at(t0 + 271), to: at(t0 + 400), keyboard: true, mouse: true), 0)
        XCTAssertNil(log.lastActivity, "injected input is not the user's")

        // A real press of the same key still counts.
        InputFilter(log: log).apply(.keyDown(0x00), injected: false, atMs: t0Ms + 5_000)
        XCTAssertEqual(log.counts(from: start, to: end), InputCounts(keyPresses: 1, clicks: 0))
    }

    func testInjectedCountsAreExactUnderAFloodAndBoundedInMemory() {
        var log = CountLog()
        for i in 0..<100_000 {
            log.add(t0 + Int64(i / 1_000)) // 1,000 events a second for 100 s
        }
        XCTAssertEqual(log.count(t0, t0 + 100), 100_000)
        XCTAssertEqual(log.count(t0 + 10, t0 + 20), 10_000)
        log.add(t0 + 50) // clock stepped back: filed with the newest second
        XCTAssertEqual(log.count(t0 + 99, t0 + 100), 1_001)
        log.add(t0 + InputLog.retentionSeconds + 200)
        XCTAssertEqual(log.count(t0, t0 + 100), 0, "older than the retention window")
        XCTAssertLessThanOrEqual(log.secondsKept, 2)
    }

    func testCaptureIsUnavailableForBlocksOverlappingAnOutage() {
        let log = InputLog()
        XCTAssertTrue(log.isCaptureHealthy)
        XCTAssertTrue(log.isCaptureAvailable(from: at(t0), to: at(t0 + 600)))

        // Silent from minute 5; found out later, recovered at minute 20.
        log.markCaptureUnavailable(sinceMs: t0Ms + 300_000)
        XCTAssertFalse(log.isCaptureHealthy)
        XCTAssertTrue(log.isCaptureAvailable(from: at(t0), to: at(t0 + 300)), "ended before the outage")
        XCTAssertFalse(log.isCaptureAvailable(from: at(t0), to: at(t0 + 600)))
        // A second report keeps the earlier start.
        log.markCaptureUnavailable(sinceMs: t0Ms + 500_000)
        XCTAssertFalse(log.isCaptureAvailable(from: at(t0 + 320), to: at(t0 + 480)))

        InputFilter(log: log).apply(.move, injected: false, atMs: t0Ms + 1_200_000)
        XCTAssertTrue(log.isCaptureHealthy)
        XCTAssertFalse(log.isCaptureAvailable(from: at(t0 + 600), to: at(t0 + 1_200)), "inside the outage")
        XCTAssertTrue(log.isCaptureAvailable(from: at(t0 + 1_200), to: at(t0 + 1_800)), "after it")
        XCTAssertTrue(log.isCaptureAvailable(from: at(t0 - 600), to: at(t0)), "before it")
    }

    func testMissingCaptureMeansInputIsNeverAvailable() {
        let log = InputLog()
        log.markCaptureUnavailable(sinceMs: 1)
        XCTAssertFalse(log.isCaptureHealthy)
        XCTAssertFalse(log.isCaptureAvailable(from: at(t0), to: at(t0 + 60)))
    }

    func testEveryEventKeepsTheCaptureAlive() {
        let log = InputLog()
        log.captureStarted(atMs: t0Ms)
        XCTAssertEqual(log.lastCaptureEventMs, t0Ms)
        // Even injected input proves the tap delivers.
        InputFilter(log: log).apply(.move, injected: true, atMs: t0Ms + 7_000)
        XCTAssertEqual(log.lastCaptureEventMs, t0Ms + 7_000)
    }

    func testTheWatchdogRestartsOnlyASilentCaptureWhileTheSystemSeesInput() {
        let limit = InputCaptureWatchdog.silenceLimitMs
        let now = t0Ms + 10 * limit
        let longAgo = now - 2 * limit
        // Silent for longer than the limit while the user typed a moment ago.
        XCTAssertTrue(InputCaptureWatchdog.needsRestart(nowMs: now, running: true, lastCaptureEventMs: longAgo,
                                                        systemIdleMs: 5_000, lastAttemptMs: longAgo))
        // The user is away: silence is expected.
        XCTAssertFalse(InputCaptureWatchdog.needsRestart(nowMs: now, running: true, lastCaptureEventMs: longAgo,
                                                         systemIdleMs: 10 * 60_000, lastAttemptMs: longAgo))
        // The capture delivered recently.
        XCTAssertFalse(InputCaptureWatchdog.needsRestart(nowMs: now, running: true, lastCaptureEventMs: now - 10_000,
                                                         systemIdleMs: 5_000, lastAttemptMs: longAgo))
        // The system will not say: no evidence, no action.
        XCTAssertFalse(InputCaptureWatchdog.needsRestart(nowMs: now, running: true, lastCaptureEventMs: longAgo,
                                                         systemIdleMs: nil, lastAttemptMs: longAgo))
        // A missing capture is retried — but at most once per limit.
        XCTAssertTrue(InputCaptureWatchdog.needsRestart(nowMs: now, running: false, lastCaptureEventMs: now,
                                                        systemIdleMs: nil, lastAttemptMs: longAgo))
        XCTAssertFalse(InputCaptureWatchdog.needsRestart(nowMs: now, running: false, lastCaptureEventMs: now,
                                                         systemIdleMs: nil, lastAttemptMs: now - 60_000))
        XCTAssertFalse(InputCaptureWatchdog.needsRestart(nowMs: now, running: true, lastCaptureEventMs: longAgo,
                                                         systemIdleMs: 5_000, lastAttemptMs: now - 60_000))
    }

    func testReturnHelpersSeeOnlyRealInput() {
        let log = InputLog()
        XCTAssertNil(log.lastActivity)
        XCTAssertNil(log.firstActivity(after: at(t0)))
        log.recordPointerActivity(atMs: t0Ms + 30_500)
        log.recordKeyPress(atMs: t0Ms + 90_000)
        XCTAssertEqual(log.lastActivity, atMs(t0Ms + 90_000))
        XCTAssertTrue(log.hasActivity(from: at(t0), to: at(t0 + 30)))
        XCTAssertFalse(log.hasActivity(from: at(t0 + 31), to: at(t0 + 89)))
        XCTAssertEqual(log.firstActivity(after: at(t0)), at(t0 + 30))
        XCTAssertEqual(log.firstActivity(after: at(t0 + 30)), at(t0 + 90))
        XCTAssertNil(log.firstActivity(after: at(t0 + 90)))
        // An older event does not move the latest input back.
        log.recordClick(atMs: t0Ms + 10_000)
        XCTAssertEqual(log.lastActivity, atMs(t0Ms + 90_000))
    }

    func testRecordingIsSafeFromSeveralThreads() {
        let log = InputLog()
        let filter = InputFilter(log: log)
        DispatchQueue.concurrentPerform(iterations: 8) { worker in
            for i in 0..<500 {
                let ms = t0Ms + Int64(worker * 1_000_000 + i * 100)
                filter.apply(.buttonDown, injected: false, atMs: ms)
                _ = log.counts(from: at(t0), to: at(t0 + 100_000))
            }
        }
        XCTAssertEqual(log.counts(from: at(t0), to: at(t0 + 100_000)).clicks, 4_000)
    }
}
