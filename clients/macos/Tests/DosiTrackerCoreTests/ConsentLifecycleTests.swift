import Foundation
import XCTest
@testable import DosiTrackerCore

/// Production-path regression suite for explicit local consent, persistence ordering,
/// fail-closed error handling, and per-input native admission gating.
final class ConsentLifecycleTests: XCTestCase {

    // MARK: - 1. Absent / Invalid Consent Fail-Closed

    func testDefaultConsentIsAbsentAndFailsClosed() {
        let consent = LocalConsent.absent
        XCTAssertFalse(consent.isGranted)
        XCTAssertFalse(consent.isRevoked)
        XCTAssertEqual(consent.permissions, ConsentedPermissions.allDisabled)
        XCTAssertFalse(consent.permissions.hasAnyActiveCapability)

        let projectPolicy = CapturePermissions.all
        let manager = ConsentManager(initial: consent)
        let effective = manager.effectivePermissions(project: projectPolicy)
        XCTAssertEqual(effective, ConsentedPermissions.allDisabled)
        XCTAssertFalse(effective.hasAnyActiveCapability)
    }

    func testMalformedJsonFileFailsClosed() throws {
        let tempDir = FileManager.default.temporaryDirectory
            .appendingPathComponent("consent-test-\(UUID().uuidString.lowercased())", isDirectory: true)
        try FileManager.default.createDirectory(at: tempDir, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: tempDir) }

        let fileURL = tempDir.appendingPathComponent("consent.json")
        // Write corrupt JSON
        try Data("{\"state\": \"unknown_invalid_schema\", corrupt: true}".utf8).write(to: fileURL)

        let store = FileConsentStore(fileURL: fileURL)
        XCTAssertThrowsError(try store.load()) { error in
            guard let consentErr = error as? ConsentError else {
                XCTFail("Expected ConsentError, got \(error)")
                return
            }
            if case .malformedJson = consentErr {
                // Expected
            } else {
                XCTFail("Expected malformedJson error, got \(consentErr)")
            }
        }

