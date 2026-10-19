//! Keyboard and mouse input, recorded as *when* it happened — never what was
//! typed.
//!
//! A dedicated thread installs the low-level hooks (`WH_KEYBOARD_LL`,
//! `WH_MOUSE_LL`) and runs their message loop. The hook procedures only
//! timestamp events: a key counts once per physical press (auto-repeat while
//! it is held does not), a click counts on button-down, moves and the wheel
//! mark activity, and software-injected events (mouse jigglers, remote tools)
//! are counted aside, never as input. A watchdog reinstalls the hooks if they
//! go silent while Windows still sees input.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use chrono::{DateTime, Utc};

/// How far back input is remembered. The longest block (a 60-minute
/// interval) can end at the last input before the longest idle threshold
/// (60 minutes) cuts it — and is only built then — plus slack for a slow
/// capture.
pub const RETENTION_SECS: i64 = 125 * 60;

// The first minute of such a block is 120 minutes old when it is built.
const _: () = assert!(RETENTION_SECS >= 120 * 60);

/// Hard cap on remembered key presses / clicks per kind. Normal typing stays
/// far below it (~10k/hour); it only bounds memory under an auto-clicker.
const MAX_EVENTS: usize = 200_000;

/// A key-down for a key already held counts again only after this long
/// without another down for it. Auto-repeat fires at least every ~1 s (the
/// longest repeat delay), so this only re-arms a key whose key-up was missed
/// — e.g. released on the secure desktop after Ctrl+Alt+Del.
const MISSED_KEY_UP_MS: i64 = 1_500;

/// How often the watchdog checks the hooks, in ms.
const WATCHDOG_EVERY_MS: i64 = 30_000;

/// How often the watchdog checks the hooks.
pub const WATCHDOG_EVERY: Duration = Duration::from_millis(WATCHDOG_EVERY_MS as u64);

/// How long the hooks may stay silent while Windows sees input before they
/// are reinstalled (and at most how often that is tried).
pub const HOOK_SILENCE_LIMIT_MS: i64 = 5 * 60 * 1000;

/// Input Windows saw within this long counts as "recent" for the watchdog.
const RECENT_INPUT_MS: i64 = 60_000;

/// Keyboard and mouse input over time, read per block.
///
/// Every count is taken over an explicit time window rather than from
/// counters reset at block boundaries, so block totals, the per-minute
/// timeline and the per-app breakdown always agree, and input made before a
/// block (while paused, signed out, or on another project) can never leak
/// into it.
pub struct InputCounter {
    key_presses: EventLog,
    mouse_clicks: EventLog,
    keyboard_activity: ActivityLog,
    mouse_activity: ActivityLog,
    injected_keyboard: CountLog,
    injected_mouse: CountLog,
    health: HookHealth,
    active: AtomicBool,
    allowed_keyboard: AtomicBool,
    allowed_mouse: AtomicBool,
}

impl Default for InputCounter {
    fn default() -> Self {
        Self {
            key_presses: EventLog::default(),
            mouse_clicks: EventLog::default(),
            keyboard_activity: ActivityLog::default(),
            mouse_activity: ActivityLog::default(),
            injected_keyboard: CountLog::default(),
            injected_mouse: CountLog::default(),
            health: HookHealth::default(),
            active: AtomicBool::new(false),
            allowed_keyboard: AtomicBool::new(false),
            allowed_mouse: AtomicBool::new(false),
        }
    }
}

impl InputCounter {
    pub fn new() -> Self {
        Self::default()
    }

    /// Configure effective sensor policy and permissions.
    /// Invariant: Eliminates transient grants by storing exact per-sensor permissions
    /// before enabling the active collection flag.
    pub fn set_policy(&self, active: bool, keyboard: bool, mouse: bool) {
        if !active {
            self.active.store(false, Ordering::SeqCst);
            self.allowed_keyboard.store(false, Ordering::SeqCst);
            self.allowed_mouse.store(false, Ordering::SeqCst);
        } else {
            self.allowed_keyboard.store(keyboard, Ordering::SeqCst);
            self.allowed_mouse.store(mouse, Ordering::SeqCst);
            self.active.store(keyboard || mouse, Ordering::SeqCst);
        }
    }

    /// Turn active tracking on or off. When inactive (e.g. paused, signed out,
    /// away, or before project selection), key presses and clicks are dropped
    /// without recording timestamps into memory buffers.
    #[allow(dead_code)]
    pub fn set_active(&self, active: bool) {
        self.set_policy(active, active, active);
    }

    pub fn is_active(&self) -> bool {
        self.active.load(Ordering::SeqCst)
    }

    /// Configure effective sensor permissions for keyboard and mouse.
    #[allow(dead_code)]
    pub fn set_permissions(&self, keyboard: bool, mouse: bool) {
        self.allowed_keyboard.store(keyboard, Ordering::SeqCst);
        self.allowed_mouse.store(mouse, Ordering::SeqCst);
    }

    pub fn is_keyboard_allowed(&self) -> bool {
        self.allowed_keyboard.load(Ordering::SeqCst)
    }

    pub fn is_mouse_allowed(&self) -> bool {
        self.allowed_mouse.load(Ordering::SeqCst)
    }

    pub(crate) fn record_key_press(&self, at_ms: i64) {
        if !self.is_active() || !self.is_keyboard_allowed() {
            return;
        }
        self.key_presses.record(at_ms);
        self.keyboard_activity.mark(at_ms.div_euclid(1000));
    }

    pub(crate) fn record_click(&self, at_ms: i64) {
        if !self.is_active() || !self.is_mouse_allowed() {
            return;
        }
        self.mouse_clicks.record(at_ms);
        self.mouse_activity.mark(at_ms.div_euclid(1000));
    }

    /// Mouse movement / wheel: counts as activity (reading and scrolling is
    /// work) but not as a click.
    fn record_mouse_activity(&self, at_ms: i64) {
        if !self.is_active() || !self.is_mouse_allowed() {
            return;
        }
        self.mouse_activity.mark(at_ms.div_euclid(1000));
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
    /// are computed from. A second with both kinds appears twice.
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

    /// Injected (synthetic) events of the allowed kinds in `[start, end)`, at
    /// one-second granularity: those whole seconds, so back-to-back blocks
    /// never share an event.
    pub fn injected_between(&self, start: DateTime<Utc>, end: DateTime<Utc>, keyboard: bool, mouse: bool) -> u64 {
        let (start, end) = (start.timestamp(), end.timestamp());
        let mut total = 0;
        if keyboard {
            total += self.injected_keyboard.count(start, end);
        }
        if mouse {
            total += self.injected_mouse.count(start, end);
        }
        total
    }

    /// When the user last gave real keyboard or mouse input (a key press,
    /// click, move or wheel), to the second, as the hooks saw it; `None`
    /// before any. Unlike [`idle_seconds`] — Windows' own idle clock, which
    /// software-injected input such as a mouse jiggler keeps resetting — only
    /// the user's own input moves it, so idle detection can rely on it while
    /// [`InputCounter::hook_healthy`].
    #[cfg_attr(not(test), expect(dead_code, reason = "read by the worker's idle detection"))]
    pub fn last_input_at(&self) -> Option<DateTime<Utc>> {
        let second = self.keyboard_activity.last_second().max(self.mouse_activity.last_second());
        (second > 0).then(|| DateTime::from_timestamp(second, 0)).flatten()
    }

    /// Are the hooks delivering input? False while they are missing, and from
    /// when they provably missed the user's input until they deliver again;
    /// the window shows "input capture unavailable" then.
    #[cfg_attr(not(test), expect(dead_code, reason = "shown by the window when input capture is unavailable"))]
    pub fn hook_healthy(&self) -> bool {
        self.health.healthy()
    }

    /// Was input capture working for all of `[start, end)`? When it was not,
    /// the block's activity is "not measured" rather than a misleading low %.
    pub fn capture_available_between(&self, start: DateTime<Utc>, end: DateTime<Utc>) -> bool {
        self.health.available_between(start.timestamp_millis(), end.timestamp_millis())
    }
}

/// Millisecond timestamps of discrete input events (key presses or clicks),
/// oldest first, bounded by [`RETENTION_SECS`] and [`MAX_EVENTS`].
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
        let horizon = at_ms - RETENTION_SECS * 1000;
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
/// [`RETENTION_SECS`] so memory stays small (< 60 KB even under constant
/// input).
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
            .is_some_and(|&oldest| oldest < second - RETENTION_SECS)
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

