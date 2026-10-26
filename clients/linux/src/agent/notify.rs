//! Desktop notifications over `org.freedesktop.Notifications` (design 13):
//! `Notify`, `CloseNotification`, `GetCapabilities`, and the `ActionInvoked`
//! / `NotificationClosed` signals delivered to a callback.
//!
//! Every notification carries the hint `desktop-entry =
//! io.github.dosibridge.DosiTracker` (so the desktop shows the app's name and
//! icon). Actions are sent only to servers that advertise `actions`, and the
//! `resident` hint (the resident "Dosi Tracker is tracking" notice when no
//! tray exists) only to servers with `persistence`. Signals about other
//! applications' notifications are ignored.
//!
//! The bus address is handed in explicitly; nothing here reads
//! `DBUS_SESSION_BUS_ADDRESS`.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use zbus::blocking::{Connection, MessageIterator};
use zbus::zvariant::Value;

use crate::paths::APP_ID;

pub const NOTIFICATIONS_BUS_NAME: &str = "org.freedesktop.Notifications";
pub const NOTIFICATIONS_PATH: &str = "/org/freedesktop/Notifications";
pub const NOTIFICATIONS_INTERFACE: &str = "org.freedesktop.Notifications";

/// `app_name` of every notification.
pub const APP_NAME: &str = "Dosi Tracker";

/// How long one call to the notification server may take.
pub const CALL_TIMEOUT: Duration = Duration::from_secs(2);

/// The `urgency` hint.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Urgency {
    Low,
    #[default]
    Normal,
    Critical,
}

impl Urgency {
    fn byte(self) -> u8 {
        match self {
            Urgency::Low => 0,
            Urgency::Normal => 1,
            Urgency::Critical => 2,
        }
    }
}

/// One notification to show.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Notification {
    pub summary: String,
    pub body: String,
    /// `(key, label)` pairs; `ActionInvoked` reports the key.
    pub actions: Vec<(String, String)>,
    /// Stay until removed (the resident tracking notice).
    pub resident: bool,
    pub urgency: Urgency,
    /// Replace this earlier notification instead of adding one.
    pub replaces: Option<u32>,
    /// Milliseconds; `-1` lets the server decide, `0` never expires.
    pub expire_timeout_ms: i32,
}

impl Notification {
    pub fn new(summary: impl Into<String>, body: impl Into<String>) -> Self {
        Self {
            summary: summary.into(),
            body: body.into(),
            actions: Vec::new(),
            resident: false,
            urgency: Urgency::Normal,
            replaces: None,
            expire_timeout_ms: -1,
        }
    }

    pub fn with_action(mut self, key: impl Into<String>, label: impl Into<String>) -> Self {
        self.actions.push((key.into(), label.into()));
        self
    }

    pub fn resident(mut self) -> Self {
        self.resident = true;
        self.expire_timeout_ms = 0;
        self
    }

    pub fn replacing(mut self, id: Option<u32>) -> Self {
        self.replaces = id;
        self
    }

    pub fn urgency(mut self, urgency: Urgency) -> Self {
        self.urgency = urgency;
        self
    }
}

/// Why the server closed a notification (`NotificationClosed` reason).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CloseReason {
    Expired,
    Dismissed,
    /// `CloseNotification` was called.
    Closed,
    Undefined,
}

impl CloseReason {
    pub fn from_code(code: u32) -> Self {
        match code {
            1 => CloseReason::Expired,
            2 => CloseReason::Dismissed,
            3 => CloseReason::Closed,
            _ => CloseReason::Undefined,
        }
    }
}

/// What happened to one of our notifications.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NotifyEvent {
    ActionInvoked { id: u32, action: String },
    Closed { id: u32, reason: CloseReason },
}

/// Why a notification could not be shown.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NotifyError {
    /// No notification server on the session bus.
    Unavailable(String),
    Failed(String),
}

