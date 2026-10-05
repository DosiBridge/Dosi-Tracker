import Foundation

/// Identity of the macOS agent, shared by the app shell, the API client and
/// the payloads it uploads.
public enum AgentInfo {
    /// Bundle identifier of the app. Also the Keychain service, the
    /// UserDefaults / MDM preference domain and the os.Logger subsystem.
    public static let bundleIdentifier = "com.dosibridge.tracker"

    /// Name shown to people (menus, prompts, the Keychain item's label).
    public static let displayName = "Dosi Tracker"

    /// Version reported when the running bundle is not the Dosi Tracker app
    /// (`swift run`, unit tests). Keep it in step with the app's Info.plist.
    public static let fallbackVersion = "0.2.0"

    /// The `platform` value sent with blocks and heartbeats (CONTRACT §1, §3).
    public static let platform = "macos"

    /// The agent version: the app bundle's `CFBundleShortVersionString`, else
    /// ``fallbackVersion``. Another bundle's version (e.g. the XCTest runner's)
    /// is never reported as the agent's.
    public static func version(bundle: Bundle = .main) -> String {
        guard bundle.bundleIdentifier == bundleIdentifier,
              let value = bundle.object(forInfoDictionaryKey: "CFBundleShortVersionString") as? String
        else {
            return fallbackVersion
        }
        let trimmed = value.trimmingCharacters(in: .whitespacesAndNewlines)
        return trimmed.isEmpty ? fallbackVersion : trimmed
    }

    /// `User-Agent` of every API request, e.g. `DosiTracker/0.2.0 (macOS 14.5)`.
    public static func userAgent(version: String, osVersion: String) -> String {
        "DosiTracker/\(version) (\(osVersion))"
    }
}
