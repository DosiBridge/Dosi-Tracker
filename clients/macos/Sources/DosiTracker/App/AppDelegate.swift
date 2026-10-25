import AppKit
import DosiTrackerCore
import Foundation

// The application's lifecycle (port of the Windows agent's `main.rs`):
//
// * at launch: the folders and the log file, one copy only (a second launch
//   shows the running copy's window and exits), the main menu, the capture
//   services, the worker, the menu-bar item and the windows;
// * while running: the lock / sleep / power events go to the worker — sleep
//   and power-off wait up to two seconds for the block to be saved locally —
//   and a second launch or a click on a notification shows the window;
// * at quit (the menu, ⌘Q, SIGTERM / SIGINT, logging out, restarting or
//   shutting down): `applicationShouldTerminate` answers `.terminateLater`,
//   the worker saves the block in progress locally first, then — unless the
//   session is ending — says "stopped" and uploads what fits in 20 s, and only
//   then is the app allowed to end.

@MainActor
final class AppDelegate: NSObject, NSApplicationDelegate {
    private var launchedAtLogin = false
    private var instanceLock: SingleInstance.Lock?
    private var fileLog: FileLog?
    private var services: AgentServices?
    private var worker: TrackerWorker?
    private var model: AppModel?
    private var windows: WindowManager?
    private var statusItem: StatusItemController?
    private let notifier = Notifier()
    private var signalSources: [DispatchSourceSignal] = []
    private var showObserver: NSObjectProtocol?
    private let powerOff = PowerOffFlag()
    private var terminating = false

    func applicationWillFinishLaunching(_ notification: Notification) {
        // Only readable while the launch event is being handled.
        launchedAtLogin = LaunchContext.launchedAsLoginItem()
    }

