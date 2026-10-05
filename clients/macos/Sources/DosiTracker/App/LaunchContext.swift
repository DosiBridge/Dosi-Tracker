import AppKit
import Foundation

// How the app was launched and why it is asked to quit, read from the Apple
// event being handled. The four-character codes are spelled out (their
// imported constant names have varied between SDKs).

@MainActor
enum LaunchContext {
    /// `kAEOpenApplication` ('oapp').
    private static let openApplicationEvent: AEEventID = 0x6F61_7070
    /// `keyAEPropData` ('prdt').
    private static let propertyDataKeyword: AEKeyword = 0x7072_6474
    /// `keyAELaunchedAsLogInItem` ('lgit').
    private static let launchedAsLogInItem: OSType = 0x6C67_6974
    /// `kAEQuitApplication` ('quit').
    private static let quitApplicationEvent: AEEventID = 0x7175_6974
    /// `kAEQuitReason` ('why?').
    private static let quitReasonKeyword: AEKeyword = 0x7768_793F
    /// `kAELogOut` ('logo'), `kAEReallyLogOut` ('rlgo'), `kAEShowRestartDialog`
    /// ('rrst'), `kAEShowShutdownDialog` ('rsdn'), `kAERestart` ('rest'),
    /// `kAEShutDown` ('shut').
    private static let systemQuitReasons: Set<OSType> = [
        0x6C6F_676F, 0x726C_676F, 0x7272_7374, 0x7273_646E, 0x7265_7374, 0x7368_7574,
    ]

    /// macOS opened the app at login (a login item). Read it while the
    /// launch event is being handled (`applicationWillFinishLaunching`).
    static func launchedAsLoginItem() -> Bool {
        guard let event = NSAppleEventManager.shared().currentAppleEvent,
              event.eventID == openApplicationEvent,
              let property = event.paramDescriptor(forKeyword: propertyDataKeyword)
        else {
            return false
        }
        return property.enumCodeValue == launchedAsLogInItem
    }

    /// The quit being handled comes from the system: the user is logging out,
    /// or the Mac restarts or shuts down. Read it in
    /// `applicationShouldTerminate`.
    static func isSystemQuit() -> Bool {
        guard let event = NSAppleEventManager.shared().currentAppleEvent,
              event.eventID == quitApplicationEvent,
              let reason = event.attributeDescriptor(forKeyword: quitReasonKeyword)
        else {
            return false
        }
        return systemQuitReasons.contains(reason.enumCodeValue)
    }
}
