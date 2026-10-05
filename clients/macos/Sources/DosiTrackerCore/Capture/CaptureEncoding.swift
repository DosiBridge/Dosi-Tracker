import Foundation

/// How captures are encoded within the server's limits (port of the Windows
/// agent's `encode_jpeg_within`, `tracking/screenshot.rs`): the preferred
/// JPEG quality first, then each lower fallback quality, keeping the first
/// encoding that fits. A capture that fits at none is not sent at all.
///
/// Thread-safe: stateless.
public enum CaptureEncoding {
    /// Standard deviation, in pixels of the (at most 1920-wide) capture, of
    /// the Gaussian blur a project can require: one blur cell. Each pixel then
    /// mixes everything within about two cells around it (±2σ, ~100 px) —
    /// several characters of several lines — so no letter or outline of one
    /// survives, while window layout and colours stay recognisable. As strong
    /// as the Windows agent's cell average plus Gaussian.
    public static let blurRadius = Double(CaptureLimits.blurCellSize)

    /// `preferred`, then every fallback quality below it
    /// (``CaptureLimits/fallbackJpegQualities``).
    public static func qualities(startingAt preferred: Double) -> [Double] {
        [preferred] + CaptureLimits.fallbackJpegQualities.filter { $0 < preferred }
    }

    /// The first encoding — trying `qualities` in order — of at most `limit`
    /// bytes.
    /// - Parameter encode: encodes at a quality; nil when encoding failed.
    /// - Throws: ``CaptureError/tooLarge`` when every encoding was over the
    ///   limit, ``CaptureError/failed(_:)`` when none could be made.
    public static func firstFitting(limit: Int, qualities: [Double], encode: (Double) -> Data?) throws -> Data {
        var encodedAny = false
        for quality in qualities {
            guard let data = encode(quality), !data.isEmpty else { continue }
            encodedAny = true
            if data.count <= limit {
                return data
            }
        }
        throw encodedAny ? CaptureError.tooLarge : CaptureError.failed("the image could not be encoded")
    }
}

/// Which webcam frame to keep (port of the Windows agent's warm-up in
/// `tracking/webcam.rs`): the first frames of most cameras are dark or green
/// while auto-exposure settles, so every frame within
/// ``CaptureLimits/webcamWarmUp`` of the first one is discarded and the first
/// frame after that is kept. (Windows times the warm-up from opening the
/// stream; timing it from the first delivered frame is never shorter.)
///
/// Not thread-safe on its own; the capture's frame queue owns it.
public struct WebcamWarmUp: Equatable, Sendable {
    public let warmUp: TimeInterval
    /// When the first frame arrived (seconds on a monotonic clock).
    public private(set) var firstFrameAt: TimeInterval?

    public init(warmUp: TimeInterval = CaptureLimits.webcamWarmUp) {
        self.warmUp = warmUp
    }

    /// Whether the frame that arrived at `time` (seconds on a monotonic
    /// clock, e.g. `ProcessInfo.systemUptime`) is the one to keep.
    public mutating func keeps(frameAt time: TimeInterval) -> Bool {
        guard let first = firstFrameAt else {
            firstFrameAt = time
            return warmUp <= 0
        }
        return time - first >= warmUp
    }
}
