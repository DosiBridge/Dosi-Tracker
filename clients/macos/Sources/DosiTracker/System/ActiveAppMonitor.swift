import AppKit
import ApplicationServices
import CoreGraphics
import DosiTrackerCore
import Foundation

/// The app and window in front, and the sampler that turns them into
/// per-app usage (port of the Windows agent's `active_window::current` and
/// `usage::spawn_sampler`).
///
/// * The app in front comes from NSWorkspace (`frontmostApplication`), kept
///   current by `didActivateApplicationNotification`; its name is the app's
///   localized name (CONTRACT §8).
/// * The window title is the focused window's, read through Accessibility
///   when the app is trusted; otherwise the title of the app's front window
///   from the window list, which other apps only reveal with Screen
///   Recording access; otherwise empty.
/// * Every ``UsageRecorder/sampleInterval`` (3 s) a sampler on a utility queue
///   credits the time since the previous sample to the window in front
///   (``UsageRecorder``). Like the Windows sampler it runs whatever the
///   tracking state: samples outside a block are never attributed to one.
///
/// Turn ``readsWindowTitles`` off when the project does not record the active
/// window, so no title is even read.
///
/// Thread-safe: the front app is cached behind a lock, Accessibility and the
/// window list are only queried on the sampler queue (an Accessibility call
/// into a busy app can wait up to ``accessibilityTimeout``), and only the
/// NSWorkspace notification touches the main thread, briefly.
public final class ActiveAppMonitor: ForegroundWindowSource, @unchecked Sendable {
    /// The app in front, as NSWorkspace last reported it.
    public struct FrontApp: Equatable, Sendable {
        public let processID: pid_t
        /// Its localized name, e.g. `Google Chrome`.
        public let name: String
        public let bundleIdentifier: String?

        public init(processID: pid_t, name: String, bundleIdentifier: String?) {
            self.processID = processID
            self.name = name
            self.bundleIdentifier = bundleIdentifier
        }
    }

    /// How long an Accessibility call into an unresponsive app may take (the
    /// system's default is 6 s).
    public static let accessibilityTimeout: Float = 1

    private let usage: UsageRecorder
    private let queue = DispatchQueue(label: "com.dosibridge.tracker.app-sampler", qos: .utility,
                                      autoreleaseFrequency: .workItem)
    private let lock = NSLock()
    private var front: FrontApp?
    private var titles = true
    private var observer: NSObjectProtocol?
    private var timer: DispatchSourceTimer?

    public init(usage: UsageRecorder) {
        self.usage = usage
    }

    deinit {
        stop()
    }

    /// Start following the app in front and sampling it; calling it again
    /// does nothing.
    public func start() {
        lock.lock()
        guard timer == nil else {
            lock.unlock()
            return
        }
        observer = NSWorkspace.shared.notificationCenter.addObserver(
            forName: NSWorkspace.didActivateApplicationNotification, object: nil, queue: nil
        ) { [weak self] notification in
            // Posted on the main thread.
            let app = notification.userInfo?[NSWorkspace.applicationUserInfoKey] as? NSRunningApplication
            self?.update(app ?? NSWorkspace.shared.frontmostApplication)
        }
        let sampler = DispatchSource.makeTimerSource(queue: queue)
        let interval = DispatchTimeInterval.milliseconds(Int(UsageRecorder.sampleInterval * 1000))
        // The first sample only starts the clock (nothing to credit yet).
        sampler.schedule(deadline: .now(), repeating: interval, leeway: .milliseconds(250))
        sampler.setEventHandler { [weak self] in
            self?.sample()
        }
        timer = sampler
        lock.unlock()

        refreshFrontmost()
        sampler.resume()
    }

    /// Stop following and sampling.
    public func stop() {
        lock.lock()
        let sampler = timer
        let token = observer
        timer = nil
        observer = nil
        lock.unlock()
        sampler?.cancel()
        if let token = token {
            NSWorkspace.shared.notificationCenter.removeObserver(token)
        }
    }

    /// The app in front (cheap: no window is read).
    public var frontmostApp: FrontApp? {
        lock.lock()
        defer { lock.unlock() }
        return front
    }

    public var frontmostAppName: String? { frontmostApp?.name }

