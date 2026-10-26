//! Fake `org.freedesktop.portal.Screenshot`: responses 0/1/2 and a synthetic
//! PNG in a temporary directory.
//!
//! Behaves like xdg-desktop-portal: `Screenshot` returns the request handle
//! `/org/freedesktop/portal/desktop/request/<sender>/<token>` (computed here
//! independently from the caller's unique name and `handle_token`), exports a
//! `Request` object there (with `Close`), and emits `Request.Response` from
//! it shortly after replying. Every call and every `Close` is recorded.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use image::ImageEncoder;
use tempfile::TempDir;
use zbus::blocking::Connection;
use zbus::interface;
use zbus::message::Header;
use zbus::zvariant::{OwnedObjectPath, OwnedValue, Value};

/// How the fake answers the next requests.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PortalAnswer {
    /// Response 0 with a `width` x `height` PNG of a solid colour.
    Screenshot { width: u32, height: u32 },
    /// Response 1: the user cancelled.
    Cancelled,
    /// Response 2: refused for another reason.
    Other,
    /// Never answer (a permission dialog nobody clicks).
    Silent,
    /// Response 0 with a `uri` that is not a local file.
    RemoteUri,
}

/// One recorded `Screenshot` call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScreenshotCall {
    pub sender: String,
    pub parent_window: String,
    pub handle_token: Option<String>,
    pub interactive: Option<bool>,
    pub handle: String,
}

/// The colour of every synthetic screenshot.
pub const SHOT_COLOUR: [u8; 3] = [0x33, 0x66, 0xcc];

#[derive(Default)]
struct State {
    answer: Mutex<Option<PortalAnswer>>,
    /// Ignore `handle_token` and choose another handle (pre-0.9 portals).
    ignore_token: Mutex<bool>,
    /// Make the PNG directory read-only after writing (deletion must fail).
    lock_dir: Mutex<bool>,
    calls: Mutex<Vec<ScreenshotCall>>,
    closes: Mutex<Vec<String>>,
    files: Mutex<Vec<PathBuf>>,
    dir: Mutex<PathBuf>,
}

struct ScreenshotIface {
    state: Arc<State>,
}

struct RequestIface {
    state: Arc<State>,
    path: String,
}

#[interface(name = "org.freedesktop.portal.Request")]
impl RequestIface {
    fn close(&self) {
        self.state.closes.lock().unwrap().push(self.path.clone());
    }
}

fn png_file(dir: &std::path::Path, index: usize, width: u32, height: u32) -> PathBuf {
    let image = image::RgbImage::from_pixel(width, height, image::Rgb(SHOT_COLOUR));
    let mut png = Vec::new();
    image::codecs::png::PngEncoder::new(&mut png)
        .write_image(
            image.as_raw(),
            width,
            height,
            image::ExtendedColorType::Rgb8,
        )
        .unwrap();
    // A space in the name exercises the URI's percent-encoding.
    let path = dir.join(format!("Screenshot from test {index}.png"));
    std::fs::write(&path, png).unwrap();
    path
}

fn file_uri(path: &std::path::Path) -> String {
    format!("file://{}", path.display().to_string().replace(' ', "%20"))
}

