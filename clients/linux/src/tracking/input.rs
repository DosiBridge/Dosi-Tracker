//! Input log: per-second buckets {keys, clicks, active, injected}; counts
//! only, never keycodes, text or coordinates (ported Windows math).
//!
//! The XI2 listener ([`super::x11_input`]) files every physical key press or
//! click as one count in the bucket of the second it happened, marks pointer
//! motion and the wheel as activity, and counts XTEST-originated events aside
//! as injected. Nothing finer than the second and nothing about *which* key or
//! button is ever stored: no API here accepts or returns a keycode or a
//! sub-second time.
//!
//! A block reads its window through [`InputLog::measurements`], which expands
//! the buckets into second-quantized timestamps (each press at `second * 1000`)
//! so the Windows functions below ([`activity_percent`], [`active_minutes`],
//! the usage timeline) run unchanged.

use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::Mutex;

use chrono::{DateTime, Utc};

/// How far back input is remembered. The longest block (a 60-minute
/// interval) can end at the last input before the longest idle threshold
/// (60 minutes) cuts it — and is only built then — plus slack for a slow
/// capture.
pub const RETENTION_SECS: i64 = 125 * 60;

// The first minute of such a block is 120 minutes old when it is built.
const _: () = assert!(RETENTION_SECS >= 120 * 60);

/// Number of per-second buckets in the ring: one per remembered second.
pub const RING_SECONDS: usize = RETENTION_SECS as usize;

/// Retention of the health history (outages), as for the buckets.
const MAX_OUTAGES: usize = 64;

/// One second of input: counts only.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct Bucket {
    /// The Unix second this bucket holds; a bucket whose second is not the
    /// one asked for is stale (its slot was reused or never written).
    second: i64,
    keys: u16,
    clicks: u16,
    injected: u16,
    active: bool,
}

/// What a block reads from the log for its window `[start, end]`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct InputMeasurements {
    /// One entry per physical key press in `[start, end)`, at `second * 1000`
    /// (Unix ms), oldest first.
    pub key_presses_ms: Vec<i64>,
    /// One entry per physical click in `[start, end)`, at `second * 1000`.
    pub clicks_ms: Vec<i64>,
    /// Unix seconds inside `start ..= end` with any physical input.
    pub active_seconds: Vec<i64>,
    /// Injected (XTEST) events in the whole seconds `[start, end)`.
    pub injected: u64,
    /// The listener was delivering input for the whole window. When false
    /// the block's counts are "not measured" rather than a misleading 0.
    pub healthy: bool,
}

/// Ring of [`RING_SECONDS`] per-second input buckets plus the listener's
/// health history.
pub struct InputLog {
    inner: Mutex<Inner>,
    /// Last second marked active by pointer motion — lets the hot path skip
    /// the lock for the many motion events landing in the same second.
    last_motion_second: AtomicI64,
}

struct Inner {
    ring: Vec<Bucket>,
    /// The newest second written; older seconds than `newest - RETENTION_SECS`
    /// are gone.
    newest: i64,
    /// The latest second with physical input, 0 before any.
    last_physical: i64,
    health: Health,
}

/// When the listener delivered input. Starts "never measured".
#[derive(Debug)]
struct Health {
    /// Since when input has not been measured (Unix s); `None` while healthy.
    /// `i64::MIN` means "never measured yet".
    unhealthy_since: Option<i64>,
    /// Ended outages `[from, to)` in Unix seconds, oldest first.
    outages: Vec<(i64, i64)>,
}

impl Default for Health {
    fn default() -> Self {
        Self {
            unhealthy_since: Some(i64::MIN),
            outages: Vec::new(),
        }
    }
}

impl Health {
    fn mark_healthy(&mut self, at: i64) {
        if let Some(from) = self.unhealthy_since.take() {
            self.outages.push((from, at));
            if self.outages.len() > MAX_OUTAGES {
                // Merge the two oldest: the span between them is counted as
                // unmeasured too, which can only make a block "not measured".
                let (oldest_from, _) = self.outages.remove(0);
                self.outages[0].0 = oldest_from;
            }
        }
    }

    fn mark_unhealthy(&mut self, at: i64) {
        if self.unhealthy_since.is_none() {
            self.unhealthy_since = Some(at);
        }
    }

    /// Was input measured for all of `[start, end)`?
    fn available_between(&self, start: i64, end: i64) -> bool {
        if let Some(since) = self.unhealthy_since {
            if since < end {
                return false;
            }
        }
        !self
            .outages
            .iter()
            .any(|&(from, to)| from < end && to > start)
    }
}

