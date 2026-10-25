//! X11 active window detection via EWMH (`_NET_ACTIVE_WINDOW`) and `WM_CLASS`.
//!
//! Enforces:
//! - Unprivileged X11 property queries without root or setuid escalation.
//! - Resolution of active window ID via EWMH `_NET_ACTIVE_WINDOW` on the root window.
//! - UTF-8 window title extraction via `_NET_WM_NAME`, falling back to `WM_NAME`.
//! - Application name resolution via `_NET_WM_PID` (mapped through `/proc/[pid]/exe`),
//!   falling back to `WM_CLASS` class or instance name.
//! - Actual X11 protocol property transport using pure-Rust `x11rb`.
//! - Layered transport seam (`X11PropertyTransport`) below `X11PropertyReader` allowing
//!   deterministic wire-level and property-level synthetic testing without a live display.
//! - Lazy connection/query acquisition: construction and `doctor` NEVER open displays or query windows.
//! - Graceful handling when no window is active (`_NET_ACTIVE_WINDOW = 0`), focused on root, or disconnected.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use super::model::{file_name, WindowInfo};
use super::procfs::ProcFsReader;
use super::traits::{ActiveWindowProvider, TrackingError};

/// Maximum bytes read for a window title property.
pub const MAX_WINDOW_TITLE_BYTES: usize = 512;

/// Maximum bytes read for a window class property.
pub const MAX_WM_CLASS_BYTES: usize = 256;

/// Raw property reply returned from X11 server protocol query.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawPropertyReply {
    /// Format of the property: 8, 16, or 32 bits per element.
    pub format: u8,
    /// X11 type atom ID (e.g. WINDOW, UTF8_STRING, STRING, CARDINAL).
    pub type_atom: u32,
    /// Raw byte payload.
    pub value: Vec<u8>,
}

impl RawPropertyReply {
    pub fn new(format: u8, type_atom: u32, value: Vec<u8>) -> Self {
        Self {
            format,
            type_atom,
            value,
        }
    }

    /// Construct a 32-bit CARDINAL or WINDOW property reply.
    pub fn new_u32(type_atom: u32, val: u32) -> Self {
        Self {
            format: 32,
            type_atom,
            value: val.to_ne_bytes().to_vec(),
        }
    }

    /// Construct an 8-bit STRING or UTF8_STRING property reply.
    pub fn new_bytes(type_atom: u32, bytes: &[u8]) -> Self {
        Self {
            format: 8,
            type_atom,
            value: bytes.to_vec(),
        }
    }

    /// Parse a raw X11 protocol wire packet using `x11rb`'s wire protocol parser.
    pub fn from_wire_bytes(bytes: &[u8]) -> Result<Self, TrackingError> {
        use x11rb::protocol::xproto::GetPropertyReply;
        use x11rb::x11_utils::TryParse;

        let (reply, _) = GetPropertyReply::try_parse(bytes).map_err(|e| {
            TrackingError::ProtocolError(format!("Malformed X11 protocol reply: {}", e))
        })?;

        Ok(Self {
            format: reply.format,
            type_atom: reply.type_,
            value: reply.value,
        })
    }
}

/// Helper to craft a compliant 32-byte header X11 GetProperty reply wire packet for tests.
pub fn craft_get_property_wire_reply(format: u8, type_atom: u32, value: &[u8]) -> Vec<u8> {
    let mut packet = vec![0u8; 32];
    packet[0] = 1; // Reply code
    packet[1] = format;
    packet[2..4].copy_from_slice(&1u16.to_ne_bytes()); // Sequence number

    let val_len: u32 = match format {
        8 => value.len() as u32,
        16 => (value.len() / 2) as u32,
        32 => (value.len() / 4) as u32,
        _ => 0,
    };
    let padded_len = (value.len() + 3) & !3;
    let length_words = (padded_len / 4) as u32;
    packet[4..8].copy_from_slice(&length_words.to_ne_bytes());
    packet[8..12].copy_from_slice(&type_atom.to_ne_bytes());
    packet[12..16].copy_from_slice(&0u32.to_ne_bytes()); // bytes_after
    packet[16..20].copy_from_slice(&val_len.to_ne_bytes());
    packet.extend_from_slice(value);
    while !packet.len().is_multiple_of(4) {
        packet.push(0);
    }
    packet
}

fn trim_null_bytes(bytes: &[u8]) -> &[u8] {
    let mut end = bytes.len();
    while end > 0 && bytes[end - 1] == 0 {
        end -= 1;
    }
    &bytes[..end]
}

/// Decode raw `_NET_ACTIVE_WINDOW` property into an active window ID.
pub fn decode_active_window(reply: &RawPropertyReply) -> Result<Option<u64>, TrackingError> {
    if reply.format != 32 || reply.value.len() < 4 {
        return Ok(None);
    }
    let xid = u32::from_ne_bytes(
        reply.value[0..4]
            .try_into()
            .map_err(|_| TrackingError::ProtocolError("Truncated window ID bytes".into()))?,
    );
    // In X11 / EWMH, window ID 0 (None) indicates no window has active focus
    if xid == 0 {
        Ok(None)
    } else {
        Ok(Some(xid as u64))
    }
}

/// Decode raw UTF-8 window title property (e.g. `_NET_WM_NAME`).
pub fn decode_utf8_title(reply: &RawPropertyReply) -> Option<String> {
    if reply.format == 8 && !reply.value.is_empty() {
        let slice = if reply.value.len() > MAX_WINDOW_TITLE_BYTES {
            &reply.value[..MAX_WINDOW_TITLE_BYTES]
        } else {
            &reply.value[..]
        };
        let trimmed = trim_null_bytes(slice);
        if !trimmed.is_empty() {
            let title = String::from_utf8_lossy(trimmed).trim().to_string();
            if !title.is_empty() {
                return Some(title);
            }
        }
    }
    None
}

