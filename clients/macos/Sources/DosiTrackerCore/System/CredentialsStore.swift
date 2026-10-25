import Foundation
import Security

/// The saved sign-in, so a relaunch (e.g. at login) resumes silently and a
/// "Sign-in expired" prompt can prefill the email: the user, the workspace
/// tracked in and the OpenIddict refresh token. The password is never kept —
/// once the refresh token is refused the user signs in again (cross-agent
/// contract), it is never re-sent silently.
public struct StoredSignIn: Codable, Equatable, Sendable, CustomStringConvertible, CustomDebugStringConvertible {
    public var username: String
    /// The workspace GUID tracked in; nil while none is chosen.
    public var workspaceId: String?
    /// Its name, for the window and the menu.
    public var workspaceName: String
    /// The latest (rotated) refresh token; nil when the server granted none,
    /// which means signing in again.
    public var refreshToken: String?
    /// Saved by a build that signed in to a workspace by name with a stored
    /// password (the retired `__tenant` sign-in): its workspace name, so its
    /// queued rows can be handed to that workspace. Never saved again.
    public private(set) var legacyWorkspaceName: String?

    public init(username: String, workspaceId: String?, workspaceName: String, refreshToken: String?) {
        self.username = username
        self.workspaceId = workspaceId
        self.workspaceName = workspaceName
        self.refreshToken = refreshToken
        legacyWorkspaceName = nil
    }

    /// The queue owner key of this sign-in; nil without a workspace.
    public var ownerKey: String? {
        guard let id = workspaceId?.nonEmptyTrimmed else { return nil }
        return Queue.ownerKey(workspaceId: id, username: username)
    }

    /// The owner key the retired build queued this user's rows under.
    public var legacyOwnerKey: String? {
        legacyWorkspaceName.map { Queue.legacyOwnerKey(workspaceName: $0, username: username) }
    }

    /// Whether it can resume without the password: a refresh token, and not
    /// the retired format.
    public var canResume: Bool {
        legacyWorkspaceName == nil && refreshToken?.nonEmptyTrimmed != nil
    }

    /// Never includes the refresh token.
    public var description: String {
        let workspace = workspaceName.nonEmptyTrimmed ?? workspaceId ?? legacyWorkspaceName ?? "(no workspace)"
        return "\(username) @ \(workspace)"
    }

    public var debugDescription: String { description }

    private enum CodingKeys: String, CodingKey {
        case username, workspaceId, workspaceName, refreshToken
        // The retired format: a workspace (tenant) name and the password.
        case workspace, password
    }

    /// Reads both formats. The retired one's password is dropped at once (it
    /// is never used again) and its workspace name kept as
    /// ``legacyWorkspaceName``.
    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        username = try c.decode(String.self, forKey: .username)
        workspaceId = try c.decodeIfPresent(String.self, forKey: .workspaceId)?.nonEmptyTrimmed
        workspaceName = try c.decodeIfPresent(String.self, forKey: .workspaceName) ?? ""
        refreshToken = try c.decodeIfPresent(String.self, forKey: .refreshToken)?.nonEmptyTrimmed
        if c.contains(.password) {
            legacyWorkspaceName = try c.decodeIfPresent(String.self, forKey: .workspace) ?? ""
            workspaceId = nil
            refreshToken = nil
        } else {
            legacyWorkspaceName = nil
        }
    }

    public func encode(to encoder: Encoder) throws {
        var c = encoder.container(keyedBy: CodingKeys.self)
        try c.encode(username, forKey: .username)
        try c.encodeIfPresent(workspaceId, forKey: .workspaceId)
        try c.encode(workspaceName, forKey: .workspaceName)
        try c.encodeIfPresent(refreshToken, forKey: .refreshToken)
    }
}

/// Where the sign-in is kept. The app uses ``KeychainCredentialsStore``;
/// tests and previews use ``InMemoryCredentialsStore``.
public protocol CredentialsStore: AnyObject, Sendable {
    /// The saved sign-in, or nil when there is none (or it is unreadable,
    /// which shows the sign-in form again).
    func load() throws -> StoredSignIn?
    /// Save (or replace) the sign-in.
    func save(_ signIn: StoredSignIn) throws
    /// Forget the sign-in (sign out). Not an error when there is none.
    func clear() throws
}

/// A Keychain failure.
public struct KeychainError: Error, Equatable, CustomStringConvertible {
    public let status: OSStatus

