import XCTest
@testable import DosiTrackerCore

/// Ports of the Windows agent's `tracking/input.rs` bucketing tests and
/// `tracking/usage.rs` timeline tests: both agents must produce the same
/// numbers for the same input.
final class ActivityMathTests: XCTestCase {
    private func percent(_ seconds: [Int64], _ start: Int64, _ end: Int64) -> Int {
        ActivityMath.activityPercent(activeSeconds: seconds, start: start, end: end)
    }

    // `title` comes last and labelled: an unlabelled defaulted parameter in
    // the middle would swallow the start of `slice("a.exe", 0, 60)`.
    private func slice(_ app: String, _ from: Int64, _ to: Int64, title: String = "") -> UsageSlice {
        UsageSlice(app: app, title: title, startMs: t0Ms + from * 1000, endMs: t0Ms + to * 1000)
    }

    // MARK: input.rs

    func testEmptyOrDegenerateBlocksAreZero() {
        XCTAssertEqual(percent([], t0, t0 + 600), 0)
        XCTAssertEqual(percent([t0], t0, t0), 0)
        XCTAssertEqual(percent([t0], t0 + 10, t0), 0)
    }

    func testInputInEveryMinuteIsFullActivity() {
        let seconds = (0..<10).map { t0 + Int64($0) * 60 + 30 }
        XCTAssertEqual(percent(seconds, t0, t0 + 600), 100)
    }

    func testCountsMinutesNotEvents() {
        // A burst of input all inside minute 0, then 2 more active minutes.
        var seconds = (0..<60).map { t0 + Int64($0) }
        seconds.append(t0 + 5 * 60 + 1)
        seconds.append(t0 + 9 * 60 + 59)
        XCTAssertEqual(percent(seconds, t0, t0 + 600), 30)
    }

    func testMinutesAreCountedFromTheBlockStart() {
        // Block starts mid-minute; one input per elapsed minute is still 100%.
        let start = t0 + 37
        let seconds = (0..<10).map { start + Int64($0) * 60 + 59 }
        XCTAssertEqual(percent(seconds, start, start + 600), 100)
    }

    func testInputOutsideTheBlockIsIgnored() {
        // Input while paused (before the block) and after it must not count.
        XCTAssertEqual(percent([t0 - 1, t0 - 120, t0 + 601, t0 + 900], t0, t0 + 600), 0)
    }

    func testATrailingSliverIsFoldedIntoTheLastMinute() {
        // 10 min + 5 s: still 10 buckets, the 5 s join the last one.
        let seconds = (0..<10).map { t0 + Int64($0) * 60 }
        XCTAssertEqual(percent(seconds, t0, t0 + 605), 100)
        // Input only in the sliver marks the last bucket.
        XCTAssertEqual(percent([t0 + 603], t0, t0 + 605), 10)
    }

    func testShortBlocksHaveAtLeastOneMinute() {
        XCTAssertEqual(percent([t0 + 5], t0, t0 + 20), 100)
        XCTAssertEqual(percent([], t0, t0 + 20), 0)
    }

    func testResultIsRoundedToTheNearestPercent() {
        // 1 of 3 minutes = 33.3% -> 33; 2 of 3 = 66.7% -> 67.
        XCTAssertEqual(percent([t0], t0, t0 + 180), 33)
        XCTAssertEqual(percent([t0, t0 + 60], t0, t0 + 180), 67)
    }

    func testBucketLengthsAddUpToTheBlock() {
        // 10 min + 29 s: ten buckets, the last one 89 s.
        let (start, end) = (t0, t0 + 629)
        let lengths = (0..<10).map { ActivityMath.bucketSeconds(index: $0, start: start, end: end) }
        XCTAssertEqual(Array(lengths[..<9]), [Int](repeating: 60, count: 9))
        XCTAssertEqual(lengths[9], 89)
        XCTAssertEqual(lengths.reduce(0, +), 629)
        // 10 min - 30 s rounds to ten buckets too; the last one is 30 s.
        XCTAssertEqual(ActivityMath.bucketSeconds(index: 9, start: t0, end: t0 + 570), 30)
        // A 20-second block is one 20-second bucket; nothing past the end.
        XCTAssertEqual(ActivityMath.bucketSeconds(index: 0, start: t0, end: t0 + 20), 20)
        XCTAssertEqual(ActivityMath.bucketSeconds(index: 1, start: t0, end: t0 + 20), 0)
        XCTAssertEqual(ActivityMath.bucketSeconds(index: 0, start: t0, end: t0), 0)
    }

