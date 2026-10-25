//! Wayland screen capture through `org.freedesktop.portal.Screenshot`
//! (non-interactive) with Denied/Unavailable states; the returned file is deleted.
//!
//! One capture is one portal request, over zbus 5 on an explicitly given
//! session bus:
//!
//! 1. Subscribe to `org.freedesktop.portal.Request.Response` on the handle
//!    path the portal will use —
//!    `/org/freedesktop/portal/desktop/request/<sender>/<token>`, where
//!    `<sender>` is our unique bus name without the `:` and with `.` replaced
//!    by `_` — *before* calling, so the answer cannot be missed.
//! 2. Call `Screenshot("", {handle_token, interactive: false})`.
//! 3. Wait at most [`DEFAULT_TIMEOUT`]. On timeout the request is closed
//!    (`Request.Close`) and the result is "needs approval" (the desktop is
//!    showing a dialog nobody answered).
//! 4. Response 0: the `uri` result (only `file://`) is opened without
//!    following symlinks, checked to be a regular file owned by our euid,
//!    decoded as PNG, and exactly that file is deleted. A failed deletion is
//!    logged and reported through [`PortalScreenSource::cleanup_failure`].
//! 5. Response 1 (cancelled) or 2 (other): "denied". The source then refuses
//!    every further request without calling the portal until [`reset`]
//!    (the user's explicit retry), so a dialog never pops up again on its own.
//!
//! No portal service on the bus (or no Screenshot interface) is "unsupported".
//!
//! [`reset`]: PortalScreenSource::reset

use std::collections::HashMap;
use std::future::Future;
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll, Waker};
use std::time::Duration;

use chrono::Utc;
use zbus::blocking::{Connection, MessageIterator};
use zbus::export::futures_core::Stream;
use zbus::zvariant::{OwnedObjectPath, OwnedValue, Value};

use super::capture::{decode_png, RawFrame, MAX_PNG_FILE_BYTES};
use super::traits::{Availability, CaptureTarget, ScreenSource, TrackingError};

/// The portal's well-known name.
pub const PORTAL_BUS_NAME: &str = "org.freedesktop.portal.Desktop";
/// The portal's object.
pub const PORTAL_OBJECT_PATH: &str = "/org/freedesktop/portal/desktop";
pub const SCREENSHOT_INTERFACE: &str = "org.freedesktop.portal.Screenshot";
pub const REQUEST_INTERFACE: &str = "org.freedesktop.portal.Request";

/// How long a request may wait for its answer.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);

/// The handle path of a request with `token` from the connection whose
/// unique name is `unique_name` (`:1.42` -> `.../request/1_42/<token>`).
pub fn request_handle_path(unique_name: &str, token: &str) -> String {
    let sender = unique_name.trim_start_matches(':').replace('.', "_");
    format!("{PORTAL_OBJECT_PATH}/request/{sender}/{token}")
}

/// A fresh handle token (`[A-Za-z0-9_]` only, as the portal requires).
pub fn new_handle_token() -> String {
    format!("dosi_{}", uuid::Uuid::new_v4().simple())
}

