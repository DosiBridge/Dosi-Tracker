//! An isolated agent for end-to-end tests: a temporary HOME/XDG tree, the
//! `dosi-tracker` daemon as a subprocess (on the fake machine in builds with
//! `test-support`), and CLI invocations with exactly that environment.
//!
//! The file credential store is opted into (there is no Secret Service in
//! the tests), CLI calls never start a daemon by themselves unless a test
//! asks for it, and nothing of the developer's session is passed on.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::time::{Duration, Instant};

use serde_json::Value;

use super::xdg::TestXdg;

pub const EXE: &str = env!("CARGO_BIN_EXE_dosi-tracker");

/// One isolated agent.
pub struct Agent {
    pub xdg: TestXdg,
    /// Extra variables for the daemon and the CLI.
    pub vars: Vec<(String, String)>,
    pub daemon: Option<Child>,
    /// The daemon's stderr (startup errors only; logs go to files).
    pub stderr_file: PathBuf,
    /// `away` / `present` for the fake machine's synthetic user.
    pub input_file: PathBuf,
}

impl Agent {
    /// A tree with a runtime dir, the file credential store and, for the
    /// fake machine, a 60x time scale.
    pub fn new() -> Self {
        Self::with_xdg(TestXdg::new())
    }

    pub fn with_xdg(xdg: TestXdg) -> Self {
        let root = xdg.home.parent().unwrap().to_path_buf();
        let input_file = root.join("fake-input");
        let vars = vec![
            ("DOSI__CREDENTIALS__STORE".to_string(), "file".to_string()),
            ("DOSI_TEST_MACHINE".to_string(), "fake".to_string()),
            ("DOSI_TEST_TIME_SCALE".to_string(), "60".to_string()),
            (
                "DOSI_TEST_INPUT_FILE".to_string(),
                input_file.to_string_lossy().into_owned(),
            ),
        ];
        Self {
            stderr_file: root.join("daemon.stderr"),
            input_file,
            xdg,
            vars,
            daemon: None,
        }
    }

    pub fn var(mut self, name: &str, value: &str) -> Self {
        self.vars.retain(|(k, _)| k != name);
        self.vars.push((name.to_string(), value.to_string()));
        self
    }

    pub fn without_var(mut self, name: &str) -> Self {
        self.vars.retain(|(k, _)| k != name);
        self
    }

    /// The command for `exe` with the agent's environment.
    pub fn command(&self, exe: impl AsRef<std::ffi::OsStr>) -> Command {
        let mut command = self.xdg.command(exe);
        command.envs(self.vars.iter().map(|(k, v)| (k.as_str(), v.as_str())));
        command
    }

    pub fn socket(&self) -> PathBuf {
        self.xdg
            .runtime_dir
            .as_ref()
            .expect("a runtime dir")
            .join("dosi-tracker")
            .join("daemon.sock")
    }

    /// Start `dosi-tracker daemon` and wait for its socket.
    pub fn start_daemon(&mut self) {
        self.start_daemon_with(EXE, &[]);
    }

    pub fn start_daemon_with(&mut self, exe: impl AsRef<Path>, extra: &[(&str, &str)]) {
        let stderr = std::fs::File::create(&self.stderr_file).unwrap();
        let mut command = self.command(exe.as_ref());
        command
            .arg("daemon")
            .envs(extra.iter().copied())
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(stderr);
        let child = command.spawn().expect("start the daemon");
        self.daemon = Some(child);
        assert!(
            self.wait_socket(Duration::from_secs(15)),
            "the daemon did not listen: {}",
            std::fs::read_to_string(&self.stderr_file).unwrap_or_default()
        );
    }

