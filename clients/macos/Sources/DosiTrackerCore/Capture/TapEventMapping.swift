import Foundation

/// Maps what the Quartz event tap delivers — a `CGEvent`'s type, key code,
/// flags and auto-repeat field — to counted input (``RawInput``): the macOS
/// counterpart of the Windows hook's `keyboard_input` / `mouse_input`
/// (`tracking/input.rs`), so both agents count the same presses and clicks.
/// It works on raw values (`CGEventTypes.h`, `IOLLEvent.h`), so it is tested
/// without CoreGraphics.
///
/// * A key-down is a press unless the system flags it as auto-repeat
///   (`kCGKeyboardEventAutorepeat`); a held key never counts again.
/// * Modifier keys arrive as `flagsChanged`. Whether this one went down or up
///   is read from its device-dependent flag (left and right keys apart), or
///   from the generic flag on a keyboard that reports none. Windows counts
///   modifiers as key presses too, so "Hello" is six presses on both.
///   Caps Lock reports a change on each press (on, then off) and none on
///   release, so every change is one press.
/// * Any mouse button down is a click; movement and drags only mark pointer
///   activity, and so does the scroll wheel (trackpad scrolling included).
/// * An event that did not come from the HID system (its source state is not
///   `kCGEventSourceStateHIDSystemState`) was posted by software — a mouse
///   jiggler, a remote-control tool — and is reported aside, never counted.
///
/// Thread-safe: stateless.
public enum TapEventMapping {
    // CGEventType raw values (CGEventTypes.h).
    public static let leftMouseDown: UInt32 = 1
    public static let leftMouseUp: UInt32 = 2
    public static let rightMouseDown: UInt32 = 3
    public static let rightMouseUp: UInt32 = 4
    public static let mouseMoved: UInt32 = 5
    public static let leftMouseDragged: UInt32 = 6
    public static let rightMouseDragged: UInt32 = 7
    public static let keyDown: UInt32 = 10
    public static let keyUp: UInt32 = 11
    public static let flagsChanged: UInt32 = 12
    public static let scrollWheel: UInt32 = 22
    public static let otherMouseDown: UInt32 = 25
    public static let otherMouseUp: UInt32 = 26
    public static let otherMouseDragged: UInt32 = 27

    /// The event types the tap listens to. Key-ups are needed so a key
    /// pressed twice in quick succession counts twice; button-ups are not
    /// (a click counts on button-down).
    public static let listenedTypes: [UInt32] = [
        keyDown, keyUp, flagsChanged,
        leftMouseDown, rightMouseDown, otherMouseDown,
        mouseMoved, leftMouseDragged, rightMouseDragged, otherMouseDragged,
        scrollWheel,
    ]

    /// ``listenedTypes`` as a `CGEventMask`.
    public static let eventMask: UInt64 = listenedTypes.reduce(UInt64(0)) { $0 | (UInt64(1) << UInt64($1)) }

    /// `kCGEventSourceStateHIDSystemState`: the source state of events typed
    /// and clicked on real hardware.
    public static let hidSystemStateID: Int64 = 1

    /// `kVK_CapsLock`.
    public static let capsLockKeyCode = 0x39

    /// All device-dependent modifier bits of the event flags
    /// (`NX_DEVICE…KEYMASK`): left/right control, shift, command, option.
    static let deviceModifierBits: UInt64 = 0x0000_207F

    /// A modifier key: its own device-dependent flag (0 when the system has
    /// none for it) and the generic flag it shares with its twin.
    struct ModifierKey: Sendable {
        let deviceMask: UInt64
        let flagMask: UInt64
    }

    /// Modifier keys by key code (`kVK_…`, HIToolbox Events.h).
    static let modifierKeys: [Int: ModifierKey] = [
        0x37: ModifierKey(deviceMask: 0x0008, flagMask: 0x0010_0000), // command
        0x36: ModifierKey(deviceMask: 0x0010, flagMask: 0x0010_0000), // right command
        0x38: ModifierKey(deviceMask: 0x0002, flagMask: 0x0002_0000), // shift
        0x3C: ModifierKey(deviceMask: 0x0004, flagMask: 0x0002_0000), // right shift
        0x3A: ModifierKey(deviceMask: 0x0020, flagMask: 0x0008_0000), // option
        0x3D: ModifierKey(deviceMask: 0x0040, flagMask: 0x0008_0000), // right option
        0x3B: ModifierKey(deviceMask: 0x0001, flagMask: 0x0004_0000), // control
        0x3E: ModifierKey(deviceMask: 0x2000, flagMask: 0x0004_0000), // right control
        0x3F: ModifierKey(deviceMask: 0, flagMask: 0x0080_0000), // fn / globe
    ]