impl std::fmt::Display for NotifyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            NotifyError::Unavailable(why) => write!(f, "no notification server: {why}"),
            NotifyError::Failed(why) => write!(f, "notification failed: {why}"),
        }
    }
}

impl std::error::Error for NotifyError {}

fn is_unavailable(e: &zbus::Error) -> bool {
    match e {
        zbus::Error::MethodError(name, _, _) => matches!(
            name.as_str(),
            "org.freedesktop.DBus.Error.ServiceUnknown"
                | "org.freedesktop.DBus.Error.NameHasNoOwner"
        ),
        zbus::Error::FDO(fdo) => matches!(
            **fdo,
            zbus::fdo::Error::ServiceUnknown(_) | zbus::fdo::Error::NameHasNoOwner(_)
        ),
        _ => false,
    }
}

fn failed(e: zbus::Error) -> NotifyError {
    if is_unavailable(&e) {
        NotifyError::Unavailable(e.to_string())
    } else {
        NotifyError::Failed(e.to_string())
    }
}

/// A connection to the notification server, with a `dosi-notify` thread
/// delivering signals about our notifications to the callback. Dropping it
/// closes the connection, which ends the thread.
pub struct Notifier {
    conn: Connection,
    capabilities: Vec<String>,
    ours: Arc<Mutex<HashSet<u32>>>,
}

impl Notifier {
    /// Connect to the session bus at `address`, ask for the server's
    /// capabilities and start listening. Fails with
    /// [`NotifyError::Unavailable`] when no server runs.
    pub fn connect(
        address: &str,
        on_event: impl Fn(NotifyEvent) + Send + 'static,
    ) -> Result<Self, NotifyError> {
        let conn = zbus::blocking::connection::Builder::address(address)
            .map_err(|e| NotifyError::Unavailable(e.to_string()))?
            .method_timeout(CALL_TIMEOUT)
            .build()
            .map_err(|e| NotifyError::Unavailable(e.to_string()))?;
        // Subscribe first, so no answer to a notification is missed.
        let rule = zbus::MatchRule::builder()
            .msg_type(zbus::message::Type::Signal)
            .interface(NOTIFICATIONS_INTERFACE)
            .and_then(|b| b.path(NOTIFICATIONS_PATH))
            .map_err(failed)?
            .build();
        let messages = MessageIterator::for_match_rule(rule, &conn, Some(64)).map_err(failed)?;
        let capabilities: Vec<String> = conn
            .call_method(
                Some(NOTIFICATIONS_BUS_NAME),
                NOTIFICATIONS_PATH,
                Some(NOTIFICATIONS_INTERFACE),
                "GetCapabilities",
                &(),
            )
            .and_then(|reply| reply.body().deserialize::<Vec<String>>())
            .map_err(failed)?;

        let ours = Arc::new(Mutex::new(HashSet::new()));
        let listener_ours = ours.clone();
        std::thread::Builder::new()
            .name("dosi-notify".into())
            .spawn(move || {
                for message in messages {
                    let Ok(message) = message else { continue };
                    if let Some(event) = decode_signal(&message) {
                        let id = match &event {
                            NotifyEvent::ActionInvoked { id, .. }
                            | NotifyEvent::Closed { id, .. } => *id,
                        };
                        let mut ids = listener_ours.lock().unwrap_or_else(|e| e.into_inner());
                        if !ids.contains(&id) {
                            continue;
                        }
                        if matches!(event, NotifyEvent::Closed { .. }) {
                            ids.remove(&id);
                        }
                        drop(ids);
                        on_event(event);
                    }
                }
            })
            .map_err(|e| NotifyError::Failed(e.to_string()))?;
        Ok(Self {
            conn,
            capabilities,
            ours,
        })
    }

    /// What the server said it supports (`actions`, `body`, `persistence`, ...).
    pub fn capabilities(&self) -> &[String] {
        &self.capabilities
    }

    pub fn supports(&self, capability: &str) -> bool {
        self.capabilities.iter().any(|c| c == capability)
    }

