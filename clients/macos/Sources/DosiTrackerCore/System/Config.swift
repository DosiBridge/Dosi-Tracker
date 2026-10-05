import Foundation

/// Why a server address cannot be used, worded for the sign-in form.
public enum ServerURLError: Error, Equatable, Sendable {
    case empty
    /// Only `http://` and `https://` are supported.
    case unsupportedScheme(String)
    case invalid
    /// `http://` to a host on the internet: macOS App Transport Security
    /// refuses cleartext connections there (only local hosts are allowed).
    case requiresHTTPS

    public var userMessage: String {
        switch self {
        case .empty:
            return "Enter the server address."
        case .unsupportedScheme(let scheme):
            return "The server address must start with https:// or http:// (not \(scheme)://)."
        case .invalid:
            return "That is not a valid server address."
        case .requiresHTTPS:
            return "The server address must use https:// (http:// works only for a server on this computer or the local network)."
        }
    }
}

/// Runtime configuration of the agent, resolved in this order (later wins):
///
/// 1. built-in defaults (a local development server),
/// 2. the app bundle's Info.plist keys (what a release build ships with),
/// 3. UserDefaults in the app's domain (`defaults write com.dosibridge.tracker
///    DosiApiBaseURL https://…`), which is also where MDM configuration
///    profiles put managed preferences,
/// 4. the server address the user saved at sign-in, unless an MDM profile
///    forces the address (``serverURLIsManaged``).
///
/// A value of the wrong type is ignored (the key keeps the earlier value).
/// The capture keys can only switch captures off, and the blur key can only
/// add blur: the project's policy from the server still decides what is
/// captured.
public struct Config: Equatable, Sendable {
    /// Preference keys, valid in Info.plist, UserDefaults and MDM profiles.
    public enum Key {
        public static let apiBaseURL = "DosiApiBaseURL"
        public static let dashboardURL = "DosiDashboardURL"
        public static let intervalMinutes = "DosiIntervalMinutes"
        public static let captureScreenshot = "DosiCaptureScreenshot"
        public static let captureWebcam = "DosiCaptureWebcam"
        public static let captureKeyboard = "DosiCaptureKeyboard"
        public static let captureMouse = "DosiCaptureMouse"
        public static let captureActiveWindow = "DosiCaptureActiveWindow"
        public static let captureRunningPrograms = "DosiCaptureRunningPrograms"
        public static let blurScreenshots = "DosiBlurScreenshots"
    }

    /// The backend API root (no trailing slash).
    public var apiBaseURL: URL
    /// The web dashboard, opened by "Open web dashboard".
    public var dashboardURL: URL
    /// Block interval used until a project is known, in minutes (5...60).
    public var intervalMinutes: Int
    /// The local capture policy (intersected with each project's).
    public var capture: CapturePermissions
    /// Blur screen captures before upload. The effective value is the
    /// project's policy OR this, so a local configuration can add blur but
    /// never remove it.
    public var blurScreenshots: Bool
    /// An MDM profile fixes the server address: the sign-in form must not
    /// offer to change it.
    public var serverURLIsManaged: Bool

    public init(apiBaseURL: URL, dashboardURL: URL, intervalMinutes: Int, capture: CapturePermissions,
                blurScreenshots: Bool = false, serverURLIsManaged: Bool = false) {
        self.apiBaseURL = apiBaseURL
        self.dashboardURL = dashboardURL
        self.intervalMinutes = intervalMinutes
        self.capture = capture
        self.blurScreenshots = blurScreenshots
        self.serverURLIsManaged = serverURLIsManaged
    }

    /// `https://localhost:6006`, the backend's development address
    /// (HttpApi.Host `App:SelfUrl`).
    public static let defaultAPIBaseURL = URL(string: "https://localhost:6006")!
    /// `http://localhost:6005`, the web app's development address.
    public static let defaultDashboardURL = URL(string: "http://localhost:6005")!

