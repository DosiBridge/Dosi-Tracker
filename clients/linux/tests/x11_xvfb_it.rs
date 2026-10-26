#![cfg(target_os = "linux")]
//! Sensors against a real X server: the harness Xvfb started by
//! `ci/xvfb-session.sh` (openbox, XI2, XTEST, MIT-SCREEN-SAVER, RandR).
//!
//! Every test is `#[ignore]` and skips unless it runs inside the harness, so
//! a developer's own display is never touched. Synthetic input comes only
//! from `xdotool` (XTEST) into that Xvfb, which the agent must classify as
//! injected; windows are `xterm`s the tests start and kill.
//!
//!   bash ci/xvfb-session.sh cargo test --locked --test x11_xvfb_it -- --ignored --test-threads=1

mod support;

use std::process::{Child, Stdio};
use std::sync::Arc;
use std::time::{Duration, Instant};

use chrono::Utc;
use dosi_tracker_linux::tracking::capture::encode_frame;
use dosi_tracker_linux::tracking::input::InputLog;
use dosi_tracker_linux::tracking::procfs::{ProcFsRunningAppsProvider, ProgramFilter};
use dosi_tracker_linux::tracking::traits::{
    ActiveWindowProvider, CaptureTarget, IdleSource, RunningAppsProvider, ScreenSource,
};
use dosi_tracker_linux::tracking::usage::{UsageRecorder, UsageSampler, STOP_TIMEOUT};
use dosi_tracker_linux::tracking::x11::X11ActiveWindowProvider;
use dosi_tracker_linux::tracking::x11_idle::X11ScreenSaverIdleSource;
use dosi_tracker_linux::tracking::x11_input::{InputKinds, X11InputListener};
use dosi_tracker_linux::tracking::x11_screen::X11ScreenSource;
use dosi_tracker_linux::tracking::SysProcFsReader;
use support::xdg::TestXdg;
use support::xvfb::{require_harness, Harness};

const BOTH: InputKinds = InputKinds {
    keyboard: true,
    mouse: true,
};

/// An explicit child environment pointing at the harness display only.
fn x_env(harness: &Harness) -> TestXdg {
    TestXdg::builder()
        .var("DISPLAY", harness.display.clone())
        // The image ships only Xvfb's built-in fonts; xterm needs a non-UTF-8 locale for them.
        .var("LANG", "C")
        .build()
}

fn run(harness: &Harness, program: &str, args: &[&str]) {
    let xdg = x_env(harness);
    let status = xdg
        .command(program)
        .args(args)
        .stdout(Stdio::null())
        .status()
        .unwrap_or_else(|e| panic!("{program} could not start: {e}"));
    assert!(status.success(), "{program} {args:?} failed: {status}");
}

/// An xterm titled `title` at `+x+y`; killed when dropped.
struct Xterm {
    child: Child,
    _xdg: TestXdg,
}

impl Xterm {
    fn start(harness: &Harness, title: &str, x: i32, y: i32) -> Self {
        let xdg = x_env(harness);
        let child = xdg
            .command("xterm")
            .args([
                "-fn",
                "fixed",
                "-T",
                title,
                "-geometry",
                &format!("80x24+{x}+{y}"),
                "-e",
                "sleep",
                "120",
            ])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("xterm");
        // Xvfb hands a new client the resource ids of the last one, and
        // openbox leaves _NET_ACTIVE_WINDOW on a window that went away: the
        // new xterm can look active by title before openbox manages it. Only
        // a window in _NET_CLIENT_LIST is managed (mapped, framed, focusable).
        let pid = child.id();
        let owners = provider(harness);
        wait_for("openbox to manage the xterm", || {
            owners
                .client_list_pids()
                .ok()
                .filter(|pids| pids.contains(&pid))
                .map(|_| ())
        });
        Self { child, _xdg: xdg }
    }

    fn pid(&self) -> u32 {
        self.child.id()
    }
}

impl Drop for Xterm {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn wait_for<T>(what: &str, mut probe: impl FnMut() -> Option<T>) -> T {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if let Some(value) = probe() {
            return value;
        }
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(50));
    }
}

fn provider(harness: &Harness) -> X11ActiveWindowProvider {
    X11ActiveWindowProvider::with_display(
        harness.display.clone(),
        Some(Arc::new(SysProcFsReader::new())),
    )
}

/// Wait until the window titled `title` is the active one.
fn wait_active(harness: &Harness, title: &str) {
    let provider = provider(harness);
    wait_for(&format!("{title} to become active"), || {
        provider
            .current_active_window()
            .ok()
            .flatten()
            .filter(|w| w.window_title == title)
    });
}

