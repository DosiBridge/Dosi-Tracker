//! Session and power events — the session locking or going inactive, the
//! machine going to sleep, shutting down or logging out — passed to the
//! worker (CONTRACT §4), ported from the Windows `session_events.rs`
//! ([`SystemEvent`], [`SystemNotice`] and their ack budgets are the same).
//!
//! A `dosi-session` thread listens with zbus's blocking API on:
//!
//! - logind (system bus), our session object: `Lock` / `Unlock`, and
//!   `PropertiesChanged` of `LockedHint` and `Active` (VT switch and user
//!   switching make the session inactive, treated like a lock);
//! - logind's Manager: `PrepareForSleep(bool)`, `PrepareForShutdown(bool)`
//!   and `SessionRemoved` of our session;
//! - the session bus: `ActiveChanged(bool)` of the `org.freedesktop`,
//!   `org.gnome`, `org.cinnamon`, `org.mate` and `org.xfce` ScreenSaver
//!   interfaces.
//!
//! The same lock usually arrives several ways (logind `Lock`, `LockedHint`,
//! the screensaver); [`EventFilter`] passes each change once.
//!
//! Before sleep and shutdown logind waits only for holders of a *delay*
//! inhibitor. The thread holds one (`sleep:shutdown`, "Dosi Tracker",
//! "Saving the block in progress", `delay`) only while the worker tracks
//! ([`InhibitorControl::set_tracking`]); on `PrepareForSleep(true)` /
//! `PrepareForShutdown(true)` it hands the worker a notice with an ack and
//! closes the inhibitor fd once the worker acknowledged — or when the budget
//! (1.5 s before sleep, 2 s before shutdown) runs out.

use std::os::fd::{AsRawFd, OwnedFd};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Utc};
use tokio::sync::mpsc::UnboundedSender;
use zbus::blocking::{Connection, MessageIterator};
use zbus::zvariant::OwnedValue;

use crate::tracking::session_probe::{
    LOGIND_BUS_NAME, LOGIND_PATH, MANAGER_INTERFACE, SESSION_INTERFACE,
};

/// `what` of the delay inhibitor.
pub const INHIBIT_WHAT: &str = "sleep:shutdown";
/// `who` of the delay inhibitor, as `systemd-inhibit --list` shows it.
pub const INHIBIT_WHO: &str = "Dosi Tracker";
/// `why` of the delay inhibitor.
pub const INHIBIT_WHY: &str = "Saving the block in progress";
/// `mode` of the inhibitor.
pub const INHIBIT_MODE: &str = "delay";

/// The ScreenSaver interfaces whose `ActiveChanged` counts as a lock.
pub const SCREENSAVER_INTERFACES: [&str; 5] = [
    "org.freedesktop.ScreenSaver",
    "org.gnome.ScreenSaver",
    "org.cinnamon.ScreenSaver",
    "org.mate.ScreenSaver",
    "org.xfce.ScreenSaver",
];

/// How long one logind call may take.
pub const CALL_TIMEOUT: Duration = Duration::from_secs(2);

/// Something that happened to the session or the machine.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SystemEvent {
    /// The session was locked (logind, the screensaver).
    Locked,
    Unlocked,
    /// The session became inactive (VT switch, user switching): nobody is
    /// at it.
    Disconnected,
    Connected,
    /// The machine is about to sleep (`PrepareForSleep(true)`).
    Suspending,
    /// The machine woke up (`PrepareForSleep(false)`).
    Resumed,
    /// The machine is shutting down or rebooting (`PrepareForShutdown(true)`).
    SessionEnding,
    /// A shutdown that was announced did not happen.
    SessionContinues,
    /// Linux: logind removed our session (logout): end the block and exit.
    SessionRemoved,
}

