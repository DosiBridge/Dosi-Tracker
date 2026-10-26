#![cfg(all(target_os = "linux", feature = "gui", feature = "test-support"))]
//! The window under the harness's Xvfb (software GL): every preview scene
//! renders and the window is found by its title with `xdotool`; against a
//! fake daemon the window says `hello` and subscribes, an `xdotool` click on
//! Pause sends `tracking.pause`, a dead daemon shows the not-running page
//! and a restarted one is reconnected to; SIGTERM ends the process cleanly.
//!
//! `#[ignore]`d: run inside `ci/xvfb-session.sh` (the `harness` stage). The
//! window's controls are located through the test-support layout probe
//! (`DOSI_TEST_GUI_PROBE`), never by guessing pixels.

mod support;

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use dosi_tracker_linux::agent::state::{
    DaemonInfo, IndicatorInfo, ProjectOption, SharedState, TrackerStatus,
};
use dosi_tracker_linux::consent::{ConsentedPermissions, LocalConsent};
use serde_json::Value;
use support::xdg::TestXdg;

const GUI: &str = env!("CARGO_BIN_EXE_dosi-tracker-gui");

/// The first frame under llvmpipe can take a while on a busy runner.
const START: Duration = Duration::from_secs(45);
const WAIT: Duration = Duration::from_secs(20);

const SCENES: [&str; 7] = [
    "main",
    "paused",
    "signed-out",
    "wayland",
    "no-tray",
    "keyring-locked",
    "portal-denied",
];

struct Window {
    child: Child,
    probe: PathBuf,
    log: PathBuf,
}

impl Window {
    fn open(xdg: &TestXdg, display: &str, args: &[&str], dir: &Path, name: &str) -> Self {
        let probe = dir.join(format!("{name}.probe.json"));
        let log = dir.join(format!("{name}.log"));
        let child = xdg
            .command(GUI)
            .args(args)
            .env("DISPLAY", display)
            .env("LIBGL_ALWAYS_SOFTWARE", "1")
            .env("LANG", "C")
            .env("DOSI_TEST_GUI_PROBE", &probe)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(std::fs::File::create(&log).unwrap())
            .spawn()
            .expect("the window starts");
        Self { child, probe, log }
    }

    fn log(&self) -> String {
        std::fs::read_to_string(&self.log).unwrap_or_default()
    }

    fn alive(&mut self) -> bool {
        self.child.try_wait().unwrap().is_none()
    }

    fn probe(&self) -> Option<Value> {
        let text = std::fs::read_to_string(&self.probe).ok()?;
        serde_json::from_str(&text).ok()
    }

    fn wait_page(&mut self, page: &str, timeout: Duration) -> Value {
        let deadline = Instant::now() + timeout;
        loop {
            if let Some(probe) = self.probe().filter(|p| p["page"] == page) {
                return probe;
            }
            assert!(self.alive(), "the window exited early:\n{}", self.log());
            assert!(
                Instant::now() < deadline,
                "no {page} page (last probe {:?}):\n{}",
                self.probe(),
                self.log()
            );
            std::thread::sleep(Duration::from_millis(100));
        }
    }

    /// End it with SIGTERM: it must close its window and exit 0.
    fn terminate(mut self) {
        // SAFETY: kill only sends a signal to our own child.
        unsafe { libc::kill(self.child.id() as libc::pid_t, libc::SIGTERM) };
        let deadline = Instant::now() + WAIT;
        loop {
            if let Some(status) = self.child.try_wait().unwrap() {
                assert!(status.success(), "SIGTERM: {status:?}\n{}", self.log());
                return;
            }
            if Instant::now() >= deadline {
                let _ = self.child.kill();
                panic!("the window did not exit on SIGTERM:\n{}", self.log());
            }
            std::thread::sleep(Duration::from_millis(50));
        }
    }
}

