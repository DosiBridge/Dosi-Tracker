import Foundation

/// What the project list says about a project the server refused
/// (`Tracker:NotProjectMember` / `Tracker:ProjectArchived`).
public enum ProjectVerdict: Equatable, Sendable {
    /// No longer in the refreshed my-projects list: the row is parked and the
    /// user must choose a project.
    case gone
    /// Still listed: keep the row and retry later.
    case available
    /// The list could not be refreshed (offline): keep the row and retry later.
    case unknown
}

/// A row parked by a pass, with the server's reason.
public struct ParkedRow: Equatable, Sendable {
    public let id: Int64
    public let projectId: String
    public let reason: String

    public init(id: Int64, projectId: String, reason: String) {
        self.id = id
        self.projectId = projectId
        self.reason = reason
    }
}

/// Why a pass stopped before the queue was drained.
public enum SyncStop: Equatable, Sendable {
    /// Keep the remaining rows; try again after the plan's delay.
    case retry(RetryPlan)
    /// The saved credentials were refused: "Sign-in expired" (stop capturing,
    /// keep the queue, ask for the password).
    case signInExpired(AuthRejection)
    /// The queue itself could not be read.
    case queueError(String)
}

/// The outcome of one upload pass.
public struct SyncReport: Equatable, Sendable {
    /// Upload attempts made (a strip-and-resend counts once).
    public var attempted = 0
    /// Rows the server accepted (and that left the queue).
    public var uploaded: [Int64] = []
    /// Rows parked as rejected.
    public var parked: [ParkedRow] = []
    /// Rows whose images were stripped after a 413 / capture error.
    public var stripped: [Int64] = []
    /// Projects the server refused that are gone from the project list: the
    /// worker leaves "Tracking" for "Choose a project" if one is current.
    public var goneProjects: [String] = []
    /// Why the pass ended early; nil when it ran out of rows or reached its
    /// upload limit.
    public var stop: SyncStop?
    /// The queue had no more of this owner's rows.
    public var drained = false

    public init() {}
}

/// One pass over the signed-in account's queued blocks, oldest first (port of
/// the Windows worker's `sync_pending`, with the CONTRACT §1 handling of each
/// answer):
///
/// * 2xx → the row leaves the queue.
/// * 413 / capture error → its images are stripped and it is resent at once.
/// * `NotProjectMember` / `ProjectArchived` → `verifyProject` refreshes the
///   project list; a project that is really gone parks the row with the
///   server's message, otherwise the row is kept and the pass stops.
/// * A definitive rejection → the row is parked with the server's message.
/// * Anything transient → the reason is noted on the row, the pass stops, and
///   the rest waits for ``SyncStop/retry(_:)``'s plan.
/// * Refused credentials → the pass stops with "Sign-in expired".
///
/// Rows are read ``AgentTiming/uploadBatchSize`` at a time and at most
/// ``AgentTiming/maxUploadsPerPass`` are tried, so a large backlog drains over
/// a few passes. Rows of any other account are never read. Freed pages go
/// back to the disk at the end.
public enum SyncPass {
    public static func run(
        queue: Queue,
        owner: String,
        maxUploads: Int = AgentTiming.maxUploadsPerPass,
        batchSize: Int = AgentTiming.uploadBatchSize,
        upload: (ActivityPayload) async -> UploadResponse,
        verifyProject: (String) async -> ProjectVerdict
    ) async -> SyncReport {
        var report = SyncReport()
        var cursor: Int64 = 0
        var verdicts: [String: ProjectVerdict] = [:]

        pass: while report.attempted < maxUploads {
            let batch: [QueuedActivity]
            do {
                batch = try queue.pendingBatch(owner: owner, afterId: cursor,
                                               limit: max(min(batchSize, maxUploads - report.attempted), 1))
            } catch {
                CoreLog.error("sync", "failed to read pending activities: \(error)")
                report.stop = .queueError("\(error)")
                break
            }
            if batch.isEmpty {
                report.drained = true
                break
            }

            for row in batch {
                // Paging by id: a row that fails to leave the queue is never
                // retried within the same pass.
                cursor = row.id
                report.attempted += 1
                var decision = UploadPolicy.decide(await upload(row.payload))

                if case .stripCapturesAndResend(let reason) = decision {
                    // The images are the problem: the time data goes up
                    // without them, at once.
                    CoreLog.warning("sync", "server refused the captures of activity \(row.id) (\(reason)); stripping them")
                    let stillQueued = (try? queue.stripCaptures(id: row.id)) ?? false
                    report.stripped.append(row.id)
                    guard stillQueued else { continue }
                    decision = UploadPolicy.decide(await upload(row.payload.strippingCaptures()))
                    if case .stripCapturesAndResend = decision {
                        // Refused again without images: not something stripping
                        // can fix. Keep the time data and back off.
                        decision = .retry(RetryPlan(reason: .unexpectedResponse(status: 413)))
                    }
                }

                switch decision {
                case .uploaded:
                    do {
                        try queue.removeSynced(id: row.id)
                    } catch {
                        CoreLog.warning("sync", "uploaded activity \(row.id) could not leave the queue: \(error)")
                    }
                    report.uploaded.append(row.id)

                case .park(let reason):
                    park(row, reason: reason, in: queue, report: &report)

                case .verifyProject(let code, let message):
                    let projectId = row.payload.projectId
                    let verdict: ProjectVerdict
                    if let known = verdicts[projectId.lowercased()] {
                        verdict = known
                    } else {
                        verdict = await verifyProject(projectId)
                        verdicts[projectId.lowercased()] = verdict
                    }
                    if verdict == .gone {
                        CoreLog.warning("sync", "project \(projectId) is gone (\(code)); parking activity \(row.id)")
                        park(row, reason: message, in: queue, report: &report)
                        if !report.goneProjects.contains(where: { $0.caseInsensitiveCompare(projectId) == .orderedSame }) {
                            report.goneProjects.append(projectId)
                        }
                    } else {
                        note(row, RetryReason.projectUnavailable.userMessage, in: queue)
                        report.stop = .retry(RetryPlan(reason: .projectUnavailable))
                        break pass
                    }

                case .retry(let plan):
                    CoreLog.warning("sync", "upload of activity \(row.id) failed (\(plan.reason.userMessage)); will retry")
                    note(row, plan.reason.userMessage, in: queue)
                    report.stop = .retry(plan)
                    break pass

                case .signInExpired(let rejection):
                    report.stop = .signInExpired(rejection)
                    break pass

                case .stripCapturesAndResend:
                    // Resolved above; never reached.
                    break
                }
            }
        }

        // Uploaded and parked rows (and their images) leave free pages behind.
        do {
            try queue.reclaimSpace()
        } catch {
            CoreLog.warning("sync", "could not reclaim the local queue's free space: \(error)")
        }
        return report
    }

    private static func park(_ row: QueuedActivity, reason: String, in queue: Queue, report: inout SyncReport) {
        CoreLog.warning("sync", "server rejected activity \(row.id); parking it (\(reason))")
        do {
            try queue.markRejected(id: row.id, reason: reason)
        } catch {
            CoreLog.error("sync", "could not park activity \(row.id): \(error)")
        }
        report.parked.append(ParkedRow(id: row.id, projectId: row.payload.projectId, reason: reason))
    }

    private static func note(_ row: QueuedActivity, _ reason: String, in queue: Queue) {
        do {
            try queue.noteAttempt(id: row.id, reason: reason)
        } catch {
            CoreLog.warning("sync", "could not note the failed attempt on activity \(row.id): \(error)")
        }
    }
}
