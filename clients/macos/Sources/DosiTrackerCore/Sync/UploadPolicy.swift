import Foundation

/// Why an upload is kept queued for a later attempt.
public enum RetryReason: Equatable, Sendable {
    /// Network error or timeout.
    case offline
    /// A 5xx.
    case serverError(status: Int)
    /// 429 Too Many Requests.
    case throttled
    /// 404, 405, 408, or any other 4xx without an ABP error code: the
    /// infrastructure (a proxy, a deployment in progress), not this block.
    case unexpectedResponse(status: Int)
    /// A business error code this agent does not know: kept rather than
    /// guessed to be permanent.
    case unrecognizedError(status: Int, code: String)
    /// 401 again right after a fresh sign-in.
    case unauthorized
    /// 402 `Tracker:SubscriptionInactive`.
    case subscriptionInactive
    /// 425 `Tracker:ActivityInFuture`.
    case clockAhead
    /// 403 `Tracker:NotProjectMember` / 409 `Tracker:ProjectArchived`, yet the
    /// project is still listed (or the list could not be refreshed).
    case projectUnavailable
    /// Signing in failed for a reason other than the credentials.
    case signInFailed(status: Int)
    /// The account is locked out for a while (ABP `account_locked`).
    case accountLocked

    /// What to tell the user while uploads wait.
    public var userMessage: String {
        switch self {
        case .offline:
            return "Offline — uploads will retry."
        case .serverError(let status):
            return "The server had a problem (HTTP \(status)) — uploads will retry."
        case .throttled:
            return "The server is busy — uploads will retry."
        case .unexpectedResponse(let status):
            return "Unexpected server response (HTTP \(status)) — uploads will retry."
        case .unrecognizedError(let status, let code):
            return "The server refused the upload (HTTP \(status) \(code)) — it will retry."
        case .unauthorized:
            return "The server did not accept the sign-in — uploads will retry."
        case .subscriptionInactive:
            return "Workspace subscription inactive"
        case .clockAhead:
            return "Your computer clock is ahead"
        case .projectUnavailable:
            return "The server refused this project — uploads will retry."
        case .signInFailed(let status):
            return "Signing in failed (HTTP \(status)) — uploads will retry."
        case .accountLocked:
            return "Your account is temporarily locked — uploads will retry."
        }
    }
}

/// A terminal answer about this device or the workspace it tracks in
/// (cross-agent contract): the device was revoked, the account is no longer a
/// member, the workspace is archived, or the request named no usable
/// workspace. Retrying cannot help: capture and heartbeats stop for that
/// workspace, its queued rows wait, and the user signs in again
/// (``Kind/deviceRevoked``) or switches workspace.
public struct AccessLoss: Equatable, Sendable {
    public enum Kind: String, Equatable, Sendable {
        /// 403 `Tracker:DeviceRevoked`: an administrator removed this Mac.
        case deviceRevoked
        /// 403 `Tracker:NotWorkspaceMember`.
        case notWorkspaceMember
        /// 403 `Tracker:WorkspaceArchived`.
        case workspaceArchived
        /// 400 `Tracker:WorkspaceRequired`: no (valid) workspace was named.
        case workspaceRequired
    }

    public let kind: Kind
    /// The server's own words, when it sent any.
    public let message: String?

    public init(kind: Kind, message: String? = nil) {
        self.kind = kind
        self.message = message
    }

    /// The access loss an error answer means, if it is one: the ABP error code
    /// decides (an older server reports business errors with other statuses).
    public static func of(status: Int, error: AbpError?) -> AccessLoss? {
        guard (400..<500).contains(status), let code = error?.trimmedCode else { return nil }
        let kind: Kind
        switch code {
        case UploadPolicy.Code.deviceRevoked:
            kind = .deviceRevoked
        case UploadPolicy.Code.notWorkspaceMember:
            kind = .notWorkspaceMember
        case UploadPolicy.Code.workspaceArchived:
            kind = .workspaceArchived
        case UploadPolicy.Code.workspaceRequired:
            kind = .workspaceRequired
        default:
            return nil
        }
        return AccessLoss(kind: kind, message: error?.summary)
    }

    /// The access loss behind a failed API call, if that is what it was.
    public static func of(_ error: ApiError) -> AccessLoss? {
        guard case .http(let failure) = error else { return nil }
        return of(status: failure.status, error: failure.error)
    }

    /// Whether it concerns this Mac in every workspace (sign in again), not
    /// one workspace (switch workspace).
    public var isDeviceWide: Bool { kind == .deviceRevoked }