/// Decode legacy STRING window title property (e.g. `WM_NAME` in Latin-1 or ASCII/UTF-8).
pub fn decode_legacy_title(reply: &RawPropertyReply) -> Option<String> {
    if reply.format == 8 && !reply.value.is_empty() {
        let slice = if reply.value.len() > MAX_WINDOW_TITLE_BYTES {
            &reply.value[..MAX_WINDOW_TITLE_BYTES]
        } else {
            &reply.value[..]
        };
        let trimmed = trim_null_bytes(slice);
        if !trimmed.is_empty() {
            let title = match std::str::from_utf8(trimmed) {
                Ok(valid) => valid.trim().to_string(),
                Err(_) => trimmed
                    .iter()
                    .map(|&b| b as char)
                    .collect::<String>()
                    .trim()
                    .to_string(),
            };
            if !title.is_empty() {
                return Some(title);
            }
        }
    }
    None
}

/// Decode raw window title from `_NET_WM_NAME` (UTF-8) with fallback to `WM_NAME` (STRING).
pub fn decode_window_title(
    net_wm_name: Option<&RawPropertyReply>,
    wm_name: Option<&RawPropertyReply>,
) -> Result<Option<String>, TrackingError> {
    // Priority 1: _NET_WM_NAME (UTF8_STRING)
    if let Some(reply) = net_wm_name {
        if let Some(title) = decode_utf8_title(reply) {
            return Ok(Some(title));
        }
    }

    // Priority 2: Fallback to WM_NAME (STRING: Latin-1 or ASCII/UTF-8)
    if let Some(reply) = wm_name {
        if let Some(title) = decode_legacy_title(reply) {
            return Ok(Some(title));
        }
    }

    Ok(None)
}

/// Decode raw `WM_CLASS` null-delimited strings into `(instance_name, class_name)`.
pub fn decode_wm_class(
    reply: &RawPropertyReply,
) -> Result<Option<(String, String)>, TrackingError> {
    if reply.format != 8 || reply.value.is_empty() {
        return Ok(None);
    }
    let slice = if reply.value.len() > MAX_WM_CLASS_BYTES {
        &reply.value[..MAX_WM_CLASS_BYTES]
    } else {
        &reply.value[..]
    };

    let mut parts = slice.split(|&b| b == 0);
    let instance_bytes = parts.next().unwrap_or_default();
    let class_bytes = parts.next().unwrap_or_default();

    let instance = String::from_utf8_lossy(instance_bytes).trim().to_string();
    let class = String::from_utf8_lossy(class_bytes).trim().to_string();

    if instance.is_empty() && class.is_empty() {
        Ok(None)
    } else if class.is_empty() {
        Ok(Some((instance.clone(), instance)))
    } else if instance.is_empty() {
        Ok(Some((class.clone(), class)))
    } else {
        Ok(Some((instance, class)))
    }
}

/// Decode raw `_NET_WM_PID` 32-bit integer property into a process ID.
pub fn decode_wm_pid(reply: &RawPropertyReply) -> Result<Option<u32>, TrackingError> {
    if reply.format != 32 || reply.value.len() < 4 {
        return Ok(None);
    }
    let pid = u32::from_ne_bytes(
        reply.value[0..4]
            .try_into()
            .map_err(|_| TrackingError::ProtocolError("Truncated PID bytes".into()))?,
    );
    if pid == 0 {
        Ok(None)
    } else {
        Ok(Some(pid))
    }
}

/// Low-level transport interface for X11 property queries below `X11PropertyReader`.
pub trait X11PropertyTransport: Send + Sync {
    /// Retrieve root window ID for default screen.
    fn root_window(&self) -> Result<u32, TrackingError>;

    /// Fetch raw property for given window and atom name.
    fn get_raw_property(
        &self,
        window: u32,
        property_name: &str,
        expected_type: Option<&str>,
        max_bytes: u32,
    ) -> Result<Option<RawPropertyReply>, TrackingError>;
}

/// In-memory synthetic X11 transport for deterministic tests below `X11PropertyReader`.
#[derive(Debug, Default)]
pub struct SyntheticX11Transport {
    root_window: Mutex<u32>,
    properties: Mutex<HashMap<(u32, String), RawPropertyReply>>,
    errors: Mutex<HashMap<(u32, String), TrackingError>>,
    global_error: Mutex<Option<TrackingError>>,
    calls: Mutex<Vec<(u32, String)>>,
}

impl SyntheticX11Transport {
    pub fn new() -> Self {
        Self {
            root_window: Mutex::new(0x0001),
            properties: Mutex::new(HashMap::new()),
            errors: Mutex::new(HashMap::new()),
            global_error: Mutex::new(None),
            calls: Mutex::new(Vec::new()),
        }
    }

    pub fn set_root_window(&self, root: u32) {
        *self.root_window.lock().unwrap() = root;
    }

    pub fn set_property(&self, window: u32, property: &str, reply: RawPropertyReply) {
        self.properties
            .lock()
            .unwrap()
            .insert((window, property.to_string()), reply);
    }

    pub fn remove_property(&self, window: u32, property: &str) {
        self.properties
            .lock()
            .unwrap()
            .remove(&(window, property.to_string()));
    }

