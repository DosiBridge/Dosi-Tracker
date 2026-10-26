import DosiTrackerCore
import XCTest
@testable import DosiTracker

/// Ports of the Windows agent's `worker/blocks.rs` tests, against the
/// `BlockTimeline` the app actually runs.
final class BlockTimelineTests: XCTestCase {
    private func at(_ hms: String) -> Date {
        let formatter = ISO8601DateFormatter()
        formatter.formatOptions = [.withInternetDateTime, .withFractionalSeconds]
        if let date = formatter.date(from: "2026-10-05T\(hms)Z") {
            return date
        }
        formatter.formatOptions = [.withInternetDateTime]
        return formatter.date(from: "2026-10-05T\(hms)Z")!
    }

    private func policy(_ intervalMinutes: Double, idle idleMinutes: Double?, screenshot: Bool = true) -> BlockPolicy {
        BlockPolicy(projectId: "p1",
                    permissions: CapturePermissions(screenshot: screenshot, webcam: false, keyboard: true, mouse: true,
                                                    activeWindow: true, runningPrograms: false),
                    blurScreenshots: false, interval: intervalMinutes * 60, idleThreshold: idleMinutes.map { $0 * 60 })
    }

    private func started(_ intervalMinutes: Double, idle idleMinutes: Double?) -> BlockTimeline {
        var blocks = BlockTimeline()
        blocks.start(at: at("10:00:00"), policy: policy(intervalMinutes, idle: idleMinutes), description: "Invoices",
                     fraction: 0.5)
        return blocks
    }

    private struct Span: Equatable {
        let start: Date
        let end: Date
        let reason: EndReason
    }

    private func spans(_ blocks: [FinishedBlock]) -> [Span] {
        blocks.map { Span(start: $0.startedAt, end: $0.endedAt, reason: $0.endReason) }
    }

    private func span(_ start: String, _ end: String, _ reason: EndReason) -> Span {
        Span(start: at(start), end: at(end), reason: reason)
    }

    private let mid: () -> Double = { 0.5 }

    func testTrackingBlocksFollowEachOtherAndNeverExceedTheInterval() {
        var blocks = started(10, idle: 5)
        // Busy throughout: input a moment ago at every tick.
        let outcome = blocks.tick(now: at("10:25:00"), lastActivity: at("10:24:59"), fraction: mid)
        XCTAssertEqual(spans(outcome.finished), [span("10:00:00", "10:10:00", .interval),
                                                 span("10:10:00", "10:20:00", .interval)])
        XCTAssertEqual(blocks.stretch?.block.startedAt, at("10:20:00"), "the next block starts where the last ended")
        XCTAssertEqual(blocks.stretch?.since, at("10:00:00"), "one stretch")
        XCTAssertNil(outcome.wentIdle)
        XCTAssertTrue(outcome.finished.allSatisfy { $0.description == "Invoices" })
    }

    func testABlockEndingWhileTheUserIsInactiveWaitsForThem() {
        var blocks = started(10, idle: 30)
        // Reading since 10:08: the 10:10 boundary finds no input.
        let outcome = blocks.tick(now: at("10:10:02"), lastActivity: at("10:08:00"), fraction: mid)
        XCTAssertTrue(outcome.finished.isEmpty, "held, not queued yet")
        XCTAssertEqual(blocks.stretch?.holding, true)
        XCTAssertEqual(blocks.stretch?.unsavedSince, at("10:00:00"))
        XCTAssertEqual(blocks.stretch?.block.startedAt, at("10:10:00"), "the next block runs meanwhile")

        // Still no input: nothing changes.
        XCTAssertTrue(blocks.tick(now: at("10:15:00"), lastActivity: at("10:08:00"), fraction: mid).finished.isEmpty)

        // A key press: it was work after all.
        let back = blocks.tick(now: at("10:16:00"), lastActivity: at("10:15:30"), fraction: mid)
        XCTAssertEqual(spans(back.finished), [span("10:00:00", "10:10:00", .interval)])
        XCTAssertEqual(blocks.stretch?.holding, false)
    }

