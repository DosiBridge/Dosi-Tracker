import AppKit
import DosiTrackerCore
import Foundation

// The UI's model on the main actor: the latest worker snapshot as published
// state for SwiftUI and the menu bar, the user's commands (each carrying the
// moment the user acted, CONTRACT §4), the privacy permissions and the login
// item, and the live numbers of the block in progress read straight from the
// input and usage logs.

/// A pause or resume the user asked for that the worker has not applied yet;
/// the UI reflects it at once, so the buttons never feel unresponsive.
enum PendingAction: Equatable {
    case pausing
    case resuming
}

/// The main window's tabs.
enum MainTab: String, CaseIterable, Identifiable {
    case timer
    case activity
    case settings

    var id: String { rawValue }

    var title: String {
        switch self {
        case .timer:
            return "Timer"
        case .activity:
            return "Activity"
        case .settings:
            return "Settings"
        }
    }

    var symbol: String {
        switch self {
        case .timer:
            return "timer"
        case .activity:
            return "chart.bar.xaxis"
        case .settings:
            return "gearshape"
        }
    }
}

/// One minute of the block in progress, for the bars.
struct MinuteBar: Equatable, Identifiable {
    let id: Int
    let active: Bool
    /// Key presses and clicks in that minute (the recorded kinds only).
    let inputs: Int
}

/// What the block in progress holds so far.
struct LiveBlock: Equatable {
    var keyPresses: Int
    var clicks: Int
    /// Nil when activity is not measured on this project.
    var activityPercent: Int?
    var minutes: [MinuteBar]
}

@MainActor
final class AppModel: ObservableObject {
    /// The timed breaks offered next to the pause button and in the menu.
    static let breakChoices: [(label: String, seconds: TimeInterval)] = [
        ("15 min", 15 * 60),
        ("30 min", 30 * 60),
        ("1 hour", 60 * 60),
    ]
    /// How often the permissions and the login item are read again.
    static let refreshInterval: TimeInterval = 5
    /// How far back "Top apps" looks.
    static let topAppsWindow: TimeInterval = 60 * 60

    @Published private(set) var snapshot = TrackerSnapshot()
    @Published private(set) var pendingAction: PendingAction?
    @Published private(set) var permissions: PrivacyStatus
    @Published private(set) var inputCapture: InputCaptureState = .stopped
    @Published private(set) var loginItem: LoginItemState
    @Published private(set) var goals: Goals
    @Published private(set) var localConsent: LocalConsent = .absent
    @Published var selectedTab: MainTab = .timer
    /// A short-lived message (e.g. "Could not open the browser").
    @Published private(set) var message: String?
    @Published private(set) var settingsError: String?

    let worker: TrackerWorker
    let device: DeviceInfo
    let paths: AppPaths
    let consentManager: ConsentManager
    private let configSource: ConfigSource
    private let preferences: Preferences
    private let input: InputLog
    private let usage: UsageRecorder
    private let frontApp: ActiveAppMonitor
    private let notifier: Notifier
    private var lastNoticeId = 0
    private var askedPermissions = Set<PrivacyPermission>()
    private var refreshTimer: Timer?
    private var messageTask: Task<Void, Never>?

    /// Called after every new snapshot (old, new): the windows react to it.
    var onSnapshotChange: ((TrackerSnapshot, TrackerSnapshot) -> Void)?

    init(worker: TrackerWorker, services: AgentServices, notifier: Notifier) {
        self.worker = worker
        device = services.device
        paths = services.paths
        consentManager = services.consentManager
        configSource = services.configSource
        preferences = services.preferences
        input = services.input
        usage = services.usage
        frontApp = services.activeApp
        self.notifier = notifier
        permissions = Permissions.status()
        loginItem = LoginItem.state
        goals = services.preferences.goals
        localConsent = services.consentManager.current
    }

    /// Read the permissions and the login item now and every few seconds (the
    /// user can change either in System Settings at any time).
    func startRefreshing() {
        refreshTimer?.invalidate()
        let timer = Timer(timeInterval: AppModel.refreshInterval, repeats: true) { [weak self] _ in
            // The timer is scheduled on the main run loop, so it fires on the
            // main thread: refresh in place instead of re-capturing the weak
            // `self` in a task (Swift 5.10 rejects that capture).
            MainActor.assumeIsolated {
                self?.refreshSystemState()
            }
        }
        RunLoop.main.add(timer, forMode: .common)
        refreshTimer = timer
        refreshSystemState()
    }

    func refreshSystemState() {
        let status = Permissions.status()
        if status != permissions {
            permissions = status
        }
        let state = LoginItem.state
        if state != loginItem {
            loginItem = state
        }
    }

