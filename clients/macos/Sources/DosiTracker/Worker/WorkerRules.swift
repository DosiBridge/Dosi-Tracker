import DosiTrackerCore
import Foundation

// Small rules of the tracking worker, as pure functions (ports of the Windows
// worker's `restored_pause`, `clean_note` and its capture constants), kept
// apart from the actor so they read — and can be tested — on their own.

enum WorkerRules {
    /// A screenshot taken this long after its block's end (a tick late) still
    /// belongs to it; one taken later — after an idle cut — does not.
    static let captureSlack: TimeInterval = 10

    /// A block built this long after it ended (it was held while the user was
    /// inactive) gets no webcam photo or capture at the end: one taken now
    /// would not show that block.
    static let lateBlock: TimeInterval = 30

    /// How often opening the local queue is retried while it fails.
    static let storageRetry: TimeInterval = 60

    /// How often reading the saved sign-in is retried after the Keychain
    /// could not be read (locked, access denied): not every minute, as each
    /// attempt may show the Keychain's own prompt.
    static let keychainRetry: TimeInterval = 5 * 60

    /// How often the queue's housekeeping (old rejects, other accounts'
    /// expired rows, image limits) runs while the app stays up (as the
    /// Windows worker's `MAINTENANCE_EVERY`).
    static let maintenanceEvery: TimeInterval = 6 * 60 * 60

    /// Where tracking resumes after a timed break ran out at `until`: there,
    /// but never before the Mac was last known awake (unlocked, woken, back
    /// on screen) at `awakeSince`, so a break that ran out during a sleep does
    /// not bill the sleep; and never in the future.
    static func breakResume(until: Date, awakeSince: Date?, now: Date) -> Date {
        min(max(until, awakeSince ?? until), now)
    }

    /// The user's latest input: the event tap's, or the hardware-only idle
    /// clock's when that is later. Secure Event Input (Terminal's Secure
    /// Keyboard Entry, password fields) hides keys from the tap but not from
    /// the hardware clock, which still ignores software-injected events.
    static func latestInput(tap: Date?, hardware: Date?) -> Date? {
        guard let tap = tap else { return hardware }
        guard let hardware = hardware else { return tap }
        return max(tap, hardware)
    }

    /// The first input after `since` (up to `now`): the tap's own, or, when
    /// it saw none, the hardware clock's latest (see
    /// ``latestInput(tap:hardware:)``).
    static func firstInput(after since: Date, now: Date, tapFirst: Date?, hardwareLast: Date?) -> Date? {
        if let first = tapFirst, first <= now {
            return first
        }
        guard let hardware = hardwareLast, hardware > since, hardware <= now else { return nil }
        return hardware
    }

    /// The pause to start with, from the one saved when the app last ran.
    ///
    /// A pause survives a restart. A timed break still running when the user
    /// opens the app goes on and resumes by itself; but a break that ran out
    /// meanwhile, or any break when macOS started the app at login, stays
    /// paused until the user resumes — tracking never restarts silently from
    /// a saved pause.
    static func restoredPause(_ saved: PauseRecord?, now: Date, launchedAtLogin: Bool) -> PauseRecord? {
        guard let pause = saved else { return nil }
        let stillRunning = pause.until.map { $0 > now } ?? false
        if stillRunning && !launchedAtLogin {
            return pause
        }
        return PauseRecord(since: pause.since, until: nil)
    }

    /// The task note as kept: whitespace and control characters collapse to
    /// single spaces, at most ``TaskNote/maxLength`` characters, trimmed; nil
    /// when blank.
    static func cleanNote(_ note: String) -> String? {
        var words: [String] = []
        var word = ""
        for scalar in note.unicodeScalars {
            if CharacterSet.whitespacesAndNewlines.contains(scalar) || CharacterSet.controlCharacters.contains(scalar) {
                if !word.isEmpty {
                    words.append(word)
                    word = ""
                }
            } else {
                word.unicodeScalars.append(scalar)
            }
        }
        if !word.isEmpty {
            words.append(word)
        }
        var kept = String(words.joined(separator: " ").prefix(TaskNote.maxLength))
        while let last = kept.last, last.isWhitespace {
            kept.removeLast()
        }
        return kept.isEmpty ? nil : kept
    }

