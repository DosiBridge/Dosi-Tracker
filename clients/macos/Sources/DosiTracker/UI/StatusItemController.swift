import AppKit
import DosiTrackerCore
import Foundation

// The menu-bar item (port of the Windows agent's `tray.rs`): the icon with a
// per-state glyph, the session timer as its title while tracking, a tooltip
// with the status and today's total, and the everyday actions one click away
// — the same menu as the Windows tray: status, today / this week, Open,
// Pause / Resume, timed breaks, Switch project, Switch workspace, Set task…,
// Sync now, Open web dashboard, Privacy consent, Settings, Start at login,
// Quit. The menu is rebuilt each time it opens, so it always shows the
// current state.

@MainActor
final class StatusItemController: NSObject, NSMenuDelegate {
    private let model: AppModel
    private let windows: WindowManager
    private let statusItem: NSStatusItem
    private var timer: Timer?
    private var shownBadge: StatusBadge?
    private var shownTitle = ""
    private var shownToolTip = ""
    private var quitting = false

    init(model: AppModel, windows: WindowManager) {
        self.model = model
        self.windows = windows
        statusItem = NSStatusBar.system.statusItem(withLength: NSStatusItem.variableLength)
        super.init()

        let menu = NSMenu()
        menu.autoenablesItems = false
        menu.delegate = self
        statusItem.menu = menu
        if let button = statusItem.button {
            button.imagePosition = .imageLeading
            button.font = NSFont.monospacedDigitSystemFont(ofSize: NSFont.systemFontSize, weight: .regular)
        }
        refresh()

        // The title is a clock: refreshed every second (only changes touch
        // the menu bar).
        let ticker = Timer(timeInterval: 1, repeats: true) { [weak self] _ in
            // The timer is scheduled on the main run loop, so it fires on the
            // main thread: refresh in place instead of re-capturing the weak
            // `self` in a task (Swift 5.10 rejects that capture).
            MainActor.assumeIsolated {
                self?.refresh()
            }
        }
        RunLoop.main.add(ticker, forMode: .common)
        timer = ticker
    }

    /// Quitting: the icon says so while the last block is saved and uploaded.
    func showQuitting() {
        quitting = true
        statusItem.button?.title = ""
        statusItem.button?.toolTip = "Dosi Tracker — saving your work before quitting…"
        shownTitle = ""
    }

    /// Bring the icon, title and tooltip in line with the snapshot.
    func refresh() {
        guard !quitting else { return }
        let snapshot = model.snapshot
        let now = Date()
        let badge = StatusBadge.of(snapshot.status)
        if badge != shownBadge {
            statusItem.button?.image = StatusIcon.image(badge)
            shownBadge = badge
        }
        var title = ""
        if snapshot.status == .tracking, let since = snapshot.trackingSince {
            title = " " + Formatting.shortClock(now.timeIntervalSince(since))
        }
        if title != shownTitle {
            statusItem.button?.title = title
            shownTitle = title
        }
        let toolTip = toolTipText(snapshot, now: now)
        if toolTip != shownToolTip {
            statusItem.button?.toolTip = toolTip
            shownToolTip = toolTip
        }
    }

    // MARK: Text

    private func stateText(_ snapshot: TrackerSnapshot, now: Date) -> String {
        switch model.pendingAction {
        case .some(.pausing):
            return "Pausing…"
        case .some(.resuming):
            return "Resuming…"
        case .none:
            break
        }
        switch snapshot.status {
        case .tracking:
            let since = snapshot.trackingSince ?? now
            return "Tracking · \(Formatting.minutesLabel(Int(now.timeIntervalSince(since) / 60)))"
        case .paused, .onBreak:
            if let resumeAt = snapshot.resumeAt {
                return "On a break · resumes at \(Formatting.time(resumeAt))"
            }
            let since = snapshot.pausedSince ?? now
            return "Paused · \(Formatting.minutesLabel(Int(now.timeIntervalSince(since) / 60)))"
        case .away:
            if let since = snapshot.awaySince {
                return "Away since \(Formatting.time(since))"
            }
            return snapshot.status.label
        default:
            return snapshot.status.label
        }
    }

    private func toolTipText(_ snapshot: TrackerSnapshot, now: Date) -> String {
        var text = "Dosi Tracker — \(stateText(snapshot, now: now))"
        if snapshot.status.isSignedIn {
            let today = Formatting.secondsLabel(model.liveTotals(now: now).today)
            if let project = snapshot.currentProject {
                text += "\n\(project.title) · Today \(today)"
            } else {
                text += "\nToday \(today)"
            }
        }
        return text
    }

