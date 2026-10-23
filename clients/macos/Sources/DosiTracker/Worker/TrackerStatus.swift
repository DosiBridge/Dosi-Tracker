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
    /// Nothing can be recorded (the local queue cannot be opened); the text
    /// says why.
    case stopped(String)

    /// One short phrase for the menu and the window's status pill.
    var label: String {
        switch self {
        case .signedOut:
            return "Signed out"
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
        case .stopped:
            return .stopped
        case .signedOut, .signInExpired, .connecting, .offline:
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

    /// Signed in with a usable account (an expired one does not count).
    var isSignedIn: Bool {
        switch self {
        case .signedOut, .signInExpired, .stopped:
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
    var consentGranted = false
    var tracking = false

    var status: TrackerStatus {
        if let failure = storageFailure {
            return .stopped(failure)
        }
        if !signedIn {
            return .signedOut
        }
        if signInExpired {
            return .signInExpired
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
        if !consentGranted {
            return .stopped("Consent required")
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