    pub fn set_property_error(&self, window: u32, property: &str, err: TrackingError) {
        self.errors
            .lock()
            .unwrap()
            .insert((window, property.to_string()), err);
    }

    pub fn set_global_error(&self, err: Option<TrackingError>) {
        *self.global_error.lock().unwrap() = err;
    }

    pub fn call_log(&self) -> Vec<(u32, String)> {
        self.calls.lock().unwrap().clone()
    }
}

impl X11PropertyTransport for SyntheticX11Transport {
    fn root_window(&self) -> Result<u32, TrackingError> {
        if let Some(ref err) = *self.global_error.lock().unwrap() {
            return Err(err.clone());
        }
        Ok(*self.root_window.lock().unwrap())
    }

    fn get_raw_property(
        &self,
        window: u32,
        property_name: &str,
        _expected_type: Option<&str>,
        _max_bytes: u32,
    ) -> Result<Option<RawPropertyReply>, TrackingError> {
        if let Some(ref err) = *self.global_error.lock().unwrap() {
            return Err(err.clone());
        }
        self.calls
            .lock()
            .unwrap()
            .push((window, property_name.to_string()));

        if let Some(err) = self
            .errors
            .lock()
            .unwrap()
            .get(&(window, property_name.to_string()))
        {
            return Err(err.clone());
        }

        let props = self.properties.lock().unwrap();
        Ok(props.get(&(window, property_name.to_string())).cloned())
    }
}

/// Real system X11 property transport using `x11rb`.
///
/// Connects lazily to the X11 server upon the first property request.
/// Construction (`SysX11Transport::new()`) never opens displays or performs I/O.
#[derive(Default)]
pub struct SysX11Transport {
    #[cfg(target_os = "linux")]
    active_connection: Mutex<Option<ActiveX11Session>>,
}

#[cfg(target_os = "linux")]
struct ActiveX11Session {
    conn: x11rb::rust_connection::RustConnection,
    root_window: u32,
    atom_cache: HashMap<String, u32>,
}

#[cfg(target_os = "linux")]
impl SysX11Transport {
    pub fn new() -> Self {
        Self {
            active_connection: Mutex::new(None),
        }
    }

    fn get_or_connect(
        &self,
    ) -> Result<(std::sync::MutexGuard<'_, Option<ActiveX11Session>>, u32), TrackingError> {
        let mut guard = self.active_connection.lock().unwrap();
        if guard.is_none() {
            let display = std::env::var("DISPLAY").map_err(|_| {
                TrackingError::DisplayUnavailable("DISPLAY environment variable not set".into())
            })?;
            if display.trim().is_empty() {
                return Err(TrackingError::DisplayUnavailable(
                    "DISPLAY environment variable is empty".into(),
                ));
            }

            let (conn, screen_num) = x11rb::connect(Some(&display)).map_err(|e| {
                TrackingError::DisplayUnavailable(format!(
                    "Failed to connect to X11 display '{}': {}",
                    display, e
                ))
            })?;

            use x11rb::connection::Connection;

            let root_window = conn
                .setup()
                .roots
                .get(screen_num)
                .map(|s| s.root)
                .ok_or_else(|| {
                    TrackingError::DisplayUnavailable(format!(
                        "Invalid screen index {} returned by X11 server",
                        screen_num
                    ))
                })?;

            *guard = Some(ActiveX11Session {
                conn,
                root_window,
                atom_cache: HashMap::new(),
            });
        }

        let root = guard.as_ref().unwrap().root_window;
        Ok((guard, root))
    }
}

#[cfg(target_os = "linux")]
impl X11PropertyTransport for SysX11Transport {
    fn root_window(&self) -> Result<u32, TrackingError> {
        let (_guard, root) = self.get_or_connect()?;
        Ok(root)
    }

    fn get_raw_property(
        &self,
        window: u32,
        property_name: &str,
        expected_type: Option<&str>,
        max_bytes: u32,
    ) -> Result<Option<RawPropertyReply>, TrackingError> {
        use x11rb::protocol::xproto::{AtomEnum, ConnectionExt};

        let (mut guard, _) = self.get_or_connect()?;
        let session = guard.as_mut().unwrap();

        let prop_atom = get_or_intern_atom(&session.conn, &mut session.atom_cache, property_name)?;

        let type_atom = match expected_type {
            Some(type_name) => {
                get_or_intern_atom(&session.conn, &mut session.atom_cache, type_name)?
            }
            None => AtomEnum::ANY.into(),
        };

        let long_length = (max_bytes + 3) / 4;
        let cookie = session
            .conn
            .get_property(false, window, prop_atom, type_atom, 0, long_length)
            .map_err(|e| {
                TrackingError::ProtocolError(format!(
                    "Failed to send GetProperty for {}: {}",
                    property_name, e
                ))
            })?;

        let reply = cookie.reply().map_err(|e| {
            TrackingError::ProtocolError(format!(
                "Failed to receive GetProperty reply for {}: {}",
                property_name, e
            ))
        })?;

        if reply.format == 0 || reply.type_ == u32::from(AtomEnum::NONE) {
            return Ok(None);
        }

        Ok(Some(RawPropertyReply {
            format: reply.format,
            type_atom: reply.type_,
            value: reply.value,
        }))
    }
}

