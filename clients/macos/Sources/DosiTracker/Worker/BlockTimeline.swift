import DosiTrackerCore
import Foundation

// Block timing: where each block starts and ends (CONTRACT §1 "Block timing
// rules" and §4) — a one-to-one port of the Windows agent's
// `worker/blocks.rs`, so both platforms cut the same blocks.
//
// Pure logic driven by explicit instants, so every rule is testable without a
// clock or the OS:
//
// * While tracking, blocks follow one another without gaps and never exceed
//   the interval.
// * A block whose end finds the user inactive is *held*: if input comes back
//   it is kept as it is; if the idle threshold confirms the user left, the cut
//   lands at their last input — even when that was in an earlier block. (The
//   input and usage logs keep two hours, enough to build a held block late.)
// * The user's own actions end the block at the moment they acted.
// * A block shorter than a minute is dropped.

/// What a block is recorded under, fixed when the block starts: a refreshed
/// policy applies from the next block on.
struct BlockPolicy: Equatable, Sendable {
    var projectId: String
    var permissions: CapturePermissions
    /// Screen captures must be blurred before upload.
    var blurScreenshots: Bool
    /// The planned length of a block (5...60 min), in seconds.
    var interval: TimeInterval
    /// No input for this long ends the block at the last input; nil when the
    /// project turned idle detection off.
    var idleThreshold: TimeInterval?

    init(projectId: String, permissions: CapturePermissions, blurScreenshots: Bool, interval: TimeInterval,
         idleThreshold: TimeInterval?) {
        self.projectId = projectId
        self.permissions = permissions
        self.blurScreenshots = blurScreenshots
        self.interval = interval
        self.idleThreshold = idleThreshold
    }

    /// The policy of `session` (its permissions already combine the server's
    /// project policy with the local configuration).
    init(session: TrackingSession) {
        self.init(projectId: session.projectId, permissions: session.permissions,
                  blurScreenshots: session.blurScreenshots, interval: session.interval,
                  idleThreshold: session.idleThreshold > 0 ? session.idleThreshold : nil)
    }
}

/// The block being accumulated.
struct OpenBlock: Equatable, Sendable {
    let id: Int
    let startedAt: Date
    let policy: BlockPolicy
    var description: String?
    /// When its screenshot is taken: a random moment in 10%...90% of the
    /// interval, so nobody can tidy their screen for it.
    let captureAt: Date
    /// The screenshot moment has been handled.
    var captured: Bool

    /// Where the block ends if nothing ends it earlier.
    var plannedEnd: Date { startedAt.addingTimeInterval(policy.interval) }
}

/// A block whose end is final: to be built and queued.
struct FinishedBlock: Equatable, Sendable {
    let id: Int
    let startedAt: Date
    var endedAt: Date
    var endReason: EndReason
    let policy: BlockPolicy
    let description: String?
    /// It ended before its screenshot moment: one may be taken at the end
    /// instead (unless the end reason forbids it).
    var captureAtEnd: Bool

    var length: TimeInterval { endedAt.timeIntervalSince(startedAt) }
}

/// One uninterrupted stretch of tracking.
struct Stretch: Equatable, Sendable {
    /// When it began (the session clock).
    let since: Date
    var block: OpenBlock
    /// Finished blocks waiting to see whether the user is still there.
    fileprivate(set) var held: [FinishedBlock]
    /// The latest activity when the first of them was held: input after it
    /// releases them.
    fileprivate(set) var heldActivity: Date?
    /// What the next block is recorded under.
    fileprivate(set) var policy: BlockPolicy
    fileprivate(set) var description: String?

    /// Where the time not yet finished begins: the first held block, else the
    /// open one.
    var unsavedSince: Date { held.first?.startedAt ?? block.startedAt }

    /// Blocks are held waiting for the user.
    var holding: Bool { !held.isEmpty }

    /// The blocks not finished yet: the held ones and the open one.
    var blockIds: [Int] { held.map { $0.id } + [block.id] }
}

/// What a tick decided.
struct TickOutcome: Equatable, Sendable {
    /// Blocks to build and queue, oldest first.
    var finished: [FinishedBlock] = []
    /// The idle threshold passed: the stretch ended at this, the user's last
    /// activity.
    var wentIdle: Date?
}

