//! The acquisition boundary: screen, webcam, foreground window and running
//! programs, behind [`AcquisitionProvider`] — the native one, and a fake for
//! tests.

use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Mutex;

use chrono::Utc;

use crate::model::WindowInfo;

use super::screenshot::{self, ScreenCapture};
use super::{active_window, webcam};

/// Abstract acquisition boundary for heavy or privacy-sensitive queries:
/// screen captures, webcam frames, foreground window, and process list.
///
/// Decouples decision and consent logic from native OS API calls, ensuring
/// tests can deterministically verify acquisition gating without real screen,
/// camera, or Win32 OS interaction.
pub trait AcquisitionProvider: Send + Sync {
    /// Capture the foreground monitor as a ScreenCapture.
    fn capture_screen(&self, blur: bool) -> anyhow::Result<ScreenCapture>;
    /// Capture a single webcam photo as base64-encoded JPEG.
    fn capture_webcam(&self) -> anyhow::Result<String>;
    /// Query the foreground window info.
    fn current_window(&self) -> Option<WindowInfo>;
    /// Query the list of running top-level programs.
    fn running_programs(&self) -> anyhow::Result<Vec<WindowInfo>>;
}

/// Production acquisition provider invoking real native Win32/xcap/nokhwa implementations.
pub struct NativeAcquisitionProvider;

impl NativeAcquisitionProvider {
    pub fn new() -> Self {
        Self
    }
}

impl Default for NativeAcquisitionProvider {
    fn default() -> Self {
        Self::new()
    }
}

impl AcquisitionProvider for NativeAcquisitionProvider {
    fn capture_screen(&self, blur: bool) -> anyhow::Result<ScreenCapture> {
        screenshot::capture_foreground_monitor(blur)
    }

    fn capture_webcam(&self) -> anyhow::Result<String> {
        webcam::capture_jpg_base64()
    }

    fn current_window(&self) -> Option<WindowInfo> {
        active_window::current()
    }

    fn running_programs(&self) -> anyhow::Result<Vec<WindowInfo>> {
        active_window::running_programs()
    }
}

/// A callback the fake runs inside an acquisition, so a test can interleave
/// e.g. a revocation with an in-flight query.
pub type AcquisitionHook = Box<dyn Fn() + Send + Sync>;

/// The screen-capture callback; it receives the requested blur.
pub type ScreenCaptureHook = Box<dyn Fn(bool) + Send + Sync>;

/// In-memory fake acquisition provider with atomic counters and canned responses.
/// Completely side-effect-free: never calls native camera, window, input, or screen APIs.
pub struct FakeAcquisitionProvider {
    screen_calls: AtomicU32,
    webcam_calls: AtomicU32,
    window_calls: AtomicU32,
    process_calls: AtomicU32,
    last_blur: AtomicBool,
    canned_screen: Mutex<Option<ScreenCapture>>,
    canned_webcam: Mutex<Option<String>>,
    canned_window: Mutex<Option<WindowInfo>>,
    canned_processes: Mutex<Option<Vec<WindowInfo>>>,
    on_current_window: Mutex<Option<AcquisitionHook>>,
    on_capture_screen: Mutex<Option<ScreenCaptureHook>>,
    on_capture_webcam: Mutex<Option<AcquisitionHook>>,
    on_running_programs: Mutex<Option<AcquisitionHook>>,
}

#[allow(dead_code)]
impl FakeAcquisitionProvider {
    pub fn new() -> Self {
        Self {
            screen_calls: AtomicU32::new(0),
            webcam_calls: AtomicU32::new(0),
            window_calls: AtomicU32::new(0),
            process_calls: AtomicU32::new(0),
            last_blur: AtomicBool::new(false),
            canned_screen: Mutex::new(None),
            canned_webcam: Mutex::new(Some("/9j/fake_webcam".into())),
            canned_window: Mutex::new(Some(WindowInfo {
                app_name: "code.exe".into(),
                window_title: "main.rs".into(),
                ..Default::default()
            })),
            canned_processes: Mutex::new(Some(vec![WindowInfo {
                app_name: "code.exe".into(),
                window_title: "".into(),
                ..Default::default()
            }])),
            on_current_window: Mutex::new(None),
            on_capture_screen: Mutex::new(None),
            on_capture_webcam: Mutex::new(None),
            on_running_programs: Mutex::new(None),
        }
    }

