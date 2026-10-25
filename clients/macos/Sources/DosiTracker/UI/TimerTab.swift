import DosiTrackerCore
import SwiftUI

// The Timer tab (port of the Windows agent's `ui/timer.rs`): the project and
// the task note, the session clock with the pause / resume control and timed
// breaks, progress against the daily and weekly goals, what the block in
// progress holds so far (per-minute bars, counts, the app in front), the
// problems that need attention, and Sync now / the web dashboard.

struct TimerTab: View {
    @ObservedObject var model: AppModel
    @State private var note = ""
    @FocusState private var noteFocused: Bool

    private var snapshot: TrackerSnapshot { model.snapshot }

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            sessionCard
            TimelineView(.periodic(from: .now, by: 1)) { context in
                VStack(alignment: .leading, spacing: 12) {
                    goalTiles(now: context.date)
                    blockCard(now: context.date)
                }
            }
            alerts
            quickActions
        }
        .onAppear {
            note = snapshot.taskNote
        }
        .onChange(of: snapshot.taskNote) { _, saved in
            if !noteFocused {
                note = saved
            }
        }
    }

    // MARK: Session

    private var sessionCard: some View {
        Card("Project", trailing: "Snapshot every \(snapshot.intervalMinutes) min") {
            VStack(alignment: .leading, spacing: 12) {
                projectPicker
                TextField("", text: $note, prompt: Text("What are you working on?"))
                    .textFieldStyle(.roundedBorder)
                    .focused($noteFocused)
                    .onSubmit {
                        model.setTaskNote(note)
                    }
                    .onChange(of: note) { _, typed in
                        if typed.count > TaskNote.maxLength {
                            note = String(typed.prefix(TaskNote.maxLength))
                        }
                    }
                    .onChange(of: noteFocused) { _, focused in
                        if !focused {
                            model.setTaskNote(note)
                        }
                    }
                    .disabled(!snapshot.status.isSignedIn)
                    .help("Sent with each block. Changing it starts a new block.")

                TimelineView(.periodic(from: .now, by: 1)) { context in
                    clock(now: context.date)
                }
                pauseControls
            }
        }
    }

    private var projectPicker: some View {
        Picker("Project", selection: Binding(
            get: { snapshot.selectedProjectId ?? "" },
            set: { id in
                if !id.isEmpty {
                    model.selectProject(id)
                }
            }
        )) {
            if snapshot.selectedProjectId == nil {
                Text(snapshot.projects.isEmpty ? "No project available" : "Choose a project…").tag("")
            }
            ForEach(snapshot.projects) { project in
                Text(project.title).tag(project.id)
            }
        }
        .labelsHidden()
        .disabled(snapshot.projects.isEmpty || model.pendingAction != nil || !snapshot.status.isSignedIn)
    }

    private func clock(now: Date) -> some View {
        let reading = ClockReadout.of(snapshot, now: now)
        return VStack(spacing: 4) {
            Text(reading.caption.uppercased())
                .font(.caption.weight(.semibold))
                .foregroundStyle(reading.tone.color)
            Text(reading.digits)
                .font(.system(size: 44, weight: .semibold, design: .rounded).monospacedDigit())
                .foregroundStyle(reading.digitsDimmed ? Color.secondary : Color.primary)
            if !reading.detail.isEmpty {
                Text(reading.detail)
                    .font(.callout)
                    .foregroundStyle(.secondary)
                    .multilineTextAlignment(.center)
                    .fixedSize(horizontal: false, vertical: true)
            }
        }
        .frame(maxWidth: .infinity)
    }

    private var pauseControls: some View {
        let status = snapshot.status
        let pending = model.pendingAction
        return VStack(spacing: 10) {
            if status.isPaused {
                Button {
                    model.resume()
                } label: {
                    Label(pending == .resuming ? "Resuming…" : "Resume tracking", systemImage: "play.fill")
                        .frame(maxWidth: .infinity)
                }
                .buttonStyle(.borderedProminent)
                .tint(Theme.success)
                .controlSize(.large)
                .disabled(pending != nil)
                .help("Start a new block now. The break is not counted.")
            } else {
                Button {
                    model.pause(for: nil)
                } label: {
                    Label(pauseLabel(status: status, pending: pending), systemImage: "pause.fill")
                        .frame(maxWidth: .infinity)
                }
                .buttonStyle(.borderedProminent)
                .controlSize(.large)
                .disabled(pending != nil || !status.canPause)
                .help("Pause tracking. The work so far is saved and uploaded.")
            }
            if status.isPaused || status.canPause {
                HStack(spacing: 6) {
                    Text(status.isPaused ? "Resume after" : "Take a break")
                        .font(.caption)
                        .foregroundStyle(.secondary)
                    ForEach(AppModel.breakChoices, id: \.label) { choice in
                        Button(choice.label) {
                            model.pause(for: choice.seconds)
                        }
                        .controlSize(.small)
                        .disabled(pending != nil)
                        .help(status.isPaused
                              ? "Stay on a break, then resume automatically in \(choice.label)"
                              : "Pause now and resume automatically in \(choice.label)")
                    }
                    if status == .onBreak {
                        Button("Never") {
                            model.pause(for: nil)
                        }
                        .controlSize(.small)
                        .disabled(pending != nil)
                        .help("Stay paused until you resume")
                    }
                }
                .frame(maxWidth: .infinity)
            }
        }
    }

    private func pauseLabel(status: TrackerStatus, pending: PendingAction?) -> String {
        if pending == .pausing {
            return "Pausing…"
        }
        return status.canPause ? "Pause tracking" : "Tracking is not running"
    }

    // MARK: Goals

    private func goalTiles(now: Date) -> some View {
        let totals = model.liveTotals(now: now)
        let goals = model.goals
        let activity = snapshot.todayProductivity
        return HStack(spacing: 10) {
            goalTile("Today", value: Formatting.secondsLabel(totals.today), fraction: totals.today / goals.dailySeconds,
                     color: Theme.brand, caption: Formatting.goalCaption(seconds: totals.today, goal: goals.dailySeconds))
            goalTile("This week", value: Formatting.secondsLabel(totals.week),
                     fraction: totals.week / goals.weeklySeconds, color: Theme.brand,
                     caption: Formatting.goalCaption(seconds: totals.week, goal: goals.weeklySeconds))
            goalTile("Activity", value: activity.map { "\($0)%" } ?? "—", fraction: Double(activity ?? 0) / 100,
                     color: Theme.activity(activity ?? 0), caption: "Today's average")
        }
    }

    private func goalTile(_ label: String, value: String, fraction: Double, color: Color, caption: String) -> some View {
        VStack(alignment: .leading, spacing: 5) {
            Text(label.uppercased()).font(.caption2.weight(.semibold)).foregroundStyle(.secondary)
            Text(value).font(.system(.title3, design: .rounded).weight(.semibold).monospacedDigit())
            ProgressBar(fraction: fraction, color: color)
            Text(caption).font(.caption2).foregroundStyle(.secondary).lineLimit(1)
        }
        .padding(12)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(RoundedRectangle(cornerRadius: 10, style: .continuous).fill(Theme.card))
        .overlay(RoundedRectangle(cornerRadius: 10, style: .continuous).stroke(Theme.border, lineWidth: 1))
    }

    // MARK: Block in progress

    @ViewBuilder
    private func blockCard(now: Date) -> some View {
        switch snapshot.status {
        case .tracking:
            if let started = snapshot.blockStartedAt {
                let next = snapshot.nextCutAt ?? now
                Card("Current block", trailing: "Saved in \(Formatting.clock(next.timeIntervalSince(now)))") {
                    VStack(alignment: .leading, spacing: 10) {
                        let interval = Double(max(snapshot.intervalMinutes, 1) * 60)
                        ProgressBar(fraction: now.timeIntervalSince(started) / interval)
                        if let live = model.liveBlock(now: now) {
                            if snapshot.capture.measuresInput {
                                MinuteBars(minutes: live.minutes, capacity: snapshot.intervalMinutes)
                            }
                            HStack(spacing: 8) {
                                if let percent = live.activityPercent {
                                    Metric(label: "Active", value: "\(percent)%", color: Theme.activity(percent))
                                }
                                if snapshot.capture.keyboard {
                                    Metric(label: "Keys", value: "\(live.keyPresses)")
                                }
                                if snapshot.capture.mouse {
                                    Metric(label: "Clicks", value: "\(live.clicks)")
                                }
                                Metric(label: "Since", value: Formatting.time(started))
                            }
                        }
                        if snapshot.capture.activeWindow, let app = model.currentAppName {
                            Divider()
                            HStack(spacing: 6) {
                                Image(systemName: "macwindow").foregroundStyle(.secondary)
                                Text("Now").font(.caption.weight(.semibold)).foregroundStyle(.secondary)
                                Text(app).font(.callout).lineLimit(1)
                            }
                        }
                    }
                }
            }
        case .paused, .onBreak:
            Card("On a break") {
                Text("The block you were working on was saved when you paused. Nothing — no screenshots, input or apps — is recorded until you resume.")
                    .font(.callout)
                    .foregroundStyle(.secondary)
                    .fixedSize(horizontal: false, vertical: true)
            }
        default:
            EmptyView()
        }
    }

    // MARK: Problems

    @ViewBuilder
    private var alerts: some View {
        if case .stopped(let reason) = snapshot.status {
            Banner(.danger, reason)
        } else if let error = snapshot.lastError {
            Banner(.warning, error)
        }
        if snapshot.status == .offline {
            HStack {
                Banner(.warning, "Cannot reach the server. Tracking starts as soon as it answers; your work is kept on this Mac.")
                Button("Try again") {
                    model.reconnect()
                }
            }
        }
        if let issue = snapshot.uploadIssue {
            Banner(.warning, "\(issue.message) Next try at \(Formatting.time(issue.nextAttemptAt)).")
        }
        if snapshot.rejectedUploads > 0 {
            Banner(.warning, "\(Formatting.count(snapshot.rejectedUploads, "block")) rejected by the server and will not be uploaded. See Activity for the reasons.")
        }
        if let clock = snapshot.clockWarning {
            Banner(.warning, "\(clock). Set the date and time automatically in System Settings → General → Date & Time.")
        }
        if snapshot.status.isSignedIn && snapshot.capture.measuresInput && model.inputCapture != .running {
            Banner(.warning, "Keyboard and mouse activity is not being measured: allow Dosi Tracker in System Settings → Privacy & Security → Input Monitoring.")
        }
        ForEach(model.missingPermissions.filter { $0 != .inputMonitoring }, id: \.self) { permission in
            HStack {
                Banner(.warning, "Allow \(permission.title) for Dosi Tracker. Needed for: \(permission.purpose.lowercased()).")
                Button("Fix…") {
                    model.fix(permission)
                }
            }
        }
    }

    // MARK: Actions

    private var quickActions: some View {
        let paused = snapshot.status.isPaused
        return HStack(spacing: 10) {
            Button {
                model.syncNow()
            } label: {
                Label(paused ? "Upload queued" : "Sync now", systemImage: "arrow.triangle.2.circlepath")
                    .frame(maxWidth: .infinity)
            }
            .disabled(!snapshot.status.isSignedIn)
            .help(paused
                  ? "Paused: uploads activity already recorded. Nothing new is captured."
                  : "Record the current block now and upload it.")
            Button {
                model.openDashboard()
            } label: {
                Label("Web dashboard", systemImage: "safari").frame(maxWidth: .infinity)
            }
            .help("Open your timesheets and reports in the browser.")
        }
        .controlSize(.large)
    }
}

