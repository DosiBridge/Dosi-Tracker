//! Wayland idle sources over D-Bus: Mutter IdleMonitor, ScreenSaver
//! `GetSessionIdleTime`, logind `IdleHint`; none means manual mode.
//!
//! - [`MutterIdleSource`] asks `org.gnome.Mutter.IdleMonitor` at
//!   `/org/gnome/Mutter/IdleMonitor/Core`, `GetIdletime` in milliseconds (GNOME).
//! - [`ScreenSaverIdleSource`] asks `org.freedesktop.ScreenSaver`
//!   `GetSessionIdleTime` in seconds (KDE and others; GNOME answers
//!   "not supported").
//! - [`LogindIdleSource`] reads the session object's `IdleHint` / `IdleSinceHint`
//!   on the system bus — coarse (set by the desktop after its own idle delay),
//!   the last resort.
//!
//! [`probe_idle_sources`] returns the first source that answers, preferring
//! Mutter on GNOME. All buses are handed in explicitly; nothing here reads
//! `DBUS_SESSION_BUS_ADDRESS`. Idle readings stay local (block cuts and the
//! away prompt) and are never uploaded.

use std::time::Duration;

use chrono::{DateTime, Utc};
use zbus::blocking::Connection;
use zbus::zvariant::OwnedValue;

use super::traits::{IdleSource, TrackingError};
use super::x11_idle::last_input_from_idle;

pub const MUTTER_IDLE_BUS_NAME: &str = "org.gnome.Mutter.IdleMonitor";
pub const MUTTER_IDLE_PATH: &str = "/org/gnome/Mutter/IdleMonitor/Core";
pub const MUTTER_IDLE_INTERFACE: &str = "org.gnome.Mutter.IdleMonitor";
pub const SCREENSAVER_BUS_NAME: &str = "org.freedesktop.ScreenSaver";
pub const SCREENSAVER_PATH: &str = "/org/freedesktop/ScreenSaver";
pub const SCREENSAVER_INTERFACE: &str = "org.freedesktop.ScreenSaver";
pub const LOGIND_BUS_NAME: &str = "org.freedesktop.login1";
pub const LOGIND_SESSION_INTERFACE: &str = "org.freedesktop.login1.Session";

/// How long one idle query may take before the source counts as failing.
pub const METHOD_TIMEOUT: Duration = Duration::from_secs(2);

/// Connect to the bus at `address` with [`METHOD_TIMEOUT`] on every call.
pub fn connect_bus(address: &str) -> Result<Connection, TrackingError> {
    zbus::blocking::connection::Builder::address(address)
        .map_err(|e| TrackingError::IoError(format!("D-Bus address {address}: {e}")))?
        .method_timeout(METHOD_TIMEOUT)
        .build()
        .map_err(|e| TrackingError::IoError(format!("D-Bus connection to {address}: {e}")))
}

fn call_failed(source: &str, e: zbus::Error) -> TrackingError {
    TrackingError::IoError(format!("{source} idle query failed: {e}"))
}

/// GNOME's idle monitor.
pub struct MutterIdleSource {
    conn: Connection,
}

impl MutterIdleSource {
    pub fn new(session_bus: Connection) -> Self {
        Self { conn: session_bus }
    }

    /// Milliseconds since the last user input.
    pub fn idle_millis(&self) -> Result<u64, TrackingError> {
        self.conn
            .call_method(
                Some(MUTTER_IDLE_BUS_NAME),
                MUTTER_IDLE_PATH,
                Some(MUTTER_IDLE_INTERFACE),
                "GetIdletime",
                &(),
            )
            .and_then(|reply| reply.body().deserialize::<u64>())
            .map_err(|e| call_failed("Mutter", e))
    }
}

impl IdleSource for MutterIdleSource {
    fn last_input(&self, now: DateTime<Utc>) -> Result<Option<DateTime<Utc>>, TrackingError> {
        Ok(Some(last_input_from_idle(now, self.idle_millis()?)))
    }

    fn kind(&self) -> &'static str {
        "mutter"
    }
}

/// `org.freedesktop.ScreenSaver` session idle time.
pub struct ScreenSaverIdleSource {
    conn: Connection,
}

impl ScreenSaverIdleSource {
    pub fn new(session_bus: Connection) -> Self {
        Self { conn: session_bus }
    }

    /// Seconds since the last user input.
    pub fn idle_seconds(&self) -> Result<u32, TrackingError> {
        self.conn
            .call_method(
                Some(SCREENSAVER_BUS_NAME),
                SCREENSAVER_PATH,
                Some(SCREENSAVER_INTERFACE),
                "GetSessionIdleTime",
                &(),
            )
            .and_then(|reply| reply.body().deserialize::<u32>())
            .map_err(|e| call_failed("ScreenSaver", e))
    }
}

impl IdleSource for ScreenSaverIdleSource {
    fn last_input(&self, now: DateTime<Utc>) -> Result<Option<DateTime<Utc>>, TrackingError> {
        let seconds = self.idle_seconds()?;
        Ok(Some(last_input_from_idle(now, u64::from(seconds) * 1000)))
    }

    fn kind(&self) -> &'static str {
        "screensaver"
    }
}

/// logind's idle hint of one session (system bus).
pub struct LogindIdleSource {
    conn: Connection,
    session_path: String,
}

