import Foundation

// Formatting shared by the window and the menu bar (port of the Windows
// agent's `ui/format.rs` and its goal captions): pure functions of numbers and
// dates.

enum Formatting {
    /// "3h 42m", or "42m" under an hour.
    static func minutesLabel(_ minutes: Int) -> String {
        let total = max(minutes, 0)
        let hours = total / 60
        let rest = total % 60
        if hours > 0 {
            return "\(hours)h " + String(format: "%02ldm", rest)
        }
        return "\(rest)m"
    }

    /// ``minutesLabel(_:)`` for a number of seconds (whole minutes, rounded
    /// down).
    static func secondsLabel(_ seconds: Double) -> String {
        guard seconds.isFinite, seconds > 0 else { return minutesLabel(0) }
        return minutesLabel(Int(seconds / 60))
    }

    /// A stopwatch reading: `1:05:09`, or `05:09` under an hour; never
    /// negative.
    static func clock(_ elapsed: TimeInterval) -> String {
        let total = elapsed.isFinite ? Int(min(max(elapsed, 0), 1_000_000_000)) : 0
        let hours = total / 3600
        let minutes = total % 3600 / 60
        let seconds = total % 60
        if hours > 0 {
            return "\(hours):" + String(format: "%02ld:%02ld", minutes, seconds)
        }
        return String(format: "%02ld:%02ld", minutes, seconds)
    }

    /// The menu-bar timer: `1:05` (hours and minutes).
    static func shortClock(_ elapsed: TimeInterval) -> String {
        let total = elapsed.isFinite ? Int(min(max(elapsed, 0), 1_000_000_000)) : 0
        return "\(total / 3600):" + String(format: "%02ld", total % 3600 / 60)
    }

    /// Wall-clock time in the user's format ("10:05" or "10:05 AM").
    static func time(_ date: Date) -> String {
        date.formatted(date: .omitted, time: .shortened)
    }

    /// "8h", "7h 30m".
    static func hoursLabel(_ hours: Double) -> String {
        guard hours.isFinite, hours > 0 else { return "0h" }
        let minutes = Int((hours * 60).rounded())
        if minutes % 60 == 0 {
            return "\(minutes / 60)h"
        }
        return minutesLabel(minutes)
    }

    /// "46% of 8h goal", or "Goal reached" once met.
    static func goalCaption(seconds: Double, goal: Double) -> String {
        guard goal > 0 else { return "" }
        if seconds >= goal {
            return "Goal reached"
        }
        let percent = max(Int((seconds / goal * 100).rounded(.down)), 0)
        return "\(percent)% of \(hoursLabel(goal / 3600)) goal"
    }

    /// `n` with a singular or plural noun: "1 upload", "3 uploads".
    static func count(_ n: Int, _ noun: String) -> String {
        n == 1 ? "1 \(noun)" : "\(n) \(noun)s"
    }

    /// "10:05 – 10:47".
    static func span(_ from: Date, _ to: Date) -> String {
        "\(time(from)) – \(time(to))"
    }

    /// The host of a server address, without the scheme ("tracker.acme.com").
    static func host(_ address: String) -> String {
        var text = address
        for prefix in ["https://", "http://"] where text.lowercased().hasPrefix(prefix) {
            text = String(text.dropFirst(prefix.count))
        }
        return text
    }
}