/// The blocks of the tracking stretch in progress, if any.
struct BlockTimeline: Sendable {
    /// A block whose end finds no input within this long of it is held until
    /// the user shows they are still there.
    static let holdGrace: TimeInterval = 5

    private(set) var stretch: Stretch?
    private var nextId = 0

    init() {}

    var isOpen: Bool { stretch != nil }

    /// The screenshot moment of a block starting at `startedAt`: `fraction`
    /// (0...1) mapped into 10%...90% of the interval, on whole milliseconds.
    static func captureMoment(startedAt: Date, interval: TimeInterval, fraction: Double) -> Date {
        let clamped = fraction.isFinite ? min(max(fraction, 0), 1) : 0.5
        let share = 0.1 + 0.8 * clamped
        let offsetMs = (interval * 1000 * share).rounded(.towardZero)
        return startedAt.addingTimeInterval(offsetMs / 1000)
    }

    /// A uniformly random fraction in `[0, 1)` (the screenshot moment).
    static func randomFraction() -> Double {
        Double.random(in: 0..<1)
    }

    /// Start tracking at `at` (a no-op while a stretch is open).
    mutating func start(at: Date, policy: BlockPolicy, description: String?, fraction: Double) {
        guard stretch == nil else { return }
        let block = makeBlock(at: at, policy: policy, description: description, fraction: fraction)
        stretch = Stretch(since: at, block: block, held: [], heldActivity: nil, policy: policy, description: description)
    }

    /// Advance to `now`. `lastActivity` is the user's latest keyboard or mouse
    /// input (nil: none seen); input before the stretch began counts as the
    /// stretch's start.
    mutating func tick(now: Date, lastActivity: Date?, fraction: () -> Double) -> TickOutcome {
        var outcome = TickOutcome()
        guard var current = stretch else { return outcome }
        let activity = lastActivity.map { max($0, current.since) } ?? current.since

        // Input since the hold — or idle detection turned off meanwhile —
        // means the held blocks were work.
        let idleOn = current.block.policy.idleThreshold != nil
        let inputSinceHold = current.heldActivity.map { activity > $0 } ?? false
        if current.holding && (!idleOn || inputSinceHold) {
            outcome.finished.append(contentsOf: BlockTimeline.release(&current))
        }

        outcome.finished.append(contentsOf: roll(&current, until: now, holdAgainst: idleOn ? activity : nil,
                                                 fraction: fraction))

        if let threshold = current.block.policy.idleThreshold, now.timeIntervalSince(activity) >= threshold {
            outcome.finished.append(contentsOf: BlockTimeline.cutAtLastActivity(current, now: now, last: activity))
            outcome.wentIdle = activity
            stretch = nil
        } else {
            stretch = current
        }
        outcome.finished = outcome.finished.filter(BlockTimeline.longEnough)
        return outcome
    }

    /// Stop tracking: the held blocks are kept as they are, and the open block
    /// ends at `at` (the moment it happened, never later than `now`) by
    /// `reason`.
    mutating func end(at: Date, now: Date, reason: EndReason) -> [FinishedBlock] {
        guard var current = stretch else { return [] }
        stretch = nil
        let end = min(at, now)
        var finished = BlockTimeline.release(&current)
        finished.append(contentsOf: roll(&current, until: end, holdAgainst: nil, fraction: { 0.5 }))
        finished.append(BlockTimeline.finish(current.block, at: end, reason: reason))
        return finished.filter(BlockTimeline.longEnough)
    }

    /// End the open block at `at` by `reason` and start the next one there;
    /// the stretch goes on (a manual sync, a task change).
    mutating func cut(at: Date, now: Date, reason: EndReason, fraction: Double) -> [FinishedBlock] {
        guard var current = stretch else { return [] }
        var end = min(at, now)
        var finished = BlockTimeline.release(&current)
        finished.append(contentsOf: roll(&current, until: end, holdAgainst: nil, fraction: { 0.5 }))
        end = max(end, current.block.startedAt)
        finished.append(BlockTimeline.finish(current.block, at: end, reason: reason))
        current.block = makeBlock(at: end, policy: current.policy, description: current.description, fraction: fraction)
        stretch = current
        return finished.filter(BlockTimeline.longEnough)
    }

