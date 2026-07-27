import Foundation

/// A project the current user may track against, as returned by
/// `GET /api/app/project/my-projects` (plain JSON array).
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
    /// Client-generated idempotency key — the backend deduplicates retries on it
    /// and rejects uploads without one.
    let clientActivityId: String
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