    /// What to tell the user; `workspace` names the workspace concerned.
    public func userMessage(workspace: String) -> String {
        let name = workspace.nonEmptyTrimmed ?? "this workspace"
        switch kind {
        case .deviceRevoked:
            return "This Mac was removed from tracking by your workspace administrator. Tracking has stopped; "
                + "sign in again once the administrator restores it."
        case .notWorkspaceMember:
            return "You are no longer a member of \(name). Tracking there has stopped; choose another workspace."
        case .workspaceArchived:
            return "\(name) was archived. Tracking there has stopped; choose another workspace."
        case .workspaceRequired:
            return "The server did not accept the workspace this Mac tracks in. Choose your workspace again."
        }
    }
}

/// When to try again: after `after` seconds, or (nil) after the exponential
/// ``UploadBackoff``.
public struct RetryPlan: Equatable, Sendable {
    public var reason: RetryReason
    public var after: TimeInterval?

    public init(reason: RetryReason, after: TimeInterval? = nil) {
        self.reason = reason
        self.after = after
    }

    /// The wait before the next attempt, given how many attempts in a row have
    /// failed (including this one).
    public func delay(consecutiveFailures: Int) -> TimeInterval {
        after ?? UploadBackoff.delay(afterConsecutiveFailures: consecutiveFailures)
    }
}

/// What the sync loop does with a queued row after an upload attempt.
public enum UploadDecision: Equatable, Sendable {
    /// 2xx: remove the row from the queue.
    case uploaded
    /// Keep the row queued and stop this pass.
    case retry(RetryPlan)
    /// The token endpoint refused the saved credentials (the client already
    /// dropped its token and signed in again on the 401): state "Sign-in
    /// expired". Stop capturing, keep the queue, ask for the password with the
    /// workspace and email prefilled.
    case signInExpired(AuthRejection)
    /// 413, `Tracker:CaptureTooLarge` or `Tracker:InvalidCaptureData`: the
    /// images are the problem. Strip them from the row and resend at once.
    case stripCapturesAndResend(reason: String)
    /// 403 `Tracker:NotProjectMember` / 409 `Tracker:ProjectArchived`: refresh
    /// the project list. If the row's project is really gone, park the row
    /// with this message and ask the user to choose a project; otherwise keep
    /// it queued.
    case verifyProject(code: String, message: String)
    /// A definitive rejection of this block: park the row (it is never sent
    /// again, and purged after 14 days) and show the message.
    case park(reason: String)
    /// The device or the workspace lost its access (``AccessLoss``): stop
    /// uploading for it; the rows wait (and expire like other stranded rows).
    case accessLost(AccessLoss)
}

/// Classification of upload results, exactly per the CONTRACT §1 table and
/// identical on both agents (port of the Windows `classify_upload_failure`).
///
/// A 401 after the client's silent re-sign-in is transient. Then capture
/// problems (413 or a capture code), whatever the status. Any other non-4xx
/// status is the server's trouble, not this block's. For a 4xx the ABP error
/// code decides — an older server reports every business error as 403, and
/// the code still means the same thing — then the status.
public enum UploadPolicy {
    public enum Code {
        public static let subscriptionInactive = "Tracker:SubscriptionInactive"
        public static let notProjectMember = "Tracker:NotProjectMember"
        public static let projectArchived = "Tracker:ProjectArchived"
        public static let activityInFuture = "Tracker:ActivityInFuture"
        public static let captureTooLarge = "Tracker:CaptureTooLarge"
        public static let invalidCaptureData = "Tracker:InvalidCaptureData"
        public static let activityOverlaps = "Tracker:ActivityOverlaps"
        public static let clientIdConflict = "Tracker:ActivityClientIdConflict"
        public static let activityTooLong = "Tracker:ActivityTooLong"
        public static let activityTooOld = "Tracker:ActivityTooOld"
        public static let timeRangeInvalid = "Tracker:ActivityTimeRangeInvalid"
        public static let deviceRevoked = "Tracker:DeviceRevoked"
        public static let notWorkspaceMember = "Tracker:NotWorkspaceMember"
        public static let workspaceArchived = "Tracker:WorkspaceArchived"
        public static let workspaceRequired = "Tracker:WorkspaceRequired"
    }

    /// Retry after an inactive subscription: hourly.
    public static let subscriptionRetry: TimeInterval = 60 * 60
    /// Retry after "activity in the future": in 5 minutes.
    public static let clockAheadRetry: TimeInterval = 5 * 60

    public static func decide(_ response: UploadResponse) -> UploadDecision {
        switch response {
        case .accepted:
            return .uploaded
        case .failed(let error):
            return decide(error)
        }
    }

