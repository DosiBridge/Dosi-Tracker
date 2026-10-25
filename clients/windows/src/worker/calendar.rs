//! Local days and weeks, for the today / this-week totals.

use chrono::{DateTime, Datelike, NaiveDate, TimeZone, Utc};

/// Each local day of this week before today as (slot, start, end): the slot
/// is the day's index from Monday, and the day runs `[start, end)`. Empty on
/// a Monday.
pub fn past_days_of_week<Tz: TimeZone>(now: DateTime<Utc>, tz: &Tz) -> Vec<(usize, DateTime<Utc>, DateTime<Utc>)> {
    let today = now.with_timezone(tz).date_naive();
    let monday = today - chrono::Days::new(u64::from(today.weekday().num_days_from_monday()));
    monday
        .iter_days()
        .take_while(|day| *day < today)
        .enumerate()
        .filter_map(|(slot, day)| Some((slot, local_midnight(day, tz)?, local_midnight(day.succ_opt()?, tz)?)))
        .collect()
}

/// Seconds of `started_at .. ended_at` that fall on or after `since` (e.g.
/// today's local midnight), so a block spanning midnight only adds today's
/// part to "today".
pub fn tracked_secs_since(started_at: DateTime<Utc>, ended_at: DateTime<Utc>, since: DateTime<Utc>) -> f64 {
    let from = started_at.max(since);
    if ended_at <= from {
        0.0
    } else {
        (ended_at - from).num_milliseconds() as f64 / 1000.0
    }
}

/// Local midnight today and on this week's Monday, as UTC instants for the
/// API. "Today" follows the user's wall clock, not UTC — otherwise the cards
/// would reset mid-day (at 06:00 in UTC+6, at 19:00 the day before in UTC-5).
pub fn local_day_and_week_start<Tz: TimeZone>(now: DateTime<Utc>, tz: &Tz) -> (DateTime<Utc>, DateTime<Utc>) {
    let today = now.with_timezone(tz).date_naive();
    // Monday-anchored week.
    let monday = today - chrono::Days::new(u64::from(today.weekday().num_days_from_monday()));
    (
        local_midnight(today, tz).unwrap_or(now),
        local_midnight(monday, tz).unwrap_or(now),
    )
}

/// The first instant of `date` in `tz`. Where a DST jump skips midnight, the
/// day starts at the first hour that exists.
pub(super) fn local_midnight<Tz: TimeZone>(date: NaiveDate, tz: &Tz) -> Option<DateTime<Utc>> {
    (0..3)
        .find_map(|hour| tz.from_local_datetime(&date.and_hms_opt(hour, 0, 0)?).earliest())
        .map(|start| start.with_timezone(&Utc))
}
