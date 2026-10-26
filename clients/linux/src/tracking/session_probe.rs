//! Session detection, logind first (design 4): X11 vs Wayland vs no session.
//!
//! 1. The environment is passed in explicitly ([`SessionEnv`], built by the
//!    binary's entry point from the unit environment or the session env
//!    file); nothing below `main` reads `DISPLAY` or `WAYLAND_DISPLAY`.
//! 2. logind on the system bus: `Manager.GetSession(XDG_SESSION_ID)`, or
//!    without one `GetUser(uid)` and that user's `Display` session. Only a
//!    session whose `User` is our euid, whose `Type` is `x11` or `wayland`,
//!    which is `Active` and not `Remote` is accepted.
//! 3. Without logind (non-systemd distributions): `XDG_SESSION_TYPE`, then
//!    `WAYLAND_DISPLAY`, then `DISPLAY`.
//! 4. On Wayland the result never carries an X11 display (XWayland only sees
//!    X11 clients: partial counts, wrong titles, black screenshots), so no X11
//!    adapter can be constructed from it.

use std::collections::BTreeMap;

use zbus::blocking::Connection;
use zbus::zvariant::{OwnedObjectPath, OwnedValue};

pub use crate::agent::state::SessionKind;

pub const LOGIND_BUS_NAME: &str = "org.freedesktop.login1";
pub const LOGIND_PATH: &str = "/org/freedesktop/login1";
pub const MANAGER_INTERFACE: &str = "org.freedesktop.login1.Manager";
pub const SESSION_INTERFACE: &str = "org.freedesktop.login1.Session";
pub const USER_INTERFACE: &str = "org.freedesktop.login1.User";

/// The default system bus when `DBUS_SYSTEM_BUS_ADDRESS` is unset.
pub const DEFAULT_SYSTEM_BUS: &str = "unix:path=/var/run/dbus/system_bus_socket";

/// The variables session detection looks at, passed explicitly.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SessionEnv {
    pub xdg_session_id: Option<String>,
    pub xdg_session_type: Option<String>,
    pub xdg_current_desktop: Option<String>,
    pub xdg_session_desktop: Option<String>,
    pub display: Option<String>,
    pub wayland_display: Option<String>,
    pub xauthority: Option<String>,
    pub dbus_session_bus_address: Option<String>,
    pub dbus_system_bus_address: Option<String>,
}

impl SessionEnv {
    /// Pick the session variables out of `vars` (for example
    /// `std::env::vars()` in the binary's entry point). Empty values count as
    /// unset.
    pub fn from_vars(vars: impl IntoIterator<Item = (String, String)>) -> Self {
        let vars: BTreeMap<String, String> = vars
            .into_iter()
            .filter(|(_, v)| !v.trim().is_empty())
            .collect();
        let get = |name: &str| vars.get(name).cloned();
        Self {
            xdg_session_id: get("XDG_SESSION_ID"),
            xdg_session_type: get("XDG_SESSION_TYPE"),
            xdg_current_desktop: get("XDG_CURRENT_DESKTOP"),
            xdg_session_desktop: get("XDG_SESSION_DESKTOP"),
            display: get("DISPLAY"),
            wayland_display: get("WAYLAND_DISPLAY"),
            xauthority: get("XAUTHORITY"),
            dbus_session_bus_address: get("DBUS_SESSION_BUS_ADDRESS"),
            dbus_system_bus_address: get("DBUS_SYSTEM_BUS_ADDRESS"),
        }
    }

    /// The system bus address: `DBUS_SYSTEM_BUS_ADDRESS` or the default.
    pub fn system_bus_address(&self) -> &str {
        self.dbus_system_bus_address
            .as_deref()
            .unwrap_or(DEFAULT_SYSTEM_BUS)
    }
}

/// Where the answer came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProbeSource {
    Logind,
    Environment,
}

/// The detected session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionInfo {
    pub kind: SessionKind,
    pub source: ProbeSource,
    /// `XDG_CURRENT_DESKTOP`, else logind's `Desktop`.
    pub desktop: Option<String>,
    /// The logind session object, e.g. `/org/freedesktop/login1/session/_32`.
    pub session_path: Option<String>,
    /// The logind session id (`XDG_SESSION_ID`), when known.
    pub session_id: Option<String>,
    /// The X11 display for the X11 adapters. Always `None` on Wayland.
    pub display: Option<String>,
    pub wayland_display: Option<String>,
    /// Only with an X11 display.
    pub xauthority: Option<String>,
    /// Why the session was not accepted (kind `None`/`Tty`), in words.
    pub reason: Option<String>,
}