#[cfg(target_os = "linux")]
fn get_or_intern_atom(
    conn: &x11rb::rust_connection::RustConnection,
    cache: &mut HashMap<String, u32>,
    name: &str,
) -> Result<u32, TrackingError> {
    use x11rb::protocol::xproto::{AtomEnum, ConnectionExt};

    match name {
        "WINDOW" => return Ok(AtomEnum::WINDOW.into()),
        "STRING" => return Ok(AtomEnum::STRING.into()),
        "CARDINAL" => return Ok(AtomEnum::CARDINAL.into()),
        "WM_NAME" => return Ok(AtomEnum::WM_NAME.into()),
        "WM_CLASS" => return Ok(AtomEnum::WM_CLASS.into()),
        _ => {}
    }

    if let Some(&atom) = cache.get(name) {
        return Ok(atom);
    }

    let cookie = conn.intern_atom(false, name.as_bytes()).map_err(|e| {
        TrackingError::ProtocolError(format!("Failed to send InternAtom for {}: {}", name, e))
    })?;

    let reply = cookie.reply().map_err(|e| {
        TrackingError::ProtocolError(format!(
            "Failed to receive InternAtom reply for {}: {}",
            name, e
        ))
    })?;

    cache.insert(name.to_string(), reply.atom);
    Ok(reply.atom)
}

#[cfg(not(target_os = "linux"))]
impl SysX11Transport {
    pub fn new() -> Self {
        Self {}
    }
}

#[cfg(not(target_os = "linux"))]
impl X11PropertyTransport for SysX11Transport {
    fn root_window(&self) -> Result<u32, TrackingError> {
        let display = std::env::var("DISPLAY").map_err(|_| {
            TrackingError::DisplayUnavailable("DISPLAY environment variable not set".into())
        })?;
        if display.trim().is_empty() {
            return Err(TrackingError::DisplayUnavailable(
                "DISPLAY environment variable is empty".into(),
            ));
        }
        Err(TrackingError::DisplayUnavailable(
            "Native X11 display transport is only available on Linux".into(),
        ))
    }

    fn get_raw_property(
        &self,
        _window: u32,
        _property_name: &str,
        _expected_type: Option<&str>,
        _max_bytes: u32,
    ) -> Result<Option<RawPropertyReply>, TrackingError> {
        Err(TrackingError::DisplayUnavailable(
            "Native X11 display transport is only available on Linux".into(),
        ))
    }
}

/// Abstract seam for reading properties from an X11 display server.
pub trait X11PropertyReader: Send + Sync {
    /// Query `_NET_ACTIVE_WINDOW` on the root window.
    /// Returns `Some(xid)` if a window is active, or `None` if no window has focus.
    fn get_active_window(&self) -> Result<Option<u64>, TrackingError>;

    /// Query `_NET_WM_NAME` (UTF-8) or `WM_NAME` on the specified window.
    fn get_window_title(&self, window_id: u64) -> Result<Option<String>, TrackingError>;

    /// Query `WM_CLASS` on the specified window. Returns `(instance_name, class_name)`.
    fn get_window_class(&self, window_id: u64) -> Result<Option<(String, String)>, TrackingError>;

    /// Query `_NET_WM_PID` on the specified window.
    fn get_window_pid(&self, window_id: u64) -> Result<Option<u32>, TrackingError>;
}

/// Real system X11 property reader implementing `X11PropertyReader`.
///
/// Wraps an underlying `X11PropertyTransport` (defaulting to `SysX11Transport`).
/// Construction is strictly lazy: never connects to displays or queries windows at startup.
pub struct SysX11PropertyReader {
    transport: Box<dyn X11PropertyTransport>,
}

impl Default for SysX11PropertyReader {
    fn default() -> Self {
        Self::new()
    }
}

impl SysX11PropertyReader {
    /// Construct default system reader backed by `SysX11Transport`.
    pub fn new() -> Self {
        Self {
            transport: Box::new(SysX11Transport::new()),
        }
    }

    /// Construct reader with a custom transport (used by deterministic tests).
    pub fn with_transport(transport: Box<dyn X11PropertyTransport>) -> Self {
        Self { transport }
    }
}

impl X11PropertyReader for SysX11PropertyReader {
    fn get_active_window(&self) -> Result<Option<u64>, TrackingError> {
        let root = self.transport.root_window()?;
        let raw = self
            .transport
            .get_raw_property(root, "_NET_ACTIVE_WINDOW", Some("WINDOW"), 4)?;
        match raw {
            Some(reply) => {
                let win = decode_active_window(&reply)?;
                match win {
                    Some(id) if id == root as u64 => Ok(None),
                    other => Ok(other),
                }
            }
            None => Ok(None),
        }
    }

    fn get_window_title(&self, window_id: u64) -> Result<Option<String>, TrackingError> {
        let win = u32::try_from(window_id).map_err(|_| {
            TrackingError::ProtocolError(format!("Invalid X11 window ID {}", window_id))
        })?;

        // Priority 1: _NET_WM_NAME (UTF8_STRING)
        let net_wm_name = self.transport.get_raw_property(
            win,
            "_NET_WM_NAME",
            Some("UTF8_STRING"),
            MAX_WINDOW_TITLE_BYTES as u32,
        )?;

        if let Some(reply) = net_wm_name.as_ref() {
            if let Some(title) = decode_utf8_title(reply) {
                return Ok(Some(title));
            }
        }

        // Priority 2: Fallback to WM_NAME (STRING) if preferred is missing or decoded to None
        let wm_name = self.transport.get_raw_property(
            win,
            "WM_NAME",
            Some("STRING"),
            MAX_WINDOW_TITLE_BYTES as u32,
        )?;

        if let Some(reply) = wm_name.as_ref() {
            if let Some(title) = decode_legacy_title(reply) {
                return Ok(Some(title));
            }
        }

        Ok(None)
    }

