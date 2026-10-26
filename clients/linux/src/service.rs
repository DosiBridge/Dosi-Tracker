//! Service integration (design 3.1 and 14): the systemd user unit, the
//! managed XDG autostart entry, the `service session-start` hook and a
//! `systemctl --user` wrapper.
//!
//! - The unit is rendered from `packaging/systemd/dosi-tracker.service.in`
//!   (`@BINDIR@`); it is written to `~/.config/systemd/user` only when no
//!   loadable unit exists (a custom tarball prefix), marked as ours.
//! - The autostart entry is rendered from
//!   `packaging/desktop/autostart.desktop.in` with an absolute `Exec` and
//!   carries `X-DosiTracker-Managed=true`. Only entries with that marker are
//!   ever rewritten or removed; anything else is refused and left untouched.
//! - `systemctl` runs with an explicit environment (never the caller's whole
//!   environment) and a time limit; a missing user manager is detected so
//!   the agent can run detached instead.
//! - [`session_start`] writes `$XDG_RUNTIME_DIR/dosi-tracker-session.env`
//!   (0600, validated values, systemd `EnvironmentFile` quoting) *before*
//!   starting the unit, restarts it when `XDG_SESSION_ID` changed, falls back
//!   to `dosi-tracker daemon --detach` without a user manager, and returns
//!   within one second.

use std::collections::BTreeMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use crate::paths::{LinuxPaths, USER_UNIT_NAME};

/// The unit template (design 14).
pub const UNIT_TEMPLATE: &str = include_str!("../packaging/systemd/dosi-tracker.service.in");

/// The autostart entry template (design 14).
pub const AUTOSTART_TEMPLATE: &str = include_str!("../packaging/desktop/autostart.desktop.in");

/// The line that marks a file as written (and owned) by the agent.
pub const MANAGED_MARKER: &str = "X-DosiTracker-Managed=true";

/// The variables the session env file hands to the unit, in file order.
pub const SESSION_ENV_KEYS: [&str; 8] = [
    "DISPLAY",
    "WAYLAND_DISPLAY",
    "XAUTHORITY",
    "XDG_SESSION_TYPE",
    "XDG_SESSION_ID",
    "XDG_CURRENT_DESKTOP",
    "XDG_SESSION_DESKTOP",
    "DBUS_SESSION_BUS_ADDRESS",
];

/// Longest value written to the session env file.
pub const MAX_ENV_VALUE_BYTES: usize = 4096;

/// [`session_start`] returns within this.
pub const SESSION_START_BUDGET: Duration = Duration::from_secs(1);

/// Default time limit of one `systemctl` call.
pub const SYSTEMCTL_TIMEOUT: Duration = Duration::from_secs(5);

/// Why a service operation failed.
#[derive(Debug)]
pub enum ServiceError {
    /// The file exists but was not written by the agent: left untouched.
    Unmanaged(PathBuf),
    /// A path cannot be used in a unit or desktop entry.
    InvalidPath(String),
    /// A session variable cannot be written to the env file.
    InvalidValue {
        name: String,
        why: String,
    },
    /// No `XDG_RUNTIME_DIR`.
    NoRuntimeDir,
    /// `systemctl` failed (exit status and stderr).
    Systemctl(String),
    /// `systemctl` did not answer in time.
    Timeout(String),
    Io(std::io::Error),
}

