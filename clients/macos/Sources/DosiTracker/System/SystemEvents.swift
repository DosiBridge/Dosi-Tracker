import AppKit
import DosiTrackerCore
import Foundation

/// The Mac's sleep, wake, power-off, session and screen-lock changes
/// (CONTRACT §4 "Lock", "Sleep / shutdown") as ``SystemEvent``s stamped with
/// the moment they arrived:
///
/// * NSWorkspace: `willSleep`, `didWake`, `willPowerOff`,
///   `sessionDidResignActive`, `sessionDidBecomeActive`, `screensDidSleep`
///   and `screensDidWake` notifications;
/// * the distributed notifications `com.apple.screenIsLocked` and
///   `com.apple.screenIsUnlocked`.
///
/// Subscribe with a handler (``subscribe(_:)``) or iterate ``stream()``.
/// Handlers run synchronously on the main thread, where the system posts
/// these notifications: keep them short and hand the event to the worker.
/// The event carries its moment, so the worker cuts the block at the lock or
/// sleep however late it gets to it — even after the Mac woke up. On
/// `willSleep` and `willPowerOff` a handler may wait briefly (at most
/// `AgentTiming.shutdownSaveBudget`) for the block to be queued locally; the
/// system carries on once the handlers have returned.
///
/// One change can arrive twice (a lock that also resigns the session), so
/// the worker treats repeats as no-ops.
///
/// Thread-safe: starting, stopping, subscribing and posting may happen on any
/// thread.
public final class SystemEvents: @unchecked Sendable {
    public typealias Handler = @Sendable (SystemEvent) -> Void

    /// Posted to the distributed center when the screen locks / unlocks.
    static let screenLockedName = Notification.Name("com.apple.screenIsLocked")
    static let screenUnlockedName = Notification.Name("com.apple.screenIsUnlocked")

    /// The NSWorkspace notifications observed, and what each means.
    static let workspaceEvents: [(name: Notification.Name, kind: SystemEventKind)] = [
        (NSWorkspace.willSleepNotification, .willSleep),
        (NSWorkspace.didWakeNotification, .didWake),
        (NSWorkspace.willPowerOffNotification, .willPowerOff),
        (NSWorkspace.sessionDidResignActiveNotification, .sessionDidResignActive),
        (NSWorkspace.sessionDidBecomeActiveNotification, .sessionDidBecomeActive),
        (NSWorkspace.screensDidSleepNotification, .screensDidSleep),
        (NSWorkspace.screensDidWakeNotification, .screensDidWake),
    ]

    private let lock = NSLock()
    private var handlers: [UUID: Handler] = [:]
    private var workspaceObservers: [NSObjectProtocol] = []
    private var distributedObservers: [NSObjectProtocol] = []

    public init() {}

    deinit {
        stop()
    }

    /// Start observing; calling it again does nothing.
    public func start() {
        lock.lock()
        defer { lock.unlock() }
        guard workspaceObservers.isEmpty && distributedObservers.isEmpty else { return }

        let workspace = NSWorkspace.shared.notificationCenter
        for event in SystemEvents.workspaceEvents {
            let kind = event.kind
            workspaceObservers.append(workspace.addObserver(forName: event.name, object: nil, queue: nil) { [weak self] _ in
                self?.post(kind)
            })
        }
        let distributed = DistributedNotificationCenter.default()
        distributedObservers.append(distributed.addObserver(forName: SystemEvents.screenLockedName, object: nil,
                                                            queue: nil) { [weak self] _ in
            self?.post(.screenLocked)
        })
        distributedObservers.append(distributed.addObserver(forName: SystemEvents.screenUnlockedName, object: nil,
                                                            queue: nil) { [weak self] _ in
            self?.post(.screenUnlocked)
        })
    }

    /// Stop observing (subscribers stay registered for a later ``start()``).
    public func stop() {
        lock.lock()
        let workspace = workspaceObservers
        let distributed = distributedObservers
        workspaceObservers = []
        distributedObservers = []
        lock.unlock()
        for observer in workspace {
            NSWorkspace.shared.notificationCenter.removeObserver(observer)
        }
        for observer in distributed {
            DistributedNotificationCenter.default().removeObserver(observer)
        }
    }

    /// Call `handler` with every event from now on; the token unsubscribes.
    @discardableResult
    public func subscribe(_ handler: @escaping Handler) -> UUID {
        let token = UUID()
        lock.lock()
        handlers[token] = handler
        lock.unlock()
        return token
    }

    public func unsubscribe(_ token: UUID) {
        lock.lock()
        handlers[token] = nil
        lock.unlock()
    }

    /// The events from now on, as an async sequence (one per caller); ending
    /// the iteration unsubscribes.
    public func stream() -> AsyncStream<SystemEvent> {
        AsyncStream { continuation in
            let token = self.subscribe { event in
                continuation.yield(event)
            }
            continuation.onTermination = { [weak self] _ in
                self?.unsubscribe(token)
            }
        }
    }

    /// Deliver an event to every subscriber. The app posts its own too —
    /// e.g. `willPowerOff` from `applicationShouldTerminate` when the system
    /// logs out — so every path ends the block the same way.
    public func post(_ kind: SystemEventKind, at moment: Date = Date()) {
        let event = SystemEvent(kind: kind, at: moment)
        lock.lock()
        let current = Array(handlers.values)
        lock.unlock()
        for handler in current {
            handler(event)
        }
    }
}
