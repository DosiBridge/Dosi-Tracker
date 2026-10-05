import XCTest
@testable import DosiTrackerCore

/// Capture encoding within the server's limits, the blur strength and the
/// webcam warm-up (ports of the Windows agent's `screenshot.rs` and
/// `webcam.rs` rules).
final class CaptureEncodingTests: XCTestCase {
    func testQualitiesStepDownFromThePreferredOne() {
        XCTAssertEqual(CaptureEncoding.qualities(startingAt: CaptureLimits.screenJpegQuality), [0.8, 0.6, 0.45, 0.3])
        XCTAssertEqual(CaptureEncoding.qualities(startingAt: CaptureLimits.thumbnailJpegQuality), [0.7, 0.6, 0.45, 0.3])
        XCTAssertEqual(CaptureEncoding.qualities(startingAt: 0.5), [0.5, 0.45, 0.3])
        XCTAssertEqual(CaptureEncoding.qualities(startingAt: 0.2), [0.2])
    }

    func testTheFirstEncodingThatFitsIsKept() throws {
        // Size grows with quality, as with real JPEGs.
        var tried: [Double] = []
        let data = try CaptureEncoding.firstFitting(limit: 500, qualities: [0.8, 0.6, 0.45, 0.3]) { quality in
            tried.append(quality)
            return Data(count: Int((quality * 1_000).rounded()))
        }
        XCTAssertEqual(data.count, 450)
        XCTAssertEqual(tried, [0.8, 0.6, 0.45], "no lower quality than needed")
    }

    func testACaptureThatFitsAtNoQualityIsNotSent() {
        XCTAssertThrowsError(try CaptureEncoding.firstFitting(limit: 10, qualities: [0.8, 0.3]) { _ in Data(count: 100) }) {
            XCTAssertEqual($0 as? CaptureError, .tooLarge)
        }
        XCTAssertThrowsError(try CaptureEncoding.firstFitting(limit: 10, qualities: [0.8, 0.3]) { _ in nil }) {
            XCTAssertEqual($0 as? CaptureError, .failed("the image could not be encoded"))
        }
        // Empty output is never an image.
        XCTAssertThrowsError(try CaptureEncoding.firstFitting(limit: 10, qualities: [0.8]) { _ in Data() })
    }

    func testAnEncoderFailingAtOneQualityTriesTheNext() throws {
        let data = try CaptureEncoding.firstFitting(limit: 10, qualities: [0.8, 0.3]) { quality in
            quality > 0.5 ? nil : Data(count: 5)
        }
        XCTAssertEqual(data.count, 5)
    }

    func testTheRequiredBlurSpansSeveralCharacters() {
        // Ordinary UI text is under 10 px a character at 1920 px wide; ±2σ
        // mixes ten or more of them, and several lines.
        XCTAssertEqual(CaptureEncoding.blurRadius, Double(CaptureLimits.blurCellSize))
        XCTAssertGreaterThanOrEqual(CaptureEncoding.blurRadius, 16)
    }

    func testWebcamFramesAreDiscardedWhileTheCameraWarmsUp() {
        var warmUp = WebcamWarmUp(warmUp: 1)
        XCTAssertFalse(warmUp.keeps(frameAt: 100.0)) // the first frame: dark
        XCTAssertFalse(warmUp.keeps(frameAt: 100.5))
        XCTAssertFalse(warmUp.keeps(frameAt: 100.99))
        XCTAssertTrue(warmUp.keeps(frameAt: 101.0))
        XCTAssertEqual(warmUp.firstFrameAt, 100.0)

        var immediate = WebcamWarmUp(warmUp: 0)
        XCTAssertTrue(immediate.keeps(frameAt: 5))
        XCTAssertEqual(WebcamWarmUp().warmUp, CaptureLimits.webcamWarmUp)
        XCTAssertLessThan(CaptureLimits.webcamWarmUp, CaptureLimits.webcamTimeout)
    }

    func testCaptureErrorsReadAsReasons() {
        XCTAssertEqual(CaptureError.permissionMissing(.screenRecording).description, "Screen Recording permission not granted")
        XCTAssertEqual(CaptureError.usageDescriptionMissing("NSCameraUsageDescription").description,
                       "the app bundle has no NSCameraUsageDescription")
        XCTAssertEqual(CaptureError.unavailable("no camera available").description, "no camera available")
        XCTAssertEqual(CaptureError.busy.description, "a previous capture is still running")
    }
}