    /// The task note of the next blocks; with `relabelOpen`, of the open block
    /// too.
    mutating func setDescription(_ description: String?, relabelOpen: Bool) {
        guard var current = stretch else { return }
        if relabelOpen {
            current.block.description = description
        }
        current.description = description
        stretch = current
    }

    /// A refreshed policy for the same project, from the next block on.
    mutating func updatePolicy(_ policy: BlockPolicy) {
        guard var current = stretch else { return }
        current.policy = policy
        stretch = current
    }

    /// The open block whose screenshot moment has come, if it is due one.
    func captureDue(now: Date) -> Int? {
        guard let block = stretch?.block else { return nil }
        guard !block.captured, block.policy.permissions.screenshot, now >= block.captureAt else { return nil }
        return block.id
    }

    /// The screenshot moment of block `id` has been handled.
    mutating func markCaptured(_ id: Int) {
        guard var current = stretch, current.block.id == id else { return }
        current.block.captured = true
        stretch = current
    }

    // MARK: Steps

    private mutating func makeBlock(at: Date, policy: BlockPolicy, description: String?, fraction: Double) -> OpenBlock {
        nextId += 1
        return OpenBlock(
            id: nextId,
            startedAt: at,
            policy: policy,
            description: description,
            captureAt: BlockTimeline.captureMoment(startedAt: at, interval: policy.interval, fraction: fraction),
            captured: false
        )
    }

    private static func finish(_ block: OpenBlock, at end: Date, reason: EndReason) -> FinishedBlock {
        let endedAt = max(end, block.startedAt)
        return FinishedBlock(
            id: block.id,
            startedAt: block.startedAt,
            endedAt: endedAt,
            endReason: reason,
            policy: block.policy,
            description: block.description,
            captureAtEnd: !block.captured && endedAt < block.captureAt
        )
    }

    private static func release(_ stretch: inout Stretch) -> [FinishedBlock] {
        stretch.heldActivity = nil
        let held = stretch.held
        stretch.held = []
        return held
    }

    /// Finish every interval boundary up to `until`, opening the next block at
    /// each. With `holdAgainst` (the latest activity, when idle detection is
    /// on), a block whose end finds the user inactive is held rather than
    /// finished — and so is every one after it.
    private mutating func roll(_ stretch: inout Stretch, until: Date, holdAgainst: Date?,
                               fraction: () -> Double) -> [FinishedBlock] {
        var finished: [FinishedBlock] = []
        while stretch.block.policy.interval > 0 && stretch.block.plannedEnd <= until {
            let end = stretch.block.plannedEnd
            let block = BlockTimeline.finish(stretch.block, at: end, reason: .interval)
            let inactive = holdAgainst.map { end.timeIntervalSince($0) > BlockTimeline.holdGrace } ?? false
            if inactive || stretch.holding {
                if !stretch.holding {
                    stretch.heldActivity = holdAgainst
                }
                stretch.held.append(block)
            } else {
                finished.append(block)
            }
            stretch.block = makeBlock(at: end, policy: stretch.policy, description: stretch.description,
                                      fraction: fraction())
        }
        return finished
    }

    /// The user left at `last`: what was tracked after it is not work. Blocks
    /// ending before it stay, the one it falls in ends there (by `idle`, with
    /// no capture), and later ones are dropped.
    private static func cutAtLastActivity(_ stretch: Stretch, now: Date, last: Date) -> [FinishedBlock] {
        let open = finish(stretch.block, at: now, reason: .idle)
        return (stretch.held + [open]).compactMap { candidate -> FinishedBlock? in
            if candidate.endedAt <= last {
                return candidate
            }
            guard candidate.startedAt < last else { return nil }
            var cut = candidate
            cut.endedAt = last
            cut.endReason = .idle
            cut.captureAtEnd = false
            return cut
        }
    }

    private static func longEnough(_ block: FinishedBlock) -> Bool {
        block.length >= AgentTiming.minimumBlock
    }
}