impl std::fmt::Display for ServiceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ServiceError::Unmanaged(path) => write!(
                f,
                "{} was not created by Dosi Tracker; it is left unchanged",
                path.display()
            ),
            ServiceError::InvalidPath(why) => write!(f, "unusable path: {why}"),
            ServiceError::InvalidValue { name, why } => write!(f, "{name}: {why}"),
            ServiceError::NoRuntimeDir => write!(f, "XDG_RUNTIME_DIR is not set"),
            ServiceError::Systemctl(why) => write!(f, "systemctl failed: {why}"),
            ServiceError::Timeout(what) => write!(f, "systemctl {what} did not answer in time"),
            ServiceError::Io(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for ServiceError {}

impl From<std::io::Error> for ServiceError {
    fn from(e: std::io::Error) -> Self {
        ServiceError::Io(e)
    }
}

/// Is `text` a file the agent wrote (it carries [`MANAGED_MARKER`], as a key
/// in a desktop entry or as a comment in a unit)?
pub fn is_managed(text: &str) -> bool {
    text.lines().any(|line| {
        let line = line.trim();
        line == MANAGED_MARKER || line.strip_prefix('#').map(str::trim) == Some(MANAGED_MARKER)
    })
}

fn require_absolute(path: &Path) -> Result<&str, ServiceError> {
    let text = path
        .to_str()
        .ok_or_else(|| ServiceError::InvalidPath(format!("{} is not UTF-8", path.display())))?;
    if !path.is_absolute() {
        return Err(ServiceError::InvalidPath(format!("{text} is not absolute")));
    }
    if text.chars().any(char::is_control) {
        return Err(ServiceError::InvalidPath(format!(
            "{} contains control characters",
            text.escape_debug()
        )));
    }
    Ok(text)
}

/// The unit with `ExecStart=<bindir>/dosi-tracker daemon --systemd`. The
/// directory must be absolute and free of characters a unit file would
/// interpret (whitespace, quotes, `\`, `%`, `$`, `;`).
pub fn render_unit(bindir: &Path) -> Result<String, ServiceError> {
    let dir = require_absolute(bindir)?;
    if let Some(bad) = dir
        .chars()
        .find(|c| c.is_whitespace() || matches!(c, '"' | '\'' | '\\' | '%' | '$' | ';'))
    {
        return Err(ServiceError::InvalidPath(format!(
            "{dir} contains {bad:?}, which a systemd unit cannot use as is; install under another prefix"
        )));
    }
    Ok(UNIT_TEMPLATE.replace("@BINDIR@", dir.trim_end_matches('/')))
}

/// The unit as written to `~/.config/systemd/user`: [`render_unit`] plus the
/// managed marker (as a comment, which systemd ignores).
pub fn render_user_unit(bindir: &Path) -> Result<String, ServiceError> {
    Ok(format!(
        "# {MANAGED_MARKER}\n# Written by \"dosi-tracker service enable\"; removed by \"dosi-tracker service disable\".\n{}",
        render_unit(bindir)?
    ))
}

/// Quote one `Exec` argument (Desktop Entry Specification): reserved
/// characters put it in double quotes, inside which `"`, `` ` ``, `$` and `\`
/// are backslash-escaped; `%` is doubled.
pub fn exec_quote(arg: &str) -> String {
    const RESERVED: &[char] = &[
        ' ', '\t', '\n', '"', '\'', '\\', '>', '<', '~', '|', '&', ';', '$', '*', '?', '#', '(',
        ')', '`',
    ];
    let arg = arg.replace('%', "%%");
    if !arg.contains(RESERVED) {
        return arg;
    }
    let mut quoted = String::from("\"");
    for c in arg.chars() {
        if matches!(c, '"' | '`' | '$' | '\\') {
            quoted.push('\\');
        }
        quoted.push(c);
    }
    quoted.push('"');
    quoted
}

/// Escape a desktop entry `string` value (`\\`, and no line breaks).
fn desktop_string(value: &str) -> String {
    value.replace('\\', "\\\\")
}

/// The managed autostart entry for the binary at `exe` (absolute).
pub fn render_autostart(exe: &Path) -> Result<String, ServiceError> {
    let exe = require_absolute(exe)?;
    Ok(AUTOSTART_TEMPLATE
        .replace("@EXEC@", &desktop_string(&exec_quote(exe)))
        .replace("@TRYEXEC@", &desktop_string(exe)))
}

/// What happened to a managed file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileChange {
    Written,
    /// Already exactly as wanted.
    Unchanged,
    Removed,
    /// There was nothing to remove.
    Absent,
}

/// The contents of `path` if it is a regular file; `None` if missing. A
/// symlink or other file type is treated as not ours.
fn read_existing(path: &Path) -> Result<Option<String>, ServiceError> {
    match std::fs::symlink_metadata(path) {
        Ok(meta) if meta.is_file() => {
            let mut text = String::new();
            std::fs::File::open(path)?
                .take(1024 * 1024)
                .read_to_string(&mut text)
                .map_err(ServiceError::Io)?;
            Ok(Some(text))
        }
        Ok(_) => Err(ServiceError::Unmanaged(path.to_path_buf())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e.into()),
    }
}

