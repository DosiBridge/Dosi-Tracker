//! The `dosi-xi2` listener: XInput 2.2 raw key/button events counted per
//! second; XTEST-originated events counted as injected only.
//!
//! The listener opens its own pure-Rust X11 connection to an explicit display
//! string and selects, on the root window for `XIAllMasterDevices`, only the
//! raw events of the input kinds the effective policy allows
//! (`XI_RawKeyPress`, `XI_RawButtonPress`, `XI_RawMotion`), plus
//! `XI_HierarchyChanged` for `XIAllDevices` to keep the set of XTEST slave
//! devices current.
//!
//! Privacy (design I3): an event is reduced to its kind (key press, click,
//! pointer activity) and the device it came from before anything is
//! recorded. The key path never reads the event's `detail` (the keycode);
//! only the button path reads it, to tell clicks (buttons 1-3, 8-9) from the
//! wheel (4-7). Events flagged `XIKeyRepeat` are dropped. Events from the
//! `Virtual core XTEST keyboard/pointer` slaves (xdotool, remote-desktop
//! tools, jigglers) count as injected only. This module never injects input
//! and never grabs devices.

use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};
use std::thread::JoinHandle;
use std::time::Duration;

use chrono::Utc;
use x11rb::connection::{Connection, RequestConnection};
use x11rb::protocol::xinput::{self, ConnectionExt as _, XIEventMask};
use x11rb::protocol::xproto::{
    self, ClientMessageEvent, ConnectionExt as _, CreateWindowAux, EventMask, WindowClass,
};
use x11rb::rust_connection::RustConnection;

use super::input::InputLog;
use super::traits::TrackingError;
use super::x11::connect_display;

/// How long [`X11InputListener::stop`] waits for the listener thread.
pub const STOP_TIMEOUT: Duration = Duration::from_secs(1);

/// Names the X server gives the XTEST slave devices.
pub const XTEST_DEVICE_NAMES: [&str; 2] =
    ["Virtual core XTEST keyboard", "Virtual core XTEST pointer"];

/// X11 `GenericEvent` response type (XI2 events arrive as generic events).
const GENERIC_EVENT: u8 = 35;

/// `XIKeyRepeat` in the raw event's flags.
const KEY_REPEAT_FLAG: u32 = 1 << 16;

/// Byte offsets inside an XI2 raw event (`xXIRawEvent`).
const OFFSET_EVTYPE: usize = 8;
const OFFSET_DETAIL: usize = 16;
const OFFSET_SOURCEID: usize = 20;
const OFFSET_FLAGS: usize = 24;
const RAW_EVENT_HEADER: usize = 32;

/// Which input kinds the listener may observe (the effective policy).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct InputKinds {
    pub keyboard: bool,
    pub mouse: bool,
}

impl InputKinds {
    pub fn any(self) -> bool {
        self.keyboard || self.mouse
    }
}

/// An XI2 event reduced to what is counted. Carries no keycode, no button
/// number, no coordinates and no timestamp.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Xi2Event {
    /// A key press (not auto-repeat) from device `source`.
    Key { source: u16 },
    /// A click (buttons 1-3, 8-9) from `source`.
    Click { source: u16 },
    /// Pointer motion or the wheel from `source`: activity, not a click.
    PointerActivity { source: u16 },
    /// Devices were added or removed: refresh the XTEST slave set.
    HierarchyChanged,
}

fn read_u16(bytes: &[u8], at: usize) -> Option<u16> {
    bytes
        .get(at..at + 2)
        .map(|b| u16::from_ne_bytes([b[0], b[1]]))
}

fn read_u32(bytes: &[u8], at: usize) -> Option<u32> {
    bytes
        .get(at..at + 4)
        .map(|b| u32::from_ne_bytes([b[0], b[1], b[2], b[3]]))
}

/// Is `button` (an XI2 raw button number) a click? Buttons 4-7 are the
/// vertical and horizontal wheel.
pub fn is_click_button(button: u32) -> bool {
    matches!(button, 1..=3 | 8 | 9)
}

