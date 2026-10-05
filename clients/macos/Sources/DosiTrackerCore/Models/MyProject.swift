import Foundation

/// A project the signed-in user may track, from
/// `GET /api/app/project/my-projects` (`MyProjectDto`, CONTRACT §2).
///
/// Decoding is lenient so a new agent keeps working against an older server:
/// `idleThresholdMinutes` defaults to 5 and `blurScreenshots` to false when
/// absent or null, and a missing capture flag reads as "not allowed".
public struct MyProject: Codable, Equatable, Sendable, Identifiable {
    /// The server's default idle threshold, used when it does not send one.
    public static let defaultIdleThresholdMinutes = 5
    /// The longest idle threshold a project can set.
    public static let maxIdleThresholdMinutes = 60

    public var id: String
    public var title: String
    public var intervalMinutes: Int
    public var allowScreenshot: Bool
    public var allowWebcam: Bool
    public var allowKeyboard: Bool
    public var allowMouse: Bool
    public var allowActiveWindow: Bool
    public var allowRunningPrograms: Bool
    /// Minutes without input before the user counts as away; 0 = off.
    public var idleThresholdMinutes: Int
    /// Screen captures and thumbnails must be blurred before upload.
    public var blurScreenshots: Bool

    public init(
        id: String,
        title: String,
        intervalMinutes: Int,
        allowScreenshot: Bool,
        allowWebcam: Bool,
        allowKeyboard: Bool,
        allowMouse: Bool,
        allowActiveWindow: Bool,
        allowRunningPrograms: Bool,
        idleThresholdMinutes: Int = MyProject.defaultIdleThresholdMinutes,
        blurScreenshots: Bool = false
    ) {
        self.id = id
        self.title = title
        self.intervalMinutes = intervalMinutes
        self.allowScreenshot = allowScreenshot
        self.allowWebcam = allowWebcam
        self.allowKeyboard = allowKeyboard
        self.allowMouse = allowMouse
        self.allowActiveWindow = allowActiveWindow
        self.allowRunningPrograms = allowRunningPrograms
        self.idleThresholdMinutes = idleThresholdMinutes
        self.blurScreenshots = blurScreenshots
    }

    /// The project's block interval, clamped to 5...60 minutes, in seconds.
    public var interval: TimeInterval { BlockClock.clampedInterval(minutes: intervalMinutes) }

    /// The idle threshold in seconds, clamped to at most 60 minutes; 0 when
    /// the project turned idle detection off.
    public var idleThreshold: TimeInterval {
        TimeInterval(min(max(idleThresholdMinutes, 0), MyProject.maxIdleThresholdMinutes) * 60)
    }

    /// The capture flags the server allows for this project.
    public var allowedCaptures: CapturePermissions {
        CapturePermissions(
            screenshot: allowScreenshot,
            webcam: allowWebcam,
            keyboard: allowKeyboard,
            mouse: allowMouse,
            activeWindow: allowActiveWindow,
            runningPrograms: allowRunningPrograms
        )
    }

    private enum CodingKeys: String, CodingKey {
        case id, title, intervalMinutes
        case allowScreenshot, allowWebcam, allowKeyboard, allowMouse, allowActiveWindow, allowRunningPrograms
        case idleThresholdMinutes, blurScreenshots
    }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        id = try c.decode(String.self, forKey: .id)
        title = try c.decodeIfPresent(String.self, forKey: .title) ?? ""
        intervalMinutes = try c.decodeIfPresent(Int.self, forKey: .intervalMinutes) ?? 10
        allowScreenshot = try c.decodeIfPresent(Bool.self, forKey: .allowScreenshot) ?? false
        allowWebcam = try c.decodeIfPresent(Bool.self, forKey: .allowWebcam) ?? false
        allowKeyboard = try c.decodeIfPresent(Bool.self, forKey: .allowKeyboard) ?? false
        allowMouse = try c.decodeIfPresent(Bool.self, forKey: .allowMouse) ?? false
        allowActiveWindow = try c.decodeIfPresent(Bool.self, forKey: .allowActiveWindow) ?? false
        allowRunningPrograms = try c.decodeIfPresent(Bool.self, forKey: .allowRunningPrograms) ?? false
        // A null threshold means the server did not set one: its default, not
        // 0 (which would turn idle detection off).
        idleThresholdMinutes = try c.decodeIfPresent(Int.self, forKey: .idleThresholdMinutes)
            ?? MyProject.defaultIdleThresholdMinutes
        blurScreenshots = try c.decodeIfPresent(Bool.self, forKey: .blurScreenshots) ?? false
    }

    public func encode(to encoder: Encoder) throws {
        var c = encoder.container(keyedBy: CodingKeys.self)
        try c.encode(id, forKey: .id)
        try c.encode(title, forKey: .title)
        try c.encode(intervalMinutes, forKey: .intervalMinutes)
        try c.encode(allowScreenshot, forKey: .allowScreenshot)
        try c.encode(allowWebcam, forKey: .allowWebcam)
        try c.encode(allowKeyboard, forKey: .allowKeyboard)
        try c.encode(allowMouse, forKey: .allowMouse)
        try c.encode(allowActiveWindow, forKey: .allowActiveWindow)
        try c.encode(allowRunningPrograms, forKey: .allowRunningPrograms)
        try c.encode(idleThresholdMinutes, forKey: .idleThresholdMinutes)
        try c.encode(blurScreenshots, forKey: .blurScreenshots)
    }

    /// Decode a my-projects response (a plain JSON array), skipping rather
    /// than failing on an entry that cannot be read.
    public static func decodeList(from data: Data) throws -> [MyProject] {
        try WireJSON.decoder().decode([LossyProject].self, from: data).compactMap { $0.project }
    }

    private struct LossyProject: Decodable {
        let project: MyProject?

        init(from decoder: Decoder) throws {
            project = try? MyProject(from: decoder)
        }
    }
}

