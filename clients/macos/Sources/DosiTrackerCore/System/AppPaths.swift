import Foundation

/// Where the agent keeps its files: never the working directory, which is `/`
/// for an app started at login.
///
/// * `~/Library/Application Support/DosiTracker`: the offline queue and the
///   single-instance lock (preferences live in UserDefaults, the sign-in in
///   the Keychain).
/// * `~/Library/Logs/DosiTracker`: the rotating log files (Console.app shows
///   this folder).
public struct AppPaths: Equatable, Sendable {
    public static let folderName = "DosiTracker"

    public let applicationSupport: URL
    public let logs: URL

    public init(applicationSupport: URL, logs: URL) {
        self.applicationSupport = applicationSupport
        self.logs = logs
    }

    /// The per-user locations.
    public static func standard(fileManager: FileManager = .default) throws -> AppPaths {
        let support = try fileManager.url(for: .applicationSupportDirectory, in: .userDomainMask,
                                          appropriateFor: nil, create: true)
        let library = try fileManager.url(for: .libraryDirectory, in: .userDomainMask,
                                          appropriateFor: nil, create: true)
        return AppPaths(
            applicationSupport: support.appendingPathComponent(folderName, isDirectory: true),
            logs: library.appendingPathComponent("Logs", isDirectory: true)
                .appendingPathComponent(folderName, isDirectory: true)
        )
    }

    /// Both folders under `root` (tests, previews).
    public static func rooted(at root: URL) -> AppPaths {
        AppPaths(
            applicationSupport: root.appendingPathComponent("Application Support", isDirectory: true)
                .appendingPathComponent(folderName, isDirectory: true),
            logs: root.appendingPathComponent("Logs", isDirectory: true)
                .appendingPathComponent(folderName, isDirectory: true)
        )
    }

    /// The offline queue.
    public var database: URL { applicationSupport.appendingPathComponent(Queue.fileName, isDirectory: false) }

    /// The lock file a second copy of the app checks (flock) before starting.
    public var lockFile: URL { applicationSupport.appendingPathComponent("dosi-tracker.lock", isDirectory: false) }

    /// The current log file; rotated copies sit next to it.
    public var logFile: URL { logs.appendingPathComponent("dosi-tracker.log", isDirectory: false) }

    /// The affirmative local consent state file.
    public var consentFile: URL { applicationSupport.appendingPathComponent("consent.json", isDirectory: false) }

    /// Create both folders (user-only access).
    public func createDirectories(fileManager: FileManager = .default) throws {
        let attributes: [FileAttributeKey: Any] = [.posixPermissions: 0o700]
        try fileManager.createDirectory(at: applicationSupport, withIntermediateDirectories: true, attributes: attributes)
        try fileManager.createDirectory(at: logs, withIntermediateDirectories: true, attributes: attributes)
    }
}
