import Foundation
import Darwin
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

    // MARK: - 7. macOS File Consent Store Hardened Storage Invariants

    func testPositiveRoundtripAndPrivateModesOnMac() throws {
        let tempDir = FileManager.default.temporaryDirectory
            .appendingPathComponent("consent-modes-\(UUID().uuidString.lowercased())", isDirectory: true)
        try FileManager.default.createDirectory(at: tempDir, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: tempDir) }

        let subDir = tempDir.appendingPathComponent("private_consent_dir", isDirectory: true)
        let fileURL = subDir.appendingPathComponent("consent.json")

        let store = FileConsentStore(fileURL: fileURL)
        XCTAssertFalse(FileConsentStore.exists(at: fileURL))

        let grant = ConsentedPermissions(keyboard: true, mouse: true)
        try store.save(.granted(grant))
        XCTAssertTrue(FileConsentStore.exists(at: fileURL))

        var dirStat = stat()
        XCTAssertEqual(lstat(subDir.path, &dirStat), 0)
        XCTAssertEqual(
            Int32(dirStat.st_mode) & 0o777,
            0o700,
            "Newly created dedicated directory must have private mode 0700"
        )

        var fileStat = stat()
        XCTAssertEqual(lstat(fileURL.path, &fileStat), 0)
        XCTAssertEqual(
            Int32(fileStat.st_mode) & 0o777,
            0o600,
            "Saved consent file must have private mode 0600 directly at creation"
        )
        XCTAssertEqual(
            fileStat.st_uid,
            geteuid(),
            "Saved consent file must be owned by current effective UID"
        )

        let loaded = try store.load()
        XCTAssertEqual(loaded, .granted(grant))

        try store.save(.revoked)
        let loadedRevoked = try store.load()
        XCTAssertEqual(loadedRevoked, .revoked)
    }

    func testTargetSymlinkAndDirectoryRefusalUnderPolicy() throws {
        let tempDir = FileManager.default.temporaryDirectory
            .appendingPathComponent("consent-refusal-\(UUID().uuidString.lowercased())", isDirectory: true)
        try FileManager.default.createDirectory(at: tempDir, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: tempDir) }

        // 1. Directory refusal
        let targetDir = tempDir.appendingPathComponent("blocker_directory.json", isDirectory: true)
        try FileManager.default.createDirectory(at: targetDir, withIntermediateDirectories: true)

        let storeDir = FileConsentStore(fileURL: targetDir)
        XCTAssertThrowsError(try storeDir.load()) { error in
            guard let consentErr = error as? ConsentError else {
                return XCTFail("Expected ConsentError, got \(error)")
            }
            if case .unsafeStorage = consentErr {
                // Expected
            } else {
                XCTFail("Expected unsafeStorage, got \(consentErr)")
            }
        }
        XCTAssertThrowsError(try storeDir.save(.absent)) { error in
            guard let consentErr = error as? ConsentError else {
                return XCTFail("Expected ConsentError, got \(error)")
            }
            if case .unsafeStorage = consentErr {
                // Expected
            } else {
                XCTFail("Expected unsafeStorage, got \(consentErr)")
            }
        }

        // 2. Symlink refusal
        let realTarget = tempDir.appendingPathComponent("real_consent.json")
        try Data("{\"state\": \"absent\"}".utf8).write(to: realTarget)

        let symlinkPath = tempDir.appendingPathComponent("symlink_consent.json")
        XCTAssertEqual(Darwin.symlink(realTarget.path, symlinkPath.path), 0)

        let storeSymlink = FileConsentStore(fileURL: symlinkPath)
        XCTAssertThrowsError(try storeSymlink.load()) { error in
            guard let consentErr = error as? ConsentError else {
                return XCTFail("Expected ConsentError, got \(error)")
            }
            if case .unsafeStorage = consentErr {
                // Expected
            } else {
                XCTFail("Expected unsafeStorage, got \(consentErr)")
            }
        }

        let grant = ConsentedPermissions(screenshot: true)
        XCTAssertThrowsError(try storeSymlink.save(.granted(grant))) { error in
            guard let consentErr = error as? ConsentError else {
                return XCTFail("Expected ConsentError, got \(error)")
            }
            if case .unsafeStorage = consentErr {
                // Expected
            } else {
                XCTFail("Expected unsafeStorage, got \(consentErr)")
            }
        }

        // Legitimate target must remain untouched
        let realContent = try String(contentsOf: realTarget, encoding: .utf8)
        XCTAssertEqual(realContent, "{\"state\": \"absent\"}")
    }

    func testCandidateCollisionFallbackSaveSucceeds() throws {
        let tempDir = FileManager.default.temporaryDirectory
            .appendingPathComponent("consent-collision-\(UUID().uuidString.lowercased())", isDirectory: true)
        try FileManager.default.createDirectory(at: tempDir, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: tempDir) }

        let fileURL = tempDir.appendingPathComponent("consent.json")

        // Precreate candidate 0 regular file with sentinel content
        let cand0 = tempDir.appendingPathComponent(".consent.json.tmp.cand_collision.0")
        try Data("CAND_0_SENTINEL_CONTENT".utf8).write(to: cand0)

        // Precreate victim sentinel file and candidate 1 symlink pointing to victim
        let victim = tempDir.appendingPathComponent("victim_sentinel.txt")
        try Data("VICTIM_SENTINEL_CONTENT".utf8).write(to: victim)

        let cand1 = tempDir.appendingPathComponent(".consent.json.tmp.cand_collision.1")
        XCTAssertEqual(Darwin.symlink(victim.path, cand1.path), 0)

        let hooks = SaveHooks(candidateNamer: { stem, attempt in
            ".\(stem).tmp.cand_collision.\(attempt)"
        })

        let store = FileConsentStore(fileURL: fileURL, hooks: hooks)
        let grant = ConsentedPermissions(screenshot: true, keyboard: true)

        try store.save(.granted(grant))

        // Assert candidate 0 sentinel remains unchanged
        let cand0Data = try Data(contentsOf: cand0)
        XCTAssertEqual(String(decoding: cand0Data, as: UTF8.self), "CAND_0_SENTINEL_CONTENT")

        // Assert candidate 1 remains a symlink and victim content was not truncated or modified
        var cand1Stat = stat()
        XCTAssertEqual(lstat(cand1.path, &cand1Stat), 0)
        XCTAssertEqual(
            Int32(cand1Stat.st_mode) & Int32(S_IFMT),
            Int32(S_IFLNK),
            "Candidate 1 must remain a symlink"
        )
        let victimData = try Data(contentsOf: victim)
        XCTAssertEqual(String(decoding: victimData, as: UTF8.self), "VICTIM_SENTINEL_CONTENT")

        // Assert fallback save succeeded and loads granted permissions
        let loaded = try store.load()
        XCTAssertEqual(loaded, .granted(grant))
    }

    func testCandidateCollisionExhaustionPreservesCandidatesAndValidConsent() throws {
        let tempDir = FileManager.default.temporaryDirectory
            .appendingPathComponent("consent-exhaust-\(UUID().uuidString.lowercased())", isDirectory: true)
        try FileManager.default.createDirectory(at: tempDir, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: tempDir) }

        let fileURL = tempDir.appendingPathComponent("consent.json")

        // Initial valid consent saved
        let initialStore = FileConsentStore(fileURL: fileURL)
        let validPerms = ConsentedPermissions(keyboard: true)
        try initialStore.save(.granted(validPerms))
        let validBytesBefore = try Data(contentsOf: fileURL)

        // Precreate all 64 candidates
        var candidatePaths: [URL] = []
        for i in 0..<64 {
            let p = tempDir.appendingPathComponent(".consent.json.tmp.exhaust.\(i).sentinel")
            try Data("EXHAUST_SENTINEL_\(i)".utf8).write(to: p)
            candidatePaths.append(p)
        }

        let hooks = SaveHooks(candidateNamer: { stem, attempt in
            ".\(stem).tmp.exhaust.\(attempt).sentinel"
        })

        let storeExhaust = FileConsentStore(fileURL: fileURL, hooks: hooks)
        XCTAssertThrowsError(try storeExhaust.save(.revoked)) { error in
            guard let consentErr = error as? ConsentError else {
                return XCTFail("Expected ConsentError, got \(error)")
            }
            if case .saveFailed(let msg) = consentErr {
                XCTAssertTrue(msg.contains("Exceeded maximum collision retries"), "Expected retry exhaustion, got: \(msg)")
            } else {
                XCTFail("Expected saveFailed, got \(consentErr)")
            }
        }

        // Assert all 64 candidate files remain unmodified
        for (i, p) in candidatePaths.enumerated() {
            let data = try Data(contentsOf: p)
            XCTAssertEqual(String(decoding: data, as: UTF8.self), "EXHAUST_SENTINEL_\(i)")
        }

        // Assert existing valid consent file remains untouched
        let validBytesAfter = try Data(contentsOf: fileURL)
        XCTAssertEqual(validBytesAfter, validBytesBefore)
        let loaded = try initialStore.load()
        XCTAssertEqual(loaded, .granted(validPerms))
    }

    func testPostOwnershipFailureRetainsValidRecordAndCleansTemp() throws {
        let tempDir = FileManager.default.temporaryDirectory
            .appendingPathComponent("consent-post-write-fail-\(UUID().uuidString.lowercased())", isDirectory: true)
        try FileManager.default.createDirectory(at: tempDir, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: tempDir) }

        let fileURL = tempDir.appendingPathComponent("consent.json")

        let initialPerms = ConsentedPermissions(keyboard: true)
        let initialStore = FileConsentStore(fileURL: fileURL)
        try initialStore.save(.granted(initialPerms))
        let validBytesBefore = try Data(contentsOf: fileURL)

        let unrelatedFile = tempDir.appendingPathComponent("unrelated_data.bin")
        try Data("UNRELATED_KEEP_SAFE".utf8).write(to: unrelatedFile)

        let unrelatedTmp = tempDir.appendingPathComponent(".consent.json.tmp.other_worker")
        try Data("OTHER_WORKER_KEEP_SAFE".utf8).write(to: unrelatedTmp)

        final class Box: @unchecked Sendable {
            var path: String?
        }
        let box = Box()

        let hooks = SaveHooks(failAfterOpen: { tempPath in
            box.path = tempPath
            throw ConsentError.saveFailed("Injected post-write failure")
        })

        let store = FileConsentStore(fileURL: fileURL, hooks: hooks)
        let manager = ConsentManager(initial: .granted(initialPerms), storage: store)

        var expanded = initialPerms
        expanded.screenshot = true

        XCTAssertThrowsError(try manager.update(.granted(expanded)))

        // Manager must not widen in-memory when disk persistence fails
        XCTAssertEqual(manager.current.permissions.screenshot, false)
        XCTAssertEqual(manager.current.permissions.keyboard, true)

        // Owned temp file must be cleaned up
        guard let ownedTemp = box.path else {
            return XCTFail("Hook did not capture owned temp path")
        }
        XCTAssertFalse(FileManager.default.fileExists(atPath: ownedTemp))

        // Existing valid consent file remains untouched
        let validBytesAfter = try Data(contentsOf: fileURL)
        XCTAssertEqual(validBytesAfter, validBytesBefore)
        let loaded = try initialStore.load()
        XCTAssertEqual(loaded, .granted(initialPerms))

        // Unrelated files remain untouched
        XCTAssertEqual(try String(contentsOf: unrelatedFile, encoding: .utf8), "UNRELATED_KEEP_SAFE")
        XCTAssertEqual(try String(contentsOf: unrelatedTmp, encoding: .utf8), "OTHER_WORKER_KEEP_SAFE")
    }

    func testRenameRefusalRetainsValidRecordAndCleansTemp() throws {
        let tempDir = FileManager.default.temporaryDirectory
            .appendingPathComponent("consent-rename-fail-\(UUID().uuidString.lowercased())", isDirectory: true)
        try FileManager.default.createDirectory(at: tempDir, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: tempDir) }

        let fileURL = tempDir.appendingPathComponent("consent.json")

        let initialPerms = ConsentedPermissions(mouse: true)
        let initialStore = FileConsentStore(fileURL: fileURL)
        try initialStore.save(.granted(initialPerms))
        let validBytesBefore = try Data(contentsOf: fileURL)

        let unrelatedFile = tempDir.appendingPathComponent("unrelated_data.bin")
        try Data("UNRELATED_KEEP_SAFE".utf8).write(to: unrelatedFile)

        let unrelatedTmp = tempDir.appendingPathComponent(".consent.json.tmp.other_worker")
        try Data("OTHER_WORKER_KEEP_SAFE".utf8).write(to: unrelatedTmp)

        final class Box: @unchecked Sendable {
            var path: String?
        }
        let box = Box()

        let hooks = SaveHooks(failRename: { tempPath, _ in
            box.path = tempPath
            throw ConsentError.saveFailed("Injected atomic rename failure")
        })

        let store = FileConsentStore(fileURL: fileURL, hooks: hooks)
        let manager = ConsentManager(initial: .granted(initialPerms), storage: store)

        var expanded = initialPerms
        expanded.activeWindow = true

        XCTAssertThrowsError(try manager.update(.granted(expanded)))

        // Manager must not widen in-memory when rename fails
        XCTAssertEqual(manager.current.permissions.activeWindow, false)
        XCTAssertEqual(manager.current.permissions.mouse, true)

        // Owned temp file must be cleaned up
        guard let ownedTemp = box.path else {
            return XCTFail("Hook did not capture owned temp path")
        }
        XCTAssertFalse(FileManager.default.fileExists(atPath: ownedTemp))

        // Existing valid consent file remains untouched
        let validBytesAfter = try Data(contentsOf: fileURL)
        XCTAssertEqual(validBytesAfter, validBytesBefore)
        let loaded = try initialStore.load()
        XCTAssertEqual(loaded, .granted(initialPerms))

        // Unrelated files remain untouched
        XCTAssertEqual(try String(contentsOf: unrelatedFile, encoding: .utf8), "UNRELATED_KEEP_SAFE")
        XCTAssertEqual(try String(contentsOf: unrelatedTmp, encoding: .utf8), "OTHER_WORKER_KEEP_SAFE")
    }

    func testUnsafePermissionsLoadFailsClosedOnMac() throws {
        let tempDir = FileManager.default.temporaryDirectory
            .appendingPathComponent("consent-unsafe-perm-\(UUID().uuidString.lowercased())", isDirectory: true)
        try FileManager.default.createDirectory(at: tempDir, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: tempDir) }

        let fileURL = tempDir.appendingPathComponent("unsafe_mode.json")
        let grant = ConsentedPermissions(screenshot: true)
        let encoder = JSONEncoder()
        encoder.outputFormatting = [.prettyPrinted]
        let data = try encoder.encode(LocalConsent.granted(grant))
        try data.write(to: fileURL)

        XCTAssertEqual(Darwin.chmod(fileURL.path, mode_t(0o666)), 0)

        let store = FileConsentStore(fileURL: fileURL)
        XCTAssertThrowsError(try store.load()) { error in
            guard let consentErr = error as? ConsentError else {
                return XCTFail("Expected ConsentError, got \(error)")
            }
            if case .unsafeStorage = consentErr {
                // Expected
            } else {
                XCTFail("Expected unsafeStorage, got \(consentErr)")
            }
        }

        let (manager, error) = ConsentManager.loadOrFailClosed(store: store)
        XCTAssertNotNil(error)
        XCTAssertEqual(
            manager.current,
            .absent,
            "ConsentManager must fail closed to .absent on unsafe file permissions"
        )
    }
}
