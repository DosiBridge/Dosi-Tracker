import XCTest
@testable import DosiTrackerCore

/// Idle → away → return (CONTRACT §4 "Idle").
final class IdleTrackerTests: XCTestCase {
    private let threshold: TimeInterval = 5 * 60

    func testNoInputForTheThresholdCutsTheBlockAtTheLastInput() {
        var idle = IdleTracker()
        let lastInput = at(t0 + 100)
        // Not yet: 4 min 59 s of silence.
        XCTAssertEqual(idle.update(now: at(t0 + 399), lastInputAt: lastInput, trackingSince: at(t0),
                                   blockStartedAt: at(t0), threshold: threshold), .none)
        XCTAssertFalse(idle.isAway)
        // Five minutes: the block ends where the input stopped.
        XCTAssertEqual(idle.update(now: at(t0 + 400), lastInputAt: lastInput, trackingSince: at(t0),
                                   blockStartedAt: at(t0), threshold: threshold), .wentIdle(cutAt: lastInput))
        XCTAssertTrue(idle.isAway)
        XCTAssertEqual(idle.phase, .away(since: lastInput))
        // Still away: nothing new.
        XCTAssertEqual(idle.update(now: at(t0 + 900), lastInputAt: lastInput, trackingSince: at(t0),
                                   blockStartedAt: at(t0), threshold: threshold), .none)
    }

    func testReturningInputStartsANewBlockAndAsksAboutTheAwayTime() {
        var idle = IdleTracker()
        _ = idle.update(now: at(t0 + 400), lastInputAt: at(t0 + 100), trackingSince: at(t0),
                        blockStartedAt: at(t0), threshold: threshold)
        // Back at 10:47 after leaving at 10:05: 42 minutes away.
        let back = at(t0 + 100 + 42 * 60)
        let event = idle.update(now: back.addingTimeInterval(2), lastInputAt: back, trackingSince: at(t0),
                                blockStartedAt: at(t0), threshold: threshold)
        XCTAssertEqual(event, .returned(AwaySpan(start: at(t0 + 100), end: back)))
        XCTAssertFalse(idle.isAway)
        XCTAssertEqual(idle.pendingAway?.roundedMinutes, 42)
        XCTAssertEqual(idle.pendingAway?.isWorthAsking, true)
    }

    func testInactivityIsNeverCountedFromBeforeTheStretchOrTheBlock() {
        var idle = IdleTracker()
        // The last input was before a wake at t0+1000: silence counts from the wake.
        XCTAssertEqual(idle.update(now: at(t0 + 1_000 + 299), lastInputAt: at(t0 + 10), trackingSince: at(t0 + 1_000),
                                   blockStartedAt: at(t0 + 1_000), threshold: threshold), .none)
        XCTAssertEqual(idle.update(now: at(t0 + 1_300), lastInputAt: at(t0 + 10), trackingSince: at(t0 + 1_000),
                                   blockStartedAt: at(t0 + 1_000), threshold: threshold), .wentIdle(cutAt: at(t0 + 1_000)))

        // Quiet since before an interval cut: the new block is empty, cut at its start.
        var other = IdleTracker()
        XCTAssertEqual(other.update(now: at(t0 + 700), lastInputAt: at(t0 + 300), trackingSince: at(t0),
                                    blockStartedAt: at(t0 + 600), threshold: threshold), .wentIdle(cutAt: at(t0 + 600)))
    }

    func testIdleDetectionOffNeverCuts() {
        var idle = IdleTracker()
        XCTAssertEqual(idle.update(now: at(t0 + 86_400), lastInputAt: at(t0), trackingSince: at(t0),
                                   blockStartedAt: at(t0), threshold: 0), .none)
        XCTAssertFalse(idle.isAway)
    }

    func testAShortAbsenceIsNotWorthAsking() {
        var idle = IdleTracker()
        // Quiet since t0+100, but an interval cut started a block at t0+380:
        // the away stretch starts there, when idle is declared at t0+400.
        XCTAssertEqual(idle.update(now: at(t0 + 400), lastInputAt: at(t0 + 100), trackingSince: at(t0),
                                   blockStartedAt: at(t0 + 380), threshold: threshold),
                       .wentIdle(cutAt: at(t0 + 380)))
        let event = idle.update(now: at(t0 + 422), lastInputAt: at(t0 + 420), trackingSince: at(t0),
                                blockStartedAt: at(t0 + 380), threshold: threshold)
        XCTAssertEqual(event, .returned(AwaySpan(start: at(t0 + 380), end: at(t0 + 420))))
        XCTAssertNil(idle.pendingAway, "under a minute: nothing to keep")
    }

    func testKeepReturnsTheSpanOnceAndDiscardRecordsNothing() {
        var idle = IdleTracker()
        _ = idle.update(now: at(t0 + 400), lastInputAt: at(t0 + 100), trackingSince: at(t0),
                        blockStartedAt: at(t0), threshold: threshold)
        _ = idle.update(now: at(t0 + 3_000), lastInputAt: at(t0 + 2_900), trackingSince: at(t0),
                        blockStartedAt: at(t0), threshold: threshold)
        XCTAssertEqual(idle.resolve(.keep), AwaySpan(start: at(t0 + 100), end: at(t0 + 2_900)))
        XCTAssertNil(idle.resolve(.keep), "answered once")

        var other = IdleTracker()
        _ = other.update(now: at(t0 + 400), lastInputAt: at(t0 + 100), trackingSince: at(t0),
                         blockStartedAt: at(t0), threshold: threshold)
        _ = other.update(now: at(t0 + 3_000), lastInputAt: at(t0 + 2_900), trackingSince: at(t0),
                         blockStartedAt: at(t0), threshold: threshold)
        XCTAssertNil(other.resolve(.discard))
        XCTAssertNil(other.pendingAway)
    }

