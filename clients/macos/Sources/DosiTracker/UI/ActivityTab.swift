import AppKit
import DosiTrackerCore
import SwiftUI

// The Activity tab (port of the Windows agent's `ui/activity.rs`): this week
// day by day (the server's totals per local day, today live), the apps used
// in the last hour, the latest screenshot exactly as uploaded, and the blocks
// recorded so far with where each one is — waiting, uploaded, or rejected and
// why.

struct ActivityTab: View {
    @ObservedObject var model: AppModel

    private static let weekdays = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"]

    private var snapshot: TrackerSnapshot { model.snapshot }

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            TimelineView(.periodic(from: .now, by: 30)) { context in
                VStack(alignment: .leading, spacing: 12) {
                    weekCard(now: context.date)
                    appsCard(now: context.date)
                }
            }
            screenshotCard
            recentCard
            queueCard
        }
    }

    // MARK: Week

    private func weekCard(now: Date) -> some View {
        let totals = model.liveTotals(now: now)
        let today = TrackedDays.weekdaySlot(of: now, timeZone: .autoupdatingCurrent)
        let hours: [Double] = (0..<7).map { day in
            if day < today {
                return day < snapshot.weekDaySeconds.count ? snapshot.weekDaySeconds[day] / 3600 : 0
            }
            return day == today ? totals.today / 3600 : 0
        }
        let bars = (0..<7).map { day in
            DayBar(id: day, label: ActivityTab.weekdays[day], hours: hours[day], today: day == today, future: day > today)
        }
        let goals = model.goals
        let worked = hours.prefix(today + 1).filter { $0 > 0 }
        let average = worked.isEmpty ? 0 : worked.reduce(0, +) / Double(worked.count)
        var best: Int?
        for day in 0...today where hours[day] > 0 && (best.map { hours[day] > hours[$0] } ?? true) {
            best = day
        }
        let bestText = best.map { "\(ActivityTab.weekdays[$0]) · \(Formatting.hoursLabel(hours[$0]))" } ?? "—"
        return Card("This week", trailing: "\(Formatting.secondsLabel(totals.week)) of \(Formatting.hoursLabel(goals.weeklySeconds / 3600))") {
            WeekChart(days: bars, goalHours: goals.dailyHours)
            HStack(spacing: 8) {
                Metric(label: "Days worked", value: "\(worked.count) of \(today + 1)")
                Metric(label: "Daily average", value: Formatting.hoursLabel(average))
                Metric(label: "Best day", value: bestText)
            }
        }
    }

    // MARK: Apps

    @ViewBuilder
    private func appsCard(now: Date) -> some View {
        Card("Top apps", trailing: "Last hour") {
            if !snapshot.capture.activeWindow {
                Text("App usage isn't recorded on this project.").font(.callout).foregroundStyle(.secondary)
            } else {
                let apps = model.topApps(now: now)
                if let longest = apps.first?.milliseconds, longest > 0 {
                    ForEach(apps, id: \.app) { total in
                        HStack(spacing: 10) {
                            AppBadge(name: total.app)
                            VStack(alignment: .leading, spacing: 4) {
                                HStack {
                                    Text(total.app).font(.callout.weight(.semibold)).lineLimit(1)
                                    Spacer()
                                    Text(total.milliseconds < 60_000 ? "<1m" : Formatting.minutesLabel(Int(total.milliseconds / 60_000)))
                                        .font(.caption)
                                        .foregroundStyle(.secondary)
                                }
                                ProgressBar(fraction: Double(total.milliseconds) / Double(longest), color: Theme.brand.opacity(0.35), height: 5)
                            }
                        }
                    }
                    Text("Foreground time on this Mac. Shared with your workspace only as part of tracked blocks.")
                        .font(.caption)
                        .foregroundStyle(.secondary)
                } else {
                    Text("Apps you use appear here as you work.").font(.callout).foregroundStyle(.secondary)
                }
            }
        }
    }

    // MARK: Screenshot

    @ViewBuilder
    private var screenshotCard: some View {
        if snapshot.capture.screenshot {
            Card("Latest screenshot", trailing: snapshot.lastScreenshot.map { "Taken at \(Formatting.time($0.takenAt))" }) {
                if let shot = snapshot.lastScreenshot, let image = NSImage(data: shot.jpeg) {
                    Image(nsImage: image)
                        .resizable()
                        .aspectRatio(contentMode: .fit)
                        .frame(maxWidth: .infinity, maxHeight: 300)
                        .clipShape(RoundedRectangle(cornerRadius: 8, style: .continuous))
                        .overlay(RoundedRectangle(cornerRadius: 8, style: .continuous).stroke(Theme.border, lineWidth: 1))
                    Text(shot.blurred
                         ? "This is what your workspace sees: blurred, as your project requires. One screenshot is taken per block."
                         : "This is what your workspace sees. One screenshot is taken per block.")
                        .font(.caption)
                        .foregroundStyle(.secondary)
                        .fixedSize(horizontal: false, vertical: true)
                } else {
                    // The moment of the next one is random and not announced.
                    Text("No screenshot yet this session. One is taken at a random moment in each block.")
                        .font(.callout)
                        .foregroundStyle(.secondary)
                        .fixedSize(horizontal: false, vertical: true)
                }
            }
        }
    }

    // MARK: Blocks

    private var recentCard: some View {
        let blocks = snapshot.recentBlocks
        return Card("Recorded this session", trailing: blocks.isEmpty ? nil : Formatting.count(blocks.count, "block")) {
            if blocks.isEmpty {
                Text("Each block is saved every \(snapshot.intervalMinutes) minutes, and when you pause or switch project.")
                    .font(.callout)
                    .foregroundStyle(.secondary)
                    .fixedSize(horizontal: false, vertical: true)
            } else {
                ForEach(blocks) { block in
                    RecentBlockRow(block: block)
                    if block.id != blocks.last?.id {
                        Divider()
                    }
                }
            }
        }
    }

    /// What is still on this Mac: waiting rows (and why the last try failed)
    /// and rows the server refused, with its reason — from earlier runs too.
    @ViewBuilder
    private var queueCard: some View {
        let rows = snapshot.queueRows.filter { row in
            row.status == .rejected || row.reason != nil
        }
        if snapshot.pendingUploads > 0 || snapshot.rejectedUploads > 0 {
            Card("On this Mac", trailing: "\(snapshot.pendingUploads) waiting · \(snapshot.rejectedUploads) rejected") {
                if rows.isEmpty {
                    Text("Blocks waiting to upload go up automatically; Sync now tries at once.")
                        .font(.callout)
                        .foregroundStyle(.secondary)
                } else {
                    ForEach(rows) { row in
                        QueueRowView(row: row, projectTitle: projectTitle(row.projectId))
                    }
                    if snapshot.rejectedUploads > 0 {
                        Text("Rejected blocks are kept here for 14 days for your records; they are never uploaded.")
                            .font(.caption)
                            .foregroundStyle(.secondary)
                            .fixedSize(horizontal: false, vertical: true)
                    }
                }
            }
        }
    }

    private func projectTitle(_ id: String?) -> String {
        guard let id = id else { return "" }
        return snapshot.projects.first { $0.id.caseInsensitiveCompare(id) == .orderedSame }?.title ?? ""
    }
}

