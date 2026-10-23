import Foundation
import Darwin

/// Granular capability permissions consented to by the local user.
/// Strict default-deny: all capabilities default to false.
public struct ConsentedPermissions: Codable, Equatable, Sendable {
    public var screenshot: Bool
    public var webcam: Bool
    public var keyboard: Bool
    public var mouse: Bool
    public var activeWindow: Bool
    public var runningPrograms: Bool
    public var blurScreenshots: Bool

    public init(
        screenshot: Bool = false,
        webcam: Bool = false,
        keyboard: Bool = false,
        mouse: Bool = false,
        activeWindow: Bool = false,
        runningPrograms: Bool = false,
        blurScreenshots: Bool = true
    ) {
        self.screenshot = screenshot
        self.webcam = webcam
        self.keyboard = keyboard
        self.mouse = mouse
        self.activeWindow = activeWindow
        self.runningPrograms = runningPrograms
        self.blurScreenshots = blurScreenshots
    }

    public static let allDisabled = ConsentedPermissions(
        screenshot: false,
        webcam: false,
        keyboard: false,
        mouse: false,
        activeWindow: false,
        runningPrograms: false,
        blurScreenshots: true
    )

    public var hasAnyActiveCapability: Bool {
        screenshot || webcam || keyboard || mouse || activeWindow || runningPrograms
    }

    /// Calculate strict intersection between local consent and a project/server policy.
    /// Invariant: A capability is admitted ONLY if both user and project grant it.
    public func intersect(_ other: CapturePermissions) -> ConsentedPermissions {
        ConsentedPermissions(
            screenshot: screenshot && other.screenshot,
            webcam: webcam && other.webcam,
            keyboard: keyboard && other.keyboard,
            mouse: mouse && other.mouse,
            activeWindow: activeWindow && other.activeWindow,
            runningPrograms: runningPrograms && other.runningPrograms,
            // Blur is enforced if either party requests it (defense-in-depth)
            blurScreenshots: blurScreenshots
        )
    }

    /// Calculate strict intersection between two local consent grants.
    public func intersect(_ other: ConsentedPermissions) -> ConsentedPermissions {
        ConsentedPermissions(
            screenshot: screenshot && other.screenshot,
            webcam: webcam && other.webcam,
            keyboard: keyboard && other.keyboard,
            mouse: mouse && other.mouse,
            activeWindow: activeWindow && other.activeWindow,
            runningPrograms: runningPrograms && other.runningPrograms,
            blurScreenshots: blurScreenshots || other.blurScreenshots
        )
    }

    /// Returns true if `self` contains any withdrawal relative to `previous`,
    /// or if blur was tightened from false to true.
    public func isNarrowerThan(_ previous: ConsentedPermissions) -> Bool {
        (previous.screenshot && !screenshot)
            || (previous.webcam && !webcam)
            || (previous.keyboard && !keyboard)
            || (previous.mouse && !mouse)
            || (previous.activeWindow && !activeWindow)
            || (previous.runningPrograms && !runningPrograms)
            || (!previous.blurScreenshots && blurScreenshots)
    }

    public func toCapturePermissions() -> CapturePermissions {
        CapturePermissions(
            screenshot: screenshot,
            webcam: webcam,
            keyboard: keyboard,
            mouse: mouse,
            activeWindow: activeWindow,
            runningPrograms: runningPrograms
        )
    }

    private enum CodingKeys: String, CodingKey {
        case screenshot, webcam, keyboard, mouse, activeWindow, runningPrograms, blurScreenshots
    }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        screenshot = try c.decodeIfPresent(Bool.self, forKey: .screenshot) ?? false
        webcam = try c.decodeIfPresent(Bool.self, forKey: .webcam) ?? false
        keyboard = try c.decodeIfPresent(Bool.self, forKey: .keyboard) ?? false
        mouse = try c.decodeIfPresent(Bool.self, forKey: .mouse) ?? false
        activeWindow = try c.decodeIfPresent(Bool.self, forKey: .activeWindow) ?? false
        runningPrograms = try c.decodeIfPresent(Bool.self, forKey: .runningPrograms) ?? false
        blurScreenshots = try c.decodeIfPresent(Bool.self, forKey: .blurScreenshots) ?? true
    }

    public func encode(to encoder: Encoder) throws {
        var c = encoder.container(keyedBy: CodingKeys.self)
        try c.encode(screenshot, forKey: .screenshot)
        try c.encode(webcam, forKey: .webcam)
        try c.encode(keyboard, forKey: .keyboard)
        try c.encode(mouse, forKey: .mouse)
        try c.encode(activeWindow, forKey: .activeWindow)
        try c.encode(runningPrograms, forKey: .runningPrograms)
        try c.encode(blurScreenshots, forKey: .blurScreenshots)
    }
}