    /// The built-in defaults.
    public static let builtIn = Config(
        apiBaseURL: defaultAPIBaseURL,
        dashboardURL: defaultDashboardURL,
        intervalMinutes: 10,
        capture: .localDefault
    )

    /// The block interval used until a project is known, in seconds.
    public var interval: TimeInterval { BlockClock.clampedInterval(minutes: intervalMinutes) }

    /// Resolve from the running app's Info.plist, the standard UserDefaults
    /// (with any MDM-managed values) and the saved server address.
    public static func load(bundle: Bundle = .main, defaults: UserDefaults = .standard, savedServerURL: String?) -> Config {
        resolve(infoDictionary: bundle.infoDictionary ?? [:], defaults: defaults, savedServerURL: savedServerURL)
    }

    /// The resolution itself, with every source passed in.
    public static func resolve(infoDictionary: [String: Any], defaults: UserDefaults, savedServerURL: String?) -> Config {
        var config = Config.builtIn
        config.apply(source: "Info.plist") { infoDictionary[$0] }
        config.apply(source: "preferences") { defaults.object(forKey: $0) }

        let managed = defaults.objectIsForced(forKey: Key.apiBaseURL)
        if !managed, let saved = savedServerURL {
            switch validateServerURL(saved) {
            case .success(let url):
                config.apiBaseURL = url
            case .failure(let error):
                CoreLog.warning("config", "the saved server address is unusable (\(error.userMessage)); using the configured one")
            }
        }
        config.serverURLIsManaged = managed
        config.intervalMinutes = min(max(config.intervalMinutes, BlockClock.minimumIntervalMinutes),
                                     BlockClock.maximumIntervalMinutes)
        return config
    }

    /// Normalize a server (API) address typed at sign-in or found in a
    /// profile (port of the Windows `normalize_server_url`): surrounding
    /// whitespace and trailing slashes go, a bare host gets `https://`, and
    /// only `http(s)` URLs with a host and no credentials, query or fragment
    /// are accepted. The scheme and host are lower-cased, a default port is
    /// dropped, and a path prefix (a server behind a reverse proxy at
    /// `/tracker`) is kept.
    public static func validateServerURL(_ input: String) -> Result<URL, ServerURLError> {
        let trimmed = input.trimmingCharacters(in: .whitespacesAndNewlines)
        var stem = trimmed
        while stem.hasSuffix("/") {
            stem.removeLast()
        }
        guard !stem.isEmpty else { return .failure(.empty) }
        // Checked here rather than left to the parser, which (for apps built
        // with the macOS 14 SDK) percent-encodes invalid characters instead of
        // refusing them.
        if trimmed.unicodeScalars.contains(where: { CharacterSet.whitespacesAndNewlines.contains($0)
            || CharacterSet.controlCharacters.contains($0) }) {
            return .failure(.invalid)
        }
        // "localhost:6006" would otherwise parse as scheme "localhost".
        let withScheme = trimmed.contains("://") ? trimmed : "https://" + trimmed
        guard let components = URLComponents(string: withScheme), let rawScheme = components.scheme else {
            return .failure(.invalid)
        }
        let scheme = rawScheme.lowercased()
        guard scheme == "http" || scheme == "https" else {
            return .failure(.unsupportedScheme(scheme))
        }
        guard let rawHost = components.host, !rawHost.isEmpty,
              components.user == nil, components.password == nil,
              components.query == nil, components.fragment == nil
        else {
            return .failure(.invalid)
        }
        let host = rawHost.lowercased()
        let isIPv6 = host.contains(":")
        // Characters a host can never hold (the WHATWG forbidden host code
        // points, `:` aside, which only an IPv6 literal has).
        let forbidden = CharacterSet(charactersIn: "#%/<>?@\\^|[]")
        let bare = isIPv6 ? host.trimmingCharacters(in: CharacterSet(charactersIn: "[]")) : host
        guard !bare.isEmpty, bare.rangeOfCharacter(from: forbidden) == nil else {
            return .failure(.invalid)
        }
        if scheme == "http" && !isLocalHost(bare, isIPv6: isIPv6) {
            return .failure(.requiresHTTPS)
        }
        // An IPv6 literal comes back without its brackets.
        let hostText = isIPv6 ? "[\(bare)]" : host
        var portText = ""
        if let port = components.port, !(scheme == "https" && port == 443), !(scheme == "http" && port == 80) {
            portText = ":\(port)"
        }
        var path = components.percentEncodedPath
        while path.hasSuffix("/") {
            path.removeLast()
        }
        guard let url = URL(string: "\(scheme)://\(hostText)\(portText)\(path)") else {
            return .failure(.invalid)
        }
        return .success(url)
    }

