//! Running application inventory via `/proc` scanning.
//!
//! Enforces:
//! - Pure user-space enumeration without root/sudo escalation.
//! - Injectable filesystem seam (`ProcFsReader`) for deterministic, cross-platform unit testing.
//! - Binary resolution prioritizing `/proc/[pid]/exe`, falling back to `/proc/[pid]/cmdline` and `/proc/[pid]/comm`.
//! - Pure executable filename extraction via `file_name()`.
//! - Deduplication, kernel thread filtering, and server cap via `program_list()`.
//! - Resilient error handling: processes exiting mid-enumeration (ENOENT) are ignored.
//! - Own-user scope ([`ProcessScope`]): only processes whose effective uid is the
//!   agent's own are listed, never the agent itself, and only programs — the owners
//!   of managed X11 windows ([`ProgramFilter::WindowOwners`]) or, on Wayland,
//!   processes in a systemd app scope ([`ProgramFilter::AppScopes`]). Other users'
//!   processes and background daemons are never read beyond their uid.

use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use super::model::{
    aggregate_running_programs, file_name, is_kernel_thread, WindowInfo, MAX_RUNNING_PROGRAMS,
};
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

/// The effective uid from the `Uid:` line of `/proc/[pid]/status`
/// (`Uid: real effective saved fs`).
pub fn parse_status_euid(status: &str) -> Option<u32> {
    status
        .lines()
        .find_map(|line| line.strip_prefix("Uid:"))
        .and_then(|ids| ids.split_ascii_whitespace().nth(1))
        .and_then(|euid| euid.parse().ok())
}

/// Does a `/proc/[pid]/cgroup` file place the process in a systemd app scope
/// of the user manager: a unit `app-*.scope` or `app-*.service` below
/// `app.slice` (how GNOME, KDE and other systemd-aware launchers start apps)?
pub fn is_app_scope_cgroup(cgroup: &str) -> bool {
    cgroup.lines().any(|line| {
        // cgroup v2: `0::/path`; v1: `N:controllers:/path` (name=systemd holds units).
        let path = line.splitn(3, ':').nth(2).unwrap_or("");
        let mut segments = path
            .split('/')
            .skip_while(|segment| *segment != "app.slice");
        if segments.next().is_none() {
            return false;
        }
        segments.any(|unit| {
            unit.starts_with("app-") && (unit.ends_with(".scope") || unit.ends_with(".service"))
        })
    })
}

/// A source of process ids allowed into the running-programs list (on X11:
/// the owners of managed windows).
pub trait PidSource: Send + Sync {
    fn pids(&self) -> Result<Vec<u32>, TrackingError>;
}

impl<F> PidSource for F
where
    F: Fn() -> Result<Vec<u32>, TrackingError> + Send + Sync,
{
    fn pids(&self) -> Result<Vec<u32>, TrackingError> {
        self()
    }
}

/// A fixed pid allow-list.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StaticPids(pub Vec<u32>);

impl PidSource for StaticPids {
    fn pids(&self) -> Result<Vec<u32>, TrackingError> {
        Ok(self.0.clone())
    }
}

/// Which of the user's processes count as running programs.
#[derive(Clone)]
pub enum ProgramFilter {
    /// Only pids from the source (X11: `_NET_CLIENT_LIST` owners).
    WindowOwners(Arc<dyn PidSource>),
    /// Only processes in a systemd app scope (Wayland).
    AppScopes,
}

impl std::fmt::Debug for ProgramFilter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ProgramFilter::WindowOwners(_) => f.write_str("WindowOwners"),
            ProgramFilter::AppScopes => f.write_str("AppScopes"),
        }
    }
}

/// Whose processes are listed: the agent's own effective uid, never the
/// agent's own pid, restricted by a [`ProgramFilter`].
#[derive(Debug, Clone)]
pub struct ProcessScope {
    pub euid: u32,
    pub own_pid: u32,
    pub filter: ProgramFilter,
}