    func testAnUnansweredPromptIsDiscardedWhenTheUserGoesIdleAgain() {
        var idle = IdleTracker()
        _ = idle.update(now: at(t0 + 400), lastInputAt: at(t0 + 100), trackingSince: at(t0),
                        blockStartedAt: at(t0), threshold: threshold)
        _ = idle.update(now: at(t0 + 3_000), lastInputAt: at(t0 + 2_900), trackingSince: at(t0),
                        blockStartedAt: at(t0 + 2_900), threshold: threshold)
        XCTAssertNotNil(idle.pendingAway)
        _ = idle.update(now: at(t0 + 2_900 + 400), lastInputAt: at(t0 + 2_900), trackingSince: at(t0),
                        blockStartedAt: at(t0 + 2_900), threshold: threshold)
        XCTAssertTrue(idle.isAway)
        XCTAssertNil(idle.pendingAway)
    }

    func testALockOrSleepWhileAwayEndsTheAwayStretch() {
        var idle = IdleTracker()
        XCTAssertNil(idle.interrupt(at: at(t0)), "not away: nothing to end")
        _ = idle.update(now: at(t0 + 400), lastInputAt: at(t0 + 100), trackingSince: at(t0),
                        blockStartedAt: at(t0), threshold: threshold)
        XCTAssertEqual(idle.interrupt(at: at(t0 + 1_000)), AwaySpan(start: at(t0 + 100), end: at(t0 + 1_000)))
        XCTAssertFalse(idle.isAway)
        XCTAssertEqual(idle.pendingAway, AwaySpan(start: at(t0 + 100), end: at(t0 + 1_000)))
        idle.reset()
        XCTAssertNil(idle.pendingAway)
        XCTAssertEqual(idle.phase, .active)
    }

    func testKeptAwayTimeIsSplitIntoBlocksOfAtMostOneInterval() {
        let span = AwaySpan(start: at(t0), end: at(t0 + 42 * 60))
        let blocks = span.keptBlocks(interval: 600)
        XCTAssertEqual(blocks.count, 5)
        XCTAssertEqual(blocks.first?.start, at(t0))
        XCTAssertEqual(blocks.last?.end, at(t0 + 42 * 60))
        XCTAssertTrue(blocks.allSatisfy { $0.duration <= 600 })
        XCTAssertTrue(AwaySpan(start: at(t0), end: at(t0 + 59)).keptBlocks(interval: 600).isEmpty)
        XCTAssertEqual(AwaySpan(start: at(t0), end: at(t0 - 5)).duration, 0)
    }

    func testABreakResumesAtItsEndWhenTheUserIsThere() {
        let end = at(t0 + 900)
        XCTAssertEqual(IdleTracker.breakResume(now: at(t0 + 899), pauseUntil: end, threshold: threshold,
                                               lastInputAt: at(t0 + 890), awaitingReturn: false), .notYet)
        // Input a minute before the end: present.
        XCTAssertEqual(IdleTracker.breakResume(now: at(t0 + 902), pauseUntil: end, threshold: threshold,
                                               lastInputAt: at(t0 + 840), awaitingReturn: false), .resume(at: end))
        // Idle detection off: always at the end.
        XCTAssertEqual(IdleTracker.breakResume(now: at(t0 + 902), pauseUntil: end, threshold: 0,
                                               lastInputAt: nil, awaitingReturn: false), .resume(at: end))
    }

    func testABreakThatEndsWhileTheUserIsAwayResumesOnlyWhenInputReturns() {
        let end = at(t0 + 900)
        // No input for the threshold before the end: wait.
        XCTAssertEqual(IdleTracker.breakResume(now: at(t0 + 902), pauseUntil: end, threshold: threshold,
                                               lastInputAt: at(t0 + 100), awaitingReturn: false), .awaitingReturn)
        XCTAssertEqual(IdleTracker.breakResume(now: at(t0 + 902), pauseUntil: end, threshold: threshold,
                                               lastInputAt: nil, awaitingReturn: false), .awaitingReturn)
        // Still nothing new.
        XCTAssertEqual(IdleTracker.breakResume(now: at(t0 + 1_500), pauseUntil: end, threshold: threshold,
                                               lastInputAt: at(t0 + 100), awaitingReturn: true), .awaitingReturn)
        // Back at t0+2000: tracking resumes then, not at the planned end.
        XCTAssertEqual(IdleTracker.breakResume(now: at(t0 + 2_003), pauseUntil: end, threshold: threshold,
                                               lastInputAt: at(t0 + 2_000), awaitingReturn: true),
                       .resume(at: at(t0 + 2_000)))
    }

    func testABreakNeverResumesBeforeAWake() {
        // The Mac slept through the end of the break and woke at t0+5000.
        let end = at(t0 + 900)
        let wake = at(t0 + 5_000)
        XCTAssertEqual(IdleTracker.breakResume(now: at(t0 + 5_004), pauseUntil: end, threshold: threshold,
                                               lastInputAt: at(t0 + 5_001), awaitingReturn: false, notBefore: wake),
                       .resume(at: wake))
        XCTAssertEqual(IdleTracker.breakResume(now: at(t0 + 5_004), pauseUntil: end, threshold: threshold,
                                               lastInputAt: at(t0 + 800), awaitingReturn: false, notBefore: wake),
                       .awaitingReturn)
    }
}
