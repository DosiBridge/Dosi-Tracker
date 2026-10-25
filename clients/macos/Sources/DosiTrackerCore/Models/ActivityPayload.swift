import Foundation

/// Why a block ended (`endReason`, CONTRACT §1 and §4).
public enum EndReason: String, Codable, Sendable, CaseIterable {
    /// The planned interval elapsed (also used for a manual "Sync now" cut).
    case interval
    /// The user paused tracking.
    case pause
    /// The user took a timed break.
    case onBreak = "break"
    /// No input for the project's idle threshold; the block ends at the last input.
    case idle
    /// The screen locked.
    case lock
    /// The Mac went to sleep (or a missed sleep was detected).
    case sleep
    /// The user switched project.
    case project
    /// The user changed the task note.
    case task
    /// The user signed out.
    case signOut = "signout"
    /// The user quit the app.
    case quit
    /// The Mac is shutting down, restarting or logging out.
    case shutdown

    /// Whether a block ending this way takes its screenshot at the end when the
    /// planned capture moment had not come yet. Never for lock, sleep, idle or
    /// shutdown: the screen then shows nothing the user worked on.
    public var allowsCaptureAtEnd: Bool {
        switch self {
        case .lock, .sleep, .idle, .shutdown:
            return false
        case .interval, .pause, .onBreak, .project, .task, .signOut, .quit:
            return true
        }
    }

    /// Parse a wire name leniently (trimmed, any case); unknown values are nil,
    /// as on the server.
    public static func parse(_ value: String) -> EndReason? {
        let wanted = value.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
        return allCases.first { $0.rawValue == wanted }
    }
}

/// A window (app + title). For foreground usage it also carries how long it
/// was in front during the block and the input made while it was; running
/// programs leave those out and always have an empty title (CONTRACT §1).
public struct WindowInfo: Codable, Equatable, Sendable {
    public var appName: String
    public var windowTitle: String
    public var seconds: Int?
    public var keyboardHits: Int?
    public var mouseClicks: Int?

    public init(appName: String, windowTitle: String, seconds: Int? = nil, keyboardHits: Int? = nil, mouseClicks: Int? = nil) {
        self.appName = appName
        self.windowTitle = windowTitle
        self.seconds = seconds
        self.keyboardHits = keyboardHits
        self.mouseClicks = mouseClicks
    }

    private enum CodingKeys: String, CodingKey {
        case appName, windowTitle, seconds, keyboardHits, mouseClicks
    }

    public init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        appName = try container.decodeIfPresent(String.self, forKey: .appName) ?? ""
        windowTitle = try container.decodeIfPresent(String.self, forKey: .windowTitle) ?? ""
        seconds = try container.decodeIfPresent(Int.self, forKey: .seconds)
        keyboardHits = try container.decodeIfPresent(Int.self, forKey: .keyboardHits)
        mouseClicks = try container.decodeIfPresent(Int.self, forKey: .mouseClicks)
    }

    public func encode(to encoder: Encoder) throws {
        var container = encoder.container(keyedBy: CodingKeys.self)
        try container.encode(appName, forKey: .appName)
        try container.encode(windowTitle, forKey: .windowTitle)
        try container.encodeIfPresent(seconds, forKey: .seconds)
        try container.encodeIfPresent(keyboardHits, forKey: .keyboardHits)
        try container.encodeIfPresent(mouseClicks, forKey: .mouseClicks)
    }
}

/// One minute of a block (`ActivityMinuteDto`): input counts, whether there
/// was any input, the app mostly in front and how many seconds had input.
/// Buckets match the activity % minutes (see ``ActivityMath``).
///
/// A count is nil when that kind of input is not measured (the project or the
/// user's consent leaves it out, or the input capture was not working): it is
/// sent as `null`, never as a 0 that would read as "no input".
public struct ActivityMinute: Codable, Equatable, Sendable {
    /// 0-based minute of the block.
    public var minute: Int
    public var keyboardHits: Int?
    public var mouseClicks: Int?
    public var active: Bool
    public var appName: String?
    /// Seconds of this bucket with any input, 0...90 (the last bucket can
    /// run up to 89 s). Nil from rows queued by builds that did not send it.
    public var activeSeconds: Int?

