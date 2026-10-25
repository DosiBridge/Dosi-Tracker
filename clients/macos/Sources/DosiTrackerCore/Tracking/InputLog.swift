import Foundation

/// Counts of discrete input events in a window.
public struct InputCounts: Equatable, Sendable {
    public var keyPresses: Int
    public var clicks: Int

    public init(keyPresses: Int, clicks: Int) {
        self.keyPresses = keyPresses
        self.clicks = clicks
    }
}

/// Keyboard and mouse input, recorded as *when* it happened — never what was
/// typed (port of the Windows agent's `InputCounter`, `tracking/input.rs`).
///
/// It keeps a millisecond timestamp per key press and click, the distinct
/// seconds with keyboard or pointer input (for the activity %), per-second
/// counts of software-injected events (mouse jigglers, remote tools — seen,
/// reported, never counted as input) and the health of the capture itself.
///
/// Every count is taken over an explicit time window rather than from counters
/// reset at block boundaries, so block totals, the per-minute timeline and the
/// per-app breakdown always agree, and input made before a block (while
/// paused, signed out, or on another project) can never leak into it.
///
/// Thread-safe: the event tap records from its own thread while the worker
/// queries. Events normally arrive through an ``InputFilter``.
public final class InputLog: @unchecked Sendable {
    /// How far back input is remembered. The longest block (a 60-minute
    /// interval) can end at the last input before the longest idle threshold
    /// (60 minutes) cuts it — and is only built then — plus slack for a slow
    /// capture.
    public static let retentionSeconds: Int64 = 125 * 60
    /// Hard cap on remembered key presses / clicks per kind. Normal typing
    /// stays far below it (~10k an hour); it only bounds memory under an
    /// auto-clicker.
    public static let maxEvents = 200_000

    private let lock = NSLock()
    private var keyPressLog = EventLog()
    private var clickLog = EventLog()
    private var keyboardActivity = ActivityLog()
    private var mouseActivity = ActivityLog()
    private var injectedKeyboard = CountLog()
    private var injectedMouse = CountLog()
    private var health = CaptureHealth()
    private var lastActivityMs: Int64?

    public init() {}

    // MARK: Recording

    /// A real (hardware) key press — a new press, not auto-repeat.
    public func recordKeyPress(atMs ms: Int64) {
        lock.lock()
        defer { lock.unlock() }
        keyPressLog.record(ms)
        keyboardActivity.mark(EpochTime.floorDiv(ms, 1000))
        noteActivity(ms)
    }

    /// A real mouse button press.
    public func recordClick(atMs ms: Int64) {
        lock.lock()
        defer { lock.unlock() }
        clickLog.record(ms)
        mouseActivity.mark(EpochTime.floorDiv(ms, 1000))
        noteActivity(ms)
    }

    /// Real pointer movement, drag or scroll: activity (reading and scrolling
    /// is work), but not a click.
    public func recordPointerActivity(atMs ms: Int64) {
        lock.lock()
        defer { lock.unlock() }
        mouseActivity.mark(EpochTime.floorDiv(ms, 1000))
        noteActivity(ms)
    }

    /// A synthetic (software-posted) keyboard or mouse event. Counted for
    /// `injectedInputEvents` only.
    public func recordInjected(keyboard: Bool, atMs ms: Int64) {
        lock.lock()
        defer { lock.unlock() }
        if keyboard {
            injectedKeyboard.add(EpochTime.floorDiv(ms, 1000))
        } else {
            injectedMouse.add(EpochTime.floorDiv(ms, 1000))
        }
    }

    private func noteActivity(_ ms: Int64) {
        if let last = lastActivityMs, last >= ms { return }
        lastActivityMs = ms
    }

    // MARK: Capture health

    /// The capture (event tap) was started or restarted at `ms`: the silence
    /// clock starts now.
    public func captureStarted(atMs ms: Int64) {
        lock.lock()
        defer { lock.unlock() }
        health.lastEventMs = ms
    }

    /// The capture delivered an event (injected or not) at `ms`: it works,
    /// and an outage, if any, ends here.
    public func sawEvent(atMs ms: Int64) {
        lock.lock()
        defer { lock.unlock() }
        health.sawEvent(ms)
    }

    /// Input capture is unavailable since `ms` (no permission, the tap could
    /// not be created, or it went silent). An ongoing outage keeps its
    /// earlier start.
    public func markCaptureUnavailable(sinceMs ms: Int64) {
        lock.lock()
        defer { lock.unlock() }
        health.markUnhealthy(ms)
    }

