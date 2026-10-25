//! `systemctl` PATH shim recording the call sequence of service commands.
//!
//! [`SystemctlShim::new`] writes an executable `systemctl` script into a
//! temporary `bin/` directory. Put [`SystemctlShim::path_var`] first in the
//! `PATH` of the code under test. Every call appends one line to a log: its
//! arguments, the mode of a watched file at that moment (so a test can prove
//! the session env file existed, 0600, *before* `start`), and the names of
//! the environment variables it received. Answers come from files the test
//! can change between calls.

use std::path::{Path, PathBuf};

use tempfile::TempDir;

/// One recorded call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShimCall {
    pub args: Vec<String>,
    /// Octal mode of the watched file (`"600"`), or `"missing"`.
    pub watched_mode: String,
    /// Names of the variables in the shim's environment, sorted.
    pub env_names: Vec<String>,
}

impl ShimCall {
    /// The arguments joined by spaces, e.g. `--user --no-block start dosi-tracker.service`.
    pub fn line(&self) -> String {
        self.args.join(" ")
    }
}

/// A recording `systemctl`.
pub struct SystemctlShim {
    dir: TempDir,
}

impl SystemctlShim {
    /// A shim with a reachable user manager and an inactive, loaded unit.
    /// `watched` is the file whose mode every call records.
    pub fn new(watched: &Path) -> Self {
        let dir = tempfile::Builder::new()
            .prefix("dosi-systemctl-shim-")
            .tempdir()
            .unwrap();
        let bin = dir.path().join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        let root = dir.path().display().to_string();
        let script = format!(
            r#"#!/bin/sh
root='{root}'
watched='{watched}'
mode=missing
if [ -e "$watched" ]; then mode=$(stat -c %a "$watched"); fi
names=$(env | sed -n 's/^\([A-Za-z_][A-Za-z0-9_]*\)=.*/\1/p' | sort | tr '\n' ',')
args=''
for a in "$@"; do args="$args$a"$(printf '\037'); done
printf '%s\t%s\t%s\n' "$args" "$mode" "$names" >> "$root/calls.log"
if [ -f "$root/sleep" ]; then sleep "$(cat "$root/sleep")"; fi
case " $* " in
  *" show-environment "*)
    if [ -f "$root/no-manager" ]; then
      echo "Failed to connect to bus: No medium found" >&2
      exit 1
    fi
    echo "HOME=$HOME"
    exit 0 ;;
  *" show "*)
    cat "$root/show.txt"
    exit 0 ;;
  *)
    if [ -f "$root/fail" ]; then echo "Job failed" >&2; exit 1; fi
    exit 0 ;;
esac
"#,
            watched = watched.display()
        );
        let path = bin.join("systemctl");
        std::fs::write(&path, script).unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        let shim = Self { dir };
        shim.set_unit(
            "loaded",
            "inactive",
            0,
            Some("/usr/lib/systemd/user/dosi-tracker.service"),
        );
        shim
    }

    fn file(&self, name: &str) -> PathBuf {
        self.dir.path().join(name)
    }

    /// `bin/` first, then the system directories (for `sh`, `stat`, `sed`...).
    pub fn path_var(&self) -> String {
        format!(
            "{}:/usr/local/bin:/usr/bin:/bin",
            self.dir.path().join("bin").display()
        )
    }

    /// What `systemctl --user show dosi-tracker.service` answers.
    pub fn set_unit(
        &self,
        load_state: &str,
        active_state: &str,
        main_pid: u32,
        fragment: Option<&str>,
    ) {
        let text = format!(
            "LoadState={load_state}\nActiveState={active_state}\nMainPID={main_pid}\nFragmentPath={}\n",
            fragment.unwrap_or("")
        );
        std::fs::write(self.file("show.txt"), text).unwrap();
    }

    /// No user manager: `show-environment` fails as it does without one.
    pub fn set_user_manager(&self, available: bool) {
        let marker = self.file("no-manager");
        if available {
            let _ = std::fs::remove_file(marker);
        } else {
            std::fs::write(marker, "").unwrap();
        }
    }

    /// Every call sleeps this many seconds first (timeouts).
    pub fn set_sleep(&self, seconds: Option<f32>) {
        match seconds {
            Some(s) => std::fs::write(self.file("sleep"), s.to_string()).unwrap(),
            None => {
                let _ = std::fs::remove_file(self.file("sleep"));
            }
        }
    }

    /// Unit verbs (start, restart, daemon-reload...) fail.
    pub fn set_failing(&self, failing: bool) {
        if failing {
            std::fs::write(self.file("fail"), "").unwrap();
        } else {
            let _ = std::fs::remove_file(self.file("fail"));
        }
    }

    pub fn calls(&self) -> Vec<ShimCall> {
        let Ok(text) = std::fs::read_to_string(self.file("calls.log")) else {
            return Vec::new();
        };
        text.lines()
            .map(|line| {
                let mut parts = line.splitn(3, '\t');
                let args = parts
                    .next()
                    .unwrap_or("")
                    .split('\u{1f}')
                    .filter(|a| !a.is_empty())
                    .map(str::to_owned)
                    .collect();
                let watched_mode = parts.next().unwrap_or("").to_owned();
                let env_names = parts
                    .next()
                    .unwrap_or("")
                    .split(',')
                    .filter(|n| !n.is_empty())
                    .map(str::to_owned)
                    .collect();
                ShimCall {
                    args,
                    watched_mode,
                    env_names,
                }
            })
            .collect()
    }

    /// The calls as `args` lines.
    pub fn lines(&self) -> Vec<String> {
        self.calls().iter().map(ShimCall::line).collect()
    }
}
