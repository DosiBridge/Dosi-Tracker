//! Foreground-window usage: which app and window were in front, and for how
//! long — the per-app breakdown and per-minute timeline of each block.
//!
//! A background thread samples the foreground window every
//! [`SAMPLE_EVERY`]; each sample credits the time since the previous one to
//! the window in front now. Consecutive samples of the same window are merged,
//! so memory grows with window switches, not with time.

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::model::{ActivityMinute, WindowInfo};

use super::input::{active_minutes, bucket_of, minute_buckets};

/// How often the foreground window is sampled. The query is a couple of
/// Win32 calls, so this costs nothing measurable while giving per-app times
/// accurate to a few seconds.
pub const SAMPLE_EVERY: Duration = Duration::from_secs(3);

/// A gap between samples longer than this means the machine slept or the
/// sampler was starved; that stretch is not credited to any window.
const MAX_SAMPLE_GAP_MS: i64 = 30_000;

/// How long usage is remembered — the longest block plus slack, as for input.
const RETENTION_MS: i64 = 65 * 60 * 1000;

/// Upper bound on the (app, window) entries sent per block.
pub const MAX_USAGE_ENTRIES: usize = 50;

/// A stretch of time one window spent in front, `[start_ms, end_ms)`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UsageSlice {
    pub app: String,
    pub title: String,
    pub start_ms: i64,
    pub end_ms: i64,
}

#[derive(Default)]
pub struct UsageRecorder {
    inner: Mutex<Inner>,
}

#[derive(Default)]
struct Inner {
    slices: VecDeque<UsageSlice>,
    last_sample_ms: Option<i64>,
}

impl UsageRecorder {
    pub fn new() -> Self {
        Self::default()
    }

    /// Credit the time since the previous sample to `window` (the one in front
    /// now). `None` — no foreground window, e.g. the secure desktop — credits
    /// nobody.
    pub fn record(&self, now_ms: i64, window: Option<WindowInfo>) {
        let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        let Some(last) = inner.last_sample_ms else {
            inner.last_sample_ms = Some(now_ms);
            return;
        };
        if now_ms <= last {
            // A racing sample already covered this instant (or the clock
            // stepped back): nothing to credit.
            return;
        }
        inner.last_sample_ms = Some(now_ms);
        if now_ms - last > MAX_SAMPLE_GAP_MS {
            return;
        }

        if let Some(window) = window.filter(|w| !w.app_name.is_empty()) {
            match inner.slices.back_mut() {
                Some(tail)
                    if tail.end_ms == last
                        && tail.app == window.app_name
                        && tail.title == window.window_title =>
                {
                    tail.end_ms = now_ms;
                }
                _ => inner.slices.push_back(UsageSlice {
                    app: window.app_name,
                    title: window.window_title,
                    start_ms: last,
                    end_ms: now_ms,
                }),
            }
        }

        let horizon = now_ms - RETENTION_MS;
        while inner.slices.front().is_some_and(|s| s.end_ms < horizon) {
            inner.slices.pop_front();
        }
    }

    /// The usage inside `[start_ms, end_ms)`, slices clipped to it.
    pub fn slices_between(&self, start_ms: i64, end_ms: i64) -> Vec<UsageSlice> {
        let inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        inner
            .slices
            .iter()
            .filter(|s| s.end_ms > start_ms && s.start_ms < end_ms)
            .map(|s| UsageSlice {
                start_ms: s.start_ms.max(start_ms),
                end_ms: s.end_ms.min(end_ms),
                ..s.clone()
            })
            .collect()
    }
}

/// Start the background sampler. It only reads the foreground window, so it
/// never needs to stop: while paused or signed out its samples simply fall
/// outside any recorded block.
pub fn spawn_sampler(recorder: Arc<UsageRecorder>) {
    let spawned = std::thread::Builder::new()
        .name("usage-sampler".into())
        .spawn(move || loop {
            recorder.record(chrono::Utc::now().timestamp_millis(), super::active_window::current());
            std::thread::sleep(SAMPLE_EVERY);
        });
    if let Err(e) = spawned {
        tracing::error!(?e, "could not start the usage sampler; per-app times will be missing");
    }
}

