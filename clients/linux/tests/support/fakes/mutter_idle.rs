//! Fake `org.gnome.Mutter.IdleMonitor` (`/org/gnome/Mutter/IdleMonitor/Core`).
//!
//! `GetIdletime` answers the idle time set by the test, in milliseconds, and
//! counts its calls.

use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::Arc;

use zbus::blocking::Connection;
use zbus::interface;

#[derive(Default)]
struct State {
    idle_ms: AtomicU64,
    calls: AtomicU32,
}

struct IdleMonitor {
    state: Arc<State>,
}

#[interface(name = "org.gnome.Mutter.IdleMonitor")]
impl IdleMonitor {
    fn get_idletime(&self) -> u64 {
        self.state.calls.fetch_add(1, Ordering::SeqCst);
        self.state.idle_ms.load(Ordering::SeqCst)
    }
}

/// A running fake owning `org.gnome.Mutter.IdleMonitor`; stops when dropped.
pub struct FakeMutterIdle {
    _conn: Connection,
    state: Arc<State>,
}

impl FakeMutterIdle {
    pub fn start(bus_address: &str, idle_ms: u64) -> Self {
        let state = Arc::new(State::default());
        state.idle_ms.store(idle_ms, Ordering::SeqCst);
        let conn = zbus::blocking::connection::Builder::address(bus_address)
            .unwrap()
            .name("org.gnome.Mutter.IdleMonitor")
            .unwrap()
            .serve_at(
                "/org/gnome/Mutter/IdleMonitor/Core",
                IdleMonitor {
                    state: state.clone(),
                },
            )
            .unwrap()
            .build()
            .expect("fake Mutter idle monitor");
        Self { _conn: conn, state }
    }

    pub fn set_idle_ms(&self, idle_ms: u64) {
        self.state.idle_ms.store(idle_ms, Ordering::SeqCst);
    }

    pub fn calls(&self) -> u32 {
        self.state.calls.load(Ordering::SeqCst)
    }
}
