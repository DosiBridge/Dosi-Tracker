import XCTest
@testable import DosiTrackerCore

/// What lock, sleep, power-off and the session's changes mean for the block
/// in progress (CONTRACT §4), and the system idle clock.
final class SystemEventTests: XCTestCase {
    func testLockSleepAndPowerOffEndTheBlockWithTheirReason() {
        XCTAssertEqual(SystemEventKind.willSleep.endReason, .sleep)
        XCTAssertEqual(SystemEventKind.willPowerOff.endReason, .shutdown)
        XCTAssertEqual(SystemEventKind.screenLocked.endReason, .lock)
        // Switching to another user leaves the screen as a lock does.
        XCTAssertEqual(SystemEventKind.sessionDidResignActive.endReason, .lock)
        for kind in [SystemEventKind.didWake, .sessionDidBecomeActive, .screenUnlocked, .screensDidSleep, .screensDidWake] {
            XCTAssertNil(kind.endReason, "\(kind)")
        }
        // None of these ends takes a screenshot at the end.
        for kind in SystemEventKind.allCases {
            if let reason = kind.endReason {
                XCTAssertFalse(reason.allowsCaptureAtEnd, "\(kind)")
            }
        }
    }

    func testWakeUnlockAndTheSessionsReturnLetTrackingResume() {
        XCTAssertEqual(SystemEventKind.allCases.filter { $0.resumesTracking },
                       [.didWake, .sessionDidBecomeActive, .screenUnlocked])
        // Nothing both ends a block and resumes tracking.
        XCTAssertTrue(SystemEventKind.allCases.allSatisfy { !($0.resumesTracking && $0.endReason != nil) })
    }

    func testAnEventKeepsTheMomentItHappened() {
        let event = SystemEvent(kind: .screenLocked, at: at(t0 + 5))
        XCTAssertEqual(event.kind, .screenLocked)
        XCTAssertEqual(event.at, at(t0 + 5))
        XCTAssertEqual(event, SystemEvent(kind: .screenLocked, at: at(t0 + 5)))
        XCTAssertNotEqual(event, SystemEvent(kind: .screenUnlocked, at: at(t0 + 5)))
    }

    func testTheSystemIdleClockGivesTheMomentOfTheLastInput() {
        let now = at(t0)
        XCTAssertEqual(SystemIdleTime.lastInput(now: now, secondsSinceLastInput: 90), at(t0 - 90))
        XCTAssertEqual(SystemIdleTime.lastInput(now: now, secondsSinceLastInput: 0), now)
        XCTAssertNil(SystemIdleTime.lastInput(now: now, secondsSinceLastInput: -1))
        XCTAssertNil(SystemIdleTime.lastInput(now: now, secondsSinceLastInput: .nan))
        XCTAssertNil(SystemIdleTime.lastInput(now: now, secondsSinceLastInput: .infinity))
    }
}
