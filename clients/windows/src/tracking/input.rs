use std::collections::VecDeque;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, Mutex};

use chrono::{DateTime, Utc};

/// How far back input activity is remembered. Must cover the longest block
/// (the interval is clamped to 60 minutes) plus slack for a slow capture.
const ACTIVITY_RETENTION_SECS: i64 = 65 * 60;

/// Hard cap on remembered key presses / clicks per kind. Normal typing stays
/// far below it (~10k/hour); it only bounds memory under an auto-clicker.
const MAX_EVENTS: usize = 200_000;

/// Keyboard and mouse input, recorded as *when* it happened (never keystroke
/// content): a millisecond timestamp per key press and click, plus a
/// per-second record of any input for the activity %.
///
/// Every count is taken over an explicit time window rather than from
/// counters reset at block boundaries, so block totals, the per-minute
/// timeline and the per-app breakdown always agree, and input made before a
/// block (while paused, signed out, or on another project) can never leak
/// into it.
#[derive(Default)]
pub struct InputCounter {
    key_presses: EventLog,
    mouse_clicks: EventLog,
    keyboard_activity: ActivityLog,
    mouse_activity: ActivityLog,
}

impl InputCounter {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn inc_keyboard(&self) {
        let now = Utc::now();
        self.key_presses.record(now.timestamp_millis());
        self.keyboard_activity.mark(now.timestamp());
    }

    pub fn inc_mouse(&self) {
        let now = Utc::now();
        self.mouse_clicks.record(now.timestamp_millis());
        self.mouse_activity.mark(now.timestamp());
    }

    /// Mouse movement / wheel: counts as activity (reading and scrolling is
    /// work) but not as a click.
    pub fn mark_mouse_activity(&self) {
        self.mouse_activity.mark(Utc::now().timestamp());
    }

    /// Timestamps (Unix ms) of key presses in `[start, end)`.
    pub fn key_presses_between(&self, start: DateTime<Utc>, end: DateTime<Utc>) -> Vec<i64> {
        self.key_presses.between(start.timestamp_millis(), end.timestamp_millis())
    }

    /// Timestamps (Unix ms) of mouse clicks in `[start, end)`.
    pub fn clicks_between(&self, start: DateTime<Utc>, end: DateTime<Utc>) -> Vec<i64> {
        self.mouse_clicks.between(start.timestamp_millis(), end.timestamp_millis())
    }

    /// (key presses, clicks) in `[start, end)` — cheap enough to call every
    /// frame for the live view.
    pub fn counts_between(&self, start: DateTime<Utc>, end: DateTime<Utc>) -> (u64, u64) {
        let (start, end) = (start.timestamp_millis(), end.timestamp_millis());
        (self.key_presses.count(start, end), self.mouse_clicks.count(start, end))
    }

    /// Unix seconds inside `started_at ..= ended_at` that had input of an
    /// allowed kind — what the activity % and the timeline's active minutes
    /// are computed from.
    pub fn active_seconds(
        &self,
        started_at: DateTime<Utc>,
        ended_at: DateTime<Utc>,
        keyboard: bool,
        mouse: bool,
    ) -> Vec<i64> {
        let (start, end) = (started_at.timestamp(), ended_at.timestamp());
        let mut seconds = Vec::new();
        if keyboard {
            seconds.extend(self.keyboard_activity.seconds_between(start, end));
        }
        if mouse {
            seconds.extend(self.mouse_activity.seconds_between(start, end));
        }
        seconds
    }

    /// Activity % (0-100) of the block `started_at ..= ended_at`, counting only
    /// the input kinds the project allows. Input from before the block (e.g.
    /// while paused) never counts, because only seconds inside the window do.
    pub fn activity_percent(
        &self,
        started_at: DateTime<Utc>,
        ended_at: DateTime<Utc>,
        keyboard: bool,
        mouse: bool,
    ) -> u8 {
        let seconds = self.active_seconds(started_at, ended_at, keyboard, mouse);
        activity_percent(&seconds, started_at.timestamp(), ended_at.timestamp())
    }
}