    /// The latest second marked, 0 before any.
    fn last_second(&self) -> i64 {
        self.last.load(Ordering::Relaxed)
    }
}

/// Event counts per Unix second, oldest first, bounded to [`RETENTION_SECS`]:
/// exact totals in constant memory however fast the events come.
#[derive(Default)]
struct CountLog {
    seconds: Mutex<VecDeque<(i64, u32)>>,
}

impl CountLog {
    fn add(&self, second: i64) {
        let mut seconds = self.seconds.lock().unwrap_or_else(|e| e.into_inner());
        match seconds.back_mut() {
            // Same second — or the clock stepped back: filed with the newest.
            Some((last, count)) if second <= *last => *count = count.saturating_add(1),
            _ => seconds.push_back((second, 1)),
        }
        let newest = seconds.back().map_or(second, |&(s, _)| s);
        while seconds.front().is_some_and(|&(oldest, _)| oldest < newest - RETENTION_SECS) {
            seconds.pop_front();
        }
    }

    /// Events in the whole seconds `[start, end)`.
    fn count(&self, start: i64, end: i64) -> u64 {
        let seconds = self.seconds.lock().unwrap_or_else(|e| e.into_inner());
        seconds
            .iter()
            .filter(|&&(s, _)| s >= start && s < end)
            .map(|&(_, count)| u64::from(count))
            .sum()
    }
}

/// Whether the hooks are delivering input, and when they were not.
#[derive(Default)]
struct HookHealth {
    /// When the hooks last delivered any event, injected or not (or were
    /// installed), in Unix ms.
    last_event_ms: AtomicI64,
    /// Since when capture has been unavailable, in Unix ms; 0 while healthy.
    unhealthy_since_ms: AtomicI64,
    /// The latest outage that has ended, `[from, to)` in Unix ms.
    last_outage: Mutex<Option<(i64, i64)>>,
}

impl HookHealth {
    fn healthy(&self) -> bool {
        self.unhealthy_since_ms.load(Ordering::Relaxed) == 0
    }

    fn last_event_ms(&self) -> i64 {
        self.last_event_ms.load(Ordering::Relaxed)
    }

    /// The hooks were (re)installed: the silence clock starts now.
    fn hooks_installed(&self, at_ms: i64) {
        self.last_event_ms.store(at_ms, Ordering::Relaxed);
    }

    /// An event arrived: the hooks work, and an outage (if any) ends here.
    fn saw_event(&self, at_ms: i64) {
        self.last_event_ms.store(at_ms, Ordering::Relaxed);
        if self.unhealthy_since_ms.load(Ordering::Relaxed) != 0 {
            let since = self.unhealthy_since_ms.swap(0, Ordering::Relaxed);
            if since != 0 {
                *self.last_outage.lock().unwrap_or_else(|e| e.into_inner()) = Some((since, at_ms));
            }
        }
    }

    /// Capture is unavailable since `since_ms` (an ongoing outage keeps its
    /// earlier start).
    fn mark_unhealthy(&self, since_ms: i64) {
        let _ = self.unhealthy_since_ms.compare_exchange(0, since_ms.max(1), Ordering::Relaxed, Ordering::Relaxed);
    }

    fn available_between(&self, start_ms: i64, end_ms: i64) -> bool {
        let since = self.unhealthy_since_ms.load(Ordering::Relaxed);
        if since != 0 && since < end_ms {
            return false;
        }
        let last = *self.last_outage.lock().unwrap_or_else(|e| e.into_inner());
        !matches!(last, Some((from, to)) if from < end_ms && to > start_ms)
    }

    /// Reset outage and silence clocks when sensors are stopped/deactivated.
    fn reset_silence(&self) {
        self.last_event_ms.store(0, Ordering::Relaxed);
        self.unhealthy_since_ms.store(0, Ordering::Relaxed);
        *self.last_outage.lock().unwrap_or_else(|e| e.into_inner()) = None;
    }
}

/// A low-level hook message, reduced to what is counted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RawInput {
    /// Key down, with its virtual-key code.
    KeyDown(u32),
    KeyUp(u32),
    ButtonDown,
    ButtonUp,
    Move,
    Wheel,
}

// Window messages the low-level hooks deliver (winuser.h).
const WM_KEYDOWN: u32 = 0x0100;
const WM_KEYUP: u32 = 0x0101;
const WM_SYSKEYDOWN: u32 = 0x0104;
const WM_SYSKEYUP: u32 = 0x0105;
const WM_MOUSEMOVE: u32 = 0x0200;
const WM_LBUTTONDOWN: u32 = 0x0201;
const WM_LBUTTONUP: u32 = 0x0202;
const WM_RBUTTONDOWN: u32 = 0x0204;
const WM_RBUTTONUP: u32 = 0x0205;
const WM_MBUTTONDOWN: u32 = 0x0207;
const WM_MBUTTONUP: u32 = 0x0208;
const WM_MOUSEWHEEL: u32 = 0x020A;
const WM_XBUTTONDOWN: u32 = 0x020B;
const WM_XBUTTONUP: u32 = 0x020C;
const WM_MOUSEHWHEEL: u32 = 0x020E;

/// A keyboard hook message (`wParam`) for virtual key `vk`.
pub fn keyboard_input(message: u32, vk: u32) -> Option<RawInput> {
    match message {
        WM_KEYDOWN | WM_SYSKEYDOWN => Some(RawInput::KeyDown(vk)),
        WM_KEYUP | WM_SYSKEYUP => Some(RawInput::KeyUp(vk)),
        _ => None,
    }
}

/// A mouse hook message (`wParam`).
pub fn mouse_input(message: u32) -> Option<RawInput> {
    match message {
        WM_MOUSEMOVE => Some(RawInput::Move),
        WM_LBUTTONDOWN | WM_RBUTTONDOWN | WM_MBUTTONDOWN | WM_XBUTTONDOWN => Some(RawInput::ButtonDown),
        WM_LBUTTONUP | WM_RBUTTONUP | WM_MBUTTONUP | WM_XBUTTONUP => Some(RawInput::ButtonUp),
        WM_MOUSEWHEEL | WM_MOUSEHWHEEL => Some(RawInput::Wheel),
        _ => None,
    }
}

/// Turns hook messages into counted input. Owned by the hook thread.
struct InputFilter {
    /// Per virtual key: when its latest down arrived (Unix ms), 0 while up.
    key_down_at: [i64; 256],
}