    /// Whether the capture is delivering input now (false while it is missing
    /// or was found silent; the window shows "input capture unavailable").
    public var isCaptureHealthy: Bool {
        lock.lock()
        defer { lock.unlock() }
        return health.unhealthySinceMs == 0
    }

    /// When the capture last delivered any event (or was started), Unix ms.
    public var lastCaptureEventMs: Int64 {
        lock.lock()
        defer { lock.unlock() }
        return health.lastEventMs
    }

    /// Was input capture working for all of `[start, end)`? When it was not,
    /// the block's activity is "not measured" rather than a misleading low %.
    public func isCaptureAvailable(from start: Date, to end: Date) -> Bool {
        isCaptureAvailable(fromMs: EpochTime.milliseconds(start), toMs: EpochTime.milliseconds(end))
    }

    public func isCaptureAvailable(fromMs start: Int64, toMs end: Int64) -> Bool {
        lock.lock()
        defer { lock.unlock() }
        return health.availableBetween(start, end)
    }

    // MARK: Queries

    /// Timestamps (Unix ms) of key presses in `[start, end)`.
    public func keyPresses(from start: Date, to end: Date) -> [Int64] {
        keyPresses(fromMs: EpochTime.milliseconds(start), toMs: EpochTime.milliseconds(end))
    }

    public func keyPresses(fromMs start: Int64, toMs end: Int64) -> [Int64] {
        lock.lock()
        defer { lock.unlock() }
        return keyPressLog.between(start, end)
    }

    /// Timestamps (Unix ms) of clicks in `[start, end)`.
    public func clicks(from start: Date, to end: Date) -> [Int64] {
        clicks(fromMs: EpochTime.milliseconds(start), toMs: EpochTime.milliseconds(end))
    }

    public func clicks(fromMs start: Int64, toMs end: Int64) -> [Int64] {
        lock.lock()
        defer { lock.unlock() }
        return clickLog.between(start, end)
    }

    /// Key presses and clicks in `[start, end)` — cheap enough to call every
    /// second for the live view.
    public func counts(from start: Date, to end: Date) -> InputCounts {
        let (from, to) = (EpochTime.milliseconds(start), EpochTime.milliseconds(end))
        lock.lock()
        defer { lock.unlock() }
        return InputCounts(keyPresses: keyPressLog.count(from, to), clicks: clickLog.count(from, to))
    }

    /// Unix seconds inside `start ... end` (both whole seconds, inclusive)
    /// that had input of an allowed kind — what the activity % and the
    /// timeline's active minutes are computed from. A second with both kinds
    /// appears twice.
    public func activeSeconds(from start: Date, to end: Date, keyboard: Bool, mouse: Bool) -> [Int64] {
        activeSeconds(fromSecond: EpochTime.seconds(start), toSecond: EpochTime.seconds(end), keyboard: keyboard, mouse: mouse)
    }

    public func activeSeconds(fromSecond start: Int64, toSecond end: Int64, keyboard: Bool, mouse: Bool) -> [Int64] {
        lock.lock()
        defer { lock.unlock() }
        var seconds: [Int64] = []
        if keyboard {
            seconds.append(contentsOf: keyboardActivity.seconds(from: start, to: end))
        }
        if mouse {
            seconds.append(contentsOf: mouseActivity.seconds(from: start, to: end))
        }
        return seconds
    }

    /// Activity % (0-100) of the block `start ... end`, counting only the
    /// allowed input kinds.
    public func activityPercent(from start: Date, to end: Date, keyboard: Bool, mouse: Bool) -> Int {
        let (first, last) = (EpochTime.seconds(start), EpochTime.seconds(end))
        let seconds = activeSeconds(fromSecond: first, toSecond: last, keyboard: keyboard, mouse: mouse)
        return ActivityMath.activityPercent(activeSeconds: seconds, start: first, end: last)
    }

    /// Injected events of the allowed kinds in the whole seconds
    /// `[start, end)`, so back-to-back blocks never share an event.
    public func injectedCount(from start: Date, to end: Date, keyboard: Bool, mouse: Bool) -> Int {
        injectedCount(fromSecond: EpochTime.seconds(start), toSecond: EpochTime.seconds(end), keyboard: keyboard, mouse: mouse)
    }

    public func injectedCount(fromSecond start: Int64, toSecond end: Int64, keyboard: Bool, mouse: Bool) -> Int {
        lock.lock()
        defer { lock.unlock() }
        var total: Int64 = 0
        if keyboard {
            total += injectedKeyboard.count(start, end)
        }
        if mouse {
            total += injectedMouse.count(start, end)
        }
        return Int(min(total, Int64(Int32.max)))
    }

