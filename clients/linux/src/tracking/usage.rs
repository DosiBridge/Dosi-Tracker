//! Per-app usage recorder: 3 s foreground slices folded into active windows
//! and the timeline's app names (ported from Windows).
//!
//! A sampler thread ([`UsageSampler`], `dosi-usage`) reads the active window
//! every [`SAMPLE_EVERY`] through an [`ActiveWindowProvider`] — in production
//! the consent-gated tracker, so a revoked capability yields no window — and
//! each sample credits the time since the previous one to the window in front
//! now. Consecutive samples of the same window are merged, so memory grows
//! with window switches, not with time.

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Condvar, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

use super::input::{active_minutes, bucket_of, bucket_seconds, minute_buckets, RETENTION_SECS};
use super::model::WindowInfo;
use super::traits::{ActiveWindowProvider, TrackingError};
use crate::agent::model::ActivityMinute;

/// How often the foreground window is sampled: a few X11 round trips, so this
/// costs nothing measurable while giving per-app times accurate to a few
/// seconds.
pub const SAMPLE_EVERY: Duration = Duration::from_secs(3);

/// A gap between samples longer than this means the machine slept or the
/// sampler was starved; that stretch is not credited to any window.
const MAX_SAMPLE_GAP_MS: i64 = 30_000;

/// How long usage is remembered — as long as input (see [`RETENTION_SECS`]).
const RETENTION_MS: i64 = RETENTION_SECS * 1000;

/// The server's cap on the (app, window) entries of one block.
pub const MAX_USAGE_ENTRIES: usize = 200;

/// Title of the row that sums an app's windows beyond the cap.
pub const OTHER_WINDOWS_TITLE: &str = "Other windows";

/// App name of the row that sums the least-used apps when even one row per
/// app would exceed the cap.
pub const OTHER_APPS_NAME: &str = "Other apps";

/// How long [`UsageSampler::stop`] waits for the sampler thread.
pub const STOP_TIMEOUT: Duration = Duration::from_secs(1);

/// A stretch of time one window spent in front, `[start_ms, end_ms)`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UsageSlice {
    pub app: String,
    pub title: String,
    pub start_ms: i64,
    pub end_ms: i64,
}

pub struct UsageRecorder {
    inner: Mutex<Inner>,
    active: AtomicBool,
}

impl Default for UsageRecorder {
    fn default() -> Self {
        Self {
            inner: Mutex::new(Inner::default()),
            active: AtomicBool::new(false),
        }
    }
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

    /// Turn active tracking on or off. When inactive (e.g. paused, signed out,
    /// away, or before project selection), foreground window samples are dropped
    /// without recording window titles or application names into memory buffers.
    pub fn set_active(&self, active: bool) {
        self.active.store(active, Ordering::SeqCst);
        if !active {
            // Discard any partial timing anchor so inactive time cannot be
            // credited to the next sample when tracking resumes.
            let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
            inner.last_sample_ms = None;
        }
    }

    pub fn is_active(&self) -> bool {
        self.active.load(Ordering::SeqCst)
    }

    /// Forget every slice (consent for active windows revoked).
    pub fn clear(&self) {
        let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        inner.slices.clear();
        inner.last_sample_ms = None;
    }

