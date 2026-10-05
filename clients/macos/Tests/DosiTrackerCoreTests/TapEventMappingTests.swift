import XCTest
@testable import DosiTrackerCore

/// The event tap's mapping to counted input — the macOS side of the Windows
/// agent's `hook_messages_are_classified` and filter tests — and the
/// watchdog's pointer proof.
final class TapEventMappingTests: XCTestCase {
    private typealias Map = TapEventMapping

    /// Event flags as the system sets them: the generic flag plus the key's
    /// device flag, and the non-coalesced bit most events carry.
    private let leftShift: UInt64 = 0x0002_0000 | 0x0002
    private let rightShift: UInt64 = 0x0002_0000 | 0x0004
    private let rightCommand: UInt64 = 0x0010_0000 | 0x0010
    private let plain: UInt64 = 0x0100

    /// One tap event, as the tap reads it.
    private struct TapEvent {
        var type: UInt32
        var key: Int = 0
        var flags: UInt64 = 0x0100
        var autorepeat = false
        var injected = false
        var ms: Int64
    }

    private func inputs(_ type: UInt32, key: Int = 0, flags: UInt64 = 0x0100, autorepeat: Bool = false) -> [RawInput] {
        Map.inputs(eventType: type, keyCode: key, flags: flags, autorepeat: autorepeat)
    }

    /// Feed tap events (ms after t0) through the mapping and a fresh filter.
    private func feed(_ events: [TapEvent]) -> InputLog {
        let log = InputLog()
        let filter = InputFilter(log: log)
        for event in events {
            for input in Map.inputs(eventType: event.type, keyCode: event.key, flags: event.flags,
                                    autorepeat: event.autorepeat) {
                filter.apply(input, injected: event.injected, atMs: t0Ms + event.ms)
            }
        }
        return log
    }

    func testKeysArePressesUnlessTheSystemFlagsARepeat() {
        XCTAssertEqual(inputs(Map.keyDown, key: 0x04), [.keyDown(0x04)])
        XCTAssertEqual(inputs(Map.keyDown, key: 0x04, autorepeat: true), [.keyRepeat(0x04)])
        XCTAssertEqual(inputs(Map.keyUp, key: 0x04), [.keyUp(0x04)])
    }

    func testButtonsMovesDragsAndTheWheel() {
        for type in [Map.leftMouseDown, Map.rightMouseDown, Map.otherMouseDown] {
            XCTAssertEqual(inputs(type), [.buttonDown], "type \(type)")
        }
        for type in [Map.leftMouseUp, Map.rightMouseUp, Map.otherMouseUp] {
            XCTAssertEqual(inputs(type), [.buttonUp], "type \(type)")
        }
        for type in [Map.mouseMoved, Map.leftMouseDragged, Map.rightMouseDragged, Map.otherMouseDragged] {
            XCTAssertEqual(inputs(type), [.move], "type \(type)")
        }
        XCTAssertEqual(inputs(Map.scrollWheel), [.wheel])
        // Anything else (the null event, tablet, gestures, the tap's own
        // disabled notices) is not input.
        for type: UInt32 in [0, 13, 14, 23, 24, 29, 0xFFFF_FFFE, 0xFFFF_FFFF] {
            XCTAssertEqual(inputs(type), [], "type \(type)")
        }
    }

    func testModifiersGoDownAndUpByTheirOwnDeviceFlag() {
        XCTAssertEqual(inputs(Map.flagsChanged, key: 0x38, flags: leftShift), [.keyDown(0x38)])
        XCTAssertEqual(inputs(Map.flagsChanged, key: 0x3C, flags: leftShift | rightShift), [.keyDown(0x3C)])
        // The left shift released while the right one is held: the generic
        // shift flag stays set, the left key's own flag does not.
        XCTAssertEqual(inputs(Map.flagsChanged, key: 0x38, flags: rightShift), [.keyUp(0x38)])
        XCTAssertEqual(inputs(Map.flagsChanged, key: 0x3C, flags: plain), [.keyUp(0x3C)])
        // Right command while the left shift is held, then released.
        XCTAssertEqual(inputs(Map.flagsChanged, key: 0x36, flags: leftShift | rightCommand), [.keyDown(0x36)])
        XCTAssertEqual(inputs(Map.flagsChanged, key: 0x36, flags: leftShift), [.keyUp(0x36)])
        // Control and option, left and right.
        XCTAssertEqual(inputs(Map.flagsChanged, key: 0x3B, flags: 0x0004_0000 | 0x0001), [.keyDown(0x3B)])
        XCTAssertEqual(inputs(Map.flagsChanged, key: 0x3E, flags: 0x0004_0000 | 0x2000), [.keyDown(0x3E)])
        XCTAssertEqual(inputs(Map.flagsChanged, key: 0x3A, flags: 0x0008_0000 | 0x0020), [.keyDown(0x3A)])
        XCTAssertEqual(inputs(Map.flagsChanged, key: 0x3D, flags: 0x0008_0000 | 0x0020), [.keyUp(0x3D)])
    }

