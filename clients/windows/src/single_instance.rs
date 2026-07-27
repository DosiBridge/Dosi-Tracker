//! Single-instance enforcement.
//!
//! Critical for correctness, not just polish: the installer both enables
//! autostart *and* creates a Start Menu shortcut, so it is easy to end up with
//! two agents running. Each would capture independently and enqueue activities
//! with its **own** `clientActivityId`, which the backend cannot deduplicate —
//! producing double-counted (and therefore over-billed) tracked time.

/// Held for the lifetime of the process; releasing it frees the name.
pub struct InstanceGuard {
    #[cfg(windows)]
    _handle: windows::Win32::Foundation::HANDLE,
}

#[cfg(windows)]
const MUTEX_NAME: &str = "Local\\DosiTracker.SingleInstance";

/// Try to become the one running instance.
///
/// Returns `None` if another instance already holds the name — the caller should
/// surface that instance's window and exit.
#[cfg(windows)]
pub fn acquire() -> Option<InstanceGuard> {
    use windows::core::HSTRING;
    use windows::Win32::Foundation::{GetLastError, ERROR_ALREADY_EXISTS};
    use windows::Win32::System::Threading::CreateMutexW;

    unsafe {
        let handle = CreateMutexW(None, true, &HSTRING::from(MUTEX_NAME)).ok()?;

        // The handle is returned even when the name already existed, so the
        // last-error code is what actually distinguishes the two cases.
        if GetLastError() == ERROR_ALREADY_EXISTS {
            let _ = windows::Win32::Foundation::CloseHandle(handle);
            return None;
        }

        Some(InstanceGuard { _handle: handle })
    }
}

/// Bring the already-running instance's window to the foreground, so clicking
/// the shortcut a second time feels like "focus the app" rather than doing
/// nothing.
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
pub fn focus_existing(_window_title: &str) {}