#[test]
#[ignore = "needs the Xvfb harness (ci/xvfb-session.sh)"]
fn xtest_input_is_counted_as_injected_only() {
    let Some(h) = require_harness() else { return };
    let log = Arc::new(InputLog::new());
    let listener = X11InputListener::start(&h.display, log.clone(), BOTH).unwrap();
    let started = Utc::now();
    assert!(listener.is_running());
    assert!(log.is_healthy());

    run(&h, "xdotool", &["type", "--delay", "20", "synthetic"]);
    run(&h, "xdotool", &["click", "1"]);
    run(&h, "xdotool", &["mousemove", "200", "200", "click", "3"]);
    run(&h, "xdotool", &["click", "4"]); // wheel

    // At least the 9 key presses, 2 clicks and the wheel button.
    let deadline = Instant::now() + Duration::from_secs(10);
    let m = loop {
        let m = log.measurements(started, Utc::now() + chrono::Duration::seconds(1));
        if m.injected >= 12 || Instant::now() > deadline {
            break m;
        }
        std::thread::sleep(Duration::from_millis(50));
    };
    assert!(m.injected >= 12, "too few injected events: {m:?}");
    assert!(
        m.key_presses_ms.is_empty(),
        "XTEST keys counted as the user's"
    );
    assert!(m.clicks_ms.is_empty(), "XTEST clicks counted as the user's");
    assert!(
        m.active_seconds.is_empty(),
        "XTEST input counted as activity"
    );
    assert_eq!(log.last_physical_input(), None);
    assert!(m.healthy);

    listener.stop().unwrap();
    assert!(!log.is_healthy(), "after stop, input is not measured");
}

#[test]
#[ignore = "needs the Xvfb harness (ci/xvfb-session.sh)"]
fn the_listener_stops_within_one_second_and_records_nothing_after() {
    let Some(h) = require_harness() else { return };
    let log = Arc::new(InputLog::new());
    let since = Utc::now() - chrono::Duration::seconds(1);
    let listener = X11InputListener::start(
        &h.display,
        log.clone(),
        InputKinds {
            keyboard: true,
            mouse: false,
        },
    )
    .unwrap();
    std::thread::sleep(Duration::from_millis(100));

    let stopping = Instant::now();
    listener.stop().unwrap();
    assert!(
        stopping.elapsed() < STOP_TIMEOUT,
        "{:?}",
        stopping.elapsed()
    );

    run(&h, "xdotool", &["type", "--delay", "10", "after stop"]);
    std::thread::sleep(Duration::from_millis(300));
    let m = log.measurements(since, Utc::now() + chrono::Duration::seconds(1));
    assert_eq!(m.injected, 0, "nothing is recorded once stopped");
    assert!(!m.healthy);
}

#[test]
#[ignore = "needs the Xvfb harness (ci/xvfb-session.sh)"]
fn the_screensaver_idle_time_is_read_from_the_server() {
    let Some(h) = require_harness() else { return };
    let source = X11ScreenSaverIdleSource::with_display(h.display.clone());
    run(&h, "xdotool", &["mousemove", "10", "10"]);
    let idle = source.idle_millis().unwrap();
    assert!(idle < 5_000, "idle {idle} ms right after input");
    std::thread::sleep(Duration::from_millis(1_200));
    assert!(source.idle_millis().unwrap() >= 1_000);
    let now = Utc::now();
    let last = source.last_input(now).unwrap().unwrap();
    assert!(last <= now && now - last < chrono::Duration::seconds(10));
}

#[test]
#[ignore = "needs the Xvfb harness (ci/xvfb-session.sh)"]
fn an_xterm_under_openbox_is_the_active_window_with_its_title() {
    let Some(h) = require_harness() else { return };
    let _xterm = Xterm::start(&h, "Synthetic", 100, 100);
    wait_active(&h, "Synthetic");
    let window = provider(&h).current_active_window().unwrap().unwrap();
    assert_eq!(window.window_title, "Synthetic");
    assert!(
        window.app_name.eq_ignore_ascii_case("xterm"),
        "app name {:?}",
        window.app_name
    );
    // An active window carries no usage numbers by itself.
    assert!(window.seconds.is_none() && window.keyboard_hits.is_none());
}

#[test]
#[ignore = "needs the Xvfb harness (ci/xvfb-session.sh)"]
fn the_client_list_names_our_window_owners_only() {
    let Some(h) = require_harness() else { return };
    let xterm = Xterm::start(&h, "Owner", 100, 100);
    wait_active(&h, "Owner");
    let owners = Arc::new(provider(&h));
    let pids = owners.client_list_pids().unwrap();
    assert!(
        pids.contains(&xterm.pid()),
        "{pids:?} lacks xterm {}",
        xterm.pid()
    );

    let programs = ProcFsRunningAppsProvider::for_current_user(ProgramFilter::WindowOwners(owners))
        .running_programs()
        .unwrap();
    let names: Vec<&str> = programs.iter().map(|p| p.app_name.as_str()).collect();
    assert!(names.contains(&"xterm"), "{names:?}");
    // The shell-less `sleep` child owns no window; the test binary (the
    // "agent") and openbox's helpers are not listed either.
    assert!(!names.contains(&"sleep"), "{names:?}");
    assert!(programs.iter().all(|p| p.window_title.is_empty()));
}

