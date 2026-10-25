//! Fake `org.freedesktop.Notifications` recording notifications and emitting
//! `ActionInvoked` / `NotificationClosed`, with configurable capabilities.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use zbus::blocking::Connection;
use zbus::interface;
use zbus::zvariant::OwnedValue;

pub const PATH: &str = "/org/freedesktop/Notifications";
pub const INTERFACE: &str = "org.freedesktop.Notifications";

/// One recorded `Notify` call.
#[derive(Debug, Clone, PartialEq)]
pub struct Shown {
    pub id: u32,
    pub app_name: String,
    pub replaces_id: u32,
    pub app_icon: String,
    pub summary: String,
    pub body: String,
    pub actions: Vec<String>,
    pub hint_keys: Vec<String>,
    pub desktop_entry: Option<String>,
    pub resident: Option<bool>,
    pub urgency: Option<u8>,
    pub expire_timeout: i32,
}

#[derive(Default)]
struct State {
    capabilities: Mutex<Vec<String>>,
    shown: Mutex<Vec<Shown>>,
    closed: Mutex<Vec<u32>>,
    next_id: Mutex<u32>,
}

struct Server {
    state: Arc<State>,
}

#[interface(name = "org.freedesktop.Notifications")]
impl Server {
    #[allow(clippy::too_many_arguments)]
    fn notify(
        &self,
        app_name: &str,
        replaces_id: u32,
        app_icon: &str,
        summary: &str,
        body: &str,
        actions: Vec<String>,
        hints: HashMap<String, OwnedValue>,
        expire_timeout: i32,
    ) -> u32 {
        let id = if replaces_id != 0 {
            replaces_id
        } else {
            let mut next = self.state.next_id.lock().unwrap();
            *next += 1;
            *next
        };
        let mut hint_keys: Vec<String> = hints.keys().cloned().collect();
        hint_keys.sort();
        self.state.shown.lock().unwrap().push(Shown {
            id,
            app_name: app_name.into(),
            replaces_id,
            app_icon: app_icon.into(),
            summary: summary.into(),
            body: body.into(),
            actions,
            hint_keys,
            desktop_entry: hints
                .get("desktop-entry")
                .and_then(|v| <&str>::try_from(&**v).ok())
                .map(str::to_owned),
            resident: hints
                .get("resident")
                .and_then(|v| bool::try_from(&**v).ok()),
            urgency: hints.get("urgency").and_then(|v| u8::try_from(&**v).ok()),
            expire_timeout,
        });
        id
    }

    async fn close_notification(
        &self,
        id: u32,
        #[zbus(signal_emitter)] emitter: zbus::object_server::SignalEmitter<'_>,
    ) {
        self.state.closed.lock().unwrap().push(id);
        let _ = emitter
            .emit(INTERFACE, "NotificationClosed", &(id, 3u32))
            .await;
    }

    fn get_capabilities(&self) -> Vec<String> {
        self.state.capabilities.lock().unwrap().clone()
    }

    fn get_server_information(&self) -> (String, String, String, String) {
        (
            "fake".into(),
            "dosi-tests".into(),
            "1.0".into(),
            "1.2".into(),
        )
    }
}

/// A running fake notification server; stops when dropped.
pub struct FakeNotifications {
    conn: Connection,
    state: Arc<State>,
}

impl FakeNotifications {
    pub fn start(bus_address: &str, capabilities: &[&str]) -> Self {
        let state = Arc::new(State::default());
        *state.capabilities.lock().unwrap() = capabilities.iter().map(|c| c.to_string()).collect();
        let conn = zbus::blocking::connection::Builder::address(bus_address)
            .unwrap()
            .name("org.freedesktop.Notifications")
            .unwrap()
            .serve_at(
                PATH,
                Server {
                    state: state.clone(),
                },
            )
            .unwrap()
            .build()
            .expect("fake notification server");
        Self { conn, state }
    }

    pub fn shown(&self) -> Vec<Shown> {
        self.state.shown.lock().unwrap().clone()
    }

    pub fn closed(&self) -> Vec<u32> {
        self.state.closed.lock().unwrap().clone()
    }

    /// The user clicked action `key` of notification `id`.
    pub fn invoke_action(&self, id: u32, key: &str) {
        self.conn
            .emit_signal(None::<&str>, PATH, INTERFACE, "ActionInvoked", &(id, key))
            .unwrap();
    }

    /// The user dismissed notification `id`.
    pub fn dismiss(&self, id: u32) {
        self.conn
            .emit_signal(
                None::<&str>,
                PATH,
                INTERFACE,
                "NotificationClosed",
                &(id, 2u32),
            )
            .unwrap();
    }
}