impl Default for InputLog {
    fn default() -> Self {
        Self::new()
    }
}

impl InputLog {
    /// An empty log, "never measured" until [`InputLog::mark_healthy`].
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(Inner {
                ring: vec![Bucket::default(); RING_SECONDS],
                newest: i64::MIN,
                last_physical: 0,
                health: Health::default(),
            }),
            last_motion_second: AtomicI64::new(i64::MIN),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// One key press in `second`. A physical one counts as a key and marks the
    /// second active; an injected one only counts as injected.
    pub fn record_key(&self, second: i64, injected: bool) {
        self.with_bucket(second, |bucket| {
            if injected {
                bucket.injected = bucket.injected.saturating_add(1);
            } else {
                bucket.keys = bucket.keys.saturating_add(1);
                bucket.active = true;
            }
        });
    }

    /// One click in `second` (buttons 1-3 and 8-9), as [`InputLog::record_key`].
    pub fn record_click(&self, second: i64, injected: bool) {
        self.with_bucket(second, |bucket| {
            if injected {
                bucket.injected = bucket.injected.saturating_add(1);
            } else {
                bucket.clicks = bucket.clicks.saturating_add(1);
                bucket.active = true;
            }
        });
    }

    /// Physical pointer motion or wheel in `second`: activity, not a click.
    pub fn record_activity(&self, second: i64) {
        if self.last_motion_second.swap(second, Ordering::Relaxed) == second {
            return;
        }
        self.with_bucket(second, |bucket| bucket.active = true);
    }

    /// An injected (XTEST) motion or wheel event in `second`.
    pub fn record_injected(&self, second: i64) {
        self.with_bucket(second, |bucket| {
            bucket.injected = bucket.injected.saturating_add(1)
        });
    }

    fn with_bucket(&self, second: i64, apply: impl FnOnce(&mut Bucket)) {
        let mut inner = self.lock();
        if inner.newest != i64::MIN && second <= inner.newest - RETENTION_SECS {
            // Older than anything remembered (the clock stepped far back).
            return;
        }
        let slot = second.rem_euclid(RING_SECONDS as i64) as usize;
        let bucket = &mut inner.ring[slot];
        if bucket.second != second {
            if bucket.second > second && bucket.second != 0 {
                // The slot already holds a newer second: this one is gone.
                return;
            }
            *bucket = Bucket {
                second,
                ..Bucket::default()
            };
        }
        apply(bucket);
        let physical = bucket.active;
        inner.newest = inner.newest.max(second);
        if physical {
            inner.last_physical = inner.last_physical.max(second);
        }
    }

    /// When the user last gave physical input (to the second); `None` before
    /// any. Injected input never moves it, so idle detection can rely on it
    /// while the listener is healthy.
    pub fn last_physical_input(&self) -> Option<DateTime<Utc>> {
        let second = self.lock().last_physical;
        (second > 0)
            .then(|| DateTime::from_timestamp(second, 0))
            .flatten()
    }

    /// The listener delivers input from `at` on.
    pub fn mark_healthy(&self, at: DateTime<Utc>) {
        self.lock().health.mark_healthy(at.timestamp());
    }

    /// Input is not measured from `at` on (listener stopped or lost its
    /// display connection).
    pub fn mark_unhealthy(&self, at: DateTime<Utc>) {
        self.lock().health.mark_unhealthy(at.timestamp());
    }

    /// Is input being measured right now?
    pub fn is_healthy(&self) -> bool {
        self.lock().health.unhealthy_since.is_none()
    }

    /// Was input measured for the whole of `[start, end)`?
    pub fn healthy_between(&self, start: DateTime<Utc>, end: DateTime<Utc>) -> bool {
        self.lock()
            .health
            .available_between(start.timestamp(), end.timestamp())
    }

    /// Forget every count (consent revoked); health is kept.
    pub fn clear(&self) {
        let mut inner = self.lock();
        inner.ring.iter_mut().for_each(|b| *b = Bucket::default());
        inner.newest = i64::MIN;
        inner.last_physical = 0;
        self.last_motion_second.store(i64::MIN, Ordering::Relaxed);
    }

    /// (key presses, clicks) in `[start, end)` — cheap enough for a live view.
    pub fn counts_between(&self, start: DateTime<Utc>, end: DateTime<Utc>) -> (u64, u64) {
        let m = self.measurements(start, end);
        (m.key_presses_ms.len() as u64, m.clicks_ms.len() as u64)
    }

