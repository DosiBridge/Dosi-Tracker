//! Windows session and power events — the screen locking, the machine going
//! to sleep, Windows shutting down — passed to the worker (CONTRACT §4).
//!
//! A hidden window on its own thread receives them:
//!
//! - `WM_WTSSESSION_CHANGE` (registered with `WTSRegisterSessionNotification`):
//!   the session locked or unlocked, or was disconnected (fast user switching,
//!   Remote Desktop) and reconnected.
//! - `WM_POWERBROADCAST`: `PBT_APMSUSPEND` before sleep or hibernation, and
//!   the resume notifications after it.
//! - `WM_QUERYENDSESSION` / `WM_ENDSESSION`: shut down, restart or sign out.
//!
//! It is a hidden *top-level* window, not a message-only (`HWND_MESSAGE`)
//! one: message-only windows never receive broadcasts, and the power and
//! end-session messages are broadcast to top-level windows.
//!
//! Before sleep and when the session ends, Windows waits only briefly for the
//! window to answer, so the window holds its answer until the worker reports
//! the block in progress saved locally — at most [`SystemEvent::ack_budget`].
//! While the session is ending, `ShutdownBlockReasonCreate` tells Windows (and
//! the user, should it take longer) what the app is doing.

use std::time::Duration;

use chrono::{DateTime, Utc};
use tokio::sync::mpsc::UnboundedSender;

/// Something that happened to the session or the machine.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SystemEvent {
    /// The session was locked (Win+L, the screen saver, closing the lid).
    Locked,
    Unlocked,
    /// The session left the console or a Remote Desktop connection (fast
    /// user switching): nobody is at it.
    Disconnected,
    Connected,
    /// The machine is about to sleep or hibernate (`PBT_APMSUSPEND`).
    Suspending,
    /// The machine woke up.
    Resumed,
    /// Windows is shutting down, restarting or signing the user out.
    SessionEnding,
    /// A shutdown that was announced did not happen (another app vetoed it).
    SessionContinues,
}

impl SystemEvent {
    /// How long the window waits for the worker to save the block in
    /// progress before answering Windows; `None` for events nobody waits on.
    /// Windows allows about two seconds for `PBT_APMSUSPEND`.
    pub fn ack_budget(self) -> Option<Duration> {
        match self {
            SystemEvent::Suspending => Some(Duration::from_millis(1_500)),
            SystemEvent::SessionEnding => Some(Duration::from_millis(2_000)),
            _ => None,
        }
    }
}

/// An event on its way to the worker, with when it happened.
#[derive(Debug)]
pub struct SystemNotice {
    pub event: SystemEvent,
    pub at: DateTime<Utc>,
    /// Signalled once the block in progress is saved; the window is waiting
    /// on it before it lets Windows go on.
    pub done: Option<std::sync::mpsc::SyncSender<()>>,
}

impl SystemNotice {
    /// Tell the waiting window the work is done.
    pub fn acknowledge(&mut self) {
        if let Some(done) = self.done.take() {
            let _ = done.try_send(());
        }
    }
}

// Window messages and their codes (winuser.h, wtsapi32.h, pbt.h).
const WM_QUERYENDSESSION: u32 = 0x0011;
const WM_ENDSESSION: u32 = 0x0016;
const WM_TIMER: u32 = 0x0113;
const WM_POWERBROADCAST: u32 = 0x0218;
const WM_WTSSESSION_CHANGE: u32 = 0x02B1;
const WTS_CONSOLE_CONNECT: usize = 0x1;
const WTS_CONSOLE_DISCONNECT: usize = 0x2;
const WTS_REMOTE_CONNECT: usize = 0x3;
const WTS_REMOTE_DISCONNECT: usize = 0x4;
const WTS_SESSION_LOCK: usize = 0x7;
const WTS_SESSION_UNLOCK: usize = 0x8;
const PBT_APMSUSPEND: usize = 0x4;
const PBT_APMRESUMECRITICAL: usize = 0x6;
const PBT_APMRESUMESUSPEND: usize = 0x7;
const PBT_APMRESUMEAUTOMATIC: usize = 0x12;

