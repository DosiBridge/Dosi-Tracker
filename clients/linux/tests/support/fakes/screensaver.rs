//! Fake `org.freedesktop.ScreenSaver` (idle time, active state, lock signals).
//!
//! `GetSessionIdleTime` answers the idle seconds set by the test, or — in
//! GNOME mode — fails with `org.freedesktop.DBus.Error.NotSupported` as
//! gnome-shell does. `GetActive` answers the active (locked) state.

use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Arc;

use zbus::blocking::Connection;
use zbus::interface;

#[derive(Default)]
struct State {
    idle_seconds: AtomicU32,
    not_supported: AtomicBool,
    active: AtomicBool,
    calls: AtomicU32,
}

struct ScreenSaver {
    state: Arc<State>,
}

#[interface(name = "org.freedesktop.ScreenSaver")]
impl ScreenSaver {
    fn get_session_idle_time(&self) -> zbus::fdo::Result<u32> {
        self.state.calls.fetch_add(1, Ordering::SeqCst);
        if self.state.not_supported.load(Ordering::SeqCst) {
            return Err(zbus::fdo::Error::NotSupported(
                "This method is not part of the idle inhibition specification".into(),
            ));
        }
        Ok(self.state.idle_seconds.load(Ordering::SeqCst))
    }

    fn get_active(&self) -> bool {
        self.state.active.load(Ordering::SeqCst)
    }
}

/// A running fake owning `org.freedesktop.ScreenSaver`; stops when dropped.
pub struct FakeScreenSaver {
    _conn: Connection,
    state: Arc<State>,
}

impl FakeScreenSaver {
    pub fn start(bus_address: &str, idle_seconds: u32) -> Self {
        let state = Arc::new(State::default());
        state.idle_seconds.store(idle_seconds, Ordering::SeqCst);
        let conn = zbus::blocking::connection::Builder::address(bus_address)
            .unwrap()
            .name("org.freedesktop.ScreenSaver")
            .unwrap()
            .serve_at(
                "/org/freedesktop/ScreenSaver",
                ScreenSaver {
                    state: state.clone(),
                },
            )
            .unwrap()
            .build()
            .expect("fake ScreenSaver");
        Self { _conn: conn, state }
    }

    /// Answer `GetSessionIdleTime` with NotSupported, as GNOME does.
    pub fn gnome_mode(&self) {
        self.state.not_supported.store(true, Ordering::SeqCst);
    }

    pub fn set_idle_seconds(&self, seconds: u32) {
        self.state.idle_seconds.store(seconds, Ordering::SeqCst);
    }

    pub fn set_active(&self, active: bool) {
        self.state.active.store(active, Ordering::SeqCst);
    }

    pub fn calls(&self) -> u32 {
        self.state.calls.load(Ordering::SeqCst)
    }
}
