import Foundation

// Dosi-Tracker macOS agent.
//
// Authenticates against the backend's OpenIddict token endpoint, fetches the
// projects the user may track (with server-pushed capture permissions), then
// once per interval captures a snapshot (screenshot / webcam / active window /
// input counts), stores it locally (offline-first), and syncs pending snapshots.
//
// Without a resolved project the agent captures nothing: the backend requires
// a real project id, so speculative snapshots could never be uploaded.
//
// Permissions required on first run (System Settings → Privacy & Security):
//   • Screen Recording  (screenshots)
//   • Accessibility     (input counts, window titles)
//   • Camera            (webcam, only if enabled)

func log(_ level: String, _ message: String) {
    FileHandle.standardError.write(Data("[\(level)] \(message)\n".utf8))
}

let config = AppConfig.load()
log("info", "starting dosi-tracker (macOS)")

let env = ProcessInfo.processInfo.environment
let username = env["DOSI_USERNAME"] ?? ""
let password = env["DOSI_PASSWORD"] ?? ""

guard !username.isEmpty else {
    log("error", "DOSI_USERNAME / DOSI_PASSWORD are not set; the agent cannot sync. Exiting.")
    exit(1)
}

// Workspace (tenant) name; leave unset for a host account.
let workspace = env["DOSI_WORKSPACE"] ?? ""
let api = ApiClient(baseURL: config.apiBaseURL, username: username, password: password, tenant: workspace)
let store = try Storage(path: "dosi-tracker.db")

/// Server-resolved tracking session: which project, which permissions, how often.
struct Session {
    let projectId: String
    let perms: CapturePermissions
    let interval: TimeInterval
}

/// Resolve project + permissions from the backend. Server-side permissions win,
/// but never enable what local config forbade.
func resolveSession() async -> Session? {
    do {
        guard let project = try await api.projects().first else {
            log("warn", "logged in, but you are not a member of any active project")
            return nil
        }
        let perms = CapturePermissions(
            screenshot: project.allowScreenshot && config.capture.screenshot,
            webcam: project.allowWebcam && config.capture.webcam,
            keyboard: project.allowKeyboard && config.capture.keyboard,
            mouse: project.allowMouse && config.capture.mouse,
            activeWindow: project.allowActiveWindow && config.capture.activeWindow,
            runningPrograms: project.allowRunningPrograms && config.capture.runningPrograms
        )
        let interval = TimeInterval(min(max(project.intervalMinutes, 5), 60) * 60)
        log("info", "tracking session resolved for project '\(project.title)'")
        return Session(projectId: project.id, perms: perms, interval: interval)
    } catch {
        log("warn", "could not resolve projects from backend: \(error.localizedDescription)")
        return nil
    }
}

/// Upload any locally-queued activities. Permanently-rejected payloads are
/// parked so they never block the queue.
func syncPending() async {
    guard let pending = try? store.pending() else { return }
    for (id, activity) in pending {
        switch await api.submit(activity: activity) {
        case .ok:
            try? store.removeSynced(id)
        case .rejected(let reason):
            log("warn", "server rejected activity \(id); parking it (\(reason))")
            try? store.markRejected(id, reason: reason)
            // Continue with the next row — one bad payload must not block the queue.
        case .transient(let reason):
            log("warn", "sync failed (\(reason)); will retry next interval")
            return
        }
    }
}

var session = await resolveSession()
let tracker = Tracker()
let interval = session?.interval ?? config.interval
log("info", "tracking every \(Int(interval / 60)) min")

// Graceful shutdown on SIGINT.
let sigintSource = DispatchSource.makeSignalSource(signal: SIGINT, queue: .main)
signal(SIGINT, SIG_IGN)
sigintSource.setEventHandler {
    Task {
        log("info", "shutting down; syncing remaining activities")
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

        if session == nil {
            // Backend was unreachable at startup (or no project yet) — keep trying.
            session = await resolveSession()
        }

        guard let current = session else {
            log("warn", "no trackable project yet; skipping capture this interval")
            continue
        }

        let activity = await tracker.snapshot(
            perms: current.perms, projectId: current.projectId, startedAt: startedAt)
        try? store.enqueue(activity)
        await syncPending()
    }
}

// Keep the process alive for the run loop (event tap + timers).
RunLoop.main.run()