impl Default for InputFilter {
    fn default() -> Self {
        Self { key_down_at: [0; 256] }
    }
}

impl InputFilter {
    fn apply(&mut self, counter: &InputCounter, input: RawInput, injected: bool, at_ms: i64) {
        if !counter.is_active() {
            return;
        }
        // Denied sensors must not mutate health or key timing either.
        let allowed = match input {
            RawInput::KeyDown(_) | RawInput::KeyUp(_) => counter.is_keyboard_allowed(),
            RawInput::ButtonDown | RawInput::ButtonUp | RawInput::Move | RawInput::Wheel => {
                counter.is_mouse_allowed()
            }
        };
        if !allowed {
            return;
        }

        counter.health.saw_event(at_ms);
        let second = at_ms.div_euclid(1000);
        if injected {
            match input {
                RawInput::KeyDown(_) => counter.injected_keyboard.add(second),
                RawInput::ButtonDown | RawInput::Move | RawInput::Wheel => counter.injected_mouse.add(second),
                RawInput::KeyUp(_) | RawInput::ButtonUp => {}
            }
            return;
        }
        match input {
            RawInput::KeyDown(vk) => {
                if self.key_down(vk, at_ms) {
                    counter.record_key_press(at_ms);
                }
            }
            RawInput::KeyUp(vk) => self.key_down_at[(vk & 0xFF) as usize] = 0,
            RawInput::ButtonDown => counter.record_click(at_ms),
            RawInput::Move | RawInput::Wheel => counter.record_mouse_activity(at_ms),
            RawInput::ButtonUp => {}
        }
    }

    /// Is this down a new physical press (not auto-repeat of a held key)?
    fn key_down(&mut self, vk: u32, at_ms: i64) -> bool {
        let slot = &mut self.key_down_at[(vk & 0xFF) as usize];
        let press = *slot == 0 || at_ms - *slot > MISSED_KEY_UP_MS;
        *slot = at_ms.max(1);
        press
    }

    /// Forget which keys are held (their key-ups may have been missed).
    pub(crate) fn reset(&mut self) {
        self.key_down_at = [0; 256];
    }
}

/// What the watchdog knows at one of its checks (all times Unix ms).
#[derive(Debug, Clone, Copy)]
pub struct WatchdogCheck {
    pub now_ms: i64,
    /// The hooks are installed.
    pub installed: bool,
    /// When the hooks last delivered an event (or were installed).
    pub last_hook_event_ms: i64,
    /// When Windows last saw keyboard or mouse input (`GetLastInputInfo`),
    /// if it says.
    pub last_input_ms: Option<i64>,
    /// Since when every check has found input going to the hooks' desktop;
    /// `None` while it goes to another one — the secure desktop of the lock
    /// screen, a UAC prompt or Ctrl+Alt+Del — which the hooks never see.
    pub own_desktop_since_ms: Option<i64>,
    /// The latest check that found the pointer somewhere else on the hooks'
    /// desktop than the check before.
    pub pointer_moved_ms: Option<i64>,
    /// The latest (re)install attempt.
    pub last_attempt_ms: i64,
}

/// What the watchdog does at a check.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WatchdogAction {
    Keep,
    /// (Re)install the hooks. `missed_real_input`: they provably missed the
    /// user's input, so capture was down since their last event.
    Reinstall { missed_real_input: bool },
}

/// Decide at a check. Missing hooks are installed again. Installed ones are
/// suspect when Windows saw input lately — after a check had already found
/// input going to the hooks' desktop, so typing a password on the lock screen
/// does not count — while they have been silent for
/// [`HOOK_SILENCE_LIMIT_MS`]; they are reinstalled then. Either is tried at
/// most once per that limit.
///
/// Silence alone does not prove the hooks dead: Windows' idle clock also
/// counts input they never see — a zero-length "virtual" mouse move (the
/// "zen" mode of mouse jigglers), or input to an elevated window — and
/// calling that "not measured" would hide idle time. Only a pointer that
/// moved on their desktop since their last event is proof they missed real
/// input.
pub fn watchdog_action(check: &WatchdogCheck) -> WatchdogAction {
    if check.now_ms - check.last_attempt_ms < HOOK_SILENCE_LIMIT_MS {
        return WatchdogAction::Keep;
    }
    if !check.installed {
        return WatchdogAction::Reinstall { missed_real_input: false };
    }
    let (Some(input_ms), Some(own_desktop_since_ms)) = (check.last_input_ms, check.own_desktop_since_ms) else {
        return WatchdogAction::Keep;
    };
    let silent = input_ms > own_desktop_since_ms
        && check.now_ms - input_ms <= RECENT_INPUT_MS
        && check.now_ms - check.last_hook_event_ms > HOOK_SILENCE_LIMIT_MS;
    if !silent {
        return WatchdogAction::Keep;
    }
    // The pointer moved at some point in the interval ending at that check.
    let missed_real_input = check
        .pointer_moved_ms
        .is_some_and(|moved| moved - WATCHDOG_EVERY_MS > check.last_hook_event_ms);
    WatchdogAction::Reinstall { missed_real_input }
}

/// Track [`WatchdogCheck::own_desktop_since_ms`] across checks: kept while
/// input stays on the hooks' desktop, started by the first check that finds
/// it there, cleared while it is elsewhere.
pub fn own_desktop_since(previous: Option<i64>, input_on_own_desktop: bool, now_ms: i64) -> Option<i64> {
    input_on_own_desktop.then(|| previous.unwrap_or(now_ms))
}

/// Did the pointer move between two checks? Only when both found it (on
/// the hooks' desktop).
pub fn pointer_moved(previous: Option<(i32, i32)>, current: Option<(i32, i32)>) -> bool {
    matches!((previous, current), (Some(before), Some(now)) if before != now)
}

/// Milliseconds since the last input, from the 64-bit tick count now and the
/// 32-bit tick of the last input (`GetLastInputInfo`), which wraps every
/// 49.7 days.
pub fn idle_millis(now_tick: u64, last_input_tick: u32) -> u64 {
    u64::from((now_tick as u32).wrapping_sub(last_input_tick))
}

/// Seconds since the last keyboard or mouse input anywhere in this session
/// (`GetLastInputInfo`), or `None` when Windows will not say. Software-
/// injected input counts here too; see [`InputCounter::last_input_at`].
#[expect(dead_code, reason = "read by the worker's idle detection")]
pub fn idle_seconds() -> Option<u64> {
    system_idle_millis().map(|ms| ms / 1000)
}

/// When Windows last saw input, in Unix ms, from its idle time at `now_ms`.
#[cfg(windows)]
fn system_last_input_ms(now_ms: i64) -> Option<i64> {
    system_idle_millis().map(|idle| now_ms.saturating_sub(i64::try_from(idle).unwrap_or(i64::MAX)))
}

/// Where the pointer is, in screen coordinates; `None` when Windows will not
/// say (e.g. while another desktop has the input).
#[cfg(windows)]
fn pointer_position() -> Option<(i32, i32)> {
    use windows::Win32::Foundation::POINT;
    use windows::Win32::UI::WindowsAndMessaging::GetCursorPos;

    let mut point = POINT::default();
    unsafe { GetCursorPos(&mut point) }.ok()?;
    Some((point.x, point.y))
}

