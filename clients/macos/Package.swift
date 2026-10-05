// swift-tools-version:5.9
import PackageDescription

// Dosi Tracker for macOS.
//
// * DosiTrackerCore: the platform-independent agent. Models and JSON coding,
//   activity math, foreground-app usage, block timing, idle handling, the
//   upload policy, the SQLite offline queue, the API client, configuration,
//   preferences, Keychain credentials and device facts. It uses Foundation,
//   SQLite3, Security and os only (no AppKit), so `swift test` covers it on a
//   CI runner without any UI or privacy permission.
// * DosiTracker: the menu-bar app built on the core: capture services
//   (ScreenCaptureKit, AVFoundation, event taps, Accessibility), the worker,
//   the UI and the app bundle. Its worker rules (BlockTimeline, WorkerRules)
//   are tested by DosiTrackerTests.
let package = Package(
    name: "DosiTracker",
    platforms: [
        .macOS(.v14) // SCScreenshotManager (ScreenCaptureKit), SMAppService, modern AVFoundation
    ],
    products: [
        .executable(name: "DosiTracker", targets: ["DosiTracker"]),
        .library(name: "DosiTrackerCore", targets: ["DosiTrackerCore"]),
    ],
    targets: [
        .target(
            name: "DosiTrackerCore",
            path: "Sources/DosiTrackerCore",
            linkerSettings: [
                .linkedLibrary("sqlite3"),
                .linkedFramework("Security"),
            ]
        ),
        .executableTarget(
            name: "DosiTracker",
            dependencies: ["DosiTrackerCore"],
            path: "Sources/DosiTracker",
            linkerSettings: [
                .linkedFramework("AppKit"),
                .linkedFramework("SwiftUI"),
                .linkedFramework("ScreenCaptureKit"),
                .linkedFramework("AVFoundation"),
                .linkedFramework("CoreMedia"),
                .linkedFramework("CoreVideo"),
                .linkedFramework("CoreGraphics"),
                .linkedFramework("CoreImage"),
                .linkedFramework("ImageIO"),
                .linkedFramework("UniformTypeIdentifiers"),
                .linkedFramework("ApplicationServices"),
                .linkedFramework("ServiceManagement"),
                .linkedFramework("IOKit"),
                .linkedFramework("UserNotifications"),
            ]
        ),
        .testTarget(
            name: "DosiTrackerCoreTests",
            dependencies: ["DosiTrackerCore"],
            path: "Tests/DosiTrackerCoreTests"
        ),
        // The app's own worker rules (block timing, break end, input, session
        // end), tested where they ship: `@testable import DosiTracker`.
        .testTarget(
            name: "DosiTrackerTests",
            dependencies: ["DosiTracker", "DosiTrackerCore"],
            path: "Tests/DosiTrackerTests"
        ),
    ]
)
