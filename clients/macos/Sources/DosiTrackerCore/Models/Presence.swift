import Foundation

/// What the agent reports it is doing (`HeartbeatDto.status`, CONTRACT §3).
public enum PresenceStatus: String, Codable, Sendable, CaseIterable {
    case tracking
    case paused
    /// A timed break.
    case onBreak = "break"
    case idle
    case locked
    /// Signed in, but no project is chosen yet.
    case choosing
    case signedOut = "signedout"
    /// The app is quitting.
    case stopped
}

/// `POST /api/app/presence/heartbeat` body (`HeartbeatDto`, CONTRACT §3).
///
/// Sent every ``AgentTiming/heartbeatInterval`` while running and signed in,
/// and at once on every status change; `stopped` / `signedout` on quit or
/// sign-out are best effort (``AgentTiming/finalHeartbeatTimeout``). A
/// failed heartbeat is never queued. Nil values are left out of the JSON.
public struct Heartbeat: Codable, Equatable, Sendable {
    public var deviceId: String
    public var deviceName: String?
    public var platform: String?
    public var osVersion: String?
    public var agentVersion: String?
    public var status: PresenceStatus
    public var projectId: String?
    /// Since when the agent has been in `status`.
    public var statusSince: Date?
    /// The app in front; only when the project allows the active window.
    public var currentApp: String?
    public var timeZone: String?
    public var utcOffsetMinutes: Int?
    /// The device clock when sent (set by ``ApiClient`` at send time).
    public var sentAt: Date?

    public init(
        deviceId: String,
        deviceName: String? = nil,
        platform: String? = AgentInfo.platform,
        osVersion: String? = nil,
        agentVersion: String? = nil,
        status: PresenceStatus,
        projectId: String? = nil,
        statusSince: Date? = nil,
        currentApp: String? = nil,
        timeZone: String? = nil,
        utcOffsetMinutes: Int? = nil,
        sentAt: Date? = nil
    ) {
        self.deviceId = deviceId
        self.deviceName = deviceName
        self.platform = platform
        self.osVersion = osVersion
        self.agentVersion = agentVersion
        self.status = status
        self.projectId = projectId
        self.statusSince = statusSince
        self.currentApp = currentApp
        self.timeZone = timeZone
        self.utcOffsetMinutes = utcOffsetMinutes
        self.sentAt = sentAt
    }

    /// A heartbeat from this device, normalized. `currentApp` must already
    /// respect the project's active-window permission (nil when it does not
    /// allow it).
    public init(
        device: DeviceInfo,
        status: PresenceStatus,
        projectId: String?,
        statusSince: Date?,
        currentApp: String?,
        timeZone zone: TimeZone = .autoupdatingCurrent,
        now: Date = Date()
    ) {
        self.init(
            deviceId: device.deviceId.lowercased(),
            deviceName: device.deviceName,
            platform: AgentInfo.platform,
            osVersion: device.osVersion,
            agentVersion: device.agentVersion,
            status: status,
            projectId: projectId,
            statusSince: statusSince,
            currentApp: currentApp,
            timeZone: DeviceInfo.wireTimeZone(zone.identifier),
            utcOffsetMinutes: DeviceInfo.utcOffsetMinutes(zone, at: now),
            sentAt: nil
        )
        self = normalized()
    }

    /// The text fields trimmed and cut to the server's limits (device name
    /// 128, platform 16, OS 64, agent version 32, current app 256, time zone
    /// 64 characters), blank values dropped and the offset kept within
    /// -840...840, so a long computer or app name never turns a heartbeat
    /// into a validation error (port of the Windows `Heartbeat::normalized`).
    public func normalized() -> Heartbeat {
        var copy = self
        copy.deviceName = Heartbeat.fit(deviceName, 128)
        copy.platform = Heartbeat.fit(platform, 16)
        copy.osVersion = Heartbeat.fit(osVersion, 64)
        copy.agentVersion = Heartbeat.fit(agentVersion, 32)
        copy.currentApp = Heartbeat.fit(currentApp, 256)
        copy.timeZone = Heartbeat.fit(timeZone, 64)
        copy.utcOffsetMinutes = utcOffsetMinutes.map { min(max($0, -840), 840) }
        return copy
    }

