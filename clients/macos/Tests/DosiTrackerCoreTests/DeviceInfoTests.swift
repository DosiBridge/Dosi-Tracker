import XCTest
@testable import DosiTrackerCore

/// Device context sent with blocks and heartbeats (CONTRACT §1, §3).
final class DeviceInfoTests: XCTestCase {
    func testTheOSVersionNamesTheReleaseAndBuild() {
        XCTAssertEqual(DeviceInfo.osVersionString(versionString: "Version 14.5 (Build 23F79)",
                                                  version: OperatingSystemVersion(majorVersion: 14, minorVersion: 5, patchVersion: 0)),
                       "macOS 14.5 (23F79)")
        XCTAssertEqual(DeviceInfo.osVersionString(versionString: "Version 14.6.1 (Build 23G93)",
                                                  version: OperatingSystemVersion(majorVersion: 14, minorVersion: 6, patchVersion: 1)),
                       "macOS 14.6.1 (23G93)")
        XCTAssertEqual(DeviceInfo.osVersionString(versionString: "unexpected",
                                                  version: OperatingSystemVersion(majorVersion: 15, minorVersion: 0, patchVersion: 0)),
                       "macOS 15.0")
        XCTAssertTrue(DeviceInfo.currentOSVersion().hasPrefix("macOS "))
        XCTAssertLessThanOrEqual(DeviceInfo.currentOSVersion().count, 64)
    }

    func testTheHostNameLosesItsLocalSuffix() {
        XCTAssertEqual(DeviceInfo.cleanHostName("Anas-MacBook-Pro.local"), "Anas-MacBook-Pro")
        XCTAssertEqual(DeviceInfo.cleanHostName("build-07.corp.example.com"), "build-07.corp.example.com")
        XCTAssertEqual(DeviceInfo.cleanHostName("   "), "Mac")
        XCTAssertEqual(DeviceInfo.cleanHostName(".local"), "Mac")
        XCTAssertFalse(DeviceInfo.hostName().isEmpty)
    }

    func testOnlyAValidUUIDIsSentAsTheDeviceId() {
        XCTAssertEqual(DeviceInfo.normalizedDeviceId(" 5B0C1B9E-8F2C-4A43-9A51-0C7F6F2D9B11 "),
                       "5b0c1b9e-8f2c-4a43-9a51-0c7f6f2d9b11")
        XCTAssertNil(DeviceInfo.normalizedDeviceId("my-mac"))
        XCTAssertNil(DeviceInfo.normalizedDeviceId(""))

        var payload = ActivityPayload(projectId: "p", startedAt: at(t0), endedAt: at(t0 + 60))
        var device = sampleDevice()
        device.deviceId = "not a uuid"
        payload.applyDeviceContext(device, zone: BlockZone(timeZone: "UTC", utcOffsetMinutes: 0))
        XCTAssertNil(payload.deviceId, "the server would refuse the whole block")
        XCTAssertEqual(payload.platform, "macos")
    }

    func testTimeZonesAreSentAsIANAIdsWithinTheServersLimit() {
        XCTAssertEqual(DeviceInfo.wireTimeZone(" Asia/Dhaka "), "Asia/Dhaka")
        XCTAssertNil(DeviceInfo.wireTimeZone(""))
        XCTAssertNil(DeviceInfo.wireTimeZone(String(repeating: "Z", count: 65)), "a cut-off id would name no zone")
        XCTAssertEqual(BlockZone(TimeZone(identifier: "Asia/Dhaka")!, at: at(t0)),
                       BlockZone(timeZone: "Asia/Dhaka", utcOffsetMinutes: 360))
    }

    func testUTCOffsetsAreInMinutesAndFollowTheZone() throws {
        let summer = utc("2026-07-14T09:00:00Z")
        let winter = utc("2026-01-14T09:00:00Z")
        XCTAssertEqual(DeviceInfo.utcOffsetMinutes(TimeZone(secondsFromGMT: 6 * 3600)!, at: summer), 360)
        XCTAssertEqual(DeviceInfo.utcOffsetMinutes(TimeZone(secondsFromGMT: -(5 * 3600 + 1800))!, at: summer), -330)
        XCTAssertEqual(DeviceInfo.utcOffsetMinutes(TimeZone(secondsFromGMT: 0)!, at: summer), 0)
        let newYork = try XCTUnwrap(TimeZone(identifier: "America/New_York"))
        XCTAssertEqual(DeviceInfo.utcOffsetMinutes(newYork, at: summer), -240, "daylight saving time")
        XCTAssertEqual(DeviceInfo.utcOffsetMinutes(newYork, at: winter), -300)
        XCTAssertTrue((-840...840).contains(DeviceInfo.utcOffsetMinutes(.current, at: Date())))
    }

    func testDeviceContextIsCutToTheServersLengths() {
        let device = DeviceInfo(deviceId: "5b0c1b9e-8f2c-4a43-9a51-0c7f6f2d9b11",
                                deviceName: String(repeating: "N", count: 300),
                                osVersion: String(repeating: "O", count: 100),
                                agentVersion: String(repeating: "1", count: 40))
        var payload = ActivityPayload(projectId: "p", startedAt: at(t0), endedAt: at(t0 + 60))
        payload.applyDeviceContext(device, zone: BlockZone(timeZone: "Asia/Dhaka", utcOffsetMinutes: 9_999))
        XCTAssertEqual(payload.deviceName?.count, 128)
        XCTAssertEqual(payload.osVersion?.count, 64)
        XCTAssertEqual(payload.agentVersion?.count, 32)
        XCTAssertEqual(payload.utcOffsetMinutes, 840)
    }

    func testTheCurrentDeviceIsDescribed() {
        let device = DeviceInfo.current(deviceId: "5b0c1b9e-8f2c-4a43-9a51-0c7f6f2d9b11", deviceName: "  ")
        XCTAssertFalse(device.deviceName.isEmpty, "a blank name falls back to the host name")
        XCTAssertEqual(device.platform, "macos")
        XCTAssertEqual(device.agentVersion, AgentInfo.fallbackVersion, "the test runner is not the app bundle")
        XCTAssertEqual(DeviceInfo.current(deviceId: "x", deviceName: "Ana's Mac").deviceName, "Ana's Mac")
        XCTAssertEqual(AgentInfo.userAgent(version: "0.2.0", osVersion: "macOS 14.5 (23F79)"),
                       "DosiTracker/0.2.0 (macOS 14.5 (23F79))")
    }

    func testCaptureSizesKeepTheAspectAndNeverUpscale() {
        XCTAssertTrue(CaptureLimits.scaledSize(width: 3840, height: 2160, maxWidth: 1920) == (1920, 1080))
        XCTAssertTrue(CaptureLimits.scaledSize(width: 3024, height: 1964, maxWidth: 1920) == (1920, 1247))
        XCTAssertTrue(CaptureLimits.scaledSize(width: 1440, height: 900, maxWidth: 1920) == (1440, 900))
        XCTAssertTrue(CaptureLimits.scaledSize(width: 1920, height: 1080, maxWidth: 480) == (480, 270))
        XCTAssertTrue(CaptureLimits.scaledSize(width: 5000, height: 1, maxWidth: 480) == (480, 1))
        XCTAssertTrue(CaptureLimits.scaledSize(width: 0, height: 0, maxWidth: 480) == (0, 0))
    }
}