/// Explicit local user consent state on this Mac.
public enum LocalConsent: Codable, Equatable, Sendable {
    /// No consent choice has been made. Fails closed (all captures blocked).
    case absent
    /// User explicitly consented to the specified capabilities.
    case granted(ConsentedPermissions)
    /// User explicitly revoked or declined tracking consent.
    case revoked

    public var isGranted: Bool {
        if case .granted = self { return true }
        return false
    }

    public var isRevoked: Bool {
        if case .revoked = self { return true }
        return false
    }

    public var permissions: ConsentedPermissions {
        switch self {
        case .granted(let p):
            return p
        case .absent, .revoked:
            return .allDisabled
        }
    }

    private enum CodingKeys: String, CodingKey {
        case state, permissions
    }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        let state = try c.decode(String.self, forKey: .state)
        switch state {
        case "absent":
            self = .absent
        case "granted":
            let perms = try c.decodeIfPresent(ConsentedPermissions.self, forKey: .permissions) ?? .allDisabled
            self = .granted(perms)
        case "revoked":
            self = .revoked
        default:
            throw DecodingError.dataCorruptedError(forKey: .state, in: c, debugDescription: "Unknown consent state: \(state)")
        }
    }

    public func encode(to encoder: Encoder) throws {
        var c = encoder.container(keyedBy: CodingKeys.self)
        switch self {
        case .absent:
            try c.encode("absent", forKey: .state)
        case .granted(let perms):
            try c.encode("granted", forKey: .state)
            try c.encode(perms, forKey: .permissions)
        case .revoked:
            try c.encode("revoked", forKey: .state)
        }
    }
}

/// Consent persistence and validation errors.
public enum ConsentError: Error, Equatable, Sendable, CustomStringConvertible {
    case ioError(String)
    case malformedJson(String)
    case saveFailed(String)
    case unsafeStorage(String)

    public var description: String {
        switch self {
        case .ioError(let s): return "Consent I/O error: \(s)"
        case .malformedJson(let s): return "Malformed consent JSON: \(s)"
        case .saveFailed(let s): return "Consent save failed: \(s)"
        case .unsafeStorage(let s): return "Unsafe consent storage: \(s)"
        }
    }
}

/// Abstract storage interface for consent persistence.
public protocol ConsentStorage: AnyObject, Sendable {
    func load() throws -> LocalConsent
    func save(_ consent: LocalConsent) throws
}

/// Internal hooks for injecting deterministic collision and failure behaviors in tests.
public struct SaveHooks: @unchecked Sendable {
    public var candidateNamer: (@Sendable (_ fileStem: String, _ attempt: Int) -> String)?
    public var failAfterOpen: (@Sendable (_ tempPath: String) throws -> Void)?
    public var failRename: (@Sendable (_ tempPath: String, _ targetPath: String) throws -> Void)?

    public init(
        candidateNamer: (@Sendable (_ fileStem: String, _ attempt: Int) -> String)? = nil,
        failAfterOpen: (@Sendable (_ tempPath: String) throws -> Void)? = nil,
        failRename: (@Sendable (_ tempPath: String, _ targetPath: String) throws -> Void)? = nil
    ) {
        self.candidateNamer = candidateNamer
        self.failAfterOpen = failAfterOpen
        self.failRename = failRename
    }
}

/// File-backed atomic consent store with private, exclusive atomic persistence.
public final class FileConsentStore: ConsentStorage, @unchecked Sendable {
    public let fileURL: URL
    private let lock = NSLock()
    internal var hooks: SaveHooks?

    public init(fileURL: URL, hooks: SaveHooks? = nil) {
        self.fileURL = fileURL
        self.hooks = hooks
    }

