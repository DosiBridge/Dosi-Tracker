import Foundation

/// Local midnight today and on this week's Monday, as instants for the API.
public struct DayAndWeekStart: Equatable, Sendable {
    public var today: Date
    public var week: Date

    public init(today: Date, week: Date) {
        self.today = today
        self.week = week
    }
}

/// One local day of this week before today, `[start, end)`, at `slot` days
/// from Monday (for the week chart).
public struct PastDay: Equatable, Sendable {
    public var slot: Int
    public var start: Date
    public var end: Date

    public init(slot: Int, start: Date, end: Date) {
        self.slot = slot
        self.start = start
        self.end = end
    }
}

/// "Today" and "this week" for the status cards (port of the Windows worker's
/// `local_day_and_week_start`, `past_days_of_week` and `tracked_secs_since`).
///
/// They follow the user's wall clock, not UTC; otherwise the cards would reset
/// mid-day (at 06:00 in UTC+6, at 19:00 the day before in UTC-5). Weeks start
/// on Monday. Where a DST change skips midnight, the day starts at its first
/// existing instant. The summary requests pass the same zone as `TimeZone`
/// (``ApiClient/summary(from:to:timeZone:)``), so the server's days match.
public enum TrackedDays {
    public static func dayAndWeekStart(now: Date, timeZone: TimeZone) -> DayAndWeekStart {
        let calendar = gregorian(timeZone)
        let today = calendar.startOfDay(for: now)
        let monday = startOfDay(calendar, offsetBy: -daysSinceMonday(calendar, now), from: today) ?? today
        return DayAndWeekStart(today: today, week: monday)
    }

    /// Each local day of this week before today; empty on a Monday.
    public static func pastDaysOfWeek(now: Date, timeZone: TimeZone) -> [PastDay] {
        let calendar = gregorian(timeZone)
        let today = calendar.startOfDay(for: now)
        let todaySlot = daysSinceMonday(calendar, now)
        var days: [PastDay] = []
        for slot in 0..<todaySlot {
            guard let start = startOfDay(calendar, offsetBy: slot - todaySlot, from: today),
                  let end = startOfDay(calendar, offsetBy: 1, from: start)
            else {
                continue
            }
            days.append(PastDay(slot: slot, start: start, end: end))
        }
        return days
    }

    /// Seconds of `start ..< end` on or after `since` (e.g. today's local
    /// midnight), so a block spanning midnight only adds today's part.
    public static func trackedSeconds(from start: Date, to end: Date, since: Date) -> Double {
        let from = start > since ? start : since
        guard end > from else { return 0 }
        return end.timeIntervalSince(from)
    }

    /// 0 on Monday ... 6 on Sunday, in `timeZone`.
    public static func weekdaySlot(of date: Date, timeZone: TimeZone) -> Int {
        daysSinceMonday(gregorian(timeZone), date)
    }

    private static func gregorian(_ timeZone: TimeZone) -> Calendar {
        var calendar = Calendar(identifier: .gregorian)
        calendar.timeZone = timeZone
        return calendar
    }

    private static func daysSinceMonday(_ calendar: Calendar, _ date: Date) -> Int {
        // .weekday: 1 = Sunday ... 7 = Saturday.
        (calendar.component(.weekday, from: date) + 5) % 7
    }

    private static func startOfDay(_ calendar: Calendar, offsetBy days: Int, from dayStart: Date) -> Date? {
        guard let shifted = calendar.date(byAdding: .day, value: days, to: dayStart) else { return nil }
        return calendar.startOfDay(for: shifted)
    }
}