    /// Wait until the agent answers `status`.
    pub fn wait_socket(&self, timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        loop {
            if self.socket().exists() && self.cli(&["status", "--json"]).status.code() != Some(3) {
                return true;
            }
            if Instant::now() >= deadline {
                return false;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    }

    /// Run the CLI (never auto-starting the daemon).
    pub fn cli(&self, args: &[&str]) -> Output {
        self.command(EXE)
            .args(args)
            .env("DOSI_NO_AUTOSTART", "1")
            .stdin(Stdio::null())
            .output()
            .expect("run the CLI")
    }

    /// Run the CLI with `stdin`.
    pub fn cli_stdin(&self, args: &[&str], stdin: &str) -> Output {
        let mut child = self
            .command(EXE)
            .args(args)
            .env("DOSI_NO_AUTOSTART", "1")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("run the CLI");
        child
            .stdin
            .take()
            .unwrap()
            .write_all(stdin.as_bytes())
            .unwrap();
        child.wait_with_output().unwrap()
    }

    /// `--json` output of `args` as JSON.
    pub fn json(&self, args: &[&str]) -> (Option<i32>, Value) {
        let mut all = vec!["--json"];
        all.extend_from_slice(args);
        let output = self.cli(&all);
        (output.status.code(), parse(&output))
    }

    /// The snapshot (`status --json` result).
    pub fn snapshot(&self) -> Value {
        let (code, value) = self.json(&["status"]);
        assert_eq!(code, Some(0), "status: {value}");
        value["result"].clone()
    }

    /// Wait until the snapshot satisfies `pred`.
    pub fn wait_state(&self, timeout: Duration, pred: impl Fn(&Value) -> bool) -> Value {
        let deadline = Instant::now() + timeout;
        loop {
            let snapshot = self.snapshot();
            if pred(&snapshot) {
                return snapshot;
            }
            if Instant::now() >= deadline {
                let logs = self.logs();
                let tail: Vec<&str> = logs.lines().rev().take(40).collect();
                panic!(
                    "timed out waiting for the state; last: {snapshot:#}\nlog tail:\n{}",
                    tail.into_iter().rev().collect::<Vec<_>>().join("\n")
                );
            }
            std::thread::sleep(Duration::from_millis(200));
        }
    }

    pub fn set_present(&self, present: bool) {
        std::fs::write(&self.input_file, if present { "present" } else { "away" }).unwrap();
    }

    /// Send `signal` to the daemon and wait (at most `timeout`) for its exit code.
    pub fn signal_daemon(&mut self, signal: i32, timeout: Duration) -> Option<i32> {
        let child = self.daemon.as_mut()?;
        // SAFETY: kill on our own child's pid.
        unsafe { libc::kill(child.id() as i32, signal) };
        wait_child(child, timeout)
    }

    pub fn daemon_pid(&self) -> Option<u32> {
        self.daemon.as_ref().map(Child::id)
    }

    /// Every file under the temporary root, with its contents.
    pub fn all_files(&self) -> Vec<(PathBuf, Vec<u8>)> {
        let root = self.xdg.home.parent().unwrap().to_path_buf();
        let mut out = Vec::new();
        let mut stack = vec![root];
        while let Some(dir) = stack.pop() {
            let Ok(entries) = std::fs::read_dir(&dir) else {
                continue;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                let Ok(meta) = std::fs::symlink_metadata(&path) else {
                    continue;
                };
                if meta.is_dir() {
                    stack.push(path);
                } else if meta.is_file() {
                    out.push((path.clone(), std::fs::read(&path).unwrap_or_default()));
                }
            }
        }
        out
    }

    /// The daemon's log text.
    pub fn logs(&self) -> String {
        let dir = self.xdg.state_dir().join("logs");
        let mut text = String::new();
        if let Ok(entries) = std::fs::read_dir(dir) {
            for entry in entries.flatten() {
                text.push_str(&std::fs::read_to_string(entry.path()).unwrap_or_default());
            }
        }
        text
    }
}

impl Default for Agent {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for Agent {
    fn drop(&mut self) {
        if let Some(child) = self.daemon.as_mut() {
            // SAFETY: kill on our own child's pid.
            unsafe { libc::kill(child.id() as i32, libc::SIGTERM) };
            if wait_child(child, Duration::from_secs(25)).is_none() {
                let _ = child.kill();
                let _ = child.wait();
            }
        }
    }
}

/// Wait for `child` to exit; its exit code.
pub fn wait_child(child: &mut Child, timeout: Duration) -> Option<i32> {
    let deadline = Instant::now() + timeout;
    loop {
        if let Ok(Some(status)) = child.try_wait() {
            return status.code();
        }
        if Instant::now() >= deadline {
            return None;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// stdout as one JSON document.
pub fn parse(output: &Output) -> Value {
    let stdout = String::from_utf8_lossy(&output.stdout);
    serde_json::from_str(stdout.trim()).unwrap_or_else(|e| {
        panic!(
            "not one JSON document ({e}):\nstdout: {stdout}\nstderr: {}",
            String::from_utf8_lossy(&output.stderr)
        )
    })
}