impl SessionInfo {
    fn none(source: ProbeSource, reason: impl Into<String>) -> Self {
        Self {
            kind: SessionKind::None,
            source,
            desktop: None,
            session_path: None,
            session_id: None,
            display: None,
            wayland_display: None,
            xauthority: None,
            reason: Some(reason.into()),
        }
    }

    /// Fill the display fields from the environment for `kind`: Wayland
    /// never yields an X11 display.
    fn with_displays(mut self, env: &SessionEnv, logind_display: Option<String>) -> Self {
        match self.kind {
            SessionKind::X11 => {
                self.display = logind_display.or_else(|| env.display.clone());
                self.xauthority = env.xauthority.clone();
                if self.display.is_none() {
                    self.kind = SessionKind::None;
                    self.reason = Some("X11 session without a display".into());
                }
            }
            SessionKind::Wayland => {
                self.wayland_display = env.wayland_display.clone();
                self.display = None;
                self.xauthority = None;
            }
            SessionKind::Tty | SessionKind::None => {}
        }
        self
    }
}

/// The logind session's properties.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LogindSession {
    pub path: String,
    pub id: String,
    pub uid: u32,
    pub session_type: String,
    pub active: bool,
    pub remote: bool,
    pub display: String,
    pub desktop: String,
}

/// Why logind could not answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LogindError {
    /// No logind on this bus (not a systemd system): use the environment.
    Unavailable(String),
    /// logind answered, but has no session for us.
    NoSession(String),
}

fn logind_failure(e: zbus::Error) -> LogindError {
    let text = e.to_string();
    let unavailable = match &e {
        zbus::Error::MethodError(name, _, _) => matches!(
            name.as_str(),
            "org.freedesktop.DBus.Error.ServiceUnknown"
                | "org.freedesktop.DBus.Error.NameHasNoOwner"
                | "org.freedesktop.DBus.Error.UnknownObject"
                | "org.freedesktop.DBus.Error.UnknownMethod"
                | "org.freedesktop.DBus.Error.UnknownInterface"
        ),
        zbus::Error::FDO(fdo) => matches!(
            **fdo,
            zbus::fdo::Error::ServiceUnknown(_)
                | zbus::fdo::Error::NameHasNoOwner(_)
                | zbus::fdo::Error::UnknownObject(_)
                | zbus::fdo::Error::UnknownMethod(_)
                | zbus::fdo::Error::UnknownInterface(_)
        ),
        _ => true,
    };
    if unavailable {
        LogindError::Unavailable(text)
    } else {
        LogindError::NoSession(text)
    }
}

fn get_property(
    conn: &Connection,
    path: &str,
    interface: &str,
    name: &str,
) -> zbus::Result<OwnedValue> {
    let reply = conn.call_method(
        Some(LOGIND_BUS_NAME),
        path,
        Some("org.freedesktop.DBus.Properties"),
        "Get",
        &(interface, name),
    )?;
    let value: OwnedValue = reply.body().deserialize()?;
    Ok(value)
}

fn string_property(conn: &Connection, path: &str, name: &str) -> zbus::Result<String> {
    let value = get_property(conn, path, SESSION_INTERFACE, name)?;
    <String>::try_from(value).map_err(zbus::Error::from)
}

fn bool_property(conn: &Connection, path: &str, name: &str) -> zbus::Result<bool> {
    let value = get_property(conn, path, SESSION_INTERFACE, name)?;
    bool::try_from(value).map_err(zbus::Error::from)
}

/// Read the session object at `path`.
pub fn read_session(conn: &Connection, path: &str) -> Result<LogindSession, LogindError> {
    let user = get_property(conn, path, SESSION_INTERFACE, "User").map_err(logind_failure)?;
    let (uid, _user_path): (u32, OwnedObjectPath) = user
        .try_into()
        .map_err(|e: zbus::zvariant::Error| logind_failure(e.into()))?;
    Ok(LogindSession {
        path: path.to_owned(),
        id: string_property(conn, path, "Id").map_err(logind_failure)?,
        uid,
        session_type: string_property(conn, path, "Type").map_err(logind_failure)?,
        active: bool_property(conn, path, "Active").map_err(logind_failure)?,
        remote: bool_property(conn, path, "Remote").map_err(logind_failure)?,
        display: string_property(conn, path, "Display").unwrap_or_default(),
        desktop: string_property(conn, path, "Desktop").unwrap_or_default(),
    })
}

