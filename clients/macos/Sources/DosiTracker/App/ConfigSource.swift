import DosiTrackerCore
import Foundation

// Where the agent's configuration comes from. The core's `Config` resolves the
// app bundle's Info.plist, UserDefaults (with MDM-managed values) and the
// server address saved at sign-in; this adds one more source a deployment can
// use without a configuration profile: an optional `config.json` in
// `~/Library/Application Support/DosiTracker` (see `config.example.json`). It
// sits between the two: it overrides what the build baked into Info.plist, and
// `defaults write` / MDM values override it.

enum ConfigFile {
    static let fileName = "config.json"

    /// The settings of the file at `url`, keyed as `Config.Key`; none when the
    /// file is absent or unreadable. The file's shape:
    ///
    /// ```json
    /// { "apiBaseURL": "https://…", "dashboardURL": "https://…", "intervalMinutes": 10,
    ///   "blurScreenshots": false, "allowInsecureHTTP": false,
    ///   "capture": { "screenshot": true, "webcam": false, "keyboard": true, "mouse": true,
    ///                "activeWindow": true, "runningPrograms": true } }
    /// ```
    static func values(at url: URL) -> [String: Any] {
        guard FileManager.default.fileExists(atPath: url.path) else { return [:] }
        guard let data = try? Data(contentsOf: url),
              let object = (try? JSONSerialization.jsonObject(with: data)) as? [String: Any]
        else {
            CoreLog.warning("config", "\(url.path) is not a JSON object; ignored")
            return [:]
        }
        var values: [String: Any] = [:]
        let top: [(String, String)] = [
            ("apiBaseURL", Config.Key.apiBaseURL),
            ("dashboardURL", Config.Key.dashboardURL),
            ("intervalMinutes", Config.Key.intervalMinutes),
            ("blurScreenshots", Config.Key.blurScreenshots),
            ("allowInsecureHTTP", Config.Key.allowInsecureHTTP),
        ]
        for (name, key) in top {
            if let value = object[name] {
                values[key] = value
            }
        }
        if let capture = object["capture"] as? [String: Any] {
            let flags: [(String, String)] = [
                ("screenshot", Config.Key.captureScreenshot),
                ("webcam", Config.Key.captureWebcam),
                ("keyboard", Config.Key.captureKeyboard),
                ("mouse", Config.Key.captureMouse),
                ("activeWindow", Config.Key.captureActiveWindow),
                ("runningPrograms", Config.Key.captureRunningPrograms),
            ]
            for (name, key) in flags {
                if let value = capture[name] {
                    values[key] = value
                }
            }
        }
        return values
    }
}

/// Resolves the configuration, with the server address saved at sign-in (or
/// another one) on top. Read again whenever needed: the file and the
/// preferences can change while the app runs.
struct ConfigSource: Sendable {
    /// The optional `config.json`.
    let fileURL: URL

    init(fileURL: URL) {
        self.fileURL = fileURL
    }

    /// The source of the app's own folders.
    init(paths: AppPaths) {
        self.init(fileURL: paths.applicationSupport.appendingPathComponent(ConfigFile.fileName, isDirectory: false))
    }

    func load(savedServerURL: String?) -> Config {
        var info = Bundle.main.infoDictionary ?? [:]
        for (key, value) in ConfigFile.values(at: fileURL) {
            info[key] = value
        }
        return Config.resolve(infoDictionary: info, defaults: .standard, savedServerURL: savedServerURL)
    }
}
