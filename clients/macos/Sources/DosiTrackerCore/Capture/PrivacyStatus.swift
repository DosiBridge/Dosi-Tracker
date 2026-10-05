import Foundation

/// A macOS privacy permission the agent uses (System Settings → Privacy &
/// Security). macOS ties each grant to the app's code signature.
public enum PrivacyPermission: String, CaseIterable, Sendable {
    /// Screen captures; also other apps' window titles when Accessibility is
    /// not granted.
    case screenRecording
    /// Keyboard and mouse activity (the event tap).
    case inputMonitoring
    /// The title of the focused window.
    case accessibility
    /// Webcam photos.
    case camera

    /// The name System Settings shows.
    public var title: String {
        switch self {
        case .screenRecording:
            return "Screen Recording"
        case .inputMonitoring:
            return "Input Monitoring"
        case .accessibility:
            return "Accessibility"
        case .camera:
            return "Camera"
        }
    }

    /// What the agent uses it for, worded for the permissions list.
    public var purpose: String {
        switch self {
        case .screenRecording:
            return "Screen captures"
        case .inputMonitoring:
            return "Keyboard and mouse activity (counts only, never what is typed)"
        case .accessibility:
            return "The title of the window you work in"
        case .camera:
            return "Webcam photos"
        }
    }

    /// The anchor of its pane in Privacy & Security.
    public var settingsAnchor: String {
        switch self {
        case .screenRecording:
            return "Privacy_ScreenCapture"
        case .inputMonitoring:
            return "Privacy_ListenEvent"
        case .accessibility:
            return "Privacy_Accessibility"
        case .camera:
            return "Privacy_Camera"
        }
    }

    /// The System Settings pane that grants it (an `x-apple.systempreferences`
    /// URL, which System Settings on macOS 13 and later still opens).
    public var settingsURL: URL {
        // The text is a fixed, valid URL for every case.
        URL(string: "x-apple.systempreferences:com.apple.preference.security?\(settingsAnchor)")!
    }
}

/// Where a permission stands.
public enum PermissionState: String, Sendable {
    case granted
    /// Not granted; the system does not say whether the user was ever asked
    /// (Screen Recording and Accessibility only answer yes or no).
    case notGranted
    /// The user has not been asked yet.
    case notDetermined
    /// The user refused, or turned it off in System Settings.
    case denied
    /// A configuration profile or parental controls forbid it.
    case restricted

    public var isGranted: Bool { self == .granted }
}

/// The privacy permissions as they stand, for the window's permissions list
/// and the menu's warnings. Read it again whenever it is shown: the user can
/// change any of them in System Settings at any time.
public struct PrivacyStatus: Equatable, Sendable {
    public var screenRecording: PermissionState
    public var inputMonitoring: PermissionState
    public var accessibility: PermissionState
    public var camera: PermissionState

    public init(screenRecording: PermissionState, inputMonitoring: PermissionState, accessibility: PermissionState,
                camera: PermissionState) {
        self.screenRecording = screenRecording
        self.inputMonitoring = inputMonitoring
        self.accessibility = accessibility
        self.camera = camera
    }

    public func state(of permission: PrivacyPermission) -> PermissionState {
        switch permission {
        case .screenRecording:
            return screenRecording
        case .inputMonitoring:
            return inputMonitoring
        case .accessibility:
            return accessibility
        case .camera:
            return camera
        }
    }

    /// Window titles can be read: the focused window's through Accessibility,
    /// or the front window's through Screen Recording.
    public var canReadWindowTitles: Bool { accessibility.isGranted || screenRecording.isGranted }

    /// The permissions the capture `policy` (what the project records) needs
    /// but lacks, in ``PrivacyPermission/allCases`` order: screen captures need
    /// Screen Recording, keyboard or mouse activity Input Monitoring, window
    /// titles Accessibility (unless Screen Recording already reveals them),
    /// and webcam photos the camera. App names and running programs need
    /// nothing.
    public func missing(for policy: CapturePermissions) -> [PrivacyPermission] {
        var missing: [PrivacyPermission] = []
        if policy.screenshot && !screenRecording.isGranted {
            missing.append(.screenRecording)
        }
        if policy.measuresInput && !inputMonitoring.isGranted {
            missing.append(.inputMonitoring)
        }
        if policy.activeWindow && !canReadWindowTitles {
            missing.append(.accessibility)
        }
        if policy.webcam && !camera.isGranted {
            missing.append(.camera)
        }
        return missing
    }
}
