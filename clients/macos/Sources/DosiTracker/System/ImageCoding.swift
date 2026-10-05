import CoreGraphics
import CoreImage
import CoreImage.CIFilterBuiltins
import DosiTrackerCore
import Foundation
import ImageIO
import UniformTypeIdentifiers

/// A `CGImage` handed between queues. Core Graphics images are immutable and
/// safe to share across threads.
struct SharedImage: @unchecked Sendable {
    let image: CGImage
}

/// The image work of the screen and webcam captures (port of the Windows
/// agent's `fit_width`, `blur_unreadable` and `encode_jpeg_within`): scaling
/// into an opaque sRGB bitmap, the blur a project can require, and JPEG
/// encoding within the server's size limits.
///
/// Thread-safe: `CIContext` is thread-safe and everything else works on
/// values. Call it off the main thread; a capture takes tens of milliseconds.
final class ImageCoding: @unchecked Sendable {
    private let context = CIContext(options: [.cacheIntermediates: false])

    /// `image` drawn into an opaque sRGB bitmap at most `maxWidth` pixels
    /// wide, aspect kept and never upscaled, with high-quality resampling;
    /// nil for an empty image.
    func fitted(_ image: CGImage, maxWidth: Int) -> CGImage? {
        let size = CaptureLimits.scaledSize(width: image.width, height: image.height, maxWidth: maxWidth)
        guard size.width > 0, size.height > 0,
              let space = CGColorSpace(name: CGColorSpace.sRGB),
              let bitmap = CGContext(data: nil, width: size.width, height: size.height, bitsPerComponent: 8,
                                     bytesPerRow: 0, space: space,
                                     bitmapInfo: CGImageAlphaInfo.noneSkipLast.rawValue)
        else {
            return nil
        }
        bitmap.interpolationQuality = .high
        bitmap.draw(image, in: CGRect(x: 0, y: 0, width: size.width, height: size.height))
        return bitmap.makeImage()
    }

    /// `image` blurred so strongly that no text can be read (CONTRACT §4): a
    /// Gaussian of ``CaptureEncoding/blurRadius`` over the image clamped at
    /// its edges (so the borders do not fade into transparency), cropped back
    /// to its own size.
    func blurred(_ image: CGImage) -> CGImage? {
        let input = CIImage(cgImage: image)
        let blur = CIFilter.gaussianBlur()
        blur.inputImage = input.clampedToExtent()
        blur.radius = Float(CaptureEncoding.blurRadius)
        guard let output = blur.outputImage?.cropped(to: input.extent) else { return nil }
        return context.createCGImage(output, from: input.extent)
    }

    /// A Core Image image (a webcam frame) rendered into a `CGImage`.
    func rendered(_ image: CIImage) -> CGImage? {
        context.createCGImage(image, from: image.extent)
    }

    /// A JPEG of `image` at `quality` (0...1); nil when encoding failed.
    static func jpeg(_ image: CGImage, quality: Double) -> Data? {
        let data = NSMutableData()
        guard let destination = CGImageDestinationCreateWithData(data as CFMutableData, UTType.jpeg.identifier as CFString,
                                                                 1, nil)
        else {
            return nil
        }
        let properties = [kCGImageDestinationLossyCompressionQuality as String: quality] as CFDictionary
        CGImageDestinationAddImage(destination, image, properties)
        guard CGImageDestinationFinalize(destination) else { return nil }
        return data as Data
    }

    /// A JPEG of `image` of at most `limit` bytes: at `quality`, else at the
    /// first lower fallback quality that fits
    /// (``CaptureEncoding/firstFitting(limit:qualities:encode:)``).
    static func jpeg(_ image: CGImage, quality: Double, limit: Int) throws -> Data {
        try CaptureEncoding.firstFitting(limit: limit, qualities: CaptureEncoding.qualities(startingAt: quality)) { attempt in
            jpeg(image, quality: attempt)
        }
    }
}
