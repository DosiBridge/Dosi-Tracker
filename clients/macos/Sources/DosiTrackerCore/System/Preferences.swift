import Foundation

/// The user's tracking goals, for the progress bars (port of the Windows
/// agent's `prefs.rs` goal rules).
public struct Goals: Equatable, Sendable {
    public static let minimumDailyHours = 1.0
    public static let maximumDailyHours = 16.0
    public static let standard = Goals(dailyHours: 8, workDaysPerWeek: 5)

    /// Hours a day the user aims to track.
    public var dailyHours: Double
    /// Working days a week; the weekly goal is the daily goal times this.
    public var workDaysPerWeek: Int

    public init(dailyHours: Double, workDaysPerWeek: Int) {
        self.dailyHours = dailyHours
        self.workDaysPerWeek = workDaysPerWeek
    }

    /// Values a stray write could put out of range, clamped: the daily goal
    /// to half-hour steps within 1...16 h (8 h if not a number), the week to
    /// 1...7 days.
    public func normalized() -> Goals {
        let hours = dailyHours.isFinite ? dailyHours : 8
        let stepped = (hours * 2).rounded() / 2
        return Goals(
            dailyHours: min(max(stepped, Goals.minimumDailyHours), Goals.maximumDailyHours),
            workDaysPerWeek: min(max(workDaysPerWeek, 1), 7)
        )
    }

    public var dailySeconds: Double { dailyHours * 3600 }
    public var weeklySeconds: Double { dailySeconds * Double(workDaysPerWeek) }
}

/// The task note ("What are you working on?", CONTRACT §4). Changing it ends
/// the block in progress (end reason `task`), so each note covers whole
/// blocks.
public enum TaskNote {
    /// What the UI field accepts.
    public static let maxLength = 200

    /// The note cut to ``maxLength`` characters.
    public static func clamped(_ text: String) -> String {
        String(text.prefix(maxLength))
    }

    /// The `description` sent with a block
    /// (``BlockAssembler/cleanDescription(_:)``); nil when blank.
    public static func description(from note: String?) -> String? {
        BlockAssembler.cleanDescription(note)
    }

    /// Whether changing the note from `old` to `new` changes what blocks
    /// carry (and so must end the block in progress).
    public static func changesDescription(from old: String?, to new: String?) -> Bool {
        description(from: old) != description(from: new)
    }
}

/// The user's own settings, kept in UserDefaults (the app's domain): goals,
/// the task note, a pause that must survive a restart, the remembered
/// project, the install's device id, the server address and the last account
/// (to prefill the sign-in form). Never what is captured: that is the
/// workspace's per-project policy, enforced by the server.
///
/// Thread-safe (UserDefaults is).
public final class Preferences: @unchecked Sendable {
    public enum Key {
        public static let dailyGoalHours = "DailyGoalHours"
        public static let workDaysPerWeek = "WorkDaysPerWeek"
        public static let taskNote = "TaskNote"
        public static let pausedSince = "PausedSince"
        public static let pauseUntil = "PauseUntil"
        public static let rememberedProjectId = "RememberedProjectId"
        public static let deviceId = "DeviceId"
        public static let savedServerURL = "SavedServerURL"
        public static let lastWorkspace = "LastWorkspace"
        public static let lastWorkspaceId = "LastWorkspaceId"
        public static let lastUsername = "LastUsername"
    }

    private let defaults: UserDefaults
    private let lock = NSLock()

    public init(defaults: UserDefaults = .standard) {
        self.defaults = defaults
    }

    /// Daily and weekly goals (normalized on read and write).
    public var goals: Goals {
        get {
            let hours = (defaults.object(forKey: Key.dailyGoalHours) as? NSNumber)?.doubleValue ?? Goals.standard.dailyHours
            let days = (defaults.object(forKey: Key.workDaysPerWeek) as? NSNumber)?.intValue ?? Goals.standard.workDaysPerWeek
            return Goals(dailyHours: hours, workDaysPerWeek: days).normalized()
        }
        set {
            let goals = newValue.normalized()
            defaults.set(goals.dailyHours, forKey: Key.dailyGoalHours)
            defaults.set(goals.workDaysPerWeek, forKey: Key.workDaysPerWeek)
        }
    }

