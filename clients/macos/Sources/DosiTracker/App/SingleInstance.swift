import AppKit
import DosiTrackerCore
import Foundation

// One copy of the agent per user session (port of the Windows agent's
// `single_instance.rs`). Two copies would each capture and upload with their
// own idempotency keys, which the server cannot deduplicate: the overlapping
// blocks of one copy would be refused and twice the screenshots taken.
//
// Two checks: another running process with our bundle identifier (what
// LaunchServices launched), and an exclusive `flock` on a lock file in
// Application Support, which also covers copies started from a terminal or
// from two different bundle locations. A second launch asks the running copy
// — over the distributed notification center — to show its window, then
// exits.

@MainActor
enum SingleInstance {
    /// Posted by a second launch: the running copy shows its window.
    static let showWindowNotification = Notification.Name("com.dosibridge.tracker.show-window")

    /// The lock, held for the life of the process.
    final class Lock {
        private let descriptor: Int32

        fileprivate init(descriptor: Int32) {
            self.descriptor = descriptor
        }

        deinit {
            _ = flock(descriptor, LOCK_UN)
            _ = close(descriptor)
        }
    }

    enum Outcome {
        /// This is the only copy (the lock is nil when the lock file could not
        /// be used; the bundle check passed).
        case acquired(Lock?)
        /// Another copy runs.
        case alreadyRunning
    }

    static func acquire(lockFile: URL) -> Outcome {
        if otherCopy() != nil {
            return .alreadyRunning
        }
        let descriptor = open(lockFile.path, O_CREAT | O_RDWR, 0o600)
        guard descriptor >= 0 else {
            CoreLog.warning("app", "could not open the lock file (\(String(cString: strerror(errno)))); not checking for another copy")
            return .acquired(nil)
        }
        if flock(descriptor, LOCK_EX | LOCK_NB) != 0 {
            let failure = errno
            _ = close(descriptor)
            if failure == EWOULDBLOCK {
                return .alreadyRunning
            }
            CoreLog.warning("app", "could not lock the lock file (\(String(cString: strerror(failure)))); not checking for another copy")
            return .acquired(nil)
        }
        return .acquired(Lock(descriptor: descriptor))
    }

    /// Another running copy launched from a bundle, if any.
    static func otherCopy() -> NSRunningApplication? {
        let me = ProcessInfo.processInfo.processIdentifier
        return NSRunningApplication.runningApplications(withBundleIdentifier: AgentInfo.bundleIdentifier)
            .first { $0.processIdentifier != me && !$0.isTerminated }
    }

    /// Ask the running copy to show its window and come to the front.
    static func showRunningCopy() {
        DistributedNotificationCenter.default().postNotificationName(showWindowNotification, object: nil,
                                                                      userInfo: nil, deliverImmediately: true)
        _ = otherCopy()?.activate(options: [])
    }
}
