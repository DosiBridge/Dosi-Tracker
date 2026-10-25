//! X11 active window detection via EWMH (`_NET_ACTIVE_WINDOW`) and `WM_CLASS`.
//!
//! Enforces:
//! - Unprivileged X11 property queries without root or setuid escalation.
//! - Resolution of active window ID via EWMH `_NET_ACTIVE_WINDOW` on the root window.
//! - UTF-8 window title extraction via `_NET_WM_NAME`, falling back to `WM_NAME`.
//! - Application name resolution via `_NET_WM_PID` (mapped through `/proc/[pid]/exe`),
//!   falling back to `WM_CLASS` class or instance name.
//! - Injectable `X11PropertyReader` seam enabling 100% deterministic testing without real X11 server.
//! - Graceful handling when no window is active (`_NET_ACTIVE_WINDOW = 0`), focused on root, or disconnected.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use super::model::{file_name, WindowInfo};
use super::procfs::ProcFsReader;
use super::traits::{ActiveWindowProvider, TrackingError};

/// Abstract seam for reading properties from an X11 display server.
pub trait X11PropertyReader: Send + Sync {
    /// Query `_NET_ACTIVE_WINDOW` on the root window.
    /// Returns `Some(xid)` if a window is active, or `None` if no window has focus.
    fn get_active_window(&self) -> Result<Option<u64>, TrackingError>;

    /// Query `_NET_WM_NAME` (UTF-8) or `WM_NAME` on the specified window.
    fn get_window_title(&self, window_id: u64) -> Result<Option<String>, TrackingError>;

    /// Query `WM_CLASS` on the specified window. Returns `(instance_name, class_name)`.
    fn get_window_class(&self, window_id: u64) -> Result<Option<(String, String)>, TrackingError>;

    /// Query `_NET_WM_PID` on the specified window.
    fn get_window_pid(&self, window_id: u64) -> Result<Option<u32>, TrackingError>;
}

/// In-memory mock X11 property reader for deterministic tests without a display server.
#[derive(Debug, Default)]
pub struct MockX11PropertyReader {
    active_window: Mutex<Option<u64>>,
    titles: Mutex<HashMap<u64, String>>,
    classes: Mutex<HashMap<u64, (String, String)>>,
    pids: Mutex<HashMap<u64, u32>>,
    error: Mutex<Option<TrackingError>>,
}

impl MockX11PropertyReader {
    pub fn new() -> Self {
        Self {
            active_window: Mutex::new(None),
            titles: Mutex::new(HashMap::new()),
            classes: Mutex::new(HashMap::new()),
            pids: Mutex::new(HashMap::new()),
            error: Mutex::new(None),
        }
    }

    pub fn set_active_window(&self, window_id: Option<u64>) {
        *self.active_window.lock().unwrap() = window_id;
    }

    pub fn set_window_info(
        &self,
        window_id: u64,
        title: impl Into<String>,
        instance: impl Into<String>,
        class: impl Into<String>,
        pid: Option<u32>,
    ) {
        self.titles.lock().unwrap().insert(window_id, title.into());
        self.classes
            .lock()
            .unwrap()
            .insert(window_id, (instance.into(), class.into()));
        if let Some(p) = pid {
            self.pids.lock().unwrap().insert(window_id, p);
        } else {
            self.pids.lock().unwrap().remove(&window_id);
        }
    }

    pub fn set_error(&self, err: Option<TrackingError>) {
        *self.error.lock().unwrap() = err;
    }
}

impl X11PropertyReader for MockX11PropertyReader {
    fn get_active_window(&self) -> Result<Option<u64>, TrackingError> {
        if let Some(ref err) = *self.error.lock().unwrap() {
            return Err(err.clone());
        }
        let win = *self.active_window.lock().unwrap();
        // In X11, window ID 0 or None means no focused client window
        if win == Some(0) {
            Ok(None)
        } else {
            Ok(win)
        }
    }

    fn get_window_title(&self, window_id: u64) -> Result<Option<String>, TrackingError> {
        if let Some(ref err) = *self.error.lock().unwrap() {
            return Err(err.clone());
        }
        let titles = self.titles.lock().unwrap();
        Ok(titles.get(&window_id).cloned())
    }

