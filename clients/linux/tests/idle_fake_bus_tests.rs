#![cfg(target_os = "linux")]
//! Wayland idle sources against fakes on private buses: Mutter IdleMonitor
//! and org.freedesktop.ScreenSaver on a session bus, logind's session idle
//! hint on a bus standing in for the system bus (its minimal fake lives in
//! this file), and the probe order that picks the first working source.

mod support;

use std::sync::{Arc, Mutex};

use chrono::{DateTime, Duration as ChronoDuration, Utc};
use dosi_tracker_linux::tracking::dbus_idle::{
    connect_bus, probe_idle_sources, IdleBuses, LogindIdleSource, MutterIdleSource,
    ScreenSaverIdleSource,
};
use dosi_tracker_linux::tracking::traits::{IdleSource, TrackingError};
use support::fakes::mutter_idle::FakeMutterIdle;
use support::fakes::screensaver::FakeScreenSaver;
use support::private_bus::PrivateTestBus;
use zbus::interface;

const SESSION_PATH: &str = "/org/freedesktop/login1/session/_31";

/// Minimal `org.freedesktop.login1.Session` with the idle hint properties.
#[derive(Clone, Default)]
struct LogindIdle {
    hint: Arc<Mutex<(bool, u64)>>,
}

#[interface(name = "org.freedesktop.login1.Session")]
impl LogindIdle {
    #[zbus(property)]
    fn idle_hint(&self) -> bool {
        self.hint.lock().unwrap().0
    }

    #[zbus(property)]
    fn idle_since_hint(&self) -> u64 {
        self.hint.lock().unwrap().1
    }
}

fn start_logind(
    bus: &PrivateTestBus,
    hint: (bool, u64),
) -> (zbus::blocking::Connection, LogindIdle) {
    let fake = LogindIdle::default();
    *fake.hint.lock().unwrap() = hint;
    let conn = zbus::blocking::connection::Builder::address(bus.address())
        .unwrap()
        .name("org.freedesktop.login1")
        .unwrap()
        .serve_at(SESSION_PATH, fake.clone())
        .unwrap()
        .build()
        .expect("fake logind");
    (conn, fake)
}

fn now() -> DateTime<Utc> {
    DateTime::from_timestamp(1_790_000_000, 0).unwrap()
}

#[test]
fn mutter_reports_milliseconds_since_the_last_input() {
    let bus = PrivateTestBus::session();
    let fake = FakeMutterIdle::start(bus.address(), 90_500);
    let source = MutterIdleSource::new(connect_bus(bus.address()).unwrap());
    assert_eq!(source.kind(), "mutter");
    assert_eq!(
        source.last_input(now()).unwrap(),
        Some(now() - ChronoDuration::milliseconds(90_500))
    );
    fake.set_idle_ms(0);
    assert_eq!(source.last_input(now()).unwrap(), Some(now()));
    assert_eq!(fake.calls(), 2);
}

#[test]
fn the_screensaver_reports_seconds_since_the_last_input() {
    let bus = PrivateTestBus::session();
    let fake = FakeScreenSaver::start(bus.address(), 42);
    let source = ScreenSaverIdleSource::new(connect_bus(bus.address()).unwrap());
    assert_eq!(source.kind(), "screensaver");
    assert_eq!(
        source.last_input(now()).unwrap(),
        Some(now() - ChronoDuration::seconds(42))
    );
    // GNOME's ScreenSaver refuses the call: an error, never a fabricated 0.
    fake.gnome_mode();
    assert!(matches!(
        source.last_input(now()),
        Err(TrackingError::IoError(_))
    ));
}

#[test]
fn logind_reports_the_idle_hint_of_the_session() {
    let system = PrivateTestBus::system();
    let (_conn, fake) = start_logind(&system, (false, 0));
    let source = LogindIdleSource::new(connect_bus(system.address()).unwrap(), SESSION_PATH);
    assert_eq!(source.kind(), "logind");
    // Not idle: input is "now".
    assert_eq!(source.last_input(now()).unwrap(), Some(now()));
    // Idle since a known moment (µs since the epoch).
    let since = now() - ChronoDuration::minutes(7);
    *fake.hint.lock().unwrap() = (true, since.timestamp_micros() as u64);
    assert_eq!(source.last_input(now()).unwrap(), Some(since));
    // Idle without a start: unknown.
    *fake.hint.lock().unwrap() = (true, 0);
    assert_eq!(source.last_input(now()).unwrap(), None);
    // Another session path does not exist: an error.
    let missing = LogindIdleSource::new(
        connect_bus(system.address()).unwrap(),
        "/org/freedesktop/login1/session/_99",
    );
    assert!(missing.last_input(now()).is_err());
}

#[test]
fn absent_services_are_errors_not_zero_idle() {
    let bus = PrivateTestBus::session();
    let conn = connect_bus(bus.address()).unwrap();
    assert!(MutterIdleSource::new(conn.clone())
        .last_input(now())
        .is_err());
    assert!(ScreenSaverIdleSource::new(conn).last_input(now()).is_err());
}

#[test]
fn the_probe_picks_the_first_working_source_for_the_desktop() {
    let session = PrivateTestBus::session();
    let system = PrivateTestBus::system();
    let _mutter = FakeMutterIdle::start(session.address(), 1_000);
    let screensaver = FakeScreenSaver::start(session.address(), 1);
    let (_logind, _fake) = start_logind(&system, (false, 0));
    let buses = IdleBuses {
        session: Some(connect_bus(session.address()).unwrap()),
        system: Some(connect_bus(system.address()).unwrap()),
        logind_session: Some(SESSION_PATH.into()),
    };

    let kind = |desktop: Option<&str>, buses: &IdleBuses| {
        probe_idle_sources(desktop, buses).map(|source| source.kind())
    };
    // GNOME prefers Mutter; KDE and others the freedesktop ScreenSaver.
    assert_eq!(kind(Some("ubuntu:GNOME"), &buses), Some("mutter"));
    assert_eq!(kind(Some("KDE"), &buses), Some("screensaver"));
    assert_eq!(kind(None, &buses), Some("screensaver"));

    // GNOME's ScreenSaver refuses the call; without Mutter, logind remains.
    screensaver.gnome_mode();
    let no_mutter_session = PrivateTestBus::session();
    let gnome_ss = FakeScreenSaver::start(no_mutter_session.address(), 1);
    gnome_ss.gnome_mode();
    let buses_without_mutter = IdleBuses {
        session: Some(connect_bus(no_mutter_session.address()).unwrap()),
        ..buses.clone()
    };
    assert_eq!(kind(Some("GNOME"), &buses_without_mutter), Some("logind"));

    // Nothing answers: manual mode.
    let empty = PrivateTestBus::session();
    let nothing = IdleBuses {
        session: Some(connect_bus(empty.address()).unwrap()),
        system: None,
        logind_session: None,
    };
    assert_eq!(kind(Some("sway"), &nothing), None);
}
