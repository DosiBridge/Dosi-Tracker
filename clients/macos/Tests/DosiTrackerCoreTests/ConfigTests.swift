import XCTest
@testable import DosiTrackerCore

/// Configuration sources and server-address normalization (ports of the
/// Windows agent's `config.rs` tests).
final class ConfigTests: XCTestCase {
    private var suiteName: String!
    private var defaults: UserDefaults!

    override func setUpWithError() throws {
        suiteName = "com.dosibridge.tracker.tests.\(UUID().uuidString)"
        defaults = try XCTUnwrap(UserDefaults(suiteName: suiteName))
    }

    override func tearDown() {
        defaults.removePersistentDomain(forName: suiteName)
        defaults = nil
        super.tearDown()
    }

    private func ok(_ input: String, allowInsecureHTTP: Bool = false, file: StaticString = #filePath,
                    line: UInt = #line) -> String? {
        switch Config.validateServerURL(input, allowInsecureHTTP: allowInsecureHTTP) {
        case .success(let url):
            return url.absoluteString
        case .failure(let error):
            XCTFail("\(input) refused: \(error)", file: file, line: line)
            return nil
        }
    }

    private func refusal(_ input: String) -> ServerURLError? {
        if case .failure(let error) = Config.validateServerURL(input) {
            return error
        }
        return nil
    }

    func testServerURLsAreNormalized() {
        XCTAssertEqual(ok("https://tracker.example.com/"), "https://tracker.example.com")
        XCTAssertEqual(ok("  HTTPS://Tracker.Example.com:8443/// "), "https://tracker.example.com:8443")
        XCTAssertEqual(ok("http://127.0.0.1:6006"), "http://127.0.0.1:6006")
        // A bare host (what people type) is https.
        XCTAssertEqual(ok("tracker.example.com"), "https://tracker.example.com")
        XCTAssertEqual(ok("localhost:6006"), "https://localhost:6006")
        // A reverse-proxy path prefix is kept; the default port is dropped.
        XCTAssertEqual(ok("https://example.com:443/tracker/"), "https://example.com/tracker")
        XCTAssertEqual(ok("http://localhost:80"), "http://localhost")
        XCTAssertEqual(ok("http://[::1]:6006/"), "http://[::1]:6006")
        XCTAssertEqual(ok("http://tracker-dev:6006", allowInsecureHTTP: true), "http://tracker-dev:6006")
    }

    /// Cleartext only to this Mac (loopback): the password and every block
    /// would otherwise cross the network readable. Anything else needs the
    /// administrator's explicit opt-in.
    func testPlainHttpIsOnlyForThisMacUnlessAnAdministratorAllowsIt() {
        XCTAssertEqual(refusal("http://tracker.corp.example.com"), .requiresHTTPS)
        XCTAssertEqual(refusal("http://example.com:80"), .requiresHTTPS)
        XCTAssertEqual(ok("https://tracker.corp.example.com"), "https://tracker.corp.example.com")
        // Not this Mac: the local network, Bonjour and bare names are refused too.
        for remote in ["http://192.168.1.20:6006", "http://10.0.0.5:6006", "http://tracker-dev:6006",
                       "http://build-mac.local:6006", "http://[fe80::1]:6006", "http://128.0.0.1",
                       "http://localhost.example.com"] {
            XCTAssertEqual(refusal(remote), .requiresHTTPS, remote)
        }
        // This Mac.
        for loopback in ["http://localhost:6006", "http://127.0.0.1:6006", "http://127.10.0.1", "http://[::1]",
                         "http://api.localhost:6006"] {
            XCTAssertNotNil(ok(loopback), loopback)
        }
        // The administrator's opt-in.
        XCTAssertEqual(ok("http://192.168.1.20:6006", allowInsecureHTTP: true), "http://192.168.1.20:6006")
        XCTAssertEqual(refusal("http://192.168.1.example"), .requiresHTTPS)
        XCTAssertTrue(ServerURLError.requiresHTTPS.userMessage.contains("https://"))
        XCTAssertTrue(ApiError.transport(TransportFailure(
            code: URLError.Code.appTransportSecurityRequiresSecureConnection.rawValue, description: "ATS"))
            .userMessage.contains("https://"))
    }

