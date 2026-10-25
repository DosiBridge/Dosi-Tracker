//! Telling time spent asleep from time spent awake, ported from the Windows
//! agent's `power.rs` (with its tests).
//!
//! logind announces a suspend (`PrepareForSleep`, see
//! [`super::session_events`]), but not always in time to act on it — a dead
//! battery, a lid closed while the agent was busy, a missing inhibitor. So the
//! worker also compares two clocks on every tick (CONTRACT §4): the wall
//! clock, which runs on while the machine sleeps, and an awake-time clock,
//! which does not. When the wall clock got ahead by more than
//! [`MISSED_SUSPEND_GAP`] between two ticks, the machine slept in between, and
//! the block in progress ends at the tick before the gap.
//!
//! The awake clock is `std::time::Instant`: on Linux it is `CLOCK_MONOTONIC`,
//! which does not count time in suspend (unlike `CLOCK_BOOTTIME`).

use chrono::{DateTime, TimeDelta, Utc};

/// The wall clock must get ahead of the awake clock by more than this between
/// two ticks before the gap counts as a missed suspend: ticks come every few
/// seconds, and a stall of a few seconds is no sleep.
pub const MISSED_SUSPEND_GAP: TimeDelta = TimeDelta::seconds(90);

/// Milliseconds the machine has been awake since the first call: time in
/// suspend is not counted (`CLOCK_MONOTONIC`).
pub fn awake_millis() -> u64 {
    static START: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
    START
        .get_or_init(std::time::Instant::now)
        .elapsed()
        .as_millis() as u64
}

/// Spots a suspend nobody announced, from consecutive ticks.
#[derive(Debug, Default, Clone)]
pub struct SleepDetector {
    /// The previous tick: wall clock and awake milliseconds.
    last: Option<(DateTime<Utc>, u64)>,
}

impl SleepDetector {
    pub fn new() -> Self {
        Self::default()
    }

    /// Record a tick. Returns the wall-clock time of the previous tick when
    /// the machine slept since it — the moment the block in progress should
    /// have ended.
    ///
    /// A wall clock stepping backwards (time sync, the user setting it) is not
    /// a sleep.
    pub fn observe(&mut self, wall: DateTime<Utc>, awake_ms: u64) -> Option<DateTime<Utc>> {
        let previous = self.last.replace((wall, awake_ms));
        let (last_wall, last_awake) = previous?;
        let wall_elapsed = wall - last_wall;
        let awake_elapsed = TimeDelta::milliseconds(
            i64::try_from(awake_ms.saturating_sub(last_awake)).unwrap_or(i64::MAX),
        );
        (wall_elapsed - awake_elapsed > MISSED_SUSPEND_GAP).then_some(last_wall)
    }

    /// Forget the previous tick (a suspend that was announced and handled is
    /// not reported again).
    pub fn reset(&mut self) {
        self.last = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(s: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc)
    }

    #[test]
    fn regular_ticks_are_no_sleep() {
        let mut detector = SleepDetector::new();
        assert_eq!(
            detector.observe(at("2026-10-05T10:00:00Z"), 1_000_000),
            None,
            "the first tick has nothing to compare"
        );
        assert_eq!(
            detector.observe(at("2026-10-05T10:00:02Z"), 1_002_000),
            None
        );
        // A late tick (the worker was busy for a minute) is not a sleep.
        assert_eq!(
            detector.observe(at("2026-10-05T10:01:02Z"), 1_062_000),
            None
        );
    }

    #[test]
    fn a_wall_clock_running_ahead_of_awake_time_is_a_missed_suspend() {
        let mut detector = SleepDetector::new();
        detector.observe(at("2026-10-05T12:02:00Z"), 5_000_000);
        // Three hours later by the wall clock, two seconds of awake time.
        let before_gap = detector.observe(at("2026-10-05T15:00:02Z"), 5_002_000);
        assert_eq!(
            before_gap,
            Some(at("2026-10-05T12:02:00Z")),
            "the block ends at the tick before the gap"
        );
        // Ticking on normally afterwards.
        assert_eq!(
            detector.observe(at("2026-10-05T15:00:04Z"), 5_004_000),
            None
        );
    }

    #[test]
    fn only_a_gap_over_ninety_seconds_counts() {
        let mut detector = SleepDetector::new();
        detector.observe(at("2026-10-05T10:00:00Z"), 0);
        assert_eq!(
            detector.observe(at("2026-10-05T10:01:32Z"), 2_000),
            None,
            "exactly 90 s ahead"
        );
        detector.observe(at("2026-10-05T10:02:00Z"), 30_000);
        assert!(
            detector
                .observe(at("2026-10-05T10:03:33Z"), 32_000)
                .is_some(),
            "91 s ahead"
        );
    }

    #[test]
    fn a_clock_set_back_is_not_a_sleep_and_reset_forgets_the_last_tick() {
        let mut detector = SleepDetector::new();
        detector.observe(at("2026-10-05T10:00:00Z"), 100_000);
        assert_eq!(detector.observe(at("2026-10-05T09:00:00Z"), 102_000), None);
        detector.reset();
        assert_eq!(
            detector.observe(at("2026-10-05T18:00:00Z"), 104_000),
            None,
            "no previous tick after a reset"
        );
    }

    #[test]
    fn the_awake_clock_moves_forward() {
        let first = awake_millis();
        std::thread::sleep(std::time::Duration::from_millis(60));
        let second = awake_millis();
        assert!(second >= first + 50, "{first} -> {second}");
    }
}
