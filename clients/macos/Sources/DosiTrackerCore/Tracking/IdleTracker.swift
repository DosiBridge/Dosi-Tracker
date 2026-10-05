import Foundation

/// A stretch the user was away: from where the block was cut (the last input,
/// or the start of the block if later) to the input that brought them back.
public struct AwaySpan: Equatable, Sendable {
    public let start: Date
    public let end: Date

    public init(start: Date, end: Date) {
        self.start = start
        self.end = end
    }

    public var duration: TimeInterval { max(end.timeIntervalSince(start), 0) }

    /// Whether keeping it could record anything (at least one minimum block),
    /// i.e. whether the "Keep as work / Discard" prompt is worth showing.
    public var isWorthAsking: Bool { duration >= AgentTiming.minimumBlock }

    /// Whole minutes away, for the prompt ("You were away for 42 min").
    public var roundedMinutes: Int { Int((duration / 60).rounded()) }

    /// The span cut into back-to-back blocks of at most `interval` each, as
    /// equal as possible (``BlockClock/splitSpan(from:to:maxLength:)``, the
    /// Windows `split_span`); nothing when it is shorter than a minute.
    public func keptBlocks(interval: TimeInterval) -> [DateInterval] {
        BlockClock.splitSpan(from: start, to: end, maxLength: interval)
    }
}

/// The user's answer to "You were away for 42 min (10:05 – 10:47)".
public enum AwayChoice: Equatable, Sendable {
    /// Record the away time as work: idle blocks of at most one interval each
    /// (``BlockAssembler/keptIdleBlocks(projectId:span:interval:description:device:timeZone:)``).
    case keep
    /// Record nothing (also what happens without an answer).
    case discard
}

/// What the worker must do after an ``IdleTracker`` update.
public enum IdleEvent: Equatable, Sendable {
    case none
    /// No input for the idle threshold: end the block at `cutAt` (end reason
    /// `idle`; dropped if shorter than a minute; no capture at the end), stop
    /// capturing, report presence `idle`.
    case wentIdle(cutAt: Date)
    /// Input is back: start a new block at `span.end` and, when
    /// `span.isWorthAsking`, ask whether to keep the away time.
    case returned(AwaySpan)
}

/// When a timed break that has run out lets tracking resume.
public enum BreakResume: Equatable, Sendable {
    /// The break is still running.
    case notYet
    /// Resume tracking with a new block starting at this moment.
    case resume(at: Date)
    /// The break is over but the user is away: keep waiting, and resume when
    /// input returns (call again with `awaitingReturn: true`).
    case awaitingReturn
}

/// Idle → away → return as a pure state machine (CONTRACT §4 "Idle").
///
/// While tracking, the worker calls
/// ``update(now:lastInputAt:trackingSince:blockStartedAt:threshold:)`` on
/// every tick (every few seconds) with the latest input the system saw (its
/// idle time, as the Windows agent reads `GetLastInputInfo`). After
/// `threshold` without input the block ends at the last input; when input
/// returns, a new block starts at that moment and the away stretch waits for
/// the user's Keep / Discard answer. An unanswered prompt counts as Discard;
/// it is dropped when the user goes idle again.
public struct IdleTracker: Equatable, Sendable {
    public enum Phase: Equatable, Sendable {
        case active
        /// Idle was declared; the block was cut at `since`.
        case away(since: Date)
    }

    public private(set) var phase: Phase = .active
    /// The away stretch waiting for Keep / Discard, if any.
    public private(set) var pendingAway: AwaySpan?

    public init() {}

    public var isAway: Bool {
        if case .away = phase { return true }
        return false
    }

    /// Advance the state machine.
    /// - Parameters:
    ///   - lastInputAt: the latest input the system saw (nil if unknown).
    ///   - trackingSince: start of the current tracking stretch; inactivity is
    ///     never counted from before it (e.g. from before a wake or unlock).
    ///   - blockStartedAt: the block in progress never ends before its start.
    ///   - threshold: seconds without input before the user is away; 0 = off.
    public mutating func update(now: Date, lastInputAt: Date?, trackingSince: Date, blockStartedAt: Date,
                                threshold: TimeInterval) -> IdleEvent {
        switch phase {
        case .active:
            guard threshold > 0 else { return .none }
            let quietSince = max(lastInputAt ?? trackingSince, trackingSince)
            guard now.timeIntervalSince(quietSince) >= threshold else { return .none }
            let cut = max(quietSince, blockStartedAt)
            phase = .away(since: cut)
            pendingAway = nil
            return .wentIdle(cutAt: cut)
        case .away(let since):
            guard let last = lastInputAt, last > since else { return .none }
            phase = .active
            let span = AwaySpan(start: since, end: last)
            pendingAway = span.isWorthAsking ? span : nil
            return .returned(span)
        }
    }

    /// The away stretch ended without the user coming back to work: the screen
    /// locked, the Mac slept, the user paused, signed out or quit. Returns the
    /// span (also kept as ``pendingAway`` when worth asking about).
    @discardableResult
    public mutating func interrupt(at moment: Date) -> AwaySpan? {
        guard case .away(let since) = phase else { return nil }
        phase = .active
        guard moment > since else {
            pendingAway = nil
            return nil
        }
        let span = AwaySpan(start: since, end: moment)
        pendingAway = span.isWorthAsking ? span : nil
        return span
    }

    /// Answer the prompt. Returns the span to record when kept.
    public mutating func resolve(_ choice: AwayChoice) -> AwaySpan? {
        let span = pendingAway
        pendingAway = nil
        return choice == .keep ? span : nil
    }

    /// Forget everything (sign-out, project switch).
    public mutating func reset() {
        phase = .active
        pendingAway = nil
    }

    /// Whether, and from when, tracking resumes after a timed break that was
    /// planned to end at `pauseUntil` ("A timed break that ends while the user
    /// is away resumes only when input returns", CONTRACT §4). Call on every
    /// tick while on a break.
    ///
    /// The user counts as present at the end when the latest input is less
    /// than `threshold` before it (or idle detection is off): tracking resumes
    /// at the planned end. Otherwise the break waits (``BreakResume/awaitingReturn``)
    /// and, on later calls with `awaitingReturn: true`, resumes at the first
    /// input seen after the end.
    /// - Parameters:
    ///   - lastInputAt: the latest input the system saw.
    ///   - notBefore: the latest wake, unlock or launch: tracking never
    ///     resumes before it (the Mac may have slept through the end).
    public static func breakResume(now: Date, pauseUntil: Date, threshold: TimeInterval, lastInputAt: Date?,
                                   awaitingReturn: Bool, notBefore: Date? = nil) -> BreakResume {
        guard now >= pauseUntil else { return .notYet }
        let floor = max(pauseUntil, notBefore ?? pauseUntil)
        if awaitingReturn {
            guard let last = lastInputAt, last > floor else { return .awaitingReturn }
            return .resume(at: min(last, now))
        }
        if threshold <= 0 {
            return .resume(at: min(floor, now))
        }
        guard let last = lastInputAt, floor.timeIntervalSince(last) < threshold else {
            return .awaitingReturn
        }
        return .resume(at: min(floor, now))
    }
}