/// Write `contents` to `path` unless an unmanaged file is there.
fn write_managed(path: &Path, contents: &str) -> Result<FileChange, ServiceError> {
    match read_existing(path)? {
        Some(existing) if !is_managed(&existing) => {
            return Err(ServiceError::Unmanaged(path.to_path_buf()))
        }
        Some(existing) if existing == contents => return Ok(FileChange::Unchanged),
        _ => {}
    }
    let dir = path
        .parent()
        .ok_or_else(|| ServiceError::InvalidPath(format!("{} has no directory", path.display())))?;
    std::fs::create_dir_all(dir)?;
    crate::paths::write_private_atomic(path, contents.as_bytes())?;
    Ok(FileChange::Written)
}

/// Remove `path` if it is managed; refuse an unmanaged file.
fn remove_managed(path: &Path) -> Result<FileChange, ServiceError> {
    match read_existing(path)? {
        None => Ok(FileChange::Absent),
        Some(existing) if !is_managed(&existing) => {
            Err(ServiceError::Unmanaged(path.to_path_buf()))
        }
        Some(_) => {
            std::fs::remove_file(path)?;
            Ok(FileChange::Removed)
        }
    }
}

/// Write the managed autostart entry at `entry` for the binary `exe`.
pub fn enable_autostart(entry: &Path, exe: &Path) -> Result<FileChange, ServiceError> {
    write_managed(entry, &render_autostart(exe)?)
}

/// Remove the managed autostart entry at `entry` (an unmanaged one is refused).
pub fn disable_autostart(entry: &Path) -> Result<FileChange, ServiceError> {
    remove_managed(entry)
}

/// Is a managed autostart entry present at `entry`?
pub fn autostart_enabled(entry: &Path) -> bool {
    matches!(read_existing(entry), Ok(Some(text)) if is_managed(&text))
}

/// The explicit environment `systemctl` and the detached daemon run with.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ServiceEnv {
    vars: BTreeMap<String, String>,
}

impl ServiceEnv {
    /// The variables a user manager client needs, picked from `vars`
    /// (`std::env::vars()` in the binary's entry point): `PATH`, `HOME`,
    /// `XDG_RUNTIME_DIR`, `DBUS_SESSION_BUS_ADDRESS`, plus `LC_ALL=C` so
    /// `systemctl` output parses.
    pub fn from_vars(vars: impl IntoIterator<Item = (String, String)>) -> Self {
        let keep = [
            "PATH",
            "HOME",
            "XDG_RUNTIME_DIR",
            "DBUS_SESSION_BUS_ADDRESS",
        ];
        let mut picked: BTreeMap<String, String> = vars
            .into_iter()
            .filter(|(k, v)| keep.contains(&k.as_str()) && !v.is_empty())
            .collect();
        picked
            .entry("PATH".into())
            .or_insert_with(|| "/usr/local/bin:/usr/bin:/bin".into());
        picked.insert("LC_ALL".into(), "C".into());
        Self { vars: picked }
    }

    pub fn get(&self, name: &str) -> Option<&str> {
        self.vars.get(name).map(String::as_str)
    }

    pub fn vars(&self) -> impl Iterator<Item = (&str, &str)> {
        self.vars.iter().map(|(k, v)| (k.as_str(), v.as_str()))
    }
}

/// `systemctl --user` with an explicit environment and a time limit.
#[derive(Debug, Clone)]
pub struct Systemctl {
    program: PathBuf,
    env: ServiceEnv,
    timeout: Duration,
}

/// What a `systemctl` call printed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandOutput {
    pub success: bool,
    pub stdout: String,
    pub stderr: String,
}

/// The unit's state (`systemctl --user show`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct UnitState {
    /// `loaded`, `not-found`, ...
    pub load_state: String,
    /// `active`, `inactive`, `activating`, `failed`, ...
    pub active_state: String,
    pub main_pid: Option<u32>,
    /// Where the unit file was loaded from.
    pub fragment_path: Option<String>,
}

impl UnitState {
    pub fn loaded(&self) -> bool {
        self.load_state == "loaded"
    }

    pub fn active(&self) -> bool {
        matches!(
            self.active_state.as_str(),
            "active" | "activating" | "reloading"
        )
    }
}

impl Systemctl {
    /// `systemctl` found through the `PATH` of `env`.
    pub fn new(env: ServiceEnv) -> Self {
        Self {
            program: PathBuf::from("systemctl"),
            env,
            timeout: SYSTEMCTL_TIMEOUT,
        }
    }

    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    pub fn env(&self) -> &ServiceEnv {
        &self.env
    }