    fn get_window_class(&self, window_id: u64) -> Result<Option<(String, String)>, TrackingError> {
        let win = u32::try_from(window_id).map_err(|_| {
            TrackingError::ProtocolError(format!("Invalid X11 window ID {}", window_id))
        })?;

        let raw = self.transport.get_raw_property(
            win,
            "WM_CLASS",
            Some("STRING"),
            MAX_WM_CLASS_BYTES as u32,
        )?;

        match raw {
            Some(reply) => decode_wm_class(&reply),
            None => Ok(None),
        }
    }

    fn get_window_pid(&self, window_id: u64) -> Result<Option<u32>, TrackingError> {
        let win = u32::try_from(window_id).map_err(|_| {
            TrackingError::ProtocolError(format!("Invalid X11 window ID {}", window_id))
        })?;

        let raw = self
            .transport
            .get_raw_property(win, "_NET_WM_PID", Some("CARDINAL"), 4)?;

        match raw {
            Some(reply) => decode_wm_pid(&reply),
            None => Ok(None),
        }
    }
}

/// In-memory mock X11 property reader for high-level tests without a display server.
#[derive(Debug, Default)]
pub struct MockX11PropertyReader {
    active_window: Mutex<Option<u64>>,
    titles: Mutex<HashMap<u64, String>>,
    classes: Mutex<HashMap<u64, (String, String)>>,
    pids: Mutex<HashMap<u64, u32>>,
    error: Mutex<Option<TrackingError>>,
}

impl MockX11PropertyReader {
    pub fn new() -> Self {
        Self {
            active_window: Mutex::new(None),
            titles: Mutex::new(HashMap::new()),
            classes: Mutex::new(HashMap::new()),
            pids: Mutex::new(HashMap::new()),
            error: Mutex::new(None),
        }
    }

    pub fn set_active_window(&self, window_id: Option<u64>) {
        *self.active_window.lock().unwrap() = window_id;
    }

    pub fn set_window_info(
        &self,
        window_id: u64,
        title: impl Into<String>,
        instance: impl Into<String>,
        class: impl Into<String>,
        pid: Option<u32>,
    ) {
        self.titles.lock().unwrap().insert(window_id, title.into());
        self.classes
            .lock()
            .unwrap()
            .insert(window_id, (instance.into(), class.into()));
        if let Some(p) = pid {
            self.pids.lock().unwrap().insert(window_id, p);
        } else {
            self.pids.lock().unwrap().remove(&window_id);
        }
    }

    pub fn set_error(&self, err: Option<TrackingError>) {
        *self.error.lock().unwrap() = err;
    }
}

impl X11PropertyReader for MockX11PropertyReader {
    fn get_active_window(&self) -> Result<Option<u64>, TrackingError> {
        if let Some(ref err) = *self.error.lock().unwrap() {
            return Err(err.clone());
        }
        let win = *self.active_window.lock().unwrap();
        if win == Some(0) {
            Ok(None)
        } else {
            Ok(win)
        }
    }

    fn get_window_title(&self, window_id: u64) -> Result<Option<String>, TrackingError> {
        if let Some(ref err) = *self.error.lock().unwrap() {
            return Err(err.clone());
        }
        let titles = self.titles.lock().unwrap();
        Ok(titles.get(&window_id).cloned())
    }

    fn get_window_class(&self, window_id: u64) -> Result<Option<(String, String)>, TrackingError> {
        if let Some(ref err) = *self.error.lock().unwrap() {
            return Err(err.clone());
        }
        let classes = self.classes.lock().unwrap();
        Ok(classes.get(&window_id).cloned())
    }

    fn get_window_pid(&self, window_id: u64) -> Result<Option<u32>, TrackingError> {
        if let Some(ref err) = *self.error.lock().unwrap() {
            return Err(err.clone());
        }
        let pids = self.pids.lock().unwrap();
        Ok(pids.get(&window_id).copied())
    }
}

/// X11 EWMH active window provider.
pub struct X11ActiveWindowProvider {
    reader: Arc<dyn X11PropertyReader>,
    proc_reader: Option<Arc<dyn ProcFsReader>>,
}

impl X11ActiveWindowProvider {
    pub fn new(
        reader: Arc<dyn X11PropertyReader>,
        proc_reader: Option<Arc<dyn ProcFsReader>>,
    ) -> Self {
        Self {
            reader,
            proc_reader,
        }
    }

    /// Construct provider backed by real `SysX11PropertyReader` and optional `/proc` reader.
    pub fn default_system(proc_reader: Option<Arc<dyn ProcFsReader>>) -> Self {
        Self::new(Arc::new(SysX11PropertyReader::new()), proc_reader)
    }

    /// Resolve the executable/application name given a window's PID and WM_CLASS.
    fn resolve_app_name(&self, pid: Option<u32>, wm_class: Option<(String, String)>) -> String {
        if let (Some(p), Some(ref proc)) = (pid, &self.proc_reader) {
            if let Ok(Some(exe_path)) = proc.read_exe(p) {
                let name = file_name(&exe_path);
                if !name.is_empty() {
                    return name.to_string();
                }
            }
            if let Ok(Some(comm)) = proc.read_comm(p) {
                let name = file_name(&comm);
                if !name.is_empty() {
                    return name.to_string();
                }
            }
        }

        if let Some((_instance, class)) = wm_class {
            let name = file_name(&class);
            if !name.is_empty() {
                return name.to_string();
            }
        }

        "Unknown".to_string()
    }
}

