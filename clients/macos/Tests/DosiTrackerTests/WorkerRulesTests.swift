import DosiTrackerCore
import XCTest
@testable import DosiTracker

/// The worker's pure rules, as the app runs them.
final class WorkerRulesTests: XCTestCase {
    private let t0 = Date(timeIntervalSince1970: 1_790_000_000)

    private func t(_ seconds: TimeInterval) -> Date {
        t0.addingTimeInterval(seconds)
    }

    // MARK: Break end

    func testABreakResumesWhereItRanOutWhenTheMacStayedAwake() {
        // Unlocked at 12:30 during a break until 13:00, checked at 13:00:03.
        XCTAssertEqual(WorkerRules.breakResume(until: t(3600), awakeSince: t(1800), now: t(3603)), t(3600))
        XCTAssertEqual(WorkerRules.breakResume(until: t(3600), awakeSince: nil, now: t(3603)), t(3600))
    }

    func testABreakThatRanOutDuringASleepResumesAtTheWake() {
        // Break until 13:00, asleep from 12:05, awake (and unlocked) at 14:00.
        XCTAssertEqual(WorkerRules.breakResume(until: t(3600), awakeSince: t(7200), now: t(7203)), t(7200))
    }

    func testABreakNeverResumesInTheFuture() {
        XCTAssertEqual(WorkerRules.breakResume(until: t(3600), awakeSince: t(9000), now: t(7200)), t(7200))
    }

    // MARK: Input under Secure Event Input

    func testTheLaterOfTheTapAndTheHardwareClockIsTheLastInput() {
        // Typing in Terminal with Secure Keyboard Entry: the tap last saw the
        // mouse at t0, the hardware clock saw keys a moment ago.
        XCTAssertEqual(WorkerRules.latestInput(tap: t(0), hardware: t(400)), t(400))
        XCTAssertEqual(WorkerRules.latestInput(tap: t(400), hardware: t(0)), t(400))
        XCTAssertEqual(WorkerRules.latestInput(tap: nil, hardware: t(5)), t(5))
        XCTAssertEqual(WorkerRules.latestInput(tap: t(5), hardware: nil), t(5))
        XCTAssertNil(WorkerRules.latestInput(tap: nil, hardware: nil))
    }

    func testComingBackIsSeenByTheTapOrTheHardwareClock() {
        let since = t(0)
        // The tap's own first input wins (it is the exact return moment).
        XCTAssertEqual(WorkerRules.firstInput(after: since, now: t(600), tapFirst: t(300), hardwareLast: t(590)), t(300))
        // Keys the tap never saw: the hardware clock's.
        XCTAssertEqual(WorkerRules.firstInput(after: since, now: t(600), tapFirst: nil, hardwareLast: t(590)), t(590))
        // Nothing since: still away.
        XCTAssertNil(WorkerRules.firstInput(after: since, now: t(600), tapFirst: nil, hardwareLast: t(-10)))
        XCTAssertNil(WorkerRules.firstInput(after: since, now: t(600), tapFirst: nil, hardwareLast: since))
        XCTAssertNil(WorkerRules.firstInput(after: since, now: t(600), tapFirst: nil, hardwareLast: nil))
    }

    // MARK: Session end

    func testAPowerOffAnnouncementCountsOnlyForAWhile() {
        let flag = PowerOffFlag()
        XCTAssertFalse(flag.isRecent(at: t(0)))
        flag.set(at: t(0))
        XCTAssertTrue(flag.isRecent(at: t(2)))
        XCTAssertTrue(flag.isRecent(at: t(PowerOffFlag.validFor)))
        // The log out was cancelled; a Quit hours later is an ordinary quit.
        XCTAssertFalse(flag.isRecent(at: t(3 * 3600)))
    }

    func testMaintenanceAndKeychainRetriesAreSpaced() {
        XCTAssertEqual(WorkerRules.maintenanceEvery, 6 * 3600)
        XCTAssertGreaterThanOrEqual(WorkerRules.keychainRetry, 60)
    }
}