/// Decode one raw event from the X connection (in the connection's byte
/// order). `None` for anything that is not a counted XI2 event of the
/// XInput extension with major opcode `xinput_opcode`, and for key repeats.
pub fn decode_xi2_event(raw: &[u8], xinput_opcode: u8) -> Option<Xi2Event> {
    if raw.len() < RAW_EVENT_HEADER || raw[0] & 0x7f != GENERIC_EVENT || raw[1] != xinput_opcode {
        return None;
    }
    let evtype = read_u16(raw, OFFSET_EVTYPE)?;
    match evtype {
        xinput::RAW_KEY_PRESS_EVENT => {
            // Only the device and the flags: the keycode is never read.
            let source = read_u16(raw, OFFSET_SOURCEID)?;
            let flags = read_u32(raw, OFFSET_FLAGS)?;
            (flags & KEY_REPEAT_FLAG == 0).then_some(Xi2Event::Key { source })
        }
        xinput::RAW_BUTTON_PRESS_EVENT => {
            let source = read_u16(raw, OFFSET_SOURCEID)?;
            let button = read_u32(raw, OFFSET_DETAIL)?;
            Some(if is_click_button(button) {
                Xi2Event::Click { source }
            } else {
                Xi2Event::PointerActivity { source }
            })
        }
        xinput::RAW_MOTION_EVENT => Some(Xi2Event::PointerActivity {
            source: read_u16(raw, OFFSET_SOURCEID)?,
        }),
        xinput::HIERARCHY_EVENT => Some(Xi2Event::HierarchyChanged),
        _ => None,
    }
}

/// File one decoded event into `log` at Unix `second`. Events from an XTEST
/// slave count as injected only; kinds outside `kinds` are dropped.
pub fn apply_event(
    log: &InputLog,
    event: Xi2Event,
    xtest_devices: &HashSet<u16>,
    kinds: InputKinds,
    second: i64,
) {
    match event {
        Xi2Event::Key { source } if kinds.keyboard => {
            log.record_key(second, xtest_devices.contains(&source))
        }
        Xi2Event::Click { source } if kinds.mouse => {
            log.record_click(second, xtest_devices.contains(&source))
        }
        Xi2Event::PointerActivity { source } if kinds.mouse => {
            if xtest_devices.contains(&source) {
                log.record_injected(second);
            } else {
                log.record_activity(second);
            }
        }
        _ => {}
    }
}

/// The XTEST slave device ids among `(id, name)` pairs.
pub fn xtest_device_ids<'a>(devices: impl IntoIterator<Item = (u16, &'a [u8])>) -> HashSet<u16> {
    devices
        .into_iter()
        .filter(|(_, name)| {
            XTEST_DEVICE_NAMES
                .iter()
                .any(|xtest| name.trim_ascii_end() == xtest.as_bytes())
        })
        .map(|(id, _)| id)
        .collect()
}

/// The raw-event masks for `kinds`: raw events on master devices only, the
/// hierarchy on all devices.
pub fn event_masks(kinds: InputKinds) -> Vec<xinput::EventMask> {
    let mut raw = XIEventMask::from(0u32);
    if kinds.keyboard {
        raw |= XIEventMask::RAW_KEY_PRESS;
    }
    if kinds.mouse {
        raw |= XIEventMask::RAW_BUTTON_PRESS | XIEventMask::RAW_MOTION;
    }
    vec![
        xinput::EventMask {
            deviceid: u16::from(xinput::Device::ALL_MASTER),
            mask: vec![raw],
        },
        xinput::EventMask {
            deviceid: u16::from(xinput::Device::ALL),
            mask: vec![XIEventMask::HIERARCHY],
        },
    ]
}

/// Masks that select nothing (used to stop delivery before the thread ends).
fn cleared_masks() -> Vec<xinput::EventMask> {
    [xinput::Device::ALL_MASTER, xinput::Device::ALL]
        .into_iter()
        .map(|device| xinput::EventMask {
            deviceid: u16::from(device),
            mask: vec![XIEventMask::from(0u32)],
        })
        .collect()
}

fn protocol(context: &str, e: impl std::fmt::Display) -> TrackingError {
    TrackingError::ProtocolError(format!("{context}: {e}"))
}

fn query_xtest_devices(conn: &RustConnection) -> Result<HashSet<u16>, TrackingError> {
    let reply = conn
        .xinput_xi_query_device(u16::from(xinput::Device::ALL))
        .map_err(|e| protocol("XIQueryDevice", e))?
        .reply()
        .map_err(|e| protocol("XIQueryDevice", e))?;
    Ok(xtest_device_ids(
        reply
            .infos
            .iter()
            .map(|info| (info.deviceid, info.name.as_slice())),
    ))
}

/// A running XI2 raw-event listener (thread `dosi-xi2`).
pub struct X11InputListener {
    conn: Arc<RustConnection>,
    root: xproto::Window,
    wake_window: xproto::Window,
    stopping: Arc<AtomicBool>,
    done: mpsc::Receiver<()>,
    thread: Option<JoinHandle<()>>,
    log: Arc<InputLog>,
}

