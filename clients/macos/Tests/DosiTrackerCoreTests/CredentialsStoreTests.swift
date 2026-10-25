import Security
import XCTest
@testable import DosiTrackerCore

/// The saved sign-in.
final class CredentialsStoreTests: XCTestCase {
    private let workspaceId = "3f2a1b0c-0000-4000-8000-00000000a001"

    private func sample(_ refreshToken: String? = "refresh-fixture-1") -> StoredSignIn {
        StoredSignIn(username: "alice@acme.com", workspaceId: workspaceId, workspaceName: "Acme",
                     refreshToken: refreshToken)
    }

    func testASavedSignInRoundTripsAndNeverPrintsItsToken() throws {
        let signIn = sample()
        let data = try JSONEncoder().encode(signIn)
        let back = try JSONDecoder().decode(StoredSignIn.self, from: data)
        XCTAssertEqual(back, signIn)
        XCTAssertTrue(back.canResume)
        XCTAssertFalse("\(signIn)".contains("refresh-fixture-1"))
        XCTAssertFalse(String(reflecting: signIn).contains("refresh-fixture-1"))
        XCTAssertEqual(signIn.ownerKey, Queue.ownerKey(workspaceId: workspaceId, username: "alice@acme.com"))
        // No password is ever written.
        let json = jsonDictionary(data)
        XCTAssertNil(json["password"])
        XCTAssertEqual(Set(json.keys), ["username", "workspaceId", "workspaceName", "refreshToken"])
    }

    func testASignInWithoutATokenOrAWorkspaceCannotResumeAlone() {
        XCTAssertFalse(sample(nil).canResume, "without a refresh token the user signs in again")
        let unchosen = StoredSignIn(username: "alice@acme.com", workspaceId: nil, workspaceName: "",
                                    refreshToken: "refresh-fixture-2")
        XCTAssertTrue(unchosen.canResume, "the workspace is chosen again after resuming")
        XCTAssertNil(unchosen.ownerKey)
    }

    /// The retired format kept the password and a workspace name: read it
    /// for the email and the queue's rows, never for the password.
    func testTheRetiredFormatIsReadWithoutItsPassword() throws {
        let retired = Data(#"{"workspace":"Acme","username":"alice@acme.com","password":"retired-fixture"}"#.utf8)
        let legacy = try JSONDecoder().decode(StoredSignIn.self, from: retired)
        XCTAssertEqual(legacy.username, "alice@acme.com")
        XCTAssertEqual(legacy.legacyWorkspaceName, "Acme")
        XCTAssertNil(legacy.workspaceId)
        XCTAssertNil(legacy.refreshToken)
        XCTAssertFalse(legacy.canResume)
        XCTAssertNil(legacy.ownerKey)
        XCTAssertEqual(legacy.legacyOwnerKey, Queue.legacyOwnerKey(workspaceName: "Acme", username: "alice@acme.com"))
        // Saved again, it is the new format: the password is gone for good.
        let rewritten = String(decoding: try JSONEncoder().encode(legacy), as: UTF8.self)
        XCTAssertFalse(rewritten.contains("retired-fixture"))
        XCTAssertThrowsError(try JSONDecoder().decode(StoredSignIn.self, from: Data(#"{"workspaceId":"x"}"#.utf8)))
    }

    func testTheInMemoryStoreSavesReplacesAndClears() throws {
        let store = InMemoryCredentialsStore()
        XCTAssertNil(try store.load())
        try store.save(StoredSignIn(username: "a", workspaceId: workspaceId, workspaceName: "Acme", refreshToken: "r1"))
        try store.save(StoredSignIn(username: "b", workspaceId: workspaceId, workspaceName: "Acme", refreshToken: "r2"))
        XCTAssertEqual(try store.load()?.username, "b")
        try store.clear()
        XCTAssertNil(try store.load())
        try store.clear() // nothing to clear: not an error
    }

    func testTheKeychainStoreSavesReplacesAndClears() throws {
        // A service of its own, so the real sign-in is never touched.
        let store = KeychainCredentialsStore(service: "com.dosibridge.tracker.tests.\(UUID().uuidString)",
                                             account: "sign-in")
        defer { try? store.clear() }
        do {
            XCTAssertNil(try store.load())
            try store.save(sample("refresh-fixture-first"))
        } catch let error as KeychainError where error.isUnavailable {
            throw XCTSkip("the Keychain is not available here: \(error)")
        }
        try store.save(sample("refresh-fixture-second"))
        XCTAssertEqual(try store.load(), sample("refresh-fixture-second"))
        try store.clear()
        XCTAssertNil(try store.load())
        try store.clear()
    }

    func testKeychainErrorsDescribeTheirStatus() {
        let error = KeychainError(status: errSecInteractionNotAllowed)
        XCTAssertTrue(error.isUnavailable)
        XCTAssertTrue(error.description.contains("\(errSecInteractionNotAllowed)"))
        XCTAssertFalse(KeychainError(status: errSecDuplicateItem).isUnavailable)
    }
}
