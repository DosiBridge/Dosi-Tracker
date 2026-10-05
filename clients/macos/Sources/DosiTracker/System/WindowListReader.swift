import CoreGraphics
import DosiTrackerCore
import Foundation

/// Reads the on-screen window list (`CGWindowListCopyWindowInfo`) into
/// ``WindowSnapshot``s for ``DisplayTargeting`` and ``WindowList``. Owners,
/// layers and bounds need no permission; other apps' window titles appear
/// only with Screen Recording access.
///
/// Thread-safe: stateless; the window server answers from any thread.
enum WindowListReader {
    /// The on-screen windows, front to back, without desktop elements.
    static func onScreenWindows() -> [WindowSnapshot] {
        let options: CGWindowListOption = [.optionOnScreenOnly, .excludeDesktopElements]
        // Window id 0 (kCGNullWindowID): every window, not those around one.
        guard let list = CGWindowListCopyWindowInfo(options, CGWindowID(0)) as? [[String: Any]] else { return [] }
        return list.compactMap { snapshot($0) }
    }

    /// One window-list entry; nil without an owner.
    static func snapshot(_ info: [String: Any]) -> WindowSnapshot? {
        guard let owner = info[kCGWindowOwnerPID as String] as? NSNumber else { return nil }
        var bounds = ScreenRect.zero
        if let dictionary = info[kCGWindowBounds as String] as? NSDictionary,
           let rect = CGRect(dictionaryRepresentation: dictionary as CFDictionary) {
            bounds = ScreenRect(rect)
        }
        return WindowSnapshot(
            ownerPID: owner.int32Value,
            layer: (info[kCGWindowLayer as String] as? NSNumber)?.intValue ?? 0,
            alpha: (info[kCGWindowAlpha as String] as? NSNumber)?.doubleValue ?? 1,
            bounds: bounds,
            name: info[kCGWindowName as String] as? String
        )
    }
}

extension ScreenRect {
    /// A Core Graphics rectangle in the global display space.
    init(_ rect: CGRect) {
        self.init(x: Double(rect.origin.x), y: Double(rect.origin.y),
                  width: Double(rect.size.width), height: Double(rect.size.height))
    }
}
