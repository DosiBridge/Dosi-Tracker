//! Small Windows shell integrations the window needs: where the app keeps its
//! files, opening a URL or a folder, asking where the desktop's usable area
//! is, and whether the window is really on screen.

use std::path::{Path, PathBuf};

use crate::storage::SQLITE_SIDECARS;

/// Name of the app's folder under `%APPDATA%` and `%LOCALAPPDATA%`.
const APP_DIR: &str = "DosiTracker";

/// File name of the offline queue (SQLite).
pub const QUEUE_DB_FILE: &str = "dosi-tracker.db";

/// The roaming per-user folder (`%APPDATA%\DosiTracker`): the sign-in and
/// preferences — small files that may follow the user between machines.
pub fn data_dir() -> Option<PathBuf> {
    dirs::config_dir().map(|d| d.join(APP_DIR))
}

/// The machine-local per-user folder (`%LOCALAPPDATA%\DosiTracker`): the
/// offline queue (with its screenshots), the logs and the device id. Large or
/// machine-specific files must not roam: a roaming profile would copy the
/// queue at every logon, and SQLite on a redirected (network) folder is fragile.
pub fn local_data_dir() -> Option<PathBuf> {
    dirs::data_local_dir().map(|d| d.join(APP_DIR))
}

/// Where the rolling log files are written.
pub fn logs_dir() -> Option<PathBuf> {
    local_data_dir().map(|d| d.join("logs"))
}

/// The offline queue's database file.
#[cfg_attr(not(test), expect(dead_code, reason = "where the worker opens the queue"))]
pub fn queue_db_path() -> Option<PathBuf> {
    local_data_dir().map(|d| d.join(QUEUE_DB_FILE))
}

/// What [`migrate_to_local`] did.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Migration {
    /// Files moved, at their new location.
    pub moved: Vec<PathBuf>,
    /// Files left where they were (their target already existed, or moving
    /// failed), with the reason.
    pub skipped: Vec<(PathBuf, String)>,
}

/// Move the offline queue and the logs that earlier builds kept in Roaming
/// AppData to Local AppData, once: each file moves only if its target does
/// not exist yet. Call it at startup, before the logger and the queue open
/// those files. (Tests exercise [`migrate_data`]; this one moves the real
/// user's files.)
#[expect(dead_code, reason = "called at startup, before the logger and the queue open")]
pub fn migrate_to_local() -> Migration {
    match (data_dir(), local_data_dir()) {
        (Some(roaming), Some(local)) if roaming != local => migrate_data(&roaming, &local),
        _ => Migration::default(),
    }
}

/// [`migrate_to_local`] between explicit folders: the queue database (with
/// its SQLite sidecar files, all or nothing) and each log file.
pub fn migrate_data(from: &Path, to: &Path) -> Migration {
    let mut report = Migration::default();
    if !from.is_dir() {
        return report;
    }
    if let Err(e) = std::fs::create_dir_all(to) {
        report.skipped.push((from.to_path_buf(), format!("cannot create {}: {e}", to.display())));
        return report;
    }
    migrate_database(&from.join(QUEUE_DB_FILE), &to.join(QUEUE_DB_FILE), &mut report);
    migrate_logs(&from.join("logs"), &to.join("logs"), &mut report);
    report
}

fn migrate_database(from: &Path, to: &Path, report: &mut Migration) {
    if !from.is_file() {
        return;
    }
    if to.exists() {
        report.skipped.push((from.to_path_buf(), format!("{} already exists", to.display())));
        return;
    }
    // A hot journal belongs to its database: sidecars first, the database
    // last, so it never arrives without its journal. On any failure
    // everything already moved goes back.
    let mut pairs: Vec<(PathBuf, PathBuf)> = SQLITE_SIDECARS
        .iter()
        .map(|suffix| (sidecar(from, suffix), sidecar(to, suffix)))
        .filter(|(source, _)| source.is_file())
        .collect();
    pairs.push((from.to_path_buf(), to.to_path_buf()));

    let mut done: Vec<&(PathBuf, PathBuf)> = Vec::new();
    for pair in &pairs {
        if let Err(e) = move_file(&pair.0, &pair.1) {
            for (source, target) in done.iter().rev() {
                let _ = move_file(target, source);
            }
            report.skipped.push((pair.0.clone(), e.to_string()));
            return;
        }
        done.push(pair);
    }
    report.moved.extend(pairs.into_iter().map(|(_, target)| target));
}