    func testKeyboardsWithoutDeviceFlagsFallBackToTheGenericFlag() {
        XCTAssertEqual(inputs(Map.flagsChanged, key: 0x3B, flags: 0x0004_0000), [.keyDown(0x3B)])
        XCTAssertEqual(inputs(Map.flagsChanged, key: 0x3B, flags: 0), [.keyUp(0x3B)])
        // Fn (the globe key) has no device flag at all.
        XCTAssertEqual(inputs(Map.flagsChanged, key: 0x3F, flags: 0x0080_0000), [.keyDown(0x3F)])
        XCTAssertEqual(inputs(Map.flagsChanged, key: 0x3F, flags: leftShift), [.keyUp(0x3F)])
    }

    func testCapsLockCountsEveryChangeAsOnePress() {
        XCTAssertEqual(inputs(Map.flagsChanged, key: Map.capsLockKeyCode, flags: 0x0001_0000),
                       [.keyDown(Map.capsLockKeyCode), .keyUp(Map.capsLockKeyCode)])
        XCTAssertEqual(inputs(Map.flagsChanged, key: Map.capsLockKeyCode, flags: plain),
                       [.keyDown(Map.capsLockKeyCode), .keyUp(Map.capsLockKeyCode)])
    }

    func testUnknownModifierChangesAreIgnored() {
        XCTAssertEqual(inputs(Map.flagsChanged, key: 0x00, flags: leftShift), [])
        XCTAssertEqual(inputs(Map.flagsChanged, key: 0x7F, flags: plain), [])
    }

    func testOnlyHardwareEventsAreTheUsersInput() {
        XCTAssertFalse(Map.isInjected(sourceStateID: 1)) // kCGEventSourceStateHIDSystemState
        XCTAssertTrue(Map.isInjected(sourceStateID: 0)) // combined session state: posted by an app
        XCTAssertTrue(Map.isInjected(sourceStateID: -1)) // a private state
        XCTAssertTrue(Map.isInjected(sourceStateID: 1_234_567)) // a private source's own id
    }

    func testTheTapListensToKeysButtonDownsMovesAndTheWheelOnly() {
        let mask = Map.eventMask
        for type in [Map.keyDown, Map.keyUp, Map.flagsChanged, Map.leftMouseDown, Map.rightMouseDown, Map.otherMouseDown,
                     Map.mouseMoved, Map.leftMouseDragged, Map.rightMouseDragged, Map.otherMouseDragged, Map.scrollWheel] {
            XCTAssertNotEqual(mask & (UInt64(1) << UInt64(type)), 0, "type \(type)")
        }
        for type in [Map.leftMouseUp, Map.rightMouseUp, Map.otherMouseUp] {
            XCTAssertEqual(mask & (UInt64(1) << UInt64(type)), 0, "type \(type)")
        }
        XCTAssertEqual(mask, 0x0A40_1CEA)
    }

    func testTypingHelloCountsSixPressesAsOnWindows() {
        // Shift, H, e, l, l, o: the double l in quick succession counts twice
        // because its key-up is seen.
        let log = feed([
            TapEvent(type: Map.flagsChanged, key: 0x38, flags: leftShift, ms: 0),
            TapEvent(type: Map.keyDown, key: 0x04, flags: leftShift, ms: 60),
            TapEvent(type: Map.keyUp, key: 0x04, flags: leftShift, ms: 120),
            TapEvent(type: Map.flagsChanged, key: 0x38, ms: 150),
            TapEvent(type: Map.keyDown, key: 0x0E, ms: 250),
            TapEvent(type: Map.keyUp, key: 0x0E, ms: 300),
            TapEvent(type: Map.keyDown, key: 0x25, ms: 400),
            TapEvent(type: Map.keyUp, key: 0x25, ms: 450),
            TapEvent(type: Map.keyDown, key: 0x25, ms: 520),
            TapEvent(type: Map.keyUp, key: 0x25, ms: 570),
            TapEvent(type: Map.keyDown, key: 0x1F, ms: 650),
            TapEvent(type: Map.keyUp, key: 0x1F, ms: 700),
        ])
        XCTAssertEqual(log.counts(from: at(t0), to: at(t0 + 1)), InputCounts(keyPresses: 6, clicks: 0))
        XCTAssertEqual(log.injectedCount(from: at(t0), to: at(t0 + 1), keyboard: true, mouse: true), 0)
    }