    /// Run `systemctl --user <args>`, killed when `deadline` passes.
    pub fn run(
        &self,
        args: &[&str],
        deadline: Option<Instant>,
    ) -> Result<CommandOutput, ServiceError> {
        let what = args.join(" ");
        let mut command = Command::new(&self.program);
        command
            .arg("--user")
            .args(args)
            .env_clear()
            .envs(self.env.vars())
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child = command.spawn()?;
        let limit = Instant::now() + self.timeout;
        let deadline = deadline.map_or(limit, |d| d.min(limit));
        loop {
            match child.try_wait()? {
                Some(_) => break,
                None if Instant::now() >= deadline => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(ServiceError::Timeout(what));
                }
                None => std::thread::sleep(Duration::from_millis(5)),
            }
        }
        let output = child.wait_with_output()?;
        Ok(CommandOutput {
            success: output.status.success(),
            stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&output.stderr).trim().to_owned(),
        })
    }

    fn checked(
        &self,
        args: &[&str],
        deadline: Option<Instant>,
    ) -> Result<CommandOutput, ServiceError> {
        let output = self.run(args, deadline)?;
        if output.success {
            Ok(output)
        } else {
            Err(ServiceError::Systemctl(format!(
                "systemctl --user {}: {}",
                args.join(" "),
                output.stderr
            )))
        }
    }

    /// Is a systemd user manager reachable? `false` without
    /// `XDG_RUNTIME_DIR`, without `systemctl`, or when it cannot connect.
    pub fn user_manager_available(&self, deadline: Option<Instant>) -> bool {
        if self.env.get("XDG_RUNTIME_DIR").is_none() {
            return false;
        }
        matches!(self.run(&["show-environment"], deadline), Ok(output) if output.success)
    }

    /// The state of `dosi-tracker.service`.
    pub fn unit_state(&self, deadline: Option<Instant>) -> Result<UnitState, ServiceError> {
        let output = self.checked(
            &[
                "show",
                USER_UNIT_NAME,
                "--property=LoadState",
                "--property=ActiveState",
                "--property=MainPID",
                "--property=FragmentPath",
            ],
            deadline,
        )?;
        let mut state = UnitState::default();
        for line in output.stdout.lines() {
            let Some((key, value)) = line.split_once('=') else {
                continue;
            };
            let value = value.trim();
            match key.trim() {
                "LoadState" => state.load_state = value.to_owned(),
                "ActiveState" => state.active_state = value.to_owned(),
                "MainPID" => state.main_pid = value.parse().ok().filter(|pid| *pid != 0),
                "FragmentPath" => {
                    state.fragment_path = (!value.is_empty()).then(|| value.to_owned())
                }
                _ => {}
            }
        }
        Ok(state)
    }

    /// `systemctl --user <verb> dosi-tracker.service`; `--no-block` returns
    /// once the job is queued.
    pub fn unit(
        &self,
        verb: &str,
        no_block: bool,
        deadline: Option<Instant>,
    ) -> Result<(), ServiceError> {
        let mut args = Vec::new();
        if no_block {
            args.push("--no-block");
        }
        args.push(verb);
        args.push(USER_UNIT_NAME);
        self.checked(&args, deadline).map(drop)
    }

    pub fn daemon_reload(&self, deadline: Option<Instant>) -> Result<(), ServiceError> {
        self.checked(&["daemon-reload"], deadline).map(drop)
    }
}

/// What [`install_user_unit`] did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UnitInstall {
    /// A packaged or prefix unit is loadable: nothing written.
    AlreadyLoadable(Option<String>),
    /// The managed unit in `~/.config/systemd/user` was written (and the
    /// manager reloaded).
    Written(PathBuf),
    Unchanged(PathBuf),
}

/// Make `dosi-tracker.service` loadable: when the manager finds no unit (or
/// only our managed one), write the managed unit for `bindir` to
/// `~/.config/systemd/user` and reload.
pub fn install_user_unit(
    paths: &LinuxPaths,
    bindir: &Path,
    systemctl: &Systemctl,
) -> Result<UnitInstall, ServiceError> {
    let target = paths.user_unit_file();
    let state = systemctl.unit_state(None)?;
    let ours = state
        .fragment_path
        .as_deref()
        .is_some_and(|p| Path::new(p) == target);
    if state.loaded() && !ours {
        return Ok(UnitInstall::AlreadyLoadable(state.fragment_path));
    }
    match write_managed(&target, &render_user_unit(bindir)?)? {
        FileChange::Unchanged => Ok(UnitInstall::Unchanged(target)),
        _ => {
            systemctl.daemon_reload(None)?;
            Ok(UnitInstall::Written(target))
        }
    }
}

