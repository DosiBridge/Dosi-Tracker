import Foundation

/// A rectangle in the global display space: points, origin at the top-left
/// corner of the main display, y growing downwards — the space of
/// `CGDisplayBounds` and of `kCGWindowBounds`.
public struct ScreenRect: Equatable, Sendable {
    public var x: Double
    public var y: Double
    public var width: Double
    public var height: Double

    public init(x: Double, y: Double, width: Double, height: Double) {
        self.x = x
        self.y = y
        self.width = width
        self.height = height
    }

    public static let zero = ScreenRect(x: 0, y: 0, width: 0, height: 0)

    /// The area; 0 for an empty or inverted rectangle.
    public var area: Double { max(width, 0) * max(height, 0) }

    /// The area this rectangle shares with `other`.
    public func intersectionArea(_ other: ScreenRect) -> Double {
        let left = max(x, other.x)
        let right = min(x + width, other.x + other.width)
        let top = max(y, other.y)
        let bottom = min(y + height, other.y + other.height)
        guard right > left, bottom > top else { return 0 }
        return (right - left) * (bottom - top)
    }
}

/// One on-screen window as `CGWindowListCopyWindowInfo` lists it (front to
/// back).
public struct WindowSnapshot: Equatable, Sendable {
    /// The owning process (`kCGWindowOwnerPID`).
    public var ownerPID: Int32
    /// The window layer (`kCGWindowLayer`): 0 for ordinary app windows;
    /// menus, the Dock, the menu bar and overlays sit higher.
    public var layer: Int
    /// Opacity (`kCGWindowAlpha`); 0 for an invisible window.
    public var alpha: Double
    /// Where it is (`kCGWindowBounds`).
    public var bounds: ScreenRect
    /// Its title (`kCGWindowName`). macOS reveals other apps' titles only to
    /// a process with Screen Recording access.
    public var name: String?

    public init(ownerPID: Int32, layer: Int, alpha: Double, bounds: ScreenRect, name: String?) {
        self.ownerPID = ownerPID
        self.layer = layer
        self.alpha = alpha
        self.bounds = bounds
        self.name = name
    }
}

/// Picking an app's working window out of the on-screen window list.
///
/// Thread-safe: stateless.
public enum WindowList {
    /// Windows narrower or lower than this (points) are helpers — invisible
    /// 1×1 windows, drag handles, tool tips — never the window someone works
    /// in.
    public static let minimumSide: Double = 32

    /// The front-most ordinary window of `pid` (the list runs front to back):
    /// on layer 0, visible and not a sliver — the app's main window.
    public static func frontWindow(of pid: Int32, in windows: [WindowSnapshot]) -> WindowSnapshot? {
        windows.first { $0.ownerPID == pid && isOrdinary($0) }
    }

    /// The title of the front-most ordinary window of `pid` that shows one;
    /// nil when none does (or the titles are withheld).
    public static func frontTitle(of pid: Int32, in windows: [WindowSnapshot]) -> String? {
        for window in windows where window.ownerPID == pid && isOrdinary(window) {
            if let name = window.name, !name.isEmpty {
                return name
            }
        }
        return nil
    }

    static func isOrdinary(_ window: WindowSnapshot) -> Bool {
        window.layer == 0 && window.alpha > 0
            && window.bounds.width >= minimumSide && window.bounds.height >= minimumSide
    }
}

/// A display: its id (`CGDirectDisplayID`) and its bounds in the global
/// display space (`CGDisplayBounds`).
public struct DisplayFrame: Equatable, Sendable {
    public var id: UInt32
    public var bounds: ScreenRect

    public init(id: UInt32, bounds: ScreenRect) {
        self.id = id
        self.bounds = bounds
    }
}

/// Which display a screen capture shows (CONTRACT §4: "Capture the monitor
/// that holds the foreground window (fallback: primary)"; the Windows
/// agent's `pick_monitor`).
///
/// Thread-safe: stateless.
public enum DisplayTargeting {
    /// The display holding the largest part of the front-most window of
    /// `frontmostPID`; when that cannot be told (no app in front, no window,
    /// a window off every display), the main display if it is listed, else
    /// the first one. With no display listed at all, the main display's id.
    public static func displayToCapture(frontmostPID: Int32?, windows: [WindowSnapshot], displays: [DisplayFrame],
                                        mainDisplayID: UInt32) -> UInt32 {
        if let pid = frontmostPID,
           let window = WindowList.frontWindow(of: pid, in: windows),
           let holder = displayHolding(window.bounds, among: displays) {
            return holder
        }
        if displays.isEmpty || displays.contains(where: { $0.id == mainDisplayID }) {
            return mainDisplayID
        }
        return displays[0].id
    }

    /// The display sharing the largest area with `bounds` (the first one on a
    /// tie); nil when none overlaps it.
    public static func displayHolding(_ bounds: ScreenRect, among displays: [DisplayFrame]) -> UInt32? {
        var best: (id: UInt32, area: Double)?
        for display in displays {
            let area = display.bounds.intersectionArea(bounds)
            guard area > 0 else { continue }
            if let current = best, area <= current.area {
                continue
            }
            best = (display.id, area)
        }
        return best?.id
    }
}