    /// The latest real input of any kind (keys, clicks, pointer), if any is
    /// remembered.
    public var lastActivity: Date? {
        lock.lock()
        defer { lock.unlock() }
        return lastActivityMs.map { EpochTime.date(milliseconds: $0) }
    }

    /// Whether any real input happened in the whole seconds `start ... end`.
    public func hasActivity(from start: Date, to end: Date) -> Bool {
        let (first, last) = (EpochTime.seconds(start), EpochTime.seconds(end))
        lock.lock()
        defer { lock.unlock() }
        return keyboardActivity.contains(from: first, to: last) || mouseActivity.contains(from: first, to: last)
    }

    /// The start of the first second with real input after the second that
    /// contains `date` (the moment someone came back).
    public func firstActivity(after date: Date) -> Date? {
        let second = EpochTime.seconds(date)
        lock.lock()
        defer { lock.unlock() }
        let candidates = [keyboardActivity.first(after: second), mouseActivity.first(after: second)].compactMap { $0 }
        return candidates.min().map { EpochTime.date(seconds: $0) }
    }

    // MARK: Test hooks

    /// Mark activity seconds directly (tests of the bucketing).
    func markKeyboardActivity(second: Int64) {
        lock.lock()
        defer { lock.unlock() }
        keyboardActivity.mark(second)
    }

    func markMouseActivity(second: Int64) {
        lock.lock()
        defer { lock.unlock() }
        mouseActivity.mark(second)
    }

    /// Distinct injected-count seconds kept (tests of the memory bound).
    var injectedSecondsKept: Int {
        lock.lock()
        defer { lock.unlock() }
        return injectedKeyboard.secondsKept + injectedMouse.secondsKept
    }
}

/// A capture-level input event, reduced to what is counted. The event tap
/// maps its `CGEvent`s to these.
public enum RawInput: Equatable, Sendable {
    /// Key down, with its key code (`kCGKeyboardEventKeycode`); modifier
    /// keys (`flagsChanged`) map to a down when their flag is set.
    case keyDown(Int)
    /// An auto-repeat of a held key (`kCGKeyboardEventAutorepeat` set): never
    /// a new press, and not activity either.
    case keyRepeat(Int)
    /// Key up (or a modifier released).
    case keyUp(Int)
    /// Any mouse button pressed.
    case buttonDown
    case buttonUp
    /// Pointer moved or dragged.
    case move
    /// Scroll wheel or trackpad scroll.
    case wheel
}

/// Turns capture events into counted input (port of the Windows
/// `InputFilter`): a key counts once per physical press however long it is
/// held, a click counts on button-down, movement and the wheel only mark
/// activity, and software-injected events are counted aside, never as input.
/// Every event, injected or not, tells the log that the capture is alive.
public final class InputFilter: @unchecked Sendable {
    /// A key-down for a key already held counts again only after this long
    /// without another down for it. Auto-repeat refreshes the key well within
    /// this, so it only re-arms a key whose key-up was missed (e.g. released
    /// while another app grabbed the keyboard, or the secure input mode).
    public static let missedKeyUpMs: Int64 = 1_500

    private let log: InputLog
    private let lock = NSLock()
    private var allowedKeyboard: Bool
    private var allowedMouse: Bool
    /// Per key: when its latest down (or repeat) arrived, Unix ms. Absent
    /// while the key is up.
    private var keyDownAt: [Int: Int64] = [:]

    public init(log: InputLog, keyboard: Bool = true, mouse: Bool = true) {
        self.log = log
        self.allowedKeyboard = keyboard
        self.allowedMouse = mouse
    }

    /// Set current affirmative input capture permissions.
    public func setPermissions(keyboard: Bool, mouse: Bool) {
        lock.lock()
        defer { lock.unlock() }
        allowedKeyboard = keyboard
        allowedMouse = mouse
        if !keyboard {
            keyDownAt.removeAll()
        }
    }

    /// The currently admitted input kinds.
    public var permissions: (keyboard: Bool, mouse: Bool) {
        lock.lock()
        defer { lock.unlock() }
        return (allowedKeyboard, allowedMouse)
    }