/// Does input go to the desktop this thread (and so its hooks) is on? Not
/// while the screen is locked or a UAC or Ctrl+Alt+Del screen is up: those
/// run on the secure desktop, which this process cannot even open.
#[cfg(windows)]
fn input_on_own_desktop() -> bool {
    use windows::Win32::Foundation::{BOOL, HANDLE};
    use windows::Win32::System::StationsAndDesktops::{
        CloseDesktop, GetThreadDesktop, GetUserObjectInformationW, OpenInputDesktop, DESKTOP_CONTROL_FLAGS,
        DESKTOP_READOBJECTS, HDESK, UOI_NAME,
    };
    use windows::Win32::System::Threading::GetCurrentThreadId;

    fn name(desktop: HDESK) -> Option<String> {
        let mut buffer = [0u16; 256];
        unsafe {
            GetUserObjectInformationW(
                HANDLE(desktop.0),
                UOI_NAME,
                Some(buffer.as_mut_ptr().cast()),
                std::mem::size_of_val(&buffer) as u32,
                None,
            )
        }
        .ok()?;
        let length = buffer.iter().position(|&c| c == 0).unwrap_or(buffer.len());
        Some(String::from_utf16_lossy(&buffer[..length]))
    }

    // The thread's own desktop handle is borrowed (never closed).
    let Some(own) = (unsafe { GetThreadDesktop(GetCurrentThreadId()) }).ok().and_then(name) else {
        return false;
    };
    let Ok(input) = (unsafe { OpenInputDesktop(DESKTOP_CONTROL_FLAGS(0), BOOL(0), DESKTOP_READOBJECTS) }) else {
        return false;
    };
    let input_name = name(input);
    let _ = unsafe { CloseDesktop(input) };
    input_name.is_some_and(|input| input.eq_ignore_ascii_case(&own))
}

#[cfg(windows)]
fn system_idle_millis() -> Option<u64> {
    use windows::Win32::System::SystemInformation::GetTickCount64;
    use windows::Win32::UI::Input::KeyboardAndMouse::{GetLastInputInfo, LASTINPUTINFO};

    let mut info = LASTINPUTINFO { cbSize: std::mem::size_of::<LASTINPUTINFO>() as u32, dwTime: 0 };
    if !unsafe { GetLastInputInfo(&mut info) }.as_bool() {
        return None;
    }
    Some(idle_millis(unsafe { GetTickCount64() }, info.dwTime))
}