impl ProcessScope {
    pub fn new(euid: u32, own_pid: u32, filter: ProgramFilter) -> Self {
        Self {
            euid,
            own_pid,
            filter,
        }
    }

    /// The scope of this process: its effective uid and pid.
    #[cfg(unix)]
    pub fn current_user(filter: ProgramFilter) -> Self {
        // SAFETY: geteuid has no preconditions and cannot fail.
        let euid = unsafe { libc::geteuid() };
        Self::new(euid, std::process::id(), filter)
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
    /// The process's effective uid (`/proc/[pid]/status`); `None` when gone
    /// or unreadable.
    fn read_uid(&self, pid: u32) -> Result<Option<u32>, TrackingError>;
    /// The process's `/proc/[pid]/cgroup`; `None` when gone or unreadable.
    fn read_cgroup(&self, pid: u32) -> Result<Option<String>, TrackingError>;
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

    fn read_uid(&self, pid: u32) -> Result<Option<u32>, TrackingError> {
        Ok(self
            .read_small_file(pid, "status")?
            .as_deref()
            .and_then(parse_status_euid))
    }

    fn read_cgroup(&self, pid: u32) -> Result<Option<String>, TrackingError> {
        self.read_small_file(pid, "cgroup")
    }
}

impl SysProcFsReader {
    /// A `/proc/[pid]/<name>` text file; `None` when the process is gone or
    /// the file unreadable.
    fn read_small_file(&self, pid: u32, name: &str) -> Result<Option<String>, TrackingError> {
        let path = self.proc_root.join(pid.to_string()).join(name);
        match std::fs::read_to_string(&path) {
            Ok(content) => Ok(Some(content)),
            Err(e)
                if matches!(
                    e.kind(),
                    std::io::ErrorKind::NotFound | std::io::ErrorKind::PermissionDenied
                ) =>
            {
                Ok(None)
            }
            // ESRCH: the process exited while the file was read.
            Err(e) if e.raw_os_error() == Some(3) => Ok(None),
            Err(e) => Err(TrackingError::IoError(format!(
                "Failed reading {} for pid {}: {}",
                name, pid, e
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
    /// Effective uid; `None` = unreadable (never listed).
    pub uid: Option<u32>,
    /// `/proc/[pid]/cgroup` content; `None` = unreadable.
    pub cgroup: Option<String>,
}

/// In-memory mock `/proc` reader for deterministic testing without OS filesystem interaction.
///
/// Processes added through the `add_*` helpers belong to [`MockProcFsReader::OWN_UID`]
/// and have no cgroup unless set otherwise.
#[derive(Debug, Default)]
pub struct MockProcFsReader {
    processes: Mutex<BTreeMap<u32, MockProcessEntry>>,
    fail_on_list: Mutex<Option<String>>,
}

impl MockProcFsReader {
    /// The uid the `add_*` helpers give their processes.
    pub const OWN_UID: u32 = 1000;

    pub fn new() -> Self {
        Self {
            processes: Mutex::new(BTreeMap::new()),
            fail_on_list: Mutex::new(None),
        }
    }

    /// Change a process's effective uid (`None` = unreadable).
    pub fn set_uid(&self, pid: u32, uid: Option<u32>) {
        if let Some(entry) = self.processes.lock().unwrap().get_mut(&pid) {
            entry.uid = uid;
        }
    }

    /// Set a process's cgroup file content.
    pub fn set_cgroup(&self, pid: u32, cgroup: Option<&str>) {
        if let Some(entry) = self.processes.lock().unwrap().get_mut(&pid) {
            entry.cgroup = cgroup.map(str::to_string);
        }
    }

    pub fn add_process(
        &self,
        pid: u32,
        comm: impl Into<String>,
        cmdline: Option<Vec<String>>,
        exe: Option<String>,
    ) {
        let comm_str = comm.into();
        let is_kthread = cmdline.is_none() && exe.is_none() && is_kernel_thread(&comm_str);
        let mut procs = self.processes.lock().unwrap();
        procs.insert(
            pid,
            MockProcessEntry {
                comm: comm_str,
                cmdline,
                exe,
                is_kernel_thread: is_kthread,
                uid: Some(Self::OWN_UID),
                cgroup: None,
            },
        );
    }

    pub fn add_user_process(
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
                uid: Some(Self::OWN_UID),
                cgroup: None,
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
                // Kernel threads belong to root.
                uid: Some(0),
                cgroup: None,
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

    fn read_uid(&self, pid: u32) -> Result<Option<u32>, TrackingError> {
        let procs = self.processes.lock().unwrap();
        Ok(procs.get(&pid).and_then(|p| p.uid))
    }

    fn read_cgroup(&self, pid: u32) -> Result<Option<String>, TrackingError> {
        let procs = self.processes.lock().unwrap();
        Ok(procs.get(&pid).and_then(|p| p.cgroup.clone()))
    }
}

/// Running applications provider based on `/proc` inspection, limited to a
/// [`ProcessScope`].
pub struct ProcFsRunningAppsProvider {
    reader: Arc<dyn ProcFsReader>,
    scope: ProcessScope,
}

impl ProcFsRunningAppsProvider {
    pub fn new(reader: Arc<dyn ProcFsReader>, scope: ProcessScope) -> Self {
        Self { reader, scope }
    }

    /// The real `/proc` of this process's own user.
    #[cfg(unix)]
    pub fn for_current_user(filter: ProgramFilter) -> Self {
        Self::new(
            Arc::new(SysProcFsReader::new()),
            ProcessScope::current_user(filter),
        )
    }

    /// Is `pid` one of the scope's processes (own euid, not the agent, and
    /// allowed by an app-scope filter)? Window-owner filtering is done by the
    /// caller, which only passes listed pids.
    fn in_scope(&self, pid: u32) -> Result<bool, TrackingError> {
        if pid == self.scope.own_pid || self.reader.read_uid(pid)? != Some(self.scope.euid) {
            return Ok(false);
        }
        Ok(match self.scope.filter {
            ProgramFilter::WindowOwners(_) => true,
            ProgramFilter::AppScopes => self
                .reader
                .read_cgroup(pid)?
                .is_some_and(|cgroup| is_app_scope_cgroup(&cgroup)),
        })
    }

    /// Resolve the executable name for a specific PID.
    pub fn resolve_app_name_for_pid(&self, pid: u32) -> Result<Option<String>, TrackingError> {
        // Skip positively identified kernel threads:
        // On native Linux, kernel threads are positively identified via PF_KTHREAD flag in /proc/[pid]/stat.
        // We skip ONLY positively confirmed kernel threads. When flags are clear or thread status is unavailable
        // (missing or unreadable stat), we proceed to resolve the user application name.
        if matches!(self.reader.is_kernel_thread(pid), Ok(true)) {
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
            if !name.is_empty() {
                return Ok(Some(name.to_string()));
            }
        }

        Ok(None)
    }
}

impl RunningAppsProvider for ProcFsRunningAppsProvider {
    fn running_programs(&self) -> Result<Vec<WindowInfo>, TrackingError> {
        let pids = match &self.scope.filter {
            ProgramFilter::WindowOwners(source) => {
                let mut seen = HashSet::new();
                let mut pids = source.pids()?;
                pids.retain(|pid| seen.insert(*pid));
                pids
            }
            ProgramFilter::AppScopes => self.reader.list_pids()?,
        };
        let mut app_names = Vec::with_capacity(pids.len().min(MAX_RUNNING_PROGRAMS * 2));

        for pid in pids {
            if !self.in_scope(pid)? {
                continue;
            }
            if let Some(name) = self.resolve_app_name_for_pid(pid)? {
                app_names.push(name);
            }
        }

        Ok(aggregate_running_programs(app_names))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A scope of the mock's own user that admits every listed pid (as if each
    /// owned a window).
    fn every_listed_pid<R: ProcFsReader + 'static>(reader: &Arc<R>) -> ProcessScope {
        let reader = reader.clone();
        ProcessScope::new(
            MockProcFsReader::OWN_UID,
            0,
            ProgramFilter::WindowOwners(Arc::new(move || reader.list_pids())),
        )
    }

    fn write_status(root: &Path, pid: u32, uid: u32) {
        std::fs::write(
            root.join(pid.to_string()).join("status"),
            format!("Name:\tx\nUmask:\t0022\nState:\tS (sleeping)\nUid:\t{uid}\t{uid}\t{uid}\t{uid}\nGid:\t{uid}\t{uid}\t{uid}\t{uid}\n"),
        )
        .unwrap();
    }

    const APP_CGROUP: &str =
        "0::/user.slice/user-1000.slice/user@1000.service/app.slice/app-gnome-firefox-4242.scope\n";

    #[test]
    fn the_effective_uid_is_read_from_status() {
        assert_eq!(
            parse_status_euid("Name:\tbash\nUid:\t1000\t1001\t1000\t1000\n"),
            Some(1001)
        );
        assert_eq!(parse_status_euid("Name:\tbash\n"), None);
        assert_eq!(parse_status_euid("Uid:\t1000\n"), None);
        assert_eq!(parse_status_euid("Uid:\tx\ty\n"), None);
    }

    #[test]
    fn app_scopes_are_recognised_in_cgroup_v1_and_v2() {
        assert!(is_app_scope_cgroup(APP_CGROUP));
        assert!(is_app_scope_cgroup(
            "0::/user.slice/user-1000.slice/user@1000.service/app.slice/app-org.kde.konsole-1234.service"
        ));
        assert!(is_app_scope_cgroup(
            "12:pids:/\n1:name=systemd:/user.slice/user-1000.slice/user@1000.service/app.slice/app-code-77.scope\n"
        ));
        // Background services of the user manager, sessions and system units are not apps.
        assert!(!is_app_scope_cgroup(
            "0::/user.slice/user-1000.slice/user@1000.service/app.slice/dbus.service"
        ));
        assert!(!is_app_scope_cgroup(
            "0::/user.slice/user-1000.slice/user@1000.service/session.slice/org.gnome.Shell@wayland.service"
        ));
        assert!(!is_app_scope_cgroup(
            "0::/user.slice/user-1000.slice/session-2.scope"
        ));
        assert!(!is_app_scope_cgroup("0::/system.slice/app-foo.service"));
        assert!(!is_app_scope_cgroup(""));
        assert!(!is_app_scope_cgroup("garbage"));
    }

    #[test]
    fn only_the_own_users_processes_are_listed_and_never_the_agent() {
        let mock = Arc::new(MockProcFsReader::new());
        mock.add_process(100, "firefox", None, Some("/usr/bin/firefox".into()));
        mock.add_process(200, "sshd", None, Some("/usr/sbin/sshd".into()));
        mock.set_uid(200, Some(0));
        mock.add_process(300, "other-user-app", None, Some("/usr/bin/gedit".into()));
        mock.set_uid(300, Some(1001));
        mock.add_process(400, "unreadable", None, Some("/usr/bin/vim".into()));
        mock.set_uid(400, None);
        mock.add_process(
            500,
            "dosi-tracker",
            None,
            Some("/usr/bin/dosi-tracker".into()),
        );

        let scope = ProcessScope::new(
            MockProcFsReader::OWN_UID,
            500,
            ProgramFilter::WindowOwners(Arc::new(StaticPids(vec![100, 200, 300, 400, 500]))),
        );
        let provider = ProcFsRunningAppsProvider::new(mock, scope);
        let names: Vec<String> = provider
            .running_programs()
            .unwrap()
            .into_iter()
            .map(|p| p.app_name)
            .collect();
        assert_eq!(names, ["firefox"]);
    }

    #[test]
    fn window_owners_restrict_the_list_to_the_given_pids() {
        let mock = Arc::new(MockProcFsReader::new());
        mock.add_process(100, "firefox", None, Some("/usr/bin/firefox".into()));
        mock.add_process(101, "bash", None, Some("/usr/bin/bash".into()));
        mock.add_process(102, "code", None, Some("/usr/share/code/code".into()));
        let scope = ProcessScope::new(
            MockProcFsReader::OWN_UID,
            1,
            // Duplicates and pids that vanished are harmless.
            ProgramFilter::WindowOwners(Arc::new(StaticPids(vec![102, 100, 102, 999]))),
        );
        let provider = ProcFsRunningAppsProvider::new(mock, scope);
        let names: Vec<String> = provider
            .running_programs()
            .unwrap()
            .into_iter()
            .map(|p| p.app_name)
            .collect();
        assert_eq!(names, ["code", "firefox"]);

        // A failing window-owner source is an error, not an empty list.
        let failing = ProcessScope::new(
            MockProcFsReader::OWN_UID,
            1,
            ProgramFilter::WindowOwners(Arc::new(|| {
                Err(TrackingError::DisplayUnavailable("gone".into()))
            })),
        );
        let provider = ProcFsRunningAppsProvider::new(Arc::new(MockProcFsReader::new()), failing);
        assert!(provider.running_programs().is_err());
    }

    #[test]
    fn app_scopes_list_only_processes_in_a_systemd_app_scope() {
        let mock = Arc::new(MockProcFsReader::new());
        mock.add_process(
            100,
            "firefox",
            None,
            Some("/usr/lib/firefox/firefox".into()),
        );
        mock.set_cgroup(100, Some(APP_CGROUP));
        mock.add_process(101, "pipewire", None, Some("/usr/bin/pipewire".into()));
        mock.set_cgroup(
            101,
            Some("0::/user.slice/user-1000.slice/user@1000.service/session.slice/pipewire.service"),
        );
        mock.add_process(102, "no-cgroup", None, Some("/usr/bin/xclock".into()));
        mock.add_process(103, "root-app", None, Some("/usr/bin/gparted".into()));
        mock.set_cgroup(103, Some(APP_CGROUP));
        mock.set_uid(103, Some(0));
        mock.add_kernel_thread(2, "kthreadd");
        mock.set_cgroup(2, Some(APP_CGROUP));

        let scope = ProcessScope::new(MockProcFsReader::OWN_UID, 1, ProgramFilter::AppScopes);
        let provider = ProcFsRunningAppsProvider::new(mock, scope);
        let names: Vec<String> = provider
            .running_programs()
            .unwrap()
            .into_iter()
            .map(|p| p.app_name)
            .collect();
        assert_eq!(names, ["firefox"]);
    }

    #[test]
    fn the_sys_reader_reads_uid_and_cgroup_and_tolerates_vanished_processes() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        std::fs::create_dir(root.join("42")).unwrap();
        write_status(root, 42, 1234);
        std::fs::write(root.join("42").join("cgroup"), APP_CGROUP).unwrap();
        let reader = SysProcFsReader::with_root(root);
        assert_eq!(reader.read_uid(42).unwrap(), Some(1234));
        assert_eq!(reader.read_cgroup(42).unwrap().as_deref(), Some(APP_CGROUP));
        assert_eq!(reader.read_uid(43).unwrap(), None);
        assert_eq!(reader.read_cgroup(43).unwrap(), None);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn the_real_proc_never_lists_the_agent_itself() {
        // Listing the test process explicitly must still drop it: it is the
        // agent's own pid.
        let own = std::process::id();
        let scope =
            ProcessScope::current_user(ProgramFilter::WindowOwners(Arc::new(StaticPids(vec![
                own,
            ]))));
        let provider = ProcFsRunningAppsProvider::new(Arc::new(SysProcFsReader::new()), scope);
        assert!(provider.running_programs().unwrap().is_empty());

        // The real status file yields this process's own effective uid.
        let euid = unsafe { libc::geteuid() };
        assert_eq!(SysProcFsReader::new().read_uid(own).unwrap(), Some(euid));
    }

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

        let provider = ProcFsRunningAppsProvider::new(mock.clone(), every_listed_pid(&mock));
        let programs = provider.running_programs().expect("Must succeed");

        let names: Vec<&str> = programs.iter().map(|p| p.app_name.as_str()).collect();
        assert_eq!(names, vec!["code", "google-chrome", "slack"]);
        assert!(programs.iter().all(|p| p.window_title.is_empty()));
    }

    #[test]
    fn procfs_running_apps_handles_list_error() {
        let mock = Arc::new(MockProcFsReader::new());
        mock.set_fail_on_list(Some("Permission denied reading /proc".into()));

        let provider = ProcFsRunningAppsProvider::new(mock.clone(), every_listed_pid(&mock));
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

        let provider = ProcFsRunningAppsProvider::new(mock.clone(), every_listed_pid(&mock));
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

        // PID 400: bracketed user process with comm only and PF_KTHREAD clear
        let p400 = root.join("400");
        std::fs::create_dir(&p400).unwrap();
        std::fs::write(
            p400.join("stat"),
            "400 ([editor]) S 1 400 400 0 -1 4194304 0 0 0 0 0 0 0 0 20 0 1 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0",
        )
        .unwrap();
        std::fs::write(p400.join("comm"), "[editor]\n").unwrap();

        // PID 500: bracketed user executable/cmdline name with PF_KTHREAD clear
        let p500 = root.join("500");
        std::fs::create_dir(&p500).unwrap();
        std::fs::write(
            p500.join("stat"),
            "500 ([custom-app]) S 1 500 500 0 -1 4194304 0 0 0 0 0 0 0 0 20 0 1 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0",
        )
        .unwrap();
        std::fs::write(p500.join("comm"), "[custom-app]\n").unwrap();
        std::fs::write(
            p500.join("cmdline"),
            b"/usr/local/bin/[custom-app]\0--run\0",
        )
        .unwrap();

        // PID 600: unknown/missing stat user fallback (stat file is absent)
        let p600 = root.join("600");
        std::fs::create_dir(&p600).unwrap();
        std::fs::write(p600.join("comm"), "[unstat-task]\n").unwrap();

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
        assert!(!sys_reader.is_kernel_thread(400).unwrap());
        assert!(!sys_reader.is_kernel_thread(500).unwrap());
        assert!(!sys_reader.is_kernel_thread(600).unwrap());

        // Check running apps provider integration
        for pid in [2, 10, 100, 200, 300, 400, 500, 600] {
            let uid = if pid < 100 {
                0
            } else {
                MockProcFsReader::OWN_UID
            };
            write_status(root, pid, uid);
        }
        let provider =
            ProcFsRunningAppsProvider::new(sys_reader.clone(), every_listed_pid(&sys_reader));
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

        // Bracketed user applications with PF_KTHREAD clear must be preserved
        assert!(names.contains(&"[editor]"));
        assert!(names.contains(&"[custom-app]"));

        // User fallback with missing/unknown stat must be preserved even if bracketed
        assert!(names.contains(&"[unstat-task]"));
    }

    #[test]
    fn mock_procfs_preserves_bracketed_user_process_with_kthread_clear() {
        let mock = Arc::new(MockProcFsReader::new());
        // Bracketed comm user process with is_kernel_thread = false
        mock.add_user_process(401, "[editor]", None, None);
        // Bracketed exe user process
        mock.add_process(
            402,
            "[custom-app]",
            Some(vec!["/usr/bin/[custom-app]".into()]),
            Some("/usr/bin/[custom-app]".into()),
        );
        // Positively identified kernel thread
        mock.add_kernel_thread(2, "kthreadd");

        let provider = ProcFsRunningAppsProvider::new(mock.clone(), every_listed_pid(&mock));
        let procs = provider.running_programs().expect("Must succeed");
        let names: Vec<&str> = procs.iter().map(|p| p.app_name.as_str()).collect();

        assert!(!names.contains(&"kthreadd"));
        assert!(names.contains(&"[editor]"));
        assert!(names.contains(&"[custom-app]"));
    }
}
