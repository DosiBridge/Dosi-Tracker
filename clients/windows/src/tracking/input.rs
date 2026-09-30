use std::collections::VecDeque;
use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use chrono::{DateTime, Utc};

/// How far back input activity is remembered. Must cover the longest block
/// (the interval is clamped to 60 minutes) plus slack for a slow capture.
const ACTIVITY_RETENTION_SECS: i64 = 65 * 60;

/// Thread-safe counters for keyboard hits and mouse clicks, plus a per-second
/// record of *when* there was input (for the activity %).
///
/// We only keep counts and timestamps (never keystroke content) to respect
/// privacy and to keep the footprint tiny.
#[derive(Default)]
pub struct InputCounter {
    keyboard: AtomicU64,
    mouse: AtomicU64,
    keyboard_activity: ActivityLog,
    mouse_activity: ActivityLog,
}

impl InputCounter {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn inc_keyboard(&self) {
        self.keyboard.fetch_add(1, Ordering::Relaxed);
        self.keyboard_activity.mark(Utc::now().timestamp());
    }

    pub fn inc_mouse(&self) {
        self.mouse.fetch_add(1, Ordering::Relaxed);
        self.mouse_activity.mark(Utc::now().timestamp());
    }

    /// Mouse movement / wheel: counts as activity (reading and scrolling is
    /// work) but not as a click.
    pub fn mark_mouse_activity(&self) {
        self.mouse_activity.mark(Utc::now().timestamp());
    }

    /// Read counts and reset them to zero. Returns (keyboard_hits, mouse_clicks).
    pub fn take(&self) -> (u64, u64) {
        (
            self.keyboard.swap(0, Ordering::Relaxed),
            self.mouse.swap(0, Ordering::Relaxed),
        )
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
        let (start, end) = (started_at.timestamp(), ended_at.timestamp());
        let mut seconds = Vec::new();
        if keyboard {
            seconds.extend(self.keyboard_activity.seconds_between(start, end));
        }
        if mouse {
            seconds.extend(self.mouse_activity.seconds_between(start, end));
        }
        activity_percent(&seconds, start, end)
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
    let span = end - start;
    if span <= 0 {
        return 0;
    }
    let minutes = ((span + 30) / 60).max(1);
    let mut active = vec![false; minutes as usize];
    for &second in active_seconds {
        if second < start || second > end {
            continue;
        }
        let index = ((second - start) / 60).min(minutes - 1);
        active[index as usize] = true;
    }
    let active_minutes = active.iter().filter(|&&a| a).count() as i64;
    // Round to the nearest whole percent.
    ((active_minutes * 100 + minutes / 2) / minutes) as u8
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
    fn take_resets_counts() {
        let counter = InputCounter::new();
        counter.inc_keyboard();
        counter.inc_keyboard();
        counter.inc_mouse();
        assert_eq!(counter.take(), (2, 1));
        assert_eq!(counter.take(), (0, 0));
    }
}
