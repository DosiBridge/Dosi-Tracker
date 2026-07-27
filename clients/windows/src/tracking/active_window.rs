use crate::model::WindowInfo;

/// Resolve the executable file name (e.g. `code.exe`) that owns a process id.
#[cfg(windows)]
fn app_name_for_pid(pid: u32) -> String {
    use windows::Win32::Foundation::{BOOL, CloseHandle, HMODULE, MAX_PATH};
    use windows::Win32::System::ProcessStatus::GetModuleFileNameExW;
    use windows::Win32::System::Threading::{
        OpenProcess, PROCESS_QUERY_INFORMATION, PROCESS_VM_READ,
    };

    if pid == 0 {
        return String::new();
    }

    unsafe {
        let mut app_name = String::new();
        // BOOL(0)/HMODULE::default() are the concrete forms accepted by the pinned
        // windows 0.58 `Param`-based signatures (bare `false`/`Some`/`None` are not).
        if let Ok(handle) = OpenProcess(PROCESS_QUERY_INFORMATION | PROCESS_VM_READ, BOOL(0), pid) {
            let mut path_buf = [0u16; MAX_PATH as usize];
            let n = GetModuleFileNameExW(handle, HMODULE::default(), &mut path_buf);
            if n > 0 {
                let full = String::from_utf16_lossy(&path_buf[..n as usize]);
                app_name = full
                    .rsplit(['\\', '/'])
                    .next()
                    .unwrap_or(&full)
                    .to_string();
            }
            let _ = CloseHandle(handle);
        }
        app_name
    }
}

/// Return the currently focused window (app + title), if any.
#[cfg(windows)]
pub fn current() -> Option<WindowInfo> {
    use windows::Win32::Foundation::HWND;
    use windows::Win32::UI::WindowsAndMessaging::{
        GetForegroundWindow, GetWindowTextW, GetWindowThreadProcessId,
    };

    unsafe {
        let hwnd: HWND = GetForegroundWindow();
        if hwnd.0.is_null() {
            return None;
        }

        // Window title
        let mut title_buf = [0u16; 512];
        let len = GetWindowTextW(hwnd, &mut title_buf);
        let window_title = String::from_utf16_lossy(&title_buf[..len as usize]);

        // Owning process executable name
        let mut pid: u32 = 0;
        GetWindowThreadProcessId(hwnd, Some(&mut pid));

        Some(WindowInfo {
            app_name: app_name_for_pid(pid),
            window_title,
        })
    }
}

#[cfg(not(windows))]
pub fn current() -> Option<WindowInfo> {
    None
}

/// Enumerate top-level visible windows with a title, deduplicated by owning
/// process — a good proxy for "programs the user is running".
#[cfg(windows)]
pub fn running_programs() -> anyhow::Result<Vec<WindowInfo>> {
    use windows::Win32::Foundation::{BOOL, HWND, LPARAM};
    use windows::Win32::UI::WindowsAndMessaging::{
        EnumWindows, GetWindowTextLengthW, GetWindowTextW, GetWindowThreadProcessId,
        IsWindowVisible,
    };

    unsafe extern "system" fn enum_proc(hwnd: HWND, lparam: LPARAM) -> BOOL {
        let windows_list = unsafe { &mut *(lparam.0 as *mut Vec<(u32, String)>) };

        unsafe {
            if IsWindowVisible(hwnd).as_bool() {
                let len = GetWindowTextLengthW(hwnd);
                if len > 0 {
                    let mut buf = vec![0u16; (len + 1) as usize];
                    let n = GetWindowTextW(hwnd, &mut buf);
                    if n > 0 {
                        let title = String::from_utf16_lossy(&buf[..n as usize]);
                        let mut pid: u32 = 0;
                        GetWindowThreadProcessId(hwnd, Some(&mut pid));
                        if pid != 0 {
                            windows_list.push((pid, title));
                        }
                    }
                }
            }
        }

        BOOL(1) // keep enumerating
    }

    let mut raw: Vec<(u32, String)> = Vec::new();
    unsafe {
        let _ = EnumWindows(
            Some(enum_proc),
            LPARAM(&mut raw as *mut Vec<(u32, String)> as isize),
        );
    }

    // One entry per process; keep the first (top-most) window title we saw.
    let mut seen_pids = std::collections::HashSet::new();
    let mut programs = Vec::new();
    for (pid, title) in raw {
        if seen_pids.insert(pid) {
            programs.push(WindowInfo {
                app_name: app_name_for_pid(pid),
                window_title: title,
            });
        }
    }
    Ok(programs)
}

#[cfg(not(windows))]
pub fn running_programs() -> anyhow::Result<Vec<WindowInfo>> {
    Ok(Vec::new())
}