    func testGoingIdleCutsAtTheLastInputEvenAcrossHeldBlocks() {
        var blocks = started(10, idle: 30)
        let lastInput = at("10:01:00")
        for tick in ["10:10:01", "10:20:01", "10:30:01"] {
            let outcome = blocks.tick(now: at(tick), lastActivity: lastInput, fraction: mid)
            XCTAssertTrue(outcome.finished.isEmpty && outcome.wentIdle == nil, "\(tick): held")
        }
        // Thirty minutes without input: the user left at 10:01.
        let outcome = blocks.tick(now: at("10:31:00"), lastActivity: lastInput, fraction: mid)
        XCTAssertEqual(outcome.wentIdle, at("10:01:00"))
        XCTAssertEqual(spans(outcome.finished), [span("10:00:00", "10:01:00", .idle)])
        XCTAssertFalse(outcome.finished[0].captureAtEnd, "no capture for an idle end")
        XCTAssertFalse(blocks.isOpen, "nothing is tracked while away")
    }

    func testAnIdleBlockUnderAMinuteIsDropped() {
        var blocks = started(10, idle: 5)
        let outcome = blocks.tick(now: at("10:05:40"), lastActivity: at("10:00:30"), fraction: mid)
        XCTAssertEqual(outcome.wentIdle, at("10:00:30"))
        XCTAssertTrue(outcome.finished.isEmpty)
        XCTAssertFalse(blocks.isOpen)
    }

    func testNoInputAtAllSinceTheStartCutsAtTheStart() {
        var blocks = started(10, idle: 5)
        let outcome = blocks.tick(now: at("10:05:00"), lastActivity: at("09:30:00"), fraction: mid)
        XCTAssertEqual(outcome.wentIdle, at("10:00:00"), "input before the stretch does not count")
        XCTAssertTrue(outcome.finished.isEmpty)
        var none = started(10, idle: 5)
        XCTAssertEqual(none.tick(now: at("10:05:00"), lastActivity: nil, fraction: mid).wentIdle, at("10:00:00"))
    }

    func testWithIdleDetectionOffNothingIsHeldOrCut() {
        var blocks = started(10, idle: nil)
        let outcome = blocks.tick(now: at("11:00:01"), lastActivity: at("10:00:05"), fraction: mid)
        XCTAssertEqual(outcome.finished.count, 6)
        XCTAssertNil(outcome.wentIdle)
        XCTAssertEqual(blocks.stretch?.holding, false)
    }

    func testHeldBlocksAreReleasedWhenIdleDetectionIsTurnedOff() {
        var blocks = started(10, idle: 30)
        _ = blocks.tick(now: at("10:10:01"), lastActivity: at("10:02:00"), fraction: mid)
        XCTAssertEqual(blocks.stretch?.holding, true)
        // The open block keeps its own policy, the next one gets the new one.
        blocks.updatePolicy(policy(10, idle: nil))
        XCTAssertTrue(blocks.tick(now: at("10:15:00"), lastActivity: at("10:02:00"), fraction: mid).finished.isEmpty)
        // The 10:10-10:20 block was still under the old policy: held too.
        XCTAssertTrue(blocks.tick(now: at("10:20:01"), lastActivity: at("10:02:00"), fraction: mid).finished.isEmpty)
        let outcome = blocks.tick(now: at("10:20:02"), lastActivity: at("10:02:00"), fraction: mid)
        XCTAssertEqual(outcome.finished.count, 2, "released once the open block has no idle threshold")
        XCTAssertNil(blocks.stretch?.block.policy.idleThreshold)
    }

    func testTheUserActionEndsTheBlockAtItsOwnTimestamp() {
        var blocks = started(10, idle: 5)
        // Paused at 10:04:30; the worker got to it two seconds later.
        let finished = blocks.end(at: at("10:04:30"), now: at("10:04:32"), reason: .pause)
        XCTAssertEqual(spans(finished), [span("10:00:00", "10:04:30", .pause)])
        // The screenshot moment (50% of the interval: 10:05) was not reached.
        XCTAssertTrue(finished[0].captureAtEnd)
        XCTAssertFalse(blocks.isOpen)
        XCTAssertTrue(blocks.end(at: at("10:06:00"), now: at("10:06:00"), reason: .pause).isEmpty, "nothing open")
    }

    func testAnActionAfterAMissedBoundaryNeverExceedsTheInterval() {
        var blocks = started(10, idle: 5)
        let finished = blocks.end(at: at("10:12:00"), now: at("10:12:01"), reason: .quit)
        XCTAssertEqual(spans(finished), [span("10:00:00", "10:10:00", .interval), span("10:10:00", "10:12:00", .quit)])
    }

