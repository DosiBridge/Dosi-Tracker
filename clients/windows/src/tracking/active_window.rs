use crate::model::WindowInfo;

/// The host process of Store (UWP) apps' frames. The app itself runs in
/// another process that owns the frame's `CoreWindow` child.
const FRAME_HOST: &str = "ApplicationFrameHost.exe";

/// The server's cap on the running-programs list of one block.
pub const MAX_RUNNING_PROGRAMS: usize = 200;

/// The file name of an executable path (`C:\…\Code.exe` → `Code.exe`).
pub fn file_name(path: &str) -> &str {
    path.rsplit(['\\', '/']).next().unwrap_or(path)
}

/// Is this the UWP frame host rather than an app?
pub fn is_frame_host(app_name: &str) -> bool {
    app_name.eq_ignore_ascii_case(FRAME_HOST)
}

/// Running programs by name only — never their window titles (privacy: the
/// titles of background windows are not the worker's focus and can hold
/// document names or e-mail subjects). One entry per executable, compared
/// case-insensitively, in first-seen (Z-) order; unnamed and frame-host
/// entries are dropped. At most [`MAX_RUNNING_PROGRAMS`].
pub fn program_list<I>(app_names: I) -> Vec<WindowInfo>
where
    I: IntoIterator<Item = String>,
{
    let mut seen = std::collections::HashSet::new();
    app_names
        .into_iter()
        .filter(|name| !name.is_empty() && !is_frame_host(name))
        .filter(|name| seen.insert(name.to_lowercase()))
        .take(MAX_RUNNING_PROGRAMS)
        .map(|app_name| WindowInfo { app_name, window_title: String::new(), ..Default::default() })
        .collect()
}

/// Resolve the executable file name (e.g. `code.exe`) that owns a process id.
///
/// `PROCESS_QUERY_LIMITED_INFORMATION` is granted across integrity levels, so
/// this also names elevated (administrator) apps, which the full query right
/// a medium-integrity process asks for is denied.
#[cfg(windows)]
fn app_name_for_pid(pid: u32) -> String {
    use windows::core::PWSTR;
    use windows::Win32::Foundation::{CloseHandle, BOOL};
    use windows::Win32::System::Threading::{
        OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION,
    };

    if pid == 0 {
        return String::new();
    }
    unsafe {
        // BOOL(0) is the concrete form the pinned windows 0.58 `Param`-based
        // signature accepts (a bare `false` is not).
        let Ok(handle) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, BOOL(0), pid) else {
            return String::new();
        };
        let mut name = String::new();
        // Long paths need more than MAX_PATH: retry once with the maximum.
        for capacity in [1024usize, 32_768] {
            let mut buffer = vec![0u16; capacity];
            let mut length = capacity as u32;
            if QueryFullProcessImageNameW(handle, PROCESS_NAME_WIN32, PWSTR(buffer.as_mut_ptr()), &mut length).is_ok() {
                let path = String::from_utf16_lossy(&buffer[..length as usize]);
                name = file_name(&path).to_string();
                break;
            }
        }
        let _ = CloseHandle(handle);
        name
    }
}

/// The process that owns a window.
#[cfg(windows)]
fn window_pid(hwnd: windows::Win32::Foundation::HWND) -> u32 {
    use windows::Win32::UI::WindowsAndMessaging::GetWindowThreadProcessId;

    let mut pid: u32 = 0;
    unsafe { GetWindowThreadProcessId(hwnd, Some(&mut pid)) };
    pid
}

/// The app behind a window: its process's executable, or — for a Store app's
/// frame — the process of the `CoreWindow` it hosts. A frame whose app is
/// suspended has no such child and stays the frame host.
#[cfg(windows)]
fn app_name_for_window(hwnd: windows::Win32::Foundation::HWND) -> String {
    let pid = window_pid(hwnd);
    let name = app_name_for_pid(pid);
    if !is_frame_host(&name) {
        return name;
    }
    match hosted_app_pid(hwnd, pid) {
        Some(app_pid) => {
            let app = app_name_for_pid(app_pid);
            if app.is_empty() {
                name
            } else {
                app
            }
        }
        None => name,
    }
}

/// The first child window owned by another process than the frame's.
#[cfg(windows)]
fn hosted_app_pid(frame: windows::Win32::Foundation::HWND, frame_pid: u32) -> Option<u32> {
    use windows::Win32::Foundation::{BOOL, HWND, LPARAM};
    use windows::Win32::UI::WindowsAndMessaging::EnumChildWindows;

    struct Search {
        frame_pid: u32,
        found: Option<u32>,
    }

    unsafe extern "system" fn visit(child: HWND, lparam: LPARAM) -> BOOL {
        let search = unsafe { &mut *(lparam.0 as *mut Search) };
        let pid = window_pid(child);
        if pid != 0 && pid != search.frame_pid {
            search.found = Some(pid);
            return BOOL(0); // stop
        }
        BOOL(1)
    }

    let mut search = Search { frame_pid, found: None };
    unsafe {
        let _ = EnumChildWindows(frame, Some(visit), LPARAM(&mut search as *mut Search as isize));
    }
    search.found
}

