//! X11 idle source: last physical XI2 input, else MIT-SCREEN-SAVER
//! `ms_since_user_input`.
//!
//! [`X11ScreenSaverIdleSource`] asks the X server (`ScreenSaverQueryInfo` on the
//! root window) how long ago the last user input was. It is the fallback
//! while the XI2 listener does not run (keyboard and mouse not consented):
//! idle and away detection are core time tracking and never uploaded. Note
//! the server counts XTEST input here too; the XI2 log's
//! [`super::input::InputLog::last_physical_input`] does not, and is preferred
//! while it is healthy.

use std::sync::Mutex;

use chrono::{DateTime, Utc};
use x11rb::connection::{Connection, RequestConnection};
use x11rb::protocol::screensaver::{self, ConnectionExt as _};
use x11rb::rust_connection::RustConnection;

use super::traits::{IdleSource, TrackingError};
use super::x11::connect_display;

/// MIT-SCREEN-SAVER idle time of one X display, connected lazily.
pub struct X11ScreenSaverIdleSource {
    display: String,
    connection: Mutex<Option<(RustConnection, u32)>>,
}

impl X11ScreenSaverIdleSource {
    /// A source for the explicit X11 `display`; nothing is opened yet.
    pub fn with_display(display: impl Into<String>) -> Self {
        Self {
            display: display.into(),
            connection: Mutex::new(None),
        }
    }

    /// Milliseconds since the last user input on the display.
    pub fn idle_millis(&self) -> Result<u32, TrackingError> {
        let mut guard = self.connection.lock().unwrap_or_else(|e| e.into_inner());
        if guard.is_none() {
            let (conn, screen) = connect_display(&self.display)?;
            let root = conn
                .setup()
                .roots
                .get(screen)
                .map(|s| s.root)
                .ok_or_else(|| TrackingError::DisplayUnavailable("no X11 screen".into()))?;
            let present = conn
                .extension_information(screensaver::X11_EXTENSION_NAME)
                .map_err(|e| TrackingError::ProtocolError(format!("QueryExtension: {e}")))?
                .is_some();
            if !present {
                return Err(TrackingError::UnsupportedSession(
                    "the X server has no MIT-SCREEN-SAVER extension".into(),
                ));
            }
            *guard = Some((conn, root));
        }
        let (conn, root) = guard.as_ref().expect("connected above");
        let reply = conn
            .screensaver_query_info(*root)
            .map_err(connection_lost)
            .and_then(|cookie| {
                cookie.reply().map_err(|e| match e {
                    x11rb::errors::ReplyError::ConnectionError(e) => connection_lost(e),
                    other => TrackingError::ProtocolError(format!("ScreenSaverQueryInfo: {other}")),
                })
            });
        match reply {
            Ok(reply) => Ok(reply.ms_since_user_input),
            Err(e) => {
                if matches!(e, TrackingError::DisplayUnavailable(_)) {
                    // Reconnect on the next call.
                    *guard = None;
                }
                Err(e)
            }
        }
    }
}

fn connection_lost(e: x11rb::errors::ConnectionError) -> TrackingError {
    TrackingError::DisplayUnavailable(format!("X11 connection lost: {e}"))
}

/// When the last input happened, given the idle time at `now`.
pub fn last_input_from_idle(now: DateTime<Utc>, idle_ms: u64) -> DateTime<Utc> {
    now - chrono::Duration::milliseconds(i64::try_from(idle_ms).unwrap_or(i64::MAX / 2))
}

impl IdleSource for X11ScreenSaverIdleSource {
    fn last_input(&self, now: DateTime<Utc>) -> Result<Option<DateTime<Utc>>, TrackingError> {
        Ok(Some(last_input_from_idle(
            now,
            u64::from(self.idle_millis()?),
        )))
    }

    fn kind(&self) -> &'static str {
        "x11-screensaver"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_last_input_is_now_minus_the_idle_time() {
        let now = DateTime::from_timestamp(1_790_000_000, 0).unwrap();
        assert_eq!(last_input_from_idle(now, 0), now);
        assert_eq!(
            last_input_from_idle(now, 61_500),
            now - chrono::Duration::milliseconds(61_500)
        );
    }

    #[test]
    fn construction_opens_nothing_and_a_missing_display_is_unavailable() {
        let source = X11ScreenSaverIdleSource::with_display("");
        assert_eq!(source.kind(), "x11-screensaver");
        assert!(matches!(
            source.last_input(Utc::now()),
            Err(TrackingError::DisplayUnavailable(_))
        ));
    }
}