    func setInputCapture(_ state: InputCaptureState) {
        if state != inputCapture {
            inputCapture = state
        }
    }

    // MARK: Snapshots

    func apply(_ next: TrackerSnapshot) {
        guard next.revision > snapshot.revision else { return }
        let old = snapshot
        snapshot = next
        localConsent = next.localConsent
        if pendingAction == .pausing && next.status.isPaused {
            pendingAction = nil
        } else if pendingAction == .resuming && !next.status.isPaused {
            pendingAction = nil
        }
        // The system prompts follow what will be recorded (the project's
        // policy AND the user's consent): never one for a capture the user
        // did not switch on.
        if next.effectiveCapture != old.effectiveCapture {
            requestMissingPermissions(for: next.effectiveCapture)
        }
        // Any signed-in state counts, waiting for consent included: the login
        // item must not depend on the user granting consent first.
        if !old.status.isSignedIn && next.status.isSignedIn {
            startAtLoginOnFirstSignIn()
        }
        for notice in next.notices where notice.id > lastNoticeId {
            lastNoticeId = notice.id
            notifier.post(id: "notice-\(notice.id)", title: AgentInfo.displayName, body: notice.text)
        }
        onSnapshotChange?(old, next)
    }

    // MARK: Consent

    func updateConsent(permissions: ConsentedPermissions) {
        let worker = self.worker
        Task {
            do {
                try await worker.updateConsent(.granted(permissions))
                settingsError = nil
            } catch {
                settingsError = "Could not save consent: \(error)"
            }
        }
    }

    /// The standard choice: keyboard and mouse counts, the app in front and
    /// blurred screenshots; never the webcam or the running programs (each
    /// is switched on on its own).
    func grantStandardConsent() {
        var standard = ConsentedPermissions.allDisabled
        standard.keyboard = true
        standard.mouse = true
        standard.activeWindow = true
        standard.screenshot = true
        standard.blurScreenshots = true
        updateConsent(permissions: standard)
    }

    func revokeConsent() {
        let worker = self.worker
        Task {
            do {
                try await worker.updateConsent(.revoked)
                settingsError = nil
            } catch {
                settingsError = "Could not save consent revocation: \(error)"
            }
        }
    }

    // MARK: Commands

    func signIn(email: String, password: String, server: String?, newPassword: String?) {
        let worker = self.worker
        Task {
            await worker.signIn(username: email, password: password, server: server, newPassword: newPassword)
        }
    }

    /// "Switch workspace…": list the user's workspaces to pick another.
    func switchWorkspace() {
        let worker = self.worker
        Task {
            await worker.switchWorkspace()
        }
    }

    /// Track in `workspaceId` (one of the snapshot's choices) from now on.
    func chooseWorkspace(_ workspaceId: String) {
        let at = Date()
        let worker = self.worker
        Task {
            await worker.chooseWorkspace(workspaceId, at: at)
        }
    }

    /// Close the workspace picker without switching.
    func cancelWorkspaceChoice() {
        let worker = self.worker
        Task {
            await worker.cancelWorkspaceChoice()
        }
    }

    func signOut() {
        let at = Date()
        let worker = self.worker
        Task {
            await worker.signOut(at: at)
        }
    }

    func selectProject(_ projectId: String) {
        guard snapshot.selectedProjectId?.caseInsensitiveCompare(projectId) != .orderedSame else { return }
        let at = Date()
        let worker = self.worker
        Task {
            await worker.selectProject(projectId, at: at)
        }
    }

    /// Pause (or, while paused, change when the break ends): `seconds` nil
    /// means until the user resumes. The click's time travels with the
    /// command: the block ends exactly there, however busy the worker is.
    func pause(for seconds: TimeInterval?) {
        let status = snapshot.status
        guard pendingAction == nil, status.isPaused || status.canPause else { return }
        if !status.isPaused {
            pendingAction = .pausing
        }
        let at = Date()
        let worker = self.worker
        Task {
            await worker.pause(resumeAfter: seconds, at: at)
            self.clearPending(.pausing)
        }
    }

    func resume() {
        guard snapshot.status.isPaused, pendingAction == nil else { return }
        pendingAction = .resuming
        let at = Date()
        let worker = self.worker
        Task {
            await worker.resume(at: at)
            self.clearPending(.resuming)
        }
    }

    /// The menu's one toggle: pause while tracking, resume while paused.
    func toggleTracking() {
        if snapshot.status.isPaused {
            resume()
        } else {
            pause(for: nil)
        }
    }

    func syncNow() {
        let at = Date()
        let worker = self.worker
        Task {
            await worker.syncNow(at: at)
        }
    }