    /// Whole minutes of a duration in seconds (rounded), for display.
    static func minutes(_ seconds: TimeInterval) -> Int {
        Int((seconds / 60).rounded())
    }
}

/// What the current project captures — the server's policy combined with the
/// local configuration. Shown read-only in the window; the live view also
/// hides input counts the project does not record.
struct CaptureFlags: Equatable, Sendable {
    var screenshot: Bool
    var webcam: Bool
    var keyboard: Bool
    var mouse: Bool
    var activeWindow: Bool
    var runningPrograms: Bool
    /// Screen captures are blurred (text unreadable) before they leave the Mac.
    var blurScreenshots: Bool

    init(screenshot: Bool, webcam: Bool, keyboard: Bool, mouse: Bool, activeWindow: Bool, runningPrograms: Bool,
         blurScreenshots: Bool) {
        self.screenshot = screenshot
        self.webcam = webcam
        self.keyboard = keyboard
        self.mouse = mouse
        self.activeWindow = activeWindow
        self.runningPrograms = runningPrograms
        self.blurScreenshots = blurScreenshots
    }

    /// The flags of `session`.
    init(session: TrackingSession) {
        let permissions = session.permissions
        self.init(screenshot: permissions.screenshot, webcam: permissions.webcam, keyboard: permissions.keyboard,
                  mouse: permissions.mouse, activeWindow: permissions.activeWindow,
                  runningPrograms: permissions.runningPrograms, blurScreenshots: session.blurScreenshots)
    }

    /// Nothing known yet (no project). Not named `none`, which would read as
    /// `Optional.none`.
    static let nothing = CaptureFlags(screenshot: false, webcam: false, keyboard: false, mouse: false,
                                      activeWindow: false, runningPrograms: false, blurScreenshots: false)

    /// Whether an activity % is measured at all.
    var measuresInput: Bool { keyboard || mouse }

    /// As the core's capture permissions (for the privacy checks).
    var permissions: CapturePermissions {
        CapturePermissions(screenshot: screenshot, webcam: webcam, keyboard: keyboard, mouse: mouse,
                           activeWindow: activeWindow, runningPrograms: runningPrograms)
    }
}

/// The project list as last resolved by the server for one account, so
/// tracking can start (or switch project) while the server is unreachable
/// (port of the Windows `ProjectCache`).
struct ProjectCache: Codable, Equatable, Sendable {
    /// How long the saved list may be used without reaching the server.
    static let maxAge: TimeInterval = 7 * 24 * 60 * 60
    /// A saved list newer than this (the device clock went backwards) is not
    /// trusted either.
    static let maxFuture: TimeInterval = 10 * 60

    /// The queue owner key of the account it belongs to.
    var owner: String
    var resolvedAt: Date
    var projects: [MyProject]

    init(owner: String, resolvedAt: Date, projects: [MyProject]) {
        self.owner = owner
        self.resolvedAt = resolvedAt
        self.projects = projects
    }

    /// Usable for at most ``maxAge`` after it was resolved.
    func isUsable(at now: Date) -> Bool {
        let age = now.timeIntervalSince(resolvedAt)
        return age <= ProjectCache.maxAge && age >= -ProjectCache.maxFuture
    }
}

/// Keeps the ``ProjectCache`` in UserDefaults (the project list holds no
/// secret; the sign-in itself is in the Keychain).
///
/// Thread-safe (UserDefaults is).
final class ProjectCacheStore: @unchecked Sendable {
    static let key = "ProjectCache"

    private let defaults: UserDefaults

    init(defaults: UserDefaults = .standard) {
        self.defaults = defaults
    }

    /// The saved list of `owner`, if it may still be used at `now`.
    func load(owner: String, now: Date) -> [MyProject]? {
        guard let data = defaults.data(forKey: ProjectCacheStore.key),
              let cache = try? WireJSON.decoder().decode(ProjectCache.self, from: data),
              cache.owner == owner, cache.isUsable(at: now)
        else {
            return nil
        }
        return cache.projects
    }

    func save(_ cache: ProjectCache) {
        do {
            defaults.set(try WireJSON.encoder().encode(cache), forKey: ProjectCacheStore.key)
        } catch {
            CoreLog.warning("worker", "could not save the project list for offline use: \(error)")
        }
    }

    func clear() {
        defaults.removeObject(forKey: ProjectCacheStore.key)
    }
}
