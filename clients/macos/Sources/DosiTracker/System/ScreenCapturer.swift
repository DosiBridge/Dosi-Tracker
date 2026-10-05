import AppKit
import CoreGraphics
import DosiTrackerCore
import Foundation
import ScreenCaptureKit

/// Screen captures (CONTRACT §4 "Screenshots"; port of the Windows agent's
/// `tracking/screenshot.rs`).
///
/// * The display that holds the front-most window of the app in front is
///   captured (its bounds from the window list), else the main display.
/// * ScreenCaptureKit renders it at the display's pixel size (from its
///   display mode, so Retina text stays sharp) scaled down to at most
///   ``CaptureLimits/maxScreenWidth`` wide; the cursor is left out.
/// * The capture is a JPEG at quality ``CaptureLimits/screenJpegQuality``
///   (lower when needed to stay under ``CaptureLimits/maxScreenBytes``) with a
///   thumbnail at most ``CaptureLimits/maxThumbnailWidth`` wide.
/// * When the project requires it, both are blurred beyond reading before
///   they leave the capturer; a blur that fails gives no capture, never an
///   unblurred one.
///
/// Screen Recording access is checked first (`CGPreflightScreenCaptureAccess`,
/// which never prompts), so a missing grant is reported as
/// ``CaptureError/permissionMissing(_:)`` rather than prompting mid-block;
/// ``Permissions/requestScreenRecording()`` asks for it.
///
/// Thread-safe: one capture at a time (a second one fails with
/// ``CaptureError/busy``); the image work runs on a utility queue, never on the
/// main thread.
public final class ScreenCapturer: ScreenCapturing, @unchecked Sendable {
    /// The process of the app in front, when the caller tracks it (e.g.
    /// ``ActiveAppMonitor/frontmostApp``); nil to ask NSWorkspace.
    public typealias FrontmostProcess = @Sendable () -> pid_t?

    private let frontmostProcess: FrontmostProcess
    private let coding = ImageCoding()
    private let encodeQueue = DispatchQueue(label: "com.dosibridge.tracker.screen-encode", qos: .utility,
                                            autoreleaseFrequency: .workItem)
    private let lock = NSLock()
    private var capturing = false

    public init(frontmostProcess: @escaping FrontmostProcess = { nil }) {
        self.frontmostProcess = frontmostProcess
    }

    public func capture(blur: Bool) async throws -> ScreenCapture {
        guard CGPreflightScreenCaptureAccess() else {
            throw CaptureError.permissionMissing(.screenRecording)
        }
        try begin()
        defer { end() }

        let pid = await frontmostProcessID()
        let content: SCShareableContent
        do {
            content = try await SCShareableContent.excludingDesktopWindows(false, onScreenWindowsOnly: true)
        } catch {
            throw ScreenCapturer.captureError(error)
        }
        let displays = content.displays
        guard !displays.isEmpty else {
            throw CaptureError.unavailable("no display to capture")
        }
        let targetID = DisplayTargeting.displayToCapture(
            frontmostPID: pid,
            windows: WindowListReader.onScreenWindows(),
            displays: displays.map { DisplayFrame(id: $0.displayID, bounds: ScreenRect(CGDisplayBounds($0.displayID))) },
            mainDisplayID: CGMainDisplayID()
        )
        let display = displays.first { $0.displayID == targetID } ?? displays[0]

        let size = ScreenCapturer.outputSize(of: display)
        let configuration = SCStreamConfiguration()
        configuration.width = size.width
        configuration.height = size.height
        // Render from the display's pixels, not its points: otherwise a Retina
        // panel narrower than 1920 points would be captured at point size and
        // scaled up, blurring the text.
        configuration.captureResolution = .best
        configuration.scalesToFit = true
        configuration.showsCursor = false
        let filter = SCContentFilter(display: display, excludingWindows: [])

        let capturedAt = Date()
        let image: CGImage
        do {
            image = try await SCScreenshotManager.captureImage(contentFilter: filter, configuration: configuration)
        } catch {
            throw ScreenCapturer.captureError(error)
        }
        return try await encode(SharedImage(image: image), blur: blur, capturedAt: capturedAt)
    }