#[test]
#[ignore = "needs the Xvfb harness (ci/xvfb-session.sh)"]
fn getimage_captures_the_monitor_of_the_active_window() {
    let Some(h) = require_harness() else { return };
    // Two RandR monitors side by side and a solid root colour.
    run(&h, "xsetroot", &["-solid", "#3366cc"]);
    run(
        &h,
        "xrandr",
        &["--setmonitor", "DOSI-LEFT", "960/254x1080/286+0+0", "none"],
    );
    run(
        &h,
        "xrandr",
        &[
            "--setmonitor",
            "DOSI-RIGHT",
            "960/254x1080/286+960+0",
            "none",
        ],
    );
    struct Monitors<'a>(&'a Harness);
    impl Drop for Monitors<'_> {
        fn drop(&mut self) {
            for name in ["DOSI-LEFT", "DOSI-RIGHT"] {
                let _ = x_env(self.0)
                    .command("xrandr")
                    .args(["--delmonitor", name])
                    .status();
            }
        }
    }
    let _monitors = Monitors(&h);

    // The active window sits on the right monitor.
    let _xterm = Xterm::start(&h, "OnTheRight", 1200, 100);
    wait_active(&h, "OnTheRight");

    let source = X11ScreenSource::with_display(h.display.clone());
    let frame = source.capture(CaptureTarget::ActiveMonitor).unwrap();
    assert_eq!(frame.image.dimensions(), (960, 1080));
    assert_eq!(frame.monitor.as_ref().unwrap().name, "DOSI-RIGHT");
    // Away from the xterm (monitor x 10 = root x 970), the root colour.
    assert_eq!(frame.image.get_pixel(10, 1000).0, [0x33, 0x66, 0xcc]);
    // The xterm (black text on white by default) is in the frame at its place.
    let inside = frame.image.get_pixel(1200 - 960 + 40, 100 + 40).0;
    assert_ne!(inside, [0x33, 0x66, 0xcc], "the xterm is captured");

    // The whole screen when asked.
    let whole = source.capture(CaptureTarget::WholeScreen).unwrap();
    assert_eq!(whole.image.dimensions(), (1920, 1080));
    assert!(whole.monitor.is_none());
    assert_eq!(whole.image.get_pixel(5, 5).0, [0x33, 0x66, 0xcc]);

    // And it goes through the shared encoder.
    let capture = encode_frame(frame, false).unwrap();
    let jpeg =
        image::load_from_memory_with_format(&capture.jpeg, image::ImageFormat::Jpeg).unwrap();
    assert_eq!((jpeg.width(), jpeg.height()), (960, 1080));
}

#[test]
#[ignore = "needs the Xvfb harness (ci/xvfb-session.sh)"]
fn the_usage_sampler_records_the_active_window_and_stops_within_one_second() {
    let Some(h) = require_harness() else { return };
    let _xterm = Xterm::start(&h, "Sampled", 100, 100);
    wait_active(&h, "Sampled");
    let recorder = Arc::new(UsageRecorder::new());
    recorder.set_active(true);
    let sampler = UsageSampler::start(
        recorder.clone(),
        Arc::new(provider(&h)),
        Duration::from_millis(100),
    )
    .unwrap();
    let slices = wait_for("usage slices", || {
        let slices = recorder.slices_between(0, i64::MAX);
        (!slices.is_empty()).then_some(slices)
    });
    assert!(slices.iter().any(|s| s.title == "Sampled"), "{slices:?}");

    let stopping = Instant::now();
    sampler.stop().unwrap();
    assert!(
        stopping.elapsed() < STOP_TIMEOUT,
        "{:?}",
        stopping.elapsed()
    );
}