impl ActiveWindowProvider for X11ActiveWindowProvider {
    fn current_active_window(&self) -> Result<Option<WindowInfo>, TrackingError> {
        let window_id = match self.reader.get_active_window()? {
            Some(id) if id > 0 => id,
            _ => return Ok(None),
        };

        let title = self.reader.get_window_title(window_id)?.unwrap_or_default();

        let wm_class = self.reader.get_window_class(window_id)?;
        let pid = self.reader.get_window_pid(window_id)?;
        let app_name = self.resolve_app_name(pid, wm_class);

        Ok(Some(WindowInfo {
            app_name,
            window_title: title,
            seconds: None,
            keyboard_hits: None,
            mouse_clicks: None,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tracking::procfs::MockProcFsReader;

    #[test]
    fn x11_returns_none_when_no_active_window() {
        let reader = Arc::new(MockX11PropertyReader::new());
        reader.set_active_window(None);
        let provider = X11ActiveWindowProvider::new(reader, None);
        assert_eq!(provider.current_active_window().unwrap(), None);
    }

    #[test]
    fn x11_returns_none_when_active_window_is_zero() {
        let reader = Arc::new(MockX11PropertyReader::new());
        reader.set_active_window(Some(0));
        let provider = X11ActiveWindowProvider::new(reader, None);
        assert_eq!(provider.current_active_window().unwrap(), None);
    }

    #[test]
    fn x11_resolves_window_title_and_wm_class_fallback() {
        let reader = Arc::new(MockX11PropertyReader::new());
        let xid = 0x1234567;
        reader.set_active_window(Some(xid));
        reader.set_window_info(xid, "Antigravity IDE", "code", "Code", None);

        let provider = X11ActiveWindowProvider::new(reader, None);
        let info = provider.current_active_window().unwrap().unwrap();
        assert_eq!(info.window_title, "Antigravity IDE");
        assert_eq!(info.app_name, "Code");
    }

    #[test]
    fn x11_resolves_app_name_via_procfs_pid() {
        let reader = Arc::new(MockX11PropertyReader::new());
        let proc = Arc::new(MockProcFsReader::new());
        let xid = 0x1234567;
        let pid = 4321;
        reader.set_active_window(Some(xid));
        reader.set_window_info(xid, "Bash Terminal", "alacritty", "Alacritty", Some(pid));
        proc.add_process(
            pid,
            "alacritty",
            Some(vec!["/usr/bin/alacritty".into()]),
            Some("/usr/bin/alacritty".into()),
        );

        let provider = X11ActiveWindowProvider::new(reader, Some(proc));
        let info = provider.current_active_window().unwrap().unwrap();
        assert_eq!(info.window_title, "Bash Terminal");
        assert_eq!(info.app_name, "alacritty");
    }

    #[test]
    fn x11_propagates_reader_errors() {
        let reader = Arc::new(MockX11PropertyReader::new());
        reader.set_error(Some(TrackingError::DisplayUnavailable(
            "Connection refused".into(),
        )));
        let provider = X11ActiveWindowProvider::new(reader, None);
        let err = provider.current_active_window().unwrap_err();
        assert_eq!(
            err,
            TrackingError::DisplayUnavailable("Connection refused".into())
        );
    }

    #[test]
    fn sys_x11_property_reader_with_synthetic_transport_e2e() {
        let transport = Box::new(SyntheticX11Transport::new());
        let root = 0x0100;
        let active_win = 0x040001a;
        let pid = 9988;

        transport.set_root_window(root);
        transport.set_property(
            root,
            "_NET_ACTIVE_WINDOW",
            RawPropertyReply::new_u32(33, active_win),
        );
        transport.set_property(
            active_win,
            "_NET_WM_NAME",
            RawPropertyReply::new_bytes(100, b"Editor - dosi-tracker\0"),
        );
        transport.set_property(
            active_win,
            "WM_CLASS",
            RawPropertyReply::new_bytes(31, b"editor\0Editor\0"),
        );
        transport.set_property(active_win, "_NET_WM_PID", RawPropertyReply::new_u32(6, pid));

        let reader = Arc::new(SysX11PropertyReader::with_transport(transport));
        let proc = Arc::new(MockProcFsReader::new());
        proc.add_process(
            pid,
            "editor",
            Some(vec!["/usr/bin/editor".into()]),
            Some("/usr/bin/editor".into()),
        );

        let provider = X11ActiveWindowProvider::new(reader, Some(proc));
        let win = provider.current_active_window().unwrap().unwrap();
        assert_eq!(win.window_title, "Editor - dosi-tracker");
        assert_eq!(win.app_name, "editor");
    }

    #[test]
    fn synthetic_transport_wire_bytes_parsing_and_property_decoding() {
        // 1. Wire bytes for active window
        let wire_active = craft_get_property_wire_reply(32, 33, &0x0400002u32.to_ne_bytes());
        let reply_active = RawPropertyReply::from_wire_bytes(&wire_active).unwrap();
        assert_eq!(
            decode_active_window(&reply_active).unwrap(),
            Some(0x0400002)
        );

        // 2. Wire bytes for unfocused active window (0)
        let wire_zero = craft_get_property_wire_reply(32, 33, &0u32.to_ne_bytes());
        let reply_zero = RawPropertyReply::from_wire_bytes(&wire_zero).unwrap();
        assert_eq!(decode_active_window(&reply_zero).unwrap(), None);

        // 3. Wire bytes for UTF8 title
        let wire_title =
            craft_get_property_wire_reply(8, 100, "বাংলা উইন্ডো — Terminal 🚀".as_bytes());
        let reply_title = RawPropertyReply::from_wire_bytes(&wire_title).unwrap();
        assert_eq!(
            decode_window_title(Some(&reply_title), None).unwrap(),
            Some("বাংলা উইন্ডো — Terminal 🚀".to_string())
        );

        // 4. Fallback to WM_NAME when _NET_WM_NAME is empty or None
        let wire_wm_name = craft_get_property_wire_reply(8, 31, b"Legacy Title\0");
        let reply_wm_name = RawPropertyReply::from_wire_bytes(&wire_wm_name).unwrap();
        assert_eq!(
            decode_window_title(None, Some(&reply_wm_name)).unwrap(),
            Some("Legacy Title".to_string())
        );

        // 5. Wire bytes for WM_CLASS
        let wire_class = craft_get_property_wire_reply(8, 31, b"gimp-bin\0Gimp\0");
        let reply_class = RawPropertyReply::from_wire_bytes(&wire_class).unwrap();
        assert_eq!(
            decode_wm_class(&reply_class).unwrap(),
            Some(("gimp-bin".to_string(), "Gimp".to_string()))
        );

        // 6. Wire bytes for _NET_WM_PID
        let wire_pid = craft_get_property_wire_reply(32, 6, &12345u32.to_ne_bytes());
        let reply_pid = RawPropertyReply::from_wire_bytes(&wire_pid).unwrap();
        assert_eq!(decode_wm_pid(&reply_pid).unwrap(), Some(12345));
    }

    #[test]
    fn synthetic_transport_malformed_and_error_handling() {
        // Truncated window ID
        let raw_trunc_win = RawPropertyReply::new(32, 33, vec![1, 2]);
        assert_eq!(decode_active_window(&raw_trunc_win).unwrap(), None);

        // Wrong format for window ID (format 8 instead of 32)
        let raw_wrong_format = RawPropertyReply::new(8, 33, vec![1, 2, 3, 4]);
        assert_eq!(decode_active_window(&raw_wrong_format).unwrap(), None);

        // Truncated PID
        let raw_trunc_pid = RawPropertyReply::new(32, 6, vec![1, 2]);
        assert_eq!(decode_wm_pid(&raw_trunc_pid).unwrap(), None);

        // PID 0 is treated as no PID
        let raw_zero_pid = RawPropertyReply::new_u32(6, 0);
        assert_eq!(decode_wm_pid(&raw_zero_pid).unwrap(), None);

        // Single string in WM_CLASS without second null
        let raw_single_class = RawPropertyReply::new_bytes(31, b"SingleApp\0");
        assert_eq!(
            decode_wm_class(&raw_single_class).unwrap(),
            Some(("SingleApp".to_string(), "SingleApp".to_string()))
        );

        // Empty WM_CLASS
        let raw_empty_class = RawPropertyReply::new_bytes(31, b"");
        assert_eq!(decode_wm_class(&raw_empty_class).unwrap(), None);

        // Corrupted wire bytes
        let malformed_wire = [0u8; 10];
        assert!(RawPropertyReply::from_wire_bytes(&malformed_wire).is_err());
    }

    #[test]
    fn root_window_as_active_window_normalizes_to_none_and_avoids_further_queries() {
        let transport = Box::new(SyntheticX11Transport::new());
        let root = 0x0100;
        transport.set_root_window(root);

        // Root window is reported as active window (e.g. desktop/background clicked)
        transport.set_property(
            root,
            "_NET_ACTIVE_WINDOW",
            RawPropertyReply::new_u32(33, root),
        );
        // Properties configured on root that should NOT be queried
        transport.set_property(
            root,
            "_NET_WM_NAME",
            RawPropertyReply::new_bytes(100, b"Desktop\0"),
        );
        transport.set_property(
            root,
            "WM_CLASS",
            RawPropertyReply::new_bytes(31, b"desktop\0Desktop\0"),
        );
        transport.set_property(root, "_NET_WM_PID", RawPropertyReply::new_u32(6, 1));

        let reader = Arc::new(SysX11PropertyReader::with_transport(transport));
        assert_eq!(reader.get_active_window().unwrap(), None);

        let provider = X11ActiveWindowProvider::new(reader, None);
        assert_eq!(provider.current_active_window().unwrap(), None);
    }

    #[test]
    fn synthetic_transport_root_window_normalization_call_log_assert() {
        let transport = Arc::new(SyntheticX11Transport::new());
        let root = 0x0100;
        transport.set_root_window(root);
        transport.set_property(
            root,
            "_NET_ACTIVE_WINDOW",
            RawPropertyReply::new_u32(33, root),
        );

        struct SharedTransport(Arc<SyntheticX11Transport>);
        impl X11PropertyTransport for SharedTransport {
            fn root_window(&self) -> Result<u32, TrackingError> {
                self.0.root_window()
            }
            fn get_raw_property(
                &self,
                window: u32,
                property_name: &str,
                expected_type: Option<&str>,
                max_bytes: u32,
            ) -> Result<Option<RawPropertyReply>, TrackingError> {
                self.0
                    .get_raw_property(window, property_name, expected_type, max_bytes)
            }
        }

        let reader = Arc::new(SysX11PropertyReader::with_transport(Box::new(
            SharedTransport(transport.clone()),
        )));
        let provider = X11ActiveWindowProvider::new(reader.clone(), None);

        assert_eq!(reader.get_active_window().unwrap(), None);
        assert_eq!(provider.current_active_window().unwrap(), None);

        let calls = transport.call_log();
        assert_eq!(calls.len(), 2); // 1 for reader.get_active_window, 1 for provider.current_active_window
        for (win, prop) in calls {
            assert_eq!(win, root);
            assert_eq!(prop, "_NET_ACTIVE_WINDOW");
        }
    }

    #[test]
    fn preferred_title_nul_only_falls_back_to_wm_name() {
        let transport = Arc::new(SyntheticX11Transport::new());
        let win = 0x0200;
        transport.set_property(
            win,
            "_NET_WM_NAME",
            RawPropertyReply::new_bytes(100, b"\0\0\0"),
        );
        transport.set_property(
            win,
            "WM_NAME",
            RawPropertyReply::new_bytes(31, b"Fallback App\0"),
        );

        struct SharedTransport(Arc<SyntheticX11Transport>);
        impl X11PropertyTransport for SharedTransport {
            fn root_window(&self) -> Result<u32, TrackingError> {
                self.0.root_window()
            }
            fn get_raw_property(
                &self,
                window: u32,
                property_name: &str,
                expected_type: Option<&str>,
                max_bytes: u32,
            ) -> Result<Option<RawPropertyReply>, TrackingError> {
                self.0
                    .get_raw_property(window, property_name, expected_type, max_bytes)
            }
        }

        let reader =
            SysX11PropertyReader::with_transport(Box::new(SharedTransport(transport.clone())));
        let title = reader.get_window_title(win as u64).unwrap();
        assert_eq!(title, Some("Fallback App".to_string()));

        let calls = transport.call_log();
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0], (win, "_NET_WM_NAME".to_string()));
        assert_eq!(calls[1], (win, "WM_NAME".to_string()));
    }

    #[test]
    fn preferred_title_whitespace_only_falls_back_to_wm_name() {
        let transport = Arc::new(SyntheticX11Transport::new());
        let win = 0x0200;
        transport.set_property(
            win,
            "_NET_WM_NAME",
            RawPropertyReply::new_bytes(100, b"   \t\r\n\0"),
        );
        transport.set_property(
            win,
            "WM_NAME",
            RawPropertyReply::new_bytes(31, b"Fallback App\0"),
        );

        struct SharedTransport(Arc<SyntheticX11Transport>);
        impl X11PropertyTransport for SharedTransport {
            fn root_window(&self) -> Result<u32, TrackingError> {
                self.0.root_window()
            }
            fn get_raw_property(
                &self,
                window: u32,
                property_name: &str,
                expected_type: Option<&str>,
                max_bytes: u32,
            ) -> Result<Option<RawPropertyReply>, TrackingError> {
                self.0
                    .get_raw_property(window, property_name, expected_type, max_bytes)
            }
        }

        let reader =
            SysX11PropertyReader::with_transport(Box::new(SharedTransport(transport.clone())));
        let title = reader.get_window_title(win as u64).unwrap();
        assert_eq!(title, Some("Fallback App".to_string()));

        let calls = transport.call_log();
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0], (win, "_NET_WM_NAME".to_string()));
        assert_eq!(calls[1], (win, "WM_NAME".to_string()));
    }

