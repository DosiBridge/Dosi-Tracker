import DosiTrackerCore
import Foundation

// Uploading the queue — an actor of its own, so a long backlog on a slow
// uplink never delays pausing, quitting, or saving the block before the Mac
// sleeps (port of the Windows agent's `worker/uploader.rs`).
//
// Each pass is the core's `SyncPass`, which handles every answer as CONTRACT
// §1 ("How agents treat upload responses") says, oldest row first. This actor
// decides when passes run: after new rows, on "Sync now", after the back-off
// (30 s doubling to 30 min, never sooner than the server's `Retry-After`), an
// hour after an inactive subscription, five minutes after "activity in the
// future", and one last bounded pass when signing out or quitting.
//
// Rows belong to a user and the workspace they were tracked in. The signed-in
// user's rows go up workspace by workspace — the current one first, then any
// left from workspaces tracked in before — each with its own `X-Workspace-Id`.
// A workspace the server no longer lets this user (or this Mac) into is left
// alone for the rest of the sign-in: its rows wait.

/// A workspace whose rows are uploaded: the queue owner and the workspace
/// GUID its rows go to.
struct UploadScope: Equatable, Sendable {
    let owner: String
    let workspaceId: String
}

/// Whose rows the uploader sends: the signed-in user and, once one is chosen,
/// the workspace tracked in now.
struct UploadUser: Equatable, Sendable {
    let username: String
    let current: UploadScope?
}

/// What the worker tells the uploader. Commands are applied in the order they
/// were sent.
enum UploadCommand: Sendable {
    /// Upload this user's rows from now on (nil: stop uploading). Rows are
    /// only ever uploaded under the token of their user.
    case account(client: ApiClient?, user: UploadUser?)
    /// New rows were queued: upload them, unless waiting out a back-off.
    case rowsQueued
    /// Upload now, whatever the back-off ("Sync now", the server is back).
    case now
    /// `username` signed out: upload what fits before `until`, then stop
    /// uploading for them (ignored when another user took over meanwhile).
    case signOut(username: String, until: Date)
}

/// What the uploader tells the worker. Each pass's events carry the queue
/// owner it ran for: a sign-out drain, or a pass for an earlier workspace,
/// is not the current account, and its answers must not be applied to it.
enum UploadEvent: Sendable {
    /// An upload attempt ended: the queue counts may have changed.
    case progress
    /// A pass for `owner` ended. `issue` is why the rest must wait (nil:
    /// nothing waits).
    case finished(owner: String, report: SyncReport, issue: UploadIssue?, clock: ClockReading?)
    /// `owner`'s project list, fetched after a 403 / 409 (CONTRACT §2).
    case projects(owner: String, [MyProject])
    /// The server no longer accepts `owner`'s sign-in.
    case signInExpired(owner: String, AuthRejection)
    /// The server refused `owner`'s rows for good (``AccessLoss``): nothing
    /// more goes up for that workspace in this sign-in (for this Mac at all,
    /// when the device was revoked).
    case accessLost(owner: String, AccessLoss)
}

