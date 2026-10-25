#![cfg(target_os = "linux")]
//! Session detection and session events against a fake logind on a private
//! bus standing in for the system bus (`DBUS_SYSTEM_BUS_ADDRESS` in the
//! daemon), plus a fake screensaver on a private session bus:
//!
//! - the probe accepts only our own, active, local X11/Wayland session and
//!   never hands out an X11 display on Wayland;
//! - `LockedHint` gives Locked, `Active=false` gives Disconnected, and the
//!   same lock arriving several ways is passed once;
//! - the delay inhibitor is held only while tracking; `PrepareForSleep(true)`
//!   keeps its fd open until the worker's ack, and closes it within the
//!   budget when no ack comes.

mod support;

use std::sync::Mutex;
use std::time::{Duration, Instant};

use dosi_tracker_linux::agent::session_events::{
    SessionEvents, SessionEventsConfig, SystemEvent, SystemNotice, INHIBIT_MODE, INHIBIT_WHAT,
    INHIBIT_WHO, INHIBIT_WHY,
};
use dosi_tracker_linux::tracking::session_probe::{probe, ProbeSource, SessionEnv, SessionKind};
use support::fakes::login1::{FakeLogind, SessionSpec, SESSION_PATH};
use support::private_bus::PrivateTestBus;
use tokio::sync::mpsc::UnboundedReceiver;

/// The inhibitor fd crosses the bus; a child process spawned by a parallel
/// test (each test starts dbus-daemons) at the wrong moment could inherit a
/// copy and keep the pipe open. These tests therefore run one at a time.
static SERIAL: Mutex<()> = Mutex::new(());

fn serial() -> std::sync::MutexGuard<'static, ()> {
    SERIAL.lock().unwrap_or_else(|e| e.into_inner())
}

fn euid() -> u32 {
    // SAFETY: geteuid has no preconditions and cannot fail.
    unsafe { libc::geteuid() }
}

fn connect(bus: &PrivateTestBus) -> zbus::blocking::Connection {
    zbus::blocking::connection::Builder::address(bus.address())
        .unwrap()
        .build()
        .unwrap()
}

fn env(pairs: &[(&str, &str)]) -> SessionEnv {
    SessionEnv::from_vars(pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())))
}

/// The next notice within `timeout`.
fn next(rx: &mut UnboundedReceiver<SystemNotice>, timeout: Duration) -> Option<SystemNotice> {
    let deadline = Instant::now() + timeout;
    loop {
        match rx.try_recv() {
            Ok(notice) => return Some(notice),
            Err(tokio::sync::mpsc::error::TryRecvError::Empty) if Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(5))
            }
            Err(_) => return None,
        }
    }
}

fn events(rx: &mut UnboundedReceiver<SystemNotice>, quiet: Duration) -> Vec<SystemEvent> {
    let mut out = Vec::new();
    while let Some(notice) = next(rx, quiet) {
        out.push(notice.event);
    }
    out
}

struct Harness {
    _system: PrivateTestBus,
    session: PrivateTestBus,
    logind: FakeLogind,
    events: SessionEvents,
    rx: UnboundedReceiver<SystemNotice>,
}

fn harness(spec: SessionSpec) -> Harness {
    let system = PrivateTestBus::system();
    let session = PrivateTestBus::session();
    let logind = FakeLogind::start(system.address(), spec);
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
    let events = SessionEvents::spawn(
        SessionEventsConfig {
            system_bus: Some(system.address().to_owned()),
            session_bus: Some(session.address().to_owned()),
            session_path: Some(SESSION_PATH.to_owned()),
        },
        tx,
    )
    .expect("session events");
    Harness {
        _system: system,
        session,
        logind,
        events,
        rx,
    }
}

#[test]
fn the_probe_accepts_our_own_active_x11_session() {
    let _serial = serial();
    let system = PrivateTestBus::system();
    let logind = FakeLogind::start(system.address(), SessionSpec::local(euid(), "x11"));
    let conn = connect(&system);
    let info = probe(
        &env(&[
            ("XDG_SESSION_ID", "2"),
            ("DISPLAY", ":7"),
            ("XAUTHORITY", "/run/user/x/auth"),
        ]),
        Some(&conn),
        euid(),
    );
    assert_eq!(info.kind, SessionKind::X11);
    assert_eq!(info.source, ProbeSource::Logind);
    assert_eq!(info.display.as_deref(), Some(":1"), "logind's display");
    assert_eq!(info.xauthority.as_deref(), Some("/run/user/x/auth"));
    assert_eq!(info.session_path.as_deref(), Some(SESSION_PATH));
    assert_eq!(info.desktop.as_deref(), Some("GNOME"));
    assert_eq!(logind.get_session_calls(), ["2"]);
}