/// The local path of a `file://` URI, percent-decoded; `None` for any other
/// scheme, a remote host, or a malformed escape.
pub fn file_uri_path(uri: &str) -> Option<std::path::PathBuf> {
    let rest = uri.strip_prefix("file://")?;
    // Only an empty or `localhost` authority is local.
    let path = if let Some(path) = rest.strip_prefix("localhost") {
        path
    } else {
        rest
    };
    if !path.starts_with('/') {
        return None;
    }
    let bytes = path.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let hex = std::str::from_utf8(bytes.get(i + 1..i + 3)?).ok()?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    if out.contains(&0) {
        return None;
    }
    use std::os::unix::ffi::OsStringExt;
    Some(std::path::PathBuf::from(std::ffi::OsString::from_vec(out)))
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum State {
    /// Nothing known yet, or the last request worked.
    Ready,
    /// The user refused: no request until `reset`.
    Denied,
    /// The last request timed out.
    NeedsApproval,
    /// No portal / no Screenshot interface.
    Unsupported(String),
    /// The last request failed for another reason.
    Unavailable(String),
}

/// Screenshot portal source on one session bus.
pub struct PortalScreenSource {
    bus_address: String,
    timeout: Duration,
    state: Mutex<State>,
    cleanup_failure: Mutex<Option<String>>,
    requests: std::sync::atomic::AtomicU64,
}

fn dbus_failed(context: &str, e: impl std::fmt::Display) -> TrackingError {
    TrackingError::IoError(format!("{context}: {e}"))
}

/// Is a method-call error "there is no such portal"?
fn is_unsupported(e: &zbus::Error) -> bool {
    match e {
        zbus::Error::MethodError(name, _, _) => matches!(
            name.as_str(),
            "org.freedesktop.DBus.Error.ServiceUnknown"
                | "org.freedesktop.DBus.Error.NameHasNoOwner"
                | "org.freedesktop.DBus.Error.UnknownMethod"
                | "org.freedesktop.DBus.Error.UnknownInterface"
                | "org.freedesktop.DBus.Error.UnknownObject"
        ),
        zbus::Error::FDO(fdo) => matches!(
            **fdo,
            zbus::fdo::Error::ServiceUnknown(_)
                | zbus::fdo::Error::NameHasNoOwner(_)
                | zbus::fdo::Error::UnknownMethod(_)
                | zbus::fdo::Error::UnknownInterface(_)
                | zbus::fdo::Error::UnknownObject(_)
        ),
        _ => false,
    }
}

impl PortalScreenSource {
    /// A source on the session bus at `bus_address`; nothing is connected yet.
    pub fn new(bus_address: impl Into<String>) -> Self {
        Self {
            bus_address: bus_address.into(),
            timeout: DEFAULT_TIMEOUT,
            state: Mutex::new(State::Ready),
            cleanup_failure: Mutex::new(None),
            requests: std::sync::atomic::AtomicU64::new(0),
        }
    }

    /// Use another answer timeout (tests).
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    /// The user asked to try again (`capture retry`, GUI button): a denied
    /// source may call the portal once more.
    pub fn reset(&self) {
        *self.state.lock().unwrap_or_else(|e| e.into_inner()) = State::Ready;
    }

    /// Portal requests made so far (not counting refused attempts).
    pub fn requests_made(&self) -> u64 {
        self.requests.load(Ordering::SeqCst)
    }

    /// The last time a returned screenshot file could not be deleted, why.
    pub fn cleanup_failure(&self) -> Option<String> {
        self.cleanup_failure
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    fn set_state(&self, state: State) {
        *self.state.lock().unwrap_or_else(|e| e.into_inner()) = state;
    }

    fn request(&self) -> Result<RawFrame, TrackingError> {
        let conn = zbus::blocking::connection::Builder::address(self.bus_address.as_str())
            .map_err(|e| dbus_failed("D-Bus address", e))?
            .build()
            .map_err(|e| TrackingError::UnsupportedSession(format!("no session bus: {e}")))?;
        let unique = conn
            .unique_name()
            .map(|n| n.to_string())
            .ok_or_else(|| dbus_failed("D-Bus", "no unique name"))?;
        let token = new_handle_token();
        let predicted = request_handle_path(&unique, &token);

        // Subscribe before calling, so the answer cannot be missed.
        let responses = subscribe(&conn, &predicted)?;

        let mut options: HashMap<&str, Value<'_>> = HashMap::new();
        options.insert("handle_token", Value::from(token.as_str()));
        options.insert("interactive", Value::from(false));
        self.requests.fetch_add(1, Ordering::SeqCst);
        let reply = conn.call_method(
            Some(PORTAL_BUS_NAME),
            PORTAL_OBJECT_PATH,
            Some(SCREENSHOT_INTERFACE),
            "Screenshot",
            &("", options),
        );
        let reply = match reply {
            Ok(reply) => reply,
            Err(e) if is_unsupported(&e) => {
                return Err(TrackingError::UnsupportedSession(format!(
                    "no Screenshot portal on the session bus: {e}"
                )))
            }
            Err(e) => return Err(dbus_failed("Screenshot portal call", e)),
        };
        let handle: OwnedObjectPath = reply
            .body()
            .deserialize()
            .map_err(|e| dbus_failed("Screenshot portal reply", e))?;

        // Old portals ignore handle_token and pick their own path.
        let responses = if handle.as_str() == predicted {
            responses
        } else {
            subscribe(&conn, handle.as_str())?
        };

        match wait_for_message(responses, self.timeout) {
            Some(Ok(message)) => {
                let (code, results): (u32, HashMap<String, OwnedValue>) = message
                    .body()
                    .deserialize()
                    .map_err(|e| dbus_failed("Request.Response", e))?;
                match code {
                    0 => self.read_result(&results),
                    _ => Err(TrackingError::PermissionDenied(format!(
                        "the screenshot request was refused (response {code})"
                    ))),
                }
            }
            Some(Err(e)) => Err(dbus_failed("Request.Response", e)),
            None => {
                // Nobody answered the desktop's dialog: withdraw the request.
                let _ = conn.call_method(
                    Some(PORTAL_BUS_NAME),
                    handle.as_str(),
                    Some(REQUEST_INTERFACE),
                    "Close",
                    &(),
                );
                Err(TrackingError::NeedsApproval(
                    "the desktop is waiting for screenshot permission".into(),
                ))
            }
        }
    }

    fn read_result(
        &self,
        results: &HashMap<String, OwnedValue>,
    ) -> Result<RawFrame, TrackingError> {
        let captured_at = Utc::now();
        let uri = results
            .get("uri")
            .and_then(|v| <&str>::try_from(&**v).ok())
            .ok_or_else(|| dbus_failed("Screenshot portal", "no uri in the response"))?;
        let path = file_uri_path(uri).ok_or_else(|| {
            dbus_failed("Screenshot portal", format!("not a local file URI: {uri}"))
        })?;
        let bytes = read_own_regular_file(&path)?;
        // The file is ours to remove once read, decodable or not.
        if let Err(reason) = remove_exact_file(&path, &bytes.identity) {
            tracing::warn!(path = %path.display(), %reason, "could not delete the portal screenshot");
            *self
                .cleanup_failure
                .lock()
                .unwrap_or_else(|e| e.into_inner()) = Some(reason);
        }
        let image = decode_png(&bytes.data)?;
        Ok(RawFrame {
            image,
            captured_at,
            monitor: None,
        })
    }
}

impl ScreenSource for PortalScreenSource {
    fn capture(&self, _target: CaptureTarget) -> Result<RawFrame, TrackingError> {
        if *self.state.lock().unwrap_or_else(|e| e.into_inner()) == State::Denied {
            return Err(TrackingError::PermissionDenied(
                "screenshots were refused; retry from the agent to ask again".into(),
            ));
        }
        let result = self.request();
        self.set_state(match &result {
            Ok(_) => State::Ready,
            Err(TrackingError::PermissionDenied(_)) => State::Denied,
            Err(TrackingError::NeedsApproval(_)) => State::NeedsApproval,
            Err(TrackingError::UnsupportedSession(reason)) => State::Unsupported(reason.clone()),
            Err(other) => State::Unavailable(other.to_string()),
        });
        result
    }

    fn availability(&self) -> Availability {
        match &*self.state.lock().unwrap_or_else(|e| e.into_inner()) {
            State::Ready => Availability::Available,
            State::Denied => Availability::Denied,
            State::NeedsApproval => Availability::NeedsApproval,
            State::Unsupported(reason) => Availability::Unsupported(reason.clone()),
            State::Unavailable(reason) => Availability::Unavailable(reason.clone()),
        }
    }
}

fn subscribe(conn: &Connection, path: &str) -> Result<MessageIterator, TrackingError> {
    let rule = zbus::MatchRule::builder()
        .msg_type(zbus::message::Type::Signal)
        .interface(REQUEST_INTERFACE)
        .and_then(|b| b.member("Response"))
        .and_then(|b| b.path(path.to_string()))
        .map_err(|e| dbus_failed("match rule", e))?
        .build();
    MessageIterator::for_match_rule(rule, conn, Some(8))
        .map_err(|e| dbus_failed("Request.Response subscription", e))
}

/// The next message of `messages`, or `None` after `timeout`.
fn wait_for_message(
    messages: MessageIterator,
    timeout: Duration,
) -> Option<zbus::Result<zbus::Message>> {
    let mut stream = Box::pin(messages.into_inner());
    let mut deadline = Deadline::new(timeout);
    zbus::block_on(std::future::poll_fn(move |cx| {
        if let Poll::Ready(item) = stream.as_mut().poll_next(cx) {
            return Poll::Ready(item);
        }
        match Pin::new(&mut deadline).poll(cx) {
            Poll::Ready(()) => Poll::Ready(None),
            Poll::Pending => Poll::Pending,
        }
    }))
}

/// A future that completes after a delay, woken by a helper thread (zbus's
/// executor timer is not public, and no other runtime is used here).
struct Deadline {
    shared: Arc<(AtomicBool, Mutex<Option<Waker>>)>,
}

impl Deadline {
    fn new(after: Duration) -> Self {
        let shared = Arc::new((AtomicBool::new(false), Mutex::new(None::<Waker>)));
        let timer = shared.clone();
        std::thread::Builder::new()
            .name("dosi-portal-timeout".into())
            .spawn(move || {
                std::thread::sleep(after);
                timer.0.store(true, Ordering::SeqCst);
                if let Some(waker) = timer.1.lock().unwrap_or_else(|e| e.into_inner()).take() {
                    waker.wake();
                }
            })
            .map(drop)
            .unwrap_or_else(|_| shared.0.store(true, Ordering::SeqCst));
        Self { shared }
    }
}

impl Future for Deadline {
    type Output = ();

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        let (expired, waker) = &*self.shared;
        *waker.lock().unwrap_or_else(|e| e.into_inner()) = Some(cx.waker().clone());
        if expired.load(Ordering::SeqCst) {
            Poll::Ready(())
        } else {
            Poll::Pending
        }
    }
}

/// The identity of a file we read: removal checks it is still the same file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct FileIdentity {
    dev: u64,
    ino: u64,
}

