import Foundation
import ScreenCaptureKit
import CoreGraphics
import ImageIO
import UniformTypeIdentifiers

/// Captures the main display via ScreenCaptureKit and returns a base64 PNG.
///
/// Requires the **Screen Recording** permission
/// (System Settings → Privacy & Security → Screen Recording).
enum ScreenshotCapturer {
    static func capturePngBase64() async throws -> String {
        let content = try await SCShareableContent.excludingDesktopWindows(false,
                                                                           onScreenWindowsOnly: true)
        guard let display = content.displays.first else {
            throw NSError(domain: "DosiTracker.Screenshot", code: 1,
                          userInfo: [NSLocalizedDescriptionKey: "no display found"])
        }

        let filter = SCContentFilter(display: display, excludingWindows: [])
        let config = SCStreamConfiguration()
        config.width = display.width
        config.height = display.height

        let cgImage = try await SCScreenshotManager.captureImage(contentFilter: filter,
                                                                 configuration: config)
        return try encodePngBase64(cgImage)
    }

    private static func encodePngBase64(_ image: CGImage) throws -> String {
        let data = NSMutableData()
        guard let dest = CGImageDestinationCreateWithData(data, UTType.png.identifier as CFString, 1, nil) else {
            throw NSError(domain: "DosiTracker.Screenshot", code: 2,
                          userInfo: [NSLocalizedDescriptionKey: "failed to create PNG destination"])
        }
        CGImageDestinationAddImage(dest, image, nil)
        guard CGImageDestinationFinalize(dest) else {
            throw NSError(domain: "DosiTracker.Screenshot", code: 3,
                          userInfo: [NSLocalizedDescriptionKey: "failed to finalize PNG"])
        }
        return (data as Data).base64EncodedString()
    }
}