    /// The counted input of one tap event (none for types it does not count).
    /// - Parameters:
    ///   - eventType: the `CGEventType` raw value.
    ///   - keyCode: `kCGKeyboardEventKeycode` (key events only).
    ///   - flags: the event's `CGEventFlags` raw value.
    ///   - autorepeat: `kCGKeyboardEventAutorepeat` is set (key-downs only).
    public static func inputs(eventType: UInt32, keyCode: Int, flags: UInt64, autorepeat: Bool) -> [RawInput] {
        switch eventType {
        case keyDown:
            return [autorepeat ? .keyRepeat(keyCode) : .keyDown(keyCode)]
        case keyUp:
            return [.keyUp(keyCode)]
        case flagsChanged:
            if keyCode == capsLockKeyCode {
                return [.keyDown(keyCode), .keyUp(keyCode)]
            }
            guard let down = modifierIsDown(keyCode: keyCode, flags: flags) else { return [] }
            return [down ? .keyDown(keyCode) : .keyUp(keyCode)]
        case leftMouseDown, rightMouseDown, otherMouseDown:
            return [.buttonDown]
        case leftMouseUp, rightMouseUp, otherMouseUp:
            return [.buttonUp]
        case mouseMoved, leftMouseDragged, rightMouseDragged, otherMouseDragged:
            return [.move]
        case scrollWheel:
            return [.wheel]
        default:
            return []
        }
    }

    /// Whether an event with this source state (`kCGEventSourceStateID`) was
    /// posted by software rather than typed or clicked on hardware.
    public static func isInjected(sourceStateID: Int64) -> Bool {
        sourceStateID != hidSystemStateID
    }

    /// Whether the modifier `keyCode` is down after a `flagsChanged` event
    /// with `flags`; nil when the key is not a modifier.
    static func modifierIsDown(keyCode: Int, flags: UInt64) -> Bool? {
        guard let key = modifierKeys[keyCode] else { return nil }
        if key.deviceMask != 0 && flags & deviceModifierBits != 0 {
            return flags & key.deviceMask != 0
        }
        return flags & key.flagMask != 0
    }
}

/// Where the input watchdog found the pointer at its checks: the proof that a
/// silent event tap missed real input (port of the Windows hook watchdog's
/// `pointer_moved` and `missed_real_input`).
///
/// Silence alone proves nothing — the system's idle clock also counts input
/// a tap legitimately misses (keys typed into a secure password field) — but
/// a pointer that moved while the tap reported nothing does. Only then is the
/// capture marked unavailable, so the blocks around it read "not measured"
/// instead of a misleading low activity %.
///
/// Not thread-safe on its own; the watchdog owns it.
public struct PointerWatch: Equatable, Sendable {
    /// The position found at the previous check, if it was known.
    public private(set) var lastX: Double?
    public private(set) var lastY: Double?
    /// The latest check that found the pointer somewhere else than the check
    /// before, Unix ms.
    public private(set) var movedAtMs: Int64?

    public init() {}

    /// Record the pointer position found at a check (nil when the system
    /// would not say). A move counts only between two known positions.
    public mutating func observe(x: Double?, y: Double?, atMs ms: Int64) {
        if let x = x, let y = y, let previousX = lastX, let previousY = lastY, x != previousX || y != previousY {
            movedAtMs = ms
        }
        lastX = x
        lastY = y
    }

    /// Whether the pointer provably moved after the capture's last event.
    /// The move happened at some point in the check interval that ended at
    /// ``movedAtMs``, so it counts only when even the start of that interval
    /// is after the last event.
    public func missedInput(lastCaptureEventMs: Int64, checkIntervalMs: Int64) -> Bool {
        guard let moved = movedAtMs else { return false }
        return moved - checkIntervalMs > lastCaptureEventMs
    }

    /// Forget everything (the capture was restarted).
    public mutating func reset() {
        lastX = nil
        lastY = nil
        movedAtMs = nil
    }
}