#[test]
#[ignore = "needs the Xvfb harness (ci/xvfb-session.sh)"]
fn the_x11_tracker_runs_its_native_sensors_only_while_tracking() {
    use dosi_tracker_linux::agent::model::EndReason;
    use dosi_tracker_linux::agent::state::SessionKind;
    use dosi_tracker_linux::agent::worker::Machine;
    use dosi_tracker_linux::consent::{ConsentManager, ConsentedPermissions, LocalConsent};
    use dosi_tracker_linux::lifecycle::{TrackerLifecycle, TrackerState};
    use dosi_tracker_linux::tracking::build::BlockSpec;
    use dosi_tracker_linux::tracking::dbus_idle::IdleBuses;
    use dosi_tracker_linux::tracking::session_probe::{ProbeSource, SessionInfo};
    use dosi_tracker_linux::tracking::system::DeviceInfo;
    use dosi_tracker_linux::tracking::tracker::Tracker;
    use std::sync::Mutex;

    let Some(h) = require_harness() else { return };
    let _xterm = Xterm::start(&h, "Tracked", 100, 100);
    wait_active(&h, "Tracked");
    let everything = ConsentedPermissions {
        screenshot: true,
        webcam: false,
        keyboard: true,
        mouse: true,
        active_window: true,
        running_programs: true,
        blur_screenshots: false,
    };
    let lifecycle = Arc::new(Mutex::new(TrackerLifecycle::new(ConsentManager::new(
        LocalConsent::Granted(everything),
        None,
    ))));
    let session = SessionInfo {
        kind: SessionKind::X11,
        source: ProbeSource::Environment,
        desktop: Some("openbox".into()),
        session_path: None,
        session_id: None,
        display: Some(h.display.clone()),
        wayland_display: None,
        xauthority: None,
        reason: None,
    };
    let device = DeviceInfo {
        device_id: "5b0c1b9e-8f2c-4a43-9a51-0c7f6f2d9b11".into(),
        device_name: Some("harness".into()),
        os_version: "Ubuntu 22.04 (Linux)".into(),
        agent_version: "0.0.0".into(),
        platform: "linux".into(),
        time_zone: Some("UTC".into()),
    };
    let tracker = Tracker::for_session(
        &session,
        &IdleBuses::default(),
        None,
        lifecycle.clone(),
        device,
    );
    let driver = tracker.driver();
    assert!(
        !driver.is_input_running() && !driver.is_usage_running(),
        "nothing runs before tracking"
    );
    // Not tracking: idle comes from MIT-SCREEN-SAVER.
    assert_eq!(
        tracker.session_summary().idle_source.as_deref(),
        Some("x11-screensaver")
    );

    tracker.apply_sensor_policy(true, Some(&everything));
    assert!(driver.is_input_running() && driver.is_usage_running());
    assert_eq!(lifecycle.lock().unwrap().state(), TrackerState::Tracking);
    assert_eq!(
        tracker.session_summary().idle_source.as_deref(),
        Some("xi2")
    );
    let started = Utc::now();

    // XTEST input is counted as injected only, never as the user's.
    run(&h, "xdotool", &["type", "--delay", "20", "abc"]);
    let log = tracker.input();
    wait_for("injected key presses", || {
        let m = log.measurements(started, Utc::now() + chrono::Duration::seconds(1));
        (m.injected >= 3).then_some(())
    });
    assert_eq!(tracker.current_app().as_deref(), Some("xterm"));
    let shot = tracker
        .capture_screen(false)
        .expect("GetImage at the block's end");

    std::thread::sleep(Duration::from_millis(3_500));
    let ended = Utc::now() + chrono::Duration::seconds(1);
    let block = BlockSpec {
        project_id: "3f2a9c1e-6b7d-4e8f-9a0b-1c2d3e4f5a6b",
        perms: &everything,
        started_at: started,
        ended_at: ended,
        end_reason: Some(EndReason::Interval),
        description: None,
    };
    let built = tracker.snapshot(&block, Some(shot));
    let activity = &built.activity;
    assert_eq!(
        activity.keyboard_hits,
        Some(0),
        "measured, and none of it was physical"
    );
    assert!(
        activity.injected_input_events.unwrap_or(0) >= 3,
        "{activity:?}"
    );
    assert!(activity.timeline.len() == 1 && activity.productivity.is_some());
    assert!(
        activity
            .active_windows
            .iter()
            .any(|w| w.window_title == "Tracked"),
        "{:?}",
        activity.active_windows
    );
    assert!(
        activity
            .running_programs
            .iter()
            .any(|p| p.app_name == "xterm"),
        "{:?}",
        activity.running_programs
    );
    assert!(
        built.screen.is_some(),
        "the capture of this generation is attached"
    );

    let stopping = Instant::now();
    tracker.apply_sensor_policy(false, None);
    assert!(
        stopping.elapsed() < STOP_TIMEOUT * 2,
        "{:?}",
        stopping.elapsed()
    );
    assert!(!driver.is_input_running() && !driver.is_usage_running());
    assert!(!log.is_healthy(), "input is not measured once stopped");
    assert_eq!(lifecycle.lock().unwrap().state(), TrackerState::Stopped);
    assert!(
        tracker.capture_screen(false).is_none(),
        "no capture while not tracking"
    );
}

#[test]
fn outside_the_harness_display_tests_skip() {
    // Run by the plain `cargo test`: the harness variables are absent there,
    // so every display test above returns early without touching a display.
    if std::env::var("DOSI_TEST_HARNESS").as_deref() == Ok("1") {
        assert!(require_harness().is_some());
    } else {
        assert!(require_harness().is_none());
    }
}