    pub fn screen_calls(&self) -> u32 {
        self.screen_calls.load(Ordering::SeqCst)
    }

    pub fn webcam_calls(&self) -> u32 {
        self.webcam_calls.load(Ordering::SeqCst)
    }

    pub fn window_calls(&self) -> u32 {
        self.window_calls.load(Ordering::SeqCst)
    }

    pub fn process_calls(&self) -> u32 {
        self.process_calls.load(Ordering::SeqCst)
    }

    pub fn last_blur(&self) -> bool {
        self.last_blur.load(Ordering::SeqCst)
    }

    pub fn set_canned_screen(&self, screen: Option<ScreenCapture>) {
        *self.canned_screen.lock().unwrap() = screen;
    }

    pub fn set_canned_webcam(&self, webcam: Option<String>) {
        *self.canned_webcam.lock().unwrap() = webcam;
    }

    pub fn set_canned_window(&self, window: Option<WindowInfo>) {
        *self.canned_window.lock().unwrap() = window;
    }

    pub fn set_canned_processes(&self, processes: Option<Vec<WindowInfo>>) {
        *self.canned_processes.lock().unwrap() = processes;
    }

    pub fn set_on_current_window<F: Fn() + Send + Sync + 'static>(&self, f: F) {
        *self.on_current_window.lock().unwrap() = Some(Box::new(f));
    }

    pub fn set_on_capture_screen<F: Fn(bool) + Send + Sync + 'static>(&self, f: F) {
        *self.on_capture_screen.lock().unwrap() = Some(Box::new(f));
    }

    pub fn set_on_capture_webcam<F: Fn() + Send + Sync + 'static>(&self, f: F) {
        *self.on_capture_webcam.lock().unwrap() = Some(Box::new(f));
    }

    pub fn set_on_running_programs<F: Fn() + Send + Sync + 'static>(&self, f: F) {
        *self.on_running_programs.lock().unwrap() = Some(Box::new(f));
    }
}

impl Default for FakeAcquisitionProvider {
    fn default() -> Self {
        Self::new()
    }
}

impl AcquisitionProvider for FakeAcquisitionProvider {
    fn capture_screen(&self, blur: bool) -> anyhow::Result<ScreenCapture> {
        self.screen_calls.fetch_add(1, Ordering::SeqCst);
        self.last_blur.store(blur, Ordering::SeqCst);
        if let Some(ref cb) = *self.on_capture_screen.lock().unwrap() {
            cb(blur);
        }
        if let Some(ref capture) = *self.canned_screen.lock().unwrap() {
            Ok(capture.clone())
        } else {
            Ok(ScreenCapture {
                jpeg_base64: "/9j/fake_screen".into(),
                thumb_jpg_base64: Some("/9j/fake_thumb".into()),
                captured_at: Utc::now(),
                blurred: blur,
            })
        }
    }

    fn capture_webcam(&self) -> anyhow::Result<String> {
        self.webcam_calls.fetch_add(1, Ordering::SeqCst);
        if let Some(ref cb) = *self.on_capture_webcam.lock().unwrap() {
            cb();
        }
        if let Some(ref b64) = *self.canned_webcam.lock().unwrap() {
            Ok(b64.clone())
        } else {
            Err(anyhow::anyhow!("fake webcam unavailable"))
        }
    }

    fn current_window(&self) -> Option<WindowInfo> {
        self.window_calls.fetch_add(1, Ordering::SeqCst);
        if let Some(ref cb) = *self.on_current_window.lock().unwrap() {
            cb();
        }
        self.canned_window.lock().unwrap().clone()
    }

    fn running_programs(&self) -> anyhow::Result<Vec<WindowInfo>> {
        self.process_calls.fetch_add(1, Ordering::SeqCst);
        if let Some(ref cb) = *self.on_running_programs.lock().unwrap() {
            cb();
        }
        if let Some(ref progs) = *self.canned_processes.lock().unwrap() {
            Ok(progs.clone())
        } else {
            Ok(Vec::new())
        }
    }
}
