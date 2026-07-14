import Foundation
import AppKit
import ApplicationServices

/// Reads the frontmost app + focused window title, and a list of running apps.
///
/// Window titles come from the **Accessibility** API and require the
/// Accessibility permission.
enum ActiveWindowMonitor {
    static func current() -> WindowInfo? {
        guard let app = NSWorkspace.shared.frontmostApplication else { return nil }
        let appName = app.localizedName ?? "Unknown"

        var windowTitle = ""
        let axApp = AXUIElementCreateApplication(app.processIdentifier)
        var focused: AnyObject?
        if AXUIElementCopyAttributeValue(axApp, kAXFocusedWindowAttribute as CFString, &focused) == .success,
           let window = focused {
            var titleRef: AnyObject?
            if AXUIElementCopyAttributeValue(window as! AXUIElement,
                                             kAXTitleAttribute as CFString, &titleRef) == .success,
               let title = titleRef as? String {
                windowTitle = title
            }
        }

        return WindowInfo(appName: appName, windowTitle: windowTitle)
    }

    /// Running, regular (non-background) applications as a "running programs" proxy.
    static func runningPrograms() -> [WindowInfo] {
        NSWorkspace.shared.runningApplications
            .filter { $0.activationPolicy == .regular }
            .map { WindowInfo(appName: $0.localizedName ?? "Unknown", windowTitle: "") }
    }
}