    // MARK: Steps

    private func begin() throws {
        lock.lock()
        defer { lock.unlock() }
        guard !capturing else { throw CaptureError.busy }
        capturing = true
    }

    private func end() {
        lock.lock()
        defer { lock.unlock() }
        capturing = false
    }

    private func frontmostProcessID() async -> pid_t? {
        if let pid = frontmostProcess() {
            return pid
        }
        return await MainActor.run { NSWorkspace.shared.frontmostApplication?.processIdentifier }
    }

    /// Encode on the utility queue, off the caller's thread.
    private func encode(_ frame: SharedImage, blur: Bool, capturedAt: Date) async throws -> ScreenCapture {
        let coding = self.coding
        return try await withCheckedThrowingContinuation { continuation in
            encodeQueue.async {
                continuation.resume(with: Result {
                    try ScreenCapturer.encodeCapture(frame.image, blur: blur, capturedAt: capturedAt, coding: coding)
                })
            }
        }
    }

    /// Turn a captured frame into what is uploaded: at most
    /// ``CaptureLimits/maxScreenWidth`` wide in sRGB, blurred when required, a
    /// JPEG within ``CaptureLimits/maxScreenBytes``, and a thumbnail of the
    /// same (blurred) frame. A thumbnail failure never costs the capture.
    static func encodeCapture(_ frame: CGImage, blur: Bool, capturedAt: Date, coding: ImageCoding) throws -> ScreenCapture {
        guard frame.width > 0, frame.height > 0 else {
            throw CaptureError.failed("the screen capture was empty")
        }
        guard let fitted = coding.fitted(frame, maxWidth: CaptureLimits.maxScreenWidth) else {
            throw CaptureError.failed("the screen capture could not be scaled")
        }
        var screen = fitted
        if blur {
            guard let blurred = coding.blurred(fitted) else {
                throw CaptureError.failed("the screen capture could not be blurred")
            }
            screen = blurred
        }
        let jpeg = try ImageCoding.jpeg(screen, quality: CaptureLimits.screenJpegQuality, limit: CaptureLimits.maxScreenBytes)

        var thumbnail: Data?
        if let small = coding.fitted(screen, maxWidth: CaptureLimits.maxThumbnailWidth) {
            thumbnail = try? ImageCoding.jpeg(small, quality: CaptureLimits.thumbnailJpegQuality,
                                              limit: CaptureLimits.maxThumbnailBytes)
        }
        if thumbnail == nil {
            CoreLog.warning("screen", "could not encode the screenshot thumbnail; sending the capture without it")
        }
        return ScreenCapture(image: jpeg, thumbnail: thumbnail, capturedAt: capturedAt, blurred: blur)
    }

    /// The size ScreenCaptureKit renders `display` at: its pixel size (from
    /// its display mode; the point size when that is unknown), scaled down to
    /// at most ``CaptureLimits/maxScreenWidth`` wide.
    static func outputSize(of display: SCDisplay) -> (width: Int, height: Int) {
        var width = display.width
        var height = display.height
        if let mode = CGDisplayCopyDisplayMode(display.displayID), mode.pixelWidth > 0, mode.pixelHeight > 0 {
            width = mode.pixelWidth
            height = mode.pixelHeight
            // A rotated display reports its mode unrotated.
            if (width >= height) != (display.width >= display.height) {
                swap(&width, &height)
            }
        }
        return CaptureLimits.scaledSize(width: width, height: height, maxWidth: CaptureLimits.maxScreenWidth)
    }

    /// A ScreenCaptureKit failure as a capture error: a refused grant (which
    /// the preflight can miss until the app is relaunched) is a missing
    /// permission.
    static func captureError(_ error: Error) -> CaptureError {
        if let streamError = error as? SCStreamError, streamError.code == .userDeclined {
            return .permissionMissing(.screenRecording)
        }
        return .failed("screen capture failed: \(error.localizedDescription)")
    }
}