/// What may be captured: the server's project policy combined with the local
/// configuration (which can only switch captures off, never on).
public struct CapturePermissions: Codable, Equatable, Sendable {
    public var screenshot: Bool
    public var webcam: Bool
    public var keyboard: Bool
    public var mouse: Bool
    public var activeWindow: Bool
    public var runningPrograms: Bool

    public init(screenshot: Bool, webcam: Bool, keyboard: Bool, mouse: Bool, activeWindow: Bool, runningPrograms: Bool) {
        self.screenshot = screenshot
        self.webcam = webcam
        self.keyboard = keyboard
        self.mouse = mouse
        self.activeWindow = activeWindow
        self.runningPrograms = runningPrograms
    }

    /// The local default, as on Windows: everything a project may ask for
    /// except the webcam, which a deployment must switch on explicitly.
    public static let localDefault = CapturePermissions(
        screenshot: true, webcam: false, keyboard: true, mouse: true, activeWindow: true, runningPrograms: true)

    /// Everything (tests, previews).
    public static let all = CapturePermissions(
        screenshot: true, webcam: true, keyboard: true, mouse: true, activeWindow: true, runningPrograms: true)

    /// Nothing at all (not named `none`, which would read as `Optional.none`).
    public static let nothing = CapturePermissions(
        screenshot: false, webcam: false, keyboard: false, mouse: false, activeWindow: false, runningPrograms: false)

    /// Only what both allow.
    public func intersection(_ other: CapturePermissions) -> CapturePermissions {
        CapturePermissions(
            screenshot: screenshot && other.screenshot,
            webcam: webcam && other.webcam,
            keyboard: keyboard && other.keyboard,
            mouse: mouse && other.mouse,
            activeWindow: activeWindow && other.activeWindow,
            runningPrograms: runningPrograms && other.runningPrograms
        )
    }

    /// Whether any input kind is recorded, i.e. whether an activity % can be
    /// measured at all.
    public var measuresInput: Bool { keyboard || mouse }
}

/// Which project to track after the project list loads (CONTRACT §2).
public enum ProjectSelection: Equatable, Sendable {
    /// Track this project.
    case selected(MyProject)
    /// Several projects and no usable choice: track nothing, ask the user
    /// ("Choose a project").
    case choose([MyProject])
    /// The user is not a tracking member of any active project.
    case noProjects

    /// The project the user just asked for, if listed; otherwise the only
    /// project when there is exactly one; otherwise the remembered project if
    /// still listed; otherwise the user must choose. Never falls back to "the
    /// first project", which silently filed time under the wrong client.
    public static func resolve(projects: [MyProject], requested: String?, remembered: String?) -> ProjectSelection {
        if projects.isEmpty {
            return .noProjects
        }
        if let wanted = find(requested, in: projects) {
            return .selected(wanted)
        }
        if projects.count == 1 {
            return .selected(projects[0])
        }
        if let previous = find(remembered, in: projects) {
            return .selected(previous)
        }
        return .choose(projects)
    }

    /// The selected project, if any.
    public var project: MyProject? {
        if case .selected(let project) = self { return project }
        return nil
    }

    /// Whether `projectId` is in `projects` (ids compare case-insensitively,
    /// as GUIDs do on the server).
    public static func contains(_ projectId: String, in projects: [MyProject]) -> Bool {
        find(projectId, in: projects) != nil
    }

    private static func find(_ id: String?, in projects: [MyProject]) -> MyProject? {
        guard let id = id?.nonEmptyTrimmed else { return nil }
        return projects.first { $0.id.caseInsensitiveCompare(id) == .orderedSame }
    }
}

/// The resolved tracking session: the project, what may be captured, and its
/// timing policy (port of the Windows worker's `resolve_session`). Rebuilt
/// whenever the project list is refreshed; a new interval or policy applies
/// from the next block on.
public struct TrackingSession: Equatable, Sendable {
    public let project: MyProject
    /// Server policy ∧ local configuration.
    public let permissions: CapturePermissions
    /// Block interval in seconds (5...60 min).
    public let interval: TimeInterval
    /// Seconds without input before the user counts as away; 0 = off.
    public let idleThreshold: TimeInterval
    /// Screen captures must be blurred before upload: the project's policy
    /// or the local configuration (which can add blur, never remove it).
    public let blurScreenshots: Bool

    public init(project: MyProject, localPolicy: CapturePermissions, localBlur: Bool = false) {
        self.project = project
        self.permissions = project.allowedCaptures.intersection(localPolicy)
        self.interval = project.interval
        self.idleThreshold = project.idleThreshold
        self.blurScreenshots = project.blurScreenshots || localBlur
    }

    /// The session of `project` under `config`.
    public init(project: MyProject, config: Config) {
        self.init(project: project, localPolicy: config.capture, localBlur: config.blurScreenshots)
    }

    public var projectId: String { project.id }
}
