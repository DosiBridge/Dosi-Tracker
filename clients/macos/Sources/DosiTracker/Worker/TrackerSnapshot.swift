import DosiTrackerCore
import Foundation

// The state the worker publishes to the UI (port of the Windows agent's
// `state.rs`): a plain value the menu bar and the windows render, replaced as
// a whole on the main actor whenever something changed. Clocks are published
// as instants rather than elapsed values, so the UI animates timers every
// second without any help from the worker.

/// A project the user may track.
struct ProjectOption: Equatable, Sendable, Identifiable {
    let id: String
    let title: String
}

/// A workspace the user may track in, as the picker lists it.
struct WorkspaceOption: Equatable, Sendable, Identifiable {
    let id: String
    let name: String
    /// "Owner", "Manager", "Member"; nil when the server does not say.
    let role: String?
    /// The workspace tracked in now (while switching).
    let isCurrent: Bool
}

/// Why queued blocks are not reaching the server right now (CONTRACT §1 "How
/// agents treat upload responses"). The blocks stay queued; each kind is
/// retried on its own schedule, and "Sync now" retries at once.
enum UploadIssue: Equatable, Sendable {
    /// The server could not be reached or failed: retried with a growing pause
    /// (30 s up to 30 min, never sooner than the server's `Retry-After`).
    case retrying(nextAttemptAt: Date, reason: String)
    /// HTTP 402: the workspace's subscription is inactive. Retried hourly.
    case subscriptionInactive(nextAttemptAt: Date)
    /// HTTP 425: the blocks end in the server's future — this Mac's clock is
    /// ahead. Retried every 5 minutes.
    case clockAhead(nextAttemptAt: Date)
    /// The local queue could not be read. Retried in a minute.
    case queueUnreadable(nextAttemptAt: Date)

    /// One line for the window and the menu.
    var message: String {
        switch self {
        case .retrying(_, let reason):
            return reason
        case .subscriptionInactive:
            return "Workspace subscription inactive — your work is kept and uploads once it is renewed."
        case .clockAhead:
            return "Your computer clock is ahead — uploads wait until the server's time catches up."
        case .queueUnreadable:
            return "The local queue could not be read — uploads will retry."
        }
    }

    var nextAttemptAt: Date {
        switch self {
        case .retrying(let next, _), .subscriptionInactive(let next), .clockAhead(let next), .queueUnreadable(let next):
            return next
        }
    }
}

/// The time the user was away, waiting for "Keep as work" or "Discard".
/// Discarding is the default: a question nobody answers records nothing
/// (CONTRACT §4).
struct AwayPrompt: Equatable, Sendable {
    /// The last input before the user left.
    let from: Date
    /// When they came back (or the screen locked / the Mac slept, which is
    /// never tracked).
    let to: Date
    /// The project the time would be kept on: the one tracked when they left.
    let projectTitle: String

    /// Whole minutes away, for "You were away for 42 min".
    var minutes: Int { max(Int(to.timeIntervalSince(from) / 60), 0) }
}

/// Something worth telling the user once (a notification and a banner).
struct Notice: Equatable, Sendable, Identifiable {
    enum Kind: Equatable, Sendable {
        /// Tracking started without the user doing anything (macOS started
        /// the app at login).
        case trackingStartedAutomatically(projectTitle: String)
        /// Signed in with several projects and none chosen: pick one to start.
        case chooseProject
        /// The project being tracked is no longer available (archived, or the
        /// membership was removed). Tracking moved to `nowTracking` when it is
        /// the only project left, otherwise it stopped until one is chosen.
        case projectUnavailable(title: String, nowTracking: String?)
        /// The local queue was damaged; it was set aside at `movedTo` and a
        /// fresh one started.
        case queueReset(movedTo: String)
        /// The saved sign-in no longer works.
        case signInExpired
        /// Access to the workspace tracked in was lost (removed from it, or it
        /// was archived): tracking stopped there.
        case workspaceLost(message: String)
        /// An administrator removed this Mac: tracking stopped, and the saved
        /// sign-in was forgotten.
        case deviceRevoked
    }

    /// Increases with every notice, so each one is shown once.
    let id: Int
    let at: Date
    let kind: Kind

    /// One line for a notification.
    var text: String {
        switch kind {
        case .trackingStartedAutomatically(let project):
            return "Dosi Tracker started when you logged in and is tracking \(project)."
        case .chooseProject:
            return "Choose the project you are working on to start tracking."
        case .projectUnavailable(let title, let next):
            if let next = next {
                return "\(title) is no longer available. Now tracking \(next)."
            }
            return "\(title) is no longer available. Choose a project to continue tracking."
        case .queueReset(let movedTo):
            return "The local queue was damaged and was set aside (\(movedTo)). Tracking continues."
        case .signInExpired:
            return "Your sign-in has expired. Enter your password to continue tracking."
        case .workspaceLost(let message):
            return message
        case .deviceRevoked:
            return "This Mac was removed from tracking by your workspace administrator. Tracking has stopped."
        }
    }
}

