# Dosi-Tracker — macOS Client (Swift)

A lightweight, background activity-tracking agent for macOS, written in Swift
using native Apple frameworks.

## Why native Swift

- Uses ScreenCaptureKit, AVFoundation, and the Accessibility API directly —
  the only reliable way to capture screen/camera/input on modern macOS.
- Event-driven input via `CGEventTap` → ~0% CPU while idle.
- No third-party dependencies (SQLite via the system library).

## What it does

Once per interval (5–60 min) it captures a snapshot for the active project:

- Screenshot (main display, via ScreenCaptureKit)
- Webcam frame (optional, via AVFoundation)
- Active window (frontmost app + focused window title)
- Running applications
- Keyboard / mouse **counts** (never keystroke content)

Snapshots are stored locally first (SQLite, offline-first) and then synced to
the backend REST API.

## Project layout

```
Sources/DosiTracker/
├── main.swift                     # agent loop + graceful shutdown
├── AppConfig.swift                # config.json + DOSI_* env vars
├── Models.swift                   # shared data types
├── ApiClient.swift                # backend REST client
├── Storage.swift                  # local SQLite queue (offline-first)
└── Tracking/
    ├── Tracker.swift              # builds one Activity snapshot
    ├── InputMonitor.swift         # CGEventTap keyboard/mouse counters
    ├── ScreenshotCapturer.swift   # ScreenCaptureKit PNG capture
    ├── WebcamCapturer.swift       # single AVFoundation JPEG frame
    └── ActiveWindowMonitor.swift  # frontmost app + window title
```

## Requirements

- macOS 14+ (Sonoma) for `SCScreenshotManager`
- Xcode 15+ / Swift 5.9+
- Must be built and run **on a Mac** (cannot compile on Windows/Linux)

## Required permissions (first run)

System Settings → Privacy & Security:

1. **Screen Recording** — screenshots
2. **Accessibility** — input counts + window titles
3. **Camera** — only if webcam capture is enabled

Without these, the corresponding captures silently return nothing (expected
macOS behavior, not a bug).

## Build & run

```bash
cd clients/macos
cp config.example.json config.json   # then edit values

export DOSI_USERNAME="you@example.com"
export DOSI_PASSWORD="..."

swift run
```

## Notes

- This is a scaffold: capture modules are working skeletons meant as a
  starting point, not a finished product.
- For true background operation, package as a **Launch Agent** (a `.plist` in
  `~/Library/LaunchAgents`) or an app bundle with a menu-bar (status item) UI.