    func applicationDidFinishLaunching(_ notification: Notification) {
        let paths = AppDelegate.resolvePaths()
        startLogging(paths)

        // Two copies would each capture and upload: the server would refuse
        // one copy's overlapping blocks, and take twice the screenshots.
        switch SingleInstance.acquire(lockFile: paths.lockFile) {
        case .alreadyRunning:
            CoreLog.info("app", "another copy is running; showing it and exiting")
            SingleInstance.showRunningCopy()
            fileLog?.flush()
            exit(0)
        case .acquired(let lock):
            instanceLock = lock
        }
        CoreLog.info("app", "starting \(AgentInfo.displayName) \(AgentInfo.version()) on \(DeviceInfo.currentOSVersion())"
                     + (launchedAtLogin ? ", opened at login" : ""))
        MainMenu.install(settings: #selector(showSettings(_:)), target: self)

        let services = AgentServices(paths: paths)
        self.services = services
        let relay = SnapshotRelay()
        let worker = TrackerWorker(dependencies: services.workerDependencies(publish: { snapshot in
            Task { @MainActor in
                relay.deliver(snapshot)
            }
        }))
        self.worker = worker
        let model = AppModel(worker: worker, services: services, notifier: notifier)
        relay.model = model
        self.model = model
        let windows = WindowManager(model: model)
        self.windows = windows
        model.onSnapshotChange = { [weak windows] old, new in
            windows?.snapshotChanged(from: old, to: new)
        }
        statusItem = StatusItemController(model: model, windows: windows)
        services.inputMonitor.setStateHandler { state in
            Task { @MainActor in
                relay.model?.setInputCapture(state)
            }
        }
        notifier.start { [weak windows] _ in
            windows?.showPrimary()
        }

        services.start()
        model.startRefreshing()
        subscribeToSystemEvents(services.systemEvents, worker: worker)
        observeShowRequests()
        handleTerminationSignals()

        let atLogin = launchedAtLogin
        Task {
            await worker.start(launchedAtLogin: atLogin)
        }
    }

    /// The Dock icon is hidden, but Finder or Spotlight can open the app
    /// again: show its window.
    func applicationShouldHandleReopen(_ sender: NSApplication, hasVisibleWindows flag: Bool) -> Bool {
        windows?.showPrimary()
        return false
    }

    func applicationSupportsSecureRestorableState(_ app: NSApplication) -> Bool {
        true
    }

    /// Save the block in progress before the app ends: locally first (and
    /// only that when the session is ending), then upload what fits.
    func applicationShouldTerminate(_ sender: NSApplication) -> NSApplication.TerminateReply {
        guard let worker = worker else { return .terminateNow }
        if terminating {
            return .terminateLater
        }
        terminating = true
        let moment = Date()
        let sessionEnding = LaunchContext.isSystemQuit() || powerOff.isRecent(at: moment)
        statusItem?.showQuitting()
        CoreLog.info("app", sessionEnding
                     ? "the session is ending; saving the block in progress"
                     : "quitting; saving the block in progress and uploading what fits")
        let budget = sessionEnding ? AgentTiming.shutdownSaveBudget : AgentTiming.quitSyncBudget + 5
        Task { @MainActor in
            do {
                try await ApiClient.withDeadline(budget) {
                    await worker.quit(at: moment, systemShutdown: sessionEnding)
                }
            } catch {
                CoreLog.warning("app", "quitting before everything was uploaded; the rest goes up on the next launch")
            }
            self.fileLog?.flush()
            NSApp.reply(toApplicationShouldTerminate: true)
        }
        return .terminateLater
    }

    func applicationWillTerminate(_ notification: Notification) {
        services?.stop()
        if let observer = showObserver {
            DistributedNotificationCenter.default().removeObserver(observer)
        }
        CoreLog.info("app", "stopped")
        fileLog?.flush()
    }

    /// ⌘, (the hidden main menu's "Settings…").
    @objc func showSettings(_ sender: Any?) {
        windows?.showMain(tab: .settings)
    }

    // MARK: Setup

    /// `~/Library/Application Support/DosiTracker` and `~/Library/Logs/DosiTracker`
    /// — never the working directory, which is `/` for an app started at login.
    private static func resolvePaths() -> AppPaths {
        let paths: AppPaths
        do {
            paths = try AppPaths.standard()
        } catch {
            let library = URL(fileURLWithPath: NSHomeDirectory(), isDirectory: true)
                .appendingPathComponent("Library", isDirectory: true)
            paths = AppPaths.rooted(at: library)
        }
        do {
            try paths.createDirectories()
        } catch {
            CoreLog.error("app", "could not create the app's folders: \(error)")
        }
        return paths
    }

    private func startLogging(_ paths: AppPaths) {
        let log = FileLog(url: paths.logFile, writesDebug: UserDefaults.standard.bool(forKey: "DosiLogDebug"))
        fileLog = log
        CoreLog.setSink { level, category, message in
            log.write(level, category, message)
        }
    }

    /// Lock, sleep and power events. Before sleep and power-off the handler
    /// waits (at most two seconds) for the worker to save the block locally;
    /// the rest are handed over without waiting.
    private func subscribeToSystemEvents(_ events: SystemEvents, worker: TrackerWorker) {
        let powerOff = self.powerOff
        events.subscribe { event in
            if event.kind == .willPowerOff {
                powerOff.set(at: event.at)
            }
            guard event.kind == .willSleep || event.kind == .willPowerOff else {
                Task {
                    await worker.handleSystemEvent(event)
                }
                return
            }
            let saved = DispatchSemaphore(value: 0)
            Task.detached {
                await worker.handleSystemEvent(event)
                saved.signal()
            }
            if saved.wait(timeout: .now() + AgentTiming.shutdownSaveBudget) == .timedOut {
                CoreLog.warning("app", "the block was not saved within \(Int(AgentTiming.shutdownSaveBudget)) s of \(event.kind.rawValue)")
            }
        }
    }

    /// A second launch asks this copy to show its window.
    private func observeShowRequests() {
        showObserver = DistributedNotificationCenter.default().addObserver(
            forName: SingleInstance.showWindowNotification, object: nil, queue: .main
        ) { [weak self] _ in
            // The observer's block is @Sendable: the hop re-captures `self`
            // weakly instead of reaching into the outer closure's capture.
            Task { @MainActor [weak self] in
                self?.windows?.showPrimary()
            }
        }
    }

    /// SIGTERM (launchd, `kill`) and SIGINT (Ctrl-C in a terminal) quit like
    /// the menu's Quit: the block in progress is saved first.
    private func handleTerminationSignals() {
        for signalNumber in [SIGTERM, SIGINT] {
            signal(signalNumber, SIG_IGN)
            let source = DispatchSource.makeSignalSource(signal: signalNumber, queue: .main)
            source.setEventHandler {
                Task { @MainActor in
                    CoreLog.info("app", "asked to stop by signal \(signalNumber); quitting")
                    NSApp.terminate(nil)
                }
            }
            source.resume()
            signalSources.append(source)
        }
    }
}

/// Hands the worker's snapshots to the model on the main actor (the model is
/// created after the worker).
@MainActor
final class SnapshotRelay {
    weak var model: AppModel?

    func deliver(_ snapshot: TrackerSnapshot) {
        model?.apply(snapshot)
    }
}

/// When the system last announced a log out, restart or shutdown (set from
/// the system-events thread, read when the app is asked to quit). It counts
/// only for a short while: another app can cancel the log out, and a Quit
/// chosen hours later is an ordinary quit (end reason `quit`, a "stopped"
/// heartbeat and a last upload).
final class PowerOffFlag: @unchecked Sendable {
    /// How long after the announcement a quit is taken as the session ending.
    static let validFor: TimeInterval = 60

    private let lock = NSLock()
    private var announcedAt: Date?

    /// Whether a session end was announced shortly before `now`.
    func isRecent(at now: Date) -> Bool {
        lock.lock()
        defer { lock.unlock() }
        guard let announced = announcedAt else { return false }
        let age = now.timeIntervalSince(announced)
        return age >= -PowerOffFlag.validFor && age <= PowerOffFlag.validFor
    }

    func set(at moment: Date = Date()) {
        lock.lock()
        announcedAt = moment
        lock.unlock()
    }
}