    func testUnusableServerURLsAreRefused() {
        XCTAssertEqual(refusal(""), .empty)
        XCTAssertEqual(refusal(" / "), .empty)
        XCTAssertEqual(refusal("ftp://example.com"), .unsupportedScheme("ftp"))
        for bad in ["https://", "https://exa mple.com", "https://user:pw@example.com", "https://example.com/?a=1",
                    "https://example.com/#x", "https://exa<mple.com"] {
            XCTAssertEqual(refusal(bad), .invalid, bad)
        }
        // The messages are meant for the sign-in form.
        XCTAssertTrue(ServerURLError.unsupportedScheme("ftp").userMessage.contains("https://"))
        XCTAssertEqual(ServerURLError.empty.userMessage, "Enter the server address.")
        XCTAssertNil(Config.normalizeServerURL("ftp://example.com"))
        XCTAssertEqual(Config.normalizeServerURL("example.com/"), URL(string: "https://example.com"))
    }

    func testNoSourcesMeanTheBuiltInDefaults() {
        let config = Config.resolve(infoDictionary: [:], defaults: defaults, savedServerURL: nil)
        XCTAssertEqual(config, Config.builtIn)
        XCTAssertEqual(config.apiBaseURL.absoluteString, "https://localhost:6006")
        XCTAssertEqual(config.dashboardURL.absoluteString, "http://localhost:6005")
        XCTAssertEqual(config.intervalMinutes, 10)
        XCTAssertTrue(config.capture.screenshot)
        XCTAssertFalse(config.capture.webcam)
        XCTAssertFalse(config.blurScreenshots)
        XCTAssertFalse(config.serverURLIsManaged)
        XCTAssertFalse(config.allowInsecureHTTP)
    }

    func testLaterSourcesWinKeyByKey() {
        let info: [String: Any] = [
            Config.Key.apiBaseURL: "https://shipped.example.com/",
            Config.Key.dashboardURL: "https://app.example.com/ ",
            Config.Key.intervalMinutes: 15,
            Config.Key.captureWebcam: true,
        ]
        defaults.set("https://preferred.example.com", forKey: Config.Key.apiBaseURL)
        defaults.set(false, forKey: Config.Key.captureScreenshot)

        let config = Config.resolve(infoDictionary: info, defaults: defaults, savedServerURL: nil)
        XCTAssertEqual(config.apiBaseURL.absoluteString, "https://preferred.example.com")
        XCTAssertEqual(config.dashboardURL.absoluteString, "https://app.example.com")
        XCTAssertEqual(config.intervalMinutes, 15)
        XCTAssertTrue(config.capture.webcam)
        XCTAssertFalse(config.capture.screenshot)
        // Untouched keys keep their defaults.
        XCTAssertTrue(config.capture.keyboard && config.capture.mouse)
        XCTAssertTrue(config.capture.activeWindow && config.capture.runningPrograms)
    }

    func testTheInsecureHttpOptInComesFromTheAdministratorsConfiguration() {
        // Without it, a configured http:// server is ignored...
        var config = Config.resolve(infoDictionary: [Config.Key.apiBaseURL: "http://10.1.2.3:6006"], defaults: defaults,
                                    savedServerURL: "http://10.9.9.9")
        XCTAssertFalse(config.allowInsecureHTTP)
        XCTAssertEqual(config.apiBaseURL, Config.defaultAPIBaseURL)
        // ...with it, the configured and the saved one are used.
        defaults.set(true, forKey: Config.Key.allowInsecureHTTP)
        config = Config.resolve(infoDictionary: [Config.Key.apiBaseURL: "http://10.1.2.3:6006"], defaults: defaults,
                                savedServerURL: nil)
        XCTAssertTrue(config.allowInsecureHTTP)
        XCTAssertEqual(config.apiBaseURL.absoluteString, "http://10.1.2.3:6006")
        config = Config.resolve(infoDictionary: [:], defaults: defaults, savedServerURL: "http://10.9.9.9")
        XCTAssertEqual(config.apiBaseURL.absoluteString, "http://10.9.9.9")
        // The dashboard only opens in the browser: any http(s) address will do.
        defaults.removeObject(forKey: Config.Key.allowInsecureHTTP)
        config = Config.resolve(infoDictionary: [Config.Key.dashboardURL: "http://dash.example.com"], defaults: defaults,
                                savedServerURL: nil)
        XCTAssertEqual(config.dashboardURL.absoluteString, "http://dash.example.com")
    }

