import Foundation

/// The name an app goes by in uploads (CONTRACT §8: on macOS the app's
/// localized name, e.g. `Google Chrome`; the server and the web map it to the
/// same display name as the Windows executable).
///
/// Thread-safe: stateless.
public enum AppNaming {
    /// The longest app name the server stores.
    public static let maxAppNameLength = 256
    /// The longest window title the server stores.
    public static let maxWindowTitleLength = 512

    /// The app's localized name (`NSRunningApplication.localizedName`); else
    /// its bundle's file name without `.app`; else its executable's name; nil
    /// when none is known. Cut to the server's length.
    public static func appName(localizedName: String?, bundleFileName: String?, executableName: String?) -> String? {
        if let name = localizedName?.nonEmptyTrimmed {
            return name.truncatedUTF16(maxAppNameLength)
        }
        if let bundle = bundleFileName?.nonEmptyTrimmed {
            let stem = bundle.lowercased().hasSuffix(".app") ? String(bundle.dropLast(4)) : bundle
            if let name = stem.nonEmptyTrimmed {
                return name.truncatedUTF16(maxAppNameLength)
            }
        }
        if let name = executableName?.nonEmptyTrimmed {
            return name.truncatedUTF16(maxAppNameLength)
        }
        return nil
    }
}

extension ForegroundWindow {
    /// A sampled front window, its texts cut to what the server stores (app
    /// names 256, window titles 512 UTF-16 units), so the per-window rows
    /// built from the samples are the rows the server keeps.
    public static func sampled(appName: String, windowTitle: String) -> ForegroundWindow {
        ForegroundWindow(appName: appName.truncatedUTF16(AppNaming.maxAppNameLength),
                         windowTitle: windowTitle.truncatedUTF16(AppNaming.maxWindowTitleLength))
    }
}
