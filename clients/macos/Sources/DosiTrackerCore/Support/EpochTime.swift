import Foundation

/// Integer Unix time, the unit all tracking math works in.
///
/// The Windows agent buckets input with chrono's whole milliseconds and whole
/// seconds; these helpers give the macOS agent the same integers so both
/// platforms produce the same activity %, timeline and per-app numbers. A
/// second is always derived from the millisecond value (floor), so the two
/// never disagree about which second an instant falls in.
public enum EpochTime {
    /// Unix milliseconds of `date`, rounded to the nearest millisecond (dates
    /// read from the wire are whole milliseconds and round-trip unchanged).
    public static func milliseconds(_ date: Date) -> Int64 {
        Int64((date.timeIntervalSince1970 * 1000).rounded())
    }

    /// The whole Unix second that contains `date`.
    public static func seconds(_ date: Date) -> Int64 {
        floorDiv(milliseconds(date), 1000)
    }

    /// The instant of a Unix millisecond.
    public static func date(milliseconds: Int64) -> Date {
        Date(timeIntervalSince1970: TimeInterval(milliseconds) / 1000)
    }

    /// The instant of a Unix second.
    public static func date(seconds: Int64) -> Date {
        Date(timeIntervalSince1970: TimeInterval(seconds))
    }

    /// Division rounding toward negative infinity (Rust's `div_euclid` for a
    /// positive divisor), unlike `/`, which truncates toward zero.
    public static func floorDiv(_ dividend: Int64, _ divisor: Int64) -> Int64 {
        let quotient = dividend / divisor
        if dividend % divisor != 0 && ((dividend < 0) != (divisor < 0)) {
            return quotient - 1
        }
        return quotient
    }
}