/// Remove the managed unit from `~/.config/systemd/user` (an unmanaged one
/// is refused).
pub fn remove_user_unit(paths: &LinuxPaths) -> Result<FileChange, ServiceError> {
    remove_managed(&paths.user_unit_file())
}

/// The session env file: `KEY="value"` lines for the [`SESSION_ENV_KEYS`]
/// present in `vars`. Values must be non-empty, at most 4 KiB, without
/// control characters, `$` or backticks; `"` and `\` are escaped as systemd
/// `EnvironmentFile` double quotes require.
pub fn render_session_env(vars: &BTreeMap<String, String>) -> Result<String, ServiceError> {
    let mut out = String::from(
        "# Written by \"dosi-tracker service session-start\" for the dosi-tracker user unit.\n",
    );
    for key in SESSION_ENV_KEYS {
        let Some(value) = vars.get(key).filter(|v| !v.is_empty()) else {
            continue;
        };
        let invalid = |why: &str| ServiceError::InvalidValue {
            name: key.to_owned(),
            why: why.to_owned(),
        };
        if value.len() > MAX_ENV_VALUE_BYTES {
            return Err(invalid("value too long"));
        }
        if value.chars().any(|c| c.is_control()) {
            return Err(invalid("value contains a line break or control character"));
        }
        if value.contains(['$', '`']) {
            return Err(invalid("value contains $ or a backtick"));
        }
        let escaped = value.replace('\\', "\\\\").replace('"', "\\\"");
        out.push_str(&format!("{key}=\"{escaped}\"\n"));
    }
    Ok(out)
}

/// Read back `XDG_SESSION_ID` from an env file written by [`render_session_env`].
pub fn session_id_in_env_file(text: &str) -> Option<String> {
    text.lines().find_map(|line| {
        let value = line.trim().strip_prefix("XDG_SESSION_ID=")?;
        let value = value.trim();
        let unquoted = value
            .strip_prefix('"')
            .and_then(|v| v.strip_suffix('"'))
            .unwrap_or(value);
        Some(unquoted.replace("\\\"", "\"").replace("\\\\", "\\"))
    })
}

/// What [`session_start`] did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionStart {
    /// `systemctl --user start` was queued.
    Started,
    /// The unit was running for another login session: restarted.
    Restarted,
    /// No user manager (or no loadable unit): the daemon was spawned
    /// detached.
    Detached { pid: u32 },
}

/// Everything [`session_start`] needs, explicitly.
#[derive(Debug, Clone)]
pub struct SessionStartContext<'a> {
    pub paths: &'a LinuxPaths,
    /// The login session's variables (any of [`SESSION_ENV_KEYS`]).
    pub session_vars: BTreeMap<String, String>,
    /// The `dosi-tracker` binary (for the detached fallback).
    pub exe: PathBuf,
    pub systemctl: &'a Systemctl,
}

/// Tells the daemon it was started by the login hook (`DOSI_LAUNCH=login`,
/// in the env file and for the detached spawn): tracking then resumes by
/// itself only as `tracking.resume_at_login` and the indicators allow.
pub const LAUNCH_VAR: &str = "DOSI_LAUNCH";
pub const LAUNCH_AT_LOGIN: &str = "login";