    /// Shared non-following immediate-parent directory validator.
    /// Checks that parent path is a regular directory (not a symlink),
    /// owned by the current effective UID, and not group- or world-writable.
    /// If `allowMissing` is true and path does not exist (ENOENT), returns false.
    @discardableResult
    private static func validateParentDirectory(atPath path: String, allowMissing: Bool = false) throws -> Bool {
        var st = stat()
        if lstat(path, &st) != 0 {
            if allowMissing && errno == ENOENT {
                return false
            }
            let err = String(cString: strerror(errno))
            throw ConsentError.ioError("Failed inspecting parent directory metadata for \(path): \(err)")
        }

        let mode = Int32(st.st_mode)
        let fileType = mode & Int32(S_IFMT)
        if fileType == Int32(S_IFLNK) {
            throw ConsentError.unsafeStorage("Parent directory \(path) is a symbolic link")
        }
        if fileType != Int32(S_IFDIR) {
            throw ConsentError.unsafeStorage("Parent path \(path) is not a directory")
        }

        let currentUid = geteuid()
        if st.st_uid != currentUid {
            throw ConsentError.unsafeStorage("Parent directory \(path) is owned by UID \(st.st_uid), expected current UID \(currentUid)")
        }

        if (mode & Int32(S_IWGRP | S_IWOTH)) != 0 {
            let octal = String(format: "%o", mode & 0o777)
            throw ConsentError.unsafeStorage("Parent directory \(path) has unsafe permissions \(octal) (group or world writable)")
        }

        return true
    }

    /// Check if target file exists and is a regular, non-symlink file under a safe immediate-parent directory.
    public static func exists(at fileURL: URL) -> Bool {
        let parentDir = fileURL.deletingLastPathComponent()
        let parentPath = parentDir.path
        if !parentPath.isEmpty {
            do {
                let parentExists = try validateParentDirectory(atPath: parentPath, allowMissing: true)
                if !parentExists {
                    return false
                }
            } catch {
                return false
            }
        }
        var st = stat()
        if lstat(fileURL.path, &st) != 0 {
            return false
        }
        let mode = Int32(st.st_mode)
        let fileType = mode & Int32(S_IFMT)
        if fileType != Int32(S_IFREG) {
            return false
        }
        if st.st_uid != geteuid() {
            return false
        }
        if (mode & Int32(S_IWGRP | S_IWOTH)) != 0 {
            return false
        }
        return true
    }

    private static func validateTargetFile(atPath path: String) throws {
        var st = stat()
        if lstat(path, &st) != 0 {
            if errno == ENOENT {
                return
            }
            let err = String(cString: strerror(errno))
            throw ConsentError.ioError("Failed inspecting metadata for \(path): \(err)")
        }

        let mode = Int32(st.st_mode)
        let fileType = mode & Int32(S_IFMT)
        if fileType == Int32(S_IFLNK) {
            throw ConsentError.unsafeStorage("Target path \(path) is a symbolic link")
        }
        if fileType != Int32(S_IFREG) {
            throw ConsentError.unsafeStorage("Target path \(path) is not a regular file")
        }

        let currentUid = geteuid()
        if st.st_uid != currentUid {
            throw ConsentError.unsafeStorage("Target path \(path) is owned by UID \(st.st_uid), expected current UID \(currentUid)")
        }

        if (mode & Int32(S_IWGRP | S_IWOTH)) != 0 {
            let octal = String(format: "%o", mode & 0o777)
            throw ConsentError.unsafeStorage("Target path \(path) has unsafe permissions \(octal) (group or world writable)")
        }
    }

    public func load() throws -> LocalConsent {
        lock.lock()
        defer { lock.unlock() }

        let parentDir = fileURL.deletingLastPathComponent()
        let parentPath = parentDir.path
        if parentPath.isEmpty {
            throw ConsentError.unsafeStorage("Target path has no parent directory")
        }

        let parentExists = try Self.validateParentDirectory(atPath: parentPath, allowMissing: true)
        if !parentExists {
            return .absent
        }

        let path = fileURL.path
        try Self.validateTargetFile(atPath: path)

        let fd = Darwin.open(path, O_RDONLY | O_NOFOLLOW)
        if fd < 0 {
            if errno == ENOENT {
                return .absent
            }
            if errno == ELOOP || errno == EMLINK {
                throw ConsentError.unsafeStorage("Target path \(path) is a symbolic link")
            }
            let err = String(cString: strerror(errno))
            throw ConsentError.ioError("Cannot open \(path): \(err)")
        }
        defer { Darwin.close(fd) }

        var st = stat()
        if Darwin.fstat(fd, &st) != 0 {
            let err = String(cString: strerror(errno))
            throw ConsentError.ioError("Cannot fstat \(path): \(err)")
        }

        let mode = Int32(st.st_mode)
        let fileType = mode & Int32(S_IFMT)
        if fileType != Int32(S_IFREG) {
            throw ConsentError.unsafeStorage("Target path \(path) is not a regular file")
        }

        let currentUid = geteuid()
        if st.st_uid != currentUid {
            throw ConsentError.unsafeStorage("Target path \(path) is owned by UID \(st.st_uid), expected current UID \(currentUid)")
        }

        if (mode & Int32(S_IWGRP | S_IWOTH)) != 0 {
            let octal = String(format: "%o", mode & 0o777)
            throw ConsentError.unsafeStorage("Target path \(path) has unsafe permissions \(octal) (group or world writable)")
        }

        var data = Data()
        var buffer = [UInt8](repeating: 0, count: 4096)
        while true {
            let bytesRead = Darwin.read(fd, &buffer, buffer.count)
            if bytesRead < 0 {
                let err = String(cString: strerror(errno))
                throw ConsentError.ioError("Cannot read \(path): \(err)")
            }
            if bytesRead == 0 {
                break
            }
            data.append(buffer, count: bytesRead)
        }

        do {
            return try JSONDecoder().decode(LocalConsent.self, from: data)
        } catch {
            throw ConsentError.malformedJson("Failed to parse \(path): \(error)")
        }
    }

