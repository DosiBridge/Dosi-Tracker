//! Single-instance enforcement, and talking to the instance that runs.
//!
//! Critical for correctness, not just polish: the installer both enables
//! autostart *and* creates a Start Menu shortcut, so it is easy to end up with
//! two agents running. Each would capture independently and enqueue activities
//! with its **own** `clientActivityId`, which the backend cannot deduplicate —
//! producing double-counted (and therefore over-billed) tracked time.
//!
//! A second launch does not run: it asks the running instance to show its
//! window ([`Signal::Show`]) and exits. `dosi-tracker --quit` asks it to quit
//! gracefully — saving the block in progress — and waits until it is gone;
//! the installer runs it before replacing the program. Both requests are
//! named events in the session's namespace that the running instance waits on.

use std::time::Duration;

/// Command-line flag: ask the running instance to quit, wait for it, exit.
pub const QUIT_FLAG: &str = "--quit";

/// Was this process started to make the running instance quit?
pub fn quit_requested(args: &[String]) -> bool {
    args.iter().any(|arg| arg == QUIT_FLAG)
}

/// A request to the running instance.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Signal {
    /// Show the window (the app was launched again).
    Show,
    /// Quit gracefully.
    Quit,
}

#[cfg(windows)]
const MUTEX_NAME: &str = "Local\\DosiTracker.SingleInstance";
#[cfg(windows)]
const SHOW_EVENT: &str = "Local\\DosiTracker.Show";
#[cfg(windows)]
const QUIT_EVENT: &str = "Local\\DosiTracker.Quit";

#[cfg(windows)]
fn event_name(signal: Signal) -> &'static str {
    match signal {
        Signal::Show => SHOW_EVENT,
        Signal::Quit => QUIT_EVENT,
    }
}

/// Held for the lifetime of the process; the OS releases the name (and the
/// events) when the process ends.
pub struct InstanceGuard {
    #[cfg(windows)]
    _mutex: windows::Win32::Foundation::HANDLE,
    /// The events other launches signal, `[show, quit]` (raw handle values,
    /// so the listener thread can wait on them).
    #[cfg(windows)]
    events: Option<[isize; 2]>,
}

/// Try to become the one running instance.
///
/// Returns `None` if another instance already holds the name — the caller
/// should [`signal`] it and exit.
#[cfg(windows)]
pub fn acquire() -> Option<InstanceGuard> {
    use windows::core::HSTRING;
    use windows::Win32::Foundation::{CloseHandle, GetLastError, BOOL, ERROR_ALREADY_EXISTS};
    use windows::Win32::System::Threading::{CreateEventW, CreateMutexW};

    unsafe {
        let mutex = CreateMutexW(None, true, &HSTRING::from(MUTEX_NAME)).ok()?;

        // The handle is returned even when the name already existed, so the
        // last-error code is what actually distinguishes the two cases.
        if GetLastError() == ERROR_ALREADY_EXISTS {
            let _ = CloseHandle(mutex);
            return None;
        }

        // Auto-reset: each signal wakes the listener once.
        let event = |name: &str| CreateEventW(None, BOOL(0), BOOL(0), &HSTRING::from(name)).ok();
        let events = match (event(SHOW_EVENT), event(QUIT_EVENT)) {
            (Some(show), Some(quit)) => Some([show.0 as isize, quit.0 as isize]),
            (show, quit) => {
                tracing::warn!("could not create the instance events; another launch cannot show or quit this one");
                for handle in [show, quit].into_iter().flatten() {
                    let _ = CloseHandle(handle);
                }
                None
            }
        };
        Some(InstanceGuard { _mutex: mutex, events })
    }
}

