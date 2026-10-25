//! Temporary HOME/XDG trees and explicit environments for child processes.
//!
//! A child process started by a test gets exactly the map from
//! [`TestXdg::env`] (through [`TestXdg::command`] or [`TestXdg::apply`], which
//! clear the inherited environment first). The developer's `DISPLAY`,
//! `WAYLAND_DISPLAY`, `DBUS_SESSION_BUS_ADDRESS` and the like are therefore
//! never seen by code under test unless a test adds them on purpose (for
//! example the harness display from [`super::xvfb`]).

use std::collections::BTreeMap;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::process::Command;

use tempfile::TempDir;

/// Variables that would connect a child to the developer's session. They are
/// never copied from the test process.
pub const SESSION_VARS: &[&str] = &[
    "DISPLAY",
    "WAYLAND_DISPLAY",
    "XAUTHORITY",
    "DBUS_SESSION_BUS_ADDRESS",
    "DBUS_SYSTEM_BUS_ADDRESS",
    "XDG_SESSION_ID",
    "XDG_SESSION_TYPE",
    "XDG_CURRENT_DESKTOP",
    "XDG_RUNTIME_DIR",
];

/// Builder for a [`TestXdg`] tree.
#[derive(Debug, Clone)]
pub struct XdgBuilder {
    create_dirs: bool,
    runtime_dir: bool,
    vars: BTreeMap<String, String>,
}

impl Default for XdgBuilder {
    fn default() -> Self {
        Self {
            create_dirs: true,
            runtime_dir: true,
            vars: BTreeMap::new(),
        }
    }
}

impl XdgBuilder {
    /// Create HOME and the XDG base directories on disk (default: yes). Off,
    /// the paths only exist in the environment, to prove a command creates
    /// nothing.
    pub fn create_dirs(mut self, create: bool) -> Self {
        self.create_dirs = create;
        self
    }

    /// Provide a private 0700 `XDG_RUNTIME_DIR` (default: yes).
    pub fn runtime_dir(mut self, provide: bool) -> Self {
        self.runtime_dir = provide;
        self
    }

    /// Add (or override) one variable of the child environment.
    pub fn var(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.vars.insert(name.into(), value.into());
        self
    }

    pub fn build(self) -> TestXdg {
        let temp = tempfile::Builder::new()
            .prefix("dosi-test-xdg-")
            .tempdir()
            .expect("Failed creating temporary XDG root");
        let root = temp.path().to_path_buf();
        let home = root.join("home");
        let config_home = home.join(".config");
        let data_home = home.join(".local").join("share");
        let state_home = home.join(".local").join("state");
        let runtime_dir = self.runtime_dir.then(|| root.join("runtime"));

        if self.create_dirs {
            for dir in [&home, &config_home, &data_home, &state_home] {
                std::fs::create_dir_all(dir).expect("Failed creating XDG directory");
            }
        }
        if let Some(runtime) = &runtime_dir {
            std::fs::create_dir_all(runtime).expect("Failed creating XDG_RUNTIME_DIR");
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(runtime, std::fs::Permissions::from_mode(0o700))
                    .expect("Failed making XDG_RUNTIME_DIR private");
            }
        }

        TestXdg {
            _temp: temp,
            home,
            config_home,
            data_home,
            state_home,
            runtime_dir,
            extra: self.vars,
        }
    }
}

/// A temporary HOME with its XDG base directories, removed when dropped.
#[derive(Debug)]
pub struct TestXdg {
    _temp: TempDir,
    pub home: PathBuf,
    pub config_home: PathBuf,
    pub data_home: PathBuf,
    pub state_home: PathBuf,
    pub runtime_dir: Option<PathBuf>,
    extra: BTreeMap<String, String>,
}

impl TestXdg {
    /// A tree with every directory created and a private runtime dir.
    pub fn new() -> Self {
        Self::builder().build()
    }

    pub fn builder() -> XdgBuilder {
        XdgBuilder::default()
    }

    /// `$XDG_CONFIG_HOME/dosi-tracker`.
    pub fn config_dir(&self) -> PathBuf {
        self.config_home.join("dosi-tracker")
    }

    /// `$XDG_DATA_HOME/dosi-tracker`.
    pub fn data_dir(&self) -> PathBuf {
        self.data_home.join("dosi-tracker")
    }

    /// `$XDG_STATE_HOME/dosi-tracker`.
    pub fn state_dir(&self) -> PathBuf {
        self.state_home.join("dosi-tracker")
    }

    /// The complete environment of a child process: HOME, the XDG bases, a
    /// fixed PATH and locale, plus the builder's extra variables. Nothing of
    /// the developer's session is included.
    pub fn env(&self) -> BTreeMap<String, String> {
        let path = |p: &Path| p.to_string_lossy().into_owned();
        let mut env = BTreeMap::new();
        env.insert("HOME".to_string(), path(&self.home));
        env.insert("XDG_CONFIG_HOME".to_string(), path(&self.config_home));
        env.insert("XDG_DATA_HOME".to_string(), path(&self.data_home));
        env.insert("XDG_STATE_HOME".to_string(), path(&self.state_home));
        if let Some(runtime) = &self.runtime_dir {
            env.insert("XDG_RUNTIME_DIR".to_string(), path(runtime));
        }
        env.insert(
            "PATH".to_string(),
            std::env::var("PATH").unwrap_or_else(|_| "/usr/local/bin:/usr/bin:/bin".to_string()),
        );
        env.insert("LANG".to_string(), "C.UTF-8".to_string());
        for (name, value) in &self.extra {
            env.insert(name.clone(), value.clone());
        }
        env
    }

    /// Replace `command`'s environment with [`Self::env`].
    pub fn apply<'a>(&self, command: &'a mut Command) -> &'a mut Command {
        command.env_clear().envs(self.env())
    }

    /// A command for `program` that runs with [`Self::env`] only.
    pub fn command(&self, program: impl AsRef<OsStr>) -> Command {
        let mut command = Command::new(program);
        self.apply(&mut command);
        command
    }
}

impl Default for TestXdg {
    fn default() -> Self {
        Self::new()
    }
}