    // MARK: NSMenuDelegate

    nonisolated func menuNeedsUpdate(_ menu: NSMenu) {
        MainActor.assumeIsolated {
            self.rebuild(menu)
        }
    }

    private func rebuild(_ menu: NSMenu) {
        menu.removeAllItems()
        let snapshot = model.snapshot
        let now = Date()
        let status = snapshot.status
        let signedIn = status.isSignedIn
        let idle = model.pendingAction == nil

        var statusLine = "● \(stateText(snapshot, now: now))"
        if signedIn, let project = snapshot.currentProject {
            statusLine += " — \(project.title)"
        }
        menu.addItem(info(statusLine))
        if signedIn {
            let totals = model.liveTotals(now: now)
            menu.addItem(info("Today \(Formatting.secondsLabel(totals.today)) · This week \(Formatting.secondsLabel(totals.week))"))
        } else if status == .choosingWorkspace {
            menu.addItem(info("Choose a workspace to start tracking"))
        } else {
            menu.addItem(info(status == .signInExpired ? "Enter your password to continue" : "Sign in to start tracking"))
        }
        addWarnings(to: menu, snapshot)

        menu.addItem(.separator())
        menu.addItem(action("Open Dosi Tracker", #selector(openWindow), key: "o"))
        menu.addItem(.separator())

        // Pause / Resume (or what the user must do first).
        switch status {
        case .signedOut, .signInExpired:
            menu.addItem(action("Sign in…", #selector(openSignIn)))
        case .choosingWorkspace:
            menu.addItem(action("Choose a workspace…", #selector(openWorkspaces)))
        case .choosingProject:
            menu.addItem(action("Choose a project…", #selector(openChooser), enabled: !snapshot.projects.isEmpty))
        case .consentRequired:
            menu.addItem(action("Choose what to record…", #selector(openConsent)))
        case .paused, .onBreak:
            menu.addItem(action(model.pendingAction == .resuming ? "Resuming…" : "Resume tracking",
                                #selector(toggleTracking), enabled: idle))
        default:
            menu.addItem(action(model.pendingAction == .pausing ? "Pausing…" : "Pause tracking",
                                #selector(toggleTracking), enabled: idle && status.canPause))
        }

        // Timed breaks; while paused they re-arm the automatic resume.
        let breaks = NSMenuItem(title: status.isPaused ? "Resume after" : "Take a break", action: nil, keyEquivalent: "")
        let breakMenu = NSMenu()
        breakMenu.autoenablesItems = false
        for choice in AppModel.breakChoices {
            let item = action(choice.label, #selector(takeBreak(_:)))
            item.representedObject = NSNumber(value: choice.seconds)
            breakMenu.addItem(item)
        }
        if status == .onBreak {
            breakMenu.addItem(.separator())
            breakMenu.addItem(action("Until I resume", #selector(pauseUntilResumed)))
        }
        breaks.submenu = breakMenu
        breaks.isEnabled = idle && (status.canPause || status.isPaused)
        menu.addItem(breaks)

        // Switch project, with a check mark on the current one.
        let projects = NSMenuItem(title: "Switch project", action: nil, keyEquivalent: "")
        let projectMenu = NSMenu()
        projectMenu.autoenablesItems = false
        for project in snapshot.projects {
            let item = action(project.title, #selector(selectProject(_:)))
            item.representedObject = project.id
            item.state = project.id.caseInsensitiveCompare(snapshot.selectedProjectId ?? "") == .orderedSame ? .on : .off
            projectMenu.addItem(item)
        }
        projects.submenu = projectMenu
        projects.isEnabled = signedIn && idle && !snapshot.projects.isEmpty
        menu.addItem(projects)
        menu.addItem(action("Switch workspace…", #selector(switchWorkspace), enabled: signedIn && idle))

        let note = snapshot.taskNote
        let shortNote = note.count > 40 ? String(note.prefix(40)) + "…" : note
        let taskTitle = note.isEmpty ? "Set task…" : "Task: \(shortNote) — Change…"
        menu.addItem(action(taskTitle, #selector(setTask), enabled: signedIn))
        menu.addItem(action(status.isPaused ? "Upload queued" : "Sync now", #selector(syncNow),
                            enabled: signedIn))

        menu.addItem(.separator())
        menu.addItem(action("Open web dashboard", #selector(openDashboard)))
        menu.addItem(action("Privacy consent…", #selector(openConsent), enabled: signedIn))
        menu.addItem(action("Settings…", #selector(openSettings), key: ","))
        let login = action("Start at login", #selector(toggleStartAtLogin), enabled: model.loginItem != .unavailable)
        login.state = model.startsAtLogin ? .on : .off
        menu.addItem(login)
        menu.addItem(.separator())
        menu.addItem(action("Quit Dosi Tracker", #selector(quit), key: "q"))
    }

    /// Problems worth seeing from the menu: each opens what fixes it.
    private func addWarnings(to menu: NSMenu, _ snapshot: TrackerSnapshot) {
        if case .stopped(let reason) = snapshot.status {
            menu.addItem(info("⚠︎ \(reason)"))
        }
        if snapshot.status == .consentRequired {
            menu.addItem(action("⚠︎ Nothing is recorded until you choose what may be…", #selector(openConsent)))
        }
        if let issue = snapshot.uploadIssue {
            menu.addItem(info("⚠︎ \(issue.message)"))
        }
        if snapshot.rejectedUploads > 0 {
            menu.addItem(action("⚠︎ \(Formatting.count(snapshot.rejectedUploads, "block")) rejected by the server…",
                                #selector(openActivity)))
        }
        if let clock = snapshot.clockWarning {
            menu.addItem(info("⚠︎ \(clock)"))
        }
        if snapshot.status.isSignedIn {
            for permission in model.missingPermissions {
                let item = action("⚠︎ Allow \(permission.title)…", #selector(fixPermission(_:)))
                item.representedObject = permission.rawValue
                menu.addItem(item)
            }
        }
    }

    private func info(_ title: String) -> NSMenuItem {
        let item = NSMenuItem(title: title, action: nil, keyEquivalent: "")
        item.isEnabled = false
        return item
    }

    private func action(_ title: String, _ selector: Selector, key: String = "", enabled: Bool = true) -> NSMenuItem {
        let item = NSMenuItem(title: title, action: selector, keyEquivalent: key)
        item.target = self
        item.isEnabled = enabled
        return item
    }

    // MARK: Actions

    @objc private func openWindow() {
        windows.showPrimary()
    }

    @objc private func openSignIn() {
        windows.showSignIn()
    }

    @objc private func openChooser() {
        windows.showChooseProject()
    }

    @objc private func openWorkspaces() {
        windows.showChooseWorkspace()
        if model.snapshot.workspaceChoices.isEmpty && !model.snapshot.findingWorkspaces {
            model.switchWorkspace()
        }
    }

    @objc private func switchWorkspace() {
        model.switchWorkspace()
        windows.showChooseWorkspace()
    }

    @objc private func openActivity() {
        windows.showMain(tab: .activity)
    }

    @objc private func openSettings() {
        windows.showMain(tab: .settings)
    }

    /// The consent card: on the Timer tab while tracking waits for it,
    /// otherwise in Settings.
    @objc private func openConsent() {
        windows.showMain(tab: model.snapshot.status == .consentRequired ? .timer : .settings)
    }

    @objc private func toggleTracking() {
        model.toggleTracking()
    }

    @objc private func takeBreak(_ sender: NSMenuItem) {
        guard let seconds = sender.representedObject as? NSNumber else { return }
        model.pause(for: seconds.doubleValue)
    }

    @objc private func pauseUntilResumed() {
        model.pause(for: nil)
    }

    @objc private func selectProject(_ sender: NSMenuItem) {
        guard let projectId = sender.representedObject as? String else { return }
        model.selectProject(projectId)
    }

    @objc private func setTask() {
        if let note = TaskNotePrompt.ask(current: model.snapshot.taskNote) {
            model.setTaskNote(note)
        }
    }

    @objc private func syncNow() {
        model.syncNow()
    }

    @objc private func openDashboard() {
        model.openDashboard()
    }

    @objc private func toggleStartAtLogin() {
        model.setStartAtLogin(!model.startsAtLogin)
        if let error = model.settingsError {
            windows.showMain(tab: .settings)
            CoreLog.warning("app", error)
        }
    }

    @objc private func fixPermission(_ sender: NSMenuItem) {
        guard let raw = sender.representedObject as? String, let permission = PrivacyPermission(rawValue: raw) else { return }
        model.fix(permission)
    }

    @objc private func quit() {
        NSApp.terminate(nil)
    }
}
