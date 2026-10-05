import Foundation

/// The app and window in front, as the capture service samples it. On macOS
/// `appName` is the app's localized name (`Google Chrome`, CONTRACT §8).
public struct ForegroundWindow: Equatable, Sendable {
    public var appName: String
    public var windowTitle: String

    public init(appName: String, windowTitle: String) {
        self.appName = appName
        self.windowTitle = windowTitle
    }
}

/// A stretch of time one window spent in front, `[startMs, endMs)` (Unix ms).
public struct UsageSlice: Equatable, Sendable {
    public var app: String
    public var title: String
    public var startMs: Int64
    public var endMs: Int64

    public init(app: String, title: String, startMs: Int64, endMs: Int64) {
        self.app = app
        self.title = title
        self.startMs = startMs
        self.endMs = endMs
    }
}

/// Foreground time of one app, for the "top apps" list.
public struct AppTotal: Equatable, Sendable {
    public var app: String
    public var milliseconds: Int64

    public init(app: String, milliseconds: Int64) {
        self.app = app
        self.milliseconds = milliseconds
    }
}

/// Foreground-window usage: which app and window were in front, and for how
/// long — the per-app breakdown and per-minute timeline of each block (port
/// of the Windows agent's `tracking/usage.rs`).
///
/// The capture service samples the front window every ``sampleInterval``;
/// each sample credits the time since the previous one to the window in front
/// now. Consecutive samples of the same window are merged, so memory grows
/// with window switches, not with time. Thread-safe.
public final class UsageRecorder: @unchecked Sendable {
    /// How often the front window should be sampled.
    public static let sampleInterval: TimeInterval = 3
    /// A gap between samples longer than this means the Mac slept or the
    /// sampler was starved; that stretch is credited to no window.
    public static let maxSampleGapMs: Int64 = 30_000
    /// How long usage is remembered: as long as input.
    public static let retentionMs: Int64 = InputLog.retentionSeconds * 1000
    /// The server's cap on the (app, window) entries of one block.
    public static let maxUsageEntries = 200
    /// Title of the row that sums an app's windows beyond the cap.
    public static let otherWindowsTitle = "Other windows"
    /// App name of the row that sums the least-used apps when even one row
    /// per app would exceed the cap.
    public static let otherAppsName = "Other apps"

    private let lock = NSLock()
    private var recorded: [UsageSlice] = []
    private var lastSampleMs: Int64?

    public init() {}

    /// Credit the time since the previous sample to `window` (the one in front
    /// now). Nil — no front window, e.g. the login window or a locked screen —
    /// credits nobody; so does an app without a name.
    public func record(at date: Date, window: ForegroundWindow?) {
        record(nowMs: EpochTime.milliseconds(date), window: window)
    }

    public func record(nowMs: Int64, window: ForegroundWindow?) {
        lock.lock()
        defer { lock.unlock() }
        guard let last = lastSampleMs else {
            lastSampleMs = nowMs
            return
        }
        // A racing sample already covered this instant (or the clock stepped
        // back): nothing to credit.
        guard nowMs > last else { return }
        lastSampleMs = nowMs
        guard nowMs - last <= UsageRecorder.maxSampleGapMs else { return }

        if let window = window, !window.appName.isEmpty {
            if let tail = recorded.last, tail.endMs == last, tail.app == window.appName, tail.title == window.windowTitle {
                recorded[recorded.count - 1].endMs = nowMs
            } else {
                recorded.append(UsageSlice(app: window.appName, title: window.windowTitle, startMs: last, endMs: nowMs))
            }
        }

        let horizon = nowMs - UsageRecorder.retentionMs
        var expired = 0
        while expired < recorded.count && recorded[expired].endMs < horizon {
            expired += 1
        }
        if expired > 0 {
            recorded.removeFirst(expired)
        }
    }

    /// The usage inside `[start, end)`, slices clipped to it.
    public func slices(from start: Date, to end: Date) -> [UsageSlice] {
        slices(fromMs: EpochTime.milliseconds(start), toMs: EpochTime.milliseconds(end))
    }

