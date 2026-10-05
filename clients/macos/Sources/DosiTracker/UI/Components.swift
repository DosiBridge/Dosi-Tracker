import AppKit
import DosiTrackerCore
import SwiftUI

// Shared SwiftUI pieces of the windows (the macOS counterpart of the Windows
// agent's `ui/widgets.rs` and `theme.rs`): the brand colours, tones, cards,
// banners, pills, progress and minute bars, and the week chart. System
// colours carry light and dark mode.

enum Theme {
    /// The brand blue (#006BFF).
    static let brand = Color(red: 0.0, green: 0.42, blue: 1.0)
    static let success = Color(red: 0.0, green: 0.64, blue: 0.54)
    static let warning = Color(red: 0.86, green: 0.55, blue: 0.0)
    static let danger = Color(red: 0.85, green: 0.21, blue: 0.24)
    static let card = Color(nsColor: .controlBackgroundColor)
    static let border = Color(nsColor: .separatorColor)
    static let track = Color(nsColor: .quaternaryLabelColor)

    /// Green when mostly active, amber when mixed, red when mostly idle.
    static func activity(_ percent: Int) -> Color {
        if percent >= 60 {
            return success
        }
        return percent >= 30 ? warning : danger
    }
}

/// The feel of a message or a status.
enum Tone {
    case success
    case warning
    case danger
    case info
    case neutral

    var color: Color {
        switch self {
        case .success:
            return Theme.success
        case .warning:
            return Theme.warning
        case .danger:
            return Theme.danger
        case .info:
            return Theme.brand
        case .neutral:
            return Color.secondary
        }
    }

    var symbol: String {
        switch self {
        case .success:
            return "checkmark.circle.fill"
        case .warning:
            return "exclamationmark.triangle.fill"
        case .danger:
            return "xmark.octagon.fill"
        case .info, .neutral:
            return "info.circle.fill"
        }
    }
}

/// A rounded card with an optional title and trailing caption.
struct Card<Content: View>: View {
    let title: String?
    let trailing: String?
    let content: Content

    init(_ title: String? = nil, trailing: String? = nil, @ViewBuilder content: () -> Content) {
        self.title = title
        self.trailing = trailing
        self.content = content()
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            if title != nil || trailing != nil {
                HStack(alignment: .firstTextBaseline) {
                    if let title = title {
                        Text(title).font(.headline)
                    }
                    Spacer(minLength: 8)
                    if let trailing = trailing {
                        Text(trailing).font(.caption).foregroundStyle(.secondary).lineLimit(1)
                    }
                }
            }
            content
        }
        .padding(14)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(RoundedRectangle(cornerRadius: 10, style: .continuous).fill(Theme.card))
        .overlay(RoundedRectangle(cornerRadius: 10, style: .continuous).stroke(Theme.border, lineWidth: 1))
    }
}

/// A one-line (wrapping) message with a tone.
struct Banner: View {
    let tone: Tone
    let text: String

    init(_ tone: Tone, _ text: String) {
        self.tone = tone
        self.text = text
    }

    var body: some View {
        HStack(alignment: .top, spacing: 8) {
            Image(systemName: tone.symbol).foregroundStyle(tone.color)
            Text(text).font(.callout).fixedSize(horizontal: false, vertical: true)
            Spacer(minLength: 0)
        }
        .padding(10)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(RoundedRectangle(cornerRadius: 8, style: .continuous).fill(tone.color.opacity(0.12)))
    }
}

/// A small coloured pill ("Tracking", "On", "72%").
struct Pill: View {
    let text: String
    let tone: Tone

    init(_ text: String, tone: Tone) {
        self.text = text
        self.tone = tone
    }

    var body: some View {
        Text(text)
            .font(.caption.weight(.semibold))
            .padding(.horizontal, 8)
            .padding(.vertical, 3)
            .foregroundStyle(tone.color)
            .background(Capsule().fill(tone.color.opacity(0.14)))
    }
}

/// A thin horizontal progress bar (fraction clamped to 0...1).
struct ProgressBar: View {
    let fraction: Double
    var color: Color = Theme.brand
    var height: CGFloat = 6

    var body: some View {
        GeometryReader { proxy in
            ZStack(alignment: .leading) {
                Capsule().fill(Theme.track)
                Capsule()
                    .fill(color)
                    .frame(width: proxy.size.width * CGFloat(fraction.isFinite ? min(max(fraction, 0), 1) : 0))
            }
        }
        .frame(height: height)
    }
}

/// A small caps label above a strong value.
struct Metric: View {
    let label: String
    let value: String
    var color: Color = .primary

    var body: some View {
        VStack(alignment: .leading, spacing: 2) {
            Text(label.uppercased()).font(.caption2.weight(.semibold)).foregroundStyle(.secondary)
            Text(value).font(.system(.body, design: .rounded).weight(.semibold).monospacedDigit()).foregroundStyle(color)
        }
        .frame(maxWidth: .infinity, alignment: .leading)
    }
}