        let (manager, error) = ConsentManager.loadOrFailClosed(store: store)
        XCTAssertNotNil(error)
        XCTAssertEqual(manager.current, .absent)
        XCTAssertEqual(manager.effectivePermissions(project: CapturePermissions.all), ConsentedPermissions.allDisabled)
    }

    // MARK: - 2. Per-Input Capability Native Admission Gating

    func testPerInputCapabilityNativeAdmissionGating() {
        let log = InputLog()
        let filter = InputFilter(log: log, keyboard: false, mouse: false)

        let tMs: Int64 = 1_790_000_000_000

        // 1. Both disabled: all events are dropped right at admission
        filter.apply(.keyDown(0x00), injected: false, atMs: tMs + 100)
        filter.apply(.buttonDown, injected: false, atMs: tMs + 200)
        filter.apply(.move, injected: false, atMs: tMs + 300)
        var counts = log.counts(fromMs: tMs, toMs: tMs + 1_000)
        XCTAssertEqual(counts.keyPresses, 0)
        XCTAssertEqual(counts.clicks, 0)
        XCTAssertEqual(log.activitySeconds(fromMs: tMs, toMs: tMs + 1_000).keyboard, 0)
        XCTAssertEqual(log.activitySeconds(fromMs: tMs, toMs: tMs + 1_000).mouse, 0)

        // 2. Keyboard-only: key presses admitted; mouse events dropped
        filter.setPermissions(keyboard: true, mouse: false)
        filter.apply(.keyDown(0x01), injected: false, atMs: tMs + 1_100)
        filter.apply(.keyUp(0x01), injected: false, atMs: tMs + 1_150)
        filter.apply(.buttonDown, injected: false, atMs: tMs + 1_200) // dropped
        filter.apply(.move, injected: false, atMs: tMs + 1_300)       // dropped

        counts = log.counts(fromMs: tMs + 1_000, toMs: tMs + 2_000)
        XCTAssertEqual(counts.keyPresses, 1)
        XCTAssertEqual(counts.clicks, 0)
        XCTAssertEqual(log.activitySeconds(fromMs: tMs + 1_000, toMs: tMs + 2_000).keyboard, 1)
        XCTAssertEqual(log.activitySeconds(fromMs: tMs + 1_000, toMs: tMs + 2_000).mouse, 0)

        // 3. Mouse-only: clicks and moves admitted; key presses dropped
        filter.setPermissions(keyboard: false, mouse: true)
        filter.apply(.keyDown(0x02), injected: false, atMs: tMs + 2_100) // dropped
        filter.apply(.buttonDown, injected: false, atMs: tMs + 2_200)
        filter.apply(.move, injected: false, atMs: tMs + 2_300)

        counts = log.counts(fromMs: tMs + 2_000, toMs: tMs + 3_000)
        XCTAssertEqual(counts.keyPresses, 0)
        XCTAssertEqual(counts.clicks, 1)
        XCTAssertEqual(log.activitySeconds(fromMs: tMs + 2_000, toMs: tMs + 3_000).keyboard, 0)
        XCTAssertEqual(log.activitySeconds(fromMs: tMs + 2_000, toMs: tMs + 3_000).mouse, 1)
    }

    // MARK: - 3. Successful Opt-In and Revoke with Atomic File Storage

    func testSuccessfulOptInAndRevocationFilePersistence() throws {
        let tempDir = FileManager.default.temporaryDirectory
            .appendingPathComponent("consent-atomic-\(UUID().uuidString.lowercased())", isDirectory: true)
        try FileManager.default.createDirectory(at: tempDir, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: tempDir) }

        let fileURL = tempDir.appendingPathComponent("consent.json")
        let store = FileConsentStore(fileURL: fileURL)

        // Initially absent
        XCTAssertEqual(try store.load(), .absent)

        let manager = ConsentManager(initial: .absent, storage: store)
        let grant = ConsentedPermissions(
            screenshot: true,
            webcam: false,
            keyboard: true,
            mouse: true,
            activeWindow: true,
            runningPrograms: false,
            blurScreenshots: true
        )

        let changed = try manager.update(.granted(grant))
        XCTAssertTrue(changed)
        XCTAssertEqual(manager.current, .granted(grant))

        // Reload fresh from file to verify atomic persistence
        let freshStore = FileConsentStore(fileURL: fileURL)
        let loaded = try freshStore.load()
        XCTAssertEqual(loaded, .granted(grant))

        // Revoke
        try manager.revoke()
        XCTAssertEqual(manager.current, .revoked)

        let loadedRevoked = try freshStore.load()
        XCTAssertEqual(loadedRevoked, .revoked)
    }

    // MARK: - 4. Pure Widening Save Failure

    func testPureWideningSaveFailureDoesNotAdmitPermissions() {
        let memStore = MemoryConsentStore(initial: .absent)
        memStore.failSaves = true

        let manager = ConsentManager(initial: .absent, storage: memStore)
        let wideningGrant = ConsentedPermissions(
            screenshot: true,
            webcam: true,
            keyboard: true,
            mouse: true,
            activeWindow: true,
            runningPrograms: true,
            blurScreenshots: false
        )

        XCTAssertThrowsError(try manager.update(.granted(wideningGrant))) { error in
            guard let consentErr = error as? ConsentError else {
                XCTFail("Expected ConsentError, got \(error)")
                return
            }
            if case .saveFailed = consentErr {
                // Expected
            } else {
                XCTFail("Expected saveFailed, got \(consentErr)")
            }
        }

        // Must remain absent! No permissions admitted on pure widening failure.
        XCTAssertEqual(manager.current, .absent)
        XCTAssertEqual(manager.effectivePermissions(project: CapturePermissions.all), ConsentedPermissions.allDisabled)
    }

    // MARK: - 5. Mixed Change: Narrowing Applied Before Blocked Save & Preserved on Error

    func testMixedChangeAppliesNarrowingBeforeBlockedSaveAndPreservesItOnSaveError() throws {
        let initialPerms = ConsentedPermissions(
            screenshot: true,
            webcam: true,
            keyboard: true,
            mouse: true,
            activeWindow: false,
            runningPrograms: false,
            blurScreenshots: false
        )

        let memStore = MemoryConsentStore(initial: .granted(initialPerms))
        let manager = ConsentManager(initial: .granted(initialPerms), storage: memStore)

        // Mixed update:
        // - Withdraws webcam and mouse (narrowing)
        // - Tightens blurScreenshots (false -> true)
        // - Adds activeWindow (expansion)
        let mixedPerms = ConsentedPermissions(
            screenshot: true,
            webcam: false,
            keyboard: true,
            mouse: false,
            activeWindow: true,
            runningPrograms: false,
            blurScreenshots: true
        )

        // Set barrier on save: assert in-memory state during save callback
        var verifiedDuringSave = false
        memStore.saveBarrier = { [weak manager] _ in
            guard let manager = manager else { return }
            // Conservative intersection must already be applied in memory!
            if case .granted(let p) = manager.current {
                XCTAssertTrue(p.screenshot)
                XCTAssertFalse(p.webcam, "Webcam must be withdrawn in-memory before save")
                XCTAssertTrue(p.keyboard)
                XCTAssertFalse(p.mouse, "Mouse must be withdrawn in-memory before save")
                XCTAssertFalse(p.activeWindow, "ActiveWindow expansion must NOT be admitted before save succeeds")
                XCTAssertTrue(p.blurScreenshots, "Blur tightening must be applied immediately as defense-in-depth")
                verifiedDuringSave = true
            } else {
                XCTFail("Expected granted interim state")
            }
            // Trigger failure
            throw ConsentError.saveFailed("Disk I/O failed during mixed update")
        }

        XCTAssertThrowsError(try manager.update(.granted(mixedPerms)))
        XCTAssertTrue(verifiedDuringSave)

        // After failure: conservative interim state must NOT be reverted to the old permissive grant!
        if case .granted(let pFinal) = manager.current {
            XCTAssertFalse(pFinal.webcam, "Withdrawn webcam must remain withdrawn after save failure")
            XCTAssertFalse(pFinal.mouse, "Withdrawn mouse must remain withdrawn after save failure")
            XCTAssertFalse(pFinal.activeWindow, "Unsaved activeWindow expansion must not be admitted")
            XCTAssertTrue(pFinal.blurScreenshots, "Tightened blur must remain enforced after save failure")
        } else {
            XCTFail("Expected granted interim state")
        }
    }

    // MARK: - 6. Identical Policy Update Does Not Advance Generation Ticket

    func testIdenticalPolicyDoesNotAdvanceGenerationTicket() throws {
        let memStore = MemoryConsentStore(initial: .absent)
        let manager = ConsentManager(initial: .absent, storage: memStore)
        let initialGen = manager.generation

        // Identical absent -> absent
        let changed = try manager.update(.absent)
        XCTAssertFalse(changed)
        XCTAssertEqual(manager.generation, initialGen)

        // Grant
        let grant = ConsentedPermissions(keyboard: true)
        let changedGrant = try manager.update(.granted(grant))
        XCTAssertTrue(changedGrant)
        let gen2 = manager.generation
        XCTAssertGreaterThan(gen2, initialGen)

        // Identical grant -> grant
        let changedSame = try manager.update(.granted(grant))
        XCTAssertFalse(changedSame)
        XCTAssertEqual(manager.generation, gen2)
    }
}
