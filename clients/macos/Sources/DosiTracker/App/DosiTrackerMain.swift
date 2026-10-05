import AppKit

// The entry point: a plain, synchronous AppKit start. The app is a menu-bar
// agent (an "accessory": no Dock icon, no main menu bar), so the activation
// policy is set before the run loop starts — also for a `swift run` binary,
// which has no Info.plist `LSUIElement`. Nothing here awaits anything: the
// main thread goes straight into `run()`, so main-actor work, NSWorkspace
// notifications and timers all flow (the old prototype blocked the main actor
// from async top-level code and never captured anything).

@main
enum DosiTrackerMain {
    @MainActor
    static func main() {
        let app = NSApplication.shared
        let delegate = AppDelegate()
        app.delegate = delegate
        app.setActivationPolicy(.accessory)
        // `delegate` is held only weakly by the application.
        withExtendedLifetime(delegate) {
            app.run()
        }
    }
}