/// The autostart hook (design 3.1): write the session env file, then start
/// (or, when `XDG_SESSION_ID` changed, restart) the unit; without a user
/// manager or a loadable unit, spawn `dosi-tracker daemon --detach`.
/// Returns within [`SESSION_START_BUDGET`].
pub fn session_start(ctx: &SessionStartContext<'_>) -> Result<SessionStart, ServiceError> {
    // systemctl gets the budget minus a margin for the fallback spawn.
    let deadline = Some(Instant::now() + SESSION_START_BUDGET - Duration::from_millis(200));
    let env_file = ctx
        .paths
        .session_env_file()
        .ok_or(ServiceError::NoRuntimeDir)?;
    let previous = read_existing(&env_file)
        .ok()
        .flatten()
        .and_then(|text| session_id_in_env_file(&text));
    let mut contents = render_session_env(&ctx.session_vars)?;
    contents.push_str(&format!("{LAUNCH_VAR}=\"{LAUNCH_AT_LOGIN}\"\n"));
    crate::paths::write_private_atomic(&env_file, contents.as_bytes())?;

    let systemctl = ctx.systemctl;
    if systemctl.user_manager_available(deadline) {
        let state = systemctl.unit_state(deadline)?;
        if state.loaded() {
            let current = ctx.session_vars.get("XDG_SESSION_ID");
            let changed = previous.is_some() && previous.as_ref() != current;
            if changed && state.active() {
                systemctl.unit("restart", true, deadline)?;
                return Ok(SessionStart::Restarted);
            }
            systemctl.unit("start", true, deadline)?;
            return Ok(SessionStart::Started);
        }
    }
    spawn_detached(ctx).map(|pid| SessionStart::Detached { pid })
}

/// `dosi-tracker daemon --detach` in its own process group, with the
/// service environment plus the session variables and the XDG bases.
fn spawn_detached(ctx: &SessionStartContext<'_>) -> Result<u32, ServiceError> {
    use std::os::unix::process::CommandExt;
    let exe = require_absolute(&ctx.exe)?;
    let mut command = Command::new(exe);
    command
        .args(["daemon", "--detach"])
        .env_clear()
        .envs(ctx.systemctl.env().vars().filter(|(k, _)| *k != "LC_ALL"))
        .envs(
            ctx.session_vars
                .iter()
                .filter(|(k, _)| SESSION_ENV_KEYS.contains(&k.as_str())),
        )
        .env(
            "XDG_CONFIG_HOME",
            ctx.paths
                .config_dir
                .parent()
                .unwrap_or(&ctx.paths.config_dir),
        )
        .env(
            "XDG_DATA_HOME",
            ctx.paths.data_dir.parent().unwrap_or(&ctx.paths.data_dir),
        )
        .env(
            "XDG_STATE_HOME",
            ctx.paths.state_dir.parent().unwrap_or(&ctx.paths.state_dir),
        )
        .env(LAUNCH_VAR, LAUNCH_AT_LOGIN)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .process_group(0);
    let child = command.spawn()?;
    Ok(child.id())
}

/// `service status`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServiceStatus {
    /// A systemd user manager is reachable.
    pub user_manager: bool,
    /// `dosi-tracker.service` is loadable.
    pub installed: bool,
    /// The managed autostart entry exists.
    pub enabled_at_login: bool,
    pub active: bool,
    pub pid: Option<u32>,
    pub unit_path: Option<String>,
}

