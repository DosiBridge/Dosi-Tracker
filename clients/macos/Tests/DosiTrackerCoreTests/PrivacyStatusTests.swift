import XCTest
@testable import DosiTrackerCore

/// The permissions summary the window and the menu show.
final class PrivacyStatusTests: XCTestCase {
    private func status(screen: PermissionState = .granted, input: PermissionState = .granted,
                        accessibility: PermissionState = .granted, camera: PermissionState = .granted) -> PrivacyStatus {
        PrivacyStatus(screenRecording: screen, inputMonitoring: input, accessibility: accessibility, camera: camera)
    }

    private func policy(_ change: (inout CapturePermissions) -> Void) -> CapturePermissions {
        var permissions = CapturePermissions.nothing
        change(&permissions)
        return permissions
    }

    func testNothingIsMissingWhenEverythingIsGranted() {
        XCTAssertEqual(status().missing(for: .all), [])
    }

    func testEachCaptureNeedsItsPermission() {
        XCTAssertEqual(status(screen: .notGranted).missing(for: policy { $0.screenshot = true }), [.screenRecording])
        XCTAssertEqual(status(input: .denied).missing(for: policy { $0.keyboard = true }), [.inputMonitoring])
        XCTAssertEqual(status(input: .notDetermined).missing(for: policy { $0.mouse = true }), [.inputMonitoring])
        XCTAssertEqual(status(camera: .restricted).missing(for: policy { $0.webcam = true }), [.camera])
        // App names and running programs need nothing.
        let nothingGranted = status(screen: .notGranted, input: .denied, accessibility: .notGranted, camera: .denied)
        XCTAssertEqual(nothingGranted.missing(for: policy { $0.runningPrograms = true }), [])
        XCTAssertEqual(nothingGranted.missing(for: .nothing), [])
    }

    func testWindowTitlesNeedAccessibilityUnlessScreenRecordingRevealsThem() {
        let titles = policy { $0.activeWindow = true }
        XCTAssertEqual(status(screen: .notGranted, accessibility: .notGranted).missing(for: titles), [.accessibility])
        XCTAssertEqual(status(screen: .granted, accessibility: .notGranted).missing(for: titles), [])
        XCTAssertEqual(status(screen: .notGranted, accessibility: .granted).missing(for: titles), [])
        XCTAssertFalse(status(screen: .notGranted, accessibility: .notGranted).canReadWindowTitles)
    }

    func testMissingPermissionsAreListedInSettingsOrder() {
        let none = status(screen: .notGranted, input: .notDetermined, accessibility: .notGranted, camera: .notDetermined)
        XCTAssertEqual(none.missing(for: .all), [.screenRecording, .inputMonitoring, .accessibility, .camera])
        // The local default never records the webcam.
        XCTAssertEqual(none.missing(for: .localDefault), [.screenRecording, .inputMonitoring, .accessibility])
        for permission in PrivacyPermission.allCases {
            XCTAssertFalse(none.state(of: permission).isGranted, "\(permission)")
            XCTAssertTrue(status().state(of: permission).isGranted, "\(permission)")
        }
    }

    func testSettingsPanesOpenPrivacyAndSecurityAtThePermission() {
        let pane = "x-apple.systempreferences:com.apple.preference.security?"
        XCTAssertEqual(PrivacyPermission.screenRecording.settingsURL.absoluteString, pane + "Privacy_ScreenCapture")
        XCTAssertEqual(PrivacyPermission.inputMonitoring.settingsURL.absoluteString, pane + "Privacy_ListenEvent")
        XCTAssertEqual(PrivacyPermission.accessibility.settingsURL.absoluteString, pane + "Privacy_Accessibility")
        XCTAssertEqual(PrivacyPermission.camera.settingsURL.absoluteString, pane + "Privacy_Camera")
        XCTAssertEqual(PrivacyPermission.screenRecording.title, "Screen Recording")
        XCTAssertEqual(PrivacyPermission.inputMonitoring.title, "Input Monitoring")
        for permission in PrivacyPermission.allCases {
            XCTAssertEqual(permission.settingsURL.scheme, "x-apple.systempreferences")
            XCTAssertFalse(permission.purpose.isEmpty)
        }
    }
}