impl X11InputListener {
    /// Connect to `display`, select the raw events for `kinds` and start
    /// counting into `log`. Fails with `UnsupportedSession` when the server
    /// lacks XInput 2.2.
    pub fn start(
        display: &str,
        log: Arc<InputLog>,
        kinds: InputKinds,
    ) -> Result<Self, TrackingError> {
        if !kinds.any() {
            return Err(TrackingError::CapabilityDenied(
                crate::lifecycle::AcquisitionCapability::KeyboardCount,
            ));
        }
        let (conn, screen) = connect_display(display)?;
        let root = conn
            .setup()
            .roots
            .get(screen)
            .map(|s| s.root)
            .ok_or_else(|| TrackingError::DisplayUnavailable("no X11 screen".into()))?;
        let xinput_opcode = conn
            .extension_information(xinput::X11_EXTENSION_NAME)
            .map_err(|e| protocol("QueryExtension XInputExtension", e))?
            .map(|info| info.major_opcode)
            .ok_or_else(|| {
                TrackingError::UnsupportedSession("the X server has no XInput extension".into())
            })?;
        let version = conn
            .xinput_xi_query_version(2, 2)
            .map_err(|e| protocol("XIQueryVersion", e))?
            .reply()
            .map_err(|e| protocol("XIQueryVersion", e))?;
        if (version.major_version, version.minor_version) < (2, 2) {
            return Err(TrackingError::UnsupportedSession(format!(
                "the X server speaks XInput {}.{}, 2.2 is required",
                version.major_version, version.minor_version
            )));
        }
        let xtest = query_xtest_devices(&conn)?;

        // An unmapped input-only window of our own: stop() sends it a client
        // message to wake the listener thread out of its blocking read.
        let wake_window = conn.generate_id().map_err(|e| protocol("generate_id", e))?;
        conn.create_window(
            0,
            wake_window,
            root,
            -1,
            -1,
            1,
            1,
            0,
            WindowClass::INPUT_ONLY,
            x11rb::COPY_FROM_PARENT,
            &CreateWindowAux::new(),
        )
        .map_err(|e| protocol("CreateWindow", e))?
        .check()
        .map_err(|e| protocol("CreateWindow", e))?;

        conn.xinput_xi_select_events(root, &event_masks(kinds))
            .map_err(|e| protocol("XISelectEvents", e))?
            .check()
            .map_err(|e| protocol("XISelectEvents", e))?;

        let conn = Arc::new(conn);
        let stopping = Arc::new(AtomicBool::new(false));
        let (done_tx, done) = mpsc::channel();
        log.mark_healthy(Utc::now());

        let thread = {
            let conn = conn.clone();
            let stopping = stopping.clone();
            let log = log.clone();
            std::thread::Builder::new()
                .name("dosi-xi2".into())
                .spawn(move || {
                    listen(
                        &conn,
                        xinput_opcode,
                        wake_window,
                        xtest,
                        kinds,
                        &stopping,
                        &log,
                    );
                    let _ = done_tx.send(());
                })
                .map_err(|e| {
                    TrackingError::IoError(format!("could not start the XI2 listener: {e}"))
                })?
        };

        Ok(Self {
            conn,
            root,
            wake_window,
            stopping,
            done,
            thread: Some(thread),
            log,
        })
    }

    /// Is the listener thread still delivering?
    pub fn is_running(&self) -> bool {
        self.thread.as_ref().is_some_and(|t| !t.is_finished())
    }

    /// Clear the event mask, wake the thread and join it (at most
    /// [`STOP_TIMEOUT`]). Input is "not measured" from now on.
    pub fn stop(mut self) -> Result<(), TrackingError> {
        self.shutdown()
    }

    fn shutdown(&mut self) -> Result<(), TrackingError> {
        let Some(thread) = self.thread.take() else {
            return Ok(());
        };
        self.stopping.store(true, Ordering::SeqCst);
        // Best effort: the connection may already be gone.
        let _ = self
            .conn
            .xinput_xi_select_events(self.root, &cleared_masks());
        let wake = ClientMessageEvent::new(32, self.wake_window, xproto::AtomEnum::NONE, [0u32; 5]);
        let _ = self
            .conn
            .send_event(false, self.wake_window, EventMask::NO_EVENT, wake);
        let _ = self.conn.flush();
        self.log.mark_unhealthy(Utc::now());

        let joined = match self.done.recv_timeout(STOP_TIMEOUT) {
            Ok(()) | Err(mpsc::RecvTimeoutError::Disconnected) => true,
            Err(mpsc::RecvTimeoutError::Timeout) => {
                // The server stopped answering: cut the socket so the blocked
                // read returns.
                force_disconnect(&self.conn);
                self.done.recv_timeout(Duration::from_millis(200)).is_ok()
            }
        };
        if joined {
            let _ = thread.join();
            let _ = self.conn.destroy_window(self.wake_window);
            let _ = self.conn.flush();
            Ok(())
        } else {
            Err(TrackingError::IoError(
                "the XI2 listener did not stop within 1 s".into(),
            ))
        }
    }
}