/// Installed / enabled at login / active / pid.
pub fn status(paths: &LinuxPaths, systemctl: &Systemctl) -> ServiceStatus {
    let enabled_at_login = autostart_enabled(&paths.autostart_entry());
    let user_manager = systemctl.user_manager_available(None);
    let state = if user_manager {
        systemctl.unit_state(None).unwrap_or_default()
    } else {
        UnitState::default()
    };
    ServiceStatus {
        user_manager,
        installed: state.loaded(),
        enabled_at_login,
        active: state.active(),
        pid: state.main_pid,
        unit_path: state.fragment_path,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_unit_points_at_the_bindir_and_keeps_its_hardening() {
        let unit = render_unit(Path::new("/home/a/.local/bin/")).unwrap();
        assert!(unit.contains("ExecStart=/home/a/.local/bin/dosi-tracker daemon --systemd\n"));
        assert!(!unit.contains("@BINDIR@"));
        for line in [
            "Type=notify",
            "EnvironmentFile=-%t/dosi-tracker-session.env",
            "RestartForceExitStatus=75",
            "UMask=0077",
            "NoNewPrivileges=yes",
            "ConditionUser=!root",
        ] {
            assert!(unit.lines().any(|l| l == line), "{line}");
        }
        assert!(
            !unit.lines().any(|l| l.trim() == "[Install]"),
            "never enabled by itself"
        );
        assert!(
            !is_managed(&unit),
            "the packaged unit is not ours to remove"
        );
        assert!(is_managed(
            &render_user_unit(Path::new("/opt/dt/bin")).unwrap()
        ));
        for bad in [
            "relative/bin",
            "/opt/my apps/bin",
            "/opt/100%/bin",
            "/opt/$HOME/bin",
        ] {
            assert!(render_unit(Path::new(bad)).is_err(), "{bad}");
        }
    }

    #[test]
    fn exec_arguments_are_quoted_as_the_desktop_spec_says() {
        assert_eq!(exec_quote("/usr/bin/dosi-tracker"), "/usr/bin/dosi-tracker");
        assert_eq!(
            exec_quote("/opt/My Apps/dosi-tracker"),
            "\"/opt/My Apps/dosi-tracker\""
        );
        assert_eq!(exec_quote("/opt/a\"b/x"), "\"/opt/a\\\"b/x\"");
        assert_eq!(exec_quote("/opt/100%/x"), "/opt/100%%/x");
        let entry = render_autostart(Path::new("/opt/My Apps/bin/dosi-tracker")).unwrap();
        assert!(entry.contains("Exec=\"/opt/My Apps/bin/dosi-tracker\" service session-start\n"));
        assert!(entry.contains("TryExec=/opt/My Apps/bin/dosi-tracker\n"));
        assert!(is_managed(&entry));
        assert!(
            !entry.contains("X-GNOME-Autostart-Phase"),
            "the systemd generator skips phased entries"
        );
        // A backslash is escaped twice: once for Exec, once for the string.
        let entry = render_autostart(Path::new("/opt/a\\b/dosi-tracker")).unwrap();
        assert!(
            entry.contains("Exec=\"/opt/a\\\\\\\\b/dosi-tracker\""),
            "{entry}"
        );
        assert!(
            render_autostart(Path::new("dosi-tracker")).is_err(),
            "Exec must be absolute"
        );
        assert!(render_autostart(Path::new("/opt/a\nb")).is_err());
    }

    #[test]
    fn the_session_env_file_is_quoted_and_validated() {
        let vars: BTreeMap<String, String> = [
            ("DISPLAY", ":0"),
            ("XAUTHORITY", "/run/user/1000/.mutter-Xwaylandauth \"x\""),
            ("XDG_SESSION_ID", "3"),
            ("DBUS_SESSION_BUS_ADDRESS", "unix:path=/run/user/1000/bus"),
            ("HOME", "/home/a"),
            ("WAYLAND_DISPLAY", ""),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_owned(), v.to_owned()))
        .collect();
        let text = render_session_env(&vars).unwrap();
        assert!(text.contains("DISPLAY=\":0\"\n"));
        assert!(text.contains("XAUTHORITY=\"/run/user/1000/.mutter-Xwaylandauth \\\"x\\\"\"\n"));
        assert!(!text.contains("HOME="), "only the session variables");
        assert!(
            !text.contains("WAYLAND_DISPLAY"),
            "empty values are left out"
        );
        assert_eq!(session_id_in_env_file(&text).as_deref(), Some("3"));

        for bad in ["a\nEVIL=1", "$(id)", "`id`", "a\0b"] {
            let mut vars = vars.clone();
            vars.insert("DISPLAY".into(), bad.into());
            assert!(
                matches!(
                    render_session_env(&vars),
                    Err(ServiceError::InvalidValue { .. })
                ),
                "{bad:?}"
            );
        }
        let mut long = vars.clone();
        long.insert(
            "XDG_CURRENT_DESKTOP".into(),
            "x".repeat(MAX_ENV_VALUE_BYTES + 1),
        );
        assert!(render_session_env(&long).is_err());
    }

    #[test]
    fn the_service_env_is_explicit() {
        let env = ServiceEnv::from_vars(
            [
                ("PATH", "/shim:/usr/bin"),
                ("HOME", "/home/a"),
                ("XDG_RUNTIME_DIR", "/run/user/1000"),
                ("DISPLAY", ":0"),
                ("SECRET_TOKEN", "x"),
            ]
            .into_iter()
            .map(|(k, v)| (k.to_owned(), v.to_owned())),
        );
        let names: Vec<&str> = env.vars().map(|(k, _)| k).collect();
        assert_eq!(names, ["HOME", "LC_ALL", "PATH", "XDG_RUNTIME_DIR"]);
        assert_eq!(
            ServiceEnv::from_vars([]).get("PATH"),
            Some("/usr/local/bin:/usr/bin:/bin")
        );
    }
}
