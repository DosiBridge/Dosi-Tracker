import DosiTrackerCore
import Foundation

// The agent's status as the menu bar, the window and the presence heartbeat
// show it (port of the Windows agent's `TrackerStatus` and of
// `worker/session.rs`'s `status` / `presence`): pure values, so the rules are
// the same on both platforms.

/// What the agent is doing. The first reason tracking is not running wins
/// (see ``StatusFacts/status``).
enum TrackerStatus: Equatable, Sendable {
    /// No saved sign-in: the sign-in window is shown.
    case signedOut
    /// Signed in to the server, but no workspace is chosen to track in: after
    /// signing in with several, or after losing access to the one tracked in
    /// (removed, archived). Nothing is tracked and no heartbeat is sent.
    case choosingWorkspace
    /// Signed in, waiting for the server's first answer (no project list yet).
    case connecting
    /// Signed in, but no project is chosen: several and none remembered, the
    /// remembered one is gone, or there are none at all.
    case choosingProject
    /// A block is being recorded.
    case tracking
    /// Paused until the user resumes; nothing is recorded.
    case paused
    /// A timed break; tracking resumes by itself when it runs out (once the
    /// user is at the computer).
    case onBreak
    /// No input for the project's idle threshold: the block ended at the last
    /// input; tracking resumes when the user is back (CONTRACT §4).
    case away
    /// The screen is locked, or another user's session is on screen.
    case locked
    /// The Mac is going to sleep or shutting down / logging out.
    case asleep
    /// Signed in, but the server cannot be reached and no recent project list
    /// is saved to track offline with.
    case offline
    /// The server no longer accepts the saved password. Tracking stopped; the
    /// queue is kept and uploads once the password is entered again.
    case signInExpired
    /// Signed in, but the user has not chosen what may be recorded on this
    /// Mac (or revoked it): nothing is captured until they do. It counts as
    /// signed in — signing out, switching project and uploading what is
    /// already queued all work — and the presence heartbeat says `stopped`.
    case consentRequired
    /// Nothing can be recorded (the local queue cannot be opened); the text
    /// says why.
    case stopped(String)

    /// One short phrase for the menu and the window's status pill.
    var label: String {
        switch self {
        case .signedOut:
            return "Signed out"
        case .choosingWorkspace:
            return "Choose a workspace"
        case .connecting:
            return "Connecting…"
        case .choosingProject:
            return "Choose a project"
        case .tracking:
            return "Tracking"
        case .paused:
            return "Paused"
        case .onBreak:
            return "On a break"
        case .away:
            return "Away"
        case .locked:
            return "Locked"
        case .asleep:
            return "Asleep"
        case .offline:
            return "Offline — will retry"
        case .signInExpired:
            return "Sign-in expired"
        case .consentRequired:
            return "Consent required"
        case .stopped:
            return "Stopped"
        }
    }

    /// What the presence heartbeat reports (CONTRACT §3); nil when no
    /// heartbeat can or should be sent (signed out, the sign-in expired, no
    /// answer from the server yet).
    var presence: PresenceStatus? {
        switch self {
        case .tracking:
            return .tracking
        case .paused:
            return .paused
        case .onBreak:
            return .onBreak
        case .away:
            return .idle
        case .locked, .asleep:
            return .locked
        case .choosingProject:
            return .choosing
        case .stopped, .consentRequired:
            return .stopped
        case .signedOut, .signInExpired, .connecting, .offline, .choosingWorkspace:
            return nil
        }
    }

    /// Pausing applies while signed in and not already paused — including
    /// while offline, so a flaky connection never stops someone from taking a
    /// break, and while away from the computer.
    var canPause: Bool {
        switch self {
        case .tracking, .offline, .away:
            return true
        default:
            return false
        }
    }

    /// Paused or on a timed break.
    var isPaused: Bool {
        self == .paused || self == .onBreak
    }

    /// The sign-in form is what the user needs.
    var needsSignIn: Bool {
        self == .signedOut || self == .signInExpired
    }

    /// Signed in with a usable account in a workspace (an expired one, or
    /// one still choosing its workspace, does not count; one waiting for the
    /// user's consent does).
    var isSignedIn: Bool {
        switch self {
        case .signedOut, .signInExpired, .stopped, .choosingWorkspace:
            return false
        default:
            return true
        }
    }
}

/// What decides the status (port of the Windows worker's `Facts`).
struct StatusFacts: Equatable, Sendable {
    /// The local queue is unusable: nothing can be recorded (why).
    var storageFailure: String?
    var signedIn = false
    /// Signed in to the server without a workspace yet.
    var choosingWorkspace = false
    var signInExpired = false
    var paused = false
    /// The pause is a timed break.
    var timedBreak = false
    var hasSession = false
    /// A project list is known (from the server, or saved).
    var projectsKnown = false
    /// The server's first answer is still awaited.
    var contacting = false
    var locked = false
    /// Asleep, or the session is ending (shutdown, restart, log out).
    var suspended = false
    var away = false
    /// The user consented to at least one kind of capture on this Mac.
    var consentGranted = false
    var tracking = false

    var status: TrackerStatus {
        if let failure = storageFailure {
            return .stopped(failure)
        }
        if !signedIn {
            return choosingWorkspace ? .choosingWorkspace : .signedOut
        }
        if signInExpired {
            return .signInExpired
        }
        // Before anything else that keeps tracking off: nothing starts
        // without it (as on Windows).
        if !consentGranted {
            return .consentRequired
        }
        if paused {
            return timedBreak ? .onBreak : .paused
        }
        if !hasSession {
            if projectsKnown {
                return .choosingProject
            }
            return contacting ? .connecting : .offline
        }
        if suspended {
            return .asleep
        }
        if locked {
            return .locked
        }
        if away {
            return .away
        }
        return tracking ? .tracking : .connecting
    }
}