    /// Everything a block needs about its window `[start, end]`.
    pub fn measurements(&self, start: DateTime<Utc>, end: DateTime<Utc>) -> InputMeasurements {
        let (start_ms, end_ms) = (start.timestamp_millis(), end.timestamp_millis());
        let (start_s, end_s) = (start.timestamp(), end.timestamp());
        let inner = self.lock();
        let healthy = inner.health.available_between(start_s, end_s);
        let mut out = InputMeasurements {
            healthy,
            ..InputMeasurements::default()
        };
        if inner.newest == i64::MIN || end_s < start_s {
            return out;
        }
        // Only seconds still in the ring can hold input.
        let first = start_s.max(inner.newest - RETENTION_SECS + 1);
        let last = end_s.min(inner.newest);
        let mut second = first;
        while second <= last {
            let bucket = inner.ring[second.rem_euclid(RING_SECONDS as i64) as usize];
            if bucket.second == second {
                let at_ms = second * 1000;
                if at_ms >= start_ms && at_ms < end_ms {
                    out.key_presses_ms
                        .extend(std::iter::repeat_n(at_ms, usize::from(bucket.keys)));
                    out.clicks_ms
                        .extend(std::iter::repeat_n(at_ms, usize::from(bucket.clicks)));
                }
                if bucket.active {
                    out.active_seconds.push(second);
                }
                if second < end_s {
                    out.injected += u64::from(bucket.injected);
                }
            }
            second += 1;
        }
        out
    }
}

/// Activity % (0-100) of the block `start ..= end` (Unix seconds): the share
/// of its minutes with any input.
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