    public init(minute: Int, keyboardHits: Int? = 0, mouseClicks: Int? = 0, active: Bool = false,
                appName: String? = nil, activeSeconds: Int? = nil) {
        self.minute = minute
        self.keyboardHits = keyboardHits
        self.mouseClicks = mouseClicks
        self.active = active
        self.appName = appName
        self.activeSeconds = activeSeconds
    }

    private enum CodingKeys: String, CodingKey {
        case minute, keyboardHits, mouseClicks, active, appName, activeSeconds
    }

    public init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        minute = try container.decodeIfPresent(Int.self, forKey: .minute) ?? 0
        // Absent (an older build's row) or null: not measured.
        keyboardHits = try container.decodeIfPresent(Int.self, forKey: .keyboardHits)
        mouseClicks = try container.decodeIfPresent(Int.self, forKey: .mouseClicks)
        active = try container.decodeIfPresent(Bool.self, forKey: .active) ?? false
        appName = try container.decodeIfPresent(String.self, forKey: .appName)
        activeSeconds = try container.decodeIfPresent(Int.self, forKey: .activeSeconds)
    }

    public func encode(to encoder: Encoder) throws {
        var container = encoder.container(keyedBy: CodingKeys.self)
        try container.encode(minute, forKey: .minute)
        try container.encodeCount(keyboardHits, forKey: .keyboardHits)
        try container.encodeCount(mouseClicks, forKey: .mouseClicks)
        try container.encode(active, forKey: .active)
        try container.encodeIfPresent(appName, forKey: .appName)
        try container.encodeIfPresent(activeSeconds, forKey: .activeSeconds)
    }
}

/// The capture format both agents use (CONTRACT §4) and the server's limits
/// on uploaded captures (decoded sizes, CONTRACT §1). A capture over its limit
/// would be dropped by the server anyway, so it is never attached. The values
/// match the Windows agent's `tracking/screenshot.rs` and `webcam.rs`.
public enum CaptureLimits {
    /// Screen capture (JPEG preferred, PNG accepted).
    public static let maxScreenBytes = 8 * 1024 * 1024
    /// The list thumbnail of the screen capture.
    public static let maxThumbnailBytes = 512 * 1024
    /// Webcam photo.
    public static let maxWebcamBytes = 2 * 1024 * 1024
    /// Screen captures are scaled to at most this width (aspect kept).
    public static let maxScreenWidth = 1920
    /// Thumbnails are scaled to at most this width (aspect kept).
    public static let maxThumbnailWidth = 480
    /// Webcam photos are scaled to at most this width (aspect kept).
    public static let maxWebcamWidth = 1280
    /// JPEG quality of screen captures (0...1).
    public static let screenJpegQuality = 0.8
    /// JPEG quality of thumbnails and webcam photos (0...1).
    public static let thumbnailJpegQuality = 0.7
    public static let webcamJpegQuality = 0.7
    /// Qualities tried, in order, when an image does not fit its limit.
    public static let fallbackJpegQualities: [Double] = [0.6, 0.45, 0.3]
    /// A required blur averages blocks this many pixels wide (wider than two
    /// characters of normal UI text, so no letter survives).
    public static let blurCellSize = 24
    /// Webcam frames delivered during this warm-up are discarded (auto-exposure).
    public static let webcamWarmUp: TimeInterval = 1
    /// A webcam that has not delivered a usable frame by then is given up on.
    public static let webcamTimeout: TimeInterval = 5

    /// The size of an image `width` × `height` scaled to at most `maxWidth`
    /// wide, aspect kept (rounded, at least 1 × 1). Never upscales.
    public static func scaledSize(width: Int, height: Int, maxWidth: Int) -> (width: Int, height: Int) {
        guard width > 0, height > 0 else { return (max(width, 0), max(height, 0)) }
        guard width > maxWidth, maxWidth > 0 else { return (width, height) }
        let scaledHeight = (Double(height) * Double(maxWidth) / Double(width)).rounded()
        return (maxWidth, max(Int(scaledHeight), 1))
    }
}

