//! Safe XDG storage and configuration path resolution for Dosi-Tracker Linux.
//!
//! Enforces strict non-mutating inspection:
//! - Resolves standard XDG Base Directory Specification paths.
//! - Inspects directory and file existence without creating directories or files.
//! - Purely side-effect-free during `doctor`, `--help`, and default invocations.
//!
//! Every state file of the agent (design 9.1) has an accessor here; none of
//! them touches the disk. Writers go through [`ensure_private_dir`] and
//! [`write_private_atomic`], which create 0700 directories and 0600 files and
//! refuse symlinks and foreign owners.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Application id: desktop entry, autostart entry, GUI app id, tray id.
pub const APP_ID: &str = "io.github.dosibridge.DosiTracker";

/// Admin defaults (packages or configuration management), read-only for the agent.
pub const SYSTEM_CONFIG_FILE: &str = "/etc/dosi-tracker/config.toml";

/// Name of the systemd user unit.
pub const USER_UNIT_NAME: &str = "dosi-tracker.service";

/// Resolved XDG paths for the Linux tracker.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LinuxPaths {
    /// Configuration directory (`$XDG_CONFIG_HOME/dosi-tracker` or `~/.config/dosi-tracker`).
    pub config_dir: PathBuf,
    /// Data directory (`$XDG_DATA_HOME/dosi-tracker` or `~/.local/share/dosi-tracker`).
    pub data_dir: PathBuf,
    /// State directory (`$XDG_STATE_HOME/dosi-tracker` or `~/.local/state/dosi-tracker`).
    pub state_dir: PathBuf,
    /// Local user consent persistence file (`<config_dir>/consent.json`).
    pub consent_file: PathBuf,
    /// `$XDG_RUNTIME_DIR` itself, when the session has one (never guessed).
    #[serde(default)]
    pub runtime_base: Option<PathBuf>,
}

impl LinuxPaths {
    /// Resolve paths using an explicitly provided home directory and optional XDG environment variables.
    /// This enables deterministic, side-effect-free testing without touching the active user profile.
    pub fn resolve(
        home_dir: Option<&Path>,
        xdg_config_home: Option<&Path>,
        xdg_data_home: Option<&Path>,
        xdg_state_home: Option<&Path>,
    ) -> Self {
        let fallback_home = home_dir.unwrap_or_else(|| Path::new("/home/unknown"));

        let config_dir = xdg_config_home
            .map(|p| p.join("dosi-tracker"))
            .unwrap_or_else(|| fallback_home.join(".config").join("dosi-tracker"));

        let data_dir = xdg_data_home
            .map(|p| p.join("dosi-tracker"))
            .unwrap_or_else(|| {
                fallback_home
                    .join(".local")
                    .join("share")
                    .join("dosi-tracker")
            });

        let state_dir = xdg_state_home
            .map(|p| p.join("dosi-tracker"))
            .unwrap_or_else(|| {
                fallback_home
                    .join(".local")
                    .join("state")
                    .join("dosi-tracker")
            });

        let consent_file = config_dir.join("consent.json");

        Self {
            config_dir,
            data_dir,
            state_dir,
            consent_file,
            runtime_base: None,
        }
    }

    /// The same paths with `$XDG_RUNTIME_DIR` set (or cleared). An empty or
    /// relative value is ignored, as the XDG specification requires.
    pub fn with_runtime_dir(mut self, xdg_runtime_dir: Option<&Path>) -> Self {
        self.runtime_base = xdg_runtime_dir
            .filter(|p| p.is_absolute())
            .map(Path::to_path_buf);
        self
    }

