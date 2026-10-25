//! Running application inventory via `/proc` scanning.
//!
//! Enforces:
//! - Pure user-space enumeration without root/sudo escalation.
//! - Injectable filesystem seam (`ProcFsReader`) for deterministic, cross-platform unit testing.
//! - Binary resolution prioritizing `/proc/[pid]/exe`, falling back to `/proc/[pid]/cmdline` and `/proc/[pid]/comm`.
//! - Pure executable filename extraction via `file_name()`.
//! - Deduplication, kernel thread filtering, and server cap via `program_list()`.
//! - Resilient error handling: processes exiting mid-enumeration (ENOENT) are ignored.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use super::model::{file_name, is_kernel_thread, program_list, WindowInfo, MAX_RUNNING_PROGRAMS};
use super::traits::{RunningAppsProvider, TrackingError};

/// Filesystem access seam for `/proc` information.
pub trait ProcFsReader: Send + Sync {
    /// Return all numeric process IDs currently present in `/proc`.
    fn list_pids(&self) -> Result<Vec<u32>, TrackingError>;
    /// Read process name from `/proc/[pid]/comm`.
    fn read_comm(&self, pid: u32) -> Result<Option<String>, TrackingError>;
    /// Read arguments from `/proc/[pid]/cmdline` (argv separated by null bytes).
    fn read_cmdline(&self, pid: u32) -> Result<Option<Vec<String>>, TrackingError>;
    /// Read the symlink destination of `/proc/[pid]/exe`.
    fn read_exe(&self, pid: u32) -> Result<Option<String>, TrackingError>;
}

/// Real system `/proc` reader.
#[derive(Debug, Clone)]
pub struct SysProcFsReader {
    proc_root: PathBuf,
}

impl SysProcFsReader {
    pub fn new() -> Self {
        Self {
            proc_root: PathBuf::from("/proc"),
        }
    }

    pub fn with_root<P: AsRef<Path>>(root: P) -> Self {
        Self {
            proc_root: root.as_ref().to_path_buf(),
        }
    }
}

impl Default for SysProcFsReader {
    fn default() -> Self {
        Self::new()
    }
}

impl ProcFsReader for SysProcFsReader {
    fn list_pids(&self) -> Result<Vec<u32>, TrackingError> {
        if !self.proc_root.is_dir() {
            return Err(TrackingError::IoError(format!(
                "Procfs root does not exist: {}",
                self.proc_root.display()
            )));
        }

        let entries = std::fs::read_dir(&self.proc_root)
            .map_err(|e| TrackingError::IoError(format!("Failed reading /proc: {}", e)))?;

        let mut pids = Vec::new();
        for entry in entries.flatten() {
            if let Ok(file_name) = entry.file_name().into_string() {
                if let Ok(pid) = file_name.parse::<u32>() {
                    pids.push(pid);
                }
            }
        }
        pids.sort_unstable();
        Ok(pids)
    }

    fn read_comm(&self, pid: u32) -> Result<Option<String>, TrackingError> {
        let comm_path = self.proc_root.join(pid.to_string()).join("comm");
        match std::fs::read_to_string(&comm_path) {
            Ok(content) => Ok(Some(content.trim().to_string())),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied => Ok(None),
            Err(e) => Err(TrackingError::IoError(format!(
                "Failed reading comm for pid {}: {}",
                pid, e
            ))),
        }
    }

    fn read_cmdline(&self, pid: u32) -> Result<Option<Vec<String>>, TrackingError> {
        let cmdline_path = self.proc_root.join(pid.to_string()).join("cmdline");
        match std::fs::read(&cmdline_path) {
            Ok(bytes) => {
                if bytes.is_empty() {
                    return Ok(None);
                }
                let args = bytes
                    .split(|&b| b == 0)
                    .filter(|slice| !slice.is_empty())
                    .map(|slice| String::from_utf8_lossy(slice).to_string())
                    .collect();
                Ok(Some(args))
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied => Ok(None),
            Err(e) => Err(TrackingError::IoError(format!(
                "Failed reading cmdline for pid {}: {}",
                pid, e
            ))),
        }
    }

    fn read_exe(&self, pid: u32) -> Result<Option<String>, TrackingError> {
        let exe_path = self.proc_root.join(pid.to_string()).join("exe");
        match std::fs::read_link(&exe_path) {
            Ok(path) => Ok(Some(path.to_string_lossy().to_string())),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied => Ok(None),
            Err(e) => Err(TrackingError::IoError(format!(
                "Failed reading exe symlink for pid {}: {}",
                pid, e
            ))),
        }
    }
}

/// Process record for in-memory mock testing.
#[derive(Debug, Clone, Default)]
pub struct MockProcessEntry {
    pub comm: String,
    pub cmdline: Option<Vec<String>>,
    pub exe: Option<String>,
}

/// In-memory mock `/proc` reader for deterministic testing without OS filesystem interaction.
#[derive(Debug, Default)]
pub struct MockProcFsReader {
    processes: Mutex<BTreeMap<u32, MockProcessEntry>>,
    fail_on_list: Mutex<Option<String>>,
}

impl MockProcFsReader {
    pub fn new() -> Self {
        Self {
            processes: Mutex::new(BTreeMap::new()),
            fail_on_list: Mutex::new(None),
        }
    }

    pub fn add_process(
        &self,
        pid: u32,
        comm: impl Into<String>,
        cmdline: Option<Vec<String>>,
        exe: Option<String>,
    ) {
        let mut procs = self.processes.lock().unwrap();
        procs.insert(
            pid,
            MockProcessEntry {
                comm: comm.into(),
                cmdline,
                exe,
            },
        );
    }

