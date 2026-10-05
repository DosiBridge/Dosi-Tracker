import DosiTrackerCore
import SwiftUI

// The Settings tab (port of the Windows agent's `ui/settings.rs`): start at
// login, the user's goals, the macOS privacy permissions with a way to fix
// each, what the current project records (the admin's policy, read-only),
// this Mac, the account with a confirmed sign-out, and help (logs, data
// folder).

struct SettingsTab: View {
    @ObservedObject var model: AppModel
    @State private var confirmSignOut = false

    private var snapshot: TrackerSnapshot { model.snapshot }

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            if let error = model.settingsError {
                Banner(.danger, error)
            }
            generalCard
            goalsCard
            permissionsCard
            policyCard
            deviceCard
            accountCard
            helpCard
        }
        .alert("Sign out of Dosi Tracker?", isPresented: $confirmSignOut) {
            Button("Sign out", role: .destructive) {
                model.signOut()
            }
            Button("Cancel", role: .cancel) {}
        } message: {
            Text("The block in progress is saved and uploaded first. Tracking stops on this Mac until someone signs in again.")
        }
    }

    // MARK: General

    private var generalCard: some View {
        Card("General") {
            settingRow("Start at login", "Open Dosi Tracker when you log in to this Mac.") {
                Toggle("", isOn: Binding(get: { model.startsAtLogin }, set: { model.setStartAtLogin($0) }))
                    .toggleStyle(.switch)
                    .labelsHidden()
                    .disabled(model.loginItem == .unavailable)
            }
            switch model.loginItem {
            case .requiresApproval:
                HStack {
                    Banner(.warning, "macOS asks you to allow Dosi Tracker in System Settings → General → Login Items.")
                    Button("Open…") {
                        model.openLoginItemSettings()
                    }
                }
            case .unavailable:
                Banner(.neutral, "Start at login needs the Dosi Tracker app (it is not available when run from a terminal).")
            case .enabled, .disabled:
                EmptyView()
            }
            Text("Closing the window keeps Dosi Tracker running in the menu bar. To stop it, choose Quit from its menu.")
                .font(.caption)
                .foregroundStyle(.secondary)
                .fixedSize(horizontal: false, vertical: true)
        }
    }

    // MARK: Goals

    private var goalsCard: some View {
        let goals = model.goals
        return Card("Goals") {
            settingRow("Daily goal", "Hours you aim to track each working day.") {
                Stepper(value: Binding(get: { model.goals.dailyHours }, set: { model.setDailyGoal(hours: $0) }),
                        in: Goals.minimumDailyHours...Goals.maximumDailyHours, step: 0.5) {
                    Text(Formatting.hoursLabel(goals.dailyHours)).monospacedDigit()
                }
            }
            Divider()
            settingRow("Working days", "Days a week; sets the weekly goal.") {
                Picker("", selection: Binding(get: { model.goals.workDaysPerWeek }, set: { model.setWorkDays($0) })) {
                    Text("5 days").tag(5)
                    Text("6 days").tag(6)
                    Text("7 days").tag(7)
                }
                .pickerStyle(.segmented)
                .labelsHidden()
                .frame(width: 200)
            }
            Text("Weekly goal: \(Formatting.hoursLabel(goals.weeklySeconds / 3600)). Goals are personal and only shown on this Mac.")
                .font(.caption)
                .foregroundStyle(.secondary)
        }
    }

    // MARK: Permissions

    private var permissionsCard: some View {
        let needed = snapshot.capture.permissions
        return Card("Permissions", trailing: "System Settings → Privacy & Security") {
            ForEach(PrivacyPermission.allCases, id: \.self) { permission in
                permissionRow(permission, needed: isNeeded(permission, by: needed))
                if permission != PrivacyPermission.allCases.last {
                    Divider()
                }
            }
            HStack(spacing: 6) {
                Text("Keyboard and mouse capture:").font(.caption).foregroundStyle(.secondary)
                Text(inputCaptureText).font(.caption.weight(.semibold))
            }
            Text("macOS ties each permission to this app's signature. Screen Recording takes effect after Dosi Tracker is restarted. Nothing is captured beyond what your project records.")
                .font(.caption)
                .foregroundStyle(.secondary)
                .fixedSize(horizontal: false, vertical: true)
        }
    }

    private func permissionRow(_ permission: PrivacyPermission, needed: Bool) -> some View {
        let state = model.permissions.state(of: permission)
        return HStack(alignment: .center, spacing: 10) {
            Image(systemName: state.isGranted ? "checkmark.shield.fill" : "exclamationmark.shield")
                .foregroundStyle(state.isGranted ? Theme.success : (needed ? Theme.warning : Color.secondary))
                .font(.system(size: 18))
            VStack(alignment: .leading, spacing: 1) {
                Text(permission.title).font(.callout.weight(.semibold))
                Text(permission.purpose + (needed ? "" : " — not used by this project"))
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .fixedSize(horizontal: false, vertical: true)
            }
            Spacer(minLength: 6)
            if state.isGranted {
                Pill("Allowed", tone: .success)
            } else {
                Button(state == .notDetermined ? "Allow…" : "Open Settings…") {
                    model.fix(permission)
                }
                .controlSize(.small)
            }
        }
    }

    private func isNeeded(_ permission: PrivacyPermission, by policy: CapturePermissions) -> Bool {
        switch permission {
        case .screenRecording:
            return policy.screenshot
        case .inputMonitoring:
            return policy.measuresInput
        case .accessibility:
            return policy.activeWindow
        case .camera:
            return policy.webcam
        }
    }

    private var inputCaptureText: String {
        switch model.inputCapture {
        case .running:
            return "running"
        case .waitingForPermission:
            return "waiting for Input Monitoring"
        case .failed:
            return "could not start (retrying)"
        case .stopped:
            return "stopped"
        }
    }

    // MARK: Policy

    private var policyCard: some View {
        let flags = snapshot.capture
        let project = snapshot.currentProject?.title ?? "your project"
        return Card("What is recorded", trailing: project) {
            policyRow("camera", "Screenshots", flags.screenshot,
                      flags.blurScreenshots
                          ? "One per block, of the screen you work on — blurred before upload"
                          : "One per block, of the screen you work on")
            policyRow("video", "Webcam photo", flags.webcam, "One per block")
            policyRow("keyboard", "Keyboard activity", flags.keyboard, "Counts only — never what you type")
            policyRow("computermouse", "Mouse activity", flags.mouse, "Clicks and movement, as counts")
            policyRow("macwindow", "Apps & window titles", flags.activeWindow, "Which app was in front, and for how long")
            policyRow("square.stack.3d.up", "Running programs", flags.runningPrograms, "The names of open apps, never their windows")
            Text(policySummary)
                .font(.caption)
                .foregroundStyle(.secondary)
                .fixedSize(horizontal: false, vertical: true)
        }
    }

    private var policySummary: String {
        var text = "Set by your workspace admin for each project. A snapshot is taken every \(snapshot.intervalMinutes) minutes."
        if let idle = snapshot.idleThresholdMinutes {
            text += " After \(idle) minutes without input you count as away, and that time is not tracked unless you keep it."
        } else if snapshot.currentProject != nil {
            text += " Idle detection is off on this project."
        }
        return text
    }

    private func policyRow(_ symbol: String, _ title: String, _ on: Bool, _ detail: String) -> some View {
        HStack(spacing: 10) {
            Image(systemName: symbol)
                .frame(width: 20)
                .foregroundStyle(on ? Color.primary : Color.secondary)
            VStack(alignment: .leading, spacing: 1) {
                Text(title).font(.callout.weight(.semibold)).foregroundStyle(on ? Color.primary : Color.secondary)
                Text(detail).font(.caption).foregroundStyle(.secondary)
            }
            Spacer(minLength: 6)
            Pill(on ? "On" : "Off", tone: on ? .success : .neutral)
        }
    }

    // MARK: This Mac

    private var deviceCard: some View {
        let device = model.device
        return Card("This Mac") {
            infoRow("Name", device.deviceName)
            infoRow("macOS", device.osVersion)
            infoRow("Dosi Tracker", device.agentVersion)
            infoRow("Time zone", TimeZone.autoupdatingCurrent.identifier)
            infoRow("Device ID", device.deviceId)
        }
    }

    private func infoRow(_ label: String, _ value: String) -> some View {
        HStack(alignment: .firstTextBaseline) {
            Text(label).font(.callout).foregroundStyle(.secondary).frame(width: 110, alignment: .leading)
            Text(value).font(.callout).textSelection(.enabled).lineLimit(2)
            Spacer(minLength: 0)
        }
    }

    // MARK: Account

    private var accountCard: some View {
        Card("Account") {
            HStack(spacing: 12) {
                Image(systemName: "person.crop.circle.fill").font(.system(size: 36)).foregroundStyle(Theme.brand)
                VStack(alignment: .leading, spacing: 2) {
                    Text(snapshot.displayName.isEmpty ? "Not signed in" : snapshot.displayName).font(.callout.weight(.semibold))
                    Text(snapshot.workspace.isEmpty ? "Host account" : "Workspace: \(snapshot.workspace)")
                        .font(.caption)
                        .foregroundStyle(.secondary)
                    Text("Server: \(snapshot.serverURL)").font(.caption).foregroundStyle(.tertiary).textSelection(.enabled)
                }
            }
            HStack(spacing: 10) {
                Button {
                    model.openDashboard()
                } label: {
                    Label("Web dashboard", systemImage: "safari").frame(maxWidth: .infinity)
                }
                Button(role: .destructive) {
                    confirmSignOut = true
                } label: {
                    Label("Sign out…", systemImage: "rectangle.portrait.and.arrow.right").frame(maxWidth: .infinity)
                }
                .disabled(!snapshot.status.isSignedIn && snapshot.status != .signInExpired)
            }
        }
    }

    // MARK: Help

    private var helpCard: some View {
        Card("Help & support", trailing: "Version \(model.device.agentVersion)") {
            HStack(spacing: 10) {
                Button {
                    model.openLogsFolder()
                } label: {
                    Label("Open logs", systemImage: "doc.text.magnifyingglass").frame(maxWidth: .infinity)
                }
                .help("Log files — attach them when reporting a problem.")
                Button {
                    model.openDataFolder()
                } label: {
                    Label("Open data folder", systemImage: "folder").frame(maxWidth: .infinity)
                }
                .help("Where Dosi Tracker keeps its offline queue.")
            }
        }
    }

    // MARK: Helpers

    /// A setting: title and description on the left, the control on the right.
    private func settingRow<Control: View>(_ title: String, _ description: String,
                                           @ViewBuilder control: () -> Control) -> some View {
        HStack(alignment: .center, spacing: 12) {
            VStack(alignment: .leading, spacing: 2) {
                Text(title).font(.callout.weight(.semibold))
                Text(description).font(.caption).foregroundStyle(.secondary).fixedSize(horizontal: false, vertical: true)
            }
            Spacer(minLength: 8)
            control()
        }
    }
}