    func testAClickJustBeforeABoundaryTheWorkerAlreadyHandledDropsTheSliver() {
        var blocks = started(10, idle: 5)
        _ = blocks.tick(now: at("10:10:00"), lastActivity: at("10:09:59"), fraction: mid)
        // Pause clicked at 10:09:59.5, reaching the worker after the boundary.
        XCTAssertTrue(blocks.end(at: at("10:09:59.500"), now: at("10:10:01"), reason: .pause).isEmpty)
    }

    func testAFutureTimestampIsClampedToNow() {
        var blocks = started(10, idle: 5)
        let finished = blocks.end(at: at("10:30:00"), now: at("10:03:00"), reason: .lock)
        XCTAssertEqual(spans(finished), [span("10:00:00", "10:03:00", .lock)])
    }

    func testStoppingKeepsHeldBlocksAsWork() {
        var blocks = started(10, idle: 30)
        _ = blocks.tick(now: at("10:10:01"), lastActivity: at("10:03:00"), fraction: mid)
        // The screen locks at 10:12, within the idle threshold.
        let finished = blocks.end(at: at("10:12:00"), now: at("10:12:00"), reason: .lock)
        XCTAssertEqual(spans(finished), [span("10:00:00", "10:10:00", .interval), span("10:10:00", "10:12:00", .lock)])
    }

    func testShortBlocksAreDroppedWhenStopping() {
        var blocks = started(10, idle: 5)
        XCTAssertTrue(blocks.end(at: at("10:00:59"), now: at("10:00:59"), reason: .project).isEmpty)
        var minute = started(10, idle: 5)
        XCTAssertEqual(minute.end(at: at("10:01:00"), now: at("10:01:00"), reason: .project).count, 1,
                       "a full minute is kept")
    }

    func testACutEndsTheBlockAndTheStretchGoesOn() {
        var blocks = started(10, idle: 5)
        blocks.setDescription("Client call", relabelOpen: false)
        let finished = blocks.cut(at: at("10:04:00"), now: at("10:04:01"), reason: .task, fraction: 0.5)
        XCTAssertEqual(spans(finished), [span("10:00:00", "10:04:00", .task)])
        XCTAssertEqual(finished[0].description, "Invoices", "the old note stays with its block")
        XCTAssertEqual(blocks.stretch?.since, at("10:00:00"))
        XCTAssertEqual(blocks.stretch?.block.startedAt, at("10:04:00"))
        XCTAssertEqual(blocks.stretch?.block.description, "Client call")
        XCTAssertEqual(blocks.stretch?.block.plannedEnd, at("10:14:00"), "a full interval from the cut")
    }

    func testAShortBlockCanTakeTheNewNoteInsteadOfEnding() {
        var blocks = started(10, idle: 5)
        blocks.setDescription("Client call", relabelOpen: true)
        XCTAssertEqual(blocks.stretch?.block.startedAt, at("10:00:00"))
        XCTAssertEqual(blocks.stretch?.block.description, "Client call")
    }

    func testARefreshedPolicyAppliesFromTheNextBlock() {
        var blocks = started(10, idle: 5)
        blocks.updatePolicy(policy(5, idle: 5, screenshot: false))
        let outcome = blocks.tick(now: at("10:10:00"), lastActivity: at("10:09:59"), fraction: mid)
        XCTAssertEqual(outcome.finished[0].policy.interval, 600, "the open block keeps its policy")
        XCTAssertEqual(blocks.stretch?.block.policy.interval, 300)
        XCTAssertEqual(blocks.stretch?.block.plannedEnd, at("10:15:00"))
    }

