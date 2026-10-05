import AVFoundation
import CoreImage
import CoreMedia
import DosiTrackerCore
import Foundation

/// One webcam photo per block (CONTRACT §4; port of the Windows agent's
/// `tracking/webcam.rs`).
///
/// The camera is opened, frames are discarded for about a second while
/// auto-exposure settles (``WebcamWarmUp``), the next frame is kept, and the
/// camera is released at once so it is not held between blocks. A camera that
/// has not delivered within ``CaptureLimits/webcamTimeout`` is given up on.
/// The photo is a JPEG at most ``CaptureLimits/maxWebcamWidth`` wide within
/// ``CaptureLimits/maxWebcamBytes``.
///
/// The capture never asks for camera access itself — a prompt shown mid-block
/// would race the timeout — and fails with
/// ``CaptureError/permissionMissing(_:)`` until ``requestAccess()`` (or System
/// Settings) granted it. Without `NSCameraUsageDescription` in the app's
/// Info.plist macOS ends any process that touches the camera, so the capture
/// and the request refuse to run then.
///
/// Thread-safe: one capture at a time — while one runs, or a camera stuck in
/// its driver has not been released yet, another fails with
/// ``CaptureError/busy`` rather than stacking up. The session is configured,
/// started and stopped on its own serial queue (`startRunning` blocks), and
/// frames arrive on another, never on the main thread.
public final class WebcamCapturer: WebcamCapturing, @unchecked Sendable {
    /// The Info.plist key macOS requires before the camera may be used.
    public static let usageDescriptionKey = "NSCameraUsageDescription"

    private let sessionQueue = DispatchQueue(label: "com.dosibridge.tracker.webcam-session", qos: .utility,
                                             autoreleaseFrequency: .workItem)
    private let frameQueue = DispatchQueue(label: "com.dosibridge.tracker.webcam-frames", qos: .utility,
                                           autoreleaseFrequency: .workItem)
    private let coding = ImageCoding()
    private let lock = NSLock()
    private var active: PhotoRun?

    public init() {}

    /// Camera access as the system reports it.
    public static var authorizationStatus: AVAuthorizationStatus {
        AVCaptureDevice.authorizationStatus(for: .video)
    }

    /// Ask for camera access (the system prompt appears only the first time;
    /// afterwards only System Settings can change it). False without the
    /// usage description, which macOS requires before asking.
    public static func requestAccess() async -> Bool {
        guard hasUsageDescription else {
            CoreLog.error("webcam", "the app bundle has no \(usageDescriptionKey); camera access cannot be requested")
            return false
        }
        return await withCheckedContinuation { continuation in
            AVCaptureDevice.requestAccess(for: .video) { granted in
                continuation.resume(returning: granted)
            }
        }
    }

    /// Whether the running bundle declares why it uses the camera.
    public static var hasUsageDescription: Bool {
        guard let text = Bundle.main.object(forInfoDictionaryKey: usageDescriptionKey) as? String else { return false }
        return !text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
    }

    public func capturePhoto() async throws -> Data {
        guard WebcamCapturer.hasUsageDescription else {
            throw CaptureError.usageDescriptionMissing(WebcamCapturer.usageDescriptionKey)
        }
        guard WebcamCapturer.authorizationStatus == .authorized else {
            throw CaptureError.permissionMissing(.camera)
        }
        guard let device = AVCaptureDevice.default(for: .video) else {
            throw CaptureError.unavailable("no camera available")
        }
        let run = try begin(device)
        return try await withCheckedThrowingContinuation { continuation in
            run.start(timeout: CaptureLimits.webcamTimeout) { result in
                continuation.resume(with: result)
            }
        }
    }

    /// Claim the camera for a new run; busy while the previous one has not
    /// released it.
    private func begin(_ device: AVCaptureDevice) throws -> PhotoRun {
        lock.lock()
        defer { lock.unlock() }
        guard active == nil else { throw CaptureError.busy }
        let run = PhotoRun(device: device, sessionQueue: sessionQueue, frameQueue: frameQueue, coding: coding)
        run.onReleased = { [weak self, weak run] in
            guard let self = self else { return }
            self.lock.lock()
            if self.active === run {
                self.active = nil
            }
            self.lock.unlock()
        }
        active = run
        return run
    }
}

/// One photo: the capture session, the frame delegate, the warm-up and the
/// single completion.
///
/// Thread-safe: its mutable state is behind a lock; the session is only
/// touched on the session queue and frames only arrive on the frame queue.
private final class PhotoRun: NSObject, AVCaptureVideoDataOutputSampleBufferDelegate, @unchecked Sendable {
    typealias Completion = (Result<Data, Error>) -> Void