/// One tracked block, as uploaded to `POST /api/app/activity`
/// (`CreateActivityDto`, CONTRACT §1).
///
/// Keys are the exact camelCase names the backend binds. Optional values that
/// are nil are left out of the JSON rather than sent as `null`: an older
/// server whose `Productivity` is a non-nullable `int` then simply keeps its
/// default instead of failing the whole request, and every new field stays
/// invisible to it. The input counts are the exception (cross-agent
/// contract): `mouseClicks` and `keyboardHits` are always sent, as `null`
/// when that kind of input is not measured and 0 only when it was measured
/// as none.
public struct ActivityPayload: Codable, Equatable, Sendable {
    public var projectId: String
    /// Client-generated idempotency key (lower-case UUID).
    public var clientActivityId: String
    public var startedAt: Date
    public var endedAt: Date
    /// Activity % (0-100); nil = not measured (input capture unavailable, or
    /// the project allows neither keyboard nor mouse).
    public var productivity: Int?
    /// Clicks in the block; nil = not measured (the project or the user's
    /// consent leaves the mouse out, or the input capture was not working).
    public var mouseClicks: Int?
    /// Key presses in the block; nil = not measured, as for ``mouseClicks``.
    public var keyboardHits: Int?
    /// The task note ("What are you working on?").
    public var description: String?
    /// Foreground usage over the block, one entry per (app, window title),
    /// most-used first.
    public var activeWindows: [WindowInfo]
    /// Apps running during the block, titles always empty.
    public var runningPrograms: [WindowInfo]
    /// Per-minute breakdown of the block.
    public var timeline: [ActivityMinute]
    /// Base64 screen capture: JPEG (preferred) or PNG, despite the name.
    public var screenshotPngBase64: String?
    /// Base64 JPEG thumbnail (at most 480 px wide) of the same capture.
    public var screenshotThumbJpgBase64: String?
    /// Base64 JPEG webcam photo.
    public var webcamJpgBase64: String?

    // Device and integrity context (CONTRACT §1, all optional).
    public var platform: String?
    public var osVersion: String?
    public var agentVersion: String?
    /// Lower-case UUID of this install; only ever a valid UUID (the server
    /// binds a `Guid?`, so anything else would fail the whole upload).
    public var deviceId: String?
    public var deviceName: String?
    /// IANA time zone of the device, e.g. `Asia/Dhaka`.
    public var timeZone: String?
    /// The device's UTC offset at `startedAt`, in minutes (+360 for Dhaka).
    public var utcOffsetMinutes: Int?
    /// Seconds of the block with no input, at minute granularity.
    public var idleSeconds: Int?
    public var endReason: EndReason?
    /// The device clock when the upload was sent (set by ``ApiClient`` at
    /// send time, never stored in the queue).
    public var sentAt: Date?
    /// When the screen capture was taken.
    public var screenshotCapturedAt: Date?
    /// True when the screen capture and thumbnail were blurred before upload.
    public var screenshotBlurred: Bool?
    /// Software-injected keyboard/mouse events seen during the block; they are
    /// excluded from the counts and the activity %.
    public var injectedInputEvents: Int?

    public init(
        projectId: String,
        clientActivityId: String = ActivityPayload.newClientActivityId(),
        startedAt: Date,
        endedAt: Date,
        productivity: Int? = nil,
        mouseClicks: Int? = 0,
        keyboardHits: Int? = 0,
        description: String? = nil,
        activeWindows: [WindowInfo] = [],
        runningPrograms: [WindowInfo] = [],
        timeline: [ActivityMinute] = []
    ) {
        self.projectId = projectId
        self.clientActivityId = clientActivityId
        self.startedAt = startedAt
        self.endedAt = endedAt
        self.productivity = productivity
        self.mouseClicks = mouseClicks
        self.keyboardHits = keyboardHits
        self.description = description
        self.activeWindows = activeWindows
        self.runningPrograms = runningPrograms
        self.timeline = timeline
    }

    /// A fresh idempotency key: a random UUID in lower case.
    public static func newClientActivityId() -> String {
        UUID().uuidString.lowercased()
    }

    /// The block's length.
    public var duration: TimeInterval { endedAt.timeIntervalSince(startedAt) }

    /// Whether any image travels with the block.
    public var hasCaptures: Bool {
        screenshotPngBase64 != nil || screenshotThumbJpgBase64 != nil || webcamJpgBase64 != nil
    }

    /// The bytes of base64 image data the block carries (what the offline
    /// queue budgets).
    public var captureBytes: Int64 {
        [screenshotPngBase64, screenshotThumbJpgBase64, webcamJpgBase64]
            .compactMap { $0 }
            .reduce(Int64(0)) { $0 + Int64($1.utf8.count) }
    }