    /// Send the task note when the user commits it (Enter, leaving the field),
    /// not on every keystroke: a change ends the block in progress.
    func setTaskNote(_ note: String) {
        guard WorkerRules.cleanNote(note) != WorkerRules.cleanNote(snapshot.taskNote) else { return }
        let at = Date()
        let worker = self.worker
        Task {
            await worker.setTaskNote(note, at: at)
        }
    }

    func resolveAway(keep: Bool) {
        let worker = self.worker
        Task {
            await worker.resolveAway(keep: keep)
        }
    }

    func reconnect() {
        let worker = self.worker
        Task {
            await worker.reconnect()
        }
    }

    private func clearPending(_ action: PendingAction) {
        if pendingAction == action {
            pendingAction = nil
        }
    }

    // MARK: Links and folders

    /// The web dashboard (`DosiDashboardURL`), in the browser. A build that
    /// names none never opens the development address.
    func openDashboard() {
        let config = configSource.load(savedServerURL: preferences.savedServerURL)
        guard config.dashboardURLIsConfigured else {
            show("The web dashboard address is not set on this Mac. Ask your workspace admin for it.")
            return
        }
        let url = config.dashboardURL
        if !NSWorkspace.shared.open(url) {
            show("Could not open \(url.absoluteString)")
        }
    }

    func openLogsFolder() {
        open(folder: paths.logs)
    }

    func openDataFolder() {
        open(folder: paths.applicationSupport)
    }

    private func open(folder: URL) {
        try? FileManager.default.createDirectory(at: folder, withIntermediateDirectories: true)
        if !NSWorkspace.shared.open(folder) {
            show("Could not open \(folder.path)")
        }
    }

    /// Show `text` for a few seconds.
    func show(_ text: String) {
        message = text
        messageTask?.cancel()
        messageTask = Task {
            try? await Task.sleep(nanoseconds: 6_000_000_000)
            if !Task.isCancelled {
                self.message = nil
            }
        }
    }

    // MARK: Settings

    var startsAtLogin: Bool {
        loginItem == .enabled || loginItem == .requiresApproval
    }

    func setStartAtLogin(_ enabled: Bool) {
        do {
            try LoginItem.setEnabled(enabled)
            settingsError = nil
        } catch {
            CoreLog.warning("app", "could not change the login item: \(error.localizedDescription)")
            settingsError = enabled
                ? "Dosi Tracker could not be added to your login items (\(error.localizedDescription))."
                : "Dosi Tracker could not be removed from your login items (\(error.localizedDescription))."
        }
        // Reflect what the system says, whether or not the change worked.
        loginItem = LoginItem.state
    }

    func openLoginItemSettings() {
        LoginItem.openSystemSettings()
    }

    /// Like the Windows installer, start at login by default: once, after the
    /// first sign-in on this Mac (macOS tells the user it added a login item).
    /// A user who turns it off later is never overridden.
    private func startAtLoginOnFirstSignIn() {
        let key = "LoginItemDefaulted"
        guard !UserDefaults.standard.bool(forKey: key), loginItem == .disabled else { return }
        UserDefaults.standard.set(true, forKey: key)
        setStartAtLogin(true)
        CoreLog.info("app", "start at login turned on after the first sign-in")
    }

    func setDailyGoal(hours: Double) {
        var updated = goals
        updated.dailyHours = hours
        preferences.goals = updated
        goals = preferences.goals
    }

    func setWorkDays(_ days: Int) {
        var updated = goals
        updated.workDaysPerWeek = days
        preferences.goals = updated
        goals = preferences.goals
    }

    // MARK: Permissions

    /// The permissions what is recorded needs (the project's policy AND the
    /// user's consent) but lacks.
    var missingPermissions: [PrivacyPermission] {
        permissions.missing(for: snapshot.effectiveCapture.permissions)
    }

    /// Keyboard and mouse should be measured right now, but the event tap
    /// cannot (no Input Monitoring, or it failed to start). Not while paused
    /// or away, when the tap is stopped on purpose.
    var inputCaptureBlocked: Bool {
        snapshot.status == .tracking && snapshot.effectiveCapture.measuresInput
            && (inputCapture == .waitingForPermission || inputCapture == .failed)
    }

    /// Fix a missing permission: the system prompt when it was never asked
    /// for, otherwise its pane in System Settings.
    func fix(_ permission: PrivacyPermission) {
        let state = permissions.state(of: permission)
        switch permission {
        case .camera where state == .notDetermined:
            Task {
                _ = await Permissions.requestCamera()
                self.refreshSystemState()
            }
            return
        case .inputMonitoring where state == .notDetermined:
            Permissions.requestInputMonitoring()
        case .screenRecording where !askedPermissions.contains(.screenRecording):
            askedPermissions.insert(.screenRecording)
            if !Permissions.requestScreenRecording() {
                Permissions.openSettings(for: permission)
            }
        default:
            Permissions.openSettings(for: permission)
        }
        refreshSystemState()
    }