struct OwnFile {
    data: Vec<u8>,
    identity: FileIdentity,
}

fn euid() -> u32 {
    // SAFETY: geteuid has no preconditions and cannot fail.
    unsafe { libc::geteuid() }
}

/// Read `path` if it is a regular file owned by our euid, never following a
/// symlink and never reading more than [`MAX_PNG_FILE_BYTES`].
fn read_own_regular_file(path: &std::path::Path) -> Result<OwnFile, TrackingError> {
    use std::io::Read;
    let file = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC)
        .open(path)
        .map_err(|e| dbus_failed("portal screenshot file", e))?;
    let meta = file
        .metadata()
        .map_err(|e| dbus_failed("portal screenshot file", e))?;
    if !meta.file_type().is_file() {
        return Err(dbus_failed("portal screenshot file", "not a regular file"));
    }
    if meta.uid() != euid() {
        return Err(dbus_failed(
            "portal screenshot file",
            "owned by another user",
        ));
    }
    if meta.len() > MAX_PNG_FILE_BYTES {
        return Err(TrackingError::EncodeError(
            "the portal screenshot is too large".into(),
        ));
    }
    let mut data = Vec::with_capacity(meta.len() as usize);
    file.take(MAX_PNG_FILE_BYTES + 1)
        .read_to_end(&mut data)
        .map_err(|e| dbus_failed("portal screenshot file", e))?;
    Ok(OwnFile {
        data,
        identity: FileIdentity {
            dev: meta.dev(),
            ino: meta.ino(),
        },
    })
}