#[cfg(not(windows))]
fn system_idle_millis() -> Option<u64> {
    None
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

/// Start the input hooks on their own thread. They are event-driven (the OS
/// calls them; the thread sleeps in `GetMessageW` otherwise), so they cost
/// ~0% CPU while idle.
#[cfg(windows)]
#[cfg_attr(not(test), expect(dead_code, reason = "legacy/test hook starter"))]
pub fn spawn_listener(counter: Arc<InputCounter>) {
    let for_thread = counter.clone();
    let spawned = std::thread::Builder::new()
        .name("input-hooks".into())
        .spawn(move || hooks::run(for_thread));
    if let Err(e) = spawned {
        tracing::error!(?e, "could not start the input hooks; activity cannot be measured");
        counter.health.mark_unhealthy(Utc::now().timestamp_millis());
    }
}

/// No input hooks on this platform: activity is never measured.
#[cfg(not(windows))]
pub fn spawn_listener(counter: Arc<InputCounter>) {
    counter.health.mark_unhealthy(1);
}

#[cfg(windows)]
#[allow(unused_imports)]
pub(crate) use hooks::{install, run_hooks_thread, uninstall, Handshake, Hooks};

#[cfg(windows)]
#[allow(dead_code)]
pub(crate) fn run_hooks_loop(counter: Arc<InputCounter>) {
    hooks::run(counter);
}

#[cfg(windows)]
mod hooks {
    use std::cell::RefCell;
    use std::sync::Arc;

    use chrono::Utc;
    use windows::core::PCWSTR;
    use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, WPARAM};
    use windows::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows::Win32::System::Threading::{GetCurrentThread, SetThreadPriority, THREAD_PRIORITY_HIGHEST};
    use windows::Win32::UI::WindowsAndMessaging::{
        CallNextHookEx, GetMessageW, SetTimer, SetWindowsHookExW, UnhookWindowsHookEx, HC_ACTION, HHOOK,
        KBDLLHOOKSTRUCT, LLKHF_INJECTED, LLMHF_INJECTED, MSG, MSLLHOOKSTRUCT, WH_KEYBOARD_LL, WH_MOUSE_LL, WM_TIMER,
    };

    use super::{
        keyboard_input, mouse_input, own_desktop_since, pointer_moved, watchdog_action, InputCounter, InputFilter,
        RawInput, WatchdogAction, WatchdogCheck, WATCHDOG_EVERY,
    };

    /// What the hook procedures (which get no user data) work with. The
    /// hooks run on the thread that installed them, inside its message loop,
    /// so a thread-local is all they need.
    struct Context {
        counter: Arc<InputCounter>,
        filter: InputFilter,
    }

    thread_local! {
        static CONTEXT: RefCell<Option<Context>> = const { RefCell::new(None) };
    }

    pub struct Hooks {
        pub keyboard: Option<HHOOK>,
        pub mouse: Option<HHOOK>,
    }

    fn now_ms() -> i64 {
        Utc::now().timestamp_millis()
    }

    pub fn install(keyboard: bool, mouse: bool) -> windows::core::Result<Hooks> {
        unsafe {
            let module = GetModuleHandleW(PCWSTR::null())?;
            let instance = HINSTANCE(module.0);
            let keyboard_hook = if keyboard {
                Some(SetWindowsHookExW(WH_KEYBOARD_LL, Some(keyboard_proc), instance, 0)?)
            } else {
                None
            };
            let mouse_hook = if mouse {
                match SetWindowsHookExW(WH_MOUSE_LL, Some(mouse_proc), instance, 0) {
                    Ok(m) => Some(m),
                    Err(e) => {
                        if let Some(k) = keyboard_hook {
                            let _ = UnhookWindowsHookEx(k);
                        }
                        return Err(e);
                    }
                }
            } else {
                None
            };
            Ok(Hooks { keyboard: keyboard_hook, mouse: mouse_hook })
        }
    }

    pub fn uninstall(mut hooks: Hooks) {
        unsafe {
            if let Some(k) = hooks.keyboard.take() {
                let _ = UnhookWindowsHookEx(k);
            }
            if let Some(m) = hooks.mouse.take() {
                let _ = UnhookWindowsHookEx(m);
            }
        }
    }

    /// Hand one event to the filter. Kept minimal: Windows drops a hook that
    /// keeps the input queue waiting.
    fn dispatch(input: RawInput, injected: bool) {
        let at_ms = now_ms();
        CONTEXT.with(|context| {
            if let Ok(mut context) = context.try_borrow_mut() {
                if let Some(Context { counter, filter }) = context.as_mut() {
                    filter.apply(counter, input, injected, at_ms);
                }
            }
        });
    }

    unsafe extern "system" fn keyboard_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
        if code == HC_ACTION as i32 && lparam.0 != 0 {
            let info = unsafe { &*(lparam.0 as *const KBDLLHOOKSTRUCT) };
            if let Some(input) = keyboard_input(wparam.0 as u32, info.vkCode) {
                dispatch(input, info.flags.0 & LLKHF_INJECTED.0 != 0);
            }
        }
        unsafe { CallNextHookEx(HHOOK::default(), code, wparam, lparam) }
    }

    unsafe extern "system" fn mouse_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
        if code == HC_ACTION as i32 && lparam.0 != 0 {
            let info = unsafe { &*(lparam.0 as *const MSLLHOOKSTRUCT) };
            if let Some(input) = mouse_input(wparam.0 as u32) {
                dispatch(input, info.flags & LLMHF_INJECTED != 0);
            }
        }
        unsafe { CallNextHookEx(HHOOK::default(), code, wparam, lparam) }
    }

    /// What the watchdog carries from one check to the next.
    struct Watchdog {
        last_attempt_ms: i64,
        own_desktop_since_ms: Option<i64>,
        /// The pointer at the previous check, if it was on the hooks' desktop.
        pointer: Option<(i32, i32)>,
        pointer_moved_ms: Option<i64>,
    }

    #[derive(Debug, Clone, Copy)]
    pub struct Handshake {
        pub thread_id: u32,
        pub success: bool,
    }

    /// Dedicated hook thread: sets up message queue, installs permitted hooks,
    /// sends start handshake, pumps messages, and cleans up upon WM_QUIT.
    pub fn run_hooks_thread(
        counter: Arc<InputCounter>,
        keyboard: bool,
        mouse: bool,
        ready_tx: std::sync::mpsc::Sender<Handshake>,
    ) {
        if let Err(e) = unsafe { SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_HIGHEST) } {
            tracing::debug!(?e, "could not raise the input hook thread's priority");
        }

        // Win32 message queues are created lazily. Force creation BEFORE handshake.
        let mut msg = MSG::default();
        unsafe {
            let _ = windows::Win32::UI::WindowsAndMessaging::PeekMessageW(
                &mut msg,
                HWND::default(),
                0,
                0,
                windows::Win32::UI::WindowsAndMessaging::PM_NOREMOVE,
            );
        }
        let thread_id = unsafe { windows::Win32::System::Threading::GetCurrentThreadId() };

        CONTEXT.with(|context| {
            *context.borrow_mut() = Some(Context {
                counter: counter.clone(),
                filter: InputFilter::default(),
            });
        });

        let started = now_ms();
        let mut hooks = match install(keyboard, mouse) {
            Ok(h) => {
                counter.health.hooks_installed(started);
                Some(h)
            }
            Err(e) => {
                tracing::error!(?e, "could not install the input hooks; activity cannot be measured");
                counter.health.mark_unhealthy(started);
                None
            }
        };

        let success = hooks.as_ref().is_some_and(|h| h.keyboard.is_some() || h.mouse.is_some() || (!keyboard && !mouse));
        let _ = ready_tx.send(Handshake { thread_id, success });
        if !success {
            return;
        }

        let mut watch = Watchdog {
            last_attempt_ms: started,
            own_desktop_since_ms: None,
            pointer: None,
            pointer_moved_ms: None,
        };

        unsafe { SetTimer(HWND::default(), 0, WATCHDOG_EVERY.as_millis() as u32, None) };
        let mut message = MSG::default();
        while unsafe { GetMessageW(&mut message, HWND::default(), 0, 0) }.0 > 0 {
            if message.message == WM_TIMER {
                watchdog(&counter, &mut hooks, &mut watch);
            }
        }
        if let Some(hooks) = hooks.take() {
            uninstall(hooks);
        }
        CONTEXT.with(|context| {
            if let Ok(mut context) = context.try_borrow_mut() {
                if let Some(context) = context.as_mut() {
                    context.filter.reset();
                }
            }
        });
        counter.health.reset_silence();
        tracing::info!("the input hook thread stopped and unhooked successfully");
    }

    /// Legacy run wrapper for tests.
    #[allow(dead_code)]
    pub fn run(counter: Arc<InputCounter>) {
        let (tx, _) = std::sync::mpsc::channel();
        run_hooks_thread(counter, true, true, tx);
    }

    /// Windows silently unhooks a hook that times out too often, and a
    /// session change can leave it dead. When Windows has seen input the
    /// hooks did not, reinstall them; when they provably missed the user's
    /// input, capture is reported unavailable until they deliver again.
    fn watchdog(counter: &InputCounter, hooks: &mut Option<Hooks>, watch: &mut Watchdog) {
        if !counter.is_active() {
            if let Some(old) = hooks.take() {
                uninstall(old);
            }
            return;
        }
        let keyboard = counter.is_keyboard_allowed();
        let mouse = counter.is_mouse_allowed();
        if !keyboard && !mouse {
            if let Some(old) = hooks.take() {
                uninstall(old);
            }
            return;
        }
        let now = now_ms();
        let on_own_desktop = super::input_on_own_desktop();
        watch.own_desktop_since_ms = own_desktop_since(watch.own_desktop_since_ms, on_own_desktop, now);
        let pointer = if on_own_desktop { super::pointer_position() } else { None };
        if pointer_moved(watch.pointer, pointer) {
            watch.pointer_moved_ms = Some(now);
        }
        watch.pointer = pointer;

        let last_event = counter.health.last_event_ms();
        let check = WatchdogCheck {
            now_ms: now,
            installed: hooks.as_ref().is_some_and(|h| h.keyboard.is_some() || h.mouse.is_some()),
            last_hook_event_ms: last_event,
            last_input_ms: super::system_last_input_ms(now),
            own_desktop_since_ms: watch.own_desktop_since_ms,
            pointer_moved_ms: watch.pointer_moved_ms,
            last_attempt_ms: watch.last_attempt_ms,
        };
        let WatchdogAction::Reinstall { missed_real_input } = watchdog_action(&check) else {
            return;
        };
        watch.last_attempt_ms = now;
        if let Some(old) = hooks.take() {
            if missed_real_input {
                tracing::warn!("the input hooks missed the user's input; reinstalling them");
                counter.health.mark_unhealthy(last_event);
            } else {
                tracing::debug!("Windows saw input the hooks did not; reinstalling them");
            }
            uninstall(old);
        }
        CONTEXT.with(|context| {
            if let Ok(mut context) = context.try_borrow_mut() {
                if let Some(context) = context.as_mut() {
                    context.filter.reset();
                }
            }
        });
        match install(keyboard, mouse) {
            Ok(new) => {
                counter.health.hooks_installed(now);
                *hooks = Some(new);
            }
            Err(e) => {
                tracing::error!(?e, "could not reinstall the input hooks");
                counter.health.mark_unhealthy(now);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const T0: i64 = 1_790_000_000; // arbitrary Unix second
    const T0_MS: i64 = T0 * 1000;

    fn at(second: i64) -> DateTime<Utc> {
        DateTime::from_timestamp(second, 0).unwrap()
    }

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
    fn activity_log_dedupes_seconds_and_drops_old_entries() {
        let log = ActivityLog::default();
        log.mark(T0);
        log.mark(T0); // same second: no duplicate
        log.mark(T0 + 1);
        assert_eq!(log.seconds_between(T0, T0 + 10), vec![T0, T0 + 1]);

        // Far in the future: everything older than the retention window goes.
        log.mark(T0 + RETENTION_SECS + 10);
        assert_eq!(
            log.seconds_between(T0 - 1, T0 + 2 * RETENTION_SECS),
            vec![T0 + RETENTION_SECS + 10]
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
        counter.set_active(true);
        let before = Utc::now() - chrono::Duration::seconds(1);
        let now_ms = || Utc::now().timestamp_millis();
        counter.record_key_press(now_ms());
        counter.record_key_press(now_ms());
        counter.record_click(now_ms());
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
        let later = 10_000 + RETENTION_SECS * 1000 + 1;
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

    #[test]
    fn hook_messages_are_classified() {
        assert_eq!(keyboard_input(WM_KEYDOWN, 0x41), Some(RawInput::KeyDown(0x41)));
        assert_eq!(keyboard_input(WM_SYSKEYDOWN, 0x12), Some(RawInput::KeyDown(0x12)));
        assert_eq!(keyboard_input(WM_KEYUP, 0x41), Some(RawInput::KeyUp(0x41)));
        assert_eq!(keyboard_input(WM_SYSKEYUP, 0x12), Some(RawInput::KeyUp(0x12)));
        assert_eq!(keyboard_input(0x0102, 0x41), None); // WM_CHAR never reaches a LL hook
        for down in [WM_LBUTTONDOWN, WM_RBUTTONDOWN, WM_MBUTTONDOWN, WM_XBUTTONDOWN] {
            assert_eq!(mouse_input(down), Some(RawInput::ButtonDown));
        }
        for up in [WM_LBUTTONUP, WM_RBUTTONUP, WM_MBUTTONUP, WM_XBUTTONUP] {
            assert_eq!(mouse_input(up), Some(RawInput::ButtonUp));
        }
        assert_eq!(mouse_input(WM_MOUSEMOVE), Some(RawInput::Move));
        assert_eq!(mouse_input(WM_MOUSEWHEEL), Some(RawInput::Wheel));
        assert_eq!(mouse_input(WM_MOUSEHWHEEL), Some(RawInput::Wheel));
        assert_eq!(mouse_input(0x0203), None); // double-click messages are not delivered to LL hooks
    }

    /// Feed `events` (input, injected, ms after T0) through a fresh filter.
    fn feed(events: &[(RawInput, bool, i64)]) -> InputCounter {
        let counter = InputCounter::new();
        counter.set_active(true);
        let mut filter = InputFilter::default();
        for &(input, injected, ms) in events {
            filter.apply(&counter, input, injected, T0_MS + ms);
        }
        counter
    }

    #[test]
    fn a_held_key_counts_once_however_long_it_repeats() {
        // Held for 3 s: a 500 ms delay, then auto-repeat every 33 ms.
        let mut events = vec![(RawInput::KeyDown(0x41), false, 0)];
        events.extend((0..76).map(|i| (RawInput::KeyDown(0x41), false, 500 + 33 * i)));
        events.push((RawInput::KeyUp(0x41), false, 3_000));
        // Pressed again: a new press.
        events.push((RawInput::KeyDown(0x41), false, 3_200));
        let counter = feed(&events);
        assert_eq!(counter.counts_between(at(T0), at(T0 + 10)), (2, 0));
        // Only the seconds of real presses are active, not the held stretch.
        assert_eq!(counter.active_seconds(at(T0), at(T0 + 10), true, true), vec![T0, T0 + 3]);
    }

    #[test]
    fn different_keys_and_chords_count_per_key() {
        let counter = feed(&[
            (RawInput::KeyDown(0x11), false, 0),  // Ctrl
            (RawInput::KeyDown(0x43), false, 40), // C
            (RawInput::KeyUp(0x43), false, 90),
            (RawInput::KeyDown(0x56), false, 300), // V, Ctrl still held
            (RawInput::KeyDown(0x11), false, 330), // Ctrl auto-repeat
            (RawInput::KeyUp(0x56), false, 380),
            (RawInput::KeyUp(0x11), false, 400),
        ]);
        assert_eq!(counter.counts_between(at(T0), at(T0 + 1)), (3, 0));
    }

    #[test]
    fn a_key_whose_release_was_missed_counts_again_after_a_pause() {
        // Ctrl+Alt+Del: the key-ups happen on the secure desktop, unseen.
        let counter = feed(&[
            (RawInput::KeyDown(0x11), false, 0),
            (RawInput::KeyDown(0x11), false, 30_000),
        ]);
        assert_eq!(counter.counts_between(at(T0), at(T0 + 60)).0, 2);
    }

    #[test]
    fn clicks_count_on_button_down_and_moves_only_mark_activity() {
        let counter = feed(&[
            (RawInput::ButtonDown, false, 0),
            (RawInput::ButtonUp, false, 80),
            (RawInput::Move, false, 61_000),
            (RawInput::Wheel, false, 125_000),
        ]);
        assert_eq!(counter.counts_between(at(T0), at(T0 + 180)), (0, 1));
        assert_eq!(counter.active_seconds(at(T0), at(T0 + 180), false, true), vec![T0, T0 + 61, T0 + 125]);
        assert_eq!(counter.activity_percent(at(T0), at(T0 + 180), true, true), 100);
        // Mouse activity does not count when only the keyboard is recorded.
        assert_eq!(counter.activity_percent(at(T0), at(T0 + 180), true, false), 0);
    }

    #[test]
    fn injected_input_is_reported_aside_and_never_counted() {
        // A mouse jiggler and a remote tool typing.
        let mut events: Vec<(RawInput, bool, i64)> = (0..10).map(|i| (RawInput::Move, true, i * 30_000)).collect();
        events.push((RawInput::KeyDown(0x41), true, 1_000));
        events.push((RawInput::KeyUp(0x41), true, 1_050));
        events.push((RawInput::ButtonDown, true, 2_000));
        let counter = feed(&events);

        let (start, end) = (at(T0), at(T0 + 300));
        assert_eq!(counter.counts_between(start, end), (0, 0));
        assert_eq!(counter.activity_percent(start, end, true, true), 0);
        assert_eq!(counter.injected_between(start, end, true, true), 12);
        assert_eq!(counter.injected_between(start, end, true, false), 1);
        assert_eq!(counter.injected_between(start, end, false, true), 11);
        // Whole seconds, half-open: the block before or after sees none of them.
        assert_eq!(counter.injected_between(at(T0 - 60), at(T0), true, true), 0);
        assert_eq!(counter.injected_between(at(T0 + 271), at(T0 + 400), true, true), 0);

        // A real press of the same key still counts.
        let mut filter = InputFilter::default();
        filter.apply(&counter, RawInput::KeyDown(0x41), false, T0_MS + 5_000);
        assert_eq!(counter.counts_between(start, end), (1, 0));
    }

    #[test]
    fn injected_counts_are_exact_under_a_flood_and_bounded_in_memory() {
        let log = CountLog::default();
        for i in 0..100_000 {
            log.add(T0 + i / 1_000); // 1,000 events a second for 100 s
        }
        assert_eq!(log.count(T0, T0 + 100), 100_000);
        assert_eq!(log.count(T0 + 10, T0 + 20), 10_000);
        log.add(T0 + 50); // clock stepped back: filed with the newest second
        assert_eq!(log.count(T0 + 99, T0 + 100), 1_001);
        log.add(T0 + RETENTION_SECS + 200);
        assert_eq!(log.count(T0, T0 + 100), 0, "older than the retention window");
        assert!(log.seconds.lock().unwrap().len() <= 2);
    }

    #[test]
    fn capture_is_unavailable_for_blocks_overlapping_an_outage() {
        let counter = InputCounter::new();
        let health = &counter.health;
        assert!(counter.hook_healthy());
        assert!(counter.capture_available_between(at(T0), at(T0 + 600)));

        // Silent from minute 5; found out later, recovered at minute 20.
        health.mark_unhealthy(T0_MS + 300_000);
        assert!(!counter.hook_healthy());
        assert!(counter.capture_available_between(at(T0), at(T0 + 300)), "ended before the outage");
        assert!(!counter.capture_available_between(at(T0), at(T0 + 600)));
        // A second report keeps the earlier start.
        health.mark_unhealthy(T0_MS + 500_000);
        assert!(!counter.capture_available_between(at(T0 + 320), at(T0 + 480)));

        let mut filter = InputFilter::default();
        counter.set_active(true);
        filter.apply(&counter, RawInput::Move, false, T0_MS + 1_200_000);
        assert!(counter.hook_healthy());
        assert!(!counter.capture_available_between(at(T0 + 600), at(T0 + 1_200)), "inside the outage");
        assert!(counter.capture_available_between(at(T0 + 1_200), at(T0 + 1_800)), "after it");
        assert!(counter.capture_available_between(at(T0 - 600), at(T0)), "before it");
    }

    #[test]
    fn missing_hooks_mean_capture_is_never_available() {
        let counter = InputCounter::new();
        counter.health.mark_unhealthy(1);
        assert!(!counter.hook_healthy());
        assert!(!counter.capture_available_between(at(T0), at(T0 + 60)));
    }

    const KEEP: WatchdogAction = WatchdogAction::Keep;
    const REINSTALL: WatchdogAction = WatchdogAction::Reinstall { missed_real_input: false };
    const DEAD: WatchdogAction = WatchdogAction::Reinstall { missed_real_input: true };

    #[test]
    fn the_watchdog_reinstalls_only_silent_hooks_while_windows_sees_input() {
        let limit = HOOK_SILENCE_LIMIT_MS;
        let now = T0_MS + 10 * limit;
        let long_ago = now - 2 * limit;
        // Silent for longer than the limit while the user typed a moment ago,
        // on the hooks' desktop all along.
        let silent = WatchdogCheck {
            now_ms: now,
            installed: true,
            last_hook_event_ms: long_ago,
            last_input_ms: Some(now - 5_000),
            own_desktop_since_ms: Some(long_ago),
            pointer_moved_ms: None,
            last_attempt_ms: long_ago,
        };
        assert_eq!(watchdog_action(&silent), REINSTALL);
        // The user is away: silence is expected.
        assert_eq!(watchdog_action(&WatchdogCheck { last_input_ms: Some(now - 10 * 60_000), ..silent }), KEEP);
        // The hooks delivered recently.
        assert_eq!(watchdog_action(&WatchdogCheck { last_hook_event_ms: now - 10_000, ..silent }), KEEP);
        // Windows will not say: no evidence, no action.
        assert_eq!(watchdog_action(&WatchdogCheck { last_input_ms: None, ..silent }), KEEP);
        // Missing hooks are retried — but at most once per limit.
        let missing = WatchdogCheck { installed: false, last_input_ms: None, ..silent };
        assert_eq!(watchdog_action(&missing), REINSTALL);
        assert_eq!(watchdog_action(&WatchdogCheck { last_attempt_ms: now - 60_000, ..missing }), KEEP);
        assert_eq!(watchdog_action(&WatchdogCheck { last_attempt_ms: now - 60_000, ..silent }), KEEP);
    }

    #[test]
    fn only_a_pointer_that_moved_unseen_proves_the_hooks_dead() {
        let limit = HOOK_SILENCE_LIMIT_MS;
        let now = T0_MS + 10 * limit;
        let last_event = now - 2 * limit;
        let silent = WatchdogCheck {
            now_ms: now,
            installed: true,
            last_hook_event_ms: last_event,
            last_input_ms: Some(now - 5_000),
            own_desktop_since_ms: Some(last_event - limit),
            pointer_moved_ms: None,
            last_attempt_ms: last_event,
        };
        // A mouse jiggler's "zen" mode: Windows' idle clock is reset by moves
        // of zero length, which no hook ever sees, and the pointer stays put.
        // Reinstalled to be safe, but the block stays measured (0% active)
        // rather than "not measured", which would hide the idle time.
        assert_eq!(watchdog_action(&silent), REINSTALL);
        // The pointer moved across the screen while the hooks saw nothing.
        assert_eq!(watchdog_action(&WatchdogCheck { pointer_moved_ms: Some(now - 30_000), ..silent }), DEAD);
        // A move in the interval that ended with their last event may be one
        // they saw.
        let check_after_last_event = last_event + 20_000;
        assert_eq!(watchdog_action(&WatchdogCheck { pointer_moved_ms: Some(check_after_last_event), ..silent }), REINSTALL);
        // No suspicion, no verdict.
        assert_eq!(watchdog_action(&WatchdogCheck { pointer_moved_ms: Some(now), last_hook_event_ms: now - 1_000, ..silent }), KEEP);
    }

    #[test]
    fn the_watchdog_ignores_input_the_hooks_could_never_see() {
        // Back from lunch: the screen was locked for an hour (the hooks saw
        // nothing), the password was typed on the secure desktop, and the
        // user has not touched anything since unlocking. Not a dead hook.
        let limit = HOOK_SILENCE_LIMIT_MS;
        let unlocked = T0_MS + 3_600_000;
        let typed_password = unlocked - 3_000;
        let first_check_after = unlocked + 10_000;
        let check = WatchdogCheck {
            now_ms: first_check_after,
            installed: true,
            last_hook_event_ms: T0_MS,
            last_input_ms: Some(typed_password),
            own_desktop_since_ms: Some(first_check_after),
            pointer_moved_ms: None,
            last_attempt_ms: T0_MS,
        };
        assert_eq!(watchdog_action(&check), KEEP);
        assert_eq!(watchdog_action(&WatchdogCheck { now_ms: first_check_after + 30_000, ..check }), KEEP);
        // Still on the lock screen: never.
        assert_eq!(watchdog_action(&WatchdogCheck { own_desktop_since_ms: None, ..check }), KEEP);
        // Input after a check had found the hooks' desktop, unseen by the
        // hooks for the whole limit: suspect.
        let later = first_check_after + limit;
        assert_eq!(watchdog_action(&WatchdogCheck { now_ms: later, last_input_ms: Some(later - 2_000), ..check }), REINSTALL);
    }

    #[test]
    fn the_own_desktop_run_starts_at_the_first_check_that_finds_it() {
        assert_eq!(own_desktop_since(None, true, 1_000), Some(1_000));
        assert_eq!(own_desktop_since(Some(1_000), true, 31_000), Some(1_000), "kept while it stays");
        assert_eq!(own_desktop_since(Some(1_000), false, 61_000), None, "locked");
        assert_eq!(own_desktop_since(None, false, 91_000), None);
        assert_eq!(own_desktop_since(None, true, 121_000), Some(121_000), "a new run after unlocking");
    }

    #[test]
    fn the_pointer_moved_only_between_two_known_positions() {
        assert!(pointer_moved(Some((10, 20)), Some((11, 20))));
        assert!(!pointer_moved(Some((10, 20)), Some((10, 20))));
        // Unknown on either side (another desktop had the input): no evidence.
        assert!(!pointer_moved(None, Some((10, 20))));
        assert!(!pointer_moved(Some((10, 20)), None));
    }

    #[test]
    fn the_last_real_input_ignores_injected_events() {
        let counter = feed(&[
            (RawInput::KeyDown(0x41), false, 2_500),
            (RawInput::KeyUp(0x41), false, 2_600),
            (RawInput::Move, false, 7_900),
            // A jiggler after that: not the user.
            (RawInput::Move, true, 60_000),
            (RawInput::KeyDown(0x42), true, 61_000),
        ]);
        assert_eq!(counter.last_input_at(), Some(at(T0 + 7)));
        assert_eq!(InputCounter::new().last_input_at(), None);
        // A click counts too.
        let clicked = feed(&[(RawInput::Move, false, 1_000), (RawInput::ButtonDown, false, 9_000)]);
        assert_eq!(clicked.last_input_at(), Some(at(T0 + 9)));
    }

    /// Needs an unlocked desktop session: `cargo test -- --ignored`. It
    /// moves the pointer one pixel right and back.
    #[cfg(windows)]
    #[test]
    #[ignore = "installs the real input hooks and nudges the pointer one pixel and back"]
    fn the_real_hooks_report_injected_input_as_injected() {
        use windows::Win32::UI::Input::KeyboardAndMouse::{SendInput, INPUT, INPUT_0, INPUT_MOUSE, MOUSEEVENTF_MOVE, MOUSEINPUT};

        let counter = Arc::new(InputCounter::new());
        spawn_listener(counter.clone());
        std::thread::sleep(Duration::from_millis(300));
        assert!(counter.hook_healthy(), "the hooks are installed");
        assert!(input_on_own_desktop(), "input goes to this desktop");
        assert!(system_idle_millis().is_some(), "Windows reports the idle time");

        let before = Utc::now() - chrono::Duration::seconds(1);
        // One pixel right and back: the pointer ends where it was. (A
        // zero-length move never reaches the hooks at all.)
        let nudge = |dx: i32| INPUT {
            r#type: INPUT_MOUSE,
            Anonymous: INPUT_0 {
                mi: MOUSEINPUT { dx, dy: 0, mouseData: 0, dwFlags: MOUSEEVENTF_MOVE, time: 0, dwExtraInfo: 0 },
            },
        };
        assert_eq!(unsafe { SendInput(&[nudge(1), nudge(-1)], std::mem::size_of::<INPUT>() as i32) }, 2);
        std::thread::sleep(Duration::from_millis(500));
        let after = Utc::now() + chrono::Duration::seconds(1);
        // Seen, and filed as injected (the user may be giving real input
        // meanwhile, so that is all that can be asserted here; the filter's
        // own tests show injected events never count).
        assert!(counter.injected_between(before, after, true, true) >= 2, "seen, as injected");
        assert!(counter.hook_healthy());
    }

    #[test]
    fn idle_time_survives_the_tick_count_wrapping() {
        assert_eq!(idle_millis(10_000, 4_000), 6_000);
        // GetLastInputInfo's 32-bit tick wrapped (every 49.7 days) after the input.
        let now = (1u64 << 32) + 100;
        let last = u32::MAX - 49;
        assert_eq!(idle_millis(now, last), 150);
        // Long after several wraps, the 64-bit count is still taken modulo 2^32.
        assert_eq!(idle_millis((5u64 << 32) + 1_000, 0), 1_000);
    }

    #[test]
    fn inactive_counter_drops_input_events_and_keeps_logs_empty() {
        let counter = InputCounter::new();
        assert!(!counter.is_active(), "InputCounter must default to inactive (opt-in)");
        let mut filter = InputFilter::default();
        let t0 = Utc::now();
        let t0_ms = t0.timestamp_millis();

        // Feed keyboard and mouse events into inactive counter
        filter.apply(&counter, RawInput::KeyDown(0x41), false, t0_ms);
        filter.apply(&counter, RawInput::ButtonDown, false, t0_ms + 100);
        filter.apply(&counter, RawInput::Move, false, t0_ms + 200);

        let t_end = t0 + chrono::Duration::seconds(5);
        assert_eq!(counter.key_presses_between(t0, t_end), Vec::<i64>::new());
        assert_eq!(counter.clicks_between(t0, t_end), Vec::<i64>::new());
        assert_eq!(counter.counts_between(t0, t_end), (0, 0));
        assert_eq!(counter.active_seconds(t0, t_end, true, true), Vec::<i64>::new());
    }

    #[test]
    fn activating_counter_accumulates_events_and_deactivating_halts_collection() {
        let counter = InputCounter::new();
        counter.set_permissions(true, true);
        counter.set_active(true);
        assert!(counter.is_active());
        let mut filter = InputFilter::default();
        let t0 = Utc::now();
        let t0_ms = t0.timestamp_millis();

        filter.apply(&counter, RawInput::KeyDown(0x41), false, t0_ms);
        filter.apply(&counter, RawInput::ButtonDown, false, t0_ms + 100);

        let t_mid = t0 + chrono::Duration::seconds(5);
        assert_eq!(counter.counts_between(t0, t_mid), (1, 1));

        // Deactivate (e.g. paused)
        counter.set_active(false);
        filter.apply(&counter, RawInput::KeyDown(0x42), false, t0_ms + 6_000);
        filter.apply(&counter, RawInput::ButtonDown, false, t0_ms + 6_100);

        let t_end = t0 + chrono::Duration::seconds(10);
        // Still only 1 key press and 1 click recorded; paused events dropped
        assert_eq!(counter.counts_between(t0, t_end), (1, 1));
    }

    #[test]
    fn inactive_filter_does_not_update_health_or_key_down_state() {
        let counter = InputCounter::new();
        assert!(!counter.is_active());
        let mut filter = InputFilter::default();
        let t0_ms = 1_000_000;

        // Apply key down while inactive
        filter.apply(&counter, RawInput::KeyDown(0x41), false, t0_ms);
        assert_eq!(counter.health.last_event_ms(), 0, "health must not record events while inactive");
        assert_eq!(filter.key_down_at[0x41], 0, "key_down_at must not track keys while inactive");

        // Now activate and apply
        counter.set_permissions(true, true);
        counter.set_active(true);
        filter.apply(&counter, RawInput::KeyDown(0x41), false, t0_ms + 10);
        assert_eq!(counter.health.last_event_ms(), t0_ms + 10);
        assert_eq!(filter.key_down_at[0x41], t0_ms + 10);

        // Reset clears held keys
        filter.reset();
        assert_eq!(filter.key_down_at[0x41], 0);
    }

    #[test]
    fn per_sensor_permission_gating_in_filter() {
        let counter = InputCounter::new();
        counter.set_active(true);
        // Keyboard allowed, mouse denied
        counter.set_permissions(true, false);
        let mut filter = InputFilter::default();
        let t0 = Utc::now();
        let t0_ms = t0.timestamp_millis();

        filter.apply(&counter, RawInput::KeyDown(0x41), false, t0_ms);
        filter.apply(&counter, RawInput::ButtonDown, false, t0_ms + 50);
        filter.apply(&counter, RawInput::Move, false, t0_ms + 60);

        let t_end = t0 + chrono::Duration::seconds(5);
        assert_eq!(counter.counts_between(t0, t_end), (1, 0), "keyboard should record, mouse should drop");

        // Switch: Mouse allowed, keyboard denied
        counter.set_permissions(false, true);
        filter.apply(&counter, RawInput::KeyDown(0x42), false, t0_ms + 100);
        filter.apply(&counter, RawInput::ButtonDown, false, t0_ms + 150);

        assert_eq!(counter.counts_between(t0, t_end), (1, 1), "second key dropped, mouse click recorded");
    }
}