    func testActiveMinutesMatchTheActivityPercent() {
        // 3 buckets; input in the first and the trailing sliver of the last.
        let active = ActivityMath.activeMinutes(activeSeconds: [t0 + 5, t0 + 185], start: t0, end: t0 + 190)
        XCTAssertEqual(active, [true, false, true])
        XCTAssertEqual(percent([t0 + 5, t0 + 185], t0, t0 + 190), 67)
        XCTAssertTrue(ActivityMath.activeMinutes(activeSeconds: [t0], start: t0, end: t0).isEmpty)
    }

    func testMinuteBucketsRoundToTheNearestMinute() {
        XCTAssertEqual(ActivityMath.minuteBuckets(start: t0, end: t0), 0)
        XCTAssertEqual(ActivityMath.minuteBuckets(start: t0, end: t0 - 5), 0)
        XCTAssertEqual(ActivityMath.minuteBuckets(start: t0, end: t0 + 1), 1)
        XCTAssertEqual(ActivityMath.minuteBuckets(start: t0, end: t0 + 89), 1)
        XCTAssertEqual(ActivityMath.minuteBuckets(start: t0, end: t0 + 90), 2)
        XCTAssertEqual(ActivityMath.minuteBuckets(start: t0, end: t0 + 3600), 60)
        // The bucket of a second clamps into the block.
        XCTAssertEqual(ActivityMath.bucketOf(second: t0 - 10, start: t0, minutes: 3), 0)
        XCTAssertEqual(ActivityMath.bucketOf(second: t0 + 61, start: t0, minutes: 3), 1)
        XCTAssertEqual(ActivityMath.bucketOf(second: t0 + 500, start: t0, minutes: 3), 2)
        XCTAssertEqual(ActivityMath.bucketOf(second: t0, start: t0, minutes: 0), 0)
    }

    // MARK: usage.rs timeline

    func testTimelineBucketsInputAndPicksTheMainAppPerMinute() {
        // 3-minute block; code for 1.5 min, then edge.
        let slices = [slice("code.exe", 0, 90), slice("msedge.exe", 90, 180)]
        let keys = [t0Ms + 1_000, t0Ms + 2_000, t0Ms + 61_000]
        let clicks = [t0Ms + 179_500]
        let active = [t0 + 1, t0 + 61]

        let timeline = ActivityMath.minuteTimeline(start: t0, end: t0 + 180, slices: slices,
                                                   keyPresses: keys, clicks: clicks, activeSeconds: active)
        XCTAssertEqual(timeline, [
            ActivityMinute(minute: 0, keyboardHits: 2, mouseClicks: 0, active: true, appName: "code.exe", activeSeconds: 1),
            // 30 s each: the tie goes to the alphabetically first app.
            ActivityMinute(minute: 1, keyboardHits: 1, mouseClicks: 0, active: true, appName: "code.exe", activeSeconds: 1),
            ActivityMinute(minute: 2, keyboardHits: 0, mouseClicks: 1, active: false, appName: "msedge.exe", activeSeconds: 0),
        ])
    }