/// Millisecond timestamps of discrete input events (key presses or clicks),
/// oldest first, bounded by [`ACTIVITY_RETENTION_SECS`] and [`MAX_EVENTS`].
#[derive(Default)]
struct EventLog {
    events: Mutex<VecDeque<i64>>,
}

impl EventLog {
    fn record(&self, at_ms: i64) {
        let mut events = self.events.lock().unwrap_or_else(|e| e.into_inner());
        // Keep the log sorted: a clock stepping backwards files the event
        // with the newest one instead of out of order.
        let at_ms = events.back().map_or(at_ms, |&last| at_ms.max(last));
        events.push_back(at_ms);
        let horizon = at_ms - ACTIVITY_RETENTION_SECS * 1000;
        while events.front().is_some_and(|&oldest| oldest < horizon) || events.len() > MAX_EVENTS {
            events.pop_front();
        }
    }

    fn range(events: &VecDeque<i64>, start_ms: i64, end_ms: i64) -> std::ops::Range<usize> {
        let from = events.partition_point(|&t| t < start_ms);
        let to = events.partition_point(|&t| t < end_ms).max(from);
        from..to
    }

    fn count(&self, start_ms: i64, end_ms: i64) -> u64 {
        let events = self.events.lock().unwrap_or_else(|e| e.into_inner());
        Self::range(&events, start_ms, end_ms).len() as u64
    }

    fn between(&self, start_ms: i64, end_ms: i64) -> Vec<i64> {
        let events = self.events.lock().unwrap_or_else(|e| e.into_inner());
        events.range(Self::range(&events, start_ms, end_ms)).copied().collect()
    }
}

/// Distinct Unix seconds in which input happened, oldest first, bounded to
/// [`ACTIVITY_RETENTION_SECS`] so memory stays small (< 40 KB even under
/// constant input).
#[derive(Default)]
struct ActivityLog {
    /// Last second recorded — lets the hot path skip the lock for the many
    /// events (typing, mouse movement) that land in the same second.
    last: AtomicI64,
    seconds: Mutex<VecDeque<i64>>,
}

impl ActivityLog {
    fn mark(&self, second: i64) {
        if self.last.swap(second, Ordering::Relaxed) == second {
            return;
        }
        let mut seconds = self.seconds.lock().unwrap_or_else(|e| e.into_inner());
        // Keep the log sorted; a clock stepping backwards just drops the event.
        if seconds.back().is_none_or(|&last| second > last) {
            seconds.push_back(second);
        }
        while seconds
            .front()
            .is_some_and(|&oldest| oldest < second - ACTIVITY_RETENTION_SECS)
        {
            seconds.pop_front();
        }
    }

    fn seconds_between(&self, start: i64, end: i64) -> Vec<i64> {
        let seconds = self.seconds.lock().unwrap_or_else(|e| e.into_inner());
        seconds
            .iter()
            .copied()
            .filter(|&s| s >= start && s <= end)
            .collect()
    }
}

/// Hubstaff-style activity %: the block is split into whole minutes (counted
/// from the block start, so an off-the-minute start is not penalised) and the
/// result is the share of those minutes that had any input.
///
/// The minute count is the block length rounded to the nearest minute (at
/// least one); a leftover fraction is folded into the last minute, so no bucket
/// is shorter than 30 s — a sliver of a minute can never cost a fully active
/// user a percentage point.
pub fn activity_percent(active_seconds: &[i64], start: i64, end: i64) -> u8 {
    let minutes = minute_buckets(start, end);
    if minutes == 0 {
        return 0;
    }
    let active = active_minutes(active_seconds, start, end);
    let active_minutes = active.iter().filter(|&&a| a).count() as i64;
    // Round to the nearest whole percent.
    ((active_minutes * 100 + minutes / 2) / minutes) as u8
}

/// Number of minute buckets in the block `start ..= end` (Unix seconds): the
/// length rounded to the nearest minute, at least one; zero for an empty or
/// inverted block.
pub fn minute_buckets(start: i64, end: i64) -> i64 {
    let span = end - start;
    if span <= 0 {
        0
    } else {
        ((span + 30) / 60).max(1)
    }
}