/// The session object for `XDG_SESSION_ID`, or the user's display session.
pub fn find_session_path(
    conn: &Connection,
    session_id: Option<&str>,
    uid: u32,
) -> Result<String, LogindError> {
    if let Some(id) = session_id {
        let reply = conn
            .call_method(
                Some(LOGIND_BUS_NAME),
                LOGIND_PATH,
                Some(MANAGER_INTERFACE),
                "GetSession",
                &(id,),
            )
            .map_err(logind_failure)?;
        let path: OwnedObjectPath = reply.body().deserialize().map_err(logind_failure)?;
        return Ok(path.as_str().to_owned());
    }
    let reply = conn
        .call_method(
            Some(LOGIND_BUS_NAME),
            LOGIND_PATH,
            Some(MANAGER_INTERFACE),
            "GetUser",
            &(uid,),
        )
        .map_err(logind_failure)?;
    let user_path: OwnedObjectPath = reply.body().deserialize().map_err(logind_failure)?;
    let display = get_property(conn, user_path.as_str(), USER_INTERFACE, "Display")
        .map_err(logind_failure)?;
    let (id, path): (String, OwnedObjectPath) = display
        .try_into()
        .map_err(|e: zbus::zvariant::Error| logind_failure(e.into()))?;
    if id.is_empty() || path.as_str() == "/" {
        return Err(LogindError::NoSession(
            "the user has no graphical session".into(),
        ));
    }
    Ok(path.as_str().to_owned())
}

/// Decide the session (design 4). `system_bus` is the connection to logind's
/// bus, `None` when there is none; `euid` is the agent's effective uid.
pub fn probe(env: &SessionEnv, system_bus: Option<&Connection>, euid: u32) -> SessionInfo {
    let logind = system_bus.map(|conn| {
        find_session_path(conn, env.xdg_session_id.as_deref(), euid)
            .and_then(|path| read_session(conn, &path))
    });
    match logind {
        Some(Ok(session)) => from_logind(env, session, euid),
        Some(Err(LogindError::NoSession(why))) => SessionInfo::none(ProbeSource::Logind, why),
        Some(Err(LogindError::Unavailable(why))) => {
            tracing::debug!(%why, "logind unavailable; detecting the session from the environment");
            from_environment(env)
        }
        None => from_environment(env),
    }
}

/// Validate a logind session (uid, Type, Active, Remote).
pub fn from_logind(env: &SessionEnv, session: LogindSession, euid: u32) -> SessionInfo {
    let mut info = SessionInfo {
        kind: SessionKind::None,
        source: ProbeSource::Logind,
        desktop: env
            .xdg_current_desktop
            .clone()
            .or_else(|| (!session.desktop.is_empty()).then(|| session.desktop.clone())),
        session_path: Some(session.path.clone()),
        session_id: (!session.id.is_empty()).then(|| session.id.clone()),
        display: None,
        wayland_display: None,
        xauthority: None,
        reason: None,
    };
    if session.uid != euid {
        info.reason = Some(format!(
            "the session belongs to another user (uid {})",
            session.uid
        ));
        info.session_path = None;
        info.desktop = None;
        return info;
    }
    if session.remote {
        info.kind = SessionKind::Tty;
        info.reason = Some("remote sessions are not tracked".into());
        return info;
    }
    let kind = match session.session_type.as_str() {
        "x11" => SessionKind::X11,
        "wayland" => SessionKind::Wayland,
        "tty" | "unspecified" | "mir" | "web" => {
            info.kind = SessionKind::Tty;
            info.reason = Some(format!(
                "not a graphical session (type {})",
                session.session_type
            ));
            return info;
        }
        other => {
            info.reason = Some(format!("unknown session type {other}"));
            return info;
        }
    };
    if !session.active {
        info.reason = Some("the session is not active".into());
        return info;
    }
    info.kind = kind;
    let logind_display = (!session.display.is_empty()).then(|| session.display.clone());
    info.with_displays(env, logind_display)
}