/// Delete `path` only if it still is the regular file we read and ours.
fn remove_exact_file(path: &std::path::Path, identity: &FileIdentity) -> Result<(), String> {
    let meta = std::fs::symlink_metadata(path).map_err(|e| e.to_string())?;
    if !meta.file_type().is_file()
        || meta.uid() != euid()
        || (FileIdentity {
            dev: meta.dev(),
            ino: meta.ino(),
        }) != *identity
    {
        return Err("the file changed after it was read; left in place".into());
    }
    std::fs::remove_file(path).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_handle_path_is_predicted_from_the_unique_name() {
        assert_eq!(
            request_handle_path(":1.42", "dosi_abc"),
            "/org/freedesktop/portal/desktop/request/1_42/dosi_abc"
        );
        let token = new_handle_token();
        assert!(token.chars().all(|c| c.is_ascii_alphanumeric() || c == '_'));
        assert_ne!(token, new_handle_token());
    }

    #[test]
    fn only_local_file_uris_are_accepted() {
        assert_eq!(
            file_uri_path("file:///tmp/Screenshot%20from%20now.png").unwrap(),
            std::path::PathBuf::from("/tmp/Screenshot from now.png")
        );
        assert_eq!(
            file_uri_path("file://localhost/tmp/a.png").unwrap(),
            std::path::PathBuf::from("/tmp/a.png")
        );
        assert!(file_uri_path("https://example.com/a.png").is_none());
        assert!(file_uri_path("file://otherhost/tmp/a.png").is_none());
        assert!(file_uri_path("file:///tmp/%zz.png").is_none());
        assert!(file_uri_path("file:///tmp/%00.png").is_none());
        assert!(file_uri_path("/tmp/a.png").is_none());
    }

    #[test]
    fn only_our_own_regular_files_are_read_and_deleted() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("shot.png");
        std::fs::write(&file, b"png").unwrap();
        let read = read_own_regular_file(&file).unwrap();
        assert_eq!(read.data, b"png");

        // A symlink is never followed.
        let link = dir.path().join("link.png");
        std::os::unix::fs::symlink(&file, &link).unwrap();
        assert!(read_own_regular_file(&link).is_err());
        // A directory is not a file.
        assert!(read_own_regular_file(dir.path()).is_err());

        // A file swapped after reading is left alone.
        let other = dir.path().join("other.png");
        std::fs::write(&other, b"other").unwrap();
        std::fs::rename(&other, &file).unwrap();
        assert!(remove_exact_file(&file, &read.identity).is_err());
        assert!(file.exists());

        let again = read_own_regular_file(&file).unwrap();
        remove_exact_file(&file, &again.identity).unwrap();
        assert!(!file.exists());
    }
}