fn migrate_logs(from: &Path, to: &Path, report: &mut Migration) {
    let Ok(entries) = std::fs::read_dir(from) else {
        return;
    };
    if let Err(e) = std::fs::create_dir_all(to) {
        report.skipped.push((from.to_path_buf(), format!("cannot create {}: {e}", to.display())));
        return;
    }
    for entry in entries.flatten() {
        let source = entry.path();
        if !source.is_file() {
            continue;
        }
        let target = to.join(entry.file_name());
        if target.exists() {
            report.skipped.push((source, format!("{} already exists", target.display())));
        } else {
            match move_file(&source, &target) {
                Ok(()) => report.moved.push(target),
                Err(e) => report.skipped.push((source, e.to_string())),
            }
        }
    }
    // Only succeeds once the folder is empty.
    let _ = std::fs::remove_dir(from);
}

/// `name` + suffix, e.g. `dosi-tracker.db-journal`.
fn sidecar(path: &Path, suffix: &str) -> PathBuf {
    let mut name = path.as_os_str().to_os_string();
    name.push(suffix);
    PathBuf::from(name)
}

/// Rename, or copy and delete when the folders are on different volumes (a
/// redirected AppData).
fn move_file(from: &Path, to: &Path) -> std::io::Result<()> {
    if std::fs::rename(from, to).is_ok() {
        return Ok(());
    }
    std::fs::copy(from, to)?;
    if let Err(e) = std::fs::remove_file(from) {
        let _ = std::fs::remove_file(to);
        return Err(e);
    }
    Ok(())
}

/// The usable desktop area of the primary monitor (the screen minus the
/// taskbar), in logical points.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WorkArea {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

impl WorkArea {
    /// The top-left corner that centres a window of `size` (outer, points) in
    /// the work area, never above or left of it.
    pub fn centered(&self, size: [f32; 2]) -> [f32; 2] {
        [
            self.x + ((self.width - size[0]) / 2.0).max(0.0),
            self.y + ((self.height - size[1]) / 2.0).max(0.0),
        ]
    }
}

/// Open a URL in the default browser, or a folder in Explorer.
#[cfg(windows)]
pub fn open(target: &str) -> anyhow::Result<()> {
    use windows::core::{HSTRING, PCWSTR};
    use windows::Win32::Foundation::HWND;
    use windows::Win32::UI::Shell::ShellExecuteW;
    use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

    // ShellExecute reports success as a value greater than 32.
    let result = unsafe {
        ShellExecuteW(
            HWND::default(),
            &HSTRING::from("open"),
            &HSTRING::from(target),
            PCWSTR::null(),
            PCWSTR::null(),
            SW_SHOWNORMAL,
        )
    };
    if result.0 as isize > 32 {
        Ok(())
    } else {
        Err(anyhow::anyhow!("Windows could not open {target}"))
    }
}

#[cfg(not(windows))]
pub fn open(_target: &str) -> anyhow::Result<()> {
    Ok(())
}

/// The primary monitor's work area in logical points (physical pixels
/// divided by the system scale), or `None` if Windows will not say.
#[cfg(windows)]
pub fn work_area() -> Option<WorkArea> {
    use windows::Win32::Foundation::RECT;
    use windows::Win32::UI::HiDpi::GetDpiForSystem;
    use windows::Win32::UI::WindowsAndMessaging::{
        SystemParametersInfoW, SPI_GETWORKAREA, SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS,
    };

    let mut rect = RECT::default();
    unsafe {
        SystemParametersInfoW(
            SPI_GETWORKAREA,
            0,
            Some(&mut rect as *mut RECT as *mut core::ffi::c_void),
            SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
        )
        .ok()?;
    }
    // A DPI-unaware caller gets both values already scaled, so the ratio is
    // right either way.
    let scale = unsafe { GetDpiForSystem() }.max(96) as f32 / 96.0;
    let width = (rect.right - rect.left) as f32 / scale;
    let height = (rect.bottom - rect.top) as f32 / scale;
    (width > 0.0 && height > 0.0).then_some(WorkArea {
        x: rect.left as f32 / scale,
        y: rect.top as f32 / scale,
        width,
        height,
    })
}

#[cfg(not(windows))]
pub fn work_area() -> Option<WorkArea> {
    None
}

/// Is the window shown (not hidden to the tray)? A minimized window still
/// counts as shown. `None` when the handle cannot be read.
#[cfg(windows)]
pub fn is_window_shown(window: &impl raw_window_handle::HasWindowHandle) -> Option<bool> {
    use raw_window_handle::RawWindowHandle;
    use windows::Win32::Foundation::HWND;
    use windows::Win32::UI::WindowsAndMessaging::IsWindowVisible;

    let RawWindowHandle::Win32(handle) = window.window_handle().ok()?.as_raw() else {
        return None;
    };
    let hwnd = HWND(handle.hwnd.get() as *mut core::ffi::c_void);
    Some(unsafe { IsWindowVisible(hwnd) }.as_bool())
}