/// What a window message means for tracking, if anything.
pub fn translate(message: u32, wparam: usize) -> Option<SystemEvent> {
    match (message, wparam) {
        (WM_WTSSESSION_CHANGE, WTS_SESSION_LOCK) => Some(SystemEvent::Locked),
        (WM_WTSSESSION_CHANGE, WTS_SESSION_UNLOCK) => Some(SystemEvent::Unlocked),
        (WM_WTSSESSION_CHANGE, WTS_CONSOLE_DISCONNECT | WTS_REMOTE_DISCONNECT) => Some(SystemEvent::Disconnected),
        (WM_WTSSESSION_CHANGE, WTS_CONSOLE_CONNECT | WTS_REMOTE_CONNECT) => Some(SystemEvent::Connected),
        (WM_POWERBROADCAST, PBT_APMSUSPEND) => Some(SystemEvent::Suspending),
        // After a wake Windows always sends RESUMEAUTOMATIC, and RESUMESUSPEND
        // as well when a user is there; either one will do.
        (WM_POWERBROADCAST, PBT_APMRESUMEAUTOMATIC | PBT_APMRESUMESUSPEND | PBT_APMRESUMECRITICAL) => {
            Some(SystemEvent::Resumed)
        }
        (WM_QUERYENDSESSION, _) => Some(SystemEvent::SessionEnding),
        // wParam is whether the session really ends.
        (WM_ENDSESSION, 0) => Some(SystemEvent::SessionContinues),
        (WM_ENDSESSION, _) => Some(SystemEvent::SessionEnding),
        _ => None,
    }
}

/// The running listener.
pub struct Listener {
    /// The hidden window (`HWND` as an integer: window handles may be used
    /// from any thread, the raw pointer type just does not say so).
    #[cfg_attr(not(test), allow(dead_code))]
    hwnd: isize,
}

/// Start the listener thread and its hidden window; `notices` receives the
/// events. Fails when the window cannot be created — tracking then goes on
/// without lock, sleep and shutdown events (idle detection and the missed-
/// suspend check still apply).
#[cfg(windows)]
pub fn spawn(notices: UnboundedSender<SystemNotice>) -> anyhow::Result<Listener> {
    let (ready_tx, ready_rx) = std::sync::mpsc::channel();
    std::thread::Builder::new()
        .name("session-events".into())
        .spawn(move || window::run(notices, ready_tx))?;
    match ready_rx.recv_timeout(Duration::from_secs(10)) {
        Ok(Ok(hwnd)) => Ok(Listener { hwnd }),
        Ok(Err(e)) => Err(anyhow::anyhow!("could not create the session events window: {e}")),
        Err(_) => Err(anyhow::anyhow!("the session events window did not start")),
    }
}

#[cfg(not(windows))]
pub fn spawn(_notices: UnboundedSender<SystemNotice>) -> anyhow::Result<Listener> {
    Err(anyhow::anyhow!("session events are only available on Windows"))
}

#[cfg(windows)]
mod window {
    use std::cell::{Cell, RefCell};

    use chrono::Utc;
    use tokio::sync::mpsc::UnboundedSender;
    use windows::core::{w, PCWSTR};
    use windows::Win32::Foundation::{BOOL, HINSTANCE, HWND, LPARAM, LRESULT, WPARAM};
    use windows::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows::Win32::UI::WindowsAndMessaging::{
        CreateWindowExW, DefWindowProcW, DispatchMessageW, GetMessageW, KillTimer, RegisterClassW, SetTimer,
        TranslateMessage, HMENU, MSG, WNDCLASSW, WS_EX_TOOLWINDOW, WS_POPUP,
    };

    use super::{translate, SystemEvent, SystemNotice, WM_ENDSESSION, WM_POWERBROADCAST, WM_QUERYENDSESSION, WM_TIMER};

    // Not in the `windows` features this crate enables (they would pull in
    // whole API families for four functions); declared from their DLLs.
    #[link(name = "wtsapi32", kind = "raw-dylib")]
    extern "system" {
        fn WTSRegisterSessionNotification(hwnd: HWND, flags: u32) -> BOOL;
        fn WTSUnRegisterSessionNotification(hwnd: HWND) -> BOOL;
    }
    #[link(name = "user32", kind = "raw-dylib")]
    extern "system" {
        fn ShutdownBlockReasonCreate(hwnd: HWND, reason: PCWSTR) -> BOOL;
        fn ShutdownBlockReasonDestroy(hwnd: HWND) -> BOOL;
    }

    /// `WTSRegisterSessionNotification`: this session's events only.
    const NOTIFY_FOR_THIS_SESSION: u32 = 0;

    /// Right after sign-in the Remote Desktop service may not be up yet and
    /// the registration fails; it is retried this often, this many times.
    const REGISTER_RETRY_TIMER: usize = 1;
    const REGISTER_RETRY_MS: u32 = 5_000;
    const REGISTER_ATTEMPTS: u32 = 24;

    thread_local! {
        /// Where the window procedure sends events (it gets no user data).
        static SINK: RefCell<Option<UnboundedSender<SystemNotice>>> = const { RefCell::new(None) };
        static REGISTER_TRIES: Cell<u32> = const { Cell::new(0) };
    }