    /// The task note as typed (at most ``TaskNote/maxLength`` characters).
    public var taskNote: String {
        get { TaskNote.clamped(defaults.string(forKey: Key.taskNote) ?? "") }
        set {
            let note = TaskNote.clamped(newValue)
            if note.isEmpty {
                defaults.removeObject(forKey: Key.taskNote)
            } else {
                defaults.set(note, forKey: Key.taskNote)
            }
        }
    }

    /// A pause in effect, kept across restarts and reboots (CONTRACT §4: the
    /// agent starts paused if it was paused).
    public var pause: PauseRecord? {
        get {
            guard let since = (defaults.object(forKey: Key.pausedSince) as? NSNumber)?.doubleValue else { return nil }
            let until = (defaults.object(forKey: Key.pauseUntil) as? NSNumber)?.doubleValue
            return PauseRecord(
                since: Date(timeIntervalSince1970: since),
                until: until.map { Date(timeIntervalSince1970: $0) }
            )
        }
        set {
            if let record = newValue {
                defaults.set(record.since.timeIntervalSince1970, forKey: Key.pausedSince)
                if let until = record.until {
                    defaults.set(until.timeIntervalSince1970, forKey: Key.pauseUntil)
                } else {
                    defaults.removeObject(forKey: Key.pauseUntil)
                }
            } else {
                defaults.removeObject(forKey: Key.pausedSince)
                defaults.removeObject(forKey: Key.pauseUntil)
            }
        }
    }

    /// The project the user last chose.
    public var rememberedProjectId: String? {
        get { defaults.string(forKey: Key.rememberedProjectId)?.nonEmptyTrimmed }
        set { setOrRemove(newValue?.nonEmptyTrimmed, forKey: Key.rememberedProjectId) }
    }

    /// The server address the user entered at sign-in.
    public var savedServerURL: String? {
        get { defaults.string(forKey: Key.savedServerURL)?.nonEmptyTrimmed }
        set { setOrRemove(newValue?.nonEmptyTrimmed, forKey: Key.savedServerURL) }
    }

    /// Name of the workspace last tracked in (shown while the saved sign-in
    /// cannot be read).
    public var lastWorkspace: String? {
        get { defaults.string(forKey: Key.lastWorkspace)?.nonEmptyTrimmed }
        set { setOrRemove(newValue?.nonEmptyTrimmed, forKey: Key.lastWorkspace) }
    }

    /// GUID of the workspace last tracked in: chosen again after signing in
    /// again, and the queue owner while the saved sign-in cannot be read.
    public var lastWorkspaceId: String? {
        get { defaults.string(forKey: Key.lastWorkspaceId)?.nonEmptyTrimmed }
        set { setOrRemove(newValue?.nonEmptyTrimmed, forKey: Key.lastWorkspaceId) }
    }

    /// Email / user name of the last sign-in.
    public var lastUsername: String? {
        get { defaults.string(forKey: Key.lastUsername)?.nonEmptyTrimmed }
        set { setOrRemove(newValue?.nonEmptyTrimmed, forKey: Key.lastUsername) }
    }

    /// This install's device id: a random UUID (lower case) generated on first
    /// use and kept from then on.
    public func deviceId() -> String {
        lock.lock()
        defer { lock.unlock() }
        if let existing = defaults.string(forKey: Key.deviceId), UUID(uuidString: existing) != nil {
            return existing.lowercased()
        }
        let created = UUID().uuidString.lowercased()
        defaults.set(created, forKey: Key.deviceId)
        return created
    }

    private func setOrRemove(_ value: String?, forKey key: String) {
        if let value = value {
            defaults.set(value, forKey: key)
        } else {
            defaults.removeObject(forKey: key)
        }
    }
}
