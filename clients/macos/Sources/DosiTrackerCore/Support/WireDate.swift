import Foundation

/// Date text as the backend reads and writes it.
///
/// Uploads use ISO-8601 in UTC with milliseconds and a `Z`
/// (`2026-07-14T10:05:00.000Z`), which ASP.NET binds as a UTC `DateTime`.
/// Parsing accepts everything ABP emits: 0-7 fractional digits, `Z`, a numeric
/// offset, or no zone at all (Unspecified-kind dates such as daily report
/// rows, which are read as UTC wall time).
///
/// Implemented with integer calendar arithmetic rather than date formatters,
/// so it is locale-proof, thread-safe and exact to the millisecond.
public enum WireDate {
    /// `2026-07-14T10:05:00.000Z`: UTC, always three fractional digits.
    public static func string(from date: Date) -> String {
        let ms = EpochTime.milliseconds(date)
        let days = EpochTime.floorDiv(ms, 86_400_000)
        let msOfDay = ms - days * 86_400_000
        let civil = civilFromDays(days)
        let hour = msOfDay / 3_600_000
        let minute = (msOfDay / 60_000) % 60
        let second = (msOfDay / 1000) % 60
        let millisecond = msOfDay % 1000
        return "\(pad(civil.year, 4))-\(pad(civil.month, 2))-\(pad(civil.day, 2))"
            + "T\(pad(hour, 2)):\(pad(minute, 2)):\(pad(second, 2)).\(pad(millisecond, 3))Z"
    }

    /// `20261005T100000Z`: UTC to the second, safe in a file name (the suffix
    /// of a queue file moved aside as damaged).
    public static func compactStamp(from date: Date) -> String {
        let seconds = EpochTime.seconds(date)
        let days = EpochTime.floorDiv(seconds, 86_400)
        let secondOfDay = seconds - days * 86_400
        let civil = civilFromDays(days)
        return "\(pad(civil.year, 4))\(pad(civil.month, 2))\(pad(civil.day, 2))"
            + "T\(pad(secondOfDay / 3600, 2))\(pad((secondOfDay / 60) % 60, 2))\(pad(secondOfDay % 60, 2))Z"
    }

    /// Parses an ISO-8601 date-time: `2026-07-14T10:05:00Z`,
    /// `2026-07-14T10:05:00.1234567Z`, `2026-07-14T16:05:00+06:00`, or
    /// `2026-07-14T00:00:00` (no zone: UTC). Returns nil for anything else.
    public static func date(from text: String) -> Date? {
        var scanner = ByteScanner(text.trimmingCharacters(in: .whitespacesAndNewlines))
        guard let year = scanner.digits(4), scanner.skip(UInt8(ascii: "-")),
              let month = scanner.digits(2), scanner.skip(UInt8(ascii: "-")),
              let day = scanner.digits(2),
              scanner.skipAny([UInt8(ascii: "T"), UInt8(ascii: "t"), UInt8(ascii: " ")]),
              let hour = scanner.digits(2), scanner.skip(UInt8(ascii: ":")),
              let minute = scanner.digits(2)
        else {
            return nil
        }

        var second: Int64 = 0
        if scanner.skip(UInt8(ascii: ":")) {
            guard let value = scanner.digits(2) else { return nil }
            second = value
        }

        var fraction: Double = 0
        if scanner.skipAny([UInt8(ascii: "."), UInt8(ascii: ",")]) {
            guard let value = scanner.fraction() else { return nil }
            fraction = value
        }

        var offset: Int64 = 0
        if !scanner.atEnd {
            if scanner.skipAny([UInt8(ascii: "Z"), UInt8(ascii: "z")]) {
                offset = 0
            } else if let sign = scanner.sign() {
                guard let hours = scanner.digits(2) else { return nil }
                var minutes: Int64 = 0
                if !scanner.atEnd {
                    _ = scanner.skip(UInt8(ascii: ":"))
                    guard let value = scanner.digits(2) else { return nil }
                    minutes = value
                }
                guard hours <= 18, minutes <= 59 else { return nil }
                offset = sign * (hours * 3600 + minutes * 60)
            } else {
                return nil
            }
        }
        guard scanner.atEnd else { return nil }

        guard (1...12).contains(month), day >= 1, day <= daysInMonth(year: year, month: month),
              hour <= 23, minute <= 59, second <= 60
        else {
            return nil
        }
        let days = daysFromCivil(year: year, month: month, day: day)
        // A leap second (:60) is read as :59.
        let seconds = days * 86_400 + hour * 3600 + minute * 60 + min(second, 59) - offset
        return Date(timeIntervalSince1970: TimeInterval(seconds) + fraction)
    }