    /// Hosts App Transport Security lets the app reach over `http://` (with
    /// `NSAllowsLocalNetworking`): IP literals, `localhost`, unqualified names
    /// and `.local` (Bonjour) names.
    static func isLocalHost(_ host: String, isIPv6: Bool) -> Bool {
        if isIPv6 || host == "localhost" || !host.contains(".") || host.hasSuffix(".local") {
            return true
        }
        let parts = host.split(separator: ".", omittingEmptySubsequences: false)
        return parts.count == 4 && parts.allSatisfy { part in
            !part.isEmpty && part.count <= 3 && part.allSatisfy { $0.isASCII && $0.isNumber } && (Int(part) ?? 256) <= 255
        }
    }

    /// ``validateServerURL(_:)`` without the reason: nil when the text cannot
    /// be a server address.
    public static func normalizeServerURL(_ input: String) -> URL? {
        if case .success(let url) = validateServerURL(input) {
            return url
        }
        return nil
    }

    /// Apply the keys `source` knows, ignoring values of the wrong type.
    private mutating func apply(source name: String, _ value: (String) -> Any?) {
        if let text = Config.string(value(Key.apiBaseURL)) {
            if let url = Config.normalizeServerURL(text) {
                apiBaseURL = url
            } else {
                CoreLog.warning("config", "\(name): \(Key.apiBaseURL) is not a usable server address; ignored")
            }
        }
        if let text = Config.string(value(Key.dashboardURL)) {
            if let url = Config.normalizeServerURL(text) {
                dashboardURL = url
            } else {
                CoreLog.warning("config", "\(name): \(Key.dashboardURL) is not a usable address; ignored")
            }
        }
        if let minutes = Config.integer(value(Key.intervalMinutes)) {
            intervalMinutes = minutes
        }
        if let flag = Config.bool(value(Key.captureScreenshot)) { capture.screenshot = flag }
        if let flag = Config.bool(value(Key.captureWebcam)) { capture.webcam = flag }
        if let flag = Config.bool(value(Key.captureKeyboard)) { capture.keyboard = flag }
        if let flag = Config.bool(value(Key.captureMouse)) { capture.mouse = flag }
        if let flag = Config.bool(value(Key.captureActiveWindow)) { capture.activeWindow = flag }
        if let flag = Config.bool(value(Key.captureRunningPrograms)) { capture.runningPrograms = flag }
        if let flag = Config.bool(value(Key.blurScreenshots)) { blurScreenshots = flag }
    }

    static func string(_ value: Any?) -> String? {
        (value as? String)?.nonEmptyTrimmed
    }

    static func integer(_ value: Any?) -> Int? {
        if let number = value as? NSNumber, CFGetTypeID(number) != CFBooleanGetTypeID() {
            return number.intValue
        }
        if let text = value as? String {
            return Int(text.trimmingCharacters(in: .whitespacesAndNewlines))
        }
        return nil
    }

    static func bool(_ value: Any?) -> Bool? {
        if let number = value as? NSNumber {
            return number.boolValue
        }
        if let text = value as? String {
            switch text.trimmingCharacters(in: .whitespacesAndNewlines).lowercased() {
            case "true", "yes", "1":
                return true
            case "false", "no", "0":
                return false
            default:
                return nil
            }
        }
        return nil
    }
}
