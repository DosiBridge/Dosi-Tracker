import XCTest
@testable import DosiTrackerCore

/// Block timing (ports of the Windows worker's `record_block` and `Clocks`
/// tests and of `split_span`), the capture plan and sleep-gap detection.
final class BlockClockTests: XCTestCase {
    func testIntervalsAreClampedToFiveToSixtyMinutes() {
        XCTAssertEqual(BlockClock.clampedInterval(minutes: 1), 300)
        XCTAssertEqual(BlockClock.clampedInterval(minutes: 10), 600)
        XCTAssertEqual(BlockClock.clampedInterval(minutes: 600), 3600)
        XCTAssertEqual(BlockClock.clampedInterval(minutes: -5), 300)
    }

    func testABlockNeverClaimsMoreThanOneInterval() {
        // Asleep for hours: the block is the last interval before the cut.
        let span = BlockClock.span(blockStartedAt: at(t0), endingAt: at(t0 + 4 * 3600), interval: 600)
        XCTAssertEqual(span, DateInterval(start: at(t0 + 4 * 3600 - 600), end: at(t0 + 4 * 3600)))
        // A normal cut keeps the real start.
        XCTAssertEqual(BlockClock.span(blockStartedAt: at(t0), endingAt: at(t0 + 420), interval: 600),
                       DateInterval(start: at(t0), end: at(t0 + 420)))
    }

    func testDegenerateWindowsAreSkipped() {
        // The clock jumped backwards, or a double cut.
        XCTAssertNil(BlockClock.span(blockStartedAt: at(t0 + 10), endingAt: at(t0), interval: 600))
        XCTAssertNil(BlockClock.span(blockStartedAt: at(t0), endingAt: at(t0), interval: 600))
    }

    func testBlocksUnderAMinuteAreNotRecorded() {
        XCTAssertFalse(BlockClock.isRecordable(DateInterval(start: at(t0), end: at(t0 + 59))))
        XCTAssertTrue(BlockClock.isRecordable(DateInterval(start: at(t0), end: at(t0 + 60))))
        XCTAssertNil(BlockClock.recordableSpan(blockStartedAt: at(t0), endingAt: at(t0 + 59), interval: 600))
        XCTAssertEqual(BlockClock.recordableSpan(blockStartedAt: at(t0), endingAt: at(t0 + 61), interval: 600),
                       DateInterval(start: at(t0), end: at(t0 + 61)))
        XCTAssertEqual(BlockClock.nextCut(blockStartedAt: at(t0), interval: 600), at(t0 + 600))
    }

    func testSpansSplitIntoEqualPiecesOfAtMostOneInterval() {
        // 42 minutes: five pieces of 8 min 24 s, back to back.
        let pieces = BlockClock.splitSpan(from: at(t0), to: at(t0 + 42 * 60), maxLength: 600)
        XCTAssertEqual(pieces.count, 5)
        XCTAssertEqual(pieces.first?.start, at(t0))
        XCTAssertEqual(pieces.last?.end, at(t0 + 42 * 60))
        XCTAssertTrue(zip(pieces, pieces.dropFirst()).allSatisfy { $0.end == $1.start }, "no gaps")
        XCTAssertTrue(pieces.allSatisfy { $0.duration <= 600 && $0.duration >= 8 * 60 })
        // Exactly one interval: one piece. Under a minute: nothing.
        XCTAssertEqual(BlockClock.splitSpan(from: at(t0), to: at(t0 + 600), maxLength: 600).count, 1)
        XCTAssertTrue(BlockClock.splitSpan(from: at(t0), to: at(t0 + 59), maxLength: 600).isEmpty)
        XCTAssertTrue(BlockClock.splitSpan(from: at(t0), to: at(t0 + 600), maxLength: 0).isEmpty)
        XCTAssertTrue(BlockClock.splitSpan(from: at(t0 + 600), to: at(t0), maxLength: 600).isEmpty)
    }

    func testTheCaptureMomentIsBetweenTenAndNinetyPercentOfTheInterval() {
        var generator = SplitMix(seed: 42)
        for _ in 0..<500 {
            let plan = CapturePlan.random(blockStartedAt: at(t0), interval: 600, using: &generator)
            let offset = plan.captureAt.timeIntervalSince(at(t0))
            XCTAssertGreaterThanOrEqual(offset, 60)
            XCTAssertLessThanOrEqual(offset, 540)
        }
        // Fractions outside the range are clamped.
        XCTAssertEqual(CapturePlan(blockStartedAt: at(t0), interval: 600, fraction: 0).captureAt, at(t0 + 60))
        XCTAssertEqual(CapturePlan(blockStartedAt: at(t0), interval: 600, fraction: 2).captureAt, at(t0 + 540))
        let plan = CapturePlan(blockStartedAt: at(t0), interval: 600, fraction: 0.5)
        XCTAssertFalse(plan.isDue(at: at(t0 + 299)))
        XCTAssertTrue(plan.isDue(at: at(t0 + 300)))
        // The system generator stays within the range too.
        let random = CapturePlan.random(blockStartedAt: at(t0), interval: 600)
        XCTAssertTrue((at(t0 + 60)...at(t0 + 540)).contains(random.captureAt))
    }

