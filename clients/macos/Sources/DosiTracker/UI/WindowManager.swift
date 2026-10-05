import AppKit
import DosiTrackerCore
import SwiftUI

// The app's windows: SwiftUI views hosted in NSWindows (NSHostingController),
// created on first use and kept (closing only hides them). It also decides
// when a window appears by itself, from the snapshots: the sign-in form when
// a sign-in is needed, the project chooser when the agent cannot pick one,
// and the idle prompt when the user comes back.

@MainActor
final class WindowManager: NSObject, NSWindowDelegate {
    private let model: AppModel
    private var mainWindow: NSWindow?
    private var signInWindow: NSWindow?
    private var idleWindow: NSWindow?
    private var chooseWindow: NSWindow?
    private var receivedFirstSnapshot = false
    /// The idle prompt was answered with a button (closing it otherwise
    /// discards the time away, the default).
    private var idleAnswered = false

    init(model: AppModel) {
        self.model = model
        super.init()
    }

    // MARK: Showing

    /// The window that matters now: the sign-in form when signed out, else the
    /// main window.
    func showPrimary() {
        if model.snapshot.status.needsSignIn {
            showSignIn()
        } else {
            showMain(tab: nil)
        }
    }

    func showMain(tab: MainTab?) {
        if model.snapshot.status.needsSignIn {
            showSignIn()
            return
        }
        if let tab = tab {
            model.selectedTab = tab
        }
        let window = mainWindow ?? makeWindow(title: "Dosi Tracker", content: MainView(model: model),
                                              size: NSSize(width: 500, height: 760), resizable: true,
                                              autosaveName: "DosiTrackerMainWindow")
        mainWindow = window
        present(window)
    }

    func showSignIn() {
        let window = signInWindow ?? makeWindow(title: "Sign in to Dosi Tracker", content: SignInView(model: model),
                                                size: NSSize(width: 440, height: 620), resizable: true,
                                                autosaveName: nil)
        signInWindow = window
        present(window)
    }

    func showChooseProject() {
        let content = ChooseProjectView(model: model) { [weak self] in
            self?.chooseWindow?.close()
        }
        let window = chooseWindow ?? makeWindow(title: "Choose a project", content: content,
                                                size: NSSize(width: 400, height: 360), resizable: false,
                                                autosaveName: nil)
        chooseWindow = window
        present(window)
    }

    func showIdlePrompt() {
        idleAnswered = false
        let content = IdlePromptView(model: model) { [weak self] in
            self?.idleAnswered = true
            self?.idleWindow?.close()
        }
        let window = idleWindow ?? makeWindow(title: "Welcome back", content: content,
                                              size: NSSize(width: 420, height: 200), resizable: false,
                                              autosaveName: nil)
        window.level = .floating
        idleWindow = window
        present(window)
    }

    private func present(_ window: NSWindow) {
        NSApp.activate()
        window.makeKeyAndOrderFront(nil)
        // Shown even when macOS declines to activate the app right now.
        window.orderFrontRegardless()
    }

    private func makeWindow<Content: View>(title: String, content: Content, size: NSSize, resizable: Bool,
                                           autosaveName: String?) -> NSWindow {
        let controller = NSHostingController(rootView: content)
        controller.sizingOptions = resizable ? [.minSize] : [.preferredContentSize]
        let window = NSWindow(contentViewController: controller)
        window.title = title
        var style: NSWindow.StyleMask = [.titled, .closable, .miniaturizable]
        if resizable {
            style.insert(.resizable)
        }
        window.styleMask = style
        window.isReleasedWhenClosed = false
        window.delegate = self
        window.setContentSize(size)
        if let name = autosaveName {
            _ = window.setFrameAutosaveName(name)
            if !window.setFrameUsingName(name) {
                window.center()
            }
        } else {
            window.center()
        }
        return window
    }

    // MARK: Reacting to the worker

    func snapshotChanged(from old: TrackerSnapshot, to new: TrackerSnapshot) {
        let first = !receivedFirstSnapshot
        receivedFirstSnapshot = true

        // A sign-in is needed: at the first launch, after signing out, when
        // the saved password stopped working.
        if new.status.needsSignIn && (first || !old.status.needsSignIn) {
            closeWindow(chooseWindow)
            closeWindow(idleWindow)
            if mainWindow?.isVisible == true || first || new.status == .signInExpired || old.status.isSignedIn {
                closeWindow(mainWindow)
                showSignIn()
            }
        }
        // Signed in from the form: on to the main window (or the chooser).
        if !new.status.needsSignIn && (old.status.needsSignIn || first) && signInWindow?.isVisible == true {
            closeWindow(signInWindow)
            if new.status != .choosingProject {
                showMain(tab: .timer)
            }
        }
        // The agent cannot pick the project (CONTRACT §2): ask.
        if new.status == .choosingProject && (old.status != .choosingProject || first) && !new.projects.isEmpty {
            showChooseProject()
        }
        if old.status == .choosingProject && new.status != .choosingProject {
            closeWindow(chooseWindow)
        }
        // Back from idle: keep or discard the time away.
        if let prompt = new.awayPrompt, prompt != old.awayPrompt {
            showIdlePrompt()
        } else if new.awayPrompt == nil && old.awayPrompt != nil {
            idleAnswered = true
            closeWindow(idleWindow)
        }
        // Nothing can be recorded: say why, loudly.
        if case .stopped = new.status, first {
            showMain(tab: .timer)
        }
    }

    private func closeWindow(_ window: NSWindow?) {
        if let open = window, open.isVisible {
            open.close()
        }
    }

    // MARK: NSWindowDelegate

    nonisolated func windowWillClose(_ notification: Notification) {
        let closing = notification.object as? NSWindow
        MainActor.assumeIsolated {
            self.windowClosing(closing)
        }
    }

    private func windowClosing(_ window: NSWindow?) {
        guard let window = window, window === idleWindow else { return }
        // Closed without an answer: the default, discard.
        if !idleAnswered && model.snapshot.awayPrompt != nil {
            model.resolveAway(keep: false)
        }
        idleAnswered = true
    }
}