#[test]
fn without_a_session_id_the_users_display_session_is_used() {
    let _serial = serial();
    let system = PrivateTestBus::system();
    let _logind = FakeLogind::start(system.address(), SessionSpec::local(euid(), "wayland"));
    let info = probe(
        &env(&[("WAYLAND_DISPLAY", "wayland-0")]),
        Some(&connect(&system)),
        euid(),
    );
    assert_eq!(info.kind, SessionKind::Wayland);
    assert_eq!(info.session_path.as_deref(), Some(SESSION_PATH));
}

#[test]
fn the_probe_rejects_another_users_session() {
    let _serial = serial();
    let system = PrivateTestBus::system();
    let _logind = FakeLogind::start(system.address(), SessionSpec::local(euid() + 1, "x11"));
    let info = probe(
        &env(&[("XDG_SESSION_ID", "2"), ("DISPLAY", ":1")]),
        Some(&connect(&system)),
        euid(),
    );
    assert_eq!(info.kind, SessionKind::None);
    assert_eq!(info.display, None, "no display of another user's session");
    assert!(info.reason.unwrap().contains("another user"));
}

#[test]
fn a_wayland_session_never_yields_an_x11_display() {
    let _serial = serial();
    let system = PrivateTestBus::system();
    let _logind = FakeLogind::start(system.address(), SessionSpec::local(euid(), "wayland"));
    let info = probe(
        &env(&[
            ("XDG_SESSION_ID", "2"),
            ("DISPLAY", ":0"),
            ("XAUTHORITY", "/run/user/x/.mutter-Xwaylandauth"),
            ("WAYLAND_DISPLAY", "wayland-0"),
            ("XDG_SESSION_TYPE", "x11"),
        ]),
        Some(&connect(&system)),
        euid(),
    );
    assert_eq!(
        info.kind,
        SessionKind::Wayland,
        "logind's Type wins over the environment"
    );
    assert_eq!(info.display, None);
    assert_eq!(info.xauthority, None);
    assert_eq!(info.wayland_display.as_deref(), Some("wayland-0"));
}

#[test]
fn inactive_remote_and_text_sessions_are_not_tracked() {
    let _serial = serial();
    for (spec, kind) in [
        (
            SessionSpec {
                active: false,
                ..SessionSpec::local(euid(), "x11")
            },
            SessionKind::None,
        ),
        (
            SessionSpec {
                remote: true,
                ..SessionSpec::local(euid(), "x11")
            },
            SessionKind::Tty,
        ),
        (SessionSpec::local(euid(), "tty"), SessionKind::Tty),
    ] {
        let system = PrivateTestBus::system();
        let _logind = FakeLogind::start(system.address(), spec.clone());
        let info = probe(
            &env(&[("XDG_SESSION_ID", "2"), ("DISPLAY", ":1")]),
            Some(&connect(&system)),
            euid(),
        );
        assert_eq!(info.kind, kind, "{spec:?}");
        assert_eq!(info.display, None);
    }
}

#[test]
fn without_logind_the_environment_decides() {
    let _serial = serial();
    // A system bus nobody serves logind on (a non-systemd distribution).
    let system = PrivateTestBus::system();
    let info = probe(
        &env(&[("XDG_SESSION_TYPE", "x11"), ("DISPLAY", ":3")]),
        Some(&connect(&system)),
        euid(),
    );
    assert_eq!(
        (info.kind, info.source),
        (SessionKind::X11, ProbeSource::Environment)
    );
    assert_eq!(info.display.as_deref(), Some(":3"));
    let info = probe(
        &env(&[("DISPLAY", ":3"), ("WAYLAND_DISPLAY", "wayland-1")]),
        None,
        euid(),
    );
    assert_eq!(info.kind, SessionKind::Wayland);
    assert_eq!(info.display, None);
}

