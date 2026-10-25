//! Self-tests of the shared test support: the guards that keep tests away from
//! the developer's display, buses and home directory must themselves hold.

mod support;

use std::collections::HashMap;

use support::xdg::{TestXdg, SESSION_VARS};
use support::xvfb::{harness_from, Harness};

fn lookup(vars: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
    let map: HashMap<String, String> = vars
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
    move |name| map.get(name).cloned()
}

const FULL: &[(&str, &str)] = &[
    ("DOSI_TEST_HARNESS", "1"),
    ("DOSI_TEST_DISPLAY", ":99"),
    ("DISPLAY", ":99"),
    ("DOSI_TEST_SESSION_BUS", "unix:path=/tmp/h/session-bus/x"),
    ("DOSI_TEST_SYSTEM_BUS", "unix:path=/tmp/h/system-bus/y"),
];

#[test]
fn harness_is_found_only_inside_the_harness_session() {
    assert_eq!(
        harness_from(lookup(FULL)),
        Some(Harness {
            display: ":99".into(),
            session_bus: "unix:path=/tmp/h/session-bus/x".into(),
            system_bus: "unix:path=/tmp/h/system-bus/y".into(),
        })
    );

    // Nothing set (a developer running `cargo test -- --ignored`).
    assert_eq!(harness_from(lookup(&[])), None);
    // A developer's own display, even with the flag set.
    let mut foreign: Vec<_> = FULL.to_vec();
    foreign.retain(|(k, _)| *k != "DISPLAY");
    foreign.push(("DISPLAY", ":0"));
    assert_eq!(harness_from(lookup(&foreign)), None);
}

#[test]
fn harness_requires_every_address_and_the_exact_flag() {
    for missing in [
        "DOSI_TEST_HARNESS",
        "DOSI_TEST_DISPLAY",
        "DOSI_TEST_SESSION_BUS",
        "DOSI_TEST_SYSTEM_BUS",
    ] {
        let vars: Vec<_> = FULL
            .iter()
            .copied()
            .filter(|(k, _)| *k != missing)
            .collect();
        assert_eq!(harness_from(lookup(&vars)), None, "without {missing}");
    }
    let mut blank: Vec<_> = FULL.to_vec();
    blank.retain(|(k, _)| *k != "DOSI_TEST_SYSTEM_BUS");
    blank.push(("DOSI_TEST_SYSTEM_BUS", "  "));
    assert_eq!(harness_from(lookup(&blank)), None, "blank address");

    let mut not_one: Vec<_> = FULL.to_vec();
    not_one.retain(|(k, _)| *k != "DOSI_TEST_HARNESS");
    not_one.push(("DOSI_TEST_HARNESS", "true"));
    assert_eq!(harness_from(lookup(&not_one)), None, "flag must be 1");
}

#[test]
fn child_environment_is_explicit_and_carries_no_session() {
    let xdg = TestXdg::builder().var("DOSI_EXTRA", "yes").build();
    let env = xdg.env();
    for var in SESSION_VARS.iter().filter(|v| **v != "XDG_RUNTIME_DIR") {
        assert!(!env.contains_key(*var), "{var} must not reach a child");
    }
    assert_eq!(env["HOME"], xdg.home.to_string_lossy());
    assert_eq!(env["XDG_CONFIG_HOME"], xdg.config_home.to_string_lossy());
    assert_eq!(env["XDG_DATA_HOME"], xdg.data_home.to_string_lossy());
    assert_eq!(env["XDG_STATE_HOME"], xdg.state_home.to_string_lossy());
    assert_eq!(env["DOSI_EXTRA"], "yes");
    assert!(
        xdg.home.starts_with(std::env::temp_dir()),
        "a throwaway HOME"
    );
    assert!(xdg.config_home.is_dir() && xdg.data_home.is_dir() && xdg.state_home.is_dir());
}

#[cfg(unix)]
#[test]
fn child_processes_see_only_the_explicit_environment() {
    use std::os::unix::fs::PermissionsExt;

    let xdg = TestXdg::new();
    let runtime = xdg.runtime_dir.clone().expect("runtime dir by default");
    let mode = std::fs::metadata(&runtime).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o700);

    let output = xdg.command("env").output().expect("env runs");
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let names: Vec<&str> = stdout
        .lines()
        .filter_map(|line| line.split_once('=').map(|(k, _)| k))
        .collect();
    for var in ["DISPLAY", "WAYLAND_DISPLAY", "DBUS_SESSION_BUS_ADDRESS"] {
        assert!(!names.contains(&var), "{var} leaked into the child");
    }
    assert!(names.contains(&"HOME") && names.contains(&"XDG_RUNTIME_DIR"));
}

#[test]
fn trees_without_directories_create_nothing() {
    let xdg = TestXdg::builder()
        .create_dirs(false)
        .runtime_dir(false)
        .build();
    assert!(!xdg.home.exists());
    assert!(xdg.runtime_dir.is_none());
    assert!(!xdg.env().contains_key("XDG_RUNTIME_DIR"));
}

#[cfg(target_os = "linux")]
mod buses {
    use super::support::private_bus::{BusKind, PrivateTestBus};

    #[test]
    fn private_buses_of_both_kinds_accept_connections() {
        let session = PrivateTestBus::session();
        let system = PrivateTestBus::system();
        assert_eq!(session.kind(), BusKind::Session);
        assert_eq!(system.kind(), BusKind::System);
        assert_ne!(session.address(), system.address());

        for bus in [&session, &system] {
            assert!(bus.address().starts_with("unix:"), "{}", bus.address());
            let conn = zbus::blocking::connection::Builder::address(bus.address())
                .unwrap()
                .build()
                .expect("connects to the private bus");
            let dbus = zbus::blocking::fdo::DBusProxy::new(&conn).unwrap();
            let names = dbus.list_names().unwrap();
            assert!(names.iter().any(|n| n.as_str() == "org.freedesktop.DBus"));
        }
    }
}
