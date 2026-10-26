//! Formatting helpers for durations, totals and timestamps in the GUI, ported
//! from the Windows agent's `ui/format.rs`. The helpers the tray shares
//! (labels, truncation, live totals) live in [`crate::agent::tray`] and are
//! re-exported here, so the window and the tray read the same.

use chrono::{DateTime, Datelike, Local, TimeZone, Utc};

pub use crate::agent::tray::{can_pause, live_totals, local_hm, minutes_label, truncate_chars};

/// [`minutes_label`] for a number of seconds (whole minutes, rounded down).
pub fn secs_label(secs: f64) -> String {
    let secs = if secs.is_finite() { secs.max(0.0) } else { 0.0 };
    minutes_label((secs / 60.0) as u64)
}

/// A duration in words, for sentences: "42 min", "1 h 12 min", "3 h".
pub fn minutes_words(total: i64) -> String {
    let total = total.max(0);
    match (total / 60, total % 60) {
        (0, minutes) => format!("{minutes} min"),
        (hours, 0) => format!("{hours} h"),
        (hours, minutes) => format!("{hours} h {minutes} min"),
    }
}

/// A stopwatch reading: `1:05:09`, or `05:09` under an hour.
pub fn clock(elapsed: chrono::Duration) -> String {
    let total = elapsed.num_seconds().max(0);
    let (hours, minutes, seconds) = (total / 3600, total % 3600 / 60, total % 60);
    if hours > 0 {
        format!("{hours}:{minutes:02}:{seconds:02}")
    } else {
        format!("{minutes:02}:{seconds:02}")
    }
}

/// Where the time away went, for the "keep it?" question: "42 min (10:05 –
/// 10:47)", naming the days when it spans midnight ("15 h (Mon 18:00 – Tue
/// 09:00)").
pub fn away_summary(from: DateTime<Utc>, to: DateTime<Utc>) -> String {
    away_summary_in(from, to, &Local)
}

/// [`away_summary`] in a given time zone.
pub fn away_summary_in<Tz: TimeZone>(from: DateTime<Utc>, to: DateTime<Utc>, tz: &Tz) -> String
where
    Tz::Offset: std::fmt::Display,
{
    let minutes = minutes_words((to - from).num_minutes());
    let (start, end) = (from.with_timezone(tz), to.with_timezone(tz));
    if start.date_naive() == end.date_naive() {
        format!(
            "{minutes} ({} – {})",
            start.format("%H:%M"),
            end.format("%H:%M")
        )
    } else {
        format!(
            "{minutes} ({} – {})",
            start.format("%a %H:%M"),
            end.format("%a %H:%M")
        )
    }
}

/// When something happened, as a list shows it: "14:05" today, "Yesterday
/// 14:05", "Mon 14:05" within the week, else "12 Sep 14:05".
pub fn day_and_time(at: DateTime<Utc>, now: DateTime<Utc>) -> String {
    day_and_time_in(at, now, &Local)
}

/// [`day_and_time`] in a given time zone.
pub fn day_and_time_in<Tz: TimeZone>(at: DateTime<Utc>, now: DateTime<Utc>, tz: &Tz) -> String
where
    Tz::Offset: std::fmt::Display,
{
    let (local, today) = (at.with_timezone(tz), now.with_timezone(tz).date_naive());
    let days_ago = (today - local.date_naive()).num_days();
    match days_ago {
        0 => local.format("%H:%M").to_string(),
        1 => format!("Yesterday {}", local.format("%H:%M")),
        2..=6 => local.format("%a %H:%M").to_string(),
        _ if local.year() == today.year() => local.format("%-d %b %H:%M").to_string(),
        _ => local.format("%-d %b %Y %H:%M").to_string(),
    }
}

/// `n` with a singular or plural noun: "1 upload", "3 uploads".
pub fn count(n: u32, noun: &str) -> String {
    if n == 1 {
        format!("1 {noun}")
    } else {
        format!("{n} {noun}s")
    }
}

/// A UTC offset in minutes as a label: `+360` → "UTC+06:00", `-330` →
/// "UTC−05:30".
pub fn utc_offset_label(minutes: i32) -> String {
    let sign = if minutes < 0 { '\u{2212}' } else { '+' };
    let minutes = minutes.unsigned_abs();
    format!("UTC{sign}{:02}:{:02}", minutes / 60, minutes % 60)
}

/// The host (and port) of a server address, for compact display:
/// `https://tracker.acme.com/api` → `tracker.acme.com/api`.
pub fn server_label(url: &str) -> String {
    let url = url.trim();
    let without_scheme = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))
        .unwrap_or(url);
    without_scheme.trim_end_matches('/').to_owned()
}

/// A measured activity level as a label; `None` (not measured) reads "—".
pub fn percent_label(percent: Option<u8>) -> String {
    match percent {
        Some(percent) => format!("{percent}%"),
        None => "—".to_owned(),
    }
}