    /// Credit the time since the previous sample to `window` (the one in front
    /// now). `None` — no foreground window — credits nobody. When tracking is
    /// inactive, samples are dropped immediately.
    pub fn record(&self, now_ms: i64, window: Option<WindowInfo>) {
        if !self.is_active() {
            return;
        }
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

/// The `dosi-usage` thread: samples the active window into a
/// [`UsageRecorder`] while the recorder is active. Started and stopped
/// synchronously by the tracker; [`UsageSampler::stop`] returns once the
/// thread is gone (at most [`STOP_TIMEOUT`]).
pub struct UsageSampler {
    signal: Arc<(Mutex<bool>, Condvar)>,
    done: mpsc::Receiver<()>,
    thread: Option<JoinHandle<()>>,
}

impl UsageSampler {
    /// Start sampling `provider` into `recorder` every `every`. Only reads the
    /// active window while the recorder is active.
    pub fn start(
        recorder: Arc<UsageRecorder>,
        provider: Arc<dyn ActiveWindowProvider>,
        every: Duration,
    ) -> Result<Self, TrackingError> {
        let signal = Arc::new((Mutex::new(false), Condvar::new()));
        let (done_tx, done) = mpsc::channel();
        let thread_signal = signal.clone();
        let thread = std::thread::Builder::new()
            .name("dosi-usage".into())
            .spawn(move || {
                let (stopped, wake) = &*thread_signal;
                loop {
                    if *stopped.lock().unwrap_or_else(|e| e.into_inner()) {
                        break;
                    }
                    if recorder.is_active() {
                        // An error (consent revoked mid-sample, display lost)
                        // credits nobody rather than guessing a window.
                        let window = provider.current_active_window().ok().flatten();
                        recorder.record(chrono::Utc::now().timestamp_millis(), window);
                    }
                    let guard = stopped.lock().unwrap_or_else(|e| e.into_inner());
                    let (guard, _) = wake
                        .wait_timeout_while(guard, every, |stopped| !*stopped)
                        .unwrap_or_else(|e| e.into_inner());
                    if *guard {
                        break;
                    }
                }
                let _ = done_tx.send(());
            })
            .map_err(|e| {
                TrackingError::IoError(format!("could not start the usage sampler: {e}"))
            })?;
        Ok(Self {
            signal,
            done,
            thread: Some(thread),
        })
    }

    /// Stop the sampler and wait for its thread (at most [`STOP_TIMEOUT`]).
    pub fn stop(mut self) -> Result<(), TrackingError> {
        self.shutdown()
    }

    fn shutdown(&mut self) -> Result<(), TrackingError> {
        let Some(thread) = self.thread.take() else {
            return Ok(());
        };
        {
            let (stopped, wake) = &*self.signal;
            *stopped.lock().unwrap_or_else(|e| e.into_inner()) = true;
            wake.notify_all();
        }
        match self.done.recv_timeout(STOP_TIMEOUT) {
            Ok(()) | Err(mpsc::RecvTimeoutError::Disconnected) => {
                let _ = thread.join();
                Ok(())
            }
            Err(mpsc::RecvTimeoutError::Timeout) => Err(TrackingError::IoError(
                "the usage sampler did not stop within 1 s".into(),
            )),
        }
    }
}

impl Drop for UsageSampler {
    fn drop(&mut self) {
        let _ = self.shutdown();
    }
}

/// Foreground time per application (window titles merged, names compared
/// case-insensitively), most-used first, at most `limit` apps — for the
/// "top apps" list.
pub fn top_apps(slices: &[UsageSlice], limit: usize) -> Vec<(String, i64)> {
    let mut totals: Vec<(String, i64)> = Vec::new();
    for slice in slices {
        let ms = slice.end_ms - slice.start_ms;
        if ms <= 0 {
            continue;
        }
        match totals
            .iter_mut()
            .find(|(app, _)| app.eq_ignore_ascii_case(&slice.app))
        {
            Some((_, total)) => *total += ms,
            None => totals.push((slice.app.clone(), ms)),
        }
    }
    // Stable sort: ties keep first-seen order.
    totals.sort_by_key(|&(_, ms)| std::cmp::Reverse(ms));
    totals.truncate(limit);
    totals
}

/// Per-(app, window) usage over the block, most-used first: foreground
/// seconds plus the key presses and clicks made while that window was in
/// front. Input outside every slice (gaps, no foreground window) is left
/// unattributed rather than guessed. Beyond the server's cap the least-used
/// windows are folded into one "Other windows" row per app, so per-app
/// totals stay complete.
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
            .or_insert_with(|| Totals {
                first_ms: slice.start_ms,
                ..Default::default()
            });
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
    let rows: Vec<WindowInfo> = usage
        .into_iter()
        .map(|(info, _, _)| info)
        .filter(|w| w.seconds != Some(0) || w.keyboard_hits != Some(0) || w.mouse_clicks != Some(0))
        .collect();
    fold_beyond_cap(rows, MAX_USAGE_ENTRIES)
}

/// Keep `rows` (most-used first) within `cap` without losing time: the most
/// used rows stay as they are and the rest are summed into one
/// [`OTHER_WINDOWS_TITLE`] row per app, keeping as many rows as fit. Should
/// even one row per app not fit, the least-used apps share one
/// [`OTHER_APPS_NAME`] row. The result is sorted most-used first again.
pub fn fold_beyond_cap(rows: Vec<WindowInfo>, cap: usize) -> Vec<WindowInfo> {
    if rows.len() <= cap || cap == 0 {
        return if cap == 0 { Vec::new() } else { rows };
    }
    // Keep the first `kept` rows, with `kept` as large as possible while the
    // kept rows plus one folded row per app of the rest fit.
    let mut tail_apps: HashSet<&str> = HashSet::new();
    let mut kept = rows.len();
    while kept > 0 && kept + tail_apps.len() > cap {
        kept -= 1;
        tail_apps.insert(rows[kept].app_name.as_str());
    }

    let mut out: Vec<WindowInfo> = rows[..kept].to_vec();
    // The rest, grouped by app in order of first appearance.
    let mut groups: Vec<Vec<&WindowInfo>> = Vec::new();
    for row in &rows[kept..] {
        match groups
            .iter_mut()
            .find(|group| group[0].app_name == row.app_name)
        {
            Some(group) => group.push(row),
            None => groups.push(vec![row]),
        }
    }
    for group in groups {
        if let [only] = group.as_slice() {
            // One window takes one row either way: keep its title.
            out.push((*only).clone());
            continue;
        }
        let mut folded = WindowInfo {
            app_name: group[0].app_name.clone(),
            window_title: OTHER_WINDOWS_TITLE.to_string(),
            seconds: Some(0),
            keyboard_hits: Some(0),
            mouse_clicks: Some(0),
        };
        for row in group {
            add_usage(&mut folded, row);
        }
        out.push(folded);
    }
    sort_most_used_first(&mut out);

    if out.len() > cap {
        // More apps than the cap: the least-used ones share a row.
        let rest = out.split_off(cap - 1);
        let mut others = WindowInfo {
            app_name: OTHER_APPS_NAME.to_string(),
            window_title: String::new(),
            seconds: Some(0),
            keyboard_hits: Some(0),
            mouse_clicks: Some(0),
        };
        for row in &rest {
            add_usage(&mut others, row);
        }
        out.push(others);
        sort_most_used_first(&mut out);
    }
    out
}

fn add_usage(total: &mut WindowInfo, row: &WindowInfo) {
    let add = |a: Option<u64>, b: Option<u64>| Some(a.unwrap_or(0) + b.unwrap_or(0));
    total.seconds = add(total.seconds, row.seconds);
    total.keyboard_hits = add(total.keyboard_hits, row.keyboard_hits);
    total.mouse_clicks = add(total.mouse_clicks, row.mouse_clicks);
}

/// Stable: ties keep their order.
fn sort_most_used_first(rows: &mut [WindowInfo]) {
    rows.sort_by_key(|row| std::cmp::Reverse(row.seconds.unwrap_or(0)));
}

/// Seconds of the block `start ..= end` without input, at minute granularity
/// (CONTRACT §1): the lengths of the timeline's inactive buckets.
pub fn idle_seconds(timeline: &[ActivityMinute], start: i64, end: i64) -> u32 {
    timeline
        .iter()
        .filter(|minute| !minute.active)
        .map(|minute| bucket_seconds(minute.minute as usize, start, end))
        .sum()
}

/// Per-minute timeline of the block `start ..= end` (Unix seconds), using the
/// same buckets as the activity %: input counts, whether the minute counts as
/// active and for how many seconds, and the app in front the longest (when
/// usage is available).
///
/// Linux delta: a kind of input that was not measured (`None`) leaves that
/// count `null` in every minute instead of a fabricated 0.
pub fn minute_timeline(
    start: i64,
    end: i64,
    slices: &[UsageSlice],
    key_presses: Option<&[i64]>,
    clicks: Option<&[i64]>,
    active_seconds: &[i64],
) -> Vec<ActivityMinute> {
    let minutes = minute_buckets(start, end);
    if minutes == 0 {
        return Vec::new();
    }
    let active = active_minutes(active_seconds, start, end);

    // Distinct active seconds per bucket (a second with both keyboard and
    // mouse input is listed twice), never more than the bucket is long.
    let mut distinct: Vec<i64> = active_seconds
        .iter()
        .copied()
        .filter(|&s| s >= start && s <= end)
        .collect();
    distinct.sort_unstable();
    distinct.dedup();
    let mut seconds_per_bucket = vec![0u32; minutes as usize];
    for second in distinct {
        seconds_per_bucket[bucket_of(second, start, minutes)] += 1;
    }

    let mut timeline: Vec<ActivityMinute> = (0..minutes as usize)
        .map(|i| ActivityMinute {
            minute: i as u32,
            keyboard_hits: key_presses.map(|_| 0),
            mouse_clicks: clicks.map(|_| 0),
            active: active[i],
            app_name: None,
            active_seconds: Some(seconds_per_bucket[i].min(bucket_seconds(i, start, end))),
        })
        .collect();

    for &ms in key_presses.unwrap_or_default() {
        if let Some(hits) = timeline[bucket_of(ms.div_euclid(1000), start, minutes)]
            .keyboard_hits
            .as_mut()
        {
            *hits += 1;
        }
    }
    for &ms in clicks.unwrap_or_default() {
        if let Some(count) = timeline[bucket_of(ms.div_euclid(1000), start, minutes)]
            .mouse_clicks
            .as_mut()
        {
            *count += 1;
        }
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
    use std::sync::atomic::AtomicU32;
    use std::time::Instant;

    const T0: i64 = 1_790_000_000; // arbitrary Unix second
    const T0_MS: i64 = T0 * 1000;

    fn window(app: &str, title: &str) -> Option<WindowInfo> {
        Some(WindowInfo {
            app_name: app.into(),
            window_title: title.into(),
            ..Default::default()
        })
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
    fn top_apps_merge_titles_and_rank_by_time() {
        let slices = [
            slice("code", "main.rs", 0, 30),
            slice("firefox", "Docs", 30, 50),
            slice("Code", "lib.rs", 50, 70),
            slice("slack", "general", 70, 75),
            slice("broken", "", 80, 80), // zero-length: ignored
        ];
        assert_eq!(
            top_apps(&slices, 10),
            vec![
                ("code".to_string(), 50_000),
                ("firefox".to_string(), 20_000),
                ("slack".to_string(), 5_000),
            ]
        );
        assert_eq!(top_apps(&slices, 1).len(), 1);
        assert!(top_apps(&[], 5).is_empty());
    }

    #[test]
    fn samples_credit_the_window_in_front_and_merge_repeats() {
        let rec = UsageRecorder::new();
        rec.set_active(true);
        rec.record(T0_MS, window("code", "main.rs")); // first sample: nothing to credit yet
        rec.record(T0_MS + 3_000, window("code", "main.rs"));
        rec.record(T0_MS + 6_000, window("code", "main.rs"));
        rec.record(T0_MS + 9_000, window("firefox", "Docs"));
        rec.record(T0_MS + 12_000, None); // no active window: not credited
        rec.record(T0_MS + 15_000, window("firefox", "Docs"));

        assert_eq!(
            rec.slices_between(0, i64::MAX),
            vec![
                slice("code", "main.rs", 0, 6),
                slice("firefox", "Docs", 6, 9),
                slice("firefox", "Docs", 12, 15),
            ]
        );
    }

    #[test]
    fn long_gaps_and_stale_samples_are_not_credited() {
        let rec = UsageRecorder::new();
        rec.set_active(true);
        rec.record(T0_MS, window("code", ""));
        rec.record(T0_MS + 120_000, window("code", "")); // machine slept
        rec.record(T0_MS + 119_000, window("code", "")); // racing, older sample
        rec.record(T0_MS + 123_000, window("code", ""));
        assert_eq!(
            rec.slices_between(0, i64::MAX),
            vec![slice("code", "", 120, 123)]
        );
    }

    #[test]
    fn slices_are_clipped_to_the_block() {
        let rec = UsageRecorder::new();
        rec.set_active(true);
        rec.record(T0_MS, window("a", ""));
        rec.record(T0_MS + 10_000, window("a", ""));
        assert_eq!(
            rec.slices_between(T0_MS + 4_000, T0_MS + 7_000),
            vec![slice("a", "", 4, 7)]
        );
        assert!(rec
            .slices_between(T0_MS + 10_000, T0_MS + 20_000)
            .is_empty());
    }

    #[test]
    fn slices_older_than_the_retention_are_dropped() {
        let rec = UsageRecorder::new();
        rec.set_active(true);
        rec.record(T0_MS, window("a", ""));
        rec.record(T0_MS + 3_000, window("a", ""));
        // Keep sampling for longer than the retention.
        let mut t = T0_MS + 3_000;
        while t < T0_MS + RETENTION_MS + 60_000 {
            t += 3_000;
            rec.record(t, window(if (t / 3_000) % 2 == 0 { "b" } else { "c" }, ""));
        }
        let all = rec.slices_between(0, i64::MAX);
        assert!(all.iter().all(|s| s.end_ms >= t - RETENTION_MS));
        assert!(all.iter().all(|s| s.app != "a"));
    }

    #[test]
    fn app_usage_groups_windows_and_attributes_input() {
        let slices = [
            slice("code", "main.rs", 0, 100),
            slice("firefox", "Docs", 100, 130),
            slice("code", "main.rs", 130, 200),
            slice("code", "lib.rs", 200, 210),
        ];
        let keys = [T0_MS + 5_000, T0_MS + 150_000, T0_MS + 205_000];
        let clicks = [T0_MS + 110_000, T0_MS + 111_000, T0_MS + 500_000]; // last: outside every slice

        let usage = app_usage(&slices, &keys, &clicks);
        assert_eq!(
            usage,
            vec![
                row("code", "main.rs", 170, 2, 0),
                row("firefox", "Docs", 30, 0, 2),
                row("code", "lib.rs", 10, 1, 0),
            ]
        );
    }

    fn row(app: &str, title: &str, s: u64, k: u64, c: u64) -> WindowInfo {
        WindowInfo {
            app_name: app.into(),
            window_title: title.into(),
            seconds: Some(s),
            keyboard_hits: Some(k),
            mouse_clicks: Some(c),
        }
    }

    fn total_seconds(rows: &[WindowInfo], app: &str) -> u64 {
        rows.iter()
            .filter(|r| r.app_name == app)
            .map(|r| r.seconds.unwrap_or(0))
            .sum()
    }

    #[test]
    fn app_usage_within_the_cap_is_left_alone() {
        let slices: Vec<UsageSlice> = (0..80)
            .map(|i| slice("app", &format!("tab {i}"), i, i + 1))
            .collect();
        let usage = app_usage(&slices, &[], &[]);
        assert_eq!(usage.len(), 80);
        assert!(usage.iter().all(|w| w.window_title != OTHER_WINDOWS_TITLE));
    }

    #[test]
    fn windows_beyond_the_cap_fold_into_other_windows_per_app() {
        // 250 browser tabs of falling length, and an editor used throughout.
        let mut slices: Vec<UsageSlice> = Vec::new();
        let mut t = 0;
        for i in 0..250 {
            let length = 300 - i;
            slices.push(slice("chrome", &format!("tab {i}"), t, t + length));
            t += length;
        }
        slices.push(slice("code", "main.rs", t, t + 20));
        let keys: Vec<i64> = (0..250).map(|i| slices[i].start_ms + 1).collect();

        let usage = app_usage(&slices, &keys, &[]);
        assert_eq!(usage.len(), MAX_USAGE_ENTRIES);
        let others: Vec<&WindowInfo> = usage
            .iter()
            .filter(|w| w.window_title == OTHER_WINDOWS_TITLE)
            .collect();
        assert_eq!(others.len(), 1);
        assert_eq!(others[0].app_name, "chrome");
        // Nothing is lost: every app's seconds and key presses add up.
        let all_chrome: u64 = (0..250).map(|i| 300 - i).sum();
        assert_eq!(total_seconds(&usage, "chrome"), all_chrome);
        assert_eq!(total_seconds(&usage, "code"), 20);
        // A lone window past the cut keeps its title (it takes one row either way).
        assert!(usage
            .iter()
            .any(|w| w.app_name == "code" && w.window_title == "main.rs"));
        let keys_total: u64 = usage.iter().map(|w| w.keyboard_hits.unwrap()).sum();
        assert_eq!(keys_total, 250);
        // Still most-used first.
        assert!(usage.windows(2).all(|w| w[0].seconds >= w[1].seconds));
    }

    #[test]
    fn folding_keeps_as_many_windows_as_fit_with_one_row_per_folded_app() {
        // Cap 5: a1 a2 a3 b1 c1 c2 (most-used first).
        let rows = vec![
            row("a", "1", 60, 1, 0),
            row("a", "2", 50, 0, 0),
            row("a", "3", 40, 0, 0),
            row("b", "1", 30, 0, 1),
            row("c", "1", 20, 0, 0),
            row("c", "2", 10, 2, 0),
        ];
        let folded = fold_beyond_cap(rows, 5);
        assert_eq!(folded.len(), 5);
        // Keeping 4 would need folds for c only: 4 + 1 = 5.
        assert_eq!(
            folded,
            vec![
                row("a", "1", 60, 1, 0),
                row("a", "2", 50, 0, 0),
                row("a", "3", 40, 0, 0),
                row("b", "1", 30, 0, 1),
                row("c", OTHER_WINDOWS_TITLE, 30, 2, 0),
            ]
        );
    }

    #[test]
    fn more_apps_than_the_cap_share_an_other_apps_row() {
        let rows: Vec<WindowInfo> = (0..8u64)
            .map(|i| row(&format!("app{i}"), "", 80 - i * 10, 1, 0))
            .collect();
        let folded = fold_beyond_cap(rows, 4);
        assert_eq!(folded.len(), 4);
        assert_eq!(
            folded.iter().map(|w| w.seconds.unwrap()).sum::<u64>(),
            (0..8u64).map(|i| 80 - i * 10).sum::<u64>()
        );
        let others = folded
            .iter()
            .find(|w| w.app_name == OTHER_APPS_NAME)
            .expect("an 'other apps' row");
        assert_eq!(others.keyboard_hits, Some(5));
        // The five least-used apps (150 s together) now lead, most-used first.
        let names: Vec<&str> = folded.iter().map(|w| w.app_name.as_str()).collect();
        assert_eq!(names, [OTHER_APPS_NAME, "app0", "app1", "app2"]);
        assert!(fold_beyond_cap(vec![row("a", "", 1, 0, 0)], 0).is_empty());
    }

    fn minute(
        m: u32,
        k: Option<u64>,
        c: Option<u64>,
        active: bool,
        app: &str,
        seconds: u32,
    ) -> ActivityMinute {
        ActivityMinute {
            minute: m,
            keyboard_hits: k,
            mouse_clicks: c,
            active,
            app_name: Some(app.into()),
            active_seconds: Some(seconds),
        }
    }

    #[test]
    fn timeline_buckets_input_and_picks_the_main_app_per_minute() {
        // 3-minute block; code for 1.5 min, then firefox.
        let slices = [slice("code", "", 0, 90), slice("firefox", "", 90, 180)];
        let keys = [T0_MS + 1_000, T0_MS + 2_000, T0_MS + 61_000];
        let clicks = [T0_MS + 179_000];
        let active = [T0 + 1, T0 + 61];

        let timeline = minute_timeline(T0, T0 + 180, &slices, Some(&keys), Some(&clicks), &active);
        assert_eq!(
            timeline,
            vec![
                minute(0, Some(2), Some(0), true, "code", 1),
                // 30 s each: the tie goes to the alphabetically first app.
                minute(1, Some(1), Some(0), true, "code", 1),
                minute(2, Some(0), Some(1), false, "firefox", 0),
            ]
        );
    }

    #[test]
    fn unmeasured_input_kinds_stay_null_in_every_minute() {
        let keys = [T0_MS + 1_000];
        let timeline = minute_timeline(T0, T0 + 120, &[], Some(&keys), None, &[T0 + 1]);
        assert_eq!(timeline.len(), 2);
        assert_eq!(timeline[0].keyboard_hits, Some(1));
        assert!(timeline.iter().all(|m| m.mouse_clicks.is_none()));
        let json = serde_json::to_value(&timeline[1]).unwrap();
        assert_eq!(json["mouseClicks"], serde_json::Value::Null);
        assert_eq!(json["keyboardHits"], 0);

        let neither = minute_timeline(T0, T0 + 60, &[], None, None, &[]);
        assert!(neither[0].keyboard_hits.is_none() && neither[0].mouse_clicks.is_none());
    }

    #[test]
    fn active_seconds_count_distinct_seconds_within_each_bucket() {
        // Keyboard and mouse in the same seconds are one second each; the
        // last bucket (89 s) can hold more than 60.
        let mut active: Vec<i64> = (0..40).map(|s| T0 + s).collect();
        active.extend((0..40).map(|s| T0 + s)); // the mouse, at the same seconds
        active.extend((60..149).map(|s| T0 + s));
        active.push(T0 + 500); // after the block: ignored
        let timeline = minute_timeline(T0, T0 + 149, &[], Some(&[]), Some(&[]), &active);
        assert_eq!(timeline.len(), 2);
        assert_eq!(timeline[0].active_seconds, Some(40));
        assert_eq!(timeline[1].active_seconds, Some(89));
        assert!(timeline.iter().all(|m| m.active_seconds <= Some(90)));
    }

    #[test]
    fn idle_seconds_are_the_inactive_buckets_lengths() {
        // 10 min + 29 s: buckets 0..8 are 60 s, bucket 9 is 89 s.
        let (start, end) = (T0, T0 + 629);
        let active: Vec<i64> = [0, 2, 3].iter().map(|m| start + m * 60 + 10).collect();
        let timeline = minute_timeline(start, end, &[], Some(&[]), Some(&[]), &active);
        // Inactive: 1, 4..8 (six 60-s buckets) and 9 (89 s).
        assert_eq!(idle_seconds(&timeline, start, end), 6 * 60 + 89);

        // All active: no idle; none active: the whole block.
        let busy: Vec<i64> = (0..10).map(|m| start + m * 60).collect();
        let quiet = |a: &[i64]| minute_timeline(start, end, &[], Some(&[]), Some(&[]), a);
        assert_eq!(idle_seconds(&quiet(&busy), start, end), 0);
        assert_eq!(idle_seconds(&quiet(&[]), start, end), 629);
        assert_eq!(idle_seconds(&[], start, end), 0);
    }

    #[test]
    fn timeline_folds_a_trailing_sliver_into_the_last_minute() {
        // 2 min + 10 s = 2 buckets; the sliver's input and time join minute 1.
        let slices = [slice("a", "", 0, 60), slice("b", "", 60, 130)];
        let keys = [T0_MS + 125_000];
        let timeline = minute_timeline(T0, T0 + 130, &slices, Some(&keys), Some(&[]), &[T0 + 125]);
        assert_eq!(timeline.len(), 2);
        assert_eq!(timeline[1].keyboard_hits, Some(1));
        assert!(timeline[1].active && !timeline[0].active);
        assert_eq!(timeline[1].app_name.as_deref(), Some("b"));
    }

    #[test]
    fn timeline_without_usage_still_has_counts() {
        let timeline = minute_timeline(T0, T0 + 60, &[], Some(&[T0_MS]), Some(&[]), &[T0]);
        assert_eq!(timeline.len(), 1);
        assert_eq!(timeline[0].keyboard_hits, Some(1));
        assert_eq!(timeline[0].app_name, None);
        assert!(minute_timeline(T0, T0, &[], None, None, &[]).is_empty());
    }

    #[test]
    fn inactive_recorder_drops_samples_and_keeps_slices_empty() {
        let rec = UsageRecorder::new();
        assert!(
            !rec.is_active(),
            "UsageRecorder must default to inactive (opt-in)"
        );
        rec.record(T0_MS, window("secret", "Confidential Document"));
        rec.record(T0_MS + 3_000, window("secret", "Confidential Document"));
        assert!(
            rec.slices_between(0, i64::MAX).is_empty(),
            "an inactive UsageRecorder must drop samples and never buffer window titles"
        );
    }

    #[test]
    fn activating_recorder_records_samples_and_deactivation_clears_timing_anchor() {
        let rec = UsageRecorder::new();
        rec.set_active(true);
        assert!(rec.is_active());
        rec.record(T0_MS, window("code", "main.rs"));
        rec.record(T0_MS + 3_000, window("code", "main.rs"));
        assert_eq!(
            rec.slices_between(0, i64::MAX),
            vec![slice("code", "main.rs", 0, 3)]
        );

        // Deactivating (e.g. paused) clears timing anchor and drops samples
        rec.set_active(false);
        rec.record(T0_MS + 6_000, window("browser", "Personal Email"));
        rec.record(T0_MS + 9_000, window("browser", "Personal Email"));
        assert_eq!(
            rec.slices_between(0, i64::MAX),
            vec![slice("code", "main.rs", 0, 3)]
        );

        // Reactivating resumes recording cleanly without back-crediting paused time
        rec.set_active(true);
        rec.record(T0_MS + 12_000, window("code", "main.rs")); // anchor sample
        rec.record(T0_MS + 15_000, window("code", "main.rs")); // 3s credited
        assert_eq!(
            rec.slices_between(0, i64::MAX),
            vec![
                slice("code", "main.rs", 0, 3),
                slice("code", "main.rs", 12, 15),
            ]
        );

        rec.clear();
        assert!(rec.slices_between(0, i64::MAX).is_empty());
    }

    /// Counts its calls; answers a fixed window.
    struct CountingProvider(AtomicU32);

    impl ActiveWindowProvider for CountingProvider {
        fn current_active_window(&self) -> Result<Option<WindowInfo>, TrackingError> {
            self.0.fetch_add(1, Ordering::SeqCst);
            Ok(window("code", "main.rs"))
        }
    }

    fn wait_until(mut cond: impl FnMut() -> bool) -> bool {
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            if cond() {
                return true;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        false
    }

    #[test]
    fn the_sampler_reads_only_while_the_recorder_is_active_and_stops_promptly() {
        let recorder = Arc::new(UsageRecorder::new());
        let provider = Arc::new(CountingProvider(AtomicU32::new(0)));
        let sampler = UsageSampler::start(
            recorder.clone(),
            provider.clone(),
            Duration::from_millis(10),
        )
        .unwrap();

        // Inactive: the provider is never asked.
        std::thread::sleep(Duration::from_millis(60));
        assert_eq!(provider.0.load(Ordering::SeqCst), 0);

        recorder.set_active(true);
        assert!(wait_until(|| provider.0.load(Ordering::SeqCst) >= 3));
        assert!(wait_until(|| !recorder
            .slices_between(0, i64::MAX)
            .is_empty()));

        let started = Instant::now();
        sampler.stop().unwrap();
        assert!(started.elapsed() < STOP_TIMEOUT);
        let calls = provider.0.load(Ordering::SeqCst);
        std::thread::sleep(Duration::from_millis(50));
        assert_eq!(
            provider.0.load(Ordering::SeqCst),
            calls,
            "no sample after stop"
        );
    }

    #[test]
    fn a_long_interval_does_not_delay_stop() {
        let recorder = Arc::new(UsageRecorder::new());
        let provider = Arc::new(CountingProvider(AtomicU32::new(0)));
        let sampler = UsageSampler::start(recorder, provider, SAMPLE_EVERY).unwrap();
        std::thread::sleep(Duration::from_millis(20));
        let started = Instant::now();
        drop(sampler);
        assert!(started.elapsed() < Duration::from_millis(500));
    }
}