/// One bar per minute of the block in progress: its height is the input of
/// that minute, its colour whether the minute counts as active. Empty slots
/// fill the rest of the interval.
struct MinuteBars: View {
    let minutes: [MinuteBar]
    /// Slots shown: the interval in minutes.
    let capacity: Int
    var height: CGFloat = 38

    var body: some View {
        let slots = max(capacity, minutes.count, 1)
        let peak = max(minutes.map { $0.inputs }.max() ?? 0, 1)
        HStack(alignment: .bottom, spacing: 2) {
            ForEach(0..<slots, id: \.self) { index in
                if index < minutes.count {
                    let minute = minutes[index]
                    let share = minute.active ? max(CGFloat(minute.inputs) / CGFloat(peak), 0.18) : 0.08
                    RoundedRectangle(cornerRadius: 2, style: .continuous)
                        .fill(minute.active ? Theme.brand : Theme.track)
                        .frame(maxWidth: .infinity)
                        .frame(height: max(height * share, 2))
                        .help(minute.active ? "Minute \(index + 1): active, \(minute.inputs) inputs" : "Minute \(index + 1): no input")
                } else {
                    RoundedRectangle(cornerRadius: 2, style: .continuous)
                        .fill(Theme.track.opacity(0.5))
                        .frame(maxWidth: .infinity)
                        .frame(height: 2)
                }
            }
        }
        .frame(height: height, alignment: .bottom)
    }
}

/// One day of the week chart.
struct DayBar: Identifiable {
    let id: Int
    let label: String
    let hours: Double
    let today: Bool
    let future: Bool
}

/// This week day by day, with the daily goal as a line.
struct WeekChart: View {
    let days: [DayBar]
    let goalHours: Double
    var height: CGFloat = 140

    var body: some View {
        let top = max(days.map { $0.hours }.max() ?? 0, goalHours, 1)
        VStack(spacing: 4) {
            ZStack(alignment: .bottom) {
                GeometryReader { proxy in
                    Path { path in
                        let y = proxy.size.height * CGFloat(1 - goalHours / top)
                        path.move(to: CGPoint(x: 0, y: y))
                        path.addLine(to: CGPoint(x: proxy.size.width, y: y))
                    }
                    .stroke(Theme.success.opacity(0.7), style: StrokeStyle(lineWidth: 1, dash: [4, 3]))
                }
                HStack(alignment: .bottom, spacing: 8) {
                    ForEach(days) { day in
                        RoundedRectangle(cornerRadius: 4, style: .continuous)
                            .fill(day.today ? Theme.brand : Theme.brand.opacity(day.future ? 0.15 : 0.45))
                            .frame(maxWidth: .infinity)
                            .frame(height: max(height * CGFloat(day.hours / top), day.hours > 0 ? 3 : 1))
                            .help("\(day.label): \(Formatting.hoursLabel(day.hours))")
                    }
                }
            }
            .frame(height: height)
            HStack(spacing: 8) {
                ForEach(days) { day in
                    Text(day.label)
                        .font(.caption2.weight(day.today ? .bold : .regular))
                        .foregroundStyle(day.today ? Color.primary : Color.secondary)
                        .frame(maxWidth: .infinity)
                }
            }
        }
    }
}

/// A rounded square with the app's initial, coloured from its name.
struct AppBadge: View {
    let name: String

    private static let hues: [Color] = [
        Color(red: 0.0, green: 0.42, blue: 1.0),
        Color(red: 0.0, green: 0.64, blue: 0.54),
        Color(red: 0.49, green: 0.23, blue: 0.93),
        Color(red: 0.86, green: 0.15, blue: 0.47),
        Color(red: 0.92, green: 0.35, blue: 0.05),
        Color(red: 0.03, green: 0.57, blue: 0.70),
    ]

    var body: some View {
        // A stable hash of the name (never the rank), as on the web.
        let hash = name.unicodeScalars.reduce(UInt32(0)) { ($0 &* 31) &+ $1.value }
        let color = AppBadge.hues[Int(hash % UInt32(AppBadge.hues.count))]
        RoundedRectangle(cornerRadius: 7, style: .continuous)
            .fill(color.opacity(0.15))
            .frame(width: 28, height: 28)
            .overlay(Text(String(name.prefix(1)).uppercased()).font(.system(size: 13, weight: .semibold)).foregroundStyle(color))
    }
}

/// The brand mark: a blue disc with a white ring and a centre dot.
struct LogoMark: View {
    var size: CGFloat = 48

    var body: some View {
        ZStack {
            Circle().fill(Theme.brand)
            Circle().stroke(Color.white, lineWidth: size * 0.09).frame(width: size * 0.54, height: size * 0.54)
            Circle().fill(Color.white).frame(width: size * 0.22, height: size * 0.22)
        }
        .frame(width: size, height: size)
    }
}