actor UploadLoop {
    typealias EventSink = @Sendable (UploadEvent) async -> Void

    /// How long after an unreadable queue the next pass is tried.
    static let queueRetry: TimeInterval = 60

    nonisolated let inbox: AsyncStream<UploadCommand>.Continuation
    private let commands: AsyncStream<UploadCommand>
    private let queue: Queue
    private let events: EventSink
    private let now: @Sendable () -> Date

    private var client: ApiClient?
    private var user: UploadUser?
    /// Workspaces the server refused for good during this sign-in.
    private var blocked: Set<String> = []
    /// Consecutive failed passes that back off (a success resets it).
    private var backoffFailures = 0
    /// No pass before this (a back-off, an inactive subscription, a clock
    /// ahead of the server's).
    private var notBefore: Date?
    private var passTask: Task<Void, Never>?
    private var wakeTask: Task<Void, Never>?
    /// A running pass stops before its next upload once this has passed.
    private var passDeadline: Date?
    /// New work arrived while a pass ran.
    private var again = false
    /// The user whose sign-out drain is running.
    private var signingOutUser: String?
    /// Quitting: no new passes.
    private var stopped = false

    init(queue: Queue, now: @escaping @Sendable () -> Date = { Date() }, events: @escaping EventSink) {
        var continuation: AsyncStream<UploadCommand>.Continuation?
        commands = AsyncStream { continuation = $0 }
        // The stream's build closure runs synchronously, so it is set here.
        inbox = continuation!
        self.queue = queue
        self.now = now
        self.events = events
    }

    /// Send a command (from any context, in order).
    nonisolated func send(_ command: UploadCommand) {
        inbox.yield(command)
    }

    /// Apply commands until the inbox ends. Started once by the worker.
    func run() async {
        for await command in commands {
            apply(command)
        }
    }

    /// Quit: one last pass, whatever the back-off, that stops starting new
    /// uploads at `until`; nothing more after it.
    func finish(until limit: Date) async {
        stopped = true
        wakeTask?.cancel()
        wakeTask = nil
        notBefore = nil
        passDeadline = min(passDeadline ?? limit, limit)
        if let running = passTask {
            await running.value
        }
        guard client != nil, user != nil, now() < limit else { return }
        passDeadline = limit
        _ = await runOnePass()
        passDeadline = nil
    }

    /// Cleanly cancel background tasks, await their termination, and close the command inbox so run() exits.
    func shutdown() async {
        stopped = true
        let wake = wakeTask
        let pass = passTask
        wake?.cancel()
        pass?.cancel()
        wakeTask = nil
        passTask = nil
        _ = await wake?.result
        _ = await pass?.result
        inbox.finish()
    }

    // MARK: Commands

    private func apply(_ command: UploadCommand) {
        guard !stopped else { return }
        switch command {
        case .account(let newClient, let newUser):
            setAccount(newClient, user: newUser)
        case .rowsQueued:
            kick()
        case .now:
            notBefore = nil
            kick()
        case .signOut(let leaving, let until):
            beginSignOut(leaving, until: until)
        }
    }

    private func setAccount(_ newClient: ApiClient?, user newUser: UploadUser?) {
        let changed = newUser?.username != user?.username || newClient !== client
        if changed && passTask != nil {
            // The previous user's pass stops now.
            passDeadline = now()
        }
        if changed {
            signingOutUser = nil
            backoffFailures = 0
            blocked = []
        }
        if let current = newUser?.current {
            // Chosen again: the server lists it, so it takes rows again.
            blocked.remove(current.owner)
        }
        client = newClient
        user = newUser
        notBefore = nil
        kick()
    }

    private func beginSignOut(_ leaving: String, until: Date) {
        guard let current = user, current.username == leaving, client != nil else { return }
        signingOutUser = leaving
        // The last pass ignores the back-off.
        notBefore = nil
        passDeadline = until
        if passTask != nil {
            again = true
        } else if pendingCount() > 0 {
            startPasses()
        } else {
            dropAccount()
        }
    }

    // MARK: Passes

    private func kick() {
        guard !stopped, client != nil, user != nil else { return }
        if passTask != nil {
            again = true
            return
        }
        if let waitUntil = notBefore, waitUntil > now() {
            scheduleWake(at: waitUntil)
            return
        }
        guard pendingCount() > 0 else { return }
        startPasses()
    }

    private func startPasses() {
        wakeTask?.cancel()
        wakeTask = nil
        passTask = Task {
            await self.runPasses()
        }
    }

    private func runPasses() async {
        while true {
            again = false
            if signingOutUser == nil {
                passDeadline = nil
            }
            let more = await runOnePass()
            if stopped {
                break
            }
            if more || (again && isReady()) {
                continue
            }
            break
        }
        passTask = nil
        if let leaving = signingOutUser, leaving == user?.username {
            dropAccount()
        }
        signingOutUser = nil
        passDeadline = nil
        if let waitUntil = notBefore, pendingCount() > 0 {
            scheduleWake(at: waitUntil)
        }
    }

    /// One `SyncPass` per workspace of the current user, the current one
    /// first. Returns whether another pass should follow at once (a pass
    /// reached its upload limit with rows left and nothing to wait for).
    private func runOnePass() async -> Bool {
        guard let passClient = client, let passUser = user else { return false }
        var more = false
        for scope in uploadScopes() {
            switch await runPass(scope, client: passClient, user: passUser) {
            case .next(let unfinished):
                more = more || unfinished
            case .stop:
                return false
            }
            // Another user took over during the pass: their schedule is their own.
            guard user?.username == passUser.username, client === passClient, !isPastDeadline() else {
                return false
            }
        }
        return more
    }

    /// How one workspace's pass ended.
    private enum PassEnd {
        /// Go on with the next workspace (`unfinished`: rows are left that
        /// the next pass can take at once).
        case next(unfinished: Bool)
        /// Nothing more in this round (a back-off, an expired sign-in, the
        /// account gone).
        case stop
    }

    private func runPass(_ scope: UploadScope, client passClient: ApiClient, user passUser: UploadUser) async -> PassEnd {
        let sink = events
        let pendingBefore = pendingCount(scope.owner)
        let workspaceId = scope.workspaceId
        let report = await SyncPass.run(
            queue: queue,
            owner: scope.owner,
            // One row at a time: each can carry a screenshot, and memory stays
            // flat however long the backlog.
            batchSize: 1,
            upload: { payload in
                if self.isPastDeadline() {
                    return .failed(.transport(.cancelled))
                }
                let response = await passClient.uploadActivity(payload, workspaceId: workspaceId)
                await sink(.progress)
                return response
            },
            verifyProject: { projectId in
                do {
                    let projects = try await passClient.myProjects(workspaceId: workspaceId)
                    await sink(.projects(owner: scope.owner, projects))
                    return ProjectSelection.contains(projectId, in: projects) ? .available : .gone
                } catch let error as ApiError {
                    if case .authRejected(let rejection) = error {
                        await sink(.signInExpired(owner: scope.owner, rejection))
                    }
                    CoreLog.warning("upload", "could not check the projects after a refusal: \(error.userMessage)")
                    return .unknown
                } catch {
                    return .unknown
                }
            }
        )
        let clock = await passClient.lastClockReading()

        // Another user took over during the pass: their schedule is their own.
        guard user?.username == passUser.username, client === passClient else {
            await sink(.finished(owner: scope.owner, report: report, issue: nil, clock: clock))
            return .stop
        }
        if !report.uploaded.isEmpty {
            backoffFailures = 0
        }
        let cutShort = isPastDeadline()
        var issue: UploadIssue?
        var end = PassEnd.next(unfinished: false)
        switch report.stop {
        case .none:
            var unfinished = !report.drained && !cutShort
            if unfinished && pendingCount(scope.owner) >= pendingBefore {
                // Rows went up but stay queued (they could not be removed, e.g.
                // the disk is full): starting over at once would resend the
                // same rows back to back. Back off instead.
                CoreLog.warning("upload", "the pass removed no rows from the queue; backing off")
                unfinished = false
                issue = schedule(RetryPlan(reason: .unexpectedResponse(status: 0)))
                end = .stop
            } else {
                end = .next(unfinished: unfinished)
            }
        case .some(.retry(let plan)):
            if !cutShort {
                issue = schedule(plan)
            }
            end = .stop
        case .some(.signInExpired(let rejection)):
            dropAccount()
            await sink(.signInExpired(owner: scope.owner, rejection))
            end = .stop
        case .some(.queueError(let message)):
            CoreLog.error("upload", "could not read the local queue: \(message)")
            let next = now().addingTimeInterval(UploadLoop.queueRetry)
            notBefore = next
            issue = .queueUnreadable(nextAttemptAt: next)
            end = .stop
        case .some(.accessLost(let loss)):
            CoreLog.warning("upload", "the server refused this workspace's uploads (\(loss.kind.rawValue)); its rows wait")
            blocked.insert(scope.owner)
            if loss.isDeviceWide {
                // This Mac may not upload anywhere any more.
                dropAccount()
                end = .stop
            }
            await sink(.accessLost(owner: scope.owner, loss))
        }
        await sink(.finished(owner: scope.owner, report: report, issue: issue, clock: clock))
        return end
    }

    /// Keep the rest queued and come back after the plan's wait (CONTRACT §1).
    private func schedule(_ plan: RetryPlan) -> UploadIssue {
        let delay: TimeInterval
        let issue: (Date) -> UploadIssue
        switch plan.reason {
        case .subscriptionInactive:
            delay = plan.after ?? UploadPolicy.subscriptionRetry
            issue = { .subscriptionInactive(nextAttemptAt: $0) }
        case .clockAhead:
            delay = plan.after ?? UploadPolicy.clockAheadRetry
            issue = { .clockAhead(nextAttemptAt: $0) }
        default:
            backoffFailures += 1
            let step = UploadBackoff.delay(afterConsecutiveFailures: backoffFailures)
            // Never sooner than the server asked, even beyond the cap.
            delay = max(step, plan.after ?? 0)
            let reason = plan.reason.userMessage
            issue = { .retrying(nextAttemptAt: $0, reason: reason) }
        }
        let next = now().addingTimeInterval(delay)
        notBefore = next
        CoreLog.warning("upload", "uploads wait \(Int(delay)) s: \(plan.reason.userMessage)")
        return issue(next)
    }

    // MARK: Helpers

    /// The workspaces whose rows go up now: the current one first, then the
    /// user's other workspaces with rows left, never one refused for good.
    private func uploadScopes() -> [UploadScope] {
        guard let current = user else { return [] }
        var scopes: [UploadScope] = []
        if let scope = current.current, !blocked.contains(scope.owner) {
            scopes.append(scope)
        }
        let others = (try? queue.pendingOwners(ofUser: current.username)) ?? []
        for owner in others where owner != current.current?.owner && !blocked.contains(owner) {
            if let workspaceId = Queue.workspaceId(ofOwner: owner) {
                scopes.append(UploadScope(owner: owner, workspaceId: workspaceId))
            }
        }
        return scopes
    }

    private func isPastDeadline() -> Bool {
        guard let deadline = passDeadline else { return false }
        return now() >= deadline
    }

    private func isReady() -> Bool {
        guard !stopped, client != nil, user != nil, !isPastDeadline() else { return false }
        if let waitUntil = notBefore, waitUntil > now() {
            return false
        }
        return pendingCount() > 0
    }

    /// Rows waiting in every workspace that takes uploads now.
    private func pendingCount() -> Int {
        uploadScopes().reduce(0) { $0 + pendingCount($1.owner) }
    }

    private func pendingCount(_ owner: String) -> Int {
        (try? queue.pendingCount(owner: owner)) ?? 0
    }

    private func dropAccount() {
        client = nil
        user = nil
        notBefore = nil
        backoffFailures = 0
        blocked = []
        wakeTask?.cancel()
        wakeTask = nil
    }

    /// Wake up at `date` to retry (a wall-clock moment; the worker also sends
    /// `.rowsQueued` after a wake from sleep, which re-checks it).
    private func scheduleWake(at date: Date) {
        wakeTask?.cancel()
        let seconds = max(date.timeIntervalSince(now()), 0)
        wakeTask = Task {
            // At most a day at a time: keeps the nanosecond count far from overflow.
            try? await Task.sleep(nanoseconds: UInt64(min(seconds, 86_400) * 1_000_000_000) + 1_000_000)
            if Task.isCancelled { return }
            self.wake()
        }
    }

    private func wake() {
        wakeTask = nil
        kick()
    }
}
