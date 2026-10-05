import Foundation

/// A change of the Mac's state the worker reacts to (CONTRACT §4 "Lock",
/// "Sleep / shutdown"), as the app's `SystemEvents` observes it.
public enum SystemEventKind: String, CaseIterable, Sendable {
    /// The Mac is about to sleep (`NSWorkspace.willSleepNotification`).
    case willSleep
    /// The Mac woke up (`NSWorkspace.didWakeNotification`).
    case didWake
    /// The user is logging out, or the Mac is restarting or shutting down
    /// (`NSWorkspace.willPowerOffNotification`).
    case willPowerOff
    /// The session left the screen: fast user switching to another account,
    /// or to the login window (`NSWorkspace.sessionDidResignActiveNotification`).
    case sessionDidResignActive
    /// The session is on the screen again
    /// (`NSWorkspace.sessionDidBecomeActiveNotification`).
    case sessionDidBecomeActive
    /// The screen locked (distributed `com.apple.screenIsLocked`).
    case screenLocked
    /// The screen unlocked (distributed `com.apple.screenIsUnlocked`).
    case screenUnlocked
    /// The displays went to sleep while the Mac keeps running
    /// (`NSWorkspace.screensDidSleepNotification`). Informational: idle
    /// detection covers a user who walked away.
    case screensDidSleep
    /// The displays woke (`NSWorkspace.screensDidWakeNotification`).
    case screensDidWake

    /// How the block in progress ends on this event; nil when it goes on.
    /// Sleep ends it with `sleep`, logging out or shutting down with
    /// `shutdown`, and a lock with `lock` — as does the session leaving the
    /// screen, which Windows also reports as a session lock.
    public var endReason: EndReason? {
        switch self {
        case .willSleep:
            return .sleep
        case .willPowerOff:
            return .shutdown
        case .screenLocked, .sessionDidResignActive:
            return .lock
        case .didWake, .sessionDidBecomeActive, .screenUnlocked, .screensDidSleep, .screensDidWake:
            return nil
        }
    }

    /// Whether the user may be back at work, so a new block can start: after
    /// a wake, an unlock, or the session's return. The worker still checks
    /// that nothing else keeps tracking stopped (e.g. a wake that lands on the
    /// lock screen is followed by `screenUnlocked` only once the user signs
    /// in).
    public var resumesTracking: Bool {
        switch self {
        case .didWake, .sessionDidBecomeActive, .screenUnlocked:
            return true
        case .willSleep, .willPowerOff, .sessionDidResignActive, .screenLocked, .screensDidSleep, .screensDidWake:
            return false
        }
    }
}

/// One system event and the moment it happened. The worker ends the block
/// at `at` — the moment of the lock or sleep — however late it gets to the
/// event (on wake, for a sleep).
public struct SystemEvent: Equatable, Sendable {
    public let kind: SystemEventKind
    public let at: Date

    public init(kind: SystemEventKind, at: Date) {
        self.kind = kind
        self.at = at
    }
}

/// The system's idle clock as a moment.
///
/// Thread-safe: stateless.
public enum SystemIdleTime {
    /// When the last input happened, from the seconds since then that
    /// `CGEventSourceSecondsSinceLastEventType` reports at `now`; nil for a
    /// reading that is not a duration (negative, NaN or infinite).
    public static func lastInput(now: Date, secondsSinceLastInput seconds: Double) -> Date? {
        guard seconds.isFinite, seconds >= 0 else { return nil }
        return now.addingTimeInterval(-seconds)
    }
}
