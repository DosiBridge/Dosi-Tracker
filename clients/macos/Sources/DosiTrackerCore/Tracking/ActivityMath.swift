import Foundation

/// The activity % and per-minute timeline of a block: a one-to-one port of
/// the Windows agent's `tracking/input.rs` bucketing and
/// `tracking/usage.rs` timeline, so both platforms report the same numbers
/// for the same input (CONTRACT §1 "Activity %").
///
/// A block `start ... end` (Unix seconds, `end` inclusive) is split into
/// minute buckets counted from its start: the block length rounded to the
/// nearest minute (at least one). Bucket `i` covers seconds
/// `[start + 60i, start + 60(i+1))`; the last bucket runs to `end` and absorbs
/// the remainder, so no bucket is shorter than 30 s and a sliver of a minute
/// can never cost a fully active user a percentage point.
public enum ActivityMath {
    /// Number of minute buckets in the block: the length rounded to the
    /// nearest minute, at least one; zero for an empty or inverted block.
    public static func minuteBuckets(start: Int64, end: Int64) -> Int {
        let span = end - start
        guard span > 0 else { return 0 }
        return Int(max((span + 30) / 60, 1))
    }

    /// The bucket a second inside the block falls into (a trailing sliver
    /// joins the last bucket).
    public static func bucketOf(second: Int64, start: Int64, minutes: Int) -> Int {
        guard minutes > 0 else { return 0 }
        let raw = (second - start) / 60
        return Int(min(max(raw, 0), Int64(minutes - 1)))
    }

    /// Length in seconds of bucket `index`: 60, except the last, which runs to
    /// the end (30...89 s, or the whole block when it is shorter than that);
    /// 0 past the end.
    public static func bucketSeconds(index: Int, start: Int64, end: Int64) -> Int {
        let minutes = minuteBuckets(start: start, end: end)
        guard minutes > 0, index >= 0, index < minutes else { return 0 }
        if index < minutes - 1 {
            return 60
        }
        return Int(max(end - start - 60 * Int64(minutes - 1), 0))
    }

    /// Which buckets had any input — the activity % is the share of `true`s.
    /// Seconds outside `start ... end` are ignored.
    public static func activeMinutes(activeSeconds: [Int64], start: Int64, end: Int64) -> [Bool] {
        let minutes = minuteBuckets(start: start, end: end)
        guard minutes > 0 else { return [] }
        var active = [Bool](repeating: false, count: minutes)
        for second in activeSeconds where second >= start && second <= end {
            active[bucketOf(second: second, start: start, minutes: minutes)] = true
        }
        return active
    }

    /// Activity % (0-100): the share of buckets that had input, rounded to
    /// the nearest percent. Input from before the block (e.g. while paused)
    /// never counts, because only seconds inside it do.
    public static func activityPercent(activeSeconds: [Int64], start: Int64, end: Int64) -> Int {
        let minutes = minuteBuckets(start: start, end: end)
        guard minutes > 0 else { return 0 }
        let activeCount = activeMinutes(activeSeconds: activeSeconds, start: start, end: end).filter { $0 }.count
        return (activeCount * 100 + minutes / 2) / minutes
    }

    /// Seconds of the block without input, at minute granularity
    /// (`idleSeconds`, CONTRACT §1): the lengths of the timeline's inactive
    /// buckets.
    public static func idleSeconds(timeline: [ActivityMinute], start: Int64, end: Int64) -> Int {
        timeline.filter { !$0.active }.reduce(0) { total, minute in
            total + bucketSeconds(index: minute.minute, start: start, end: end)
        }
    }

    /// Per-minute timeline of the block, on the same buckets as the activity
    /// %: key presses and clicks (Unix ms, bucketed by their second), whether
    /// the minute counts as active and for how many distinct seconds (never
    /// more than the bucket is long), and the app in front the longest (when
    /// usage is available).
    public static func minuteTimeline(
        start: Int64,
        end: Int64,
        slices: [UsageSlice],
        keyPresses: [Int64],
        clicks: [Int64],
        activeSeconds: [Int64]
    ) -> [ActivityMinute] {
        let minutes = minuteBuckets(start: start, end: end)
        guard minutes > 0 else { return [] }
        let active = activeMinutes(activeSeconds: activeSeconds, start: start, end: end)

        // Distinct active seconds per bucket (a second with both keyboard and
        // mouse input is listed twice).
        var secondsPerBucket = [Int](repeating: 0, count: minutes)
        for second in Set(activeSeconds) where second >= start && second <= end {
            secondsPerBucket[bucketOf(second: second, start: start, minutes: minutes)] += 1
        }

        let apps = mainApps(slices, start: start, minutes: minutes)
        var timeline: [ActivityMinute] = []
        timeline.reserveCapacity(minutes)
        for index in 0..<minutes {
            timeline.append(ActivityMinute(
                minute: index,
                active: active[index],
                appName: apps[index],
                activeSeconds: min(secondsPerBucket[index], bucketSeconds(index: index, start: start, end: end))
            ))
        }
        for ms in keyPresses {
            let bucket = bucketOf(second: EpochTime.floorDiv(ms, 1000), start: start, minutes: minutes)
            timeline[bucket].keyboardHits = (timeline[bucket].keyboardHits ?? 0) + 1
        }
        for ms in clicks {
            let bucket = bucketOf(second: EpochTime.floorDiv(ms, 1000), start: start, minutes: minutes)
            timeline[bucket].mouseClicks = (timeline[bucket].mouseClicks ?? 0) + 1
        }
        return timeline
    }

    /// The app in front the longest in each bucket of the block that starts
    /// at `start` (Unix seconds); ties go to the app whose name sorts first
    /// (UTF-8 byte order), so the result never depends on hash order.
    public static func mainApps(_ slices: [UsageSlice], start: Int64, minutes: Int) -> [String?] {
        guard minutes > 0 else { return [] }
        var perBucket = [[String: Int64]](repeating: [:], count: minutes)
        for slice in slices {
            var from = slice.startMs
            while from < slice.endMs {
                let bucket = bucketOf(second: EpochTime.floorDiv(from, 1000), start: start, minutes: minutes)
                // Bucket i spans [start + 60i, start + 60(i+1)) seconds; the
                // last one runs to the end of the block.
                let bucketEnd: Int64 = bucket == minutes - 1 ? Int64.max : (start + 60 * Int64(bucket + 1)) * 1000
                let to = min(slice.endMs, bucketEnd)
                perBucket[bucket][slice.app, default: 0] += to - from
                from = to
            }
        }
        return perBucket.map { apps -> String? in
            var best: (app: String, ms: Int64)?
            for (app, ms) in apps {
                if let current = best {
                    if ms > current.ms || (ms == current.ms && app.utf8.lexicographicallyPrecedes(current.app.utf8)) {
                        best = (app, ms)
                    }
                } else {
                    best = (app, ms)
                }
            }
            return best?.app
        }
    }
}