impl LogindIdleSource {
    /// `session_path` is the session's object path, e.g.
    /// `/org/freedesktop/login1/session/_32` (from the session probe).
    pub fn new(system_bus: Connection, session_path: impl Into<String>) -> Self {
        Self {
            conn: system_bus,
            session_path: session_path.into(),
        }
    }

    fn property(&self, name: &str) -> Result<OwnedValue, TrackingError> {
        self.conn
            .call_method(
                Some(LOGIND_BUS_NAME),
                self.session_path.as_str(),
                Some("org.freedesktop.DBus.Properties"),
                "Get",
                &(LOGIND_SESSION_INTERFACE, name),
            )
            .and_then(|reply| reply.body().deserialize::<OwnedValue>())
            .map_err(|e| call_failed("logind", e))
    }

    /// (`IdleHint`, `IdleSinceHint` in µs since the epoch).
    pub fn idle_hint(&self) -> Result<(bool, u64), TrackingError> {
        let decode = |e: zbus::zvariant::Error| {
            TrackingError::ProtocolError(format!("logind idle property: {e}"))
        };
        let idle = bool::try_from(self.property("IdleHint")?).map_err(decode)?;
        let since = u64::try_from(self.property("IdleSinceHint")?).map_err(decode)?;
        Ok((idle, since))
    }
}

/// The last input implied by logind's hint at `now`: now while not idle;
/// the idle start while idle; unknown when idle without a start.
pub fn last_input_from_hint(
    now: DateTime<Utc>,
    idle: bool,
    since_micros: u64,
) -> Option<DateTime<Utc>> {
    if !idle {
        return Some(now);
    }
    if since_micros == 0 {
        return None;
    }
    let micros = i64::try_from(since_micros).ok()?;
    DateTime::from_timestamp_micros(micros).map(|since| since.min(now))
}

impl IdleSource for LogindIdleSource {
    fn last_input(&self, now: DateTime<Utc>) -> Result<Option<DateTime<Utc>>, TrackingError> {
        let (idle, since) = self.idle_hint()?;
        Ok(last_input_from_hint(now, idle, since))
    }

    fn kind(&self) -> &'static str {
        "logind"
    }
}

/// The buses (and logind session) idle sources may use; all optional.
#[derive(Default, Clone)]
pub struct IdleBuses {
    pub session: Option<Connection>,
    pub system: Option<Connection>,
    /// The session's logind object path.
    pub logind_session: Option<String>,
}

/// Is `desktop` (an `XDG_CURRENT_DESKTOP` value such as `ubuntu:GNOME`) GNOME?
pub fn is_gnome(desktop: Option<&str>) -> bool {
    desktop.is_some_and(|d| d.split(':').any(|part| part.eq_ignore_ascii_case("GNOME")))
}

/// The first idle source that answers on this desktop, or `None` (manual
/// mode). GNOME prefers Mutter; others try `org.freedesktop.ScreenSaver`
/// first; logind's hint is the last resort.
pub fn probe_idle_sources(desktop: Option<&str>, buses: &IdleBuses) -> Option<Box<dyn IdleSource>> {
    let now = Utc::now();
    let mut candidates: Vec<Box<dyn IdleSource>> = Vec::new();
    if let Some(session) = &buses.session {
        let mutter: Box<dyn IdleSource> = Box::new(MutterIdleSource::new(session.clone()));
        let screensaver: Box<dyn IdleSource> =
            Box::new(ScreenSaverIdleSource::new(session.clone()));
        if is_gnome(desktop) {
            candidates.extend([mutter, screensaver]);
        } else {
            candidates.extend([screensaver, mutter]);
        }
    }
    if let (Some(system), Some(path)) = (&buses.system, &buses.logind_session) {
        candidates.push(Box::new(LogindIdleSource::new(
            system.clone(),
            path.clone(),
        )));
    }
    candidates
        .into_iter()
        .find(|source| match source.last_input(now) {
            Ok(_) => true,
            Err(e) => {
                tracing::debug!(source = source.kind(), error = %e, "idle source unavailable");
                false
            }
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gnome_is_recognised_in_composite_desktop_names() {
        assert!(is_gnome(Some("GNOME")));
        assert!(is_gnome(Some("ubuntu:GNOME")));
        assert!(is_gnome(Some("gnome-classic:gnome")));
        assert!(!is_gnome(Some("KDE")));
        assert!(!is_gnome(Some("X-Cinnamon")));
        assert!(!is_gnome(None));
    }

    #[test]
    fn the_logind_hint_maps_to_a_last_input() {
        let now = DateTime::from_timestamp(1_790_000_000, 0).unwrap();
        assert_eq!(last_input_from_hint(now, false, 0), Some(now));
        assert_eq!(last_input_from_hint(now, true, 0), None);
        let since = 1_789_999_000_000_000u64;
        assert_eq!(
            last_input_from_hint(now, true, since),
            DateTime::from_timestamp(1_789_999_000, 0)
        );
        // A hint in the future (clock skew) is clamped to now.
        assert_eq!(
            last_input_from_hint(now, true, 1_790_000_100_000_000),
            Some(now)
        );
    }

    #[test]
    fn without_buses_there_is_no_idle_source() {
        assert!(probe_idle_sources(Some("GNOME"), &IdleBuses::default()).is_none());
        assert!(connect_bus("not an address").is_err());
    }
}