/// The thumbnail of the latest screenshot, shown so people can see exactly
/// what was captured.
struct ScreenshotPreview: Equatable, Sendable {
    let takenAt: Date
    /// JPEG bytes (the small list thumbnail, not the full capture).
    let jpeg: Data
    let blurred: Bool

    static func == (lhs: ScreenshotPreview, rhs: ScreenshotPreview) -> Bool {
        lhs.takenAt == rhs.takenAt && lhs.blurred == rhs.blurred && lhs.jpeg.count == rhs.jpeg.count
    }
}

/// The snapshot the UI renders.
struct TrackerSnapshot: Equatable, Sendable {
    /// How many notices are kept for the UI to pick up.
    static let maxNotices = 8

    /// Increases with every published snapshot, so a late one never replaces a
    /// newer one on the main actor.
    var revision = 0

    var status: TrackerStatus = .signedOut
    /// The signed-in user (their email / user name).
    var displayName = ""
    /// Name of the workspace tracked in; empty while none is chosen.
    var workspace = ""
    /// The API address this account talks to.
    var serverURL = ""
    /// An MDM profile fixes the server address: the sign-in form must not
    /// offer to change it.
    var serverURLIsManaged = false
    /// The sign-in form's suggestions: the last email and server.
    var suggestedUsername = ""
    var suggestedServerURL = ""
    /// Whether the server answered the latest request. Tracking goes on
    /// offline (on the saved project list); blocks upload once it is back.
    var serverReachable = true

    var projects: [ProjectOption] = []
    var selectedProjectId: String?

    /// The workspaces to choose from: after signing in with several, after
    /// losing access to one, or when "Switch workspace" asked for them.
    var workspaceChoices: [WorkspaceOption] = []
    /// The workspace list is being fetched.
    var findingWorkspaces = false
    /// Why there is nothing (else) to choose, or why the last workspace was
    /// left.
    var workspaceNote: String?

    /// Seconds tracked today / this week: the server's totals, plus blocks
    /// recorded since they were fetched. The time not saved yet is added live
    /// by the UI (see ``unsavedSince``).
    var trackedTodaySeconds: Double = 0
    var trackedWeekSeconds: Double = 0
    /// Seconds tracked on each local day of this week, Monday first, for the
    /// week chart. Days before today come from the server; today's slot is not
    /// used (the UI shows the live total).
    var weekDaySeconds: [Double] = Array(repeating: 0, count: 7)
    /// Today's average activity % from the server; nil when nothing was
    /// measured.
    var todayProductivity: Int?

    var intervalMinutes = 10
    /// Minutes without input before the block is ended as idle; nil when the
    /// project turned idle detection off (or no project is known).
    var idleThresholdMinutes: Int?
    var capture = CaptureFlags.nothing
    /// Current local affirmative consent state.
    var localConsent = LocalConsent.absent
    /// Current effective capture flags (local consent intersected with project policy).
    var effectiveCapture = CaptureFlags.nothing
    /// "What are you working on?" — sent as each block's description.
    var taskNote = ""

    /// Blocks recorded during this run, newest first.
    var recentBlocks: [RecentBlock] = []
    var lastScreenshot: ScreenshotPreview?

    /// While tracking: when this uninterrupted stretch began (sign-in,
    /// resume, project switch, unlock, return), when the block in progress
    /// began, when it is saved (its planned end — never the screenshot
    /// moment, which is random and not shown), and where the time not saved
    /// yet begins (earlier than the block while finished blocks wait to see
    /// whether the user is still there).
    var trackingSince: Date?
    var blockStartedAt: Date?
    var nextCutAt: Date?
    var unsavedSince: Date?
    /// While paused: since when, and when tracking resumes by itself (a timed
    /// break) — nil means until the user resumes.
    var pausedSince: Date?
    var resumeAt: Date?
    /// While away: the last input before the user left.
    var awaySince: Date?
    /// After coming back: keep the time away as work, or discard it?
    var awayPrompt: AwayPrompt?

    var lastSync: Date?
    var pendingUploads = 0
    /// Uploads the server permanently rejected, kept visible so dropped data
    /// is never invisible.
    var rejectedUploads = 0
    /// The newest queued and parked rows, with the server's reason for each
    /// parked one (and why the last attempt of a queued one failed).
    var queueRows: [QueuedRow] = []
    /// Why uploads are held back right now, if they are.
    var uploadIssue: UploadIssue?
    /// "Your computer clock is off by N min", once it is (timestamps are never
    /// rewritten).
    var clockWarning: String?

    var lastError: String?
    /// Why the last sign-in failed.
    var loginError: String?
    /// The server demands a new password before signing in (the token to
    /// send with it).
    var passwordChangeToken: String?
    var signingIn = false
    /// Things to tell the user once, oldest first (at most ``maxNotices``).
    var notices: [Notice] = []

    /// The project being tracked, if one is selected and still listed.
    var currentProject: ProjectOption? {
        guard let id = selectedProjectId else { return nil }
        return projects.first { $0.id.caseInsensitiveCompare(id) == .orderedSame }
    }

    /// The same snapshot without its revision, for change detection.
    var withoutRevision: TrackerSnapshot {
        var copy = self
        copy.revision = 0
        return copy
    }
}
