import Foundation

/// Why a capture service delivered nothing. A failed capture never costs the
/// block: the worker logs it and records the block without that capture
/// (CONTRACT §1 "Captures never cost time data").
public enum CaptureError: Error, Equatable, Sendable, CustomStringConvertible {
    /// The privacy permission the capture needs has not been granted.
    case permissionMissing(PrivacyPermission)
    /// The app bundle lacks this Info.plist usage description, without which
    /// macOS ends the process as soon as the device is touched.
    case usageDescriptionMissing(String)
    /// There is nothing to capture (no display, no camera).
    case unavailable(String)
    /// A previous capture of the same kind is still running.
    case busy
    /// The device did not deliver in time.
    case timedOut
    /// The image does not fit the server's size limit even at the lowest
    /// quality tried.
    case tooLarge
    /// The system refused or failed the capture.
    case failed(String)

    public var description: String {
        switch self {
        case .permissionMissing(let permission):
            return "\(permission.title) permission not granted"
        case .usageDescriptionMissing(let key):
            return "the app bundle has no \(key)"
        case .unavailable(let what):
            return what
        case .busy:
            return "a previous capture is still running"
        case .timedOut:
            return "the device did not deliver in time"
        case .tooLarge:
            return "the image is larger than the server accepts"
        case .failed(let reason):
            return reason
        }
    }
}

// The capture services the worker drives. The app implements them with
// ScreenCaptureKit, AVFoundation, Accessibility and NSWorkspace
// (`Sources/DosiTracker/System`); tests can stand in fakes, so the worker's
// rules stay testable without permissions or hardware.

/// Takes the screen capture of a block (the app's `ScreenCapturer`).
public protocol ScreenCapturing: AnyObject, Sendable {
    /// Capture the display that holds the window the user works in, as a
    /// JPEG at most ``CaptureLimits/maxScreenWidth`` wide with a thumbnail at
    /// most ``CaptureLimits/maxThumbnailWidth`` wide, both blurred beyond
    /// reading when `blur` (CONTRACT §4 "Screenshots").
    func capture(blur: Bool) async throws -> ScreenCapture
}

/// Takes the webcam photo of a block (the app's `WebcamCapturer`).
public protocol WebcamCapturing: AnyObject, Sendable {
    /// One JPEG photo at most ``CaptureLimits/maxWebcamWidth`` wide, taken
    /// after the camera warmed up (``CaptureLimits/webcamWarmUp``), or an
    /// error after ``CaptureLimits/webcamTimeout``.
    func capturePhoto() async throws -> Data
}

/// The app and window in front (the app's `ActiveAppMonitor`).
public protocol ForegroundWindowSource: AnyObject, Sendable {
    /// The app and window in front now; nil when no app is.
    func currentWindow() async -> ForegroundWindow?
    /// The name of the app in front, without reading any window (cheap; the
    /// heartbeat's `currentApp`).
    var frontmostAppName: String? { get }
}

/// The programs running now (the app's `RunningApps`).
public protocol RunningProgramsSource: AnyObject, Sendable {
    /// App names only, never window titles (CONTRACT §1 `runningPrograms`).
    func runningProgramNames() -> [String]
}

/// The system's own idle clock (the app's `IdleMonitor`).
public protocol SystemIdleSource: AnyObject, Sendable {
    /// When the system last saw keyboard or mouse input; nil when it cannot
    /// say.
    func lastInputAt(now: Date) -> Date?
}