impl SystemEvent {
    /// How long the thread waits for the worker to save the block in
    /// progress before letting logind go on; `None` for events nobody waits on.
    pub fn ack_budget(self) -> Option<Duration> {
        match self {
            SystemEvent::Suspending => Some(Duration::from_millis(1_500)),
            SystemEvent::SessionEnding => Some(Duration::from_millis(2_000)),
            _ => None,
        }
    }
}

/// An event on its way to the worker, with when it happened.
#[derive(Debug)]
pub struct SystemNotice {
    pub event: SystemEvent,
    pub at: DateTime<Utc>,
    /// Signalled once the block in progress is saved; the session thread is
    /// waiting on it before it lets logind go on.
    pub done: Option<std::sync::mpsc::SyncSender<()>>,
}

impl SystemNotice {
    /// Tell the waiting thread the work is done.
    pub fn acknowledge(&mut self) {
        if let Some(done) = self.done.take() {
            let _ = done.try_send(());
        }
    }
}

/// A D-Bus signal, decoded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Signal {
    /// logind `Session.Lock` / `Session.Unlock` on our session.
    Lock,
    Unlock,
    /// Our session's `LockedHint` and/or `Active` changed.
    SessionProperties {
        locked_hint: Option<bool>,
        active: Option<bool>,
    },
    PrepareForSleep(bool),
    PrepareForShutdown(bool),
    /// `SessionRemoved` of our session.
    SessionRemoved,
    /// A ScreenSaver's `ActiveChanged`.
    ScreenSaverActive(bool),
}

/// Turns signals into events, each change once.
#[derive(Debug, Clone)]
pub struct EventFilter {
    locked: bool,
    active: bool,
    suspending: bool,
    ending: bool,
    removed: bool,
}

impl EventFilter {
    /// Start from the session's current state.
    pub fn new(locked: bool, active: bool) -> Self {
        Self {
            locked,
            active,
            suspending: false,
            ending: false,
            removed: false,
        }
    }

    pub fn suspending(&self) -> bool {
        self.suspending
    }

    pub fn ending(&self) -> bool {
        self.ending
    }

    /// The events `signal` means, in order (a property change can carry two).
    pub fn feed(&mut self, signal: &Signal) -> Vec<SystemEvent> {
        let mut events = Vec::new();
        let lock = |filter: &mut Self, locked: bool, events: &mut Vec<SystemEvent>| {
            if filter.locked != locked {
                filter.locked = locked;
                events.push(if locked {
                    SystemEvent::Locked
                } else {
                    SystemEvent::Unlocked
                });
            }
        };
        match signal {
            Signal::Lock | Signal::ScreenSaverActive(true) => lock(self, true, &mut events),
            Signal::Unlock | Signal::ScreenSaverActive(false) => lock(self, false, &mut events),
            Signal::SessionProperties {
                locked_hint,
                active,
            } => {
                if let Some(locked) = locked_hint {
                    lock(self, *locked, &mut events);
                }
                if let Some(active) = active {
                    if self.active != *active {
                        self.active = *active;
                        events.push(if *active {
                            SystemEvent::Connected
                        } else {
                            SystemEvent::Disconnected
                        });
                    }
                }
            }
            Signal::PrepareForSleep(start) => {
                if self.suspending != *start {
                    self.suspending = *start;
                    events.push(if *start {
                        SystemEvent::Suspending
                    } else {
                        SystemEvent::Resumed
                    });
                }
            }
            Signal::PrepareForShutdown(start) => {
                if self.ending != *start {
                    self.ending = *start;
                    events.push(if *start {
                        SystemEvent::SessionEnding
                    } else {
                        SystemEvent::SessionContinues
                    });
                }
            }
            Signal::SessionRemoved => {
                if !self.removed {
                    self.removed = true;
                    events.push(SystemEvent::SessionRemoved);
                }
            }
        }
        events
    }
}

