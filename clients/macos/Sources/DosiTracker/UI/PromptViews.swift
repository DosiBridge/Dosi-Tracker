import AppKit
import DosiTrackerCore
import SwiftUI

// The small windows that ask the user something (CONTRACT §2, §4): "You were
// away for 42 min — keep it as work or discard it?" after coming back from
// idle, "Choose the project you are working on" when the agent cannot pick
// one, and "What are you working on?" from the menu bar.

/// After an idle stretch. No answer means discard: nothing is recorded until
/// "Keep as work" is chosen.
struct IdlePromptView: View {
    @ObservedObject var model: AppModel
    let close: () -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: 14) {
            if let prompt = model.snapshot.awayPrompt {
                HStack(alignment: .top, spacing: 12) {
                    Image(systemName: "moon.zzz.fill").font(.system(size: 28)).foregroundStyle(Theme.warning)
                    VStack(alignment: .leading, spacing: 4) {
                        Text("You were away for \(Formatting.minutesLabel(prompt.minutes))")
                            .font(.title3.weight(.semibold))
                        Text("\(Formatting.span(prompt.from, prompt.to)), with no keyboard or mouse input.")
                            .foregroundStyle(.secondary)
                        Text(prompt.projectTitle.isEmpty
                             ? "Keep this time as work? It is recorded as idle time, with your current task note."
                             : "Keep this time as work on \(prompt.projectTitle)? It is recorded as idle time, with your current task note.")
                            .font(.callout)
                            .foregroundStyle(.secondary)
                            .fixedSize(horizontal: false, vertical: true)
                    }
                }
                HStack {
                    Spacer()
                    Button("Discard") {
                        model.resolveAway(keep: false)
                        close()
                    }
                    .keyboardShortcut(.cancelAction)
                    Button("Keep as work") {
                        model.resolveAway(keep: true)
                        close()
                    }
                    .keyboardShortcut(.defaultAction)
                }
            } else {
                Text("Nothing to decide right now.").foregroundStyle(.secondary)
                HStack {
                    Spacer()
                    Button("Close", action: close).keyboardShortcut(.defaultAction)
                }
            }
        }
        .padding(20)
        .frame(width: 420)
    }
}

/// When the agent cannot pick the project (CONTRACT §2: several, none
/// remembered). Tracking starts once one is chosen.
struct ChooseProjectView: View {
    @ObservedObject var model: AppModel
    let close: () -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: 14) {
            HStack(spacing: 10) {
                LogoMark(size: 32)
                VStack(alignment: .leading, spacing: 2) {
                    Text("Choose a project").font(.title3.weight(.semibold))
                    Text("Pick the project you are working on to start tracking.")
                        .font(.callout)
                        .foregroundStyle(.secondary)
                }
            }
            if model.snapshot.projects.isEmpty {
                Banner(.warning, model.snapshot.lastError ?? TrackerWorker.noProjectsMessage)
            } else {
                ScrollView {
                    VStack(spacing: 6) {
                        ForEach(model.snapshot.projects) { project in
                            Button {
                                model.selectProject(project.id)
                                close()
                            } label: {
                                HStack {
                                    Text(project.title).font(.callout.weight(.medium))
                                    Spacer()
                                    if project.id == model.snapshot.selectedProjectId {
                                        Image(systemName: "checkmark").foregroundStyle(Theme.brand)
                                    }
                                }
                                .padding(.horizontal, 12)
                                .padding(.vertical, 9)
                                .contentShape(Rectangle())
                                .background(RoundedRectangle(cornerRadius: 8, style: .continuous).fill(Theme.card))
                                .overlay(RoundedRectangle(cornerRadius: 8, style: .continuous).stroke(Theme.border, lineWidth: 1))
                            }
                            .buttonStyle(.plain)
                        }
                    }
                }
                .frame(maxHeight: 320)
            }
            Text("You can switch project at any time from the menu bar.")
                .font(.caption)
                .foregroundStyle(.secondary)
            HStack {
                Spacer()
                Button("Later", action: close).keyboardShortcut(.cancelAction)
            }
        }
        .padding(20)
        .frame(width: 400)
    }
}

/// "What are you working on?" as a native dialog (the menu bar's "Set
/// task…"). Returns the new note, or nil when cancelled.
@MainActor
enum TaskNotePrompt {
    static func ask(current: String) -> String? {
        let alert = NSAlert()
        alert.messageText = "What are you working on?"
        alert.informativeText = "Sent with each block as its description. Changing it starts a new block."
        let field = NSTextField(frame: NSRect(x: 0, y: 0, width: 300, height: 24))
        field.stringValue = current
        field.placeholderString = "e.g. Invoice export for ACME"
        alert.accessoryView = field
        alert.addButton(withTitle: "Save")
        alert.addButton(withTitle: "Cancel")
        alert.window.initialFirstResponder = field
        NSApp.activate()
        guard alert.runModal() == .alertFirstButtonReturn else { return nil }
        return String(field.stringValue.prefix(TaskNote.maxLength))
    }
}