    /// Ask for what newly recorded captures need: each system prompt at
    /// most once per launch (macOS itself shows each only once), Accessibility
    /// only once ever and only when Screen Recording cannot reveal window
    /// titles instead.
    private func requestMissingPermissions(for flags: CaptureFlags) {
        let status = Permissions.status()
        if flags.screenshot && !status.screenRecording.isGranted
            && askedPermissions.insert(.screenRecording).inserted {
            Permissions.requestScreenRecording()
        }
        if flags.measuresInput && status.inputMonitoring == .notDetermined
            && askedPermissions.insert(.inputMonitoring).inserted {
            Permissions.requestInputMonitoring()
        }
        if flags.webcam && status.camera == .notDetermined && askedPermissions.insert(.camera).inserted {
            Task {
                _ = await Permissions.requestCamera()
                self.refreshSystemState()
            }
        }
        let askedAccessibilityKey = "AskedAccessibility"
        if flags.activeWindow && !flags.screenshot && !status.canReadWindowTitles
            && !UserDefaults.standard.bool(forKey: askedAccessibilityKey) {
            UserDefaults.standard.set(true, forKey: askedAccessibilityKey)
            Permissions.requestAccessibility()
        }
        refreshSystemState()
    }

    // MARK: Live numbers

    /// Today / this week including the time not saved yet, in seconds. The
    /// worker publishes the saved totals; the rest is added here.
    func liveTotals(now: Date) -> (today: Double, week: Double) {
        var today = snapshot.trackedTodaySeconds
        var week = snapshot.trackedWeekSeconds
        if let since = snapshot.unsavedSince ?? snapshot.blockStartedAt {
            let starts = TrackedDays.dayAndWeekStart(now: now, timeZone: .autoupdatingCurrent)
            today += TrackedDays.trackedSeconds(from: since, to: now, since: starts.today)
            week += TrackedDays.trackedSeconds(from: since, to: now, since: starts.week)
        }
        return (today, week)
    }

    /// The block in progress so far: counts, activity % and per-minute bars,
    /// over the kinds of input that are recorded, on the same buckets as
    /// the upload.
    func liveBlock(now: Date) -> LiveBlock? {
        guard snapshot.status == .tracking, let start = snapshot.blockStartedAt, now > start else { return nil }
        let flags = snapshot.effectiveCapture
        let counts = input.counts(from: start, to: now)
        var percent: Int?
        var bars: [MinuteBar] = []
        // Not measured while input capture is unavailable (as the upload says
        // with no activity %): never "Active 0%".
        if flags.measuresInput && input.isCaptureAvailable(from: start, to: now) {
            percent = input.activityPercent(from: start, to: now, keyboard: flags.keyboard, mouse: flags.mouse)
            let first = EpochTime.seconds(start)
            let last = EpochTime.seconds(now)
            let buckets = ActivityMath.minuteBuckets(start: first, end: last)
            if buckets > 0 {
                let seconds = input.activeSeconds(from: start, to: now, keyboard: flags.keyboard, mouse: flags.mouse)
                let active = ActivityMath.activeMinutes(activeSeconds: seconds, start: first, end: last)
                var inputs = [Int](repeating: 0, count: buckets)
                var stamps: [Int64] = []
                if flags.keyboard {
                    stamps.append(contentsOf: input.keyPresses(from: start, to: now))
                }
                if flags.mouse {
                    stamps.append(contentsOf: input.clicks(from: start, to: now))
                }
                for ms in stamps {
                    let bucket = ActivityMath.bucketOf(second: EpochTime.floorDiv(ms, 1000), start: first, minutes: buckets)
                    inputs[bucket] += 1
                }
                bars = (0..<buckets).map { index in
                    MinuteBar(id: index, active: index < active.count && active[index], inputs: inputs[index])
                }
            }
        }
        return LiveBlock(keyPresses: flags.keyboard ? counts.keyPresses : 0, clicks: flags.mouse ? counts.clicks : 0,
                         activityPercent: percent, minutes: bars)
    }

    /// Foreground time per app over the last hour, most-used first.
    func topApps(now: Date, limit: Int = 6) -> [AppTotal] {
        UsageRecorder.topApps(usage.slices(from: now.addingTimeInterval(-AppModel.topAppsWindow), to: now), limit: limit)
    }

    /// The app in front now (its name only).
    var currentAppName: String? {
        frontApp.frontmostAppName
    }
}
