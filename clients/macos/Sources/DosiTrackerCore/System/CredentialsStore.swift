import Foundation
import Security

/// The saved sign-in, so a relaunch (e.g. at login) resumes silently and a
/// "Sign-in expired" prompt can prefill the workspace and email.
public struct StoredSignIn: Codable, Equatable, Sendable, CustomStringConvertible, CustomDebugStringConvertible {
    /// Workspace (tenant) name; empty for a host account.
    public var workspace: String
    public var username: String
    public var password: String

    public init(workspace: String, username: String, password: String) {
        self.workspace = workspace
        self.username = username
        self.password = password
    }

    public init(account: ApiClient.Account) {
        self.init(workspace: account.workspace, username: account.username, password: account.password)
    }

    public var account: ApiClient.Account {
        ApiClient.Account(workspace: workspace, username: username, password: password)
    }

    /// The queue owner key of this sign-in.
    public var ownerKey: String { Queue.ownerKey(workspace: workspace, username: username) }

    /// Never includes the password.
    public var description: String { account.description }

    public var debugDescription: String { description }

    private enum CodingKeys: String, CodingKey {
        case workspace, username, password
    }

    /// A missing workspace reads as a host account.
    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        workspace = try c.decodeIfPresent(String.self, forKey: .workspace) ?? ""
        username = try c.decode(String.self, forKey: .username)
        password = try c.decode(String.self, forKey: .password)
    }

    public func encode(to encoder: Encoder) throws {
        var c = encoder.container(keyedBy: CodingKeys.self)
        try c.encode(workspace, forKey: .workspace)
        try c.encode(username, forKey: .username)
        try c.encode(password, forKey: .password)
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
/// (`kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly`). The password is
/// never written anywhere else.
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