/// Return the currently focused window (app + title), if any.
#[cfg(windows)]
pub fn current() -> Option<WindowInfo> {
    use windows::Win32::Foundation::HWND;
    use windows::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, GetWindowTextW};

    unsafe {
        let hwnd: HWND = GetForegroundWindow();
        if hwnd.0.is_null() {
            return None;
        }

        // Window title
        let mut title_buf = [0u16; 512];
        let len = GetWindowTextW(hwnd, &mut title_buf);
        let window_title = String::from_utf16_lossy(&title_buf[..len.max(0) as usize]);

        Some(WindowInfo {
            app_name: app_name_for_window(hwnd),
            window_title,
            ..Default::default()
        })
    }
}

#[cfg(not(windows))]
pub fn current() -> Option<WindowInfo> {
    None
}

/// The programs with a window open: visible, titled, not cloaked (a
/// suspended Store app's window is "visible" but cloaked) top-level windows —
/// what Alt+Tab shows — by app name only (see [`program_list`]).
#[cfg(windows)]
pub fn running_programs() -> anyhow::Result<Vec<WindowInfo>> {
    use windows::Win32::Foundation::{BOOL, HWND, LPARAM};
    use windows::Win32::Graphics::Dwm::{DwmGetWindowAttribute, DWMWA_CLOAKED};
    use windows::Win32::UI::WindowsAndMessaging::{EnumWindows, GetWindowTextLengthW, IsWindowVisible};

    unsafe extern "system" fn enum_proc(hwnd: HWND, lparam: LPARAM) -> BOOL {
        let windows_list = unsafe { &mut *(lparam.0 as *mut Vec<HWND>) };
        unsafe {
            if IsWindowVisible(hwnd).as_bool() && GetWindowTextLengthW(hwnd) > 0 {
                let mut cloaked: u32 = 0;
                let cloaked_known = DwmGetWindowAttribute(
                    hwnd,
                    DWMWA_CLOAKED,
                    &mut cloaked as *mut u32 as *mut core::ffi::c_void,
                    std::mem::size_of::<u32>() as u32,
                )
                .is_ok();
                if !(cloaked_known && cloaked != 0) {
                    windows_list.push(hwnd);
                }
            }
        }
        BOOL(1) // keep enumerating
    }

    let mut windows_found: Vec<HWND> = Vec::new();
    unsafe {
        EnumWindows(Some(enum_proc), LPARAM(&mut windows_found as *mut Vec<HWND> as isize))?;
    }
    Ok(program_list(windows_found.into_iter().map(app_name_for_window)))
}

#[cfg(not(windows))]
pub fn running_programs() -> anyhow::Result<Vec<WindowInfo>> {
    Ok(Vec::new())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn executable_paths_become_file_names() {
        assert_eq!(file_name(r"C:\Program Files\Microsoft VS Code\Code.exe"), "Code.exe");
        assert_eq!(file_name("/usr/bin/zsh"), "zsh");
        assert_eq!(file_name("notepad.exe"), "notepad.exe");
        assert_eq!(file_name(""), "");
    }

    #[test]
    fn the_frame_host_is_recognised_in_any_case() {
        assert!(is_frame_host("ApplicationFrameHost.exe"));
        assert!(is_frame_host("applicationframehost.EXE"));
        assert!(!is_frame_host("Calculator.exe"));
    }

    #[test]
    fn running_programs_are_names_only_and_one_per_executable() {
        let names = ["Code.exe", "chrome.exe", "code.exe", "", "ApplicationFrameHost.exe", "Chrome.EXE", "slack.exe"];
        let programs = program_list(names.iter().map(|n| n.to_string()));
        let listed: Vec<&str> = programs.iter().map(|p| p.app_name.as_str()).collect();
        assert_eq!(listed, ["Code.exe", "chrome.exe", "slack.exe"]);
        assert!(programs.iter().all(|p| p.window_title.is_empty()));
        // Bare entries: no usage fields either.
        assert!(programs.iter().all(|p| p.seconds.is_none() && p.keyboard_hits.is_none()));
    }

    #[test]
    fn running_programs_respect_the_server_cap() {
        let programs = program_list((0..500).map(|i| format!("app{i}.exe")));
        assert_eq!(programs.len(), MAX_RUNNING_PROGRAMS);
    }

    /// Needs a real desktop session: `cargo test -- --ignored`.
    #[test]
    #[ignore = "reads the real desktop"]
    fn names_the_foreground_app_and_running_programs() {
        let current = current().expect("a foreground window");
        println!("in front: {} — {}", current.app_name, current.window_title);
        assert!(!current.app_name.is_empty());
        let programs = running_programs().unwrap();
        println!("running: {:?}", programs.iter().map(|p| &p.app_name).collect::<Vec<_>>());
        assert!(programs.iter().all(|p| p.window_title.is_empty()));
    }
}