#[cfg(not(windows))]
pub fn is_window_shown(_window: &impl raw_window_handle::HasWindowHandle) -> Option<bool> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn centering_stays_inside_the_work_area() {
        let area = WorkArea { x: 0.0, y: 0.0, width: 1280.0, height: 680.0 };
        assert_eq!(area.centered([480.0, 600.0]), [400.0, 40.0]);
        // Taller than the area: pinned to its top rather than pushed above it.
        assert_eq!(area.centered([480.0, 900.0]), [400.0, 0.0]);

        // A taskbar on the left or top offsets the area.
        let offset = WorkArea { x: 48.0, y: 40.0, width: 1000.0, height: 700.0 };
        assert_eq!(offset.centered([400.0, 500.0]), [348.0, 140.0]);
    }

    #[test]
    fn the_queue_and_logs_are_machine_local_and_preferences_roam() {
        let (Some(roaming), Some(local)) = (data_dir(), local_data_dir()) else {
            return; // no profile folders in this environment
        };
        assert!(roaming.ends_with(APP_DIR) && local.ends_with(APP_DIR));
        assert_ne!(roaming, local);
        assert_eq!(logs_dir(), Some(local.join("logs")));
        assert_eq!(queue_db_path(), Some(local.join(QUEUE_DB_FILE)));
    }

    fn temp_root() -> PathBuf {
        let root = std::env::temp_dir().join(format!("dosi-migrate-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        root
    }

    fn write(path: &Path, text: &str) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }

    #[test]
    fn migration_moves_the_queue_with_its_journal_and_the_logs() {
        let root = temp_root();
        let (roaming, local) = (root.join("Roaming"), root.join("Local"));
        write(&roaming.join(QUEUE_DB_FILE), "db");
        write(&sidecar(&roaming.join(QUEUE_DB_FILE), "-journal"), "journal");
        write(&roaming.join("logs").join("dosi-tracker.2026-10-01.log"), "old log");
        write(&roaming.join("preferences.json"), "{}");
        write(&roaming.join("credentials.bin"), "secret");

        let report = migrate_data(&roaming, &local);
        assert!(report.skipped.is_empty(), "{:?}", report.skipped);
        assert_eq!(report.moved.len(), 3);
        assert_eq!(std::fs::read_to_string(local.join(QUEUE_DB_FILE)).unwrap(), "db");
        assert_eq!(std::fs::read_to_string(sidecar(&local.join(QUEUE_DB_FILE), "-journal")).unwrap(), "journal");
        assert_eq!(std::fs::read_to_string(local.join("logs").join("dosi-tracker.2026-10-01.log")).unwrap(), "old log");
        assert!(!roaming.join(QUEUE_DB_FILE).exists());
        assert!(!roaming.join("logs").exists(), "the emptied logs folder is removed");
        // Preferences and the sign-in stay roaming.
        assert!(roaming.join("preferences.json").exists() && roaming.join("credentials.bin").exists());

        // Running it again is a no-op.
        assert_eq!(migrate_data(&roaming, &local), Migration::default());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn migration_never_overwrites_what_is_already_local() {
        let root = temp_root();
        let (roaming, local) = (root.join("Roaming"), root.join("Local"));
        write(&roaming.join(QUEUE_DB_FILE), "old db");
        write(&local.join(QUEUE_DB_FILE), "new db");
        write(&roaming.join("logs").join("a.log"), "old a");
        write(&roaming.join("logs").join("b.log"), "old b");
        write(&local.join("logs").join("a.log"), "new a");

        let report = migrate_data(&roaming, &local);
        assert_eq!(report.moved, vec![local.join("logs").join("b.log")]);
        assert_eq!(report.skipped.len(), 2);
        assert_eq!(std::fs::read_to_string(local.join(QUEUE_DB_FILE)).unwrap(), "new db");
        assert_eq!(std::fs::read_to_string(roaming.join(QUEUE_DB_FILE)).unwrap(), "old db");
        assert_eq!(std::fs::read_to_string(local.join("logs").join("a.log")).unwrap(), "new a");
        assert!(roaming.join("logs").join("a.log").exists(), "kept where it was");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn migration_without_old_files_does_nothing() {
        let root = temp_root();
        assert_eq!(migrate_data(&root.join("missing"), &root.join("Local")), Migration::default());
        assert!(!root.join("Local").exists());
        let _ = std::fs::remove_dir_all(&root);
    }
}