/// Length in seconds of bucket `index` of the block `start ..= end`: 60,
/// except the last, which runs to the end (30..=89 s, or the whole block when
/// it is shorter than that).
pub fn bucket_seconds(index: usize, start: i64, end: i64) -> u32 {
    let minutes = minute_buckets(start, end);
    if minutes == 0 || index as i64 >= minutes {
        return 0;
    }
    if (index as i64) < minutes - 1 {
        60
    } else {
        (end - start - 60 * (minutes - 1)).max(0) as u32
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    const T0: i64 = 1_790_000_000; // arbitrary Unix second
    const T0_MS: i64 = T0 * 1000;

    fn at(second: i64) -> DateTime<Utc> {
        DateTime::from_timestamp(second, 0).unwrap()
    }

    fn healthy_log(from: i64) -> InputLog {
        let log = InputLog::new();
        log.mark_healthy(at(from));
        log
    }

    // ---- Ported Windows tests (pure functions) ----

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
    fn bucket_lengths_add_up_to_the_block() {
        // 10 min + 29 s: ten buckets, the last one 89 s.
        let (start, end) = (T0, T0 + 629);
        let lengths: Vec<u32> = (0..10).map(|i| bucket_seconds(i, start, end)).collect();
        assert_eq!(&lengths[..9], &[60; 9]);
        assert_eq!(lengths[9], 89);
        assert_eq!(lengths.iter().sum::<u32>(), 629);
        // 10 min - 30 s rounds to ten buckets too; the last one is 30 s.
        assert_eq!(bucket_seconds(9, T0, T0 + 570), 30);
        // A 20-second block is one 20-second bucket; nothing past the end.
        assert_eq!(bucket_seconds(0, T0, T0 + 20), 20);
        assert_eq!(bucket_seconds(1, T0, T0 + 20), 0);
        assert_eq!(bucket_seconds(0, T0, T0), 0);
    }

    #[test]
    fn active_minutes_match_the_activity_percent() {
        // 3 buckets; input in the first and the trailing sliver of the last.
        let active = active_minutes(&[T0 + 5, T0 + 185], T0, T0 + 190);
        assert_eq!(active, vec![true, false, true]);
        assert_eq!(activity_percent(&[T0 + 5, T0 + 185], T0, T0 + 190), 67);
        assert!(active_minutes(&[T0], T0, T0).is_empty());
    }

    // ---- InputLog ----

    #[test]
    fn presses_are_second_quantized_and_windows_are_half_open() {
        let log = healthy_log(T0);
        log.record_key(T0 + 1, false);
        log.record_key(T0 + 1, false);
        log.record_click(T0 + 2, false);
        log.record_key(T0 + 3, false);

        // Back-to-back blocks [T0, T0+2) and [T0+2, T0+10): each event once.
        let first = log.measurements(at(T0), at(T0 + 2));
        assert_eq!(first.key_presses_ms, vec![T0_MS + 1000, T0_MS + 1000]);
        assert!(first.clicks_ms.is_empty());
        let second = log.measurements(at(T0 + 2), at(T0 + 10));
        assert_eq!(second.key_presses_ms, vec![T0_MS + 3000]);
        assert_eq!(second.clicks_ms, vec![T0_MS + 2000]);
        // Every timestamp is a whole second: no sub-second timing exists.
        assert!(first
            .key_presses_ms
            .iter()
            .chain(&second.key_presses_ms)
            .chain(&second.clicks_ms)
            .all(|ms| ms % 1000 == 0));
        assert_eq!(log.counts_between(at(T0), at(T0 + 10)), (3, 1));
    }

    #[test]
    fn active_seconds_are_inclusive_like_windows() {
        let log = healthy_log(T0);
        log.record_activity(T0);
        log.record_activity(T0); // same second: one entry
        log.record_click(T0 + 60, false);
        log.record_activity(T0 + 120);
        let m = log.measurements(at(T0), at(T0 + 120));
        assert_eq!(m.active_seconds, vec![T0, T0 + 60, T0 + 120]);
        assert_eq!(activity_percent(&m.active_seconds, T0, T0 + 120), 100);
    }

    #[test]
    fn injected_input_is_counted_aside_and_never_as_input() {
        let log = healthy_log(T0);
        log.record_key(T0 + 1, true);
        log.record_click(T0 + 1, true);
        log.record_injected(T0 + 2);
        let m = log.measurements(at(T0), at(T0 + 10));
        assert!(m.key_presses_ms.is_empty());
        assert!(m.clicks_ms.is_empty());
        assert!(m.active_seconds.is_empty());
        assert_eq!(m.injected, 3);
        assert_eq!(log.last_physical_input(), None);
    }

    #[test]
    fn the_last_physical_input_ignores_injected_events() {
        let log = healthy_log(T0);
        log.record_activity(T0 + 5);
        log.record_key(T0 + 30, true);
        assert_eq!(log.last_physical_input(), Some(at(T0 + 5)));
        log.record_click(T0 + 40, false);
        assert_eq!(log.last_physical_input(), Some(at(T0 + 40)));
    }

    #[test]
    fn a_new_log_is_never_measured_until_the_listener_runs() {
        let log = InputLog::new();
        assert!(!log.is_healthy());
        assert!(!log.measurements(at(T0), at(T0 + 60)).healthy);
        log.mark_healthy(at(T0 + 10));
        // A block that started before the listener ran is not measured.
        assert!(!log.healthy_between(at(T0), at(T0 + 60)));
        assert!(log.healthy_between(at(T0 + 10), at(T0 + 60)));
    }

    #[test]
    fn blocks_overlapping_an_outage_are_not_measured() {
        let log = healthy_log(T0);
        log.mark_unhealthy(at(T0 + 100));
        assert!(!log.is_healthy());
        assert!(!log.healthy_between(at(T0 + 50), at(T0 + 150)));
        assert!(log.healthy_between(at(T0), at(T0 + 100)));
        log.mark_healthy(at(T0 + 200));
        assert!(log.is_healthy());
        assert!(!log.healthy_between(at(T0 + 150), at(T0 + 250)));
        assert!(log.healthy_between(at(T0 + 200), at(T0 + 300)));
        // A second unhealthy mark keeps the earlier start of an outage.
        log.mark_unhealthy(at(T0 + 400));
        log.mark_unhealthy(at(T0 + 500));
        assert!(!log.healthy_between(at(T0 + 450), at(T0 + 460)));
    }

    #[test]
    fn the_outage_history_is_bounded_and_stays_conservative() {
        let log = healthy_log(T0);
        for i in 0..(MAX_OUTAGES as i64 + 10) {
            log.mark_unhealthy(at(T0 + 100 * i + 10));
            log.mark_healthy(at(T0 + 100 * i + 20));
        }
        assert!(log.lock().health.outages.len() <= MAX_OUTAGES);
        // Merging never turns an unmeasured span into a measured one.
        assert!(!log.healthy_between(at(T0 - 10), at(T0 - 5)));
        assert!(!log.healthy_between(at(T0 + 12), at(T0 + 15)));
        let last = T0 + 100 * (MAX_OUTAGES as i64 + 9);
        assert!(log.healthy_between(at(last + 30), at(last + 60)));
    }

    #[test]
    fn counts_saturate_instead_of_wrapping() {
        let log = healthy_log(T0);
        for _ in 0..70_000 {
            log.record_key(T0, false);
        }
        let m = log.measurements(at(T0), at(T0 + 1));
        assert_eq!(m.key_presses_ms.len(), usize::from(u16::MAX));
    }

    #[test]
    fn old_seconds_fall_out_of_the_ring() {
        let log = healthy_log(T0);
        log.record_key(T0, false);
        log.record_key(T0 + RETENTION_SECS + 10, false);
        let all = log.measurements(at(T0 - 1), at(T0 + 2 * RETENTION_SECS));
        assert_eq!(all.key_presses_ms, vec![(T0 + RETENTION_SECS + 10) * 1000]);
        // A clock stepping back past the retention drops the event.
        log.record_key(T0 + 5, false);
        let again = log.measurements(at(T0 - 1), at(T0 + 2 * RETENTION_SECS));
        assert_eq!(again.key_presses_ms.len(), 1);
    }

    #[test]
    fn clear_forgets_every_count() {
        let log = healthy_log(T0);
        log.record_key(T0, false);
        log.record_activity(T0 + 1);
        log.clear();
        let m = log.measurements(at(T0 - 10), at(T0 + 10));
        assert!(m.key_presses_ms.is_empty() && m.active_seconds.is_empty());
        assert_eq!(log.last_physical_input(), None);
        // Motion in the same second as before the clear is recorded again.
        log.record_activity(T0 + 1);
        assert_eq!(
            log.measurements(at(T0), at(T0 + 10)).active_seconds,
            vec![T0 + 1]
        );
    }

    #[test]
    fn the_ring_has_a_fixed_size() {
        let log = healthy_log(T0);
        for s in 0..(3 * RING_SECONDS as i64) {
            log.record_activity(T0 + s);
        }
        assert_eq!(log.lock().ring.len(), RING_SECONDS);
        let m = log.measurements(at(T0), at(T0 + 3 * RING_SECONDS as i64));
        assert!(m.active_seconds.len() <= RING_SECONDS);
    }

    #[derive(Debug, Clone, Copy)]
    enum Op {
        Key(i64, bool),
        Click(i64, bool),
        Motion(i64),
        Injected(i64),
    }

    fn op() -> impl Strategy<Value = Op> {
        let second = 0i64..600;
        prop_oneof![
            (second.clone(), any::<bool>()).prop_map(|(s, i)| Op::Key(s, i)),
            (second.clone(), any::<bool>()).prop_map(|(s, i)| Op::Click(s, i)),
            second.clone().prop_map(Op::Motion),
            second.prop_map(Op::Injected),
        ]
    }

    proptest! {
        /// Splitting a window anywhere never loses or duplicates an event, and
        /// the totals equal what was recorded.
        #[test]
        fn adjacent_windows_partition_every_event(ops in proptest::collection::vec(op(), 0..400), cut in 0i64..600) {
            let log = healthy_log(T0);
            let (mut keys, mut clicks, mut injected) = (0usize, 0usize, 0u64);
            for op in &ops {
                match *op {
                    Op::Key(s, i) => { log.record_key(T0 + s, i); if i { injected += 1 } else { keys += 1 } }
                    Op::Click(s, i) => { log.record_click(T0 + s, i); if i { injected += 1 } else { clicks += 1 } }
                    Op::Motion(s) => log.record_activity(T0 + s),
                    Op::Injected(s) => { log.record_injected(T0 + s); injected += 1 }
                }
            }
            let whole = log.measurements(at(T0), at(T0 + 600));
            let left = log.measurements(at(T0), at(T0 + cut));
            let right = log.measurements(at(T0 + cut), at(T0 + 600));
            prop_assert_eq!(whole.key_presses_ms.len(), keys);
            prop_assert_eq!(whole.clicks_ms.len(), clicks);
            prop_assert_eq!(whole.injected, injected);
            prop_assert_eq!(left.key_presses_ms.len() + right.key_presses_ms.len(), keys);
            prop_assert_eq!(left.clicks_ms.len() + right.clicks_ms.len(), clicks);
            prop_assert_eq!(left.injected + right.injected, injected);
            // Sorted, whole-second, inside the window.
            prop_assert!(whole.key_presses_ms.windows(2).all(|w| w[0] <= w[1]));
            prop_assert!(whole.key_presses_ms.iter().all(|&ms| ms % 1000 == 0 && (T0_MS..T0_MS + 600_000).contains(&ms)));
            prop_assert!(whole.active_seconds.windows(2).all(|w| w[0] < w[1]));
            prop_assert!(whole.active_seconds.len() <= 601);
        }

        /// The activity % stays a percentage for any input.
        #[test]
        fn activity_percent_is_bounded(seconds in proptest::collection::vec(-100i64..800, 0..200), len in 0i64..700) {
            let pct = activity_percent(&seconds.iter().map(|s| T0 + s).collect::<Vec<_>>(), T0, T0 + len);
            prop_assert!(pct <= 100);
        }
    }
}
