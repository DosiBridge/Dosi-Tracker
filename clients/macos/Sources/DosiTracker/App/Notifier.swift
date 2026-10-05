import DosiTrackerCore
import Foundation
import UserNotifications

// User notifications for the things worth telling once (port of the Windows
// tray balloons): tracking started by itself at login, choose a project, the
// project became unavailable, the sign-in expired, the local queue was reset.
// Permission is asked the first time one is posted. Notifications need the
// app bundle; a `swift run` binary only logs them.

final class Notifier: NSObject, UNUserNotificationCenterDelegate, @unchecked Sendable {
    private let lock = NSLock()
    private var authorizationRequested = false
    private var openHandler: ((String) -> Void)?

    /// Notifications work only from the app bundle (the notification center
    /// refuses a process without one).
    static var isAvailable: Bool {
        Bundle.main.bundleURL.pathExtension == "app" && Bundle.main.bundleIdentifier != nil
    }

    private var center: UNUserNotificationCenter? {
        Notifier.isAvailable ? UNUserNotificationCenter.current() : nil
    }

    /// Become the notification center's delegate; `onOpen` runs on the main
    /// thread with the identifier of a notification the user clicked.
    func start(onOpen: @escaping (String) -> Void) {
        lock.lock()
        openHandler = onOpen
        lock.unlock()
        center?.delegate = self
    }

    /// Show `body` under `title`; `id` replaces an earlier one with the same id.
    func post(id: String, title: String, body: String) {
        guard let center = center else {
            CoreLog.info("notify", "\(title): \(body)")
            return
        }
        requestAuthorizationOnce(center)
        let content = UNMutableNotificationContent()
        content.title = title
        content.body = body
        let request = UNNotificationRequest(identifier: id, content: content, trigger: nil)
        center.add(request) { error in
            if let error = error {
                CoreLog.warning("notify", "could not show a notification: \(error.localizedDescription)")
            }
        }
    }

    private func requestAuthorizationOnce(_ center: UNUserNotificationCenter) {
        lock.lock()
        let first = !authorizationRequested
        authorizationRequested = true
        lock.unlock()
        guard first else { return }
        center.requestAuthorization(options: [.alert, .sound]) { granted, error in
            if let error = error {
                CoreLog.warning("notify", "notification permission: \(error.localizedDescription)")
            } else if !granted {
                CoreLog.info("notify", "notifications are turned off for Dosi Tracker")
            }
        }
    }

    // MARK: UNUserNotificationCenterDelegate

    /// Shown even while one of the app's windows is in front.
    func userNotificationCenter(_ center: UNUserNotificationCenter, willPresent notification: UNNotification,
                                withCompletionHandler completionHandler: @escaping (UNNotificationPresentationOptions) -> Void) {
        completionHandler([.banner, .list])
    }

    func userNotificationCenter(_ center: UNUserNotificationCenter, didReceive response: UNNotificationResponse,
                                withCompletionHandler completionHandler: @escaping () -> Void) {
        let id = response.notification.request.identifier
        lock.lock()
        let handler = openHandler
        lock.unlock()
        DispatchQueue.main.async {
            handler?(id)
        }
        completionHandler()
    }
}
