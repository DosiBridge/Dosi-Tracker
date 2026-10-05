import CoreGraphics
import DosiTrackerCore
import Foundation

/// The system's own idle clock: how long since any keyboard or mouse input
/// (`CGEventSourceSecondsSinceLastEventType`), the macOS counterpart of the
/// Windows agent's `GetLastInputInfo` reading. It needs no permission.
///
/// The combined session state counts every event of the session, including
/// input posted by software (a mouse jiggler keeps it at zero), so idle
/// detection should prefer the input log's own `lastActivity` — real input
/// only — while the event tap is healthy, and fall back to this when it is
/// not (as the Windows worker does). The HID system state counts hardware
/// input only.
///
/// Thread-safe: stateless; the call may be made from any thread.
public final class IdleMonitor: SystemIdleSource, @unchecked Sendable {
    /// `kCGAnyInputEventType` (`(CGEventType)~0`), which Swift does not
    /// import: any keyboard or mouse event. (The raw value is also that of
    /// `.tapDisabledByUserInput`, so the initializer cannot fail.)
    public static let anyInputEventType: CGEventType = CGEventType(rawValue: ~0)!

    private let stateID: CGEventSourceStateID

    /// - Parameter stateID: `.combinedSessionState` (all input of the
    ///   session) or `.hidSystemState` (hardware input only).
    public init(stateID: CGEventSourceStateID = .combinedSessionState) {
        self.stateID = stateID
    }

    /// Seconds since the last keyboard or mouse input; nil when the system
    /// gives no usable reading.
    public func secondsSinceLastInput() -> TimeInterval? {
        let seconds = CGEventSource.secondsSinceLastEventType(stateID, eventType: IdleMonitor.anyInputEventType)
        guard seconds.isFinite, seconds >= 0 else { return nil }
        return seconds
    }

    public func lastInputAt(now: Date) -> Date? {
        let seconds = CGEventSource.secondsSinceLastEventType(stateID, eventType: IdleMonitor.anyInputEventType)
        return SystemIdleTime.lastInput(now: now, secondsSinceLastInput: seconds)
    }
}