    public init(status: OSStatus) {
        self.status = status
    }

    /// The Keychain cannot be used right now (locked, no user interaction
    /// allowed, or an entitlement is missing), as opposed to a broken item.
    public var isUnavailable: Bool {
        status == errSecInteractionNotAllowed || status == errSecNotAvailable
            || status == errSecMissingEntitlement || status == errSecNoSuchKeychain
            || status == errSecNoDefaultKeychain
    }

    public var description: String {
        let message = SecCopyErrorMessageString(status, nil) as String? ?? "unknown error"
        return "Keychain error \(status): \(message)"
    }
}

/// The sign-in as one generic-password item (`kSecClassGenericPassword`) in
/// the user's Keychain: service `com.dosibridge.tracker`, accessible after
/// the first unlock and never synced or migrated to another device
/// (`kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly`). The refresh token is
/// never written anywhere else, and the password is not kept at all.
public final class KeychainCredentialsStore: CredentialsStore {
    public static let defaultService = AgentInfo.bundleIdentifier
    public static let defaultAccount = "sign-in"

    public let service: String
    public let account: String
    /// Use the data-protection keychain. It needs a signed app with a
    /// keychain-access-group entitlement; without one the file-based login
    /// keychain is used.
    public let useDataProtectionKeychain: Bool

    public init(service: String = KeychainCredentialsStore.defaultService,
                account: String = KeychainCredentialsStore.defaultAccount,
                useDataProtectionKeychain: Bool = false) {
        self.service = service
        self.account = account
        self.useDataProtectionKeychain = useDataProtectionKeychain
    }

    public func load() throws -> StoredSignIn? {
        var query = baseQuery()
        query[kSecReturnData as String] = true
        query[kSecMatchLimit as String] = kSecMatchLimitOne
        var result: CFTypeRef?
        let status = SecItemCopyMatching(query as CFDictionary, &result)
        if status == errSecItemNotFound {
            return nil
        }
        guard status == errSecSuccess else {
            throw KeychainError(status: status)
        }
        guard let data = result as? Data else { return nil }
        // An unreadable item counts as "no sign-in": the form is shown again.
        return try? JSONDecoder().decode(StoredSignIn.self, from: data)
    }

    public func save(_ signIn: StoredSignIn) throws {
        let data = try JSONEncoder().encode(signIn)
        let update: [String: Any] = [kSecValueData as String: data]
        let status = SecItemUpdate(baseQuery() as CFDictionary, update as CFDictionary)
        if status == errSecSuccess {
            return
        }
        guard status == errSecItemNotFound else {
            throw KeychainError(status: status)
        }

        var item = baseQuery()
        item[kSecValueData as String] = data
        item[kSecAttrLabel as String] = "\(AgentInfo.displayName) sign-in"
        item[kSecAttrAccessible as String] = kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly
        var added = SecItemAdd(item as CFDictionary, nil)
        if added == errSecParam {
            // Some file-based keychains reject the accessibility class; the
            // item is still kept in this user's keychain only.
            item.removeValue(forKey: kSecAttrAccessible as String)
            added = SecItemAdd(item as CFDictionary, nil)
        }
        guard added == errSecSuccess else {
            throw KeychainError(status: added)
        }
    }

    public func clear() throws {
        let status = SecItemDelete(baseQuery() as CFDictionary)
        guard status == errSecSuccess || status == errSecItemNotFound else {
            throw KeychainError(status: status)
        }
    }

    private func baseQuery() -> [String: Any] {
        var query: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: service,
            kSecAttrAccount as String: account,
        ]
        if useDataProtectionKeychain {
            query[kSecUseDataProtectionKeychain as String] = true
        }
        return query
    }
}

/// A sign-in kept in memory only (tests, previews).
public final class InMemoryCredentialsStore: CredentialsStore, @unchecked Sendable {
    private let lock = NSLock()
    private var stored: StoredSignIn?

    public init(_ signIn: StoredSignIn? = nil) {
        stored = signIn
    }

    public func load() throws -> StoredSignIn? {
        lock.lock()
        defer { lock.unlock() }
        return stored
    }

    public func save(_ signIn: StoredSignIn) throws {
        lock.lock()
        defer { lock.unlock() }
        stored = signIn
    }

    public func clear() throws {
        lock.lock()
        defer { lock.unlock() }
        stored = nil
    }
}