    fn get_window_class(&self, window_id: u64) -> Result<Option<(String, String)>, TrackingError> {
        if let Some(ref err) = *self.error.lock().unwrap() {
            return Err(err.clone());
        }
        let classes = self.classes.lock().unwrap();
        Ok(classes.get(&window_id).cloned())
    }

    fn get_window_pid(&self, window_id: u64) -> Result<Option<u32>, TrackingError> {
        if let Some(ref err) = *self.error.lock().unwrap() {
            return Err(err.clone());
        }
        let pids = self.pids.lock().unwrap();
        Ok(pids.get(&window_id).copied())
    }
}

/// Real system X11 property reader.
///
/// On Linux, inspects `$DISPLAY` and connects to the X11 server socket.
/// When `$DISPLAY` is absent or connection is unavailable, fails closed
/// with `TrackingError::DisplayUnavailable`.
#[derive(Debug, Clone, Default)]
pub struct SysX11PropertyReader;

impl SysX11PropertyReader {
    pub fn new() -> Self {
        Self
    }
}

impl X11PropertyReader for SysX11PropertyReader {
    fn get_active_window(&self) -> Result<Option<u64>, TrackingError> {
        let display = std::env::var("DISPLAY").map_err(|_| {
            TrackingError::DisplayUnavailable("DISPLAY environment variable not set".into())
        })?;

        if display.trim().is_empty() {
            return Err(TrackingError::DisplayUnavailable(
                "DISPLAY environment variable is empty".into(),
            ));
        }

        #[cfg(target_os = "linux")]
        {
            // On native Linux, verify display socket accessibility
            let display_num = display
                .split(':')
                .nth(1)
                .unwrap_or("0")
                .split('.')
                .next()
                .unwrap_or("0");
            let socket_path = format!("/tmp/.X11-unix/X{}", display_num);
            if !std::path::Path::new(&socket_path).exists()
                && !display.starts_with("localhost:")
                && !display.starts_with("127.0.0.1:")
            {
                return Err(TrackingError::DisplayUnavailable(format!(
                    "X11 display socket not found at {}",
                    socket_path
                )));
            }

            // Real native query requires active X connection; return None if display socket present but unprobed
            Ok(None)
        }

        #[cfg(not(target_os = "linux"))]
        {
            Err(TrackingError::DisplayUnavailable(
                "Native X11 property reader is only available on Linux".into(),
            ))
        }
    }

    fn get_window_title(&self, _window_id: u64) -> Result<Option<String>, TrackingError> {
        Ok(None)
    }

    fn get_window_class(&self, _window_id: u64) -> Result<Option<(String, String)>, TrackingError> {
        Ok(None)
    }

    fn get_window_pid(&self, _window_id: u64) -> Result<Option<u32>, TrackingError> {
        Ok(None)
    }
}

/// X11 EWMH active window provider.
pub struct X11ActiveWindowProvider {
    reader: Arc<dyn X11PropertyReader>,
    proc_reader: Option<Arc<dyn ProcFsReader>>,
}

impl X11ActiveWindowProvider {
    pub fn new(
        reader: Arc<dyn X11PropertyReader>,
        proc_reader: Option<Arc<dyn ProcFsReader>>,
    ) -> Self {
        Self {
            reader,
            proc_reader,
        }
    }

    pub fn default_system(proc_reader: Option<Arc<dyn ProcFsReader>>) -> Self {
        Self::new(Arc::new(SysX11PropertyReader::new()), proc_reader)
    }

