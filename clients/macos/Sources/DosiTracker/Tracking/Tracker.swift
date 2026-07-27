import Foundation

/// Collects one activity snapshot per interval. Input listening is event-driven
/// (see InputMonitor); heavy captures run once per interval, then the agent
/// sleeps — keeping idle CPU near zero.
///
/// Capture permissions are passed per snapshot because the effective set
/// (server-side project settings ∧ local config) may only become known after
/// the first successful sync with the backend.
final class Tracker {
    private let input = InputMonitor()
    private let webcam = WebcamCapturer()

    init() {
        input.start()
    }

    func snapshot(perms: CapturePermissions, projectId: String, startedAt: Date) async -> Activity {
        let (keyboardHits, mouseClicks) = input.take()

        let activeWindows = perms.activeWindow
            ? [ActiveWindowMonitor.current()].compactMap { $0 }
            : []

        let runningPrograms = perms.runningPrograms
            ? ActiveWindowMonitor.runningPrograms()
            : []

        var screenshot: String?
        if perms.screenshot {
            screenshot = try? await ScreenshotCapturer.capturePngBase64()
        }

        var webcamImage: String?
        if perms.webcam {
            webcamImage = try? await webcam.captureJpgBase64()
        }

        return Activity(
            clientActivityId: UUID().uuidString.lowercased(),
            projectId: projectId,
            startedAt: startedAt,
            endedAt: Date(),
            description: nil,
            mouseClicks: perms.mouse ? mouseClicks : 0,
            keyboardHits: perms.keyboard ? keyboardHits : 0,
            activeWindows: activeWindows,
            runningPrograms: runningPrograms,
            screenshotPngBase64: screenshot,
            webcamJpgBase64: webcamImage
        )
    }
}