impl Drop for Window {
    fn drop(&mut self) {
        if self.child.try_wait().ok().flatten().is_none() {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }
}

fn xdotool(display: &str, args: &[&str]) -> std::process::Output {
    Command::new("xdotool")
        .args(args)
        .env_clear()
        .env("DISPLAY", display)
        .output()
        .expect("xdotool runs")
}

/// The window titled "Dosi Tracker", found with `xdotool search --name`.
fn find_window(display: &str, timeout: Duration) -> String {
    let deadline = Instant::now() + timeout;
    loop {
        let out = xdotool(
            display,
            &["search", "--onlyvisible", "--name", "^Dosi Tracker$"],
        );
        if let Some(id) = String::from_utf8_lossy(&out.stdout).lines().last() {
            return id.trim().to_owned();
        }
        assert!(Instant::now() < deadline, "no window titled Dosi Tracker");
        std::thread::sleep(Duration::from_millis(200));
    }
}

#[test]
#[ignore = "needs the Xvfb harness (ci/xvfb-session.sh)"]
fn every_preview_scene_renders() {
    let Some(h) = support::xvfb::require_harness() else {
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    for scene in SCENES {
        let xdg = TestXdg::new();
        let mut window = Window::open(&xdg, &h.display, &["--preview", scene], dir.path(), scene);
        let page = if scene == "signed-out" {
            "login"
        } else {
            "main"
        };
        window.wait_page(page, START);
        let id = find_window(&h.display, WAIT);
        assert!(!id.is_empty());
        // A few more frames: nothing in the scene panics.
        std::thread::sleep(Duration::from_millis(500));
        assert!(window.alive(), "{scene}:\n{}", window.log());
        window.terminate();
    }
}

fn signed_in_snapshot() -> Value {
    let mut consent = ConsentedPermissions::all_disabled();
    consent.keyboard = true;
    let s = SharedState {
        status: TrackerStatus::Tracking,
        display_name: "ayesha@acme.test".into(),
        workspace: "Acme".into(),
        projects: vec![ProjectOption {
            id: "p1".into(),
            title: "Website".into(),
        }],
        selected_project: Some("p1".into()),
        tracking_since: Some(chrono::Utc::now()),
        block_started_at: Some(chrono::Utc::now()),
        consent: LocalConsent::Granted(consent),
        indicator: IndicatorInfo {
            tray: true,
            notifications: true,
        },
        daemon: Some(DaemonInfo {
            version: env!("CARGO_PKG_VERSION").into(),
            pid: 4242,
        }),
        ..SharedState::default()
    };
    serde_json::to_value(s).unwrap()
}

fn wait_for(what: &str, timeout: Duration, mut ok: impl FnMut() -> bool) {
    let deadline = Instant::now() + timeout;
    while !ok() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(100));
    }
}

#[test]
#[ignore = "needs the Xvfb harness (ci/xvfb-session.sh)"]
fn the_window_drives_a_fake_daemon_and_survives_its_restart() {
    let Some(h) = support::xvfb::require_harness() else {
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let xdg = TestXdg::new();
    let socket = xdg
        .runtime_dir
        .as_ref()
        .unwrap()
        .join("dosi-tracker")
        .join("daemon.sock");
    let (daemon, methods) = support::fake_daemon::recording(&socket, signed_in_snapshot());
    let mut window = Window::open(&xdg, &h.display, &[], dir.path(), "fake-daemon");
    let probe = window.wait_page("main", START);
    {
        let seen = methods.lock().unwrap();
        assert_eq!(seen.first().map(String::as_str), Some("hello"), "{seen:?}");
        assert!(seen.iter().any(|m| m == "state.subscribe"), "{seen:?}");
    }

    // Click Pause where the window drew it.
    let id = find_window(&h.display, WAIT);
    let probe = if probe["controls"]["toggle"].is_array() {
        probe
    } else {
        wait_for("the pause button", WAIT, || {
            window
                .probe()
                .is_some_and(|p| p["controls"]["toggle"].is_array())
        });
        window.probe().unwrap()
    };
    let ppp = probe["pixels_per_point"].as_f64().unwrap_or(1.0);
    let rect: Vec<f64> = probe["controls"]["toggle"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_f64().unwrap())
        .collect();
    let x = ((rect[0] + rect[2] / 2.0) * ppp).round().to_string();
    let y = ((rect[1] + rect[3] / 2.0) * ppp).round().to_string();
    let out = xdotool(
        &h.display,
        &["mousemove", "--window", &id, &x, &y, "click", "1"],
    );
    assert!(out.status.success(), "{out:?}");
    wait_for("tracking.pause", WAIT, || {
        methods
            .lock()
            .unwrap()
            .iter()
            .any(|m| m == "tracking.pause")
    });

    // The daemon dies: the window says so instead of showing stale data.
    drop(daemon);
    window.wait_page("not_running", WAIT);

    // It comes back: the window reconnects by itself.
    let (_daemon, methods) = support::fake_daemon::recording(&socket, signed_in_snapshot());
    window.wait_page("main", WAIT);
    assert!(methods
        .lock()
        .unwrap()
        .iter()
        .any(|m| m == "state.subscribe"));

    window.terminate();
}

#[test]
fn the_smoke_tests_skip_outside_the_harness() {
    if std::env::var("DOSI_TEST_HARNESS").as_deref() != Ok("1") {
        assert!(support::xvfb::require_harness().is_none());
    }
}
