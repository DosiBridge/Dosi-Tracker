import DosiTrackerCore
import XCTest
@testable import DosiTracker

/// Which status wins (the first reason tracking is not running), and what each
/// status allows — the Windows agent's `status` / `presence` rules.
final class TrackerStatusTests: XCTestCase {
    /// Signed in with a project, consented, tracking.
    private func tracking() -> StatusFacts {
        var facts = StatusFacts()
        facts.signedIn = true
        facts.hasSession = true
        facts.projectsKnown = true
        facts.consentGranted = true
        facts.tracking = true
        return facts
    }

    func testWithoutConsentTheUserIsAskedForIt() {
        var facts = tracking()
        facts.consentGranted = false
        facts.tracking = false
        XCTAssertEqual(facts.status, .consentRequired)
        // Before a pause, a missing project or the server being out of reach.
        facts.paused = true
        XCTAssertEqual(facts.status, .consentRequired)
        facts.paused = false
        facts.hasSession = false
        XCTAssertEqual(facts.status, .consentRequired)
        facts.projectsKnown = false
        facts.contacting = true
        XCTAssertEqual(facts.status, .consentRequired)
        // Once given, those apply again.
        facts.consentGranted = true
        XCTAssertEqual(facts.status, .connecting)
    }

    func testWaitingForConsentCountsAsSignedIn() {
        let status = TrackerStatus.consentRequired
        XCTAssertTrue(status.isSignedIn, "sign out, switch project and upload what is queued all work")
        XCTAssertFalse(status.needsSignIn)
        XCTAssertFalse(status.canPause, "nothing runs to pause")
        XCTAssertFalse(status.isPaused)
        XCTAssertEqual(status.presence, .stopped, "the dashboard sees the device, not tracking")
        XCTAssertEqual(status.label, "Consent required")
        XCTAssertEqual(StatusBadge.of(status), .attention)
    }

    func testSigningInAndTheQueueComeBeforeConsent() {
        var facts = StatusFacts()
        facts.consentGranted = false
        XCTAssertEqual(facts.status, .signedOut)
        facts.choosingWorkspace = true
        XCTAssertEqual(facts.status, .choosingWorkspace)
        facts = tracking()
        facts.consentGranted = false
        facts.signInExpired = true
        XCTAssertEqual(facts.status, .signInExpired)
        facts.storageFailure = "The local queue cannot be opened."
        XCTAssertEqual(facts.status, .stopped("The local queue cannot be opened."))
        XCTAssertFalse(facts.status.isSignedIn)
    }

    func testWithConsentTheUsualOrderApplies() {
        var facts = tracking()
        XCTAssertEqual(facts.status, .tracking)
        facts.away = true
        XCTAssertEqual(facts.status, .away)
        facts.locked = true
        XCTAssertEqual(facts.status, .locked)
        facts.suspended = true
        XCTAssertEqual(facts.status, .asleep)
        facts.paused = true
        facts.timedBreak = true
        XCTAssertEqual(facts.status, .onBreak)
        facts = tracking()
        facts.hasSession = false
        XCTAssertEqual(facts.status, .choosingProject)
    }
}
