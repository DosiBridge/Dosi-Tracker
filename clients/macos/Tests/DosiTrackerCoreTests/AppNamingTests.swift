import XCTest
@testable import DosiTrackerCore

/// App names and sampled window texts as uploaded (CONTRACT §8).
final class AppNamingTests: XCTestCase {
    func testTheLocalizedNameIsTheAppsName() {
        XCTAssertEqual(AppNaming.appName(localizedName: "Google Chrome", bundleFileName: "Google Chrome.app",
                                         executableName: "Google Chrome"), "Google Chrome")
        XCTAssertEqual(AppNaming.appName(localizedName: "  Xcode \n", bundleFileName: nil, executableName: nil), "Xcode")
    }

    func testTheBundleOrExecutableNameStandsInForAMissingLocalizedName() {
        XCTAssertEqual(AppNaming.appName(localizedName: nil, bundleFileName: "Visual Studio Code.app",
                                         executableName: "Electron"), "Visual Studio Code")
        XCTAssertEqual(AppNaming.appName(localizedName: " ", bundleFileName: "Tool.APP", executableName: nil), "Tool")
        XCTAssertEqual(AppNaming.appName(localizedName: nil, bundleFileName: ".app", executableName: "python3"), "python3")
        XCTAssertEqual(AppNaming.appName(localizedName: nil, bundleFileName: nil, executableName: "zsh"), "zsh")
        XCTAssertNil(AppNaming.appName(localizedName: "", bundleFileName: "", executableName: nil))
    }

    func testNamesAndTitlesAreCutToWhatTheServerStores() {
        let long = String(repeating: "a", count: 300)
        XCTAssertEqual(AppNaming.appName(localizedName: long, bundleFileName: nil, executableName: nil)?.utf16.count, 256)

        let sampled = ForegroundWindow.sampled(appName: long, windowTitle: String(repeating: "é", count: 600))
        XCTAssertEqual(sampled.appName.utf16.count, AppNaming.maxAppNameLength)
        XCTAssertEqual(sampled.windowTitle.utf16.count, AppNaming.maxWindowTitleLength)
        // A character is never split: an emoji that would straddle the limit goes.
        let emoji = ForegroundWindow.sampled(appName: "Notes", windowTitle: String(repeating: "x", count: 511) + "🙂")
        XCTAssertEqual(emoji.windowTitle, String(repeating: "x", count: 511))
        // Short texts are kept as they are.
        XCTAssertEqual(ForegroundWindow.sampled(appName: "Mail", windowTitle: "Inbox"),
                       ForegroundWindow(appName: "Mail", windowTitle: "Inbox"))
    }
}