    /// Called on the session queue once the session has stopped and the
    /// camera is free again.
    var onReleased: (() -> Void)?

    private let device: AVCaptureDevice
    private let sessionQueue: DispatchQueue
    private let frameQueue: DispatchQueue
    private let coding: ImageCoding
    private let session = AVCaptureSession()
    private let output = AVCaptureVideoDataOutput()
    private let lock = NSLock()
    private var warmUp = WebcamWarmUp()
    private var completion: Completion?
    private var finished = false

    init(device: AVCaptureDevice, sessionQueue: DispatchQueue, frameQueue: DispatchQueue, coding: ImageCoding) {
        self.device = device
        self.sessionQueue = sessionQueue
        self.frameQueue = frameQueue
        self.coding = coding
        super.init()
    }

    /// Open the camera and wait for the photo; `completion` runs exactly
    /// once: with the photo, an error, or ``CaptureError/timedOut`` after
    /// `timeout`.
    func start(timeout: TimeInterval, completion: @escaping Completion) {
        lock.lock()
        self.completion = completion
        lock.unlock()

        // Not on the session queue: a camera stuck in `startRunning` must not
        // hold up its own timeout.
        DispatchQueue.global(qos: .utility).asyncAfter(deadline: .now() + timeout) { [weak self] in
            self?.finish(.failure(CaptureError.timedOut))
        }
        sessionQueue.async { [self] in
            do {
                try configure()
            } catch {
                finish(.failure(error))
                return
            }
            guard !isFinished else { return }
            session.startRunning()
        }
    }

    private var isFinished: Bool {
        lock.lock()
        defer { lock.unlock() }
        return finished
    }

    /// On the session queue.
    private func configure() throws {
        session.beginConfiguration()
        defer { session.commitConfiguration() }
        let input: AVCaptureDeviceInput
        do {
            input = try AVCaptureDeviceInput(device: device)
        } catch {
            throw CaptureError.failed("the camera could not be opened: \(error.localizedDescription)")
        }
        guard session.canAddInput(input) else {
            throw CaptureError.failed("the camera cannot be used now")
        }
        session.addInput(input)
        // What this camera supports, once it is attached: 720p is plenty for
        // a photo at most 1280 wide.
        if session.canSetSessionPreset(.hd1280x720) {
            session.sessionPreset = .hd1280x720
        } else if session.canSetSessionPreset(.high) {
            session.sessionPreset = .high
        }

        output.videoSettings = [kCVPixelBufferPixelFormatTypeKey as String: NSNumber(value: kCVPixelFormatType_32BGRA)]
        output.alwaysDiscardsLateVideoFrames = true
        output.setSampleBufferDelegate(self, queue: frameQueue)
        guard session.canAddOutput(output) else {
            throw CaptureError.failed("the camera's frames cannot be read")
        }
        session.addOutput(output)
    }

    // MARK: AVCaptureVideoDataOutputSampleBufferDelegate (frame queue)

    func captureOutput(_ output: AVCaptureOutput, didOutput sampleBuffer: CMSampleBuffer,
                       from connection: AVCaptureConnection) {
        lock.lock()
        let keep = !finished && warmUp.keeps(frameAt: ProcessInfo.processInfo.systemUptime)
        lock.unlock()
        guard keep else { return }

        guard let pixels = CMSampleBufferGetImageBuffer(sampleBuffer),
              let frame = coding.rendered(CIImage(cvPixelBuffer: pixels)),
              let photo = coding.fitted(frame, maxWidth: CaptureLimits.maxWebcamWidth)
        else {
            finish(.failure(CaptureError.failed("the camera delivered an unreadable frame")))
            return
        }
        finish(Result {
            try ImageCoding.jpeg(photo, quality: CaptureLimits.webcamJpegQuality, limit: CaptureLimits.maxWebcamBytes)
        })
    }

    // MARK: Completion

    /// Deliver the result (only the first one counts) and release the camera.
    private func finish(_ result: Result<Data, Error>) {
        lock.lock()
        let callback = finished ? nil : completion
        finished = true
        completion = nil
        lock.unlock()
        guard let callback = callback else { return }

        sessionQueue.async { [self] in
            output.setSampleBufferDelegate(nil, queue: nil)
            if session.isRunning {
                session.stopRunning()
            }
            onReleased?()
            onReleased = nil
        }
        callback(result)
    }
}
