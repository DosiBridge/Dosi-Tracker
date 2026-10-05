import DosiTrackerCore
import Foundation

// The rotating log file in `~/Library/Logs/DosiTracker` (Console.app shows the
// folder). Every line the core and the app log also goes to the unified log
// (`CoreLog`, subsystem `com.dosibridge.tracker`); this file is what a user
// attaches to a support request, since an app started at login has no
// terminal. Lines never contain passwords, tokens or captured content.

final class FileLog: @unchecked Sendable {
    /// A file is rotated once it would grow past this.
    static let maxBytes: UInt64 = 2 * 1024 * 1024
    /// Rotated copies kept (`dosi-tracker.log.1` … `.4`), so the folder stays
    /// under ~10 MB.
    static let keptFiles = 4

    private let url: URL
    private let writesDebug: Bool
    private let queue = DispatchQueue(label: "com.dosibridge.tracker.file-log", qos: .utility)
    // Confined to `queue`.
    private var handle: FileHandle?
    private var size: UInt64 = 0

    /// - Parameters:
    ///   - url: the current log file (rotated copies sit next to it).
    ///   - writesDebug: also write debug lines (the `DosiLogDebug` preference).
    init(url: URL, writesDebug: Bool) {
        self.url = url
        self.writesDebug = writesDebug
    }

    /// Append one line (asynchronously; ``flush()`` waits for it).
    func write(_ level: LogLevel, _ category: String, _ message: String) {
        if level == .debug && !writesDebug {
            return
        }
        let line = "\(WireDate.string(from: Date())) [\(level.rawValue)] \(category): \(message)\n"
        queue.async {
            self.append(Data(line.utf8))
        }
    }

    /// Wait until every line written so far is on disk (at quit).
    func flush() {
        queue.sync {
            try? self.handle?.synchronize()
        }
    }

    // MARK: On the queue

    private func append(_ data: Data) {
        if handle == nil {
            open()
        }
        if size + UInt64(data.count) > FileLog.maxBytes {
            rotate()
        }
        guard let handle = handle else { return }
        do {
            try handle.write(contentsOf: data)
            size += UInt64(data.count)
        } catch {
            // The disk is full or the file went away: try a fresh file next time.
            try? handle.close()
            self.handle = nil
        }
    }

    private func open() {
        let fileManager = FileManager.default
        try? fileManager.createDirectory(at: url.deletingLastPathComponent(), withIntermediateDirectories: true,
                                         attributes: [.posixPermissions: 0o700])
        if !fileManager.fileExists(atPath: url.path) {
            _ = fileManager.createFile(atPath: url.path, contents: nil, attributes: [.posixPermissions: 0o600])
        }
        guard let opened = try? FileHandle(forWritingTo: url) else { return }
        size = (try? opened.seekToEnd()) ?? 0
        handle = opened
    }

    private func rotate() {
        try? handle?.close()
        handle = nil
        let fileManager = FileManager.default
        let path = url.path
        try? fileManager.removeItem(atPath: "\(path).\(FileLog.keptFiles)")
        if FileLog.keptFiles > 1 {
            for index in stride(from: FileLog.keptFiles - 1, through: 1, by: -1) {
                let from = "\(path).\(index)"
                if fileManager.fileExists(atPath: from) {
                    try? fileManager.moveItem(atPath: from, toPath: "\(path).\(index + 1)")
                }
            }
        }
        try? fileManager.moveItem(atPath: path, toPath: "\(path).1")
        open()
    }
}