/// Without logind: `XDG_SESSION_TYPE`, then `WAYLAND_DISPLAY`, then `DISPLAY`.
pub fn from_environment(env: &SessionEnv) -> SessionInfo {
    let declared = env
        .xdg_session_type
        .as_deref()
        .map(|t| t.trim().to_ascii_lowercase());
    let kind = match declared.as_deref() {
        Some("x11") => SessionKind::X11,
        Some("wayland") => SessionKind::Wayland,
        Some("tty") => SessionKind::Tty,
        _ if env.wayland_display.is_some() => SessionKind::Wayland,
        _ if env.display.is_some() => SessionKind::X11,
        _ => SessionKind::None,
    };
    let reason = match kind {
        SessionKind::Tty => Some("not a graphical session (type tty)".to_owned()),
        SessionKind::None => Some("no graphical session in the environment".to_owned()),
        _ => None,
    };
    let info = SessionInfo {
        kind,
        source: ProbeSource::Environment,
        desktop: env.xdg_current_desktop.clone(),
        session_path: None,
        session_id: env.xdg_session_id.clone(),
        display: None,
        wayland_display: None,
        xauthority: None,
        reason,
    };
    info.with_displays(env, None)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env(pairs: &[(&str, &str)]) -> SessionEnv {
        SessionEnv::from_vars(pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())))
    }

    fn session(kind: &str) -> LogindSession {
        LogindSession {
            path: "/org/freedesktop/login1/session/_32".into(),
            id: "2".into(),
            uid: 1000,
            session_type: kind.into(),
            active: true,
            remote: false,
            display: ":1".into(),
            desktop: "GNOME".into(),
        }
    }

    #[test]
    fn the_environment_decides_without_logind() {
        let x11 = from_environment(&env(&[
            ("DISPLAY", ":0"),
            ("XAUTHORITY", "/run/user/1000/xauth"),
        ]));
        assert_eq!(x11.kind, SessionKind::X11);
        assert_eq!(x11.display.as_deref(), Some(":0"));
        assert_eq!(x11.xauthority.as_deref(), Some("/run/user/1000/xauth"));
        assert_eq!(x11.source, ProbeSource::Environment);

        // WAYLAND_DISPLAY wins over DISPLAY (XWayland), and no X11 display is kept.
        let wayland =
            from_environment(&env(&[("DISPLAY", ":0"), ("WAYLAND_DISPLAY", "wayland-0")]));
        assert_eq!(wayland.kind, SessionKind::Wayland);
        assert_eq!(wayland.display, None);
        assert_eq!(wayland.xauthority, None);
        assert_eq!(wayland.wayland_display.as_deref(), Some("wayland-0"));

        // XDG_SESSION_TYPE is believed first.
        let declared =
            from_environment(&env(&[("XDG_SESSION_TYPE", "wayland"), ("DISPLAY", ":0")]));
        assert_eq!(declared.kind, SessionKind::Wayland);
        assert_eq!(declared.display, None);
        let tty = from_environment(&env(&[("XDG_SESSION_TYPE", "tty"), ("DISPLAY", ":0")]));
        assert_eq!(tty.kind, SessionKind::Tty);
        assert_eq!(from_environment(&env(&[])).kind, SessionKind::None);
        // An X11 type without a display is no usable session.
        assert_eq!(
            from_environment(&env(&[("XDG_SESSION_TYPE", "x11")])).kind,
            SessionKind::None
        );
        // Empty values count as unset.
        assert_eq!(
            from_environment(&env(&[("DISPLAY", " ")])).kind,
            SessionKind::None
        );
    }

    #[test]
    fn a_logind_session_is_validated() {
        let e = env(&[
            ("DISPLAY", ":9"),
            ("WAYLAND_DISPLAY", "wayland-1"),
            ("XDG_CURRENT_DESKTOP", "ubuntu:GNOME"),
        ]);
        let x11 = from_logind(&e, session("x11"), 1000);
        assert_eq!(x11.kind, SessionKind::X11);
        assert_eq!(x11.display.as_deref(), Some(":1"), "logind's display wins");
        assert_eq!(x11.desktop.as_deref(), Some("ubuntu:GNOME"));
        assert_eq!(
            x11.session_path.as_deref(),
            Some("/org/freedesktop/login1/session/_32")
        );

        let wayland = from_logind(&e, session("wayland"), 1000);
        assert_eq!(wayland.kind, SessionKind::Wayland);
        assert_eq!(wayland.display, None, "never an X11 display on Wayland");

        let other = from_logind(&e, session("x11"), 1001);
        assert_eq!(other.kind, SessionKind::None);
        assert!(other.reason.unwrap().contains("another user"));
        assert_eq!(other.session_path, None);

        let inactive = from_logind(
            &e,
            LogindSession {
                active: false,
                ..session("x11")
            },
            1000,
        );
        assert_eq!(inactive.kind, SessionKind::None);
        let remote = from_logind(
            &e,
            LogindSession {
                remote: true,
                ..session("x11")
            },
            1000,
        );
        assert_eq!(remote.kind, SessionKind::Tty);
        assert_eq!(from_logind(&e, session("tty"), 1000).kind, SessionKind::Tty);
        let no_desktop_env = from_logind(&env(&[]), session("x11"), 1000);
        assert_eq!(
            no_desktop_env.desktop.as_deref(),
            Some("GNOME"),
            "logind's Desktop as a fallback"
        );
    }
}