    /// Attach the screen capture (JPEG or PNG bytes) and its thumbnail taken
    /// at `capturedAt`. A capture over ``CaptureLimits/maxScreenBytes`` is not
    /// attached (nor its thumbnail); a thumbnail over its own limit is left
    /// out alone. Returns whether the screen capture was attached.
    @discardableResult
    public mutating func attachScreen(image: Data, thumbnail: Data?, capturedAt: Date, blurred: Bool) -> Bool {
        guard !image.isEmpty, image.count <= CaptureLimits.maxScreenBytes else { return false }
        screenshotPngBase64 = image.base64EncodedString()
        if let thumbnail = thumbnail, !thumbnail.isEmpty, thumbnail.count <= CaptureLimits.maxThumbnailBytes {
            screenshotThumbJpgBase64 = thumbnail.base64EncodedString()
        } else {
            screenshotThumbJpgBase64 = nil
        }
        screenshotCapturedAt = capturedAt
        screenshotBlurred = blurred
        return true
    }

    /// Attach a webcam JPEG; one over ``CaptureLimits/maxWebcamBytes`` is not
    /// attached. Returns whether it was.
    @discardableResult
    public mutating func attachWebcam(jpeg: Data) -> Bool {
        guard !jpeg.isEmpty, jpeg.count <= CaptureLimits.maxWebcamBytes else { return false }
        webcamJpgBase64 = jpeg.base64EncodedString()
        return true
    }

    /// The same block without any image (what a `413` / `CaptureTooLarge`
    /// resend and a parked row keep).
    public func strippingCaptures() -> ActivityPayload {
        var copy = self
        copy.screenshotPngBase64 = nil
        copy.screenshotThumbJpgBase64 = nil
        copy.webcamJpgBase64 = nil
        copy.screenshotCapturedAt = nil
        copy.screenshotBlurred = nil
        return copy
    }

    /// Stamp the device context (CONTRACT §1): platform, OS, agent version,
    /// device id and name, and the time zone with the UTC offset at the
    /// block's start. Text is cut to the server's lengths; a device id that is
    /// not a UUID and a zone id longer than 64 characters are left out.
    public mutating func applyDeviceContext(_ device: DeviceInfo, zone: BlockZone) {
        platform = AgentInfo.platform
        osVersion = device.osVersion.nonEmptyTrimmed?.truncatedUTF16(64)
        agentVersion = device.agentVersion.nonEmptyTrimmed?.truncatedUTF16(32)
        deviceId = DeviceInfo.normalizedDeviceId(device.deviceId)
        deviceName = device.deviceName.nonEmptyTrimmed?.truncatedUTF16(128)
        timeZone = zone.timeZone.flatMap { DeviceInfo.wireTimeZone($0) }
        utcOffsetMinutes = min(max(zone.utcOffsetMinutes, -840), 840)
    }

    // MARK: Codable

    private enum CodingKeys: String, CodingKey {
        case projectId, clientActivityId, startedAt, endedAt, productivity, mouseClicks, keyboardHits
        case description, activeWindows, runningPrograms, timeline
        case screenshotPngBase64, screenshotThumbJpgBase64, webcamJpgBase64
        case platform, osVersion, agentVersion, deviceId, deviceName, timeZone, utcOffsetMinutes
        case idleSeconds, endReason, sentAt, screenshotCapturedAt, screenshotBlurred, injectedInputEvents
    }