    /// Parses an RFC 7231 IMF-fixdate (`Mon, 05 Oct 2026 08:49:37 GMT`), the
    /// format of the `Date` and `Retry-After` response headers.
    public static func httpDate(from text: String) -> Date? {
        var rest = Substring(text.trimmingCharacters(in: .whitespacesAndNewlines))
        if let comma = rest.firstIndex(of: ",") {
            rest = rest[rest.index(after: comma)...]
        }
        let parts = rest.split(separator: " ")
        guard parts.count == 4 || parts.count == 5,
              let day = Int64(parts[0]),
              let month = monthNumber(parts[1]),
              let year = Int64(parts[2])
        else {
            return nil
        }
        let clock = parts[3].split(separator: ":")
        guard clock.count == 3,
              let hour = Int64(clock[0]), let minute = Int64(clock[1]), let second = Int64(clock[2])
        else {
            return nil
        }
        if parts.count == 5 {
            let zone = parts[4].uppercased()
            guard zone == "GMT" || zone == "UTC" || zone == "Z" || zone == "+0000" else { return nil }
        }
        guard (1...12).contains(month), day >= 1, day <= daysInMonth(year: year, month: month),
              (0...23).contains(hour), (0...59).contains(minute), (0...60).contains(second)
        else {
            return nil
        }
        let days = daysFromCivil(year: year, month: month, day: day)
        return Date(timeIntervalSince1970: TimeInterval(days * 86_400 + hour * 3600 + minute * 60 + min(second, 59)))
    }

    // MARK: - Calendar arithmetic (proleptic Gregorian, H. Hinnant's algorithms)

    /// Days since 1970-01-01 of a civil date.
    static func daysFromCivil(year: Int64, month: Int64, day: Int64) -> Int64 {
        let y = month <= 2 ? year - 1 : year
        let era = (y >= 0 ? y : y - 399) / 400
        let yearOfEra = y - era * 400
        let shiftedMonth = (month + 9) % 12 // March = 0
        let dayOfYear = (153 * shiftedMonth + 2) / 5 + day - 1
        let dayOfEra = yearOfEra * 365 + yearOfEra / 4 - yearOfEra / 100 + dayOfYear
        return era * 146_097 + dayOfEra - 719_468
    }

    /// The civil date of a day count since 1970-01-01.
    static func civilFromDays(_ days: Int64) -> (year: Int64, month: Int64, day: Int64) {
        let z = days + 719_468
        let era = (z >= 0 ? z : z - 146_096) / 146_097
        let dayOfEra = z - era * 146_097
        let yearOfEra = (dayOfEra - dayOfEra / 1460 + dayOfEra / 36_524 - dayOfEra / 146_096) / 365
        let dayOfYear = dayOfEra - (365 * yearOfEra + yearOfEra / 4 - yearOfEra / 100)
        let shiftedMonth = (5 * dayOfYear + 2) / 153
        let day = dayOfYear - (153 * shiftedMonth + 2) / 5 + 1
        let month = shiftedMonth < 10 ? shiftedMonth + 3 : shiftedMonth - 9
        let year = yearOfEra + era * 400 + (month <= 2 ? 1 : 0)
        return (year, month, day)
    }

    static func daysInMonth(year: Int64, month: Int64) -> Int64 {
        switch month {
        case 2:
            let leap = (year % 4 == 0 && year % 100 != 0) || year % 400 == 0
            return leap ? 29 : 28
        case 4, 6, 9, 11:
            return 30
        default:
            return 31
        }
    }

    static func monthNumber(_ name: Substring) -> Int64? {
        let names = ["jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec"]
        guard let index = names.firstIndex(of: name.lowercased()) else { return nil }
        return Int64(index + 1)
    }

    static func pad(_ value: Int64, _ width: Int) -> String {
        let digits = String(value)
        guard digits.count < width else { return digits }
        return String(repeating: "0", count: width - digits.count) + digits
    }
}

/// Byte-level cursor over ASCII text, for the date parsers.
struct ByteScanner {
    private let bytes: [UInt8]
    private var index = 0

    init(_ text: String) {
        bytes = Array(text.utf8)
    }

    var atEnd: Bool { index >= bytes.count }

    /// Consume `byte` if it is next.
    mutating func skip(_ byte: UInt8) -> Bool {
        guard index < bytes.count, bytes[index] == byte else { return false }
        index += 1
        return true
    }

    /// Consume the next byte if it is one of `choices`.
    mutating func skipAny(_ choices: [UInt8]) -> Bool {
        guard index < bytes.count, choices.contains(bytes[index]) else { return false }
        index += 1
        return true
    }

    /// Exactly `count` decimal digits as a number.
    mutating func digits(_ count: Int) -> Int64? {
        guard index + count <= bytes.count else { return nil }
        var value: Int64 = 0
        for offset in 0..<count {
            let byte = bytes[index + offset]
            guard byte >= 48, byte <= 57 else { return nil }
            value = value * 10 + Int64(byte - 48)
        }
        index += count
        return value
    }

    /// One or more decimal digits read as a fraction (`1234567` = 0.1234567);
    /// digits beyond the ninth are consumed but ignored.
    mutating func fraction() -> Double? {
        var value: Int64 = 0
        var scale: Double = 1
        var count = 0
        while index < bytes.count, bytes[index] >= 48, bytes[index] <= 57 {
            if count < 9 {
                value = value * 10 + Int64(bytes[index] - 48)
                scale *= 10
            }
            count += 1
            index += 1
        }
        return count == 0 ? nil : Double(value) / scale
    }

    /// `+` as 1 or `-` as -1.
    mutating func sign() -> Int64? {
        guard index < bytes.count else { return nil }
        switch bytes[index] {
        case UInt8(ascii: "+"):
            index += 1
            return 1
        case UInt8(ascii: "-"):
            index += 1
            return -1
        default:
            return nil
        }
    }
}
