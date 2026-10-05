import AppKit
import ApplicationServices
import AVFoundation
import CoreGraphics
import DosiTrackerCore
import Foundation
// IOHIDCheckAccess / IOHIDRequestAccess and their request and access types
// (macOS 10.15+) are declared among IOKit's HID event-system headers.
import IOKit.hid
import IOKit.hidsystem

/// The privacy permissions the capture services need: their state for the
/// UI (``status()``), the requests that show the system prompts, and the
/// System Settings panes that grant them.
///
/// macOS shows each prompt only once per app; after a refusal only System
/// Settings can grant the permission, so the UI should offer
/// ``openSettings(for:)`` whenever a permission is missing. Screen Recording
/// granted while the app runs may only take effect after a relaunch.
///
/// Thread-safe: the state reads may be made from any thread (they are cheap);
/// the requests return at once and ``openSettings(for:)`` hops to the main
/// thread.
public enum Permissions {
    /// The value of `kAXTrustedCheckOptionPrompt`, spelled out because its
    /// imported type has varied between SDKs.
    static let accessibilityPromptOption = "AXTrustedCheckOptionPrompt"

    /// All four permissions as they stand now.
    public static func status() -> PrivacyStatus {
        PrivacyStatus(
            screenRecording: screenRecordingState(),
            inputMonitoring: inputMonitoringState(),
            accessibility: accessibilityState(),
            camera: cameraState()
        )
    }

    /// Screen Recording (`CGPreflightScreenCaptureAccess`, which never
    /// prompts). The system only answers yes or no.
    public static func screenRecordingState() -> PermissionState {
        CGPreflightScreenCaptureAccess() ? .granted : .notGranted
    }

    /// Input Monitoring, which listening to the keyboard and mouse needs
    /// (`IOHIDCheckAccess` for listen events). The IOKit and Core Graphics
    /// checks have been seen to disagree, so either one saying yes counts.
    public static func inputMonitoringState() -> PermissionState {
        let access = IOHIDCheckAccess(kIOHIDRequestTypeListenEvent)
        if access == kIOHIDAccessTypeGranted || CGPreflightListenEventAccess() {
            return .granted
        }
        if access == kIOHIDAccessTypeDenied {
            return .denied
        }
        return .notDetermined
    }

    /// Accessibility (`AXIsProcessTrusted`), for the focused window's title.
    /// The system only answers yes or no.
    public static func accessibilityState() -> PermissionState {
        AXIsProcessTrusted() ? .granted : .notGranted
    }

    /// Camera access.
    public static func cameraState() -> PermissionState {
        switch AVCaptureDevice.authorizationStatus(for: .video) {
        case .authorized:
            return .granted
        case .denied:
            return .denied
        case .restricted:
            return .restricted
        case .notDetermined:
            return .notDetermined
        @unknown default:
            return .notGranted
        }
    }

    // MARK: Requests

    /// Ask for Screen Recording: the system prompt the first time, nothing
    /// after that. Returns whether it is granted now.
    @discardableResult
    public static func requestScreenRecording() -> Bool {
        CGRequestScreenCaptureAccess()
    }

    /// Ask for Input Monitoring: the system prompt the first time, nothing
    /// after that. Returns whether it is granted now; ``InputMonitor`` picks a
    /// later grant up by itself.
    @discardableResult
    public static func requestInputMonitoring() -> Bool {
        IOHIDRequestAccess(kIOHIDRequestTypeListenEvent)
    }

    /// Ask for Accessibility: the system dialog pointing to System Settings.
    /// Returns whether it is granted now.
    @discardableResult
    public static func requestAccessibility() -> Bool {
        AXIsProcessTrustedWithOptions([accessibilityPromptOption: true] as CFDictionary)
    }

    /// Ask for camera access (see ``WebcamCapturer/requestAccess()``).
    public static func requestCamera() async -> Bool {
        await WebcamCapturer.requestAccess()
    }

    /// Open the pane of System Settings → Privacy & Security that grants
    /// `permission`.
    public static func openSettings(for permission: PrivacyPermission) {
        let url = permission.settingsURL
        if Thread.isMainThread {
            openSettingsURL(url)
        } else {
            DispatchQueue.main.async {
                Permissions.openSettingsURL(url)
            }
        }
    }

    private static func openSettingsURL(_ url: URL) {
        if !NSWorkspace.shared.open(url) {
            CoreLog.warning("permissions", "could not open System Settings at \(url.absoluteString)")
        }
    }
}