    public func save(_ consent: LocalConsent) throws {
        lock.lock()
        defer { lock.unlock() }

        let parentDir = fileURL.deletingLastPathComponent()
        let parentPath = parentDir.path
        if parentPath.isEmpty {
            throw ConsentError.saveFailed("Target path has no parent directory")
        }

        let parentExists = try Self.validateParentDirectory(atPath: parentPath, allowMissing: true)
        if !parentExists {
            do {
                try FileManager.default.createDirectory(at: parentDir, withIntermediateDirectories: true,
                                                        attributes: [.posixPermissions: 0o700])
            } catch {
                throw ConsentError.saveFailed("Cannot create directory \(parentPath): \(error.localizedDescription)")
            }
            _ = try Self.validateParentDirectory(atPath: parentPath, allowMissing: false)
        }

        let targetPath = fileURL.path
        try Self.validateTargetFile(atPath: targetPath)

        let encoder = JSONEncoder()
        encoder.outputFormatting = [.prettyPrinted, .sortedKeys]
        let data: Data
        do {
            data = try encoder.encode(consent)
        } catch {
            throw ConsentError.saveFailed("Cannot encode consent: \(error)")
        }

        let fileStem = fileURL.lastPathComponent
        let maxRetries = 64
        var openedFd: Int32 = -1
        var ownedTempPath: String?

        for attempt in 0..<maxRetries {
            let tempName: String
            if let namer = hooks?.candidateNamer {
                tempName = namer(fileStem, attempt)
            } else {
                let pid = ProcessInfo.processInfo.processIdentifier
                let nonce = UInt64.random(in: 0...UInt64.max)
                tempName = String(format: ".%@.tmp.%d.%d.%016llx", fileStem, pid, attempt, nonce)
            }

            let candidatePath = (parentPath as NSString).appendingPathComponent(tempName)
            let fd = open(candidatePath, O_WRONLY | O_CREAT | O_EXCL, mode_t(S_IRUSR | S_IWUSR))
            if fd >= 0 {
                openedFd = fd
                ownedTempPath = candidatePath
                break
            } else if errno == EEXIST {
                continue
            } else {
                let err = String(cString: strerror(errno))
                throw ConsentError.saveFailed("Failed opening exclusive temporary file \(candidatePath): \(err)")
            }
        }

        guard openedFd >= 0, let tempPath = ownedTempPath else {
            throw ConsentError.saveFailed("Exceeded maximum collision retries creating temporary consent file")
        }

        var writeFailed = false
        var failureMessage = ""

        data.withUnsafeBytes { rawBuffer in
            guard let baseAddress = rawBuffer.baseAddress else { return }
            var ptr = baseAddress
            var remaining = data.count
            while remaining > 0 {
                let written = write(openedFd, ptr, remaining)
                if written < 0 {
                    if errno == EINTR { continue }
                    writeFailed = true
                    failureMessage = "Write failed on \(tempPath): \(String(cString: strerror(errno)))"
                    return
                }
                remaining -= written
                ptr = ptr.advanced(by: written)
            }
        }

        if !writeFailed {
            if fsync(openedFd) != 0 {
                writeFailed = true
                failureMessage = "fsync failed on \(tempPath): \(String(cString: strerror(errno)))"
            }
        }

        if let failAfterOpen = hooks?.failAfterOpen, !writeFailed {
            do {
                try failAfterOpen(tempPath)
            } catch {
                writeFailed = true
                failureMessage = "Injected post-write failure on \(tempPath): \(error.localizedDescription)"
            }
        }

        close(openedFd)

        if writeFailed {
            unlink(tempPath)
            throw ConsentError.saveFailed(failureMessage)
        }

        if let failRename = hooks?.failRename {
            do {
                try failRename(tempPath, targetPath)
            } catch {
                unlink(tempPath)
                throw ConsentError.saveFailed("Injected rename failure for \(targetPath): \(error.localizedDescription)")
            }
        }

        if Darwin.rename(tempPath, targetPath) != 0 {
            let err = String(cString: strerror(errno))
            unlink(tempPath)
            throw ConsentError.saveFailed("Failed atomically replacing consent file \(targetPath) with \(tempPath): \(err)")
        }
    }
}