#[interface(name = "org.freedesktop.portal.Screenshot")]
impl ScreenshotIface {
    async fn screenshot(
        &self,
        parent_window: &str,
        options: HashMap<String, OwnedValue>,
        #[zbus(header)] header: Header<'_>,
        #[zbus(connection)] conn: &zbus::Connection,
        #[zbus(object_server)] server: &zbus::ObjectServer,
    ) -> zbus::fdo::Result<OwnedObjectPath> {
        let sender = header
            .sender()
            .map(|s| s.to_string())
            .ok_or_else(|| zbus::fdo::Error::Failed("no sender".into()))?;
        let handle_token = options
            .get("handle_token")
            .and_then(|v| <&str>::try_from(&**v).ok())
            .map(str::to_string);
        let interactive = options
            .get("interactive")
            .and_then(|v| bool::try_from(&**v).ok());
        let ignore_token = *self.state.ignore_token.lock().unwrap();
        let index = self.state.calls.lock().unwrap().len();
        let token = match (&handle_token, ignore_token) {
            (Some(token), false) => token.clone(),
            _ => format!("portal_chosen_{index}"),
        };
        let handle = format!(
            "/org/freedesktop/portal/desktop/request/{}/{}",
            sender.trim_start_matches(':').replace('.', "_"),
            token
        );
        self.state.calls.lock().unwrap().push(ScreenshotCall {
            sender,
            parent_window: parent_window.to_string(),
            handle_token,
            interactive,
            handle: handle.clone(),
        });
        server
            .at(
                handle.as_str(),
                RequestIface {
                    state: self.state.clone(),
                    path: handle.clone(),
                },
            )
            .await
            .map_err(|e| zbus::fdo::Error::Failed(e.to_string()))?;

        let answer = self
            .state
            .answer
            .lock()
            .unwrap()
            .clone()
            .unwrap_or(PortalAnswer::Silent);
        let response: Option<(u32, HashMap<String, OwnedValue>)> = match answer {
            PortalAnswer::Screenshot { width, height } => {
                let dir = self.state.dir.lock().unwrap().clone();
                let path = png_file(&dir, index, width, height);
                self.state.files.lock().unwrap().push(path.clone());
                if *self.state.lock_dir.lock().unwrap() {
                    use std::os::unix::fs::PermissionsExt;
                    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o500)).unwrap();
                }
                let uri = OwnedValue::try_from(Value::from(file_uri(&path))).unwrap();
                Some((0, HashMap::from([("uri".to_string(), uri)])))
            }
            PortalAnswer::Cancelled => Some((1, HashMap::new())),
            PortalAnswer::Other => Some((2, HashMap::new())),
            PortalAnswer::RemoteUri => {
                let uri =
                    OwnedValue::try_from(Value::from("https://example.invalid/shot.png")).unwrap();
                Some((0, HashMap::from([("uri".to_string(), uri)])))
            }
            PortalAnswer::Silent => None,
        };
        if let Some(body) = response {
            // Answer after the method reply, as the real portal does.
            let conn = Connection::from(conn.clone());
            let path = handle.clone();
            std::thread::spawn(move || {
                std::thread::sleep(Duration::from_millis(30));
                conn.emit_signal(
                    None::<&str>,
                    path.as_str(),
                    "org.freedesktop.portal.Request",
                    "Response",
                    &body,
                )
                .expect("emit Response");
            });
        }
        OwnedObjectPath::try_from(handle).map_err(|e| zbus::fdo::Error::Failed(e.to_string()))
    }
}

/// A running fake portal, owning `org.freedesktop.portal.Desktop` on a
/// private session bus. Stops when dropped.
pub struct FakePortal {
    _conn: Connection,
    state: Arc<State>,
    dir: TempDir,
}

impl FakePortal {
    pub fn start(bus_address: &str, answer: PortalAnswer) -> Self {
        let dir = tempfile::Builder::new()
            .prefix("dosi-fake-portal-")
            .tempdir()
            .unwrap();
        let state = Arc::new(State::default());
        *state.answer.lock().unwrap() = Some(answer);
        *state.dir.lock().unwrap() = dir.path().to_path_buf();
        let conn = zbus::blocking::connection::Builder::address(bus_address)
            .unwrap()
            .name("org.freedesktop.portal.Desktop")
            .unwrap()
            .serve_at(
                "/org/freedesktop/portal/desktop",
                ScreenshotIface {
                    state: state.clone(),
                },
            )
            .unwrap()
            .build()
            .expect("fake portal connection");
        Self {
            _conn: conn,
            state,
            dir,
        }
    }

    pub fn set_answer(&self, answer: PortalAnswer) {
        *self.state.answer.lock().unwrap() = Some(answer);
    }

    pub fn ignore_handle_token(&self, ignore: bool) {
        *self.state.ignore_token.lock().unwrap() = ignore;
    }

    /// After writing a PNG, make the directory read-only so it cannot be
    /// deleted (the test restores the mode before the directory is dropped).
    pub fn lock_dir_after_write(&self, lock: bool) {
        *self.state.lock_dir.lock().unwrap() = lock;
    }

    /// Where the synthetic PNGs are written.
    pub fn dir(&self) -> &std::path::Path {
        self.dir.path()
    }

    pub fn calls(&self) -> Vec<ScreenshotCall> {
        self.state.calls.lock().unwrap().clone()
    }

    /// Request paths that were closed.
    pub fn closes(&self) -> Vec<String> {
        self.state.closes.lock().unwrap().clone()
    }

    /// Every PNG handed out so far.
    pub fn files(&self) -> Vec<PathBuf> {
        self.state.files.lock().unwrap().clone()
    }
}