/// Per-(app, window) usage over the block, most-used first: foreground
/// seconds plus the key presses and clicks made while that window was in
/// front. Input outside every slice (gaps, no foreground window) is left
/// unattributed rather than guessed.
pub fn app_usage(slices: &[UsageSlice], key_presses: &[i64], clicks: &[i64]) -> Vec<WindowInfo> {
    #[derive(Default)]
    struct Totals {
        ms: i64,
        keys: u64,
        clicks: u64,
        first_ms: i64,
    }

    let mut by_window: HashMap<(&str, &str), Totals> = HashMap::new();
    for slice in slices {
        let totals = by_window
            .entry((slice.app.as_str(), slice.title.as_str()))
            .or_insert_with(|| Totals { first_ms: slice.start_ms, ..Default::default() });
        totals.ms += slice.end_ms - slice.start_ms;
        totals.keys += count_in(key_presses, slice.start_ms, slice.end_ms);
        totals.clicks += count_in(clicks, slice.start_ms, slice.end_ms);
    }

    let mut usage: Vec<(WindowInfo, i64, i64)> = by_window
        .into_iter()
        .map(|((app, title), t)| {
            let info = WindowInfo {
                app_name: app.to_string(),
                window_title: title.to_string(),
                seconds: Some(((t.ms + 500) / 1000) as u64),
                keyboard_hits: Some(t.keys),
                mouse_clicks: Some(t.clicks),
            };
            (info, t.ms, t.first_ms)
        })
        .collect();
    // Most time first; ties by first appearance so the order is stable.
    usage.sort_by(|a, b| b.1.cmp(&a.1).then(a.2.cmp(&b.2)));
    usage
        .into_iter()
        .map(|(info, _, _)| info)
        .filter(|w| w.seconds != Some(0) || w.keyboard_hits != Some(0) || w.mouse_clicks != Some(0))
        .take(MAX_USAGE_ENTRIES)
        .collect()
}

/// Per-minute timeline of the block `start ..= end` (Unix seconds), using the
/// same buckets as the activity %: input counts, whether the minute counts as
/// active, and the app in front the longest (when usage is available).
pub fn minute_timeline(
    start: i64,
    end: i64,
    slices: &[UsageSlice],
    key_presses: &[i64],
    clicks: &[i64],
    active_seconds: &[i64],
) -> Vec<ActivityMinute> {
    let minutes = minute_buckets(start, end);
    if minutes == 0 {
        return Vec::new();
    }
    let active = active_minutes(active_seconds, start, end);
    let mut timeline: Vec<ActivityMinute> = (0..minutes as usize)
        .map(|i| ActivityMinute {
            minute: i as u32,
            keyboard_hits: 0,
            mouse_clicks: 0,
            active: active[i],
            app_name: None,
        })
        .collect();

    for &ms in key_presses {
        timeline[bucket_of(ms.div_euclid(1000), start, minutes)].keyboard_hits += 1;
    }
    for &ms in clicks {
        timeline[bucket_of(ms.div_euclid(1000), start, minutes)].mouse_clicks += 1;
    }

    // Foreground milliseconds per app in each bucket; bucket i spans
    // [start + 60i, start + 60(i+1)) seconds, the last one running to the end.
    let mut per_bucket: Vec<HashMap<&str, i64>> = vec![HashMap::new(); minutes as usize];
    for slice in slices {
        let mut from = slice.start_ms;
        while from < slice.end_ms {
            let bucket = bucket_of(from.div_euclid(1000), start, minutes);
            let bucket_end = if bucket as i64 == minutes - 1 {
                i64::MAX
            } else {
                (start + 60 * (bucket as i64 + 1)) * 1000
            };
            let to = slice.end_ms.min(bucket_end);
            *per_bucket[bucket].entry(slice.app.as_str()).or_default() += to - from;
            from = to;
        }
    }
    for (minute, apps) in timeline.iter_mut().zip(per_bucket) {
        minute.app_name = apps
            .into_iter()
            // Most time wins; ties go to the alphabetically first app so the
            // result does not depend on hash order.
            .max_by(|a, b| a.1.cmp(&b.1).then(b.0.cmp(a.0)))
            .map(|(app, _)| app.to_string());
    }
    timeline
}