    func testActiveSecondsCountDistinctSecondsWithinEachBucket() {
        // Keyboard and mouse in the same seconds are one second each; the last
        // bucket (89 s) can hold more than 60.
        var active = (0..<40).map { t0 + Int64($0) }
        active += (0..<40).map { t0 + Int64($0) } // the mouse, at the same seconds
        active += (60..<149).map { t0 + Int64($0) }
        active.append(t0 + 500) // after the block: ignored
        let timeline = ActivityMath.minuteTimeline(start: t0, end: t0 + 149, slices: [], keyPresses: [], clicks: [],
                                                   activeSeconds: active)
        XCTAssertEqual(timeline.count, 2)
        XCTAssertEqual(timeline[0].activeSeconds, 40)
        XCTAssertEqual(timeline[1].activeSeconds, 89)
        XCTAssertTrue(timeline.allSatisfy { ($0.activeSeconds ?? 0) <= 90 })
    }

    func testIdleSecondsAreTheInactiveBucketsLengths() {
        // 10 min + 29 s: buckets 0..8 are 60 s, bucket 9 is 89 s.
        let (start, end) = (t0, t0 + 629)
        let active = [0, 2, 3].map { start + Int64($0) * 60 + 10 }
        let timeline = ActivityMath.minuteTimeline(start: start, end: end, slices: [], keyPresses: [], clicks: [],
                                                   activeSeconds: active)
        // Inactive: 1, 4..8 (six 60-s buckets) and 9 (89 s).
        XCTAssertEqual(ActivityMath.idleSeconds(timeline: timeline, start: start, end: end), 6 * 60 + 89)

        // All active: no idle; none active: the whole block.
        let busy = (0..<10).map { start + Int64($0) * 60 }
        let busyTimeline = ActivityMath.minuteTimeline(start: start, end: end, slices: [], keyPresses: [], clicks: [],
                                                       activeSeconds: busy)
        XCTAssertEqual(ActivityMath.idleSeconds(timeline: busyTimeline, start: start, end: end), 0)
        let quietTimeline = ActivityMath.minuteTimeline(start: start, end: end, slices: [], keyPresses: [], clicks: [],
                                                        activeSeconds: [])
        XCTAssertEqual(ActivityMath.idleSeconds(timeline: quietTimeline, start: start, end: end), 629)
        XCTAssertEqual(ActivityMath.idleSeconds(timeline: [], start: start, end: end), 0)
    }

    func testTimelineFoldsATrailingSliverIntoTheLastMinute() {
        // 2 min + 10 s = 2 buckets; the sliver's input and time join minute 1.
        let slices = [slice("a.exe", 0, 60), slice("b.exe", 60, 130)]
        let timeline = ActivityMath.minuteTimeline(start: t0, end: t0 + 130, slices: slices,
                                                   keyPresses: [t0Ms + 125_000], clicks: [], activeSeconds: [t0 + 125])
        XCTAssertEqual(timeline.count, 2)
        XCTAssertEqual(timeline[1].keyboardHits, 1)
        XCTAssertTrue(timeline[1].active && !timeline[0].active)
        XCTAssertEqual(timeline[1].appName, "b.exe")
    }

    func testTimelineWithoutUsageStillHasCounts() {
        let timeline = ActivityMath.minuteTimeline(start: t0, end: t0 + 60, slices: [], keyPresses: [t0Ms], clicks: [],
                                                   activeSeconds: [t0])
        XCTAssertEqual(timeline.count, 1)
        XCTAssertEqual(timeline[0].keyboardHits, 1)
        XCTAssertNil(timeline[0].appName)
        XCTAssertTrue(ActivityMath.minuteTimeline(start: t0, end: t0, slices: [], keyPresses: [], clicks: [],
                                                  activeSeconds: []).isEmpty)
    }

    func testMainAppsSplitSlicesAtBucketBoundaries() {
        // One slice spanning three buckets credits each bucket its own part.
        let apps = ActivityMath.mainApps([slice("Xcode", 30, 150), slice("Safari", 150, 180)], start: t0, minutes: 3)
        XCTAssertEqual(apps, ["Xcode", "Xcode", "Safari"])
        XCTAssertEqual(ActivityMath.mainApps([], start: t0, minutes: 2), [nil, nil])
        XCTAssertTrue(ActivityMath.mainApps([slice("Xcode", 0, 60)], start: t0, minutes: 0).isEmpty)
    }
}