    static func fit(_ value: String?, _ limit: Int) -> String? {
        guard let trimmed = value?.nonEmptyTrimmed else { return nil }
        return trimmed.truncatedUTF16(limit).nonEmptyTrimmed
    }
}

/// One device's live status (`PresenceDto`, CONTRACT §3), from
/// `GET /api/app/presence/team`: everyone's devices for callers with
/// `Tracker.Activities.ViewAll`, otherwise the caller's own — e.g. to warn
/// that tracking also runs on another computer (overlapping blocks are
/// refused). Read leniently: missing text is nil, an unreadable date is nil.
public struct PresenceInfo: Codable, Equatable, Sendable, Identifiable {
    public var userId: String
    public var userName: String
    public var name: String?
    public var deviceId: String
    public var deviceName: String?
    public var platform: String?
    public var osVersion: String?
    public var agentVersion: String?
    /// The status as sent (see ``presenceStatus``).
    public var status: String
    public var projectId: String?
    public var projectTitle: String?
    public var statusSince: Date?
    public var lastSeenAt: Date?
    /// Seen within the last 150 s; otherwise shown as "Offline · last seen …".
    public var online: Bool
    public var currentApp: String?
    public var timeZone: String?
    public var utcOffsetMinutes: Int?

    public init(userId: String, userName: String, name: String? = nil, deviceId: String, deviceName: String? = nil,
                platform: String? = nil, osVersion: String? = nil, agentVersion: String? = nil, status: String,
                projectId: String? = nil, projectTitle: String? = nil, statusSince: Date? = nil, lastSeenAt: Date? = nil,
                online: Bool, currentApp: String? = nil, timeZone: String? = nil, utcOffsetMinutes: Int? = nil) {
        self.userId = userId
        self.userName = userName
        self.name = name
        self.deviceId = deviceId
        self.deviceName = deviceName
        self.platform = platform
        self.osVersion = osVersion
        self.agentVersion = agentVersion
        self.status = status
        self.projectId = projectId
        self.projectTitle = projectTitle
        self.statusSince = statusSince
        self.lastSeenAt = lastSeenAt
        self.online = online
        self.currentApp = currentApp
        self.timeZone = timeZone
        self.utcOffsetMinutes = utcOffsetMinutes
    }

    /// One user's device.
    public var id: String { "\(userId)/\(deviceId)" }

    /// The status, when it is one this agent knows.
    public var presenceStatus: PresenceStatus? { PresenceStatus(rawValue: status) }

    /// Whether this is the device `deviceId` (ids compare case-insensitively,
    /// as GUIDs do on the server).
    public func isDevice(_ otherDeviceId: String) -> Bool {
        deviceId.caseInsensitiveCompare(otherDeviceId.trimmingCharacters(in: .whitespacesAndNewlines)) == .orderedSame
    }

