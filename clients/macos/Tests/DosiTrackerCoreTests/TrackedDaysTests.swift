import XCTest
@testable import DosiTrackerCore

/// "Today" and "this week" in the user's zone, and the recent-blocks list
/// (ports of the Windows worker's tests).
final class TrackedDaysTests: XCTestCase {
    private func zone(hours: Int) -> TimeZone {
        TimeZone(secondsFromGMT: hours * 3600)!
    }

    func testTodayStartsAtLocalMidnightAheadOfUTC() {
        // 02:00 on Thursday 24 Sep in UTC+6 is still Wednesday in UTC.
        let starts = TrackedDays.dayAndWeekStart(now: utc("2026-09-23T20:00:00Z"), timeZone: zone(hours: 6))
        XCTAssertEqual(starts.today, utc("2026-09-23T18:00:00Z")) // Thu 24 Sep 00:00 +06
        XCTAssertEqual(starts.week, utc("2026-09-20T18:00:00Z")) // Mon 21 Sep 00:00 +06
    }

    func testTodayStartsAtLocalMidnightBehindUTC() {
        // 22:00 on Sunday 27 Sep in UTC-5 is already Monday in UTC.
        let starts = TrackedDays.dayAndWeekStart(now: utc("2026-09-28T03:00:00Z"), timeZone: zone(hours: -5))
        XCTAssertEqual(starts.today, utc("2026-09-27T05:00:00Z")) // Sun 27 Sep 00:00 -05
        XCTAssertEqual(starts.week, utc("2026-09-21T05:00:00Z")) // Mon 21 Sep 00:00 -05
    }

    func testUTCKeepsUTCMidnights() {
        let starts = TrackedDays.dayAndWeekStart(now: utc("2026-09-23T12:34:56Z"), timeZone: zone(hours: 0))
        XCTAssertEqual(starts.today, utc("2026-09-23T00:00:00Z"))
        XCTAssertEqual(starts.week, utc("2026-09-21T00:00:00Z"))
    }

    func testADaylightSavingChangeKeepsWallClockMidnights() throws {
        // Europe/Berlin moves its clocks back on Sunday 25 Oct 2026.
        let berlin = try XCTUnwrap(TimeZone(identifier: "Europe/Berlin"))
        let starts = TrackedDays.dayAndWeekStart(now: utc("2026-10-28T09:00:00Z"), timeZone: berlin)
        XCTAssertEqual(starts.today, utc("2026-10-27T23:00:00Z")) // Wed 28 Oct 00:00 +01
        XCTAssertEqual(starts.week, utc("2026-10-25T23:00:00Z")) // Mon 26 Oct 00:00 +01
        let days = TrackedDays.pastDaysOfWeek(now: utc("2026-10-26T09:00:00Z"), timeZone: berlin)
        XCTAssertTrue(days.isEmpty, "Monday")
        let sunday = TrackedDays.pastDaysOfWeek(now: utc("2026-10-25T09:00:00Z"), timeZone: berlin)
        // Saturday 24 Oct (+02) is a normal 24-hour day.
        XCTAssertEqual(sunday.last, PastDay(slot: 5, start: utc("2026-10-23T22:00:00Z"), end: utc("2026-10-24T22:00:00Z")))
    }

    func testPastDaysOfTheWeekAreLocalDaysBeforeToday() {
        // Thursday 24 Sep, 02:00 in UTC+6: Monday to Wednesday are past.
        let days = TrackedDays.pastDaysOfWeek(now: utc("2026-09-23T20:00:00Z"), timeZone: zone(hours: 6))
        XCTAssertEqual(days.count, 3)
        XCTAssertEqual(days[0], PastDay(slot: 0, start: utc("2026-09-20T18:00:00Z"), end: utc("2026-09-21T18:00:00Z")))
        XCTAssertEqual(days[2], PastDay(slot: 2, start: utc("2026-09-22T18:00:00Z"), end: utc("2026-09-23T18:00:00Z")))

        // On a Monday nothing is past yet; on a Sunday, six days are.
        XCTAssertTrue(TrackedDays.pastDaysOfWeek(now: utc("2026-09-21T09:00:00Z"), timeZone: zone(hours: 0)).isEmpty)
        XCTAssertEqual(TrackedDays.pastDaysOfWeek(now: utc("2026-09-27T09:00:00Z"), timeZone: zone(hours: 0)).count, 6)
        XCTAssertEqual(TrackedDays.weekdaySlot(of: utc("2026-09-27T09:00:00Z"), timeZone: zone(hours: 0)), 6)
        XCTAssertEqual(TrackedDays.weekdaySlot(of: utc("2026-09-21T09:00:00Z"), timeZone: zone(hours: 0)), 0)
    }

