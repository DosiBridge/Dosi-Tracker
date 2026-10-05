import XCTest
@testable import DosiTrackerCore

/// Which display a screen capture shows, and which window an app works in
/// (the macOS side of the Windows agent's `pick_monitor` tests).
final class DisplayTargetingTests: XCTestCase {
    /// A laptop panel, a large display to its right (raised a little) and
    /// one to its left (negative coordinates).
    private let main = DisplayFrame(id: 1, bounds: ScreenRect(x: 0, y: 0, width: 1512, height: 982))
    private let right = DisplayFrame(id: 2, bounds: ScreenRect(x: 1512, y: -200, width: 2560, height: 1440))
    private let left = DisplayFrame(id: 3, bounds: ScreenRect(x: -1920, y: 0, width: 1920, height: 1080))

    private func window(_ pid: Int32, _ x: Double, _ y: Double, _ width: Double = 800, _ height: Double = 600,
                        layer: Int = 0, alpha: Double = 1, name: String? = nil) -> WindowSnapshot {
        WindowSnapshot(ownerPID: pid, layer: layer, alpha: alpha,
                       bounds: ScreenRect(x: x, y: y, width: width, height: height), name: name)
    }

    private func target(_ pid: Int32?, _ windows: [WindowSnapshot], displays: [DisplayFrame]? = nil,
                        mainDisplayID: UInt32 = 1) -> UInt32 {
        DisplayTargeting.displayToCapture(frontmostPID: pid, windows: windows, displays: displays ?? [main, right, left],
                                          mainDisplayID: mainDisplayID)
    }

    func testTheDisplayHoldingTheFrontWindowIsCaptured() {
        XCTAssertEqual(target(99, [window(99, 2000, 100)]), 2)
        XCTAssertEqual(target(99, [window(99, -1500, 200)]), 3)
        XCTAssertEqual(target(99, [window(99, 100, 100)]), 1)
    }

    func testAWindowAcrossTwoDisplaysGoesToTheOneHoldingMoreOfIt() {
        // 300 points on the panel, 500 on the right display.
        XCTAssertEqual(target(7, [window(7, 1212, 100)]), 2)
        // 600 points on the panel, 200 on the right display.
        XCTAssertEqual(target(7, [window(7, 912, 100)]), 1)
    }

    func testOnlyTheAppsFrontOrdinaryWindowCounts() {
        let windows = [
            window(5, 2000, 0, layer: 25), // its menu-bar item
            window(42, 100, 100), // another app's window, in front
            window(5, 1600, 100, 10, 10), // a sliver
            window(5, 1600, 100, alpha: 0), // invisible
            window(5, -1000, 100), // its main window, on the left display
            window(5, 2000, 100), // one further back
        ]
        XCTAssertEqual(target(5, windows), 3)
        XCTAssertEqual(WindowList.frontWindow(of: 5, in: windows), window(5, -1000, 100))
        XCTAssertNil(WindowList.frontWindow(of: 6, in: windows))
    }

    func testWithoutAFrontWindowTheMainDisplayIsCaptured() {
        let windows = [window(5, 2000, 100)]
        XCTAssertEqual(target(nil, windows), 1, "no app in front")
        XCTAssertEqual(target(8, windows), 1, "the app has no window")
        XCTAssertEqual(target(5, [window(5, 9000, 9000)]), 1, "a window off every display")
        XCTAssertEqual(target(nil, windows, displays: [right, left]), 2, "the main display is not listed: the first")
        XCTAssertEqual(target(nil, windows, displays: []), 1, "no display listed")
    }

    func testTheFrontTitleIsTheAppsFrontNamedWindow() {
        let windows = [
            window(5, 0, 0, name: ""), // titles withheld or empty
            window(9, 0, 0, name: "Other app"),
            window(5, 0, 0, layer: 3, name: "Palette"),
            window(5, 0, 0, name: "Report.pdf"),
            window(5, 0, 0, name: "Older"),
        ]
        XCTAssertEqual(WindowList.frontTitle(of: 5, in: windows), "Report.pdf")
        XCTAssertNil(WindowList.frontTitle(of: 5, in: [window(5, 0, 0)]), "no Screen Recording access")
        XCTAssertNil(WindowList.frontTitle(of: 1, in: windows))
    }

    func testRectanglesOverlapOnlyWhereTheyShareArea() {
        let rect = ScreenRect(x: 0, y: 0, width: 100, height: 50)
        XCTAssertEqual(rect.area, 5_000)
        XCTAssertEqual(rect.intersectionArea(ScreenRect(x: 50, y: 25, width: 100, height: 100)), 50 * 25)
        XCTAssertEqual(rect.intersectionArea(ScreenRect(x: 100, y: 0, width: 10, height: 10)), 0, "touching edges")
        XCTAssertEqual(rect.intersectionArea(ScreenRect(x: -10, y: -10, width: 500, height: 500)), 5_000)
        XCTAssertEqual(ScreenRect(x: 0, y: 0, width: -5, height: 10).area, 0)
        XCTAssertNil(DisplayTargeting.displayHolding(ScreenRect.zero, among: [main]))
    }
}