    /// Resolve paths from current process environment variables.
    /// Pure inspection: does NOT create any directories or files.
    pub fn from_env() -> Self {
        let home = std::env::var_os("HOME").map(PathBuf::from);
        let xdg_config = std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from);
        let xdg_data = std::env::var_os("XDG_DATA_HOME").map(PathBuf::from);
        let xdg_state = std::env::var_os("XDG_STATE_HOME").map(PathBuf::from);
        let xdg_runtime = std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from);

        Self::resolve(
            home.as_deref(),
            xdg_config.as_deref(),
            xdg_data.as_deref(),
            xdg_state.as_deref(),
        )
        .with_runtime_dir(xdg_runtime.as_deref())
    }

    /// `$XDG_CONFIG_HOME` (the parent of [`Self::config_dir`]).
    fn config_home(&self) -> &Path {
        self.config_dir.parent().unwrap_or(&self.config_dir)
    }

    /// `$XDG_RUNTIME_DIR/dosi-tracker` (0700): socket and locks. `None`
    /// without `XDG_RUNTIME_DIR`; the daemon refuses to start then.
    pub fn runtime_dir(&self) -> Option<PathBuf> {
        self.runtime_base.as_ref().map(|p| p.join("dosi-tracker"))
    }

    /// `$XDG_RUNTIME_DIR/dosi-tracker/daemon.sock` (0600): the IPC endpoint.
    pub fn socket_path(&self) -> Option<PathBuf> {
        self.runtime_dir().map(|p| p.join("daemon.sock"))
    }

    /// `$XDG_RUNTIME_DIR/dosi-tracker/daemon.lock`: held by the daemon for life.
    pub fn daemon_lock_path(&self) -> Option<PathBuf> {
        self.runtime_dir().map(|p| p.join("daemon.lock"))
    }

    /// `$XDG_RUNTIME_DIR/dosi-tracker/gui.lock`: one GUI window per user.
    pub fn gui_lock_path(&self) -> Option<PathBuf> {
        self.runtime_dir().map(|p| p.join("gui.lock"))
    }

    /// `$XDG_RUNTIME_DIR/dosi-tracker-session.env` (0600): the session
    /// environment the autostart hook hands to the user unit.
    pub fn session_env_file(&self) -> Option<PathBuf> {
        self.runtime_base
            .as_ref()
            .map(|p| p.join("dosi-tracker-session.env"))
    }

    /// `$XDG_STATE_HOME/dosi-tracker/session.json` (0600): account, workspace
    /// and project cache, pause; never a secret.
    pub fn session_file(&self) -> PathBuf {
        self.state_dir.join("session.json")
    }

    /// `$XDG_STATE_HOME/dosi-tracker/consent-log.jsonl` (0600): consent receipts.
    pub fn consent_log_file(&self) -> PathBuf {
        self.state_dir.join("consent-log.jsonl")
    }

    /// `$XDG_STATE_HOME/dosi-tracker/logs` (0700): daily logs outside journald.
    pub fn logs_dir(&self) -> PathBuf {
        self.state_dir.join("logs")
    }

    /// `$XDG_DATA_HOME/dosi-tracker/outbox.db` (0600, plus -wal and -shm).
    pub fn outbox_db(&self) -> PathBuf {
        self.data_dir.join("outbox.db")
    }

    /// `$XDG_DATA_HOME/dosi-tracker/device.json` (0600): the random device id.
    pub fn device_file(&self) -> PathBuf {
        self.data_dir.join("device.json")
    }

    /// `$XDG_DATA_HOME/dosi-tracker/credentials.json` (0600): the opt-in file
    /// credential store, which never holds a password.
    pub fn credentials_file(&self) -> PathBuf {
        self.data_dir.join("credentials.json")
    }

    /// `$XDG_CONFIG_HOME/dosi-tracker/credentials.json`: written by the
    /// foundation login; reported by doctor and removed after a new sign-in.
    pub fn legacy_credentials_file(&self) -> PathBuf {
        self.config_dir.join("credentials.json")
    }

    /// `$XDG_CONFIG_HOME/dosi-tracker/config.toml` (0600): user configuration.
    pub fn config_file(&self) -> PathBuf {
        self.config_dir.join("config.toml")
    }

    /// [`SYSTEM_CONFIG_FILE`]: admin defaults below the user configuration.
    pub fn system_config_file(&self) -> PathBuf {
        PathBuf::from(SYSTEM_CONFIG_FILE)
    }

    /// `$XDG_CONFIG_HOME/autostart/io.github.dosibridge.DosiTracker.desktop`:
    /// written only by `dosi-tracker service enable`.
    pub fn autostart_entry(&self) -> PathBuf {
        self.config_home()
            .join("autostart")
            .join(format!("{APP_ID}.desktop"))
    }

    /// `$XDG_CONFIG_HOME/systemd/user`: where `service enable` writes the unit
    /// when no packaged or prefix unit is loadable.
    pub fn user_unit_dir(&self) -> PathBuf {
        self.config_home().join("systemd").join("user")
    }

    /// `$XDG_CONFIG_HOME/systemd/user/dosi-tracker.service`.
    pub fn user_unit_file(&self) -> PathBuf {
        self.user_unit_dir().join(USER_UNIT_NAME)
    }

    /// Check if configuration directory exists on disk without creating it.
    pub fn config_dir_exists(&self) -> bool {
        self.config_dir.is_dir()
    }

    /// Check if data directory exists on disk without creating it.
    pub fn data_dir_exists(&self) -> bool {
        self.data_dir.is_dir()
    }

    /// Check if consent file exists on disk without creating it.
    pub fn consent_file_exists(&self) -> bool {
        self.consent_file.is_file()
    }
}