/// Decode a signal from logind (`from_logind`) or the session bus.
/// `session_path` is our logind session; signals of other sessions are ignored.
pub fn decode(
    message: &zbus::Message,
    from_logind: bool,
    session_path: Option<&str>,
) -> Option<Signal> {
    let header = message.header();
    let interface = header.interface()?.as_str().to_owned();
    let member = header.member()?.as_str().to_owned();
    let path = header
        .path()
        .map(|p| p.as_str().to_owned())
        .unwrap_or_default();
    let body = message.body();
    if !from_logind {
        if member == "ActiveChanged" && SCREENSAVER_INTERFACES.contains(&interface.as_str()) {
            return body
                .deserialize::<bool>()
                .ok()
                .map(Signal::ScreenSaverActive);
        }
        return None;
    }
    let ours = session_path.is_some_and(|ours| ours == path);
    match (interface.as_str(), member.as_str()) {
        (MANAGER_INTERFACE, "PrepareForSleep") if path == LOGIND_PATH => {
            body.deserialize::<bool>().ok().map(Signal::PrepareForSleep)
        }
        (MANAGER_INTERFACE, "PrepareForShutdown") if path == LOGIND_PATH => body
            .deserialize::<bool>()
            .ok()
            .map(Signal::PrepareForShutdown),
        (MANAGER_INTERFACE, "SessionRemoved") if path == LOGIND_PATH => {
            let (_id, removed): (String, zbus::zvariant::OwnedObjectPath) =
                body.deserialize().ok()?;
            session_path
                .is_some_and(|ours| ours == removed.as_str())
                .then_some(Signal::SessionRemoved)
        }
        (SESSION_INTERFACE, "Lock") if ours => Some(Signal::Lock),
        (SESSION_INTERFACE, "Unlock") if ours => Some(Signal::Unlock),
        ("org.freedesktop.DBus.Properties", "PropertiesChanged") if ours => {
            let (changed_interface, changed, _invalidated): (
                String,
                std::collections::HashMap<String, OwnedValue>,
                Vec<String>,
            ) = body.deserialize().ok()?;
            if changed_interface != SESSION_INTERFACE {
                return None;
            }
            let flag = |name: &str| changed.get(name).and_then(|v| bool::try_from(&**v).ok());
            let locked_hint = flag("LockedHint");
            let active = flag("Active");
            (locked_hint.is_some() || active.is_some()).then_some(Signal::SessionProperties {
                locked_hint,
                active,
            })
        }
        _ => None,
    }
}

/// Where to listen.
#[derive(Debug, Clone, Default)]
pub struct SessionEventsConfig {
    /// logind's bus (`DBUS_SYSTEM_BUS_ADDRESS` or the default); `None`: no logind.
    pub system_bus: Option<String>,
    /// The session bus (screensavers); `None`: none.
    pub session_bus: Option<String>,
    /// Our logind session object (from the session probe).
    pub session_path: Option<String>,
}

enum Input {
    Message {
        message: zbus::Message,
        from_logind: bool,
    },
    SetTracking(bool),
    Stop,
}

/// Tells the session thread whether the worker is tracking (the inhibitor is
/// held only then). Cheap to clone; never blocks.
#[derive(Clone)]
pub struct InhibitorControl {
    tx: mpsc::Sender<Input>,
}

impl InhibitorControl {
    pub fn set_tracking(&self, tracking: bool) {
        let _ = self.tx.send(Input::SetTracking(tracking));
    }
}

/// The session's state when listening started.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InitialState {
    pub locked: bool,
    pub active: bool,
}

