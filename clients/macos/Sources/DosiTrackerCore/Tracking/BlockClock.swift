import Foundation

/// Block boundaries (CONTRACT §1 "Block timing rules", port of the Windows
/// worker's `record_block` / `save_block_in_progress` and of `split_span`).
///
/// A block runs from where the previous one ended to the moment it is cut
/// (interval elapsed, pause, idle, lock, sleep, …). It never claims more than
/// one interval: after a stall the wall-clock gap can be hours, and the user
/// was not working through it. Blocks shorter than a minute are dropped.
public enum BlockClock {
    public static let minimumIntervalMinutes = 5
    public static let maximumIntervalMinutes = 60

    /// A project's interval clamped to 5...60 minutes, in seconds.
    public static func clampedInterval(minutes: Int) -> TimeInterval {
        TimeInterval(min(max(minutes, minimumIntervalMinutes), maximumIntervalMinutes) * 60)
    }

    /// The span of the block that ends at `end`: it starts at
    /// `blockStartedAt`, but never more than `interval` before `end`. Nil when
    /// the span is empty or inverted (the clock stepped back, a double cut) —
    /// skip it rather than record a block the server would reject.
    public static func span(blockStartedAt: Date, endingAt end: Date, interval: TimeInterval) -> DateInterval? {
        let earliest = end.addingTimeInterval(-interval)
        let start = blockStartedAt > earliest ? blockStartedAt : earliest
        guard start < end else { return nil }
        return DateInterval(start: start, end: end)
    }

    /// Whether a block of this span is long enough to be recorded.
    public static func isRecordable(_ span: DateInterval) -> Bool {
        span.duration >= AgentTiming.minimumBlock
    }

    /// The recordable span of the block that ends at `end` (see
    /// ``span(blockStartedAt:endingAt:interval:)``), nil when there is none
    /// or it is shorter than ``AgentTiming/minimumBlock``.
    public static func recordableSpan(blockStartedAt: Date, endingAt end: Date, interval: TimeInterval) -> DateInterval? {
        guard let span = span(blockStartedAt: blockStartedAt, endingAt: end, interval: interval),
              isRecordable(span)
        else {
            return nil
        }
        return span
    }

    /// When a block that started at `blockStartedAt` is cut by the interval.
    public static func nextCut(blockStartedAt: Date, interval: TimeInterval) -> Date {
        blockStartedAt.addingTimeInterval(interval)
    }

    /// Split `from ..< to` into back-to-back pieces of at most `maxLength`
    /// each, as equal as possible (no sliver at the end; boundaries on whole
    /// milliseconds). Nothing when the span is shorter than
    /// ``AgentTiming/minimumBlock`` or `maxLength` is not positive.
    public static func splitSpan(from: Date, to: Date, maxLength: TimeInterval) -> [DateInterval] {
        let fromMs = EpochTime.milliseconds(from)
        let totalMs = EpochTime.milliseconds(to) - fromMs
        let maxMs = Int64((maxLength * 1000).rounded())
        guard to.timeIntervalSince(from) >= AgentTiming.minimumBlock, totalMs > 0, maxMs > 0 else { return [] }
        let pieces = (totalMs + maxMs - 1) / maxMs
        func boundary(_ index: Int64) -> Date {
            if index == 0 { return from }
            if index == pieces { return to }
            return EpochTime.date(milliseconds: fromMs + totalMs * index / pieces)
        }
        var spans: [DateInterval] = []
        spans.reserveCapacity(Int(pieces))
        for index in 0..<pieces {
            spans.append(DateInterval(start: boundary(index), end: boundary(index + 1)))
        }
        return spans
    }
}

/// When a block's screenshot is taken (CONTRACT §4): once per block, at a
/// random moment between 10% and 90% of the planned interval. A block that
/// ends before that moment, and is long enough to be recorded, takes it at the
/// end instead, unless it ended by lock, sleep, idle or shutdown.
public struct CapturePlan: Equatable, Sendable {
    /// The planned moment is drawn from this share of the interval.
    public static let fractionRange: ClosedRange<Double> = 0.1...0.9

    public let blockStartedAt: Date
    public let captureAt: Date

    /// A plan at `fraction` (clamped to 10-90%) of `interval` after the start.
    public init(blockStartedAt: Date, interval: TimeInterval, fraction: Double) {
        let clamped = min(max(fraction, CapturePlan.fractionRange.lowerBound), CapturePlan.fractionRange.upperBound)
        self.blockStartedAt = blockStartedAt
        self.captureAt = blockStartedAt.addingTimeInterval(interval * clamped)
    }

    /// A plan at a random moment of the block.
    public static func random(blockStartedAt: Date, interval: TimeInterval) -> CapturePlan {
        var generator = SystemRandomNumberGenerator()
        return random(blockStartedAt: blockStartedAt, interval: interval, using: &generator)
    }

    public static func random<G: RandomNumberGenerator>(blockStartedAt: Date, interval: TimeInterval,
                                                       using generator: inout G) -> CapturePlan {
        CapturePlan(blockStartedAt: blockStartedAt, interval: interval,
                    fraction: Double.random(in: fractionRange, using: &generator))
    }

    /// Whether the planned moment has come.
    public func isDue(at now: Date) -> Bool {
        now >= captureAt
    }

    /// Whether a block ending now (`endReason`, nil for an interval cut),
    /// without a capture yet, takes it at the end.
    public static func capturesAtEnd(endReason: EndReason?, span: DateInterval, alreadyCaptured: Bool) -> Bool {
        !alreadyCaptured && (endReason?.allowsCaptureAtEnd ?? true) && BlockClock.isRecordable(span)
    }