    public func slices(fromMs startMs: Int64, toMs endMs: Int64) -> [UsageSlice] {
        lock.lock()
        defer { lock.unlock() }
        return recorded.compactMap { slice in
            guard slice.endMs > startMs && slice.startMs < endMs else { return nil }
            return UsageSlice(app: slice.app, title: slice.title,
                              startMs: max(slice.startMs, startMs), endMs: min(slice.endMs, endMs))
        }
    }

    // MARK: Aggregation

    /// Foreground time per application (window titles merged, names compared
    /// ASCII-case-insensitively), most-used first (ties keep first-seen
    /// order), at most `limit` apps — the window's "top apps" list.
    public static func topApps(_ slices: [UsageSlice], limit: Int) -> [AppTotal] {
        var totals: [AppTotal] = []
        for slice in slices {
            let ms = slice.endMs - slice.startMs
            guard ms > 0 else { continue }
            if let index = totals.firstIndex(where: { asciiCaseInsensitiveEqual($0.app, slice.app) }) {
                totals[index].milliseconds += ms
            } else {
                totals.append(AppTotal(app: slice.app, milliseconds: ms))
            }
        }
        let ranked = stableSorted(totals) { $0.milliseconds > $1.milliseconds }
        return Array(ranked.prefix(max(limit, 0)))
    }

    /// Per-(app, window) usage over the block, most-used first: foreground
    /// seconds plus the key presses and clicks (Unix ms, sorted) made while
    /// that window was in front. Input outside every slice (gaps, no front
    /// window) is left unattributed rather than guessed. Beyond the server's
    /// cap the least-used windows are folded into one ``otherWindowsTitle``
    /// row per app (see ``foldBeyondCap(_:cap:)``), so per-app totals stay
    /// complete.
    public static func appUsage(_ slices: [UsageSlice], keyPresses: [Int64], clicks: [Int64]) -> [WindowInfo] {
        var totals: [UsageTotals] = []
        var index: [WindowKey: Int] = [:]
        for slice in slices {
            let key = WindowKey(app: slice.app, title: slice.title)
            let position: Int
            if let existing = index[key] {
                position = existing
            } else {
                position = totals.count
                index[key] = position
                totals.append(UsageTotals(app: slice.app, title: slice.title, firstMs: slice.startMs))
            }
            totals[position].ms += slice.endMs - slice.startMs
            totals[position].keys += countIn(keyPresses, slice.startMs, slice.endMs)
            totals[position].clicks += countIn(clicks, slice.startMs, slice.endMs)
        }
        // Most time first; ties by first appearance, so the order is stable.
        let sorted = stableSorted(totals) { lhs, rhs in
            lhs.ms != rhs.ms ? lhs.ms > rhs.ms : lhs.firstMs < rhs.firstMs
        }
        let rows = sorted
            .map { $0.windowInfo }
            .filter { $0.seconds != 0 || $0.keyboardHits != 0 || $0.mouseClicks != 0 }
        return foldBeyondCap(rows, cap: maxUsageEntries)
    }