    public static func decide(_ error: ApiError) -> UploadDecision {
        switch error {
        case .authRejected(let rejection):
            return .signInExpired(rejection)
        case .transport:
            return .retry(RetryPlan(reason: .offline))
        case .tokenFailed(let failure):
            if failure.status == 429 {
                return .retry(RetryPlan(reason: .throttled, after: failure.retryAfter))
            }
            if failure.isAccountLocked {
                return .retry(RetryPlan(reason: .accountLocked, after: failure.retryAfter))
            }
            return .retry(RetryPlan(reason: .signInFailed(status: failure.status), after: failure.retryAfter))
        case .invalidConfiguration, .invalidResponse:
            return .retry(RetryPlan(reason: .unexpectedResponse(status: 0)))
        case .encoding(let message):
            return .park(reason: message)
        case .http(let failure):
            return decide(status: failure.status, error: failure.error, retryAfter: failure.retryAfter)
        }
    }

    /// Classify an API answer. `retryAfter` (the server's `Retry-After`) is
    /// honoured by every transient outcome that has no fixed wait.
    public static func decide(status: Int, error: AbpError?, retryAfter: TimeInterval?) -> UploadDecision {
        if (200..<300).contains(status) {
            return .uploaded
        }
        if status == 401 {
            // Signing in worked, yet the fresh token was refused: retry later.
            return .retry(RetryPlan(reason: .unauthorized))
        }
        let code = error?.trimmedCode
        let message = error?.summary ?? "HTTP \(status)"

        if status == 413 || code == Code.captureTooLarge || code == Code.invalidCaptureData {
            return .stripCapturesAndResend(reason: message)
        }
        guard (400..<500).contains(status) else {
            // 5xx (or anything unexpected): the server's trouble, not this block's.
            let reason: RetryReason = status >= 500 ? .serverError(status: status) : .unexpectedResponse(status: status)
            return .retry(RetryPlan(reason: reason, after: retryAfter))
        }
        if let loss = AccessLoss.of(status: status, error: error) {
            return .accessLost(loss)
        }

        switch code {
        case .some(Code.subscriptionInactive):
            return .retry(RetryPlan(reason: .subscriptionInactive, after: subscriptionRetry))
        case .some(Code.notProjectMember), .some(Code.projectArchived):
            return .verifyProject(code: code ?? "", message: message)
        case .some(Code.activityInFuture):
            return .retry(RetryPlan(reason: .clockAhead, after: clockAheadRetry))
        case .some(Code.activityOverlaps), .some(Code.clientIdConflict), .some(Code.activityTooLong),
             .some(Code.activityTooOld), .some(Code.timeRangeInvalid):
            return .park(reason: message)
        case .some(let other):
            // A business error this agent does not know: keep the block and
            // retry later rather than guess that it is permanent.
            return .retry(RetryPlan(reason: .unrecognizedError(status: status, code: other), after: retryAfter))
        case .none:
            if status == 400, error?.hasValidationErrors == true {
                return .park(reason: message)
            }
            // 404, 405, 408, 429, or any 4xx without an ABP error code.
            let reason: RetryReason = status == 429 ? .throttled : .unexpectedResponse(status: status)
            return .retry(RetryPlan(reason: reason, after: retryAfter))
        }
    }
}

/// Exponential backoff for transient upload failures (CONTRACT §1): 30 s,
/// 1, 2, 4, 8, 16 min, then every 30 min.
public enum UploadBackoff {
    public static let minimum: TimeInterval = 30
    public static let maximum: TimeInterval = 30 * 60

    /// The wait after `failures` consecutive failed attempts (0 = none).
    public static func delay(afterConsecutiveFailures failures: Int) -> TimeInterval {
        guard failures > 0 else { return 0 }
        let doublings = min(failures - 1, 16)
        return min(minimum * Double(1 << doublings), maximum)
    }
}

/// When the sync loop may try again: the consecutive failures so far and the
/// earliest next attempt. A success (or new work the user asked to upload
/// now, e.g. "Sync now") resets it.
public struct RetrySchedule: Equatable, Sendable {
    public private(set) var consecutiveFailures = 0
    public private(set) var nextAttemptAt: Date?
    /// The reason of the latest failure, for the status line.
    public private(set) var lastReason: RetryReason?

    public init() {}

    /// Whether an attempt may run at `now`.
    public func isDue(at now: Date) -> Bool {
        guard let next = nextAttemptAt else { return true }
        return now >= next
    }

    /// A pass stopped on `plan` at `now`: back off. Returns when to try again.
    @discardableResult
    public mutating func failed(_ plan: RetryPlan, at now: Date) -> Date {
        consecutiveFailures += 1
        lastReason = plan.reason
        let next = now.addingTimeInterval(plan.delay(consecutiveFailures: consecutiveFailures))
        nextAttemptAt = next
        return next
    }

    /// An attempt succeeded (or the user asked to sync now): no waiting.
    public mutating func reset() {
        consecutiveFailures = 0
        nextAttemptAt = nil
        lastReason = nil
    }
}