    /// Whether window titles are read (default on). Off when the project
    /// does not record the active window: samples then carry the app only.
    public var readsWindowTitles: Bool {
        get {
            lock.lock()
            defer { lock.unlock() }
            return titles
        }
        set {
            lock.lock()
            titles = newValue
            lock.unlock()
        }
    }

    /// The app and window in front now, read on the sampler queue.
    public func currentWindow() async -> ForegroundWindow? {
        await withCheckedContinuation { continuation in
            queue.async { [weak self] in
                continuation.resume(returning: self?.readWindow())
            }
        }
    }

    // MARK: Sampling (sampler queue)

    private func sample() {
        let now = Date()
        usage.record(at: now, window: readWindow())
    }

    /// The window in front; nil when no app is (or the one last reported has
    /// quit).
    private func readWindow() -> ForegroundWindow? {
        lock.lock()
        let app = front
        let readTitles = titles
        lock.unlock()
        guard let app = app, NSRunningApplication(processIdentifier: app.processID) != nil else { return nil }
        let title = readTitles ? WindowTitleReader.title(of: app.processID) : ""
        return ForegroundWindow.sampled(appName: app.name, windowTitle: title)
    }

    // MARK: The app in front

    private func refreshFrontmost() {
        if Thread.isMainThread {
            update(NSWorkspace.shared.frontmostApplication)
        } else {
            DispatchQueue.main.async { [weak self] in
                self?.update(NSWorkspace.shared.frontmostApplication)
            }
        }
    }

    private func update(_ app: NSRunningApplication?) {
        let snapshot = app.flatMap { ActiveAppMonitor.frontApp(of: $0) }
        lock.lock()
        front = snapshot
        lock.unlock()
    }

    static func frontApp(of app: NSRunningApplication) -> FrontApp? {
        guard !app.isTerminated,
              let name = AppNaming.appName(localizedName: app.localizedName,
                                           bundleFileName: app.bundleURL?.lastPathComponent,
                                           executableName: app.executableURL?.lastPathComponent)
        else {
            return nil
        }
        return FrontApp(processID: app.processIdentifier, name: name, bundleIdentifier: app.bundleIdentifier)
    }
}

/// Reads the title of an app's working window: the focused window's through
/// Accessibility when this app is trusted, else the front window's from the
/// window list (other apps' titles only with Screen Recording access), else
/// nothing.
///
/// Thread-safe: stateless. Call it off the main thread: an Accessibility call
/// into a busy app waits up to ``ActiveAppMonitor/accessibilityTimeout``.
enum WindowTitleReader {
    static func title(of pid: pid_t) -> String {
        // This app's own windows: from the window list, which always shows
        // them, rather than an Accessibility call into ourselves.
        let own = pid == ProcessInfo.processInfo.processIdentifier
        if !own, AXIsProcessTrusted(), let title = focusedWindowTitle(of: pid) {
            return title
        }
        if own || CGPreflightScreenCaptureAccess() {
            return WindowList.frontTitle(of: pid, in: WindowListReader.onScreenWindows()) ?? ""
        }
        return ""
    }

    /// The title of `pid`'s focused window (`kAXFocusedWindowAttribute`,
    /// `kAXTitleAttribute`); nil when the app does not say.
    static func focusedWindowTitle(of pid: pid_t) -> String? {
        let application = AXUIElementCreateApplication(pid)
        _ = AXUIElementSetMessagingTimeout(application, ActiveAppMonitor.accessibilityTimeout)
        var focused: CFTypeRef?
        guard AXUIElementCopyAttributeValue(application, kAXFocusedWindowAttribute as CFString, &focused) == .success,
              let window = focused, CFGetTypeID(window) == AXUIElementGetTypeID()
        else {
            return nil
        }
        // A timeout set on one element applies to that element only: the
        // window needs its own, or the title read waits the global ~6 s.
        let windowElement = window as! AXUIElement
        _ = AXUIElementSetMessagingTimeout(windowElement, ActiveAppMonitor.accessibilityTimeout)
        var title: CFTypeRef?
        guard AXUIElementCopyAttributeValue(windowElement, kAXTitleAttribute as CFString, &title) == .success
        else {
            return nil
        }
        return title as? String
    }
}
