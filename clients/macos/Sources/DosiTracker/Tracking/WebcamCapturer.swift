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

    /// AVCaptureVideoDataOutput holds its delegate **weakly**, so we must keep the
    /// FrameHandler (and its session) alive on the capturer itself until a frame
    /// arrives — otherwise it deallocates immediately, tears down the session, and
    /// the continuation never resumes (hanging the whole agent).
    private var activeHandler: FrameHandler?
    private let lock = NSLock()

    /// Hard cap so a camera that never delivers a frame cannot block the agent forever.
    private let timeout: TimeInterval = 10

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

                // Resume exactly once and release the retained handler/session.
                let finish: (Result<String, Error>) -> Void = { [weak self] result in
                    guard let self else { return }
                    let handlerToStop: FrameHandler? = self.lock.guarded {
                        let h = self.activeHandler
                        self.activeHandler = nil
                        return h
                    }
                    guard let handlerToStop else { return } // already finished
                    handlerToStop.session?.stopRunning()
                    continuation.resume(with: result)
                }

                let handler = FrameHandler(context: ciContext, completion: finish)
                handler.session = session
                output.setSampleBufferDelegate(handler, queue: DispatchQueue(label: "dosi.webcam"))

                guard session.canAddOutput(output) else {
                    continuation.resume(throwing: Self.error("cannot add camera output"))
                    return
                }
                session.addOutput(output)

                // Strong reference on the capturer keeps the delegate alive for the capture.
                lock.guarded { self.activeHandler = handler }

                // Watchdog: if no frame arrives in time, fail rather than hang forever.
                DispatchQueue.global().asyncAfter(deadline: .now() + timeout) {
                    finish(.failure(Self.error("timed out waiting for a webcam frame")))
                }

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
        fileprivate var session: AVCaptureSession?
        private let doneLock = NSLock()
        private var done = false

        init(context: CIContext, completion: @escaping (Result<String, Error>) -> Void) {
            self.context = context
            self.completion = completion
        }

        func captureOutput(_ output: AVCaptureOutput,
                           didOutput sampleBuffer: CMSampleBuffer,
                           from connection: AVCaptureConnection) {
            doneLock.lock()
            if done { doneLock.unlock(); return }
            done = true
            doneLock.unlock()

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

private extension NSLock {
    func guarded<T>(_ body: () -> T) -> T {
        lock(); defer { unlock() }
        return body()
    }
}
