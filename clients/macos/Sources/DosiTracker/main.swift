import Foundation

// Dosi-Tracker macOS agent.
//
// Logs in, picks a trackable project, then once per interval captures a
// snapshot (screenshot / webcam / active window / input counts), stores it
// locally (offline-first), and syncs pending snapshots to the backend.
//
// Permissions required on first run (System Settings → Privacy & Security):
//   • Screen Recording  (screenshots)
//   • Accessibility     (input counts, window titles)
//   • Camera            (webcam, only if enabled)

let config = AppConfig.load()
FileHandle.standardError.write(Data("[info] starting dosi-tracker (macOS)\n".utf8))

let api = ApiClient(baseURL: config.apiBaseURL)
let store = try Storage(path: "dosi-tracker.db")

let env = ProcessInfo.processInfo.environment
let username = env["DOSI_USERNAME"] ?? ""
let password = env["DOSI_PASSWORD"] ?? ""

// Resolve project + permissions from the backend, falling back to local config.
func resolveSession() async -> (projectId: String, perms: CapturePermissions, interval: TimeInterval) {
    if !username.isEmpty {
        do {
            let session = try await api.login(username: username, password: password)
            FileHandle.standardError.write(Data("[info] logged in as \(session.displayName)\n".utf8))
        } catch {
            FileHandle.standardError.write(Data("[warn] login failed: \(error.localizedDescription)\n".utf8))
        }
    }

    if let project = (try? await api.projects())?.first {
        let perms = CapturePermissions(
            screenshot: project.allowScreenshot && config.capture.screenshot,
            webcam: project.allowWebcam && config.capture.webcam,
            keyboard: project.allowKeyboard,
            mouse: project.allowMouse,
            activeWindow: project.allowActiveWindow,
            runningPrograms: project.allowRunningPrograms
        )
        let interval = TimeInterval(min(max(project.intervalMinutes, 5), 60) * 60)
        return (project.id, perms, interval)
    }

    FileHandle.standardError.write(Data("[warn] no project available; using local config\n".utf8))
    return ("local", config.capture, config.interval)
}

func syncPending() async {
    guard let pending = try? store.pending() else { return }
    for (id, activity) in pending {
        do {
            try await api.submit(activity: activity)
            try? store.markSynced(id)
        } catch {
            // Keep queued; retry next interval.
            FileHandle.standardError.write(Data("[warn] sync failed; will retry\n".utf8))
            break
        }
    }
}

let (projectId, perms, interval) = await resolveSession()
let tracker = Tracker(perms: perms)
FileHandle.standardError.write(Data("[info] tracking every \(Int(interval / 60)) min for project \(projectId)\n".utf8))

// Graceful shutdown on SIGINT.
let sigintSource = DispatchSource.makeSignalSource(signal: SIGINT, queue: .main)
signal(SIGINT, SIG_IGN)
sigintSource.setEventHandler {
    Task {
        FileHandle.standardError.write(Data("[info] shutting down; syncing remaining activities\n".utf8))
        await syncPending()
        exit(0)
    }
}
sigintSource.resume()

// Main interval loop.
Task {
    while true {
        let startedAt = Date()
        try? await Task.sleep(nanoseconds: UInt64(interval * 1_000_000_000))
        let activity = await tracker.snapshot(projectId: projectId, startedAt: startedAt)
        try? store.enqueue(activity)
        await syncPending()
    }
}

// Keep the process alive for the run loop (event tap + timers).
RunLoop.main.run()
