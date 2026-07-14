import Foundation

/// Runtime configuration, loaded from `config.json` next to the executable
/// and/or `DOSI_*` environment variables. Per-project permissions returned by
/// the backend normally override these defaults.
struct AppConfig: Codable {
    var apiBaseURL: String
    var intervalMinutes: Int
    var capture: CapturePermissions

    static let `default` = AppConfig(
        apiBaseURL: "https://localhost:44300",
        intervalMinutes: 10,
        capture: CapturePermissions(
            screenshot: true,
            webcam: false,
            keyboard: true,
            mouse: true,
            activeWindow: true,
            runningPrograms: true
        )
    )

    /// Snapshot interval, clamped to 5...60 minutes. Longer = lower CPU/battery.
    var interval: TimeInterval {
        TimeInterval(min(max(intervalMinutes, 5), 60) * 60)
    }

    static func load() -> AppConfig {
        var config = AppConfig.default

        // 1) config.json next to the executable, if present.
        let url = URL(fileURLWithPath: "config.json")
        if let data = try? Data(contentsOf: url),
           let decoded = try? JSONDecoder().decode(AppConfig.self, from: data) {
            config = decoded
        }

        // 2) Environment overrides.
        let env = ProcessInfo.processInfo.environment
        if let base = env["DOSI_API_BASE_URL"] { config.apiBaseURL = base }
        if let iv = env["DOSI_INTERVAL_MINUTES"], let v = Int(iv) { config.intervalMinutes = v }

        return config
    }
}

struct CapturePermissions: Codable {
    var screenshot: Bool
    var webcam: Bool
    var keyboard: Bool
    var mouse: Bool
    var activeWindow: Bool
    var runningPrograms: Bool
}