#[cfg(windows)]
impl InstanceGuard {
    /// Call `handler` (on a background thread) whenever another launch
    /// signals this instance.
    pub fn listen(&self, handler: impl Fn(Signal) + Send + 'static) {
        use windows::Win32::Foundation::{BOOL, HANDLE, WAIT_OBJECT_0};
        use windows::Win32::System::Threading::{WaitForMultipleObjects, INFINITE};

        let Some(events) = self.events else {
            return;
        };
        let spawned = std::thread::Builder::new().name("instance-signals".into()).spawn(move || {
            let handles = events.map(|handle| HANDLE(handle as *mut core::ffi::c_void));
            loop {
                let woke = unsafe { WaitForMultipleObjects(&handles, BOOL(0), INFINITE) };
                match woke.0.wrapping_sub(WAIT_OBJECT_0.0) {
                    0 => handler(Signal::Show),
                    1 => handler(Signal::Quit),
                    _ => {
                        tracing::error!(?woke, "waiting for instance signals failed");
                        return;
                    }
                }
            }
        });
        if let Err(e) = spawned {
            tracing::error!(?e, "could not start the instance signal listener");
        }
    }
}

/// Signal the running instance. `false` when none listens (nothing runs, or
/// an older build that does not know the signals).
#[cfg(windows)]
pub fn signal(signal: Signal) -> bool {
    set_event(event_name(signal))
}

/// Set the named event, if it exists.
#[cfg(windows)]
fn set_event(name: &str) -> bool {
    use windows::core::HSTRING;
    use windows::Win32::Foundation::{CloseHandle, BOOL};
    use windows::Win32::System::Threading::{OpenEventW, SetEvent, EVENT_MODIFY_STATE};

    unsafe {
        let Ok(event) = OpenEventW(EVENT_MODIFY_STATE, BOOL(0), &HSTRING::from(name)) else {
            return false;
        };
        let signalled = SetEvent(event).is_ok();
        let _ = CloseHandle(event);
        signalled
    }
}

/// Wait until no instance runs any more (its single-instance mutex is
/// released or abandoned when it exits), at most `timeout`. `true` once it
/// is gone.
#[cfg(windows)]
pub fn wait_until_gone(timeout: Duration) -> bool {
    wait_released(MUTEX_NAME, timeout)
}

/// Wait until the named mutex is released (or abandoned by its owner
/// exiting), at most `timeout`; `true` at once when it does not exist.
#[cfg(windows)]
fn wait_released(name: &str, timeout: Duration) -> bool {
    use windows::core::HSTRING;
    use windows::Win32::Foundation::{CloseHandle, BOOL, WAIT_ABANDONED, WAIT_OBJECT_0};
    use windows::Win32::System::Threading::{OpenMutexW, ReleaseMutex, WaitForSingleObject, SYNCHRONIZATION_SYNCHRONIZE};

    unsafe {
        let Ok(mutex) = OpenMutexW(SYNCHRONIZATION_SYNCHRONIZE, BOOL(0), &HSTRING::from(name)) else {
            // No such name: nothing runs.
            return true;
        };
        let millis = u32::try_from(timeout.as_millis()).unwrap_or(u32::MAX - 1);
        let woke = WaitForSingleObject(mutex, millis);
        let gone = woke == WAIT_OBJECT_0 || woke == WAIT_ABANDONED;
        if gone {
            // Owning it now: let go at once, so a new instance can start.
            let _ = ReleaseMutex(mutex);
        }
        let _ = CloseHandle(mutex);
        gone
    }
}

/// Bring an already-running instance's window to the foreground by its
/// title — for a running build too old to listen for [`Signal::Show`].
#[cfg(windows)]
pub fn focus_existing(window_title: &str) {
    use windows::core::HSTRING;
    use windows::Win32::UI::WindowsAndMessaging::{
        FindWindowW, SetForegroundWindow, ShowWindow, SW_RESTORE, SW_SHOW,
    };

    unsafe {
        if let Ok(hwnd) = FindWindowW(None, &HSTRING::from(window_title)) {
            if !hwnd.0.is_null() {
                // The window may be hidden in the tray, so show *and* restore it.
                let _ = ShowWindow(hwnd, SW_SHOW);
                let _ = ShowWindow(hwnd, SW_RESTORE);
                let _ = SetForegroundWindow(hwnd);
            }
        }
    }
}

#[cfg(not(windows))]
pub fn acquire() -> Option<InstanceGuard> {
    Some(InstanceGuard {})
}