/// In-memory fake consent storage with fault injection and barrier callbacks for deterministic testing.
public final class MemoryConsentStore: ConsentStorage, @unchecked Sendable {
    private let lock = NSLock()
    private var consent: LocalConsent
    public var failSaves = false
    public var failLoads = false
    public private(set) var saveCount = 0
    public private(set) var loadCount = 0
    public var saveBarrier: (@Sendable (LocalConsent) throws -> Void)?

    public init(initial: LocalConsent = .absent) {
        self.consent = initial
    }

    public func load() throws -> LocalConsent {
        lock.lock()
        defer { lock.unlock() }
        loadCount += 1
        if failLoads {
            throw ConsentError.ioError("Simulated load failure")
        }
        return consent
    }

    public func save(_ consent: LocalConsent) throws {
        lock.lock()
        saveCount += 1
        let barrier = saveBarrier
        let fail = failSaves
        lock.unlock()

        if let barrier = barrier {
            try barrier(consent)
        }
        if fail {
            throw ConsentError.saveFailed("Simulated persistence failure")
        }

        lock.lock()
        self.consent = consent
        lock.unlock()
    }

    public var current: LocalConsent {
        lock.lock()
        defer { lock.unlock() }
        return consent
    }
}

/// Explicit transition record tracking serialization, immediate conservative state, and target state.
public struct ConsentTransition: Equatable, Sendable {
    public let sequence: UInt64
    public let immediate: LocalConsent
    public let target: LocalConsent
    public let changed: Bool

    public init(sequence: UInt64, immediate: LocalConsent, target: LocalConsent, changed: Bool) {
        self.sequence = sequence
        self.immediate = immediate
        self.target = target
        self.changed = changed
    }
}

/// In-memory manager for local consent with immediate revocation and persistence-first widening semantics.
public final class ConsentManager: @unchecked Sendable {
    private let lock = NSLock()
    private var currentConsent: LocalConsent
    public let storage: (any ConsentStorage)?
    public private(set) var generation: UInt64 = 1
    private var transitionSequence: UInt64 = 0

    /// Dedicated serial queue ensuring disk persistence operations execute in strict FIFO order off any actor.
    private let persistenceQueue = DispatchQueue(label: "com.dosibridge.dositracker.consent.persistence", qos: .userInitiated)

    public init(initial: LocalConsent = .absent, storage: (any ConsentStorage)? = nil) {
        self.currentConsent = initial
        self.storage = storage
    }

    public static func loadOrFailClosed(store: any ConsentStorage) -> (ConsentManager, ConsentError?) {
        do {
            let loaded = try store.load()
            return (ConsentManager(initial: loaded, storage: store), nil)
        } catch let err as ConsentError {
            return (ConsentManager(initial: .absent, storage: store), err)
        } catch {
            return (ConsentManager(initial: .absent, storage: store), .malformedJson(error.localizedDescription))
        }
    }

    public var current: LocalConsent {
        lock.lock()
        defer { lock.unlock() }
        return currentConsent
    }

    /// Compute effective permissions against a project policy.
    public func effectivePermissions(project: CapturePermissions?) -> ConsentedPermissions {
        lock.lock()
        let c = currentConsent
        lock.unlock()
        guard let project = project else { return .allDisabled }
        switch c {
        case .granted(let local):
            return local.intersect(project)
        case .absent, .revoked:
            return .allDisabled
        }
    }