    func testTheServerSavedAtSignInOverridesTheConfiguration() {
        defaults.set("https://configured.example.com", forKey: Config.Key.apiBaseURL)
        var config = Config.resolve(infoDictionary: [:], defaults: defaults, savedServerURL: nil)
        XCTAssertEqual(config.apiBaseURL.absoluteString, "https://configured.example.com")
        config = Config.resolve(infoDictionary: [:], defaults: defaults, savedServerURL: "saved.example.com/")
        XCTAssertEqual(config.apiBaseURL.absoluteString, "https://saved.example.com")
        // A damaged saved value never breaks the agent.
        config = Config.resolve(infoDictionary: [:], defaults: defaults, savedServerURL: "ftp://saved")
        XCTAssertEqual(config.apiBaseURL.absoluteString, "https://configured.example.com")
    }

    func testValuesOfTheWrongTypeOrUnusableAddressesAreIgnored() {
        let info: [String: Any] = [
            Config.Key.intervalMinutes: "ten",
            Config.Key.apiBaseURL: "ftp://tracker.example.com",
            Config.Key.captureKeyboard: "maybe",
            Config.Key.captureMouse: ["no"],
        ]
        let config = Config.resolve(infoDictionary: info, defaults: defaults, savedServerURL: nil)
        XCTAssertEqual(config.intervalMinutes, 10)
        XCTAssertEqual(config.apiBaseURL, Config.defaultAPIBaseURL)
        XCTAssertTrue(config.capture.keyboard)
        XCTAssertTrue(config.capture.mouse)
        // Text forms of numbers and switches are read.
        let text: [String: Any] = [Config.Key.intervalMinutes: " 20 ", Config.Key.captureKeyboard: "no"]
        let fromText = Config.resolve(infoDictionary: text, defaults: defaults, savedServerURL: nil)
        XCTAssertEqual(fromText.intervalMinutes, 20)
        XCTAssertFalse(fromText.capture.keyboard)
        // A boolean is not a number of minutes.
        XCTAssertEqual(Config.resolve(infoDictionary: [Config.Key.intervalMinutes: true], defaults: defaults,
                                      savedServerURL: nil).intervalMinutes, 10)
    }

    func testTheIntervalIsClamped() {
        XCTAssertEqual(Config.resolve(infoDictionary: [Config.Key.intervalMinutes: 1], defaults: defaults,
                                      savedServerURL: nil).intervalMinutes, 5)
        XCTAssertEqual(Config.resolve(infoDictionary: [Config.Key.intervalMinutes: 600], defaults: defaults,
                                      savedServerURL: nil).interval, 3600)
    }

    func testLocalBlurCanBeAddedButNeverRemoved() {
        let project = MyProject(id: "p", title: "P", intervalMinutes: 10, allowScreenshot: true, allowWebcam: false,
                                allowKeyboard: true, allowMouse: true, allowActiveWindow: true, allowRunningPrograms: true,
                                blurScreenshots: false)
        let local = Config.resolve(infoDictionary: [Config.Key.blurScreenshots: true], defaults: defaults, savedServerURL: nil)
        XCTAssertTrue(local.blurScreenshots)
        XCTAssertTrue(TrackingSession(project: project, config: local).blurScreenshots)

        var blurred = project
        blurred.blurScreenshots = true
        let plain = Config.resolve(infoDictionary: [Config.Key.blurScreenshots: false], defaults: defaults,
                                   savedServerURL: nil)
        XCTAssertTrue(TrackingSession(project: blurred, config: plain).blurScreenshots)
        XCTAssertFalse(TrackingSession(project: project, config: plain).blurScreenshots)
    }

