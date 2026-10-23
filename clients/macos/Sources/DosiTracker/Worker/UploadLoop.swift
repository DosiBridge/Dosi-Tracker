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

/// What the worker tells the uploader. Commands are applied in the order they
/// were sent.
enum UploadCommand: Sendable {
    /// Upload this account's rows from now on (nil: stop uploading). Rows are
    /// only ever uploaded under the token of their owner.
    case account(client: ApiClient?, owner: String?)
    /// New rows were queued: upload them, unless waiting out a back-off.
    case rowsQueued
    /// Upload now, whatever the back-off ("Sync now", the server is back).
    case now
    /// `owner` signed out: upload what fits before `until`, then stop
    /// uploading for it (ignored when another account took over meanwhile).
    case signOut(owner: String, until: Date)
}

/// What the uploader tells the worker. Each pass's events carry the account
/// (queue owner) it ran for: a sign-out drain may still run for an account
/// that is no longer the current one, and its answers must not be applied to
/// the account signed in meanwhile.
enum UploadEvent: Sendable {
    /// An upload attempt ended: the queue counts may have changed.
    case progress
    /// A pass for `owner` ended. `issue` is why the rest must wait (nil:
    /// nothing waits).
    case finished(owner: String, report: SyncReport, issue: UploadIssue?, clock: ClockReading?)
    /// `owner`'s project list, fetched after a 403 / 409 (CONTRACT §2).
    case projects(owner: String, [MyProject])
    /// The server no longer accepts `owner`'s saved password.
    case signInExpired(owner: String, AuthRejection)
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
    private var owner: String?
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
    /// The account whose sign-out drain is running.
    private var signingOutOwner: String?
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
        guard client != nil, owner != nil, now() < limit else { return }
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
        case .account(let newClient, let newOwner):
            setAccount(newClient, owner: newOwner)
        case .rowsQueued:
            kick()
        case .now:
            notBefore = nil
            kick()
        case .signOut(let leaving, let until):
            beginSignOut(leaving, until: until)
        }
    }

    private func setAccount(_ newClient: ApiClient?, owner newOwner: String?) {
        let changed = newOwner != owner || newClient !== client
        if changed && passTask != nil {
            // The previous account's pass stops now.
            passDeadline = now()
        }
        if changed {
            signingOutOwner = nil
            backoffFailures = 0
        }
        client = newClient
        owner = newOwner
        notBefore = nil
        kick()
    }

    private func beginSignOut(_ leaving: String, until: Date) {
        guard let current = owner, current == leaving, client != nil else { return }
        signingOutOwner = leaving
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
        guard !stopped, client != nil, owner != nil else { return }
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
            if signingOutOwner == nil {
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
        if let leaving = signingOutOwner, leaving == owner {
            dropAccount()
        }
        signingOutOwner = nil
        passDeadline = nil
        if let waitUntil = notBefore, pendingCount() > 0 {
            scheduleWake(at: waitUntil)
        }
    }

    /// One `SyncPass` for the current account. Returns whether another pass
    /// should follow at once (the pass reached its upload limit with rows
    /// left and nothing to wait for).
    private func runOnePass() async -> Bool {
        guard let passClient = client, let passOwner = owner else { return false }
        let sink = events
        let pendingBefore = pendingCount()
        let report = await SyncPass.run(
            queue: queue,
            owner: passOwner,
            // One row at a time: each can carry a screenshot, and memory stays
            // flat however long the backlog.
            batchSize: 1,
            upload: { payload in
                if await self.isPastDeadline() {
                    return .failed(.transport(.cancelled))
                }
                let response = await passClient.uploadActivity(payload)
                await sink(.progress)
                return response
            },
            verifyProject: { projectId in
                do {
                    let projects = try await passClient.myProjects()
                    await sink(.projects(owner: passOwner, projects))
                    return ProjectSelection.contains(projectId, in: projects) ? .available : .gone
                } catch let error as ApiError {
                    if case .authRejected(let rejection) = error {
                        await sink(.signInExpired(owner: passOwner, rejection))
                    }
                    CoreLog.warning("upload", "could not check the projects after a refusal: \(error.userMessage)")
                    return .unknown
                } catch {
                    return .unknown
                }
            }
        )
        let clock = await passClient.lastClockReading()

        // Another account took over during the pass: its schedule is its own.
        guard owner == passOwner else {
            await sink(.finished(owner: passOwner, report: report, issue: nil, clock: clock))
            return false
        }
        if !report.uploaded.isEmpty {
            backoffFailures = 0
        }
        let cutShort = isPastDeadline()
        var issue: UploadIssue?
        var more = false
        switch report.stop {
        case .none:
            more = !report.drained && !cutShort
            if more && pendingCount() >= pendingBefore {
                // Rows went up but stay queued (they could not be removed, e.g.
                // the disk is full): starting over at once would resend the
                // same rows back to back. Back off instead.
                CoreLog.warning("upload", "the pass removed no rows from the queue; backing off")
                more = false
                issue = schedule(RetryPlan(reason: .unexpectedResponse(status: 0)))
            }
        case .some(.retry(let plan)):
            if !cutShort {
                issue = schedule(plan)
            }
        case .some(.signInExpired(let rejection)):
            dropAccount()
            await sink(.signInExpired(owner: passOwner, rejection))
        case .some(.queueError(let message)):
            CoreLog.error("upload", "could not read the local queue: \(message)")
            let next = now().addingTimeInterval(UploadLoop.queueRetry)
            notBefore = next
            issue = .queueUnreadable(nextAttemptAt: next)
        }
        await sink(.finished(owner: passOwner, report: report, issue: issue, clock: clock))
        return more
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

    private func isPastDeadline() -> Bool {
        guard let deadline = passDeadline else { return false }
        return now() >= deadline
    }

    private func isReady() -> Bool {
        guard !stopped, client != nil, owner != nil, !isPastDeadline() else { return false }
        if let waitUntil = notBefore, waitUntil > now() {
            return false
        }
        return pendingCount() > 0
    }

    private func pendingCount() -> Int {
        guard let owner = owner else { return 0 }
        return (try? queue.pendingCount(owner: owner)) ?? 0
    }

    private func dropAccount() {
        client = nil
        owner = nil
        notBefore = nil
        backoffFailures = 0
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
            await self.wake()
        }
    }

    private func wake() {
        wakeTask = nil
        kick()
    }
}
