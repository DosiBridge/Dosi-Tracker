import XCTest
@testable import DosiTrackerCore

/// The user's own preferences in UserDefaults (goal rules ported from the
/// Windows agent's `prefs.rs`).
final class PreferencesTests: XCTestCase {
    private var suiteName: String!
    private var defaults: UserDefaults!
    private var preferences: Preferences!

    override func setUpWithError() throws {
        suiteName = "com.dosibridge.tracker.tests.\(UUID().uuidString)"
        defaults = try XCTUnwrap(UserDefaults(suiteName: suiteName))
        preferences = Preferences(defaults: defaults)
    }

    override func tearDown() {
        defaults.removePersistentDomain(forName: suiteName)
        preferences = nil
        defaults = nil
        super.tearDown()
    }

    func testNothingSavedGivesTheDefaults() {
        XCTAssertEqual(preferences.goals, Goals.standard)
        XCTAssertEqual(preferences.taskNote, "")
        XCTAssertNil(preferences.pause)
        XCTAssertNil(preferences.rememberedProjectId)
        XCTAssertNil(preferences.savedServerURL)
        XCTAssertNil(preferences.lastWorkspace)
        XCTAssertNil(preferences.lastUsername)
    }

    func testGoalsRoundTrip() {
        preferences.goals = Goals(dailyHours: 6.5, workDaysPerWeek: 4)
        XCTAssertEqual(Preferences(defaults: defaults).goals, Goals(dailyHours: 6.5, workDaysPerWeek: 4))
    }

    func testOutOfRangeGoalsAreClamped() {
        XCTAssertEqual(Goals(dailyHours: 40.3, workDaysPerWeek: 9).normalized(), Goals(dailyHours: 16, workDaysPerWeek: 7))
        XCTAssertEqual(Goals(dailyHours: 7.3, workDaysPerWeek: 0).normalized(), Goals(dailyHours: 7.5, workDaysPerWeek: 1))
        let notANumber = Goals(dailyHours: .nan, workDaysPerWeek: 5).normalized()
        XCTAssertEqual(notANumber.dailyHours, 8)
        XCTAssertEqual(notANumber.weeklySeconds, 8 * 3600 * 5)
        XCTAssertEqual(Goals(dailyHours: 0.2, workDaysPerWeek: 5).normalized().dailyHours, 1)

        // A stray write is clamped on read.
        defaults.set(99.0, forKey: Preferences.Key.dailyGoalHours)
        defaults.set(-3, forKey: Preferences.Key.workDaysPerWeek)
        XCTAssertEqual(preferences.goals, Goals(dailyHours: 16, workDaysPerWeek: 1))
    }

    func testTheTaskNoteIsKeptWithinTheFieldLimit() {
        preferences.taskNote = "Review PR #42"
        XCTAssertEqual(preferences.taskNote, "Review PR #42")
        preferences.taskNote = String(repeating: "x", count: 500)
        XCTAssertEqual(preferences.taskNote.count, TaskNote.maxLength)
        preferences.taskNote = ""
        XCTAssertNil(defaults.object(forKey: Preferences.Key.taskNote))
    }

    func testAPauseSurvivesARestart() {
        let pause = PauseRecord(since: utc("2026-10-05T18:00:00Z"), until: nil)
        preferences.pause = pause
        XCTAssertEqual(Preferences(defaults: defaults).pause, pause)

        let timedBreak = PauseRecord(since: utc("2026-10-05T12:00:00Z"), until: utc("2026-10-05T12:30:00Z"))
        preferences.pause = timedBreak
        XCTAssertEqual(Preferences(defaults: defaults).pause, timedBreak)

        // An open-ended pause after a break drops the old end.
        preferences.pause = pause
        XCTAssertNil(Preferences(defaults: defaults).pause?.until)

        preferences.pause = nil
        XCTAssertNil(Preferences(defaults: defaults).pause)
        XCTAssertNil(defaults.object(forKey: Preferences.Key.pauseUntil))
    }

    func testTheRememberedProjectAndSignInPrefillAreTrimmed() {
        preferences.rememberedProjectId = "  p1  "
        XCTAssertEqual(preferences.rememberedProjectId, "p1")
        preferences.rememberedProjectId = "   "
        XCTAssertNil(preferences.rememberedProjectId)

        preferences.savedServerURL = "https://tracker.example.com"
        preferences.lastWorkspace = "Acme"
        preferences.lastUsername = "alice@acme.com"
        let reloaded = Preferences(defaults: defaults)
        XCTAssertEqual(reloaded.savedServerURL, "https://tracker.example.com")
        XCTAssertEqual(reloaded.lastWorkspace, "Acme")
        XCTAssertEqual(reloaded.lastUsername, "alice@acme.com")
        preferences.lastWorkspace = nil
        XCTAssertNil(preferences.lastWorkspace)
    }

    func testTheDeviceIdIsCreatedOnceAndKept() {
        let first = preferences.deviceId()
        XCTAssertNotNil(UUID(uuidString: first))
        XCTAssertEqual(first, first.lowercased())
        XCTAssertEqual(Preferences(defaults: defaults).deviceId(), first, "stable across runs")

        // A damaged value gets a fresh id (and is repaired).
        defaults.set("not-a-uuid", forKey: Preferences.Key.deviceId)
        let replaced = preferences.deviceId()
        XCTAssertNotEqual(replaced, first)
        XCTAssertNotNil(UUID(uuidString: replaced))
        XCTAssertEqual(preferences.deviceId(), replaced)
    }

    func testTheDeviceIdIsCreatedOnceUnderConcurrentFirstUse() {
        let lock = NSLock()
        var ids = Set<String>()
        DispatchQueue.concurrentPerform(iterations: 8) { _ in
            let id = preferences.deviceId()
            lock.lock()
            ids.insert(id)
            lock.unlock()
        }
        XCTAssertEqual(ids.count, 1)
    }
}