    private enum CodingKeys: String, CodingKey {
        case userId, userName, name, deviceId, deviceName, platform, osVersion, agentVersion, status
        case projectId, projectTitle, statusSince, lastSeenAt, online, currentApp, timeZone, utcOffsetMinutes
    }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        userId = try c.decode(String.self, forKey: .userId)
        userName = try c.decodeIfPresent(String.self, forKey: .userName) ?? ""
        name = try c.decodeIfPresent(String.self, forKey: .name)
        deviceId = try c.decode(String.self, forKey: .deviceId)
        deviceName = try c.decodeIfPresent(String.self, forKey: .deviceName)
        platform = try c.decodeIfPresent(String.self, forKey: .platform)
        osVersion = try c.decodeIfPresent(String.self, forKey: .osVersion)
        agentVersion = try c.decodeIfPresent(String.self, forKey: .agentVersion)
        status = try c.decodeIfPresent(String.self, forKey: .status) ?? ""
        projectId = try c.decodeIfPresent(String.self, forKey: .projectId)
        projectTitle = try c.decodeIfPresent(String.self, forKey: .projectTitle)
        let since: String? = try? c.decodeIfPresent(String.self, forKey: .statusSince)
        statusSince = since.flatMap { WireDate.date(from: $0) }
        let seen: String? = try? c.decodeIfPresent(String.self, forKey: .lastSeenAt)
        lastSeenAt = seen.flatMap { WireDate.date(from: $0) }
        online = try c.decodeIfPresent(Bool.self, forKey: .online) ?? false
        currentApp = try c.decodeIfPresent(String.self, forKey: .currentApp)
        timeZone = try c.decodeIfPresent(String.self, forKey: .timeZone)
        utcOffsetMinutes = try c.decodeIfPresent(Int.self, forKey: .utcOffsetMinutes)
    }

    public func encode(to encoder: Encoder) throws {
        var c = encoder.container(keyedBy: CodingKeys.self)
        try c.encode(userId, forKey: .userId)
        try c.encode(userName, forKey: .userName)
        try c.encodeIfPresent(name, forKey: .name)
        try c.encode(deviceId, forKey: .deviceId)
        try c.encodeIfPresent(deviceName, forKey: .deviceName)
        try c.encodeIfPresent(platform, forKey: .platform)
        try c.encodeIfPresent(osVersion, forKey: .osVersion)
        try c.encodeIfPresent(agentVersion, forKey: .agentVersion)
        try c.encode(status, forKey: .status)
        try c.encodeIfPresent(projectId, forKey: .projectId)
        try c.encodeIfPresent(projectTitle, forKey: .projectTitle)
        try c.encodeIfPresent(statusSince.map { WireDate.string(from: $0) }, forKey: .statusSince)
        try c.encodeIfPresent(lastSeenAt.map { WireDate.string(from: $0) }, forKey: .lastSeenAt)
        try c.encode(online, forKey: .online)
        try c.encodeIfPresent(currentApp, forKey: .currentApp)
        try c.encodeIfPresent(timeZone, forKey: .timeZone)
        try c.encodeIfPresent(utcOffsetMinutes, forKey: .utcOffsetMinutes)
    }

    /// Decode a `ListResultDto<PresenceDto>` (`{"items":[…]}`), skipping
    /// rather than failing on an entry that cannot be read.
    public static func decodeList(from data: Data) throws -> [PresenceInfo] {
        try WireJSON.decoder().decode(ListResult.self, from: data).items.compactMap { $0.value }
    }

    private struct ListResult: Decodable {
        let items: [Lossy]
    }

    private struct Lossy: Decodable {
        let value: PresenceInfo?

        init(from decoder: Decoder) throws {
            value = try? PresenceInfo(from: decoder)
        }
    }
}

/// `HeartbeatResultDto`. The server time is read leniently: with or without
/// a zone (a bare one is UTC), and an unreadable value is nil rather than an
/// error, so the heartbeat still counts.
public struct HeartbeatResult: Codable, Equatable, Sendable {
    public var serverTime: Date?

    public init(serverTime: Date?) {
        self.serverTime = serverTime
    }

    private enum CodingKeys: String, CodingKey {
        case serverTime
    }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        let text: String? = try? c.decodeIfPresent(String.self, forKey: .serverTime)
        serverTime = text.flatMap { WireDate.date(from: $0) }
    }

    public func encode(to encoder: Encoder) throws {
        var c = encoder.container(keyedBy: CodingKeys.self)
        try c.encodeIfPresent(serverTime.map { WireDate.string(from: $0) }, forKey: .serverTime)
    }
}
