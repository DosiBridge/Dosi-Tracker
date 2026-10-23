import DosiTrackerCore
import Foundation

// The tracking worker: a port of the Windows agent's `worker.rs` (with its
// `worker/session.rs` rules and `worker/control.rs` server conversations) plus
// the CONTRACT behaviour, as one actor.
//
// It decides when blocks start and end — from the user's commands, the lock,
// sleep and power events, the user's input and the clocks — builds them
// through the core's `BlockAssembler` and queues them locally. It never waits
// on the network while doing so: signing in, the project list, the presence
// heartbeat and the totals run as tasks whose answers come back as actor
// calls, and uploads run in their own actor (`UploadLoop`). After every change
// it publishes a `TrackerSnapshot` for the UI.

actor TrackerWorker {
    /// Everything the worker uses from the app and the system, behind the
    /// core's protocols, so its rules can run against fakes.
    struct Dependencies: Sendable {
        var paths: AppPaths
        var configSource: ConfigSource
        var credentials: any CredentialsStore
        var preferences: Preferences
        var projectCache: ProjectCacheStore
        var consentManager: ConsentManager
        var device: DeviceInfo
        var input: InputLog
        var usage: UsageRecorder
        var screen: any ScreenCapturing
        var webcam: any WebcamCapturing
        var foreground: any ForegroundWindowSource
        var running: any RunningProgramsSource
        var systemIdle: any SystemIdleSource
        /// The hardware-only idle clock (`.hidSystemState`): input the event
        /// tap cannot see (Secure Event Input), never injected events.
        var hardwareIdle: any SystemIdleSource
        /// Whether window titles may be read (the tracked project records the
        /// active window).
        var setReadsWindowTitles: @Sendable (Bool) -> Void
        /// Synchronously configure driver states according to effective per-capability consent.
        var applyDriverPolicy: @Sendable (ConsentedPermissions) -> Void = { _ in }
        /// Hook invoked when shutdown reaches the boundary of joining pending tasks.
        var onShutdownJoining: @Sendable () -> Void = {}
        /// Hook invoked when any capture task (current or retired) finishes on the worker actor.
        var onCaptureCompleted: @Sendable (Int, UInt64) -> Void = { _, _ in }
        /// Factory for API clients (injectable with mock/offline URLSessions for tests).
        var makeApiClient: @Sendable (URL, ApiClient.Account) -> ApiClient = { ApiClient(baseURL: $0, account: $1) }
        /// Receives every new snapshot (from the worker's executor).
        var publish: @Sendable (TrackerSnapshot) -> Void
        var now: @Sendable () -> Date = { Date() }
        /// A clock that does not run while the Mac sleeps (the sleep-gap check
        /// and the refresh schedules).
        var uptime: @Sendable () -> TimeInterval = { ProcessInfo.processInfo.systemUptime }
    }

    /// The signed-in account and its API client (shared with the uploader:
    /// one token).
    struct AccountContext: Sendable {
        let client: ApiClient
        /// The queue owner key of the account.
        let owner: String
        let workspace: String
        let username: String
        let serverURL: URL
    }

    /// Whether to take the time to capture: not before sleep or shutdown,
    /// when the block must be saved at once (and the network is off limits).
    enum Speed: Equatable, Sendable {
        case normal
        case fast

        static func forReason(_ reason: EndReason) -> Speed {
            reason == .sleep || reason == .shutdown ? .fast : .normal
        }
    }

    /// The user is away: no input for the idle threshold (or a break ran out
    /// with nobody at the computer).
    struct Away: Sendable {
        /// The last activity before they left.
        var since: Date
        /// Where the time away stops counting: the screen locked or the Mac
        /// slept meanwhile, and that time is never tracked.
        var until: Date?
        /// After a break: tracking resumes with the next input, nothing to ask.
        var afterBreak: Bool
        /// The project tracked when they left, for "Keep as work".
        var session: TrackingSession?
    }

    /// The time away, waiting for "Keep as work" or "Discard".
    struct PendingPrompt: Sendable {
        let from: Date
        let to: Date
        let session: TrackingSession
    }

    /// A finished block on its way into the queue.
    struct RecordJob: Sendable {
        let block: FinishedBlock
        let speed: Speed
        let owner: String
        let projectTitle: String
        /// Built long after it ended (it was held): no capture is taken now.
        let late: Bool
    }

    struct PresenceKey: Equatable, Sendable {
        let status: PresenceStatus
        let projectId: String?
    }

    /// What one totals refresh brought back.
    struct Totals: Sendable {
        var today: ReportSummary?
        var week: ReportSummary?
        var weekDays: [Double]?
        var failure: ApiError?
        var clock: ClockReading?
    }

    static let noProjectsMessage = "You are not a member of any active project."
    /// The today / this-week totals are fetched again this often…
    static let totalsEvery: TimeInterval = 5 * 60
    /// …and after uploads, but not more often than this.
    static let totalsSpacing: TimeInterval = 15
    /// Budget of a regular heartbeat, sign-in included.
    static let heartbeatTimeout: TimeInterval = 10
    /// Signing out lets the uploader finish the account's queue this long.
    static let signOutUploadGrace: TimeInterval = 15
    /// How many queue rows the window lists.
    static let queueRowsShown = 20

    private let deps: Dependencies
    private var config: Config
    private var started = false
    private var launchedAtLogin = false

    // Storage and uploads.
    private var queue: Queue?
    private var storageFailure: String?
    private var storageRetryAt: TimeInterval = 0
    private var uploader: UploadLoop?
    private var uploadTask: Task<Void, Never>?

    // The account.
    private var saved: StoredSignIn?
    /// The saved sign-in could not be read (Keychain locked or access denied):
    /// the account last signed in, from the preferences, so its rows and its
    /// remembered state are not treated as another account's.
    private var unreadableOwner: String?
    /// When to try reading the saved sign-in again (uptime clock).
    private var keychainRetryAt: TimeInterval?
    private var account: AccountContext?
    private var expired = false
    private var signingIn = false
    /// The server's first answer since starting is awaited.
    private var contacting = false
    private var reachable = true
    /// The project list: the server's, or the saved one while offline.
    private var projects: [MyProject]?
    private var session: TrackingSession?
    private var chooseNotified = false
    private var taskNote: String?

    // What keeps tracking off.
    private var pauseRecord: PauseRecord?
    private var away: Away?
    private var prompt: PendingPrompt?
    private var locked = false
    private var disconnected = false
    /// Since when the Mac is (announced to be) asleep.
    private var suspended: Date?
    /// Since when the session is (announced to be) ending.
    private var shuttingDown: Date?
    /// The last moment the Mac was known to be awake with someone at it
    /// (unlocked, woken, back on screen): a break that ran out before it
    /// resumes there, not inside the sleep.
    private var awakeSince: Date?
    private var quitting = false

    // Blocks and captures.
    private var blocks = BlockTimeline()
    private struct StampedScreen: Sendable {
        let screen: ScreenCapture
        let generation: UInt64
    }
    /// Screenshots taken for blocks not queued yet, with lifecycle generation ticket.
    private var completedScreens: [Int: StampedScreen] = [:]
    private var lifecycleGeneration: UInt64 = 1
    private struct InFlightCapture: Sendable {
        let acquisitionId: UInt64
        let generation: UInt64
        let task: Task<Void, Never>
    }
    /// Active screenshots being taken, keyed by block id.
    private var inFlightCaptureTasks: [Int: InFlightCapture] = [:]
    /// Cancelled or superseded screenshot tasks whose handles are retained to join on shutdown.
    private var retiredCaptureTasks: [Task<Void, Never>] = []
    private var nextAcquisitionId: UInt64 = 1
    /// Finished blocks waiting to be built and queued, oldest first.
    private var recordJobs: [RecordJob] = []
    /// The job the recorder is building now.
    private var recordingJob: RecordJob?
    /// Blocks queued at once because the session ended while the recorder
    /// was building them: it must not queue them again.
    private var flushedJobs: Set<Int> = []
    private var recorder: Task<Void, Never>?
    private var sleepDetector = SleepGapDetector()
    /// Tell the user when tracking starts by itself after a launch at login.
    private var announceAutostart = false
    private var tickTask: Task<Void, Never>?

    // Schedules, on the uptime clock.
    private var nextProjectsRefresh: TimeInterval
    private var nextMaintenance: TimeInterval
    private var nextHeartbeat: TimeInterval = 0
    private var nextTotals: TimeInterval = 0
    private var totalsDueAt: TimeInterval
    private var totalsWanted = false
    private var projectsInFlight = false
    private var projectsTask: Task<Void, Never>?
    private var totalsInFlight = false
    private var totalsTask: Task<Void, Never>?
    private var heartbeatInFlight = false
    private var heartbeatTask: Task<Void, Never>?
    private var farewellTask: Task<Void, Never>?
    private var queuedHeartbeat: Heartbeat?
    private var presenceSent: PresenceKey?
    private var presenceSince: Date
    private var clockReading: ClockReading?
    /// Local midnight of the day the week's past days were fetched on, and when.
    private var weekDaysFetchedFor: Date?
    private var weekDaysFetchedAt: Date?

    // What is published.
    private var shared = TrackerSnapshot()
    private var recent = RecentBlocks()
    private var lastPublished: TrackerSnapshot?
    private var revision = 0
    private var nextNoticeId = 1

    init(dependencies: Dependencies) {
        deps = dependencies
        config = dependencies.configSource.load(savedServerURL: dependencies.preferences.savedServerURL)
        presenceSince = dependencies.now()
        let uptime = dependencies.uptime()
        nextProjectsRefresh = uptime + AgentTiming.projectRefreshInterval
        totalsDueAt = uptime + TrackerWorker.totalsEvery
        nextMaintenance = uptime + WorkerRules.maintenanceEvery
    }

    // MARK: - Starting

    /// Pick up where the last run left off: the saved sign-in, its pause and
    /// task note, and — when the server cannot be reached — the project list
    /// saved with it (at most a week old), so tracking starts offline too.
    func start(launchedAtLogin: Bool, startPeriodicTicks: Bool = true) {
        guard !started else { return }
        started = true
        self.launchedAtLogin = launchedAtLogin
        openStorage(now: deps.now())
        publish()
        if startPeriodicTicks {
            tickTask = Task {
                await self.runTicks()
            }
        }
    }

    /// Open the queue (retried every minute while it fails: nothing can be
    /// recorded without it) and, once it is open, the saved sign-in.
    private func openStorage(now: Date) {
        let savedSignIn = loadSavedSignIn()
        let opened: Queue
        do {
            opened = try Queue.open(at: deps.paths.database, legacyOwner: savedSignIn?.ownerKey, now: now)
        } catch {
            CoreLog.error("worker", "could not open the local queue: \(error)")
            storageFailure = "The local queue cannot be opened (\(error)). Nothing is recorded until it can — retrying every minute."
            storageRetryAt = deps.uptime() + WorkerRules.storageRetry
            return
        }
        queue = opened
        storageFailure = nil
        let loop = UploadLoop(queue: opened, now: deps.now) { [weak self] event in
            await self?.handleUpload(event)
        }
        uploader = loop
        uploadTask = Task {
            await loop.run()
        }
        // Reclaim space (old rejects, other accounts' expired rows, over-limit
        // images), and show what is left.
        opened.maintain(currentOwner: savedSignIn?.ownerKey ?? unreadableOwner)
        publishQueue(owner: savedSignIn?.ownerKey)
        if let aside = opened.movedAside {
            pushNotice(.queueReset(movedTo: aside.path), at: now)
        }
        resumeSignIn(savedSignIn, now: now)
    }

    /// The saved sign-in; nil when there is none, or when the Keychain could
    /// not be read. That is not "nobody signed in": the account last signed
    /// in is then remembered as ``unreadableOwner`` and the read is tried
    /// again later.
    private func loadSavedSignIn() -> StoredSignIn? {
        do {
            let loaded = try deps.credentials.load()
            unreadableOwner = nil
            keychainRetryAt = nil
            return loaded
        } catch {
            CoreLog.warning("worker", "could not read the saved sign-in: \(error)")
            unreadableOwner = lastKnownOwner()
            keychainRetryAt = deps.uptime() + WorkerRules.keychainRetry
            return nil
        }
    }

    /// The account last signed in, from the preferences.
    private func lastKnownOwner() -> String? {
        guard let username = deps.preferences.lastUsername?.trimmingCharacters(in: .whitespacesAndNewlines),
              !username.isEmpty
        else {
            return nil
        }
        return Queue.ownerKey(workspace: deps.preferences.lastWorkspace ?? "", username: username)
    }

    /// The Keychain could not be read at launch: try again, and pick the
    /// sign-in up once it can be read.
    private func retrySavedSignIn(now: Date) {
        guard account == nil, !signingIn, let retryAt = keychainRetryAt, deps.uptime() >= retryAt else { return }
        keychainRetryAt = nil
        if let found = loadSavedSignIn() {
            CoreLog.info("worker", "the saved sign-in can be read now")
            resumeSignIn(found, now: now)
        }
    }

    private func resumeSignIn(_ savedSignIn: StoredSignIn?, now: Date) {
        shared.suggestedWorkspace = deps.preferences.lastWorkspace ?? ""
        shared.suggestedUsername = deps.preferences.lastUsername ?? ""
        shared.suggestedServerURL = config.apiBaseURL.absoluteString
        shared.serverURL = config.apiBaseURL.absoluteString
        guard let savedSignIn = savedSignIn else { return }

        let restored = WorkerRules.restoredPause(deps.preferences.pause, now: now, launchedAtLogin: launchedAtLogin)
        if restored != deps.preferences.pause {
            deps.preferences.pause = restored
        }
        pauseRecord = restored
        taskNote = WorkerRules.cleanNote(deps.preferences.taskNote)

        let client = deps.makeApiClient(config.apiBaseURL, savedSignIn.account)
        let context = AccountContext(client: client, owner: savedSignIn.ownerKey, workspace: savedSignIn.workspace,
                                     username: savedSignIn.username, serverURL: config.apiBaseURL)
        // Offline first: the saved list is used at once; the server's answer
        // replaces it shortly.
        projects = deps.projectCache.load(owner: context.owner, now: now)
        shared.displayName = savedSignIn.username
        shared.workspace = savedSignIn.workspace
        saved = savedSignIn
        account = context
        contacting = true
        requestProjects()
        nextProjectsRefresh = deps.uptime() + AgentTiming.reconnectInterval
        uploader?.send(.account(client: client, owner: context.owner))
        totalsWanted = true
        announceAutostart = launchedAtLogin
        chooseSession(now: now)
        reconcile(at: now, reason: .interval)
    }

    // MARK: - Projects

    /// Without a session: pick the project to track (CONTRACT §2) — or ask.
    private func chooseSession(now: Date) {
        publishProjects()
        guard session == nil, let list = projects else { return }
        switch ProjectSelection.resolve(projects: list, requested: nil, remembered: deps.preferences.rememberedProjectId) {
        case .selected(let project):
            select(project)
        case .choose:
            clearError(TrackerWorker.noProjectsMessage)
            if !chooseNotified {
                chooseNotified = true
                pushNotice(.chooseProject, at: now)
            }
        case .noProjects:
            CoreLog.warning("worker", "signed in, but not a member of any active project")
            shared.lastError = TrackerWorker.noProjectsMessage
        }
    }

    /// Track `project` from the next block on, and remember it.
    private func select(_ project: MyProject) {
        CoreLog.info("worker", "tracking session resolved: \(project.title)")
        let resolved = TrackingSession(project: project, config: config)
        session = resolved
        chooseNotified = false
        clearError(TrackerWorker.noProjectsMessage)
        deps.preferences.rememberedProjectId = project.id
        deps.setReadsWindowTitles(resolved.permissions.activeWindow)
    }

    private func publishProjects() {
        shared.projects = (projects ?? []).map { ProjectOption(id: $0.id, title: $0.title) }
    }

    /// The server's project list: saved for offline starts, the tracked
    /// project's refreshed policy applied from the next block on — or, if that
    /// project is gone, tracking stopped there.
    private func applyProjects(_ list: [MyProject]) {
        let now = deps.now()
        contacting = false
        setReachable(true)
        nextProjectsRefresh = deps.uptime() + AgentTiming.projectRefreshInterval
        if let current = account {
            deps.projectCache.save(ProjectCache(owner: current.owner, resolvedAt: now, projects: list))
        }
        projects = list
        if let current = session {
            if let listed = list.first(where: { $0.id.caseInsensitiveCompare(current.projectId) == .orderedSame }) {
                let refreshed = TrackingSession(project: listed, config: config)
                blocks.updatePolicy(BlockPolicy(session: refreshed))
                session = refreshed
                deps.setReadsWindowTitles(refreshed.permissions.activeWindow)
            } else {
                projectGone(now: now)
            }
        }
        chooseSession(now: now)
        reconcile(at: now, reason: .project)
    }

    /// The tracked project was archived or the membership removed. What was
    /// tracked on it so far is kept (the server takes a block that started
    /// before the archiving); tracking stops, unless exactly one project is
    /// left to move to.
    private func projectGone(now: Date) {
        guard let gone = session else { return }
        session = nil
        CoreLog.warning("worker", "the tracked project is no longer available: \(gone.project.title)")
        let finished = blocks.end(at: now, now: now, reason: .project)
        record(finished, speed: .normal)
        deps.preferences.rememberedProjectId = nil
        var next: String?
        if let list = projects, list.count == 1 {
            next = list[0].title
        }
        // The notice already asks to choose when that is needed.
        chooseNotified = true
        pushNotice(.projectUnavailable(title: gone.project.title, nowTracking: next), at: now)
    }

    private func requestProjects() {
        guard let current = account, !expired, !projectsInFlight else { return }
        projectsInFlight = true
        let client = current.client
        let owner = current.owner
        projectsTask = Task {
            let result: Result<[MyProject], ApiError>
            do {
                result = .success(try await client.myProjects())
            } catch let error as ApiError {
                result = .failure(error)
            } catch {
                result = .failure(.invalidResponse(error.localizedDescription))
            }
            let clock = await client.lastClockReading()
            await self.projectsLoaded(result, owner: owner, clock: clock)
        }
    }

    private func projectsLoaded(_ result: Result<[MyProject], ApiError>, owner: String, clock: ClockReading?) {
        projectsInFlight = false
        projectsTask = nil
        guard account?.owner == owner, !expired, !quitting else { return }
        noteClock(clock)
        switch result {
        case .success(let list):
            applyProjects(list)
        case .failure(let error):
            CoreLog.warning("worker", "could not load the projects: \(error.userMessage)")
            report(error)
            contacting = false
            if !error.isAuthRejection {
                nextProjectsRefresh = deps.uptime() + AgentTiming.reconnectInterval
            }
        }
        publish()
    }

    // MARK: - Tracking on or off

    private var currentEffectivePermissions: ConsentedPermissions {
        guard shouldTrack, let current = session else { return .allDisabled }
        return deps.consentManager.effectivePermissions(project: current.project.allowedCaptures)
    }

    private var shouldTrack: Bool {
        queue != nil && account != nil && !expired && session != nil && pauseRecord == nil && away == nil
            && !locked && !disconnected && suspended == nil && shuttingDown == nil && !quitting
            && deps.consentManager.current.isGranted
            && deps.consentManager.current.permissions.hasAnyActiveCapability
    }

    /// Start or stop the stretch to match the facts; a stop ends the block at
    /// `moment` by `reason`.
    private func reconcile(at moment: Date, reason: EndReason) {
        let now = deps.now()
        let effective = currentEffectivePermissions
        deps.applyDriverPolicy(effective)

        if shouldTrack {
            guard !blocks.isOpen, let current = session else { return }
            blocks.start(at: min(moment, now), policy: BlockPolicy(session: current), description: taskNote,
                         fraction: BlockTimeline.randomFraction())
            if announceAutostart {
                announceAutostart = false
                pushNotice(.trackingStartedAutomatically(projectTitle: current.project.title), at: now)
            }
        } else if blocks.isOpen {
            let finished = blocks.end(at: moment, now: now, reason: reason)
            record(finished, speed: Speed.forReason(reason))
        }
    }

    /// Synchronously signal drivers to stop immediately before any long operations or waits.
    private func stopDriversNow() {
        deps.applyDriverPolicy(.allDisabled)
    }

    // MARK: - Recording

    /// Build the finished blocks and queue them locally. Fast blocks (before
    /// sleep or shutdown) are built at once, without captures; the others go
    /// through the recorder, in order, which may take the screenshot at the
    /// end and the webcam photo first.
    private func record(_ finished: [FinishedBlock], speed: Speed) {
        guard !finished.isEmpty, let current = account else {
            pruneScreens()
            return
        }
        let now = deps.now()
        var queuedFast = false
        for block in finished {
            let job = RecordJob(block: block, speed: speed, owner: current.owner,
                                projectTitle: projectTitle(of: block.policy.projectId),
                                late: now.timeIntervalSince(block.endedAt) > WorkerRules.lateBlock)
            // Counted in the totals at once; the server's totals, refreshed
            // after the upload, replace the estimate.
            addEstimate(start: block.startedAt, end: block.endedAt, sign: 1, now: now)
            if speed == .fast {
                if commit(assemble(job, screen: screen(for: block), webcam: nil, windowNow: nil, effective: currentEffectivePermissions), job: job, now: now) {
                    queuedFast = true
                }
                releaseScreen(block.id)
            } else {
                recordJobs.append(job)
            }
        }
        pruneScreens()
        if queuedFast {
            // Before sleep or shutdown nothing goes over the network.
            queue?.enforceLimitsLogged()
            publishQueue(owner: account?.owner)
        }
        startRecorderIfNeeded()
    }

    private func startRecorderIfNeeded() {
        guard recorder == nil, !recordJobs.isEmpty else { return }
        recorder = Task {
            await self.runRecorder()
        }
    }

    private func runRecorder() async {
        var queued = false
        while !recordJobs.isEmpty && !Task.isCancelled {
            let job = recordJobs.removeFirst()
            recordingJob = job
            let payload = await buildPayload(job)
            recordingJob = nil
            // Queued meanwhile, at once, because the session ended.
            if flushedJobs.remove(job.block.id) != nil {
                continue
            }
            if commit(payload, job: job, now: deps.now()) {
                queued = true
            }
            releaseScreen(job.block.id)
        }
        recorder = nil
        if queued {
            queue?.enforceLimitsLogged()
            publishQueue(owner: account?.owner)
            uploader?.send(.rowsQueued)
        }
        publish()
    }

    /// Await any running recorder pass to ensure all finished blocks are queued and published.
    func flushRecorder() async {
        if let rec = recorder {
            _ = await rec.result
        }
    }

    /// The session is ending (log out, restart, shut down): the blocks still
    /// waiting for the recorder, and the one it is building, are queued now
    /// without captures, since the process may be gone before the recorder
    /// gets to them (on Windows the block is queued before the session-end
    /// answer).
    private func flushRecordJobsFast() {
        var pending = recordJobs
        recordJobs.removeAll()
        if let building = recordingJob {
            flushedJobs.insert(building.block.id)
            pending.insert(building, at: 0)
        }
        guard !pending.isEmpty else { return }
        let now = deps.now()
        var queued = false
        for job in pending {
            let fast = RecordJob(block: job.block, speed: .fast, owner: job.owner, projectTitle: job.projectTitle,
                                 late: job.late)
            if commit(assemble(fast, screen: screen(for: job.block), webcam: nil, windowNow: nil, effective: currentEffectivePermissions), job: fast, now: now) {
                queued = true
            }
            if let entry = inFlightCaptureTasks.removeValue(forKey: job.block.id) {
                entry.task.cancel()
                retiredCaptureTasks.append(entry.task)
            }
            releaseScreen(job.block.id)
        }
        if queued {
            queue?.enforceLimitsLogged()
            publishQueue(owner: account?.owner)
        }
    }

    /// The payload of a block ending normally: its screenshot (the one taken
    /// at its random moment, or one now when it ended before that moment and
    /// may take one at the end), its webcam photo, and the window in front.
    private func buildPayload(_ job: RecordJob) async -> ActivityPayload {
        let block = job.block
        let currentEffective = currentEffectivePermissions
        if completedScreens[block.id] == nil, let pending = inFlightCaptureTasks[block.id]?.task {
            await pending.value
        }
        var shot = screen(for: block)
        let policy = block.policy
        if shot == nil && !job.late && block.captureAtEnd && currentEffective.screenshot
            && policy.permissions.screenshot && block.endReason.allowsCaptureAtEnd {
            let gen = lifecycleGeneration
            let captured = await captureScreenNow(blur: currentEffective.blurScreenshots || policy.blurScreenshots)
            if gen == lifecycleGeneration && currentEffectivePermissions.screenshot {
                shot = captured
            }
        }
        var photo: Data?
        if currentEffective.webcam && policy.permissions.webcam && !job.late && block.endReason.allowsCaptureAtEnd {
            photo = await capturePhotoNow()
        }
        // The window in front now is the block's last one — gated on current effective permission
        var windowNow: ForegroundWindow?
        if !job.late && currentEffective.activeWindow && policy.permissions.activeWindow {
            windowNow = await deps.foreground.currentWindow()
        }
        return assemble(job, screen: shot, webcam: photo, windowNow: windowNow, effective: currentEffective)
    }

    /// Measure and assemble the payload of `job` over its own window only
    /// (the core's `BlockAssembler`, identical to the Windows agent's).
    private func assemble(_ job: RecordJob, screen shot: ScreenCapture?, webcam: Data?,
                          windowNow: ForegroundWindow?, effective: ConsentedPermissions) -> ActivityPayload {
        let block = job.block
        var permissions = block.policy.permissions
        if job.speed == .fast || job.late {
            permissions.webcam = false
        }
        let spec = BlockSpec(projectId: block.policy.projectId, permissions: permissions,
                             blurScreenshots: effective.blurScreenshots || block.policy.blurScreenshots,
                             startedAt: block.startedAt, endedAt: block.endedAt, endReason: block.endReason,
                             taskNote: block.description)
        let programs = (effective.runningPrograms && permissions.runningPrograms) ? deps.running.runningProgramNames() : []
        let measured = BlockAssembler.measure(spec, input: deps.input, usage: deps.usage, windowNow: windowNow,
                                              runningPrograms: programs, screen: shot, webcamJpeg: webcam)
        return BlockAssembler.build(spec, measured: measured, device: deps.device,
                                    zone: BlockZone(TimeZone.autoupdatingCurrent, at: block.startedAt))
    }

    /// Queue one block and show it. Returns whether it was saved.
    private func commit(_ payload: ActivityPayload, job: RecordJob, now: Date) -> Bool {
        guard let queue = queue else {
            addEstimate(start: job.block.startedAt, end: job.block.endedAt, sign: -1, now: now)
            return false
        }
        let rowId: Int64
        do {
            rowId = try queue.enqueue(payload, owner: job.owner)
        } catch {
            CoreLog.error("worker", "could not queue a block: \(error)")
            shared.lastError = "Could not save locally: \(error)"
            addEstimate(start: job.block.startedAt, end: job.block.endedAt, sign: -1, now: now)
            return false
        }
        CoreLog.debug("worker", "block \(rowId) queued (\(payload.endReason?.rawValue ?? "interval"))")
        // Another account signed in meanwhile: its lists must not show this.
        guard account?.owner == job.owner else { return true }
        recent.push(RecentBlock(rowId: rowId, projectTitle: job.projectTitle, payload: payload))
        if let encoded = payload.screenshotThumbJpgBase64, let thumbnail = Data(base64Encoded: encoded) {
            shared.lastScreenshot = ScreenshotPreview(takenAt: payload.screenshotCapturedAt ?? now, jpeg: thumbnail,
                                                      blurred: payload.screenshotBlurred ?? false)
        }
        return true
    }

    /// Add (`sign` 1) or take back (-1) a block's seconds in today's and this
    /// week's totals.
    private func addEstimate(start: Date, end: Date, sign: Double, now: Date) {
        let starts = TrackedDays.dayAndWeekStart(now: now, timeZone: .autoupdatingCurrent)
        let today = TrackedDays.trackedSeconds(from: start, to: end, since: starts.today)
        let week = TrackedDays.trackedSeconds(from: start, to: end, since: starts.week)
        shared.trackedTodaySeconds = max(shared.trackedTodaySeconds + sign * today, 0)
        shared.trackedWeekSeconds = max(shared.trackedWeekSeconds + sign * week, 0)
    }

    private func projectTitle(of projectId: String) -> String {
        if let listed = projects?.first(where: { $0.id.caseInsensitiveCompare(projectId) == .orderedSame }) {
            return listed.title
        }
        return session?.project.title ?? ""
    }

    // MARK: - Screenshots

    /// The screenshot moment of block `id` has come: take it now.
    private func startCapture(_ id: Int) {
        guard let stretch = blocks.stretch else { return }
        let currentEffective = currentEffectivePermissions
        guard currentEffective.screenshot && stretch.block.policy.permissions.screenshot else { return }
        let blur = currentEffective.blurScreenshots || stretch.block.policy.blurScreenshots
        blocks.markCaptured(id)
        let capturer = deps.screen
        let gen = lifecycleGeneration
        let acqId = nextAcquisitionId
        nextAcquisitionId &+= 1
        let task = Task {
            var shot: ScreenCapture?
            do {
                shot = try await capturer.capture(blur: blur)
            } catch {
                CoreLog.warning("worker", "screen capture failed: \(error)")
            }
            await self.captureFinished(id, shot, acquisitionId: acqId, generation: gen)
        }
        inFlightCaptureTasks[id] = InFlightCapture(acquisitionId: acqId, generation: gen, task: task)
    }

    private func captureFinished(_ id: Int, _ shot: ScreenCapture?, acquisitionId acqId: UInt64, generation gen: UInt64) {
        defer {
            deps.onCaptureCompleted(id, acqId)
        }
        let wasInFlight = inFlightCaptureTasks[id]?.acquisitionId == acqId
        if wasInFlight {
            inFlightCaptureTasks[id] = nil
        }
        guard wasInFlight && gen == lifecycleGeneration else { return }
        guard let shot = shot, isLive(id) else { return }
        completedScreens[id] = StampedScreen(screen: shot, generation: gen)
    }

    /// Trigger an active capture for the current open block if screenshot capability is consented and authorized.
    func triggerActiveCapture() {
        guard let stretch = blocks.stretch else { return }
        startCapture(stretch.block.id)
    }

    /// The block's own screenshot, if one was taken in time: one taken after
    /// its end (e.g. after an idle cut) shows time that is not in it.
    private func screen(for block: FinishedBlock) -> ScreenCapture? {
        guard let stamped = completedScreens[block.id],
              stamped.generation == lifecycleGeneration,
              stamped.screen.capturedAt <= block.endedAt.addingTimeInterval(WorkerRules.captureSlack)
        else {
            return nil
        }
        return stamped.screen
    }

    private func captureScreenNow(blur: Bool) async -> ScreenCapture? {
        do {
            return try await deps.screen.capture(blur: blur)
        } catch {
            CoreLog.warning("worker", "screen capture failed: \(error)")
            return nil
        }
    }

    private func capturePhotoNow() async -> Data? {
        do {
            return try await deps.webcam.capturePhoto()
        } catch {
            CoreLog.warning("worker", "webcam capture failed: \(error)")
            return nil
        }
    }

    /// Whether block `id` is still to be queued.
    private func isLive(_ id: Int) -> Bool {
        if blocks.stretch?.blockIds.contains(id) == true || recordingJob?.block.id == id {
            return true
        }
        return recordJobs.contains { $0.block.id == id }
    }

    private func releaseScreen(_ id: Int) {
        completedScreens[id] = nil
        if let entry = inFlightCaptureTasks.removeValue(forKey: id) {
            entry.task.cancel()
            retiredCaptureTasks.append(entry.task)
        }
    }

    /// Forget screenshots of blocks that are gone (dropped by an idle cut).
    private func pruneScreens() {
        for id in completedScreens.keys where !isLive(id) {
            completedScreens[id] = nil
        }
        for (id, entry) in inFlightCaptureTasks where !isLive(id) {
            entry.task.cancel()
            retiredCaptureTasks.append(entry.task)
            inFlightCaptureTasks[id] = nil
        }
    }

    // MARK: - The clock

    private func runTicks() async {
        while !Task.isCancelled && !quitting {
            tick()
            do {
                try await Task.sleep(nanoseconds: UInt64(AgentTiming.tickInterval * 1_000_000_000))
            } catch {
                return
            }
        }
    }

    func tick(at moment: Date? = nil) {
        let now = moment ?? deps.now()
        guard queue != nil else {
            if deps.uptime() >= storageRetryAt {
                openStorage(now: now)
            }
            publish()
            return
        }
        retrySavedSignIn(now: now)
        switch sleepDetector.tick(wall: now, monotonic: deps.uptime()) {
        case .gap(let lastTickAt, _, let missing):
            CoreLog.info("worker", "the Mac slept without saying so (\(Int(missing)) s); the block ends before the gap")
            missedSuspend(beforeGap: lastTickAt, now: now)
        case .clockSteppedBack(let seconds):
            CoreLog.info("worker", "the clock stepped back by \(Int(seconds)) s")
        case .steady:
            break
        }
        // Input after a sleep or shutdown was announced proves the Mac is awake
        // and the session goes on (the wake or the cancellation was never
        // heard): tracking must not stay off for good.
        if let announced = suspended ?? shuttingDown, let back = firstInputAfter(announced, now: now) {
            CoreLog.warning("worker", "input after an announced sleep or shutdown; tracking again")
            suspended = nil
            shuttingDown = nil
            backAtComputer(at: back)
        }
        checkReturn(now: now)
        checkBreakEnd(now: now)
        if let id = blocks.captureDue(now: now) {
            startCapture(id)
        }
        let outcome = blocks.tick(now: now, lastActivity: lastInput(now: now), fraction: BlockTimeline.randomFraction)
        record(outcome.finished, speed: .normal)
        if let since = outcome.wentIdle {
            wentAway(since: since)
        }
        housekeeping()
        maintainQueue()
        publish()
    }

    /// The queue's housekeeping, every few hours while the app stays up: the
    /// 14-day purge of parked rows must not wait for the next launch.
    private func maintainQueue() {
        let uptime = deps.uptime()
        guard let queue = queue, uptime >= nextMaintenance, suspended == nil, shuttingDown == nil, !quitting else {
            return
        }
        nextMaintenance = uptime + WorkerRules.maintenanceEvery
        queue.maintain(currentOwner: account?.owner ?? unreadableOwner)
        publishQueue(owner: account?.owner)
    }

    /// The wall clock jumped ahead of the uptime clock: the Mac slept and
    /// nobody said so. The block ends at the last tick before the gap.
    private func missedSuspend(beforeGap: Date, now: Date) {
        // Awake again now, whether or not a block was running: a break that
        // ran out during the sleep resumes from here.
        awakeSince = now
        freezeAway(at: beforeGap)
        guard let stretch = blocks.stretch, stretch.since < beforeGap else { return }
        let finished = blocks.end(at: beforeGap, now: now, reason: .sleep)
        record(finished, speed: .fast)
        uploader?.send(.rowsQueued)
        // A new block from now on.
        reconcile(at: now, reason: .sleep)
    }

    /// No input for the idle threshold: the block ended at the last input.
    private func wentAway(since: Date) {
        CoreLog.info("worker", "no input for the idle threshold; away")
        // An earlier question nobody answered: no answer means discard.
        prompt = nil
        away = Away(since: since, until: nil, afterBreak: false, session: session)
    }

    private var present: Bool {
        !(locked || disconnected || suspended != nil || shuttingDown != nil)
    }

    private func checkReturn(now: Date) {
        guard let since = away?.since, present else { return }
        if let back = firstInputAfter(since, now: now) {
            comeBack(at: back)
        }
    }

    /// The user is back at `moment`: a new block starts there, and — unless
    /// they were on a break — they are asked about the time away.
    private func comeBack(at moment: Date) {
        guard let left = away else { return }
        away = nil
        let at = max(min(moment, deps.now()), left.since)
        if !left.afterBreak, let leftSession = left.session {
            let to = left.until ?? at
            if to.timeIntervalSince(left.since) >= AgentTiming.minimumBlock {
                prompt = PendingPrompt(from: left.since, to: to, session: leftSession)
            }
        }
        CoreLog.info("worker", "back at the computer")
        reconcile(at: at, reason: .interval)
    }

    /// The screen locked or the Mac slept while away: that time is never
    /// tracked, so the time away ends there.
    private func freezeAway(at moment: Date) {
        guard var current = away else { return }
        if current.until == nil {
            current.until = max(moment, current.since)
        }
        away = current
    }

    /// A timed break ran out: tracking resumes where it ended — or, when
    /// nobody is at the computer, with the next input (CONTRACT §4).
    private func checkBreakEnd(now: Date) {
        guard let current = pauseRecord, let until = current.until, until <= now else { return }
        pauseRecord = nil
        persistPause()
        let threshold = session.flatMap { $0.idleThreshold > 0 ? $0.idleThreshold : nil }
        let last = max(lastInput(now: now) ?? current.since, current.since)
        if session != nil, let threshold = threshold, now.timeIntervalSince(last) >= threshold {
            CoreLog.info("worker", "the break is over, but nobody is at the computer; resuming with the next input")
            away = Away(since: last, until: nil, afterBreak: true, session: nil)
        } else {
            CoreLog.info("worker", "the break is over; resuming tracking")
            reconcile(at: WorkerRules.breakResume(until: until, awakeSince: awakeSince, now: now), reason: .interval)
        }
    }

    /// Refreshes due: the projects, the totals.
    private func housekeeping() {
        guard account != nil, !expired, suspended == nil, shuttingDown == nil, !quitting else { return }
        let uptime = deps.uptime()
        if uptime >= nextProjectsRefresh {
            requestProjects()
            let online = reachable && !contacting
            nextProjectsRefresh = uptime + (online ? AgentTiming.projectRefreshInterval : AgentTiming.reconnectInterval)
        }
        if uptime >= totalsDueAt {
            totalsWanted = true
        }
        if totalsWanted && uptime >= nextTotals {
            requestTotals()
            totalsWanted = false
            nextTotals = uptime + TrackerWorker.totalsSpacing
            totalsDueAt = uptime + TrackerWorker.totalsEvery
        }
    }

    /// The user's latest keyboard or mouse input: while the event tap works,
    /// its own view (real input only), or the hardware-only idle clock when
    /// that is later (keys typed under Secure Event Input never reach the
    /// tap); otherwise the system's idle clock, which mouse jigglers also
    /// reset.
    private func lastInput(now: Date) -> Date? {
        if deps.input.isCaptureHealthy {
            return WorkerRules.latestInput(tap: deps.input.lastActivity,
                                           hardware: deps.hardwareIdle.lastInputAt(now: now))
        }
        return deps.systemIdle.lastInputAt(now: now)
    }

    /// The user's first input after `since`, up to `now` (coming back).
    private func firstInputAfter(_ since: Date, now: Date) -> Date? {
        if deps.input.isCaptureHealthy {
            return WorkerRules.firstInput(after: since, now: now, tapFirst: deps.input.firstActivity(after: since),
                                          hardwareLast: deps.hardwareIdle.lastInputAt(now: now))
        }
        guard let last = deps.systemIdle.lastInputAt(now: now), last > since else { return nil }
        return last
    }

    // MARK: - Lock, sleep and power events

    /// A change of the Mac's state, at the moment it happened (CONTRACT §4).
    func handleSystemEvent(_ event: SystemEvent) {
        guard queue != nil, !quitting else { return }
        let moment = event.at
        switch event.kind {
        case .screenLocked:
            stopDriversNow()
            locked = true
            freezeAway(at: moment)
            reconcile(at: moment, reason: .lock)
        case .sessionDidResignActive:
            stopDriversNow()
            disconnected = true
            freezeAway(at: moment)
            reconcile(at: moment, reason: .lock)
        case .screenUnlocked:
            locked = false
            backAtComputer(at: moment)
        case .sessionDidBecomeActive:
            disconnected = false
            backAtComputer(at: moment)
        case .willSleep:
            stopDriversNow()
            if suspended == nil {
                suspended = moment
            }
            freezeAway(at: moment)
            reconcile(at: moment, reason: .sleep)
        case .didWake:
            if suspended != nil {
                suspended = nil
                // Handled: the tick after the wake must not report it again.
                sleepDetector.reset()
                backAtComputer(at: moment)
                uploader?.send(.rowsQueued)
            }
        case .willPowerOff:
            stopDriversNow()
            if shuttingDown == nil {
                shuttingDown = moment
                freezeAway(at: moment)
                reconcile(at: moment, reason: .shutdown)
                flushRecordJobsFast()
            }
        case .screensDidSleep, .screensDidWake:
            break
        }
        publish()
    }

    /// Unlocked, back on screen, awake again: someone is at the computer (the
    /// password typed on the lock screen is input the event tap never sees).
    private func backAtComputer(at moment: Date) {
        awakeSince = moment
        if away != nil && present {
            comeBack(at: moment)
        } else {
            reconcile(at: moment, reason: .interval)
        }
    }

    // MARK: - The user's commands

    /// A click in the window or the menu is input too: an away stretch ends
    /// there.
    private func userActed(at moment: Date) {
        announceAutostart = false
        if away != nil && present {
            comeBack(at: moment)
        }
    }

    /// Check a sign-in typed in the form; only one the server accepted
    /// replaces the current account. `server` is the address typed under
    /// Advanced (nil or blank: the configured one); `newPassword` answers a
    /// password change the server demanded.
    func signIn(workspace: String, username: String, password: String, server: String?, newPassword: String?) async {
        guard queue != nil, !quitting, !signingIn else { return }
        announceAutostart = false
        if account != nil && !expired {
            CoreLog.warning("worker", "sign-in ignored: already signed in")
            return
        }
        let trimmedWorkspace = workspace.trimmingCharacters(in: .whitespacesAndNewlines)
        let trimmedUsername = username.trimmingCharacters(in: .whitespacesAndNewlines)
        var url = config.apiBaseURL
        var typedServer: String?
        if !config.serverURLIsManaged, let typed = server?.trimmingCharacters(in: .whitespacesAndNewlines), !typed.isEmpty {
            switch Config.validateServerURL(typed) {
            case .success(let valid):
                url = valid
                typedServer = valid.absoluteString
            case .failure(let error):
                shared.loginError = error.userMessage
                publish()
                return
            }
        }
        var change: ApiClient.PasswordChange?
        if let token = shared.passwordChangeToken, let fresh = newPassword, !fresh.isEmpty {
            change = ApiClient.PasswordChange(changePasswordToken: token, newPassword: fresh)
        }
        signingIn = true
        shared.loginError = nil
        publish()

        let candidate = deps.makeApiClient(url, ApiClient.Account(workspace: trimmedWorkspace,
                                                                   username: trimmedUsername,
                                                                   password: password))
        do {
            try await candidate.signIn(passwordChange: change)
        } catch {
            signingIn = false
            signInFailed(error)
            publish()
            return
        }
        let projectsResult: Result<[MyProject], ApiError>
        do {
            projectsResult = .success(try await candidate.myProjects())
        } catch let error as ApiError {
            projectsResult = .failure(error)
        } catch {
            projectsResult = .failure(.invalidResponse(error.localizedDescription))
        }
        let accepted = await candidate.currentAccount()
        let clock = await candidate.lastClockReading()
        signingIn = false
        guard !quitting else { return }
        noteClock(clock)
        signedIn(client: candidate, account: accepted, serverURL: url, typedServer: typedServer,
                 projects: projectsResult)
        publish()
    }

    private func signInFailed(_ error: Error) {
        let apiError = error as? ApiError
        if let apiError = apiError, case .authRejected(let rejection) = apiError {
            CoreLog.info("worker", "the server refused the sign-in: \(rejection.userMessage)")
            shared.passwordChangeToken = rejection.kind == .mustChangePassword ? rejection.changePasswordToken : nil
        } else {
            CoreLog.warning("worker", "could not sign in: \(apiError?.userMessage ?? error.localizedDescription)")
        }
        shared.loginError = apiError?.userMessage ?? error.localizedDescription
    }

    /// The server accepted a sign-in. The same account (its password entered
    /// again after it expired) keeps everything it remembered; another one
    /// starts afresh.
    private func signedIn(client: ApiClient, account accepted: ApiClient.Account, serverURL: URL, typedServer: String?,
                          projects result: Result<[MyProject], ApiError>) {
        let now = deps.now()
        let owner = accepted.ownerKey
        // A Keychain that could not be read is not another account.
        let same = (saved?.ownerKey ?? unreadableOwner) == owner
        unreadableOwner = nil
        keychainRetryAt = nil
        if !same && account != nil {
            forgetAccount(now: now)
        }
        if !same {
            deps.preferences.rememberedProjectId = nil
            deps.preferences.pause = nil
            deps.preferences.taskNote = ""
            deps.projectCache.clear()
            pauseRecord = nil
            taskNote = nil
        }
        let stored = StoredSignIn(account: accepted)
        do {
            try deps.credentials.save(stored)
        } catch {
            CoreLog.error("worker", "could not save the sign-in: \(error)")
            shared.lastError = "Your sign-in could not be saved in the Keychain (\(error)). You will be asked again next time."
        }
        saved = stored
        deps.preferences.lastWorkspace = accepted.workspace
        deps.preferences.lastUsername = accepted.username
        if let typed = typedServer {
            // Remembered only when it differs from the configured address.
            let configured = deps.configSource.load(savedServerURL: nil).apiBaseURL
            deps.preferences.savedServerURL = serverURL == configured ? nil : typed
            config = deps.configSource.load(savedServerURL: deps.preferences.savedServerURL)
        }

        account = AccountContext(client: client, owner: owner, workspace: accepted.workspace,
                                 username: accepted.username, serverURL: serverURL)
        expired = false
        contacting = false
        // Say where this device stands at once.
        presenceSent = nil
        shared.passwordChangeToken = nil
        shared.loginError = nil
        shared.displayName = accepted.username
        shared.workspace = accepted.workspace
        shared.serverURL = serverURL.absoluteString
        shared.suggestedWorkspace = accepted.workspace
        shared.suggestedUsername = accepted.username
        shared.suggestedServerURL = serverURL.absoluteString
        setReachable(true)
        if !same {
            queue?.maintain(currentOwner: owner)
        }
        publishQueue(owner: owner)
        uploader?.send(.account(client: client, owner: owner))
        totalsWanted = true
        switch result {
        case .success(let list):
            applyProjects(list)
        case .failure(let error):
            CoreLog.warning("worker", "signed in, but the projects could not be loaded: \(error.userMessage)")
            nextProjectsRefresh = deps.uptime() + AgentTiming.reconnectInterval
            chooseSession(now: now)
            reconcile(at: now, reason: .interval)
        }
    }

    /// Save the block in progress (ended by `signout`), say so to the server,
    /// forget the saved sign-in and upload what the account left queued (for
    /// a while, in the background).
    func signOut(at moment: Date) async {
        guard queue != nil, !quitting else { return }
        stopDriversNow()
        let now = deps.now()
        let leaving = account
        if let leaving = leaving {
            let finished = blocks.end(at: moment, now: now, reason: .signOut)
            record(finished, speed: .normal)
            if !expired, let heartbeat = makeHeartbeat(.signedOut, since: now, now: now) {
                farewell(heartbeat, client: leaving.client)
            }
        }
        do {
            try deps.credentials.clear()
        } catch {
            CoreLog.warning("worker", "could not forget the saved sign-in: \(error)")
        }
        deps.preferences.rememberedProjectId = nil
        deps.preferences.pause = nil
        deps.preferences.taskNote = ""
        deps.projectCache.clear()
        forgetAccount(now: now)
        publish()
        guard let leaving = leaving else { return }
        // The account's last block reaches the queue before its last uploads.
        if let running = recorder {
            await running.value
        }
        uploader?.send(.signOut(owner: leaving.owner,
                                until: deps.now().addingTimeInterval(TrackerWorker.signOutUploadGrace)))
    }

    /// Let go of the account: nothing it had carries over to the next one. Its
    /// queued rows stay (tagged with it) until it signs in again.
    private func forgetAccount(now: Date) {
        let finished = blocks.end(at: now, now: now, reason: .signOut)
        record(finished, speed: .normal)
        saved = nil
        account = nil
        expired = false
        contacting = false
        projects = nil
        session = nil
        chooseNotified = false
        taskNote = nil
        pauseRecord = nil
        away = nil
        prompt = nil
        presenceSent = nil
        queuedHeartbeat = nil
        announceAutostart = false
        weekDaysFetchedFor = nil
        weekDaysFetchedAt = nil
        pruneScreens()
        recent.removeAll()
        publishQueue(owner: nil)
        deps.setReadsWindowTitles(false)
        shared.displayName = ""
        shared.workspace = ""
        shared.serverURL = config.apiBaseURL.absoluteString
        shared.projects = []
        shared.trackedTodaySeconds = 0
        shared.trackedWeekSeconds = 0
        shared.weekDaySeconds = Array(repeating: 0, count: 7)
        shared.todayProductivity = nil
        shared.lastScreenshot = nil
        shared.uploadIssue = nil
        shared.lastError = nil
        shared.passwordChangeToken = nil
    }

    /// Track another project; the block in progress ends (by `project`) at
    /// the click. The saved list works offline too.
    func selectProject(_ projectId: String, at moment: Date) {
        guard queue != nil, !quitting, account != nil, !expired else { return }
        userActed(at: moment)
        if let current = session, current.projectId.caseInsensitiveCompare(projectId) == .orderedSame {
            publish()
            return
        }
        guard let project = projects?.first(where: { $0.id.caseInsensitiveCompare(projectId) == .orderedSame }) else {
            CoreLog.warning("worker", "cannot switch to a project that is not listed")
            return
        }
        let now = deps.now()
        // The work so far belongs to the project it was done on.
        let finished = blocks.end(at: moment, now: now, reason: .project)
        record(finished, speed: .normal)
        select(project)
        reconcile(at: moment, reason: .project)
        publish()
    }

    /// Stop tracking: the work recorded so far is saved first (ended by
    /// `pause`, or `break` for a timed one). With `resumeAfter`, tracking
    /// resumes by itself when it runs out (once the user is at the computer);
    /// while already paused, only when (or whether) that happens changes. The
    /// pause survives a restart.
    func pause(resumeAfter: TimeInterval?, at moment: Date) {
        guard queue != nil, !quitting, account != nil, !expired else { return }
        stopDriversNow()
        userActed(at: moment)
        let until = resumeAfter.map { moment.addingTimeInterval($0) }
        if let current = pauseRecord {
            pauseRecord = PauseRecord(since: current.since, until: until)
        } else {
            away = nil
            pauseRecord = PauseRecord(since: moment, until: until)
            reconcile(at: moment, reason: until == nil ? .pause : .onBreak)
        }
        persistPause()
        publish()
    }

    /// End a pause: not a second of it (or input made during it) is billed; a
    /// fresh block starts at the click.
    func resume(at moment: Date) {
        guard queue != nil, !quitting, pauseRecord != nil else { return }
        announceAutostart = false
        pauseRecord = nil
        persistPause()
        reconcile(at: moment, reason: .interval)
        publish()
    }

    /// Save the block in progress now (if it is a minute old) and upload at
    /// once — or, while paused, only upload what is already queued.
    func syncNow(at moment: Date) {
        guard queue != nil, !quitting else { return }
        userActed(at: moment)
        let now = deps.now()
        if let stretch = blocks.stretch,
           min(moment, now).timeIntervalSince(stretch.block.startedAt) >= AgentTiming.minimumBlock {
            let finished = blocks.cut(at: moment, now: now, reason: .interval, fraction: BlockTimeline.randomFraction())
            record(finished, speed: .normal)
        }
        uploader?.send(.now)
        totalsWanted = true
        nextTotals = deps.uptime()
        housekeeping()
        publish()
    }

    /// A new task note: it is each block's description, so the block in
    /// progress ends with the old one (by `task`). A block under a minute old
    /// takes the new note instead, rather than being dropped as too short.
    func setTaskNote(_ text: String, at moment: Date) {
        guard queue != nil, !quitting else { return }
        userActed(at: moment)
        let note = WorkerRules.cleanNote(text)
        guard note != taskNote else {
            publish()
            return
        }
        taskNote = note
        if account != nil {
            deps.preferences.taskNote = note ?? ""
        }
        let now = deps.now()
        if let stretch = blocks.stretch {
            if min(moment, now).timeIntervalSince(stretch.block.startedAt) < AgentTiming.minimumBlock {
                blocks.setDescription(note, relabelOpen: true)
            } else {
                blocks.setDescription(note, relabelOpen: false)
                let finished = blocks.cut(at: moment, now: now, reason: .task, fraction: BlockTimeline.randomFraction())
                record(finished, speed: .normal)
            }
        }
        publish()
    }

    /// "Keep as work": the time away is queued as idle blocks of at most one
    /// interval, on the project tracked then, with the current task note.
    /// "Discard" records nothing.
    func resolveAway(keep: Bool) {
        guard let pending = prompt else { return }
        prompt = nil
        defer { publish() }
        guard keep else {
            CoreLog.info("worker", "time away discarded (\(WorkerRules.minutes(pending.to.timeIntervalSince(pending.from))) min)")
            return
        }
        // Also while the sign-in is expired: the rows wait under the account
        // and go up once the password is entered again (as on Windows).
        guard let current = account, let queue = queue else { return }
        let now = deps.now()
        let span = AwaySpan(start: pending.from, end: pending.to)
        let payloads = BlockAssembler.keptIdleBlocks(projectId: pending.session.projectId, span: span,
                                                     interval: pending.session.interval, description: taskNote,
                                                     device: deps.device, timeZone: .autoupdatingCurrent)
        var queued = 0
        for payload in payloads {
            do {
                let rowId = try queue.enqueue(payload, owner: current.owner)
                queued += 1
                addEstimate(start: payload.startedAt, end: payload.endedAt, sign: 1, now: now)
                recent.push(RecentBlock(rowId: rowId, projectTitle: pending.session.project.title, payload: payload))
            } catch {
                CoreLog.error("worker", "could not queue the time kept: \(error)")
                shared.lastError = "Could not save locally: \(error)"
            }
        }
        if queued > 0 {
            queue.enforceLimitsLogged()
            publishQueue(owner: current.owner)
            uploader?.send(.rowsQueued)
        }
        CoreLog.info("worker", "time away kept as work (\(queued) blocks)")
    }

    /// Try the server again now (the user pressed "Try again").
    func reconnect() {
        guard account != nil, !expired, !quitting else { return }
        nextProjectsRefresh = 0
        uploader?.send(.now)
        housekeeping()
        publish()
    }

    /// Quit: save the block in progress first — so whatever happens next it
    /// goes up on the next launch — then say "stopped" and upload what fits
    /// in ``AgentTiming/quitSyncBudget``. When the Mac is shutting down or
    /// logging out, only the local save happens (no network).
    func quit(at moment: Date, systemShutdown: Bool) async {
        guard !quitting else { return }
        quitting = true
        stopDriversNow()
        tickTask?.cancel()
        let started = deps.now()
        let finished = blocks.end(at: moment, now: started, reason: systemShutdown ? .shutdown : .quit)
        record(finished, speed: systemShutdown ? .fast : .normal)
        // An unanswered question about time away: no answer means discard.
        prompt = nil
        guard !systemShutdown else {
            // The process ends within moments: nothing may wait for captures.
            flushRecordJobsFast()
            publish()
            return
        }
        publish()
        if let running = recorder {
            await running.value
        }
        if let current = account, !expired, let heartbeat = makeHeartbeat(.stopped, since: started, now: started) {
            do {
                _ = try await current.client.sendHeartbeat(heartbeat, timeout: AgentTiming.finalHeartbeatTimeout)
            } catch {
                CoreLog.debug("worker", "the last heartbeat did not go out: \(error)")
            }
        }
        await uploader?.finish(until: started.addingTimeInterval(AgentTiming.quitSyncBudget))
    }

    /// Cleanly shut down all asynchronous background tasks, join loops, and stop drivers.
    func shutdown() async {
        quitting = true
        stopDriversNow()

        tickTask?.cancel()
        _ = await tickTask?.result
        tickTask = nil

        projectsTask?.cancel()
        _ = await projectsTask?.result
        projectsTask = nil

        totalsTask?.cancel()
        _ = await totalsTask?.result
        totalsTask = nil

        heartbeatTask?.cancel()
        _ = await heartbeatTask?.result
        heartbeatTask = nil

        farewellTask?.cancel()
        _ = await farewellTask?.result
        farewellTask = nil

        let pendingCaptures = inFlightCaptureTasks.values.map { $0.task } + retiredCaptureTasks
        inFlightCaptureTasks.removeAll()
        retiredCaptureTasks.removeAll()

        deps.onShutdownJoining()

        for task in pendingCaptures {
            task.cancel()
        }
        for task in pendingCaptures {
            _ = await task.result
        }
        completedScreens.removeAll()

        if let loop = uploader {
            await loop.shutdown()
        }
        _ = await uploadTask?.result
        uploadTask = nil

        if let rec = recorder {
            rec.cancel()
            _ = await rec.result
            recorder = nil
        }
    }

    /// Update local affirmative consent and reconcile active collection drivers immediately.
    ///
    /// TRANSITION & PERSISTENCE ORDERING INVARIANTS:
    /// 1. Withdrawals and tightened blur apply IMMEDIATELY and synchronously on the actor before disk I/O.
    /// 2. Active drivers are stopped/reconfigured synchronously before persistence can block.
    /// 3. In-flight samples and completed screens are invalidated immediately.
    /// 4. Persistence runs asynchronously off the actor on a dedicated serial queue so pause/lock/quit
    ///    and newer updates remain completely responsive without blocking actor execution.
    /// 5. Only successful persistence admits expansions.
    /// 6. If a newer transition arrives while an older save is outstanding, the older save cannot
    ///    resurrect withdrawn permissions (superseded check).
    /// 7. On save failure, conservative narrowing remains enforced; expansions are denied.
    func updateConsent(_ next: LocalConsent) async throws {
        let (transition, changed) = deps.consentManager.prepareTransition(to: next)
        guard changed else { return }

        // Step 1: Invalidate in-flight capture tasks and immediately stop/reconfigure drivers synchronously on the actor
        lifecycleGeneration &+= 1
        for (_, entry) in inFlightCaptureTasks {
            entry.task.cancel()
            retiredCaptureTasks.append(entry.task)
        }
        inFlightCaptureTasks.removeAll()
        completedScreens.removeAll()
        reconcile(at: deps.now(), reason: .interval)
        publish()

        // Step 2: Perform persistence off-actor so pause/lock/quit and newer updates remain responsive
        do {
            try await deps.consentManager.persistAsync(transition.target)
        } catch {
            // On save failure: conservative state is preserved!
            deps.consentManager.failTransition(transition)
            reconcile(at: deps.now(), reason: .interval)
            publish()
            throw error
        }

        // Step 3: Persistence succeeded: commit target state if not superseded by a newer transition
        let committed = deps.consentManager.commitTransition(transition)
        if committed {
            lifecycleGeneration &+= 1
            reconcile(at: deps.now(), reason: .interval)
            publish()
        }
    }

    // MARK: - What the uploader reports

    private func handleUpload(_ event: UploadEvent) {
        switch event {
        case .progress:
            publishQueue(owner: account?.owner)
        case .finished(let owner, let report, let issue, let clock):
            noteClock(clock)
            // A previous account's sign-out drain: nothing of it is shown.
            guard account?.owner == owner else {
                publishQueue(owner: account?.owner)
                break
            }
            recent.apply(report)
            if !report.uploaded.isEmpty {
                shared.lastSync = deps.now()
                totalsWanted = true
                CoreLog.debug("worker", "\(report.uploaded.count) blocks reached the server")
            }
            if !report.parked.isEmpty {
                // After a 403 / 409 the projects are fetched again (CONTRACT §2).
                CoreLog.info("worker", "\(report.parked.count) blocks rejected by the server; checking the projects again")
                nextProjectsRefresh = 0
            }
            if account != nil {
                shared.uploadIssue = issue
            }
            publishQueue(owner: account?.owner)
        case .projects(let owner, let list):
            if account?.owner == owner && !expired && !quitting {
                applyProjects(list)
            }
        case .signInExpired(let owner, let rejection):
            if account?.owner == owner {
                expire(reason: rejection.userMessage)
            }
        }
        publish()
    }

    // MARK: - Totals

    private func requestTotals() {
        guard let current = account, !expired, !totalsInFlight else { return }
        totalsInFlight = true
        let client = current.client
        let owner = current.owner
        let now = deps.now()
        let zone = TimeZone.autoupdatingCurrent
        let starts = TrackedDays.dayAndWeekStart(now: now, timeZone: zone)
        // The week's earlier days: once a day, then again only when stale.
        var fresh = false
        if let day = weekDaysFetchedFor, let fetchedAt = weekDaysFetchedAt {
            fresh = day == starts.today && now.timeIntervalSince(fetchedAt) < AgentTiming.weekHistoryTTL
        }
        let pastDays = fresh ? [] : TrackedDays.pastDaysOfWeek(now: now, timeZone: zone)
        let fetchDays = !fresh
        let zoneId = DeviceInfo.wireTimeZone(zone.identifier)
        totalsTask = Task {
            var totals = Totals()
            do {
                // Scoped to the signed-in user: an owner or a manager would
                // otherwise get the whole team's time.
                let userId = try await client.currentUserId()
                totals.today = try await client.summary(from: starts.today, to: now, timeZone: zoneId, userId: userId)
                totals.week = try await client.summary(from: starts.week, to: now, timeZone: zoneId, userId: userId)
                if fetchDays {
                    var seconds = [Double](repeating: 0, count: 7)
                    for day in pastDays where day.slot >= 0 && day.slot < 7 {
                        let summary = try await client.summary(from: day.start, to: day.end, timeZone: zoneId,
                                                               userId: userId)
                        seconds[day.slot] = summary.trackedSeconds
                    }
                    totals.weekDays = seconds
                }
            } catch let error as ApiError {
                totals.failure = error
            } catch {
                totals.failure = .invalidResponse(error.localizedDescription)
            }
            totals.clock = await client.lastClockReading()
            await self.totalsLoaded(totals, owner: owner, dayStart: starts.today, fetchedAt: now)
        }
    }

    /// Show the totals that arrived; on failure what is on screen stays (and
    /// the failure is logged), and the next refresh tries again.
    private func totalsLoaded(_ totals: Totals, owner: String, dayStart: Date, fetchedAt: Date) {
        totalsInFlight = false
        totalsTask = nil
        guard account?.owner == owner, !quitting else { return }
        noteClock(totals.clock)
        if let today = totals.today {
            shared.trackedTodaySeconds = today.trackedSeconds
            shared.todayProductivity = today.averageProductivity.map { Int(min(max($0, 0), 100).rounded()) }
        }
        if let week = totals.week {
            shared.trackedWeekSeconds = week.trackedSeconds
        }
        if let days = totals.weekDays {
            shared.weekDaySeconds = days
            weekDaysFetchedFor = dayStart
            weekDaysFetchedAt = fetchedAt
        }
        if let failure = totals.failure {
            CoreLog.warning("worker", "could not load the totals: \(failure.userMessage)")
            report(failure)
        } else {
            setReachable(true)
        }
        publish()
    }

    // MARK: - Presence

    /// Presence (CONTRACT §3): every minute while signed in, and at once on
    /// every change of status or project. Never queued.
    private func maybeHeartbeat(status: TrackerStatus, now: Date) {
        let online = account != nil && !expired && suspended == nil && shuttingDown == nil && !quitting
        guard online, let presence = status.presence else { return }
        let key = PresenceKey(status: presence, projectId: session?.projectId)
        let changed = presenceSent != key
        if changed {
            presenceSince = now
        }
        let uptime = deps.uptime()
        guard changed || uptime >= nextHeartbeat else { return }
        if let heartbeat = makeHeartbeat(presence, since: presenceSince, now: now) {
            sendHeartbeat(heartbeat)
        }
        presenceSent = key
        nextHeartbeat = uptime + AgentTiming.heartbeatInterval
    }

    /// A heartbeat saying `status` since `since`; nil without a usable device
    /// id (the server needs one).
    private func makeHeartbeat(_ status: PresenceStatus, since: Date, now: Date) -> Heartbeat? {
        guard DeviceInfo.normalizedDeviceId(deps.device.deviceId) != nil else { return nil }
        // The app in front only while tracking, and only when the project
        // records the active window.
        let recordsWindow = session?.permissions.activeWindow ?? false
        let currentApp = status == .tracking && recordsWindow ? deps.foreground.frontmostAppName : nil
        return Heartbeat(device: deps.device, status: status, projectId: session?.projectId, statusSince: since,
                         currentApp: currentApp, timeZone: .autoupdatingCurrent, now: now)
    }

    private func sendHeartbeat(_ heartbeat: Heartbeat) {
        guard let current = account else { return }
        if heartbeatInFlight {
            // The latest one goes out when the one in flight is done.
            queuedHeartbeat = heartbeat
            return
        }
        heartbeatInFlight = true
        let client = current.client
        let owner = current.owner
        heartbeatTask = Task {
            var failure: ApiError?
            do {
                _ = try await client.sendHeartbeat(heartbeat, timeout: TrackerWorker.heartbeatTimeout)
            } catch let error as ApiError {
                failure = error
            } catch {
                failure = .transport(TransportFailure(code: -1, description: error.localizedDescription))
            }
            let clock = await client.lastClockReading()
            await self.heartbeatFinished(owner: owner, failure: failure, clock: clock)
        }
    }

    private func heartbeatFinished(owner: String, failure: ApiError?, clock: ClockReading?) {
        heartbeatInFlight = false
        heartbeatTask = nil
        guard account?.owner == owner, !expired, !quitting else {
            queuedHeartbeat = nil
            return
        }
        noteClock(clock)
        if let failure = failure {
            // Never retried or queued: the next one is due in a minute.
            CoreLog.debug("worker", "heartbeat failed: \(failure.userMessage)")
            report(failure)
        } else {
            setReachable(true)
        }
        if let next = queuedHeartbeat {
            queuedHeartbeat = nil
            sendHeartbeat(next)
        }
        publish()
    }

    /// The last heartbeat before letting go of the account: best effort,
    /// never waited for.
    private func farewell(_ heartbeat: Heartbeat, client: ApiClient) {
        farewellTask = Task {
            do {
                _ = try await client.sendHeartbeat(heartbeat, timeout: AgentTiming.finalHeartbeatTimeout)
            } catch {
                CoreLog.debug("worker", "the last heartbeat did not go out: \(error)")
            }
        }
    }

    // MARK: - The server's state

    /// Tell what a failed request means: a refused sign-in, or the server
    /// being out of reach.
    private func report(_ error: ApiError) {
        switch error {
        case .authRejected(let rejection):
            expire(reason: rejection.userMessage)
        case .transport:
            setReachable(false)
        default:
            // The server answered, if not with what was asked.
            setReachable(true)
        }
    }

    /// Back online: retry the uploads now, and re-validate the projects.
    private func setReachable(_ value: Bool) {
        if value && !reachable {
            CoreLog.info("worker", "the server is reachable again")
            uploader?.send(.now)
            nextProjectsRefresh = 0
        }
        if !value {
            contacting = false
        }
        reachable = value
    }

    /// The server refused the saved password (changed, reset, the account
    /// deactivated): tracking stops, the queue is kept for when it is entered
    /// again (CONTRACT §1).
    private func expire(reason: String) {
        guard account != nil, !expired else { return }
        let now = deps.now()
        CoreLog.warning("worker", "the saved sign-in is no longer accepted: \(reason)")
        let finished = blocks.end(at: now, now: now, reason: .signOut)
        record(finished, speed: .normal)
        expired = true
        contacting = false
        away = nil
        // A pending "You were away" question stays: time kept is queued under
        // the account and goes up once the password is entered again.
        uploader?.send(.account(client: nil, owner: nil))
        shared.loginError = "Your sign-in has expired (\(reason)). Enter your password to continue — your tracked time is kept."
        pushNotice(.signInExpired, at: now)
    }

    private func noteClock(_ reading: ClockReading?) {
        if let reading = reading {
            clockReading = reading
        }
    }

    // MARK: - Publishing

    private func persistPause() {
        guard account != nil else { return }
        deps.preferences.pause = pauseRecord
    }

    private func pushNotice(_ kind: Notice.Kind, at moment: Date) {
        shared.notices.append(Notice(id: nextNoticeId, at: moment, kind: kind))
        nextNoticeId += 1
        let excess = shared.notices.count - TrackerSnapshot.maxNotices
        if excess > 0 {
            shared.notices.removeFirst(excess)
        }
    }

    private func clearError(_ message: String) {
        if shared.lastError == message {
            shared.lastError = nil
        }
    }

    /// Show the signed-in account's queue: pending / rejected counts and the
    /// newest rows with the server's reasons (nothing when signed out).
    private func publishQueue(owner: String?) {
        guard let queue = queue, let owner = owner else {
            shared.pendingUploads = 0
            shared.rejectedUploads = 0
            shared.queueRows = []
            return
        }
        shared.pendingUploads = (try? queue.pendingCount(owner: owner)) ?? 0
        shared.rejectedUploads = (try? queue.rejectedCount(owner: owner)) ?? 0
        shared.queueRows = (try? queue.recentRows(owner: owner, limit: TrackerWorker.queueRowsShown)) ?? []
    }

    private func facts() -> StatusFacts {
        var facts = StatusFacts()
        if queue == nil {
            facts.storageFailure = storageFailure ?? "The local queue is not open yet."
        }
        facts.signedIn = account != nil
        facts.signInExpired = expired
        facts.paused = pauseRecord != nil
        facts.timedBreak = pauseRecord?.until != nil
        facts.hasSession = session != nil
        facts.projectsKnown = projects != nil
        facts.contacting = contacting || signingIn
        facts.locked = locked || disconnected
        facts.suspended = suspended != nil || shuttingDown != nil
        facts.away = away != nil
        facts.consentGranted = deps.consentManager.current.isGranted && deps.consentManager.current.permissions.hasAnyActiveCapability
        facts.tracking = blocks.isOpen
        return facts
    }

    /// Publish the status and the clocks the UI animates (only when something
    /// changed), and send a heartbeat when the presence changed or one is due.
    private func publish() {
        let now = deps.now()
        let status = facts().status
        var next = shared
        next.status = status
        let stretch = status == .tracking ? blocks.stretch : nil
        next.trackingSince = stretch?.since
        next.blockStartedAt = stretch?.block.startedAt
        next.nextCutAt = stretch?.block.plannedEnd
        next.unsavedSince = stretch?.unsavedSince
        let shownPause = status.isPaused ? pauseRecord : nil
        next.pausedSince = shownPause?.since
        next.resumeAt = shownPause?.until
        next.awaySince = status == .away ? away?.since : nil
        next.awayPrompt = prompt.map { AwayPrompt(from: $0.from, to: $0.to, projectTitle: $0.session.project.title) }
        next.selectedProjectId = session?.projectId
        // Before a project is resolved, the configured interval.
        next.intervalMinutes = session.map { WorkerRules.minutes($0.interval) } ?? config.intervalMinutes
        next.idleThresholdMinutes = session.flatMap { $0.idleThreshold > 0 ? WorkerRules.minutes($0.idleThreshold) : nil }
        next.capture = session.map { CaptureFlags(session: $0) } ?? CaptureFlags.nothing
        next.localConsent = deps.consentManager.current
        next.effectiveCapture = CaptureFlags(consented: currentEffectivePermissions)
        next.taskNote = taskNote ?? ""
        next.serverReachable = reachable
        next.serverURLIsManaged = config.serverURLIsManaged
        next.recentBlocks = recent.items
        next.clockWarning = clockReading?.warning
        next.signingIn = signingIn

        let comparable = next.withoutRevision
        if comparable != lastPublished {
            lastPublished = comparable
            revision += 1
            next.revision = revision
            deps.publish(next)
        }
        maybeHeartbeat(status: status, now: now)
    }
}