    /// Whether a capture taken at `capturedAt` belongs to the block `span`. A
    /// capture taken after the block's end (e.g. while the user was already
    /// idle, before idle was declared) shows time that is not in the block and
    /// is dropped.
    public static func belongs(capturedAt: Date, to span: DateInterval) -> Bool {
        capturedAt >= span.start && capturedAt <= span.end
    }
}

/// The worker's clocks: whether and until when tracking is paused, and where
/// the current stretch and block began (port of the Windows worker's
/// `Clocks`). Timestamps come from the caller, so a pause or quit ends the
/// block at the moment the user acted, not when the worker gets to it.
public struct TrackingClocks: Equatable, Sendable {
    /// Set while paused (an open-ended pause or a timed break).
    public private(set) var pausedSince: Date?
    /// When a timed break ends by itself; nil for an open-ended pause.
    public private(set) var pauseUntil: Date?
    /// Start of the uninterrupted tracking stretch (the session timer).
    public private(set) var trackingSince: Date
    /// Real start of the block being accumulated.
    public private(set) var blockStartedAt: Date
    /// When the block in progress is cut by its interval.
    public private(set) var nextCutAt: Date
    /// Interval of the block in progress (a refreshed interval applies from
    /// the next block on).
    public private(set) var blockInterval: TimeInterval
    /// Interval of the blocks after this one.
    public private(set) var interval: TimeInterval

    public init(now: Date, interval: TimeInterval) {
        pausedSince = nil
        pauseUntil = nil
        trackingSince = now
        blockStartedAt = now
        blockInterval = interval
        self.interval = interval
        nextCutAt = now.addingTimeInterval(interval)
    }

    public var isPaused: Bool { pausedSince != nil }

    /// A timed pause ("Take a break").
    public var isOnBreak: Bool { pausedSince != nil && pauseUntil != nil }

    /// Begin a fresh tracking stretch at `now` (sign-in, resume, unlock, wake,
    /// project switch, return from idle): a new block, the session timer from
    /// zero, and a full interval until the next cut. Clears any pause. Input
    /// made before `now` never counts, because a block only reads input
    /// inside its own span.
    public mutating func startTracking(at now: Date) {
        pausedSince = nil
        pauseUntil = nil
        trackingSince = now
        startBlock(at: now)
    }

    /// Pause at `moment`, resuming by itself at `until` (nil: when the user
    /// resumes). While already paused, only the resume time changes.
    public mutating func pause(at moment: Date, until: Date?) {
        if pausedSince == nil {
            pausedSince = moment
        }
        pauseUntil = until
    }

    /// The block in progress was recorded (or dropped) up to `end`; the next
    /// one starts there, with the current interval.
    public mutating func blockEnded(at end: Date) {
        startBlock(at: end)
    }

    /// Use `newInterval` from the next block on.
    public mutating func setInterval(_ newInterval: TimeInterval) {
        interval = newInterval
    }

    /// Whether the block in progress has reached its cut.
    public func isCutDue(at now: Date) -> Bool {
        !isPaused && now >= nextCutAt
    }

    /// The pause to persist across restarts, if any.
    public var persistedPause: PauseRecord? {
        guard let since = pausedSince else { return nil }
        return PauseRecord(since: since, until: pauseUntil)
    }

    /// Re-enter a pause that was persisted before the app quit.
    public mutating func restore(_ record: PauseRecord) {
        pausedSince = record.since
        pauseUntil = record.until
    }

    private mutating func startBlock(at moment: Date) {
        blockStartedAt = moment
        blockInterval = interval
        nextCutAt = moment.addingTimeInterval(interval)
    }
}

/// A persisted pause: since when, and until when for a timed break.
public struct PauseRecord: Equatable, Sendable {
    public var since: Date
    public var until: Date?

    public init(since: Date, until: Date?) {
        self.since = since
        self.until = until
    }
}

/// Detects a sleep the app was not told about (CONTRACT §4): between two
/// ticks the wall clock advanced more than ``threshold`` beyond a monotonic
/// clock that does not run while the Mac sleeps — use
/// `ProcessInfo.processInfo.systemUptime` (mach absolute time). The block
/// then ends at the last tick before the gap. A wall clock stepping back by
/// more than the threshold is reported too.
public struct SleepGapDetector: Equatable, Sendable {
    public static let threshold: TimeInterval = 90

    public enum Event: Equatable, Sendable {
        case steady
        /// The Mac slept (or the clock jumped forward): end the block at
        /// `lastTickAt`, start a new one at `resumedAt`.
        case gap(lastTickAt: Date, resumedAt: Date, missing: TimeInterval)
        /// The wall clock stepped back by `seconds`.
        case clockSteppedBack(seconds: TimeInterval)
    }

    private var lastWall: Date?
    private var lastMonotonic: TimeInterval?

    public init() {}

    /// Feed one tick: the wall clock and the monotonic clock.
    public mutating func tick(wall: Date, monotonic: TimeInterval) -> Event {
        defer {
            lastWall = wall
            lastMonotonic = monotonic
        }
        guard let previousWall = lastWall, let previousMonotonic = lastMonotonic else { return .steady }
        let drift = wall.timeIntervalSince(previousWall) - (monotonic - previousMonotonic)
        if drift > SleepGapDetector.threshold {
            return .gap(lastTickAt: previousWall, resumedAt: wall, missing: drift)
        }
        if drift < -SleepGapDetector.threshold {
            return .clockSteppedBack(seconds: -drift)
        }
        return .steady
    }

    /// Forget the previous tick (after a known sleep/wake or a restart of the
    /// tick loop).
    public mutating func reset() {
        lastWall = nil
        lastMonotonic = nil
    }
}
