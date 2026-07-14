import Foundation
import CoreGraphics

/// Event-driven keyboard/mouse counter using a CGEventTap.
///
/// The tap is installed on a background run loop; macOS pushes events to us,
/// so idle CPU stays near zero. We only count events — never key contents.
///
/// Requires the **Accessibility** permission
/// (System Settings → Privacy & Security → Accessibility).
final class InputMonitor {
    private let lock = NSLock()
    private var keyboardHits = 0
    private var mouseClicks = 0
    private var eventTap: CFMachPort?

    func start() {
        let mask: CGEventMask =
            (1 << CGEventType.keyDown.rawValue) |
            (1 << CGEventType.leftMouseDown.rawValue) |
            (1 << CGEventType.rightMouseDown.rawValue) |
            (1 << CGEventType.otherMouseDown.rawValue)

        let callback: CGEventTapCallBack = { _, type, _, refcon in
            guard let refcon = refcon else { return nil }
            let monitor = Unmanaged<InputMonitor>.fromOpaque(refcon).takeUnretainedValue()
            switch type {
            case .keyDown:
                monitor.bump(keyboard: true)
            case .leftMouseDown, .rightMouseDown, .otherMouseDown:
                monitor.bump(keyboard: false)
            default:
                break
            }
            return nil // listen-only; do not swallow events
        }

        let selfPtr = Unmanaged.passUnretained(self).toOpaque()
        guard let tap = CGEvent.tapCreate(
            tap: .cgSessionEventTap,
            place: .headInsertEventTap,
            options: .listenOnly,
            eventsOfInterest: mask,
            callback: callback,
            userInfo: selfPtr
        ) else {
            FileHandle.standardError.write(Data("[warn] failed to create event tap; grant Accessibility permission\n".utf8))
            return
        }
        eventTap = tap

        let runLoopSource = CFMachPortCreateRunLoopSource(kCFAllocatorDefault, tap, 0)
        Thread.detachNewThread {
            let loop = CFRunLoopGetCurrent()
            CFRunLoopAddSource(loop, runLoopSource, .commonModes)
            CGEvent.tapEnable(tap: tap, enable: true)
            CFRunLoopRun()
        }
    }

    private func bump(keyboard: Bool) {
        lock.lock(); defer { lock.unlock() }
        if keyboard { keyboardHits += 1 } else { mouseClicks += 1 }
    }

    /// Return current counts and reset. Returns (keyboardHits, mouseClicks).
    func take() -> (Int, Int) {
        lock.lock(); defer { lock.unlock() }
        let k = keyboardHits, m = mouseClicks
        keyboardHits = 0; mouseClicks = 0
        return (k, m)
    }
}