/// What the big clock shows for a status.
struct ClockReadout {
    let caption: String
    let tone: Tone
    let digits: String
    let digitsDimmed: Bool
    let detail: String

    static func of(_ snapshot: TrackerSnapshot, now: Date) -> ClockReadout {
        switch snapshot.status {
        case .tracking:
            let since = snapshot.trackingSince ?? now
            return ClockReadout(caption: "Tracking time", tone: .success,
                                digits: Formatting.clock(now.timeIntervalSince(since)), digitsDimmed: false,
                                detail: "Session started at \(Formatting.time(since))")
        case .paused, .onBreak:
            let since = snapshot.pausedSince ?? now
            let detail: String
            if let resumeAt = snapshot.resumeAt {
                detail = "Resumes at \(Formatting.time(resumeAt)) · in \(Formatting.clock(resumeAt.timeIntervalSince(now)))"
            } else {
                detail = "Nothing is recorded until you resume."
            }
            return ClockReadout(caption: "On a break", tone: .warning,
                                digits: Formatting.clock(now.timeIntervalSince(since)), digitsDimmed: false, detail: detail)
        case .away:
            let since = snapshot.awaySince.map { Formatting.time($0) } ?? ""
            return idle("Away", .warning, "No input since \(since). Tracking resumes when you are back.")
        case .locked:
            return idle("Locked", .warning, "Nothing is recorded while the screen is locked.")
        case .asleep:
            return idle("Asleep", .warning, "Nothing is recorded while the Mac sleeps.")
        case .connecting:
            return idle("Connecting", .info, "Connecting to your workspace…")
        case .offline:
            return idle("Offline", .warning, "Waiting for the server or a project. Work is kept on this Mac.")
        case .choosingProject:
            return idle("Choose a project", .info, "Pick the project you are working on to start tracking.")
        case .choosingWorkspace:
            return idle("Choose a workspace", .info, "Pick the workspace you track in to start tracking.")
        case .signedOut:
            return idle("Signed out", .neutral, "")
        case .signInExpired:
            return idle("Sign-in expired", .danger, "Enter your password to continue.")
        case .stopped:
            return idle("Stopped", .danger, "Nothing is recorded right now.")
        }
    }

    private static func idle(_ caption: String, _ tone: Tone, _ detail: String) -> ClockReadout {
        ClockReadout(caption: caption, tone: tone, digits: "--:--", digitsDimmed: true, detail: detail)
    }
}