/// The running `dosi-session` thread; stops when dropped.
pub struct SessionEvents {
    control: InhibitorControl,
    holding: Arc<AtomicBool>,
    initial: InitialState,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl SessionEvents {
    /// Connect, subscribe (before returning, so no signal sent afterwards is
    /// missed) and start the thread. Without any bus there is nothing to
    /// listen to: an error (idle detection and the missed-suspend check
    /// still apply).
    pub fn spawn(
        config: SessionEventsConfig,
        notices: UnboundedSender<SystemNotice>,
    ) -> anyhow::Result<Self> {
        let connect = |address: &str| -> anyhow::Result<Connection> {
            Ok(zbus::blocking::connection::Builder::address(address)?
                .method_timeout(CALL_TIMEOUT)
                .build()?)
        };
        let system = config.system_bus.as_deref().map(connect).transpose()?;
        let session = match config.session_bus.as_deref().map(connect) {
            Some(Ok(conn)) => Some(conn),
            Some(Err(e)) => {
                tracing::warn!(%e, "no session bus: screensaver locks are not seen");
                None
            }
            None => None,
        };
        if system.is_none() && session.is_none() {
            anyhow::bail!("no bus to listen on for session events");
        }

        let (tx, rx) = mpsc::channel::<Input>();
        let mut subscriptions = Vec::new();
        if let Some(conn) = &system {
            let rule = zbus::MatchRule::builder()
                .msg_type(zbus::message::Type::Signal)
                .sender(LOGIND_BUS_NAME)?
                .build();
            subscriptions.push((MessageIterator::for_match_rule(rule, conn, Some(64))?, true));
        }
        if let Some(conn) = &session {
            let rule = zbus::MatchRule::builder()
                .msg_type(zbus::message::Type::Signal)
                .member("ActiveChanged")?
                .build();
            subscriptions.push((
                MessageIterator::for_match_rule(rule, conn, Some(64))?,
                false,
            ));
        }
        for (messages, from_logind) in subscriptions {
            let tx = tx.clone();
            std::thread::Builder::new()
                .name("dosi-session-rx".into())
                .spawn(move || {
                    for message in messages {
                        let Ok(message) = message else { continue };
                        if tx
                            .send(Input::Message {
                                message,
                                from_logind,
                            })
                            .is_err()
                        {
                            break;
                        }
                    }
                })?;
        }

        let initial = match (&system, config.session_path.as_deref()) {
            (Some(conn), Some(path)) => InitialState {
                locked: session_flag(conn, path, "LockedHint").unwrap_or(false),
                active: session_flag(conn, path, "Active").unwrap_or(true),
            },
            _ => InitialState {
                locked: false,
                active: true,
            },
        };
        let holding = Arc::new(AtomicBool::new(false));
        let mut worker = Listener {
            system,
            session,
            session_path: config.session_path,
            notices,
            filter: EventFilter::new(initial.locked, initial.active),
            tracking: false,
            inhibitor: None,
            holding: holding.clone(),
        };
        let thread = std::thread::Builder::new()
            .name("dosi-session".into())
            .spawn(move || worker.run(rx))?;
        Ok(Self {
            control: InhibitorControl { tx },
            holding,
            initial,
            thread: Some(thread),
        })
    }

    pub fn control(&self) -> InhibitorControl {
        self.control.clone()
    }

    /// The session's lock and active state when listening started.
    pub fn initial(&self) -> InitialState {
        self.initial
    }

    /// The delay inhibitor is held right now.
    pub fn holding_inhibitor(&self) -> bool {
        self.holding.load(Ordering::SeqCst)
    }

    /// Stop listening and release the inhibitor.
    pub fn stop(&mut self) {
        let _ = self.control.tx.send(Input::Stop);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

impl Drop for SessionEvents {
    fn drop(&mut self) {
        self.stop();
    }
}

fn session_flag(conn: &Connection, path: &str, name: &str) -> Option<bool> {
    let reply = conn
        .call_method(
            Some(LOGIND_BUS_NAME),
            path,
            Some("org.freedesktop.DBus.Properties"),
            "Get",
            &(SESSION_INTERFACE, name),
        )
        .ok()?;
    let value: OwnedValue = reply.body().deserialize().ok()?;
    bool::try_from(value).ok()
}

/// Take a delay inhibitor from logind.
pub fn take_inhibitor(conn: &Connection) -> zbus::Result<OwnedFd> {
    let reply = conn.call_method(
        Some(LOGIND_BUS_NAME),
        LOGIND_PATH,
        Some(MANAGER_INTERFACE),
        "Inhibit",
        &(INHIBIT_WHAT, INHIBIT_WHO, INHIBIT_WHY, INHIBIT_MODE),
    )?;
    let body = reply.body();
    let fd: zbus::zvariant::Fd<'_> = body.deserialize()?;
    // The copy zbus received is not close-on-exec: mark it so a child spawned
    // meanwhile cannot keep the inhibitor (and so the suspend) waiting.
    // SAFETY: fcntl on a valid descriptor borrowed from the message.
    unsafe {
        libc::fcntl(fd.as_raw_fd(), libc::F_SETFD, libc::FD_CLOEXEC);
    }
    let owned = std::os::fd::AsFd::as_fd(&fd)
        .try_clone_to_owned()
        .map_err(|e| zbus::Error::InputOutput(Arc::new(e)))?;
    Ok(owned)
}

struct Listener {
    system: Option<Connection>,
    session: Option<Connection>,
    session_path: Option<String>,
    notices: UnboundedSender<SystemNotice>,
    filter: EventFilter,
    tracking: bool,
    inhibitor: Option<OwnedFd>,
    holding: Arc<AtomicBool>,
}

impl Listener {
    fn run(&mut self, rx: mpsc::Receiver<Input>) {
        while let Ok(input) = rx.recv() {
            match input {
                Input::Message {
                    message,
                    from_logind,
                } => {
                    if let Some(signal) =
                        decode(&message, from_logind, self.session_path.as_deref())
                    {
                        drop(message);
                        for event in self.filter.feed(&signal) {
                            self.deliver(event);
                        }
                    }
                }
                Input::SetTracking(tracking) => {
                    self.tracking = tracking;
                    self.sync_inhibitor();
                }
                Input::Stop => break,
            }
        }
        self.release();
        // Closing ends the receiving threads' message streams.
        if let Some(conn) = self.system.take() {
            let _ = conn.close();
        }
        if let Some(conn) = self.session.take() {
            let _ = conn.close();
        }
    }

    /// Hold the inhibitor exactly while tracking and no sleep or shutdown is
    /// under way.
    fn sync_inhibitor(&mut self) {
        let want = self.tracking && !self.filter.suspending() && !self.filter.ending();
        if want && self.inhibitor.is_none() {
            if let Some(conn) = &self.system {
                match take_inhibitor(conn) {
                    Ok(fd) => {
                        self.inhibitor = Some(fd);
                        self.holding.store(true, Ordering::SeqCst);
                    }
                    Err(e) => {
                        tracing::warn!(%e, "no delay inhibitor: a suspend may cut the block without saving it")
                    }
                }
            }
        } else if !want {
            self.release();
        }
    }

    fn release(&mut self) {
        if self.inhibitor.take().is_some() {
            self.holding.store(false, Ordering::SeqCst);
        }
    }

    /// Hand `event` to the worker; before sleep or shutdown, wait (at most
    /// the budget) for the block in progress to be saved, then let go.
    fn deliver(&mut self, event: SystemEvent) {
        let at = Utc::now();
        let (done, waiter) = match event.ack_budget() {
            Some(budget) => {
                let (done, waiter) = std::sync::mpsc::sync_channel(1);
                (Some(done), Some((waiter, budget)))
            }
            None => (None, None),
        };
        let sent = self.notices.send(SystemNotice { event, at, done }).is_ok();
        if let Some((waiter, budget)) = waiter {
            if sent && self.inhibitor.is_some() && waiter.recv_timeout(budget).is_err() {
                tracing::warn!(
                    ?event,
                    ?budget,
                    "the block in progress was not saved in time"
                );
            }
            // Let the machine go on.
            self.release();
        }
        if matches!(event, SystemEvent::Resumed | SystemEvent::SessionContinues) {
            self.sync_inhibitor();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn locks_from_every_source_are_passed_once() {
        let mut filter = EventFilter::new(false, true);
        assert_eq!(filter.feed(&Signal::Lock), [SystemEvent::Locked]);
        // logind's LockedHint and the screensaver say the same thing again.
        let hint = Signal::SessionProperties {
            locked_hint: Some(true),
            active: None,
        };
        assert!(filter.feed(&hint).is_empty());
        assert!(filter.feed(&Signal::ScreenSaverActive(true)).is_empty());
        assert_eq!(
            filter.feed(&Signal::ScreenSaverActive(false)),
            [SystemEvent::Unlocked]
        );
        assert!(filter.feed(&Signal::Unlock).is_empty());
        // Starting locked: an unlock is the first change.
        let mut locked = EventFilter::new(true, true);
        assert!(locked.feed(&Signal::Lock).is_empty());
        assert_eq!(locked.feed(&Signal::Unlock), [SystemEvent::Unlocked]);
    }

    #[test]
    fn an_inactive_session_is_a_disconnect_and_one_change_can_carry_two() {
        let mut filter = EventFilter::new(false, true);
        let both = Signal::SessionProperties {
            locked_hint: Some(true),
            active: Some(false),
        };
        assert_eq!(
            filter.feed(&both),
            [SystemEvent::Locked, SystemEvent::Disconnected]
        );
        assert!(filter.feed(&both).is_empty(), "duplicates are dropped");
        let back = Signal::SessionProperties {
            locked_hint: None,
            active: Some(true),
        };
        assert_eq!(filter.feed(&back), [SystemEvent::Connected]);
    }

    #[test]
    fn sleep_shutdown_and_removal_map_to_events() {
        let mut filter = EventFilter::new(false, true);
        assert_eq!(
            filter.feed(&Signal::PrepareForSleep(true)),
            [SystemEvent::Suspending]
        );
        assert!(filter.suspending());
        assert!(
            filter.feed(&Signal::PrepareForSleep(true)).is_empty(),
            "announced once"
        );
        assert_eq!(
            filter.feed(&Signal::PrepareForSleep(false)),
            [SystemEvent::Resumed]
        );
        assert!(
            filter.feed(&Signal::PrepareForShutdown(false)).is_empty(),
            "nothing was announced"
        );
        assert_eq!(
            filter.feed(&Signal::PrepareForShutdown(true)),
            [SystemEvent::SessionEnding]
        );
        assert!(filter.ending());
        assert_eq!(
            filter.feed(&Signal::PrepareForShutdown(false)),
            [SystemEvent::SessionContinues]
        );
        assert_eq!(
            filter.feed(&Signal::SessionRemoved),
            [SystemEvent::SessionRemoved]
        );
        assert!(filter.feed(&Signal::SessionRemoved).is_empty());
    }

    #[test]
    fn only_sleep_and_shutdown_are_waited_on() {
        assert_eq!(
            SystemEvent::Suspending.ack_budget(),
            Some(Duration::from_millis(1_500))
        );
        assert_eq!(
            SystemEvent::SessionEnding.ack_budget(),
            Some(Duration::from_millis(2_000))
        );
        for event in [
            SystemEvent::Locked,
            SystemEvent::Unlocked,
            SystemEvent::Disconnected,
            SystemEvent::Connected,
            SystemEvent::Resumed,
            SystemEvent::SessionContinues,
            SystemEvent::SessionRemoved,
        ] {
            assert_eq!(event.ack_budget(), None, "{event:?}");
        }
        let (done, waiter) = std::sync::mpsc::sync_channel(1);
        let mut notice = SystemNotice {
            event: SystemEvent::Suspending,
            at: Utc::now(),
            done: Some(done),
        };
        notice.acknowledge();
        notice.acknowledge();
        assert!(waiter.try_recv().is_ok());
        assert!(waiter.try_recv().is_err(), "acknowledged once");
    }
}
