import Security
import XCTest
@testable import DosiTrackerCore

/// The saved sign-in.
final class CredentialsStoreTests: XCTestCase {
    func testASavedSignInRoundTripsAndNeverPrintsItsPassword() throws {
        let signIn = StoredSignIn(workspace: "Acme", username: "alice@acme.com", password: "s3cr3t")
        let back = try JSONDecoder().decode(StoredSignIn.self, from: JSONEncoder().encode(signIn))
        XCTAssertEqual(back, signIn)
        XCTAssertFalse("\(signIn)".contains("s3cr3t"))
        XCTAssertFalse(String(reflecting: signIn).contains("s3cr3t"))
        XCTAssertEqual(signIn.ownerKey, Queue.ownerKey(workspace: "Acme", username: "alice@acme.com"))
        XCTAssertEqual(StoredSignIn(account: signIn.account), signIn)
    }

    func testAnItemWithoutAWorkspaceIsAHostAccount() throws {
        let legacy = try JSONDecoder().decode(StoredSignIn.self, from: Data(#"{"username":"admin","password":"x"}"#.utf8))
        XCTAssertEqual(legacy.workspace, "")
        XCTAssertThrowsError(try JSONDecoder().decode(StoredSignIn.self, from: Data(#"{"username":"admin"}"#.utf8)))
    }

    func testTheInMemoryStoreSavesReplacesAndClears() throws {
        let store = InMemoryCredentialsStore()
        XCTAssertNil(try store.load())
        try store.save(StoredSignIn(workspace: "Acme", username: "a", password: "1"))
        try store.save(StoredSignIn(workspace: "Acme", username: "b", password: "2"))
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
            try store.save(StoredSignIn(workspace: "Acme", username: "alice@acme.com", password: "first"))
        } catch let error as KeychainError where error.isUnavailable {
            throw XCTSkip("the Keychain is not available here: \(error)")
        }
        try store.save(StoredSignIn(workspace: "Acme", username: "alice@acme.com", password: "second"))
        XCTAssertEqual(try store.load(), StoredSignIn(workspace: "Acme", username: "alice@acme.com", password: "second"))
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
