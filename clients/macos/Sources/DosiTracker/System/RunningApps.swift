import AppKit
import DosiTrackerCore
import Foundation

/// The programs running now, by name only (CONTRACT §1 `runningPrograms`;
/// the Windows agent's `running_programs`): the regular apps — those with a
/// Dock icon and windows — never background agents or menu-bar helpers, and
/// never a window title (the titles of background windows can hold document
/// names or e-mail subjects). Names are the apps' localized names, as for the
/// app in front (CONTRACT §8); duplicates and the server's cap are handled by
/// `BlockAssembler.runningProgramRows`.
///
/// Thread-safe: NSWorkspace's list of running applications may be read from
/// any thread (it is returned atomically).
public final class RunningApps: RunningProgramsSource {
    public init() {}

    public func runningProgramNames() -> [String] {
        NSWorkspace.shared.runningApplications.compactMap { (app: NSRunningApplication) -> String? in
            guard app.activationPolicy == .regular, !app.isTerminated else { return nil }
            return AppNaming.appName(localizedName: app.localizedName,
                                     bundleFileName: app.bundleURL?.lastPathComponent,
                                     executableName: app.executableURL?.lastPathComponent)
        }
    }
}
