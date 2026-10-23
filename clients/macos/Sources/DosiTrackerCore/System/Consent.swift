import Foundation

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

    public var description: String {
        switch self {
        case .ioError(let s): return "Consent I/O error: \(s)"
        case .malformedJson(let s): return "Malformed consent JSON: \(s)"
        case .saveFailed(let s): return "Consent save failed: \(s)"
        }
    }
}

/// Abstract storage interface for consent persistence.
public protocol ConsentStorage: AnyObject, Sendable {
    func load() throws -> LocalConsent
    func save(_ consent: LocalConsent) throws
}

/// File-backed atomic consent store.
public final class FileConsentStore: ConsentStorage, @unchecked Sendable {
    public let fileURL: URL
    private let lock = NSLock()

    public init(fileURL: URL) {
        self.fileURL = fileURL
    }

    public func load() throws -> LocalConsent {
        lock.lock()
        defer { lock.unlock() }
        let path = fileURL.path
        guard FileManager.default.fileExists(atPath: path) else {
            return .absent
        }
        let data: Data
        do {
            data = try Data(contentsOf: fileURL)
        } catch {
            throw ConsentError.ioError("Cannot read \(path): \(error.localizedDescription)")
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
        do {
            try FileManager.default.createDirectory(at: parentDir, withIntermediateDirectories: true,
                                                    attributes: [.posixPermissions: 0o700])
        } catch {
            throw ConsentError.saveFailed("Cannot create directory \(parentDir.path): \(error.localizedDescription)")
        }

        let encoder = JSONEncoder()
        encoder.outputFormatting = [.prettyPrinted, .sortedKeys]
        let data: Data
        do {
            data = try encoder.encode(consent)
        } catch {
            throw ConsentError.saveFailed("Cannot encode consent: \(error)")
        }

        let pid = ProcessInfo.processInfo.processIdentifier
        let tmpURL = parentDir.appendingPathComponent("consent.tmp.\(pid).\(UUID().uuidString.lowercased())", isDirectory: false)
        do {
            try data.write(to: tmpURL, options: .atomic)
            if FileManager.default.fileExists(atPath: fileURL.path) {
                _ = try FileManager.default.replaceItemAt(fileURL, withItemAt: tmpURL)
            } else {
                try FileManager.default.moveItem(at: tmpURL, to: fileURL)
            }
        } catch {
            try? FileManager.default.removeItem(at: tmpURL)
            throw ConsentError.saveFailed("Failed to atomically save \(fileURL.path): \(error.localizedDescription)")
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