#[cfg(not(windows))]
impl InstanceGuard {
    pub fn listen(&self, _handler: impl Fn(Signal) + Send + 'static) {}
}

#[cfg(not(windows))]
pub fn signal(_signal: Signal) -> bool {
    false
}

#[cfg(not(windows))]
pub fn wait_until_gone(_timeout: Duration) -> bool {
    true
}

#[cfg(not(windows))]
pub fn focus_existing(_window_title: &str) {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_quit_flag_is_recognised_anywhere_on_the_command_line() {
        let args = |list: &[&str]| list.iter().map(|a| a.to_string()).collect::<Vec<_>>();
        assert!(quit_requested(&args(&["--quit"])));
        assert!(quit_requested(&args(&["--autostart", "--quit"])));
        assert!(!quit_requested(&args(&["--autostart"])));
        assert!(!quit_requested(&args(&["--quit-now"])));
        assert!(!quit_requested(&[]));
    }

    /// Private names, so a real running tracker is never disturbed.
    #[cfg(windows)]
    fn private_name(kind: &str) -> String {
        format!("Local\\DosiTracker.Test.{kind}.{}", uuid::Uuid::new_v4())
    }

    #[cfg(windows)]
    #[test]
    fn signals_reach_the_listener_and_a_missing_one_is_reported() {
        use windows::core::HSTRING;
        use windows::Win32::Foundation::BOOL;
        use windows::Win32::System::Threading::{CreateEventW, CreateMutexW};

        let (show, quit, mutex) = (private_name("Show"), private_name("Quit"), private_name("Mutex"));
        assert!(!set_event(&show), "nobody listens yet");
        let guard = unsafe {
            InstanceGuard {
                _mutex: CreateMutexW(None, true, &HSTRING::from(mutex.as_str())).unwrap(),
                events: Some([
                    CreateEventW(None, BOOL(0), BOOL(0), &HSTRING::from(show.as_str())).unwrap().0 as isize,
                    CreateEventW(None, BOOL(0), BOOL(0), &HSTRING::from(quit.as_str())).unwrap().0 as isize,
                ]),
            }
        };
        let (tx, rx) = std::sync::mpsc::channel();
        guard.listen(move |signal| {
            let _ = tx.send(signal);
        });

        assert!(set_event(&show));
        assert_eq!(rx.recv_timeout(Duration::from_secs(2)), Ok(Signal::Show));
        assert!(set_event(&quit));
        assert_eq!(rx.recv_timeout(Duration::from_secs(2)), Ok(Signal::Quit));
        // Each signal wakes the listener once.
        assert!(rx.recv_timeout(Duration::from_millis(100)).is_err());
    }

    #[cfg(windows)]
    #[test]
    fn quitting_waits_until_the_instance_has_exited() {
        use windows::core::HSTRING;
        use windows::Win32::System::Threading::CreateMutexW;

        let name = private_name("Mutex");
        assert!(wait_released(&name, Duration::from_millis(10)), "nothing runs under that name");

        // An "instance" that holds the name for a while, then exits without
        // releasing it (abandoned, as when the process ends).
        let (ready_tx, ready_rx) = std::sync::mpsc::channel();
        let held = name.clone();
        let instance = std::thread::spawn(move || {
            let mutex = unsafe { CreateMutexW(None, true, &HSTRING::from(held.as_str())) }.unwrap();
            ready_tx.send(()).unwrap();
            std::thread::sleep(Duration::from_millis(400));
            mutex.0 as isize
        });
        ready_rx.recv().unwrap();
        assert!(!wait_released(&name, Duration::from_millis(50)), "still running");
        let started = std::time::Instant::now();
        assert!(wait_released(&name, Duration::from_secs(5)), "gone once its thread ended");
        assert!(started.elapsed() < Duration::from_secs(4));
        let handle = instance.join().unwrap();
        unsafe {
            let _ = windows::Win32::Foundation::CloseHandle(windows::Win32::Foundation::HANDLE(handle as *mut core::ffi::c_void));
        }
    }
}