    func testTheLocalPolicyCanOnlySwitchCapturesOff() {
        let project = MyProject(id: "p", title: "P", intervalMinutes: 10, allowScreenshot: false, allowWebcam: true,
                                allowKeyboard: true, allowMouse: true, allowActiveWindow: true, allowRunningPrograms: true)
        var config = Config.builtIn
        config.capture = .all
        let session = TrackingSession(project: project, config: config)
        XCTAssertFalse(session.permissions.screenshot, "the project forbids it")
        XCTAssertTrue(session.permissions.webcam)
        config.capture.keyboard = false
        XCTAssertFalse(TrackingSession(project: project, config: config).permissions.keyboard)
        XCTAssertEqual(session.interval, 600)
        XCTAssertEqual(session.idleThreshold, 300)
    }

    func testProjectsAreChosenAsTheContractSays() {
        func project(_ id: String) -> MyProject {
            MyProject(id: id, title: id, intervalMinutes: 10, allowScreenshot: true, allowWebcam: false, allowKeyboard: true,
                      allowMouse: true, allowActiveWindow: true, allowRunningPrograms: true)
        }
        let one = [project("a")]
        let two = [project("a"), project("b")]
        XCTAssertEqual(ProjectSelection.resolve(projects: [], requested: "a", remembered: "a"), .noProjects)
        // Exactly one: auto-selected.
        XCTAssertEqual(ProjectSelection.resolve(projects: one, requested: nil, remembered: nil), .selected(project("a")))
        XCTAssertEqual(ProjectSelection.resolve(projects: one, requested: nil, remembered: "gone"), .selected(project("a")))
        // Several: the remembered one if still listed (ids compare like GUIDs) ...
        XCTAssertEqual(ProjectSelection.resolve(projects: two, requested: nil, remembered: " B "), .selected(project("b")))
        // ... otherwise the user chooses; never "the first one".
        XCTAssertEqual(ProjectSelection.resolve(projects: two, requested: nil, remembered: nil), .choose(two))
        XCTAssertEqual(ProjectSelection.resolve(projects: two, requested: nil, remembered: "gone"), .choose(two))
        // An explicit choice wins.
        XCTAssertEqual(ProjectSelection.resolve(projects: two, requested: "a", remembered: "b").project, project("a"))
        XCTAssertNil(ProjectSelection.choose(two).project)
        XCTAssertTrue(ProjectSelection.contains("A", in: two))
        XCTAssertFalse(ProjectSelection.contains("c", in: two))
    }

    func testAppPathsLiveUnderTheUsersLibrary() throws {
        let paths = try AppPaths.standard()
        XCTAssertTrue(paths.applicationSupport.path.hasSuffix("/Library/Application Support/DosiTracker"))
        XCTAssertTrue(paths.logs.path.hasSuffix("/Library/Logs/DosiTracker"))
        XCTAssertEqual(paths.database.lastPathComponent, "dosi-tracker.db")

        let root = FileManager.default.temporaryDirectory.appendingPathComponent("dosi-paths-\(UUID().uuidString)")
        defer { try? FileManager.default.removeItem(at: root) }
        let rooted = AppPaths.rooted(at: root)
        try rooted.createDirectories()
        var isDirectory: ObjCBool = false
        XCTAssertTrue(FileManager.default.fileExists(atPath: rooted.applicationSupport.path, isDirectory: &isDirectory))
        XCTAssertTrue(isDirectory.boolValue)
        XCTAssertTrue(FileManager.default.fileExists(atPath: rooted.logs.path))
        let permissions = try FileManager.default.attributesOfItem(atPath: rooted.applicationSupport.path)[.posixPermissions]
        XCTAssertEqual((permissions as? NSNumber)?.intValue, 0o700)
    }
}
