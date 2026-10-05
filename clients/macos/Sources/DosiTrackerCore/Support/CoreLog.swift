import Foundation
import os

/// Severity of a log line.
public enum LogLevel: String, Sendable {
    case debug, info, warning, error
}

/// Logging for the core: every line goes to the unified log
/// (`subsystem: com.dosibridge.tracker`), and to an optional sink the app sets
/// at launch to also write its rotating file in `~/Library/Logs/DosiTracker`.
///
/// Messages never contain secrets (passwords, tokens) or captured content.
public enum CoreLog {
    /// Receives (level, category, message) for every line.
    public typealias Sink = @Sendable (LogLevel, String, String) -> Void

    /// Install (or remove, with nil) the extra sink.
    public static func setSink(_ sink: Sink?) {
        SinkBox.shared.set(sink)
    }

    public static func log(_ level: LogLevel, _ category: String, _ message: String) {
        let logger = Logger(subsystem: AgentInfo.bundleIdentifier, category: category)
        switch level {
        case .debug:
            logger.debug("\(message, privacy: .public)")
        case .info:
            logger.info("\(message, privacy: .public)")
        case .warning:
            logger.warning("\(message, privacy: .public)")
        case .error:
            logger.error("\(message, privacy: .public)")
        }
        SinkBox.shared.get()?(level, category, message)
    }

    public static func debug(_ category: String, _ message: String) { log(.debug, category, message) }
    public static func info(_ category: String, _ message: String) { log(.info, category, message) }
    public static func warning(_ category: String, _ message: String) { log(.warning, category, message) }
    public static func error(_ category: String, _ message: String) { log(.error, category, message) }
}

/// The installed sink, behind a lock.
private final class SinkBox: @unchecked Sendable {
    static let shared = SinkBox()

    private let lock = NSLock()
    private var sink: CoreLog.Sink?

    func set(_ newSink: CoreLog.Sink?) {
        lock.lock()
        defer { lock.unlock() }
        sink = newSink
    }

    func get() -> CoreLog.Sink? {
        lock.lock()
        defer { lock.unlock() }
        return sink
    }
}