    pub fn run(notices: UnboundedSender<SystemNotice>, ready: std::sync::mpsc::Sender<Result<isize, String>>) {
        SINK.with(|sink| *sink.borrow_mut() = Some(notices));
        let hwnd = match create() {
            Ok(hwnd) => hwnd,
            Err(e) => {
                tracing::error!(?e, "could not create the session events window");
                let _ = ready.send(Err(e.to_string()));
                return;
            }
        };
        register(hwnd);
        let _ = ready.send(Ok(hwnd.0 as isize));

        let mut message = MSG::default();
        // 0 is WM_QUIT, -1 an error.
        while unsafe { GetMessageW(&mut message, HWND::default(), 0, 0) }.0 > 0 {
            unsafe {
                let _ = TranslateMessage(&message);
                DispatchMessageW(&message);
            }
        }
        let _ = unsafe { WTSUnRegisterSessionNotification(hwnd) };
        tracing::warn!("the session events window stopped");
    }

    fn create() -> windows::core::Result<HWND> {
        let class_name = w!("DosiTracker.SessionEvents");
        unsafe {
            let instance = HINSTANCE(GetModuleHandleW(PCWSTR::null())?.0);
            let class = WNDCLASSW {
                lpfnWndProc: Some(window_proc),
                hInstance: instance,
                lpszClassName: class_name,
                ..Default::default()
            };
            // Fails harmlessly when the class exists already (a second
            // listener in the same process, as in the tests).
            RegisterClassW(&class);
            // Never shown; a tool window stays out of the taskbar and Alt+Tab
            // even if something did show it.
            CreateWindowExW(
                WS_EX_TOOLWINDOW,
                class_name,
                w!("Dosi Tracker session events"),
                WS_POPUP,
                0,
                0,
                0,
                0,
                HWND::default(),
                HMENU::default(),
                instance,
                None,
            )
        }
    }

    /// Ask for lock/unlock notifications, retrying on a timer while the
    /// service is not up yet.
    fn register(hwnd: HWND) {
        let attempt = REGISTER_TRIES.with(|tries| {
            tries.set(tries.get() + 1);
            tries.get()
        });
        if unsafe { WTSRegisterSessionNotification(hwnd, NOTIFY_FOR_THIS_SESSION) }.as_bool() {
            if attempt > 1 {
                unsafe {
                    let _ = KillTimer(hwnd, REGISTER_RETRY_TIMER);
                }
                tracing::info!(attempt, "lock notifications registered");
            }
            return;
        }
        let error = windows::core::Error::from_win32();
        if attempt == 1 {
            unsafe { SetTimer(hwnd, REGISTER_RETRY_TIMER, REGISTER_RETRY_MS, None) };
        }
        if attempt >= REGISTER_ATTEMPTS {
            unsafe {
                let _ = KillTimer(hwnd, REGISTER_RETRY_TIMER);
            }
            tracing::error!(?error, "lock notifications are unavailable; locked time is cut by idle detection only");
        } else {
            tracing::warn!(?error, attempt, "could not register for lock notifications yet; retrying");
        }
    }

    unsafe extern "system" fn window_proc(hwnd: HWND, message: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
        if message == WM_TIMER && wparam.0 == REGISTER_RETRY_TIMER {
            register(hwnd);
            return LRESULT(0);
        }
        if let Some(event) = translate(message, wparam.0) {
            let ending = event == SystemEvent::SessionEnding;
            if ending {
                let _ = unsafe { ShutdownBlockReasonCreate(hwnd, w!("Saving your tracked time…")) };
            }
            deliver(event);
            if ending {
                let _ = unsafe { ShutdownBlockReasonDestroy(hwnd) };
            }
            match message {
                // TRUE: the session may end / the machine may sleep.
                WM_QUERYENDSESSION | WM_POWERBROADCAST => return LRESULT(1),
                WM_ENDSESSION => return LRESULT(0),
                _ => {}
            }
        }
        unsafe { DefWindowProcW(hwnd, message, wparam, lparam) }
    }

