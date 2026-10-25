import DosiTrackerCore
import SwiftUI

// The main window (port of the Windows agent's `ui/mod.rs` layout): a header
// with the account and the status pill, the Timer / Activity / Settings tabs,
// and a footer with the upload state and the version.

struct MainView: View {
    @ObservedObject var model: AppModel

    var body: some View {
        VStack(spacing: 0) {
            header
            Divider()
            Picker("", selection: $model.selectedTab) {
                ForEach(MainTab.allCases) { tab in
                    Label(tab.title, systemImage: tab.symbol).tag(tab)
                }
            }
            .pickerStyle(.segmented)
            .labelsHidden()
            .padding(.horizontal, 16)
            .padding(.vertical, 10)
            ScrollView {
                VStack(alignment: .leading, spacing: 12) {
                    if let message = model.message {
                        Banner(.warning, message)
                    }
                    switch model.selectedTab {
                    case .timer:
                        TimerTab(model: model)
                    case .activity:
                        ActivityTab(model: model)
                    case .settings:
                        SettingsTab(model: model)
                    }
                }
                .padding(.horizontal, 16)
                .padding(.bottom, 16)
            }
            Divider()
            footer
        }
        .frame(minWidth: 440, idealWidth: 500, minHeight: 560, idealHeight: 760)
    }

    private var header: some View {
        HStack(spacing: 10) {
            LogoMark(size: 30)
            VStack(alignment: .leading, spacing: 1) {
                Text("Dosi Tracker").font(.headline)
                Text(accountLine).font(.caption).foregroundStyle(.secondary).lineLimit(1)
            }
            Spacer()
            let visual = StatusVisual.of(model.snapshot.status, pending: model.pendingAction)
            Pill(visual.label, tone: visual.tone)
        }
        .padding(.horizontal, 16)
        .padding(.vertical, 10)
    }

    private var accountLine: String {
        let workspace = model.snapshot.workspace.trimmingCharacters(in: .whitespacesAndNewlines)
        let name = model.snapshot.displayName
        if workspace.isEmpty {
            return name
        }
        return name.isEmpty ? workspace : "\(workspace) · \(name)"
    }

    private var footer: some View {
        let state = SyncLine.of(model.snapshot)
        return HStack(spacing: 6) {
            Circle().fill(state.tone.color).frame(width: 7, height: 7)
            Text(state.text).font(.caption).foregroundStyle(.secondary).lineLimit(1)
            Spacer()
            Text("v\(model.device.agentVersion)").font(.caption).foregroundStyle(.tertiary)
        }
        .padding(.horizontal, 16)
        .padding(.vertical, 7)
    }
}

/// Tone and label of the status pill; a pause or resume in flight shows
/// first.
struct StatusVisual {
    let tone: Tone
    let label: String

    static func of(_ status: TrackerStatus, pending: PendingAction?) -> StatusVisual {
        switch pending {
        case .some(.pausing):
            return StatusVisual(tone: .warning, label: "Pausing…")
        case .some(.resuming):
            return StatusVisual(tone: .success, label: "Resuming…")
        case .none:
            break
        }
        switch status {
        case .tracking:
            return StatusVisual(tone: .success, label: status.label)
        case .paused, .onBreak, .away, .locked, .asleep:
            return StatusVisual(tone: .warning, label: status.label)
        case .offline:
            return StatusVisual(tone: .warning, label: "Offline")
        case .connecting, .choosingProject, .choosingWorkspace:
            return StatusVisual(tone: .info, label: status.label)
        case .signedOut:
            return StatusVisual(tone: .neutral, label: status.label)
        case .stopped, .signInExpired:
            return StatusVisual(tone: .danger, label: status.label)
        }
    }
}

/// The footer's one-line upload state: the most important first.
struct SyncLine {
    let tone: Tone
    let text: String

    static func of(_ snapshot: TrackerSnapshot) -> SyncLine {
        if snapshot.rejectedUploads > 0 {
            return SyncLine(tone: .danger,
                            text: "\(Formatting.count(snapshot.rejectedUploads, "upload")) rejected by the server")
        }
        if let issue = snapshot.uploadIssue {
            return SyncLine(tone: .warning, text: issue.message)
        }
        if snapshot.status == .offline || !snapshot.serverReachable {
            return SyncLine(tone: .warning, text: "Offline — uploads will retry")
        }
        if snapshot.pendingUploads > 0 {
            return SyncLine(tone: .warning, text: "\(Formatting.count(snapshot.pendingUploads, "upload")) waiting to sync")
        }
        if let synced = snapshot.lastSync {
            return SyncLine(tone: .success, text: "All synced · \(Formatting.time(synced))")
        }
        return SyncLine(tone: .neutral, text: "Nothing to sync yet")
    }
}