/// One recorded block: its time, project, upload state and activity.
struct RecentBlockRow: View {
    let block: RecentBlock

    var body: some View {
        HStack(alignment: .top, spacing: 10) {
            UploadStateIcon(state: block.upload)
            VStack(alignment: .leading, spacing: 2) {
                let minutes = Int(block.endedAt.timeIntervalSince(block.startedAt) / 60)
                Text("\(Formatting.span(block.startedAt, block.endedAt))  ·  \(Formatting.minutesLabel(minutes))")
                    .font(.callout.weight(.semibold))
                Text(detail).font(.caption).foregroundStyle(.secondary).lineLimit(2)
                if block.upload == .rejected, let reason = block.message {
                    Text(reason).font(.caption).foregroundStyle(Theme.danger).fixedSize(horizontal: false, vertical: true)
                }
            }
            Spacer(minLength: 6)
            if let percent = block.productivity {
                Pill("\(percent)%", tone: percent >= 60 ? .success : (percent >= 30 ? .warning : .danger))
                    .help("Activity: the share of minutes with keyboard or mouse input")
            } else {
                Pill("n/a", tone: .neutral).help("Activity is not measured on this project")
            }
        }
    }

    private var detail: String {
        var parts: [String] = []
        if !block.projectTitle.isEmpty {
            parts.append(block.projectTitle)
        }
        if block.hasScreenshot {
            parts.append("screenshot")
        }
        if let reason = block.endReason, reason != .interval {
            parts.append("ended by \(reason.rawValue)")
        }
        switch block.upload {
        case .queued:
            parts.append("waiting to upload")
        case .uploaded:
            parts.append("uploaded")
        case .rejected:
            parts.append("rejected by the server")
        }
        return parts.joined(separator: " · ")
    }
}

/// A queued or parked row of the local queue.
struct QueueRowView: View {
    let row: QueuedRow
    let projectTitle: String

    var body: some View {
        HStack(alignment: .top, spacing: 10) {
            UploadStateIcon(state: row.status == .rejected ? .rejected : .queued)
            VStack(alignment: .leading, spacing: 2) {
                if let started = row.startedAt, let ended = row.endedAt {
                    Text(Formatting.span(started, ended) + (projectTitle.isEmpty ? "" : "  ·  \(projectTitle)"))
                        .font(.callout.weight(.semibold))
                }
                if let reason = row.reason {
                    Text(reason)
                        .font(.caption)
                        .foregroundStyle(row.status == .rejected ? Theme.danger : Color.secondary)
                        .fixedSize(horizontal: false, vertical: true)
                }
            }
            Spacer(minLength: 0)
        }
    }
}

/// Uploaded ✓, waiting ↻, rejected ✕.
struct UploadStateIcon: View {
    let state: UploadState

    var body: some View {
        Image(systemName: symbol)
            .font(.system(size: 18))
            .foregroundStyle(color)
            .help(hint)
    }

    private var symbol: String {
        switch state {
        case .uploaded:
            return "checkmark.circle.fill"
        case .queued:
            return "arrow.triangle.2.circlepath.circle.fill"
        case .rejected:
            return "xmark.circle.fill"
        }
    }

    private var color: Color {
        switch state {
        case .uploaded:
            return Theme.success
        case .queued:
            return Theme.warning
        case .rejected:
            return Theme.danger
        }
    }

    private var hint: String {
        switch state {
        case .uploaded:
            return "Uploaded"
        case .queued:
            return "Saved on this Mac, waiting to upload"
        case .rejected:
            return "Rejected by the server"
        }
    }
}