    #[test]
    fn valid_preferred_title_avoids_legacy_fallback_query() {
        let transport = Arc::new(SyntheticX11Transport::new());
        let win = 0x0200;
        transport.set_property(
            win,
            "_NET_WM_NAME",
            RawPropertyReply::new_bytes(100, b"Modern Editor\0"),
        );
        transport.set_property(
            win,
            "WM_NAME",
            RawPropertyReply::new_bytes(31, b"Legacy Editor\0"),
        );

        struct SharedTransport(Arc<SyntheticX11Transport>);
        impl X11PropertyTransport for SharedTransport {
            fn root_window(&self) -> Result<u32, TrackingError> {
                self.0.root_window()
            }
            fn get_raw_property(
                &self,
                window: u32,
                property_name: &str,
                expected_type: Option<&str>,
                max_bytes: u32,
            ) -> Result<Option<RawPropertyReply>, TrackingError> {
                self.0
                    .get_raw_property(window, property_name, expected_type, max_bytes)
            }
        }

        let reader =
            SysX11PropertyReader::with_transport(Box::new(SharedTransport(transport.clone())));
        let title = reader.get_window_title(win as u64).unwrap();
        assert_eq!(title, Some("Modern Editor".to_string()));

        // Crucial test: WM_NAME was never queried because preferred decoded successfully!
        let calls = transport.call_log();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0], (win, "_NET_WM_NAME".to_string()));
    }

    #[test]
    fn legacy_latin1_string_wm_name_fallback() {
        let transport = Arc::new(SyntheticX11Transport::new());
        let win = 0x0200;
        // _NET_WM_NAME is missing
        transport.set_property(
            win,
            "WM_NAME",
            RawPropertyReply::new_bytes(31, b"Caf\xe9\0"),
        );

        struct SharedTransport(Arc<SyntheticX11Transport>);
        impl X11PropertyTransport for SharedTransport {
            fn root_window(&self) -> Result<u32, TrackingError> {
                self.0.root_window()
            }
            fn get_raw_property(
                &self,
                window: u32,
                property_name: &str,
                expected_type: Option<&str>,
                max_bytes: u32,
            ) -> Result<Option<RawPropertyReply>, TrackingError> {
                self.0
                    .get_raw_property(window, property_name, expected_type, max_bytes)
            }
        }

        let reader =
            SysX11PropertyReader::with_transport(Box::new(SharedTransport(transport.clone())));
        let title = reader.get_window_title(win as u64).unwrap();
        assert_eq!(title, Some("Café".to_string()));
    }
}