    /// Hand the event to the worker; for a suspend or the session ending,
    /// wait (briefly) until the worker has saved the block in progress.
    fn deliver(event: SystemEvent) {
        let at = Utc::now();
        let (done, waiter) = match event.ack_budget() {
            Some(budget) => {
                let (done, waiter) = std::sync::mpsc::sync_channel(1);
                (Some(done), Some((waiter, budget)))
            }
            None => (None, None),
        };
        let sent = SINK.with(|sink| {
            sink.borrow()
                .as_ref()
                .is_some_and(|notices| notices.send(SystemNotice { event, at, done }).is_ok())
        });
        if let (true, Some((waiter, budget))) = (sent, waiter) {
            if waiter.recv_timeout(budget).is_err() {
                tracing::warn!(?event, ?budget, "the block in progress was not saved in time");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_messages_map_to_events() {
        assert_eq!(translate(WM_WTSSESSION_CHANGE, WTS_SESSION_LOCK), Some(SystemEvent::Locked));
        assert_eq!(translate(WM_WTSSESSION_CHANGE, WTS_SESSION_UNLOCK), Some(SystemEvent::Unlocked));
        assert_eq!(translate(WM_WTSSESSION_CHANGE, WTS_CONSOLE_DISCONNECT), Some(SystemEvent::Disconnected));
        assert_eq!(translate(WM_WTSSESSION_CHANGE, WTS_REMOTE_DISCONNECT), Some(SystemEvent::Disconnected));
        assert_eq!(translate(WM_WTSSESSION_CHANGE, WTS_CONSOLE_CONNECT), Some(SystemEvent::Connected));
        assert_eq!(translate(WM_WTSSESSION_CHANGE, WTS_REMOTE_CONNECT), Some(SystemEvent::Connected));
        assert_eq!(translate(WM_WTSSESSION_CHANGE, 0x5), None, "a logon is not a lock");
    }

    #[test]
    fn power_and_end_session_messages_map_to_events() {
        assert_eq!(translate(WM_POWERBROADCAST, PBT_APMSUSPEND), Some(SystemEvent::Suspending));
        for resume in [PBT_APMRESUMEAUTOMATIC, PBT_APMRESUMESUSPEND, PBT_APMRESUMECRITICAL] {
            assert_eq!(translate(WM_POWERBROADCAST, resume), Some(SystemEvent::Resumed));
        }
        assert_eq!(translate(WM_POWERBROADCAST, 0x8013), None, "power setting changes are not tracked");
        assert_eq!(translate(WM_QUERYENDSESSION, 0), Some(SystemEvent::SessionEnding));
        assert_eq!(translate(WM_ENDSESSION, 1), Some(SystemEvent::SessionEnding));
        assert_eq!(translate(WM_ENDSESSION, 0), Some(SystemEvent::SessionContinues));
        assert_eq!(translate(0x0010, 0), None, "WM_CLOSE");
        assert_eq!(SystemEvent::Suspending.ack_budget(), Some(Duration::from_millis(1_500)));
        assert!(SystemEvent::SessionEnding.ack_budget().unwrap() <= Duration::from_secs(2));
        assert_eq!(SystemEvent::Locked.ack_budget(), None);
    }

    /// Send `message` to the listener's window as Windows would, from its own
    /// thread (`SendMessageW` waits for the answer).
    #[cfg(windows)]
    fn send(listener: &Listener, message: u32, wparam: usize) -> std::thread::JoinHandle<isize> {
        use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
        use windows::Win32::UI::WindowsAndMessaging::SendMessageW;
        let hwnd = listener.hwnd;
        std::thread::spawn(move || unsafe { SendMessageW(HWND(hwnd as *mut core::ffi::c_void), message, WPARAM(wparam), LPARAM(0)) }.0)
    }

    #[cfg(windows)]
    #[test]
    fn the_hidden_window_forwards_events_and_waits_for_the_block_to_be_saved() {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let listener = spawn(tx).expect("the window is created");

        // A lock is passed on; nobody waits for it.
        let before = Utc::now();
        assert_eq!(send(&listener, WM_WTSSESSION_CHANGE, WTS_SESSION_LOCK).join().unwrap(), 0);
        let notice = rx.try_recv().expect("delivered before SendMessage returned");
        assert_eq!(notice.event, SystemEvent::Locked);
        assert!(notice.at >= before && notice.done.is_none());

        // The session ending: Windows gets its answer (TRUE) once the worker
        // acknowledges, well before the budget runs out.
        let started = std::time::Instant::now();
        let answer = send(&listener, WM_QUERYENDSESSION, 0);
        let mut notice = rx.blocking_recv().expect("delivered");
        assert_eq!(notice.event, SystemEvent::SessionEnding);
        notice.acknowledge();
        assert_eq!(answer.join().unwrap(), 1, "the session may end");
        assert!(started.elapsed() < Duration::from_millis(1_500), "answered on the acknowledgement");

        // Nobody acknowledging does not hang Windows: it is answered when the
        // budget runs out.
        let started = std::time::Instant::now();
        let answer = send(&listener, WM_POWERBROADCAST, PBT_APMSUSPEND);
        let unacknowledged = rx.blocking_recv().expect("delivered");
        assert_eq!(unacknowledged.event, SystemEvent::Suspending);
        assert_eq!(answer.join().unwrap(), 1);
        assert!(started.elapsed() >= Duration::from_millis(1_400));
        drop(unacknowledged);
    }
}
