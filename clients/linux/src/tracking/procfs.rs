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

/// Linux kernel task flag indicating a kernel thread (`#define PF_KTHREAD 0x00200000` in `include/linux/sched.h`).
pub const PF_KTHREAD: u64 = 0x0020_0000;

/// Robust bounded parser for `/proc/[pid]/stat` to determine if a task is a kernel thread.
///
/// In `/proc/[pid]/stat`, field 2 is `(comm)`, which can contain spaces and parentheses.
/// We find the last `)` and parse the whitespace-separated fields that follow:
/// - Index 0: state (`R`, `S`, etc.)
/// - Index 1: ppid
/// - Index 2: pgrp
/// - Index 3: session
/// - Index 4: tty_nr
/// - Index 5: tpgid
/// - Index 6: flags (Field 9 of `/proc/[pid]/stat`)
///
/// Returns `true` if `flags & PF_KTHREAD != 0`.
pub fn parse_stat_is_kernel_thread(stat_line: &str) -> bool {
    let after_comm = match stat_line.rfind(')') {
        Some(idx) => &stat_line[idx + 1..],
        None => return false,
    };

    let mut fields = after_comm.split_ascii_whitespace();
    let flags_str = match fields.nth(6) {
        Some(f) => f,
        None => return false,
    };

    match flags_str.parse::<u64>() {
        Ok(flags) => (flags & PF_KTHREAD) != 0,
        Err(_) => false,
    }
}

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
    /// Detect if the process is positively identified as a kernel thread.
    fn is_kernel_thread(&self, pid: u32) -> Result<bool, TrackingError>;
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

    fn is_kernel_thread(&self, pid: u32) -> Result<bool, TrackingError> {
        let stat_path = self.proc_root.join(pid.to_string()).join("stat");
        match std::fs::read_to_string(&stat_path) {
            Ok(content) => Ok(parse_stat_is_kernel_thread(&content)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
            Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied => Ok(false),
            Err(e) => Err(TrackingError::IoError(format!(
                "Failed reading stat for pid {}: {}",
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
    pub is_kernel_thread: bool,
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
                is_kernel_thread: false,
            },
        );
    }

    pub fn add_kernel_thread(&self, pid: u32, comm: impl Into<String>) {
        let mut procs = self.processes.lock().unwrap();
        procs.insert(
            pid,
            MockProcessEntry {
                comm: comm.into(),
                cmdline: None,
                exe: None,
                is_kernel_thread: true,
            },
        );
    }

    pub fn set_kernel_thread(&self, pid: u32, is_kthread: bool) {
        let mut procs = self.processes.lock().unwrap();
        if let Some(entry) = procs.get_mut(&pid) {
            entry.is_kernel_thread = is_kthread;
        }
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

    fn is_kernel_thread(&self, pid: u32) -> Result<bool, TrackingError> {
        let procs = self.processes.lock().unwrap();
        Ok(procs.get(&pid).map(|p| p.is_kernel_thread).unwrap_or(false))
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
    pub fn resolve_app_name_for_pid(&self, pid: u32) -> Result<Option<String>, TrackingError> {
        // Skip positively identified kernel threads
        if self.reader.is_kernel_thread(pid)? {
            return Ok(None);
        }

        // Priority 1: read_exe (canonical absolute target of /proc/[pid]/exe)
        if let Some(exe) = self.reader.read_exe(pid)? {
            let name = file_name(&exe).trim();
            if !name.is_empty() && !name.contains("(deleted)") {
                return Ok(Some(name.to_string()));
            }
        }

        // Priority 2: read_cmdline argv[0]
        if let Some(args) = self.reader.read_cmdline(pid)? {
            if let Some(first) = args.first() {
                let name = file_name(first).trim();
                if !name.is_empty() {
                    return Ok(Some(name.to_string()));
                }
            }
        }

        // Priority 3: read_comm (16-char kernel comm)
        // Preserves ordinary user processes where exe/cmdline is unavailable (e.g. zombies or permission-denied)
        if let Some(comm) = self.reader.read_comm(pid)? {
            let name = comm.trim();
            if !name.is_empty() && !is_kernel_thread(name) {
                return Ok(Some(name.to_string()));
            }
        }

        Ok(None)
    }
}

impl RunningAppsProvider for ProcFsRunningAppsProvider {
    fn running_programs(&self) -> Result<Vec<WindowInfo>, TrackingError> {
        let pids = self.reader.list_pids()?;
        let mut app_names = Vec::with_capacity(pids.len().min(MAX_RUNNING_PROGRAMS * 2));

        for pid in pids {
            if let Some(name) = self.resolve_app_name_for_pid(pid)? {
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

    #[test]
    fn parse_stat_is_kernel_thread_handles_various_formats() {
        // 1. kthreadd (flag 2129984 = 0x208040, PF_KTHREAD set)
        let kthreadd_stat =
            "2 (kthreadd) S 0 0 0 0 -1 2129984 0 0 0 0 0 0 0 0 20 0 1 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0";
        assert!(parse_stat_is_kernel_thread(kthreadd_stat));

        // 2. kworker (flag 69238848 = 0x04208040, PF_KTHREAD set)
        let kworker_stat =
            "10 (kworker/u16:0) I 2 0 0 0 -1 69238848 0 0 0 0 0 0 0 0 20 0 1 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0";
        assert!(parse_stat_is_kernel_thread(kworker_stat));

        // 3. User process bash (flag 4194304 = 0x400000, PF_KTHREAD NOT set)
        let bash_stat =
            "100 (bash) S 1 100 100 0 -1 4194304 0 0 0 0 0 0 0 0 20 0 1 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0";
        assert!(!parse_stat_is_kernel_thread(bash_stat));

        // 4. Comm containing spaces and nested parentheses
        let complex_comm_stat =
            "300 (worker (job) ) S 1 300 300 0 -1 4194304 0 0 0 0 0 0 0 0 20 0 1 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0";
        assert!(!parse_stat_is_kernel_thread(complex_comm_stat));

        // 5. Malformed lines
        assert!(!parse_stat_is_kernel_thread(""));
        assert!(!parse_stat_is_kernel_thread("corrupted stat line"));
        assert!(!parse_stat_is_kernel_thread("100 (incomplete) S"));
    }

    #[test]
    fn fake_proc_root_sys_proc_fs_reader_excludes_kthreads_and_preserves_user_fallback() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();

        // PID 2: unbracketed kthreadd kernel thread
        let p2 = root.join("2");
        std::fs::create_dir(&p2).unwrap();
        std::fs::write(
            p2.join("stat"),
            "2 (kthreadd) S 0 0 0 0 -1 2129984 0 0 0 0 0 0 0 0 20 0 1 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0",
        )
        .unwrap();
        std::fs::write(p2.join("comm"), "kthreadd\n").unwrap();
        std::fs::write(p2.join("cmdline"), "").unwrap();

        // PID 10: unbracketed kworker kernel thread
        let p10 = root.join("10");
        std::fs::create_dir(&p10).unwrap();
        std::fs::write(
            p10.join("stat"),
            "10 (kworker/u16:0) I 2 0 0 0 -1 69238848 0 0 0 0 0 0 0 0 20 0 1 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0",
        )
        .unwrap();
        std::fs::write(p10.join("comm"), "kworker/u16:0\n").unwrap();
        std::fs::write(p10.join("cmdline"), "").unwrap();

        // PID 100: normal user process with cmdline
        let p100 = root.join("100");
        std::fs::create_dir(&p100).unwrap();
        std::fs::write(
            p100.join("stat"),
            "100 (bash) S 1 100 100 0 -1 4194304 0 0 0 0 0 0 0 0 20 0 1 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0",
        )
        .unwrap();
        std::fs::write(p100.join("comm"), "bash\n").unwrap();
        std::fs::write(p100.join("cmdline"), b"/usr/bin/bash\0-i\0").unwrap();

        // PID 200: ordinary user process with missing cmdline/exe (e.g. zombie or unprivileged access)
        let p200 = root.join("200");
        std::fs::create_dir(&p200).unwrap();
        std::fs::write(
            p200.join("stat"),
            "200 (zombie-worker) Z 1 200 200 0 -1 4194304 0 0 0 0 0 0 0 0 20 0 1 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0",
        )
        .unwrap();
        std::fs::write(p200.join("comm"), "zombie-worker\n").unwrap();
        // cmdline is empty, exe is absent

        // PID 300: process with spaces and parentheses in comm
        let p300 = root.join("300");
        std::fs::create_dir(&p300).unwrap();
        std::fs::write(
            p300.join("stat"),
            "300 (worker (job) ) S 1 300 300 0 -1 4194304 0 0 0 0 0 0 0 0 20 0 1 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0",
        )
        .unwrap();
        std::fs::write(p300.join("comm"), "worker (job) \n").unwrap();

        // Non-numeric entry in proc root (must be ignored by list_pids)
        let cpuinfo = root.join("cpuinfo");
        std::fs::write(cpuinfo, "processor: 0\n").unwrap();

        let sys_reader = Arc::new(SysProcFsReader::with_root(root));

        // Check kernel thread detection directly on SysProcFsReader
        assert!(sys_reader.is_kernel_thread(2).unwrap());
        assert!(sys_reader.is_kernel_thread(10).unwrap());
        assert!(!sys_reader.is_kernel_thread(100).unwrap());
        assert!(!sys_reader.is_kernel_thread(200).unwrap());
        assert!(!sys_reader.is_kernel_thread(300).unwrap());

        // Check running apps provider integration
        let provider = ProcFsRunningAppsProvider::new(sys_reader);
        let programs = provider
            .running_programs()
            .expect("running_programs must succeed");

        let names: Vec<&str> = programs.iter().map(|p| p.app_name.as_str()).collect();

        // Positively identified kernel threads must be excluded even if unbracketed
        assert!(!names.contains(&"kthreadd"));
        assert!(!names.contains(&"kworker/u16:0"));

        // User applications must be retained
        assert!(names.contains(&"bash"));
        // Ordinary user fallback on comm must be preserved when exe/cmdline are absent
        assert!(names.contains(&"zombie-worker"));
        assert!(names.contains(&"worker (job)"));
    }
}