#[test]
fn locks_arrive_once_whatever_announces_them() {
    let _serial = serial();
    let mut h = harness(SessionSpec::local(euid(), "x11"));
    assert!(!h.events.initial().locked && h.events.initial().active);

    // LockedHint alone gives Locked.
    h.logind.set_locked_hint(true);
    assert_eq!(
        events(&mut h.rx, Duration::from_millis(300)),
        [SystemEvent::Locked]
    );
    // The same lock again from logind's Lock signal and GNOME's screensaver.
    h.logind.lock();
    let screensaver = connect(&h.session);
    screensaver
        .emit_signal(
            None::<&str>,
            "/org/gnome/ScreenSaver",
            "org.gnome.ScreenSaver",
            "ActiveChanged",
            &true,
        )
        .unwrap();
    assert!(
        events(&mut h.rx, Duration::from_millis(300)).is_empty(),
        "duplicates are dropped"
    );
    // Unlocking from the screensaver passes once too.
    screensaver
        .emit_signal(
            None::<&str>,
            "/org/gnome/ScreenSaver",
            "org.gnome.ScreenSaver",
            "ActiveChanged",
            &false,
        )
        .unwrap();
    h.logind.unlock();
    h.logind.set_locked_hint(false);
    assert_eq!(
        events(&mut h.rx, Duration::from_millis(300)),
        [SystemEvent::Unlocked]
    );

    // Another session locking is not ours; an unknown interface is ignored.
    h.logind.lock_other_session();
    screensaver
        .emit_signal(
            None::<&str>,
            "/org/example/Saver",
            "org.example.ScreenSaver",
            "ActiveChanged",
            &true,
        )
        .unwrap();
    assert!(events(&mut h.rx, Duration::from_millis(300)).is_empty());
    // The other desktops' screensavers count.
    screensaver
        .emit_signal(
            None::<&str>,
            "/org/mate/ScreenSaver",
            "org.mate.ScreenSaver",
            "ActiveChanged",
            &true,
        )
        .unwrap();
    assert_eq!(
        events(&mut h.rx, Duration::from_millis(300)),
        [SystemEvent::Locked]
    );
}

#[test]
fn an_inactive_session_is_disconnected() {
    let _serial = serial();
    let mut h = harness(SessionSpec::local(euid(), "x11"));
    h.logind.set_active(false);
    assert_eq!(
        events(&mut h.rx, Duration::from_millis(300)),
        [SystemEvent::Disconnected]
    );
    h.logind.set_active(false);
    assert!(events(&mut h.rx, Duration::from_millis(200)).is_empty());
    h.logind.set_active(true);
    assert_eq!(
        events(&mut h.rx, Duration::from_millis(300)),
        [SystemEvent::Connected]
    );
}

#[test]
fn starting_locked_is_reported_by_the_initial_state() {
    let _serial = serial();
    let h = harness(SessionSpec {
        locked_hint: true,
        ..SessionSpec::local(euid(), "x11")
    });
    assert!(h.events.initial().locked);
}

#[test]
fn the_inhibitor_is_held_only_while_tracking() {
    let _serial = serial();
    let h = harness(SessionSpec::local(euid(), "x11"));
    std::thread::sleep(Duration::from_millis(200));
    assert_eq!(h.logind.inhibit_count(), 0, "not tracking: no inhibitor");

    let control = h.events.control();
    control.set_tracking(true);
    assert!(h.logind.wait_inhibits(1, Duration::from_secs(2)));
    h.logind.with_inhibit(0, |call| {
        assert_eq!(
            (
                call.what.as_str(),
                call.who.as_str(),
                call.why.as_str(),
                call.mode.as_str()
            ),
            (INHIBIT_WHAT, INHIBIT_WHO, INHIBIT_WHY, INHIBIT_MODE)
        );
        assert_eq!(INHIBIT_WHAT, "sleep:shutdown");
        assert_eq!(INHIBIT_MODE, "delay");
        assert!(!call.closed(), "held while tracking");
    });
    assert!(h.events.holding_inhibitor());
    control.set_tracking(true);
    std::thread::sleep(Duration::from_millis(200));
    assert_eq!(
        h.logind.inhibit_count(),
        1,
        "one inhibitor, not one per call"
    );

    control.set_tracking(false);
    assert!(h
        .logind
        .with_inhibit(0, |call| call.wait_closed(Duration::from_secs(1))));
    assert!(!h.events.holding_inhibitor());
}

#[test]
fn before_sleep_the_inhibitor_stays_until_the_block_is_saved() {
    let _serial = serial();
    let mut h = harness(SessionSpec::local(euid(), "x11"));
    h.events.control().set_tracking(true);
    assert!(h.logind.wait_inhibits(1, Duration::from_secs(2)));

    h.logind.prepare_for_sleep(true);
    let mut notice = next(&mut h.rx, Duration::from_secs(2)).expect("delivered");
    assert_eq!(notice.event, SystemEvent::Suspending);
    assert!(notice.done.is_some(), "the worker is asked to acknowledge");
    // Saving takes a moment: the fd stays open meanwhile.
    std::thread::sleep(Duration::from_millis(400));
    assert!(
        !h.logind.with_inhibit(0, |c| c.closed()),
        "still held before the ack"
    );
    let acked = Instant::now();
    notice.acknowledge();
    assert!(h
        .logind
        .with_inhibit(0, |c| c.wait_closed(Duration::from_millis(500))));
    assert!(
        acked.elapsed() < Duration::from_millis(500),
        "released at the ack"
    );

    // A repeated announcement is not a second suspend.
    h.logind.prepare_for_sleep(true);
    assert!(next(&mut h.rx, Duration::from_millis(300)).is_none());
    // After the wake, tracking holds a fresh inhibitor.
    h.logind.prepare_for_sleep(false);
    assert_eq!(
        next(&mut h.rx, Duration::from_secs(2)).unwrap().event,
        SystemEvent::Resumed
    );
    assert!(h.logind.wait_inhibits(2, Duration::from_secs(2)));
    assert!(!h.logind.with_inhibit(1, |c| c.closed()));
}