    pub fn set_fail_on_list(&self, error: Option<String>) {
        *self.fail_on_list.lock().unwrap() = error;
    }
}

impl ProcFsReader for MockProcFsReader {
    fn list_pids(&self) -> Result<Vec<u32>, TrackingError> {
        if let Some(ref err) = *self.fail_on_list.lock().unwrap() {
            return Err(TrackingError::IoError(err.clone()));
        }
        let procs = self.processes.lock().unwrap();
        Ok(procs.keys().copied().collect())
    }

    fn read_comm(&self, pid: u32) -> Result<Option<String>, TrackingError> {
        let procs = self.processes.lock().unwrap();
        Ok(procs.get(&pid).map(|p| p.comm.clone()))
    }

    fn read_cmdline(&self, pid: u32) -> Result<Option<Vec<String>>, TrackingError> {
        let procs = self.processes.lock().unwrap();
        Ok(procs.get(&pid).and_then(|p| p.cmdline.clone()))
    }

    fn read_exe(&self, pid: u32) -> Result<Option<String>, TrackingError> {
        let procs = self.processes.lock().unwrap();
        Ok(procs.get(&pid).and_then(|p| p.exe.clone()))
    }
}

/// Running applications provider based on `/proc` inspection.
pub struct ProcFsRunningAppsProvider {
    reader: Arc<dyn ProcFsReader>,
}

impl ProcFsRunningAppsProvider {
    pub fn new(reader: Arc<dyn ProcFsReader>) -> Self {
        Self { reader }
    }

    pub fn default_system() -> Self {
        Self::new(Arc::new(SysProcFsReader::new()))
    }

    /// Resolve the executable name for a specific PID.
    pub fn resolve_app_name_for_pid(&self, pid: u32) -> Option<String> {
        // Priority 1: read_exe (canonical absolute target of /proc/[pid]/exe)
        if let Ok(Some(exe)) = self.reader.read_exe(pid) {
            let name = file_name(&exe).trim();
            if !name.is_empty() && !name.contains("(deleted)") {
                return Some(name.to_string());
            }
        }

        // Priority 2: read_cmdline argv[0]
        if let Ok(Some(args)) = self.reader.read_cmdline(pid) {
            if let Some(first) = args.first() {
                let name = file_name(first).trim();
                if !name.is_empty() {
                    return Some(name.to_string());
                }
            }
        }

        // Priority 3: read_comm (16-char kernel comm)
        if let Ok(Some(comm)) = self.reader.read_comm(pid) {
            let name = comm.trim();
            if !name.is_empty() && !is_kernel_thread(name) {
                return Some(name.to_string());
            }
        }

        None
    }
}

impl RunningAppsProvider for ProcFsRunningAppsProvider {
    fn running_programs(&self) -> Result<Vec<WindowInfo>, TrackingError> {
        let pids = self.reader.list_pids()?;
        let mut app_names = Vec::with_capacity(pids.len().min(MAX_RUNNING_PROGRAMS * 2));

        for pid in pids {
            if let Some(name) = self.resolve_app_name_for_pid(pid) {
                app_names.push(name);
            }
        }

        Ok(program_list(app_names))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn procfs_running_apps_deduplicates_and_ignores_kernel_threads() {
        let mock = Arc::new(MockProcFsReader::new());

        // Process 1: code with exe
        mock.add_process(
            1001,
            "code",
            Some(vec!["/usr/share/code/code".into(), "--no-sandbox".into()]),
            Some("/usr/share/code/code".into()),
        );

        // Process 2: code child thread / renderer (duplicate name)
        mock.add_process(
            1002,
            "code",
            Some(vec![
                "/usr/share/code/code".into(),
                "--type=renderer".into(),
            ]),
            Some("/usr/share/code/code".into()),
        );

        // Process 3: chrome with cmdline only (no exe)
        mock.add_process(
            2001,
            "chrome",
            Some(vec!["/opt/google/chrome/google-chrome".into()]),
            None,
        );

        // Process 4: slack with comm only
        mock.add_process(3001, "slack", None, None);

        // Process 5: kernel thread
        mock.add_process(2, "[kworker/u16:0]", None, None);

        let provider = ProcFsRunningAppsProvider::new(mock);
        let programs = provider.running_programs().expect("Must succeed");

        let names: Vec<&str> = programs.iter().map(|p| p.app_name.as_str()).collect();
        assert_eq!(names, vec!["code", "google-chrome", "slack"]);
        assert!(programs.iter().all(|p| p.window_title.is_empty()));
    }

    #[test]
    fn procfs_running_apps_handles_list_error() {
        let mock = Arc::new(MockProcFsReader::new());
        mock.set_fail_on_list(Some("Permission denied reading /proc".into()));

        let provider = ProcFsRunningAppsProvider::new(mock);
        let res = provider.running_programs();
        assert!(matches!(res, Err(TrackingError::IoError(_))));
    }

    #[test]
    fn procfs_running_apps_respects_max_cap() {
        let mock = Arc::new(MockProcFsReader::new());
        for i in 1..=300 {
            mock.add_process(
                i,
                format!("process_{}", i),
                Some(vec![format!("/usr/bin/process_{}", i)]),
                Some(format!("/usr/bin/process_{}", i)),
            );
        }

        let provider = ProcFsRunningAppsProvider::new(mock);
        let programs = provider.running_programs().expect("Must succeed");
        assert_eq!(programs.len(), MAX_RUNNING_PROGRAMS);
    }
}