fn count_in(sorted_ms: &[i64], start_ms: i64, end_ms: i64) -> u64 {
    let from = sorted_ms.partition_point(|&t| t < start_ms);
    let to = sorted_ms.partition_point(|&t| t < end_ms).max(from);
    (to - from) as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    const T0: i64 = 1_790_000_000; // arbitrary Unix second
    const T0_MS: i64 = T0 * 1000;

    fn window(app: &str, title: &str) -> Option<WindowInfo> {
        Some(WindowInfo { app_name: app.into(), window_title: title.into(), ..Default::default() })
    }

    fn slice(app: &str, title: &str, from_s: i64, to_s: i64) -> UsageSlice {
        UsageSlice {
            app: app.into(),
            title: title.into(),
            start_ms: T0_MS + from_s * 1000,
            end_ms: T0_MS + to_s * 1000,
        }
    }

    #[test]
    fn samples_credit_the_window_in_front_and_merge_repeats() {
        let rec = UsageRecorder::new();
        rec.record(T0_MS, window("code.exe", "main.rs")); // first sample: nothing to credit yet
        rec.record(T0_MS + 3_000, window("code.exe", "main.rs"));
        rec.record(T0_MS + 6_000, window("code.exe", "main.rs"));
        rec.record(T0_MS + 9_000, window("msedge.exe", "Docs"));
        rec.record(T0_MS + 12_000, None); // secure desktop: not credited
        rec.record(T0_MS + 15_000, window("msedge.exe", "Docs"));

        assert_eq!(
            rec.slices_between(0, i64::MAX),
            vec![
                slice("code.exe", "main.rs", 0, 6),
                slice("msedge.exe", "Docs", 6, 9),
                slice("msedge.exe", "Docs", 12, 15),
            ]
        );
    }

    #[test]
    fn long_gaps_and_stale_samples_are_not_credited() {
        let rec = UsageRecorder::new();
        rec.record(T0_MS, window("code.exe", ""));
        rec.record(T0_MS + 120_000, window("code.exe", "")); // machine slept
        rec.record(T0_MS + 119_000, window("code.exe", "")); // racing, older sample
        rec.record(T0_MS + 123_000, window("code.exe", ""));
        assert_eq!(rec.slices_between(0, i64::MAX), vec![slice("code.exe", "", 120, 123)]);
    }

    #[test]
    fn slices_are_clipped_to_the_block() {
        let rec = UsageRecorder::new();
        rec.record(T0_MS, window("a.exe", ""));
        rec.record(T0_MS + 10_000, window("a.exe", ""));
        assert_eq!(
            rec.slices_between(T0_MS + 4_000, T0_MS + 7_000),
            vec![slice("a.exe", "", 4, 7)]
        );
        assert!(rec.slices_between(T0_MS + 10_000, T0_MS + 20_000).is_empty());
    }

    #[test]
    fn app_usage_groups_windows_and_attributes_input() {
        let slices = [
            slice("code.exe", "main.rs", 0, 100),
            slice("msedge.exe", "Docs", 100, 130),
            slice("code.exe", "main.rs", 130, 200),
            slice("code.exe", "lib.rs", 200, 210),
        ];
        let keys = [T0_MS + 5_000, T0_MS + 150_000, T0_MS + 205_000];
        let clicks = [T0_MS + 110_000, T0_MS + 111_000, T0_MS + 500_000]; // last: outside every slice

        let usage = app_usage(&slices, &keys, &clicks);
        let row = |app: &str, title: &str, s: u64, k: u64, c: u64| WindowInfo {
            app_name: app.into(),
            window_title: title.into(),
            seconds: Some(s),
            keyboard_hits: Some(k),
            mouse_clicks: Some(c),
        };
        assert_eq!(
            usage,
            vec![
                row("code.exe", "main.rs", 170, 2, 0),
                row("msedge.exe", "Docs", 30, 0, 2),
                row("code.exe", "lib.rs", 10, 1, 0),
            ]
        );
    }

    #[test]
    fn app_usage_is_capped() {
        let slices: Vec<UsageSlice> =
            (0..80).map(|i| slice("app.exe", &format!("tab {i}"), i, i + 1)).collect();
        assert_eq!(app_usage(&slices, &[], &[]).len(), MAX_USAGE_ENTRIES);
    }

    #[test]
    fn timeline_buckets_input_and_picks_the_main_app_per_minute() {
        // 3-minute block; code for 1.5 min, then edge.
        let slices = [slice("code.exe", "", 0, 90), slice("msedge.exe", "", 90, 180)];
        let keys = [T0_MS + 1_000, T0_MS + 2_000, T0_MS + 61_000];
        let clicks = [T0_MS + 179_500];
        let active = [T0 + 1, T0 + 61];

        let timeline = minute_timeline(T0, T0 + 180, &slices, &keys, &clicks, &active);
        let minute = |m: u32, k: u64, c: u64, active: bool, app: &str| ActivityMinute {
            minute: m,
            keyboard_hits: k,
            mouse_clicks: c,
            active,
            app_name: Some(app.into()),
        };
        assert_eq!(
            timeline,
            vec![
                minute(0, 2, 0, true, "code.exe"),
                // 30 s each: the tie goes to the alphabetically first app.
                minute(1, 1, 0, true, "code.exe"),
                minute(2, 0, 1, false, "msedge.exe"),
            ]
        );
    }

    #[test]
    fn timeline_folds_a_trailing_sliver_into_the_last_minute() {
        // 2 min + 10 s = 2 buckets; the sliver's input and time join minute 1.
        let slices = [slice("a.exe", "", 0, 60), slice("b.exe", "", 60, 130)];
        let timeline = minute_timeline(T0, T0 + 130, &slices, &[T0_MS + 125_000], &[], &[T0 + 125]);
        assert_eq!(timeline.len(), 2);
        assert_eq!(timeline[1].keyboard_hits, 1);
        assert!(timeline[1].active && !timeline[0].active);
        assert_eq!(timeline[1].app_name.as_deref(), Some("b.exe"));
    }

    #[test]
    fn timeline_without_usage_still_has_counts() {
        let timeline = minute_timeline(T0, T0 + 60, &[], &[T0_MS], &[], &[T0]);
        assert_eq!(timeline.len(), 1);
        assert_eq!(timeline[0].keyboard_hits, 1);
        assert_eq!(timeline[0].app_name, None);
        assert!(minute_timeline(T0, T0, &[], &[], &[], &[]).is_empty());
    }
}