    func testAChangedConsentAdjustsTheOpenBlockAndMeasuredInputOnlyNarrows() throws {
        var blocks = started(10, idle: 5)
        var narrower = policy(10, idle: 5, screenshot: false)
        narrower.permissions.keyboard = false
        narrower.blurScreenshots = true
        blocks.adjustOpenBlock(to: narrower)
        let open = try XCTUnwrap(blocks.stretch?.block)
        XCTAssertEqual(open.startedAt, at("10:00:00"), "its timing stays")
        XCTAssertEqual(open.plannedEnd, at("10:10:00"))
        XCTAssertEqual(open.captureAt, at("10:05:00"))
        XCTAssertEqual(open.description, "Invoices")
        XCTAssertFalse(open.policy.permissions.keyboard)
        XCTAssertFalse(open.policy.permissions.screenshot)
        XCTAssertTrue(open.policy.permissions.mouse)
        XCTAssertTrue(open.policy.blurScreenshots)
        XCTAssertNil(blocks.captureDue(now: at("10:06:00")), "no screenshot once it is withdrawn")

        // Consent given again: a screenshot (taken after the change) may be
        // taken again; the keyboard, unmeasured so far, stays unmeasured, and
        // the blur stays.
        var wider = policy(10, idle: 5)
        wider.permissions.runningPrograms = true
        blocks.adjustOpenBlock(to: wider)
        let adjusted = try XCTUnwrap(blocks.stretch?.block.policy)
        XCTAssertFalse(adjusted.permissions.keyboard)
        XCTAssertTrue(adjusted.permissions.screenshot)
        XCTAssertTrue(adjusted.permissions.runningPrograms)
        XCTAssertTrue(adjusted.blurScreenshots)
        XCTAssertNotNil(blocks.captureDue(now: at("10:06:00")))

        var stopped = BlockTimeline()
        stopped.adjustOpenBlock(to: narrower)
        XCTAssertNil(stopped.stretch, "nothing to adjust while not tracking")
    }

    func testTheScreenshotMomentIsRandomWithinTenToNinetyPercent() {
        let start = at("10:00:00")
        XCTAssertEqual(BlockTimeline.captureMoment(startedAt: start, interval: 600, fraction: 0), at("10:01:00"))
        XCTAssertEqual(BlockTimeline.captureMoment(startedAt: start, interval: 600, fraction: 1), at("10:09:00"))
        XCTAssertEqual(BlockTimeline.captureMoment(startedAt: start, interval: 600, fraction: 0.5), at("10:05:00"))
        XCTAssertEqual(BlockTimeline.captureMoment(startedAt: start, interval: 600, fraction: .nan), at("10:05:00"))
        for _ in 0..<200 {
            let fraction = BlockTimeline.randomFraction()
            XCTAssertTrue((0..<1).contains(fraction))
            let moment = BlockTimeline.captureMoment(startedAt: start, interval: 600, fraction: fraction)
            XCTAssertTrue(moment >= at("10:01:00") && moment <= at("10:09:00"))
        }
        XCTAssertGreaterThan(Set((0..<20).map { _ in BlockTimeline.randomFraction() }).count, 15, "not a constant")
    }

    func testTheScreenshotIsDueOnceAtItsMomentWhenAllowed() {
        var blocks = started(10, idle: 5)
        XCTAssertNil(blocks.captureDue(now: at("10:04:59")))
        guard let id = blocks.captureDue(now: at("10:05:00")) else { return XCTFail("due at 50%") }
        blocks.markCaptured(id)
        XCTAssertNil(blocks.captureDue(now: at("10:06:00")), "once per block")
        // Ending after the moment: no second capture at the end.
        XCTAssertFalse(blocks.end(at: at("10:07:00"), now: at("10:07:00"), reason: .pause)[0].captureAtEnd)

        var noScreens = BlockTimeline()
        noScreens.start(at: at("10:00:00"), policy: policy(10, idle: 5, screenshot: false), description: nil,
                        fraction: 0.5)
        XCTAssertNil(noScreens.captureDue(now: at("10:09:00")))
    }

    /// The tick sequence of a break that ran out during a sleep (the Windows
    /// worker's `a_break_that_runs_out_during_a_sleep_*`): the stretch starts
    /// at the wake, not at the break's end, so no sleep is queued as work.
    func testABreakThatRunsOutDuringASleepResumesAtTheWake() {
        // Break 12:00-13:00, lid closed at 12:05, opened (and unlocked) at 14:00.
        let until = at("13:00:00")
        let woke = at("14:00:00")
        let now = at("14:00:03")
        let resume = WorkerRules.breakResume(until: until, awakeSince: woke, now: now)
        XCTAssertEqual(resume, woke)

        var blocks = BlockTimeline()
        blocks.start(at: resume, policy: policy(10, idle: 5), description: nil, fraction: 0.5)
        let outcome = blocks.tick(now: now, lastActivity: at("14:00:01"), fraction: mid)
        XCTAssertTrue(outcome.finished.isEmpty, "nothing of the sleep is queued")
        XCTAssertEqual(blocks.stretch?.block.startedAt, woke)
    }
}
