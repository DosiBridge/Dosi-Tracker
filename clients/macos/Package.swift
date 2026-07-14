// swift-tools-version:5.9
import PackageDescription

let package = Package(
    name: "DosiTracker",
    platforms: [
        .macOS(.v14) // SCScreenshotManager (ScreenCaptureKit) + modern AVFoundation
    ],
    targets: [
        .executableTarget(
            name: "DosiTracker",
            path: "Sources/DosiTracker"
        )
    ]
)
