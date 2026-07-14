import Foundation
import AVFoundation
import CoreImage

/// Captures a single webcam frame via AVFoundation and returns a base64 JPEG.
///
/// Requires the **Camera** permission
/// (System Settings → Privacy & Security → Camera). The session is torn down
/// after one frame so the camera is not held between intervals.
final class WebcamCapturer: NSObject {
    private let ciContext = CIContext()

    func captureJpgBase64() async throws -> String {
        try await withCheckedThrowingContinuation { continuation in
            guard let device = AVCaptureDevice.default(.builtInWideAngleCamera,
                                                       for: .video, position: .front)
                ?? AVCaptureDevice.default(for: .video) else {
                continuation.resume(throwing: Self.error("no camera available"))
                return
            }

            do {
                let session = AVCaptureSession()
                session.sessionPreset = .photo

                let input = try AVCaptureDeviceInput(device: device)
                guard session.canAddInput(input) else {
                    continuation.resume(throwing: Self.error("cannot add camera input"))
                    return
                }
                session.addInput(input)

                let output = AVCaptureVideoDataOutput()
                let handler = FrameHandler(context: ciContext) { result in
                    session.stopRunning()
                    continuation.resume(with: result)
                }
                output.setSampleBufferDelegate(handler,
                                               queue: DispatchQueue(label: "dosi.webcam"))
                guard session.canAddOutput(output) else {
                    continuation.resume(throwing: Self.error("cannot add camera output"))
                    return
                }
                session.addOutput(output)
                // Keep a strong ref alive until the frame arrives.
                handler.retainSession(session)
                session.startRunning()
            } catch {
                continuation.resume(throwing: error)
            }
        }
    }

    private static func error(_ message: String) -> NSError {
        NSError(domain: "DosiTracker.Webcam", code: 1,
                userInfo: [NSLocalizedDescriptionKey: message])
    }

    /// Grabs the first sample buffer, encodes JPEG, then signals completion.
    private final class FrameHandler: NSObject, AVCaptureVideoDataOutputSampleBufferDelegate {
        private let context: CIContext
        private let completion: (Result<String, Error>) -> Void
        private var session: AVCaptureSession?
        private var done = false

        init(context: CIContext, completion: @escaping (Result<String, Error>) -> Void) {
            self.context = context
            self.completion = completion
        }

        func retainSession(_ session: AVCaptureSession) { self.session = session }

        func captureOutput(_ output: AVCaptureOutput,
                           didOutput sampleBuffer: CMSampleBuffer,
                           from connection: AVCaptureConnection) {
            guard !done else { return }
            done = true

            guard let pixelBuffer = CMSampleBufferGetImageBuffer(sampleBuffer) else {
                completion(.failure(WebcamCapturer.error("no pixel buffer")))
                return
            }
            let ciImage = CIImage(cvPixelBuffer: pixelBuffer)
            guard let jpeg = context.jpegRepresentation(
                of: ciImage,
                colorSpace: CGColorSpaceCreateDeviceRGB(),
                options: [kCGImageDestinationLossyCompressionQuality as CIImageRepresentationOption: 0.7]
            ) else {
                completion(.failure(WebcamCapturer.error("failed to encode JPEG")))
                return
            }
            completion(.success(jpeg.base64EncodedString()))
        }
    }
}
