//! Local days and the today / this-week totals.

use super::*;

// ---- Local days and totals ------------------------------------------------------------

#[test]
fn today_starts_at_local_midnight_ahead_of_utc() {
    // 02:00 on Thursday 24 Sep in UTC+6 is still Wednesday in UTC.
    let dhaka = FixedOffset::east_opt(6 * 3600).unwrap();
    let (today, week) = local_day_and_week_start(utc("2026-09-23T20:00:00Z"), &dhaka);
    assert_eq!(today, utc("2026-09-23T18:00:00Z")); // Thu 24 Sep 00:00 +06
    assert_eq!(week, utc("2026-09-20T18:00:00Z")); // Mon 21 Sep 00:00 +06
}

#[test]
fn today_starts_at_local_midnight_behind_utc() {
    // 22:00 on Sunday 27 Sep in UTC-5 is already Monday in UTC.
    let bogota = FixedOffset::west_opt(5 * 3600).unwrap();
    let (today, week) = local_day_and_week_start(utc("2026-09-28T03:00:00Z"), &bogota);
    assert_eq!(today, utc("2026-09-27T05:00:00Z")); // Sun 27 Sep 00:00 -05
    assert_eq!(week, utc("2026-09-21T05:00:00Z")); // Mon 21 Sep 00:00 -05
}

#[test]
fn utc_zone_keeps_utc_midnights() {
    let (today, week) = local_day_and_week_start(utc("2026-09-23T12:34:56Z"), &Utc);
    assert_eq!(today, utc("2026-09-23T00:00:00Z"));
    assert_eq!(week, utc("2026-09-21T00:00:00Z"));
}

#[test]
fn past_days_of_the_week_are_local_days_before_today() {
    let dhaka = FixedOffset::east_opt(6 * 3600).unwrap();
    // Thursday 24 Sep, 02:00 in UTC+6: Monday to Wednesday are past.
    let days = past_days_of_week(utc("2026-09-23T20:00:00Z"), &dhaka);
    assert_eq!(days.len(), 3);
    assert_eq!(days[0], (0, utc("2026-09-20T18:00:00Z"), utc("2026-09-21T18:00:00Z")));
    assert_eq!(days[2], (2, utc("2026-09-22T18:00:00Z"), utc("2026-09-23T18:00:00Z")));

    // On a Monday nothing is past yet.
    assert!(past_days_of_week(utc("2026-09-21T09:00:00Z"), &Utc).is_empty());
    // On a Sunday, six days are.
    assert_eq!(past_days_of_week(utc("2026-09-27T09:00:00Z"), &Utc).len(), 6);
}

#[test]
fn only_the_part_of_a_block_after_the_cutoff_is_counted() {
    let midnight = utc("2026-09-30T18:00:00Z");
    // Entirely after: all of it.
    assert_eq!(tracked_secs_since(utc("2026-09-30T18:10:00Z"), utc("2026-09-30T18:15:00Z"), midnight), 300.0);
    // Spanning midnight: only the part after it.
    assert_eq!(tracked_secs_since(utc("2026-09-30T17:58:00Z"), utc("2026-09-30T18:03:30Z"), midnight), 210.0);
    // Entirely before: nothing.
    assert_eq!(tracked_secs_since(utc("2026-09-30T17:00:00Z"), utc("2026-09-30T17:05:00Z"), midnight), 0.0);
}