/// The bucket a second inside the block falls into (a trailing sliver joins
/// the last bucket).
pub fn bucket_of(second: i64, start: i64, minutes: i64) -> usize {
    ((second - start) / 60).clamp(0, minutes - 1) as usize
}

/// Which minute buckets of the block had any input — the activity % is the
/// share of `true`s.
pub fn active_minutes(active_seconds: &[i64], start: i64, end: i64) -> Vec<bool> {
    let minutes = minute_buckets(start, end);
    if minutes == 0 {
        return Vec::new();
    }
    let mut active = vec![false; minutes as usize];
    for &second in active_seconds {
        if second < start || second > end {
            continue;
        }
        active[bucket_of(second, start, minutes)] = true;
    }
    active
}

/// Spawn a background thread with a global, event-driven input hook.
/// Uses `rdev::listen`, which blocks on the OS event queue (no polling).
pub fn spawn_listener(counter: Arc<InputCounter>) {
    std::thread::spawn(move || {
        let callback = move |event: rdev::Event| match event.event_type {
            rdev::EventType::KeyPress(_) => counter.inc_keyboard(),
            rdev::EventType::ButtonPress(_) => counter.inc_mouse(),
            rdev::EventType::MouseMove { .. } | rdev::EventType::Wheel { .. } => {
                counter.mark_mouse_activity()
            }
            _ => {}
        };

        if let Err(e) = rdev::listen(callback) {
            tracing::error!(?e, "global input listener stopped");
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    const T0: i64 = 1_790_000_000; // arbitrary Unix second

    #[test]
    fn empty_or_degenerate_blocks_are_zero() {
        assert_eq!(activity_percent(&[], T0, T0 + 600), 0);
        assert_eq!(activity_percent(&[T0], T0, T0), 0);
        assert_eq!(activity_percent(&[T0], T0 + 10, T0), 0);
    }

    #[test]
    fn input_in_every_minute_is_full_activity() {
        let seconds: Vec<i64> = (0..10).map(|m| T0 + m * 60 + 30).collect();
        assert_eq!(activity_percent(&seconds, T0, T0 + 600), 100);
    }

    #[test]
    fn counts_minutes_not_events() {
        // A burst of input all inside minute 0, then 2 more active minutes.
        let mut seconds: Vec<i64> = (0..60).map(|s| T0 + s).collect();
        seconds.push(T0 + 5 * 60 + 1);
        seconds.push(T0 + 9 * 60 + 59);
        assert_eq!(activity_percent(&seconds, T0, T0 + 600), 30);
    }

    #[test]
    fn minutes_are_counted_from_the_block_start() {
        // Block starts mid-minute; one input per elapsed minute is still 100%.
        let start = T0 + 37;
        let seconds: Vec<i64> = (0..10).map(|m| start + m * 60 + 59).collect();
        assert_eq!(activity_percent(&seconds, start, start + 600), 100);
    }

    #[test]
    fn input_outside_the_block_is_ignored() {
        // Input while paused (before the block) and after it must not count.
        let seconds = [T0 - 1, T0 - 120, T0 + 601, T0 + 900];
        assert_eq!(activity_percent(&seconds, T0, T0 + 600), 0);
    }

    #[test]
    fn a_trailing_sliver_is_folded_into_the_last_minute() {
        // 10 min + 5 s: still 10 buckets, the 5 s join the last one.
        let seconds: Vec<i64> = (0..10).map(|m| T0 + m * 60).collect();
        assert_eq!(activity_percent(&seconds, T0, T0 + 605), 100);
        // Input only in the sliver marks the last bucket.
        assert_eq!(activity_percent(&[T0 + 603], T0, T0 + 605), 10);
    }

    #[test]
    fn short_blocks_have_at_least_one_minute() {
        assert_eq!(activity_percent(&[T0 + 5], T0, T0 + 20), 100);
        assert_eq!(activity_percent(&[], T0, T0 + 20), 0);
    }

    #[test]
    fn result_is_rounded_to_the_nearest_percent() {
        // 1 of 3 minutes = 33.3% -> 33; 2 of 3 = 66.7% -> 67.
        assert_eq!(activity_percent(&[T0], T0, T0 + 180), 33);
        assert_eq!(activity_percent(&[T0, T0 + 60], T0, T0 + 180), 67);
    }

    #[test]
    fn activity_log_dedupes_seconds_and_drops_old_entries() {
        let log = ActivityLog::default();
        log.mark(T0);
        log.mark(T0); // same second: no duplicate
        log.mark(T0 + 1);
        assert_eq!(log.seconds_between(T0, T0 + 10), vec![T0, T0 + 1]);

        // Far in the future: everything older than the retention window goes.
        log.mark(T0 + ACTIVITY_RETENTION_SECS + 10);
        assert_eq!(
            log.seconds_between(T0 - 1, T0 + 2 * ACTIVITY_RETENTION_SECS),
            vec![T0 + ACTIVITY_RETENTION_SECS + 10]
        );
    }

    #[test]
    fn counter_respects_allowed_input_kinds() {
        let counter = InputCounter::new();
        counter.keyboard_activity.mark(T0 + 10);
        counter.mouse_activity.mark(T0 + 70);
        let start = DateTime::from_timestamp(T0, 0).unwrap();
        let end = DateTime::from_timestamp(T0 + 120, 0).unwrap();

        assert_eq!(counter.activity_percent(start, end, true, true), 100);
        assert_eq!(counter.activity_percent(start, end, true, false), 50);
        assert_eq!(counter.activity_percent(start, end, false, true), 50);
        assert_eq!(counter.activity_percent(start, end, false, false), 0);
    }

    #[test]
    fn counts_are_taken_over_a_window() {
        let counter = InputCounter::new();
        let before = Utc::now() - chrono::Duration::seconds(1);
        counter.inc_keyboard();
        counter.inc_keyboard();
        counter.inc_mouse();
        let after = Utc::now() + chrono::Duration::seconds(1);
        assert_eq!(counter.counts_between(before, after), (2, 1));
        assert_eq!(counter.key_presses_between(before, after).len(), 2);
        assert_eq!(counter.clicks_between(before, after).len(), 1);
        // Nothing before the window counts in it.
        assert_eq!(counter.counts_between(after, after + chrono::Duration::seconds(5)), (0, 0));
    }

    #[test]
    fn event_windows_are_half_open_so_blocks_never_share_an_event() {
        let log = EventLog::default();
        for t in [1_000, 2_000, 3_000] {
            log.record(t);
        }
        // Back-to-back blocks [1000, 2000) and [2000, 3001): each event once.
        assert_eq!(log.between(1_000, 2_000), vec![1_000]);
        assert_eq!(log.between(2_000, 3_001), vec![2_000, 3_000]);
        assert_eq!(log.count(3_001, 9_000), 0);
        assert_eq!(log.count(5_000, 1_000), 0); // inverted window
    }

    #[test]
    fn event_log_stays_sorted_and_bounded() {
        let log = EventLog::default();
        log.record(10_000);
        log.record(9_000); // clock stepped back: filed with the newest
        assert_eq!(log.between(0, 20_000), vec![10_000, 10_000]);

        // Far in the future: everything older than the retention window goes.
        let later = 10_000 + ACTIVITY_RETENTION_SECS * 1000 + 1;
        log.record(later);
        assert_eq!(log.between(0, i64::MAX), vec![later]);
    }

    #[test]
    fn active_minutes_match_the_activity_percent() {
        // 3 buckets; input in the first and the trailing sliver of the last.
        let active = active_minutes(&[T0 + 5, T0 + 185], T0, T0 + 190);
        assert_eq!(active, vec![true, false, true]);
        assert_eq!(activity_percent(&[T0 + 5, T0 + 185], T0, T0 + 190), 67);
        assert!(active_minutes(&[T0], T0, T0).is_empty());
    }
}