    /// Prepares an explicit transition to `next`.
    /// Synchronously computes the conservative in-memory immediate state
    /// (withdrawals applied, blur tightened, expansions deferred) under lock,
    /// advances sequence, and updates `currentConsent` to the immediate conservative state.
    public func prepareTransition(to next: LocalConsent) -> (transition: ConsentTransition, changed: Bool) {
        lock.lock()
        defer { lock.unlock() }

        let prev = currentConsent
        if prev == next {
            let transition = ConsentTransition(sequence: transitionSequence, immediate: prev, target: next, changed: false)
            return (transition, false)
        }

        transitionSequence &+= 1
        generation &+= 1
        let seq = transitionSequence

        let immediate: LocalConsent
        switch (prev, next) {
        case (.granted(let pOld), .granted(let pNew)):
            // Conservative intersection: only capabilities present in BOTH are kept.
            // Blur is enforced if either requested it (defense-in-depth).
            let pImmediate = ConsentedPermissions(
                screenshot: pOld.screenshot && pNew.screenshot,
                webcam: pOld.webcam && pNew.webcam,
                keyboard: pOld.keyboard && pNew.keyboard,
                mouse: pOld.mouse && pNew.mouse,
                activeWindow: pOld.activeWindow && pNew.activeWindow,
                runningPrograms: pOld.runningPrograms && pNew.runningPrograms,
                blurScreenshots: pOld.blurScreenshots || pNew.blurScreenshots
            )
            immediate = .granted(pImmediate)

        case (.granted, .revoked), (.granted, .absent):
            // Immediate withdrawal
            immediate = next

        case (.absent, .granted), (.revoked, .granted):
            // Pure widening: expansions must NOT be admitted before save succeeds!
            // Immediate state remains the inactive state (all disabled).
            immediate = prev

        default:
            // absent <-> revoked
            immediate = next
        }

        currentConsent = immediate
        let transition = ConsentTransition(sequence: seq, immediate: immediate, target: next, changed: true)
        return (transition, true)
    }

    /// Commits the target state of a successful transition if it has not been superseded.
    /// Returns true if committed; false if superseded by a newer transition.
    @discardableResult
    public func commitTransition(_ transition: ConsentTransition) -> Bool {
        lock.lock()
        defer { lock.unlock() }

        guard transition.sequence == transitionSequence else {
            // Superseded by a newer transition! Do not resurrect or mutate state.
            return false
        }

        currentConsent = transition.target
        return true
    }

    /// Reconciles state after a failed transition.
    /// If not superseded, leaves `currentConsent` as the conservative immediate state
    /// (narrowings preserved, expansions denied).
    public func failTransition(_ transition: ConsentTransition) {
        lock.lock()
        defer { lock.unlock() }

        guard transition.sequence == transitionSequence else {
            return
        }

        currentConsent = transition.immediate
    }

    /// Executes persistence asynchronously on the serial persistence queue, off any actor.
    public func persistAsync(_ consent: LocalConsent) async throws {
        guard let storage = storage else { return }
        try await withCheckedThrowingContinuation { (continuation: CheckedContinuation<Void, Error>) in
            persistenceQueue.async {
                do {
                    try storage.save(consent)
                    continuation.resume()
                } catch {
                    continuation.resume(throwing: error)
                }
            }
        }
    }

    /// Update consent state.
    ///
    /// CRITICAL PRIVACY & PERSISTENCE INVARIANTS:
    /// - Capabilities removed in `next` are withdrawn IMMEDIATELY in-memory before disk I/O.
    /// - Blur tightening (`blurScreenshots` false -> true) is applied IMMEDIATELY as defense-in-depth.
    /// - Newly added capabilities in `next` are admitted ONLY AFTER persistence succeeds.
    /// - If persistence fails on a mixed update (narrowing + expansion), removed permissions
    ///   remain withdrawn and tightened blur remains enforced, but new permissions are NOT granted.
    /// - Pure widening remains strictly persistence-first.
    /// - Identical policy does NOT manufacture ticket invalidation; actual policy/consent withdrawal/regrant does.
    @discardableResult
    public func update(_ next: LocalConsent) throws -> Bool {
        let (transition, changed) = prepareTransition(to: next)
        guard changed else { return false }

        if let storage = storage {
            do {
                try storage.save(transition.target)
            } catch {
                failTransition(transition)
                throw error
            }
        }

        _ = commitTransition(transition)
        return true
    }

    /// Revoke all consent immediately.
    public func revoke() throws {
        try update(.revoked)
    }
}