    /// Lenient: rows queued by older builds (no productivity, timeline,
    /// thumbnail or device context) still load, and an unknown end reason
    /// (queued by a newer build) reads as nil instead of making the row
    /// unreadable.
    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        projectId = try c.decode(String.self, forKey: .projectId)
        clientActivityId = try c.decode(String.self, forKey: .clientActivityId)
        startedAt = try c.decode(Date.self, forKey: .startedAt)
        endedAt = try c.decode(Date.self, forKey: .endedAt)
        productivity = try c.decodeIfPresent(Int.self, forKey: .productivity)
        // Null: not measured. Absent only in rows of builds that always
        // measured (they sent 0 for nothing), so it reads as 0.
        mouseClicks = try c.contains(.mouseClicks) ? c.decodeIfPresent(Int.self, forKey: .mouseClicks) : 0
        keyboardHits = try c.contains(.keyboardHits) ? c.decodeIfPresent(Int.self, forKey: .keyboardHits) : 0
        description = try c.decodeIfPresent(String.self, forKey: .description)
        activeWindows = try c.decodeIfPresent([WindowInfo].self, forKey: .activeWindows) ?? []
        runningPrograms = try c.decodeIfPresent([WindowInfo].self, forKey: .runningPrograms) ?? []
        timeline = try c.decodeIfPresent([ActivityMinute].self, forKey: .timeline) ?? []
        screenshotPngBase64 = try c.decodeIfPresent(String.self, forKey: .screenshotPngBase64)
        screenshotThumbJpgBase64 = try c.decodeIfPresent(String.self, forKey: .screenshotThumbJpgBase64)
        webcamJpgBase64 = try c.decodeIfPresent(String.self, forKey: .webcamJpgBase64)
        platform = try c.decodeIfPresent(String.self, forKey: .platform)
        osVersion = try c.decodeIfPresent(String.self, forKey: .osVersion)
        agentVersion = try c.decodeIfPresent(String.self, forKey: .agentVersion)
        deviceId = try c.decodeIfPresent(String.self, forKey: .deviceId)
        deviceName = try c.decodeIfPresent(String.self, forKey: .deviceName)
        timeZone = try c.decodeIfPresent(String.self, forKey: .timeZone)
        utcOffsetMinutes = try c.decodeIfPresent(Int.self, forKey: .utcOffsetMinutes)
        idleSeconds = try c.decodeIfPresent(Int.self, forKey: .idleSeconds)
        // A value of the wrong type reads as unknown too.
        let reason: String? = try? c.decodeIfPresent(String.self, forKey: .endReason)
        endReason = reason.flatMap { EndReason.parse($0) }
        sentAt = try c.decodeIfPresent(Date.self, forKey: .sentAt)
        screenshotCapturedAt = try c.decodeIfPresent(Date.self, forKey: .screenshotCapturedAt)
        screenshotBlurred = try c.decodeIfPresent(Bool.self, forKey: .screenshotBlurred)
        injectedInputEvents = try c.decodeIfPresent(Int.self, forKey: .injectedInputEvents)
    }

    public func encode(to encoder: Encoder) throws {
        var c = encoder.container(keyedBy: CodingKeys.self)
        try c.encode(projectId, forKey: .projectId)
        try c.encode(clientActivityId, forKey: .clientActivityId)
        try c.encode(startedAt, forKey: .startedAt)
        try c.encode(endedAt, forKey: .endedAt)
        try c.encodeIfPresent(productivity, forKey: .productivity)
        try c.encodeCount(mouseClicks, forKey: .mouseClicks)
        try c.encodeCount(keyboardHits, forKey: .keyboardHits)
        try c.encodeIfPresent(description, forKey: .description)
        try c.encode(activeWindows, forKey: .activeWindows)
        try c.encode(runningPrograms, forKey: .runningPrograms)
        try c.encode(timeline, forKey: .timeline)
        try c.encodeIfPresent(screenshotPngBase64, forKey: .screenshotPngBase64)
        try c.encodeIfPresent(screenshotThumbJpgBase64, forKey: .screenshotThumbJpgBase64)
        try c.encodeIfPresent(webcamJpgBase64, forKey: .webcamJpgBase64)
        try c.encodeIfPresent(platform, forKey: .platform)
        try c.encodeIfPresent(osVersion, forKey: .osVersion)
        try c.encodeIfPresent(agentVersion, forKey: .agentVersion)
        try c.encodeIfPresent(deviceId, forKey: .deviceId)
        try c.encodeIfPresent(deviceName, forKey: .deviceName)
        try c.encodeIfPresent(timeZone, forKey: .timeZone)
        try c.encodeIfPresent(utcOffsetMinutes, forKey: .utcOffsetMinutes)
        try c.encodeIfPresent(idleSeconds, forKey: .idleSeconds)
        try c.encodeIfPresent(endReason?.rawValue, forKey: .endReason)
        try c.encodeIfPresent(sentAt, forKey: .sentAt)
        try c.encodeIfPresent(screenshotCapturedAt, forKey: .screenshotCapturedAt)
        try c.encodeIfPresent(screenshotBlurred, forKey: .screenshotBlurred)
        try c.encodeIfPresent(injectedInputEvents, forKey: .injectedInputEvents)
    }
}

extension KeyedEncodingContainer {
    /// An input count as the contract wants it: the number, or an explicit
    /// `null` when it was not measured (never left out, never a made-up 0).
    mutating func encodeCount(_ count: Int?, forKey key: Key) throws {
        if let count = count {
            try encode(count, forKey: key)
        } else {
            try encodeNil(forKey: key)
        }
    }
}
