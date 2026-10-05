import Foundation
import ServiceManagement

// "Start at login" through `SMAppService.mainApp` (macOS 13+; port of the
// Windows agent's `autostart.rs`): the app registers itself as a login item,
// shown in System Settings → General → Login Items, where the user can also
// turn it off. It needs the app bundle; a `swift run` binary cannot register.

enum LoginItemState: Equatable, Sendable {
    case enabled
    case disabled
    /// Registered, but the user must allow it in System Settings → Login Items.
    case requiresApproval
    /// The system cannot find the app (it does not run from an app bundle).
    case unavailable
}

enum LoginItem {
    static var state: LoginItemState {
        switch SMAppService.mainApp.status {
        case .enabled:
            return .enabled
        case .requiresApproval:
            return .requiresApproval
        case .notRegistered:
            return .disabled
        case .notFound:
            return .unavailable
        @unknown default:
            return .disabled
        }
    }

    /// Register or unregister the app as a login item. Synchronous on purpose:
    /// `unregister` also has an async overload, which an async caller would
    /// pick instead.
    static func setEnabled(_ enabled: Bool) throws {
        if enabled {
            try SMAppService.mainApp.register()
        } else {
            try SMAppService.mainApp.unregister()
        }
    }

    /// Open System Settings → General → Login Items (to approve the item).
    static func openSystemSettings() {
        SMAppService.openSystemSettingsLoginItems()
    }
}