    func testOnlyThePartOfABlockAfterTheCutoffIsCounted() {
        let midnight = utc("2026-09-30T18:00:00Z")
        // Entirely after: all of it.
        XCTAssertEqual(TrackedDays.trackedSeconds(from: utc("2026-09-30T18:10:00Z"), to: utc("2026-09-30T18:15:00Z"),
                                                  since: midnight), 300)
        // Spanning midnight: only the part after it.
        XCTAssertEqual(TrackedDays.trackedSeconds(from: utc("2026-09-30T17:58:00Z"), to: utc("2026-09-30T18:03:30Z"),
                                                  since: midnight), 210)
        // Entirely before: nothing.
        XCTAssertEqual(TrackedDays.trackedSeconds(from: utc("2026-09-30T17:00:00Z"), to: utc("2026-09-30T17:05:00Z"),
                                                  since: midnight), 0)
    }

    private func recent(_ rowId: Int64) -> RecentBlock {
        RecentBlock(rowId: rowId, projectTitle: "Website", startedAt: utc("2026-09-30T10:00:00Z"),
                    endedAt: utc("2026-09-30T10:10:00Z"), productivity: 50, keyboardHits: 0, mouseClicks: 0,
                    hasScreenshot: false, endReason: .interval)
    }

    func testRecentBlocksAreNewestFirstAndBounded() {
        var blocks = RecentBlocks()
        for rowId in 1...Int64(RecentBlocks.limit + 3) {
            blocks.push(recent(rowId))
        }
        XCTAssertEqual(blocks.items.count, RecentBlocks.limit)
        XCTAssertEqual(blocks.items.first?.rowId, Int64(RecentBlocks.limit + 3))
        XCTAssertEqual(blocks.items.last?.rowId, 4)
        blocks.removeAll()
        XCTAssertTrue(blocks.items.isEmpty)
    }

    func testUploadResultsAreMatchedToTheirBlock() {
        var blocks = RecentBlocks()
        blocks.push(recent(1))
        blocks.push(recent(2))
        blocks.setUploadState(rowId: 1, .uploaded)
        blocks.setUploadState(rowId: 2, .rejected, message: "Overlaps another block.")
        blocks.setUploadState(rowId: 99, .uploaded) // not listed: ignored
        XCTAssertEqual(blocks.items[0].upload, .rejected)
        XCTAssertEqual(blocks.items[0].message, "Overlaps another block.")
        XCTAssertEqual(blocks.items[1].upload, .uploaded)
    }

    func testARecentBlockSummarisesItsPayload() {
        var payload = ActivityPayload(projectId: "p", startedAt: at(t0), endedAt: at(t0 + 600), productivity: nil,
                                      mouseClicks: 3, keyboardHits: 9)
        payload.endReason = .idle
        payload.screenshotPngBase64 = "x"
        let block = RecentBlock(rowId: 7, projectTitle: "Website", payload: payload)
        XCTAssertEqual(block.id, 7)
        XCTAssertNil(block.productivity, "not measured stays not measured")
        XCTAssertEqual(block.keyboardHits, 9)
        XCTAssertTrue(block.hasScreenshot)
        XCTAssertEqual(block.endReason, .idle)
        XCTAssertEqual(block.upload, .queued)
    }
}
