import Foundation

/// Facts about this Mac and this install that travel with blocks and
/// heartbeats (CONTRACT §1, §3), read with Foundation only. The parts that do
/// not change while the agent runs are collected once; the time zone is read
/// per block instead (it follows the user when they travel).
public struct DeviceInfo: Equatable, Sendable {
    /// Random id generated once per install (``Preferences/deviceId()``).
    public var deviceId: String
    /// The computer's name.
    public var deviceName: String
    /// e.g. `macOS 14.5 (23F79)`.
    public var osVersion: String
    /// e.g. `0.2.0`.
    public var agentVersion: String
    /// Always `macos`.
    public var platform: String { AgentInfo.platform }

    public init(deviceId: String, deviceName: String, osVersion: String, agentVersion: String) {
        self.deviceId = deviceId
        self.deviceName = deviceName
        self.osVersion = osVersion
        self.agentVersion = agentVersion
    }

    /// This Mac. `deviceName` defaults to the host name; the app may pass the
    /// friendlier "Computer Name" from System Settings instead.
    public static func current(deviceId: String, deviceName: String? = nil, bundle: Bundle = .main) -> DeviceInfo {
        DeviceInfo(
            deviceId: deviceId,
            deviceName: deviceName?.nonEmptyTrimmed ?? hostName(),
            osVersion: currentOSVersion(),
            agentVersion: AgentInfo.version(bundle: bundle)
        )
    }

    /// The running macOS, e.g. `macOS 14.5 (23F79)`.
    public static func currentOSVersion() -> String {
        let info = ProcessInfo.processInfo
        return osVersionString(versionString: info.operatingSystemVersionString, version: info.operatingSystemVersion)
    }

    /// `macOS <major>.<minor>[.<patch>] (<build>)` from
    /// `ProcessInfo.operatingSystemVersionString` ("Version 14.5 (Build 23F79)")
    /// and the numeric version; at most 64 characters (the server's limit).
    public static func osVersionString(versionString: String, version: OperatingSystemVersion) -> String {
        var number = "\(version.majorVersion).\(version.minorVersion)"
        if version.patchVersion > 0 {
            number += ".\(version.patchVersion)"
        }
        var text = "macOS \(number)"
        if let marker = versionString.range(of: "Build ") {
            let rest = versionString[marker.upperBound...]
            let build = rest.prefix { $0 != ")" }.trimmingCharacters(in: .whitespaces)
            if !build.isEmpty {
                text += " (\(build))"
            }
        }
        return text.truncatedUTF16(64)
    }

    /// The host name without a `.local` suffix (never a blocking DNS lookup).
    public static func hostName() -> String {
        var buffer = [CChar](repeating: 0, count: 256)
        guard gethostname(&buffer, buffer.count) == 0 else { return "Mac" }
        buffer[buffer.count - 1] = 0
        let raw = buffer.withUnsafeBufferPointer { pointer -> String in
            guard let base = pointer.baseAddress else { return "" }
            return String(cString: base)
        }
        return cleanHostName(raw)
    }

    /// A host name as shown: without a `.local` suffix, "Mac" when blank.
    static func cleanHostName(_ raw: String) -> String {
        var name = raw.trimmingCharacters(in: .whitespacesAndNewlines)
        if name.lowercased().hasSuffix(".local") {
            name = String(name.dropLast(6))
        }
        return name.nonEmptyTrimmed ?? "Mac"
    }

    /// `deviceId` as sent: a valid UUID in lower case, nil otherwise (the
    /// server binds a `Guid?`; anything else would fail the whole upload).
    public static func normalizedDeviceId(_ value: String) -> String? {
        guard let uuid = UUID(uuidString: value.trimmingCharacters(in: .whitespacesAndNewlines)) else { return nil }
        return uuid.uuidString.lowercased()
    }

    /// An IANA zone id as sent: trimmed; nil when blank or longer than the
    /// server's 64 characters (a cut-off id would name no zone).
    public static func wireTimeZone(_ identifier: String) -> String? {
        guard let zone = identifier.nonEmptyTrimmed, zone.utf16.count <= 64 else { return nil }
        return zone
    }

    /// The UTC offset of `zone` at `date`, in minutes (it follows daylight
    /// saving time), within the server's -840...840.
    public static func utcOffsetMinutes(_ zone: TimeZone, at date: Date) -> Int {
        let minutes = zone.secondsFromGMT(for: date) / 60
        return min(max(minutes, -840), 840)
    }
}