#[test]
fn without_an_ack_the_inhibitor_is_released_when_the_budget_runs_out() {
    let _serial = serial();
    let mut h = harness(SessionSpec::local(euid(), "x11"));
    h.events.control().set_tracking(true);
    assert!(h.logind.wait_inhibits(1, Duration::from_secs(2)));

    h.logind.prepare_for_sleep(true);
    let started = Instant::now();
    let notice = next(&mut h.rx, Duration::from_secs(2)).expect("delivered");
    assert_eq!(notice.event, SystemEvent::Suspending);
    // Nobody acknowledges (the worker is stuck).
    assert!(h
        .logind
        .with_inhibit(0, |c| c.wait_closed(Duration::from_millis(2_500))));
    let waited = started.elapsed();
    assert!(
        waited >= Duration::from_millis(1_300),
        "waited for the budget: {waited:?}"
    );
    assert!(
        waited < Duration::from_millis(2_300),
        "never longer than the budget: {waited:?}"
    );
    drop(notice);
}

#[test]
fn before_shutdown_the_budget_is_two_seconds() {
    let _serial = serial();
    let mut h = harness(SessionSpec::local(euid(), "x11"));
    h.events.control().set_tracking(true);
    assert!(h.logind.wait_inhibits(1, Duration::from_secs(2)));

    h.logind.prepare_for_shutdown(true);
    let started = Instant::now();
    let notice = next(&mut h.rx, Duration::from_secs(2)).expect("delivered");
    assert_eq!(notice.event, SystemEvent::SessionEnding);
    assert!(h
        .logind
        .with_inhibit(0, |c| c.wait_closed(Duration::from_millis(3_000))));
    let waited = started.elapsed();
    assert!(waited >= Duration::from_millis(1_800), "{waited:?}");
    assert!(waited < Duration::from_millis(2_800), "{waited:?}");
    drop(notice);
    // While shutting down no new inhibitor is taken; a cancelled shutdown
    // restores it.
    h.events.control().set_tracking(true);
    std::thread::sleep(Duration::from_millis(200));
    assert_eq!(h.logind.inhibit_count(), 1);
    h.logind.prepare_for_shutdown(false);
    assert_eq!(
        next(&mut h.rx, Duration::from_secs(2)).unwrap().event,
        SystemEvent::SessionContinues
    );
    assert!(h.logind.wait_inhibits(2, Duration::from_secs(2)));
}

#[test]
fn removing_our_session_is_reported_once() {
    let _serial = serial();
    let mut h = harness(SessionSpec::local(euid(), "x11"));
    h.logind
        .session_removed("/org/freedesktop/login1/session/_99");
    assert!(
        next(&mut h.rx, Duration::from_millis(300)).is_none(),
        "another session"
    );
    h.logind.session_removed(SESSION_PATH);
    assert_eq!(
        next(&mut h.rx, Duration::from_secs(2)).unwrap().event,
        SystemEvent::SessionRemoved
    );
    h.logind.session_removed(SESSION_PATH);
    assert!(next(&mut h.rx, Duration::from_millis(300)).is_none());
}

#[test]
fn stopping_releases_the_inhibitor_and_ends_the_threads() {
    let _serial = serial();
    let mut h = harness(SessionSpec::local(euid(), "x11"));
    h.events.control().set_tracking(true);
    assert!(h.logind.wait_inhibits(1, Duration::from_secs(2)));
    let started = Instant::now();
    h.events.stop();
    assert!(started.elapsed() < Duration::from_secs(1));
    assert!(h
        .logind
        .with_inhibit(0, |c| c.wait_closed(Duration::from_millis(500))));
    // Signals after stopping go nowhere.
    h.logind.lock();
    assert!(next(&mut h.rx, Duration::from_millis(200)).is_none());
}
