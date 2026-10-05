import XCTest
@testable import DosiTrackerCore

/// Ports of the Windows agent's `tracking/usage.rs` tests.
final class UsageRecorderTests: XCTestCase {
    private func window(_ app: String, _ title: String) -> ForegroundWindow {
        ForegroundWindow(appName: app, windowTitle: title)
    }

    private func slice(_ app: String, _ title: String, _ from: Int64, _ to: Int64) -> UsageSlice {
        UsageSlice(app: app, title: title, startMs: t0Ms + from * 1000, endMs: t0Ms + to * 1000)
    }

    private func row(_ app: String, _ title: String, _ seconds: Int, _ keys: Int, _ clicks: Int) -> WindowInfo {
        WindowInfo(appName: app, windowTitle: title, seconds: seconds, keyboardHits: keys, mouseClicks: clicks)
    }

    private func totalSeconds(_ rows: [WindowInfo], _ app: String) -> Int {
        rows.filter { $0.appName == app }.reduce(0) { $0 + ($1.seconds ?? 0) }
    }

    func testTopAppsMergeTitlesAndRankByTime() {
        let slices = [
            slice("code.exe", "main.rs", 0, 30),
            slice("msedge.exe", "Docs", 30, 50),
            slice("Code.exe", "lib.rs", 50, 70),
            slice("slack.exe", "general", 70, 75),
            slice("broken.exe", "", 80, 80), // zero-length: ignored
        ]
        XCTAssertEqual(UsageRecorder.topApps(slices, limit: 10), [
            AppTotal(app: "code.exe", milliseconds: 50_000),
            AppTotal(app: "msedge.exe", milliseconds: 20_000),
            AppTotal(app: "slack.exe", milliseconds: 5_000),
        ])
        XCTAssertEqual(UsageRecorder.topApps(slices, limit: 1).count, 1)
        XCTAssertTrue(UsageRecorder.topApps([], limit: 5).isEmpty)
    }

    func testSamplesCreditTheWindowInFrontAndMergeRepeats() {
        let recorder = UsageRecorder()
        recorder.record(nowMs: t0Ms, window: window("code.exe", "main.rs")) // first sample: nothing to credit yet
        recorder.record(nowMs: t0Ms + 3_000, window: window("code.exe", "main.rs"))
        recorder.record(nowMs: t0Ms + 6_000, window: window("code.exe", "main.rs"))
        recorder.record(nowMs: t0Ms + 9_000, window: window("msedge.exe", "Docs"))
        recorder.record(nowMs: t0Ms + 12_000, window: nil) // locked screen: not credited
        recorder.record(nowMs: t0Ms + 15_000, window: window("msedge.exe", "Docs"))

        XCTAssertEqual(recorder.slices(fromMs: 0, toMs: Int64.max), [
            slice("code.exe", "main.rs", 0, 6),
            slice("msedge.exe", "Docs", 6, 9),
            slice("msedge.exe", "Docs", 12, 15),
        ])
    }

    func testLongGapsAndStaleSamplesAreNotCredited() {
        let recorder = UsageRecorder()
        recorder.record(nowMs: t0Ms, window: window("code.exe", ""))
        recorder.record(nowMs: t0Ms + 120_000, window: window("code.exe", "")) // the Mac slept
        recorder.record(nowMs: t0Ms + 119_000, window: window("code.exe", "")) // racing, older sample
        recorder.record(nowMs: t0Ms + 123_000, window: window("code.exe", ""))
        XCTAssertEqual(recorder.slices(fromMs: 0, toMs: Int64.max), [slice("code.exe", "", 120, 123)])
    }

    func testUnnamedAppsAreNotCredited() {
        let recorder = UsageRecorder()
        recorder.record(nowMs: t0Ms, window: nil)
        recorder.record(nowMs: t0Ms + 3_000, window: window("", "untitled"))
        XCTAssertTrue(recorder.slices(fromMs: 0, toMs: Int64.max).isEmpty)
    }

    func testSlicesAreClippedToTheBlock() {
        let recorder = UsageRecorder()
        recorder.record(nowMs: t0Ms, window: window("a.exe", ""))
        recorder.record(nowMs: t0Ms + 10_000, window: window("a.exe", ""))
        XCTAssertEqual(recorder.slices(fromMs: t0Ms + 4_000, toMs: t0Ms + 7_000), [slice("a.exe", "", 4, 7)])
        XCTAssertTrue(recorder.slices(fromMs: t0Ms + 10_000, toMs: t0Ms + 20_000).isEmpty)
        // The Date-based query is the same.
        XCTAssertEqual(recorder.slices(from: at(t0 + 4), to: at(t0 + 7)), [slice("a.exe", "", 4, 7)])
    }

    func testOldUsageIsForgottenAfterTheRetention() {
        let recorder = UsageRecorder()
        recorder.record(nowMs: t0Ms, window: window("a.exe", ""))
        recorder.record(nowMs: t0Ms + 3_000, window: window("a.exe", ""))
        // Keep sampling, much later than the retention window.
        let later = t0Ms + UsageRecorder.retentionMs + 60_000
        recorder.record(nowMs: later, window: window("b.exe", ""))
        recorder.record(nowMs: later + 3_000, window: window("b.exe", ""))
        XCTAssertEqual(recorder.slices(fromMs: 0, toMs: Int64.max).map { $0.app }, ["b.exe"])
    }