    /// Keep `rows` (most-used first) within `cap` without losing time: the
    /// most-used rows stay as they are and the rest are summed into one
    /// ``otherWindowsTitle`` row per app, keeping as many rows as fit (an
    /// app with a single row past the cut keeps its title). Should even one
    /// row per app not fit, the least-used apps share one ``otherAppsName``
    /// row. The result is sorted most-used first again.
    public static func foldBeyondCap(_ rows: [WindowInfo], cap: Int) -> [WindowInfo] {
        guard cap > 0 else { return [] }
        guard rows.count > cap else { return rows }

        // Keep the first `kept` rows, with `kept` as large as possible while
        // the kept rows plus one folded row per app of the rest fit.
        var tailApps = Set<String>()
        var kept = rows.count
        while kept > 0 && kept + tailApps.count > cap {
            kept -= 1
            tailApps.insert(rows[kept].appName)
        }

        var out = Array(rows[..<kept])
        // The rest, grouped by app in order of first appearance.
        var groups: [[WindowInfo]] = []
        for row in rows[kept...] {
            if let position = groups.firstIndex(where: { $0[0].appName == row.appName }) {
                groups[position].append(row)
            } else {
                groups.append([row])
            }
        }
        for group in groups {
            if group.count == 1 {
                // One window takes one row either way: keep its title.
                out.append(group[0])
                continue
            }
            var folded = WindowInfo(appName: group[0].appName, windowTitle: otherWindowsTitle,
                                    seconds: 0, keyboardHits: 0, mouseClicks: 0)
            for row in group {
                addUsage(&folded, row)
            }
            out.append(folded)
        }
        out = sortedMostUsedFirst(out)

        if out.count > cap {
            // More apps than the cap: the least-used ones share a row.
            let rest = out[(cap - 1)...]
            var others = WindowInfo(appName: otherAppsName, windowTitle: "", seconds: 0, keyboardHits: 0, mouseClicks: 0)
            for row in rest {
                addUsage(&others, row)
            }
            out = Array(out[..<(cap - 1)])
            out.append(others)
            out = sortedMostUsedFirst(out)
        }
        return out
    }

    // MARK: Helpers

    private static func addUsage(_ total: inout WindowInfo, _ row: WindowInfo) {
        total.seconds = (total.seconds ?? 0) + (row.seconds ?? 0)
        total.keyboardHits = (total.keyboardHits ?? 0) + (row.keyboardHits ?? 0)
        total.mouseClicks = (total.mouseClicks ?? 0) + (row.mouseClicks ?? 0)
    }

    /// Most seconds first; ties keep their order.
    private static func sortedMostUsedFirst(_ rows: [WindowInfo]) -> [WindowInfo] {
        stableSorted(rows) { ($0.seconds ?? 0) > ($1.seconds ?? 0) }
    }

    /// How many of the sorted timestamps fall in `[startMs, endMs)`.
    static func countIn(_ sortedMs: [Int64], _ startMs: Int64, _ endMs: Int64) -> Int {
        let from = sortedLowerBound(sortedMs, from: 0, startMs)
        let to = max(sortedLowerBound(sortedMs, from: 0, endMs), from)
        return to - from
    }

    static func asciiCaseInsensitiveEqual(_ lhs: String, _ rhs: String) -> Bool {
        lhs.utf8.elementsEqual(rhs.utf8) { asciiLower($0) == asciiLower($1) }
    }

    private static func asciiLower(_ byte: UInt8) -> UInt8 {
        (byte >= 65 && byte <= 90) ? byte + 32 : byte
    }
}

/// `elements` sorted by `areInIncreasingOrder`, keeping the original order of
/// elements that compare equal (Swift's `sorted` does not promise that).
func stableSorted<T>(_ elements: [T], by areInIncreasingOrder: (T, T) -> Bool) -> [T] {
    elements.enumerated()
        .sorted { lhs, rhs in
            if areInIncreasingOrder(lhs.element, rhs.element) { return true }
            if areInIncreasingOrder(rhs.element, lhs.element) { return false }
            return lhs.offset < rhs.offset
        }
        .map { $0.element }
}

/// An (app, window title) pair.
struct WindowKey: Hashable {
    let app: String
    let title: String
}

/// Running totals of one (app, window) over a block.
struct UsageTotals {
    let app: String
    let title: String
    let firstMs: Int64
    var ms: Int64 = 0
    var keys = 0
    var clicks = 0

    init(app: String, title: String, firstMs: Int64) {
        self.app = app
        self.title = title
        self.firstMs = firstMs
    }

    var windowInfo: WindowInfo {
        WindowInfo(appName: app, windowTitle: title, seconds: Int((ms + 500) / 1000), keyboardHits: keys, mouseClicks: clicks)
    }
}
