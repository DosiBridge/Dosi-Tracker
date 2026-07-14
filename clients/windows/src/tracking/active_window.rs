use crate::model::WindowInfo;

/// Return the currently focused window (app + title), if any.
#[cfg(windows)]
pub fn current() -> Option<WindowInfo> {
    use windows::Win32::Foundation::{CloseHandle, HWND, MAX_PATH};
    use windows::Win32::System::ProcessStatus::GetModuleFileNameExW;
    use windows::Win32::System::Threading::{
        OpenProcess, PROCESS_QUERY_INFORMATION, PROCESS_VM_READ,
    };
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

        let mut app_name = String::new();
        if pid != 0 {
            if let Ok(handle) =
                OpenProcess(PROCESS_QUERY_INFORMATION | PROCESS_VM_READ, false, pid)
            {
                let mut path_buf = [0u16; MAX_PATH as usize];
                let n = GetModuleFileNameExW(Some(handle), None, &mut path_buf);
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
        }

        Some(WindowInfo {
            app_name,
            window_title,
        })
    }
}

#[cfg(not(windows))]
pub fn current() -> Option<WindowInfo> {
    None
}

/// Enumerate top-level visible windows as a proxy for "running programs".
#[cfg(windows)]
pub fn running_programs() -> anyhow::Result<Vec<WindowInfo>> {
    // A fuller implementation would EnumWindows + dedupe by process.
    // For the scaffold we report the current foreground window.
    Ok(current().into_iter().collect())
}

#[cfg(not(windows))]
pub fn running_programs() -> anyhow::Result<Vec<WindowInfo>> {
    Ok(Vec::new())
}