    func testAppUsageGroupsWindowsAndAttributesInput() {
        let slices = [
            slice("code.exe", "main.rs", 0, 100),
            slice("msedge.exe", "Docs", 100, 130),
            slice("code.exe", "main.rs", 130, 200),
            slice("code.exe", "lib.rs", 200, 210),
        ]
        let keys = [t0Ms + 5_000, t0Ms + 150_000, t0Ms + 205_000]
        let clicks = [t0Ms + 110_000, t0Ms + 111_000, t0Ms + 500_000] // last: outside every slice

        XCTAssertEqual(UsageRecorder.appUsage(slices, keyPresses: keys, clicks: clicks), [
            row("code.exe", "main.rs", 170, 2, 0),
            row("msedge.exe", "Docs", 30, 0, 2),
            row("code.exe", "lib.rs", 10, 1, 0),
        ])
    }

    func testAppUsageWithinTheCapIsLeftAlone() {
        let slices = (0..<80).map { slice("app.exe", "tab \($0)", Int64($0), Int64($0) + 1) }
        let usage = UsageRecorder.appUsage(slices, keyPresses: [], clicks: [])
        XCTAssertEqual(usage.count, 80)
        XCTAssertTrue(usage.allSatisfy { $0.windowTitle != UsageRecorder.otherWindowsTitle })
    }

    func testWindowsBeyondTheCapFoldIntoOtherWindowsPerApp() {
        // 250 browser tabs of falling length, and an editor used throughout.
        var slices: [UsageSlice] = []
        var t: Int64 = 0
        for i in 0..<250 {
            let length = Int64(300 - i)
            slices.append(slice("chrome.exe", "tab \(i)", t, t + length))
            t += length
        }
        slices.append(slice("code.exe", "main.rs", t, t + 20))
        let keys = (0..<250).map { slices[$0].startMs + 1 }

        let usage = UsageRecorder.appUsage(slices, keyPresses: keys, clicks: [])
        XCTAssertEqual(usage.count, UsageRecorder.maxUsageEntries)
        let others = usage.filter { $0.windowTitle == UsageRecorder.otherWindowsTitle }
        XCTAssertEqual(others.count, 1)
        XCTAssertEqual(others.first?.appName, "chrome.exe")
        // Nothing is lost: every app's seconds and key presses add up.
        let allChrome = (0..<250).reduce(0) { $0 + (300 - $1) }
        XCTAssertEqual(totalSeconds(usage, "chrome.exe"), allChrome)
        XCTAssertEqual(totalSeconds(usage, "code.exe"), 20)
        // A lone window past the cut keeps its title (it takes one row either way).
        XCTAssertTrue(usage.contains { $0.appName == "code.exe" && $0.windowTitle == "main.rs" })
        XCTAssertEqual(usage.reduce(0) { $0 + ($1.keyboardHits ?? 0) }, 250)
        // Still most-used first.
        XCTAssertTrue(zip(usage, usage.dropFirst()).allSatisfy { ($0.seconds ?? 0) >= ($1.seconds ?? 0) })
    }

    func testFoldingKeepsAsManyWindowsAsFitWithOneRowPerFoldedApp() {
        // Cap 5: a1 a2 a3 b1 c1 c2 (most-used first).
        let rows = [
            row("a", "1", 60, 1, 0),
            row("a", "2", 50, 0, 0),
            row("a", "3", 40, 0, 0),
            row("b", "1", 30, 0, 1),
            row("c", "1", 20, 0, 0),
            row("c", "2", 10, 2, 0),
        ]
        // Keeping 4 would need folds for c only: 4 + 1 = 5.
        XCTAssertEqual(UsageRecorder.foldBeyondCap(rows, cap: 5), [
            row("a", "1", 60, 1, 0),
            row("a", "2", 50, 0, 0),
            row("a", "3", 40, 0, 0),
            row("b", "1", 30, 0, 1),
            row("c", UsageRecorder.otherWindowsTitle, 30, 2, 0),
        ])
    }

    func testMoreAppsThanTheCapShareAnOtherAppsRow() {
        let rows = (0..<8).map { row("app\($0)", "", 80 - $0 * 10, 1, 0) }
        let folded = UsageRecorder.foldBeyondCap(rows, cap: 4)
        XCTAssertEqual(folded.count, 4)
        XCTAssertEqual(folded.reduce(0) { $0 + ($1.seconds ?? 0) }, (0..<8).reduce(0) { $0 + 80 - $1 * 10 })
        let others = folded.first { $0.appName == UsageRecorder.otherAppsName }
        XCTAssertEqual(others?.keyboardHits, 5)
        // The five least-used apps (150 s together) now lead, most-used first.
        XCTAssertEqual(folded.map { $0.appName }, [UsageRecorder.otherAppsName, "app0", "app1", "app2"])
        XCTAssertTrue(UsageRecorder.foldBeyondCap([row("a", "", 1, 0, 0)], cap: 0).isEmpty)
    }

    func testRecordingIsSafeFromSeveralThreads() {
        let recorder = UsageRecorder()
        DispatchQueue.concurrentPerform(iterations: 4) { worker in
            for i in 0..<200 {
                recorder.record(nowMs: t0Ms + Int64(i * 1_000 + worker), window: window("app\(worker)", ""))
                _ = recorder.slices(fromMs: t0Ms, toMs: t0Ms + 1_000_000)
            }
        }
        let slices = recorder.slices(fromMs: 0, toMs: Int64.max)
        XCTAssertTrue(zip(slices, slices.dropFirst()).allSatisfy { $0.endMs <= $1.startMs }, "slices never overlap")
    }
}
