//! Fake `org.freedesktop.login1` Manager, Session and User on a private
//! system bus: lock/unlock, `LockedHint` and `Active` changes,
//! `PrepareForSleep` / `PrepareForShutdown`, `SessionRemoved`, and `Inhibit`
//! returning the write end of a pipe whose read end the fake keeps — so a
//! test can see exactly when the agent closes its inhibitor.

use std::collections::HashMap;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use zbus::blocking::Connection;
use zbus::interface;
use zbus::zvariant::{OwnedObjectPath, Value};

pub const MANAGER_PATH: &str = "/org/freedesktop/login1";
pub const SESSION_PATH: &str = "/org/freedesktop/login1/session/_32";
pub const SESSION_INTERFACE: &str = "org.freedesktop.login1.Session";
pub const MANAGER_INTERFACE: &str = "org.freedesktop.login1.Manager";

/// The session the fake serves.
#[derive(Debug, Clone)]
pub struct SessionSpec {
    pub id: String,
    pub uid: u32,
    pub session_type: String,
    pub active: bool,
    pub remote: bool,
    pub display: String,
    pub desktop: String,
    pub locked_hint: bool,
}

impl SessionSpec {
    /// An active local session of `uid` of type `session_type`.
    pub fn local(uid: u32, session_type: &str) -> Self {
        Self {
            id: "2".into(),
            uid,
            session_type: session_type.into(),
            active: true,
            remote: false,
            display: if session_type == "x11" {
                ":1".into()
            } else {
                String::new()
            },
            desktop: "GNOME".into(),
            locked_hint: false,
        }
    }
}

/// One `Inhibit` call, with the read end of the pipe handed out.
pub struct InhibitCall {
    pub what: String,
    pub who: String,
    pub why: String,
    pub mode: String,
    pub at: Instant,
    read_end: OwnedFd,
}

impl InhibitCall {
    /// Every copy of the write end is closed.
    pub fn closed(&self) -> bool {
        self.wait_closed(Duration::ZERO)
    }

    /// Wait up to `timeout` for every copy of the write end to close.
    pub fn wait_closed(&self, timeout: Duration) -> bool {
        let mut pfd = libc::pollfd {
            fd: self.read_end.as_raw_fd(),
            events: libc::POLLIN,
            revents: 0,
        };
        let ms = timeout.as_millis().min(i32::MAX as u128) as i32;
        // SAFETY: one valid pollfd for the duration of the call.
        let n = unsafe { libc::poll(&mut pfd, 1, ms) };
        n > 0 && pfd.revents & libc::POLLHUP != 0
    }
}

#[derive(Default)]
struct State {
    session: Mutex<Option<SessionSpec>>,
    /// `GetUser(uid)` answers for this uid; `Display` is the session above.
    user_uid: Mutex<Option<u32>>,
    inhibits: Mutex<Vec<InhibitCall>>,
    get_session_calls: Mutex<Vec<String>>,
}

struct Manager {
    state: Arc<State>,
}

#[interface(name = "org.freedesktop.login1.Manager")]
impl Manager {
    fn get_session(&self, id: &str) -> zbus::fdo::Result<OwnedObjectPath> {
        self.state
            .get_session_calls
            .lock()
            .unwrap()
            .push(id.to_owned());
        let session = self.state.session.lock().unwrap();
        match session.as_ref() {
            Some(spec) if spec.id == id => Ok(OwnedObjectPath::try_from(SESSION_PATH).unwrap()),
            _ => Err(zbus::fdo::Error::Failed(format!("No session '{id}' known"))),
        }
    }

    fn get_user(&self, uid: u32) -> zbus::fdo::Result<OwnedObjectPath> {
        match *self.state.user_uid.lock().unwrap() {
            Some(known) if known == uid => Ok(OwnedObjectPath::try_from(format!(
                "/org/freedesktop/login1/user/_{uid}"
            ))
            .unwrap()),
            _ => Err(zbus::fdo::Error::Failed(format!(
                "User ID {uid} is not logged in"
            ))),
        }
    }

    fn inhibit(
        &self,
        what: &str,
        who: &str,
        why: &str,
        mode: &str,
    ) -> zbus::fdo::Result<zbus::zvariant::OwnedFd> {
        let mut fds = [0; 2];
        // SAFETY: pipe2 fills two descriptors on success.
        if unsafe { libc::pipe2(fds.as_mut_ptr(), libc::O_CLOEXEC) } != 0 {
            return Err(zbus::fdo::Error::Failed("pipe2 failed".into()));
        }
        // SAFETY: both descriptors were just created and are owned here.
        let (read_end, write_end) =
            unsafe { (OwnedFd::from_raw_fd(fds[0]), OwnedFd::from_raw_fd(fds[1])) };
        self.state.inhibits.lock().unwrap().push(InhibitCall {
            what: what.into(),
            who: who.into(),
            why: why.into(),
            mode: mode.into(),
            at: Instant::now(),
            read_end,
        });
        Ok(zbus::zvariant::OwnedFd::from(write_end))
    }
}

struct Session {
    state: Arc<State>,
}

impl Session {
    fn spec(&self) -> SessionSpec {
        self.state
            .session
            .lock()
            .unwrap()
            .clone()
            .expect("session served")
    }
}

#[interface(name = "org.freedesktop.login1.Session")]
impl Session {
    #[zbus(property)]
    fn id(&self) -> String {
        self.spec().id
    }

    #[zbus(property)]
    fn user(&self) -> (u32, OwnedObjectPath) {
        let uid = self.spec().uid;
        (
            uid,
            OwnedObjectPath::try_from(format!("/org/freedesktop/login1/user/_{uid}")).unwrap(),
        )
    }