impl Drop for X11InputListener {
    fn drop(&mut self) {
        let _ = self.shutdown();
    }
}

#[cfg(unix)]
fn force_disconnect(conn: &RustConnection) {
    use std::os::fd::AsRawFd;
    // SAFETY: shutdown(2) on a socket fd we own (through the connection) has
    // no memory-safety preconditions; the fd stays open until the connection
    // is dropped.
    unsafe {
        libc::shutdown(conn.stream().as_raw_fd(), libc::SHUT_RDWR);
    }
}

#[cfg(not(unix))]
fn force_disconnect(_conn: &RustConnection) {}

fn listen(
    conn: &RustConnection,
    xinput_opcode: u8,
    wake_window: xproto::Window,
    mut xtest: HashSet<u16>,
    kinds: InputKinds,
    stopping: &AtomicBool,
    log: &InputLog,
) {
    loop {
        let raw = match conn.wait_for_raw_event() {
            Ok(raw) => raw,
            Err(e) => {
                if !stopping.load(Ordering::SeqCst) {
                    tracing::warn!(error = %e, "XI2 listener lost its X11 connection");
                    log.mark_unhealthy(Utc::now());
                }
                return;
            }
        };
        if stopping.load(Ordering::SeqCst) {
            return;
        }
        if raw.first().map(|t| t & 0x7f) == Some(xproto::CLIENT_MESSAGE_EVENT)
            && read_u32(&raw, 4) == Some(wake_window)
        {
            continue;
        }
        match decode_xi2_event(&raw, xinput_opcode) {
            Some(Xi2Event::HierarchyChanged) => match query_xtest_devices(conn) {
                Ok(devices) => xtest = devices,
                Err(e) => {
                    // Without a current XTEST set, synthetic input could be
                    // counted as the user's: stop measuring instead.
                    tracing::warn!(error = %e, "could not refresh the XTEST devices");
                    log.mark_unhealthy(Utc::now());
                    return;
                }
            },
            Some(event) => apply_event(log, event, &xtest, kinds, Utc::now().timestamp()),
            None => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn masks_select_only_the_allowed_kinds() {
        let keyboard_only = event_masks(InputKinds {
            keyboard: true,
            mouse: false,
        });
        assert_eq!(keyboard_only[0].deviceid, 1, "XIAllMasterDevices");
        assert_eq!(keyboard_only[0].mask, vec![XIEventMask::RAW_KEY_PRESS]);
        assert_eq!(keyboard_only[1].deviceid, 0, "XIAllDevices");
        assert_eq!(keyboard_only[1].mask, vec![XIEventMask::HIERARCHY]);

        let mouse_only = event_masks(InputKinds {
            keyboard: false,
            mouse: true,
        });
        assert_eq!(
            mouse_only[0].mask,
            vec![XIEventMask::RAW_BUTTON_PRESS | XIEventMask::RAW_MOTION]
        );
        assert!(cleared_masks()
            .iter()
            .all(|m| m.mask == vec![XIEventMask::from(0u32)]));
    }

    #[test]
    fn only_click_buttons_are_clicks() {
        for click in [1, 2, 3, 8, 9] {
            assert!(is_click_button(click));
        }
        for other in [0, 4, 5, 6, 7, 10, 255] {
            assert!(!is_click_button(other));
        }
    }

    #[test]
    fn xtest_devices_are_found_by_name() {
        let devices: Vec<(u16, &[u8])> = vec![
            (2, b"Virtual core pointer"),
            (3, b"Virtual core keyboard"),
            (4, b"Virtual core XTEST pointer"),
            (5, b"Virtual core XTEST keyboard"),
            (6, b"Logitech USB Receiver"),
        ];
        assert_eq!(xtest_device_ids(devices), HashSet::from([4, 5]));
    }

    #[test]
    fn listening_needs_an_allowed_kind_and_a_display() {
        let log = Arc::new(InputLog::new());
        assert!(matches!(
            X11InputListener::start(":0", log.clone(), InputKinds::default()),
            Err(TrackingError::CapabilityDenied(_))
        ));
        assert!(matches!(
            X11InputListener::start(
                "",
                log,
                InputKinds {
                    keyboard: true,
                    mouse: true
                }
            ),
            Err(TrackingError::DisplayUnavailable(_))
        ));
    }
}
