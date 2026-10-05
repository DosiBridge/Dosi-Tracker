import AppKit
import DosiTrackerCore
import Foundation
import SystemConfiguration

// The app's composition root: the folders, preferences and configuration, the
// device facts, and the capture services (the core's input and usage logs fed
// by the event tap and the front-app sampler; screen, webcam, running apps,
// the idle clock and the system events). Created once at launch; the worker
// receives them as its dependencies.

@MainActor
final class AgentServices {
    let paths: AppPaths
    let preferences: Preferences
    let configSource: ConfigSource
    let device: DeviceInfo
    let credentials: KeychainCredentialsStore
    let projectCache: ProjectCacheStore

    let input: InputLog
    let usage: UsageRecorder
    let inputMonitor: InputMonitor
    let activeApp: ActiveAppMonitor
    let screen: ScreenCapturer
    let webcam: WebcamCapturer
    let running: RunningApps
    let idle: IdleMonitor
    let systemEvents: SystemEvents

    init(paths: AppPaths) {
        self.paths = paths
        preferences = Preferences()
        configSource = ConfigSource(paths: paths)
        device = DeviceInfo.current(deviceId: preferences.deviceId(), deviceName: AgentServices.computerName())
        credentials = KeychainCredentialsStore()
        projectCache = ProjectCacheStore()

        let inputLog = InputLog()
        let usageRecorder = UsageRecorder()
        let frontApp = ActiveAppMonitor(usage: usageRecorder)
        input = inputLog
        usage = usageRecorder
        inputMonitor = InputMonitor(log: inputLog)
        activeApp = frontApp
        // The display to capture is the one holding the app in front.
        screen = ScreenCapturer(frontmostProcess: { [weak frontApp] in
            frontApp?.frontmostApp?.processID
        })
        webcam = WebcamCapturer()
        running = RunningApps()
        idle = IdleMonitor()
        systemEvents = SystemEvents()
    }

    /// Start listening. The event tap does not ask for Input Monitoring by
    /// itself: the app asks once a project needs keyboard or mouse activity.
    func start() {
        activeApp.readsWindowTitles = false
        activeApp.start()
        inputMonitor.start(requestingPermission: false)
        systemEvents.start()
    }

    func stop() {
        systemEvents.stop()
        inputMonitor.stop()
        activeApp.stop()
    }

    /// The worker's dependencies; `publish` receives every snapshot.
    func workerDependencies(publish: @escaping @Sendable (TrackerSnapshot) -> Void) -> TrackerWorker.Dependencies {
        let frontApp = activeApp
        return TrackerWorker.Dependencies(
            paths: paths,
            configSource: configSource,
            credentials: credentials,
            preferences: preferences,
            projectCache: projectCache,
            device: device,
            input: input,
            usage: usage,
            screen: screen,
            webcam: webcam,
            foreground: frontApp,
            running: running,
            systemIdle: idle,
            hardwareIdle: IdleMonitor(stateID: .hidSystemState),
            setReadsWindowTitles: { reads in
                frontApp.readsWindowTitles = reads
            },
            publish: publish
        )
    }

    /// The computer's name as System Settings → General → Sharing shows it
    /// ("Ana's MacBook Pro"); nil when it is not set.
    static func computerName() -> String? {
        guard let name = SCDynamicStoreCopyComputerName(nil, nil) else { return nil }
        let text = (name as String).trimmingCharacters(in: .whitespacesAndNewlines)
        return text.isEmpty ? nil : text
    }
}