    #[zbus(property, name = "Type")]
    fn session_type(&self) -> String {
        self.spec().session_type
    }

    #[zbus(property)]
    fn active(&self) -> bool {
        self.spec().active
    }

    #[zbus(property)]
    fn remote(&self) -> bool {
        self.spec().remote
    }

    #[zbus(property)]
    fn display(&self) -> String {
        self.spec().display
    }

    #[zbus(property)]
    fn desktop(&self) -> String {
        self.spec().desktop
    }

    #[zbus(property)]
    fn locked_hint(&self) -> bool {
        self.spec().locked_hint
    }
}

struct User {
    state: Arc<State>,
}

#[interface(name = "org.freedesktop.login1.User")]
impl User {
    #[zbus(property)]
    fn display(&self) -> (String, OwnedObjectPath) {
        match self.state.session.lock().unwrap().as_ref() {
            Some(spec) if spec.session_type == "x11" || spec.session_type == "wayland" => (
                spec.id.clone(),
                OwnedObjectPath::try_from(SESSION_PATH).unwrap(),
            ),
            _ => (String::new(), OwnedObjectPath::try_from("/").unwrap()),
        }
    }
}

/// A running fake owning `org.freedesktop.login1`; stops when dropped.
pub struct FakeLogind {
    conn: Connection,
    state: Arc<State>,
}

impl FakeLogind {
    /// Serve the Manager, `session` at [`SESSION_PATH`] and that session's
    /// user (so `GetUser(session.uid)` works).
    pub fn start(bus_address: &str, session: SessionSpec) -> Self {
        let state = Arc::new(State::default());
        let uid = session.uid;
        *state.user_uid.lock().unwrap() = Some(uid);
        *state.session.lock().unwrap() = Some(session);
        let conn = zbus::blocking::connection::Builder::address(bus_address)
            .unwrap()
            .name("org.freedesktop.login1")
            .unwrap()
            .serve_at(
                MANAGER_PATH,
                Manager {
                    state: state.clone(),
                },
            )
            .unwrap()
            .serve_at(
                SESSION_PATH,
                Session {
                    state: state.clone(),
                },
            )
            .unwrap()
            .serve_at(
                format!("/org/freedesktop/login1/user/_{uid}"),
                User {
                    state: state.clone(),
                },
            )
            .unwrap()
            .build()
            .expect("fake logind");
        Self { conn, state }
    }

    fn emit(
        &self,
        path: &str,
        interface: &str,
        member: &str,
        body: &(impl serde::Serialize + zbus::zvariant::DynamicType),
    ) {
        self.conn
            .emit_signal(None::<&str>, path, interface, member, body)
            .expect("emit signal");
    }

    /// `Session.Lock` on our session.
    pub fn lock(&self) {
        self.emit(SESSION_PATH, SESSION_INTERFACE, "Lock", &());
    }

    pub fn unlock(&self) {
        self.emit(SESSION_PATH, SESSION_INTERFACE, "Unlock", &());
    }

    /// `Lock` emitted on another session's object.
    pub fn lock_other_session(&self) {
        self.emit(
            "/org/freedesktop/login1/session/_99",
            SESSION_INTERFACE,
            "Lock",
            &(),
        );
    }

    fn properties_changed(&self, changes: &[(&str, bool)]) {
        let changed: HashMap<&str, Value<'_>> =
            changes.iter().map(|(k, v)| (*k, Value::from(*v))).collect();
        self.emit(
            SESSION_PATH,
            "org.freedesktop.DBus.Properties",
            "PropertiesChanged",
            &(SESSION_INTERFACE, changed, Vec::<&str>::new()),
        );
    }

    pub fn set_locked_hint(&self, locked: bool) {
        self.state
            .session
            .lock()
            .unwrap()
            .as_mut()
            .unwrap()
            .locked_hint = locked;
        self.properties_changed(&[("LockedHint", locked)]);
    }

    pub fn set_active(&self, active: bool) {
        self.state.session.lock().unwrap().as_mut().unwrap().active = active;
        self.properties_changed(&[("Active", active)]);
    }

    pub fn prepare_for_sleep(&self, start: bool) {
        self.emit(MANAGER_PATH, MANAGER_INTERFACE, "PrepareForSleep", &start);
    }

    pub fn prepare_for_shutdown(&self, start: bool) {
        self.emit(
            MANAGER_PATH,
            MANAGER_INTERFACE,
            "PrepareForShutdown",
            &start,
        );
    }

    /// `SessionRemoved` for our session (or another path).
    pub fn session_removed(&self, path: &str) {
        let path = OwnedObjectPath::try_from(path).unwrap();
        self.emit(
            MANAGER_PATH,
            MANAGER_INTERFACE,
            "SessionRemoved",
            &("2", path),
        );
    }

    /// How many `Inhibit` calls were made.
    pub fn inhibit_count(&self) -> usize {
        self.state.inhibits.lock().unwrap().len()
    }

    /// Run `f` on inhibit call `index`.
    pub fn with_inhibit<R>(&self, index: usize, f: impl FnOnce(&InhibitCall) -> R) -> R {
        f(&self.state.inhibits.lock().unwrap()[index])
    }

    /// Wait until `count` inhibit calls were made (or `timeout`).
    pub fn wait_inhibits(&self, count: usize, timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline {
            if self.inhibit_count() >= count {
                return true;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        self.inhibit_count() >= count
    }

    /// Session ids asked for with `GetSession`.
    pub fn get_session_calls(&self) -> Vec<String> {
        self.state.get_session_calls.lock().unwrap().clone()
    }
}
