//! Formatting and small derivations shared by the window and the tray.

use chrono::{DateTime, Datelike, Local, TimeZone, Utc};

use crate::state::{SharedState, TrackerStatus};
use crate::worker::{local_day_and_week_start, tracked_secs_since};

/// "3h 42m", or "42m" under an hour.
pub fn minutes_label(total: u64) -> String {
    let hours = total / 60;
    let minutes = total % 60;
    if hours > 0 {
        format!("{hours}h {minutes:02}m")
    } else {
        format!("{minutes}m")
    }
}

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

/// Wall-clock `HH:MM` in the user's time zone.
pub fn local_hm(at: DateTime<Utc>) -> String {
    at.with_timezone(&Local).format("%H:%M").to_string()
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
        format!("{minutes} ({} – {})", start.format("%H:%M"), end.format("%H:%M"))
    } else {
        format!("{minutes} ({} – {})", start.format("%a %H:%M"), end.format("%a %H:%M"))
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

/// Today / this week including the block in progress, in seconds. The worker
/// publishes the saved totals; the time not saved yet is added here every
/// frame.
pub fn live_totals(snapshot: &SharedState, now: DateTime<Utc>) -> (f64, f64) {
    let (mut today, mut week) = (snapshot.tracked_today_secs, snapshot.tracked_week_secs);
    if let Some(block) = snapshot.unsaved_since.or(snapshot.block_started_at) {
        let (today_start, week_start) = local_day_and_week_start(now, &Local);
        today += tracked_secs_since(block, now, today_start);
        week += tracked_secs_since(block, now, week_start);
    }
    (today, week)
}

/// Pausing applies while signed in and not already paused — including while
/// offline, so a flaky connection never stops someone from taking a break,
/// and while away from the computer.
pub fn can_pause(status: TrackerStatus) -> bool {
    matches!(status, TrackerStatus::Tracking | TrackerStatus::Offline | TrackerStatus::Away)
}

/// Case-insensitive app aliases, keyed by the lower-cased name with ".exe"
/// stripped (CONTRACT §8) — the same table as the web's `friendlyAppName`
/// and the server's reports, so an app reads the same here as on the
/// dashboard. The second group maps every canonical name to itself, so a
/// name that is already canonical stays as it is.
const APP_ALIASES: [(&str, &str); 40] = [
    ("chrome", "Google Chrome"),
    ("google chrome", "Google Chrome"),
    ("msedge", "Microsoft Edge"),
    ("code", "Visual Studio Code"),
    ("firefox", "Firefox"),
    ("slack", "Slack"),
    ("teams", "Microsoft Teams"),
    ("ms-teams", "Microsoft Teams"),
    ("outlook", "Outlook"),
    ("winword", "Word"),
    ("excel", "Excel"),
    ("powerpnt", "PowerPoint"),
    ("explorer", "File Explorer"),
    ("finder", "Finder"),
    ("windowsterminal", "Windows Terminal"),
    ("terminal", "Terminal"),
    ("iterm2", "iTerm2"),
    ("devenv", "Visual Studio"),
    ("idea64", "IntelliJ IDEA"),
    ("rider64", "Rider"),
    ("figma", "Figma"),
    ("zoom", "Zoom"),
    ("zoom.us", "Zoom"),
    ("notepad", "Notepad"),
    ("applicationframehost", "Windows app"),
    ("lockapp", "Lock screen"),
    ("microsoft edge", "Microsoft Edge"),
    ("visual studio code", "Visual Studio Code"),
    ("microsoft teams", "Microsoft Teams"),
    ("word", "Word"),
    ("powerpoint", "PowerPoint"),
    ("file explorer", "File Explorer"),
    ("windows terminal", "Windows Terminal"),
    ("visual studio", "Visual Studio"),
    ("intellij idea", "IntelliJ IDEA"),
    ("rider", "Rider"),
    ("windows app", "Windows app"),
    ("lock screen", "Lock screen"),
    ("unknown app", UNKNOWN_APP),
    ("other apps", "Other apps"),
];

/// Shown for a window whose app could not be named.
pub const UNKNOWN_APP: &str = "Unknown app";

/// The canonical name of an app as the agent reports it (CONTRACT §8):
/// `.exe` stripped, the alias table applied case-insensitively, otherwise
/// each word's first letter upper-cased (`mytool.exe` → `Mytool`,
/// `WINWORD.EXE` → `Word`).
pub fn app_display_name(raw: &str) -> String {
    let mut stem = tidy(raw);
    while let Some(base) = strip_exe(&stem) {
        stem = tidy(base);
    }
    if stem.is_empty() {
        return UNKNOWN_APP.to_owned();
    }
    let key = stem.to_lowercase();
    if let Some((_, alias)) = APP_ALIASES.iter().find(|(name, _)| *name == key) {
        return (*alias).to_owned();
    }
    stem.split(' ').map(capitalize).collect::<Vec<_>>().join(" ")
}

/// `name` without a final `.exe` (any case), if it has one.
fn strip_exe(name: &str) -> Option<&str> {
    let cut = name.len().checked_sub(4)?;
    let (base, suffix) = (name.get(..cut)?, name.get(cut..)?);
    suffix.eq_ignore_ascii_case(".exe").then_some(base)
}

/// Trim and collapse whitespace runs to single spaces.
fn tidy(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Upper-case a word's first character and keep the rest as it is. A
/// character that upper-cases to several ("ß") is kept, as the web does.
fn capitalize(word: &str) -> String {
    let mut chars = word.chars();
    match chars.next() {
        Some(first) => {
            let mut upper = first.to_uppercase();
            let head = match (upper.next(), upper.next()) {
                (Some(single), None) => single,
                _ => first,
            };
            std::iter::once(head).chain(chars).collect()
        }
        None => String::new(),
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

/// At most `max` characters, ending in "…" when cut (Windows tooltips hold
/// 127 characters, and long names would stretch menus and pills).
pub fn truncate_chars(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_owned();
    }
    let mut cut: String = text.chars().take(max.saturating_sub(1)).collect();
    cut.truncate(cut.trim_end().len());
    cut.push('…');
    cut
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

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::FixedOffset;

    fn at(s: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc)
    }

    #[test]
    fn clock_reads_like_a_stopwatch() {
        assert_eq!(clock(chrono::Duration::seconds(0)), "00:00");
        assert_eq!(clock(chrono::Duration::seconds(309)), "05:09");
        assert_eq!(clock(chrono::Duration::seconds(3_909)), "1:05:09");
        // A deadline already passed never shows a negative time.
        assert_eq!(clock(chrono::Duration::seconds(-4)), "00:00");
    }

    #[test]
    fn minute_labels() {
        assert_eq!(minutes_label(0), "0m");
        assert_eq!(minutes_label(42), "42m");
        assert_eq!(minutes_label(222), "3h 42m");
        assert_eq!(secs_label(f64::NAN), "0m");
        assert_eq!(secs_label(-30.0), "0m");
        assert_eq!(secs_label(3_659.0), "1h 00m");
    }

    #[test]
    fn durations_read_as_words() {
        assert_eq!(minutes_words(42), "42 min");
        assert_eq!(minutes_words(72), "1 h 12 min");
        assert_eq!(minutes_words(180), "3 h");
        assert_eq!(minutes_words(-3), "0 min");
    }

    #[test]
    fn time_away_reads_like_the_contract_example() {
        let dhaka = FixedOffset::east_opt(6 * 3600).unwrap();
        let summary = away_summary_in(at("2026-10-05T04:05:00Z"), at("2026-10-05T04:47:30Z"), &dhaka);
        assert_eq!(summary, "42 min (10:05 – 10:47)");
        // Overnight: the days are named.
        let overnight = away_summary_in(at("2026-10-05T12:00:00Z"), at("2026-10-06T03:00:00Z"), &dhaka);
        assert_eq!(overnight, "15 h (Mon 18:00 – Tue 09:00)");
    }

    #[test]
    fn list_times_name_the_day_when_it_is_not_today() {
        let utc = Utc;
        let now = at("2026-10-07T15:00:00Z"); // a Wednesday
        assert_eq!(day_and_time_in(at("2026-10-07T09:30:00Z"), now, &utc), "09:30");
        assert_eq!(day_and_time_in(at("2026-10-06T09:30:00Z"), now, &utc), "Yesterday 09:30");
        assert_eq!(day_and_time_in(at("2026-10-04T09:30:00Z"), now, &utc), "Sun 09:30");
        assert_eq!(day_and_time_in(at("2026-09-12T09:30:00Z"), now, &utc), "12 Sep 09:30");
        assert_eq!(day_and_time_in(at("2025-12-30T09:30:00Z"), now, &utc), "30 Dec 2025 09:30");
    }

    #[test]
    fn live_totals_add_the_running_block() {
        let now = Utc::now();
        let mut snapshot = SharedState {
            tracked_today_secs: 600.0,
            tracked_week_secs: 3_600.0,
            ..SharedState::default()
        };
        assert_eq!(live_totals(&snapshot, now), (600.0, 3_600.0));

        // A block that began 90 s ago (and today, locally) is added to both.
        snapshot.block_started_at = Some(now - chrono::Duration::seconds(90));
        let (today_start, _) = local_day_and_week_start(now, &Local);
        let expected_today = 600.0 + tracked_secs_since(now - chrono::Duration::seconds(90), now, today_start);
        let (today, week) = live_totals(&snapshot, now);
        assert_eq!(today, expected_today);
        assert!(week > 3_600.0 && week <= 3_690.0);
    }

    #[test]
    fn app_names_follow_the_canonical_mapping() {
        assert_eq!(app_display_name("code.exe"), "Visual Studio Code");
        assert_eq!(app_display_name("Code.exe"), "Visual Studio Code");
        assert_eq!(app_display_name("WINWORD.EXE"), "Word");
        assert_eq!(app_display_name("msedge.exe"), "Microsoft Edge");
        assert_eq!(app_display_name("ApplicationFrameHost.exe"), "Windows app");
        assert_eq!(app_display_name("dosi-tracker.exe"), "Dosi-tracker");
        // Anything else: each word capitalized, the rest kept.
        assert_eq!(app_display_name("myTool.exe"), "MyTool");
        assert_eq!(app_display_name("  some   tool.EXE.exe "), "Some Tool");
        assert_eq!(app_display_name("ß-app.exe"), "ß-app");
        // Multi-byte names never split a character.
        assert_eq!(app_display_name("ßßa"), "ßßa");
        assert_eq!(app_display_name("été.exe"), "Été");
        // Canonical names stay as they are.
        assert_eq!(app_display_name("Windows app"), "Windows app");
        assert_eq!(app_display_name("  "), "Unknown app");
        assert_eq!(app_display_name(".exe"), "Unknown app");
    }

    #[test]
    fn counts_pluralise() {
        assert_eq!(count(1, "upload"), "1 upload");
        assert_eq!(count(0, "upload"), "0 uploads");
        assert_eq!(count(3, "upload"), "3 uploads");
    }

    #[test]
    fn long_text_is_cut_with_an_ellipsis() {
        let long = "x".repeat(300);
        let cut = truncate_chars(&long, 127);
        assert_eq!(cut.chars().count(), 127);
        assert!(cut.ends_with('…'));
        assert_eq!(truncate_chars("short", 127), "short");
        // No dangling space before the ellipsis.
        assert_eq!(truncate_chars("Checkout page redesign", 10), "Checkout…");
    }

    #[test]
    fn utc_offsets_read_as_labels() {
        assert_eq!(utc_offset_label(360), "UTC+06:00");
        assert_eq!(utc_offset_label(-330), "UTC\u{2212}05:30");
        assert_eq!(utc_offset_label(0), "UTC+00:00");
        assert_eq!(utc_offset_label(345), "UTC+05:45");
    }

    #[test]
    fn server_labels_drop_the_scheme() {
        assert_eq!(server_label("https://tracker.acme.com"), "tracker.acme.com");
        assert_eq!(server_label("http://10.0.0.5:6006/tracker/"), "10.0.0.5:6006/tracker");
        assert_eq!(server_label(" localhost:6006 "), "localhost:6006");
    }
}