    /// The `Notify` arguments for `notification` on this server.
    fn arguments<'a>(
        &self,
        notification: &'a Notification,
    ) -> (Vec<&'a str>, HashMap<&'static str, Value<'a>>) {
        let actions = if self.supports("actions") {
            notification
                .actions
                .iter()
                .flat_map(|(key, label)| [key.as_str(), label.as_str()])
                .collect()
        } else {
            Vec::new()
        };
        let mut hints: HashMap<&'static str, Value<'a>> = HashMap::new();
        hints.insert("desktop-entry", Value::from(APP_ID));
        hints.insert("urgency", Value::from(notification.urgency.byte()));
        if notification.resident && self.supports("persistence") {
            hints.insert("resident", Value::from(true));
        }
        (actions, hints)
    }

    /// Show `notification`; returns its id (for [`Self::close`] and replacing).
    pub fn notify(&self, notification: &Notification) -> Result<u32, NotifyError> {
        let (actions, hints) = self.arguments(notification);
        let id: u32 = self
            .conn
            .call_method(
                Some(NOTIFICATIONS_BUS_NAME),
                NOTIFICATIONS_PATH,
                Some(NOTIFICATIONS_INTERFACE),
                "Notify",
                &(
                    APP_NAME,
                    notification.replaces.unwrap_or(0),
                    APP_ID,
                    notification.summary.as_str(),
                    notification.body.as_str(),
                    actions,
                    hints,
                    notification.expire_timeout_ms,
                ),
            )
            .and_then(|reply| reply.body().deserialize::<u32>())
            .map_err(failed)?;
        self.ours
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(id);
        Ok(id)
    }

    /// Remove notification `id`.
    pub fn close(&self, id: u32) -> Result<(), NotifyError> {
        self.conn
            .call_method(
                Some(NOTIFICATIONS_BUS_NAME),
                NOTIFICATIONS_PATH,
                Some(NOTIFICATIONS_INTERFACE),
                "CloseNotification",
                &(id,),
            )
            .map(drop)
            .map_err(failed)
    }
}

impl Drop for Notifier {
    fn drop(&mut self) {
        // Ends the listener's message stream.
        let _ = self.conn.clone().close();
    }
}

fn decode_signal(message: &zbus::Message) -> Option<NotifyEvent> {
    let header = message.header();
    if header.interface()?.as_str() != NOTIFICATIONS_INTERFACE {
        return None;
    }
    let body = message.body();
    match header.member()?.as_str() {
        "ActionInvoked" => {
            let (id, action): (u32, String) = body.deserialize().ok()?;
            Some(NotifyEvent::ActionInvoked { id, action })
        }
        "NotificationClosed" => {
            let (id, reason): (u32, u32) = body.deserialize().ok()?;
            Some(NotifyEvent::Closed {
                id,
                reason: CloseReason::from_code(reason),
            })
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn close_reasons_follow_the_specification() {
        assert_eq!(CloseReason::from_code(1), CloseReason::Expired);
        assert_eq!(CloseReason::from_code(2), CloseReason::Dismissed);
        assert_eq!(CloseReason::from_code(3), CloseReason::Closed);
        assert_eq!(CloseReason::from_code(4), CloseReason::Undefined);
        assert_eq!(CloseReason::from_code(99), CloseReason::Undefined);
    }

    #[test]
    fn notifications_are_built_with_actions_and_residency() {
        let n = Notification::new("Dosi Tracker is tracking Website", "")
            .with_action("pause", "Pause")
            .with_action("open", "Open")
            .resident()
            .replacing(Some(7))
            .urgency(Urgency::Low);
        assert_eq!(n.actions.len(), 2);
        assert!(n.resident);
        assert_eq!(n.expire_timeout_ms, 0, "a resident notice never expires");
        assert_eq!(n.replaces, Some(7));
        assert_eq!(n.urgency.byte(), 0);
        assert_eq!(Notification::new("a", "b").expire_timeout_ms, -1);
    }
}