    /// Count one event at `atMs` (Unix ms).
    public func apply(_ input: RawInput, injected: Bool, atMs ms: Int64) {
        lock.lock()
        let kbAllowed = allowedKeyboard
        let mouseAllowed = allowedMouse
        lock.unlock()

        let isKb: Bool
        switch input {
        case .keyDown, .keyRepeat, .keyUp:
            isKb = true
        case .buttonDown, .buttonUp, .move, .wheel:
            isKb = false
        }

        // Native admission gating: drop unconsented events before recording or waking health
        if isKb && !kbAllowed { return }
        if !isKb && !mouseAllowed { return }

        log.sawEvent(atMs: ms)
        if injected {
            // Seen, reported, never counted as the user's input. A release is
            // not an event of its own (as on Windows): a posted key press or
            // click counts once, not twice.
            switch input {
            case .keyDown, .keyRepeat:
                log.recordInjected(keyboard: true, atMs: ms)
            case .buttonDown, .move, .wheel:
                log.recordInjected(keyboard: false, atMs: ms)
            case .keyUp, .buttonUp:
                break
            }
            return
        }
        switch input {
        case .keyDown(let code):
            if isNewPress(code, at: ms) {
                log.recordKeyPress(atMs: ms)
            }
        case .keyRepeat(let code):
            refreshHeld(code, at: ms)
        case .keyUp(let code):
            lock.lock()
            keyDownAt[code] = nil
            lock.unlock()
        case .buttonDown:
            log.recordClick(atMs: ms)
        case .move, .wheel:
            log.recordPointerActivity(atMs: ms)
        case .buttonUp:
            break
        }
    }

    /// Forget which keys are held (their key-ups may have been missed while
    /// the capture was down).
    public func reset() {
        lock.lock()
        defer { lock.unlock() }
        keyDownAt.removeAll()
    }

    /// Is this down a new physical press (not auto-repeat of a held key)?
    private func isNewPress(_ code: Int, at ms: Int64) -> Bool {
        lock.lock()
        defer { lock.unlock() }
        let press: Bool
        if let previous = keyDownAt[code] {
            press = ms - previous > InputFilter.missedKeyUpMs
        } else {
            press = true
        }
        keyDownAt[code] = ms
        return press
    }

    private func refreshHeld(_ code: Int, at ms: Int64) {
        lock.lock()
        defer { lock.unlock() }
        if keyDownAt[code] != nil {
            keyDownAt[code] = ms
        }
    }
}

/// When the input capture should be restarted (port of the Windows hook
/// watchdog): when it is missing, or when the system saw input recently
/// while the capture has been silent for ``silenceLimitMs`` — tried at most
/// once per that limit. The system's idle time comes from
/// `CGEventSourceSecondsSinceLastEventType`.
public enum InputCaptureWatchdog {
    /// How often the watchdog should check.
    public static let checkInterval: TimeInterval = 30
    /// How long the capture may stay silent while the system sees input
    /// before it is restarted (and at most how often that is tried).
    public static let silenceLimitMs: Int64 = 5 * 60 * 1000
    /// Input the system saw within this long counts as recent.
    public static let recentInputMs: Int64 = 60_000

    public static func needsRestart(nowMs: Int64, running: Bool, lastCaptureEventMs: Int64,
                                    systemIdleMs: Int64?, lastAttemptMs: Int64) -> Bool {
        if nowMs - lastAttemptMs < silenceLimitMs {
            return false
        }
        if !running {
            return true
        }
        guard let idle = systemIdleMs else {
            return false
        }
        return idle <= recentInputMs && nowMs - lastCaptureEventMs > silenceLimitMs
    }
}

/// Millisecond timestamps of discrete events (key presses, clicks), oldest
/// first, bounded by ``InputLog/retentionSeconds`` and ``InputLog/maxEvents``.
/// Not thread-safe on its own; ``InputLog`` locks around it.
struct EventLog {
    private var events: [Int64] = []
    /// Index of the oldest live event; earlier slots are dropped lazily.
    private var head = 0

    var liveCount: Int { events.count - head }

    mutating func record(_ atMs: Int64) {
        // Keep the log sorted: a clock stepping backwards files the event with
        // the newest one instead of out of order.
        var at = atMs
        if head < events.count, let newest = events.last, newest > at {
            at = newest
        }
        events.append(at)
        let horizon = at - InputLog.retentionSeconds * 1000
        while head < events.count && (events[head] < horizon || events.count - head > InputLog.maxEvents) {
            head += 1
        }
        compact()
    }

    /// Events in `[startMs, endMs)`.
    func between(_ startMs: Int64, _ endMs: Int64) -> [Int64] {
        Array(events[indices(startMs, endMs)])
    }

    func count(_ startMs: Int64, _ endMs: Int64) -> Int {
        indices(startMs, endMs).count
    }

    private func indices(_ startMs: Int64, _ endMs: Int64) -> Range<Int> {
        let from = sortedLowerBound(events, from: head, startMs)
        let to = max(sortedLowerBound(events, from: head, endMs), from)
        return from..<to
    }

    private mutating func compact() {
        if head > 0 && (head == events.count || (head >= 4096 && head * 2 >= events.count)) {
            events.removeFirst(head)
            head = 0
        }
    }
}