    func testABlockEndingEarlyTakesItsCaptureAtTheEndUnlessLockedAsleepIdleOrShuttingDown() {
        let span = DateInterval(start: at(t0), end: at(t0 + 120))
        XCTAssertTrue(CapturePlan.capturesAtEnd(endReason: .pause, span: span, alreadyCaptured: false))
        XCTAssertTrue(CapturePlan.capturesAtEnd(endReason: nil, span: span, alreadyCaptured: false))
        XCTAssertFalse(CapturePlan.capturesAtEnd(endReason: .pause, span: span, alreadyCaptured: true))
        for reason in [EndReason.lock, .sleep, .idle, .shutdown] {
            XCTAssertFalse(CapturePlan.capturesAtEnd(endReason: reason, span: span, alreadyCaptured: false), "\(reason)")
        }
        // Too short to record: no capture either.
        XCTAssertFalse(CapturePlan.capturesAtEnd(endReason: .quit, span: DateInterval(start: at(t0), end: at(t0 + 30)),
                                                 alreadyCaptured: false))
        XCTAssertTrue(CapturePlan.belongs(capturedAt: at(t0 + 60), to: span))
        XCTAssertFalse(CapturePlan.belongs(capturedAt: at(t0 + 121), to: span))
    }

    func testOnlyLockSleepIdleAndShutdownForbidACaptureAtTheEnd() {
        let forbidden = EndReason.allCases.filter { !$0.allowsCaptureAtEnd }
        XCTAssertEqual(Set(forbidden), [.idle, .lock, .sleep, .shutdown])
    }

    func testStartingAStretchResetsTheClocksAndClearsAPause() {
        var clocks = TrackingClocks(now: utc("2026-09-30T09:00:00Z"), interval: 300)
        clocks.pause(at: utc("2026-09-30T10:00:00Z"), until: utc("2026-09-30T10:30:00Z"))
        XCTAssertTrue(clocks.isPaused && clocks.isOnBreak)

        let now = utc("2026-09-30T11:00:00Z")
        clocks.startTracking(at: now)
        XCTAssertFalse(clocks.isPaused)
        XCTAssertNil(clocks.pauseUntil)
        XCTAssertEqual(clocks.trackingSince, now)
        XCTAssertEqual(clocks.blockStartedAt, now)
        XCTAssertEqual(clocks.nextCutAt.timeIntervalSince(clocks.blockStartedAt), 300)
    }

    func testAPauseKeepsItsStartAndOnlyMovesItsEnd() {
        var clocks = TrackingClocks(now: at(t0), interval: 600)
        clocks.pause(at: at(t0 + 100), until: at(t0 + 1_000))
        clocks.pause(at: at(t0 + 200), until: nil)
        XCTAssertEqual(clocks.pausedSince, at(t0 + 100))
        XCTAssertNil(clocks.pauseUntil)
        XCTAssertFalse(clocks.isOnBreak)
        XCTAssertFalse(clocks.isCutDue(at: at(t0 + 5_000)), "nothing is cut while paused")
        XCTAssertEqual(clocks.persistedPause, PauseRecord(since: at(t0 + 100), until: nil))

        var restored = TrackingClocks(now: at(t0 + 9_000), interval: 600)
        restored.restore(PauseRecord(since: at(t0 + 100), until: at(t0 + 9_999)))
        XCTAssertTrue(restored.isPaused && restored.isOnBreak)
        XCTAssertNil(TrackingClocks(now: at(t0), interval: 600).persistedPause)
    }

    func testANewIntervalAppliesFromTheNextBlockOn() {
        var clocks = TrackingClocks(now: at(t0), interval: 600)
        clocks.setInterval(300)
        XCTAssertEqual(clocks.blockInterval, 600, "the block in progress keeps its interval")
        XCTAssertEqual(clocks.nextCutAt, at(t0 + 600))
        XCTAssertFalse(clocks.isCutDue(at: at(t0 + 599)))
        XCTAssertTrue(clocks.isCutDue(at: at(t0 + 600)))

        // The next block starts where this one ended.
        clocks.blockEnded(at: at(t0 + 600))
        XCTAssertEqual(clocks.blockStartedAt, at(t0 + 600))
        XCTAssertEqual(clocks.blockInterval, 300)
        XCTAssertEqual(clocks.nextCutAt, at(t0 + 900))
        XCTAssertEqual(clocks.trackingSince, at(t0), "the session timer keeps running")
    }

    func testAMissedSleepIsDetectedFromTheWallAndMonotonicClocks() {
        var detector = SleepGapDetector()
        XCTAssertEqual(detector.tick(wall: at(t0), monotonic: 1_000), .steady)
        XCTAssertEqual(detector.tick(wall: at(t0 + 5), monotonic: 1_005), .steady)
        // A slow tick is not a sleep.
        XCTAssertEqual(detector.tick(wall: at(t0 + 65), monotonic: 1_065), .steady)
        // 40 minutes of wall time, 5 seconds of uptime: the Mac slept.
        XCTAssertEqual(detector.tick(wall: at(t0 + 65 + 2_400), monotonic: 1_070),
                       .gap(lastTickAt: at(t0 + 65), resumedAt: at(t0 + 2_465), missing: 2_395))
        // Within the threshold: steady.
        XCTAssertEqual(detector.tick(wall: at(t0 + 2_465 + 95), monotonic: 1_080), .steady)
        // The wall clock stepped back by 10 minutes.
        XCTAssertEqual(detector.tick(wall: at(t0 + 2_560 - 600), monotonic: 1_085), .clockSteppedBack(seconds: 605))
        detector.reset()
        XCTAssertEqual(detector.tick(wall: at(t0 + 99_999), monotonic: 1), .steady, "nothing to compare after a reset")
    }
}

/// A small deterministic generator (SplitMix64) for repeatable random tests.
struct SplitMix: RandomNumberGenerator {
    private var state: UInt64

    init(seed: UInt64) {
        state = seed
    }

    mutating func next() -> UInt64 {
        state &+= 0x9E37_79B9_7F4A_7C15
        var z = state
        z = (z ^ (z >> 30)) &* 0xBF58_476D_1CE4_E5B9
        z = (z ^ (z >> 27)) &* 0x94D0_49BB_1331_11EB
        return z ^ (z >> 31)
    }
}