/// Create `dir` (and missing parents) as a private directory of the current
/// user, or check an existing one.
///
/// The directory itself ends up 0700. Refused: a symlink, a non-directory, a
/// directory owned by another user, and one that is group- or world-writable
/// (someone else may already have planted files in it). An existing private
/// directory with read/search bits for others (e.g. 0755) is tightened to 0700.
#[cfg(unix)]
pub fn ensure_private_dir(dir: &Path) -> std::io::Result<()> {
    ensure_private_dir_owned_by(dir, current_euid())
}

#[cfg(unix)]
fn ensure_private_dir_owned_by(dir: &Path, expected_uid: u32) -> std::io::Result<()> {
    use std::os::unix::fs::{DirBuilderExt, MetadataExt, PermissionsExt};

    if let Some(parent) = dir.parent().filter(|p| !p.as_os_str().is_empty()) {
        if std::fs::symlink_metadata(parent).is_err() {
            std::fs::create_dir_all(parent)?;
        }
    }
    match std::fs::DirBuilder::new().mode(0o700).create(dir) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(e) => return Err(e),
    }

    let meta = std::fs::symlink_metadata(dir)?;
    if meta.file_type().is_symlink() {
        return Err(refused(dir, "is a symlink"));
    }
    if !meta.is_dir() {
        return Err(refused(dir, "is not a directory"));
    }
    if meta.uid() != expected_uid {
        return Err(refused(dir, "is owned by another user"));
    }
    if meta.mode() & 0o022 != 0 {
        return Err(refused(dir, "is writable by other users"));
    }
    if meta.mode() & 0o777 != 0o700 {
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

/// Replace `path` with `contents` atomically, as a 0600 file of the current
/// user: write a fresh `O_EXCL` temporary file next to it, fsync it, rename
/// it over `path`, then fsync the directory. A crash leaves either the old
/// or the new contents, never a mix.
///
/// Refused: `path` being a symlink or a non-regular file, or owned by another
/// user. The parent directory must already exist (see [`ensure_private_dir`]).
#[cfg(unix)]
pub fn write_private_atomic(path: &Path, contents: &[u8]) -> std::io::Result<()> {
    write_private_atomic_owned_by(path, contents, current_euid())
}

#[cfg(unix)]
fn write_private_atomic_owned_by(
    path: &Path,
    contents: &[u8],
    expected_uid: u32,
) -> std::io::Result<()> {
    use std::io::Write;
    use std::os::unix::fs::{MetadataExt, OpenOptionsExt};

    let file_name = path
        .file_name()
        .ok_or_else(|| refused(path, "has no file name"))?;
    let parent = match path.parent() {
        Some(p) if !p.as_os_str().is_empty() => p,
        _ => Path::new("."),
    };

    match std::fs::symlink_metadata(path) {
        Ok(meta) => {
            if meta.file_type().is_symlink() {
                return Err(refused(path, "is a symlink"));
            }
            if !meta.is_file() {
                return Err(refused(path, "is not a regular file"));
            }
            if meta.uid() != expected_uid {
                return Err(refused(path, "is owned by another user"));
            }
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(e),
    }

    let mut temp_name = std::ffi::OsString::from(".");
    temp_name.push(file_name);
    temp_name.push(format!(
        ".tmp-{}-{}",
        std::process::id(),
        uuid::Uuid::new_v4().simple()
    ));
    let temp_path = parent.join(temp_name);

    let result = (|| {
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(&temp_path)?;
        file.write_all(contents)?;
        file.sync_all()?;
        drop(file);
        std::fs::rename(&temp_path, path)?;
        std::fs::File::open(parent)?.sync_all()
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temp_path);
    }
    result
}

#[cfg(unix)]
fn current_euid() -> u32 {
    // SAFETY: geteuid has no preconditions and cannot fail.
    unsafe { libc::geteuid() }
}

#[cfg(unix)]
fn refused(path: &Path, why: &str) -> std::io::Error {
    std::io::Error::new(
        std::io::ErrorKind::PermissionDenied,
        format!("refusing {}: it {why}", path.display()),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_default_fallback_paths_from_home() {
        let home = Path::new("/home/worker");
        let paths = LinuxPaths::resolve(Some(home), None, None, None);

        assert_eq!(
            paths.config_dir,
            PathBuf::from("/home/worker/.config/dosi-tracker")
        );
        assert_eq!(
            paths.data_dir,
            PathBuf::from("/home/worker/.local/share/dosi-tracker")
        );
        assert_eq!(
            paths.state_dir,
            PathBuf::from("/home/worker/.local/state/dosi-tracker")
        );
        assert_eq!(
            paths.consent_file,
            PathBuf::from("/home/worker/.config/dosi-tracker/consent.json")
        );
    }

    #[test]
    fn respects_explicit_xdg_overrides() {
        let home = Path::new("/home/worker");
        let custom_config = Path::new("/custom/config");
        let custom_data = Path::new("/custom/data");
        let custom_state = Path::new("/custom/state");

        let paths = LinuxPaths::resolve(
            Some(home),
            Some(custom_config),
            Some(custom_data),
            Some(custom_state),
        );

        assert_eq!(
            paths.config_dir,
            PathBuf::from("/custom/config/dosi-tracker")
        );
        assert_eq!(paths.data_dir, PathBuf::from("/custom/data/dosi-tracker"));
        assert_eq!(paths.state_dir, PathBuf::from("/custom/state/dosi-tracker"));
        assert_eq!(
            paths.consent_file,
            PathBuf::from("/custom/config/dosi-tracker/consent.json")
        );
    }

    #[test]
    fn resolution_creates_zero_directories_or_files() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().join("home_dummy");
        // Home does not exist yet
        assert!(!home.exists());

        let paths = LinuxPaths::resolve(Some(&home), None, None, None);

        // Path resolution must NOT create home or subdirectories
        assert!(!home.exists());
        assert!(!paths.config_dir.exists());
        assert!(!paths.data_dir.exists());
        assert!(!paths.state_dir.exists());
        assert!(!paths.consent_file.exists());
        assert!(!paths.config_dir_exists());
        assert!(!paths.consent_file_exists());
    }

    #[test]
    fn every_state_path_resolves_under_its_xdg_base() {
        let paths = LinuxPaths::resolve(Some(Path::new("/home/worker")), None, None, None)
            .with_runtime_dir(Some(Path::new("/run/user/1000")));
        let p = PathBuf::from;

        assert_eq!(paths.runtime_dir(), Some(p("/run/user/1000/dosi-tracker")));
        assert_eq!(
            paths.socket_path(),
            Some(p("/run/user/1000/dosi-tracker/daemon.sock"))
        );
        assert_eq!(
            paths.daemon_lock_path(),
            Some(p("/run/user/1000/dosi-tracker/daemon.lock"))
        );
        assert_eq!(
            paths.gui_lock_path(),
            Some(p("/run/user/1000/dosi-tracker/gui.lock"))
        );
        assert_eq!(
            paths.session_env_file(),
            Some(p("/run/user/1000/dosi-tracker-session.env"))
        );
        assert_eq!(
            paths.session_file(),
            p("/home/worker/.local/state/dosi-tracker/session.json")
        );
        assert_eq!(
            paths.consent_log_file(),
            p("/home/worker/.local/state/dosi-tracker/consent-log.jsonl")
        );
        assert_eq!(
            paths.logs_dir(),
            p("/home/worker/.local/state/dosi-tracker/logs")
        );
        assert_eq!(
            paths.outbox_db(),
            p("/home/worker/.local/share/dosi-tracker/outbox.db")
        );
        assert_eq!(
            paths.device_file(),
            p("/home/worker/.local/share/dosi-tracker/device.json")
        );
        assert_eq!(
            paths.credentials_file(),
            p("/home/worker/.local/share/dosi-tracker/credentials.json")
        );
        assert_eq!(
            paths.legacy_credentials_file(),
            p("/home/worker/.config/dosi-tracker/credentials.json")
        );
        assert_eq!(
            paths.config_file(),
            p("/home/worker/.config/dosi-tracker/config.toml")
        );
        assert_eq!(
            paths.system_config_file(),
            p("/etc/dosi-tracker/config.toml")
        );
        assert_eq!(
            paths.autostart_entry(),
            p("/home/worker/.config/autostart/io.github.dosibridge.DosiTracker.desktop")
        );
        assert_eq!(
            paths.user_unit_dir(),
            p("/home/worker/.config/systemd/user")
        );
        assert_eq!(
            paths.user_unit_file(),
            p("/home/worker/.config/systemd/user/dosi-tracker.service")
        );
    }

    #[test]
    fn config_home_derived_paths_follow_an_xdg_override() {
        let paths = LinuxPaths::resolve(
            Some(Path::new("/home/worker")),
            Some(Path::new("/custom/config")),
            None,
            None,
        );
        assert_eq!(
            paths.autostart_entry(),
            PathBuf::from("/custom/config/autostart/io.github.dosibridge.DosiTracker.desktop")
        );
        assert_eq!(
            paths.user_unit_dir(),
            PathBuf::from("/custom/config/systemd/user")
        );
    }

    #[test]
    fn runtime_paths_are_absent_without_a_usable_runtime_dir() {
        let paths = LinuxPaths::resolve(Some(Path::new("/home/worker")), None, None, None);
        assert_eq!(paths.runtime_dir(), None);
        assert_eq!(paths.socket_path(), None);
        assert_eq!(paths.daemon_lock_path(), None);
        assert_eq!(paths.gui_lock_path(), None);
        assert_eq!(paths.session_env_file(), None);

        // The XDG specification says a relative value must be ignored.
        let relative = paths.with_runtime_dir(Some(Path::new("run/user/1000")));
        assert_eq!(relative.socket_path(), None);
    }

    #[test]
    fn accessors_create_nothing() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().join("home_dummy");
        let runtime = temp.path().join("runtime_dummy");
        let paths =
            LinuxPaths::resolve(Some(&home), None, None, None).with_runtime_dir(Some(&runtime));

        let all = [
            paths.runtime_dir().unwrap(),
            paths.socket_path().unwrap(),
            paths.daemon_lock_path().unwrap(),
            paths.gui_lock_path().unwrap(),
            paths.session_env_file().unwrap(),
            paths.session_file(),
            paths.consent_log_file(),
            paths.logs_dir(),
            paths.outbox_db(),
            paths.device_file(),
            paths.credentials_file(),
            paths.legacy_credentials_file(),
            paths.config_file(),
            paths.autostart_entry(),
            paths.user_unit_dir(),
            paths.user_unit_file(),
        ];
        for path in all {
            assert!(!path.exists(), "{} must not be created", path.display());
        }
        assert!(!home.exists());
        assert!(!runtime.exists());
    }

    #[cfg(unix)]
    mod private_files {
        use super::super::*;
        use std::os::unix::fs::{MetadataExt, PermissionsExt};

        fn mode(path: &Path) -> u32 {
            std::fs::symlink_metadata(path).unwrap().mode() & 0o777
        }

        #[test]
        fn ensure_private_dir_creates_missing_dirs_as_0700() {
            let temp = tempfile::tempdir().unwrap();
            let dir = temp.path().join("a").join("dosi-tracker");
            ensure_private_dir(&dir).unwrap();
            assert!(dir.is_dir());
            assert_eq!(mode(&dir), 0o700);
            // Idempotent.
            ensure_private_dir(&dir).unwrap();
            assert_eq!(mode(&dir), 0o700);
        }

        #[test]
        fn ensure_private_dir_tightens_a_readable_dir_of_ours() {
            let temp = tempfile::tempdir().unwrap();
            let dir = temp.path().join("state");
            std::fs::create_dir(&dir).unwrap();
            std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755)).unwrap();
            ensure_private_dir(&dir).unwrap();
            assert_eq!(mode(&dir), 0o700);
        }

        #[test]
        fn ensure_private_dir_refuses_dirs_writable_by_others() {
            let temp = tempfile::tempdir().unwrap();
            for unsafe_mode in [0o777, 0o770, 0o702] {
                let dir = temp.path().join(format!("open-{unsafe_mode:o}"));
                std::fs::create_dir(&dir).unwrap();
                std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(unsafe_mode))
                    .unwrap();
                let err = ensure_private_dir(&dir).unwrap_err();
                assert_eq!(err.kind(), std::io::ErrorKind::PermissionDenied);
                assert_eq!(mode(&dir), unsafe_mode, "a refused dir is left alone");
            }
        }

        #[test]
        fn ensure_private_dir_refuses_symlinks_and_files() {
            let temp = tempfile::tempdir().unwrap();
            let real = temp.path().join("real");
            std::fs::create_dir(&real).unwrap();
            let link = temp.path().join("link");
            std::os::unix::fs::symlink(&real, &link).unwrap();
            let err = ensure_private_dir(&link).unwrap_err();
            assert_eq!(err.kind(), std::io::ErrorKind::PermissionDenied);

            let file = temp.path().join("file");
            std::fs::write(&file, b"x").unwrap();
            assert!(ensure_private_dir(&file).is_err());
        }

        #[test]
        fn ensure_private_dir_refuses_a_foreign_owner() {
            let temp = tempfile::tempdir().unwrap();
            let dir = temp.path().join("dir");
            std::fs::create_dir(&dir).unwrap();
            let foreign = current_euid().wrapping_add(1);
            let err = ensure_private_dir_owned_by(&dir, foreign).unwrap_err();
            assert_eq!(err.kind(), std::io::ErrorKind::PermissionDenied);
            assert!(err.to_string().contains("another user"));
        }

        #[test]
        fn write_private_atomic_writes_and_replaces_0600_files() {
            let temp = tempfile::tempdir().unwrap();
            let file = temp.path().join("session.json");
            write_private_atomic(&file, b"first").unwrap();
            assert_eq!(std::fs::read(&file).unwrap(), b"first");
            assert_eq!(mode(&file), 0o600);

            // An existing, looser file of ours is replaced by a 0600 one.
            std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o644)).unwrap();
            write_private_atomic(&file, b"second").unwrap();
            assert_eq!(std::fs::read(&file).unwrap(), b"second");
            assert_eq!(mode(&file), 0o600);

            // No temporary file is left behind.
            let names: Vec<_> = std::fs::read_dir(temp.path())
                .unwrap()
                .map(|e| e.unwrap().file_name())
                .collect();
            assert_eq!(names, vec![std::ffi::OsString::from("session.json")]);
        }

        #[test]
        fn write_private_atomic_refuses_symlink_targets() {
            let temp = tempfile::tempdir().unwrap();
            let victim = temp.path().join("victim");
            std::fs::write(&victim, b"keep me").unwrap();
            let link = temp.path().join("config.toml");
            std::os::unix::fs::symlink(&victim, &link).unwrap();

            let err = write_private_atomic(&link, b"evil").unwrap_err();
            assert_eq!(err.kind(), std::io::ErrorKind::PermissionDenied);
            assert_eq!(std::fs::read(&victim).unwrap(), b"keep me");
            assert!(std::fs::symlink_metadata(&link)
                .unwrap()
                .file_type()
                .is_symlink());
        }

        #[test]
        fn write_private_atomic_refuses_non_files_and_foreign_owners() {
            let temp = tempfile::tempdir().unwrap();
            let dir = temp.path().join("adir");
            std::fs::create_dir(&dir).unwrap();
            assert!(write_private_atomic(&dir, b"x").is_err());

            let file = temp.path().join("device.json");
            std::fs::write(&file, b"theirs").unwrap();
            let foreign = current_euid().wrapping_add(1);
            let err = write_private_atomic_owned_by(&file, b"ours", foreign).unwrap_err();
            assert_eq!(err.kind(), std::io::ErrorKind::PermissionDenied);
            assert_eq!(std::fs::read(&file).unwrap(), b"theirs");
        }

        #[test]
        fn write_private_atomic_needs_an_existing_parent() {
            let temp = tempfile::tempdir().unwrap();
            let file = temp.path().join("missing").join("outbox.db");
            assert!(write_private_atomic(&file, b"x").is_err());
            assert!(!temp.path().join("missing").exists());
        }
    }
}