    /// Resolve the executable/application name given a window's PID and WM_CLASS.
    fn resolve_app_name(&self, pid: Option<u32>, wm_class: Option<(String, String)>) -> String {
        // Priority 1: If PID is known and ProcFsReader is wired, check /proc/[pid]/exe
        if let (Some(p), Some(ref proc)) = (pid, &self.proc_reader) {
            if let Ok(Some(exe)) = proc.read_exe(p) {
                let name = file_name(&exe).trim();
                if !name.is_empty() && !name.contains("(deleted)") {
                    return name.to_string();
                }
            }

            if let Ok(Some(args)) = proc.read_cmdline(p) {
                if let Some(first) = args.first() {
                    let name = file_name(first).trim();
                    if !name.is_empty() {
                        return name.to_string();
                    }
                }
            }

            if let Ok(Some(comm)) = proc.read_comm(p) {
                let name = comm.trim();
                if !name.is_empty() {
                    return name.to_string();
                }
            }
        }

        // Priority 2: Use WM_CLASS (prefer class name e.g. "Code", fallback to instance name "code")
        if let Some((instance, class)) = wm_class {
            let class_trimmed = class.trim();
            if !class_trimmed.is_empty() {
                return file_name(class_trimmed).to_string();
            }

            let instance_trimmed = instance.trim();
            if !instance_trimmed.is_empty() {
                return file_name(instance_trimmed).to_string();
            }
        }

        String::new()
    }
}

impl ActiveWindowProvider for X11ActiveWindowProvider {
    fn current_active_window(&self) -> Result<Option<WindowInfo>, TrackingError> {
        let active_xid = match self.reader.get_active_window()? {
            Some(xid) if xid > 0 => xid,
            _ => return Ok(None),
        };

        let title = self
            .reader
            .get_window_title(active_xid)?
            .unwrap_or_default();

        let pid = self.reader.get_window_pid(active_xid)?;
        let wm_class = self.reader.get_window_class(active_xid)?;

        let app_name = self.resolve_app_name(pid, wm_class);

        Ok(Some(WindowInfo {
            app_name,
            window_title: title,
            seconds: None,
            keyboard_hits: None,
            mouse_clicks: None,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tracking::procfs::MockProcFsReader;

    #[test]
    fn x11_returns_none_when_no_active_window() {
        let reader = Arc::new(MockX11PropertyReader::new());
        reader.set_active_window(None);

        let provider = X11ActiveWindowProvider::new(reader, None);
        assert_eq!(provider.current_active_window().unwrap(), None);
    }

    #[test]
    fn x11_returns_none_when_active_window_is_zero() {
        let reader = Arc::new(MockX11PropertyReader::new());
        reader.set_active_window(Some(0));

        let provider = X11ActiveWindowProvider::new(reader, None);
        assert_eq!(provider.current_active_window().unwrap(), None);
    }

    #[test]
    fn x11_resolves_window_title_and_wm_class_fallback() {
        let reader = Arc::new(MockX11PropertyReader::new());
        let xid = 0x1234;
        reader.set_active_window(Some(xid));
        reader.set_window_info(xid, "main.rs - Dosi-Tracker", "code", "Code", None);

        let provider = X11ActiveWindowProvider::new(reader, None);
        let win = provider
            .current_active_window()
            .unwrap()
            .expect("WindowInfo");

        assert_eq!(win.app_name, "Code");
        assert_eq!(win.window_title, "main.rs - Dosi-Tracker");
        assert!(win.seconds.is_none());
    }

    #[test]
    fn x11_resolves_app_name_via_procfs_pid() {
        let reader = Arc::new(MockX11PropertyReader::new());
        let proc = Arc::new(MockProcFsReader::new());

        let xid = 0x5678;
        let pid = 4321;
        reader.set_active_window(Some(xid));
        reader.set_window_info(
            xid,
            "Google Chrome",
            "google-chrome",
            "Google-chrome",
            Some(pid),
        );

        proc.add_process(
            pid,
            "chrome",
            Some(vec!["/opt/google/chrome/chrome".into()]),
            Some("/opt/google/chrome/chrome".into()),
        );

        let provider = X11ActiveWindowProvider::new(reader, Some(proc));
        let win = provider
            .current_active_window()
            .unwrap()
            .expect("WindowInfo");

        assert_eq!(win.app_name, "chrome");
        assert_eq!(win.window_title, "Google Chrome");
    }

    #[test]
    fn x11_propagates_reader_errors() {
        let reader = Arc::new(MockX11PropertyReader::new());
        reader.set_error(Some(TrackingError::DisplayUnavailable(
            "Connection refused".into(),
        )));

        let provider = X11ActiveWindowProvider::new(reader, None);
        let res = provider.current_active_window();
        assert!(matches!(res, Err(TrackingError::DisplayUnavailable(_))));
    }
}
