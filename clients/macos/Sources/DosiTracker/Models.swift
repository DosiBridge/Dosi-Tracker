import Foundation

struct AuthSession: Codable {
    let accessToken: String
    let userId: String
    let displayName: String
    let expiresAt: Date
}

struct Project: Codable {
    let id: String
    let title: String
    let intervalMinutes: Int
    let allowScreenshot: Bool
    let allowWebcam: Bool
    let allowKeyboard: Bool
    let allowMouse: Bool
    let allowActiveWindow: Bool
    let allowRunningPrograms: Bool
}

struct WindowInfo: Codable {
    let appName: String
    let windowTitle: String
}

/// One tracked time block uploaded to the backend.
struct Activity: Codable {
    let projectId: String
    let startedAt: Date
    let endedAt: Date
    var description: String?
    var mouseClicks: Int
    var keyboardHits: Int
    var activeWindows: [WindowInfo]
    var runningPrograms: [WindowInfo]
    /// Base64-encoded PNG.
    var screenshotPngBase64: String?
    /// Base64-encoded JPEG.
    var webcamJpgBase64: String?
}