/// Distinct Unix seconds in which input happened, oldest first, bounded to
/// ``InputLog/retentionSeconds`` so memory stays small (< 60 KB even under
/// constant input). Not thread-safe on its own.
struct ActivityLog {
    /// The last second marked: the many events (typing, pointer movement) that
    /// land in the same second skip the rest.
    private var last: Int64?
    private var marked: [Int64] = []
    private var head = 0

    mutating func mark(_ second: Int64) {
        if last == second { return }
        last = second
        // Keep the log sorted; a clock stepping backwards just drops the event.
        if head == marked.count || second > marked[marked.count - 1] {
            marked.append(second)
        }
        while head < marked.count && marked[head] < second - InputLog.retentionSeconds {
            head += 1
        }
        if head > 0 && (head == marked.count || (head >= 1024 && head * 2 >= marked.count)) {
            marked.removeFirst(head)
            head = 0
        }
    }

    /// Remembered seconds in `start ... end` (inclusive).
    func seconds(from start: Int64, to end: Int64) -> [Int64] {
        guard end >= start else { return [] }
        let from = sortedLowerBound(marked, from: head, start)
        let to = max(sortedLowerBound(marked, from: head, end + 1), from)
        return Array(marked[from..<to])
    }

    func contains(from start: Int64, to end: Int64) -> Bool {
        guard end >= start else { return false }
        let index = sortedLowerBound(marked, from: head, start)
        return index < marked.count && marked[index] <= end
    }

    /// The first remembered second strictly after `second`.
    func first(after second: Int64) -> Int64? {
        let index = sortedLowerBound(marked, from: head, second + 1)
        return index < marked.count ? marked[index] : nil
    }
}

/// Event counts per Unix second, oldest first, bounded to
/// ``InputLog/retentionSeconds``: exact totals in constant memory however fast
/// the events come. Not thread-safe on its own.
struct CountLog {
    private var seconds: [Int64] = []
    private var counts: [Int64] = []
    private var head = 0

    var secondsKept: Int { seconds.count - head }

    mutating func add(_ second: Int64) {
        if head < seconds.count, second <= seconds[seconds.count - 1] {
            // Same second — or the clock stepped back: filed with the newest.
            counts[counts.count - 1] += 1
        } else {
            seconds.append(second)
            counts.append(1)
        }
        let newest = seconds[seconds.count - 1]
        while head < seconds.count && seconds[head] < newest - InputLog.retentionSeconds {
            head += 1
        }
        if head > 0 && (head == seconds.count || (head >= 1024 && head * 2 >= seconds.count)) {
            seconds.removeFirst(head)
            counts.removeFirst(head)
            head = 0
        }
    }

    /// Events in the whole seconds `[start, end)`.
    func count(_ start: Int64, _ end: Int64) -> Int64 {
        guard end > start else { return 0 }
        var total: Int64 = 0
        var index = sortedLowerBound(seconds, from: head, start)
        while index < seconds.count && seconds[index] < end {
            total += counts[index]
            index += 1
        }
        return total
    }
}

/// Whether the capture is delivering input, and when it was not. Not
/// thread-safe on its own.
struct CaptureHealth {
    /// When the capture last delivered any event (or was started), Unix ms.
    var lastEventMs: Int64 = 0
    /// Since when capture has been unavailable, Unix ms; 0 while healthy.
    var unhealthySinceMs: Int64 = 0
    /// The latest outage that has ended, `[from, to)` in Unix ms.
    var lastOutage: (from: Int64, to: Int64)?

    mutating func sawEvent(_ ms: Int64) {
        lastEventMs = ms
        if unhealthySinceMs != 0 {
            lastOutage = (unhealthySinceMs, ms)
            unhealthySinceMs = 0
        }
    }

    mutating func markUnhealthy(_ sinceMs: Int64) {
        if unhealthySinceMs == 0 {
            unhealthySinceMs = max(sinceMs, 1)
        }
    }

    func availableBetween(_ startMs: Int64, _ endMs: Int64) -> Bool {
        if unhealthySinceMs != 0 && unhealthySinceMs < endMs {
            return false
        }
        if let outage = lastOutage, outage.from < endMs && outage.to > startMs {
            return false
        }
        return true
    }
}

/// Index of the first element `>= value` in the sorted `values[from...]`.
func sortedLowerBound(_ values: [Int64], from start: Int, _ value: Int64) -> Int {
    var low = start
    var high = values.count
    while low < high {
        let mid = (low + high) / 2
        if values[mid] < value {
            low = mid + 1
        } else {
            high = mid
        }
    }
    return low
}