    func testRepeatsAndAJigglerNeverCountAsTheUsersInput() {
        // A key held for two seconds: one press, then flagged repeats.
        var events = [TapEvent(type: Map.keyDown, key: 0x31, ms: 0)]
        events += (0..<30).map { TapEvent(type: Map.keyDown, key: 0x31, autorepeat: true, ms: 500 + 50 * Int64($0)) }
        events.append(TapEvent(type: Map.keyUp, key: 0x31, ms: 2_100))
        // A software jiggler moving the pointer every second for a minute.
        events += (0..<60).map { TapEvent(type: Map.mouseMoved, injected: true, ms: 3_000 + 1_000 * Int64($0)) }
        let log = feed(events)

        XCTAssertEqual(log.counts(from: at(t0), to: at(t0 + 120)), InputCounts(keyPresses: 1, clicks: 0))
        XCTAssertEqual(log.activeSeconds(from: at(t0), to: at(t0 + 120), keyboard: true, mouse: true), [t0])
        XCTAssertEqual(log.injectedCount(from: at(t0), to: at(t0 + 120), keyboard: true, mouse: true), 60)
        // Every event, injected or not, shows the tap is alive.
        XCTAssertEqual(log.lastCaptureEventMs, t0Ms + 62_000)
    }

    func testClicksScrollingAndDragsAreActivity() {
        let log = feed([
            TapEvent(type: Map.leftMouseDown, ms: 0),
            TapEvent(type: Map.scrollWheel, ms: 61_000),
            TapEvent(type: Map.leftMouseDragged, ms: 125_000),
        ])
        XCTAssertEqual(log.counts(from: at(t0), to: at(t0 + 180)), InputCounts(keyPresses: 0, clicks: 1))
        XCTAssertEqual(log.activityPercent(from: at(t0), to: at(t0 + 180), keyboard: false, mouse: true), 100)
        XCTAssertEqual(log.activityPercent(from: at(t0), to: at(t0 + 180), keyboard: true, mouse: false), 0)
    }

    // MARK: PointerWatch

    func testThePointerMovedOnlyBetweenTwoKnownPositions() {
        var watch = PointerWatch()
        watch.observe(x: 10, y: 10, atMs: 1_000)
        XCTAssertNil(watch.movedAtMs) // first sighting
        watch.observe(x: 10, y: 10, atMs: 11_000)
        XCTAssertNil(watch.movedAtMs) // still
        watch.observe(x: nil, y: nil, atMs: 21_000)
        watch.observe(x: 50, y: 10, atMs: 31_000)
        XCTAssertNil(watch.movedAtMs, "unknown in between: no move proven")
        watch.observe(x: 50, y: 12, atMs: 41_000)
        XCTAssertEqual(watch.movedAtMs, 41_000)
    }

    func testOnlyAMoveAfterTheLastEventProvesTheTapMissedInput() {
        var watch = PointerWatch()
        watch.observe(x: 0, y: 0, atMs: 0)
        watch.observe(x: 5, y: 5, atMs: 30_000)
        // It moved at some point in (20 s, 30 s]: proof after an event at 19 s…
        XCTAssertTrue(watch.missedInput(lastCaptureEventMs: 19_000, checkIntervalMs: 10_000))
        // …but not after one at 25 s, which may have been that very move.
        XCTAssertFalse(watch.missedInput(lastCaptureEventMs: 25_000, checkIntervalMs: 10_000))
        XCTAssertFalse(PointerWatch().missedInput(lastCaptureEventMs: 0, checkIntervalMs: 10_000))

        watch.reset()
        XCTAssertEqual(watch, PointerWatch())
    }
}
